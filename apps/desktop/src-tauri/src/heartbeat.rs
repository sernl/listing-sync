//! The control-plane check-in: what this device tells the server about itself,
//! and what it does with the answer.
//!
//! Decision D14 in `docs/notes/design/vendoo-for-teachers-rethink.md` puts a
//! device registry and a per-device sign-out on the server. This module is the
//! device's half of it. Two rules shape everything below.
//!
//! Only metadata leaves. The report carries the device's own identity, the
//! host facts, and one line per marketplace saying whether a session is held —
//! never a cookie, never a jar, never anything a marketplace would accept as
//! authentication. [`SessionReport`] is structurally unable to carry one, for
//! the same reason [`crate::session::SessionStatus`] is.
//!
//! The server decides and the device complies. A heartbeat answers whether
//! this device has been signed out; if it has, the device forgets its stored
//! sessions through the [`SessionStore`] it already owns, closes its gate, and
//! records the state so the interface can say what happened. That is the whole
//! of the wipe, and its limit is stated rather than engineered away: a device
//! that never reaches the server never learns it was revoked, and keeps its
//! marketplace cookies until the marketplace expires them. Nothing here can do
//! better, because the sessions are on this machine and never on the server.
//!
//! [`ControlPlane`] is the seam, exactly as [`crate::scheduler::WorkSource`] is
//! for work. [`Offline`] reaches nothing and is what a build with no configured
//! transport gets; [`crate::control_plane::HttpControlPlane`] is the wire
//! implementation, added on 2026-09-03 with the founder-disclosed `reqwest`
//! edge.
//!
//! The answer also carries the entitlement. The server mints a token per
//! check-in under D10, this module verifies it against the key compiled into
//! the binary and installs the gate, and an answer with no token closes that
//! gate rather than leaving the last one standing — which is what keeps a
//! lapsed subscription to one revalidation window instead of one grace window.

use core::future::Future;
use core::pin::Pin;

use tam_types::{Marketplace, Timestamp, TransportClass};

use crate::device::{DeviceId, DeviceIdentity};
use crate::entitlement::{Entitlement, EntitlementGate, PUBLIC_KEY_BYTES};
use crate::notify::CycleSummary;
use crate::scheduler::{Readiness, Scheduler, TickReport, WorkSource};
use crate::session::{SessionStore, StoreError};
use crate::state::DesktopState;

/// The version this build reports. Read from the manifest rather than written
/// down, so a release cannot report the previous version's number.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What this machine is, as the registry records it. The operating system is
/// `std::env::consts::OS`, which is the vocabulary `tauri-plugin-os` reports
/// and the one the console's device page matches a browser's user agent
/// against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostFacts {
    pub os: &'static str,
    pub arch: &'static str,
    pub app_version: &'static str,
}

impl HostFacts {
    /// This build, running on this machine.
    #[must_use]
    pub const fn here() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            app_version: APP_VERSION,
        }
    }
}

/// What a device says about one marketplace session. The three values the
/// server's closed vocabulary accepts, and every one of them is something this
/// device can tell without making a marketplace request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Connected,
    SignedOut,
    Wiped,
}

impl SessionState {
    /// The token the server's `device_marketplace_session.status` column
    /// stores. One word across the device, the wire and the console.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::SignedOut => "signed_out",
            Self::Wiped => "wiped",
        }
    }
}

/// One line of the report.
///
/// No jar, and no field a jar could travel in. The label is what the
/// marketplace already showed the seller, which is why it is the only string
/// here that came from a marketplace at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReport {
    pub marketplace: Marketplace,
    pub account_label: Option<String>,
    pub status: SessionState,
}

/// What a heartbeat answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckIn {
    /// The seller signed this device out from the console. Everything stored
    /// here is to be forgotten.
    pub revoked: bool,
    /// The entitlement token this check-in was granted, unverified: it is a
    /// string off the wire until [`check_in`] has checked it against the key
    /// this build carries.
    ///
    /// Absent means the server minted none — a lapsed plan, a halt, a revoked
    /// device, or a deployment with no signing key — and the device treats
    /// that as no entitlement at all. It is deliberately not an error: a
    /// heartbeat's first job is delivering revocation, and a device that
    /// refused the whole answer for want of a token would stop learning it had
    /// been signed out.
    pub entitlement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlPlaneError {
    /// This build has no way to reach the server. The one error [`Offline`]
    /// produces, and a distinct fact from a request that failed.
    NotConfigured,
    /// The server refused or could not be reached.
    Refused(String),
    /// The device is not in the registry, so a heartbeat has nothing to stamp.
    /// The caller re-registers rather than retrying.
    Unregistered,
    /// Nobody is signed in to the console on this device, so there is no
    /// session to speak under and no request was made. Not a fault and not a
    /// revocation: it is the ordinary state of a machine at a sign-in screen,
    /// and it resolves itself when the seller signs in.
    NoSession,
}

impl core::fmt::Display for ControlPlaneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotConfigured => f.write_str("this build has no control-plane transport"),
            Self::Refused(why) => write!(f, "the control plane refused: {why}"),
            Self::Unregistered => f.write_str("this device is not registered"),
            Self::NoSession => f.write_str("nobody is signed in to the console on this device"),
        }
    }
}

impl core::error::Error for ControlPlaneError {}

/// One control-plane call, boxed so the trait stays object-safe. The same
/// shape [`crate::scheduler::PullFuture`] and `tam-api`'s `JwksSource` use.
pub type PlaneFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ControlPlaneError>> + Send + 'a>>;

