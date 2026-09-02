//! The join between the seller's stored session and the transport the
//! marketplace adapters send through.
//!
//! Decision D1 puts every request to a no-API marketplace on this machine
//! under the seller's own session. The login webview captures that session and
//! files it in the operating system's keychain ([`crate::session`]); the
//! adapters that compose the requests are generic over
//! [`tam_marketplace::transport::Transport`] and hold no credential of their
//! own. This module is the one place the two meet, and therefore the one place
//! a stored jar becomes a live client.
//!
//! Three properties are enforced here rather than remembered.
//!
//! The jar is read from the store per request rather than captured once, so a
//! seller who signs in again is picked up at the next request instead of at
//! the next restart. The client itself is rebuilt only when the stored record
//! changed, because a client built per request would discard its connection
//! pool and its TLS session cache.
//!
//! A session-authenticated request may reach exactly one host: the origin
//! whose cookies it carries. The adapters' own live transports assert the same
//! rule from inside, and asserting it again here is not redundancy — it is
//! what lets a fake transport prove the rule in a unit test, and what refuses
//! a future transport that forgot.
//!
//! Nothing here reaches a formatter. The one type that could carry material
//! writes its own `Debug`, and the cache key is a capture instant and a count
//! rather than any function of a cookie.

use std::sync::Arc;

use tam_marketplace::transport::{HttpRequest, HttpResponse, Transport, TransportError};
use tam_marketplace::ConnectFailure;
use tam_types::{Marketplace, Timestamp};
use tokio::sync::Mutex;

use crate::connect::login_target;
use crate::session::{SessionStore, StoreError};

/// Why this device can hold no local transport for a marketplace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoLocalTransport {
    /// The marketplace publishes an official API, so its automation runs
    /// server-side under a sanctioned token and this device never holds a
    /// session for it. The same refusal [`crate::connect::login_target`]
    /// makes, restated at the transport so the two-branch rule holds on both
    /// halves of the login-then-send path.
    NotSellerDevice(Marketplace),
    /// The login target's cookie origin is not an absolute http(s) URL, so no
    /// host can be named to bind the session to. Unreachable with the
    /// compiled-in targets, and a compile-time constant away from being
    /// reachable, which is why it is a variant rather than an assumption.
    UnnamedOrigin(Marketplace),
}

impl core::fmt::Display for NoLocalTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::NotSellerDevice(marketplace) => write!(
                f,
                "{marketplace:?} publishes an official API, so no request for it originates on \
                 this device"
            ),
            Self::UnnamedOrigin(marketplace) => write!(
                f,
                "{marketplace:?}'s cookie origin names no host, so a session could not be bound \
                 to one"
            ),
        }
    }
}

impl core::error::Error for NoLocalTransport {}

/// Why a request could not be given the seller's session.
///
/// Every variant is a refusal before the network, which is what makes the
/// mapping onto [`TransportError::NotSent`] honest rather than convenient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTransportError {
    /// Nothing is stored for this marketplace on this device.
    NoSession(Marketplace),
    /// The keychain refused or the stored record did not parse.
    Store(StoreError),
    /// The stored jar could not be turned into a client. The diagnostic is
    /// the builder's own, and no builder interpolates a cookie into one.
    Unusable(String),
}

impl core::fmt::Display for SessionTransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoSession(marketplace) => write!(
                f,
                "this device holds no {marketplace:?} session, so it can compose no request for it"
            ),
            Self::Store(why) => write!(f, "{why}"),
            Self::Unusable(why) => write!(f, "the stored session is not usable: {why}"),
        }
    }
}

impl core::error::Error for SessionTransportError {}

/// The host of an absolute http(s) URL, without userinfo or port.
///
/// `None` for anything else, which every caller turns into a refusal: a
/// request whose destination cannot be named is not one to send. The same
/// parse the adapters' live transports each hold privately, repeated because
/// the assertion it feeds is made here as well as there.
#[must_use]
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    host.split(':').next()
}

/// How one marketplace's live client is built from the seller's cookies.
///
/// A trait with a receiver rather than a free function, so a test can
/// substitute a builder that records what it was handed. That is the only way
/// to observe cookie attachment without a socket: the shipping builders pass
/// the header to a session type that redacts it and then into a client whose
/// header map nothing can read back.
pub trait LiveTransport: Send + Sync {
    type Live: Transport;

