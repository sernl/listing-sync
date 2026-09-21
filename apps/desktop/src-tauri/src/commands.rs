//! The commands the console calls, and the login webview behind the first of
//! them.
//!
//! One rule governs the login window: the marketplace page gets no capability
//! at all. It is not listed in `capabilities/default.json`, so `invoke` is
//! unreachable from it even before the site's own content-security policy
//! blocks `ipc.localhost`. Everything this module learns about the login it
//! learns by reading the webview's cookie store from Rust.

use core::time::Duration;
use std::sync::Arc;

use serde::Serialize;
use tam_types::Marketplace;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::connect::{login_target, return_url, ConnectVerdict, LoginTarget};
use crate::heartbeat::{check_in, first_run, CheckIn, CheckInError, SIGNED_OUT_HERE};
use crate::run::wall_now;
use crate::session::{Cookie, CookieJar, SessionRecord, SessionStatus};
use crate::state::{DesktopState, DeviceActivity};
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
    /// Which machine this is, as the registry knows it.
    ///
    /// Carried so the console can say which of the seller's machines it is
    /// running on, which it could not do at all before: it lists the
    /// organisation's devices and had no way to tell which row was the one the
    /// page was drawn on, so a seller reading "signed out" in the list could
    /// not tell whether it meant this machine or another one.
    pub device_id: String,
    /// The name that machine is listed under, so a sentence can name it
    /// instead of printing the id.
    pub device_name: String,
}

/// What a connect did, which is not the same question on the two surfaces.
///
/// On a computer the login runs in a second window, this call waits for it,
/// and the answer is the session. On a phone there is one window and it is
/// about to become the marketplace's own page, so the page that asked is gone
/// before there is anything to answer: this call says only that the sign-in is
/// opening, and the verdict comes back in the address the console is resumed
/// at. The third variant is the one answer both surfaces give without opening
/// anything: a machine the seller signed out from the console cannot hold a
/// marketplace login, and it is told before a password is typed rather than
/// after. Variants rather than an optional field, because "no session yet",
/// "no session" and "not on this machine" are three different facts and a
/// nullable one would collapse them.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ConnectOutcome {
    /// The login completed in a window of its own and the session is filed.
    Captured { session: SessionStatus },
    /// The sign-in is replacing this page. Nothing is filed yet, and the
    /// verdict arrives as `crate::connect::RETURN_PARAM` on the way back.
    Opening,
    /// This machine was signed out from the console, so nothing was opened
    /// and nothing was filed. The same word
    /// [`crate::connect::ConnectVerdict::SignedOut`] travels as, so the
    /// console words one sentence for the two channels.
    SignedOut,
    /// The organisation has not granted the seller-device consent for this
    /// marketplace on the current notice, so nothing was opened and nothing
    /// was filed. The same word
    /// [`crate::connect::ConnectVerdict::ConsentRequired`] travels as, so the
    /// console words one sentence for the two channels.
    ConsentRequired,
    /// The shop this sign-in speaks for is already connected to another
    /// Teachouse account. The same word
    /// [`crate::connect::ConnectVerdict::BoundElsewhere`] travels as, so the
    /// console words one sentence for the two channels.
    BoundElsewhere,
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
///
/// The check-in comes first, and that is what the founder's 0.7.0 was missing:
/// a machine signed out from the console still opened the marketplace's login,
/// took the seller through it, and only then discovered — in
/// [`file_session`]'s own check-in — that the capture could not be kept. The
/// seller typed a password for nothing, once per attempt, forever. Asking
/// before opening turns that into one sentence naming the one remedy.
pub(crate) async fn connect_on_target<R: tauri::Runtime>(
    app: AppHandle<R>,
    target: LoginTarget,
    surface: ConnectSurface,
) -> Result<ConnectOutcome, CommandError> {
    if signed_out_here(&app).await {
        return Ok(ConnectOutcome::SignedOut);
    }
    if !consent_stands_here(&app, target.marketplace).await? {
        return Ok(ConnectOutcome::ConsentRequired);
    }
    match surface {
        ConnectSurface::SecondWindow => in_a_second_window(app, target).await,
        ConnectSurface::OneWindow => in_this_window(&app, target).map(|()| ConnectOutcome::Opening),
    }
}

/// Whether the seller has agreed to the seller-device notice for this
/// marketplace, asked of the server at the moment of the press.
///
/// Not fail-open, and that is the difference from [`signed_out_here`]: an
/// unreachable server there leaves the last standing in place, because a
/// revocation is a rare event and an outage must not strand a machine in
/// good standing. A grant is the opposite — the ordinary state before the
/// seller has agreed is "no" — so a read that could not be made is an error
/// the seller sees, not a sign-in that opens on the assumption they agreed.
async fn consent_stands_here<R: tauri::Runtime>(
    app: &AppHandle<R>,
    marketplace: Marketplace,
) -> Result<bool, CommandError> {
    let state = app.state::<DesktopState>();
    state
        .control_plane()
        .consent_stands(marketplace)
        .await
        .map_err(|why| CommandError(format!("your permissions could not be read: {why}")))
}

