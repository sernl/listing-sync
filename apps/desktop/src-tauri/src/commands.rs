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

use crate::connect::{login_target, return_url, ConnectVerdict, LoginTarget};
use crate::heartbeat::{check_in, first_run, CheckInError};
use crate::run::wall_now;
use crate::session::{Cookie, CookieJar, SessionRecord, SessionStatus};
use crate::state::{DesktopState, DeviceActivity, WorkEvent};
use crate::webview_session::CONSOLE_WINDOW;

/// How often the login window's cookie store is read while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How long a login may take before the window is abandoned. Generous: a
/// seller may have to fetch a second factor from another device.
const LOGIN_DEADLINE: Duration = Duration::from_mins(10);

/// How long the marketplace's own page has to replace the console before the
/// attempt is given up on. Generous, because a phone on mobile data is slow to
/// commit a first load, and bounded, because a navigation the platform dropped
/// would otherwise wait out the whole login deadline saying nothing.
const SIGN_IN_ARRIVAL: Duration = Duration::from_secs(30);

/// How long the phone's navigation waits before replacing the console.
///
/// Tauri delivers a command's answer by evaluating a callback in whatever page
/// the webview is showing, and offers no hook for when that has happened. On
/// the one-window surface the navigation would otherwise race it and run our
/// callback inside the marketplace's own page — the single thing this module's
/// fence exists to prevent. A quarter of a second is far longer than an eval
/// on the same thread and far shorter than a seller notices.
const CONSOLE_HANDOVER: Duration = Duration::from_millis(250);

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
#[derive(Debug, Clone, Serialize)]
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
    /// Why the check-in did not reach the server, in the words
    /// [`crate::heartbeat::ControlPlaneError`] already writes for a person, and
    /// `None` when it did reach.
    ///
    /// The four sentences name no credential, no jar and no host but our own
    /// control plane, and they are the whole difference between "nothing
    /// appeared in the list" and a cause somebody can act on: no transport in
    /// this build, the plane refused, this device is not registered, nobody is
    /// signed in here. Carried rather than discarded because a phone has no
    /// other channel — its log is private storage and its stdout needs a cable.
    pub detail: Option<String>,
}

/// What a connect did, which is not the same question on the two surfaces.
///
/// On a computer the login runs in a second window, this call waits for it,
/// and the answer is the session. On a phone there is one window and it is
/// about to become the marketplace's own page, so the page that asked is gone
/// before there is anything to answer: this call says only that the sign-in is
/// opening, and the verdict comes back in the address the console is resumed
/// at. Two variants rather than an optional field, because "no session yet"
/// and "no session" are different facts and a nullable one would collapse
/// them.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ConnectOutcome {
    /// The login completed in a window of its own and the session is filed.
    Captured { session: SessionStatus },
    /// The sign-in is replacing this page. Nothing is filed yet, and the
    /// verdict arrives as `crate::connect::RETURN_PARAM` on the way back.
    Opening,
}

/// Which shape a login takes here, decided by the surface rather than by the
/// marketplace.
///
/// A value rather than a `cfg`, and that is what makes the phone's arm
/// testable on a host: both bodies compile everywhere and only the selection
/// is platform-dependent, so a test can ask for the one-window arm on a
/// developer's machine instead of on a handset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectSurface {
    /// A second window this application owns, which leaves the console
    /// standing behind it. Every desktop platform.
    SecondWindow,
    /// The one window, navigated to the marketplace and navigated back.
    ///
    /// Android has a single Activity, and tao's `Window::new` takes the next
    /// Android context with no window created, so a second window answers
    /// `OsError::NoAvailableActivity` (tao 0.35.3,
    /// `src/platform_impl/android/mod.rs`) rather than opening. Reading the jar
    /// is unaffected: `CookieManager` is process-global on Android, so the same
    /// store answers whichever webview asks.
    OneWindow,
}

/// The surface this build runs on.
#[must_use]
pub const fn connect_surface() -> ConnectSurface {
    if cfg!(target_os = "android") {
        ConnectSurface::OneWindow
    } else {
        ConnectSurface::SecondWindow
    }
}

/// Opens the marketplace's own login page on this device, waits for the
/// session to appear in the webview's cookie store, and files it in this
/// platform's session store.
///
/// Refuses outright for a marketplace with an official API: that branch's
/// automation runs server-side under a sanctioned token and never logs in
/// here.
///
/// Generic over the runtime for the reason [`forget_session`] is: the mock
/// runtime a host test builds cannot hand an `AppHandle<Wry>` to a command
/// that names one, and the one-window arm is a body that has never run on a
/// handset in this tree.
#[tauri::command]
pub async fn connect_marketplace<R: tauri::Runtime>(
    app: AppHandle<R>,
    marketplace: Marketplace,
) -> Result<ConnectOutcome, CommandError> {
    connect_on(app, marketplace, connect_surface()).await
}

/// The command's body with the surface handed to it.
async fn connect_on<R: tauri::Runtime>(
    app: AppHandle<R>,
    marketplace: Marketplace,
    surface: ConnectSurface,
) -> Result<ConnectOutcome, CommandError> {
    connect_on_target(app, login_target(marketplace)?, surface).await
}

/// The dispatch, with the marketplace already resolved to a login page.
///
/// Split from [`connect_on`] so a host test can name a target this crate does
/// not export: the two real ones are marketplace sign-in pages, and a test
/// that reached one would be making the request D1 says only a seller's own
/// device makes, on a machine that is not a seller's.
pub(crate) async fn connect_on_target<R: tauri::Runtime>(
    app: AppHandle<R>,
    target: LoginTarget,
    surface: ConnectSurface,
) -> Result<ConnectOutcome, CommandError> {
    match surface {
        ConnectSurface::SecondWindow => in_a_second_window(app, target)
            .await
            .map(|session| ConnectOutcome::Captured { session }),
        ConnectSurface::OneWindow => in_this_window(&app, target).map(|()| ConnectOutcome::Opening),
    }
}

/// The marketplace's name as a seller writes it, for a window a seller reads.
/// The `Debug` spelling is the Rust identifier and puts "Tpt" in a title bar.
const fn display_name(marketplace: Marketplace) -> &'static str {
    match marketplace {
        Marketplace::Tes => "Tes",
        Marketplace::Etsy => "Etsy",
        Marketplace::Tpt => "TPT",
    }
}

#[cfg(test)]
mod display_name_tests {
    use super::display_name;
    use tam_types::Marketplace;

    #[test]
    fn display_name_titles_a_sign_in_window_the_way_a_seller_writes_it() {
        let titles: Vec<String> = Marketplace::ALL
            .iter()
            .map(|&marketplace| format!("Sign in to {}", display_name(marketplace)))
            .collect();
        assert_eq!(
            titles,
            ["Sign in to Tes", "Sign in to Etsy", "Sign in to TPT"],
        );
    }
}