    /// The marketplace whose stored session this builds from.
    fn marketplace(&self) -> Marketplace;

    /// Builds a client that will send `cookie_header` to that marketplace.
    ///
    /// The argument is the credential, so a failure states a reason and never
    /// a value.
    fn build(&self, cookie_header: &str) -> Result<Self::Live, String>;
}

/// Whether a cached client is still the stored one.
///
/// A capture instant and a cookie count rather than anything derived from the
/// jar. A session record is only ever replaced wholesale, by a capture that
/// stamps a fresh instant, so two records agreeing on both are the same
/// record; deriving the key from the cookies would put a function of the
/// credential in memory and buy nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Freshness {
    captured_at: Timestamp,
    cookies: usize,
}

struct Cached<L> {
    freshness: Freshness,
    live: Arc<L>,
}

/// The seller's session, as a [`Transport`] the adapters can take.
pub struct SessionTransport<B: LiveTransport> {
    builder: B,
    store: Arc<dyn SessionStore>,
    /// The one host a session-authenticated request may reach.
    origin_host: &'static str,
    cached: Mutex<Option<Cached<B::Live>>>,
}

impl<B: LiveTransport> core::fmt::Debug for SessionTransport<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Non-exhaustive rather than complete, and deliberately so: the cache
        // holds a client built from the seller's cookies, and the only safe
        // number of fields to print from it is none.
        f.debug_struct("SessionTransport")
            .field("marketplace", &self.builder.marketplace())
            .field("origin_host", &self.origin_host)
            .finish_non_exhaustive()
    }
}

impl<B: LiveTransport> SessionTransport<B> {
    /// Binds a builder to the store the login flow writes to.
    ///
    /// The origin comes from [`crate::connect::login_target`] rather than
    /// from the builder, so the host a session may reach is the same constant
    /// the login window captured cookies for. Two places naming it separately
    /// is how they come to disagree.
    pub fn new(builder: B, store: Arc<dyn SessionStore>) -> Result<Self, NoLocalTransport> {
        let marketplace = builder.marketplace();
        let target = login_target(marketplace)
            .map_err(|_| NoLocalTransport::NotSellerDevice(marketplace))?;
        let origin_host =
            host_of(target.cookie_origin).ok_or(NoLocalTransport::UnnamedOrigin(marketplace))?;
        Ok(Self {
            builder,
            store,
            origin_host,
            cached: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn marketplace(&self) -> Marketplace {
        self.builder.marketplace()
    }

    /// The one host this transport's session may be sent to.
    #[must_use]
    pub const fn origin_host(&self) -> &'static str {
        self.origin_host
    }

    /// Whether a session is stored for this marketplace at all.
    ///
    /// The scheduler consults it before running anything, so a device nobody
    /// has signed in on makes no marketplace request rather than one that is
    /// refused inside the transport.
    pub async fn has_session(&self) -> Result<bool, StoreError> {
        Ok(self.store.get(self.builder.marketplace()).await?.is_some())
    }

    /// Drops the cached client, so the next request rebuilds from the store.
    ///
    /// Called when the marketplace has said the session is no longer good.
    /// The freshness check below already picks up a record the seller
    /// replaced; this is for the other direction, where the record is
    /// unchanged and the cookies inside it have stopped working, and the
    /// seller re-signed-in against the same stored instant.
    pub async fn invalidate(&self) {
        *self.cached.lock().await = None;
    }

    /// The live client for the currently stored session, built if the stored
    /// record has changed since the last one was.
    async fn resolve(&self) -> Result<Arc<B::Live>, SessionTransportError> {
        let marketplace = self.builder.marketplace();
        let record = self
            .store
            .get(marketplace)
            .await
            .map_err(SessionTransportError::Store)?
            .ok_or(SessionTransportError::NoSession(marketplace))?;
        let freshness = Freshness {
            captured_at: record.captured_at,
            cookies: record.jar.len(),
        };

        let mut cached = self.cached.lock().await;
        if let Some(current) = cached.as_ref() {
            if current.freshness == freshness {
                return Ok(Arc::clone(&current.live));
            }
        }
        let live = Arc::new(
            self.builder
                .build(&record.jar.header_value())
                .map_err(SessionTransportError::Unusable)?,
        );
        *cached = Some(Cached {
            freshness,
            live: Arc::clone(&live),
        });
        drop(cached);
        Ok(live)
    }
}

impl<B: LiveTransport> Transport for SessionTransport<B> {
    /// Sends under the seller's own session, or not at all.
    ///
    /// A refusal here is always [`TransportError::NotSent`], and that is the
    /// load-bearing half: every path that returns one has provably not reached
    /// the network, so the caller may retry. The [`ConnectFailure`] variant is
    /// diagnostic only, exactly as the adapters' own transports treat theirs,
    /// and no detail travels with it because the seam has no field for one —
    /// the actionable cases are diagnosed by [`Self::has_session`] before a
    /// run starts rather than inside it.
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        if request.auth.is_session() {
            let host =
                host_of(&request.url).ok_or(TransportError::NotSent(ConnectFailure::DnsFailure))?;
            if host != self.origin_host {
                return Err(TransportError::NotSent(ConnectFailure::NoRouteToHost));
            }
        }
        let live = self
            .resolve()
            .await
            .map_err(|_| TransportError::NotSent(ConnectFailure::NoRouteToHost))?;
        live.send(request).await
    }
}

/// TeachersPayTeachers, over the adapter crate's own live client.
///
/// The client is that crate's rather than one written here, because its
/// request shaping is measured rather than incidental: the double-submit CSRF
/// header, the per-hop `sec-fetch` envelope and the browser identity are what
/// the captures recorded, and a second implementation of them would be a
/// second thing to keep true.
#[derive(Debug, Clone, Copy, Default)]
pub struct TptLive;

impl LiveTransport for TptLive {
    type Live = tam_marketplace_tpt::ReqwestTransport;