/// Whether this machine may hold a marketplace login at all, asked of the
/// server rather than remembered.
///
/// The answer is read off the state rather than off the check-in, because the
/// two failures differ: a check-in that reached the server has just written
/// the standing it was told, and one that could not reach it leaves the last
/// standing we know of in place. A revoked device that is offline is still
/// refused — its store was wiped when it learned of the revocation, and every
/// cycle since has closed its gate — and a device that has never reached the
/// server is not refused, because nothing said it was signed out.
async fn signed_out_here<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    let state = app.state::<DesktopState>();
    // Discarded rather than raised, the judgement `forget_session` makes for
    // the same call: an unreachable server is not evidence of revocation, and
    // refusing the sign-in over an outage would strand a seller whose machine
    // is in good standing.
    check_in(&state, state.control_plane()).await.ok();
    state.revoked()
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
) -> Result<ConnectOutcome, CommandError> {
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
    let (jar, external_id) = match await_session(&target, &mut capture, move || {
        watched.get_webview_window(&closed).is_none()
    })
    .await
    {
        Ok(captured) => captured,
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
    match file_session(&app, marketplace, jar, external_id).await {
        Ok(session) => Ok(ConnectOutcome::Captured { session }),
        // Not a failure to report as one: the seller signed in and nothing
        // went wrong with the sign-in. What cannot happen is keeping it here,
        // and that is one sentence with one remedy rather than a diagnostic.
        Err(NotFiled::SignedOut) => Ok(ConnectOutcome::SignedOut),
        Err(NotFiled::BoundElsewhere) => Ok(ConnectOutcome::BoundElsewhere),
        Err(NotFiled::Failed(why)) => Err(why),
    }
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
    let attempt = app.state::<DesktopState>().begin_login().ok_or_else(|| {
        CommandError("a marketplace sign-in is already open on this phone".to_owned())
    })?;

    let app = app.clone();
    // The token crosses the task boundary and is dropped only after the final
    // verdict navigation. A second attempt cannot overtake a slow identity
    // check and be interrupted by this attempt's late answer.
    let attempt = attempt;
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
        drop(attempt);
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
        // Asked again before filing, because the sign-in took as long as the
        // seller took: a grant withdrawn on the Account page while the
        // password was being typed must not be answered by a session filed
        // under it. The jar is dropped unfiled either way.
        Ok(_)
            if !consent_stands_here(app, target.marketplace)
                .await
                .is_ok_and(|stands| stands) =>
        {
            ConnectVerdict::ConsentRequired
        }
        Ok((jar, external_id)) => {
            match file_session(app, target.marketplace, jar, external_id).await {
                Ok(_) => ConnectVerdict::Captured,
                // The revocation reached this device between the Connect press
                // and the sign-in finishing — the pre-flight check-in in
                // `connect_on_target` caught every earlier one — and the wipe it
                // triggered took the capture with it. Named as itself rather than
                // as `NotKept`, because the remedy is signing this machine back in
                // and pressing Connect again does nothing for it.
                Err(NotFiled::SignedOut) => ConnectVerdict::SignedOut,
                // The shop is another account's, which is a person's job and not
                // this surface's: the capture is gone and pressing Connect again
                // would be refused identically.
                Err(NotFiled::BoundElsewhere) => ConnectVerdict::BoundElsewhere,
                // Not `Refused`: the sign-in opened and the seller finished it.
                // The keychain or the store refused it, and the diagnostic has
                // nowhere to go on this surface.
                Err(NotFiled::Failed(_)) => ConnectVerdict::NotKept,
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
/// What a verification learned about a captured jar: whether the marketplace
/// answered it as the seller, and the storefront it named where the same read
/// named one.
///
/// A pair rather than a `bool` because the two facts come out of one request.
/// The identity read is the verification for Tes, so reducing it to a yes and
/// then asking again for the identifier would be a second marketplace request
/// for an answer already in hand.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Verified {
    pub(crate) authenticated: bool,
    pub(crate) external_id: Option<String>,
}

type VerifyFuture<'a> = core::pin::Pin<
    Box<dyn core::future::Future<Output = Result<Verified, CommandError>> + Send + 'a>,
>;
type VerifyJar = Box<dyn for<'a> FnMut(Marketplace, &'a CookieJar) -> VerifyFuture<'a> + Send>;

fn verify_login_candidate(marketplace: Marketplace, jar: &CookieJar) -> VerifyFuture<'_> {
    Box::pin(async move {
        match marketplace {
            Marketplace::Tes => crate::marketplace::verify_tes_login(jar)
                .await
                .map(|seller| Verified {
                    authenticated: seller.is_some(),
                    external_id: seller,
                })
                .map_err(CommandError),
            // TPT's login is filed unverified, and the storefront read that
            // would name the shop is left to the first import: it is the one
            // request whose shape a capture has measured, and making it here
            // would be a probe against a session nothing has proven.
            Marketplace::Tpt => Ok(Verified {
                authenticated: true,
                external_id: None,
            }),
            Marketplace::Etsy => Err(CommandError(
                "Etsy does not use a device-held login".to_owned(),
            )),
        }
    })
}

/// What a login capture reads, and how long it is given to read it.
///
/// The reader is a value rather than a call to [`read_jar`] in place, and that
/// is what puts the success path under test at all:
/// `tauri::test::MockRuntime::cookies_for_url` answers every read with an empty
/// jar, so on the mock runtime `LoginTarget::has_login_cookies` is false forever and
/// neither a capture nor a deadline can be reached. Without the seam the one
/// outcome the whole surface exists to produce would ship with no test at any
/// level.
struct Capture {
    poll: Duration,
    deadline: Duration,
    verification_interval: Duration,
    read: ReadJar,
    verify: VerifyJar,
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
            verification_interval: Duration::from_secs(5),
            read: Box::new(move || read_jar(&window, &origin)),
            verify: Box::new(verify_login_candidate),
        }
    }
}

