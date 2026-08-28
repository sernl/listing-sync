//! The live transport: reqwest over rustls with mandatory timeouts, and two
//! clients rather than one, because reqwest's `default_headers` apply to
//! every host a client reaches. A single cookie-bearing client would send the
//! seller's TPT session to Amazon on every S3 call; here the jar lives on the
//! session client alone, and what a request declares about its own
//! authentication decides which client may carry it.
//!
//! Redirects are never followed. The create and edit submits answer 302 with
//! an empty body, and the `Location` is the only place the product id
//! appears — a transport that chased it would discard the identifier and hand
//! the flow the product page instead.
//!
//! The header envelope is per request rather than per client, because the
//! capture distinguishes three shapes on one host: the product form renders
//! and the submits answering them are document navigations, the GraphQL and
//! upload hops are XHRs carrying the mirrored CSRF header and the XHR marker,
//! and the clock read and the signing oracle are plain same-origin fetches.
//! Each shape is sent what the capture recorded for it, because a request
//! claiming to be a browser while carrying none of a browser's other signals
//! is what Cloudflare challenges: the same document navigation was answered
//! with an interstitial under a truncated `Mozilla/5.0` and with the form
//! under a real browser's headers. A challenge that arrives anyway is named
//! rather than guessed at, by `crate::classify::classify_form_page`, which
//! reads the interstitial markers before the scrape runs.

use core::error::Error as _;

use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestAuth, RequestBody, ResponseHeader, Transport,
    TransportError,
};
use tam_marketplace::ConnectFailure;

use crate::endpoints::{self, ORIGIN};
use crate::session::TptSession;

/// Mirrors the `csrfToken` cookie; the double-submit pair TPT validates.
const CSRF_HEADER: &str = "x-csrf-token";
/// Sent by the TPT client on every XHR call, on both services.
const REQUESTED_WITH_HEADER: &str = "x-requested-with";
/// The single header that distinguishes a gateway call from a graph call.
const GATEWAY_VERSION_HEADER: &str = "x-gateway-auth-version";
const GATEWAY_VERSION: &str = "2";
/// The job token, which no response body carries.
const QUEUE_TRACKING_HEADER: &str = "x-queue-tracking-id";

/// The seller's own browser, verbatim from the founder's HAR captures. Sent
/// rather than a shortened stand-in because a truncated `Mozilla/5.0` claims
/// to be a browser and matches no browser's remaining signals, which is the
/// state Cloudflare challenges the document navigations in.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:153.0) Gecko/20100101 Firefox/153.0";
/// Constant across every recorded hop, on both hosts.
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// Every session hop is same-origin, so the site is a client default and only
/// the mode, the destination and the accepted media types vary by shape.
const SEC_FETCH_SITE_HEADER: &str = "sec-fetch-site";
const SEC_FETCH_MODE_HEADER: &str = "sec-fetch-mode";
const SEC_FETCH_DEST_HEADER: &str = "sec-fetch-dest";
const SEC_FETCH_USER_HEADER: &str = "sec-fetch-user";
const SAME_ORIGIN: &str = "same-origin";

const NAVIGATION_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const GRAPHQL_ACCEPT: &str = "application/json";
/// The upload hops are jQuery's, and jQuery sends its own list.
const JQUERY_ACCEPT: &str = "application/json, text/javascript, */*; q=0.01";
const ANY_ACCEPT: &str = "*/*";

/// The only host a TPT session may reach.
const SESSION_HOST: &str = "www.teacherspayteachers.com";
/// Path-style S3, so the bucket is in the path and the host is bare. The
/// suffix form is accepted too, for a virtual-host address no capture uses.
const S3_HOST: &str = "s3.amazonaws.com";

/// The four XHR hops post `application/x-www-form-urlencoded`. The seam has
/// no variant for a form body, so it travels as raw bytes and is labelled
/// here — these four are the only raw bodies a TPT session ever sends, and
/// the one raw body that is not theirs rides an S3 signature instead.
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded; charset=UTF-8";

