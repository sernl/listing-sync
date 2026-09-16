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

    /// What this tick says about how soon the next one should be, in the
    /// coordinator's own vocabulary.
    ///
    /// Three answers rather than the event list, because the cadence turns on
    /// exactly three things: a pull that could not be made at all, a
    /// marketplace another of the seller's devices is holding, and everything
    /// else. A blocked marketplace is deliberately `Quiet` — it made no
    /// request, so there is nothing to back off from, and the refusal
    /// resolves itself the moment the seller signs in rather than after a
    /// delay this device imposed.
    #[must_use]
    pub fn discovery(&self) -> Discovery {
        let mut held = false;
        for (_, event) in &self.events {
            match *event {
                WorkEvent::Failed { .. } => return Discovery::Failed,
                WorkEvent::Held { .. } => held = true,
                WorkEvent::Idle
                | WorkEvent::Started { .. }
                | WorkEvent::Settled { .. }
                | WorkEvent::Parked { .. }
                | WorkEvent::Abandoned { .. }
                | WorkEvent::Blocked { .. } => {}
            }
        }
        if held {
            Discovery::Held
        } else {
            Discovery::Quiet
        }
    }
}

/// Whether the hourly deadline has come round, given when it last ran.
///
/// The one wall-clock predicate the coordinator keeps, and it is kept for the
/// deadline rather than for the fast pass: the hour is the founder-gated
/// figure ([`Scheduler::DEFAULT_CADENCE`], aligned with D11's token validity),
/// so it is measured against an instant rather than against how long this
/// process happens to have been running.
///
/// A clock that has moved backwards since the last run reads as not due, and
/// resolves itself once the clock passes the stamp. Answering "due" to a
/// backwards jump would hand an oscillating clock a full sweep every time it
/// oscillated, which is the cost this exists to remove.
pub(crate) fn work_is_due(last: Option<Timestamp>, now: Timestamp, cadence: Duration) -> bool {
    let Some(last) = last else {
        return true;
    };
    let cadence = i64::try_from(cadence.as_millis()).unwrap_or(i64::MAX);
    now.0.saturating_sub(last.0) >= cadence
}

/// What one discovery pass found, as far as the next one's timing is
/// concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discovery {
    /// The control plane answered and had nothing this device must act on.
    Quiet,
    /// Another of the seller's devices holds a marketplace this one asked
    /// about, so this one asks less often rather than polling a queue it
    /// cannot win.
    Held,
    /// The pass could not be made. The only outcome that backs off.
    Failed,
}

/// How often this device looks for work the seller created somewhere else,
/// with nothing held.
///
/// The figure the control plane already suggests on every claim
/// (`next_poll_ms` in `tam-api`'s work route), written here rather than read
/// off the wire: D1 makes the device's own timer the authority, and a server
/// that could set this cadence would be a server saying "now".
pub const DISCOVERY_IDLE: Duration = Duration::from_secs(10);

/// The same, where a marketplace is held by another of the seller's devices.
/// The server's other suggestion, for the same reason.
pub const DISCOVERY_HELD: Duration = Duration::from_secs(30);

/// How often this device reports what it holds and learns whether the seller
/// signed it out.
///
/// Five minutes rather than the hour the sweep keeps: the check-in is one
/// request carrying a session list, and the hour was the interval at which a
/// revocation, a lapsed plan and a renamed machine reached the console. It
/// bounds revocation latency, which is the reason it exists at all.
pub const CHECK_IN_EVERY: Duration = Duration::from_mins(5);

/// How often a signed-out device checks in, which is the only thing it does.
///
/// More often than an ordinary check-in, not less: while this device is
/// revoked it performs no work and makes no marketplace request, so the probe
/// is the cheapest call this client has, and what it is watching for is the
/// seller's explicit restore landing on the server. It never registers and
/// never restores; only the console's restore route clears the mark.
pub const REVOKED_PROBE: Duration = Duration::from_mins(1);

/// What a run of failed discovery passes waits, in order, before trying
/// again.
///
/// Bounded at a minute: the ceiling is the point of the ladder. An unbounded
/// backoff would turn a five-minute outage into an hour of silence, and the
/// discovery pass is the only thing standing between work the seller created
/// in the browser and an hour of waiting.
pub const DISCOVERY_BACKOFF: [Duration; 4] = [
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(40),
    Duration::from_mins(1),
];

/// Whether the seller has this device in front of them, and where that answer
/// comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// A computer with the application running. The process being alive is
    /// the whole of the condition.
    Running,
    /// A phone, whose answer is asked of the platform before every pass and
    /// is never inferred from a clock.
    ///
    /// D3 leaves a phone no background schedule, and polling from the
    /// background is a promise the platform breaks and a battery the seller
    /// notices. A time-boxed window after a resume is not this fact and is
    /// wrong in both directions: it stops discovering while the seller is
    /// still looking at the app — the exact case continuous discovery exists
    /// for — and it goes on polling after they have left. So the answer is
    /// read from the count of started activities the application keeps for the
    /// life of the process ([`crate::android_name::ForegroundSource`]), and
    /// this arm only records what was last read.
    Foreground,
}