/// The computer's login: a window of its own, awaited by the caller.
async fn in_a_second_window<R: tauri::Runtime>(
    app: AppHandle<R>,
    target: LoginTarget,
) -> Result<SessionStatus, CommandError> {
    let marketplace = target.marketplace;
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
        .title(format!("Sign in to {}", display_name(marketplace)))
        .inner_size(1_040.0, 800.0)
        .build()?;

    let watched = app.clone();
    let closed = label.clone();
    let mut capture = Capture::of(&window, &origin);
    let jar = match await_session(&target, &mut capture, move || {
        watched.get_webview_window(&closed).is_none()
    })
    .await
    {
        Ok(jar) => jar,
        Err(Waited::Left) => {
            return Err(CommandError(
                "the login window was closed before the sign-in completed".to_owned(),
            ))
        }
        Err(Waited::Deadline) => {
            window.destroy().ok();
            return Err(CommandError(
                "the sign-in did not complete before the window's deadline".to_owned(),
            ));
        }
        // The webview's own diagnostic, unaltered. It is what a seller saw
        // before the two surfaces were split and it names the failure; a
        // verdict code in its place would say "refused" and nothing else.
        Err(Waited::Unreadable(why)) => return Err(why),
    };

    window.destroy().ok();
    file_session(&app, marketplace, jar).await
}

/// The phone's login: the console's own window, navigated away and navigated
/// back with the verdict.
///
/// Answers before the navigation rather than after it, and the whole shape
/// follows from that: an invoke promise cannot survive the unload of the page
/// holding it, so nothing this returns can carry a result, and the capture
/// runs on the runtime with no caller waiting on it.
fn in_this_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
    target: LoginTarget,
) -> Result<(), CommandError> {
    let Some(window) = app.get_webview_window(CONSOLE_WINDOW) else {
        return Err(CommandError(
            "this application has no console window to sign in from".to_owned(),
        ));
    };
    let origin =
        tauri::Url::parse(target.cookie_origin).map_err(|why| CommandError(why.to_string()))?;
    let capture = Capture::of(&window, &origin);
    capture_here(app, target, window, capture)
}

/// The same login with the capture handed to it, which is the seam a host test
/// enters through: the mock runtime's cookie store is empty by construction, so
/// a capture and a deadline are unreachable without substituting the read and
/// shortening the wait.
fn capture_here<R: tauri::Runtime>(
    app: &AppHandle<R>,
    target: LoginTarget,
    window: tauri::WebviewWindow<R>,
    mut capture: Capture,
) -> Result<(), CommandError> {
    let login = tauri::Url::parse(target.login_url).map_err(|why| CommandError(why.to_string()))?;
    let base = crate::control_plane::base_url();
    let console = tauri::Url::parse(&base).map_err(|why| CommandError(why.to_string()))?;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CONSOLE_HANDOVER).await;
        let verdict = if window.navigate(login).is_ok() {
            capture_in_place(&app, &window, &target, &console, &mut capture).await
        } else {
            ConnectVerdict::Refused
        };
        if let Ok(back) = tauri::Url::parse(&return_url(&base, target.marketplace, verdict)) {
            window.navigate(back).ok();
        }
    });
    Ok(())
}

/// Wait out the phone's sign-in and file whatever it produced, as one verdict.
async fn capture_in_place<R: tauri::Runtime>(
    app: &AppHandle<R>,
    window: &tauri::WebviewWindow<R>,
    target: &LoginTarget,
    console: &tauri::Url,
    capture: &mut Capture,
) -> ConnectVerdict {
    // A phone's abandon is the back gesture, which walks the webview's history
    // and so lands it back at our own origin rather than closing anything —
    // `MainActivity.kt` turns that handling on, against the default Tauri's
    // generated activity sets. So "at our own origin" cannot mean abandoned
    // until the sign-in has actually replaced us.
    // Waiting for that here rather than carrying a flag through the poll is
    // what makes the check exact: `navigate` is a message to the platform's
    // main thread and the address only changes when the load commits, so a
    // poll that ran first would read the console's own address and report the
    // sign-in abandoned half a second after the seller asked for it.
    if !sign_in_showing(window, console).await {
        return ConnectVerdict::Refused;
    }
    let console = console.clone();
    let watched = window.clone();
    let gone = move || {
        watched
            .url()
            .is_ok_and(|at| at.origin() == console.origin())
    };
    match await_session(target, capture, gone).await {
        Ok(jar) => {
            if file_session(app, target.marketplace, jar).await.is_ok() {
                ConnectVerdict::Captured
            } else {
                // Not `Refused`: the sign-in opened and the seller finished it.
                // The cause the seller can act on is a device signed out from
                // the console, which `file_session`'s own check-in learns of
                // and which wipes the store — and telling them the sign-in
                // could not be opened would name the one thing that did happen.
                ConnectVerdict::NotKept
            }
        }
        Err(Waited::Left) => ConnectVerdict::Abandoned,
        Err(Waited::Deadline) => ConnectVerdict::Deadline,
        // The diagnostic has nowhere to go on this surface: there is no caller
        // left to hand a sentence to, and the return leg carries a verdict
        // rather than prose. `NotKept` rather than `Refused` for the same
        // reason as above — the sign-in was showing when the read failed, so
        // nothing was saved and nothing failed to open.
        Err(Waited::Unreadable(_)) => ConnectVerdict::NotKept,
    }
}

/// Whether the marketplace's page replaced the console within
/// [`SIGN_IN_ARRIVAL`].
///
/// False is a sign-in that never opened at all, which is a different thing
/// from one the seller did not complete and is told as such: a navigation the
/// platform dropped, or a page that could not begin to load.
///
/// Polled at [`POLL_INTERVAL`] rather than faster, and the reason is the cost
/// of the question rather than the value of the answer: on Android
/// `WebviewWindow::url` is a message to the platform's main thread and a
/// blocking wait on its reply, aimed at the one thread that is at this moment
/// committing a first remote page load. The only consumer of this is a loop
/// that then polls at that interval anyway.
async fn sign_in_showing<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    console: &tauri::Url,
) -> bool {
    let deadline = tokio::time::Instant::now() + SIGN_IN_ARRIVAL;
    loop {
        if window.url().is_ok_and(|at| at.origin() != console.origin()) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// One reading of a webview's cookie store.
///
/// Read from Rust rather than from the page: `document.cookie` cannot see the
/// HttpOnly session cookie, which is the only one that matters.
type ReadJar = Box<dyn FnMut() -> Result<CookieJar, CommandError> + Send>;

/// What a login capture reads, and how long it is given to read it.
///
/// The reader is a value rather than a call to [`read_jar`] in place, and that
/// is what puts the success path under test at all:
/// `tauri::test::MockRuntime::cookies_for_url` answers every read with an empty
/// jar, so on the mock runtime `LoginTarget::is_logged_in` is false forever and
/// neither a capture nor a deadline can be reached. Without the seam the one
/// outcome the whole surface exists to produce would ship with no test at any
/// level.
struct Capture {
    poll: Duration,
    deadline: Duration,
    read: ReadJar,
}

impl Capture {
    /// The capture a seller's sign-in gets: this window's own cookie store, on
    /// the intervals both surfaces share.
    fn of<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>, origin: &tauri::Url) -> Self {
        let window = window.clone();
        let origin = origin.clone();
        Self {
            poll: POLL_INTERVAL,
            deadline: LOGIN_DEADLINE,
            read: Box::new(move || read_jar(&window, &origin)),
        }
    }
}

/// Read the jar every [`Capture::poll`] until the marketplace's own session is
/// in it, and answer why not when it never is.
///
/// The one wait both surfaces take, so the logged-in condition, the interval
/// and the deadline cannot come to differ between a computer and a phone.
/// `left` is the surface's own way of saying the seller has gone: a closed
/// window on one, a return to our own origin on the other.
async fn await_session(
    target: &LoginTarget,
    capture: &mut Capture,
    mut left: impl FnMut() -> bool + Send,
) -> Result<CookieJar, Waited> {
    let deadline = tokio::time::Instant::now() + capture.deadline;
    loop {
        tokio::time::sleep(capture.poll).await;
        if left() {
            return Err(Waited::Left);
        }
        let jar = (capture.read)().map_err(Waited::Unreadable)?;
        if target.is_logged_in(&jar) {
            return Ok(jar);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Waited::Deadline);
        }
    }
}

