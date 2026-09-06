//! The local timer.
//!
//! Decision D1 and section 6 of `docs/notes/design/client-side-architecture.md`
//! move the schedule to the seller's device without changing what it is: the
//! cadence stays deterministic and cron-shaped, and only the location of the
//! timer moves. The seller configures it, the timer fires here, and the client
//! pulls whatever declarative work is pending; the server never says "do it
//! now", which is the causation the legal analysis turns on.
//!
//! Everything a tick may refuse on is decided here, before [`WorkSource`] is
//! touched at all. That ordering is the whole of the gate: a check made after
//! the request has gone out is not one. The four refusals are the seller's
//! sign-out, the absent console session, the entitlement, and the absent
//! marketplace session, and each is reported as itself rather than as a
//! generic "not syncing", because the seller acts on each differently.

use core::future::Future;
use core::pin::Pin;
use core::time::Duration;

use tam_types::{Marketplace, Timestamp};

use crate::entitlement::EntitlementGate;
use crate::session::SessionStore;
use crate::state::{BlockReason, WorkEvent};

/// What one pull of pending work returns, boxed so the trait stays
/// object-safe. The same shape `tam-api`'s `JwksSource` uses.
pub type PullFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<WorkEvent>, WorkError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkError(pub String);

impl core::fmt::Display for WorkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the work source refused: {}", self.0)
    }
}

impl core::error::Error for WorkError {}

/// Where the declarative work the control plane has queued comes from, and
/// what running it came to.
///
/// The events returned are what the seller is shown: what was claimed, what it
/// settled as, and what it stopped on. They are returned rather than written,
/// so the scheduler owns the order they are recorded in and a test can assert
/// that order without a running application.
pub trait WorkSource: Send + Sync {
    fn pull(&self, marketplace: Marketplace) -> PullFuture<'_>;
}

/// A source with nothing in it. The honest default for a build with no
/// control-plane work source bound.
#[derive(Debug, Default)]
pub struct NoWork;

impl WorkSource for NoWork {
    fn pull(&self, _marketplace: Marketplace) -> PullFuture<'_> {
        Box::pin(async { Ok(vec![WorkEvent::Idle]) })
    }
}

/// What this device knows about itself before it asks for work.
///
/// A struct rather than four parameters, so adding a fifth fact is a change to
/// one type rather than to every caller, and so the three device-wide facts
/// and the per-marketplace one are visibly different things.
pub struct Readiness<'a> {
    /// The last check-in said the seller signed this device out.
    pub revoked: bool,
    /// The last check-in found a console session to speak under.
    pub signed_in: bool,
    pub gate: &'a EntitlementGate,
    /// Where this device's marketplace sessions are. Read per tick rather than
    /// remembered, because the seller may sign in to a marketplace between two
    /// ticks and should not have to wait for a third.
    pub sessions: &'a dyn SessionStore,
}

/// What one tick did, per marketplace, in the order it happened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickReport {
    pub events: Vec<(Marketplace, WorkEvent)>,
}

impl TickReport {
    /// The marketplaces this tick refused, and why. A view rather than a
    /// second field, so there is one record of what happened.
    #[must_use]
    pub fn blocked(&self) -> Vec<(Marketplace, BlockReason)> {
        self.events
            .iter()
            .filter_map(|(marketplace, event)| match *event {
                WorkEvent::Blocked { reason } => Some((*marketplace, reason)),
                WorkEvent::Idle
                | WorkEvent::Held { .. }
                | WorkEvent::Started { .. }
                | WorkEvent::Settled { .. }
                | WorkEvent::Parked { .. }
                | WorkEvent::Abandoned { .. }
                | WorkEvent::Failed { .. } => None,
            })
            .collect()
    }
}

