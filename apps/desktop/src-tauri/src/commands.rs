//! The commands the console calls, and the login webview behind the first of
//! them.
//!
//! One rule governs the login window: the marketplace page gets no capability
//! at all. It is not listed in `capabilities/default.json`, so `invoke` is
//! unreachable from it even before the site's own content-security policy
//! blocks `ipc.localhost`. Everything this module learns about the login it
//! learns by reading the webview's cookie store from Rust.

use core::time::Duration;

use serde::Serialize;
use tam_types::Marketplace;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::connect::login_target;
use crate::heartbeat::{check_in, first_run, CheckInError};
use crate::run::wall_now;
use crate::session::{Cookie, CookieJar, SessionRecord, SessionStatus};
use crate::state::{DesktopState, DeviceActivity, WorkEvent};

/// How often the login window's cookie store is read while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How long a login may take before the window is abandoned. Generous: a
/// seller may have to fetch a second factor from another device.
const LOGIN_DEADLINE: Duration = Duration::from_mins(10);

/// Every command failure, as one string the interface can show.
///
/// Deliberately opaque: the underlying errors are keychain and webview
/// diagnostics, and a jar must never be interpolated into one.
#[derive(Debug, Clone, Serialize)]
pub struct CommandError(pub String);

impl From<crate::connect::NotSellerDevice> for CommandError {
    fn from(why: crate::connect::NotSellerDevice) -> Self {
        Self(why.to_string())
    }
}

impl From<crate::session::StoreError> for CommandError {
    fn from(why: crate::session::StoreError) -> Self {
        Self(why.to_string())
    }
}

impl From<tauri::Error> for CommandError {
    fn from(why: tauri::Error) -> Self {
        Self(why.to_string())
    }
}

impl From<CheckInError> for CommandError {
    fn from(why: CheckInError) -> Self {
        Self(why.to_string())
    }
}

/// What this device's standing with the server is, as the interface reads it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DeviceState {
    /// The seller signed this device out from the console. Sessions have been
    /// forgotten and no work will run.
    pub revoked: bool,
    /// The last check-in reached the server. False means we do not know our
    /// standing rather than that we are in good standing, which is why the
    /// interface must not read `revoked: false` alone as permission.
    pub reached_server: bool,
    /// Somebody is signed in to the console on this device, so there is a
    /// session to speak under. False is the ordinary state of a machine at a
    /// sign-in screen, and is a different fact from either of the two above.
    pub signed_in: bool,
}

/// Opens the marketplace's own login page in a window on this device, waits
/// for the session to appear in that window's cookie store, and files it in
/// the keychain.
///
/// Refuses outright for a marketplace with an official API: that branch's
/// automation runs server-side under a sanctioned token and never logs in
/// here.
#[tauri::command]
pub async fn connect_marketplace(
    app: AppHandle,
    marketplace: Marketplace,
) -> Result<SessionStatus, CommandError> {
    let target = login_target(marketplace)?;
    let label = format!("login-{marketplace:?}");

    if let Some(existing) = app.get_webview_window(&label) {
        existing.set_focus().ok();
        return Err(CommandError(
            "a login window for this marketplace is already open".to_owned(),
        ));
    }

    let url = tauri::Url::parse(target.login_url).map_err(|why| CommandError(why.to_string()))?;
    let origin =
        tauri::Url::parse(target.cookie_origin).map_err(|why| CommandError(why.to_string()))?;
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title(format!("Sign in to {marketplace:?}"))
        .inner_size(1_040.0, 800.0)
        .build()?;

    let deadline = tokio::time::Instant::now() + LOGIN_DEADLINE;
    let jar = loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        if app.get_webview_window(&label).is_none() {
            return Err(CommandError(
                "the login window was closed before the sign-in completed".to_owned(),
            ));
        }
        // Read from Rust rather than from the page: `document.cookie` cannot
        // see the HttpOnly session cookie, which is the only one that matters.
        let jar = read_jar(&window, &origin)?;
        if target.is_logged_in(&jar) {
            break jar;
        }
        if tokio::time::Instant::now() >= deadline {
            window.destroy().ok();
            return Err(CommandError(
                "the sign-in did not complete before the window's deadline".to_owned(),
            ));
        }
    };

    window.destroy().ok();

    let state = app.state::<DesktopState>();
    let record = SessionRecord {
        marketplace,
        // No label. The only value that names the account is the marketplace's
        // own identity read, and that is a marketplace request, which this
        // slice makes none of.
        account_label: None,
        captured_at: wall_now(),
        device_id: state.device().id.clone(),
        jar,
    };
    state.store().put(&record).await?;

    // D14: a device the seller has already signed out must not keep a session
    // it just captured, so the check-in that would learn of it happens before
    // the capture is reported as a success. A check-in that could not reach the
    // server is not evidence of revocation and leaves the capture standing.
    if let Ok(answer) = check_in(&state, state.control_plane()).await {
        if answer.revoked {
            return Err(CommandError(
                "this device has been signed out from the console, so the marketplace \
                 session was not kept"
                    .to_owned(),
            ));
        }
    }
    Ok(SessionStatus::of(&record))
}

