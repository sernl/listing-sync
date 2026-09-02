//! What the running application holds: the device it is, where sessions go,
//! whether the server currently says it may work, and what it has been doing.

use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::VecDeque;
use std::sync::Arc;

use serde::Serialize;
use tam_domain::ItemOutcome;
use tam_types::{Marketplace, Timestamp};
use tokio::sync::Mutex;

use crate::device::{DeviceId, DeviceIdentity};
use crate::entitlement::EntitlementGate;
use crate::heartbeat::{ControlPlane, Offline};
use crate::session::SessionStore;

/// How many activity entries the console can read back.
///
/// Bounded rather than growing: this is an interface convenience and not a
/// journal. The ledger on the server is the record of what happened to an
/// item, and it is the one an operator reads.
pub const ACTIVITY_MAX: usize = 64;

/// Why a marketplace was not worked this tick. A closed set rather than a
/// message, because the seller acts on each of these differently and an
/// interface that could only show a sentence would have to parse one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    /// The seller signed this device out from the console. Checked first,
    /// because it is the only one of these the seller did on purpose.
    Revoked,
    /// Nobody is signed in to the console on this device, so there is no
    /// session to reach the control plane under. The ordinary state of a
    /// machine at a sign-in screen, and it resolves itself.
    NotSignedIn,
    /// The entitlement gate refuses this marketplace: lapsed, past the grace
    /// deadline, or never granted.
    NotEntitled,
    /// Nobody has signed in to this marketplace on this device, so there is no
    /// session to compose a request under.
    NoSession,
}

/// One thing this device did, or declined to do, for one marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkEvent {
    /// The control plane was asked and had nothing due.
    Idle,
    /// Another of the seller's devices holds the only slot for this
    /// marketplace account, so this one backs off for the delay the envelope
    /// suggested rather than polling a queue it cannot win.
    Held { next_poll_ms: u64 },
    /// An item was claimed and the interpreter started on it.
    Started { item: String },
    /// The item reached a terminal outcome, in the ledger's own vocabulary
    /// rather than a sentence: the console renders it, and a string the
    /// interface had to parse would be a second vocabulary to keep in step.
    Settled { item: String, outcome: ItemOutcome },
    /// The item parked on something the seller must clear.
    Parked { item: String, blocked_on: String },
    /// The run stopped without settling. The lease is left to expire and the
    /// server requeues with the epoch bumped: the stall bias, as the seller
    /// sees it.
    Abandoned { item: String, reason: String },
    /// Nothing was attempted, for a reason the seller can act on.
    Blocked { reason: BlockReason },
    /// The pull or the settle failed.
    Failed { detail: String },
}

/// What this device did, as the console shows it.
///
/// Every entry names the device. That is D1's interface rule rather than a
/// convenience: with the work running on the seller's own machines, "which
/// device did this" becomes a question the seller can ask, and one they must
/// be able to answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceActivity {
    pub device: DeviceId,
    pub at: Timestamp,
    pub marketplace: Marketplace,
    #[serde(flatten)]
    pub event: WorkEvent,
}

pub struct DesktopState {
    device: DeviceIdentity,
    store: Arc<dyn SessionStore>,
    /// Replaced wholesale at each check-in rather than mutated, so a
    /// half-applied entitlement never exists.
    gate: Mutex<EntitlementGate>,
    /// The seller signed this device out from the console, and the last
    /// check-in said so. Kept apart from the gate because the two answer
    /// different questions: the gate says whether work may run, and this says
    /// why it may not, which is what the interface shows.
    ///
    /// Shared rather than owned, because it is also the stop signal every run
    /// in flight observes: a check-in that learns of a revocation must reach
    /// the interpreter before its next marketplace request, not at the next
    /// tick.
    revoked: Arc<AtomicBool>,
    /// Whether the last check-in found a console session to speak under. A
    /// separate fact from the gate and from revocation: a machine sitting at
    /// a sign-in screen is neither entitled nor revoked, and the interface
    /// owes the seller that distinction rather than a bare "not syncing".
    signed_in: AtomicBool,
    /// How this device reaches the server's registry. `Offline` by default,
    /// because this slice ships no transport and a client that believed it had
    /// checked in would never learn it had been revoked.
    plane: Arc<dyn ControlPlane>,
    /// The most recent entries, newest last, bounded at [`ACTIVITY_MAX`].
    activity: Mutex<VecDeque<DeviceActivity>>,
}

