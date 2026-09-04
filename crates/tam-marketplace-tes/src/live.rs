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

use core::error::Error as _;

use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestAuth, RequestBody, ResponseHeader, Transport,
    TransportError,
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

fn build_client(
    headers: reqwest::header::HeaderMap,
    redirects: reqwest::redirect::Policy,
) -> Result<reqwest::Client, TransportBuildError> {
    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(redirects)
        .timeout(core::time::Duration::from_secs(30))
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
fn session_client(session: &TesSession) -> Result<reqwest::Client, TransportBuildError> {
    build_client(session_headers(session)?, same_host_only())
}

/// The client that carries no credential of ours.
///
/// It follows redirects, and that discloses nothing: the signed CDN url the
/// bundle download lands on is fetched here and may redirect again within its
/// own CDN.
fn bare_client() -> Result<reqwest::Client, TransportBuildError> {
    build_client(
        bare_headers(),
        reqwest::redirect::Policy::limited(MAX_REDIRECTS),
    )
}

impl ReqwestTransport {
    pub fn new(session: &TesSession) -> Result<Self, TransportBuildError> {
        Ok(Self {
            session: session_client(session)?,
            bare: bare_client()?,
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
    let headers = project_headers(response.headers());
    // `.bytes()` not `.text()`: text decodes lossily and would silently
    // corrupt every download bundle.
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

/// Projects the real header map through the seam's allow-list.
///
/// `Location` alone, because it is the only one any Tes flow reads: a
/// redirect the client declined to follow is legible only through it.
/// Everything outside the allow-list is dropped at the boundary, so
/// `Set-Cookie` has nowhere to land and a recording cannot carry one however
/// Tes answers.
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

    use tam_marketplace::transport::{HttpRequest, RequestAuth, ResponseHeader, TransportError};

    use super::{
        bare_client, route, send_over, session_client, Route, MAX_REDIRECTS, SESSION_HOST,
    };
    use crate::session::TesSession;

    fn bare() -> reqwest::Client {
        bare_client().expect("the bare client builds")
    }

    /// The shipping session client, built exactly as `ReqwestTransport::new`
    /// builds it, so a policy that stopped being applied there fails here.
    fn session() -> reqwest::Client {
        session_client(&a_session()).expect("the session client builds")
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

    /// The custody assertion this whole policy exists for.
    ///
    /// Tes's bundle download answers a 302 to a signed CDN url on another
    /// host. `default_headers` are re-sent on every hop, so a client that
    /// followed it would hand the seller's session cookie to that host, and
    /// neither host assertion would see it: `route` runs once, against the url
    /// the caller named. A 200 here would be that leak.
    #[tokio::test]
    async fn the_session_client_does_not_follow_a_redirect_to_another_host() {
        let (url, server) = redirecting(|port| format!("http://localhost:{port}/signed"));
        let answer = send_over(&session(), HttpRequest::get(url))
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
        let answer = send_over(&session(), HttpRequest::get(url))
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
        send_over(&session(), HttpRequest::get(url))
            .await
            .expect("the probe server answers");
        let head = server.join().expect("the probe server finishes");
        assert!(
            head.contains("cookie: tessession=secret"),
            "the control: the probe can see a cookie when one is sent, and saw: {head}"
        );
    }
}