/// The server's device registry, as this device reaches it.
///
/// Two calls, matching `POST /v1/devices` and
/// `POST /v1/devices/{device}/heartbeat`. Nothing here takes a jar, which is
/// what makes it impossible for this seam to carry one.
pub trait ControlPlane: Send + Sync {
    fn register<'a>(&'a self, device: &'a DeviceIdentity, facts: HostFacts) -> PlaneFuture<'a, ()>;

    fn heartbeat<'a>(
        &'a self,
        device: &'a DeviceId,
        sessions: &'a [SessionReport],
    ) -> PlaneFuture<'a, CheckIn>;

    /// Whether the control plane answers at all, asked without a session.
    ///
    /// The console is served from that origin, so a window navigated there
    /// while it is unreachable shows the browser's own failure page — a seller
    /// reading "this site can't be reached" about software they just installed.
    /// Probing first is what lets the application say the true thing instead.
    /// `/healthz` because it is the one route that needs no session and no
    /// version segment.
    fn reachable(&self) -> PlaneFuture<'_, ()>;

    /// Which inventory a sync request names as its source.
    ///
    /// Read from the request rather than taken as an argument, and that is the
    /// point rather than a convenience: the console asks this device to start
    /// an import by request id alone, so a console that named the inventory
    /// could ask a device to enumerate a shop the request does not name. The
    /// server answers under the organisation's own session, so another
    /// tenant's request is a 404 here and the command reports it as a refusal.
    fn sync_request_source(
        &self,
        request: tam_types::Uuid,
    ) -> PlaneFuture<'_, tam_types::InventoryId>;
}

/// The control plane a build with no configured transport gets: one that
/// reaches nothing.
///
/// Deliberately an error rather than a silent success. A device that believed
/// it had checked in would never learn it had been revoked, which is the exact
/// failure the registry exists to prevent.
#[derive(Debug, Default)]
pub struct Offline;

impl ControlPlane for Offline {
    fn reachable(&self) -> PlaneFuture<'_, ()> {
        Box::pin(core::future::ready(Err(ControlPlaneError::NotConfigured)))
    }

    fn sync_request_source(
        &self,
        _request: tam_types::Uuid,
    ) -> PlaneFuture<'_, tam_types::InventoryId> {
        Box::pin(core::future::ready(Err(ControlPlaneError::NotConfigured)))
    }

    fn register<'a>(
        &'a self,
        _device: &'a DeviceIdentity,
        _facts: HostFacts,
    ) -> PlaneFuture<'a, ()> {
        Box::pin(async { Err(ControlPlaneError::NotConfigured) })
    }

    fn heartbeat<'a>(
        &'a self,
        _device: &'a DeviceId,
        _sessions: &'a [SessionReport],
    ) -> PlaneFuture<'a, CheckIn> {
        Box::pin(async { Err(ControlPlaneError::NotConfigured) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckInError {
    Plane(ControlPlaneError),
    Store(StoreError),
}

impl core::fmt::Display for CheckInError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Plane(why) => write!(f, "{why}"),
            Self::Store(why) => write!(f, "{why}"),
        }
    }
}

impl core::error::Error for CheckInError {}

impl From<ControlPlaneError> for CheckInError {
    fn from(why: ControlPlaneError) -> Self {
        Self::Plane(why)
    }
}

impl From<StoreError> for CheckInError {
    fn from(why: StoreError) -> Self {
        Self::Store(why)
    }
}

/// What this device holds, as the report the server takes.
///
/// The whole set rather than a delta: the server replaces what it has with
/// what this returns, so a marketplace missing here is taken as no longer
/// held. Only seller-device marketplaces are walked — a marketplace with an
/// official API never has a session on this machine, and the server refuses a
/// report naming one.
pub async fn report_of(store: &dyn SessionStore) -> Result<Vec<SessionReport>, StoreError> {
    let mut held = Vec::new();
    for marketplace in Marketplace::ALL {
        if marketplace.transport_class() != TransportClass::SellerDevice {
            continue;
        }
        let Some(record) = store.get(marketplace).await? else {
            continue;
        };
        held.push(SessionReport {
            marketplace,
            account_label: record.account_label.clone(),
            status: SessionState::Connected,
        });
    }
    Ok(held)
}

/// One check-in: report what this device holds, and act on the answer.
///
/// A revoked answer wipes before it returns, so a caller that ignores the
/// return value has still had the sessions removed. The gate is closed in the
/// same step, which is what stops the scheduler pulling work with a session it
/// is about to forget.
pub async fn check_in(
    state: &DesktopState,
    plane: &dyn ControlPlane,
) -> Result<CheckIn, CheckInError> {
    let sessions = report_of(state.store()).await?;
    let answer = match plane.heartbeat(&state.device().id, &sessions).await {
        Ok(answer) => answer,
        Err(why) => {
            // Not being signed in is a fact about this device that the
            // interface shows, so it is recorded even though the call failed.
            // Nothing else is: an unreachable server tells us nothing about
            // our standing, and least of all that we are still signed in.
            if why == ControlPlaneError::NoSession {
                state.set_signed_in(false);
            }
            return Err(why.into());
        }
    };
    if answer.revoked {
        // Revocation wins over any token in the same answer. `wipe` closes the
        // gate, and installing an entitlement after it would hand a signed-out
        // device permission the seller has just withdrawn.
        wipe(state).await?;
    } else {
        state
            .set_gate(gate_for(
                &state.device().id,
                state.verifying_keys(),
                &answer,
            ))
            .await;
    }
    state.set_signed_in(true);
    state.set_revoked(answer.revoked);
    Ok(answer)
}