/// What is due now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// Report what this device holds, and learn whether it was signed out.
    pub check_in: bool,
    /// Look for explicit work: an import run the seller started in the
    /// browser, and whatever the queue holds for this device now.
    pub discover: bool,
    /// The hourly deadline: one full cycle, which is a check-in and a
    /// discovery pass together.
    pub sweep: bool,
}

impl Due {
    /// Nothing to do, which is the honest answer for a phone the seller put
    /// away.
    const NOTHING: Self = Self {
        check_in: false,
        discover: false,
        sweep: false,
    };

    #[must_use]
    pub const fn any(self) -> bool {
        self.check_in || self.discover || self.sweep
    }
}

/// The two clocks a pass is decided against.
///
/// Two rather than one, because the questions are different. The hourly
/// deadline is a statement about the seller's day and is stamped on their wall
/// clock; the fast cadences are statements about this process and are measured
/// as elapsed time, so a wall-clock correction cannot stop a running device
/// checking in or discovering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clocks {
    /// The seller's wall clock, for the hourly deadline alone.
    pub wall: Timestamp,
    /// Monotonic elapsed time since this process started, for every fast
    /// cadence and for the failure ladder.
    pub since_start: Duration,
}

/// The one coordinator this installation runs: which of the three activities
/// is due, and how long to wait when none is.
///
/// Three activities on three cadences rather than one cycle on one, because
/// they cost different things and answer different questions. Discovery is a
/// claim poll and a read of the open-runs list — no marketplace is touched
/// unless an item is actually claimed — so it can run at the ten seconds the
/// server itself suggests. The check-in carries this device's session list, so
/// it runs at five minutes. The hourly sweep is the deterministic cron-shaped
/// deadline D1 keeps on the device, and it stays hourly: making it more
/// frequent would be a change to the schedule the seller configured, not a
/// repair.
///
/// Pure, and driven by facts the caller passes in: the two clock readings in
/// [`Clocks`], and — on a phone — whether the seller is looking at the
/// application. Every timing property (the ladder, the ceiling, the
/// coalescing, the foreground gate) is therefore a test with no clock, no
/// runtime, no platform and no network.
///
/// One of these per process, held by the supervisor loop. It is the structure
/// that makes "coalesce concurrent triggers" true rather than hoped for: a
/// start-up, a resume and a seller pressing a button all call
/// [`Self::triggered`] on the same value, and what they produce is one
/// immediate pass rather than three loops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coordination {
    cadence: Duration,
    presence: Presence,
    /// The hourly deadline's own stamp, on the seller's wall clock. Wall
    /// rather than monotonic deliberately: the hour is a cron-shaped deadline
    /// in the seller's day, it survives a process that slept, and
    /// [`work_is_due`] already states what a backwards clock does to it.
    last_sweep: Option<Timestamp>,
    /// The fast activities' stamps, as elapsed time since this process
    /// started.
    ///
    /// Monotonic, and that is a repair rather than a detail: measured on the
    /// wall clock, a correction half an hour backwards — an NTP step, a
    /// seller fixing their timezone — left a running device unable to reach
    /// its own ten-second discovery or five-minute check-in until wall time
    /// caught up again.
    last_check_in: Option<Duration>,
    last_discovery: Option<Duration>,
    /// Each half of the discovery pass's own memory of how it has been
    /// going. Two rather than one, because they fail and recover
    /// independently and one counter let either half's success delete the
    /// other's outage.
    imports: Cadence,
    work: Cadence,
    /// The last check-in that reached the server said this device was signed
    /// out.
    revoked: bool,
    /// What the platform last said about the seller being here. Always true
    /// under [`Presence::Running`]; on a phone it is whatever
    /// [`Self::observed_foreground`] was last handed, which the loop reads
    /// from the activity's lifecycle before every pass.
    present: bool,
    /// A trigger is waiting to be served: the next pass happens now rather
    /// than on the cadence.
    immediate: bool,
}

impl Coordination {
    /// A computer with the application running.
    #[must_use]
    pub const fn running(cadence: Duration) -> Self {
        Self::new(cadence, Presence::Running)
    }

    /// A phone, which discovers only while the seller is looking at it.
    ///
    /// It starts not present and stays that way until the platform says
    /// otherwise: `observed_foreground` is asked before every pass, and a
    /// build whose bridge answered nothing therefore polls nothing in the
    /// background. What still happens without an answer is the trigger — a
    /// resume is itself the seller arriving — which is served once and then
    /// stops.
    #[cfg(any(mobile, test))]
    #[must_use]
    pub const fn on_a_phone(cadence: Duration) -> Self {
        Self::new(cadence, Presence::Foreground)
    }

    const fn new(cadence: Duration, presence: Presence) -> Self {
        Self {
            cadence,
            presence,
            last_sweep: None,
            last_check_in: None,
            last_discovery: None,
            imports: Cadence::NEW,
            work: Cadence::NEW,
            revoked: false,
            present: matches!(presence, Presence::Running),
            immediate: true,
        }
    }