/// Registers this device with the server's registry and checks in.
///
/// The console calls it when it loads, which is what first run means for a
/// client whose interface is the console. Registration is idempotent on the
/// server, so calling it again is a refresh rather than a second machine.
#[tauri::command]
pub async fn device_check_in(app: AppHandle) -> Result<DeviceState, CommandError> {
    let state = app.state::<DesktopState>();
    match first_run(&state, state.control_plane()).await {
        Ok(answer) => Ok(DeviceState {
            revoked: answer.revoked,
            reached_server: true,
            signed_in: true,
        }),
        Err(CheckInError::Plane(_)) => Ok(DeviceState {
            revoked: state.revoked(),
            reached_server: false,
            signed_in: state.signed_in(),
        }),
        Err(why) => Err(CommandError::from(why)),
    }
}

/// What this device has been doing, oldest first.
///
/// Every entry names the device that produced it. With the work running on the
/// seller's own machines, "which device did this" is a question the seller can
/// now ask, and D1's interface rule is that they must be able to answer it.
#[tauri::command]
pub async fn device_activity(app: AppHandle) -> Result<Vec<DeviceActivity>, CommandError> {
    Ok(app.state::<DesktopState>().activity().await)
}

/// Whether this device holds a session for a marketplace, and nothing about
/// what is in it.
#[tauri::command]
pub async fn session_status(
    app: AppHandle,
    marketplace: Marketplace,
) -> Result<SessionStatus, CommandError> {
    let found = app.state::<DesktopState>().store().get(marketplace).await?;
    Ok(found.as_ref().map_or_else(
        || SessionStatus::disconnected(marketplace),
        SessionStatus::of,
    ))
}

/// Removes the stored session. The seller's disconnect, and the only way a
/// captured jar leaves this device's keychain.
#[tauri::command]
pub async fn forget_session(
    app: AppHandle,
    marketplace: Marketplace,
) -> Result<SessionStatus, CommandError> {
    app.state::<DesktopState>()
        .store()
        .forget(marketplace)
        .await?;
    Ok(SessionStatus::disconnected(marketplace))
}

fn read_jar(window: &tauri::WebviewWindow, origin: &tauri::Url) -> Result<CookieJar, CommandError> {
    let cookies = window.cookies_for_url(origin.clone())?;
    Ok(CookieJar::new(
        cookies
            .into_iter()
            .map(|cookie| Cookie {
                name: cookie.name().to_owned(),
                value: cookie.value().to_owned(),
            })
            .collect(),
    ))
}

/// What starting an import answered.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportStarted {
    /// Echoed so the console can match the answer to the request it asked
    /// about, rather than assuming the only start in flight is its own.
    pub request: String,
    /// How many resources the seller's shop holds, known because the
    /// enumeration happens before this answer rather than after it. The console
    /// can show a total from the first moment instead of a count with no
    /// denominator.
    pub described: u32,
}

