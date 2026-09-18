//! The live transport: reqwest over rustls with mandatory timeouts, and two
//! clients rather than one, because reqwest's `default_headers` apply to
//! every host a client reaches. A single cookie-bearing client sends the
//! seller's Tes session to Amazon on the S3 upload; here the jar lives on
//! the session client alone, and what a request declares about its own
//! authentication decides which client may carry it.
//!
//! That split is decided once, per request, against the url the caller named.
//! A redirect is a second destination the caller never named, so the session
//! client follows one only while the host does not change and stops otherwise.
//! The stopped hop surfaces as the 3xx it is, with its `Location` projected
//! through the seam's allow-list, which is what lets a flow re-issue it on a
//! client carrying no session. Tes's own bundle download is that case: it
//! answers a 302 to a signed CDN url whose signature is the authorisation, and
//! re-issuing it deliberately is what keeps the two clients' rule intact
//! across a hop the marketplace chose.
//!
//! The seller's cookies are the one piece of client state that changes while
//! the client lives. Tes rotates a session on its authenticated responses, so
//! they are held in [`SessionCookies`] and attached per request rather than
//! fixed in `default_headers`, and every `Set-Cookie` is merged back into that
//! holder here — below the seam, which still carries none.

use core::error::Error as _;
use std::sync::RwLock;

use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestAuth, RequestBody, ResponseHeader, Transport,
    TransportError,
};
use tam_marketplace::ConnectFailure;

use crate::endpoints::host_of;
use crate::session::TesSession;

/// The only host a Tes session may reach.
const SESSION_HOST: &str = "www.tes.com";

/// Where a presigned upload goes. The bucket differs per environment; the
/// suffix is what the policy's own bucket condition resolves against.
const S3_HOST_SUFFIX: &str = ".s3.amazonaws.com";

pub struct ReqwestTransport {
    session: reqwest::Client,
    /// The seller's cookies, which outlive any one request and change under
    /// us as Tes rotates them. On the transport rather than on the client
    /// because a `reqwest::Client`'s default headers are fixed at build.
    cookies: SessionCookies,
    bare: reqwest::Client,
    redirected: reqwest::Client,
}

#[derive(Debug)]
pub struct TransportBuildError(pub String);

impl core::fmt::Display for TransportBuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "cannot build the live transport: {}", self.0)
    }
}

impl core::error::Error for TransportBuildError {}

/// The headers every client sends. The session client is this plus the jar,
/// so the two differ in exactly one header and the difference is visible in
/// one place.
fn bare_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("Mozilla/5.0"),
    );
    headers
}

/// The seller's session cookies, as the live client holds them between
/// requests.
///
/// Held here rather than in `default_headers`, and that is the whole of a
/// defect this type exists to have fixed. Tes rotates a session on its own
/// authenticated responses, so a `Cookie` header fixed at construction is a
/// session that stops working a few hours after it was captured however much
/// the seller keeps using it: every `Set-Cookie` Tes sent was dropped, and the
/// device went on presenting the superseded values until Tes answered every
/// call as a lapsed session.
///
/// The rotated values are merged here, at the live boundary, and are never
/// projected into [`HttpResponse`] — so [`project_headers`]' allow-list still
/// has nowhere for a `Set-Cookie` to land, a cassette still cannot carry one,
/// and the seam's no-credential invariant is untouched.
///
/// `Debug` redacts, for the same reason [`TesSession`]'s does.
pub struct SessionCookies(RwLock<String>);

impl core::fmt::Debug for SessionCookies {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SessionCookies(redacted)")
    }
}

impl SessionCookies {
    fn new(header: String) -> Self {
        Self(RwLock::new(header))
    }

    /// The `name=value; name=value` header as it now stands.
    ///
    /// A poisoned lock is read through rather than propagated. The only writer
    /// is [`Self::absorb`], which allocates and parses owned strings and has
    /// no call that can unwind mid-update; and a session the seller can no
    /// longer send is a worse outcome than one value an unrelated unwind
    /// happened to touch.
    #[must_use]
    pub fn header(&self) -> String {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Merges whatever `Set-Cookie` a response carried into the held header.
    ///
    /// By name and in place, so a rotation replaces a value rather than
    /// appending a second cookie of the same name that Tes would then see
    /// twice. A cookie sent with an empty value is dropped, because that is
    /// how a server withdraws one.
    ///
    /// Attributes are discarded. `Domain`, `Path`, `Secure` and `Expires`
    /// describe a cookie store, and this is not one: the single host these
    /// cookies may reach is already asserted by [`route`], and an expiry is
    /// Tes's to enforce on the next request rather than ours to model.
    fn absorb(&self, headers: &reqwest::header::HeaderMap) {
        let refreshed: Vec<(String, String)> = headers
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter_map(cookie_pair)
            .collect();
        if refreshed.is_empty() {
            return;
        }
        let mut held = self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut pairs: Vec<(String, String)> = held.split(';').filter_map(cookie_pair).collect();
        for (name, value) in refreshed {
            match pairs.iter_mut().find(|(present, _)| *present == name) {
                Some(slot) => slot.1 = value,
                None => pairs.push((name, value)),
            }
        }
        pairs.retain(|(_, value)| !value.is_empty());
        *held = pairs
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
    }
}

/// One `name=value`, read from a `Cookie` element or from the first element of
/// a `Set-Cookie`. `None` for anything that names nothing.
fn cookie_pair(cookie: &str) -> Option<(String, String)> {
    let first = cookie.split(';').next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_owned(), value.trim().to_owned()))
}

/// How many same-host hops one request may take before it is refused.
///
/// Named rather than left to reqwest's default, because supplying a custom
/// policy replaces that default entirely: a policy that only decided on the
/// host would follow a same-host loop for ever. Ten is reqwest's own default,
/// so the bare client's `Policy::limited` and the session client's custom
/// policy cap at the same place.
const MAX_REDIRECTS: usize = 10;

