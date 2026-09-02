//! The seller's marketplace session, as it exists on the seller's own device.
//!
//! This is the custody that section 5 of `docs/notes/design/client-side-architecture.md`
//! relocates rather than eliminates: the cookie jar the server used to hold in
//! `tam-session-broker` now lives in the operating system's keychain on the
//! machine that captured it, and no path in this crate sends it anywhere.
//!
//! Two properties are enforced here rather than remembered. The jar never
//! reaches a formatter, so no `Debug` line, panic message or log can carry it;
//! and the status type the interface reads is a separate struct that has no
//! field the jar could travel in.

pub mod keychain;
pub mod memory;

use core::future::Future;
use core::pin::Pin;

use serde::{Deserialize, Serialize};
use tam_types::{Marketplace, Timestamp};

use crate::device::DeviceId;

/// One cookie, as the webview's cookie store reported it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
}

impl core::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Cookie(redacted)")
    }
}

/// The cookies that constitute one logged-in marketplace session.
///
/// `Debug` prints a count and nothing else. Not even the names: a rule with
/// one exception is a rule somebody extends, and a name buys little that the
/// count and [`CookieJar::contains`] do not.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieJar(Vec<Cookie>);

impl core::fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CookieJar({} cookies, redacted)", self.0.len())
    }
}

impl CookieJar {
    #[must_use]
    pub fn new(cookies: Vec<Cookie>) -> Self {
        Self(cookies)
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|cookie| cookie.name == name)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The `name=value; name=value` string the marketplace adapters take, in
    /// jar order. The one method that yields the credential itself.
    #[must_use]
    pub fn header_value(&self) -> String {
        self.0
            .iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A captured session, as it is written to the keychain.
///
/// `Debug` is derived on every field but the jar, whose own `Debug` redacts,
/// so the derive is safe here and stays safe as fields are added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub marketplace: Marketplace,
    /// Whatever the marketplace made cheaply visible at capture time; `None`
    /// is normal and is never worth a second request to fill in.
    pub account_label: Option<String>,
    pub captured_at: Timestamp,
    pub device_id: DeviceId,
    pub jar: CookieJar,
}

/// What the interface is allowed to see about a session. Structurally unable
/// to carry the jar, which is why the command returns this rather than a
/// filtered [`SessionRecord`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub marketplace: Marketplace,
    pub connected: bool,
    pub account_label: Option<String>,
    pub captured_at: Option<Timestamp>,
    pub cookie_count: usize,
}

impl SessionStatus {
    #[must_use]
    pub fn disconnected(marketplace: Marketplace) -> Self {
        Self {
            marketplace,
            connected: false,
            account_label: None,
            captured_at: None,
            cookie_count: 0,
        }
    }

    #[must_use]
    pub fn of(record: &SessionRecord) -> Self {
        Self {
            marketplace: record.marketplace,
            connected: true,
            account_label: record.account_label.clone(),
            captured_at: Some(record.captured_at),
            cookie_count: record.jar.len(),
        }
    }
}

/// A store failure. The diagnostic is the backend's own message; a jar is
/// never interpolated into one, because this type is printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Backend(String),
    Codec(String),
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Backend(why) => write!(f, "the session store refused: {why}"),
            Self::Codec(why) => write!(f, "the stored session did not parse: {why}"),
        }
    }
}

impl core::error::Error for StoreError {}

/// One store operation, boxed so the trait stays object-safe. The same shape
/// `tam-api`'s `JwksSource` uses.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Where captured sessions live. One entry per marketplace, replaced rather
/// than accumulated, because a second login to the same marketplace supersedes
/// the first.
///
/// Asynchronous because two of the three platform backends are: Secret Service
/// is D-Bus and the in-memory double guards itself with a `tokio::sync::Mutex`,
/// the workspace's only sanctioned mutex.
pub trait SessionStore: Send + Sync {
    fn put<'a>(&'a self, record: &'a SessionRecord) -> StoreFuture<'a, ()>;
    fn get(&self, marketplace: Marketplace) -> StoreFuture<'_, Option<SessionRecord>>;
    fn forget(&self, marketplace: Marketplace) -> StoreFuture<'_, ()>;
}

/// The key one marketplace's entry is filed under. An exhaustive match rather
/// than the serde spelling, so adding a marketplace is a compile error here
/// rather than a silently different key.
#[must_use]
pub const fn entry_key(marketplace: Marketplace) -> &'static str {
    match marketplace {
        Marketplace::Tes => "session.tes",
        Marketplace::Etsy => "session.etsy",
        Marketplace::Tpt => "session.tpt",
    }
}

#[cfg(test)]
mod tests {
    use super::{Cookie, CookieJar, SessionRecord, SessionStatus};
    use crate::device::DeviceId;
    use tam_types::{Marketplace, Timestamp};

    pub(crate) fn a_jar() -> CookieJar {
        CookieJar::new(vec![
            Cookie {
                name: "csrfToken".to_owned(),
                value: "deadbeef".to_owned(),
            },
            Cookie {
                name: "sessionKey".to_owned(),
                value: "s3cr3t".to_owned(),
            },
        ])
    }

    pub(crate) fn a_record(marketplace: Marketplace) -> SessionRecord {
        SessionRecord {
            marketplace,
            account_label: Some("Founder's Classroom".to_owned()),
            captured_at: Timestamp(1_756_000_000),
            device_id: DeviceId::from_raw("11112222333344445555666677778888"),
            jar: a_jar(),
        }
    }

    #[test]
    fn debug_redacts_the_jar_and_every_cookie_in_it() {
        let record = a_record(Marketplace::Tpt);
        let printed = format!("{record:?}");
        assert!(
            !printed.contains("s3cr3t") && !printed.contains("deadbeef"),
            "a cookie value reached a formatter, which is the leak this type exists to prevent: \
             {printed}"
        );
        assert!(
            !printed.contains("csrfToken") && !printed.contains("sessionKey"),
            "a cookie name reached a formatter; the jar prints a count and nothing else: {printed}"
        );
        assert!(
            printed.contains("CookieJar(2 cookies, redacted)"),
            "the redaction should still say how much was held: {printed}"
        );
    }

    #[test]
    fn the_header_value_is_the_pairs_in_jar_order() {
        assert_eq!(
            a_jar().header_value(),
            "csrfToken=deadbeef; sessionKey=s3cr3t",
            "this is the string the marketplace adapters parse, so its shape is a contract"
        );
    }

    #[test]
    fn the_status_the_interface_reads_carries_no_cookie() {
        let record = a_record(Marketplace::Tes);
        let status = SessionStatus::of(&record);
        let printed = format!("{status:?}");
        assert!(
            !printed.contains("s3cr3t"),
            "the status type must be structurally unable to carry a credential: {printed}"
        );
        assert_eq!(
            status.cookie_count, 2,
            "the count is what the interface shows"
        );
        assert!(status.connected);
    }

    #[test]
    fn a_missing_session_reads_as_disconnected_rather_than_as_an_error() {
        let status = SessionStatus::disconnected(Marketplace::Tpt);
        assert!(!status.connected);
        assert_eq!(status.cookie_count, 0);
        assert_eq!(status.captured_at, None);
    }
}
