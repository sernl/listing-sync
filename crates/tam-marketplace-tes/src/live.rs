//! The live transport: reqwest over rustls with mandatory timeouts, the
//! session header injected at construction so it exists in no request value.

use core::error::Error as _;

use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestBody, Transport, TransportError,
};
use tam_marketplace::ConnectFailure;

use crate::session::TesSession;

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

impl ReqwestTransport {
    pub fn new(session: &TesSession) -> Result<Self, TransportBuildError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::COOKIE,
            reqwest::header::HeaderValue::from_str(session.header_value())
                .map_err(|error| TransportBuildError(error.to_string()))?,
        );
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("Mozilla/5.0"),
        );
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
        let builder = match request.method {
            Method::Get => self.client.get(&request.url),
            Method::Post => self.client.post(&request.url),
            Method::Put => self.client.put(&request.url),
            Method::Delete => self.client.delete(&request.url),
        };
        let builder = match request.body {
            RequestBody::Empty => builder,
            RequestBody::Json(value) => builder.json(&value),
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
                builder.multipart(form)
            }
        };
        let response = builder
            .send()
            .await
            .map_err(|error| classify_reqwest(&error))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| TransportError::AfterSend {
                detail: error.to_string(),
            })?;
        Ok(HttpResponse { status, body })
    }
}
