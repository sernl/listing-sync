//! The expiry sweep: a batch nobody finished is settled, and the files
//! attached to it are released.
//!
//! An abandoned batch costs the seller storage they cannot see and cannot free
//! — bytes charged against their quota that belong to no product — which is why
//! the founder approved a deadline rather than leaving one open forever. The
//! deadline is stored on the batch when it is written, so a seller is told a
//! date and that date does not move under them when the constant does.
//!
//! Cross-tenant by construction, like the job-event pruner and the snapshot
//! retention pass beside it: one turn covers every organisation, so it runs on
//! a `tam_engine` pool with the SELECT and UPDATE grants migration 0058 gives
//! and sees nothing at all under `tam_app`'s forced row-level security.
//!
//! What this pass does not yet do is delete the bytes. Releasing the handle is
//! what it can do today: nothing in this repository deletes a blob — the object
//! store declares `put` and `get` and no `delete`, `delete_product` is a soft
//! delete that leaves blobs standing, and `stored_bytes` counts `blob` rows —
//! so the deletion lands with the route that first binds a file to a row, which
//! is where the references it would delete are created.

use tam_storage::{ImportBatchRepo, StorageError, SweepReport};
use tam_types::Timestamp;

/// How often one pass runs.
///
/// The deadline is measured in days, so the pass is not a latency-sensitive
/// loop: an hour is two orders under the shortest window it enforces and
/// matches the retention pruner beside it, so the two do not need separate
/// reasoning about how often a background loop should wake.
pub const SWEEP_INTERVAL_SECS: u64 = tam_limits::ledger::PRUNE_INTERVAL_SECS;

/// How many batches one pass settles.
///
/// One batch per organisation can be open at a time, so this is a ceiling on
/// organisations expiring inside one interval rather than on rows. It bounds
/// the statement's lock footprint; a backlog past it is settled by the next
/// pass an hour later, which is well inside a deadline measured in days.
pub const SWEEP_BATCH: i64 = 500;

/// One open batch per organisation means [`SWEEP_BATCH`] bounds organisations
/// rather than rows, so it has to be large enough that a pass is not the
/// binding constraint on how fast a backlog clears. A claim about two
/// constants, checked where the compiler checks it rather than in a test that
/// could not fail at run time for any input.
const _: () = assert!(
    SWEEP_BATCH >= 100,
    "the pass bounds organisations, not rows, so it clears a plausible backlog in one turn"
);

/// What the batch records about why it was settled, in the seller's own words.
///
/// Stated rather than left null, because the console renders this line on a
/// batch that closed without the seller closing it, and "abandoned" with no
/// reason reads as something having gone wrong.
pub const SWEPT_DETAIL: &str =
    "this import was not finished before its deadline, so it was closed and the files attached \
     to it were released";

/// One pass, settling every batch whose deadline has passed.
///
/// `now` is passed in rather than read here, for the reason the rest of this
/// workspace passes clocks in: a pass that reads its own clock cannot be tested
/// at a boundary.
pub async fn pass(batches: &ImportBatchRepo, now: Timestamp) -> Result<SweepReport, StorageError> {
    batches.sweep_pass(now, SWEEP_BATCH, SWEPT_DETAIL).await
}

#[cfg(test)]
mod tests {
    use super::{SWEEP_INTERVAL_SECS, SWEPT_DETAIL};

    /// The pass has to wake often enough that a batch is swept near its
    /// deadline rather than days past it. Stated as a relationship between the
    /// two numbers rather than as a comment, so moving either one that far
    /// fails here.
    #[test]
    fn the_pass_wakes_far_more_often_than_the_deadline_it_enforces() {
        const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
        let window = u64::try_from(tam_limits::import::BATCH_EXPIRY_DAYS)
            .unwrap_or(0)
            .saturating_mul(SECONDS_PER_DAY);
        assert!(
            SWEEP_INTERVAL_SECS.saturating_mul(24) < window,
            "a pass every {SWEEP_INTERVAL_SECS}s must be well inside a {window}s deadline"
        );
    }

    /// The seller reads this line on a batch that closed without them closing
    /// it, so it says what happened rather than only that it happened.
    #[test]
    fn the_recorded_reason_states_what_was_released() {
        assert!(
            SWEPT_DETAIL.contains("deadline") && SWEPT_DETAIL.contains("released"),
            "the line states both why the batch closed and what it cost: {SWEPT_DETAIL}"
        );
    }
}