/// The gate an answer installs.
///
/// A token that verifies opens the gate for exactly the marketplaces it names.
/// Everything else closes it: a token this build has no key for, one minted for
/// another machine, a malformed one, and — the case that matters most — an
/// answer carrying none at all. A withheld token has to replace the one being
/// held rather than leave it standing, because leaving it would run the grace
/// window from the last good token and turn a lapsed plan into twenty-five
/// hours of further work instead of one.
///
/// A rejected token and an absent one are not told apart, and the seller sees
/// the same thing for both: the next tick refuses every marketplace with
/// [`crate::state::BlockReason::NotEntitled`]. That is not lossy — both mean
/// this device may not work — and the device cannot tell a key rotation from a
/// forgery in any case.
fn gate_for(
    device: &DeviceId,
    verifying_keys: &[[u8; PUBLIC_KEY_BYTES]],
    answer: &CheckIn,
) -> EntitlementGate {
    answer
        .entitlement
        .as_deref()
        .and_then(|token| Entitlement::verify(token, verifying_keys, device).ok())
        .map_or_else(EntitlementGate::closed, EntitlementGate::holding)
}

/// Registers this device and immediately checks in, which is what first run
/// does. Registration is idempotent on the server, so a restart re-registers
/// rather than needing to remember whether it had.
pub async fn first_run(
    state: &DesktopState,
    plane: &dyn ControlPlane,
) -> Result<CheckIn, CheckInError> {
    if let Err(why) = plane.register(state.device(), HostFacts::here()).await {
        if why == ControlPlaneError::NoSession {
            state.set_signed_in(false);
        }
        return Err(why.into());
    }
    check_in(state, plane).await
}

/// One check-in, registering this device first if the server does not know it.
///
/// The registry refuses to create a device from a heartbeat, so an id the
/// server has never seen answers [`ControlPlaneError::Unregistered`] and every
/// later check-in answers the same until something registers. Nothing did:
/// [`first_run`] is reached only from the `device_check_in` command, so a
/// device whose console never called it never appeared in the seller's list at
/// all. This is the repair, and it is on the scheduled path rather than in the
/// console because the console is the half that can be absent.
///
/// Only [`ControlPlaneError::Unregistered`], and only from the check-in. A
/// refusal is an outage and re-registering through one would read a bad
/// gateway as a lost registration; the same variant off the ledger and payload
/// transports means a missing job or a missing payload and never reaches here.
///
/// At most one registration and one retry per cycle, so a server that answers
/// not-found to both costs two requests a tick rather than a loop. Registering
/// cannot restore a revoked device: `revoked_at` is its own column and the
/// upsert does not touch it, and a revoked device is a row that exists, so it
/// answers a heartbeat rather than a not-found and never reaches this arm.
///
/// Two call sites, both on the scheduled path and both in this module:
/// [`cycle`], and [`resume`]'s branch for a phone brought forward before its
/// work is due. Neither is the console's, which reaches [`first_run`] instead.
async fn check_in_or_register(
    state: &DesktopState,
    plane: &dyn ControlPlane,
) -> Result<CheckIn, CheckInError> {
    match check_in(state, plane).await {
        Err(CheckInError::Plane(ControlPlaneError::Unregistered)) => {
            plane
                .register(state.device(), HostFacts::here())
                .await
                .map_err(CheckInError::from)?;
            check_in(state, plane).await
        }
        other => other,
    }
}

/// One scheduled cycle: check in, then pull whatever work the gate still
/// allows.
///
/// The check-in comes first because it is what learns of a revocation, and a
/// cycle that pulled first would spend a round of work under sessions it was
/// about to forget. A check-in that could not reach the server does not stop
/// the cycle — an offline period is not a revocation, and D11's grace window
/// exists precisely so a device keeps working through one — while a revoked
/// one stops it without a second decision, because the gate it just closed
/// refuses every marketplace.
///
/// One notification per cycle that settled anything, and never one per item:
/// the summary is taken over the whole report once it is recorded, so a
/// fifty-item run raises one. A notification the platform would not show is
/// logged and nothing more, because by then the work is done and recorded.
pub async fn cycle<W: WorkSource + ?Sized>(
    state: &DesktopState,
    plane: &dyn ControlPlane,
    scheduler: &Scheduler,
    source: &W,
    now: Timestamp,
) -> TickReport {
    check_in_or_register(state, plane).await.ok();
    let gate = state.gate().await;
    let report = scheduler
        .tick(
            &Readiness {
                revoked: state.revoked(),
                signed_in: state.signed_in(),
                gate: &gate,
                sessions: state.store(),
            },
            source,
            now,
        )
        .await;
    // Stamped with the tick's instant rather than each event's own: this is
    // the interface's record of what the device did, and the ledger on the
    // server is the record of what happened to an item.
    for (marketplace, event) in &report.events {
        state.record(*marketplace, now, event.clone()).await;
    }
    if let Some(summary) = CycleSummary::of(&report) {
        if let Err(why) = state.notifier().notify(&summary).await {
            eprintln!("the cycle's notification could not be raised: {why}");
        }
    }
    report
}