/// Read candidates until the marketplace's login condition is confirmed.
///
/// The one wait both surfaces take, so the logged-in condition, the interval
/// and the deadline cannot come to differ between a computer and a phone.
/// `left` is the surface's own way of saying the seller has gone: a closed
/// window on one, a return to our own origin on the other.
///
/// Answers the storefront the verification named beside the jar, because the
/// read that admitted the jar is the read that named it.
async fn await_session(
    target: &LoginTarget,
    capture: &mut Capture,
    mut left: impl FnMut() -> bool + Send,
) -> Result<(CookieJar, Option<String>), Waited> {
    let deadline = tokio::time::Instant::now() + capture.deadline;
    let mut rejected = None;
    let mut verify_after = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(capture.poll).await;
        if left() {
            return Err(Waited::Left);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(Waited::Deadline);
        }
        let jar = (capture.read)().map_err(Waited::Unreadable)?;
        if target.has_login_cookies(&jar)
            && (rejected.as_ref() != Some(&jar) || now >= verify_after)
        {
            let verified =
                tokio::time::timeout_at(deadline, (capture.verify)(target.marketplace, &jar))
                    .await
                    .map_err(|_| Waited::Deadline)?
                    .map_err(Waited::Unreadable)?;
            if left() {
                return Err(Waited::Left);
            }
            if verified.authenticated {
                return Ok((jar, verified.external_id));
            }
            // Login can activate a session without replacing its cookie.
            // Changed jars are checked immediately; unchanged ones are bounded.
            verify_after = tokio::time::Instant::now() + capture.verification_interval;
            rejected = Some(jar);
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

/// Why a captured jar was not kept.
///
/// Three arms rather than one string, because each is a different sentence
/// and a different remedy on both surfaces: a machine signed out from the
/// console is signed back in once and then every sign-in holds, a shop
/// another account holds is a person's job and no press of Connect changes
/// it, and a store that refused is a fault the seller can only retry. A
/// single error type collapsed the first and the last, and what the founder
/// read for one was the prose of the other.
enum NotFiled {
    /// The check-in that follows the capture answered that this machine was
    /// signed out from the console, and the wipe it ran took the capture with
    /// it.
    SignedOut,
    /// The check-in that follows the capture was refused because the shop
    /// this session speaks for is bound to another organisation. The capture
    /// is forgotten here: kept, it would be reported on every later beat and
    /// refused on every later beat, and a device whose check-in always fails
    /// never learns it was signed out either.
    BoundElsewhere,
    /// The store refused, with its own diagnostic.
    Failed(CommandError),
}

impl From<crate::session::StoreError> for NotFiled {
    fn from(why: crate::session::StoreError) -> Self {
        Self::Failed(why.into())
    }
}

/// File a captured jar, and tell the server before calling it a success.
async fn file_session<R: tauri::Runtime>(
    app: &AppHandle<R>,
    marketplace: Marketplace,
    jar: CookieJar,
    external_id: Option<String>,
) -> Result<SessionStatus, NotFiled> {
    let state = app.state::<DesktopState>();
    let captured_at = wall_now();
    let record = SessionRecord {
        marketplace,
        // No label. The storefront name the marketplace shows the seller is a
        // display string that authenticates nothing, and reading it is a
        // second marketplace request; `external_id` below is the identifier,
        // and it came out of the read that admitted this jar.
        account_label: None,
        captured_at,
        device_id: state.device().id.clone(),
        jar,
        // `await_session` did not file this jar until the marketplace
        // answered it as the seller, so the capture instant is also the
        // instant it was last proven. Leaving it unproven would report a
        // session the seller has this moment signed in to as needing another
        // sign-in.
        verified_at: Some(captured_at),
        external_id,
    };
    state.store().put(&record).await?;

    // D14: a device the seller has already signed out must not keep a session
    // it just captured, so the check-in that would learn of it happens before
    // the capture is reported as a success. A check-in that could not reach the
    // server is not evidence of revocation and leaves the capture standing.
    //
    // The storefront refusal is the same shape and the opposite disposition:
    // the server read the report and will not accept it, so the capture goes
    // rather than standing. It is the check-in's only `Rejected` answer.
    match check_in(&state, state.control_plane()).await {
        Ok(answer) if answer.revoked => return Err(NotFiled::SignedOut),
        Err(crate::heartbeat::CheckInError::Plane(
            crate::heartbeat::ControlPlaneError::Rejected(said),
        )) => {
            eprintln!("the server refused this storefront: {said}");
            state.store().forget(marketplace).await?;
            return Err(NotFiled::BoundElsewhere);
        }
        Ok(_) | Err(_) => {}
    }
    Ok(SessionStatus::of(&record, captured_at))
}

/// Registers this device with the server's registry and checks in.
///
/// The console calls it when it loads, which is what first run means for a
/// client whose interface is the console. Registration is idempotent on the
/// server, so calling it again is a refresh rather than a second machine.
///
/// A device already known to be signed out checks in and does not register,
/// and that is the invariant rather than an optimisation: registration cannot
/// clear a revocation — `revoked_at` is its own column and the upsert does not
/// touch it — but a client that attempted one on every press would be a client
/// whose recovery path was a registration, which is the shape the seller's
/// explicit restore exists to be the only instance of. What this press does
/// while revoked is observe: a restore performed in the console lands here as
/// an answer that is no longer revoked.
///
/// The coordinator is woken either way, so work the seller queued in the
/// browser is picked up now rather than at the next cadence.
#[tauri::command]
pub async fn device_check_in(app: AppHandle) -> Result<DeviceState, CommandError> {
    let state = app.state::<DesktopState>();
    let answer = if state.revoked() {
        check_in(&state, state.control_plane()).await
    } else {
        first_run(&state, state.control_plane()).await
    };
    wake(&app);
    device_state(&state, answer)
}

/// Ask the coordinator for an immediate pass, where one is running.
///
/// Absent in a host test, which builds a state and no loop, so this is a read
/// of managed state that tolerates its absence rather than an `expect`: a
/// command that panicked without a coordinator would be a command only the
/// shipped build could answer.
fn wake<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(wake) = app.try_state::<Arc<crate::Wake>>() {
        wake.now();
    }
}

/// What the console is told a check-in found, this machine's identity
/// included.
///
/// Split from the command because the command takes the concrete runtime
/// Tauri hands it and a host test cannot build one: everything the console
/// actually reads is decided here, where it can be asked what it says.
fn device_state(
    state: &DesktopState,
    answer: Result<CheckIn, CheckInError>,
) -> Result<DeviceState, CommandError> {
    // Named on every answer, including the ones that reached nothing: which
    // machine this is, is a fact about the machine and not about the call, and
    // a console that lost the name on an outage could not say "this machine"
    // in the one list where it matters most.
    let device_id = state.device().id.as_str().to_owned();
    let device_name = state.device().label.clone();
    match answer {
        Ok(answer) => Ok(DeviceState {
            revoked: answer.revoked,
            reached_server: true,
            signed_in: true,
            detail: None,
            device_id,
            device_name,
        }),
        Err(CheckInError::Plane(why)) => Ok(DeviceState {
            revoked: state.revoked(),
            reached_server: false,
            signed_in: state.signed_in(),
            detail: Some(why.to_string()),
            device_id,
            device_name,
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

// ----------------------------------------------------------------- library

/// The sentence a console reads when this machine is keeping no files: this
/// build has no library at all, or the library cannot be opened right now.
const NO_LIBRARY: &str = "this machine is not keeping files";

/// The library, asked for at the moment the seller asked for it.
///
/// The answer is about now rather than about start-up. A phone launched from
/// its lock screen cannot open the library — the Keystore refuses every
/// operation while the keyguard shows — and by the time the seller has the
/// console in front of them it can, so the refusal must be re-asked rather
/// than remembered. Why it refused goes to the log, where a developer
/// reading a device's output can see it; the seller gets the one sentence
/// they can act on, because "Keystore operation failed" tells them nothing.
async fn library_of(app: &AppHandle) -> Result<Arc<crate::library::Library>, CommandError> {
    let slot = app
        .state::<DesktopState>()
        .library()
        .ok_or_else(|| CommandError(NO_LIBRARY.to_owned()))?;
    slot.get().await.map_err(|why| {
        eprintln!("the library on this machine could not be opened: {why}");
        CommandError(NO_LIBRARY.to_owned())
    })
}

fn hash_of(hex: &str) -> Result<tam_types::ContentHash, CommandError> {
    if hex.len() != 64 {
        return Err(CommandError(
            "a file is named by its 64-hex digest".to_owned(),
        ));
    }
    let mut bytes = [0u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let pair = core::str::from_utf8(pair)
            .map_err(|_| CommandError("a file is named by its hex digest".to_owned()))?;
        bytes[index] = u8::from_str_radix(pair, 16)
            .map_err(|_| CommandError("a file is named by its hex digest".to_owned()))?;
    }
    Ok(tam_types::ContentHash(bytes))
}

/// One kept file, as the console lists it: the digest as hex rather than
/// as the byte array `ContentHash` serialises to.
#[derive(Debug, Clone, Serialize)]
pub struct LibraryEntryView {
    pub hash: String,
    pub file_name: String,
    pub content_type: String,
    pub byte_len: u64,
    pub marketplace: Marketplace,
    pub resource: String,
    pub kept_at: tam_types::Timestamp,
    pub pinned: bool,
}

impl From<crate::library::LibraryEntry> for LibraryEntryView {
    fn from(entry: crate::library::LibraryEntry) -> Self {
        Self {
            hash: tam_secrets::hex_encode(&entry.hash.0),
            file_name: entry.file_name,
            content_type: entry.content_type,
            byte_len: entry.byte_len,
            marketplace: entry.marketplace,
            resource: entry.resource,
            kept_at: entry.kept_at,
            pinned: entry.pinned,
        }
    }
}

/// Every file kept on this machine, newest first.
#[tauri::command]
pub async fn library_entries(app: AppHandle) -> Result<Vec<LibraryEntryView>, CommandError> {
    let library = library_of(&app).await?;
    Ok(library
        .entries()
        .await
        .into_iter()
        .map(LibraryEntryView::from)
        .collect())
}

/// The bytes kept on this machine, as the seller would count them.
#[tauri::command]
pub async fn library_usage(app: AppHandle) -> Result<u64, CommandError> {
    Ok(library_of(&app).await?.usage().await)
}

/// One kept file's bytes, in the clear, for a preview or a viewer in the
/// console's own window. Raw rather than JSON, so a thirty-megabyte PDF is
/// not base64 in a string.
#[tauri::command]
pub async fn library_read(
    app: AppHandle,
    hash: String,
) -> Result<tauri::ipc::Response, CommandError> {
    let library = library_of(&app).await?;
    let bytes = library
        .read(hash_of(&hash)?)
        .await
        .map_err(|why| CommandError(why.to_string()))?
        .ok_or_else(|| CommandError("that file is not kept on this machine".to_owned()))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Removes one kept file from this machine. The listing and the marketplace
/// copy are untouched, as the console's confirmation says.
#[tauri::command]
pub async fn library_remove(app: AppHandle, hash: String) -> Result<(), CommandError> {
    library_of(&app)
        .await?
        .remove(hash_of(&hash)?)
        .await
        .map_err(|why| CommandError(why.to_string()))
}

#[tauri::command]
pub async fn library_settings(
    app: AppHandle,
) -> Result<crate::library::LibrarySettings, CommandError> {
    Ok(library_of(&app).await?.settings().await)
}

#[tauri::command]
pub async fn set_library_settings(
    app: AppHandle,
    keep_originals: bool,
) -> Result<crate::library::LibrarySettings, CommandError> {
    library_of(&app)
        .await?
        .set_keep_originals(keep_originals)
        .await
        .map_err(|why| CommandError(why.to_string()))
}

#[cfg(target_os = "android")]
pub(crate) struct AndroidLibraryOpener<R: tauri::Runtime>(
    pub(crate) tauri::plugin::PluginHandle<R>,
);

/// Hands one kept file to whatever application this machine opens that
/// type with.
///
/// The plaintext is written under the application's own cache directory —
/// on Android the `cache-path` the manifest's FileProvider exports — and the
/// platform opener is asked to open it. The seller asked for exactly this,
/// so the copy is theirs to have; the sealed library is untouched. Where the
/// opener refuses, the refusal is the answer and there is no other route.
#[tauri::command]
pub async fn library_open_external(app: AppHandle, hash: String) -> Result<(), CommandError> {
    #[cfg(not(target_os = "android"))]
    use tauri_plugin_opener::OpenerExt as _;
    let library = library_of(&app).await?;
    let digest = hash_of(&hash)?;
    let entry = library
        .entries()
        .await
        .into_iter()
        .find(|entry| entry.hash == digest)
        .ok_or_else(|| CommandError("that file is not kept on this machine".to_owned()))?;
    let bytes = library
        .read(digest)
        .await
        .map_err(|why| CommandError(why.to_string()))?
        .ok_or_else(|| CommandError("that file is not kept on this machine".to_owned()))?;
    let extension = std::path::Path::new(&entry.file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|why| CommandError(why.to_string()))?
        .join("open");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|why| CommandError(why.to_string()))?;
    let path = dir.join(format!("{hash}.{extension}"));
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|why| CommandError(why.to_string()))?;
    #[cfg(target_os = "android")]
    {
        app.state::<AndroidLibraryOpener<tauri::Wry>>()
            .0
            .run_mobile_plugin::<()>("openLibraryFile", serde_json::json!({ "path": path }))
            .map_err(|why| CommandError(why.to_string()))
    }
    #[cfg(not(target_os = "android"))]
    {
        app.opener()
            .open_path(path.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|why| CommandError(why.to_string()))
    }
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
        |record| SessionStatus::of(record, wall_now()),
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
///
/// Durable acceptance rather than a result: the answer says the run is this
/// device's, under this attempt, until the lease it names expires. What the
/// shop holds is no longer here, because it is no longer known by the time
/// this answers — the enumeration happens after, and the run's own execution
/// view is where the console reads the counts from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportStarted {
    /// Echoed so the console can match the answer to the run it asked about,
    /// rather than assuming the only start in flight is its own.
    pub run: String,
    /// The fence this device now holds. Every page and every report it posts
    /// carries it, and a later attempt's arrival is what invalidates them.
    pub attempt: u64,
    /// When the lease lapses if nothing renews it, in milliseconds since the
    /// epoch: the console's own timestamp convention, and the server's clock
    /// rather than this device's.
    pub lease_expires_at: i64,
}

/// What continuing an import answered. The same acceptance, for the half the
/// seller's selection names.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportContinued {
    pub run: String,
    pub attempt: u64,
    pub lease_expires_at: i64,
}

/// What stopping an import on this device answered.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportStopped {
    pub run: String,
    /// Whether this device was working the run at all. False is not a
    /// failure: the seller may be stopping a run another device holds, and
    /// the server settles that one.
    pub was_running: bool,
    /// Whether the stop was written down on this device.
    ///
    /// False means the work here has ended but the intention was not kept, so
    /// a restart could pick the run up again before the server settles it.
    /// Surfaced rather than folded into the line below, because it is a
    /// different thing from waiting on the server and has a different remedy.
    pub recorded: bool,
    /// Whether the server has yet to be told. Always true here: this device
    /// never learns at this moment that the server agreed. The console's own
    /// abandon call is what settles the run, and the local mark is cleared
    /// only once a later cycle sees the server stop offering it — so the
    /// honest answer now is "stopped on this device, confirmation pending"
    /// rather than a claim about a server this device has not spoken to.
    pub server_pending: bool,
}

/// The catalogue reader for a shop, chosen by which marketplace it is.
///
/// Named here rather than discovered as an adapter error mid-pass, the same
/// way `SellerFiles::fetch` names it for a download. A marketplace with an
/// official API is read on our own servers under a sanctioned token, so it is
/// not a shop this device enumerates.
///
/// It reads the session store out of the import context rather than out of
/// the application state, because a run outlives the command that started it:
/// the task holds the handles, not a borrow of the state.
pub(crate) fn catalogue_for(
    ctx: &crate::import::ImportContext,
    source: tam_types::InventoryId,
) -> Result<Box<dyn crate::import::CatalogueSource>, String> {
    match source.marketplace() {
        Marketplace::Tes => Ok(Box::new(crate::work::SellerCatalogue::new(
            Arc::clone(&ctx.sessions),
            source,
        ))),
        Marketplace::Tpt => Ok(Box::new(crate::work::TptSellerCatalogue::new(
            Arc::clone(&ctx.sessions),
            source,
        ))),
        other @ Marketplace::Etsy => Err(format!(
            "this device cannot read a {other:?} catalogue: a marketplace with an official API is read on our own servers rather than here"
        )),
    }
}

/// The factory every real build hands the supervisor.
#[must_use]
pub fn live_catalogue() -> crate::import::CatalogueFactory {
    Arc::new(catalogue_for)
}

/// The transport an import posts its pages over, or the refusal a build with
/// none owes the seller.
fn require_ledger(state: &DesktopState) -> Result<crate::import::ImportContext, CommandError> {
    crate::import::ImportContext::of(state).ok_or_else(|| {
        CommandError(
            "this build has no way to reach the server, so an import would have nowhere to post what it read".to_owned(),
        )
    })
}

/// One press, as the supervisor reads it.
///
/// The run and nothing else, because that is all the console knows and all it
/// should: the device reads which shop the run names from the run itself, and
/// it does that after taking the fence so a read that fails is recorded
/// against a run this device owns rather than lost with the window.
fn pressed(
    run: tam_types::Uuid,
    phase: crate::import::RunPhase,
    takeover: Option<bool>,
) -> crate::import::RunOrder {
    crate::import::RunOrder {
        run,
        source: None,
        phase,
        intent: crate::import::StartIntent::Pressed {
            takeover: takeover.unwrap_or(false),
        },
    }
}

/// Whether this device may take work, decided against the server rather than
/// against a cached mark.
///
/// The cached mark alone was a bug with a seam in it. The seller's recovery is
/// two calls the console makes in order — the restore route, then a check-in —
/// and the second may fail on its own: a restore that succeeded while its
/// check-in dropped leaves the console showing a machine that is signed back
/// in and this process still holding `revoked`. Every explicit start then
/// refused locally, naming an act the server had already undone, until the
/// next scheduled check-in came round.
///
/// So a cached revocation is a reason to *ask*, never a reason to refuse. One
/// check-in, which is the call that learns the current answer and installs the
/// gate that goes with it:
///
/// - revoked still, as the server says now — refuse, in the seller's words.
/// - not revoked any more — go ahead; `check_in` has already cleared the mark
///   and installed the entitlement this run will be worked under.
/// - unreachable — go ahead. An outage is not a revocation and must never be
///   rendered as one; the claim is the authorisation boundary and the server
///   refuses there if the machine really is signed out, now as
///   [`crate::heartbeat::ControlPlaneError::Revoked`] carrying the same
///   sentence rather than as a response body.
///
/// [`check_in`] rather than [`first_run`], deliberately and by the same rule
/// the whole repair follows: nothing here registers, and nothing here
/// restores. A revoked device that re-registered would be a device lifting its
/// own revocation, and the only thing that clears the mark is the seller's
/// explicit restore.
///
/// The cost is one request on the press after a sign-out, and none at all on
/// every other press: an unrevoked device does not reach the check-in.
async fn refuse_if_signed_out(state: &DesktopState) -> Result<(), CommandError> {
    if !state.revoked() {
        return Ok(());
    }
    match check_in(state, state.control_plane()).await {
        Ok(answer) if answer.revoked => Err(CommandError(SIGNED_OUT_HERE.to_owned())),
        Ok(_) | Err(_) => Ok(()),
    }
}

/// Asks this device to read the shop one run names.
///
/// Answers once the run is durably this device's rather than once the shop
/// has been read, and that is the repair rather than a convenience. The old
/// command awaited the enumeration, so every failure before the first page
/// lived in one window's promise: a seller who navigated away — which the
/// console does as soon as a start is accepted — left the run in the state
/// the server gave it at creation, which the console renders as under way.
/// Now the claim is taken first, every refusal after it is reported to the
/// run, and the walk happens in a task the window's lifetime has no bearing
/// on.
///
/// `takeover` is false unless the seller has confirmed that this machine
/// should take a run another one holds. An ordinary press must not wrench a
/// run out of a phone that is reading a shop right now, so the claim is
/// refused and the console offers the confirmation instead.
#[tauri::command]
pub async fn start_import<R: tauri::Runtime>(
    app: AppHandle<R>,
    run: tam_types::Uuid,
    takeover: Option<bool>,
) -> Result<ImportStarted, CommandError> {
    let state = app.state::<DesktopState>();
    refuse_if_signed_out(state.inner()).await?;
    let ctx = require_ledger(state.inner())?;
    let accepted = ctx
        .supervisor
        .accept(
            &ctx,
            pressed(run, crate::import::RunPhase::Discover, takeover),
        )
        .await
        .map_err(|why| CommandError(why.to_string()))?;
    Ok(ImportStarted {
        run: uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string(),
        attempt: accepted.attempt,
        lease_expires_at: accepted.lease_expires_at,
    })
}

/// Asks this device to read the resources the seller ticked.
///
/// The same acceptance as the half above, for the same reason: a shop of
/// several hundred resources is minutes of work, the seller navigates away,
/// and the run's own view is what they come back to. What the selection holds
/// is not counted here — the server froze that total when it accepted the
/// selection, and counting it again on the device would be a second answer to
/// a question that already has one.
#[tauri::command]
pub async fn continue_import<R: tauri::Runtime>(
    app: AppHandle<R>,
    run: tam_types::Uuid,
    takeover: Option<bool>,
) -> Result<ImportContinued, CommandError> {
    let state = app.state::<DesktopState>();
    refuse_if_signed_out(state.inner()).await?;
    let ctx = require_ledger(state.inner())?;
    let accepted = ctx
        .supervisor
        .accept(
            &ctx,
            pressed(run, crate::import::RunPhase::Describe, takeover),
        )
        .await
        .map_err(|why| CommandError(why.to_string()))?;
    Ok(ImportContinued {
        run: uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string(),
        attempt: accepted.attempt,
        lease_expires_at: accepted.lease_expires_at,
    })
}

/// Stops this device's work on one run.
///
/// The console's Stop settles the run on the server, which is the authority;
/// this is the half only the device can do, and before it existed the two
/// disagreed — the run settled while the phone went on making marketplace
/// requests for it, because the claim set could say a run was running and
/// could not stop it.
///
/// It needs no server: raising the run's own handle is local and immediate,
/// which is what makes a stop work on a phone with no signal. It does need
/// the journal, because a stop that lived only in memory was undone by the
/// next restart.
#[tauri::command]
pub async fn stop_import<R: tauri::Runtime>(
    app: AppHandle<R>,
    run: tam_types::Uuid,
) -> Result<ImportStopped, CommandError> {
    let state = app.state::<DesktopState>();
    let journal = state.journal();
    let stopped = state
        .supervisor()
        .cancel(&journal, &state.device().id, run)
        .await;
    Ok(ImportStopped {
        run: uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string(),
        was_running: stopped.was_running,
        recorded: stopped.recorded,
        server_pending: stopped.server_pending,
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

        let refusal = super::start_import(app.handle().clone(), tam_types::Uuid([0x71; 16]), None)
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

    /// A control plane that answers one standing, and counts what it was
    /// asked. Registrations are counted because the number that matters is
    /// zero: a revoked device must never register its way back in.
    struct Standing {
        revoked: bool,
        reachable: bool,
        beats: core::sync::atomic::AtomicUsize,
        registrations: core::sync::atomic::AtomicUsize,
    }

    impl Standing {
        fn saying(revoked: bool) -> Self {
            Self {
                revoked,
                reachable: true,
                beats: core::sync::atomic::AtomicUsize::new(0),
                registrations: core::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn unreachable() -> Self {
            Self {
                reachable: false,
                ..Self::saying(true)
            }
        }
    }

    impl crate::heartbeat::ControlPlane for Standing {
        fn reachable(&self) -> crate::heartbeat::PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn consent_stands(
            &self,
            _marketplace: tam_types::Marketplace,
        ) -> crate::heartbeat::PlaneFuture<'_, bool> {
            Box::pin(core::future::ready(Ok(true)))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> crate::heartbeat::PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_run_facts(
            &self,
            _run: tam_types::Uuid,
        ) -> crate::heartbeat::PlaneFuture<'_, crate::import::RunFacts> {
            Box::pin(core::future::ready(Err(
                crate::heartbeat::ControlPlaneError::NotConfigured,
            )))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a DeviceId,
            _run: tam_types::Uuid,
        ) -> crate::heartbeat::PlaneFuture<'a, Vec<String>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn open_import_runs<'a>(
            &'a self,
            _device: &'a DeviceId,
        ) -> crate::heartbeat::PlaneFuture<'a, Vec<crate::import::OpenImportRun>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn register<'a>(
            &'a self,
            _device: &'a DeviceIdentity,
            _facts: crate::heartbeat::HostFacts,
        ) -> crate::heartbeat::PlaneFuture<'a, ()> {
            self.registrations
                .fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            Box::pin(core::future::ready(Ok(())))
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            _sessions: &'a [crate::heartbeat::SessionReport],
        ) -> crate::heartbeat::PlaneFuture<'a, crate::heartbeat::CheckIn> {
            self.beats
                .fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            if !self.reachable {
                return Box::pin(core::future::ready(Err(
                    crate::heartbeat::ControlPlaneError::Refused("no route".to_owned()),
                )));
            }
            let revoked = self.revoked;
            Box::pin(core::future::ready(Ok(crate::heartbeat::CheckIn {
                revoked,
                entitlement: None,
            })))
        }
    }

    fn signed_out_state(plane: Arc<Standing>) -> DesktopState {
        let state = DesktopState::with_control_plane(
            DeviceIdentity {
                id: DeviceId::from_raw("11112222333344445555666677778888"),
                label: "a test machine".to_owned(),
            },
            Arc::new(MemorySessionStore::default()),
            plane,
        );
        // What a device holds after a check-in that reported a sign-out.
        state.set_revoked(true);
        state
    }

    /// A machine the seller signed out refuses the import in words a teacher
    /// can read, and refuses it on what the server says now rather than on
    /// what this process last heard.
    ///
    /// Both halves are the property. The sentence is compared against
    /// `SIGNED_OUT_HERE` itself, which is the string the console exports as
    /// `DEVICE_SIGNED_OUT` and compares against to stop offering Resume at
    /// all — so a seller reads the act they performed and its remedy, never
    /// the server's forbidden body, which is what the founder met on Android.
    /// And the check-in is what decides it, so a stale mark cannot refuse a
    /// machine the server has already restored.
    ///
    /// Measured against a build that would otherwise refuse for a different
    /// reason — no ledger transport — so what this pins is that the sign-out
    /// is decided first rather than that any refusal happens.
    #[tokio::test]
    async fn a_machine_the_server_still_calls_signed_out_refuses_an_import_in_a_sentence() {
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::start_import])
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        let plane = Arc::new(Standing::saying(true));
        app.manage(signed_out_state(Arc::clone(&plane)));

        let refusal = super::start_import(app.handle().clone(), tam_types::Uuid([0x71; 16]), None)
            .await
            .expect_err("a signed-out machine may not run an import");
        assert_eq!(
            refusal.0,
            crate::heartbeat::SIGNED_OUT_HERE,
            "the seller reads the sign-out and its remedy, not the transport's own complaint"
        );
        assert!(
            !refusal.0.contains('{') && !refusal.0.contains("403"),
            "and nothing off the wire reaches them: {}",
            refusal.0
        );

        let continued =
            super::continue_import(app.handle().clone(), tam_types::Uuid([0x71; 16]), None)
                .await
                .expect_err("nor may it continue one");
        assert_eq!(
            continued.0,
            crate::heartbeat::SIGNED_OUT_HERE,
            "both halves of the flow refuse the same way, or one of them would be the way in"
        );
        assert_eq!(
            plane
                .registrations
                .load(core::sync::atomic::Ordering::SeqCst),
            0,
            "and it asked without registering: a revoked device that registered its way back \
             would be lifting its own revocation"
        );
        drop(app);
    }

    /// A restore the console performed is honoured on the next press, even
    /// though the check-in that should have cleared the mark never landed.
    ///
    /// This is the seam. Signing a machine back in is two calls — the restore
    /// route, then a check-in — and the second can fail on its own. The
    /// console then shows a machine that is signed back in while this process
    /// still holds `revoked`, and a guard that trusted that mark refused every
    /// press until the next scheduled check-in came round, naming an act the
    /// server had already undone.
    #[tokio::test]
    async fn a_press_after_a_restore_asks_the_server_rather_than_trusting_a_stale_mark() {
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::start_import])
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        // The restore landed on the server; this process never heard about it.
        let plane = Arc::new(Standing::saying(false));
        app.manage(signed_out_state(Arc::clone(&plane)));

        let answer = super::start_import(app.handle().clone(), tam_types::Uuid([0x71; 16]), None)
            .await
            .expect_err("this build still has no ledger transport");
        assert!(
            answer.0.contains("no way to reach the server"),
            "the press got past the sign-out and failed on what is actually missing, rather \
             than being refused for a revocation the server had already lifted. Got: {}",
            answer.0
        );
        assert_eq!(
            plane.beats.load(core::sync::atomic::Ordering::SeqCst),
            1,
            "one check-in, which is the call that learns the standing and installs the gate"
        );
        assert!(
            !app.state::<DesktopState>().revoked(),
            "and the mark is cleared by that check-in rather than by the press: nothing here \
             restores, and only the server's answer moves it"
        );
        drop(app);
    }

    /// An outage on that check-in is not a sign-out.
    ///
    /// The direction matters more than the outcome. A dropped connection says
    /// nothing about the seller's decision, so rendering it as "this machine
    /// was signed out" would accuse them of an act they did not perform and
    /// send them to a restore they do not need. The claim is the authorisation
    /// boundary, and the server refuses there if the machine really is signed
    /// out — as `ControlPlaneError::Revoked`, carrying this same sentence.
    #[tokio::test]
    async fn a_check_in_that_could_not_reach_us_is_not_read_as_a_sign_out() {
        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![super::start_import])
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        let plane = Arc::new(Standing::unreachable());
        app.manage(signed_out_state(Arc::clone(&plane)));

        let answer = super::start_import(app.handle().clone(), tam_types::Uuid([0x71; 16]), None)
            .await
            .expect_err("this build still has no ledger transport");
        assert_ne!(
            answer.0,
            crate::heartbeat::SIGNED_OUT_HERE,
            "an outage must not be rendered as the seller's own sign-out"
        );
        assert!(
            app.state::<DesktopState>().revoked(),
            "and it clears nothing either: a server we could not reach said nothing about the \
             mark, so it still stands"
        );
        drop(app);
    }
}