impl DesktopState {
    #[must_use]
    pub fn new(device: DeviceIdentity, store: Arc<dyn SessionStore>) -> Self {
        Self::with_control_plane(device, store, Arc::new(Offline))
    }

    #[must_use]
    pub fn with_control_plane(
        device: DeviceIdentity,
        store: Arc<dyn SessionStore>,
        plane: Arc<dyn ControlPlane>,
    ) -> Self {
        Self {
            device,
            store,
            gate: Mutex::new(EntitlementGate::closed()),
            revoked: Arc::new(AtomicBool::new(false)),
            signed_in: AtomicBool::new(false),
            plane,
            activity: Mutex::new(VecDeque::new()),
        }
    }

    #[must_use]
    pub fn control_plane(&self) -> &dyn ControlPlane {
        self.plane.as_ref()
    }

    #[must_use]
    pub const fn device(&self) -> &DeviceIdentity {
        &self.device
    }

    #[must_use]
    pub fn store(&self) -> &dyn SessionStore {
        self.store.as_ref()
    }

    /// The gate as it currently stands, cloned out rather than borrowed, so no
    /// caller holds the lock across the work it then decides to do.
    pub async fn gate(&self) -> EntitlementGate {
        self.gate.lock().await.clone()
    }

    /// Installs the entitlement a check-in returned.
    pub async fn set_gate(&self, gate: EntitlementGate) {
        *self.gate.lock().await = gate;
    }

    /// Whether the last check-in said this device had been signed out.
    #[must_use]
    pub fn revoked(&self) -> bool {
        self.revoked.load(Ordering::SeqCst)
    }

    /// Records what a check-in answered. Set from the answer rather than only
    /// ever raised, so signing the device back in on the console clears it at
    /// the next check-in instead of needing a restart.
    pub fn set_revoked(&self, revoked: bool) {
        self.revoked.store(revoked, Ordering::SeqCst);
    }

    /// The stop signal a run in flight observes. Raised by exactly the same
    /// write [`Self::set_revoked`] makes, so there is one fact rather than two
    /// that could disagree.
    #[must_use]
    pub fn stopper(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.revoked)
    }

    /// Whether the last check-in had a console session to speak under.
    ///
    /// Starts false, because a device that has never checked in has not found
    /// one, and claiming otherwise would let the interface show a working
    /// state the device has no evidence for.
    #[must_use]
    pub fn signed_in(&self) -> bool {
        self.signed_in.load(Ordering::SeqCst)
    }

    pub fn set_signed_in(&self, signed_in: bool) {
        self.signed_in.store(signed_in, Ordering::SeqCst);
    }

    /// Records one thing this device did, dropping the oldest entry once the
    /// ring is full.
    pub async fn record(&self, marketplace: Marketplace, at: Timestamp, event: WorkEvent) {
        let mut activity = self.activity.lock().await;
        if activity.len() == ACTIVITY_MAX {
            activity.pop_front();
        }
        activity.push_back(DeviceActivity {
            device: self.device.id.clone(),
            at,
            marketplace,
            event,
        });
        drop(activity);
    }

    /// What this device has been doing, oldest first.
    pub async fn activity(&self) -> Vec<DeviceActivity> {
        self.activity.lock().await.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopState;
    use crate::device::{DeviceId, DeviceIdentity};
    use crate::entitlement::EntitlementGate;
    use crate::session::memory::MemorySessionStore;
    use std::sync::Arc;
    use tam_types::{Marketplace, Timestamp};

    #[tokio::test]
    async fn a_fresh_application_may_work_nothing_until_a_check_in_says_otherwise() {
        let state = DesktopState::new(
            DeviceIdentity {
                id: DeviceId::from_raw("1111"),
                label: "founder-pc".to_owned(),
            },
            Arc::new(MemorySessionStore::new()),
        );
        for marketplace in Marketplace::ALL {
            assert!(
                !state
                    .gate()
                    .await
                    .may_work(marketplace, Timestamp(1_756_000_000_000)),
                "the gate starts closed, so a client that never reached the server does no work"
            );
        }
        assert_eq!(state.gate().await, EntitlementGate::closed());
        assert!(
            !state.revoked(),
            "a device nobody has signed out is not revoked; the closed gate is the \
             absence of an entitlement, which is a different fact"
        );
        assert!(
            !state.signed_in(),
            "a device that has never checked in has found no session, and must not \
             claim one it has no evidence for"
        );
    }
}
