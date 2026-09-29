//! This device's side of opening an imported file in a browser.
//!
//! A file imported from a marketplace stays on the seller's devices; Teachouse
//! never keeps it. When the seller opens one in the console, the server asks a
//! device that holds it for the bytes and passes them to the browser without
//! keeping them. This module is how a device hears that ask and answers it.
//!
//! It waits on one long poll, `GET /v1/devices/{device}/streams`, which the
//! server holds open until a stream is waiting or twenty-five seconds pass.
//! Each poll is also what tells the server this device can answer right now,
//! so the console calls a device offline within a minute of it stopping. Each
//! stream it hands over carries a capability the server signed with its
//! entitlement key, and nothing is sent until that capability checks out:
//! signed by a key this build embeds, meant for serving, not expired, naming
//! this device and exactly the stream, file and range asked for, and not used
//! before. Then the device opens the file from its sealed library and posts
//! exactly the bytes asked for; anything short of that is an empty answer
//! naming the reason, so the browser is told at once rather than left waiting.
//!
//! The checks and the cut are plain functions ([`verify`], [`Replays`],
//! [`range`]) so they are tested without a socket; [`Server::run`] is the loop
//! around them. Up to four streams are answered at once, the poll is repeated
//! as soon as it answers, and a poll that fails for the network waits five
//! seconds before the next. A device the seller signed out, one the server no
//! longer knows, or one nobody is signed in on stops serving; the schedule
//! starts it again once the device is in good standing.
//!
//! A PDF viewer reads a file a range at a time, and every range is a stream of
//! its own. The library opens a file whole, so the most recently opened file
//! is kept open for a minute, which is what spares a sixty-page PDF from being
//! decrypted sixty times. One file only, and only one of at most
//! [`KEPT_OPEN_BYTES_MAX`]: the point is the next few ranges, not a cache.
//!
//! On Android the loop runs only while [`should_serve`] says so: while the
//! app is on screen, while a stream is being answered, and for ten minutes
//! after the later of those. While it runs, the phone holds a foreground
//! service with the notification "Teachouse is sharing your files" and a
//! partial wake lock, so it keeps answering with the screen off; when it
//! stops, the service goes with it. The limits are the platform's and they are
//! real. Doze does not cut a foreground service's network, but some
//! manufacturers' battery savers stop a service however it was started, and
//! Android 15 ends a `dataSync` service after six hours in a day. Ten minutes
//! after the last use the service stops on purpose: a locked phone that is not
//! serving does not poll, and the console tells the seller that device is
//! offline and to open the app there. `docs/notes/design/android-client.md`
//! has the same account.

use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use tam_domain::serve::{Claims, Refusal, StreamRequest, AUDIENCE, ISSUER};
use tam_types::ContentHash;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant;

use crate::control_plane::{Answer, Answered, HttpControlPlane};
use crate::device::DeviceId;
use crate::entitlement::{PUBLIC_KEY_BYTES, SMALL_ORDER_KEY};
use crate::heartbeat::ControlPlaneError;
use crate::library::{hash_from_hex, Library};

/// How many streams are answered at once. More waits for a free one before
/// the next poll, which is the back-pressure: a device already sending four
/// ranges does not ask for a fifth.
pub const CONCURRENT_STREAMS: usize = 4;

/// How long a poll that failed for the network waits before the next.
pub const BACKOFF: Duration = Duration::from_secs(5);

/// How far past its `exp` a capability is still accepted, for a clock on this
/// device that runs a little behind the server's.
pub const LEEWAY_SECS: i64 = 30;

/// How long a phone goes on serving after the seller last had the app on
/// screen or a stream was last answered.
pub const LINGER: Duration = Duration::from_mins(10);

/// How long the most recently opened file stays open without being read.
pub const KEPT_OPEN_FOR: Duration = Duration::from_mins(1);

/// The largest file kept open between streams. A larger one is opened for
/// each stream and let go: holding it would put the whole file in memory for
/// a minute on a phone that may not have it to spare.
pub const KEPT_OPEN_BYTES_MAX: usize = 128 * 1024 * 1024;

