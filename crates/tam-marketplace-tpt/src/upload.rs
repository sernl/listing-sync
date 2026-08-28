//! The out-of-band upload: thirteen hops between "here are some bytes" and
//! "here is the handle the product form accepts".
//!
//! Two opaque handles pass through it and they are not interchangeable.
//! `/uploads/upload_file` mints an [`UploadHandle`] naming a staged S3 object;
//! the processing queue exchanges that for a [`ProcessedHandle`], and only the
//! second is what `data[ItemDigital][product]` takes. Both are server-side
//! encrypted envelopes over an object path, so both redact in `Debug`.
//!
//! Nothing here sleeps on its own account. The queue is polled through the
//! [`Pause`] capability, so the worker binds a real timer, a test binds an
//! instant return, and this crate stays free of an async runtime.

use tam_marketplace::{AdapterError, IdempotencyKey, Pause};
use tam_types::{FailureCode, FailureDetail};

/// The staged-object handle `/uploads/upload_file` returns. Consumed by
/// `/uploads/process_file` and by nothing else — in particular it is never
/// what the product form takes.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadHandle(String);

impl core::fmt::Debug for UploadHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("UploadHandle(redacted)")
    }
}

impl UploadHandle {
    #[must_use]
    pub const fn new(handle: String) -> Self {
        Self(handle)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The processed-asset handle the queue's terminal answer carries. This is
/// what `data[ItemDigital][product]` posts and what `/converter/generate_thumbs`
/// takes.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessedHandle(String);

impl core::fmt::Debug for ProcessedHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProcessedHandle(redacted)")
    }
}

impl ProcessedHandle {
    #[must_use]
    pub const fn new(handle: String) -> Self {
        Self(handle)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An async job's identifier, which arrives only in the `x-queue-tracking-id`
/// response header. A flow that reads bodies alone never finds it.
#[derive(Clone, PartialEq, Eq)]
pub struct QueueJob(String);

impl core::fmt::Debug for QueueJob {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("QueueJob(redacted)")
    }
}

impl QueueJob {
    #[must_use]
    pub const fn new(job: String) -> Self {
        Self(job)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the thumbnail job's terminal answer carries. The collection key is
/// the second handle the product form consumes.
#[derive(Clone, PartialEq, Eq)]
pub struct ThumbnailCollection {
    collection_key: String,
    thumbnail_count: usize,
}

impl core::fmt::Debug for ThumbnailCollection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ThumbnailCollection({} thumbnails, key redacted)",
            self.thumbnail_count
        )
    }
}

impl ThumbnailCollection {
    #[must_use]
    pub const fn new(collection_key: String, thumbnail_count: usize) -> Self {
        Self {
            collection_key,
            thumbnail_count,
        }
    }

    #[must_use]
    pub fn collection_key(&self) -> &str {
        &self.collection_key
    }

    #[must_use]
    pub const fn thumbnail_count(&self) -> usize {
        self.thumbnail_count
    }
}

/// The interval between queue polls. Measured across the five recorded polls
/// as 1.152, 1.167 and 1.210 seconds — a fixed cadence with no backoff — so
/// this reproduces the client's own rhythm rather than inventing one.
pub const QUEUE_POLL_INTERVAL_MS: u32 = 1_200;

/// How many polls a job gets before the flow gives up on it. The recorded
/// jobs terminated in two and three polls for a 224 KB image; this ceiling is
/// this connector's own patience, not a measured upstream timeout, and it is
/// generous because the product slot accepts files up to four gibibytes.
pub const QUEUE_POLL_MAX: u32 = 150;

/// A queue answer, as the poll classifies it. The `data` member changes type
/// between an empty array and an object across these states, which is why the
/// three are named rather than deserialised into one shape.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueState {
    /// `status: 0`, `data: []`.
    Queued,
    /// `status: 1`, `data: {value, message}` where value is a 0..1 fraction.
    Running { fraction: f64 },
    /// `status: 2`, `data` carrying the job's payload.
    Complete(QueuePayload),
}

/// The two terminal payloads the two jobs produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuePayload {
    Processed(ProcessedHandle),
    Thumbnails(ThumbnailCollection),
}

/// The `?rand=<float>` cache-buster the client appends to every XHR write.
///
/// Derived from the idempotency key and the hop's ordinal rather than from a
/// random source: an adapter that reads entropy cannot be replayed, and a
/// cassette that records a random query string can never be matched again. It
/// is a cache-buster, so all TPT requires is that consecutive calls differ.
#[must_use]
pub fn cache_buster(key: IdempotencyKey, ordinal: u32) -> String {
    let mut mixed: u64 = 0x243F_6A88_85A3_08D3;
    for byte in key.0 .0 {
        mixed = mixed.rotate_left(7) ^ u64::from(byte);
        mixed = mixed.wrapping_mul(0x0100_0000_01B3);
    }
    mixed ^= u64::from(ordinal).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // Sixteen decimal digits behind the point, which is the shape the
    // captured `rand` values take. Integer arithmetic throughout: a float
    // here would be a lossy cast for no gain.
    let digits = mixed % 10_000_000_000_000_000;
    format!("0.{digits:016}")
}

