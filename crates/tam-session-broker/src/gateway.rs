//! The lease gateway: the JSON-API-era form of the design's "driver endpoint
//! for a browser the broker primed". Per lease the broker binds an ephemeral
//! loopback listener and proxies allow-listed upstream routes with the seller
//! cookie injected server-side, so a worker uses a connection and cannot read
//! one. The allow-list is enforced here in Rust — a route outside it is 403,
//! by a test rather than a review — which makes roster, account-administration
//! and payout routes structurally unreachable.
//!
//! Two properties beyond proxying live here, and both are custody rather than
//! plumbing. A lease presents a bearer token or is refused before its path is
//! even matched, because a loopback listener is reachable by every process on
//! the host and an untrusted one would otherwise ride the seller's session for
//! the lease's whole term. And the upstream's `Set-Cookie` renewals are
//! absorbed into the lease's jar and resealed into the vault, because a
//! forwarded-and-discarded renewal leaves the stored session ageing on the
//! original cookie's schedule however often it is used.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use tam_secrets::Secret;
use tam_types::Marketplace;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::jar::CookieJar;

/// The Tes prefixes a lease may reach, and nothing else. Adding one is a
/// deliberate edit here, guarded by the refusal test. The middle three are the
/// first-party reads — the seller's own catalogue and the two-step published
/// bundle download — distinct from the write paths above them, and the last is
/// the session-renewal route the longevity probe drives.
const TES_PREFIXES: &[&str] = &[
    "/api/v2/resources",
    "/api/resources/v3/draft",
    "/api/v2/dashboard",
    "/resource-detail/api/download",
    "/teaching-resource/download",
    TES_REFRESH_ROUTE,
];

/// The Tpt prefixes a lease may reach: the two GraphQL services, the two
/// product forms that are also their own submit targets, the five upload and
/// queue hops, and the seller's own bundle download. The bucket hops are
/// absent deliberately — they carry an S3 signature rather than the session,
/// the live transport already refuses a session-authenticated request to the
/// bucket, and routing them through a cookie-injecting proxy would send the
/// seller's Tpt session to Amazon.
const TPT_PREFIXES: &[&str] = &[
    "/graph/graphql",
    "/gateway/graphql",
    "/My-Products/New/Digital-Next",
    "/itemsDigital/editNext",
    "/uploads/upload_file",
    "/uploads/time",
    "/uploads/sign_auth",
    "/uploads/process_file",
    "/queue/results",
    "/converter/generate_thumbs",
    "/Download",
];

/// Tes renews a session on this route, and the longevity probe's four-and-a-
/// half-day run is entirely what it buys: an hourly hit here with cookie
/// persistence kept the session authenticated where the same account's
/// non-renewing sealed copy expired inside a day.
pub(crate) const TES_REFRESH_ROUTE: &str = "/api/authn/refresh-cookies";

/// Tpt's cookie whose value is mirrored into `x-csrf-token`; the double submit
/// Tpt validates. The worker cannot compute this — it never sees the jar — so
/// the gateway mirrors it on the worker's behalf.
const TPT_CSRF_COOKIE: &str = "csrfToken";
const TPT_CSRF_HEADER: &str = "x-csrf-token";

/// The routes a lease may reach on this marketplace.
const fn allowed_prefixes(marketplace: Marketplace) -> &'static [&'static str] {
    match marketplace {
        Marketplace::Tes => TES_PREFIXES,
        Marketplace::Tpt => TPT_PREFIXES,
        // No Etsy connector exists, so an Etsy lease may reach nothing.
        Marketplace::Etsy => &[],
    }
}

/// The route that renews a session without performing any other work, where
/// the marketplace has one.
///
/// Tpt has none: no capture of that platform shows a renewal endpoint, and its
/// sessions sit behind Cloudflare, so `None` here is the observed state rather
/// than an omission to be filled in by guessing at a path.
pub(crate) const fn refresh_route(marketplace: Marketplace) -> Option<&'static str> {
    match marketplace {
        Marketplace::Tes => Some(TES_REFRESH_ROUTE),
        Marketplace::Tpt | Marketplace::Etsy => None,
    }
}