/// Whether a phone should be serving now.
///
/// While the app is on screen, while a stream is being answered, and for
/// [`LINGER`] after the later of the two. `last_foreground` is the last moment
/// the app was seen on screen, which is now while it is; `last_request` is
/// the last moment a stream arrived or finished. Neither ever seen is a phone
/// that has no reason to serve.
///
/// A computer is always in the foreground by this reading, so the same loop
/// runs there and never stops for this reason.
#[must_use]
pub fn should_serve(
    now: Instant,
    last_foreground: Option<Instant>,
    last_request: Option<Instant>,
    in_flight: bool,
) -> bool {
    in_flight
        || last_foreground
            .max(last_request)
            .is_some_and(|latest| now.saturating_duration_since(latest) < LINGER)
}

/// Checks the capability a stream came with against the stream itself.
///
/// The signature must verify under one of `keys` with the serving audience
/// and the server as issuer, `exp` must not have passed by more than
/// [`LEEWAY_SECS`] at `now` (seconds since the epoch), and the claims must name
/// this device and exactly the stream, file and range the request asks for.
/// Anything else is [`Refusal::Capability`], and deliberately without saying
/// which check failed: the answer goes back to whoever asked.
///
/// Whether the stream was answered before is [`Replays`]'s question, asked
/// after this one so a forged capability cannot fill the replay set.
pub fn verify(
    request: &StreamRequest,
    device: &DeviceId,
    keys: &[[u8; PUBLIC_KEY_BYTES]],
    now: i64,
) -> Result<Claims, Refusal> {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[ISSUER]);
    validation.required_spec_claims =
        HashSet::from(["exp".to_owned(), "aud".to_owned(), "iss".to_owned()]);
    // Checked below against `now`, which is what makes this a function of its
    // arguments rather than of the clock.
    validation.validate_exp = false;
    let claims = keys
        .iter()
        .filter(|key| **key != SMALL_ORDER_KEY)
        .find_map(|key| {
            decode::<Claims>(
                &request.capability,
                &DecodingKey::from_ed_der(key),
                &validation,
            )
            .ok()
        })
        .ok_or(Refusal::Capability)?
        .claims;
    let fresh = claims
        .exp
        .checked_add(LEEWAY_SECS)
        .is_some_and(|until| now <= until);
    let named = claims.device == device.as_str()
        && claims.stream == request.stream
        && claims.hash == request.hash
        && claims.first == request.first
        && claims.last == request.last;
    if fresh && named {
        Ok(claims)
    } else {
        Err(Refusal::Capability)
    }
}

/// The streams this device has answered and whose capabilities still stand.
///
/// A capability is good for one stream, once. Remembered until it expires,
/// after which [`verify`] refuses it anyway, so the set holds only the last
/// couple of minutes of streams and is pruned on every admission.
#[derive(Debug, Default)]
pub struct Replays(HashMap<String, i64>);

impl Replays {
    /// Admits a verified stream the first time and refuses it after.
    pub fn admit(&mut self, claims: &Claims, now: i64) -> Result<(), Refusal> {
        self.0
            .retain(|_, exp| exp.saturating_add(LEEWAY_SECS) >= now);
        if self.0.contains_key(&claims.stream) {
            return Err(Refusal::Capability);
        }
        self.0.insert(claims.stream.clone(), claims.exp);
        Ok(())
    }
}

/// Bytes `first` to `last` inclusive of `file`, without copying them, or
/// [`Refusal::Range`] where that range is not inside the file.
pub fn range(file: &Bytes, first: u64, last: u64) -> Result<Bytes, Refusal> {
    let first = usize::try_from(first).map_err(|_| Refusal::Range)?;
    let last = usize::try_from(last).map_err(|_| Refusal::Range)?;
    if first > last || last >= file.len() {
        return Err(Refusal::Range);
    }
    Ok(file.slice(first..=last))
}

/// What the loop is told about the app's place on screen, and what it asks of
/// the platform while it runs.
///
/// A trait so the loop is one piece of code on every platform: a computer is
/// always in the foreground and needs nothing kept awake, and a phone answers
/// both through its own bridge.
pub trait Presence: Send + Sync {
    /// Whether the seller has the app on screen.
    fn foreground(&self) -> bool;

    /// Keep this process answering with the screen off (`true`), or let it
    /// go (`false`). Called once as a serving run starts and once as it ends.
    fn keep_serving(&self, on: bool);
}

/// A computer: always present, and nothing to keep awake.
#[derive(Debug, Clone, Copy, Default)]
pub struct Desktop;

impl Presence for Desktop {
    fn foreground(&self) -> bool {
        true
    }

    fn keep_serving(&self, _on: bool) {}
}