/// A redirect chain that never arrived.
///
/// Its own type because `Attempt::error` takes one, and reqwest's equivalent
/// is private.
#[derive(Debug)]
struct TooManyRedirects;

impl core::fmt::Display for TooManyRedirects {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("too many redirects")
    }
}

impl core::error::Error for TooManyRedirects {}

/// Follows a redirect only while the host does not change.
///
/// What this is not: the thing standing between the seller's cookie and a
/// third-party host. reqwest already strips `Cookie`, `Authorization`,
/// `cookie2`, `Proxy-Authorization` and `WWW-Authenticate` from any followed
/// redirect whose host or port changes — `remove_sensitive_headers`, called
/// unconditionally after every follow — so the cookie was never going to reach
/// the CDN, and the gateway transport's `Authorization` was covered by the
/// same mechanism. An earlier version of this comment claimed otherwise and
/// was wrong.
///
/// What it is, and both halves earn it. First, that stripping is a
/// dependency's internal behaviour rather than a contract: nothing in
/// reqwest's public API promises it, a minor release could narrow the list,
/// and a custody rule this codebase states in its own module documentation
/// should not rest on a list we do not own. Second, and concretely, that list
/// is not everything `default_headers` carries. It does not include
/// `User-Agent`, and it would not include whatever a future edit adds to
/// [`session_headers`] — the TPT adapter's session client already carries a
/// browser identity and a CSRF header beside its cookie, which is the shape
/// this one drifts toward. The rule here is about the destination rather than
/// about a header list, so it stays true as the headers change.
///
/// A cross-host hop stops rather than errors, and that is the design half
/// rather than the safety half: the response the caller gets is the 3xx with
/// its `Location`, which is a fact a flow can act on by re-issuing the hop on
/// a client carrying nothing. An error would leave the caller unable to
/// distinguish a declined hop from a marketplace that is down.
///
/// Reaching the hop cap errors, exactly as `Policy::limited` does, because a
/// redirect loop is not a destination anybody can re-issue.
fn same_host_only() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let crossed = attempt
            .previous()
            .last()
            .is_none_or(|from| from.host_str() != attempt.url().host_str());
        if crossed {
            attempt.stop()
        } else if attempt.previous().len() > MAX_REDIRECTS {
            // `>` rather than `>=`, matching `Policy::limited`'s own
            // comparison, so the constant means one bound rather than two and
            // the session client's cap is where it was before this policy
            // existed.
            attempt.error(TooManyRedirects)
        } else {
            attempt.follow()
        }
    })
}

/// Every Tes API call is small and answers fast, so thirty seconds is the
/// ceiling for the session and the redirected clients. The one exception is
/// the presigned S3 upload, which carries the resource's own bundle: a
/// fifteen-megabyte ZIP from a tablet on domestic Wi-Fi took about thirty-five
/// seconds on 2026-09-18, so a thirty-second ceiling turned every create into
/// a timed-out submit that the reconcile then found had landed. The upload's
/// ceiling is therefore its own, sized under the driver's measured worst-case
/// submit budget (`MEASURED_SUBMIT_WORST_CASE_MS`, 180 s against the 600 s
/// lease) rather than under an API round trip.
const API_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(30);
const UPLOAD_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(150);

fn build_client(
    headers: reqwest::header::HeaderMap,
    redirects: reqwest::redirect::Policy,
    timeout: core::time::Duration,
) -> Result<reqwest::Client, TransportBuildError> {
    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(redirects)
        .timeout(timeout)
        .connect_timeout(core::time::Duration::from_secs(10))
        .build()
        .map_err(|error| TransportBuildError(error.to_string()))
}

/// The client the seller's session rides, and the only constructor for one.
///
/// A free function rather than two lines inside [`ReqwestTransport::new`] so
/// that the tests below drive the same construction the shipping transport
/// does. A test that assembled its own client would prove the policy works
/// and not that anything uses it, which is the failure mode this exists to
/// rule out.
///
/// It carries no cookie of its own. What makes it the session client is its
/// redirect policy; the jar is attached per request out of
/// [`SessionCookies`], because the values move on as Tes rotates them and a
/// client's default headers cannot.
fn session_client() -> Result<reqwest::Client, TransportBuildError> {
    build_client(bare_headers(), same_host_only(), API_TIMEOUT)
}

/// The client that carries no credential of ours as client state: what it
/// carries is the presigned S3 upload, authorised by the form fields it posts.
///
/// It follows redirects, at reqwest's own cap. The bundle download's signed
/// CDN url is not fetched here — that is [`redirected_client`], which follows
/// nothing.
fn bare_client() -> Result<reqwest::Client, TransportBuildError> {
    build_client(
        bare_headers(),
        reqwest::redirect::Policy::limited(MAX_REDIRECTS),
        UPLOAD_TIMEOUT,
    )
}

/// The client a marketplace-named hop rides, and it follows nothing.
///
/// `Policy::none()` is the whole of "one hop", and without it the phrase is
/// prose rather than behaviour. Every shape rule this transport applies is
/// checked once, in [`route`], against the url the flow was handed; a client
/// that followed a further redirect would carry the request past all of them
/// to a destination nothing asserted on, and the flow's own second-3xx
/// refusal would never be reached, because the client would have consumed the
/// 3xx before the flow saw it.
fn redirected_client() -> Result<reqwest::Client, TransportBuildError> {
    build_client(
        bare_headers(),
        reqwest::redirect::Policy::none(),
        API_TIMEOUT,
    )
}

impl ReqwestTransport {
    pub fn new(session: &TesSession) -> Result<Self, TransportBuildError> {
        // Validated here rather than per request. The captured header is the
        // one value in this holder that did not come from Tes's own
        // `Set-Cookie`, so it is the one that can be malformed, and refusing
        // to build is a better answer than a request that cannot be composed.
        reqwest::header::HeaderValue::from_str(session.header_value())
            .map_err(|error| TransportBuildError(error.to_string()))?;
        Ok(Self {
            session: session_client()?,
            cookies: SessionCookies::new(session.header_value().to_owned()),
            bare: bare_client()?,
            redirected: redirected_client()?,
        })
    }