    fn marketplace(&self) -> Marketplace {
        Marketplace::Tpt
    }

    fn build(&self, cookie_header: &str) -> Result<Self::Live, String> {
        let session = tam_marketplace_tpt::TptSession::from_cookie_header(cookie_header.to_owned())
            .map_err(|why| why.to_string())?;
        tam_marketplace_tpt::ReqwestTransport::new(&session).map_err(|why| why.to_string())
    }
}

/// Tes, over the adapter crate's own live client, for the same reason.
#[derive(Debug, Clone, Copy, Default)]
pub struct TesLive;

impl LiveTransport for TesLive {
    type Live = tam_marketplace_tes::ReqwestTransport;

    fn marketplace(&self) -> Marketplace {
        Marketplace::Tes
    }

    fn build(&self, cookie_header: &str) -> Result<Self::Live, String> {
        let session = tam_marketplace_tes::TesSession::from_cookie_header(cookie_header.to_owned())
            .map_err(|why| why.to_string())?;
        tam_marketplace_tes::ReqwestTransport::new(&session).map_err(|why| why.to_string())
    }
}

/// The shipping TPT transport: the keychain jar, bound to TPT's own origin.
pub type TptTransport = SessionTransport<TptLive>;

/// The shipping Tes transport, likewise.
pub type TesTransport = SessionTransport<TesLive>;

#[cfg(test)]
mod tests {
    use super::{
        host_of, LiveTransport, NoLocalTransport, SessionTransport, SessionTransportError, TesLive,
        TptLive,
    };
    use crate::device::DeviceId;
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use std::sync::Arc;
    use tam_marketplace::transport::{
        HttpRequest, HttpResponse, RequestAuth, Transport, TransportError,
    };
    use tam_marketplace::ConnectFailure;
    use tam_types::{Marketplace, Timestamp};
    use tokio::sync::Mutex;

    const TPT_URL: &str = "https://www.teacherspayteachers.com/graph/graphql";
    const TES_URL: &str = "https://www.tes.com/api/v2/resources/1";
    const S3_URL: &str = "https://s3.amazonaws.com/tpt-bucket/object";

    /// What one build of a client was handed, and what that client was then
    /// asked to send. The whole point of the fake: the shipping clients
    /// redact their cookies and expose no header map, so this is where the
    /// attachment is observable at all.
    #[derive(Debug, Default)]
    struct Ledger {
        built_with: Mutex<Vec<String>>,
        sent: Mutex<Vec<(String, String)>>,
    }

    impl Ledger {
        async fn headers(&self) -> Vec<String> {
            self.built_with.lock().await.clone()
        }

        async fn sends(&self) -> Vec<(String, String)> {
            self.sent.lock().await.clone()
        }
    }

