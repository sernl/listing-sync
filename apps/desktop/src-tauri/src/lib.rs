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
pub mod console_session;
pub mod control_plane;
pub mod device;
pub mod entitlement;
pub mod heartbeat;
pub mod scheduler;
pub mod session;
pub mod state;
pub mod webview_session;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::control_plane::{base_url, HttpControlPlane};
use crate::device::DeviceIdentity;
use crate::heartbeat::cycle;
use crate::scheduler::{NoWork, Scheduler};
use crate::session::keychain::KeychainSessionStore;
use crate::state::DesktopState;
use crate::webview_session::WebviewSession;

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

            // The session is resolved per request from the console's own
            // window, not captured here: the application starts before the
            // seller signs in, and the cookie appears afterwards.
            let origin = base_url();
            let sessions = Arc::new(WebviewSession::new(app.handle().clone(), &origin)?);
            let plane = Arc::new(HttpControlPlane::against(&origin, sessions)?);
            app.manage(DesktopState::with_control_plane(
                device,
                Arc::new(KeychainSessionStore::new()),
                plane,
            ));

            tauri::async_runtime::spawn(run_schedule(app.handle().clone()));
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

/// The local timer, running for the life of the process.
///
/// One cycle per tick: check in, then pull whatever work the entitlement gate
/// still allows. The check-in is the half that matters today, because it is
/// how a device the seller signed out from the console learns to wipe between
/// console loads; the work pull reaches [`NoWork`] until the engine driver
/// split lands a real source.
///
/// Nothing in the loop can panic: `cycle` swallows a failed check-in on
/// purpose — an offline period is not a revocation — and returns a report
/// rather than raising. That is what makes the dropped join handle safe here,
/// which is the property the workspace's ban on bare `tokio::spawn` protects.
#[expect(
    clippy::infinite_loop,
    reason = "a supervisor loop for the life of the process; the application exits by exiting"
)]
async fn run_schedule(app: AppHandle) {
    let scheduler = Scheduler::hourly_over_seller_device_marketplaces();
    // The first tick of an interval completes immediately, so there is a
    // check-in at start-up as well as one per cadence. That one races the
    // console window's creation on purpose and loses harmlessly: with no
    // window there is no session, which is `NoSession` — no request, no wipe,
    // and the state simply says nobody is signed in yet.
    let mut ticks = tokio::time::interval(scheduler.cadence());
    loop {
        ticks.tick().await;
        let state = app.state::<DesktopState>();
        cycle(
            &state,
            state.control_plane(),
            &scheduler,
            &NoWork,
            wall_now(),
        )
        .await;
    }
}

/// The client is a clock-reading process boundary in the same sense the
/// serving binary is: time enters the scheduler as data from here.
#[expect(
    clippy::disallowed_methods,
    reason = "the desktop client is a clock-reading process boundary; the local timer is the \
              location decision D1 moves, and it reads the seller's own clock"
)]
fn wall_now() -> tam_types::Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    tam_types::Timestamp(i64::try_from(millis).unwrap_or(0))
}