pub struct ReqwestTransport {
    session: reqwest::Client,
    bare: reqwest::Client,
    csrf_token: String,
}

#[derive(Debug)]
pub struct TransportBuildError(pub String);

impl core::fmt::Display for TransportBuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "cannot build the live transport: {}", self.0)
    }
}

impl core::error::Error for TransportBuildError {}

fn static_name(name: &'static str) -> reqwest::header::HeaderName {
    reqwest::header::HeaderName::from_static(name)
}

/// The headers every client sends: the browser identity, which the captures
/// carry unchanged on both hosts. The session client is this plus the jar and
/// the one same-origin constant, so the two differ in exactly two headers and
/// the difference is visible in one place; everything varying by hop shape is
/// applied per request instead.
fn bare_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static(USER_AGENT),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        reqwest::header::HeaderValue::from_static(ACCEPT_LANGUAGE),
    );
    headers
}

fn session_headers(
    session: &TptSession,
) -> Result<reqwest::header::HeaderMap, TransportBuildError> {
    let mut headers = bare_headers();
    headers.insert(
        static_name(SEC_FETCH_SITE_HEADER),
        reqwest::header::HeaderValue::from_static(SAME_ORIGIN),
    );
    let mut set = |name: reqwest::header::HeaderName, value: &str| {
        reqwest::header::HeaderValue::from_str(value)
            .map(|encoded| headers.insert(name, encoded))
            .map(|_| ())
            .map_err(|error| TransportBuildError(error.to_string()))
    };
    set(reqwest::header::COOKIE, session.header_value())?;
    Ok(headers)
}

fn build_client(
    headers: reqwest::header::HeaderMap,
) -> Result<reqwest::Client, TransportBuildError> {
    reqwest::Client::builder()
        .default_headers(headers)
        // The submit's 302 carries the product id in its Location and an
        // empty body; following it would discard the identifier.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(core::time::Duration::from_secs(30))
        .connect_timeout(core::time::Duration::from_secs(10))
        .build()
        .map_err(|error| TransportBuildError(error.to_string()))
}

impl ReqwestTransport {
    pub fn new(session: &TptSession) -> Result<Self, TransportBuildError> {
        Ok(Self {
            session: build_client(session_headers(session)?)?,
            bare: build_client(bare_headers())?,
            csrf_token: session.csrf_token().to_owned(),
        })
    }
}

/// Which client a request may ride.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Session,
    Bare,
}

/// The host of an absolute http(s) url, without userinfo or port. `None` for
/// anything else, which the caller turns into a refusal: a request whose
/// destination cannot be named is not one to send.
fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    host.split(':').next()
}

fn is_s3_host(host: &str) -> bool {
    host == S3_HOST || host.ends_with(".s3.amazonaws.com")
}

/// The host assertion. A request's declared authentication and its
/// destination must agree or it never leaves, so the seller's session cannot
/// reach the bucket and an upload signature cannot reach TPT, whatever
/// request value a future flow builds.
fn route(request: &HttpRequest) -> Result<Route, TransportError> {
    let host = host_of(&request.url).ok_or(TransportError::NotSent(ConnectFailure::DnsFailure))?;
    let permitted = match request.auth {
        RequestAuth::Session => host == SESSION_HOST,
        RequestAuth::Anonymous | RequestAuth::S3SigV2 { .. } => is_s3_host(host),
    };
    if !permitted {
        return Err(TransportError::NotSent(ConnectFailure::NoRouteToHost));
    }
    Ok(if request.auth.is_session() {
        Route::Session
    } else {
        Route::Bare
    })
}

