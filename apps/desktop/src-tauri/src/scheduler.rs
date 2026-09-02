//! The local timer.
//!
//! Decision D1 and section 6 of `docs/notes/design/client-side-architecture.md`
//! move the schedule to the seller's device without changing what it is: the
//! cadence stays deterministic and cron-shaped, and only the location of the
//! timer moves. The seller configures it, the timer fires here, and the client
//! pulls whatever declarative work is pending; the server never says "do it
//! now", which is the causation the legal analysis turns on.
//!
//! Nothing in this slice contacts a marketplace. [`WorkSource`] is the seam
//! where that will eventually happen and its only implementation is
//! [`NoWork`], which does nothing at all. The engine driver split that gives
//! it a real implementation is a separate stream.

use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use tam_types::{Marketplace, Timestamp};

use crate::entitlement::EntitlementGate;

/// What one pull of pending work returns, boxed so the trait stays
/// object-safe. The same shape `tam-api`'s `JwksSource` uses.
pub type PullFuture<'a> = Pin<Box<dyn Future<Output = Result<usize, WorkError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkError(pub String);

impl core::fmt::Display for WorkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the work source refused: {}", self.0)
    }
}

impl core::error::Error for WorkError {}

/// Where the declarative work the control plane has queued comes from.
///
/// The count returned is items accepted, and is reported rather than acted on;
/// executing an item is the driver's job, not the scheduler's.
pub trait WorkSource: Send + Sync {
    fn pull(&self, marketplace: Marketplace) -> PullFuture<'_>;
}

/// The only implementation in this slice: a source with nothing in it.
#[derive(Debug, Default)]
pub struct NoWork;

impl WorkSource for NoWork {
    fn pull(&self, _marketplace: Marketplace) -> PullFuture<'_> {
        Box::pin(async { Ok(0) })
    }
}

/// What one tick did, per marketplace, so the interface can name the device
/// that did the work and the reason it did not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickReport {
    pub pulled: Vec<(Marketplace, usize)>,
    /// Marketplaces the entitlement gate refused: lapsed, revoked, or past
    /// the grace deadline. Reported rather than silently skipped, because the
    /// seller is owed the reason their sync stopped.
    pub blocked: Vec<Marketplace>,
    pub failed: Vec<(Marketplace, WorkError)>,
}

/// A deterministic, cron-shaped timer over the marketplaces this device works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheduler {
    cadence: Duration,
    marketplaces: Vec<Marketplace>,
}

impl Scheduler {
    /// Hourly, which is also D11's token validity, so a device that is syncing
    /// is revalidating at the same rhythm.
    pub const DEFAULT_CADENCE: Duration = Duration::from_hours(1);

    #[must_use]
    pub fn new(cadence: Duration, marketplaces: Vec<Marketplace>) -> Self {
        Self {
            cadence,
            marketplaces,
        }
    }

    /// Every seller-device marketplace, at the default cadence.
    #[must_use]
    pub fn hourly_over_seller_device_marketplaces() -> Self {
        Self::new(
            Self::DEFAULT_CADENCE,
            Marketplace::ALL
                .into_iter()
                .filter(|marketplace| crate::connect::login_target(*marketplace).is_ok())
                .collect(),
        )
    }

    #[must_use]
    pub const fn cadence(&self) -> Duration {
        self.cadence
    }

    #[must_use]
    pub fn marketplaces(&self) -> &[Marketplace] {
        &self.marketplaces
    }

