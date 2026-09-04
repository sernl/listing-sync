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
// `tauri::mobile_entry_point` emits a sibling `stop_unwind` function that
// wraps the entry in `std::panic::catch_unwind`, because this is an
// `extern "C"` boundary and unwinding across one is undefined behaviour. That
// generated item is a sibling of `run` rather than part of it, so no
// attribute on `run` can cover it and one placed there is reported unfulfilled
// (verified: tauri-macros 2.6.3, `src/mobile.rs:63`). The expectation is
// therefore crate-level, and `cfg_attr(mobile, ...)` keeps it off the desktop
// build, where the macro expands to nothing and there would be nothing to
// expect. clippy.toml is unchanged, this fails the build the day Tauri stops
// needing the wrapper, and the residual is stated rather than hidden: the only
// Android-only source in this crate is `session/android_key.rs`, which the
// host lane does not lint, so a disallowed call added there would not be
// caught. Everything else is shared code the host lane checks.
#![cfg_attr(
    mobile,
    expect(
        clippy::disallowed_methods,
        reason = "the mobile entry point's catch_unwind is Tauri's, at an FFI boundary that \
                  requires it; the call site is the macro, not this crate"
    )
)]

pub mod commands;
pub mod connect;
pub mod console_session;
pub mod control_plane;
pub mod device;
pub mod entitlement;
pub mod heartbeat;
pub mod import;
pub mod ledger;
pub mod marketplace;
pub mod payload;
pub mod run;
pub mod scheduler;
pub mod session;
pub mod startup;
pub mod state;
pub mod updater;
pub mod webview_session;
pub mod work;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::control_plane::{base_url, HttpControlPlane};
use crate::device::DeviceIdentity;
use crate::heartbeat::cycle;
use crate::run::wall_now;
use crate::scheduler::Scheduler;
// The credential store this platform actually has. `keyring` covers Windows,
// macOS and Linux; on Android it has no backend at all, so the jar is sealed
// into a file instead, under a key the Android Keystore holds.
#[cfg(target_os = "android")]
use crate::session::encrypted::{DeviceKeySource, EncryptedSessionStore};
#[cfg(not(target_os = "android"))]
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
#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[expect(
    clippy::exit,
    reason = "tauri::generate_context! expands to a process exit on a malformed bundle; the call \
              site is the macro, not this crate"
)]
pub fn run() {
    startup::catch_panics();
    // What replaces the timer on a phone. The activity is resumed whenever the
    // seller brings the application forward, and that is the only moment a
    // device which was signed out elsewhere can learn it, because D3 leaves it
    // no background schedule to learn it in.
    #[cfg(mobile)]
    let resumed = Arc::new(tokio::sync::Notify::new());
    #[cfg(mobile)]
    let on_resume = Arc::clone(&resumed);
    // A second clone, moved into `setup` rather than borrowed by it: the
    // closure outlives this function.
    #[cfg(mobile)]
    let on_start = Arc::clone(&resumed);
    let builder = tauri::Builder::default().plugin(tauri_plugin_os::init());
    // `tauri-plugin-updater` declares `platforms.support.android.level = "none"`
    // in its own manifest, so a phone updates through the store it was
    // installed from and never through us. Registering it there anyway would
    // give the console an update surface that answers nothing. Shadowing
    // rather than a `mut` binding, which would be pointlessly mutable on the
    // build where this line is compiled out.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(session_key_bridge());
    let built = builder
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let device: DeviceIdentity =
                device::load_or_create(&data_dir, &tauri_plugin_os::hostname())?;

            // The session is resolved per request from the console's own
            // window, not captured here: the application starts before the
            // seller signs in, and the cookie appears afterwards.
            let origin = base_url();
            let sessions = Arc::new(WebviewSession::new(app.handle().clone(), &origin)?);
            let plane = Arc::new(HttpControlPlane::against(&origin, sessions)?);
            #[cfg(not(target_os = "android"))]
            let store = Arc::new(KeychainSessionStore::new());
            // The key source is filed in managed state by the plugin below,
            // and plugins are initialised before this closure runs, so it is
            // always there by now (tauri 2.11.5, `src/app.rs:2440` and
            // `:2531`). Its absence would be a build-order fault rather than
            // a runtime condition, so it fails start-up loudly through
            // `startup` rather than degrading to a store that forgets.
            #[cfg(target_os = "android")]
            let store = Arc::new(EncryptedSessionStore::in_data_dir(
                &data_dir,
                Arc::clone(app.state::<Arc<dyn DeviceKeySource>>().inner()),
            ));
            // Method-call syntax rather than `Arc::clone`, which would resolve
            // its own type parameter against the annotation and refuse the
            // unsizing coercion these two bindings exist to perform.
            let sessions: Arc<dyn crate::session::SessionStore> = store.clone();
            let registry: Arc<dyn crate::heartbeat::ControlPlane> = plane.clone();
            let state = DesktopState::with_control_plane(device.clone(), sessions, registry);
            let work = DeviceWork::new(
                device.id.clone(),
                plane,
                LiveMarketplaces::new(store, state.gate_handle()),
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
            // One task rather than two, because the order is the guarantee.
            // Installing an update ends the process — on Windows from inside
            // the install itself (tauri-plugin-updater 2.11.0,
            // `src/updater.rs:876`) — so it has to happen before the schedule
            // claims any work rather than during a marketplace request. The
            // check is bounded, so an unreachable endpoint delays the first
            // cycle by `updater::CHECK_TIMEOUT` and no more.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    updater::check_at_startup(&handle).await;
                    run_schedule(handle, work).await;
                });
            }
            #[cfg(mobile)]
            tauri::async_runtime::spawn(run_schedule(
                app.handle().clone(),
                work,
                Arc::clone(&on_start),
            ));
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
    // Two callbacks rather than one with a platform-dead branch: the desktop
    // build has nothing to do with the event, and naming a binding it never
    // reads would be a lie the linter is right to catch.
    #[cfg(desktop)]
    app.run(|_, _| {});
    #[cfg(mobile)]
    app.run(move |_, event| {
        if matches!(event, tauri::RunEvent::Resumed) {
            on_resume.notify_one();
        }
    });
}

