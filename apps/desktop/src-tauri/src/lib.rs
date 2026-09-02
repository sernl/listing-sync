//! The Teachouse desktop client.
//!
//! It hosts the existing SvelteKit console and owns the seller's marketplace
//! sessions on the seller's own device. That ownership is the whole point:
//! decision D1 in `docs/notes/design/vendoo-for-teachers-rethink.md` splits
//! automation in two, and for a marketplace with no official API — Tes and
//! TeachersPayTeachers today — every request must originate here, under the
//! seller's own session, with the server acting only as a control plane.
//!
//! What this slice contains: the shell, the login-and-capture flow, a device
//! identity, the entitlement gate, and a scheduler skeleton. What it does not
//! contain, deliberately, is a single marketplace request. The engine driver
//! split that will make one is a separate stream, and
//! [`scheduler::WorkSource`] is the seam it will arrive through.

#![forbid(unsafe_code)]

pub mod commands;
pub mod connect;
pub mod device;
pub mod entitlement;
pub mod heartbeat;
pub mod scheduler;
pub mod session;
pub mod state;

use std::sync::Arc;

use tauri::Manager;

use crate::device::DeviceIdentity;
use crate::session::keychain::KeychainSessionStore;
use crate::state::DesktopState;

/// Starts the application.
///
/// # Panics
///
/// If the Tauri context or the application data directory is unusable, which
/// is a broken installation rather than a runtime condition.
#[expect(
    clippy::expect_used,
    reason = "a client that cannot resolve its own data directory has no correct degraded mode: \
              it would generate a new device identity on every launch and re-register forever"
)]
#[expect(
    clippy::exit,
    reason = "tauri::generate_context! expands to a process exit on a malformed bundle; the call \
              site is the macro, not this crate"
)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let device: DeviceIdentity =
                device::load_or_create(&data_dir, &tauri_plugin_os::hostname())?;
            app.manage(DesktopState::new(
                device,
                Arc::new(KeychainSessionStore::new()),
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect_marketplace,
            commands::session_status,
            commands::forget_session,
            commands::device_check_in,
        ])
        .run(tauri::generate_context!())
        .expect("the Teachouse desktop client starts");
}
