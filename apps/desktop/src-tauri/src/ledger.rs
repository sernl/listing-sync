//! The ledger, as the device reaches it.
//!
//! `ItemLedger` is the client-reachable half of the split, and every method is
//! keyed on a `LeaseRef` so the server derives the organisation, the
//! connection, the inventory and the epoch from the lease it issued rather
//! than trusting an argument this device supplied. This module is the wire
//! form of that trait and nothing more: it adds no operation, widens no scope,
//! and answers nothing locally.
//!
//! Answering locally is the point worth stating. Six of the twelve methods
//! return a value the interpreter branches on, and each of those six is a
//! decision the server must keep. `open_attempt`'s `AttemptInFlight` is the
//! duplicate-create fence. `request_grant` is the rate ceiling, and a governed
//! party that answered it would be setting its own. `settle_attempt`'s
//! disposition is the binding decision. `preflight_failed`'s streak decides a
//! give-up. `connection_for` decides whether the run may proceed at all. So
//! every one of them is a round trip, and none is cached.

use tam_domain::SellerEvent;
use tam_engine_driver::ports::ItemLedger;
use tam_engine_driver::vocabulary::{
    AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, GrantKind, ItemVerdict, LeaseRef,
    LedgerAnswer, LedgerCall, LedgerError, NewAttempt, PreflightStreak, Renewed, SettleEnvelope,
};
use tam_types::{ConnectionId, InventoryId, JobEventPayload, Timestamp};

use crate::device::DeviceId;
use crate::heartbeat::PlaneFuture;
use crate::run::LeaseDeadline;

/// Where a ledger call goes. One path for the whole trait rather than twelve,
/// because the call is a tagged value and a path per variant would be the same
/// union spelled twice.
#[must_use]
pub fn ledger_path(device: &DeviceId) -> String {
    format!("/v1/devices/{device}/ledger")
}

/// Where the terminal settle goes. Separate because it already exists, and
/// because it is the one call fenced on the holder as well as the epoch.
#[must_use]
pub fn settle_path(device: &DeviceId) -> String {
    format!("/v1/devices/{device}/settle")
}

/// One authenticated POST to one of our own control-plane paths.
///
/// A trait rather than a direct call so the ledger protocol is testable
/// without a socket, and so the one implementation that opens one keeps the
/// resolve-the-session-then-send order in the single place that states why
/// that order matters.
pub trait LedgerTransport: Send + Sync {
    fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String>;
}

/// The ledger over HTTP.
pub struct HttpLedger<T: LedgerTransport> {
    device: DeviceId,
    transport: T,
    /// What the run last parked on.
    ///
    /// Kept because this is the only place on the device where it exists: the
    /// interpreter reports a park as a bare verdict, and what the seller has
    /// to act on is the gate it parked on. Read once after the run, so the
    /// console can say why rather than only that.
    parked_on: tokio::sync::Mutex<Option<String>>,
    /// Where a successful renew moves the run's deadline.
    ///
    /// Absent in the tests that exercise the protocol alone. A run that has
    /// one gets its lease extended by the server's own numbers; a run that has
    /// none still renews, and still stops on a refusal, because the fence is
    /// the server's answer rather than this handle.
    deadline: Option<LeaseDeadline>,
}

impl<T: LedgerTransport> core::fmt::Debug for HttpLedger<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpLedger")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

/// Anything the transport, the encoder or the decoder refused.
///
/// Opaque on purpose: `LedgerError::Refused` is the interpreter's only answer
/// to a condition it cannot act on, and the stall bias does the rest.
fn refused(why: &dyn core::fmt::Display) -> LedgerError {
    LedgerError::Refused {
        detail: why.to_string(),
    }
}

impl<T: LedgerTransport> HttpLedger<T> {
    #[must_use]
    pub fn new(device: DeviceId, transport: T) -> Self {
        Self {
            device,
            transport,
            parked_on: tokio::sync::Mutex::new(None),
            deadline: None,
        }
    }