/// The shape of one session hop, which is what decides its header envelope
/// and, for the two XHR shapes, its CSRF header and XHR marker.
///
/// The distinction is the capture's own: `/uploads/upload_file`,
/// `/uploads/process_file`, `/converter/generate_thumbs`, `/queue/results`
/// and both GraphQL services carry the pair; the two form renders and the two
/// multipart posts answering them are document navigations and carry neither;
/// `/uploads/time` and `/uploads/sign_auth` are plain cookie-authenticated
/// GETs the page makes without the XHR marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hop {
    Navigation,
    GraphqlXhr,
    FormXhr,
    Fetch,
}

impl Hop {
    const fn accept(self) -> &'static str {
        match self {
            Self::Navigation => NAVIGATION_ACCEPT,
            Self::GraphqlXhr => GRAPHQL_ACCEPT,
            Self::FormXhr => JQUERY_ACCEPT,
            Self::Fetch => ANY_ACCEPT,
        }
    }

    const fn fetch_mode(self) -> &'static str {
        match self {
            Self::Navigation => "navigate",
            Self::GraphqlXhr | Self::FormXhr | Self::Fetch => "cors",
        }
    }

    const fn fetch_dest(self) -> &'static str {
        match self {
            Self::Navigation => "document",
            Self::GraphqlXhr | Self::FormXhr | Self::Fetch => "empty",
        }
    }

    const fn is_xhr(self) -> bool {
        matches!(self, Self::GraphqlXhr | Self::FormXhr)
    }
}

/// Which shape a session request wears. The destination decides a navigation,
/// because both form urls are navigated to and posted to alike; the body
/// decides the rest, because the XHR hops are exactly the ones carrying a
/// JSON or a urlencoded payload.
fn hop(request: &HttpRequest) -> Hop {
    if endpoints::is_product_form(&request.url) {
        Hop::Navigation
    } else {
        match request.body {
            RequestBody::Json(_) => Hop::GraphqlXhr,
            RequestBody::Bytes(_) => Hop::FormXhr,
            RequestBody::Empty | RequestBody::Multipart { .. } => Hop::Fetch,
        }
    }
}

/// What one shape carries beyond the client defaults, as the captures
/// recorded it.
fn envelope(hop: Hop, method: Method) -> Vec<(reqwest::header::HeaderName, &'static str)> {
    let mut headers = vec![
        (reqwest::header::ACCEPT, hop.accept()),
        (static_name(SEC_FETCH_MODE_HEADER), hop.fetch_mode()),
        (static_name(SEC_FETCH_DEST_HEADER), hop.fetch_dest()),
    ];
    match hop {
        Hop::Navigation => {
            headers.push((reqwest::header::UPGRADE_INSECURE_REQUESTS, "1"));
            // A browser sends neither on the render it navigates to and both
            // on the submit that answers it: an Origin on a same-origin
            // document GET is a signal no browser emits.
            if !matches!(method, Method::Get) {
                headers.push((static_name(SEC_FETCH_USER_HEADER), "?1"));
                headers.push((reqwest::header::ORIGIN, ORIGIN));
            }
        }
        Hop::GraphqlXhr | Hop::FormXhr => headers.push((reqwest::header::ORIGIN, ORIGIN)),
        Hop::Fetch => {}
    }
    headers
}

/// The browser envelope for one session request. Session-authenticated hops
/// alone: the bucket sees the identity headers of the bare client and nothing
/// naming the marketplace.
fn apply_session_envelope(
    builder: reqwest::RequestBuilder,
    request: &HttpRequest,
    csrf_token: &str,
) -> reqwest::RequestBuilder {
    let shape = hop(request);
    let builder = envelope(shape, request.method)
        .into_iter()
        .fold(builder, |carried, (name, value)| {
            carried.header(name, value)
        });
    if shape.is_xhr() {
        builder
            .header(static_name(CSRF_HEADER), csrf_token)
            .header(static_name(REQUESTED_WITH_HEADER), "XMLHttpRequest")
    } else {
        builder
    }
}