/// Starts the catalogue import for one request, in the background.
///
/// Background rather than awaited, because a shop of several hundred resources
/// is minutes of work and the seller navigates away: the command answers as
/// soon as the pass is running, and the console watches the request's own view
/// for progress. The task therefore outlives this call by design.
///
/// Every refusal is named, and each is checked here rather than left to the
/// pass, because a refusal the seller sees immediately is one they can act on
/// while a refusal that surfaces from a background task is one they have to
/// discover. The pass re-checks the entitlement and the revocation itself,
/// between resources, which is a different guarantee: this stops a run that
/// should not start, and that stops a run that should not continue.
///
/// One consequence of holding the running set in memory, stated rather than
/// removed: a device that restarts mid-pass forgets what was running, so a
/// second start re-enumerates the shop. That is survivable rather than
/// wasteful in the way it looks — the route's breadcrumb identity is the
/// marketplace resource id within the request, so every resource an earlier
/// pass posted is recognised and becomes a no-op, and only what had not been
/// described is described again.
///
/// Generic over the runtime where its neighbours are not, so the mock runtime
/// can invoke it by name. That is the only assertion that this command is
/// registered at all — the console's own test can compare its constant to a
/// copy of itself and nothing more — and it is worth one type parameter.
#[tauri::command]
pub async fn start_import<R: tauri::Runtime>(
    app: AppHandle<R>,
    request: tam_types::Uuid,
) -> Result<ImportStarted, CommandError> {
    let state = app.state::<DesktopState>();
    let Some(ledger) = state.ledger() else {
        return Err(CommandError(
            "this build has no way to reach the server, so an import would have nowhere to post what it read".to_owned(),
        ));
    };
    // Which shop, read from the request itself. The console asks by request id
    // alone, so a console that named the inventory could ask this device to
    // enumerate a shop the request does not name; and the server answers this
    // under the organisation's own session, so another tenant's request is
    // absent here rather than readable.
    let source = state
        .control_plane()
        .sync_request_source(request)
        .await
        .map_err(|why| CommandError(why.to_string()))?;
    let marketplace = source.marketplace();
    // Named here rather than discovered as an adapter error mid-pass, the same
    // way `SellerFiles::fetch` names it for a download. Tpt's own-catalogue
    // read is uncaptured and Etsy's automation is server-side under a
    // sanctioned token, so neither is a shop this device enumerates.
    if marketplace != Marketplace::Tes {
        return Err(CommandError(format!(
            "this device cannot read a {marketplace:?} catalogue: no capture exists for it, and a marketplace with an official API is read on our own servers rather than here"
        )));
    }
    if state.store().get(marketplace).await?.is_none() {
        return Err(CommandError(format!(
            "this device is not signed in to {marketplace:?}, so it cannot read your shop. Connect it on this device and start the import again"
        )));
    }
    let now = wall_now();
    if !state.gate_handle().lock().await.may_work(marketplace, now) {
        return Err(CommandError(
            "your subscription does not currently allow work to run on this device, so the import was not started".to_owned(),
        ));
    }
    if !state.claim_import(request).await {
        return Err(CommandError(
            "an import for this request is already running on this device".to_owned(),
        ));
    }

    let pass = crate::import::ImportPass::new(
        state.device().id.clone(),
        crate::work::SellerCatalogue::new(state.store_handle(), source),
        ledger,
        request,
        crate::import::SourcePermission {
            marketplace,
            gate: state.gate_handle(),
            stopper: state.stopper(),
        },
    );
    // Enumerated here, while the seller is still looking. Everything that can
    // fail before a single page is posted fails in this call — the entitlement,
    // a revocation, and the catalogue read itself — and each is a sentence the
    // seller can act on. Answering "started" and enumerating in the background
    // would leave those failures with nowhere to go: the request's own view
    // holds nothing until a page lands, so the console would sit on its
    // pre-start state indefinitely with the seller told nothing at all.
    let catalogue = match pass.enumerate(now).await {
        Ok(catalogue) => catalogue,
        Err(why) => {
            state.release_import(request).await;
            return Err(CommandError(why.to_string()));
        }
    };
    let described = u32::try_from(catalogue.len()).unwrap_or(u32::MAX);

    let handle = app.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        // What the pass posts is the request's, and the console reads it from
        // the request's own view; a second copy of that here would drift. What
        // is NOT the request's is a terminal failure that posted nothing — a
        // first page that could not be sent, a sign-out mid-pass — and in
        // exactly those cases the request's view holds nothing until the
        // report below puts the reason in it.
        let outcome = pass.describe_all(catalogue, wall_now, |_progress| {}).await;
        let state = handle.state::<DesktopState>();
        if let Err(why) = outcome {
            // To the REQUEST, because that is the page the seller is looking
            // at: a terminal failure that posted nothing leaves the request
            // holding exactly nothing, so without this the console watches a
            // state that never changes.
            //
            // The activity record beside it is read by nothing today, and that
            // is stated rather than implied: `record` appends to an in-memory
            // ring buffer, `device_activity` returns it, and no console screen
            // calls that command — `settings/devices` is a redirect stub. It is
            // kept because a devices screen is the place a seller looks when
            // they do not know WHICH request went wrong, and the record has to
            // exist before that screen can read it. Until it does, the request
            // page above is the only place this failure is visible.
            pass.report_failure(&why).await;
            state
                .record(
                    marketplace,
                    wall_now(),
                    WorkEvent::Abandoned {
                        item: uuid::Uuid::from_bytes(request.0)
                            .as_hyphenated()
                            .to_string(),
                        reason: why.to_string(),
                    },
                )
                .await;
        }
        // Released on both endings. A pass that ended by an error and left its
        // claim standing would refuse every later attempt for this request
        // until the application restarted, which is a worse failure than the
        // one that caused it.
        state.release_import(request).await;
    });
    Ok(ImportStarted {
        request: uuid::Uuid::from_bytes(request.0)
            .as_hyphenated()
            .to_string(),
        described,
    })
}

