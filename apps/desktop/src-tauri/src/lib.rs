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
// needing the wrapper, and the residual is stated rather than hidden: the
// Android-only sources in this crate are `session/android_key.rs` and
// `android_name.rs`, which the host lane does not lint, so a disallowed call
// added to either would not be caught. Everything else is shared code the
// host lane checks.
#![cfg_attr(
    mobile,
    expect(
        clippy::disallowed_methods,
        reason = "the mobile entry point's catch_unwind is Tauri's, at an FFI boundary that \
                  requires it; the call site is the macro, not this crate"
    )
)]

#[cfg(target_os = "android")]
pub mod android_name;
pub mod commands;
pub mod connect;
pub mod console_session;
pub mod control_plane;
pub mod device;
pub mod entitlement;
pub mod heartbeat;
pub mod import;
pub mod ledger;
pub mod library;
pub mod library_sync;
pub mod marketplace;
pub mod notify;
pub mod payload;
pub mod run;
pub mod scheduler;
pub mod session;
pub mod startup;
pub mod state;
pub mod transfer;
#[cfg(desktop)]
pub mod updater;
pub mod webview_session;
pub mod work;

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::control_plane::{base_url, install_crypto_provider, HttpControlPlane};
use crate::device::DeviceIdentity;
use crate::notify::{DeviceNotifier, PluginSurface};
use crate::run::wall_now;
use crate::scheduler::Scheduler;
// The credential store this platform actually has. `keyring` covers Windows,
// macOS and Linux; on Android it has no backend at all, so the jar is sealed
// into a file instead, under a key the Android Keystore holds.
#[cfg(target_os = "android")]
use crate::android_name::DeviceNameSource;
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
    // Here rather than in `setup`, because the first reqwest client in this
    // process is Tauri's and not ours: a development build for a phone builds
    // one while preparing the window, which Tauri does before it calls
    // `setup`. Its own documentation states why that is fatal without this.
    install_crypto_provider();
    // Every trigger reaches the one coordinator through this: start-up, the
    // console's own commands, and — on a phone — the resume, which is the
    // only moment D3 leaves a handset to act in, because Doze stops
    // `JobScheduler` and the exemption that would evade it is barred by Play
    // policy.
    let wake = Arc::new(Wake::default());
    #[cfg(mobile)]
    let on_resume = Arc::clone(&wake);
    // A second clone, moved into `setup` rather than borrowed by it: the
    // closure outlives this function.
    let on_start = Arc::clone(&wake);
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    // `tauri-plugin-updater` declares `platforms.support.android.level = "none"`
    // in its own manifest, so a phone updates through the store it was
    // installed from and never through us. Registering it there anyway would
    // give the console an update surface that answers nothing. Shadowing
    // rather than a `mut` binding, which would be pointlessly mutable on the
    // build where this line is compiled out.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let built = builder
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let device: DeviceIdentity =
                device::load_or_create(&data_dir, &this_machines_name(app))?;

            // The session is resolved per request from the console's own
            // window, not captured here: the application starts before the
            // seller signs in, and the cookie appears afterwards.
            let origin = base_url();
            let sessions = Arc::new(WebviewSession::new(app.handle().clone(), &origin)?);
            let plane = Arc::new(HttpControlPlane::against(&origin, sessions)?);
            #[cfg(not(target_os = "android"))]
            let store = Arc::new(KeychainSessionStore::new());
            // The key source is filed in managed state by the plugin below,
            // which is registered after `build` returns and before `run` is
            // called, while this closure runs from inside `run` on the `Ready`
            // event (tauri 2.11.5, `src/app.rs:1424`). So it is always there by
            // now. Its absence would be a build-order fault rather than a
            // runtime condition, so it fails start-up loudly through `startup`
            // rather than degrading to a store that forgets.
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
            // The same object again, as the other trait it implements. An
            // import posts its pages over the ledger transport rather than the
            // registry, and a state holding only the first could hand the
            // second to nothing.
            let ledger: Arc<dyn crate::ledger::LedgerTransport> = plane.clone();
            let notifier: Arc<dyn crate::notify::Notifier> = Arc::new(DeviceNotifier::new(
                PluginSurface::new(app.handle().clone()),
            ));
            // The library key, from the same custody the session has on this
            // platform, under its own entry. A library that cannot open is
            // logged and left absent rather than failing start-up: the
            // seller can still connect and import, and the console says the
            // files are not being kept.
            #[cfg(not(target_os = "android"))]
            let library_key = library::keychain_library_key(session::keychain::SERVICE);
            #[cfg(target_os = "android")]
            let library_key = tauri::async_runtime::block_on(library::sealed_library_key(
                &data_dir,
                app.state::<Arc<dyn DeviceKeySource>>().inner().as_ref(),
            ));
            let library = match library_key.and_then(|key| library::Library::open(&data_dir, key)) {
                Ok(library) => Some(Arc::new(library)),
                Err(why) => {
                    eprintln!("the library on this machine could not be opened: {why}");
                    None
                }
            };
            let mut state = DesktopState::with_control_plane(device.clone(), sessions, registry)
                .with_ledger(ledger)
                .with_journal(Arc::new(crate::import::FileJournal::in_data_dir(&data_dir)))
                .with_notifier(notifier);
            if let Some(library) = &library {
                state = state.with_library(Arc::clone(library));
            }
            // The parts the schedule needs to keep this machine's library in
            // step with the seller's others: the endpoint is bound inside the
            // schedule's own task, because binding is asynchronous and this
            // closure is not.
            let syncing = library
                .as_ref()
                .map(|library| (device.id.clone(), Arc::clone(&plane), Arc::clone(library)));
            let work = DeviceWork::new(
                device.id.clone(),
                plane,
                LiveMarketplaces::new(store, state.gate_handle()),
                &data_dir,
                state.stopper(),
            )
            .reading(library);
            app.manage(state);
            // The handle the commands reach the coordinator through. Managed
            // rather than passed, because a command is handed an `AppHandle`
            // and nothing else.
            app.manage(Arc::clone(&on_start));

            // The bootstrap can invoke as soon as its document loads.
            // Expose the window only after its command state exists.
            let window = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or("the main window configuration is missing")?;
            tauri::WebviewWindowBuilder::from_config(app, window)?.build()?;

            // A process killed mid-run runs neither the payload cache's discard
            // nor its drop, so start-up is where its bytes stop being kept.
            let sweep_dir = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                payload::sweep(&sweep_dir).await.ok();
            });
            // An import this device was working when the process died is
            // recovered by `run_schedule`'s first cycle. It must not have a
            // separate start-up task: `DesktopState` begins with its
            // entitlement gate closed, while the cycle checks in and installs
            // the current entitlement before `serve_open_runs` can claim or
            // report against a nominated run.
            //
            // On desktop that ordered cycle follows the bounded updater check;
            // on mobile it runs immediately. Both are earlier than their next
            // cadence or resume, without racing an uninitialised gate.
            // Installing an update ends the process — on Windows from inside
            // the install itself (tauri-plugin-updater 2.11.0,
            // `src/updater.rs:876`) — so it has to happen before the schedule
            // claims any work rather than during a marketplace request. The
            // check is bounded, so an unreachable endpoint delays the first
            // cycle by `updater::CHECK_TIMEOUT` and no more.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                let wake = Arc::clone(&on_start);
                tauri::async_runtime::spawn(async move {
                    updater::check_at_startup(&handle).await;
                    run_schedule(handle, work, wake, syncing).await;
                });
            }
            #[cfg(mobile)]
            tauri::async_runtime::spawn(run_schedule(
                app.handle().clone(),
                work,
                Arc::clone(&on_start),
                syncing,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect_marketplace,
            commands::session_status,
            commands::forget_session,
            commands::device_check_in,
            commands::device_activity,
            commands::start_import,
            commands::continue_import,
            commands::stop_import,
            commands::retry_console,
            commands::set_theme,
            commands::library_entries,
            commands::library_usage,
            commands::library_read,
            commands::library_remove,
            commands::library_settings,
            commands::set_library_settings,
            commands::library_open_external,
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
    // Registered here rather than on the builder, and the placement is the
    // whole point: a plugin added to the builder is initialised inside `build`
    // (tauri 2.11.5, `src/app.rs:2440`), which is before `opening` has resolved
    // the data directory, so a failure there reports to a stderr the Windows
    // release build discards and to no file at all. Moving it after `opening`
    // puts the one initialisation that talks to the Android Keystore inside the
    // region that has a log. `AppHandle::plugin` initialises immediately
    // (`src/app.rs:527`), so the key source is still managed before `setup`
    // reads it on the `Ready` event.
    #[cfg(target_os = "android")]
    if let Err(why) = app.handle().plugin(session_key_bridge()) {
        startup::fatal(&why);
    }
    // Two callbacks rather than one with a platform-dead branch: the desktop
    // build has nothing to do with the event, and naming a binding it never
    // reads would be a lie the linter is right to catch.
    #[cfg(desktop)]
    app.run(|_, _| {});
    #[cfg(mobile)]
    app.run(move |_, event| {
        if matches!(event, tauri::RunEvent::Resumed) {
            on_resume.now();
        }
    });
}

/// What this machine is called in the seller's list.
///
/// The hostname everywhere but Android, where it is a loopback name no seller
/// would recognise and the phone's own model is read instead. D14 says the
/// device is "labelled with the hostname"; on a phone that decision's intent —
/// a name the seller can pick their machine out by — is served by the model
/// and defeated by the hostname.
///
/// The label is refreshed from this source on every launch while the id is
/// not, so a phone whose name could not be read once is renamed on the launch
/// after and does not become a second machine.
#[cfg(target_os = "android")]
fn this_machines_name(app: &tauri::App) -> String {
    app.state::<Arc<dyn DeviceNameSource>>()
        .label()
        .unwrap_or_else(|| device::ANDROID_FALLBACK_LABEL.to_owned())
}

#[cfg(not(target_os = "android"))]
fn this_machines_name(_app: &tauri::App) -> String {
    tauri_plugin_os::hostname()
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
            // One Kotlin class answers every command, so one handle serves
            // all three bindings and the later ones are clones rather than
            // second registrations.
            let phone = Arc::new(android_name::PhoneName::new(handle.clone()));
            let names: Arc<dyn DeviceNameSource> = phone.clone();
            // The same object again, as the other trait it implements: the
            // coordinator asks it whether the seller is looking at the
            // application before every pass.
            let here: Arc<dyn android_name::ForegroundSource> = phone;
            let keys: Arc<dyn DeviceKeySource> =
                Arc::new(session::android_key::KeystoreKey::new(handle));
            app.manage(names);
            app.manage(here);
            app.manage(keys);
            Ok(())
        })
        .build()
}