/// The hop ordinals the cache-buster is derived from, so two calls to the
/// same endpoint in one submit do not collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hop {
    ProcessFile,
    GenerateThumbs,
    /// The nth poll of the nth job.
    QueuePoll {
        job: u32,
        attempt: u32,
    },
}

impl Hop {
    #[must_use]
    pub const fn ordinal(self) -> u32 {
        match self {
            Self::ProcessFile => 1,
            Self::GenerateThumbs => 2,
            Self::QueuePoll { job, attempt } => {
                // Offset past the two enqueue hops, then spread by job so a
                // poll of job two never reuses a poll of job one's buster.
                100_u32
                    .saturating_add(job.saturating_mul(1_000))
                    .saturating_add(attempt)
            }
        }
    }
}

/// A [`Pause`] that returns immediately. The cassette-driven tests replay a
/// recorded poll sequence, where a real 1.2-second wait would add nothing but
/// wall-clock time to the gated lane.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstantPause;

impl Pause for InstantPause {
    fn pause(&self, _ms: u32) -> impl core::future::Future<Output = ()> + Send {
        core::future::ready(())
    }
}

/// A job that never reached a terminal state. Not an ambiguity: no product
/// form has been posted at this point, so nothing landed and nothing needs
/// reconciling.
#[must_use]
pub fn queue_exhausted(polls: u32) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::UploadRejected,
        detail: FailureDetail(format!(
            "the upload processing job did not reach a terminal state in {polls} polls at \
             {QUEUE_POLL_INTERVAL_MS}ms; no product form was posted, so nothing was created"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{cache_buster, queue_exhausted, Hop, InstantPause, ProcessedHandle, UploadHandle};
    use tam_marketplace::{IdempotencyKey, Pause as _};
    use tam_types::Uuid;

    fn key(seed: u8) -> IdempotencyKey {
        IdempotencyKey(Uuid([seed; 16]))
    }

    #[test]
    fn the_cache_buster_is_a_function_of_the_key_and_the_hop() {
        assert_eq!(
            cache_buster(key(7), Hop::ProcessFile.ordinal()),
            cache_buster(key(7), Hop::ProcessFile.ordinal()),
            "a replayed submit must build the same url, or no cassette can match it"
        );
        assert_ne!(
            cache_buster(key(7), Hop::ProcessFile.ordinal()),
            cache_buster(key(7), Hop::GenerateThumbs.ordinal()),
            "consecutive calls must differ, which is the whole job of a cache-buster"
        );
        assert_ne!(
            cache_buster(key(7), Hop::ProcessFile.ordinal()),
            cache_buster(key(8), Hop::ProcessFile.ordinal()),
            "two attempts are two url sequences"
        );
    }

    #[test]
    fn the_cache_buster_looks_like_the_float_the_client_sends() {
        let buster = cache_buster(key(1), 1);
        assert!(
            buster.starts_with("0.") && buster.chars().count() == 18,
            "the captured values are a leading zero, a point and sixteen digits, got {buster:?}"
        );
        assert!(
            buster.chars().skip(2).all(|digit| digit.is_ascii_digit()),
            "nothing but digits behind the point, got {buster:?}"
        );
    }

    #[test]
    fn no_two_polls_of_either_job_share_an_ordinal() {
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(Hop::ProcessFile.ordinal());
        seen.insert(Hop::GenerateThumbs.ordinal());
        let mut count = 2;
        for job in 1..=2_u32 {
            for attempt in 0..50_u32 {
                seen.insert(Hop::QueuePoll { job, attempt }.ordinal());
                count += 1;
            }
        }
        assert_eq!(
            seen.len(),
            count,
            "a collision would replay one poll's url for another's"
        );
    }

    #[test]
    fn the_two_handles_never_print_themselves() {
        let upload = UploadHandle::new("stagedsecret".to_owned());
        let processed = ProcessedHandle::new("processedsecret".to_owned());
        let printed = format!("{upload:?} {processed:?}");
        assert!(
            !printed.contains("secret"),
            "both handles are encrypted envelopes over an object path, and Debug printed {printed}"
        );
    }

    #[test]
    fn the_instant_pause_returns_without_waiting() {
        futures::executor::block_on(InstantPause.pause(60_000));
    }

    #[test]
    fn an_exhausted_queue_is_a_refusal_and_not_an_ambiguity() {
        let refused = queue_exhausted(150);
        let tam_marketplace::AdapterError::Rejected { detail, .. } = refused else {
            panic!("no form was posted, so nothing landed and nothing is ambiguous");
        };
        assert!(
            detail.0.contains("nothing was created"),
            "the refusal states why it is safe to say so, got {detail:?}"
        );
    }
}
