//! What the running application holds: the device it is, where sessions go,
//! and whether the server currently says it may work.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::device::DeviceIdentity;
use crate::entitlement::EntitlementGate;
use crate::session::SessionStore;

pub struct DesktopState {
    device: DeviceIdentity,
    store: Arc<dyn SessionStore>,
    /// Replaced wholesale at each check-in rather than mutated, so a
    /// half-applied entitlement never exists.
    gate: Mutex<EntitlementGate>,
}

impl DesktopState {
    #[must_use]
    pub fn new(device: DeviceIdentity, store: Arc<dyn SessionStore>) -> Self {
        Self {
            device,
            store,
            gate: Mutex::new(EntitlementGate::closed()),
        }
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
    }
}