fn path_is_allowed(marketplace: Marketplace, path: &str) -> bool {
    allowed_prefixes(marketplace)
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

/// Headers the worker may not dictate, stripped from every forwarded request.
///
/// `cookie` and `authorization` are the custody boundary itself: the gateway
/// supplies the first and consumes the second, and a worker that could set
/// either would be choosing its own credential. The rest are hop-by-hop or
/// framing headers that belong to this connection rather than the upstream's.
const REQUEST_HEADER_DENY: &[&str] = &[
    "cookie",
    "authorization",
    // Derived from the jar, which only this process holds. A forwarded copy
    // would ride alongside the one the gateway mirrors and put two
    // `x-csrf-token` headers on a double-submit check.
    TPT_CSRF_HEADER,
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Headers never relayed back. `set-cookie` heads the list and is the point:
/// the renewals it carries are absorbed into the jar and resealed, and handing
/// them to the worker would give away exactly the session material the whole
/// gateway exists to withhold.
const RESPONSE_HEADER_DENY: &[&str] = &[
    "set-cookie",
    "set-cookie2",
    "content-length",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn denied(deny: &[&str], name: &HeaderName) -> bool {
    deny.iter().any(|banned| name.as_str() == *banned)
}

/// Where a renewed jar goes when the upstream changes it.
///
/// A trait rather than the vault itself so that the capture-and-reseal
/// decision is provable without a database: the gateway's own tests drive a
/// recording sink, and the sealing half is proved separately against the
/// broker role.
pub(crate) trait SessionSink: Send + Sync {
    fn reseal<'a>(
        &'a self,
        cookie_header: String,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + 'a>>;
}

struct GatewayState {
    upstream_base: String,
    marketplace: Marketplace,
    token: String,
    jar: Mutex<CookieJar>,
    sink: Arc<dyn SessionSink>,
    client: reqwest::Client,
}

/// A bound gateway: its loopback address, the token a caller must present, and
/// the token that aborts it. The task drops when the cancellation token fires,
/// closing the listener.
pub(crate) struct Gateway {
    pub(crate) endpoint: String,
    pub(crate) token: String,
    pub(crate) cancel: CancellationToken,
}

#[derive(Debug)]
pub(crate) struct GatewayError(pub String);

impl core::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "gateway: {}", self.0)
    }
}

impl core::error::Error for GatewayError {}

/// The client every proxied hop rides. Built here rather than taken as an
/// argument so the mandatory timeouts are in one place.
fn proxy_client() -> Result<reqwest::Client, GatewayError> {
    reqwest::Client::builder()
        .timeout(core::time::Duration::from_secs(30))
        .connect_timeout(core::time::Duration::from_secs(10))
        // The Tpt submits answer 302 with the product id in `Location` and an
        // empty body; following the redirect would discard the identifier
        // before the worker ever sees it.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| GatewayError(error.to_string()))
}

/// What one lease's gateway needs to exist: where it proxies, which
/// marketplace's allow-list applies, the credential it injects, and where a
/// renewal of that credential goes.
pub(crate) struct LeaseGateway {
    pub(crate) upstream_base: String,
    pub(crate) marketplace: Marketplace,
    pub(crate) cookie: Secret,
    pub(crate) sink: Arc<dyn SessionSink>,
}

/// Binds an ephemeral loopback gateway proxying to the lease's upstream with
/// its cookie injected. Returns once bound; serves until the token is
/// cancelled.
pub(crate) async fn spawn(
    lease: LeaseGateway,
    parent: &CancellationToken,
) -> Result<Gateway, GatewayError> {
    let LeaseGateway {
        upstream_base,
        marketplace,
        cookie,
        sink,
    } = lease;
    let token = tam_secrets::random_token();
    let state = Arc::new(GatewayState {
        upstream_base,
        marketplace,
        token: token.clone(),
        jar: Mutex::new(CookieJar::from_cookie_header(cookie.expose())),
        sink,
        client: proxy_client()?,
    });
    let router = Router::new().fallback(any(proxy)).with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| GatewayError(error.to_string()))?;
    let local = listener
        .local_addr()
        .map_err(|error| GatewayError(error.to_string()))?;
    let cancel = parent.child_token();
    let serve_cancel = cancel.clone();
    // The one supervised task the broker spawns; the disallowed spawn lint
    // targets the worker pool's fire-and-forget, not this owned lease task.
    #[expect(
        clippy::disallowed_methods,
        reason = "the gateway task is owned by the lease and aborted by its cancellation token, not a fire-and-forget spawn"
    )]
    tokio::spawn(async move {
        let shutdown = async move { serve_cancel.cancelled().await };
        let _served = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await;
    });
    Ok(Gateway {
        endpoint: format!("http://{local}"),
        token,
        cancel,
    })
}