    /// Binds this ledger to the run whose deadline a renew should move.
    #[must_use]
    pub fn moving(mut self, deadline: LeaseDeadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// What the run parked on, if it parked.
    pub async fn parked_on(&self) -> Option<String> {
        self.parked_on.lock().await.clone()
    }

    async fn call(&self, call: &LedgerCall) -> Result<LedgerAnswer, LedgerError> {
        let body = serde_json::to_string(call).map_err(|why| refused(&why))?;
        let reply = self
            .transport
            .post(&ledger_path(&self.device), body)
            .await
            .map_err(|why| refused(&why))?;
        let answer: LedgerAnswer = serde_json::from_str(&reply).map_err(|why| refused(&why))?;
        match answer {
            LedgerAnswer::Refused { error } => Err(error),
            landed @ (LedgerAnswer::Done
            | LedgerAnswer::Connection { .. }
            | LedgerAnswer::Streak { .. }
            | LedgerAnswer::Bound { .. }
            | LedgerAnswer::Grant { .. }
            | LedgerAnswer::Renewed { .. }) => Ok(landed),
        }
    }

    /// A call whose only successful answer is that it landed.
    async fn write(&self, call: &LedgerCall) -> Result<(), LedgerError> {
        self.call(call).await?.landed(call_name(call))
    }
}

/// The name of a call, for a refusal that must not quote the call itself: a
/// `NewAttempt` carries the whole projected body, and an error message is not
/// where that belongs.
#[must_use]
pub const fn call_name(call: &LedgerCall) -> &'static str {
    match *call {
        LedgerCall::ConnectionFor { .. } => "connection_for",
        LedgerCall::PreflightSucceeded { .. } => "preflight_succeeded",
        LedgerCall::PreflightFailed { .. } => "preflight_failed",
        LedgerCall::Park { .. } => "park",
        LedgerCall::OpenAttempt { .. } => "open_attempt",
        LedgerCall::SettleAttempt { .. } => "settle_attempt",
        LedgerCall::GateConnection { .. } => "gate_connection",
        LedgerCall::HaltThisTenant { .. } => "halt_this_tenant",
        LedgerCall::RequestGrant { .. } => "request_grant",
        LedgerCall::Renew { .. } => "renew",
        LedgerCall::RecordEvent { .. } => "record_event",
        LedgerCall::Notify { .. } => "notify",
    }
}

/// An answer of the wrong shape for the call.
///
/// A refusal rather than a panic: the device does not get to assume the server
/// it is talking to is the one it was built against, and the interpreter's
/// answer to a condition it cannot act on is the stall bias.
fn unexpected(call: &'static str) -> LedgerError {
    LedgerError::Refused {
        detail: format!("the ledger answered a shape that is not {call}'s"),
    }
}