/// One resume's worth of a phone: always a check-in, and a scheduler tick only
/// when the last one was longer ago than the cadence.
///
/// A phone has no timer — Android's Doze stops `JobScheduler` and the
/// battery-optimisation exemption that would evade it is barred by Play
/// policy — so a resume is the only moment it can act, and until this split
/// every resume ran a full [`cycle`]. A phone brought forward twenty times an
/// hour therefore posted twenty work claims and made twenty rounds of
/// marketplace requests, which is the D3 deviation
/// `docs/notes/design/android-client.md` describes rather than the behaviour
/// it describes.
///
/// The check-in is not gated. It is the only channel by which a phone learns
/// the seller signed it out, and gating it would make a revocation wait for
/// the hour rather than for the next time the seller looks.
///
/// Answers the tick's report where it ticked and `None` where it only checked
/// in, so the caller stamps its instant on the branch that actually worked
/// rather than on every resume — which would hold the gate closed forever.
#[cfg(any(mobile, test))]
pub(crate) async fn resume<W: WorkSource + ?Sized>(
    state: &DesktopState,
    plane: &dyn ControlPlane,
    scheduler: &Scheduler,
    source: &W,
    at: ResumeAt,
) -> Option<TickReport> {
    if crate::scheduler::work_is_due(at.last_tick, at.now, scheduler.cadence()) {
        return Some(cycle(state, plane, scheduler, source, at.now).await);
    }
    check_in_or_register(state, plane).await.ok();
    None
}

/// When the last scheduler tick ran, and when this resume is.
///
/// A struct rather than two arguments of the same type, which are one careless
/// swap from a phone that works on every resume or never works at all.
#[cfg(any(mobile, test))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResumeAt {
    /// `None` before anything has ticked on this run, which is due by
    /// definition: nothing else sets the stamp.
    ///
    /// No caller reaches that branch today. The mobile loop in `lib.rs` seeds
    /// the stamp with `Some(wall_now())` before its first resume, so `None`
    /// arrives only from a test; it is kept because the seeding is the loop's
    /// choice rather than this type's, and a caller that did not seed would
    /// otherwise have no way to say it had never ticked.
    pub(crate) last_tick: Option<Timestamp>,
    pub(crate) now: Timestamp,
}