    /// Start-up, a resume, or a seller act in the application: check in and
    /// discover now.
    ///
    /// Idempotent, which is what coalescing means here: ten triggers between
    /// two passes produce one pass, because this sets a flag rather than
    /// queueing anything. It deliberately does not bring the hourly deadline
    /// forward; a seller who opens the application twenty times has not asked
    /// for twenty sweeps.
    ///
    /// It says nothing about presence. A resume is served because it is a
    /// trigger, and whether the passes after it continue is the platform's
    /// answer rather than this one's — which is the difference between
    /// discovering while the seller is here and discovering for a while after
    /// they have gone.
    pub const fn triggered(&mut self) {
        self.immediate = true;
    }

    /// What the platform says about the seller being here, read before every
    /// pass on a phone.
    ///
    /// On a computer this is not called and would change nothing: the process
    /// running is the whole of the condition there.
    pub const fn observed_foreground(&mut self, present: bool) {
        if matches!(self.presence, Presence::Foreground) {
            self.present = present;
            // A trigger does not outlive the foreground it was raised in. A
            // resume notification that arrived as the seller was already
            // leaving — or one raised by a console command whose window went
            // away — would otherwise sit here and fire a pass from the
            // background, which is the one thing a phone must not do.
            if !present {
                self.immediate = false;
            }
        }
    }

    /// What to do at `at`.
    #[must_use]
    pub fn due(&self, at: Clocks) -> Due {
        if !self.present && !self.immediate {
            // A phone the seller is not looking at asks for nothing: no
            // check-in, no discovery and no sweep. The trigger is the one
            // exception, and it is not an exception to the rule so much as the
            // rule's other half — a resume is the seller arriving, and it is
            // served once rather than opening a window.
            return Due::NOTHING;
        }
        if self.revoked {
            // The check-in probe and nothing else. Work discovery stops
            // because a revoked device may not work, and the probe continues
            // because an explicit restore on the server is the one thing this
            // device is waiting to observe.
            return Due {
                check_in: self.immediate
                    || passed(self.last_check_in, at.since_start, REVOKED_PROBE),
                discover: false,
                sweep: false,
            };
        }
        let sweep = work_is_due(self.last_sweep, at.wall, self.cadence);
        Due {
            // A sweep is a check-in and a discovery pass together, so the two
            // are not also reported as due: one deadline, one set of requests.
            check_in: !sweep
                && (self.immediate || passed(self.last_check_in, at.since_start, CHECK_IN_EVERY)),
            discover: !sweep
                && (self.immediate || passed(self.last_discovery, at.since_start, self.gap())),
            sweep,
        }
    }

    /// How long until the earliest activity is due, for the loop to sleep.
    ///
    /// Zero where something is due already. Capped at the cadence, so a
    /// device with nothing to do still reaches its hourly deadline.
    ///
    /// A phone the seller is not looking at parks on that same cap and asks
    /// for nothing when it wakes: the loop re-reads the lifecycle first, so
    /// the wake is a local read and never a request. What actually brings such
    /// a phone back is the resume, which is a trigger rather than a timer.
    #[must_use]
    pub fn sleep(&self, at: Clocks) -> Duration {
        if self.due(at).any() {
            return Duration::ZERO;
        }
        if !self.present {
            return self.cadence;
        }
        let check_in = left(
            self.last_check_in,
            at.since_start,
            if self.revoked {
                REVOKED_PROBE
            } else {
                CHECK_IN_EVERY
            },
        );
        if self.revoked {
            return check_in;
        }
        // The sweep's remainder is measured on the wall clock it is stamped
        // on, and capped at the cadence so a clock that jumped forwards or
        // backwards cannot produce a nap longer than an hour.
        check_in
            .min(left(self.last_discovery, at.since_start, self.gap()))
            .min(remaining(self.last_sweep, at.wall, self.cadence).min(self.cadence))
    }

    /// A check-in happened, whether or not it reached the server.
    ///
    /// Stamped either way: a device that restamped only on success would
    /// retry a dropped connection as fast as its loop could turn, and the
    /// check-in's own floor is what bounds revocation latency rather than any
    /// number of attempts inside it.
    pub const fn checked_in(&mut self, at: Clocks) {
        self.last_check_in = Some(at.since_start);
        self.immediate = false;
    }

    /// What a check-in that reached the server said about this device's
    /// standing. Only such a check-in may move it: an unreachable server says
    /// nothing about whether the seller signed this machine out.
    pub const fn observed_revoked(&mut self, revoked: bool) {
        self.revoked = revoked;
    }

    /// Stamps the start of every discovery poll, even while an earlier work
    /// task remains in flight. Completion never restamps a long-running task.
    pub const fn discovering(&mut self, at: Clocks) {
        self.last_discovery = Some(at.since_start);
        self.immediate = false;
    }

    /// Records the work queue's result without changing the discovery stamp.
    pub const fn discovered(&mut self, what: Discovery) {
        self.work.record(what);
    }

    /// Records the import poll's result independently: an idle work queue
    /// must not reset the import endpoint's retry delay.
    pub const fn imports_discovered(&mut self, what: Discovery) {
        self.imports.record(what);
    }

