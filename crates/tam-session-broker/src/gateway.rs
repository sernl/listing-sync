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

use crate::jar::{CookieJar, JarChange};

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

/// Resolves a request against the lease's upstream into the exact URL that
/// will be sent, so the string checked and the string sent are the same one.
///
/// This is the whole of the traversal defence and it is structural rather than
/// a filter. Checking `Uri::path()` and then sending a separately-built string
/// checks a different value from the one that travels: URL resolution collapses
/// dot-segments, so `/api/v2/dashboard/../../../api/v2/payouts` passes a check
/// on the raw path and arrives at `/api/v2/payouts` — a payout route the
/// compliance floor requires to be structurally unreachable, reached with the
/// seller's cookie injected. Percent-encoded `%2e%2e` collapses the same way,
/// so rejecting a literal `..` substring does not close it either.
///
/// Resolving once and checking `Url::path()` — already normalized, already
/// percent-decoded in the segments that matter — removes the gap rather than
/// filtering what falls through it. The resolved `Url` is then handed to
/// reqwest as a `Url` rather than as a string, so nothing re-parses it.
///
/// `None` on a base or reference that will not resolve, which the caller
/// refuses: a request whose destination cannot be determined is not one to
/// send with a credential attached.
fn resolve(upstream_base: &str, uri: &axum::http::Uri) -> Option<reqwest::Url> {
    let base = reqwest::Url::parse(upstream_base).ok()?;
    let reference = match uri.query() {
        Some(query) => format!("{}?{query}", uri.path()),
        None => uri.path().to_owned(),
    };
    let resolved = base.join(&reference).ok()?;
    // `join` on an absolute reference would silently retarget the whole
    // request at another host; only same-origin resolutions may be sent.
    if resolved.origin() != base.origin() {
        return None;
    }
    // An encoded separator survives normalisation and is therefore a second
    // way for the checked path and the effective path to disagree — this time
    // across the wire rather than within this process. `..%2f..` is one
    // segment to the URL parser, so it collapses nothing and keeps the request
    // inside an allowed prefix; an upstream that decodes before it routes then
    // sees the traversal this side already approved. Refusing the encoding
    // costs nothing real, because no allow-listed route takes a path segment
    // containing a slash or a backslash.
    (!encodes_a_separator(resolved.path())).then_some(resolved)
}