    /// The seller's session as it now stands, rotations included.
    ///
    /// The one method that yields the credential, and it exists because the
    /// alternative is a jar that goes stale wherever it was stored while this
    /// client quietly works: the next process, or any rebuild of this
    /// transport, would seed itself from the captured values Tes has already
    /// superseded. The caller that stored the jar writes back what this
    /// returns.
    #[must_use]
    pub fn session_cookies(&self) -> String {
        self.cookies.header()
    }

    /// Which client a route rides.
    ///
    /// Its own function so the join is assertable. Each end of it is proved
    /// on its own — `route` returns the variant, each client keeps its policy
    /// — and a test of either stays green if this mapping hands a
    /// marketplace-named hop to a client that follows redirects.
    fn client_for(&self, route: Route) -> &reqwest::Client {
        match route {
            Route::Session => &self.session,
            Route::Bare => &self.bare,
            Route::Redirected => &self.redirected,
        }
    }
}

/// Which client a request may ride.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Session,
    Bare,
    /// A destination the marketplace named.
    Redirected,
}

/// The host assertion. A request's declared authentication and its
/// destination must agree or it never leaves, so the seller's session cannot
/// reach the bucket and an upload signature cannot reach Tes, whatever
/// request value a future flow builds.
fn route(request: &HttpRequest) -> Result<Route, TransportError> {
    let host = host_of(&request.url).ok_or(TransportError::NotSent(ConnectFailure::DnsFailure))?;
    let permitted = match request.auth {
        RequestAuth::Session => host == SESSION_HOST,
        RequestAuth::Anonymous | RequestAuth::S3SigV2 { .. } => host.ends_with(S3_HOST_SUFFIX),
        // Any host but ours, and https only. The destination was named by the
        // marketplace rather than chosen here, so there is no constant to
        // assert it against; what is asserted instead is that it is not the
        // session origin, which keeps the two routes disjoint and stops a
        // redirect being used to replay a credential-free request back into
        // Tes as though the session client had sent it.
        RequestAuth::Redirected => host != SESSION_HOST && request.url.starts_with("https://"),
    };
    if !permitted {
        return Err(TransportError::NotSent(ConnectFailure::NoRouteToHost));
    }
    Ok(match request.auth {
        RequestAuth::Session => Route::Session,
        RequestAuth::Redirected => Route::Redirected,
        RequestAuth::Anonymous | RequestAuth::S3SigV2 { .. } => Route::Bare,
    })
}

/// `is_connect` covers DNS, refusal and handshake alike and reqwest exposes
/// no finer split without string-sniffing, so the connect class walks the
/// io source where one exists and otherwise reports the catch-all route
/// variant; the variant is diagnostic, the `NotSent` classification is the
/// load-bearing part.
fn connect_failure(error: &reqwest::Error) -> ConnectFailure {
    let mut source: Option<&(dyn core::error::Error + 'static)> = error.source();
    while let Some(current) = source {
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            return if io.kind() == std::io::ErrorKind::ConnectionRefused {
                ConnectFailure::TcpRefused
            } else {
                ConnectFailure::NoRouteToHost
            };
        }
        source = current.source();
    }
    ConnectFailure::NoRouteToHost
}

fn classify_reqwest(error: &reqwest::Error) -> TransportError {
    if error.is_connect() {
        TransportError::NotSent(connect_failure(error))
    } else {
        TransportError::AfterSend {
            detail: error.to_string(),
        }
    }
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let route = route(&request)?;
        // The holder travels with the session route and with no other, which
        // is the same one-host rule `route` just applied, stated once more in
        // the only place a cookie is attached: the bare and redirected
        // clients cannot acquire one by a later edit to this match.
        let cookies = match route {
            Route::Session => Some(&self.cookies),
            Route::Bare | Route::Redirected => None,
        };
        send_over_capped(
            self.client_for(route),
            request,
            REDIRECTED_BODY_MAX,
            cookies,
        )
        .await
    }
}

/// The per-request signature an `S3SigV2` request carries, and nothing else:
/// no client-wide header, so it cannot outlive the one request it signs.
fn apply_auth(builder: reqwest::RequestBuilder, auth: &RequestAuth) -> reqwest::RequestBuilder {
    match auth {
        RequestAuth::Session | RequestAuth::Anonymous | RequestAuth::Redirected => builder,
        RequestAuth::S3SigV2 {
            access_key_id,
            signature,
            amz_date,
            content_md5,
            content_type,
        } => {
            let signed = builder
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("AWS {access_key_id}:{signature}"),
                )
                .header("x-amz-date", amz_date)
                .header(reqwest::header::CONTENT_TYPE, content_type);
            match content_md5 {
                Some(digest) => signed.header("content-md5", digest),
                None => signed,
            }
        }
    }
}

fn apply_body(
    builder: reqwest::RequestBuilder,
    body: RequestBody,
) -> Result<reqwest::RequestBuilder, TransportError> {
    match body {
        RequestBody::Empty => Ok(builder),
        RequestBody::Json(value) => Ok(builder.json(&value)),
        // Tes has no raw-body write: its one upload is the S3 POST-policy
        // form below, whose payload is a multipart part. A raw PUT here
        // would be a shape no capture has observed, so it never leaves.
        RequestBody::Bytes(_) => Err(TransportError::NotSent(ConnectFailure::NoRouteToHost)),
        RequestBody::Multipart { fields, file } => {
            let mut form = reqwest::multipart::Form::new();
            for (name, value) in fields {
                form = form.text(name, value);
            }
            if let Some(part) = file {
                let piece = reqwest::multipart::Part::bytes(part.bytes)
                    .file_name(part.file_name)
                    .mime_str(&part.content_type)
                    .map_err(|_| {
                        // A malformed content type means the request
                        // provably never left; the cause enum has no
                        // finer slot for a local construction fault.
                        TransportError::NotSent(ConnectFailure::NoRouteToHost)
                    })?;
                form = form.part(part.part_name, piece);
            }
            Ok(builder.multipart(form))
        }
    }
}

