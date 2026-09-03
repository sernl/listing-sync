//! What the interpreter needs from this device, besides the ledger.
//!
//! `tam-engine-driver` takes no runtime, no clock and no randomness by
//! design: `just purity` bans them from that crate so the boundary is proved
//! by compilation. Everything it refuses to read for itself enters through
//! these four capabilities, and this is the process boundary that reads them.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;
use std::sync::Arc;

use tam_engine_driver::driver::NowSource;
use tam_engine_driver::ports::{Cancellation, IdSource};
use tam_engine_driver::vocabulary::Renewed;
use tam_marketplace::Pause;
use tam_types::{Timestamp, Uuid};

/// The seller's own wall clock, in milliseconds since the epoch.
///
/// A clock before the epoch saturates to zero, which reads as "at the epoch"
/// rather than as something stranger.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "the desktop client is a clock-reading process boundary; D1 moves the timer to the \
              seller's machine, so this is the clock it moves to"
)]
pub fn wall_now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(i64::try_from(millis).unwrap_or(0))
}

/// The driver's clock seam, bound to this machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceClock;

impl NowSource for DeviceClock {
    fn now(&self) -> Timestamp {
        wall_now()
    }
}

/// Where a fresh attempt id comes from on this device.
///
/// The driver mints the attempt id and passes it to `open_attempt` so a lost
/// response is recoverable by re-offering the same id, rather than by spending
/// another of the item's attempts. That property is why this is a capability
/// at all.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceIds;

impl IdSource for DeviceIds {
    fn new_id(&self) -> Uuid {
        Uuid(*uuid::Uuid::new_v4().as_bytes())
    }
}

/// The wait between verification reads: a real sleep, because this is the
/// running client rather than a cassette replay.
#[derive(Debug, Clone, Copy, Default)]
pub struct SleepingPause;

impl Pause for SleepingPause {
    fn pause(&self, ms: u32) -> impl core::future::Future<Output = ()> + Send {
        tokio::time::sleep(Duration::from_millis(u64::from(ms)))
    }
}

/// Why a run was stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// The lease the server issued has run out on this device's own reckoning.
    LeaseExpired,
    /// The entitlement gate closed, or the seller signed this device out,
    /// while the run was in flight.
    Revoked,
}

/// The stop condition for one run of one item.
///
/// Two facts, and neither of them compares two clocks. The server states its
/// deadline as `server_deadline_ms` beside its own `server_now_ms`, which is a
/// duration it vouches for; the device converts that duration into a local
/// monotonic instant the moment the envelope arrives, so a device whose wall
/// clock is wrong still stops in step with the lease. The second fact is the
/// entitlement's: a device revoked or lapsed mid-run stops before its next
/// marketplace request rather than after it.
///
/// Both arrive at the interpreter through the one seam it has for stopping,
/// and that is deliberate. `run_item` consults this at the top of every loop
/// iteration, and the transition table emits at most one network-bearing
/// effect per batch, so a check there is exact rather than conservative. A
/// stop produces the shape `BudgetGrant::Exhausted` already produces — the
/// open attempt settled abandoned and the run `Abandoned` — never a new
/// terminal outcome, so a revocation mid-run cannot settle an item on evidence
/// the run does not have.
///
/// The instant is `tokio::time::Instant` rather than the standard library's
/// for a reason beyond testability: it is the clock this crate's own timers
/// run on, so the deadline and the sleep between verification tries cannot
/// disagree, and a test advances both together.
#[derive(Debug, Clone)]
pub struct RunGate {
    /// The instant this gate was created, which every stored offset is
    /// measured from.
    base: tokio::time::Instant,
    /// How many milliseconds after `base` the lease ends.
    ///
    /// An integer rather than an `Instant` behind a lock because
    /// [`Cancellation::is_cancelled`] is synchronous and must not await: a
    /// renew arriving on another task moves this with one atomic store, and
    /// every check reads it without blocking.
    expires_after_ms: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
}

/// A handle that moves a run's deadline when the server extends its lease.
///
/// Held by the ledger client, which is where a renew's answer arrives. It
/// carries no clock of its own: it is given the server's two instants and
/// moves the deadline by their difference, which is the same arithmetic
/// [`RunGate::from_envelope`] does on the claim.
#[derive(Clone)]
pub struct LeaseDeadline {
    base: tokio::time::Instant,
    expires_after_ms: Arc<AtomicU64>,
}