/// Whether the request carries this lease's bearer token.
///
/// Compared over the whole string rather than short-circuiting on the first
/// differing byte, so the check does not leak the token's prefix by timing to
/// a local process that can retry without limit.
fn presents_token(headers: &HeaderMap, expected: &str) -> bool {
    let Some(offered) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    if offered.len() != expected.len() {
        return false;
    }
    offered
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

async fn proxy(State(state): State<Arc<GatewayState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    // Authentication precedes routing: an unauthenticated caller learns
    // nothing about which paths this lease would have served.
    if !presents_token(&parts.headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            "this lease requires its bearer token",
        )
            .into_response();
    }
    let path = parts.uri.path();
    if !path_is_allowed(state.marketplace, path) {
        return (
            StatusCode::FORBIDDEN,
            "route is not on the lease allow-list",
        )
            .into_response();
    }
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let url = format!("{}{}{}", state.upstream_base, path, query);

    let cap = usize::try_from(tam_limits::http::UPLOAD_BODY_BYTES_MAX).unwrap_or(usize::MAX);
    let Ok(bytes) = axum::body::to_bytes(body, cap).await else {
        return (StatusCode::BAD_REQUEST, "body too large").into_response();
    };

    let mut outbound = state.client.request(parts.method.clone(), &url);
    for (name, value) in &parts.headers {
        if !denied(REQUEST_HEADER_DENY, name) {
            outbound = outbound.header(name, value);
        }
    }
    let (cookie_header, csrf) = {
        let jar = state.jar.lock().await;
        (
            jar.to_cookie_header(),
            jar.value(TPT_CSRF_COOKIE).map(str::to_owned),
        )
    };
    outbound = outbound.header(axum::http::header::COOKIE, cookie_header);
    // The double submit is derived from the jar, which only this process
    // holds, so the gateway mirrors it rather than trusting the worker to.
    if state.marketplace == Marketplace::Tpt {
        if let Some(token) = csrf {
            outbound = outbound.header(TPT_CSRF_HEADER, token);
        }
    }
    outbound = outbound.body(bytes.to_vec());

    match outbound.send().await {
        Ok(upstream) => relay(&state, upstream).await,
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            format!("upstream unreachable: {error}"),
        )
            .into_response(),
    }
}

/// Absorbs the response's renewals into the jar and reseals if anything
/// actually changed.
///
/// The debounce is the point of the return value from `apply_set_cookie`: a
/// busy lease receives the same `Set-Cookie` on nearly every hop, and
/// resealing each time would cost a database write and a fresh envelope per
/// request to store bytes already stored.
async fn absorb_renewals(state: &GatewayState, headers: &reqwest::header::HeaderMap) {
    let mut jar = state.jar.lock().await;
    let mut changed = false;
    for value in headers.get_all(reqwest::header::SET_COOKIE) {
        if let Ok(text) = value.to_str() {
            changed |= jar.apply_set_cookie(text);
        }
    }
    let renewed = changed.then(|| jar.to_cookie_header());
    // Released before the reseal: the sink writes to the database, and a lease
    // driving two hops at once must not queue the second behind that write.
    drop(jar);
    if let Some(cookie_header) = renewed {
        state.sink.reseal(cookie_header).await;
    }
}

async fn relay(state: &GatewayState, upstream: reqwest::Response) -> Response {
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let upstream_headers = upstream.headers().clone();
    absorb_renewals(state, &upstream_headers).await;
    let body = upstream.bytes().await.unwrap_or_else(|_| Bytes::new());
    let mut response = (status, body).into_response();
    let relayed = response.headers_mut();
    for (name, value) in &upstream_headers {
        if !denied(RESPONSE_HEADER_DENY, name) {
            relayed.insert(name, value.clone());
        }
    }
    response
}