/// Why a wait ended without a session.
///
/// Not [`ConnectVerdict`] itself, and the difference is the third arm: a jar
/// that could not be read carries the webview's own diagnostic, which a
/// computer shows the seller verbatim and a phone has nowhere to put. Mapping
/// to a verdict here would throw that sentence away for both.
enum Waited {
    /// The seller went: a window closed on one surface, a return to our own
    /// origin on the other.
    Left,
    Deadline,
    Unreadable(CommandError),
}

/// File a captured jar, and tell the server before calling it a success.
async fn file_session<R: tauri::Runtime>(
    app: &AppHandle<R>,
    marketplace: Marketplace,
    jar: CookieJar,
) -> Result<SessionStatus, CommandError> {
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
            detail: None,
        }),
        Err(CheckInError::Plane(why)) => Ok(DeviceState {
            revoked: state.revoked(),
            reached_server: false,
            signed_in: state.signed_in(),
            detail: Some(why.to_string()),
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
///
/// Generic over the runtime for the reason [`start_import`] is: the mock
/// runtime a host test builds cannot hand an `AppHandle<Wry>` to a command
/// that names one, and this command's own body — a forget followed by a
/// check-in — is what those tests exist to pin.
#[tauri::command]
pub async fn forget_session<R: tauri::Runtime>(
    app: AppHandle<R>,
    marketplace: Marketplace,
) -> Result<SessionStatus, CommandError> {
    let state = app.state::<DesktopState>();
    state.store().forget(marketplace).await?;
    // The mirror of `connect_marketplace` above, and for the same reason read
    // the other way round: the server's per-device session list is replaced
    // only by a check-in, so between a forget and the next scheduled one the
    // registry goes on listing a login for a jar that no longer exists. That
    // window is an hour on a computer and until the next resume on a phone,
    // and the seller is looking at both answers on one screen.
    //
    // `check_in` rather than the self-healing `check_in_or_register`: a device
    // the server has never seen has no session row to correct.
    //
    // Discarded rather than raised, the same judgement `cycle` makes: the jar
    // is already gone, and a check-in that could not reach the server is not a
    // reason to report the disconnect as failed.
    check_in(&state, state.control_plane()).await.ok();
    Ok(SessionStatus::disconnected(marketplace))
}

fn read_jar<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
    origin: &tauri::Url,
) -> Result<CookieJar, CommandError> {
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
    /// Echoed so the console can match the answer to the run it asked about,
    /// rather than assuming the only start in flight is its own.
    pub run: String,
    /// How many resources the seller's shop holds, known because the
    /// enumeration happens before this answer rather than after it. The
    /// console can show a total from the first moment instead of a count with
    /// no denominator.
    pub listed: u32,
}

/// What continuing an import answered.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportContinued {
    pub run: String,
    /// How many resources the seller ticked, read back from the run rather
    /// than counted by the console.
    pub selected: u32,
}

/// Everything a pass needs, checked before one is built.
///
/// Every refusal is named, and each is checked here rather than left to the
/// pass, because a refusal the seller sees immediately is one they can act on
/// while a refusal that surfaces from a background task is one they have to
/// discover. The pass re-checks the entitlement and the revocation itself,
/// between resources, which is a different guarantee: this stops a run that
/// should not start, and that stops a run that should not continue.
pub(crate) struct ImportReady<P: crate::ledger::LedgerTransport + ?Sized> {
    pub(crate) pass: crate::import::ImportPass<Box<dyn crate::import::CatalogueSource>, P>,
    pub(crate) marketplace: Marketplace,
    pub(crate) now: tam_types::Timestamp,
}

/// The catalogue reader for a shop, chosen by which marketplace it is.
///
/// Named here rather than discovered as an adapter error mid-pass, the same
/// way `SellerFiles::fetch` names it for a download. A marketplace with an
/// official API is read on our own servers under a sanctioned token, so it is
/// not a shop this device enumerates.
pub(crate) fn catalogue_for(
    state: &DesktopState,
    source: tam_types::InventoryId,
) -> Result<Box<dyn crate::import::CatalogueSource>, String> {
    match source.marketplace() {
        Marketplace::Tes => Ok(Box::new(crate::work::SellerCatalogue::new(
            state.store_handle(),
            source,
        ))),
        Marketplace::Tpt => Ok(Box::new(crate::work::TptSellerCatalogue::new(
            state.store_handle(),
            source,
        ))),
        other @ Marketplace::Etsy => Err(format!(
            "this device cannot read a {other:?} catalogue: a marketplace with an official API is read on our own servers rather than here"
        )),
    }
}

/// Checks every gate, claims the single flight, and builds the pass.
///
/// Shared by both halves of the flow because both make marketplace requests
/// under the same rules: a seller whose subscription lapsed between ticking
/// and continuing must be refused at the second press as firmly as at the
/// first. Shared with [`crate::import::serve_open_run`] for the same reason,
/// which is also why the single-flight claim is taken here rather than by each
/// caller: a scheduled pass and a console press must not walk one shop twice.
///
/// `source` is the run's own inventory, read from the run by every caller
/// rather than taken from the console: a console that named the shop could
/// otherwise ask this device to enumerate one the run does not name.
pub(crate) async fn ready_to_import(
    state: &DesktopState,
    run: tam_types::Uuid,
    source: tam_types::InventoryId,
    catalogue: crate::import::CatalogueFactory<'_>,
) -> Result<ImportReady<dyn crate::ledger::LedgerTransport>, CommandError> {
    let ledger = require_ledger(state)?;
    let marketplace = source.marketplace();
    let catalogue = catalogue(state, source).map_err(CommandError)?;
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
    if !state.claim_import(run).await {
        return Err(CommandError(
            "an import for this run is already running on this device".to_owned(),
        ));
    }
    Ok(ImportReady {
        pass: crate::import::ImportPass::new(
            state.device().id.clone(),
            catalogue,
            ledger,
            run,
            crate::import::SourcePermission {
                marketplace,
                gate: state.gate_handle(),
                stopper: state.stopper(),
            },
        ),
        marketplace,
        now,
    })
}