impl LeaseDeadline {
    /// Moves the deadline out to what the server now says the lease runs to.
    ///
    /// Never shortens it. A renew answering a nearer deadline than the one
    /// standing would be the server offering less time than we already had,
    /// which a heartbeat cannot do, and `fetch_max` makes a late answer
    /// harmless rather than a run cut short by its own bookkeeping.
    pub fn extend(&self, renewed: Renewed) {
        let remaining = renewed
            .server_deadline_ms
            .saturating_sub(renewed.server_now_ms)
            .max(0);
        let elapsed = u64::try_from(
            tokio::time::Instant::now()
                .saturating_duration_since(self.base)
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        let moved = elapsed.saturating_add(u64::try_from(remaining).unwrap_or(0));
        self.expires_after_ms.fetch_max(moved, Ordering::SeqCst);
    }
}

impl RunGate {
    /// A gate that expires `lease_ms` from now.
    ///
    /// A non-positive duration expires immediately: an envelope whose deadline
    /// has already passed by the server's own reckoning is one to abandon
    /// rather than to run, and the stall bias makes the lease the server's to
    /// reissue.
    #[must_use]
    pub fn lasting(lease_ms: i64) -> Self {
        Self {
            base: tokio::time::Instant::now(),
            expires_after_ms: Arc::new(AtomicU64::new(u64::try_from(lease_ms).unwrap_or(0))),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The handle a ledger client holds to move this deadline on a renew.
    #[must_use]
    pub fn deadline(&self) -> LeaseDeadline {
        LeaseDeadline {
            base: self.base,
            expires_after_ms: Arc::clone(&self.expires_after_ms),
        }
    }

    /// When the lease ends, as this gate currently understands it.
    fn expires_at(&self) -> tokio::time::Instant {
        self.base + Duration::from_millis(self.expires_after_ms.load(Ordering::SeqCst))
    }

    /// The gate the envelope describes: the server's deadline minus the
    /// server's own reading of now, both stated in the same message.
    #[must_use]
    pub fn from_envelope(server_now_ms: i64, server_deadline_ms: i64) -> Self {
        Self::lasting(server_deadline_ms.saturating_sub(server_now_ms))
    }

    /// Watches a flag shared with the rest of the application instead of this
    /// gate's own, so a check-in that learned of a revocation stops every run
    /// in flight rather than only the next one to start.
    #[must_use]
    pub fn stopped_by(mut self, stopped: Arc<AtomicBool>) -> Self {
        self.stopped = stopped;
        self
    }

    /// Stops the run at its next check. Idempotent, and cannot be undone: a
    /// run told to stop does not resume.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// A handle that stops this run from elsewhere — the check-in that learns
    /// of a revocation, or the seller closing the application.
    #[must_use]
    pub fn stopper(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stopped)
    }

    /// Why the run stopped, or `None` while it may continue.
    #[must_use]
    pub fn cause(&self) -> Option<StopCause> {
        if self.stopped.load(Ordering::SeqCst) {
            Some(StopCause::Revoked)
        } else if tokio::time::Instant::now() >= self.expires_at() {
            Some(StopCause::LeaseExpired)
        } else {
            None
        }
    }

    /// How much of the lease is left. Zero once it has run out.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.expires_at()
            .saturating_duration_since(tokio::time::Instant::now())
    }
}

impl Cancellation for RunGate {
    fn is_cancelled(&self) -> bool {
        self.cause().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{wall_now, DeviceClock, DeviceIds, Renewed, RunGate, SleepingPause, StopCause};
    use core::time::Duration;
    use tam_engine_driver::driver::NowSource;
    use tam_engine_driver::ports::{Cancellation, IdSource};
    use tam_marketplace::Pause;

    #[tokio::test(start_paused = true)]
    async fn the_gate_expires_on_the_duration_the_server_vouched_for() {
        // The server's own two instants, five minutes apart, as the claim
        // endpoint computes them. The absolute values are deliberately far
        // from this machine's clock: nothing here may compare the two.
        let gate = RunGate::from_envelope(1_000_000_000_000, 1_000_000_300_000);
        assert_eq!(gate.cause(), None, "a fresh lease has not expired");
        assert_eq!(gate.remaining(), Duration::from_mins(5));

        tokio::time::advance(Duration::from_secs(299)).await;
        assert!(
            !gate.is_cancelled(),
            "the run continues for the whole of the lease the server issued"
        );

        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            gate.cause(),
            Some(StopCause::LeaseExpired),
            "past the deadline the device stops and lets the lease expire, which is the stall \
             bias: the server's reaper requeues with the epoch bumped"
        );
        assert_eq!(gate.remaining(), Duration::ZERO);
    }