#[cfg(test)]
mod import_command_tests {
    use crate::device::{DeviceId, DeviceIdentity};
    use crate::session::memory::MemorySessionStore;
    use crate::state::DesktopState;
    use std::sync::Arc;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Manager;

    /// The application answers `start_import`, and answers an unregistered
    /// name differently.
    ///
    /// This is r-c7's M1 closed on the desktop side. The console sends the
    /// string `start_import`; its own test could only compare that constant to
    /// a copy of itself, and nothing checked that the application answers it.
    /// Before C5b nothing did, and a seller pressing the button read
    /// `Command start_import not found` rendered as though it were a considered
    /// refusal.
    ///
    /// Two things had to exist before this test could tell the difference, and
    /// both are worth stating because the first attempt at it could not. The
    /// application manifest in `build.rs`, without which Tauri's access control
    /// resolves no permission for an application command and refuses every name
    /// identically — measured: before the manifest, `start_import` and a name
    /// nothing registers both answered `not allowed. Plugin not found`. And the
    /// real generated context rather than a mock one, because a mock context
    /// carries no capabilities and so denies everything for the same reason.
    ///
    /// The webview is built at the control plane's own origin, which is where
    /// the seller's console actually runs, so this exercises the grant as well
    /// as the registration: at any other origin the capability does not apply
    /// and the answer is a permission refusal naming the origins that are
    /// allowed.
    ///
    /// It exercises the ROOT path only. That the grant also covers
    /// `/sync/requests/<id>` rests on `RemoteUrlPattern::from_str` rewriting an
    /// empty pathname to `*` (tauri-utils 2.9.3, src/acl/mod.rs:291-298),
    /// which is present in the pinned version and worth naming: without it the
    /// pattern would match `/` alone, every console route would be refused, and
    /// nothing here would have caught it.
    #[test]
    fn the_application_answers_the_command_the_console_sends() {
        use tauri::test::{get_ipc_response, mock_builder, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        // `generate_context!` expands to code the exit lint sees; it is the
        // real context rather than a mock one deliberately, because a mock
        // carries no capabilities and would deny every command identically,
        // which is the whole thing this test exists to distinguish.
        #[expect(
            clippy::exit,
            reason = "the generated context's own expansion, not a call this test makes"
        )]
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::start_import])
            .build(tauri::generate_context!())
            .expect("the application builds");
        // Managed, so a dispatched command answers rather than panicking on
        // unmanaged state. This build has no ledger transport, so the answer is
        // the named refusal — which is the proof: an unregistered or ungranted
        // command never reaches the body that produces it.
        app.manage(DesktopState::new(
            DeviceIdentity {
                id: DeviceId::from_raw("11112222333344445555666677778888"),
                label: "a test machine".to_owned(),
            },
            Arc::new(MemorySessionStore::default()),
        ));
        let origin: tauri::Url = crate::control_plane::DEFAULT_BASE_URL
            .parse()
            .expect("the compiled origin is a url");
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::External(origin.clone()))
            .build()
            .expect("the console window builds");
        let ask = |cmd: &str| {
            get_ipc_response(
                &webview,
                InvokeRequest {
                    cmd: cmd.to_owned(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: origin.clone(),
                    body: serde_json::json!({
                        "request": "71717171-7171-7171-7171-717171717171"
                    })
                    .into(),
                    headers: tauri::http::HeaderMap::default(),
                    invoke_key: INVOKE_KEY.to_owned(),
                },
            )
        };

        let mine = format!("{:?}", ask(crate::import::START_IMPORT_COMMAND).err());
        assert!(
            mine.contains("no way to reach the server"),
            "the console's string must reach the handler and come back with the handler's own \
             refusal: neither unregistered nor ungranted at the origin the console runs at. \
             Got: {mine}"
        );

        let bogus = format!("{:?}", ask("a_command_nothing_registers").err());
        assert!(
            bogus.contains("Command not found"),
            "and a name nothing registers must still be refused as one, or the assertion \
             above would pass for a command that does not exist. Got: {bogus}"
        );
        drop(app);
    }

    /// The command refuses, by name, when this build cannot post a page.
    ///
    /// What this does NOT prove is that the application registers the command,
    /// and that absence is deliberate rather than an oversight. Tauri's access
    /// control runs before dispatch, so in a mock application with no
    /// capability an unregistered name and a registered one answer
    /// identically: both give `not allowed. Plugin not found`. That was
    /// measured rather than assumed — a probe invoked `start_import` and
    /// `a_command_nothing_registers` through the same harness and compared the
    /// two strings. So an IPC test here would assert a property it does not
    /// have, which is the defect this suite exists to avoid rather than to
    /// commit. The registration assertion is the test above, which arrived with
    /// the app manifest and the capability in this same change; what this one
    /// keeps is its own narrower job, pinning the refusal's wording.
    #[tokio::test]
    async fn the_command_refuses_a_build_that_cannot_post_a_page() {
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::start_import])
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        app.manage(DesktopState::new(
            DeviceIdentity {
                id: DeviceId::from_raw("11112222333344445555666677778888"),
                label: "a test machine".to_owned(),
            },
            Arc::new(MemorySessionStore::default()),
        ));

        let refusal = super::start_import(app.handle().clone(), tam_types::Uuid([0x71; 16]))
            .await
            .expect_err("a build with no ledger transport cannot import");
        assert!(
            refusal.0.contains("no way to reach the server"),
            "the refusal names what is missing rather than failing in the background, because a \
             seller can act on the first and can only discover the second. Got: {}",
            refusal.0
        );
        drop(app);
    }
}

/// Re-probes the control plane and opens the console if it answers.
///
/// The fallback page's one button. It exists so a seller whose network came
/// back does not have to restart the application to find out, and it is a
/// command rather than a page reload because the probe and the navigation are
/// the application's to do — the fallback page is bundled and has no origin to
/// reach anything from.
#[tauri::command]
pub async fn retry_console<R: tauri::Runtime>(app: AppHandle<R>) -> Result<(), CommandError> {
    crate::retry_console_from(&app).await.map_err(CommandError)
}