/// The transport an import posts its pages over, or the refusal a build with
/// none owes the seller.
fn require_ledger(
    state: &DesktopState,
) -> Result<std::sync::Arc<dyn crate::ledger::LedgerTransport>, CommandError> {
    state.ledger().ok_or_else(|| {
        CommandError(
            "this build has no way to reach the server, so an import would have nowhere to post what it read".to_owned(),
        )
    })
}

/// The run's source inventory, read from the run itself.
///
/// The console asks by run id alone, so a console that named the inventory
/// could ask this device to enumerate a shop the run does not name; and the
/// server answers this under the organisation's own session, so another
/// tenant's run is absent here rather than readable.
async fn source_of(
    state: &DesktopState,
    run: tam_types::Uuid,
) -> Result<tam_types::InventoryId, CommandError> {
    // Before the run is read rather than after it, so a build that could post
    // nothing says that rather than reporting the transport failure its own
    // absence produced. The order the seller reads the refusals in is the
    // order they can act on them.
    require_ledger(state)?;
    state
        .control_plane()
        .import_run_source(run)
        .await
        .map_err(|why| CommandError(why.to_string()))
}

/// Reads the seller's shop and posts what is in it, so they can choose.
///
/// Awaited rather than backgrounded, and that is the difference from the pass
/// that follows: enumerating is one walk the seller is watching, everything
/// that can fail before a page lands fails in this call, and the answer is
/// what makes the selection step renderable at all. Answering "started" and
/// enumerating in the background would leave those failures with nowhere to
/// go — the run's own view holds nothing until a page lands, so the console
/// would sit on its pre-start state with the seller told nothing.
#[tauri::command]
pub async fn start_import<R: tauri::Runtime>(
    app: AppHandle<R>,
    run: tam_types::Uuid,
) -> Result<ImportStarted, CommandError> {
    let state = app.state::<DesktopState>();
    let source = source_of(state.inner(), run).await?;
    let ready = ready_to_import(state.inner(), run, source, &catalogue_for).await?;
    let listed = match ready.pass.enumerate(ready.now).await {
        Ok(listed) => listed,
        Err(why) => {
            state.release_import(run).await;
            return Err(CommandError(why.to_string()));
        }
    };
    let counted = u32::try_from(listed.len()).unwrap_or(u32::MAX);
    if let Err(why) = ready.pass.post_listing(listed).await {
        state.release_import(run).await;
        return Err(CommandError(why.to_string()));
    }
    // Released here rather than held across the seller's decision: the
    // describe pass claims it again, and holding it through a step that may
    // take the seller a week would refuse every retry until the application
    // restarted.
    state.release_import(run).await;
    Ok(ImportStarted {
        run: uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string(),
        listed: counted,
    })
}

