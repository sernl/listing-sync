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
//! The pass carries a second rule beside the deadline. A commit is chunked and
//! each chunk is one request the seller's console sends, so a closed console
//! or a server restart mid-chunk leaves a batch `importing` with nothing
//! driving it — a state the console reads as being created right now, and one
//! that refuses every bind — until the deadline abandons it days later, with
//! the products its finished rows became standing beside a batch that says it
//! was given up. The rule reads the stamp each claim writes on the batch: a
//! claim older than any chunk could run is a dead pass, and its batch is
//! settled where every row has an outcome and returned to the seller where any
//! has none. The stale rule runs first, so a batch that finished and was never
//! settled reaches its deadline as `imported` rather than `abandoned`.
//!
//! What this pass does not yet do is delete the bytes. Releasing the handle is
//! what it can do today: nothing in this repository deletes a blob — the object
//! store declares `put` and `get` and no `delete`, `delete_product` is a soft
//! delete that leaves blobs standing, and `stored_bytes` counts `blob` rows —
//! so the deletion lands with the route that first binds a file to a row, which
//! is where the references it would delete are created.

use tam_storage::{ImportBatchRepo, StorageError};
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

/// How old a chunk's claim has to be before the batch it belongs to is judged
/// left behind by a dead pass.
///
/// A chunk is `ROWS_PER_CHUNK` creates — a second or two of work, tens of
/// seconds under a slow store — so a claim a quarter of an hour old with the
/// batch still `importing` is a console that closed or a server that
/// restarted, not a chunk at work. A pacing figure rather than a seller-visible
/// ceiling, held here beside the pass it paces as `ROWS_PER_CHUNK` is held
/// beside the commit, rather than in `tam-limits` where the founder-gated
/// limits are. Generous by two orders over a chunk's own duration because
/// judging too soon has a cost: a live chunk whose batch was returned under it
/// finishes its rows and then finds no `importing` batch to settle, and the
/// seller has to press the button once more.
pub const STALE_CLAIM_SECS: u64 = 15 * 60;

/// A claim is judged dead only well past the seconds a chunk takes, checked
/// where the compiler checks it.
const _: () = assert!(
    STALE_CLAIM_SECS >= 5 * 60,
    "the stale window must dwarf a chunk's own duration"
);

/// What the batch records about why it was settled, in the seller's own words.
///
/// Stated rather than left null, because the console renders this line on a
/// batch that closed without the seller closing it, and "abandoned" with no
/// reason reads as something having gone wrong.
pub const SWEPT_DETAIL: &str =
    "This import wasn't finished in time, so it was closed and its files were removed.";

/// What one pass did, across both of its rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassReport {
    /// Batches past their deadline, settled to `abandoned`.
    pub abandoned: u64,
    /// Rows of those batches whose file handles were released.
    pub released: u64,
    /// Batches a dead pass left `importing` with every row settled, now
    /// `imported` or `failed` with the seller's notification written.
    pub settled: u64,
    /// Batches a dead pass left `importing` with rows still to create,
    /// returned to the seller to finish.
    pub reopened: u64,
}

/// One pass: the stale rule over every batch a dead pass left `importing`,
/// then the deadline over every batch past it.
///
/// `now` is passed in rather than read here, for the reason the rest of this
/// workspace passes clocks in: a pass that reads its own clock cannot be tested
/// at a boundary.
pub async fn pass(batches: &ImportBatchRepo, now: Timestamp) -> Result<PassReport, StorageError> {
    let window = i64::try_from(STALE_CLAIM_SECS)
        .unwrap_or(i64::MAX)
        .saturating_mul(1_000);
    let stale = batches
        .sweep_stale(Timestamp(now.0.saturating_sub(window)), now, SWEEP_BATCH)
        .await?;
    let expired = batches.sweep_pass(now, SWEEP_BATCH, SWEPT_DETAIL).await?;
    Ok(PassReport {
        abandoned: expired.abandoned,
        released: expired.released,
        settled: stale.settled,
        reopened: stale.reopened,
    })
}

#[cfg(test)]
mod tests {
    use super::{STALE_CLAIM_SECS, SWEEP_INTERVAL_SECS, SWEPT_DETAIL};

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

    /// A batch a dead pass left behind is returned to the seller long before
    /// its deadline would abandon it, or the stale rule would be a second
    /// deadline rather than a recovery. Stated as a relationship between the
    /// two numbers rather than as a comment, so moving either one that far
    /// fails here.
    #[test]
    fn a_stale_claim_is_judged_well_inside_the_deadline() {
        const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
        let window = u64::try_from(tam_limits::import::BATCH_EXPIRY_DAYS)
            .unwrap_or(0)
            .saturating_mul(SECONDS_PER_DAY);
        assert!(
            STALE_CLAIM_SECS
                .saturating_add(SWEEP_INTERVAL_SECS)
                .saturating_mul(24)
                < window,
            "a claim goes stale and is swept within {STALE_CLAIM_SECS}s plus one \
             {SWEEP_INTERVAL_SECS}s interval, which must be well inside a {window}s deadline"
        );
    }

    /// The seller reads this line on a batch that closed without them closing
    /// it, so it says what happened rather than only that it happened.
    #[test]
    fn the_recorded_reason_states_what_was_released() {
        assert!(
            SWEPT_DETAIL.contains("in time") && SWEPT_DETAIL.contains("removed"),
            "the line states both why the batch closed and what it cost: {SWEPT_DETAIL}"
        );
    }
}