    struct Recording {
        ledger: Arc<Ledger>,
        cookie_header: String,
    }

    impl Transport for Recording {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.ledger
                .sent
                .lock()
                .await
                .push((request.url.clone(), self.cookie_header.clone()));
            Ok(HttpResponse::plain(200, b"{}".to_vec()))
        }
    }

    struct FakeLive {
        marketplace: Marketplace,
        ledger: Arc<Ledger>,
        refuse: bool,
    }

    impl FakeLive {
        fn for_marketplace(marketplace: Marketplace, ledger: &Arc<Ledger>) -> Self {
            Self {
                marketplace,
                ledger: Arc::clone(ledger),
                refuse: false,
            }
        }

        const fn refusing(mut self) -> Self {
            self.refuse = true;
            self
        }
    }

    impl LiveTransport for FakeLive {
        type Live = Recording;

        fn marketplace(&self) -> Marketplace {
            self.marketplace
        }

        fn build(&self, cookie_header: &str) -> Result<Self::Live, String> {
            // `try_lock` because `build` is synchronous by design -- the
            // shipping builders are -- and the fixture is single-threaded, so
            // contention here would be a defect in the test rather than a
            // condition to tolerate.
            self.ledger
                .built_with
                .try_lock()
                .expect("the fixture ledger is uncontended")
                .push(cookie_header.to_owned());
            if self.refuse {
                return Err("the jar carries no usable cookie".to_owned());
            }
            Ok(Recording {
                ledger: Arc::clone(&self.ledger),
                cookie_header: cookie_header.to_owned(),
            })
        }
    }

    fn jar(pairs: &[(&str, &str)]) -> CookieJar {
        CookieJar::new(
            pairs
                .iter()
                .map(|(name, value)| Cookie {
                    name: (*name).to_owned(),
                    value: (*value).to_owned(),
                })
                .collect(),
        )
    }

    fn record(marketplace: Marketplace, at: i64, jar: CookieJar) -> SessionRecord {
        SessionRecord {
            marketplace,
            account_label: None,
            captured_at: Timestamp(at),
            device_id: DeviceId::from_raw("11112222333344445555666677778888"),
            jar,
        }
    }

    fn tpt_record(at: i64, value: &str) -> SessionRecord {
        record(
            Marketplace::Tpt,
            at,
            jar(&[("csrfToken", "csrf-1"), ("sessionKey", value)]),
        )
    }

    async fn store_holding(records: &[SessionRecord]) -> Arc<dyn SessionStore> {
        let store = Arc::new(MemorySessionStore::new());
        for entry in records {
            store.put(entry).await.expect("the fixture store accepts");
        }
        store
    }