/// One check-in, and what it learned about this device's standing.
///
/// Inline in the loop, because it is one request under a bounded timeout: the
/// whole point of the split is that nothing slow shares a thread with it.
///
/// The standing is read back off the state rather than off the answer,
/// because only a check-in that reached the server moves it: an unreachable
/// server says nothing about whether the seller signed this machine out, and
/// a coordinator that read an outage as a revocation would stop discovering
/// for the length of the outage.
async fn run_check_in(app: &AppHandle) -> bool {
    let state = app.state::<DesktopState>();
    heartbeat::check_in_or_register(&state, state.control_plane())
        .await
        .ok();
    state.revoked()
}

/// The cheap pass: ask which import runs this device owes work on.
///
/// Inline as well. It makes one request and dispatches whatever it finds into
/// the import supervisor's own tracked tasks, so it returns in the time of
/// that request however long the imports it started go on for.
async fn run_open_runs(app: &AppHandle) -> scheduler::Discovery {
    let state = app.state::<DesktopState>();
    crate::import::serve_open_runs(&state, state.control_plane()).await
}

/// The heavy pass: claim whatever the queue holds and run it to a verdict.
///
/// Spawned rather than awaited in the loop, and supervised one at a time. One
/// claimed job may legitimately take half an hour — an upload, a
/// reconciliation walk — and before this split that job held the five-minute
/// check-in and the ten-second import poll behind it for its whole duration.
/// One at a time is also what keeps the claim honest: a second concurrent pass
/// would ask for work while the first still held an item, which is how the
/// same item gets claimed twice.
///
/// Nothing here can panic: `work_pending` returns a report rather than
/// raising, which is the property the workspace's ban on bare `tokio::spawn`
/// protects.
async fn run_work<W: scheduler::WorkSource>(
    app: &AppHandle,
    scheduler: &Scheduler,
    work: &W,
) -> scheduler::Discovery {
    let state = app.state::<DesktopState>();
    heartbeat::work_pending(&state, scheduler, work, wall_now())
        .await
        .discovery()
}

