//! The HTTP API as a library, so integration tests drive the router
//! in-process with no bound port.
//!
//! The crate owns the router, the extractors and the error mapping; a binary
//! that serves it owns a listener and nothing else. Every route mounted under
//! `/{version}` receives its version through the [`APIVersion`] extractor, so
//! an unknown version is refused with the same structured body as any other
//! fault rather than falling through to a bare not-found. The tenant boundary
//! is the [`OrgContext`] extractor; a handler that forgets it cannot name an
//! organisation, because no other source of one exists.

#![forbid(unsafe_code)]

pub mod error;
pub mod jobs;
pub mod openapi;
pub mod resources;
pub mod session;
pub mod stream;
pub mod version;

use axum::{
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, UserId};

pub use crate::{
    error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind, Disclosure},
    session::{OrgContext, StreamAuth, SESSION_COOKIE},
    version::{APIVersion, VersionError},
};

/// Everything the serving binary decides and the library consumes. One value
/// crosses the boundary so a decision cannot arrive ambiently; handlers read
/// it from router state. Its `Default` is the fail-closed posture.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    pub disclosure: Disclosure,
    /// The credential broker's unix socket; revocation answers 503 without
    /// it rather than pretending.
    pub broker_socket: Option<std::path::PathBuf>,
}

/// How the current instant enters a handler: as a function the binary
/// supplies, because the lint table bans ambient clock reads and a test
/// injects a fixed one here.
pub type WallClock = fn() -> Timestamp;

/// The router's state: the pool, the binary's decisions, and the clock.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Config,
    pub wall: WallClock,
}

impl AppState {
    /// The one way a storage or infrastructure fault becomes a response:
    /// through the disclosure split. Trace identifiers arrive with the
    /// observability milestone; until then the entry is deliberately
    /// untraced rather than pseudo-traced.
    #[must_use]
    pub fn internal(&self, internals: &str) -> APIError {
        APIError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            APIErrorEntry::internal(self.config.disclosure, internals, "untraced"),
        )
    }
}

/// What a versioned health probe answers with. The version is echoed back
/// because it is the extractor's own output, which is what the probe exists
/// to exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub version: APIVersion,
}

/// Who the session speaks for, echoed back; the client's first authenticated
/// call and the session floor's own probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Whoami {
    pub org: OrgId,
    pub user: UserId,
}

/// The whole API surface this build serves, configured by the binary.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/{version}/healthz", get(versioned_healthz))
        .route("/{version}/whoami", get(whoami))
        .route(
            "/{version}/session",
            post(session::exchange).delete(session::logout),
        )
        .route(
            "/{version}/jobs",
            post(jobs::create_job).get(jobs::list_jobs),
        )
        .route("/{version}/jobs/{job}", get(jobs::job_view))
        .route("/{version}/jobs/{job}/items", get(jobs::job_items))
        .route("/{version}/jobs/{job}/items/{item}", get(jobs::item_detail))
        .route("/{version}/events/stream", get(stream::events_stream))
        .route("/{version}/products", get(resources::list_products))
        .route(
            "/{version}/products/{product}",
            get(resources::product_view),
        )
        .route("/{version}/connections", get(resources::list_connections))
        .route(
            "/{version}/connections/{connection}/revoke",
            post(resources::revoke_connection),
        )
        .route(
            "/{version}/reconciliation/items",
            get(resources::list_queue),
        )
        .route(
            "/{version}/reconciliation/items/{item}/resolve",
            post(resources::resolve_item),
        )
        .route(
            "/{version}/reconciliation/items/{item}/no-counterpart",
            post(resources::no_counterpart_item),
        )
        .route(
            "/{version}/reconciliation/stats",
            get(resources::queue_stats),
        )
        .route("/{version}/mappings", get(resources::list_mappings))
        .route("/{version}/status", get(resources::status))
        .route("/{version}/openapi.json", get(openapi::serve_document))
        .with_state(state)
}

/// The unversioned liveness probe, which predates any version negotiation and
/// therefore takes no extractor.
async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn versioned_healthz(version: APIVersion) -> Json<Health> {
    Json(Health { version })
}

async fn whoami(_version: APIVersion, context: OrgContext) -> Json<Whoami> {
    Json(Whoami {
        org: context.org,
        user: context.user,
    })
}