    #[tokio::test]
    async fn the_stored_jar_is_what_the_client_is_built_with() {
        let ledger = Arc::new(Ledger::default());
        let held = tpt_record(1_756_000_000_000, "s3cr3t");
        let store = store_holding(std::slice::from_ref(&held)).await;
        let bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tpt, &ledger), store)
                .expect("tpt is a seller-device marketplace");

        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the request is sent");

        assert_eq!(
            ledger.headers().await,
            vec![held.jar.header_value()],
            "the adapters must send the seller's own session, verbatim as the store holds it"
        );
        assert_eq!(
            ledger.sends().await,
            vec![(TPT_URL.to_owned(), held.jar.header_value())],
            "and that session is what reaches the marketplace's own origin"
        );
    }

    #[tokio::test]
    async fn a_session_request_to_another_origin_never_reaches_the_transport() {
        let ledger = Arc::new(Ledger::default());
        let store = store_holding(&[tpt_record(1_756_000_000_000, "s3cr3t")]).await;
        let bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tpt, &ledger), store)
                .expect("tpt is a seller-device marketplace");

        assert_eq!(
            bridge.send(HttpRequest::get(TES_URL.to_owned())).await,
            Err(TransportError::NotSent(ConnectFailure::NoRouteToHost)),
            "the seller's TPT session must never be offered to another marketplace"
        );
        assert_eq!(
            bridge
                .send(HttpRequest::get("https://evil.example/collect".to_owned()))
                .await,
            Err(TransportError::NotSent(ConnectFailure::NoRouteToHost)),
            "nor to anywhere else"
        );
        assert_eq!(
            bridge
                .send(HttpRequest::get("teacherspayteachers.com/x".to_owned()))
                .await,
            Err(TransportError::NotSent(ConnectFailure::DnsFailure)),
            "a destination that cannot be named is not one to send to"
        );
        assert!(
            ledger.sends().await.is_empty(),
            "the refusal must come before the client, not after it"
        );
        assert!(
            ledger.headers().await.is_empty(),
            "and before a client carrying the jar is built at all"
        );
    }

    #[tokio::test]
    async fn a_request_that_is_not_session_authenticated_is_not_bound_to_that_origin() {
        let ledger = Arc::new(Ledger::default());
        let store = store_holding(&[tpt_record(1_756_000_000_000, "s3cr3t")]).await;
        let bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tpt, &ledger), store)
                .expect("tpt is a seller-device marketplace");

        let upload = HttpRequest::post_multipart(
            S3_URL.to_owned(),
            vec![("key".to_owned(), "object".to_owned())],
            None,
            RequestAuth::Anonymous,
        );
        bridge.send(upload).await.expect("the upload is delegated");

        assert_eq!(
            ledger.sends().await.len(),
            1,
            "the upload hop is signed in its own body and goes to the bucket, so binding it to \
             the marketplace origin would break the create rather than protect anything"
        );
    }

    #[tokio::test]
    async fn one_marketplaces_session_never_builds_anothers_client() {
        let ledger = Arc::new(Ledger::default());
        let tpt = tpt_record(1_756_000_000_000, "tpt-secret");
        let tes = record(
            Marketplace::Tes,
            1_756_000_000_001,
            jar(&[("TESSession", "tes-secret")]),
        );
        let store = store_holding(&[tpt.clone(), tes.clone()]).await;

        let tpt_bridge = SessionTransport::new(
            FakeLive::for_marketplace(Marketplace::Tpt, &ledger),
            Arc::clone(&store),
        )
        .expect("tpt is a seller-device marketplace");
        let tes_bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tes, &ledger), store)
                .expect("tes is a seller-device marketplace");

        tpt_bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the tpt request is sent");
        tes_bridge
            .send(HttpRequest::get(TES_URL.to_owned()))
            .await
            .expect("the tes request is sent");

        assert_eq!(
            ledger.sends().await,
            vec![
                (TPT_URL.to_owned(), tpt.jar.header_value()),
                (TES_URL.to_owned(), tes.jar.header_value()),
            ],
            "each origin receives only the session captured for it; the store is keyed on the \
             marketplace and the lookup never widens"
        );
    }

    #[tokio::test]
    async fn a_replaced_record_is_picked_up_without_a_restart() {
        let ledger = Arc::new(Ledger::default());
        let first = tpt_record(1_756_000_000_000, "expired");
        let store = store_holding(std::slice::from_ref(&first)).await;
        let bridge = SessionTransport::new(
            FakeLive::for_marketplace(Marketplace::Tpt, &ledger),
            Arc::clone(&store),
        )
        .expect("tpt is a seller-device marketplace");

        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the first request is sent");
        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the second request is sent");
        assert_eq!(
            ledger.headers().await.len(),
            1,
            "an unchanged record reuses its client, or every request would discard the \
             connection pool"
        );

        let renewed = tpt_record(1_756_000_600_000, "fresh");
        store
            .put(&renewed)
            .await
            .expect("the seller signs in again");
        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the third request is sent");

        assert_eq!(
            ledger.headers().await,
            vec![first.jar.header_value(), renewed.jar.header_value()],
            "a cookie that expired is replaced by signing in again, and the very next request \
             must carry the new jar rather than waiting for a restart"
        );
    }

    #[tokio::test]
    async fn invalidating_rebuilds_from_the_store_even_when_the_record_is_unchanged() {
        let ledger = Arc::new(Ledger::default());
        let store = store_holding(&[tpt_record(1_756_000_000_000, "s3cr3t")]).await;
        let bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tpt, &ledger), store)
                .expect("tpt is a seller-device marketplace");

        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the first request is sent");
        bridge.invalidate().await;
        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the second request is sent");

        assert_eq!(
            ledger.headers().await.len(),
            2,
            "a marketplace that rejected the session must be re-read from the store rather than \
             retried through a client we already know it refuses"
        );
    }

    #[tokio::test]
    async fn a_device_with_no_stored_session_makes_no_request() {
        let ledger = Arc::new(Ledger::default());
        let store = store_holding(&[]).await;
        let bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tpt, &ledger), store)
                .expect("tpt is a seller-device marketplace");

        assert!(
            !bridge.has_session().await.expect("the store answers"),
            "the scheduler asks this before it runs anything"
        );
        assert_eq!(
            bridge.send(HttpRequest::get(TPT_URL.to_owned())).await,
            Err(TransportError::NotSent(ConnectFailure::NoRouteToHost)),
            "and a request that slipped through is refused before the network rather than sent \
             unauthenticated"
        );
        assert!(ledger.sends().await.is_empty());
    }

    #[tokio::test]
    async fn a_jar_the_builder_refuses_produces_no_request_and_quotes_no_cookie() {
        let ledger = Arc::new(Ledger::default());
        let store = store_holding(&[tpt_record(1_756_000_000_000, "s3cr3t")]).await;
        let bridge = SessionTransport::new(
            FakeLive::for_marketplace(Marketplace::Tpt, &ledger).refusing(),
            store,
        )
        .expect("tpt is a seller-device marketplace");

        assert_eq!(
            bridge.send(HttpRequest::get(TPT_URL.to_owned())).await,
            Err(TransportError::NotSent(ConnectFailure::NoRouteToHost))
        );
        assert!(ledger.sends().await.is_empty());
    }

    #[tokio::test]
    async fn nothing_this_module_prints_carries_a_cookie() {
        let ledger = Arc::new(Ledger::default());
        let store = store_holding(&[tpt_record(1_756_000_000_000, "s3cr3t")]).await;
        let bridge =
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Tpt, &ledger), store)
                .expect("tpt is a seller-device marketplace");
        bridge
            .send(HttpRequest::get(TPT_URL.to_owned()))
            .await
            .expect("the request is sent");

        let printed = format!("{bridge:?}");
        assert!(
            !printed.contains("s3cr3t") && !printed.contains("csrf-1"),
            "a client built from the seller's jar must not print one: {printed}"
        );

        let refusals = [
            SessionTransportError::NoSession(Marketplace::Tpt).to_string(),
            SessionTransportError::Unusable(
                TptLive
                    .build("sessionKey=s3cr3t")
                    .err()
                    .expect("a jar with no csrfToken is refused"),
            )
            .to_string(),
        ];
        for refusal in refusals {
            assert!(
                !refusal.contains("s3cr3t"),
                "a refusal must state a reason and never a value: {refusal}"
            );
        }
    }

    #[test]
    fn a_local_transport_exists_for_exactly_the_seller_device_marketplaces() {
        let store: Arc<dyn SessionStore> = Arc::new(MemorySessionStore::new());
        assert_eq!(
            SessionTransport::new(TptLive, Arc::clone(&store))
                .expect("tpt has a local transport")
                .origin_host(),
            "www.teacherspayteachers.com"
        );
        assert_eq!(
            SessionTransport::new(TesLive, Arc::clone(&store))
                .expect("tes has a local transport")
                .origin_host(),
            "www.tes.com"
        );

        let ledger = Arc::new(Ledger::default());
        assert_eq!(
            SessionTransport::new(FakeLive::for_marketplace(Marketplace::Etsy, &ledger), store)
                .err(),
            Some(NoLocalTransport::NotSellerDevice(Marketplace::Etsy)),
            "Etsy's automation is sanctioned and runs server-side; a local transport for it \
             would put the request on the wrong machine"
        );
    }

    #[test]
    fn the_shipping_builders_refuse_a_jar_they_cannot_authorise() {
        assert!(
            TptLive.build("sessionKey=only").is_err(),
            "TPT mirrors csrfToken into a header and can authorise nothing without it"
        );
        assert!(TptLive.build("csrfToken=t; sessionKey=s").is_ok());
        assert!(
            TesLive.build("   ").is_err(),
            "an empty jar is not a session"
        );
        assert!(TesLive.build("TESSession=s").is_ok());
    }

    #[test]
    fn a_host_is_read_off_an_absolute_url_and_nothing_else() {
        assert_eq!(host_of("https://www.tes.com/a/b?c#d"), Some("www.tes.com"));
        assert_eq!(host_of("http://host:8080/a"), Some("host"));
        assert_eq!(host_of("https://user@host/a"), Some("host"));
        assert_eq!(host_of("/relative"), None);
        assert_eq!(host_of("ftp://host/a"), None);
    }
}
