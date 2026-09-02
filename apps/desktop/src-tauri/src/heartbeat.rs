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
//! There is no wire implementation in this slice. [`ControlPlane`] is the seam,
//! exactly as [`crate::scheduler::WorkSource`] is for work, and its only
//! implementation is [`Offline`], which reaches nothing. The crate has no HTTP
//! client and adding one is a founder-gated dependency decision.

use core::future::Future;
use core::pin::Pin;

use tam_types::{Marketplace, Timestamp, TransportClass};

use crate::device::{DeviceId, DeviceIdentity};
use crate::entitlement::EntitlementGate;
use crate::scheduler::{Scheduler, TickReport, WorkSource};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckIn {
    /// The seller signed this device out from the console. Everything stored
    /// here is to be forgotten.
    pub revoked: bool,
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
}

/// The only implementation in this slice: a control plane that reaches
/// nothing.
///
/// Deliberately an error rather than a silent success. A device that believed
/// it had checked in would never learn it had been revoked, which is the exact
/// failure the registry exists to prevent.
#[derive(Debug, Default)]
pub struct Offline;

impl ControlPlane for Offline {
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
        wipe(state).await?;
    }
    state.set_signed_in(true);
    state.set_revoked(answer.revoked);
    Ok(answer)
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
pub async fn cycle<W: WorkSource + ?Sized>(
    state: &DesktopState,
    plane: &dyn ControlPlane,
    scheduler: &Scheduler,
    source: &W,
    now: Timestamp,
) -> TickReport {
    check_in(state, plane).await.ok();
    scheduler.tick(&state.gate().await, source, now).await
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
        registrations: AtomicUsize,
        beats: AtomicUsize,
        last: tokio::sync::Mutex<Vec<SessionReport>>,
    }

    impl Fake {
        fn new(revoked: bool) -> Self {
            Self {
                revoked,
                registrations: AtomicUsize::new(0),
                beats: AtomicUsize::new(0),
                last: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        async fn reported(&self) -> Vec<SessionReport> {
            self.last.lock().await.clone()
        }
    }

    impl ControlPlane for Fake {
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
            Box::pin(async move {
                *self.last.lock().await = sessions.to_vec();
                Ok(CheckIn { revoked })
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
                Box::pin(async { Ok(1) })
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
            report.blocked,
            vec![Marketplace::Tpt, Marketplace::Tes],
            "a revoked device works nothing, and the seller is told which marketplaces"
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
}
