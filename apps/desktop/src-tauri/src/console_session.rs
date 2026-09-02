//! Where the console session comes from, and why it is read rather than held.
//!
//! Every control-plane call speaks as the seller, under the same session
//! cookie the console itself uses. That cookie is `HttpOnly`, so the console's
//! own JavaScript cannot read it and cannot hand it to us; the only way to
//! obtain it is from Rust, out of the main window's cookie store — the same
//! route `commands::connect_marketplace` already uses to read a marketplace
//! session out of the login window, and for the same reason.
//!
//! It is resolved per request rather than captured once. A device starts
//! before the seller signs in, the sign-in happens in the window after start,
//! and a session captured at construction would be missing forever. Resolving
//! per request also means the device stops speaking the moment the seller
//! signs out, without anything having to notice and clear a field.
//!
//! Nothing here stores the value. It is read, handed to one request, and
//! dropped.

use core::future::Future;
use core::pin::Pin;

/// The cookie the API reads the console session from, mirroring
/// `tam_api::SESSION_COOKIE`.
///
/// A literal rather than an import: depending on `tam-api` from here would
/// pull axum, sqlx and the entire server graph into a client binary. The
/// pg-gated `devices_flow.rs` pins the same literal on the server side, so a
/// rename fails there rather than silently unauthenticating this client.
pub const SESSION_COOKIE: &str = "tam_session";

/// Why a session could not be read. Distinct from there being none: a cookie
/// store we could not open is a fault, and no cookie in it is the ordinary
/// state of a device nobody has signed in on yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUnreadable(pub String);

impl core::fmt::Display for SessionUnreadable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the console session could not be read: {}", self.0)
    }
}

impl core::error::Error for SessionUnreadable {}

pub type SessionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<String>, SessionUnreadable>> + Send + 'a>>;

/// The console session this device speaks under, or `None` when nobody is
/// signed in on it.
pub trait SessionSource: Send + Sync {
    fn session(&self) -> SessionFuture<'_>;
}

/// A source with nothing in it: the device is not signed in, and never will be
/// through this source. The honest default for a build with no window.
#[derive(Debug, Default)]
pub struct NoSession;

impl SessionSource for NoSession {
    fn session(&self) -> SessionFuture<'_> {
        Box::pin(async { Ok(None) })
    }
}

#[cfg(test)]
mod tests {
    use super::{NoSession, SessionSource, SessionUnreadable};

    #[tokio::test]
    async fn a_source_with_nothing_in_it_is_not_signed_in_rather_than_broken() {
        assert_eq!(
            NoSession.session().await,
            Ok(None),
            "no session is the ordinary state of a device nobody has signed in on"
        );
    }

    #[test]
    fn an_unreadable_store_says_so_without_quoting_a_cookie() {
        let why = SessionUnreadable("the window is gone".to_owned()).to_string();
        assert!(why.contains("the window is gone"));
    }
}
