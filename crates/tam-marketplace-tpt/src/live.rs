//! The live transport: reqwest over rustls with mandatory timeouts. The whole
//! auth envelope — the cookie jar, the mirrored CSRF header, the origin and
//! the XHR marker — is injected at construction, so it exists in no request
//! value and no cassette.
//!
//! The one header that cannot be a construction default is the gateway's
//! `x-gateway-auth-version`, because the two services share a client and are
//! told apart by the URL the seam already carries.

use core::error::Error as _;

use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestBody, Transport, TransportError,
};
use tam_marketplace::ConnectFailure;

use crate::endpoints::{self, ORIGIN};
use crate::session::TptSession;

/// Mirrors the `csrfToken` cookie; the double-submit pair TPT validates.
const CSRF_HEADER: &str = "x-csrf-token";
/// Sent by the TPT client on every API call, on both services.
const REQUESTED_WITH_HEADER: &str = "x-requested-with";
/// The single header that distinguishes a gateway call from a graph call.
const GATEWAY_VERSION_HEADER: &str = "x-gateway-auth-version";
const GATEWAY_VERSION: &str = "2";

pub struct ReqwestTransport {
    client: reqwest::Client,
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

impl ReqwestTransport {
    pub fn new(session: &TptSession) -> Result<Self, TransportBuildError> {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut set = |name: reqwest::header::HeaderName, value: &str| {
            reqwest::header::HeaderValue::from_str(value)
                .map(|encoded| headers.insert(name, encoded))
                .map(|_| ())
                .map_err(|error| TransportBuildError(error.to_string()))
        };
        set(reqwest::header::COOKIE, session.header_value())?;
        set(static_name(CSRF_HEADER), session.csrf_token())?;
        set(static_name(REQUESTED_WITH_HEADER), "XMLHttpRequest")?;
        set(reqwest::header::ORIGIN, ORIGIN)?;
        set(reqwest::header::USER_AGENT, "Mozilla/5.0")?;
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(core::time::Duration::from_secs(30))
            .connect_timeout(core::time::Duration::from_secs(10))
            .build()
            .map_err(|error| TransportBuildError(error.to_string()))?;
        Ok(Self { client })
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

impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let builder = match request.method {
            Method::Get => self.client.get(&request.url),
            Method::Post => self.client.post(&request.url),
            Method::Put => self.client.put(&request.url),
            Method::Delete => self.client.delete(&request.url),
        };
        let builder = if endpoints::is_gateway(&request.url) {
            builder.header(static_name(GATEWAY_VERSION_HEADER), GATEWAY_VERSION)
        } else {
            builder
        };
        let builder = match request.body {
            RequestBody::Empty => builder,
            RequestBody::Json(value) => builder.json(&value),
            // No TPT flow this crate builds posts a multipart form: the write
            // path that does is capture-gated in the M7 plan, so sending one
            // here would be a shape nothing has observed.
            RequestBody::Multipart { .. } => {
                return Err(TransportError::NotSent(ConnectFailure::NoRouteToHost))
            }
        };
        let response = builder
            .send()
            .await
            .map_err(|error| classify_reqwest(&error))?;
        let status = response.status().as_u16();
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
        })
    }
}