/// Whether a path carries a percent-encoded `/` or `\\`.
fn encodes_a_separator(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    lowered.contains("%2f") || lowered.contains("%5c")
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

/// A future this trait's methods return. Boxed because the trait is used as an
/// object, so that the gateway's own tests can drive a recording double.
type SinkFuture<'a> = core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + 'a>>;

/// Where the custody facts a live session produces are recorded.
///
/// A trait rather than the vault itself so that the capture decisions are
/// provable without a database: the gateway's own tests drive a recording
/// sink, and the writing half is proved separately against the broker role.
///
/// The three methods are the only writers of the freshness axis, and the split
/// between them is the invariant: `session_verified_at` means a real read
/// proved this session live, so only [`SessionSink::reseal`] and
/// [`SessionSink::record_verified`] may set it and both are reached only from
/// a 2xx upstream answer. Linking sets it never — a credential that has been
/// sealed has not thereby been shown to work.
pub(crate) trait SessionSink: Send + Sync {
    /// A renewal arrived on a 2xx answer: store it, and record that the
    /// session was proved live just now.
    fn reseal(&self, cookie_header: String) -> SinkFuture<'_>;

    /// A 2xx answer carrying no cookie change still proves the session live.
    fn record_verified(&self) -> SinkFuture<'_>;

    /// A non-2xx answer, or one whose cookies were cleared. Advances the
    /// consecutive-failure count without asserting the session is dead —
    /// classifying a failure as an authentication problem is a decision this
    /// layer deliberately does not make.
    fn record_failure(&self) -> SinkFuture<'_>;
}

struct GatewayState {
    upstream_base: String,
    marketplace: Marketplace,
    token: String,
    jar: Mutex<CookieJar>,
    sink: Arc<dyn SessionSink>,
    client: reqwest::Client,
    /// Whether this lease has already recorded a live verification.
    ///
    /// One write per lease rather than one per hop: a lease runs for minutes
    /// and issues many requests, the freshness window it feeds is a day wide,
    /// and stamping every 2xx would spend a database write per proxied request
    /// to record a fact already recorded.
    verified_recorded: std::sync::atomic::AtomicBool,
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
        verified_recorded: std::sync::atomic::AtomicBool::new(false),
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
    let Some(url) = resolve(&state.upstream_base, &parts.uri) else {
        return (StatusCode::BAD_REQUEST, "the request url does not resolve").into_response();
    };
    if !path_is_allowed(state.marketplace, url.path()) {
        return (
            StatusCode::FORBIDDEN,
            "route is not on the lease allow-list",
        )
            .into_response();
    }

    let cap = usize::try_from(tam_limits::http::UPLOAD_BODY_BYTES_MAX).unwrap_or(usize::MAX);
    let Ok(bytes) = axum::body::to_bytes(body, cap).await else {
        return (StatusCode::BAD_REQUEST, "body too large").into_response();
    };

    let mut outbound = state.client.request(parts.method.clone(), url);
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

/// What the jar did across every `Set-Cookie` on one response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Absorbed {
    /// Nothing changed, or the change was a restatement.
    Unchanged,
    /// At least one cookie was renewed and none was removed.
    Renewed,
    /// At least one cookie was removed — the shape a logout takes.
    Cleared,
}

/// Applies a response's `Set-Cookie` headers to a scratch copy of the jar and
/// reports what they did, without committing anything.
fn absorbed_into(jar: &mut CookieJar, headers: &reqwest::header::HeaderMap) -> Absorbed {
    let mut outcome = Absorbed::Unchanged;
    for value in headers.get_all(reqwest::header::SET_COOKIE) {
        let Ok(text) = value.to_str() else { continue };
        match jar.apply_set_cookie(text) {
            JarChange::Unchanged => {}
            JarChange::Renewed => {
                if outcome == Absorbed::Unchanged {
                    outcome = Absorbed::Renewed;
                }
            }
            // A clearing dominates: one removal on a response makes the whole
            // response a logout, whatever else it renewed alongside it.
            JarChange::Cleared => outcome = Absorbed::Cleared,
        }
    }
    outcome
}

/// Records what one upstream answer proved about the session, and stores a
/// renewal only when the answer earned the right to be believed.
///
/// Three rules, and each closes a way the stored credential or the freshness
/// axis could be corrupted by a response that proves nothing:
///
/// A non-2xx answer absorbs nothing. A 401, a Cloudflare challenge and a
/// sign-in interstitial all carry `Set-Cookie`, and all of them are the
/// marketplace replacing a working session with an anonymous one; sealing that
/// would overwrite the only stored credential with one that cannot
/// authenticate, and stamping it verified would make a dead connection read
/// healthy for a day. Such an answer advances the failure count instead.
///
/// A cleared cookie is never sealed, whatever the status. A logout arrives as
/// `Max-Age=0` or an empty value on an otherwise ordinary response, and the
/// vault row is updated in place with no history, so one sealed logout is
/// unrecoverable. This mirrors the guard the operator seal already has, where
/// `cookie_header_from_netscape` refuses an empty jar.
///
/// `session_verified_at` is written here and in [`refresh_session`] and
/// nowhere else. That is the invariant the whole function exists to hold: the
/// column means a real read proved this session live, so linking must not set
/// it and neither must anything that has not just seen a 2xx.
async fn absorb_renewals(
    state: &GatewayState,
    status: StatusCode,
    headers: &reqwest::header::HeaderMap,
) {
    if !status.is_success() {
        // Deliberately without touching the jar: an answer that proves nothing
        // must not move the session this lease is still driving with.
        state.sink.record_failure().await;
        return;
    }
    let mut jar = state.jar.lock().await;
    // Decided on a copy and committed only once. A clearing must not reach the
    // live jar either: this lease is still driving requests with it, and a
    // logout absorbed in memory would make every remaining hop anonymous even
    // though nothing was ever written to the vault.
    let mut candidate = jar.clone();
    let outcome = absorbed_into(&mut candidate, headers);
    let renewed = match outcome {
        Absorbed::Renewed if !candidate.is_empty() => {
            *jar = candidate;
            Some(jar.to_cookie_header())
        }
        Absorbed::Renewed | Absorbed::Cleared | Absorbed::Unchanged => None,
    };
    // Released before the sink writes: the sink reaches the database, and a
    // lease driving two hops at once must not queue the second behind it.
    drop(jar);

    match outcome {
        Absorbed::Cleared => state.sink.record_failure().await,
        Absorbed::Renewed => match renewed {
            Some(cookie_header) => state.sink.reseal(cookie_header).await,
            // A renewal that empties the jar is not a renewal; unreachable by
            // construction and refused rather than trusted.
            None => state.sink.record_failure().await,
        },
        Absorbed::Unchanged => {
            // A 2xx carrying no renewal still proves the session live, and is
            // the ordinary case: most authenticated reads set no cookie.
            if !state
                .verified_recorded
                .swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                state.sink.record_verified().await;
            }
        }
    }
}