/// The most recently opened file, while it is still being read.
struct Opened {
    hash: ContentHash,
    bytes: Bytes,
    used: Instant,
}

/// Everything answering a stream needs apart from the network: who this
/// device is, the keys it trusts, its library, and what it has answered.
pub struct Responder {
    device: DeviceId,
    keys: Vec<[u8; PUBLIC_KEY_BYTES]>,
    library: Arc<Library>,
    replays: Mutex<Replays>,
    recent: Mutex<Option<Opened>>,
}

impl core::fmt::Debug for Responder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Responder")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl Responder {
    #[must_use]
    pub fn new(device: DeviceId, keys: Vec<[u8; PUBLIC_KEY_BYTES]>, library: Arc<Library>) -> Self {
        Self {
            device,
            keys,
            library,
            replays: Mutex::new(Replays::default()),
            recent: Mutex::new(None),
        }
    }

    /// The bytes one stream asks for, or why it gets none, at `now` seconds
    /// since the epoch. Every check in the order the contract states it: the
    /// capability, then whether it was used, then the file, then the range.
    pub async fn prepare(&self, request: &StreamRequest, now: i64) -> Result<Bytes, Refusal> {
        let claims = verify(request, &self.device, &self.keys, now)?;
        self.replays.lock().await.admit(&claims, now)?;
        let file = self.open(&claims.hash).await?;
        range(&file, claims.first, claims.last)
    }

    /// One file's plaintext, from the file kept open where it is that one.
    ///
    /// The lock is held across the library read on purpose: four ranges of a
    /// file nobody has opened yet arrive together, and the first opens it
    /// while the other three wait for it rather than each decrypting their
    /// own copy.
    async fn open(&self, hex: &str) -> Result<Bytes, Refusal> {
        let hash = hash_from_hex(hex).ok_or(Refusal::Missing)?;
        let now = Instant::now();
        let mut recent = self.recent.lock().await;
        if let Some(opened) = recent
            .as_mut()
            .filter(|opened| opened.hash == hash && now.duration_since(opened.used) < KEPT_OPEN_FOR)
        {
            opened.used = now;
            return Ok(opened.bytes.clone());
        }
        // Let the old file go before opening the next, so no more than one
        // is held at once.
        *recent = None;
        let bytes = match self.library.read(hash).await {
            Ok(Some(plaintext)) => Bytes::from(plaintext),
            Ok(None) => return Err(Refusal::Missing),
            Err(why) => {
                eprintln!("a file asked for could not be opened from the library: {why}");
                return Err(Refusal::Missing);
            }
        };
        if bytes.len() <= KEPT_OPEN_BYTES_MAX {
            *recent = Some(Opened {
                hash,
                bytes: bytes.clone(),
                used: now,
            });
        }
        drop(recent);
        Ok(bytes)
    }

    /// Lets the kept file go once nobody has read it for [`KEPT_OPEN_FOR`].
    async fn let_go_of_stale(&self, now: Instant) {
        let mut recent = self.recent.lock().await;
        if recent
            .as_ref()
            .is_some_and(|opened| now.duration_since(opened.used) >= KEPT_OPEN_FOR)
        {
            *recent = None;
        }
    }
}

/// Why a serving run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// A phone nobody has used for [`LINGER`].
    Idle,
    /// The server would not take this device's polls: nobody is signed in,
    /// the seller signed the device out, or the server does not know it.
    /// Serving waits for a check-in to settle the device's standing.
    Stopped(ControlPlaneError),
}

/// The serving loop and what it shares with the streams it answers.
pub struct Server {
    plane: Arc<HttpControlPlane>,
    responder: Responder,
    /// Streams being answered now.
    in_flight: AtomicUsize,
    /// When a stream last arrived or finished, for [`should_serve`].
    last_request: Mutex<Option<Instant>>,
}

impl core::fmt::Debug for Server {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Server")
            .field("responder", &self.responder)
            .finish_non_exhaustive()
    }
}

