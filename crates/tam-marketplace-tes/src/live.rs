//! The live transport: reqwest over rustls with mandatory timeouts, and two
//! clients rather than one, because reqwest's `default_headers` apply to
//! every host a client reaches. A single cookie-bearing client sends the
//! seller's Tes session to Amazon on the S3 upload; here the jar lives on
//! the session client alone, and what a request declares about its own
//! authentication decides which client may carry it.

use core::error::Error as _;

use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestAuth, RequestBody, Transport, TransportError,
};
use tam_marketplace::ConnectFailure;

use crate::session::TesSession;

/// The only host a Tes session may reach.
const SESSION_HOST: &str = "www.tes.com";

/// Where a presigned upload goes. The bucket differs per environment; the
/// suffix is what the policy's own bucket condition resolves against.
const S3_HOST_SUFFIX: &str = ".s3.amazonaws.com";

pub struct ReqwestTransport {
    session: reqwest::Client,
    bare: reqwest::Client,
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

fn session_headers(
    session: &TesSession,
) -> Result<reqwest::header::HeaderMap, TransportBuildError> {
    let mut headers = bare_headers();
    headers.insert(
        reqwest::header::COOKIE,
        reqwest::header::HeaderValue::from_str(session.header_value())
            .map_err(|error| TransportBuildError(error.to_string()))?,
    );
    Ok(headers)
}

fn build_client(
    headers: reqwest::header::HeaderMap,
) -> Result<reqwest::Client, TransportBuildError> {
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(core::time::Duration::from_secs(30))
        .connect_timeout(core::time::Duration::from_secs(10))
        .build()
        .map_err(|error| TransportBuildError(error.to_string()))
}

impl ReqwestTransport {
    pub fn new(session: &TesSession) -> Result<Self, TransportBuildError> {
        Ok(Self {
            session: build_client(session_headers(session)?)?,
            bare: build_client(bare_headers())?,
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

/// The host assertion. A request's declared authentication and its
/// destination must agree or it never leaves, so the seller's session cannot
/// reach the bucket and an upload signature cannot reach Tes, whatever
/// request value a future flow builds.
fn route(request: &HttpRequest) -> Result<Route, TransportError> {
    let host = host_of(&request.url).ok_or(TransportError::NotSent(ConnectFailure::DnsFailure))?;
    let permitted = match request.auth {
        RequestAuth::Session => host == SESSION_HOST,
        RequestAuth::Anonymous | RequestAuth::S3SigV2 { .. } => host.ends_with(S3_HOST_SUFFIX),
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
        let client = match route(&request)? {
            Route::Session => &self.session,
            Route::Bare => &self.bare,
        };
        send_over(client, request).await
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
    let builder = match request.method {
        Method::Get => client.get(&request.url),
        Method::Post => client.post(&request.url),
        Method::Put => client.put(&request.url),
        Method::Delete => client.delete(&request.url),
    };
    let builder = apply_auth(builder, &request.auth);
    let builder = apply_body(builder, request.body)?;
    let response = builder
        .send()
        .await
        .map_err(|error| classify_reqwest(&error))?;
    let status = response.status().as_u16();
    // `.bytes()` not `.text()`: text decodes lossily and would silently
    // corrupt every download bundle.
    let body = response
        .bytes()
        .await
        .map_err(|error| TransportError::AfterSend {
            detail: error.to_string(),
        })?;
    Ok(HttpResponse::plain(status, body.to_vec()))
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
        let inner = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(core::time::Duration::from_secs(30))
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

    use tam_marketplace::transport::{HttpRequest, RequestAuth, TransportError};

    use super::{bare_headers, build_client, route, send_over, Route, SESSION_HOST};
    use crate::session::TesSession;

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

    #[tokio::test]
    async fn the_bare_client_sends_no_cookie() {
        let (url, server) = echo_once();
        let bare = build_client(bare_headers()).expect("the bare client builds");
        send_over(&bare, HttpRequest::get(url))
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
        let session = TesSession::from_cookie_header("TESSession=secret".to_owned())
            .expect("a non-empty header is a session");
        let (url, server) = echo_once();
        let client = build_client(super::session_headers(&session).expect("the headers build"))
            .expect("the session client builds");
        send_over(&client, HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            head.contains("cookie: tessession=secret"),
            "the control: the probe can see a cookie when one is sent, and saw: {head}"
        );
    }
}