    /// The hourly deadline has started. It is a whole cycle, so it stamps the
    /// two activities it contains as well as itself — which is what stops the
    /// hour producing a sweep and then a second round of the same requests.
    pub const fn swept(&mut self, at: Clocks) {
        self.last_sweep = Some(at.wall);
        self.checked_in(at);
        self.discovering(at);
    }

    /// Both endpoints share a poll, so the healthy endpoint must not accelerate
    /// retries against the failing one.
    fn gap(&self) -> Duration {
        self.imports.gap().max(self.work.gap())
    }
}

/// Retry timing for one discovery source. Import and work results arrive
/// independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cadence {
    /// Consecutive failed passes on this half, saturating at the ladder's
    /// length. A success on this half — and only on this half — resets it.
    failures: usize,
    /// This half last found a marketplace held by another device.
    held: bool,
}

impl Cadence {
    const NEW: Self = Self {
        failures: 0,
        held: false,
    };

    /// What this half's finished call came to.
    const fn record(&mut self, what: Discovery) {
        match what {
            Discovery::Quiet => {
                self.failures = 0;
                self.held = false;
            }
            Discovery::Held => {
                self.failures = 0;
                self.held = true;
            }
            // Saturating rather than wrapping, and the ladder's last rung is
            // the ceiling: an outage lasting a day still retries every minute.
            Discovery::Failed => {
                if self.failures < DISCOVERY_BACKOFF.len() {
                    self.failures += 1;
                }
            }
        }
    }

    /// What this half asks the next pass to wait: the ladder while failing,
    /// the server's own held suggestion while another device holds a
    /// marketplace, and its idle one otherwise.
    fn gap(self) -> Duration {
        match self.failures.checked_sub(1) {
            Some(rung) => DISCOVERY_BACKOFF[rung.min(DISCOVERY_BACKOFF.len() - 1)],
            None if self.held => DISCOVERY_HELD,
            None => DISCOVERY_IDLE,
        }
    }
}

/// How much of `gap` is left since `last`, saturating at zero.
fn remaining(last: Option<Timestamp>, now: Timestamp, gap: Duration) -> Duration {
    let Some(last) = last else {
        return Duration::ZERO;
    };
    let gap = i64::try_from(gap.as_millis()).unwrap_or(i64::MAX);
    let done = now.0.saturating_sub(last.0);
    u64::try_from(gap.saturating_sub(done)).map_or(Duration::ZERO, Duration::from_millis)
}

/// Whether `gap` of monotonic time has passed since `last`. No stamp means
/// never done, which is due.
fn passed(last: Option<Duration>, now: Duration, gap: Duration) -> bool {
    last.is_none_or(|last| now.saturating_sub(last) >= gap)
}

