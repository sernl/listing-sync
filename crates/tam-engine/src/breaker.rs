//! The fleet circuit breaker: the machine already halts one tenant's
//! inventory on any ambiguous create; this adds the rate trip — a rise in
//! adverse settlements across tenants halts the inventory for the whole
//! fleet, durably, in the same tables every lease scan reads.
//!
//! Adverse is `failed`, `ambiguous` and `blocked` together, because the
//! predicate this has to answer is whether the marketplace is working rather
//! than how far into a write each item got. A condition that stops every
//! tenant before the write settles `blocked` fleet-wide, and it is exactly
//! the shape of event the breaker exists to catch.

use tam_storage::{HaltCause, HaltRepo, JobRepo, StorageError};
use tam_types::Timestamp;

/// Judgement defaults, deliberately conservative: with fewer settlements
/// than the sample floor the breaker stays quiet, because three failures in
/// three attempts at dawn is a bad marketplace minute, not a fleet event.
pub const BREAKER_WINDOW_MS: i64 = 30 * 60 * 1000;
pub const BREAKER_MIN_SAMPLE: i64 = 5;
pub const BREAKER_TRIP_PERMILLE: i64 = 500;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BreakerReport {
    pub tripped: Vec<String>,
}

pub async fn run_breaker(
    jobs: &JobRepo,
    halts: &HaltRepo,
    now: Timestamp,
) -> Result<BreakerReport, StorageError> {
    let since = Timestamp(now.0 - BREAKER_WINDOW_MS);
    let mut report = BreakerReport::default();
    for window in jobs.recent_outcomes(since).await? {
        if window.settled < BREAKER_MIN_SAMPLE {
            continue;
        }
        let trips = window.adverse.saturating_mul(1000)
            >= BREAKER_TRIP_PERMILLE.saturating_mul(window.settled);
        if trips {
            halts
                .raise_fleet_inventory(
                    window.inventory,
                    &HaltCause {
                        raised_by: "breaker".to_owned(),
                        reason: format!(
                            "{} of {} settlements ended failed, ambiguous or blocked in the window",
                            window.adverse, window.settled
                        ),
                        at: now,
                    },
                )
                .await?;
            report.tripped.push(format!("{:?}", window.inventory));
        }
    }
    Ok(report)
}