/// Describes what the seller ticked, in the background.
///
/// Background rather than awaited, because a shop of several hundred
/// resources is minutes of work and the seller navigates away: the command
/// answers as soon as the pass is running, and the console watches the run's
/// own view for progress. The task therefore outlives this call by design.
///
/// One consequence of holding the running set in memory, stated rather than
/// removed: a device that restarts mid-pass forgets what was running, so a
/// second continue re-describes the selection. That is survivable rather than
/// wasteful in the way it looks — the route's breadcrumb identity is the
/// marketplace resource id within the run, so every resource an earlier pass
/// posted is recognised and becomes a no-op, and only what had not been
/// described is described again.
///
/// Generic over the runtime where its neighbours are not, so the mock runtime
/// can invoke it by name. That is the only assertion that this command is
/// registered at all — the console's own test can compare its constant to a
/// copy of itself and nothing more — and it is worth one type parameter.
#[tauri::command]
pub async fn continue_import<R: tauri::Runtime>(
    app: AppHandle<R>,
    run: tam_types::Uuid,
) -> Result<ImportContinued, CommandError> {
    let state = app.state::<DesktopState>();
    let source = source_of(state.inner(), run).await?;
    let ready = ready_to_import(state.inner(), run, source, &catalogue_for).await?;
    let marketplace = ready.marketplace;
    let pass = ready.pass;
    // The run's own record of what was ticked, not a list the browser carried
    // over: a seller who chose, closed the window and came back continues the
    // import they chose.
    let selection = match state
        .control_plane()
        .import_selection(&state.device().id, run)
        .await
    {
        Ok(selection) => selection,
        Err(why) => {
            state.release_import(run).await;
            return Err(CommandError(why.to_string()));
        }
    };
    let chosen: Vec<i64> = selection
        .iter()
        .filter_map(|locator| locator.parse().ok())
        .collect();
    let counted = u32::try_from(chosen.len()).unwrap_or(u32::MAX);

    let handle = app.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        // What the pass posts is the run's, and the console reads it from the
        // run's own view; a second copy of that here would drift. What is NOT
        // the run's is a terminal failure that posted nothing — a first page
        // that could not be sent, a sign-out mid-pass — and in exactly those
        // cases the run's view holds nothing until the report below puts the
        // reason in it.
        let outcome = pass.describe_all(chosen, wall_now, |_progress| {}).await;
        let state = handle.state::<DesktopState>();
        if let Err(why) = outcome {
            // To the RUN, because that is the page the seller is looking at:
            // a terminal failure that posted nothing leaves the run holding
            // exactly nothing, so without this the console watches a state
            // that never changes.
            //
            // The activity record beside it is read by nothing today, and
            // that is stated rather than implied: `record` appends to an
            // in-memory ring buffer, `device_activity` returns it, and no
            // console screen calls that command — `settings/devices` is a
            // redirect stub. It is kept because a devices screen is the place
            // a seller looks when they do not know WHICH import went wrong,
            // and the record has to exist before that screen can read it.
            pass.report_failure(&why).await;
            state
                .record(
                    marketplace,
                    wall_now(),
                    WorkEvent::Abandoned {
                        item: uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string(),
                        reason: why.to_string(),
                    },
                )
                .await;
        }
        // Released on both endings. A pass that ended by an error and left
        // its claim standing would refuse every later attempt for this run
        // until the application restarted, which is a worse failure than the
        // one that caused it.
        state.release_import(run).await;
    });
    Ok(ImportContinued {
        run: uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string(),
        selected: counted,
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
            .invoke_handler(tauri::generate_handler![
                super::start_import,
                super::continue_import
            ])
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
                        "run": "71717171-7171-7171-7171-717171717171"
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

        let second = format!("{:?}", ask(crate::import::CONTINUE_IMPORT_COMMAND).err());
        assert!(
            second.contains("no way to reach the server"),
            "and so must the second half of the same flow, which is a second command name the \
             manifest and the capability both have to carry. Got: {second}"
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

#[cfg(test)]
mod session_command_tests {
    use super::forget_session;
    use crate::device::{DeviceId, DeviceIdentity};
    use crate::heartbeat::{
        CheckIn, ControlPlane, ControlPlaneError, HostFacts, PlaneFuture, SessionReport,
    };
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::DesktopState;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tam_types::{Marketplace, Timestamp};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Manager;

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::from_raw("11112222333344445555666677778888"),
            label: "founder-pc".to_owned(),
        }
    }

    fn a_record(marketplace: Marketplace) -> SessionRecord {
        SessionRecord {
            marketplace,
            account_label: None,
            captured_at: Timestamp(1_756_000_000_000),
            device_id: identity().id,
            jar: CookieJar::new(vec![Cookie {
                name: "sessionKey".to_owned(),
                value: "s3cr3t".to_owned(),
            }]),
        }
    }

    /// What the device told the server, and how often.
    ///
    /// Every heartbeat is kept rather than only the last, because the property
    /// under test is that a forget produces exactly one and that the one it
    /// produces names what the store holds AFTER the forget. A fake keeping
    /// only the last would pass for an implementation that checked in first.
    struct Recorder {
        answer: Result<CheckIn, ControlPlaneError>,
        beats: tokio::sync::Mutex<Vec<Vec<SessionReport>>>,
        registrations: AtomicUsize,
    }

    impl Recorder {
        fn answering(answer: Result<CheckIn, ControlPlaneError>) -> Arc<Self> {
            Arc::new(Self {
                answer,
                beats: tokio::sync::Mutex::new(Vec::new()),
                registrations: AtomicUsize::new(0),
            })
        }

        fn allowing() -> Arc<Self> {
            Self::answering(Ok(CheckIn {
                revoked: false,
                entitlement: None,
            }))
        }

        async fn beats(&self) -> Vec<Vec<SessionReport>> {
            self.beats.lock().await.clone()
        }
    }

    impl ControlPlane for Recorder {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_run_source(
            &self,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a crate::device::DeviceId,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'a, Vec<String>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn open_import_run<'a>(
            &'a self,
            _device: &'a crate::device::DeviceId,
        ) -> PlaneFuture<'a, Option<crate::import::OpenImportRun>> {
            Box::pin(core::future::ready(Ok(None)))
        }

        fn register<'a>(
            &'a self,
            _device: &'a DeviceIdentity,
            _facts: HostFacts,
        ) -> PlaneFuture<'a, ()> {
            self.registrations.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            sessions: &'a [SessionReport],
        ) -> PlaneFuture<'a, CheckIn> {
            let answer = self.answer.clone();
            Box::pin(async move {
                self.beats.lock().await.push(sessions.to_vec());
                answer
            })
        }
    }

    /// An application carrying the state these commands read, on the mock
    /// runtime. No capability and no real context: the grant is not what these
    /// tests are about, and `import_command_tests` above holds the one
    /// assertion that the console's strings reach a registered command.
    fn app_holding(
        store: Arc<MemorySessionStore>,
        plane: Arc<Recorder>,
    ) -> tauri::App<tauri::test::MockRuntime> {
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        app.manage(DesktopState::with_control_plane(identity(), store, plane));
        app
    }

    fn marketplaces_in(beat: &[SessionReport]) -> Vec<Marketplace> {
        beat.iter().map(|line| line.marketplace).collect()
    }

    /// The application answers the disconnect the console sends, at the origin
    /// the console runs at.
    ///
    /// Here because this command's signature changed: it became generic over
    /// the runtime so a host test could hand it a mock one, and
    /// `generate_handler!` expands a generic command differently from a
    /// concrete one. Nothing in the type system says the expansion still
    /// registers under the name the console sends, and a registration that
    /// silently stopped would reach a seller as `Command forget_session not
    /// found` rendered as though it were a considered refusal — which is the
    /// failure `import_command_tests` above exists to prevent for the other
    /// generic command.
    ///
    /// The real generated context and the control plane's own origin, for the
    /// reasons that test states at length: a mock context carries no
    /// capabilities and would refuse every name identically.
    #[test]
    fn the_application_answers_the_disconnect_the_console_sends() {
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        #[expect(
            clippy::exit,
            reason = "the generated context's own expansion, not a call this test makes"
        )]
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::forget_session])
            .build(tauri::generate_context!())
            .expect("the application builds");
        app.manage(DesktopState::new(
            identity(),
            Arc::new(MemorySessionStore::default()),
        ));
        let origin: tauri::Url = crate::control_plane::DEFAULT_BASE_URL
            .parse()
            .expect("the compiled origin is a url");
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::External(origin.clone()))
            .build()
            .expect("the console window builds");

        let answer = get_ipc_response(
            &webview,
            InvokeRequest {
                // The console's own string, spelled here as it spells it: the
                // name is what the registration and the grant are keyed on.
                cmd: "forget_session".to_owned(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: origin,
                body: serde_json::json!({ "marketplace": "Tes" }).into(),
                headers: tauri::http::HeaderMap::default(),
                invoke_key: INVOKE_KEY.to_owned(),
            },
        );

        let said = format!("{answer:?}");
        assert!(
            answer.is_ok(),
            "the console's own string must reach the handler and come back with the handler's \
             own answer: neither unregistered nor ungranted at the origin the console runs at. \
             Got: {said}"
        );
        assert!(
            said.contains("Tes") && said.contains("false"),
            "and the answer is this command's own — the marketplace it was asked about, \
             disconnected. Got: {said}"
        );
        drop(app);
    }

    /// The defect this whole change exists for: the server's per-device
    /// session list is replaced only by a check-in, so a forget that told it
    /// nothing left the registry listing a login for a jar that no longer
    /// exists — for an hour on a computer, and until the next resume on a
    /// phone.
    #[tokio::test]
    async fn a_forget_tells_the_server_what_this_device_now_holds() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tes))
            .await
            .expect("tes stores");
        store
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("tpt stores");
        let plane = Recorder::allowing();
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));

        forget_session(app.handle().clone(), Marketplace::Tes)
            .await
            .expect("the forget answers");

        let beats = plane.beats().await;
        assert_eq!(
            beats.len(),
            1,
            "exactly one check-in: none at all leaves the stale row standing, and the count is \
             what distinguishes that from this. Got: {beats:?}"
        );
        assert_eq!(
            marketplaces_in(&beats[0]),
            vec![Marketplace::Tpt],
            "and it reports the store as it stands AFTER the forget, so a check-in placed before \
             it — which would report Tes as still held and re-derive the link straight back to \
             linked — fails here. Got: {beats:?}"
        );
        drop(app);
    }

    /// The jar is already gone by the time the server is told, so an
    /// unreachable server is not a reason to report the disconnect as failed.
    /// The obvious wrong implementation is `?` on the check-in.
    #[tokio::test]
    async fn a_forget_stands_when_the_server_cannot_be_told() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tes))
            .await
            .expect("tes stores");
        let plane = Recorder::answering(Err(ControlPlaneError::Refused("no route".to_owned())));
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));

        let answer = forget_session(app.handle().clone(), Marketplace::Tes)
            .await
            .expect("a forget the server could not be told of is still a forget");
        assert_eq!(answer.marketplace, Marketplace::Tes);
        assert!(
            !answer.connected,
            "the jar is gone whatever the server heard"
        );
        assert!(
            store
                .get(Marketplace::Tes)
                .await
                .expect("the store reads")
                .is_none(),
            "and it is gone from the store, which is the only copy there is"
        );
        drop(app);
    }

    /// A device signed out from the console while a disconnect is in flight
    /// learns of it through this very check-in, and `check_in` wipes before it
    /// returns. The disconnect the seller asked for still succeeded.
    #[tokio::test]
    async fn a_revoked_answer_during_a_forget_is_not_a_failed_disconnect() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&a_record(Marketplace::Tes))
            .await
            .expect("tes stores");
        store
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("tpt stores");
        let plane = Recorder::answering(Ok(CheckIn {
            revoked: true,
            entitlement: None,
        }));
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));

        forget_session(app.handle().clone(), Marketplace::Tes)
            .await
            .expect("a revocation learned of mid-disconnect does not fail the disconnect");

        for marketplace in Marketplace::ALL {
            assert!(
                store
                    .get(marketplace)
                    .await
                    .expect("the store reads")
                    .is_none(),
                "and the revocation wiped every marketplace, {marketplace:?} included"
            );
        }
        drop(app);
    }

    /// A stub sign-in the mock runtime never loads and the host never fetches.
    ///
    /// A `LoginTarget` this crate does not export, so no test reaches a
    /// marketplace: the discard port answers nothing by definition, and the
    /// mock webview records a navigation rather than performing one.
    fn a_stub_target() -> crate::connect::LoginTarget {
        crate::connect::LoginTarget {
            marketplace: Marketplace::Tpt,
            login_url: "http://127.0.0.1:9/stub-sign-in",
            cookie_origin: "http://127.0.0.1:9",
            required: &["stubSession"],
            required_any: &[],
        }
    }

    /// The console's own window, at the origin the console is served from.
    fn a_console_window(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
        let origin: tauri::Url = crate::control_plane::DEFAULT_BASE_URL
            .parse()
            .expect("the compiled origin is a url");
        tauri::WebviewWindowBuilder::new(
            app,
            super::CONSOLE_WINDOW,
            tauri::WebviewUrl::External(origin),
        )
        .build()
        .expect("the console window builds")
    }

    /// Wait for the one window to arrive somewhere, or say where it stopped.
    async fn settles_at(
        window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
        wanted: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = tokio::time::Instant::now() + core::time::Duration::from_secs(20);
        loop {
            let at = window.url().map(|url| url.to_string()).unwrap_or_default();
            if wanted(&at) {
                return at;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the window never got there; it is at {at}"
            );
            tokio::time::sleep(core::time::Duration::from_millis(50)).await;
        }
    }

    /// The phone's arm answers before it navigates, and files nothing.
    ///
    /// Both halves matter and they are the same half of D1 read twice. The
    /// answer has to precede the navigation because Tauri delivers it by
    /// evaluating a callback in whatever page the webview is showing, and the
    /// page it would otherwise show is the marketplace's — the one page this
    /// module's fence exists to keep our own code out of. And nothing may be
    /// filed at this point because no sign-in has happened yet: a store
    /// written here would be a session claimed on the strength of a button
    /// press.
    #[tokio::test]
    async fn the_one_window_arm_opens_the_sign_in_and_files_nothing() {
        let store = Arc::new(MemorySessionStore::new());
        let app = app_holding(Arc::clone(&store), Recorder::allowing());
        let window = a_console_window(&app);

        let answer = super::connect_on_target(
            app.handle().clone(),
            a_stub_target(),
            super::ConnectSurface::OneWindow,
        )
        .await
        .expect("the phone's arm opens rather than refusing");
        assert!(
            matches!(answer, super::ConnectOutcome::Opening),
            "a phone answers that the sign-in is opening; the verdict cannot come back this \
             way, because the page holding the promise is about to be unloaded"
        );
        assert_eq!(
            window.url().expect("the window has a url").as_str(),
            "https://teachouse.stowiq.io/",
            "and it has not navigated yet, so the answer above is evaluated in the console's \
             own page rather than in the marketplace's"
        );

        let arrived = settles_at(&window, |at| at.starts_with("http://127.0.0.1:9/")).await;
        assert_eq!(arrived, "http://127.0.0.1:9/stub-sign-in");
        for marketplace in Marketplace::ALL {
            assert!(
                store
                    .get(marketplace)
                    .await
                    .expect("the store reads")
                    .is_none(),
                "nothing is filed for {marketplace:?} by opening a sign-in"
            );
        }
        drop(app);
    }

    /// A sign-in the seller walks away from returns the console with the
    /// verdict in the address.
    ///
    /// The whole return leg in one test, and it is the leg that has no
    /// counterpart on a computer: there the seller closes a window and the
    /// command that opened it is still there to answer. Here the caller is
    /// gone, the seller's back gesture walks the webview's history onto our own
    /// origin, and the only thing left to tell them with is the address it is
    /// navigated to next. Without this a phone would come back to the console
    /// in silence on every outcome but success.
    ///
    /// The gesture reaching our origin at all is `MainActivity.kt`'s doing and
    /// is not provable here: the mock runtime has no back gesture, so this
    /// stands in for it by navigating. What that file's one line buys is
    /// recorded in `docs/notes/design/android-client.md` and checked on a
    /// handset by `docs/notes/runbooks/android-phone-check.md`.
    #[tokio::test]
    async fn a_sign_in_the_seller_leaves_returns_the_console_with_the_verdict() {
        let store = Arc::new(MemorySessionStore::new());
        let app = app_holding(Arc::clone(&store), Recorder::allowing());
        let window = a_console_window(&app);
        let console: tauri::Url = crate::control_plane::DEFAULT_BASE_URL
            .parse()
            .expect("the compiled origin is a url");

        super::connect_on_target(
            app.handle().clone(),
            a_stub_target(),
            super::ConnectSurface::OneWindow,
        )
        .await
        .expect("the phone's arm opens");

        settles_at(&window, |at| at.starts_with("http://127.0.0.1:9/")).await;
        // Long enough for the arrival check to have seen the sign-in showing.
        // Without it this test could navigate back inside that window and prove
        // the arrival timeout rather than the abandon it is about.
        tokio::time::sleep(core::time::Duration::from_millis(700)).await;
        // The back gesture: the one webview is at our origin again, with no
        // window having closed and nothing having been signed in to.
        window.navigate(console).expect("the window navigates back");

        let back = settles_at(&window, |at| at.contains("connect=")).await;
        assert_eq!(
            back, "https://teachouse.stowiq.io/marketplaces?connect=abandoned&marketplace=Tpt",
            "the seller is returned to the page they pressed Connect on, and it is told which \
             marketplace ended how"
        );
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_none(),
            "and an abandoned sign-in files nothing"
        );
        drop(app);
    }

    /// A capture is not a success until the server has been told, and a
    /// revoked device keeps nothing.
    ///
    /// D14, on the half both surfaces now share. It matters more on a phone
    /// than on a computer: the phone's capture runs with no caller waiting on
    /// it, so this check is the only thing standing between a device the
    /// seller signed out and a marketplace session sealed on it.
    #[tokio::test]
    async fn a_capture_on_a_revoked_device_is_refused_after_the_check_in() {
        let store = Arc::new(MemorySessionStore::new());
        let plane = Recorder::answering(Ok(CheckIn {
            revoked: true,
            entitlement: None,
        }));
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));

        let refusal = super::file_session(
            app.handle(),
            Marketplace::Tpt,
            CookieJar::new(vec![Cookie {
                name: "sessionKey".to_owned(),
                value: "s3cr3t".to_owned(),
            }]),
        )
        .await
        .expect_err("a device signed out from the console does not keep what it just captured");
        assert!(
            refusal.0.contains("signed out from the console"),
            "and the seller is told which of the two things went wrong. Got: {}",
            refusal.0
        );
        assert_eq!(
            plane.beats().await.len(),
            1,
            "the check-in happened, which is what makes the refusal above evidence rather than \
             a guess"
        );
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_none(),
            "and the wipe the revocation triggers took the capture with it"
        );
        drop(app);
    }

    /// The application answers `connect_marketplace` at the origin the console
    /// runs at, and refuses the sanctioned branch by name.
    ///
    /// Here for the reason the disconnect test above is here: this command's
    /// signature changed, from a concrete runtime to a generic one and from
    /// `SessionStatus` to `ConnectOutcome`, and `generate_handler!` expands a
    /// generic command differently. A registration that silently stopped would
    /// reach a seller as `Command connect_marketplace not found` rendered as a
    /// considered refusal.
    ///
    /// Etsy rather than a device-branch marketplace deliberately: it is the one
    /// argument that reaches the body and comes back without touching a
    /// webview, so this proves the name, the grant and the two-branch rule at
    /// once and opens nothing.
    #[test]
    fn the_application_answers_the_connect_the_console_sends() {
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        #[expect(
            clippy::exit,
            reason = "the generated context's own expansion, not a call this test makes"
        )]
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::connect_marketplace])
            .build(tauri::generate_context!())
            .expect("the application builds");
        app.manage(DesktopState::new(
            identity(),
            Arc::new(MemorySessionStore::default()),
        ));
        let origin: tauri::Url = crate::control_plane::DEFAULT_BASE_URL
            .parse()
            .expect("the compiled origin is a url");
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::External(origin.clone()))
            .build()
            .expect("the console window builds");

        let refusal = format!(
            "{:?}",
            get_ipc_response(
                &webview,
                InvokeRequest {
                    cmd: "connect_marketplace".to_owned(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: origin,
                    body: serde_json::json!({ "marketplace": "Etsy" }).into(),
                    headers: tauri::http::HeaderMap::default(),
                    invoke_key: INVOKE_KEY.to_owned(),
                },
            )
            .err()
        );
        assert!(
            refusal.contains("publishes an official API"),
            "the console's string must reach the handler and come back with the two-branch \
             rule's own refusal: neither unregistered nor ungranted at the origin the console \
             runs at. Got: {refusal}"
        );
        drop(app);
    }

    /// A capture with the jar substituted and the wait shortened.
    ///
    /// `tauri::test::MockRuntime::cookies_for_url` answers every read with an
    /// empty jar, so without this the two verdicts that matter most are
    /// unreachable on a host: `is_logged_in` is false forever, so a capture
    /// never happens, and a deadline is ten minutes away.
    fn a_capture(
        poll: core::time::Duration,
        deadline: core::time::Duration,
        read: impl FnMut() -> Result<CookieJar, super::CommandError> + Send + 'static,
    ) -> super::Capture {
        super::Capture {
            poll,
            deadline,
            read: Box::new(read),
        }
    }

    /// The jar a completed stub sign-in leaves behind.
    fn a_signed_in_jar() -> CookieJar {
        CookieJar::new(vec![Cookie {
            name: "stubSession".to_owned(),
            value: "s3cr3t".to_owned(),
        }])
    }

    /// A sign-in that completes is filed, reported to the server, and the
    /// console is returned saying so.
    ///
    /// The success path, which is the entire point of the surface and which no
    /// test reached before: the mock runtime's cookie store is empty by
    /// construction, so `Captured` was produced by nothing at any level and the
    /// first seller to press Connect on a phone would have been the first
    /// execution of it.
    ///
    /// Three assertions rather than one, because three different wrong
    /// implementations reach the same address. One that answers `Captured`
    /// without writing the store leaves a phone claiming a login it does not
    /// hold, and the store assertion catches it. One that reports success
    /// before the check-in — dropping the D14 guard the desktop arm has — files
    /// a session on a device the seller may have signed out, and the single
    /// heartbeat catches it. One that maps the outcome to any other verdict, or
    /// never navigates back at all, leaves the seller on the marketplace's page
    /// or reading a failure, and the address catches both.
    #[tokio::test]
    async fn a_sign_in_that_completes_is_filed_and_the_console_is_told() {
        let store = Arc::new(MemorySessionStore::new());
        let plane = Recorder::allowing();
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));
        let window = a_console_window(&app);

        super::capture_here(
            app.handle(),
            a_stub_target(),
            window.clone(),
            a_capture(
                core::time::Duration::from_millis(50),
                core::time::Duration::from_secs(30),
                || Ok(a_signed_in_jar()),
            ),
        )
        .expect("the phone's arm opens");

        let back = settles_at(&window, |at| at.contains("connect=")).await;
        assert_eq!(
            back, "https://teachouse.stowiq.io/marketplaces?connect=captured&marketplace=Tpt",
            "a completed sign-in returns the console to the page it left, saying which \
             marketplace was connected"
        );
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_some(),
            "and the session it captured is on the device, or the sentence above is a claim \
             about nothing"
        );
        assert_eq!(
            plane.beats().await.len(),
            1,
            "and the server was told before the capture was called a success, which is the \
             only thing standing between a device the seller signed out and a marketplace \
             session sealed on it"
        );
        drop(app);
    }

    /// A sign-in the seller never completes ends at the deadline, files
    /// nothing, and says which of the failures it was.
    ///
    /// The other verdict the mock runtime's empty jar made unreachable, and it
    /// is reachable here only because the wait is a value: ten minutes is not a
    /// test. Three wrong implementations it catches. One that maps a deadline
    /// to `abandoned` or `refused` tells a seller who waited too long either
    /// that they walked away or that the page never opened, and the address
    /// catches it. One whose poll never notices the deadline — reading the
    /// production constant rather than the wait it was given, say — never
    /// returns the console at all, and the twenty-second settle catches it. One
    /// that files whatever the last read produced writes an empty jar as a
    /// session, and the store catches it.
    #[tokio::test]
    async fn a_sign_in_that_runs_out_of_time_files_nothing_and_says_so() {
        let store = Arc::new(MemorySessionStore::new());
        let plane = Recorder::allowing();
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));
        let window = a_console_window(&app);

        super::capture_here(
            app.handle(),
            a_stub_target(),
            window.clone(),
            a_capture(
                core::time::Duration::from_millis(20),
                core::time::Duration::from_millis(100),
                || Ok(CookieJar::new(Vec::new())),
            ),
        )
        .expect("the phone's arm opens");

        let back = settles_at(&window, |at| at.contains("connect=")).await;
        assert_eq!(
            back, "https://teachouse.stowiq.io/marketplaces?connect=deadline&marketplace=Tpt",
            "the seller is returned to the console and told the sign-in ran out of time, which \
             is a different thing from leaving it and a different thing from it never opening"
        );
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_none(),
            "and a sign-in that never produced a session files none"
        );
        assert!(
            plane.beats().await.is_empty(),
            "and nothing is reported to the server, because nothing was captured"
        );
        drop(app);
    }

    /// A page at a marketplace's own origin reaches none of our commands, in
    /// the one window every capability names.
    ///
    /// The fence this surface creates, and the one it has that a computer does
    /// not need. On a computer the marketplace page sits in a window labelled
    /// `login-<Marketplace>`, absent from every capability, and that absence
    /// refuses it. On a phone it sits in window `main` — the label every
    /// capability here names — and the only thing left refusing it is the
    /// per-invoke remote-origin check against the one origin `console.json`
    /// grants.
    ///
    /// Nothing tested that check. `the_capability_grants_the_origin_this_build_uses`
    /// parses the JSON, and the two tests above invoke from the console's own
    /// origin, so both pass unchanged if a second entry is added to
    /// `remote.urls`, if the pattern is widened to a wildcard host, or if
    /// `default.json` gains a `remote` block. Any of those hands a marketplace
    /// page `start_import` and the seller's catalogue.
    ///
    /// Asserted as a pair rather than on the refusal alone: each command has a
    /// refusal of its own that only the body produces, so a call that reached
    /// the body is distinguishable here from one the ACL stopped.
    #[test]
    fn a_marketplace_page_in_the_console_window_reaches_no_command() {
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        use tauri::{WebviewUrl, WebviewWindowBuilder};

        #[expect(
            clippy::exit,
            reason = "the generated context's own expansion, not a call this test makes"
        )]
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![
                super::connect_marketplace,
                super::start_import
            ])
            .build(tauri::generate_context!())
            .expect("the application builds");
        app.manage(DesktopState::new(
            identity(),
            Arc::new(MemorySessionStore::default()),
        ));
        let origin: tauri::Url = crate::control_plane::DEFAULT_BASE_URL
            .parse()
            .expect("the compiled origin is a url");
        let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::External(origin))
            .build()
            .expect("the console window builds");
        // Where a marketplace sign-in leaves this window on a phone. Not a
        // real host: `InvokeRequest.url` is what the ACL matches on, and on
        // Android it is tracked in Kotlin at `onPageStarted` rather than
        // supplied by the page, so a page cannot claim a different one.
        let marketplace_page: tauri::Url = "https://www.example.invalid/login"
            .parse()
            .expect("the marketplace page is a url");

        let ask = |cmd: &str, body: serde_json::Value| {
            format!(
                "{:?}",
                get_ipc_response(
                    &webview,
                    InvokeRequest {
                        cmd: cmd.to_owned(),
                        callback: tauri::ipc::CallbackFn(0),
                        error: tauri::ipc::CallbackFn(1),
                        url: marketplace_page.clone(),
                        body: body.into(),
                        headers: tauri::http::HeaderMap::default(),
                        invoke_key: INVOKE_KEY.to_owned(),
                    },
                )
                .err()
            )
        };

        let connect = ask(
            "connect_marketplace",
            serde_json::json!({ "marketplace": "Etsy" }),
        );
        assert!(
            connect.contains("not allowed"),
            "a page at a marketplace's origin must be refused `connect_marketplace` outright. \
             Got: {connect}"
        );
        assert!(
            !connect.contains("publishes an official API"),
            "and refused before the body, or the refusal above is the two-branch rule speaking \
             and not the fence. Got: {connect}"
        );

        let import = ask(
            crate::import::START_IMPORT_COMMAND,
            serde_json::json!({ "request": "71717171-7171-7171-7171-717171717171" }),
        );
        assert!(
            import.contains("not allowed"),
            "and `start_import`, which is the one that would hand it the seller's catalogue. \
             Got: {import}"
        );
        assert!(
            !import.contains("no way to reach the server"),
            "and refused before the body, or this build's missing ledger transport is what \
             stopped it rather than the origin. Got: {import}"
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

/// Moves the window's own chrome onto the palette the console is drawn in.
///
/// The webview paints the page; the title bar, the frame and the platform
/// scrollbars are the window's, and without this a seller who picks Dark gets
/// a dark console inside a light window. `None` hands the window back to the
/// system, which is what the console's System choice means.
///
/// `tauri.conf.json` names no `theme` for the window, deliberately: unset is
/// "follow the system", which is exactly the console's own default choice, so
/// the window is already right for every seller who has chosen nothing and is
/// corrected by this command within one page load for everyone else. Pinning
/// a theme there would instead make the startup window wrong for half of them.
///
/// A word rather than a boolean, and the same three the console stores, so
/// System survives the trip instead of being resolved on the way and pinned
/// to whatever the machine preferred at the moment of the click.
///
/// Unknown words are refused rather than defaulted: a console and an
/// application that disagree on the vocabulary is a mismatch worth seeing in
/// the log, and the console swallows the refusal because the page it is
/// looking at is already the right colour.
///
/// `async` with nothing awaited, as every command above it is. An owned
/// `AppHandle` is what Tauri injects, and a synchronous body that only
/// borrows it trips `needless_pass_by_value` under the workspace's pedantic
/// lints; taking a reference is not open, because `&AppHandle` is not a
/// command argument Tauri knows how to supply.
#[tauri::command]
pub async fn set_theme<R: tauri::Runtime>(
    app: AppHandle<R>,
    theme: String,
) -> Result<(), CommandError> {
    let wanted = match theme.as_str() {
        "light" => Some(tauri::Theme::Light),
        "dark" => Some(tauri::Theme::Dark),
        "system" => None,
        other => return Err(CommandError(format!("unknown theme {other}"))),
    };
    let Some(window) = app.get_webview_window(CONSOLE_WINDOW) else {
        // The console window is the only window this application has, so
        // there is nothing to theme and nothing to report.
        return Ok(());
    };
    window.set_theme(wanted)?;
    Ok(())
}