/// The one coordinator this installation runs, for the life of the process.
///
/// One loop, not three: the cadences differ but the activities share a device,
/// a control plane and an import supervisor, and separate loops would be
/// separate chances to run two of them at once. Every trigger — start-up, a
/// resume, a seller pressing a button in the console — is one notification on
/// [`Wake`], which coalesces by construction: `Notify` holds one permit, so
/// twenty triggers between two passes produce one pass.
///
/// On a phone every pass is gated on the seller actually having the
/// application in front of them, read immediately before the pass is decided
/// from the process-lifetime count of started activities
/// ([`android_name::ForegroundSource`]).
/// That read is local and free; it is not a clock, and it is not a
/// window opened by a resume — such a window stops discovering while the
/// seller is still working and goes on polling after they have left, which
/// are the two failures this gate exists to have neither of.
///
/// Three activities, two of which are inline and one supervised. The check-in
/// and the open-runs poll are one request each and run in the loop; the work
/// pull may claim a job with a thirty-minute budget, so it runs as a single
/// supervised task and the loop goes on serving the other two while it does.
/// One such task at a time, which is also what stops an item being claimed
/// twice.
///
/// What each activity costs is in [`heartbeat::work_pending`] and
/// [`crate::import::serve_open_runs`]; how the three are timed is in
/// [`scheduler::Coordination`].
async fn run_schedule<W: scheduler::WorkSource + 'static>(
    app: AppHandle,
    work: W,
    wake: Arc<Wake>,
    syncing: Option<(
        crate::device::DeviceId,
        Arc<HttpControlPlane>,
        Arc<library::Library>,
    )>,
) {
    // The transfer endpoint, bound once for the life of the process. A
    // library whose endpoint cannot bind is still a library: imports keep
    // their originals and the console lists them; only the direct copy to
    // another machine is off, and the log says so.
    let mut library_sync = None;
    if let Some((device, plane, library)) = syncing {
        match library.node_secret().await {
            Ok(secret) => match transfer::Transfer::start(Arc::clone(&library), secret).await {
                Ok(transfer) => {
                    library_sync = Some(library_sync::LibrarySync {
                        device,
                        plane,
                        library,
                        transfer: Arc::new(transfer),
                    });
                }
                Err(why) => eprintln!("this machine's transfer endpoint did not start: {why}"),
            },
            Err(why) => eprintln!("this machine's node key could not be read: {why}"),
        }
    }
    let scheduler = Scheduler::hourly_over_seller_device_marketplaces();
    // Shared with the supervised task rather than moved into it, so the next
    // pass uses the same source and the same claim identity.
    let work = Arc::new(work);
    #[cfg(desktop)]
    let mut plan = scheduler::Coordination::running(scheduler.cadence());
    #[cfg(mobile)]
    let mut plan = scheduler::Coordination::on_a_phone(scheduler.cadence());
    // Monotonic, and read once: every fast cadence is measured against this
    // rather than against the wall clock, so a clock correction cannot stop a
    // running device checking in or discovering.
    let started = tokio::time::Instant::now();
    // Start-up is a trigger, so the first pass is immediate: a check-in
    // before anything else, which races the console window's creation on
    // purpose and loses harmlessly — with no window there is no session,
    // which is `NoSession`, so no request is made and the state simply says
    // nobody is signed in yet.
    plan.triggered();
    let mut running: Option<tauri::async_runtime::JoinHandle<scheduler::Discovery>> = None;
    loop {
        #[cfg(mobile)]
        plan.observed_foreground(in_the_foreground(&app));
        let due = plan.due(reading(started));
        // A sweep is the whole cycle, so its stamps are taken first and its
        // two halves are then performed by the same code the fast passes use.
        if due.sweep {
            plan.swept(reading(started));
        }
        if due.sweep || due.check_in {
            if let Some(sync) = &library_sync {
                sync.advertise().await;
            }
            let revoked = run_check_in(&app).await;
            if !due.sweep {
                plan.checked_in(reading(started));
            }
            plan.observed_revoked(revoked);
            // The wants after the check-in, and only while this machine is
            // in good standing: a signed-out machine fetches nothing.
            if let (Some(sync), false) = (&library_sync, revoked) {
                sync.serve_wants().await;
            }
        }
        // Re-read after the check-in rather than the plan re-asked. The
        // revocation may have just arrived, in which case the gate is closed
        // and nothing may be claimed; and on a phone the seller may have left
        // while that request was in flight, which must stop the pass here
        // rather than after it. Asking the plan again instead would drop the
        // seller's own trigger, because serving the check-in is what clears
        // it.
        let standing = !app.state::<DesktopState>().revoked();
        #[cfg(mobile)]
        let standing = standing && in_the_foreground(&app);
        if (due.sweep || due.discover) && standing {
            // Stamp every import poll, even while a work task is running;
            // otherwise discovery stays due and the one-second floor takes over.
            if !due.sweep {
                plan.discovering(reading(started));
            }
            let found = run_open_runs(&app).await;
            plan.imports_discovered(found);
            // Work runs independently, with at most one task in flight.
            if running.is_none() {
                let handle = app.clone();
                let timer = scheduler.clone();
                let source = Arc::clone(&work);
                running = Some(tauri::async_runtime::spawn(async move {
                    run_work(&handle, &timer, source.as_ref()).await
                }));
            }
        }
        let nap = plan.sleep(reading(started));
        // Keep a sleep floor while work runs if a lifecycle change prevents
        // a due poll from being served.
        let nap = if running.is_some() {
            nap.max(core::time::Duration::from_secs(1))
        } else {
            nap
        };
        let mut finished = None;
        tokio::select! {
            () = tokio::time::sleep(nap) => {}
            () = wake.notified() => plan.triggered(),
            found = settled(&mut running) => finished = Some(found),
        }
        if let Some(found) = finished {
            running = None;
            plan.discovered(found);
            plan.observed_revoked(app.state::<DesktopState>().revoked());
        }
    }
}