/// One wire path for every reqwest-backed transport; which client — session,
/// bare or gateway-bound — is the caller's construction.
async fn send_over(
    client: &reqwest::Client,
    request: HttpRequest,
) -> Result<HttpResponse, TransportError> {
    send_over_capped(client, request, REDIRECTED_BODY_MAX, None).await
}

/// The same path with the bound named, and every shipping caller passes
/// [`REDIRECTED_BODY_MAX`]. A parameter because the shipping value is a
/// gigabyte: a test driving this call site at the real ceiling would have to
/// allocate a gigabyte to fail it, so nothing would assert that a
/// `Redirected` body is read under a bound at all.
///
/// `cookies` is the seller's session where one may ride at all, and it is
/// both halves of a rotation: the header goes out from it and every
/// `Set-Cookie` that comes back is merged into it. `None` is a client that
/// carries none of ours, which is the bare and redirected routes and the
/// gateway transport.
async fn send_over_capped(
    client: &reqwest::Client,
    request: HttpRequest,
    cap: u64,
    cookies: Option<&SessionCookies>,
) -> Result<HttpResponse, TransportError> {
    let bounded = matches!(request.auth, RequestAuth::Redirected);
    let builder = match request.method {
        Method::Get => client.get(&request.url),
        Method::Post => client.post(&request.url),
        Method::Put => client.put(&request.url),
        Method::Delete => client.delete(&request.url),
    };
    let builder = apply_auth(builder, &request.auth);
    let builder = match cookies {
        Some(held) => builder.header(
            reqwest::header::COOKIE,
            reqwest::header::HeaderValue::from_str(&held.header())
                // `NotSent` is the truth here: nothing has been composed, let
                // alone sent. Unreachable with a jar Tes itself rotated, and
                // one refusal away from being reachable, which is why it is
                // an arm rather than an assumption.
                .map_err(|_| TransportError::NotSent(ConnectFailure::NoRouteToHost))?,
        ),
        None => builder,
    };
    let builder = apply_body(builder, request.body)?;
    let response = builder
        .send()
        .await
        .map_err(|error| classify_reqwest(&error))?;
    let status = response.status().as_u16();
    // Before the projection, and the order is the point: the rotation is
    // taken off the real header map, into the holder, and the allow-list then
    // runs on the same map and lets nothing of it through.
    if let Some(held) = cookies {
        held.absorb(response.headers());
    }
    let headers = project_headers(response.headers());
    // `.bytes()` not `.text()`: text decodes lossily and would silently
    // corrupt every download bundle.
    let body = if bounded {
        bounded_body(response, cap).await?
    } else {
        response
            .bytes()
            .await
            .map_err(|error| TransportError::AfterSend {
                detail: error.to_string(),
            })?
            .to_vec()
    };
    Ok(HttpResponse {
        status,
        body,
        headers,
    })
}

/// The largest body a marketplace-named hop may return.
///
/// The archive budget rather than a per-file ceiling, because what comes back
/// is a bundle and a bundle may hold several files. A per-file number would
/// refuse a legitimate multi-file resource, and this migration's premise is
/// that size does not block it. `tam-limits` already owns the largest archive
/// this system will process, so this names that rather than inventing a second
/// number to disagree with it.
const REDIRECTED_BODY_MAX: u64 = tam_limits::ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX;