#[cfg(test)]
mod import_early_failure_tests {
    //! What a run is told when the device refuses before it reads anything.
    //!
    //! The incident of 2026-09-12 is exactly this hole: the server accepted
    //! six runs, every one of them held zero items and a null `read_total`,
    //! and five were abandoned by the seller while one sat in `reading`. The
    //! device had already decided it could not proceed — no local session for
    //! the shop, or a source it cannot enumerate at all — and it said so to
    //! the window that pressed the button and to nothing else. A window that
    //! has navigated away is nowhere, so the run kept the state the server
    //! gave it on creation.
    //!
    //! Driven through the real command rather than through a pass or a
    //! reporter in isolation. The defect is in what the command caller does
    //! with a refusal, so a test that called a reporter itself would pass
    //! over the whole of it.

    use crate::device::{DeviceId, DeviceIdentity};
    use crate::heartbeat::{CheckIn, ControlPlane, HostFacts, PlaneFuture, SessionReport};
    use crate::ledger::LedgerTransport;
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::DesktopState;
    use std::sync::Arc;
    use tam_types::{Marketplace, Timestamp};
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Manager;
    use tokio::sync::Mutex;

    const DEVICE: &str = "11112222333344445555666677778888";
    /// The run the server already accepted. One value, because the assertion
    /// is that what reaches the transport names this run rather than that
    /// something reached it.
    const RUN: tam_types::Uuid = tam_types::Uuid([0x71; 16]);
    const NOW: Timestamp = Timestamp(1_756_000_000_000);

