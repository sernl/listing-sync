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

// `keyring` 3.6.3 has no Android backend, and this crate's manifest declares
// it only for the three platforms that do, so on Android the module below has
// no crate to reach. `encrypted` is what holds the jar there instead.
#[cfg(not(target_os = "android"))]
pub mod keychain;
// The Keystore-backed key source, which only Android has. The store it feeds
// is portable and compiled everywhere; this half is the one platform-bound
// file, and it is deliberately as thin as a file can be.
#[cfg(target_os = "android")]
pub mod android_key;
pub mod encrypted;
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

    /// The jar a `name=value; name=value` header describes.
    ///
    /// The inverse of [`Self::header_value`], and it exists because a
    /// marketplace that rotates its session hands the rotated cookies back as
    /// a header rather than as a jar: this is how the live transport's current
    /// session becomes something the store can hold. Anything that names
    /// nothing is dropped rather than stored as a nameless cookie.
    #[must_use]
    pub fn from_header_value(header: &str) -> Self {
        Self(
            header
                .split(';')
                .filter_map(|element| {
                    let (name, value) = element.trim().split_once('=')?;
                    let name = name.trim();
                    (!name.is_empty()).then(|| Cookie {
                        name: name.to_owned(),
                        value: value.trim().to_owned(),
                    })
                })
                .collect(),
        )
    }
}

/// How long a session's last proof of life counts for.
///
/// Longer than the five-minute check-in that renews it, so a beat the network
/// ate does not read as a lapse, and far shorter than the few hours a Tes
/// session survived unattended: the whole point is that a session nobody has
/// proven recently is reported as what it is rather than as connected.
pub const PROOF_MAX_AGE: core::time::Duration = core::time::Duration::from_mins(15);

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
    /// When the marketplace itself last answered these cookies as an
    /// authenticated principal, and `None` when it last refused them or has
    /// never been asked.
    ///
    /// Distinct from `captured_at`, and the distinction is the defect it
    /// closes: a stored jar is evidence that somebody signed in once, not
    /// evidence that the session still works. A device that read presence as
    /// connection reported a jar Tes had stopped accepting as `connected`
    /// every five minutes, so the server re-linked the connection and every
    /// claim behind it burned an item.
    ///
    /// `#[serde(default)]` so a record written before this field existed
    /// parses, as an unproven one — which is the honest reading of a jar
    /// nothing has ever verified.
    #[serde(default)]
    pub verified_at: Option<Timestamp>,
    /// The marketplace's own identifier for the storefront these cookies
    /// speak for: Tes's `userId`, TPT's `author.id`.
    ///
    /// Read from a route that names its principal and takes no selector, so
    /// it is the marketplace's assertion about whose shop this is rather than
    /// anything the seller typed. It travels to the server on every check-in,
    /// which digests it and keeps only the digest; it is what makes one
    /// storefront belong to one account.
    ///
    /// `#[serde(default)]` so a record written before this field existed
    /// parses, claiming no storefront -- which is the honest reading of a jar
    /// nothing has ever asked the question of.
    #[serde(default)]
    pub external_id: Option<String>,
}

impl SessionRecord {
    /// Whether the marketplace has proven this session inside
    /// [`PROOF_MAX_AGE`] of `now`.
    ///
    /// A proof stamped in the future is not one: a clock correction must not
    /// hand a lapsed session an indefinite reprieve.
    #[must_use]
    pub fn proven_at(&self, now: Timestamp) -> bool {
        let Some(verified) = self.verified_at else {
            return false;
        };
        let age = now.0.saturating_sub(verified.0);
        let max = i64::try_from(PROOF_MAX_AGE.as_millis()).unwrap_or(i64::MAX);
        (0..=max).contains(&age)
    }
}

