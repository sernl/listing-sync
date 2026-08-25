//! The lease gateway: the JSON-API-era form of the design's "driver endpoint
//! for a browser the broker primed". Per lease the broker binds an ephemeral
//! loopback listener and proxies allow-listed upstream routes with the seller
//! cookie injected server-side, so a worker uses a connection and cannot read
//! one. The allow-list is enforced here in Rust — a route outside it is 403,
//! by a test rather than a review — which makes roster, account-administration
//! and payout routes structurally unreachable.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use tam_secrets::Secret;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// The write-path prefixes a lease may reach, and nothing else. Adding one is
/// a deliberate edit here, guarded by the refusal test.
pub(crate) const ALLOWED_PREFIXES: [&str; 2] = ["/api/v2/resources", "/api/resources/v3/draft"];

fn path_is_allowed(path: &str) -> bool {
    ALLOWED_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

struct GatewayState {
    upstream_base: String,
    cookie: Secret,
    client: reqwest::Client,
}

/// A bound gateway: its loopback address and the token that aborts it. The
/// task drops when the token fires, closing the listener.
pub(crate) struct Gateway {
    pub(crate) endpoint: String,
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

/// Binds an ephemeral loopback gateway proxying to `upstream_base` with
/// `cookie` injected. Returns once bound; serves until the token is cancelled.
pub(crate) async fn spawn(
    upstream_base: String,
    cookie: Secret,
    parent: &CancellationToken,
) -> Result<Gateway, GatewayError> {
    let client = reqwest::Client::builder()
        .timeout(core::time::Duration::from_secs(30))
        .build()
        .map_err(|error| GatewayError(error.to_string()))?;
    let state = Arc::new(GatewayState {
        upstream_base,
        cookie,
        client,
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
        cancel,
    })
}

async fn proxy(State(state): State<Arc<GatewayState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    if !path_is_allowed(path) {
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

    let method = parts.method.clone();
    let mut outbound = state.client.request(method.clone(), &url);
    if let Some(content_type) = parts.headers.get(axum::http::header::CONTENT_TYPE) {
        outbound = outbound.header(axum::http::header::CONTENT_TYPE, content_type);
    }
    outbound = outbound
        .header(axum::http::header::COOKIE, state.cookie.expose())
        .body(bytes.to_vec());

    match outbound.send().await {
        Ok(upstream) => relay(upstream).await,
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            format!("upstream unreachable: {error}"),
        )
            .into_response(),
    }
}

async fn relay(upstream: reqwest::Response) -> Response {
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| HeaderValue::from_str(value).ok());
    let body = upstream.bytes().await.unwrap_or_else(|_| Bytes::new());
    let mut response = (status, body).into_response();
    if let Some(content_type) = content_type {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, content_type);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::path_is_allowed;

    #[test]
    fn the_allow_list_admits_the_write_paths_and_refuses_the_rest() {
        assert!(path_is_allowed("/api/v2/resources"), "create is admitted");
        assert!(
            path_is_allowed("/api/v2/resources/9001/draft"),
            "metadata is admitted"
        );
        assert!(
            path_is_allowed("/api/resources/v3/draft/9001/attachment"),
            "the attachment path is admitted"
        );
        assert!(
            !path_is_allowed("/api/v2/account/roster"),
            "account administration is refused"
        );
        assert!(
            !path_is_allowed("/api/v2/payouts"),
            "payout routes are refused"
        );
        assert!(
            !path_is_allowed("/api/v2/resourcesX"),
            "a prefix that is not a path boundary is refused"
        );
    }
}