    /// The run transport, as the server would see it: every call this device
    /// made, with the path and the body it carried.
    ///
    /// Both traits on one object because that is what a real build has — the
    /// control plane and the ledger transport are two traits over one
    /// `HttpControlPlane` — and because the question this fake answers is
    /// "did anything about this refusal leave the device", which cannot be
    /// asked of a transport the command does not hold.
    struct RunObserver {
        source: tam_types::InventoryId,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl RunObserver {
        fn for_source(source: tam_types::InventoryId) -> Arc<Self> {
            Arc::new(Self {
                source,
                calls: Mutex::new(Vec::new()),
            })
        }

        /// Everything the device posted that names this run, as one string per
        /// call. The path as well as the body, because a claim names the run
        /// in its path alone.
        async fn about(&self, run: tam_types::Uuid) -> Vec<String> {
            let hyphenated = uuid::Uuid::from_bytes(run.0).as_hyphenated().to_string();
            let simple = uuid::Uuid::from_bytes(run.0).simple().to_string();
            self.calls
                .lock()
                .await
                .iter()
                .map(|(path, body)| format!("{path} {body}"))
                .filter(|call| call.contains(&hyphenated) || call.contains(&simple))
                .collect()
        }
    }

    impl LedgerTransport for RunObserver {
        /// Records the call, and answers a claim with a lease.
        ///
        /// The lease is the one thing this fake has to do rather than merely
        /// observe: the device claims the run before it checks whether it can
        /// read the shop, which is the order that makes the refusal durable,
        /// so a fake that answered a claim with nothing would stop the flow
        /// before the refusal this test is about.
        fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String> {
            Box::pin(async move {
                let claimed = path.ends_with("/claim");
                self.calls.lock().await.push((path.to_owned(), body));
                if claimed {
                    return Ok(serde_json::json!({
                        "attempt": 1,
                        "lease_expires_at": 1_756_000_060_000_i64,
                    })
                    .to_string());
                }
                Ok(String::new())
            })
        }
    }