/// Both clock readings, taken together so one pass is decided against one
/// instant.
fn reading(started: tokio::time::Instant) -> scheduler::Clocks {
    scheduler::Clocks {
        wall: wall_now(),
        since_start: started.elapsed(),
    }
}

/// The supervised job's outcome, or never where there is no job.
///
/// `pending` rather than an immediate answer, so the branch simply does not
/// fire when nothing is running: a future that returned at once would turn the
/// select into a spin. A task that panicked answers `Failed`, which puts the
/// next pass on the ladder rather than repeating it immediately.
async fn settled(
    running: &mut Option<tauri::async_runtime::JoinHandle<scheduler::Discovery>>,
) -> scheduler::Discovery {
    match running.as_mut() {
        Some(handle) => handle.await.unwrap_or(scheduler::Discovery::Failed),
        None => core::future::pending().await,
    }
}

/// Whether the seller is looking at this application, asked of the platform.
///
/// The bridge is registered by [`session_key_bridge`] on Android. Where it is
/// absent — an iOS build, which registers no such plugin — the answer is true,
/// and the platform is what enforces the property there instead: iOS suspends
/// a backgrounded process, so its timers do not run and there is nothing to
/// gate. Android is the one surface where a loop can keep ticking behind the
/// seller's back, and that is the surface the bridge covers.
#[cfg(mobile)]
fn in_the_foreground(app: &AppHandle) -> bool {
    app.try_state::<Arc<dyn android_name::ForegroundSource>>()
        .is_none_or(|here| here.foreground())
}