/// `is_connect` covers DNS, refusal and handshake alike and reqwest exposes
/// no finer split without string-sniffing, so the connect class walks the io
/// source where one exists and otherwise reports the catch-all route variant;
/// the variant is diagnostic, the `NotSent` classification is the load-bearing
/// part.
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

/// The per-request signature an `S3SigV2` request carries, and nothing else:
/// no client-wide header, so it cannot outlive the one request it signs.
fn apply_auth(builder: reqwest::RequestBuilder, auth: &RequestAuth) -> reqwest::RequestBuilder {
    match auth {
        RequestAuth::Session | RequestAuth::Anonymous => builder,
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
    session_authenticated: bool,
) -> Result<reqwest::RequestBuilder, TransportError> {
    match body {
        RequestBody::Empty => Ok(builder),
        RequestBody::Json(value) => Ok(builder.json(&value)),
        RequestBody::Bytes(bytes) => Ok(if session_authenticated {
            builder
                .header(reqwest::header::CONTENT_TYPE, FORM_CONTENT_TYPE)
                .body(bytes)
        } else {
            // The S3 raw bodies — an object part and the completion XML —
            // whose content type is bound into the signature and is set by
            // `apply_auth` rather than here.
            builder.body(bytes)
        }),
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
                        // provably never left; the cause enum has no finer
                        // slot for a local construction fault.
                        TransportError::NotSent(ConnectFailure::NoRouteToHost)
                    })?;
                form = form.part(part.part_name, piece);
            }
            Ok(builder.multipart(form))
        }
    }
}

/// Projects the real header map through the seam's allow-list. Everything
/// outside these three is dropped at the boundary, so `Set-Cookie` has
/// nowhere to land and a recording cannot carry one however TPT answers.
fn project_headers(headers: &reqwest::header::HeaderMap) -> Vec<(ResponseHeader, String)> {
    let mut projected = Vec::new();
    let mut take = |name: reqwest::header::HeaderName, which: ResponseHeader| {
        if let Some(value) = headers.get(&name).and_then(|value| value.to_str().ok()) {
            projected.push((which, value.to_owned()));
        }
    };
    take(reqwest::header::LOCATION, ResponseHeader::Location);
    take(reqwest::header::ETAG, ResponseHeader::ETag);
    take(
        static_name(QUEUE_TRACKING_HEADER),
        ResponseHeader::QueueTrackingId,
    );
    projected
}

impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let route = route(&request)?;
        let client = match route {
            Route::Session => &self.session,
            Route::Bare => &self.bare,
        };
        let builder = match request.method {
            Method::Get => client.get(&request.url),
            Method::Post => client.post(&request.url),
            Method::Put => client.put(&request.url),
            Method::Delete => client.delete(&request.url),
        };
        let session_authenticated = route == Route::Session;
        let builder = if session_authenticated {
            apply_session_envelope(builder, &request, &self.csrf_token)
        } else {
            builder
        };
        let builder = if endpoints::is_gateway(&request.url) {
            builder.header(static_name(GATEWAY_VERSION_HEADER), GATEWAY_VERSION)
        } else {
            builder
        };
        let builder = apply_auth(builder, &request.auth);
        let builder = apply_body(builder, request.body, session_authenticated)?;
        let response = builder
            .send()
            .await
            .map_err(|error| classify_reqwest(&error))?;
        let status = response.status().as_u16();
        let headers = project_headers(response.headers());
        // `.bytes()` not `.text()`: text decodes lossily and would silently
        // corrupt any non-UTF-8 payload.
        let body = response
            .bytes()
            .await
            .map_err(|error| TransportError::AfterSend {
                detail: error.to_string(),
            })?;
        Ok(HttpResponse {
            status,
            body: body.to_vec(),
            headers,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, Write as _};

    use tam_marketplace::transport::{
        HttpRequest, Method, RequestAuth, RequestBody, ResponseHeader, TransportError,
    };

    use super::{
        bare_headers, build_client, envelope, hop, project_headers, route, session_headers,
        static_name, Hop, Route, ACCEPT_LANGUAGE, SEC_FETCH_MODE_HEADER, SESSION_HOST, USER_AGENT,
    };
    use crate::endpoints::{FormTarget, ORIGIN};
    use crate::session::TptSession;

    fn sigv2() -> RequestAuth {
        RequestAuth::S3SigV2 {
            access_key_id: "AKIAPLACEHOLDER00000".to_owned(),
            signature: "c2lnbmF0dXJl".to_owned(),
            amz_date: "Fri, 28 Aug 2026 05:57:21 GMT".to_owned(),
            content_md5: None,
            content_type: "image/png".to_owned(),
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
    fn the_seller_session_never_reaches_the_bucket() {
        assert_eq!(
            route(&HttpRequest::get(
                "https://s3.amazonaws.com/live.digital.upload/object".to_owned()
            )),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::NoRouteToHost
            )),
            "a session-authenticated request to the bucket is the leak itself"
        );
        assert_eq!(
            route(&HttpRequest::put_signed(
                "https://s3.amazonaws.com/live.digital.upload/object?partNumber=1".to_owned(),
                b"x".to_vec(),
                sigv2(),
            )),
            Ok(Route::Bare),
            "a signed part carries its own authorisation and must carry no cookie"
        );
        assert_eq!(
            route(&HttpRequest::put_signed(
                "https://www.teacherspayteachers.com/uploads/upload_file".to_owned(),
                b"x".to_vec(),
                sigv2(),
            )),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::NoRouteToHost
            )),
            "an upload signature has no business at the marketplace origin"
        );
        assert_eq!(
            route(&HttpRequest::get(format!(
                "https://{SESSION_HOST}/graph/graphql"
            ))),
            Ok(Route::Session),
            "the ordinary api call still rides the session"
        );
        assert_eq!(
            route(&HttpRequest::get("/graph/graphql".to_owned())),
            Err(TransportError::NotSent(
                tam_marketplace::ConnectFailure::DnsFailure
            )),
            "a url whose destination cannot be named fails closed"
        );
    }

    #[test]
    fn every_hop_is_classified_as_the_capture_recorded_it() {
        let form = FormTarget::CreateDigital.url();
        assert_eq!(
            hop(&HttpRequest::get(form.clone())),
            Hop::Navigation,
            "the form render is a document navigation"
        );
        assert_eq!(
            hop(&HttpRequest {
                method: Method::Post,
                url: form,
                body: RequestBody::Multipart {
                    fields: vec![],
                    file: None
                },
                auth: RequestAuth::Session,
            }),
            Hop::Navigation,
            "and so is the submit answering it, which the capture shows carries no CSRF header"
        );
        assert_eq!(
            hop(&HttpRequest::post_json(
                format!("{ORIGIN}/graph/graphql?opname=MyProductListings"),
                serde_json::json!({})
            )),
            Hop::GraphqlXhr,
            "a GraphQL call is an XHR and mirrors the token"
        );
        assert_eq!(
            hop(&HttpRequest {
                method: Method::Post,
                url: format!("{ORIGIN}/uploads/upload_file"),
                body: RequestBody::Bytes(b"job=abc".to_vec()),
                auth: RequestAuth::Session,
            }),
            Hop::FormXhr,
            "the four urlencoded upload hops are XHRs and mirror the token"
        );
        assert_eq!(
            hop(&HttpRequest::get(format!(
                "{ORIGIN}/uploads/time?requestTime=1787896640872"
            ))),
            Hop::Fetch,
            "the clock read and the signing oracle are plain fetches, and carry neither"
        );
        assert!(
            Hop::GraphqlXhr.is_xhr() && Hop::FormXhr.is_xhr(),
            "the two XHR shapes are the two that mirror the token"
        );
        assert!(
            !Hop::Navigation.is_xhr() && !Hop::Fetch.is_xhr(),
            "and neither document navigations nor plain fetches do"
        );
    }

    #[test]
    fn only_the_submit_navigation_and_the_xhr_hops_name_the_origin() {
        let has = |headers: &[(reqwest::header::HeaderName, &'static str)],
                   name: &reqwest::header::HeaderName| {
            headers.iter().any(|(carried, _)| carried == name)
        };
        let render = envelope(Hop::Navigation, Method::Get);
        assert!(
            !has(&render, &reqwest::header::ORIGIN),
            "a browser sends no Origin on a same-origin document GET, and sent: {render:?}"
        );
        assert!(
            has(&render, &reqwest::header::UPGRADE_INSECURE_REQUESTS)
                && render.contains(&(reqwest::header::ACCEPT, super::NAVIGATION_ACCEPT)),
            "the render is a navigation asking for HTML, and sent: {render:?}"
        );
        let submit = envelope(Hop::Navigation, Method::Post);
        assert!(
            has(&submit, &reqwest::header::ORIGIN)
                && has(&submit, &static_name(super::SEC_FETCH_USER_HEADER)),
            "the submit is a user-activated navigation that does name it, and sent: {submit:?}"
        );
        let xhr = envelope(Hop::GraphqlXhr, Method::Post);
        assert!(
            has(&xhr, &reqwest::header::ORIGIN)
                && xhr.contains(&(static_name(SEC_FETCH_MODE_HEADER), "cors")),
            "an XHR names the origin and is a cors fetch, and sent: {xhr:?}"
        );
        let plain = envelope(Hop::Fetch, Method::Get);
        assert!(
            !has(&plain, &reqwest::header::ORIGIN),
            "the clock read names none either, and sent: {plain:?}"
        );
    }

    #[test]
    fn only_the_three_allow_listed_headers_survive_the_boundary() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::SET_COOKIE,
            reqwest::header::HeaderValue::from_static("sessionKey=secret"),
        );
        headers.insert(
            reqwest::header::LOCATION,
            reqwest::header::HeaderValue::from_static("/Product/test-17511712"),
        );
        headers.insert(
            reqwest::header::ETAG,
            reqwest::header::HeaderValue::from_static("\"28a6dcac\""),
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("x-queue-tracking-id"),
            reqwest::header::HeaderValue::from_static("e2fdc706"),
        );
        let projected = project_headers(&headers);
        assert_eq!(
            projected.len(),
            3,
            "three variants exist, so three headers can survive, got {projected:?}"
        );
        assert!(
            !format!("{projected:?}").contains("secret"),
            "Set-Cookie has no variant to land in, and the projection carried: {projected:?}"
        );
        assert_eq!(
            projected
                .iter()
                .find(|(name, _)| *name == ResponseHeader::Location)
                .map(|(_, value)| value.as_str()),
            Some("/Product/test-17511712"),
            "the Location is the only place the new product id appears"
        );
    }

    /// A one-shot loopback server answering 200 and handing back the request
    /// head verbatim, so a test asserts on the headers that actually went out
    /// rather than on the ones we believe we configured.
    fn echo_once(status_line: &'static str) -> (String, std::thread::JoinHandle<String>) {
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
                .write_all(status_line.as_bytes())
                .expect("the response goes out");
            head.to_lowercase()
        });
        (format!("http://127.0.0.1:{port}/probe"), handle)
    }

    async fn probe(
        client: &reqwest::Client,
        request: HttpRequest,
    ) -> Result<super::HttpResponse, TransportError> {
        let builder = match request.method {
            Method::Get => client.get(&request.url),
            Method::Post => client.post(&request.url),
            Method::Put => client.put(&request.url),
            Method::Delete => client.delete(&request.url),
        };
        let builder = if request.auth.is_session() {
            super::apply_session_envelope(builder, &request, "deadbeef")
        } else {
            builder
        };
        let builder = super::apply_auth(builder, &request.auth);
        let builder = super::apply_body(builder, request.body, request.auth.is_session())?;
        let response = builder
            .send()
            .await
            .map_err(|error| super::classify_reqwest(&error))?;
        let status = response.status().as_u16();
        let headers = project_headers(response.headers());
        let body = response
            .bytes()
            .await
            .map_err(|error| TransportError::AfterSend {
                detail: error.to_string(),
            })?;
        Ok(super::HttpResponse {
            status,
            body: body.to_vec(),
            headers,
        })
    }

    #[tokio::test]
    async fn the_bare_client_sends_no_cookie() {
        let (url, server) =
            echo_once("HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        let bare = build_client(bare_headers()).expect("the bare client builds");
        probe(
            &bare,
            HttpRequest::put_signed(url, b"bytes".to_vec(), sigv2()),
        )
        .await
        .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            !head.contains("cookie:"),
            "the client every S3 call rides must carry no session cookie, and sent: {head}"
        );
        assert!(
            head.contains("authorization: aws akiaplaceholder00000:"),
            "the signature travels per request, and the head was: {head}"
        );
        assert!(
            head.contains(&format!("user-agent: {}", USER_AGENT.to_lowercase()))
                && head.contains(&format!(
                    "accept-language: {}",
                    ACCEPT_LANGUAGE.to_lowercase()
                )),
            "both clients carry the seller's browser identity, and the head was: {head}"
        );
    }

    #[tokio::test]
    async fn the_session_client_sends_the_cookie_and_never_follows_the_submit_redirect() {
        let session = TptSession::from_cookie_header("csrfToken=deadbeef".to_owned())
            .expect("a header carrying the token is a session");
        let (url, server) = echo_once(
            "HTTP/1.1 302 Found\r\nlocation: /Product/test-17511712\r\n\
             content-length: 0\r\nconnection: close\r\n\r\n",
        );
        let client = build_client(session_headers(&session).expect("the headers build"))
            .expect("the session client builds");
        let response = probe(&client, HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            head.contains("cookie: csrftoken=deadbeef"),
            "the control: the probe can see a cookie when one is sent, and saw: {head}"
        );
        assert!(
            !head.contains("origin:") && !head.contains("x-csrf-token:"),
            "a plain session GET names no origin and mirrors no token, and sent: {head}"
        );
        assert!(
            head.contains("accept: */*") && head.contains("sec-fetch-dest: empty"),
            "the clock read and the signing oracle are plain fetches, and sent: {head}"
        );
        assert_eq!(
            head.lines()
                .filter(|line| line.starts_with("accept:"))
                .count(),
            1,
            "the per-request Accept replaces reqwest's own rather than joining it: {head}"
        );
        assert_eq!(
            response.status, 302,
            "a followed redirect would report the product page's status instead"
        );
        assert_eq!(
            response.header(ResponseHeader::Location),
            Some("/Product/test-17511712"),
            "and the identifier survives only because the redirect was not chased"
        );
    }

    #[tokio::test]
    async fn a_session_raw_body_is_labelled_as_a_form() {
        let (url, server) =
            echo_once("HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        let session = TptSession::from_cookie_header("csrfToken=deadbeef".to_owned())
            .expect("a header carrying the token is a session");
        let client = build_client(session_headers(&session).expect("the headers build"))
            .expect("the session client builds");
        probe(
            &client,
            HttpRequest {
                method: Method::Post,
                url,
                body: RequestBody::Bytes(b"job=abc".to_vec()),
                auth: RequestAuth::Session,
            },
        )
        .await
        .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            head.contains("content-type: application/x-www-form-urlencoded"),
            "the four upload hops post a form, and the head was: {head}"
        );
        assert!(
            head.contains("x-csrf-token: deadbeef")
                && head.contains("x-requested-with: xmlhttprequest")
                && head.contains(&format!("origin: {}", ORIGIN.to_lowercase())),
            "an upload hop is an XHR, which mirrors the token and names the origin, and sent: \
             {head}"
        );
    }
}