/// Drives the marketplace's own renewal route with the sealed jar and absorbs
/// whatever it answers with, which is the broker's side of keeping a session
/// alive between leases.
///
/// Returns the upstream status. A marketplace with no renewal route cannot be
/// refreshed and says so through [`refresh_route`] rather than here.
pub(crate) async fn refresh_session(
    upstream_base: &str,
    marketplace: Marketplace,
    cookie: &Secret,
    sink: &dyn SessionSink,
) -> Result<u16, GatewayError> {
    let route = refresh_route(marketplace)
        .ok_or_else(|| GatewayError(format!("{marketplace:?} has no session-renewal route")))?;
    let client = proxy_client()?;
    let response = client
        .get(format!("{upstream_base}{route}"))
        .header(axum::http::header::COOKIE, cookie.expose())
        .send()
        .await
        .map_err(|error| GatewayError(error.to_string()))?;
    let status = response.status().as_u16();
    let mut jar = CookieJar::from_cookie_header(cookie.expose());
    let mut changed = false;
    for value in response.headers().get_all(reqwest::header::SET_COOKIE) {
        if let Ok(text) = value.to_str() {
            changed |= jar.apply_set_cookie(text);
        }
    }
    if changed {
        sink.reseal(jar.to_cookie_header()).await;
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::extract::State;
    use axum::response::IntoResponse as _;
    use axum::routing::any;
    use axum::Router;
    use tam_secrets::Secret;
    use tam_types::Marketplace;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use super::{
        path_is_allowed, refresh_route, spawn, LeaseGateway, SessionSink, TES_REFRESH_ROUTE,
    };

    #[test]
    fn the_tes_allow_list_admits_the_write_and_read_paths_and_refuses_the_rest() {
        let tes = Marketplace::Tes;
        assert!(
            path_is_allowed(tes, "/api/v2/resources"),
            "create is admitted"
        );
        assert!(
            path_is_allowed(tes, "/api/v2/resources/9001/draft"),
            "metadata is admitted"
        );
        assert!(
            path_is_allowed(tes, "/api/resources/v3/draft/9001/attachment"),
            "the attachment path is admitted"
        );
        assert!(
            path_is_allowed(tes, "/api/v2/dashboard/getAllResources"),
            "the seller's own catalogue list is admitted"
        );
        assert!(
            path_is_allowed(tes, "/api/v2/dashboard/getAllDrafts"),
            "the seller's own draft list is admitted"
        );
        assert!(
            path_is_allowed(tes, "/resource-detail/api/download/9001"),
            "the download manifest is admitted"
        );
        assert!(
            path_is_allowed(tes, "/teaching-resource/download/9001/bundle"),
            "the bundle download is admitted"
        );
        assert!(
            path_is_allowed(tes, TES_REFRESH_ROUTE),
            "the renewal route the longevity probe drives must be reachable through a lease"
        );
        assert!(
            !path_is_allowed(tes, "/api/v2/account/roster"),
            "account administration is refused"
        );
        assert!(
            !path_is_allowed(tes, "/api/v2/payouts"),
            "payout routes are refused"
        );
        assert!(
            !path_is_allowed(tes, "/api/v2/resourcesX"),
            "a prefix that is not a path boundary is refused"
        );
        assert!(
            !path_is_allowed(tes, "/api/v2/dashboardX"),
            "the read prefixes are bounded at a path separator too"
        );
    }

    #[test]
    fn the_tpt_allow_list_admits_the_connector_hops_and_refuses_the_rest() {
        let tpt = Marketplace::Tpt;
        for admitted in [
            "/graph/graphql",
            "/gateway/graphql",
            "/My-Products/New/Digital-Next",
            "/itemsDigital/editNext/13042099",
            "/uploads/upload_file",
            "/uploads/time",
            "/uploads/sign_auth",
            "/uploads/process_file",
            "/queue/results",
            "/converter/generate_thumbs",
            "/Download/Sample-Unit-12854712",
        ] {
            assert!(
                path_is_allowed(tpt, admitted),
                "the connector drives {admitted}, so a lease must reach it"
            );
        }
        assert!(
            !path_is_allowed(tpt, "/My-Account/Payouts"),
            "account and payout routes stay structurally unreachable on Tpt too"
        );
        assert!(
            !path_is_allowed(tpt, "/graph/graphql-not-the-gateway"),
            "a prefix that is not a path boundary is refused"
        );
    }

    #[test]
    fn the_allow_lists_do_not_leak_across_marketplaces() {
        assert!(
            !path_is_allowed(Marketplace::Tpt, "/api/v2/resources"),
            "a Tpt lease must not reach a Tes route"
        );
        assert!(
            !path_is_allowed(Marketplace::Tes, "/graph/graphql"),
            "a Tes lease must not reach a Tpt route"
        );
        assert!(
            !path_is_allowed(Marketplace::Etsy, "/api/v2/resources"),
            "there is no Etsy connector, so an Etsy lease reaches nothing at all"
        );
    }

    #[test]
    fn only_tes_advertises_a_renewal_route() {
        assert_eq!(
            refresh_route(Marketplace::Tes),
            Some(TES_REFRESH_ROUTE),
            "Tes renews on the route the probe has driven for days"
        );
        assert_eq!(
            refresh_route(Marketplace::Tpt),
            None,
            "no Tpt capture shows a renewal endpoint, so the answer is None rather than a guess"
        );
    }

    fn shared_sink(sink: &Arc<RecordingSink>) -> Arc<dyn SessionSink> {
        let concrete: Arc<RecordingSink> = Arc::clone(sink);
        concrete
    }

    /// Records what the gateway asked to have resealed, so a test can assert
    /// both that a renewal was captured and that an unchanged jar was not
    /// written back.
    #[derive(Default)]
    struct RecordingSink {
        resealed: Mutex<Vec<String>>,
    }

    impl SessionSink for RecordingSink {
        fn reseal<'a>(
            &'a self,
            cookie_header: String,
        ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.resealed.lock().await.push(cookie_header);
            })
        }
    }

    /// What the fake upstream saw and what it will answer with.
    struct Upstream {
        seen_cookies: Mutex<Vec<String>>,
        seen_csrf: Mutex<Vec<Option<String>>>,
        set_cookies: Mutex<Vec<Vec<String>>>,
    }

    impl Upstream {
        fn new(set_cookies: Vec<Vec<String>>) -> Arc<Self> {
            Arc::new(Self {
                seen_cookies: Mutex::new(Vec::new()),
                seen_csrf: Mutex::new(Vec::new()),
                set_cookies: Mutex::new(set_cookies),
            })
        }
    }

    async fn upstream_handler(
        State(state): State<Arc<Upstream>>,
        request: axum::extract::Request,
    ) -> axum::response::Response {
        let cookie = request
            .headers()
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let csrf = request
            .headers()
            .get(super::TPT_CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        state.seen_cookies.lock().await.push(cookie);
        state.seen_csrf.lock().await.push(csrf);
        let next = {
            let mut queued = state.set_cookies.lock().await;
            if queued.is_empty() {
                Vec::new()
            } else {
                queued.remove(0)
            }
        };
        let mut response = (axum::http::StatusCode::OK, "ok").into_response();
        for value in next {
            if let Ok(encoded) = axum::http::HeaderValue::from_str(&value) {
                response
                    .headers_mut()
                    .append(axum::http::header::SET_COOKIE, encoded);
            }
        }
        response
    }

    /// Binds the fake upstream and returns its base url.
    async fn serve_upstream(state: Arc<Upstream>) -> String {
        let router = Router::new()
            .fallback(any(upstream_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the fake upstream binds");
        let local = listener
            .local_addr()
            .expect("the fake upstream has an address");
        #[expect(
            clippy::disallowed_methods,
            reason = "the fixture upstream lives for the test process, which is the supervision"
        )]
        tokio::spawn(async move {
            let _served = axum::serve(listener, router).await;
        });
        format!("http://{local}")
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(core::time::Duration::from_secs(10))
            .build()
            .expect("the test client builds")
    }

    #[tokio::test]
    async fn a_renewal_is_captured_resealed_and_carried_on_the_next_request() {
        // The first hop renews the session cookie; the second restates the
        // value the jar now holds, which must not cost a second reseal.
        let upstream = Upstream::new(vec![
            vec!["session=renewed; Path=/; HttpOnly".to_owned()],
            vec!["session=renewed; Path=/".to_owned()],
        ]);
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=original; other=keep".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");

        let http = client();
        for _ in 0..2u8 {
            let response = http
                .get(format!(
                    "{}/api/v2/dashboard/getAllResources",
                    gateway.endpoint
                ))
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", gateway.token),
                )
                .send()
                .await
                .expect("the proxied request completes");
            assert_eq!(
                response.status().as_u16(),
                200,
                "the fake upstream answers 200"
            );
            assert!(
                response
                    .headers()
                    .get(reqwest::header::SET_COOKIE)
                    .is_none(),
                "the renewal must be absorbed by the broker, never relayed to the worker"
            );
        }

        let seen = upstream.seen_cookies.lock().await.clone();
        assert_eq!(seen.len(), 2, "both requests reached the upstream");
        assert_eq!(
            seen.first().map(String::as_str),
            Some("session=original; other=keep"),
            "the first hop carries the sealed jar"
        );
        assert_eq!(
            seen.get(1).map(String::as_str),
            Some("session=renewed; other=keep"),
            "the second hop carries the renewed value, which is the whole point of capturing it"
        );

        let resealed = sink.resealed.lock().await.clone();
        assert_eq!(
            resealed,
            vec!["session=renewed; other=keep".to_owned()],
            "one meaningful change reseals exactly once; a restated value must not reseal again"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn a_request_without_the_lease_token_is_refused_before_the_route_is_matched() {
        let upstream = Upstream::new(Vec::new());
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=secret".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");
        let http = client();
        let allowed = format!("{}/api/v2/dashboard/getAllResources", gateway.endpoint);

        let unauthenticated = http
            .get(&allowed)
            .send()
            .await
            .expect("the request completes");
        assert_eq!(
            unauthenticated.status().as_u16(),
            401,
            "a local process without the lease token must not ride the seller's session"
        );

        let wrong = http
            .get(&allowed)
            .header(axum::http::header::AUTHORIZATION, "Bearer 00000000")
            .send()
            .await
            .expect("the request completes");
        assert_eq!(
            wrong.status().as_u16(),
            401,
            "a token that is not this lease's is no token"
        );

        let refused_route = http
            .get(format!("{}/api/v2/payouts", gateway.endpoint))
            .send()
            .await
            .expect("the request completes");
        assert_eq!(
            refused_route.status().as_u16(),
            401,
            "authentication precedes routing, so an anonymous caller learns nothing about \
             which paths this lease serves"
        );

        assert!(
            upstream.seen_cookies.lock().await.is_empty(),
            "no refused request may reach the upstream carrying the seller's cookie"
        );

        let authorised = http
            .get(&allowed)
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", gateway.token),
            )
            .send()
            .await
            .expect("the request completes");
        assert_eq!(
            authorised.status().as_u16(),
            200,
            "the lease's own token passes, or the token would be a lockout rather than a guard"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn a_tpt_lease_mirrors_the_csrf_cookie_the_worker_cannot_see() {
        let upstream = Upstream::new(Vec::new());
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tpt,
                cookie: Secret::new("sessionKey=abc; csrfToken=deadbeef".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");

        let response = client()
            .post(format!(
                "{}/graph/graphql?opname=MyProductListings",
                gateway.endpoint
            ))
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", gateway.token),
            )
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect("the proxied request completes");
        assert_eq!(
            response.status().as_u16(),
            200,
            "the fake upstream answers 200"
        );
        assert_eq!(
            upstream.seen_csrf.lock().await.first().cloned().flatten(),
            Some("deadbeef".to_owned()),
            "the worker never sees the jar, so the gateway must mirror the csrfToken cookie \
             into x-csrf-token or every Tpt write fails the double-submit check"
        );
        assert_eq!(
            upstream
                .seen_cookies
                .lock()
                .await
                .first()
                .map(String::as_str),
            Some("sessionKey=abc; csrfToken=deadbeef"),
            "the sealed jar is injected server-side"
        );
        root.cancel();
    }
}