/// Reads a body in chunks, refusing once it passes the cap.
///
/// Chunked rather than `bytes()` with a length check afterwards, because a
/// check applied to bytes already in memory is not a bound: by the time it
/// fails, whatever was sent has been allocated. `Content-Length` is not
/// consulted for the same reason it is not consulted anywhere else here — it
/// is the sender's claim about the sender's own body.
async fn bounded_body(response: reqwest::Response, cap: u64) -> Result<Vec<u8>, TransportError> {
    let mut response = response;
    let mut body: Vec<u8> = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| TransportError::AfterSend {
                detail: error.to_string(),
            })?;
        let Some(chunk) = chunk else { break };
        let so_far = u64::try_from(body.len().saturating_add(chunk.len())).unwrap_or(u64::MAX);
        if so_far > cap {
            // `Refused`, not `AfterSend`: the response arrived and we declined
            // it. `AfterSend` means the outcome is unknown, which classifies
            // as an ambiguity, which halts the tenant and asks an operator to
            // decide something already decided here.
            return Err(TransportError::Refused {
                detail: format!(
                    "the redirected hop returned more than {cap} bytes, past the largest archive this system will process"
                ),
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Projects the real header map through the seam's allow-list.
///
/// `Location` alone, because it is the only one any Tes flow reads: a
/// redirect the client declined to follow is legible only through it.
/// Everything outside the allow-list is dropped at the boundary, so
/// `Set-Cookie` has nowhere to land and a recording cannot carry one however
/// Tes answers. A rotated session is not an exception to that: it is taken
/// out of the same header map into [`SessionCookies`] before this runs, and
/// what crosses the seam is still `Location` and nothing else.
fn project_headers(headers: &reqwest::header::HeaderMap) -> Vec<(ResponseHeader, String)> {
    headers
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| vec![(ResponseHeader::Location, value.to_owned())])
        .unwrap_or_default()
}

/// The gateway-facing transport: no cookie of its own — the broker's gateway
/// injects the session server-side — and every request rebased from the
/// canonical origin onto the leased loopback endpoint.
pub struct GatewayTransport {
    inner: reqwest::Client,
    base: String,
}

impl GatewayTransport {
    /// `lease_token` is the bearer credential the broker minted for this
    /// lease. It is a client-wide default header rather than a per-request
    /// one because every request this transport makes goes to the lease and
    /// nowhere else; without it each is refused 401, since the gateway's
    /// loopback listener is reachable by any process on the host.
    pub fn new(base: String, lease_token: &str) -> Result<Self, TransportBuildError> {
        let mut headers = bare_headers();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {lease_token}"))
                .map_err(|error| TransportBuildError(error.to_string()))?,
        );
        // One client for every hop, and the hops include the bundle upload,
        // so the ceiling is the upload's; an API call that took anywhere near
        // it would be a fault the classifier reports either way.
        let inner = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(UPLOAD_TIMEOUT)
            .connect_timeout(core::time::Duration::from_secs(10))
            .build()
            .map_err(|error| TransportBuildError(error.to_string()))?;
        Ok(Self { inner, base })
    }

    fn rebase(&self, url: &str) -> String {
        match url.strip_prefix("https://www.tes.com") {
            Some(path) => format!("{}{path}", self.base),
            None => url.to_owned(),
        }
    }
}

impl Transport for GatewayTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let rebased = HttpRequest {
            url: self.rebase(&request.url),
            ..request
        };
        send_over(&self.inner, rebased).await
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, Write as _};

    use tam_marketplace::transport::{HttpRequest, RequestAuth, ResponseHeader, TransportError};

    use super::{
        bare_client, bounded_body, route, send_over, send_over_capped, session_client,
        HttpResponse, ReqwestTransport, Route, SessionCookies, MAX_REDIRECTS, REDIRECTED_BODY_MAX,
        SESSION_HOST,
    };
    use crate::session::TesSession;

    fn bare() -> reqwest::Client {
        bare_client().expect("the bare client builds")
    }

    /// The shipping session client, built exactly as `ReqwestTransport::new`
    /// builds it, so a policy that stopped being applied there fails here.
    fn session() -> reqwest::Client {
        session_client().expect("the session client builds")
    }

    /// The captured jar, in the holder the shipping transport keeps it in.
    fn held() -> SessionCookies {
        SessionCookies::new("TESSession=secret".to_owned())
    }

    /// One send on the session route, composed exactly as
    /// `<ReqwestTransport as Transport>::send` composes it: the session
    /// client, the shipping cap, and the cookie holder both halves of a
    /// rotation run through.
    async fn send_session(
        cookies: &SessionCookies,
        request: HttpRequest,
    ) -> Result<HttpResponse, TransportError> {
        send_over_capped(&session(), request, REDIRECTED_BODY_MAX, Some(cookies)).await
    }

    fn a_session() -> TesSession {
        TesSession::from_cookie_header("TESSession=secret".to_owned())
            .expect("a non-empty header is a session")
    }

    fn s3_upload() -> HttpRequest {
        crate::endpoints::s3_upload_request(
            &crate::endpoints::PresignedUpload {
                s3_url: "https://tes-uploads.s3.amazonaws.com/".to_owned(),
                fields: vec![("policy".to_owned(), "p".to_owned())],
                attachment: serde_json::json!({}),
            },
            tam_marketplace::transport::FilePart {
                part_name: "file".to_owned(),
                file_name: "pack.pdf".to_owned(),
                content_type: "application/pdf".to_owned(),
                bytes: b"x".to_vec(),
            },
        )
    }

    fn sigv2() -> RequestAuth {
        RequestAuth::S3SigV2 {
            access_key_id: "AKIAPLACEHOLDER".to_owned(),
            signature: "c2lnbmF0dXJl".to_owned(),
            amz_date: "Thu, 28 Aug 2026 00:00:00 GMT".to_owned(),
            content_md5: None,
            content_type: "application/pdf".to_owned(),
        }
    }

    #[test]
    fn the_session_host_is_the_origin_every_builder_uses() {
        assert_eq!(
            crate::endpoints::ORIGIN,
            format!("https://{SESSION_HOST}"),
            "the host assertion and the endpoint builders must name one origin"
        );
    }

    #[test]
    fn an_s3_upload_never_rides_the_session_client() {
        assert_eq!(
            route(&s3_upload()),
            Ok(Route::Bare),
            "the presigned upload is authorised by its own form fields, so it must carry no cookie"
        );
        assert_eq!(
            route(&HttpRequest::get(
                "https://tes-uploads.s3.amazonaws.com/object".to_owned()
            )),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::NoRouteToHost
            )),
            "a session-authenticated request to the bucket is the leak itself and must not leave"
        );
        assert_eq!(
            route(&HttpRequest::put_signed(
                "https://www.tes.com/api/v2/resources".to_owned(),
                b"x".to_vec(),
                sigv2(),
            )),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::NoRouteToHost
            )),
            "an upload signature has no business at the marketplace origin"
        );
        assert_eq!(
            route(&HttpRequest::get(
                "https://www.tes.com/api/v2/resources".to_owned()
            )),
            Ok(Route::Session),
            "the ordinary api call still rides the session"
        );
        assert_eq!(
            route(&HttpRequest::get("/api/v2/resources".to_owned())),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::DnsFailure
            )),
            "a url whose destination cannot be named fails closed"
        );
    }

    /// Where a marketplace-named hop may and may not go.
    ///
    /// It carries nothing of ours, so its destination is genuinely the
    /// marketplace's business and no host constant is asserted — a content
    /// network's host is one the marketplace can re-point without telling
    /// anybody. What is asserted is the request's shape, and the session-host
    /// refusal is the load-bearing half: without it a `Location` pointing back
    /// at Tes would let a credential-free request be replayed into the
    /// marketplace as though the session client had sent it.
    #[test]
    fn a_redirected_hop_may_reach_a_content_network_but_never_our_own_origin() {
        let redirected = |url: &str| HttpRequest {
            method: tam_marketplace::transport::Method::Get,
            url: url.to_owned(),
            body: tam_marketplace::transport::RequestBody::Empty,
            auth: RequestAuth::Redirected,
        };

        assert_eq!(
            route(&redirected(
                "https://d111111abcdef8.cloudfront.net/bundle?Signature=abc"
            )),
            Ok(Route::Redirected),
            "a signed url on a content network rides the client that carries nothing and \
             follows nothing"
        );
        // Host comparison rather than byte comparison. Both of these are the
        // session origin to DNS and to the marketplace, and both passed the
        // not-our-origin test while it was a string equality — which is a way
        // back into the session origin chosen by a `Location` rather than by
        // us.
        for spelling in [
            "https://WWW.TES.COM/api/v2/resources/1",
            "https://www.tes.com./api/v2/resources/1",
        ] {
            assert_eq!(
                route(&redirected(spelling)),
                Err(TransportError::NotSent(
                    tam_marketplace::ConnectFailure::NoRouteToHost
                )),
                "{spelling} is the session origin however it is spelled"
            );
        }
        assert_eq!(
            route(&redirected("https://www.tes.com/api/v2/resources/1")),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::NoRouteToHost
            )),
            "a location pointing back at the marketplace is refused, or a redirect becomes a \
             way to replay a credential-free request into the session origin"
        );
        assert_eq!(
            route(&redirected("http://d111111abcdef8.cloudfront.net/bundle")),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::NoRouteToHost
            )),
            "https only: the seller's file is not fetched in the clear because a marketplace \
             named a plaintext url"
        );
        // A `Location` may name an address rather than a name, and a bracketed
        // IPv6 authority is the spelling a naive parse reads as a host called
        // `[`: its port is not behind the first colon.
        assert_eq!(
            route(&redirected("https://[2606:4700:4700::1111]:443/bundle")),
            Ok(Route::Redirected),
            "a bracketed IPv6 literal is a host like any other"
        );
        assert_eq!(
            route(&redirected("https://[::1/bundle")),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::DnsFailure
            )),
            "an authority no parser can read is not a destination: fail closed rather than \
             route on whatever the first colon left behind"
        );
    }

    /// A one-shot loopback server answering 200 and handing back the request
    /// head verbatim, so a test asserts on the headers that actually went out
    /// rather than on the ones we believe we configured.
    fn echo_once() -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the client connects");
            let mut reader =
                std::io::BufReader::new(stream.try_clone().expect("the stream clones"));
            let mut head = String::new();
            loop {
                let mut line = String::new();
                let read = reader.read_line(&mut line).expect("a header line arrives");
                let blank = read == 0 || line == "\r\n";
                head.push_str(&line);
                if blank {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .expect("the response goes out");
            head.to_lowercase()
        });
        (format!("http://127.0.0.1:{port}/probe"), handle)
    }

    /// Reads one request head off a stream, to the blank line.
    fn read_head(stream: &std::net::TcpStream) -> String {
        let mut reader = std::io::BufReader::new(stream.try_clone().expect("the stream clones"));
        let mut head = String::new();
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("a header line arrives");
            let blank = read == 0 || line == "\r\n";
            head.push_str(&line);
            if blank {
                break;
            }
        }
        head.to_lowercase()
    }

    /// A loopback server that answers one 302 to `location` and then, if the
    /// client comes back, one 200 carrying `followed`.
    ///
    /// The port is handed to the caller so it can address the same server as a
    /// second host name, which is what makes a cross-host redirect testable
    /// without leaving the loopback: `127.0.0.1` and `localhost` are two hosts
    /// to a client and one machine to the operating system.
    fn redirecting(to: impl Fn(u16) -> String) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        let location = to(port);
        let handle = std::thread::spawn(move || {
            let mut heads = Vec::new();
            let (mut first, _) = listener.accept().expect("the client connects");
            heads.push(read_head(&first));
            // `connection: close` so a follow opens a fresh connection and the
            // second accept below is the second hop rather than a hang.
            first
                .write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: \
                         0\r\nconnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .expect("the redirect goes out");
            drop(first);
            // Non-blocking with a bounded wait, because the case this exists
            // to prove is the one where no second connection ever arrives: a
            // blocking accept there would hang the test rather than fail it.
            listener
                .set_nonblocking(true)
                .expect("the listener goes non-blocking");
            // Counted rather than timed. Reading the clock is a disallowed
            // method here — time enters this tree as data — and a bounded
            // count answers the same question: two hundred polls of ten
            // milliseconds is ample for a follow that was going to arrive.
            for _ in 0..200 {
                match listener.accept() {
                    Ok((mut second, _)) => {
                        second
                            .set_nonblocking(false)
                            .expect("the accepted stream blocks");
                        heads.push(read_head(&second));
                        second
                            .write_all(
                                b"HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: \
                                  close\r\n\r\nfollowed",
                            )
                            .expect("the second answer goes out");
                        break;
                    }
                    Err(why) if why.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(why) => panic!("the listener failed: {why}"),
                }
            }
            heads
        });
        (format!("http://127.0.0.1:{port}/bundle"), handle)
    }

    /// A loopback server answering `hops` same-host redirects and then a 200.
    ///
    /// Same host throughout, so the policy's cross-host arm never fires and
    /// what is under test is the cap alone.
    fn hopping(hops: usize) -> (String, std::thread::JoinHandle<usize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        let handle = std::thread::spawn(move || {
            // Non-blocking throughout: a client that stops at the cap simply
            // stops connecting, and a blocking accept would hang the test
            // rather than end it. Counted rather than timed, because reading
            // the clock is a disallowed method here.
            listener
                .set_nonblocking(true)
                .expect("the listener goes non-blocking");
            let mut served = 0usize;
            let mut idle = 0usize;
            // One more than the chain, so an over-following client is observed
            // rather than silently tolerated.
            while served < hops + 2 && idle < 200 {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        idle = 0;
                        stream
                            .set_nonblocking(false)
                            .expect("the accepted stream blocks");
                        read_head(&stream);
                        served += 1;
                        let answer = if served <= hops {
                            format!(
                                "HTTP/1.1 302 Found\r\nlocation: \
                                 http://127.0.0.1:{port}/hop{served}\r\ncontent-length: \
                                 0\r\nconnection: close\r\n\r\n"
                            )
                        } else {
                            "HTTP/1.1 200 OK\r\ncontent-length: 8\r\nconnection: \
                             close\r\n\r\nfollowed"
                                .to_owned()
                        };
                        if stream.write_all(answer.as_bytes()).is_err() {
                            break;
                        }
                    }
                    Err(why) if why.kind() == std::io::ErrorKind::WouldBlock => {
                        idle += 1;
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(why) => panic!("the listener failed: {why}"),
                }
            }
            served
        });
        (format!("http://127.0.0.1:{port}/start"), handle)
    }

    /// The cap is where `Policy::limited` puts it, not one hop earlier.
    ///
    /// The custom policy replaces reqwest's default outright, so the constant
    /// has to mean the same bound on both clients or the session client
    /// quietly stops accepting a chain the bare client still would.
    #[tokio::test]
    async fn the_session_client_follows_up_to_the_cap_and_refuses_past_it() {
        let (url, server) = hopping(MAX_REDIRECTS);
        let answer = send_over(&session(), HttpRequest::get(url))
            .await
            .expect("a chain at the cap is followed to its end");
        assert_eq!(answer.status, 200, "ten hops is the documented allowance");
        assert_eq!(answer.body, b"followed");
        server.join().expect("the probe server finishes");

        let (url, server) = hopping(MAX_REDIRECTS + 1);
        let refused = send_over(&session(), HttpRequest::get(url))
            .await
            .expect_err("a chain past the cap does not answer");
        assert!(
            matches!(refused, TransportError::AfterSend { .. }),
            "a chain that never arrives is an error rather than a stop: nobody can re-issue a \
             loop, so there is no Location worth handing back. Got: {refused:?}"
        );
        server.join().expect("the probe server finishes");
    }

    /// One hop, proved against a client rather than against a cassette, and
    /// against the client the transport itself selects for the route.
    ///
    /// The flow's second-3xx refusal is unreachable live unless the client
    /// declines to follow: a following client consumes the redirect itself and
    /// the flow never sees one. So this asserts the client, and it fails under
    /// the policy the bare client uses, which is what the redirected leg had
    /// before this fold. Reached through `client_for` rather than through
    /// `redirected_client` directly, so the same assertion also fails if
    /// `Route::Redirected` is ever mapped to a following client.
    #[tokio::test]
    async fn the_redirected_client_follows_nothing() {
        let transport = ReqwestTransport::new(&a_session()).expect("the transport builds");
        let (url, server) = redirecting(|port| format!("http://127.0.0.1:{port}/second"));
        let answer = send_over(
            transport.client_for(Route::Redirected),
            HttpRequest {
                method: tam_marketplace::transport::Method::Get,
                url,
                body: tam_marketplace::transport::RequestBody::Empty,
                auth: RequestAuth::Redirected,
            },
        )
        .await
        .expect("the redirect comes back rather than failing");

        assert_eq!(
            answer.status, 302,
            "the hop is handed back for the flow to refuse; a 200 here means the client \
             followed it and every shape rule was checked against a url that is no longer \
             where the request went"
        );
        let heads = server.join().expect("the probe server finishes");
        assert_eq!(
            heads.len(),
            1,
            "and the second destination was never contacted, same-host though it is"
        );
    }

    /// The bound is a bound, not a check after the fact.
    ///
    /// Driven at a tiny cap rather than the shipping one, because asserting a
    /// gigabyte ceiling would mean allocating a gigabyte to fail it. What is
    /// under test is the comparison and the class of error it raises, and
    /// neither depends on the number.
    #[tokio::test]
    async fn a_body_past_its_cap_is_refused_rather_than_reported_as_lost() {
        let body = b"0123456789";
        let (url, server) = serving(body);
        let at_limit = bounded_body(
            bare_client()
                .expect("the bare client builds")
                .get(&url)
                .send()
                .await
                .expect("the probe server answers"),
            body.len() as u64,
        )
        .await
        .expect("a body exactly at the cap is not past it");
        assert_eq!(at_limit, body, "the whole body arrives at the limit");
        server.join().expect("the probe server finishes");

        let (url, server) = serving(body);
        let over = bounded_body(
            bare_client()
                .expect("the bare client builds")
                .get(&url)
                .send()
                .await
                .expect("the probe server answers"),
            (body.len() - 1) as u64,
        )
        .await
        .expect_err("a body one byte past the cap is refused");
        assert!(
            matches!(over, TransportError::Refused { .. }),
            "refused rather than lost: `AfterSend` would classify as an ambiguity and halt a \
             tenant over a decision already taken here. Got: {over:?}"
        );
        server.join().expect("the probe server finishes");
    }

    /// The bound is applied where the read is issued, not merely written
    /// there.
    ///
    /// `bounded_body` is proved as a function above; this proves that a
    /// `Redirected` request is read through it, on the client the route
    /// selects. Driven at a tiny cap for the same reason as that test: the
    /// shipping ceiling is a gigabyte and asserting it would mean allocating
    /// one.
    #[tokio::test]
    async fn a_redirected_read_is_bounded_where_it_is_issued() {
        let transport = ReqwestTransport::new(&a_session()).expect("the transport builds");
        let (url, server) = serving(b"0123456789");
        let refused = send_over_capped(
            transport.client_for(Route::Redirected),
            HttpRequest {
                method: tam_marketplace::transport::Method::Get,
                url,
                body: tam_marketplace::transport::RequestBody::Empty,
                auth: RequestAuth::Redirected,
            },
            4,
            None,
        )
        .await
        .expect_err("a body past the cap is not a response");
        assert!(
            matches!(refused, TransportError::Refused { .. }),
            "an unbounded read here hands the body back and nothing else notices. Got: \
             {refused:?}"
        );
        server.join().expect("the probe server finishes");
    }

    /// A loopback server answering one 200 with a fixed body.
    fn serving(body: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the client connects");
            read_head(&stream);
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(head.as_bytes())
                .expect("the head goes out");
            stream.write_all(body).expect("the body goes out");
        });
        (format!("http://127.0.0.1:{port}/body"), handle)
    }

    /// The custody assertion this whole policy exists for.
    ///
    /// Tes's bundle download answers a 302 to a signed CDN url on another
    /// host. A request's headers are re-sent on every hop, so a client that
    /// followed it would hand the seller's session cookie to that host, and
    /// neither host assertion would see it: `route` runs once, against the url
    /// the caller named. A 200 here would be that leak.
    #[tokio::test]
    async fn the_session_client_does_not_follow_a_redirect_to_another_host() {
        let (url, server) = redirecting(|port| format!("http://localhost:{port}/signed"));
        let answer = send_session(&held(), HttpRequest::get(url))
            .await
            .expect("the redirect comes back rather than failing");

        assert_eq!(
            answer.status, 302,
            "a cross-host redirect stops at the seam: a 200 here means the hop was followed \
             and the seller's cookie went to a host nothing asserted on"
        );
        assert!(
            answer
                .header(ResponseHeader::Location)
                .is_some_and(|location| location.contains("/signed")),
            "the stopped hop comes back legible, or a flow has no way to re-issue it on a \
             client carrying nothing: {:?}",
            answer.headers
        );

        let heads = server.join().expect("the probe server finishes");
        assert_eq!(
            heads.len(),
            1,
            "the second host was never contacted at all, which is the property rather than \
             its consequence"
        );
    }

    /// The control, and it is not a formality.
    ///
    /// A blanket refusal to follow would have been the simpler policy and it
    /// would break classification: a draft's download manifest answers a
    /// same-origin redirect to an `?error=notfound` page, and the adapter
    /// reads that page's body to tell an unpublished resource from a failed
    /// read.
    #[tokio::test]
    async fn a_same_host_redirect_is_still_followed() {
        let (url, server) = redirecting(|port| format!("http://127.0.0.1:{port}/error=notfound"));
        let answer = send_session(&held(), HttpRequest::get(url))
            .await
            .expect("the probe server answers");

        assert_eq!(answer.status, 200, "a same-host hop is followed as before");
        assert_eq!(
            answer.body, b"followed",
            "and the body is the one behind the redirect, which is what classification reads"
        );

        let heads = server.join().expect("the probe server finishes");
        assert_eq!(heads.len(), 2, "both hops reached the one host");
        assert!(
            heads
                .iter()
                .all(|head| head.contains("cookie: tessession=secret")),
            "a same-host follow keeps the session, because the host it was asserted against \
             has not changed: {heads:?}"
        );
    }

    #[tokio::test]
    async fn the_bare_client_sends_no_cookie() {
        let (url, server) = echo_once();
        send_over(&bare(), HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            !head.contains("cookie:"),
            "the client the S3 upload rides must carry no session cookie, and sent: {head}"
        );
        assert!(
            head.contains("user-agent: mozilla/5.0"),
            "the bare client still identifies itself, and sent: {head}"
        );
    }

    #[tokio::test]
    async fn the_session_client_does_send_the_cookie() {
        let (url, server) = echo_once();
        send_session(&held(), HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            head.contains("cookie: tessession=secret"),
            "the control: the probe can see a cookie when one is sent, and saw: {head}"
        );
    }

    /// The defect this holder exists to have fixed.
    ///
    /// Tes rotates a session on its authenticated responses. A transport that
    /// dropped the `Set-Cookie` went on presenting the values it captured
    /// until Tes stopped accepting them, which a seller saw as every call
    /// answering as a lapsed session a few hours after signing in. The
    /// assertion is on the next request's own header, because the holder
    /// agreeing with itself would prove only that a string was stored.
    #[tokio::test]
    async fn a_rotated_session_is_what_the_next_request_carries() {
        let cookies = held();
        let (url, server) = setting_cookies(&["TESSession=rotated; Path=/; HttpOnly"]);
        send_session(&cookies, HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        server.join().expect("the probe server finishes");

        let (url, server) = echo_once();
        send_session(&cookies, HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            head.contains("cookie: tessession=rotated"),
            "the second request carries what Tes last issued, or the session goes stale while \
             the seller keeps using it: {head}"
        );
    }

    /// A rotation replaces a value rather than appending a second cookie of
    /// the same name, and a cookie Tes cleared is gone rather than sent empty.
    #[test]
    fn absorbing_replaces_by_name_and_drops_what_was_cleared() {
        let cookies = SessionCookies::new("csrfToken=old; TESSession=old".to_owned());
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("TESSession=new; Path=/"),
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("csrfToken=; Max-Age=0"),
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("tesUser=added"),
        );
        cookies.absorb(&headers);
        assert_eq!(
            cookies.header(),
            "TESSession=new; tesUser=added",
            "a rotation is an update in place, a cleared cookie is dropped, and a new one is \
             appended; a jar that grew a second TESSession would be sent twice"
        );
    }

    /// A response carrying no rotation leaves the captured jar exactly as it
    /// was, so an ordinary read cannot empty a working session.
    #[test]
    fn a_response_with_no_set_cookie_changes_nothing() {
        let cookies = held();
        cookies.absorb(&reqwest::header::HeaderMap::new());
        assert_eq!(cookies.header(), "TESSession=secret");
    }

    /// A loopback server answering one 200 with the given `Set-Cookie` lines.
    fn setting_cookies(cookies: &[&str]) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        let head = cookies.iter().fold(String::new(), |mut head, cookie| {
            use core::fmt::Write as _;
            write!(head, "set-cookie: {cookie}\r\n").expect("a String accepts every write");
            head
        });
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the client connects");
            let sent = read_head(&stream);
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\n{head}content-length: 0\r\nconnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .expect("the response goes out");
            sent
        });
        (format!("http://127.0.0.1:{port}/probe"), handle)
    }
}