async fn relay(state: &GatewayState, upstream: reqwest::Response) -> Response {
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let upstream_headers = upstream.headers().clone();
    absorb_renewals(state, status, &upstream_headers).await;
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

/// Drives the marketplace's own renewal route with the sealed jar and records
/// what it answered, which is the broker's side of keeping a session alive
/// between leases.
///
/// The status is acted on here rather than returned for somebody else to act
/// on, because the two failure classes are not distinguishable afterwards. A
/// refresh that could not be sent and a refresh answered `401` are both
/// "the refresh did not work", and only the second is evidence about the
/// session; treating the status as merely informational is how a refresh loop
/// ends up counting network faults and never counting expired sessions.
///
/// A marketplace with no renewal route cannot be refreshed and says so through
/// [`refresh_route`] rather than here.
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
    let status = response.status();
    if !status.is_success() {
        sink.record_failure().await;
        return Ok(status.as_u16());
    }
    let mut jar = CookieJar::from_cookie_header(cookie.expose());
    match absorbed_into(&mut jar, response.headers()) {
        // A renewal that emptied the jar is not a renewal, so it joins the
        // clearing rather than the sealing: both leave the stored credential
        // as it was and both count as a failure to prove the session live.
        Absorbed::Renewed if !jar.is_empty() => sink.reseal(jar.to_cookie_header()).await,
        Absorbed::Cleared | Absorbed::Renewed => sink.record_failure().await,
        Absorbed::Unchanged => sink.record_verified().await,
    }
    Ok(status.as_u16())
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
        path_is_allowed, refresh_route, resolve, spawn, LeaseGateway, SessionSink, SinkFuture,
        TES_REFRESH_ROUTE,
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
        verified: Mutex<u32>,
        failures: Mutex<u32>,
    }

    impl SessionSink for RecordingSink {
        fn reseal(&self, cookie_header: String) -> SinkFuture<'_> {
            Box::pin(async move {
                self.resealed.lock().await.push(cookie_header);
            })
        }

        fn record_verified(&self) -> SinkFuture<'_> {
            Box::pin(async move {
                *self.verified.lock().await += 1;
            })
        }

        fn record_failure(&self) -> SinkFuture<'_> {
            Box::pin(async move {
                *self.failures.lock().await += 1;
            })
        }
    }

    /// What the fake upstream saw and what it will answer with.
    struct Upstream {
        seen_cookies: Mutex<Vec<String>>,
        seen_csrf: Mutex<Vec<Option<String>>>,
        seen_paths: Mutex<Vec<String>>,
        set_cookies: Mutex<Vec<Vec<String>>>,
        status: axum::http::StatusCode,
    }

    impl Upstream {
        fn new(set_cookies: Vec<Vec<String>>) -> Arc<Self> {
            Self::answering(axum::http::StatusCode::OK, set_cookies)
        }

        fn answering(status: axum::http::StatusCode, set_cookies: Vec<Vec<String>>) -> Arc<Self> {
            Arc::new(Self {
                seen_cookies: Mutex::new(Vec::new()),
                seen_csrf: Mutex::new(Vec::new()),
                seen_paths: Mutex::new(Vec::new()),
                set_cookies: Mutex::new(set_cookies),
                status,
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
        state
            .seen_paths
            .lock()
            .await
            .push(request.uri().path().to_owned());
        let next = {
            let mut queued = state.set_cookies.lock().await;
            if queued.is_empty() {
                Vec::new()
            } else {
                queued.remove(0)
            }
        };
        let mut response = (state.status, "ok").into_response();
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
    /// Drives raw bytes at the loopback listener.
    ///
    /// The reqwest-driven tests cannot reach the traversal defence at all:
    /// reqwest resolves the URL client-side, so `..` is already collapsed
    /// before anything goes on the wire and the gateway never sees the shape
    /// under test. Only a hand-written request line puts the dot-segments in
    /// front of the allow-list.
    async fn raw_get(endpoint: &str, token: &str, target: &str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let address = endpoint.trim_start_matches("http://");
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("the gateway accepts a raw connection");
        let request = format!(
            "GET {target} HTTP/1.1\r\nHost: lease\r\nAuthorization: Bearer {token}\r\n\
             Connection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("the raw request writes");
        let mut buffer = [0u8; 4096];
        let read = stream
            .read(&mut buffer)
            .await
            .expect("the gateway answers the raw request");
        String::from_utf8_lossy(&buffer[..read]).into_owned()
    }

    fn resolved_path(target: &str) -> Option<String> {
        let uri: axum::http::Uri = target.parse().ok()?;
        resolve("https://www.tes.com", &uri).map(|url| url.path().to_owned())
    }

    #[test]
    fn dot_segments_collapse_before_the_allow_list_sees_the_path() {
        // The headline reproduction, pinned exactly: this is the request that
        // reached the payout route with the seller cookie injected, because the
        // allow-list read the raw path while the send built a separate string
        // that url resolution then normalised.
        assert_eq!(
            resolved_path("/api/v2/dashboard/../../../api/v2/payouts").as_deref(),
            Some("/api/v2/payouts"),
            "the traversal resolves onto the payout route, which is why checking the raw path \
             checked a value that never travelled"
        );

        // The property, over every encoding the collapse accepts. Where each
        // one lands varies with how many segments it pops; that none of them
        // lands anywhere the allow-list admits is the whole point.
        for target in [
            "/api/v2/dashboard/../../../api/v2/payouts",
            "/api/v2/dashboard/../account/roster",
            "/api/v2/dashboard/%2e%2e/%2e%2e/api/v2/payouts",
            "/api/v2/dashboard/%2E%2E/%2E%2E/%2E%2E/api/v2/payouts",
            "/api/v2/dashboard/..%2f../api/v2/payouts",
            "/api/v2/dashboard/./../../api/v2/account/roster",
            "/api/resources/v3/draft/../../../../api/v2/payouts",
        ] {
            // Two ways to be safe, and a traversal must take one of them:
            // refused outright at resolution, or normalised onto a path the
            // allow-list denies. What it must never do is normalise onto an
            // admitted prefix while still carrying the means to move.
            let Some(path) = resolved_path(target) else {
                continue;
            };
            assert!(
                !path_is_allowed(Marketplace::Tes, &path),
                "{target} normalises to {path}, and no traversal may land on a route the \
                 compliance floor requires to be structurally unreachable"
            );
            assert!(
                !path.contains(".."),
                "{target} must be checked after normalisation, not before: {path} still \
                 carries dot-segments, so the checked value and the sent value disagree again"
            );
        }
    }

    #[test]
    fn an_encoded_separator_is_refused_rather_than_passed_to_the_upstream() {
        // `..%2f..` is one segment to the parser, so it collapses nothing and
        // stays inside an allowed prefix. An upstream that decodes before it
        // routes would then perform the traversal this side just approved, so
        // the encoding is refused here rather than trusted to mean nothing.
        for target in [
            "/api/v2/dashboard/..%2f../api/v2/payouts",
            "/api/v2/dashboard/..%2F..%2Fapi/v2/payouts",
            "/api/v2/dashboard/..%5c../api/v2/payouts",
        ] {
            assert_eq!(
                resolved_path(target),
                None,
                "{target} carries an encoded separator and must not be sent at all"
            );
        }
    }

    #[test]
    fn a_benign_path_survives_resolution_unharmed() {
        assert_eq!(
            resolved_path("/api/v2/dashboard/getAllResources").as_deref(),
            Some("/api/v2/dashboard/getAllResources"),
            "normalising must not disturb an ordinary route"
        );
        assert!(
            path_is_allowed(
                Marketplace::Tes,
                &resolved_path("/api/v2/resources/9001/draft").expect("resolves")
            ),
            "a real write path still passes after the fix"
        );
    }

    #[test]
    fn a_reference_that_would_leave_the_upstream_is_refused() {
        let uri: axum::http::Uri = "https://evil.test/api/v2/dashboard"
            .parse()
            .expect("an absolute uri parses");
        assert_eq!(
            resolve("https://www.tes.com", &uri).map(|url| url.to_string()),
            Some("https://www.tes.com/api/v2/dashboard".to_owned()),
            "only the path is taken from the reference, so an absolute url cannot retarget \
             the lease at another host"
        );
    }

    #[tokio::test]
    async fn a_traversal_over_raw_bytes_never_reaches_a_denied_route() {
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

        for target in [
            "/api/v2/dashboard/../../../api/v2/payouts",
            "/api/v2/dashboard/%2e%2e/%2e%2e/api/v2/payouts",
            "/api/v2/dashboard/../account/roster",
            "/api/resources/v3/draft/../../../api/v2/payouts",
        ] {
            let answer = raw_get(&gateway.endpoint, &gateway.token, target).await;
            assert!(
                answer.starts_with("HTTP/1.1 403"),
                "{target} must be refused by the allow-list; the gateway answered: {answer}"
            );
        }
        assert!(
            upstream.seen_paths.lock().await.is_empty(),
            "no traversal may reach the upstream at all, with or without the cookie"
        );

        // The same connection still serves the routes it is for, so the fix is
        // a normalisation rather than a blanket refusal.
        let allowed = raw_get(
            &gateway.endpoint,
            &gateway.token,
            "/api/v2/dashboard/getAllResources",
        )
        .await;
        assert!(
            allowed.starts_with("HTTP/1.1 200"),
            "an ordinary route must still pass: {allowed}"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn a_tpt_traversal_cannot_reach_the_account_routes_either() {
        let upstream = Upstream::new(Vec::new());
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tpt,
                cookie: Secret::new("csrfToken=deadbeef".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");

        for target in [
            "/graph/../My-Account/Payouts",
            "/uploads/time/%2e%2e/%2e%2e/My-Account/Payouts",
        ] {
            let answer = raw_get(&gateway.endpoint, &gateway.token, target).await;
            assert!(
                answer.starts_with("HTTP/1.1 403"),
                "{target} must be refused on Tpt too: {answer}"
            );
        }
        assert!(
            upstream.seen_paths.lock().await.is_empty(),
            "no Tpt traversal reaches the upstream"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn a_non_success_answer_neither_reseals_nor_records_a_verification() {
        // A 401 carrying Set-Cookie is exactly how an expired session and a
        // sign-in interstitial arrive. Absorbing it would overwrite the only
        // sealed credential with an anonymous one and stamp the connection
        // verified, so the seller's page would read connected for a day
        // against a session that authenticates nothing.
        let upstream = Upstream::answering(
            axum::http::StatusCode::UNAUTHORIZED,
            vec![vec!["session=anonymous; Path=/".to_owned()]],
        );
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=live".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");

        let response = client()
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
        assert_eq!(response.status().as_u16(), 401, "the refusal is relayed");

        assert!(
            sink.resealed.lock().await.is_empty(),
            "a 401 must never overwrite the stored credential"
        );
        assert_eq!(
            *sink.verified.lock().await,
            0,
            "a 401 proves the session is not live, so nothing may record it as verified"
        );
        assert_eq!(
            *sink.failures.lock().await,
            1,
            "a 401 is evidence about the session and must advance the failure count"
        );

        // And the lease keeps driving with the credential it started on.
        assert_eq!(
            upstream
                .seen_cookies
                .lock()
                .await
                .first()
                .map(String::as_str),
            Some("session=live"),
            "the live jar is what was sent"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn a_success_with_no_cookie_change_still_records_the_session_live() {
        let upstream = Upstream::new(vec![Vec::new(), Vec::new()]);
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=live".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");

        for _ in 0..2u8 {
            let response = client()
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
            assert_eq!(response.status().as_u16(), 200, "the read succeeds");
        }

        assert_eq!(
            *sink.verified.lock().await,
            1,
            "most authenticated reads set no cookie, so this is the production writer of \
             session_verified_at — and it writes once per lease rather than once per hop"
        );
        assert!(
            sink.resealed.lock().await.is_empty(),
            "nothing changed, so nothing is sealed"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn a_session_clearing_cookie_never_reaches_the_vault() {
        // A logout arrives as a 200 with Max-Age=0. The vault row is updated
        // in place with no history, so one sealed logout destroys the only
        // copy of a working credential.
        let upstream = Upstream::new(vec![vec!["session=; Max-Age=0; Path=/".to_owned()]]);
        let base = serve_upstream(Arc::clone(&upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let gateway = spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=live".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the gateway binds");

        let response = client()
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
        assert_eq!(response.status().as_u16(), 200, "the logout answers 200");

        assert!(
            sink.resealed.lock().await.is_empty(),
            "a cleared session must never be sealed over a working one"
        );
        assert_eq!(
            *sink.verified.lock().await,
            0,
            "a response that ended the session did not prove it live"
        );
        assert_eq!(
            *sink.failures.lock().await,
            1,
            "a cleared session is a failure signal, not a silent no-op"
        );
        root.cancel();
    }

    #[tokio::test]
    async fn one_leases_token_does_not_open_another_lease() {
        let first_upstream = Upstream::new(Vec::new());
        let second_upstream = Upstream::new(Vec::new());
        let first_base = serve_upstream(Arc::clone(&first_upstream)).await;
        let second_base = serve_upstream(Arc::clone(&second_upstream)).await;
        let sink = Arc::new(RecordingSink::default());
        let root = CancellationToken::new();
        let first = spawn(
            LeaseGateway {
                upstream_base: first_base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=first".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the first gateway binds");
        let second = spawn(
            LeaseGateway {
                upstream_base: second_base,
                marketplace: Marketplace::Tes,
                cookie: Secret::new("session=second".to_owned()),
                sink: shared_sink(&sink),
            },
            &root,
        )
        .await
        .expect("the second gateway binds");

        assert_ne!(
            first.token, second.token,
            "two leases must not share a token"
        );
        assert_eq!(
            first.token.len(),
            second.token.len(),
            "both are the same length, so the refusal below is about identity rather than shape"
        );

        let crossed = client()
            .get(format!(
                "{}/api/v2/dashboard/getAllResources",
                second.endpoint
            ))
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", first.token),
            )
            .send()
            .await
            .expect("the request completes");
        assert_eq!(
            crossed.status().as_u16(),
            401,
            "a token is scoped to the lease that minted it; a length check alone would let \
             one tenant's worker ride another tenant's seller session"
        );
        assert!(
            second_upstream.seen_cookies.lock().await.is_empty(),
            "the other lease's session was never reached"
        );
        root.cancel();
    }
}