    /// One tick. The gate is consulted per marketplace before the work source
    /// is touched at all, so a refused marketplace produces no call rather
    /// than a call whose result is discarded.
    pub async fn tick<W: WorkSource + ?Sized>(
        &self,
        gate: &EntitlementGate,
        source: &W,
        now: Timestamp,
    ) -> TickReport {
        let mut report = TickReport::default();
        for marketplace in self.marketplaces.iter().copied() {
            if !gate.may_work(marketplace, now) {
                report.blocked.push(marketplace);
                continue;
            }
            match source.pull(marketplace).await {
                Ok(count) => report.pulled.push((marketplace, count)),
                Err(why) => report.failed.push((marketplace, why)),
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::{NoWork, PullFuture, Scheduler, TickReport, WorkSource};
    use crate::entitlement::{Claims, Entitlement, EntitlementGate};
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::time::Duration;
    use tam_types::{Marketplace, Timestamp};

    /// Seconds, because the entitlement claims are JWT deadlines.
    const NOW_SECONDS: i64 = 1_756_000_000;
    /// The same instant as a `Timestamp`, which counts milliseconds.
    const NOW: Timestamp = Timestamp(NOW_SECONDS * 1_000);

    /// A source that records that it was reached. The count is the whole point
    /// of the gate test: a gate that refuses after the call has already gone
    /// out is not a gate.
    #[derive(Debug, Default)]
    struct CountingSource(AtomicUsize);

    impl WorkSource for CountingSource {
        fn pull(&self, _marketplace: Marketplace) -> PullFuture<'_> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(1) })
        }
    }

    fn gate_over(marketplaces: Vec<Marketplace>) -> EntitlementGate {
        EntitlementGate::holding(Entitlement::from_verified_claims(Claims {
            sub: "org-1".to_owned(),
            aud: crate::entitlement::AUDIENCE.to_owned(),
            iss: crate::entitlement::ISSUER.to_owned(),
            device: "11112222333344445555666677778888".to_owned(),
            marketplaces,
            exp: NOW_SECONDS + 3_600,
            grace: NOW_SECONDS + 3_600 + 86_400,
        }))
    }

    fn open_gate() -> EntitlementGate {
        gate_over(vec![Marketplace::Tpt, Marketplace::Tes])
    }

    fn scheduler() -> Scheduler {
        Scheduler::new(
            Duration::from_mins(1),
            vec![Marketplace::Tpt, Marketplace::Tes],
        )
    }

    #[tokio::test]
    async fn a_closed_gate_blocks_every_marketplace_and_reaches_no_work_source() {
        let source = CountingSource::default();
        let report = scheduler()
            .tick(&EntitlementGate::closed(), &source, NOW)
            .await;
        assert_eq!(
            report.blocked,
            vec![Marketplace::Tpt, Marketplace::Tes],
            "with no entitlement every marketplace is refused, and the seller is told which"
        );
        assert!(report.pulled.is_empty());
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            0,
            "the gate must be consulted before the work source, not after"
        );
    }

    #[tokio::test]
    async fn the_default_scheduler_is_hourly_over_the_seller_device_marketplaces_only() {
        let scheduler = Scheduler::hourly_over_seller_device_marketplaces();
        assert_eq!(scheduler.cadence(), Scheduler::DEFAULT_CADENCE);
        assert!(
            !scheduler.marketplaces().contains(&Marketplace::Etsy),
            "Etsy's automation is sanctioned and runs server-side; a local timer for it would \
             put the request on the wrong machine"
        );
        assert!(scheduler.marketplaces().contains(&Marketplace::Tpt));
        assert!(scheduler.marketplaces().contains(&Marketplace::Tes));
    }

    #[tokio::test]
    async fn an_open_gate_reaches_the_work_source_and_this_slice_has_nothing_in_it() {
        let report = scheduler().tick(&open_gate(), &NoWork, NOW).await;
        assert_eq!(
            report,
            TickReport {
                pulled: vec![(Marketplace::Tpt, 0), (Marketplace::Tes, 0)],
                blocked: vec![],
                failed: vec![],
            },
            "the tick runs, and returns nothing to do, because no marketplace request is made \
             anywhere in this slice"
        );
    }

    #[tokio::test]
    async fn one_revoked_marketplace_blocks_only_itself() {
        let source = CountingSource::default();
        let report = scheduler()
            .tick(&gate_over(vec![Marketplace::Tes]), &source, NOW)
            .await;
        assert_eq!(report.blocked, vec![Marketplace::Tpt]);
        assert_eq!(report.pulled, vec![(Marketplace::Tes, 1)]);
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            1,
            "revoking one marketplace must not stop the others"
        );
    }
}