/// Registers the Kotlin class that holds the session-sealing secret, and
/// files the resulting handle in managed state where `setup` picks it up.
///
/// A plugin rather than a call in `setup`, because `register_android_plugin`
/// lives on `PluginApi`, which only a plugin's own setup is handed (tauri
/// 2.11.5, `src/plugin/mobile.rs:208`). Registering it here rather than
/// publishing a plugin crate is what keeps this to one Kotlin file with no
/// Gradle module and no direct `jni` dependency.
#[cfg(target_os = "android")]
fn session_key_bridge<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("session-key")
        .setup(|app, api| {
            let handle = api.register_android_plugin(
                session::android_key::PLUGIN_IDENTIFIER,
                session::android_key::PLUGIN_CLASS,
            )?;
            let keys: Arc<dyn DeviceKeySource> =
                Arc::new(session::android_key::KeystoreKey::new(handle));
            app.manage(keys);
            Ok(())
        })
        .build()
}

/// One cycle: check in, then pull whatever work the entitlement gate still
/// allows.
///
/// The check-in is the half that matters today, because it is how a device the
/// seller signed out from the console learns to wipe between console loads.
///
/// Nothing here can panic: `cycle` swallows a failed check-in on purpose — an
/// offline period is not a revocation — and returns a report rather than
/// raising. That is what makes the dropped join handles below safe, which is
/// the property the workspace's ban on bare `tokio::spawn` protects.
async fn run_cycle<W: scheduler::WorkSource>(app: &AppHandle, scheduler: &Scheduler, work: &W) {
    let state = app.state::<DesktopState>();
    cycle(&state, state.control_plane(), scheduler, work, wall_now()).await;
}

/// The local timer, running for the life of the process.
#[cfg(desktop)]
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
        run_cycle(&app, &scheduler, &work).await;
    }
}

/// The same cycle on a phone, with no timer behind it.
///
/// D3 limits a phone to work the seller starts, and the platform is why:
/// Android's Doze stops `JobScheduler` and therefore `WorkManager`, and the
/// battery-optimisation exemption that would evade it is barred by Play
/// policy. A cadence here would be a promise the platform breaks, so there is
/// none — the scheduler's own cadence field is never read on this path, and it
/// is constructed only for the marketplace set it carries. What replaces the
/// timer is the seller: one cycle at start-up, one on every resume, and the
/// console's own commands in between.
#[cfg(mobile)]
#[expect(
    clippy::infinite_loop,
    reason = "a supervisor loop for the life of the process; the application exits by exiting"
)]
async fn run_schedule<W: scheduler::WorkSource>(
    app: AppHandle,
    work: W,
    resumed: Arc<tokio::sync::Notify>,
) {
    let scheduler = Scheduler::hourly_over_seller_device_marketplaces();
    run_cycle(&app, &scheduler, &work).await;
    loop {
        resumed.notified().await;
        run_cycle(&app, &scheduler, &work).await;
    }
}