/// Forgets every marketplace session on this device and closes the gate.
///
/// Every marketplace rather than only the seller-device ones: this is the
/// removal, and skipping a key because of what we believe about its transport
/// class would leave a stored session behind on the one path whose job is
/// leaving none.
async fn wipe(state: &DesktopState) -> Result<(), StoreError> {
    for marketplace in Marketplace::ALL {
        state.store().forget(marketplace).await?;
    }
    state.set_gate(EntitlementGate::closed()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        check_in, first_run, report_of, CheckIn, ControlPlane, ControlPlaneError, HostFacts,
        Offline, PlaneFuture, SessionReport, SessionState,
    };
    use crate::device::{DeviceId, DeviceIdentity};
    use crate::entitlement::testing::{at, claims, mint, test_key, TestKey, GRACE, NOW, VALIDITY};
    use crate::entitlement::EntitlementGate;
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::DesktopState;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tam_types::{Marketplace, Timestamp};

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::from_raw("11112222333344445555666677778888"),
            label: "founder-pc".to_owned(),
        }
    }

    fn a_record(marketplace: Marketplace, label: Option<&str>) -> SessionRecord {
        SessionRecord {
            marketplace,
            account_label: label.map(str::to_owned),
            captured_at: Timestamp(1_756_000_000_000),
            device_id: identity().id,
            jar: CookieJar::new(vec![Cookie {
                name: "sessionKey".to_owned(),
                value: "s3cr3t".to_owned(),
            }]),
        }
    }

    fn state_with(store: Arc<MemorySessionStore>) -> DesktopState {
        DesktopState::new(identity(), store)
    }

    /// A control plane that answers whatever it was built with, and keeps the
    /// last report so a test can assert what actually left the device.
    struct Fake {
        revoked: bool,
        entitlement: Option<String>,
        registrations: AtomicUsize,
        beats: AtomicUsize,
        last: tokio::sync::Mutex<Vec<SessionReport>>,
    }

    impl Fake {
        fn new(revoked: bool) -> Self {
            Self {
                revoked,
                entitlement: None,
                registrations: AtomicUsize::new(0),
                beats: AtomicUsize::new(0),
                last: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        /// The same plane, answering with a token as well.
        fn granting(token: String) -> Self {
            Self {
                entitlement: Some(token),
                ..Self::new(false)
            }
        }

        async fn reported(&self) -> Vec<SessionReport> {
            self.last.lock().await.clone()
        }
    }

    impl ControlPlane for Fake {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn register<'a>(
            &'a self,
            _device: &'a DeviceIdentity,
            _facts: HostFacts,
        ) -> PlaneFuture<'a, ()> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            sessions: &'a [SessionReport],
        ) -> PlaneFuture<'a, CheckIn> {
            self.beats.fetch_add(1, Ordering::SeqCst);
            let revoked = self.revoked;
            let entitlement = self.entitlement.clone();
            Box::pin(async move {
                *self.last.lock().await = sessions.to_vec();
                Ok(CheckIn {
                    revoked,
                    entitlement,
                })
            })
        }
    }

    #[tokio::test]
    async fn the_report_names_every_held_seller_device_marketplace_and_nothing_else() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tpt, Some("Founder's Classroom")))
            .await
            .expect("tpt stores");
        store
            .put(&a_record(Marketplace::Etsy, None))
            .await
            .expect("etsy stores");

        let report = report_of(store.as_ref()).await.expect("the report builds");
        assert_eq!(
            report
                .iter()
                .map(|line| line.marketplace)
                .collect::<Vec<_>>(),
            vec![Marketplace::Tpt],
            "Etsy's automation runs server-side, so no line for it may leave this device"
        );
        assert_eq!(
            report[0].account_label.as_deref(),
            Some("Founder's Classroom")
        );
        assert_eq!(report[0].status, SessionState::Connected);
    }

    #[tokio::test]
    async fn no_report_can_carry_a_cookie() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tes, None))
            .await
            .expect("tes stores");
        let printed = format!("{:?}", report_of(store.as_ref()).await.expect("it builds"));
        assert!(
            !printed.contains("s3cr3t") && !printed.contains("sessionKey"),
            "the report type must be structurally unable to carry a credential: {printed}"
        );
    }

    #[tokio::test]
    async fn a_check_in_that_is_not_revoked_leaves_the_sessions_alone() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tpt, None))
            .await
            .expect("tpt stores");
        let state = state_with(Arc::clone(&store));
        let plane = Fake::new(false);

        let answer = check_in(&state, &plane).await.expect("the check-in lands");
        assert!(!answer.revoked);
        assert!(!state.revoked(), "nothing revoked this device");
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_some(),
            "a routine check-in must not touch what the device holds"
        );
        assert_eq!(
            plane.reported().await.len(),
            1,
            "the whole set is reported, so the server can replace rather than merge"
        );
    }

    #[tokio::test]
    async fn a_revoked_answer_forgets_every_session_and_closes_the_gate() {
        let store = Arc::new(MemorySessionStore::new());
        for marketplace in Marketplace::ALL {
            store
                .put(&a_record(marketplace, None))
                .await
                .expect("the session stores");
        }
        let state = state_with(Arc::clone(&store));
        let plane = Fake::new(true);

        let answer = check_in(&state, &plane).await.expect("the check-in lands");
        assert!(answer.revoked);
        assert!(
            state.revoked(),
            "the interface has to be able to say why syncing stopped"
        );
        for marketplace in Marketplace::ALL {
            assert_eq!(
                store.get(marketplace).await.expect("the store reads"),
                None,
                "{marketplace:?} must be forgotten, including one whose transport class \
                 means it should never have been stored"
            );
        }
        for marketplace in Marketplace::ALL {
            assert!(
                !state
                    .gate()
                    .await
                    .may_work(marketplace, Timestamp(1_756_000_000_000)),
                "a revoked device works nothing"
            );
        }
    }

    #[tokio::test]
    async fn a_revoked_check_in_stops_the_cycle_before_any_work_is_pulled() {
        use crate::scheduler::{PullFuture, Scheduler, WorkSource};
        use core::time::Duration;

        #[derive(Debug, Default)]
        struct CountingSource(AtomicUsize);

        impl WorkSource for CountingSource {
            fn pull(&self, _marketplace: Marketplace) -> PullFuture<'_> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(vec![crate::state::WorkEvent::Idle]) })
            }
        }

        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tpt, None))
            .await
            .expect("tpt stores");
        let state = state_with(Arc::clone(&store));
        let source = CountingSource::default();
        let scheduler = Scheduler::new(
            Duration::from_mins(1),
            vec![Marketplace::Tpt, Marketplace::Tes],
        );

        let report = super::cycle(
            &state,
            &Fake::new(true),
            &scheduler,
            &source,
            Timestamp(1_756_000_000_000),
        )
        .await;

        assert_eq!(
            report.blocked(),
            vec![
                (Marketplace::Tpt, crate::state::BlockReason::Revoked),
                (Marketplace::Tes, crate::state::BlockReason::Revoked),
            ],
            "a revoked device works nothing, and the seller is told which marketplaces and why"
        );
        assert_eq!(
            state.activity().await.len(),
            2,
            "and the console can read back what this device did, naming the device that did it"
        );
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            0,
            "the check-in must close the gate before the work source is reached, not after"
        );
        assert_eq!(
            store.get(Marketplace::Tpt).await.expect("the store reads"),
            None,
            "and the session it would have worked under is gone"
        );
    }

    #[tokio::test]
    async fn first_run_registers_before_it_checks_in() {
        let store = Arc::new(MemorySessionStore::new());
        let state = state_with(store);
        let plane = Fake::new(false);
        first_run(&state, &plane).await.expect("first run lands");
        assert_eq!(plane.registrations.load(Ordering::SeqCst), 1);
        assert_eq!(plane.beats.load(Ordering::SeqCst), 1);
    }

    /// A registry with no row for this device until something registers one,
    /// which is what every device in this repository has faced since the
    /// registry landed: `POST /v1/devices` had one caller and nothing called
    /// it.
    struct Registry {
        registered: core::sync::atomic::AtomicBool,
        registrations: AtomicUsize,
        beats: AtomicUsize,
    }

    impl Registry {
        fn empty() -> Self {
            Self {
                registered: core::sync::atomic::AtomicBool::new(false),
                registrations: AtomicUsize::new(0),
                beats: AtomicUsize::new(0),
            }
        }

        fn holding_this_device() -> Self {
            let registry = Self::empty();
            registry.registered.store(true, Ordering::SeqCst);
            registry
        }
    }

    impl ControlPlane for Registry {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn register<'a>(
            &'a self,
            _device: &'a DeviceIdentity,
            _facts: HostFacts,
        ) -> PlaneFuture<'a, ()> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            self.registered.store(true, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            _sessions: &'a [SessionReport],
        ) -> PlaneFuture<'a, CheckIn> {
            self.beats.fetch_add(1, Ordering::SeqCst);
            let known = self.registered.load(Ordering::SeqCst);
            Box::pin(async move {
                if known {
                    Ok(CheckIn {
                        revoked: false,
                        entitlement: None,
                    })
                } else {
                    Err(ControlPlaneError::Unregistered)
                }
            })
        }
    }

    /// A server that is there and unhappy: every call refused, which is what an
    /// outage or a bad gateway looks like from here.
    struct Unreachable {
        registrations: AtomicUsize,
    }

    impl ControlPlane for Unreachable {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Err(ControlPlaneError::Refused(
                "502".to_owned(),
            ))))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Err(ControlPlaneError::Refused(
                "502".to_owned(),
            ))))
        }

        fn register<'a>(
            &'a self,
            _device: &'a DeviceIdentity,
            _facts: HostFacts,
        ) -> PlaneFuture<'a, ()> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            _sessions: &'a [SessionReport],
        ) -> PlaneFuture<'a, CheckIn> {
            Box::pin(core::future::ready(Err(ControlPlaneError::Refused(
                "502".to_owned(),
            ))))
        }
    }

    /// The scheduler and work source a cycle needs, with nothing to pull.
    fn idle_cycle_parts() -> (crate::scheduler::Scheduler, crate::scheduler::NoWork) {
        (
            crate::scheduler::Scheduler::new(
                core::time::Duration::from_mins(1),
                vec![Marketplace::Tpt],
            ),
            crate::scheduler::NoWork,
        )
    }

    #[tokio::test]
    async fn a_cycle_registers_the_device_the_server_does_not_know_and_checks_in_again() {
        let state = state_with(Arc::new(MemorySessionStore::new()));
        let plane = Registry::empty();
        let (scheduler, source) = idle_cycle_parts();

        super::cycle(
            &state,
            &plane,
            &scheduler,
            &source,
            Timestamp(1_756_000_000_000),
        )
        .await;

        assert_eq!(
            plane.registrations.load(Ordering::SeqCst),
            1,
            "an unknown device registers itself, which is the only way a machine \
             ever reaches the seller's list"
        );
        assert_eq!(
            plane.beats.load(Ordering::SeqCst),
            2,
            "one check-in that learned it was unknown, and one that landed after \
             registering; a registration with no second check-in would leave the \
             device with no entitlement until the next tick"
        );
        assert!(
            state.signed_in(),
            "the retried check-in is what the state records, not the refusal that preceded it"
        );
    }

    #[tokio::test]
    async fn a_cycle_against_a_registry_that_knows_this_device_registers_nothing() {
        let state = state_with(Arc::new(MemorySessionStore::new()));
        let plane = Registry::holding_this_device();
        let (scheduler, source) = idle_cycle_parts();

        super::cycle(
            &state,
            &plane,
            &scheduler,
            &source,
            Timestamp(1_756_000_000_000),
        )
        .await;

        assert_eq!(
            plane.registrations.load(Ordering::SeqCst),
            0,
            "the ordinary tick costs one request, not two"
        );
        assert_eq!(plane.beats.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_refused_check_in_is_an_outage_and_never_a_lost_registration() {
        let state = state_with(Arc::new(MemorySessionStore::new()));
        let plane = Unreachable {
            registrations: AtomicUsize::new(0),
        };
        let (scheduler, source) = idle_cycle_parts();

        super::cycle(
            &state,
            &plane,
            &scheduler,
            &source,
            Timestamp(1_756_000_000_000),
        )
        .await;

        assert_eq!(
            plane.registrations.load(Ordering::SeqCst),
            0,
            "a device that re-registered through every outage would rewrite its own \
             registry row on a schedule for no reason"
        );
    }

    #[tokio::test]
    async fn a_revoked_device_is_a_row_that_exists_so_the_self_heal_never_sees_it() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tpt, None))
            .await
            .expect("tpt stores");
        let state = state_with(Arc::clone(&store));
        let plane = Fake::new(true);
        let (scheduler, source) = idle_cycle_parts();

        super::cycle(
            &state,
            &plane,
            &scheduler,
            &source,
            Timestamp(1_756_000_000_000),
        )
        .await;

        assert_eq!(
            plane.registrations.load(Ordering::SeqCst),
            0,
            "a re-registration here would hand a signed-out device a fresh row and \
             undo the sign-out the seller performed"
        );
        assert!(state.revoked());
        assert_eq!(
            store.get(Marketplace::Tpt).await.expect("the store reads"),
            None,
            "and the wipe still ran"
        );
    }

    #[tokio::test]
    async fn a_build_with_no_transport_says_so_rather_than_reporting_success() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tpt, None))
            .await
            .expect("tpt stores");
        let state = state_with(Arc::clone(&store));

        let refused = check_in(&state, &Offline)
            .await
            .expect_err("there is no wire implementation in this slice");
        assert_eq!(
            refused.to_string(),
            ControlPlaneError::NotConfigured.to_string()
        );
        assert!(
            !state.revoked(),
            "a check-in that never happened is not evidence of revocation"
        );
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_some(),
            "and it must not wipe on a failure to reach the server, which would make \
             every offline period a disconnect"
        );
    }

    /// A phone brought forward again a minute later checks in and works
    /// nothing.
    ///
    /// Both halves are asserted against the same run, because either alone
    /// passes for an implementation that is wrong in the other direction: a
    /// resume that pulled nothing AND checked in nothing would be a phone that
    /// never learns it was signed out, and a resume that did both would be the
    /// deviation this split closes. The first resume in the pair is what makes
    /// the second severe — it proves the gate, the session and the entitlement
    /// are all open, so the second resume's zero is the cadence refusing rather
    /// than a readiness check that would have refused anyway.
    ///
    /// What this does not cover is the mobile loop itself, which is
    /// `#[cfg(mobile)]` and unreachable from a host test. It holds the
    /// `Option<Timestamp>` this takes as an argument and stamps it on the
    /// `Some` branch; that wiring is three lines and is proved by the Android
    /// target's own compile, not by this.
    #[tokio::test]
    async fn a_resume_before_its_work_is_due_checks_in_and_pulls_nothing() {
        use crate::scheduler::{PullFuture, Scheduler, WorkSource};

        #[derive(Debug, Default)]
        struct CountingSource(AtomicUsize);

        impl WorkSource for CountingSource {
            fn pull(&self, _marketplace: Marketplace) -> PullFuture<'_> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(vec![crate::state::WorkEvent::Idle]) })
            }
        }

        let key = test_key();
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tpt, None))
            .await
            .expect("tpt stores");
        let state = DesktopState::with_verifying_keys(identity(), store, vec![key.public]);
        let plane = Fake::granting(mint(&key, &claims(NOW, vec![Marketplace::Tpt])));
        let scheduler = Scheduler::new(Scheduler::DEFAULT_CADENCE, vec![Marketplace::Tpt]);
        let source = CountingSource::default();

        let started = super::resume(
            &state,
            &plane,
            &scheduler,
            &source,
            super::ResumeAt {
                last_tick: None,
                now: at(NOW),
            },
        )
        .await;

        assert!(
            started.is_some(),
            "a phone with no previous tick works on the first resume, or it never works at all"
        );
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            1,
            "and the work source really was reached, which is what makes the count below mean \
             the cadence rather than a closed gate"
        );
        assert_eq!(plane.beats.load(Ordering::SeqCst), 1);

        let again = super::resume(
            &state,
            &plane,
            &scheduler,
            &source,
            super::ResumeAt {
                last_tick: Some(at(NOW)),
                now: at(NOW + 60),
            },
        )
        .await;

        assert!(
            again.is_none(),
            "a resume a minute later is not a tick, and answering Some would stamp an instant \
             that never worked"
        );
        assert_eq!(
            source.0.load(Ordering::SeqCst),
            1,
            "a phone brought forward twenty times an hour claims work once, not twenty times"
        );
        assert_eq!(
            plane.beats.load(Ordering::SeqCst),
            2,
            "but it checks in every time, because that is the only channel by which it learns \
             the seller signed it out"
        );
    }

    // ------------------------------------------------------------ D10's token

    /// A state that verifies against `key`, and a plane granting a token this
    /// device, signed with it. Driven through the real `check_in` rather than
    /// at the verifier's own seam, because the wiring is what these assert.
    fn granted(key: &TestKey, marketplaces: Vec<Marketplace>) -> (DesktopState, Fake) {
        let granted = claims(NOW, marketplaces);
        let state = DesktopState::with_verifying_keys(
            identity(),
            Arc::new(MemorySessionStore::new()),
            vec![key.public],
        );
        (state, Fake::granting(mint(key, &granted)))
    }

    fn verifying_against(key: &TestKey) -> DesktopState {
        DesktopState::with_verifying_keys(
            identity(),
            Arc::new(MemorySessionStore::new()),
            vec![key.public],
        )
    }

    #[tokio::test]
    async fn a_verified_token_opens_the_gate_for_exactly_the_marketplaces_it_names() {
        let key = test_key();
        let (state, plane) = granted(&key, vec![Marketplace::Tpt]);

        check_in(&state, &plane).await.expect("the check-in lands");

        let gate = state.gate().await;
        assert!(gate.may_work(Marketplace::Tpt, at(NOW)));
        assert!(
            !gate.may_work(Marketplace::Tes, at(NOW)),
            "a marketplace the token does not name stays refused, which is how one \
             marketplace is revoked across the installed fleet without an update"
        );
    }

    #[tokio::test]
    async fn the_installed_gate_carries_d11s_grace_window() {
        let key = test_key();
        let (state, plane) = granted(&key, vec![Marketplace::Tpt]);

        check_in(&state, &plane).await.expect("the check-in lands");

        let gate = state.gate().await;
        assert!(
            gate.may_work(Marketplace::Tpt, at(NOW + VALIDITY + GRACE - 1)),
            "inside the grace the seller keeps working while our server is unreachable"
        );
        assert!(
            !gate.may_work(Marketplace::Tpt, at(NOW + VALIDITY + GRACE + 1)),
            "past it the gate fails closed, which is the kill-switch latency the founder \
             commits to publicly"
        );
    }

    #[tokio::test]
    async fn a_token_minted_for_another_machine_leaves_the_gate_closed() {
        let key = test_key();
        let mut elsewhere = claims(NOW, vec![Marketplace::Tpt]);
        elsewhere.device = "ffffffffffffffffffffffffffffffff".to_owned();
        let state = verifying_against(&key);

        check_in(&state, &Fake::granting(mint(&key, &elsewhere)))
            .await
            .expect("the check-in lands");

        assert_eq!(
            state.gate().await,
            EntitlementGate::closed(),
            "a token copied to a second machine must not work there"
        );
    }

    #[tokio::test]
    async fn a_token_this_build_has_no_key_for_leaves_the_gate_closed() {
        let (signer, verifier) = (test_key(), test_key());
        let state = verifying_against(&verifier);
        let token = mint(&signer, &claims(NOW, vec![Marketplace::Tpt]));

        check_in(&state, &Fake::granting(token))
            .await
            .expect("the check-in lands");

        assert_eq!(
            state.gate().await,
            EntitlementGate::closed(),
            "a token signed by a key this build does not carry is refused, not read"
        );
    }

    #[tokio::test]
    async fn an_answer_with_no_token_closes_a_gate_that_was_open() {
        let key = test_key();
        let (state, granting) = granted(&key, vec![Marketplace::Tpt]);
        check_in(&state, &granting)
            .await
            .expect("the first check-in lands");
        assert!(state.gate().await.may_work(Marketplace::Tpt, at(NOW)));

        check_in(&state, &Fake::new(false))
            .await
            .expect("the second check-in lands");

        assert_eq!(
            state.gate().await,
            EntitlementGate::closed(),
            "a withheld token must replace the one being held: leaving it standing would run \
             the grace window from the last good token and turn a lapsed plan into \
             twenty-five further hours of work rather than one"
        );
    }

    #[tokio::test]
    async fn a_check_in_that_never_reached_the_server_leaves_an_open_gate_open() {
        let key = test_key();
        let (state, granting) = granted(&key, vec![Marketplace::Tpt]);
        check_in(&state, &granting)
            .await
            .expect("the first check-in lands");

        check_in(&state, &Offline)
            .await
            .expect_err("there is no transport to reach");

        assert!(
            state.gate().await.may_work(Marketplace::Tpt, at(NOW)),
            "an offline period is not a lapse, and D11's grace window exists precisely so a \
             seller keeps working through one"
        );
    }

    #[tokio::test]
    async fn a_revoked_answer_closes_the_gate_even_when_it_carries_a_valid_token() {
        let key = test_key();
        let state = verifying_against(&key);
        let token = mint(&key, &claims(NOW, vec![Marketplace::Tpt]));

        check_in(
            &state,
            &Fake {
                revoked: true,
                ..Fake::granting(token)
            },
        )
        .await
        .expect("the check-in lands");

        assert_eq!(
            state.gate().await,
            EntitlementGate::closed(),
            "the sign-out the seller performed wins over any grant riding in the same answer"
        );
    }

    // ------------------------------------------------- the cycle's notification

    /// A source that settles two items a pull, one well and one badly, so the
    /// notice has two counts to name.
    struct SettlingSource;

    impl crate::scheduler::WorkSource for SettlingSource {
        fn pull(&self, _marketplace: Marketplace) -> crate::scheduler::PullFuture<'_> {
            use crate::state::WorkEvent;
            use tam_domain::ItemOutcome;
            Box::pin(async {
                Ok(vec![
                    WorkEvent::Started {
                        item: "one".to_owned(),
                    },
                    WorkEvent::Settled {
                        item: "one".to_owned(),
                        outcome: ItemOutcome::Succeeded,
                    },
                    WorkEvent::Started {
                        item: "two".to_owned(),
                    },
                    WorkEvent::Settled {
                        item: "two".to_owned(),
                        outcome: ItemOutcome::Failed,
                    },
                ])
            })
        }
    }

    /// A state whose gate the plane will open for TPT, holding a TPT session,
    /// with a recorder where the plugin would be.
    async fn notifying_state(
        key: &TestKey,
    ) -> (
        DesktopState,
        Fake,
        Arc<crate::notify::testing::RecordingNotifier>,
    ) {
        let (state, plane) = granted(key, vec![Marketplace::Tpt]);
        let recorder = Arc::new(crate::notify::testing::RecordingNotifier::new());
        let state = state.with_notifier(recorder.clone());
        state
            .store()
            .put(&a_record(Marketplace::Tpt, None))
            .await
            .expect("tpt stores");
        (state, plane, recorder)
    }

    #[tokio::test]
    async fn a_cycle_that_settled_anything_raises_one_notification_naming_the_counts() {
        use crate::scheduler::Scheduler;
        let key = test_key();
        let (state, plane, recorder) = notifying_state(&key).await;
        let scheduler = Scheduler::new(Scheduler::DEFAULT_CADENCE, vec![Marketplace::Tpt]);

        let report = super::cycle(&state, &plane, &scheduler, &SettlingSource, at(NOW)).await;

        assert_eq!(
            report
                .events
                .iter()
                .filter(|(_, event)| matches!(event, crate::state::WorkEvent::Settled { .. }))
                .count(),
            2,
            "the gate, the session and the entitlement are all open, so two items settled; \
             without this the single notice below would prove nothing"
        );
        let notices = recorder.notices().await;
        assert_eq!(
            notices.len(),
            1,
            "two items settled and one notification was raised: per cycle, never per item"
        );
        assert_eq!(
            notices[0].title, "Your TPT sync finished — 1 of 2 resources updated",
            "the title names the marketplace and both figures, because one item failed"
        );
        assert_eq!(
            notices[0].body, "1 succeeded, 1 failed",
            "the body names each count under the console's own word for the outcome"
        );
    }

    #[tokio::test]
    async fn a_cycle_that_settled_nothing_raises_none() {
        use crate::scheduler::Scheduler;
        let key = test_key();
        let (state, plane, recorder) = notifying_state(&key).await;
        let scheduler = Scheduler::new(Scheduler::DEFAULT_CADENCE, vec![Marketplace::Tpt]);

        super::cycle(
            &state,
            &plane,
            &scheduler,
            &crate::scheduler::NoWork,
            at(NOW),
        )
        .await;

        assert!(
            state.signed_in() && state.gate().await.may_work(Marketplace::Tpt, at(NOW)),
            "the cycle reached the work source and found nothing due, rather than being \
             refused before it, which is what makes the silence below the summary's"
        );
        assert_eq!(
            recorder.notices().await,
            vec![],
            "a cycle with nothing due says nothing"
        );

        super::cycle(
            &state,
            &Fake::new(true),
            &scheduler,
            &SettlingSource,
            at(NOW),
        )
        .await;

        assert_eq!(
            recorder.notices().await,
            vec![],
            "a revoked device works nothing and therefore announces nothing"
        );
    }
}