/// Reading one answer as the shape its call requires.
///
/// One exhaustive match per shape, written once rather than at each of the
/// twelve call sites, so a variant added to [`LedgerAnswer`] fails to compile
/// in six places rather than being absorbed by a wildcard.
trait ReadAnswer {
    fn landed(self, call: &'static str) -> Result<(), LedgerError>;
    fn connection(self, call: &'static str) -> Result<Option<ConnectionId>, LedgerError>;
    fn streak(self, call: &'static str) -> Result<PreflightStreak, LedgerError>;
    fn disposition(self, call: &'static str) -> Result<BindDisposition, LedgerError>;
    fn grant(self, call: &'static str) -> Result<BudgetGrant, LedgerError>;
    fn renewed(self, call: &'static str) -> Result<Renewed, LedgerError>;
}

impl ReadAnswer for LedgerAnswer {
    fn landed(self, call: &'static str) -> Result<(), LedgerError> {
        match self {
            Self::Done => Ok(()),
            Self::Connection { .. }
            | Self::Streak { .. }
            | Self::Bound { .. }
            | Self::Grant { .. }
            | Self::Renewed { .. }
            | Self::Refused { .. } => Err(unexpected(call)),
        }
    }

    fn connection(self, call: &'static str) -> Result<Option<ConnectionId>, LedgerError> {
        match self {
            Self::Connection { connection } => Ok(connection),
            Self::Done
            | Self::Streak { .. }
            | Self::Bound { .. }
            | Self::Grant { .. }
            | Self::Renewed { .. }
            | Self::Refused { .. } => Err(unexpected(call)),
        }
    }

    fn streak(self, call: &'static str) -> Result<PreflightStreak, LedgerError> {
        match self {
            Self::Streak { streak } => Ok(streak),
            Self::Done
            | Self::Connection { .. }
            | Self::Bound { .. }
            | Self::Grant { .. }
            | Self::Renewed { .. }
            | Self::Refused { .. } => Err(unexpected(call)),
        }
    }

    fn disposition(self, call: &'static str) -> Result<BindDisposition, LedgerError> {
        match self {
            Self::Bound { disposition } => Ok(disposition),
            Self::Done
            | Self::Connection { .. }
            | Self::Streak { .. }
            | Self::Grant { .. }
            | Self::Renewed { .. }
            | Self::Refused { .. } => Err(unexpected(call)),
        }
    }

    fn renewed(self, call: &'static str) -> Result<Renewed, LedgerError> {
        match self {
            Self::Renewed { renewed } => Ok(renewed),
            Self::Done
            | Self::Connection { .. }
            | Self::Streak { .. }
            | Self::Bound { .. }
            | Self::Grant { .. }
            | Self::Refused { .. } => Err(unexpected(call)),
        }
    }

    fn grant(self, call: &'static str) -> Result<BudgetGrant, LedgerError> {
        match self {
            Self::Grant { grant } => Ok(grant),
            Self::Done
            | Self::Connection { .. }
            | Self::Streak { .. }
            | Self::Bound { .. }
            | Self::Renewed { .. }
            | Self::Refused { .. } => Err(unexpected(call)),
        }
    }
}

impl<T: LedgerTransport> ItemLedger for HttpLedger<T> {
    async fn connection_for(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
    ) -> Result<Option<ConnectionId>, LedgerError> {
        let call = LedgerCall::ConnectionFor {
            lease: *lease,
            inventory,
        };
        self.call(&call).await?.connection(call_name(&call))
    }

    async fn preflight_succeeded(&self, lease: &LeaseRef) -> Result<(), LedgerError> {
        self.write(&LedgerCall::PreflightSucceeded { lease: *lease })
            .await
    }

    async fn preflight_failed(
        &self,
        lease: &LeaseRef,
        edge_class: bool,
    ) -> Result<PreflightStreak, LedgerError> {
        let call = LedgerCall::PreflightFailed {
            lease: *lease,
            edge_class,
        };
        self.call(&call).await?.streak(call_name(&call))
    }

    async fn park(&self, lease: &LeaseRef, blocked_on: &str) -> Result<(), LedgerError> {
        *self.parked_on.lock().await = Some(blocked_on.to_owned());
        self.write(&LedgerCall::Park {
            lease: *lease,
            blocked_on: blocked_on.to_owned(),
        })
        .await
    }

    /// The terminal call, and the only one that does not go to the ledger
    /// path.
    ///
    /// `/settle` already exists and is fenced on the epoch and the holder's
    /// device id. Its envelope carries the device's asserted instant; the
    /// server records the terminal event and releases the lease in one
    /// transaction, so the driver must not follow settlement with a second
    /// event request under a lease that no longer exists.
    async fn settle_item(
        &self,
        lease: &LeaseRef,
        verdict: &ItemVerdict,
        at: Timestamp,
    ) -> Result<(), LedgerError> {
        let body = serde_json::to_string(&SettleEnvelope {
            lease: *lease,
            verdict: verdict.clone(),
            at_ms: at.0,
        })
        .map_err(|why| refused(&why))?;
        self.transport
            .post(&settle_path(&self.device), body)
            .await
            .map(|_| ())
            .map_err(|why| refused(&why))
    }

    async fn open_attempt(
        &self,
        lease: &LeaseRef,
        new: &NewAttempt,
        at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.write(&LedgerCall::OpenAttempt {
            lease: *lease,
            new: new.clone(),
            at_ms: at.0,
        })
        .await
    }

    async fn settle_attempt(
        &self,
        lease: &LeaseRef,
        attempt: AttemptRef,
        verdict: &AttemptVerdict,
        at: Timestamp,
    ) -> Result<BindDisposition, LedgerError> {
        let call = LedgerCall::SettleAttempt {
            lease: *lease,
            attempt,
            verdict: verdict.clone(),
            at_ms: at.0,
        };
        self.call(&call).await?.disposition(call_name(&call))
    }

    async fn gate_connection(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
        at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.write(&LedgerCall::GateConnection {
            lease: *lease,
            inventory,
            at_ms: at.0,
        })
        .await
    }

    async fn halt_this_tenant(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
        reason: &str,
        at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.write(&LedgerCall::HaltThisTenant {
            lease: *lease,
            inventory,
            reason: reason.to_owned(),
            at_ms: at.0,
        })
        .await
    }

    async fn request_grant(
        &self,
        lease: &LeaseRef,
        connection: ConnectionId,
        kind: GrantKind,
        at: Timestamp,
    ) -> Result<BudgetGrant, LedgerError> {
        let call = LedgerCall::RequestGrant {
            lease: *lease,
            connection,
            kind,
            at_ms: at.0,
        };
        self.call(&call).await?.grant(call_name(&call))
    }

    /// The heartbeat.
    ///
    /// On success the run's deadline moves to what the server now says the
    /// lease runs to — its numbers, not ours. On a refusal the answer is a
    /// `StaleLease`, which the interpreter turns into an abandoned run at the
    /// next loop top: a lease we no longer hold is one we must not write
    /// under, and the heartbeat sits before every request precisely so that is
    /// discovered before one goes out.
    async fn renew(&self, lease: &LeaseRef) -> Result<Renewed, LedgerError> {
        let call = LedgerCall::Renew { lease: *lease };
        let renewed = self.call(&call).await?.renewed(call_name(&call))?;
        if let Some(deadline) = self.deadline.as_ref() {
            deadline.extend(renewed);
        }
        Ok(renewed)
    }

    async fn record_event(
        &self,
        lease: &LeaseRef,
        payload: &JobEventPayload,
        at: Timestamp,
    ) -> Result<(), LedgerError> {
        if matches!(payload, JobEventPayload::ItemSettled { .. }) {
            return Ok(());
        }
        self.write(&LedgerCall::RecordEvent {
            lease: *lease,
            payload: payload.clone(),
            at_ms: at.0,
        })
        .await
    }

    async fn notify(
        &self,
        lease: &LeaseRef,
        event: SellerEvent,
        at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.write(&LedgerCall::Notify {
            lease: *lease,
            event,
            at_ms: at.0,
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{ledger_path, settle_path, HttpLedger, LedgerTransport};
    use crate::device::DeviceId;
    use crate::heartbeat::{ControlPlaneError, PlaneFuture};
    use tam_domain::{ItemOutcome, JobItemId, SellerEvent};
    use tam_engine_driver::ports::ItemLedger;
    use tam_engine_driver::vocabulary::{
        AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, GrantKind,
        ItemVerdict, LandingEffect, LeaseRef, LedgerAnswer, LedgerCall, LedgerError, NewAttempt,
        PreflightStreak, SettleEnvelope,
    };
    use tam_types::{
        ConnectionId, InventoryId, JobEventPayload, MappingId, OrgId, Timestamp, Uuid,
    };
    use tokio::sync::Mutex;

    const DEVICE: &str = "11112222333344445555666677778888";
    const AT: Timestamp = Timestamp(1_756_000_000_000);

    fn uuid(last: u8) -> Uuid {
        let mut raw = [0u8; 16];
        raw[15] = last;
        Uuid(raw)
    }

    fn lease() -> LeaseRef {
        LeaseRef {
            org: OrgId(uuid(1)),
            item: JobItemId(uuid(2)),
            lease_epoch: 7,
        }
    }

    /// A ledger endpoint that answers a canned reply and records the path and
    /// the exact JSON it was sent, so the wire shape is assertable without a
    /// socket.
    struct FakeLedger {
        answer: Result<String, ControlPlaneError>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl FakeLedger {
        fn answering(answer: &LedgerAnswer) -> Self {
            Self {
                answer: Ok(serde_json::to_string(answer).expect("the fixture answer encodes")),
                sent: Mutex::new(Vec::new()),
            }
        }

        fn refusing(why: &str) -> Self {
            Self {
                answer: Err(ControlPlaneError::Refused(why.to_owned())),
                sent: Mutex::new(Vec::new()),
            }
        }

        async fn last(&self) -> (String, String) {
            self.sent
                .lock()
                .await
                .last()
                .cloned()
                .expect("the ledger transport was reached")
        }

        async fn last_call(&self) -> LedgerCall {
            let (_, body) = self.last().await;
            serde_json::from_str(&body).expect("what went out is a LedgerCall")
        }
    }

    impl LedgerTransport for FakeLedger {
        fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String> {
            Box::pin(async move {
                self.sent.lock().await.push((path.to_owned(), body));
                self.answer.clone()
            })
        }
    }

    fn ledger(answer: &LedgerAnswer) -> HttpLedger<FakeLedger> {
        HttpLedger::new(DeviceId::from_raw(DEVICE), FakeLedger::answering(answer))
    }

    #[tokio::test]
    async fn every_call_goes_to_the_one_ledger_path_carrying_its_own_lease() {
        let ledger = ledger(&LedgerAnswer::Done);
        ledger
            .preflight_succeeded(&lease())
            .await
            .expect("the write lands");

        let (path, _) = ledger.transport.last().await;
        assert_eq!(
            path,
            ledger_path(&DeviceId::from_raw(DEVICE)),
            "the device is in the path because the server authorises every ledger write against \
             the lease that device holds"
        );
        assert_eq!(
            ledger.transport.last_call().await,
            LedgerCall::PreflightSucceeded { lease: lease() },
            "and the lease travels in the call, so the server derives org, connection, inventory \
             and epoch from what it issued rather than from an argument"
        );
    }

    #[tokio::test]
    async fn a_connection_read_is_a_round_trip_and_its_absence_is_an_answer() {
        let connected = ledger(&LedgerAnswer::Connection {
            connection: Some(ConnectionId(uuid(3))),
        });
        assert_eq!(
            connected
                .connection_for(&lease(), InventoryId::Tpt)
                .await
                .expect("the read lands"),
            Some(ConnectionId(uuid(3)))
        );
        assert_eq!(
            connected.transport.last_call().await,
            LedgerCall::ConnectionFor {
                lease: lease(),
                inventory: InventoryId::Tpt,
            }
        );

        let unlinked = ledger(&LedgerAnswer::Connection { connection: None });
        assert_eq!(
            unlinked
                .connection_for(&lease(), InventoryId::Tpt)
                .await
                .expect("the read lands"),
            None,
            "no linked connection at run time abandons the run; it is not an error"
        );
    }

    #[tokio::test]
    async fn the_rate_ceiling_is_the_servers_answer_and_never_this_devices() {
        let exhausted = ledger(&LedgerAnswer::Grant {
            grant: BudgetGrant::Exhausted,
        });
        assert_eq!(
            exhausted
                .request_grant(&lease(), ConnectionId(uuid(3)), GrantKind::Write, AT)
                .await
                .expect("the grant request lands"),
            BudgetGrant::Exhausted
        );

        let LedgerCall::RequestGrant { kind, .. } = exhausted.transport.last_call().await else {
            panic!("a grant request goes out as one");
        };
        assert_eq!(kind, GrantKind::Write);

        let body = exhausted.transport.last().await.1;
        assert!(
            !body.contains("window") && !body.contains("ceiling"),
            "a governed party that supplied either would be setting its own rate limit: {body}"
        );
    }

    #[tokio::test]
    async fn the_create_fence_is_the_servers_refusal_and_reaches_the_interpreter_intact() {
        let fenced = ledger(&LedgerAnswer::Refused {
            error: LedgerError::AttemptInFlight,
        });
        assert_eq!(
            fenced
                .open_attempt(
                    &lease(),
                    &NewAttempt {
                        attempt: uuid(9),
                        mapping: MappingId(uuid(4)),
                        intent: AttemptIntent {
                            body: serde_json::json!({ "title": "a worksheet" }),
                            hash: vec![1, 2, 3],
                        },
                    },
                    AT,
                )
                .await,
            Err(LedgerError::AttemptInFlight),
            "the standing in-flight attempt is the only fence against a second create, and it is \
             the server's to hold"
        );
    }

    #[tokio::test]
    async fn a_stale_lease_is_carried_back_as_itself() {
        let stolen = ledger(&LedgerAnswer::Refused {
            error: LedgerError::StaleLease,
        });
        assert_eq!(
            stolen.preflight_succeeded(&lease()).await,
            Err(LedgerError::StaleLease),
            "another holder owns this item now, and the epoch fence saying so is not a transport \
             failure"
        );
    }

    #[tokio::test]
    async fn each_branching_answer_is_read_as_its_own_shape() {
        let streak = ledger(&LedgerAnswer::Streak {
            streak: PreflightStreak {
                failures: 2,
                edge_only: true,
            },
        });
        assert_eq!(
            streak
                .preflight_failed(&lease(), true)
                .await
                .expect("the write lands"),
            PreflightStreak {
                failures: 2,
                edge_only: true,
            }
        );

        let bound = ledger(&LedgerAnswer::Bound {
            disposition: BindDisposition::Bound,
        });
        assert_eq!(
            bound
                .settle_attempt(
                    &lease(),
                    AttemptRef {
                        attempt: uuid(9),
                        mapping: MappingId(uuid(4)),
                    },
                    &AttemptVerdict {
                        state: "committed".to_owned(),
                        failure_code: None,
                        landing: LandingEffect::None,
                    },
                    AT,
                )
                .await
                .expect("the settle lands"),
            BindDisposition::Bound
        );
    }

    #[tokio::test]
    async fn an_answer_of_the_wrong_shape_refuses_rather_than_panicking() {
        let wrong = ledger(&LedgerAnswer::Done);
        let why = wrong
            .request_grant(&lease(), ConnectionId(uuid(3)), GrantKind::VerifyRead, AT)
            .await
            .expect_err("a unit answer is not a grant");
        assert_eq!(
            why,
            LedgerError::Refused {
                detail: "the ledger answered a shape that is not request_grant's".to_owned(),
            },
            "the device does not get to assume the server it is talking to is the one it was \
             built against"
        );
    }

    #[tokio::test]
    async fn a_transport_refusal_becomes_a_ledger_refusal_rather_than_a_success() {
        let offline = HttpLedger::new(
            DeviceId::from_raw(DEVICE),
            FakeLedger::refusing("the control plane is unreachable"),
        );
        let why = offline
            .preflight_succeeded(&lease())
            .await
            .expect_err("an unreachable control plane is not a landed write");
        assert!(
            matches!(why, LedgerError::Refused { .. }),
            "the interpreter's only answer to a condition it cannot act on is the stall bias"
        );
    }

    #[tokio::test]
    async fn the_settle_goes_to_the_fenced_settle_path_as_a_settle_envelope() {
        let settling = ledger(&LedgerAnswer::Done);
        let verdict = ItemVerdict {
            outcome: ItemOutcome::Succeeded,
            failure_code: None,
            failure_detail: None,
        };
        settling
            .settle_item(&lease(), &verdict, AT)
            .await
            .expect("the settle lands");

        let (path, body) = settling.transport.last().await;
        assert_eq!(
            path,
            settle_path(&DeviceId::from_raw(DEVICE)),
            "the settle keeps its own path, which is fenced on the holder's device id as well as \
             on the epoch"
        );
        assert_eq!(
            serde_json::from_str::<SettleEnvelope>(&body).expect("a settle envelope goes out"),
            SettleEnvelope {
                lease: lease(),
                verdict,
                at_ms: AT.0,
            }
        );
    }

    #[tokio::test]
    async fn the_notification_carries_the_closed_event_and_never_a_topic() {
        let notifying = ledger(&LedgerAnswer::Done);
        notifying
            .notify(&lease(), SellerEvent::ReauthRequired, AT)
            .await
            .expect("the notification lands");

        assert_eq!(
            notifying.transport.last_call().await,
            LedgerCall::Notify {
                lease: lease(),
                event: SellerEvent::ReauthRequired,
                at_ms: AT.0,
            },
            "the topic is a total function of the event, so no relay name crosses this boundary"
        );
    }

    #[tokio::test]
    async fn the_halt_this_device_can_ask_for_is_fixed_to_its_own_tenant() {
        let halting = ledger(&LedgerAnswer::Done);
        halting
            .halt_this_tenant(&lease(), InventoryId::Tes, "repeated 403", AT)
            .await
            .expect("the halt lands");

        let body = halting.transport.last().await.1;
        assert!(
            body.contains("halt_this_tenant") && body.contains("Tes"),
            "the scope is the tenant's own inventory: {body}"
        );
        assert!(
            !body.contains("scope"),
            "there is no scope argument, so the wider halts stay the breaker's and the canary's: \
             {body}"
        );
    }

    #[tokio::test]
    async fn an_event_is_recorded_as_the_ledgers_own_payload() {
        let recording = ledger(&LedgerAnswer::Done);
        let payload = JobEventPayload::ItemActionFinished { sequence: 3 };
        recording
            .record_event(&lease(), &payload, AT)
            .await
            .expect("the event lands");
        assert_eq!(
            recording.transport.last_call().await,
            LedgerCall::RecordEvent {
                lease: lease(),
                payload,
                at_ms: AT.0,
            }
        );
    }

    #[tokio::test]
    async fn the_terminal_event_is_already_recorded_by_settle_and_is_not_posted_after_lease_release(
    ) {
        let recording = ledger(&LedgerAnswer::Done);
        recording
            .record_event(
                &lease(),
                &JobEventPayload::ItemSettled {
                    outcome: "Succeeded".to_owned(),
                },
                AT,
            )
            .await
            .expect("the atomic settle already recorded this event");
        assert!(
            recording.transport.sent.lock().await.is_empty(),
            "posting the event after settlement reaches a route that must reject the released \
             lease and makes a successful write look abandoned on the device"
        );
    }

    #[tokio::test]
    async fn a_park_names_the_gate_and_leaves_both_the_span_and_the_instant_to_the_server() {
        let parking = ledger(&LedgerAnswer::Done);
        parking
            .park(&lease(), "reauth_required")
            .await
            .expect("the park lands");
        assert_eq!(
            parking.transport.last_call().await,
            LedgerCall::Park {
                lease: lease(),
                blocked_on: "reauth_required".to_owned(),
            },
            "a device that could name its own park span could hold its tenant's queue for as \
             long as it liked, so the wire carries the gate and nothing else"
        );
    }

    /// A renew the server honours moves the run's deadline by its numbers.
    #[tokio::test(start_paused = true)]
    async fn a_renewal_moves_the_gate_by_the_servers_own_numbers() {
        use tam_engine_driver::ports::Cancellation as _;
        use tam_engine_driver::vocabulary::Renewed;
        let gate = crate::run::RunGate::from_envelope(0, 60_000);
        let ledger = HttpLedger::new(
            DeviceId::from_raw(DEVICE),
            FakeLedger::answering(&LedgerAnswer::Renewed {
                renewed: Renewed {
                    server_now_ms: 5_000_000_000_000,
                    server_deadline_ms: 5_000_000_300_000,
                },
            }),
        )
        .moving(gate.deadline());

        let renewed = ledger.renew(&lease()).await.expect("the heartbeat lands");
        assert_eq!(
            renewed.server_deadline_ms - renewed.server_now_ms,
            300_000,
            "the remaining lease is a subtraction of the server's own two instants"
        );
        tokio::time::advance(core::time::Duration::from_mins(2)).await;
        assert!(
            ItemLedger::renew(&ledger, &lease()).await.is_ok(),
            "the second heartbeat still lands"
        );
        assert!(
            !gate.is_cancelled(),
            "past the original sixty seconds the run continues, because the renew moved the \
             deadline the gate reads"
        );
    }

    /// A refused renew stops the run at its next loop top, before the next
    /// request rather than after it.
    #[tokio::test]
    async fn a_refused_renewal_stops_the_run_before_the_next_request() {
        let ledger = ledger(&LedgerAnswer::Refused {
            error: LedgerError::StaleLease,
        });
        let refused = ledger.renew(&lease()).await;
        assert_eq!(
            refused,
            Err(LedgerError::StaleLease),
            "a lease we no longer hold comes back as the answer the interpreter branches on, \
             not as a transport fault it would retry"
        );
    }
}