/// The one handle every trigger reaches the coordinator through.
///
/// Managed state rather than a field on [`DesktopState`], because the
/// coordinator belongs to the running application and not to the device: the
/// host tests build a state without a loop behind it, and a command that
/// notified a coordinator that does not exist would be a command that only
/// works in the shipped build.
#[derive(Debug, Default)]
pub struct Wake(tokio::sync::Notify);

impl Wake {
    /// Ask for an immediate pass. Fire-and-forget and idempotent: several
    /// calls between two passes produce one.
    pub fn now(&self) {
        self.0.notify_one();
    }

    async fn notified(&self) {
        self.0.notified().await;
    }
}

/// Whether replacing the page the window is showing leaves it in history.
///
/// A value rather than a `cfg` for the reason `ConnectSurface` is one: both
/// arms compile on every target, so a host test can ask for the phone's
/// without being on a phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartUpNav {
    /// Leave it behind, which is what `navigate` does everywhere.
    Push,
    /// Replace it. Android alone, because it is the only surface where
    /// anything reads the history: `MainActivity` turns wry's back handling
    /// on, so back walks the webview's history, and the bundled start page
    /// would otherwise sit one entry behind the console's first screen — a
    /// seller pressing back at the console root would meet a page they never
    /// asked for instead of leaving the app.
    Replace,
}