/// Whether a scheduler tick is due, given when the last one ran.
///
/// The phone's missing timer, in one predicate. D3 limits a phone to work the
/// seller starts, and the platform agrees: Android's Doze stops
/// `JobScheduler` and therefore `WorkManager`, so a resume is the only moment
/// a phone can act at all. But a resume is also a moment a seller produces
/// twenty times an hour, and a full cycle per resume is twenty work claims
/// posted to the control plane and twenty rounds of marketplace requests from
/// a handset. The check-in is not gated by this and must not be — it is the
/// only channel by which a phone learns it was signed out — and the work pull
/// is the half with no such warrant.
///
/// The cadence read is [`Scheduler::DEFAULT_CADENCE`], the hour the desktop
/// timer already keeps, rather than a second number invented for phones: one
/// founder-gated limit, in one place.
///
/// A clock that has moved backwards since the last tick reads as not due, and
/// resolves itself once the clock passes the stamp. Answering "due" to a
/// backwards jump would hand an oscillating clock a pull on every resume,
/// which is the cost this exists to remove.
///
/// Compiled for the mobile build and for tests. The desktop timer deliberately
/// does not consult it: `tokio::time::interval` already fires at the cadence,
/// and measuring that same gap on a wall clock could round to a millisecond
/// under it and skip an hourly tick.
#[cfg(any(mobile, test))]
pub(crate) fn work_is_due(last: Option<Timestamp>, now: Timestamp, cadence: Duration) -> bool {
    let Some(last) = last else {
        return true;
    };
    let cadence = i64::try_from(cadence.as_millis()).unwrap_or(i64::MAX);
    now.0.saturating_sub(last.0) >= cadence
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

    /// Why this marketplace is not to be worked, or `None` to go ahead.
    ///
    /// Ordered by what the seller can do about it: the sign-out they performed
    /// themselves, then the console sign-in that resolves itself, then the
    /// entitlement, then the marketplace login. A store that cannot be read is
    /// not a refusal — it is a fault, and it travels as one.
    async fn refusal(
        &self,
        ready: &Readiness<'_>,
        marketplace: Marketplace,
        now: Timestamp,
    ) -> Result<Option<BlockReason>, WorkError> {
        if ready.revoked {
            return Ok(Some(BlockReason::Revoked));
        }
        if !ready.signed_in {
            return Ok(Some(BlockReason::NotSignedIn));
        }
        if !ready.gate.may_work(marketplace, now) {
            return Ok(Some(BlockReason::NotEntitled));
        }
        let held = ready
            .sessions
            .get(marketplace)
            .await
            .map_err(|why| WorkError(why.to_string()))?;
        if held.is_none() {
            return Ok(Some(BlockReason::NoSession));
        }
        Ok(None)
    }

    /// One tick. Every refusal is decided before the work source is reached,
    /// so a refused marketplace produces no call rather than a call whose
    /// result is discarded.
    pub async fn tick<W: WorkSource + ?Sized>(
        &self,
        ready: &Readiness<'_>,
        source: &W,
        now: Timestamp,
    ) -> TickReport {
        let mut report = TickReport::default();
        for marketplace in self.marketplaces.iter().copied() {
            match self.refusal(ready, marketplace, now).await {
                Ok(Some(reason)) => {
                    report
                        .events
                        .push((marketplace, WorkEvent::Blocked { reason }));
                    continue;
                }
                Ok(None) => {}
                Err(why) => {
                    report
                        .events
                        .push((marketplace, WorkEvent::Failed { detail: why.0 }));
                    continue;
                }
            }
            match source.pull(marketplace).await {
                Ok(events) => report
                    .events
                    .extend(events.into_iter().map(|event| (marketplace, event))),
                Err(why) => report
                    .events
                    .push((marketplace, WorkEvent::Failed { detail: why.0 })),
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::{work_is_due, NoWork, PullFuture, Readiness, Scheduler, WorkSource};
    use crate::device::DeviceId;
    use crate::entitlement::{Claims, Entitlement, EntitlementGate};
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::{BlockReason, WorkEvent};
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::time::Duration;
    use tam_types::{Marketplace, Timestamp};

    /// Seconds, because the entitlement claims are JWT deadlines.
    const NOW_SECONDS: i64 = 1_756_000_000;
    /// The same instant as a `Timestamp`, which counts milliseconds.
    const NOW: Timestamp = Timestamp(NOW_SECONDS * 1_000);

    /// A source that records that it was reached. The count is the whole point
    /// of the gate tests: a gate that refuses after the call has already gone
    /// out is not a gate.
    #[derive(Debug, Default)]
    struct CountingSource(AtomicUsize);

    impl WorkSource for CountingSource {
        fn pull(&self, _marketplace: Marketplace) -> PullFuture<'_> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(vec![WorkEvent::Started {
                    item: "an-item".to_owned(),
                }])
            })
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

    async fn sessions_for(marketplaces: &[Marketplace]) -> MemorySessionStore {
        let store = MemorySessionStore::new();
        for marketplace in marketplaces {
            store
                .put(&SessionRecord {
                    marketplace: *marketplace,
                    account_label: None,
                    captured_at: NOW,
                    device_id: DeviceId::from_raw("11112222333344445555666677778888"),
                    jar: CookieJar::new(vec![Cookie {
                        name: "TESSession".to_owned(),
                        value: "value".to_owned(),
                    }]),
                })
                .await
                .expect("the fixture store accepts");
        }
        store
    }

    fn ready<'a>(gate: &'a EntitlementGate, sessions: &'a dyn SessionStore) -> Readiness<'a> {
        Readiness {
            revoked: false,
            signed_in: true,
            gate,
            sessions,
        }
    }

    #[tokio::test]
    async fn a_closed_gate_blocks_every_marketplace_and_reaches_no_work_source() {
        let source = CountingSource::default();
        let closed = EntitlementGate::closed();
        let sessions = sessions_for(&[Marketplace::Tpt, Marketplace::Tes]).await;
        let report = scheduler()
            .tick(&ready(&closed, &sessions), &source, NOW)
            .await;

        assert_eq!(
            report.blocked(),
            vec![
                (Marketplace::Tpt, BlockReason::NotEntitled),
                (Marketplace::Tes, BlockReason::NotEntitled),
            ],
            "with no entitlement every marketplace is refused, and the seller is told which and \
             why"
        );
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            0,
            "the gate must be consulted before the work source, not after"
        );
    }

    #[tokio::test]
    async fn a_revoked_device_stops_the_tick_before_anything_else_is_consulted() {
        let source = CountingSource::default();
        let gate = open_gate();
        let sessions = sessions_for(&[Marketplace::Tpt, Marketplace::Tes]).await;
        let report = scheduler()
            .tick(
                &Readiness {
                    revoked: true,
                    signed_in: true,
                    gate: &gate,
                    sessions: &sessions,
                },
                &source,
                NOW,
            )
            .await;

        assert_eq!(
            report.blocked(),
            vec![
                (Marketplace::Tpt, BlockReason::Revoked),
                (Marketplace::Tes, BlockReason::Revoked),
            ],
            "a device the seller signed out is refused as signed out, not as unentitled: the \
             seller did this on purpose and is owed the reason they chose"
        );
        assert_eq!(source.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_device_nobody_is_signed_in_on_pulls_nothing() {
        let source = CountingSource::default();
        let gate = open_gate();
        let sessions = sessions_for(&[Marketplace::Tpt, Marketplace::Tes]).await;
        let report = scheduler()
            .tick(
                &Readiness {
                    revoked: false,
                    signed_in: false,
                    gate: &gate,
                    sessions: &sessions,
                },
                &source,
                NOW,
            )
            .await;

        assert_eq!(
            report.blocked(),
            vec![
                (Marketplace::Tpt, BlockReason::NotSignedIn),
                (Marketplace::Tes, BlockReason::NotSignedIn),
            ],
            "with no console session there is nothing to reach the control plane under"
        );
        assert_eq!(source.0.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_marketplace_with_no_stored_session_is_not_pulled_for() {
        let source = CountingSource::default();
        let gate = open_gate();
        let sessions = sessions_for(&[Marketplace::Tes]).await;
        let report = scheduler()
            .tick(&ready(&gate, &sessions), &source, NOW)
            .await;

        assert_eq!(
            report.blocked(),
            vec![(Marketplace::Tpt, BlockReason::NoSession)],
            "a marketplace the seller has not signed in to has no session to compose a request \
             under, so no work is claimed for it"
        );
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            1,
            "and the marketplace that does have one is unaffected"
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
    async fn a_ready_device_reaches_the_work_source_for_every_marketplace_it_holds() {
        let gate = open_gate();
        let sessions = sessions_for(&[Marketplace::Tpt, Marketplace::Tes]).await;
        let report = scheduler()
            .tick(&ready(&gate, &sessions), &NoWork, NOW)
            .await;

        assert_eq!(
            report.events,
            vec![
                (Marketplace::Tpt, WorkEvent::Idle),
                (Marketplace::Tes, WorkEvent::Idle),
            ],
            "the tick runs and the control plane answers idle, which is a different fact from \
             being refused and is shown as one"
        );
        assert!(report.blocked().is_empty());
    }

    #[tokio::test]
    async fn one_revoked_marketplace_blocks_only_itself() {
        let source = CountingSource::default();
        let gate = gate_over(vec![Marketplace::Tes]);
        let sessions = sessions_for(&[Marketplace::Tpt, Marketplace::Tes]).await;
        let report = scheduler()
            .tick(&ready(&gate, &sessions), &source, NOW)
            .await;

        assert_eq!(
            report.blocked(),
            vec![(Marketplace::Tpt, BlockReason::NotEntitled)]
        );
        assert_eq!(
            report.events.last(),
            Some(&(
                Marketplace::Tes,
                WorkEvent::Started {
                    item: "an-item".to_owned()
                }
            ))
        );
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            1,
            "revoking one marketplace must not stop the others"
        );
    }

    /// The phone's cadence, case by case.
    ///
    /// The first row is the one that matters most: a phone that answered false
    /// with no previous tick would never work at all, because nothing else
    /// sets the stamp. The exact-cadence row is the one an implementation
    /// written with `>` fails, which would push every tick to the resume after
    /// the one it was due on.
    #[test]
    fn work_is_due_only_once_a_cadence_has_passed() {
        let hour = Scheduler::DEFAULT_CADENCE;
        let at = |ms: i64| Timestamp(1_756_000_000_000 + ms);
        let cases = [
            (
                None,
                at(0),
                true,
                "no previous tick: the first resume works",
            ),
            (
                Some(at(0)),
                at(1),
                false,
                "a second resume in the same breath does not",
            ),
            (
                Some(at(0)),
                at(3_599_999),
                false,
                "nor one a millisecond short of the hour",
            ),
            (
                Some(at(0)),
                at(3_600_000),
                true,
                "the hour exactly is due, or a tick slips to the resume after",
            ),
            (Some(at(0)), at(7_200_000), true, "and anything past it"),
            (
                Some(at(3_600_000)),
                at(0),
                false,
                "a clock that moved backwards is not due, and resolves itself",
            ),
        ];
        for (last, now, expected, why) in cases {
            assert_eq!(work_is_due(last, now, hour), expected, "{why}");
        }
    }
}