/// What the interface is allowed to see about a session. Structurally unable
/// to carry the jar, which is why the command returns this rather than a
/// filtered [`SessionRecord`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub marketplace: Marketplace,
    /// Whether the marketplace still accepts this session, rather than
    /// whether a jar is held for it. The two came apart in production: a
    /// lapsed Tes session was still stored, still shown as connected, and
    /// refused every call.
    pub connected: bool,
    pub account_label: Option<String>,
    pub captured_at: Option<Timestamp>,
    pub cookie_count: usize,
    /// When the marketplace last answered these cookies as the seller.
    ///
    /// Carried so the console can say which it is: a session nobody has
    /// proven in a quarter of an hour reads as needing a fresh sign-in, and
    /// the seller is told that rather than left to infer it from work that
    /// stopped.
    pub verified_at: Option<Timestamp>,
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
            verified_at: None,
        }
    }

    /// A held session as of `now`, which is what decides whether its last
    /// proof still counts.
    #[must_use]
    pub fn of(record: &SessionRecord, now: Timestamp) -> Self {
        Self {
            marketplace: record.marketplace,
            connected: record.proven_at(now),
            account_label: record.account_label.clone(),
            captured_at: Some(record.captured_at),
            cookie_count: record.jar.len(),
            verified_at: record.verified_at,
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
    use super::{Cookie, CookieJar, SessionRecord, SessionStatus, PROOF_MAX_AGE};
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
            external_id: None,
            marketplace,
            account_label: Some("Founder's Classroom".to_owned()),
            captured_at: Timestamp(1_756_000_000),
            device_id: DeviceId::from_raw("11112222333344445555666677778888"),
            jar: a_jar(),
            verified_at: Some(Timestamp(1_756_000_000)),
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
        let status = SessionStatus::of(&record, record.captured_at);
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

    /// The defect, stated as the smallest thing that used to be wrong: a jar
    /// is held, and the marketplace has not answered it for hours.
    #[test]
    fn a_session_nobody_has_proven_lately_is_not_connected_merely_because_it_is_held() {
        let record = a_record(Marketplace::Tes);
        let four_hours_on = Timestamp(record.captured_at.0 + 4 * 60 * 60 * 1_000);
        assert!(
            !record.proven_at(four_hours_on),
            "a Tes session lived about four hours; a device that read presence as connection \
             reported this one as connected and burned an item on every claim behind it"
        );
        let status = SessionStatus::of(&record, four_hours_on);
        assert!(
            !status.connected,
            "and the interface says so rather than showing the seller a working connection"
        );
        assert_eq!(
            status.cookie_count, 2,
            "the jar is kept: the seller may be about to sign in again, and a rotation could \
             still be stored against it"
        );
    }

    #[test]
    fn a_proof_holds_for_its_whole_window_and_not_one_millisecond_past_it() {
        let record = a_record(Marketplace::Tes);
        let at = record.captured_at.0;
        let window = i64::try_from(PROOF_MAX_AGE.as_millis()).expect("the window fits");
        assert!(record.proven_at(Timestamp(at + window)), "the edge counts");
        assert!(!record.proven_at(Timestamp(at + window + 1)));
    }

    /// A clock correction must not hand a lapsed session an open-ended
    /// reprieve: a proof stamped after the instant being asked about is not
    /// one.
    #[test]
    fn a_proof_from_the_future_does_not_count() {
        let record = a_record(Marketplace::Tes);
        assert!(!record.proven_at(Timestamp(record.captured_at.0 - 1)));
    }

    /// A record written before the field existed parses as unproven, which is
    /// the honest reading of a jar nothing has ever verified — and it must
    /// parse rather than be rejected, or an upgrade would silently lose every
    /// stored session.
    #[test]
    fn a_record_stored_before_proofs_existed_parses_as_unproven() {
        // Serialised from a real record and then stripped, rather than
        // hand-written: a literal would pin this test to whatever spelling
        // the fields happen to have today instead of to the field's absence,
        // which is the only thing under test.
        let mut stored =
            serde_json::to_value(a_record(Marketplace::Tes)).expect("a record serialises");
        stored
            .as_object_mut()
            .expect("a record is an object")
            .remove("verified_at")
            .expect("the field was there to remove");
        let record: SessionRecord =
            serde_json::from_value(stored).expect("a record without the field still parses");
        assert_eq!(record.verified_at, None);
        assert!(!record.proven_at(Timestamp(1_756_000_000)));
    }

    /// The round trip the rotation write-back depends on: what the live
    /// transport hands back as a header is what the store holds as a jar.
    #[test]
    fn a_header_round_trips_through_the_jar_it_describes() {
        let jar = CookieJar::from_header_value("csrfToken=deadbeef; sessionKey=s3cr3t");
        assert_eq!(jar.len(), 2);
        assert_eq!(jar.header_value(), "csrfToken=deadbeef; sessionKey=s3cr3t");
        assert!(jar.contains("sessionKey"));
    }

    #[test]
    fn a_header_element_that_names_nothing_is_dropped_rather_than_stored() {
        let jar = CookieJar::from_header_value("  ; TESSession=v ; =orphan; ");
        assert_eq!(jar.header_value(), "TESSession=v");
    }

    #[test]
    fn a_missing_session_reads_as_disconnected_rather_than_as_an_error() {
        let status = SessionStatus::disconnected(Marketplace::Tpt);
        assert!(!status.connected);
        assert_eq!(status.cookie_count, 0);
        assert_eq!(status.captured_at, None);
    }
}