/// Which of the two this build does.
pub(crate) const fn start_up_nav() -> StartUpNav {
    if cfg!(target_os = "android") {
        StartUpNav::Replace
    } else {
        StartUpNav::Push
    }
}

/// The script that goes somewhere without leaving here in history.
///
/// `location.replace` because Tauri exposes `navigate`, which always pushes,
/// and no replacing form. The address is written as a JSON string rather than
/// interpolated: it comes from `TAM_CONTROL_PLANE` or the compiled constant
/// rather than from any page, but a quote or a backslash in it would otherwise
/// end the literal, and the page this runs in is ours.
pub(crate) fn replacing(url: &str) -> String {
    format!(
        "location.replace({})",
        serde_json::Value::String(url.to_owned())
    )
}

/// The console's canonical home on `origin`.
///
/// `/resources` rather than `/app`: `/app` is a client-side redirect, so using
/// it makes a cold Android WebView download and start the console only to
/// navigate again. Going straight to the catalogue leaves one document load
/// between the bundled connection page and usable UI. The path is joined
/// rather than written so an override with a trailing slash and one without
/// both resolve to the same address.
pub(crate) fn console_home(origin: &str) -> Result<tauri::Url, String> {
    let mut url = tauri::Url::parse(origin).map_err(|why| why.to_string())?;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    url.join("resources").map_err(|why| why.to_string())
}

/// Put the window on `url`, leaving the page it was showing in history or not.
pub(crate) fn show_console<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    url: tauri::Url,
    how: StartUpNav,
) -> Result<(), String> {
    match how {
        StartUpNav::Push => window.navigate(url).map_err(|why| why.to_string()),
        StartUpNav::Replace => window
            .eval(replacing(url.as_str()))
            .map_err(|why| why.to_string()),
    }
}

/// The retry command's body, here rather than in `commands.rs` because the
/// opener and the origin both live in this module.
pub(crate) async fn retry_console_from<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    let origin = control_plane::base_url();
    let Some(window) = app.get_webview_window("main") else {
        return Err("this build has no console window".to_owned());
    };
    let state = app.state::<DesktopState>();
    state
        .control_plane()
        .reachable()
        .await
        .map_err(|why| why.to_string())?;
    let url = console_home(&origin)?;
    show_console(&window, url, start_up_nav())
}

