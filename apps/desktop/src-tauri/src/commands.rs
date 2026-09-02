//! The three commands the console calls, and the login webview behind the
//! first of them.
//!
//! One rule governs the login window: the marketplace page gets no capability
//! at all. It is not listed in `capabilities/default.json`, so `invoke` is
//! unreachable from it even before the site's own content-security policy
//! blocks `ipc.localhost`. Everything this module learns about the login it
//! learns by reading the webview's cookie store from Rust.

use core::time::Duration;

use serde::Serialize;
use tam_types::{Marketplace, Timestamp};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::connect::login_target;
use crate::heartbeat::{check_in, first_run, CheckInError};
use crate::session::{Cookie, CookieJar, SessionRecord, SessionStatus};
use crate::state::DesktopState;

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
        }),
        Err(CheckInError::Plane(_)) => Ok(DeviceState {
            revoked: state.revoked(),
            reached_server: false,
        }),
        Err(why) => Err(CommandError::from(why)),
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

/// The client is a clock-reading process boundary in the same sense the
/// serving binary is: time enters the record as data from here. A clock before
/// the epoch saturates to zero, which reads as "captured at the epoch" rather
/// than as something stranger.
#[expect(
    clippy::disallowed_methods,
    reason = "the desktop client is a clock-reading process boundary; time enters the session record as data from here"
)]
fn wall_now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(i64::try_from(millis).unwrap_or(0))
}
