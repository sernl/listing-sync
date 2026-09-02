//! What the running application holds: the device it is, where sessions go,
//! and whether the server currently says it may work.

use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::device::DeviceIdentity;
use crate::entitlement::EntitlementGate;
use crate::heartbeat::{ControlPlane, Offline};
use crate::session::SessionStore;

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
    revoked: AtomicBool,
    /// How this device reaches the server's registry. `Offline` by default,
    /// because this slice ships no transport and a client that believed it had
    /// checked in would never learn it had been revoked.
    plane: Arc<dyn ControlPlane>,
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
            revoked: AtomicBool::new(false),
            plane,
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
    }
}
