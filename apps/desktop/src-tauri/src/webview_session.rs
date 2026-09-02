//! The Tauri-backed [`SessionSource`]: the console's own session cookie, read
//! from the main window's cookie store.
//!
//! Separated from [`crate::console_session`] because this half cannot be
//! exercised without a running Tauri application, while the trait and
//! everything that consumes it can. The logic that decides what to do with a
//! present or absent session is therefore all on the testable side of the
//! line, and what lives here is one cookie-store read.
//!
//! `cookies_for_url` is called from an `async` context on purpose: it
//! deadlocks on Windows when called from a synchronous command or an event
//! handler, which is the same constraint the three console commands carry.

use tauri::{AppHandle, Manager};

use crate::console_session::{SessionFuture, SessionSource, SessionUnreadable, SESSION_COOKIE};

/// The window the console runs in. The one window
/// `capabilities/default.json` names, and the only one whose cookie store is
/// ours to read: a marketplace login window is deliberately absent from every
/// capability and its cookies belong to the marketplace.
pub const CONSOLE_WINDOW: &str = "main";

pub struct WebviewSession {
    app: AppHandle,
    /// The origin the console is served from, which is also the origin its
    /// session cookie is scoped to.
    origin: tauri::Url,
}

impl core::fmt::Debug for WebviewSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WebviewSession")
            .field("origin", &self.origin.as_str())
            .finish_non_exhaustive()
    }
}

impl WebviewSession {
    pub fn new(app: AppHandle, origin: &str) -> Result<Self, SessionUnreadable> {
        let origin = tauri::Url::parse(origin).map_err(|why| SessionUnreadable(why.to_string()))?;
        Ok(Self { app, origin })
    }
}

impl SessionSource for WebviewSession {
    fn session(&self) -> SessionFuture<'_> {
        Box::pin(async move {
            // A window that is not there yet is not a fault. The application
            // manages its state during setup, before the console has
            // finished loading, and a check-in that raced that would
            // otherwise report a broken cookie store rather than a device
            // nobody has signed in on.
            let Some(window) = self.app.get_webview_window(CONSOLE_WINDOW) else {
                return Ok(None);
            };
            let cookies = window
                .cookies_for_url(self.origin.clone())
                .map_err(|why| SessionUnreadable(why.to_string()))?;
            Ok(cookies
                .into_iter()
                .find(|cookie| cookie.name() == SESSION_COOKIE)
                .map(|cookie| cookie.value().to_owned()))
        })
    }
}
