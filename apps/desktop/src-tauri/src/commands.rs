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
    Ok(SessionStatus::of(&record))
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