    impl ControlPlane for RunObserver {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn consent_stands(&self, _marketplace: Marketplace) -> PlaneFuture<'_, bool> {
            Box::pin(core::future::ready(Ok(true)))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(self.source)))
        }

        /// The run exists and names its shop: the server accepted this import,
        /// which is what makes the silence that follows a defect rather than a
        /// refusal to start something that was never started.
        fn import_run_facts(
            &self,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'_, crate::import::RunFacts> {
            Box::pin(core::future::ready(Ok(crate::import::RunFacts {
                source: self.source,
                discovered: 0,
                processed: 0,
                described: 0,
                enumeration_complete: false,
            })))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a DeviceId,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'a, Vec<String>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn open_import_runs<'a>(
            &'a self,
            _device: &'a DeviceId,
        ) -> PlaneFuture<'a, Vec<crate::import::OpenImportRun>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn register<'a>(
            &'a self,
            _device: &'a DeviceIdentity,
            _facts: HostFacts,
        ) -> PlaneFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            _sessions: &'a [SessionReport],
        ) -> PlaneFuture<'a, CheckIn> {
            Box::pin(async {
                Ok(CheckIn {
                    revoked: false,
                    entitlement: None,
                })
            })
        }
    }

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::from_raw(DEVICE),
            label: "the founder's phone".to_owned(),
        }
    }

    /// An application holding the state the command reads, with the observer
    /// as both the registry and the run transport.
    fn app_with(
        store: Arc<MemorySessionStore>,
        observer: &Arc<RunObserver>,
    ) -> tauri::App<tauri::test::MockRuntime> {
        let app = mock_builder()
            .build(mock_context(noop_assets()))
            .expect("the mock application builds");
        // Cloned at the concrete type and then unsized, which is the form the
        // application's own `setup` uses: `Arc::clone` would resolve its type
        // parameter against the annotation and refuse the coercion.
        let registry: Arc<dyn ControlPlane> = Arc::<RunObserver>::clone(observer);
        let ledger: Arc<dyn LedgerTransport> = Arc::<RunObserver>::clone(observer);
        app.manage(
            DesktopState::with_control_plane(identity(), store, registry).with_ledger(ledger),
        );
        app
    }

    async fn signed_in_to(marketplace: Marketplace) -> Arc<MemorySessionStore> {
        let store = Arc::new(MemorySessionStore::default());
        let record = SessionRecord {
            external_id: None,
            marketplace,
            account_label: Some("the seller".to_owned()),
            captured_at: NOW,
            device_id: identity().id,
            jar: CookieJar::new(vec![Cookie {
                name: "sessionKey".to_owned(),
                value: "s3cr3t".to_owned(),
            }]),
            verified_at: Some(NOW),
        };
        store
            .put(&record)
            .await
            .expect("the memory store keeps a session");
        store
    }

    /// A start the device cannot perform reaches the run, not only the window.
    ///
    /// The seller pressed Start on a phone that holds no Tes session. The
    /// device knows that before it composes a single marketplace request, so
    /// there is nothing slow or uncertain about the answer — and the run it
    /// was handed is the one page the seller looks at afterwards. A refusal
    /// that travels only in the command's rejection is lost the moment the
    /// console navigates, which is what the console does as soon as a start
    /// is accepted.
    ///
    /// Asserted on the transport rather than on what the command answered,
    /// deliberately: telling the pressing window is not wrong, it is
    /// insufficient, so the property is that the run was told and not that
    /// the window was not.
    #[tokio::test]
    async fn a_start_without_a_local_session_reports_to_the_run() {
        let observer = RunObserver::for_source(tam_types::InventoryId::Tes);
        // Signed in to the other no-API marketplace, so the state under test
        // is "no session for THIS shop" rather than "no sessions at all",
        // which is also the shape of the reported incident: a phone that had
        // been connected to something.
        let store = signed_in_to(Marketplace::Tpt).await;
        let app = app_with(store, &observer);

        let answered = super::start_import(app.handle().clone(), RUN, None).await;

        let about = observer.about(RUN).await;
        assert!(
            !about.is_empty(),
            "the run the server accepted must learn that this device refused it, or the console \
             shows `reading` forever while the only account of the refusal is a promise the \
             pressing window has already dropped. The device answered {answered:?} and posted \
             nothing naming the run"
        );
        let told = about.join("\n");
        assert!(
            told.contains("missing_session"),
            "and it must name the class the seller acts on — connecting this device — rather \
             than a sentence the console has to parse. Got: {told}"
        );
        drop(app);
    }

    /// A source this device cannot enumerate at all reaches the run too.
    ///
    /// The second early refusal on the same path, and the one that shows the
    /// first is about delivery rather than about sessions: here the device
    /// holds a session, and what it cannot do is read that kind of shop from
    /// a device at all. Both refusals happen before a page exists, both are
    /// durable facts about the run, and both were silent.
    #[tokio::test]
    async fn an_import_of_an_unsupported_source_reports_to_the_run() {
        let observer = RunObserver::for_source(tam_types::InventoryId::Etsy);
        let store = signed_in_to(Marketplace::Etsy).await;
        let app = app_with(store, &observer);

        let answered = super::start_import(app.handle().clone(), RUN, None).await;

        let about = observer.about(RUN).await;
        assert!(
            !about.is_empty(),
            "a run whose source no device can read must be failed on the server rather than \
             left open: nothing will ever claim it. The device answered {answered:?} and posted \
             nothing naming the run"
        );
        let told = about.join("\n");
        assert!(
            told.contains("unsupported_source"),
            "and the reason is that class rather than a missing session or a transport fault, \
             because the seller's remedy differs for each. Got: {told}"
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
            external_id: None,
            marketplace,
            account_label: None,
            captured_at: Timestamp(1_756_000_000_000),
            device_id: identity().id,
            jar: CookieJar::new(vec![Cookie {
                name: "sessionKey".to_owned(),
                value: "s3cr3t".to_owned(),
            }]),
            verified_at: Some(Timestamp(1_756_000_000_000)),
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

        fn consent_stands(&self, _marketplace: Marketplace) -> PlaneFuture<'_, bool> {
            Box::pin(core::future::ready(Ok(true)))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_run_facts(
            &self,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'_, crate::import::RunFacts> {
            Box::pin(core::future::ready(Ok(crate::import::RunFacts {
                source: tam_types::InventoryId::Tes,
                discovered: 0,
                processed: 0,
                described: 0,
                enumeration_complete: false,
            })))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a crate::device::DeviceId,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'a, Vec<String>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn open_import_runs<'a>(
            &'a self,
            _device: &'a crate::device::DeviceId,
        ) -> PlaneFuture<'a, Vec<crate::import::OpenImportRun>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
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
            "https://teachouse.io/",
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
            back, "https://teachouse.io/marketplaces?connect=abandoned&marketplace=Tpt",
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
            None,
        )
        .await
        .expect_err("a device signed out from the console does not keep what it just captured");
        assert!(
            matches!(refusal, super::NotFiled::SignedOut),
            "and it is named as a machine signed out from the console rather than as a store \
             that refused, because those are two sentences and two remedies. Got: \
             {}",
            match refusal {
                super::NotFiled::SignedOut => "signed out".to_owned(),
                super::NotFiled::BoundElsewhere => "the shop is another account's".to_owned(),
                super::NotFiled::Failed(why) => why.0,
            }
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

    /// The defect the founder met in 0.7.0, caught at the press instead of
    /// after it.
    ///
    /// Both of his machines had been signed out from the console, and every
    /// Connect took him through a whole marketplace sign-in — page, e-mail,
    /// password, second factor — before the check-in behind the capture said
    /// the session had not been saved. Nothing in the two surfaces asked
    /// beforehand, and both of them now do: the answer is an outcome the
    /// console can word, and the sign-in is not opened at all.
    ///
    /// Both surfaces in one loop, because the failure was one and the answer
    /// has to be one: a phone that returned `Opening` here would navigate to
    /// the marketplace anyway, and a computer that returned `Captured` would
    /// claim a session the next cycle wipes.
    #[tokio::test]
    async fn a_connect_on_a_machine_signed_out_from_the_console_opens_nothing() {
        for surface in [
            super::ConnectSurface::OneWindow,
            super::ConnectSurface::SecondWindow,
        ] {
            let store = Arc::new(MemorySessionStore::new());
            let plane = Recorder::answering(Ok(CheckIn {
                revoked: true,
                entitlement: None,
            }));
            let app = app_holding(Arc::clone(&store), Arc::clone(&plane));
            let window = a_console_window(&app);

            let answer = super::connect_on_target(app.handle().clone(), a_stub_target(), surface)
                .await
                .expect("a machine signed out from the console is an answer, not a failure");
            assert_eq!(
                serde_json::to_value(&answer).expect("an outcome serialises"),
                serde_json::json!({ "outcome": "signed_out" }),
                "the console reads the outcome word and words one sentence for it, so this is \
                 the whole channel between the refusal and what the seller is told. On \
                 {surface:?}"
            );
            assert_eq!(
                plane.beats().await.len(),
                1,
                "and the standing was asked of the server at the press rather than remembered \
                 from the last cycle, which is what makes the refusal current. On {surface:?}"
            );

            // Longer than `CONSOLE_HANDOVER`, so a navigation that was going
            // to happen has happened by now. Without the wait this would pass
            // for the phone's arm reading the answer and navigating anyway.
            tokio::time::sleep(core::time::Duration::from_millis(600)).await;
            assert_eq!(
                window.url().expect("the window has a url").as_str(),
                "https://teachouse.io/",
                "the one window never left the console, so the seller was never shown a login \
                 they cannot keep. On {surface:?}"
            );
            assert!(
                app.get_webview_window("login-Tpt").is_none(),
                "and no second window was built either. On {surface:?}"
            );
            drop(app);
        }
    }

    /// The check-in answer names which machine it came from, on both endings.
    ///
    /// The console's only channel for it: it lists the organisation's devices
    /// from the server and had no way to tell which row was the machine the
    /// page was drawn on, so the founder read "signed out" in a list of two
    /// and could not tell which of them it meant. Asserted as the whole
    /// document because these are field names a browser reads by spelling, and
    /// on both endings because a name dropped when the server is unreachable
    /// is a name missing exactly when the seller needs the sentence.
    #[test]
    fn the_check_in_answer_names_this_machine_on_both_endings() {
        let state = DesktopState::new(identity(), Arc::new(MemorySessionStore::default()));

        let reached = super::device_state(
            &state,
            Ok(CheckIn {
                revoked: true,
                entitlement: None,
            }),
        )
        .expect("an answer from the server is a state, revoked or not");
        assert_eq!(
            serde_json::to_value(&reached).expect("a device state serialises"),
            serde_json::json!({
                "revoked": true,
                "reached_server": true,
                "signed_in": true,
                "detail": null,
                "device_id": "11112222333344445555666677778888",
                "device_name": "founder-pc",
            }),
        );

        let unreached = super::device_state(
            &state,
            Err(crate::heartbeat::CheckInError::Plane(
                ControlPlaneError::NotConfigured,
            )),
        )
        .expect("a check-in that reached nothing is still a state");
        assert_eq!(unreached.device_id, "11112222333344445555666677778888");
        assert_eq!(
            unreached.device_name, "founder-pc",
            "the machine is the same machine whether or not the server answered"
        );
        assert!(!unreached.reached_server);
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
    /// unreachable on a host: `has_login_cookies` is false forever, so a capture
    /// never happens, and a deadline is ten minutes away.
    fn a_capture(
        poll: core::time::Duration,
        deadline: core::time::Duration,
        read: impl FnMut() -> Result<CookieJar, super::CommandError> + Send + 'static,
    ) -> super::Capture {
        super::Capture {
            poll,
            deadline,
            verification_interval: core::time::Duration::from_secs(5),
            read: Box::new(read),
            verify: Box::new(|_, _| {
                Box::pin(core::future::ready(Ok(super::Verified {
                    authenticated: true,
                    external_id: None,
                })))
            }),
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
            back, "https://teachouse.io/marketplaces?connect=captured&marketplace=Tpt",
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
            back, "https://teachouse.io/marketplaces?connect=deadline&marketplace=Tpt",
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

    #[tokio::test]
    async fn an_anonymous_tes_cookie_does_not_complete_the_sign_in() {
        let store = Arc::new(MemorySessionStore::new());
        let plane = Recorder::allowing();
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));
        let window = a_console_window(&app);
        let target = super::login_target(Marketplace::Tes).expect("TES signs in on the device");
        let mut capture = a_capture(
            core::time::Duration::from_millis(20),
            core::time::Duration::from_millis(100),
            || {
                Ok(CookieJar::new(vec![Cookie {
                    name: "TESSession".to_owned(),
                    value: "anonymous-session".to_owned(),
                }]))
            },
        );
        capture.verify =
            Box::new(|_, _| Box::pin(core::future::ready(Ok(super::Verified::default()))));
        super::capture_here(app.handle(), target, window.clone(), capture)
            .expect("the phone opens TES");

        let back = settles_at(&window, |at| at.contains("connect=")).await;
        assert_eq!(
            back,
            "https://teachouse.io/marketplaces?connect=deadline&marketplace=Tes"
        );
        assert!(store
            .get(Marketplace::Tes)
            .await
            .expect("store reads")
            .is_none());
        assert!(plane.beats().await.is_empty());
        drop(app);
    }

    #[tokio::test]
    async fn a_tes_login_can_activate_without_replacing_the_cookie() {
        let store = Arc::new(MemorySessionStore::new());
        let plane = Recorder::allowing();
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));
        let window = a_console_window(&app);
        let target = super::login_target(Marketplace::Tes).expect("TES signs in on the device");
        let mut capture = a_capture(
            core::time::Duration::from_millis(20),
            core::time::Duration::from_secs(5),
            || {
                Ok(CookieJar::new(vec![Cookie {
                    name: "TESSession".to_owned(),
                    value: "same-session".to_owned(),
                }]))
            },
        );
        capture.verification_interval = core::time::Duration::from_millis(20);
        let mut probes = 0;
        capture.verify = Box::new(move |_, _| {
            probes += 1;
            Box::pin(core::future::ready(Ok(super::Verified {
                authenticated: probes == 2,
                external_id: None,
            })))
        });
        super::capture_here(app.handle(), target, window.clone(), capture)
            .expect("the phone opens TES");

        let back = settles_at(&window, |at| at.contains("connect=")).await;
        assert_eq!(
            back,
            "https://teachouse.io/marketplaces?connect=captured&marketplace=Tes"
        );
        assert!(store
            .get(Marketplace::Tes)
            .await
            .expect("store reads")
            .is_some());
        drop(app);
    }

    #[tokio::test]
    async fn a_second_phone_login_cannot_overtake_the_first() {
        let store = Arc::new(MemorySessionStore::new());
        let plane = Recorder::allowing();
        let app = app_holding(Arc::clone(&store), Arc::clone(&plane));
        let window = a_console_window(&app);
        let never_verified = || {
            let mut capture = a_capture(
                core::time::Duration::from_millis(20),
                core::time::Duration::from_millis(100),
                || Ok(a_signed_in_jar()),
            );
            capture.verify =
                Box::new(|_, _| Box::pin(core::future::ready(Ok(super::Verified::default()))));
            capture
        };
        super::capture_here(
            app.handle(),
            a_stub_target(),
            window.clone(),
            never_verified(),
        )
        .expect("the first sign-in owns the phone");

        let refusal = super::capture_here(
            app.handle(),
            a_stub_target(),
            window.clone(),
            never_verified(),
        )
        .expect_err("one webview cannot hold two sign-ins");
        assert!(refusal.0.contains("already open on this phone"));
        let back = settles_at(&window, |at| at.contains("connect=")).await;
        assert!(back.contains("connect=deadline"));
        assert!(store
            .get(Marketplace::Tpt)
            .await
            .expect("store reads")
            .is_none());
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
    /// Nothing tested that check. The two tests above invoke from the
    /// console's own origin, so both pass unchanged if a second entry is added
    /// to `remote.urls`, if the pattern is widened to a wildcard host, or if
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
