//! The Teachouse desktop client.
//!
//! It hosts the existing SvelteKit console and owns the seller's marketplace
//! sessions on the seller's own device. That ownership is the whole point:
//! decision D1 in `docs/notes/design/vendoo-for-teachers-rethink.md` splits
//! automation in two, and for a marketplace with no official API — Tes and
//! TeachersPayTeachers today — every request must originate here, under the
//! seller's own session, with the server acting only as a control plane.
//!
//! What this contains: the shell, the login-and-capture flow, a device
//! identity, the entitlement gate, the control-plane check-in, and the data
//! plane itself — the pull, the interpreter, the seller's own session, and the
//! settle. Every marketplace request for a no-API marketplace originates here
//! and nowhere else; `docs/notes/design/desktop-data-plane.md` states the
//! custody line and what remains owed.

#![forbid(unsafe_code)]

pub mod commands;
pub mod connect;
pub mod console_session;
pub mod control_plane;
pub mod device;
pub mod entitlement;
pub mod heartbeat;
pub mod ledger;
pub mod marketplace;
pub mod payload;
pub mod run;
pub mod scheduler;
pub mod session;
pub mod startup;
pub mod state;
pub mod webview_session;
pub mod work;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::control_plane::{base_url, HttpControlPlane};
use crate::device::DeviceIdentity;
use crate::heartbeat::cycle;
use crate::run::wall_now;
use crate::scheduler::Scheduler;
use crate::session::keychain::KeychainSessionStore;
use crate::state::DesktopState;
use crate::webview_session::WebviewSession;
use crate::work::{DeviceWork, LiveMarketplaces};

/// Starts the application.
///
/// A failure to configure the application or to run its event loop is
/// reported through [`startup`] and exits non-zero, rather than panicking into
/// a stderr the release build has no console to show. There is no correct
/// degraded mode to fall back to: a client that cannot resolve its own data
/// directory would generate a new device identity on every launch and
/// re-register forever.
#[expect(
    clippy::exit,
    reason = "tauri::generate_context! expands to a process exit on a malformed bundle; the call \
              site is the macro, not this crate"
)]
pub fn run() {
    startup::catch_panics();
    let built = tauri::Builder::default()
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
            let store = Arc::new(KeychainSessionStore::new());
            // Method-call syntax rather than `Arc::clone`, which would resolve
            // its own type parameter against the annotation and refuse the
            // unsizing coercion these two bindings exist to perform.
            let sessions: Arc<dyn crate::session::SessionStore> = store.clone();
            let registry: Arc<dyn crate::heartbeat::ControlPlane> = plane.clone();
            let state = DesktopState::with_control_plane(device.clone(), sessions, registry);
            let work = DeviceWork::new(
                device.id.clone(),
                plane,
                LiveMarketplaces::new(store),
                &data_dir,
                state.stopper(),
            );
            app.manage(state);

            // A process killed mid-run runs neither the payload cache's discard
            // nor its drop, so start-up is where its bytes stop being kept.
            let sweep_dir = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                payload::sweep(&sweep_dir).await.ok();
            });
            tauri::async_runtime::spawn(run_schedule(app.handle().clone(), work));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect_marketplace,
            commands::session_status,
            commands::forget_session,
            commands::device_check_in,
            commands::device_activity,
        ])
        .build(tauri::generate_context!());

    // `build` and `run` rather than `Builder::run`, which is exactly the two
    // in sequence (tauri 2.11.5, `src/app.rs:2449`). Splitting them is what
    // puts the opening line in the log before the window is created rather
    // than after: Tauri builds the window declared in `tauri.conf.json` first
    // and calls the `setup` closure above second, in one function whose
    // failure it raises as a panic (`src/app.rs:1424`, `src/app.rs:2524`). A
    // missing WebView2 runtime therefore fails ahead of anything this crate
    // runs, and writing the opening line only from `setup` would leave that —
    // the likeliest Windows cause — with no log at all.
    let app = match built {
        Ok(app) => app,
        Err(why) => startup::fatal(&why),
    };
    match app.path().app_data_dir() {
        Ok(data_dir) => startup::opening(&data_dir),
        Err(why) => startup::fatal(&why),
    }
    app.run(|_, _| {});
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
async fn run_schedule<W: scheduler::WorkSource>(app: AppHandle, work: W) {
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
        cycle(&state, state.control_plane(), &scheduler, &work, wall_now()).await;
    }
}