/// How much of `gap` is left since `last`, on the monotonic clock.
fn left(last: Option<Duration>, now: Duration, gap: Duration) -> Duration {
    let Some(last) = last else {
        return Duration::ZERO;
    };
    gap.saturating_sub(now.saturating_sub(last))
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
            plan: crate::entitlement::Plan::Subscriber,
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

    /// The hourly deadline, case by case.
    ///
    /// The first row is the one that matters most: a device that answered
    /// false with no previous sweep would never sweep at all, because nothing
    /// else sets the stamp. The exact-cadence row is the one an implementation
    /// written with `>` fails, which would push every sweep to the pass after
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

    /// The coordinator, driven by a clock the test holds.
    mod coordination {
        use super::super::{
            Clocks, Coordination, Discovery, CHECK_IN_EVERY, DISCOVERY_BACKOFF, DISCOVERY_HELD,
            DISCOVERY_IDLE, REVOKED_PROBE,
        };
        use crate::scheduler::Scheduler;
        use core::time::Duration;
        use tam_types::Timestamp;

        const START: i64 = 1_756_000_000_000;

        /// Both clocks, `after` into the run. The wall clock is held in step
        /// with the monotonic one here; the test that separates them moves one
        /// and not the other, which is the whole of what it asserts.
        fn at(after: Duration) -> Clocks {
            Clocks {
                wall: Timestamp(START + i64::try_from(after.as_millis()).expect("the offset fits")),
                since_start: after,
            }
        }

        /// A computer that has served its first pass, so nothing is immediate
        /// and every cadence is measured from the start of the run.
        fn settled() -> Coordination {
            let mut plan = Coordination::running(Scheduler::DEFAULT_CADENCE);
            plan.swept(at(Duration::ZERO));
            plan.discovered(Discovery::Quiet);
            plan
        }

        /// A discovery pass begun and finished at one instant, which is what
        /// a cheap pass with an empty queue is.
        fn served(plan: &mut Coordination, at: Clocks, what: Discovery) {
            plan.discovering(at);
            plan.discovered(what);
        }

        /// A whole discovery pass, both halves, in the order
        /// `lib.rs::run_schedule` performs them: the pass is stamped as it
        /// begins, the import poll answers inline, and the work queue's
        /// answer lands afterwards from the supervised task.
        fn served_both(plan: &mut Coordination, at: Clocks, imports: Discovery, work: Discovery) {
            plan.discovering(at);
            plan.imports_discovered(imports);
            plan.discovered(work);
        }

        /// The same for the hourly deadline.
        fn served_sweep(plan: &mut Coordination, at: Clocks, what: Discovery) {
            plan.swept(at);
            plan.discovered(what);
        }

        /// One turn of the supervisor loop, as `lib.rs::run_schedule` takes
        /// it: whichever activity is due, served, and nothing else.
        fn serve(plan: &mut Coordination, now: Duration) {
            let due = plan.due(at(now));
            if due.sweep {
                plan.swept(at(now));
                plan.discovered(Discovery::Quiet);
            } else {
                if due.check_in {
                    plan.checked_in(at(now));
                }
                if due.discover {
                    plan.discovering(at(now));
                    plan.discovered(Discovery::Quiet);
                }
            }
        }

        /// The founder's own case: a machine nobody has touched for hours, and
        /// work created in a browser somewhere else.
        ///
        /// Before the coordinator this waited for the hourly tick — up to an
        /// hour after the seller pressed a button in another window. The bound
        /// asserted is the server's own idle suggestion, and it holds after six
        /// minutes and after three hours alike: no local activity is required
        /// for discovery to go on happening, which is the whole difference
        /// between this and an attentive window opened by a keystroke.
        #[test]
        fn work_created_elsewhere_is_discovered_within_the_idle_gap_however_long_the_machine_has_been_idle(
        ) {
            for quiet in [Duration::from_mins(6), Duration::from_hours(3)] {
                let mut plan = settled();
                let mut now = Duration::ZERO;
                while now < quiet {
                    serve(&mut plan, now);
                    now += Duration::from_secs(1);
                }
                // The seller queues work in a browser at `quiet`. The device
                // learns of it by asking; this is when it next asks.
                let mut waited = None;
                while now < quiet + Duration::from_mins(2) {
                    let due = plan.due(at(now));
                    if due.discover || due.sweep {
                        waited = Some(now.saturating_sub(quiet));
                        break;
                    }
                    now += Duration::from_secs(1);
                }
                let waited = waited.expect("the device asks again within two minutes");
                assert!(
                    waited <= DISCOVERY_IDLE,
                    "after {quiet:?} idle the device asked again within {waited:?}, which must be \
                     inside the {DISCOVERY_IDLE:?} the control plane itself suggests"
                );
            }
        }

        /// A marketplace held by another of the seller's devices slows this
        /// one to the server's other suggestion, and no further.
        #[test]
        fn a_held_marketplace_polls_at_thirty_seconds_rather_than_stopping() {
            let mut plan = settled();
            served(&mut plan, at(Duration::ZERO), Discovery::Held);
            assert!(
                !plan.due(at(DISCOVERY_IDLE)).discover,
                "the idle gap is not the held gap"
            );
            assert!(
                plan.due(at(DISCOVERY_HELD)).discover,
                "a held answer is still discovery, at the held gap: an import this device could \
                 run must not wait on a marketplace another device holds"
            );
        }

        /// The ladder, and its ceiling.
        ///
        /// The whole run stays inside the hour on the wall clock, so what is
        /// asserted is the ladder rather than the hourly deadline arriving and
        /// serving a pass for its own reasons. The ceiling itself does not
        /// need a day of failures to state: it needs more failures than the
        /// ladder has rungs, which is what the second loop provides.
        #[test]
        fn failures_back_off_ten_twenty_forty_sixty_and_never_further() {
            let mut plan = settled();
            let mut now = Duration::ZERO;
            for (attempt, gap) in DISCOVERY_BACKOFF.iter().enumerate() {
                served(&mut plan, at(now), Discovery::Failed);
                assert!(
                    !plan
                        .due(at((now + *gap).saturating_sub(Duration::from_millis(1))))
                        .discover,
                    "failure {attempt} waits its whole rung"
                );
                assert!(
                    plan.due(at(now + *gap)).discover,
                    "failure {attempt} retries after {gap:?}"
                );
                now += *gap;
            }
            // A fifth, sixth and twentieth failure stay on the last rung —
            // twenty-four minutes of them, which is still inside the hour.
            for _ in 0..20_u32 {
                served(&mut plan, at(now), Discovery::Failed);
                now += Duration::from_mins(1);
            }
            assert!(
                now < Scheduler::DEFAULT_CADENCE,
                "the run stays inside the hour, or the sweep would serve the pass this asserts"
            );
            served(&mut plan, at(now), Discovery::Failed);
            let due = plan.due(at(now + Duration::from_mins(1)));
            assert!(
                !plan.due(at(now + Duration::from_secs(59))).discover,
                "and the ceiling is a whole minute rather than a shorter rung"
            );
            assert!(
                due.discover,
                "the ceiling is the last rung: however long an outage lasts, the device goes on \
                 retrying every minute rather than backing off for ever"
            );
        }

        /// One success clears the ladder, or an outage would leave a working
        /// device polling at the ceiling for the rest of the session.
        #[test]
        fn a_success_resets_the_ladder() {
            let mut plan = settled();
            served(&mut plan, at(Duration::ZERO), Discovery::Failed);
            served(&mut plan, at(Duration::from_secs(10)), Discovery::Quiet);
            assert!(
                plan.due(at(Duration::from_secs(10) + DISCOVERY_IDLE))
                    .discover,
                "back to the idle gap"
            );
        }

        /// The hourly deadline runs once an hour however often discovery runs,
        /// and a machine that was away does not replay the hours it missed as
        /// a burst.
        #[test]
        fn the_hourly_sweep_stays_hourly_under_continuous_discovery() {
            let mut plan = settled();
            let mut sweeps = 0_u32;
            let mut sweep_instants = Vec::new();
            // Four hours of ten-second passes, driven exactly as the loop
            // drives it.
            let mut now = Duration::ZERO;
            while now < Duration::from_hours(4) {
                let due = plan.due(at(now));
                if due.sweep {
                    sweeps += 1;
                    sweep_instants.push(now);
                    served_sweep(&mut plan, at(now), Discovery::Quiet);
                } else if due.discover {
                    served(&mut plan, at(now), Discovery::Quiet);
                }
                now += Duration::from_secs(1);
            }
            assert_eq!(
                sweeps, 3,
                "one sweep an hour and no more, at the hour, the second and the third — the \
                 fourth falls on the instant the window ends. Discovery ran fourteen thousand \
                 times in between and added not one sweep, which is the property: the cheap pass \
                 is frequent and the expensive deadline is not. Sweeps seen at {sweep_instants:?}"
            );
        }

        /// A signed-out device probes and does nothing else.
        ///
        /// Both halves matter. It must not discover, because a revoked device
        /// may not work; it must go on checking in, because the seller's
        /// explicit restore lands on the server and this probe is how this
        /// device observes it.
        #[test]
        fn a_signed_out_device_only_checks_in_and_never_discovers() {
            let mut plan = settled();
            plan.observed_revoked(true);
            let mut now = Duration::ZERO;
            let mut probes = 0_u32;
            while now < Duration::from_mins(10) {
                let due = plan.due(at(now));
                assert!(!due.discover, "a signed-out device discovers nothing");
                assert!(!due.sweep, "and sweeps nothing");
                if due.check_in {
                    probes += 1;
                    plan.checked_in(at(now));
                }
                now += Duration::from_secs(1);
            }
            assert_eq!(
                probes, 9,
                "one check-in a minute while revoked — the tenth falls on the instant the \
                 window ends — which is how a restore performed in the console is noticed"
            );
            // The restore lands, and this device returns to work.
            plan.observed_revoked(false);
            assert!(
                plan.due(at(now + DISCOVERY_IDLE)).discover,
                "observing that the mark is gone is what re-opens discovery, and nothing here \
                 cleared it locally"
            );
        }

        /// The check-in keeps its own five-minute floor, whatever discovery is
        /// doing.
        #[test]
        fn the_check_in_runs_on_its_own_cadence() {
            let mut plan = settled();
            let mut now = Duration::ZERO;
            let mut check_ins = 0_u32;
            while now < CHECK_IN_EVERY {
                let due = plan.due(at(now));
                if due.check_in {
                    check_ins += 1;
                    plan.checked_in(at(now));
                }
                if due.discover {
                    served(&mut plan, at(now), Discovery::Quiet);
                }
                now += Duration::from_secs(1);
            }
            assert_eq!(
                check_ins, 0,
                "the sweep at zero counts as this device's check-in, and nothing inside the \
                 five minutes after it is another one: discovery does not drag the check-in \
                 along with it"
            );
            assert!(
                plan.due(at(CHECK_IN_EVERY)).check_in,
                "and the floor is five minutes"
            );
        }

        /// Every trigger between two passes is one pass.
        #[test]
        fn concurrent_triggers_coalesce_into_one_immediate_pass() {
            let mut plan = settled();
            let now = at(Duration::from_secs(1));
            // Start-up, a resume and a seller pressing Check in now, in the
            // same breath.
            for _ in 0..3_u32 {
                plan.triggered();
            }
            let due = plan.due(now);
            assert!(
                due.check_in && due.discover,
                "the trigger is served at once"
            );
            assert!(
                !due.sweep,
                "and never brings the hourly deadline forward: twenty resumes are not twenty \
                 sweeps"
            );
            plan.checked_in(now);
            served(&mut plan, now, Discovery::Quiet);
            assert!(
                !plan.due(now).any(),
                "one pass serves all three triggers, rather than one pass each"
            );
        }

        /// A phone the seller is holding discovers for as long as they hold
        /// it, and one they have put down asks for nothing.
        ///
        /// Presence is the platform's answer, handed in before each pass. The
        /// half-hour is the half that matters: a window opened by a resume
        /// would have stopped discovering long before it, while the seller was
        /// still reading the screen and still waiting for the import they
        /// started in a browser.
        #[test]
        fn a_phone_discovers_while_the_seller_is_looking_at_it_and_not_after() {
            let mut plan = Coordination::on_a_phone(Scheduler::DEFAULT_CADENCE);
            let mut now = Duration::ZERO;
            let mut passes = 0_u32;
            // Half an hour with the application in front of the seller.
            while now < Duration::from_mins(30) {
                plan.observed_foreground(true);
                let due = plan.due(at(now));
                if due.sweep {
                    served_sweep(&mut plan, at(now), Discovery::Quiet);
                } else {
                    if due.check_in {
                        plan.checked_in(at(now));
                    }
                    if due.discover {
                        passes += 1;
                        served(&mut plan, at(now), Discovery::Quiet);
                    }
                }
                now += Duration::from_secs(1);
            }
            assert_eq!(
                passes, 179,
                "a phone the seller is using discovers every ten seconds for as long as they use \
                 it: half an hour is a hundred and seventy-nine passes — the first ten seconds \
                 belong to the start-up sweep, which discovers as part of itself — and not a \
                 five-minute window's worth followed by silence"
            );

            // They lock the screen. The next pass is the last.
            plan.observed_foreground(false);
            assert!(
                !plan.due(at(now)).any(),
                "a phone in a pocket asks for nothing at all: no check-in, no discovery and no \
                 sweep, whatever any cadence would otherwise have said"
            );
            let much_later = now + Duration::from_hours(3);
            plan.observed_foreground(false);
            assert!(
                !plan.due(at(much_later)).any(),
                "and three hours of that changes nothing, so no request is ever made from the \
                 background"
            );

            // They pick it up. The resume is a trigger and is served at once
            // rather than waiting on the next observation. Three and a half
            // hours have passed, so the hourly deadline is what serves it —
            // and a sweep is a check-in and a pass together, which is the same
            // two activities the fast cadences would have run.
            plan.triggered();
            let due = plan.due(at(much_later));
            assert!(
                due.any(),
                "a resume is served at once, or a phone the seller came back to would sit idle"
            );
            assert!(
                due.sweep,
                "and the hour having passed while it was away, the pass that serves it is the \
                 full cycle: a check-in — the only channel by which a phone learns it was signed \
                 out — and a discovery together"
            );
        }

        /// A bridge that could not answer stops the polling and still serves
        /// the seller's own arrival.
        ///
        /// The direction is the point: an unreadable lifecycle must not become
        /// a licence to poll from the background, and must not silence the
        /// resume either.
        #[test]
        fn a_phone_that_cannot_say_where_it_is_polls_nothing_and_still_serves_a_resume() {
            let mut plan = Coordination::on_a_phone(Scheduler::DEFAULT_CADENCE);
            // Inside the hour throughout, so what is asserted is the presence
            // gate rather than the hourly deadline arriving.
            plan.swept(at(Duration::ZERO));
            plan.discovered(Discovery::Quiet);
            plan.observed_foreground(false);
            assert!(
                !plan.due(at(Duration::from_mins(40))).any(),
                "no answer means no polling"
            );
            plan.triggered();
            let due = plan.due(at(Duration::from_mins(40)));
            assert!(
                due.check_in,
                "and the seller bringing the application forward is still served"
            );
            assert!(due.discover, "with the pass that picks up their work");
        }

        /// A trigger does not outlive the foreground it was raised in.
        ///
        /// The race is real: a console command or a resume notification can
        /// arrive as the seller is already leaving, and a trigger that
        /// survived it would fire a pass from the background — the one thing
        /// a phone must not do.
        #[test]
        fn a_trigger_raised_as_the_seller_leaves_does_not_fire_from_the_background() {
            let mut plan = Coordination::on_a_phone(Scheduler::DEFAULT_CADENCE);
            plan.observed_foreground(true);
            plan.swept(at(Duration::ZERO));
            plan.discovered(Discovery::Quiet);
            plan.triggered();
            // The loop reads the lifecycle before it reads the plan, and by
            // then the activity has stopped.
            plan.observed_foreground(false);
            assert!(
                !plan.due(at(Duration::from_secs(1))).any(),
                "the pending trigger is dropped with the foreground it belonged to"
            );
            // And a real arrival is still a real arrival.
            plan.observed_foreground(true);
            plan.triggered();
            assert!(
                plan.due(at(Duration::from_secs(2))).any(),
                "while the seller actually being here is served at once"
            );
        }

        /// The fast cadences are measured on a monotonic clock, so a wall
        /// clock correction cannot stop a running device.
        ///
        /// The failure this pins was a real one: measured on the wall clock, a
        /// half-hour step backwards — an NTP correction, a seller fixing their
        /// timezone — left `work_is_due` false and the remaining gap at half an
        /// hour plus the cadence, so a device that was discovering every ten
        /// seconds stopped discovering and stopped checking in until wall time
        /// caught up.
        #[test]
        fn a_wall_clock_correction_does_not_stop_the_fast_cadences() {
            let mut plan = settled();
            let rolled_back = Clocks {
                // Half an hour backwards on the wall, while the process has
                // gone on running for eleven seconds.
                wall: Timestamp(START - 30 * 60 * 1000),
                since_start: DISCOVERY_IDLE + Duration::from_secs(1),
            };
            let due = plan.due(rolled_back);
            assert!(
                due.discover,
                "discovery is due on elapsed time, whatever the wall clock now says"
            );
            assert!(
                !due.sweep,
                "and the hourly deadline is not brought forward by it either: it is stamped on \
                 the wall clock and a backwards jump reads as not due"
            );
            assert_eq!(
                plan.sleep(rolled_back),
                Duration::ZERO,
                "so the loop does not nap through the correction"
            );
            // The sweep's own remainder stays bounded by the cadence rather
            // than by however far the clock moved.
            plan.checked_in(rolled_back);
            served(&mut plan, rolled_back, Discovery::Quiet);
            assert!(
                plan.sleep(rolled_back) <= Scheduler::DEFAULT_CADENCE,
                "and the nap is capped at the cadence, not at the size of the jump"
            );
        }

        /// The import poll's outage survives the work queue's idle answer.
        ///
        /// A discovery pass has two halves which fail and recover for
        /// different reasons: the open-runs read, made inline, and the work
        /// pull, made by the supervised task. Counting both on one ladder let
        /// the healthy half delete the sick one's backoff — the import
        /// endpoint answered `Failed`, the empty queue answered `Quiet` a
        /// moment later and reset the counter, and every cycle went back to
        /// the ten-second gap, which is the failing endpoint being hammered
        /// for as long as it stays down rather than a ladder being climbed.
        #[test]
        fn an_idle_work_result_cannot_erase_import_backoff() {
            let mut plan = settled();
            served_both(
                &mut plan,
                at(Duration::ZERO),
                Discovery::Failed,
                Discovery::Quiet,
            );
            assert!(
                plan.due(at(DISCOVERY_BACKOFF[0])).discover,
                "one import failure is the ladder's first rung"
            );
            let second = DISCOVERY_BACKOFF[0];
            served_both(&mut plan, at(second), Discovery::Failed, Discovery::Quiet);
            assert!(
                !plan.due(at(second + DISCOVERY_BACKOFF[0])).discover,
                "a second import failure is on the second rung: the queue's quiet answer is not \
                 news about the import endpoint and must not send this pass back to ten seconds"
            );
            assert_eq!(
                plan.sleep(at(second + DISCOVERY_BACKOFF[0])),
                Duration::from_secs(10),
                "and the nap is the rest of that rung rather than zero"
            );
            assert!(
                plan.due(at(second + DISCOVERY_BACKOFF[1])).discover,
                "which is twenty seconds after the attempt that failed"
            );
            // The import endpoint comes back. The lane that recovered is the
            // lane whose ladder clears.
            let third = second + DISCOVERY_BACKOFF[1];
            served_both(&mut plan, at(third), Discovery::Quiet, Discovery::Quiet);
            assert!(
                plan.due(at(third + DISCOVERY_IDLE)).discover,
                "a recovered import half returns the pass to the idle gap"
            );
        }

        /// The same property from the other side, which is a different
        /// failure: the work queue's outage must survive the import poll's
        /// quiet answer, and that answer arrives first — the import half is
        /// inline and the work half is supervised, so a cycle always records
        /// the imports before the work.
        #[test]
        fn an_idle_import_result_cannot_erase_work_backoff() {
            let mut plan = settled();
            served_both(
                &mut plan,
                at(Duration::ZERO),
                Discovery::Quiet,
                Discovery::Failed,
            );
            let second = DISCOVERY_BACKOFF[0];
            served_both(&mut plan, at(second), Discovery::Quiet, Discovery::Failed);
            assert!(
                !plan.due(at(second + DISCOVERY_BACKOFF[0])).discover,
                "the work queue's second failure is on the second rung however many quiet \
                 open-runs reads landed in between"
            );
            assert!(plan.due(at(second + DISCOVERY_BACKOFF[1])).discover);
            // One pass serves both halves, so its gap is whichever half asks
            // for more: a recovered queue does not pull the pass forward onto
            // a marketplace another device holds.
            let third = second + DISCOVERY_BACKOFF[1];
            served_both(&mut plan, at(third), Discovery::Held, Discovery::Quiet);
            assert!(
                !plan.due(at(third + DISCOVERY_IDLE)).discover,
                "the held half is the slower half, and the pass answers to it"
            );
            assert!(
                plan.due(at(third + DISCOVERY_HELD)).discover,
                "at the held gap, with the work queue's ladder cleared by its own success"
            );
        }

        /// The loop's nap: never past the earliest deadline, and never zero
        /// when nothing is due, which would spin.
        #[test]
        fn the_nap_reaches_the_earliest_deadline_and_no_further() {
            let mut plan = settled();
            served(&mut plan, at(Duration::ZERO), Discovery::Quiet);
            assert_eq!(
                plan.sleep(at(Duration::ZERO)),
                DISCOVERY_IDLE,
                "discovery is the earliest of the three"
            );
            plan.observed_revoked(true);
            assert_eq!(
                plan.sleep(at(Duration::ZERO)),
                REVOKED_PROBE,
                "and while revoked it is the probe, because nothing else is coming"
            );
        }
    }
}