#[cfg(test)]
mod start_up_tests {
    use super::{console_home, replacing, show_console, StartUpNav};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    /// The bundled page the window is showing before the console is reached.
    const START: &str = "http://tauri.localhost/";

    fn a_window(
        app: &tauri::App<tauri::test::MockRuntime>,
        label: &str,
    ) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
        WebviewWindowBuilder::new(
            app,
            label,
            WebviewUrl::External(START.parse().expect("the start page is a url")),
        )
        .build()
        .expect("the window builds")
    }

    /// The address is a JSON string rather than an interpolation.
    ///
    /// It comes from `TAM_CONTROL_PLANE` or the compiled constant rather than
    /// from a page, so this is not a defence against a marketplace; it is a
    /// defence against a developer's override with a quote in it silently
    /// ending the literal and leaving a syntax error that fails as a window
    /// which never leaves the start page.
    #[test]
    fn the_address_cannot_end_the_string_it_travels_in() {
        assert_eq!(
            replacing("https://teachouse.io"),
            r#"location.replace("https://teachouse.io")"#
        );
        // Read back rather than pattern-matched: the property is that the
        // argument is one JSON string carrying exactly the address, which is
        // the same thing as saying nothing in the address escaped it.
        let awkward = r#"https://x/");alert("1"#;
        let script = replacing(awkward);
        let argument = script
            .strip_prefix("location.replace(")
            .and_then(|rest| rest.strip_suffix(')'))
            .expect("the script calls location.replace with one argument");
        let read_back: String =
            serde_json::from_str(argument).expect("the argument is one JSON string");
        assert_eq!(
            read_back, awkward,
            "a quote in the address must survive as part of the address rather than ending the \
             literal and starting a statement. Script was: {script}"
        );
    }

    /// The window opens on the console's canonical home, not on the origin or
    /// its client-side redirect.
    ///
    /// The origin is where the landing page answers; `/app` would load the
    /// console once merely to redirect to `/resources`.
    #[test]
    fn the_window_opens_on_the_console_home() {
        for origin in [
            "https://teachouse.io",
            "https://teachouse.io/",
            "http://127.0.0.1:8080",
        ] {
            let home = console_home(origin).expect("the origin is a url");
            assert_eq!(home.path(), "/resources", "{origin}");
            assert_eq!(
                home.host_str(),
                tauri::Url::parse(origin).unwrap().host_str()
            );
        }
        assert!(console_home("not a url").is_err());
    }

    /// A computer leaves the start page in history and a phone does not.
    ///
    /// The whole point, and it is asserted through the window's own address
    /// rather than through the script, because that is what a back press
    /// reads: `navigate` moves the address and pushes an entry, while the
    /// replacing arm moves the page from inside it and leaves no entry for
    /// back to find. On a phone the difference is a seller pressing back at
    /// the console root meeting the bundled start page instead of leaving the
    /// app, which is the one thing turning back handling on made possible.
    #[test]
    fn only_the_computer_leaves_the_start_page_behind() {
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        let console: tauri::Url = "https://teachouse.io/"
            .parse()
            .expect("the console origin is a url");

        let pushed = a_window(&app, "pushed");
        show_console(&pushed, console.clone(), StartUpNav::Push).expect("a computer navigates");
        assert_eq!(
            pushed.url().expect("the window has a url").as_str(),
            console.as_str(),
            "a computer navigates, which is what leaves the start page one entry behind"
        );

        let replaced = a_window(&app, "replaced");
        show_console(&replaced, console, StartUpNav::Replace).expect("a phone replaces");
        assert_eq!(
            replaced.url().expect("the window has a url").as_str(),
            START,
            "a phone must not navigate: `navigate` is the call that pushes, so reaching the \
             console has to happen from inside the page instead"
        );
        drop(app);
    }

    /// The surface is chosen by the platform rather than by a caller.
    #[test]
    fn a_phone_replaces_and_everything_else_pushes() {
        assert_eq!(
            super::start_up_nav(),
            if cfg!(target_os = "android") {
                StartUpNav::Replace
            } else {
                StartUpNav::Push
            }
        );
    }
}