    /// A renewed run outlives the deadline the claim gave it.
    ///
    /// This is the heartbeat's whole purpose: before it, a run doing slow work
    /// lost its item to a reaper that could not tell slow from gone, and the
    /// device could do nothing about it. The extension is the server's numbers
    /// and nothing of ours — the deadline moves by `server_deadline_ms` minus
    /// `server_now_ms`, both stated in the same answer.
    #[tokio::test(start_paused = true)]
    async fn a_renewed_run_outlives_the_deadline_the_claim_gave_it() {
        let gate = RunGate::from_envelope(1_000_000_000_000, 1_000_000_060_000);
        let deadline = gate.deadline();
        assert_eq!(gate.remaining(), Duration::from_mins(1));

        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(!gate.is_cancelled(), "still inside the original lease");
        // The server answers a fresh five minutes from an instant of its own,
        // deliberately unrelated to this machine's clock.
        deadline.extend(Renewed {
            server_now_ms: 2_000_000_000_000,
            server_deadline_ms: 2_000_000_300_000,
        });

        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(
            !gate.is_cancelled(),
            "past the original deadline the run continues, because the server extended the \
             lease and said by how much"
        );
        // The renew arrived 59s into the run and bought 300s from that
        // moment, so the deadline is 359s from the start. Sampling here, past
        // the 300s a renewal that dropped the elapsed term would have set,
        // is what separates the two: without the elapsed term the run is
        // already over by now, and it is not.
        tokio::time::advance(Duration::from_secs(259)).await;
        assert!(
            !gate.is_cancelled(),
            "the renewal buys its 300s from the moment it was answered, not from the moment \
             the run began: a deadline that dropped the elapsed time already run would cut \
             this run short by the 59s it had spent"
        );

        tokio::time::advance(Duration::from_secs(40)).await;
        assert_eq!(
            gate.cause(),
            Some(StopCause::LeaseExpired),
            "and it stops at the renewed deadline, which is still the server's rather than \
             a lease the device granted itself"
        );
    }

    /// A renew that answered a nearer deadline never shortens the run.
    #[tokio::test(start_paused = true)]
    async fn a_late_or_smaller_renewal_cannot_cut_a_run_short() {
        let gate = RunGate::from_envelope(0, 300_000);
        let deadline = gate.deadline();
        deadline.extend(Renewed {
            server_now_ms: 0,
            server_deadline_ms: 1_000,
        });
        tokio::time::advance(Duration::from_mins(1)).await;
        assert!(
            !gate.is_cancelled(),
            "an answer offering less time than the run already had is not something a \
             heartbeat can do, so it leaves the standing deadline alone"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_envelope_whose_deadline_has_already_passed_runs_nothing() {
        let gate = RunGate::from_envelope(1_000_000_300_000, 1_000_000_000_000);
        assert_eq!(
            gate.cause(),
            Some(StopCause::LeaseExpired),
            "a deadline behind the server's own now is not a lease to start work under"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_revocation_stops_the_run_before_the_deadline_does() {
        let gate = RunGate::from_envelope(0, 300_000);
        let stopper = gate.stopper();
        assert!(!gate.is_cancelled());

        stopper.store(true, core::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            gate.cause(),
            Some(StopCause::Revoked),
            "an entitlement that closed mid-run stops the interpreter at its next loop top, \
             before the next marketplace request rather than after it"
        );

        gate.stop();
        assert_eq!(
            gate.cause(),
            Some(StopCause::Revoked),
            "stopping is idempotent and cannot be undone"
        );
    }

    #[test]
    fn a_fresh_attempt_id_is_fresh() {
        let ids = DeviceIds;
        let (first, second) = (ids.new_id(), ids.new_id());
        assert_ne!(
            first, second,
            "the driver mints the attempt id so a lost response is recoverable by re-offering \
             the same one; two runs must not collide"
        );
        assert_ne!(first.0, [0u8; 16], "a zero id is not a generated one");
    }

    #[test]
    fn the_clock_reads_this_machine() {
        // 2020-01-01, comfortably before any run of this build.
        const AFTER: i64 = 1_577_836_800_000;
        let seam = DeviceClock.now().0;
        assert!(
            seam > AFTER,
            "the driver's clock seam reads the seller's own wall clock"
        );
        assert!(
            (seam - wall_now().0).abs() < 1_000,
            "and reads the same one the rest of the client does"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_pause_is_a_real_wait() {
        let before = tokio::time::Instant::now();
        SleepingPause.pause(2_000).await;
        assert!(
            tokio::time::Instant::now().duration_since(before) >= Duration::from_secs(2),
            "the verification poll waits for the marketplace to settle; an instant return here \
             would spend the budget without giving it time to"
        );
    }
}