/// Counts a stream out however its task ends.
struct InFlight(Arc<Server>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Server {
    #[must_use]
    pub fn new(plane: Arc<HttpControlPlane>, responder: Responder) -> Self {
        Self {
            plane,
            responder,
            in_flight: AtomicUsize::new(0),
            last_request: Mutex::new(None),
        }
    }

    /// Serves until [`should_serve`] says stop or the server stops taking
    /// this device's polls, holding the platform's keep-alive for exactly
    /// that long.
    pub async fn run(self: Arc<Self>, presence: Arc<dyn Presence>) -> Ended {
        presence.keep_serving(true);
        let ended = Arc::clone(&self).serve(presence.as_ref()).await;
        presence.keep_serving(false);
        ended
    }

    async fn serve(self: Arc<Self>, presence: &dyn Presence) -> Ended {
        let permits = Arc::new(Semaphore::new(CONCURRENT_STREAMS));
        let mut last_foreground = None;
        loop {
            let now = Instant::now();
            if presence.foreground() {
                last_foreground = Some(now);
            }
            let last_request = *self.last_request.lock().await;
            let in_flight = self.in_flight.load(Ordering::SeqCst) > 0;
            if !should_serve(now, last_foreground, last_request, in_flight) {
                return Ended::Idle;
            }
            self.responder.let_go_of_stale(now).await;
            match self.plane.stream_requests(&self.responder.device).await {
                Ok(asked) => {
                    for request in asked.requests {
                        self.touch().await;
                        let Ok(permit) = Arc::clone(&permits).acquire_owned().await else {
                            // Never closed, so never reached; ending the run
                            // is the honest reading if it ever were.
                            return Ended::Idle;
                        };
                        self.in_flight.fetch_add(1, Ordering::SeqCst);
                        let counted = InFlight(Arc::clone(&self));
                        tauri::async_runtime::spawn(async move {
                            counted.0.answer(request).await;
                            counted.0.touch().await;
                            drop(counted);
                            drop(permit);
                        });
                    }
                }
                Err(
                    why @ (ControlPlaneError::NoSession
                    | ControlPlaneError::Revoked
                    | ControlPlaneError::Unregistered
                    | ControlPlaneError::Denied(_)),
                ) => return Ended::Stopped(why),
                Err(why) => {
                    eprintln!(
                        "this device could not wait for files to open, and will retry: {why}"
                    );
                    tokio::time::sleep(BACKOFF).await;
                }
            }
        }
    }

    async fn touch(&self) {
        *self.last_request.lock().await = Some(Instant::now());
    }

    /// Answers one stream: the bytes, or an empty body naming why not.
    async fn answer(&self, request: StreamRequest) {
        // The stream id becomes a path segment, so anything that is not the
        // id shape the server issues is dropped here rather than sent.
        if uuid::Uuid::try_parse(&request.stream).is_err() {
            eprintln!("a stream with a malformed id was ignored");
            return;
        }
        let now = crate::run::wall_now().0.div_euclid(1_000);
        let answer = match self.responder.prepare(&request, now).await {
            Ok(body) => Answer::Bytes(body),
            Err(refusal) => {
                eprintln!("refused stream {}: {}", request.stream, refusal.word());
                Answer::Refused(refusal)
            }
        };
        let sent = match &answer {
            Answer::Bytes(body) => Some(body.len()),
            Answer::Refused(_) => None,
        };
        match self
            .plane
            .answer_stream(&self.responder.device, &request, answer)
            .await
        {
            Ok(Answered::Delivered) => {
                if let Some(len) = sent {
                    eprintln!(
                        "served stream {}: bytes {}-{} ({len} bytes)",
                        request.stream, request.first, request.last
                    );
                }
            }
            Ok(Answered::Gone) => {}
            Err(why) => eprintln!("stream {} was not answered: {why}", request.stream),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{range, should_serve, verify, Replays, Responder, LEEWAY_SECS, LINGER};
    use crate::entitlement::testing::{device, test_key, TestKey, NOW};
    use crate::entitlement::PUBLIC_KEY_BYTES;
    use crate::library::{Library, LibraryEntry};
    use bytes::Bytes;
    use core::time::Duration;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::sync::Arc;
    use tam_domain::serve::{Claims, Refusal, StreamRequest, AUDIENCE, ISSUER, VALIDITY_SECS};
    use tam_secrets::{hex_encode, Kek};
    use tam_types::{ContentHash, Marketplace, Timestamp};
    use tokio::time::Instant;

    const STREAM: &str = "0b7c4a56-8f0e-4c1a-9d33-2a6f1e5b7c90";
    const FILE: &[u8] = b"%PDF-1.7 a worksheet about fractions, kept on this device";

    fn hash() -> ContentHash {
        ContentHash(*blake3::hash(FILE).as_bytes())
    }

    fn claims(first: u64, last: u64) -> Claims {
        Claims {
            sub: "org-1".to_owned(),
            aud: AUDIENCE.to_owned(),
            iss: ISSUER.to_owned(),
            device: device().as_str().to_owned(),
            stream: STREAM.to_owned(),
            hash: hex_encode(&hash().0),
            first,
            last,
            exp: NOW + VALIDITY_SECS,
        }
    }

    fn sign(key: &EncodingKey, claims: &Claims) -> String {
        encode(&Header::new(jsonwebtoken::Algorithm::EdDSA), claims, key)
            .expect("the test capability signs")
    }

    /// The request the server would send with this capability.
    fn asked(key: &TestKey, claims: &Claims) -> StreamRequest {
        StreamRequest {
            stream: claims.stream.clone(),
            hash: claims.hash.clone(),
            first: claims.first,
            last: claims.last,
            capability: sign(&key.encoding, claims),
        }
    }

    fn embedded(key: &TestKey) -> Vec<[u8; PUBLIC_KEY_BYTES]> {
        key.only().to_vec()
    }

    async fn library_holding(bytes: &[u8]) -> Arc<Library> {
        let dir = std::env::temp_dir().join(format!(
            "teachouse-serve-{}-{}",
            std::process::id(),
            tam_secrets::random_token()
        ));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        let library = Library::open(&dir, Kek::from_bytes(&[0x3C; 32]).expect("a key"))
            .expect("the library opens");
        let entry = LibraryEntry {
            hash: ContentHash(*blake3::hash(bytes).as_bytes()),
            file_name: "fractions.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            byte_len: bytes.len() as u64,
            marketplace: Marketplace::Tpt,
            resource: "101".to_owned(),
            kept_at: Timestamp(1_000),
            pinned: false,
        };
        library.keep(entry, bytes).await.expect("the file is kept");
        Arc::new(library)
    }

    #[test]
    fn a_capability_for_exactly_this_stream_verifies() {
        let key = test_key();
        let good = claims(0, 9);
        assert_eq!(
            verify(&asked(&key, &good), &device(), &key.only(), NOW),
            Ok(good)
        );
    }

    #[test]
    fn a_capability_for_another_device_is_refused() {
        let key = test_key();
        let mut other = claims(0, 9);
        other.device = "99998888777766665555444433332222".to_owned();
        assert_eq!(
            verify(&asked(&key, &other), &device(), &key.only(), NOW),
            Err(Refusal::Capability)
        );
    }

    /// Every field the request carries must be the one the server signed: a
    /// capability lifted from one stream cannot open another, another file,
    /// or a wider range of the same file.
    #[test]
    fn a_request_that_differs_from_what_was_signed_is_refused() {
        let key = test_key();
        let signed = claims(0, 9);
        let honest = asked(&key, &signed);
        let altered = [
            StreamRequest {
                stream: "7d1b0e2a-3c4f-4a5b-8c6d-9e0f1a2b3c4d".to_owned(),
                ..honest.clone()
            },
            StreamRequest {
                hash: "ab".repeat(32),
                ..honest.clone()
            },
            StreamRequest {
                first: 1,
                ..honest.clone()
            },
            StreamRequest {
                last: 100,
                ..honest.clone()
            },
        ];
        for request in altered {
            assert_eq!(
                verify(&request, &device(), &key.only(), NOW),
                Err(Refusal::Capability),
                "{request:?}"
            );
        }
    }

    #[test]
    fn an_expired_capability_is_refused_after_the_leeway() {
        let key = test_key();
        let good = claims(0, 9);
        let request = asked(&key, &good);
        assert!(
            verify(&request, &device(), &key.only(), good.exp + LEEWAY_SECS).is_ok(),
            "a clock thirty seconds behind the server's still answers"
        );
        assert_eq!(
            verify(&request, &device(), &key.only(), good.exp + LEEWAY_SECS + 1),
            Err(Refusal::Capability)
        );
    }

    #[test]
    fn a_capability_signed_by_another_key_is_refused() {
        let trusted = test_key();
        let stranger = test_key();
        let request = asked(&stranger, &claims(0, 9));
        assert_eq!(
            verify(&request, &device(), &trusted.only(), NOW),
            Err(Refusal::Capability)
        );
        assert_eq!(
            verify(&request, &device(), &[], NOW),
            Err(Refusal::Capability),
            "a build with no key verifies nothing"
        );
    }

    #[test]
    fn a_capability_for_another_audience_is_refused() {
        let key = test_key();
        let mut entitlement = claims(0, 9);
        entitlement.aud = crate::entitlement::AUDIENCE.to_owned();
        assert_eq!(
            verify(&asked(&key, &entitlement), &device(), &key.only(), NOW),
            Err(Refusal::Capability)
        );
    }

    #[test]
    fn a_stream_is_answered_once() {
        let good = claims(0, 9);
        let mut replays = Replays::default();
        assert_eq!(replays.admit(&good, NOW), Ok(()));
        assert_eq!(replays.admit(&good, NOW + 1), Err(Refusal::Capability));
        // Once the capability is past its leeway it is forgotten here and
        // refused by `verify` instead, so the set does not grow for ever.
        let later = good.exp + LEEWAY_SECS + 1;
        assert_eq!(replays.admit(&claims(0, 1), later), Ok(()));
    }

    #[test]
    fn a_range_is_cut_inclusively_and_refused_outside_the_file() {
        let file = Bytes::from_static(b"0123456789");
        assert_eq!(range(&file, 2, 4), Ok(Bytes::from_static(b"234")));
        assert_eq!(range(&file, 9, 9), Ok(Bytes::from_static(b"9")));
        assert_eq!(range(&file, 0, 9), Ok(file.clone()));
        assert_eq!(range(&file, 5, 10), Err(Refusal::Range), "past the end");
        assert_eq!(range(&file, 10, 10), Err(Refusal::Range));
        assert_eq!(range(&file, 4, 2), Err(Refusal::Range), "backwards");
        assert_eq!(range(&file, 0, u64::MAX), Err(Refusal::Range));
    }

    #[tokio::test]
    async fn a_verified_stream_is_served_from_the_library() {
        let key = test_key();
        let responder = Responder::new(device(), embedded(&key), library_holding(FILE).await);
        let request = asked(&key, &claims(9, 15));
        assert_eq!(
            responder.prepare(&request, NOW).await,
            Ok(Bytes::from_static(&FILE[9..=15]))
        );
        assert_eq!(
            responder.prepare(&request, NOW).await,
            Err(Refusal::Capability),
            "the same stream again is a replay"
        );
        // A second range of the same file, as a PDF viewer asks for, from the
        // file kept open.
        let mut next = claims(0, 3);
        next.stream = "1f2e3d4c-5b6a-4978-8a9b-0c1d2e3f4a5b".to_owned();
        assert_eq!(
            responder.prepare(&asked(&key, &next), NOW).await,
            Ok(Bytes::from_static(&FILE[..=3]))
        );
    }

    #[tokio::test]
    async fn a_file_this_device_no_longer_holds_is_missing() {
        let key = test_key();
        let responder = Responder::new(
            device(),
            embedded(&key),
            library_holding(b"another file entirely").await,
        );
        assert_eq!(
            responder.prepare(&asked(&key, &claims(0, 3)), NOW).await,
            Err(Refusal::Missing)
        );
    }

    #[tokio::test]
    async fn a_range_past_the_end_of_the_file_is_refused_as_a_range() {
        let key = test_key();
        let responder = Responder::new(device(), embedded(&key), library_holding(FILE).await);
        let past = claims(0, FILE.len() as u64);
        assert_eq!(
            responder.prepare(&asked(&key, &past), NOW).await,
            Err(Refusal::Range)
        );
    }

    #[test]
    fn a_phone_serves_while_used_and_for_ten_minutes_after() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        assert!(!should_serve(at(0), None, None, false), "never used");
        assert!(should_serve(at(0), Some(at(0)), None, false), "on screen");
        assert!(
            should_serve(at(599), Some(at(0)), None, false),
            "within ten minutes of leaving the app"
        );
        assert!(
            !should_serve(start + LINGER, Some(at(0)), None, false),
            "ten minutes after leaving the app"
        );
        assert!(
            should_serve(at(900), Some(at(0)), Some(at(600)), false),
            "a stream answered while locked extends the window from that stream"
        );
        assert!(
            !should_serve(at(1_200), Some(at(0)), Some(at(600)), false),
            "and it runs out ten minutes after the last one"
        );
        assert!(
            should_serve(at(86_400), Some(at(0)), Some(at(0)), true),
            "never while a stream is being answered"
        );
    }
}
