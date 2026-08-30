//! The HTTP API as a library, so integration tests drive the router
//! in-process with no bound port.
//!
//! The crate owns the router, the extractors and the error mapping; a binary
//! that serves it owns a listener and nothing else. Every route mounted under
//! `/{version}` receives its version through the [`APIVersion`] extractor, so
//! an unknown version is refused with the same structured body as any other
//! fault rather than falling through to a bare not-found. The tenant boundary
//! is the [`OrgContext`] extractor; a handler that forgets it cannot name an
//! organisation, because no other source of one exists. The operator surface
//! under `/{version}/admin` is the one exception to that keying, and it is a
//! different extractor rather than a flag on the same one: [`OperatorContext`]
//! names no organisation at all, and the pool it reads through is a second
//! one whose privileges are enumerated table by table.

#![forbid(unsafe_code)]

pub mod admin;
pub mod analytics;
pub mod auth;
pub mod billing;
pub mod error;
pub mod jobs;
pub mod openapi;
pub mod org;
pub mod paddle;
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
    auth::{
        AuthBridge, JwkSet, JwksFuture, JwksSource, JwksUnavailable, VerifiedSubject, AUDIENCE,
    },
    billing::{BillingView, SubscriptionView, WebhookSecret, ORG_CUSTOM_DATA_KEY},
    error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind, Disclosure},
    session::{OperatorContext, OrgContext, StreamAuth, SESSION_COOKIE},
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
    /// Paddle's notification-webhook secret, which is the only thing that
    /// authenticates the billing webhook. Absent, that route answers 503:
    /// there is no unauthenticated mode of it to fall back to.
    pub paddle_webhook_secret: Option<billing::WebhookSecret>,
}

/// How the current instant enters a handler: as a function the binary
/// supplies, because the lint table bans ambient clock reads and a test
/// injects a fixed one here.
pub type WallClock = fn() -> Timestamp;

/// The router's state: the pool, the binary's decisions, the clock, and the
/// identity service when one is configured.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Config,
    pub wall: WallClock,
    /// The identity bridge, shared across the clone axum makes per request so
    /// one key-set cache serves the whole process. Absent means no identity
    /// service is configured, and a login assertion is refused rather than
    /// verified against nothing.
    pub auth: Option<std::sync::Arc<auth::AuthBridge>>,
    /// The operator backoffice's pool, connected as `tam_backoffice`, whose
    /// reach is the SELECT grants and read policies of migration 0037.
    ///
    /// A second pool rather than a wider grant on the first: the tenant fence
    /// is what makes every other handler safe, and the way to keep a handler
    /// from crossing it by accident is to give it no connection that can.
    /// Absent means this deployment serves no operator surface, and every
    /// `/admin` route refuses.
    pub backoffice: Option<PgPool>,
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
        .route("/{version}/org", get(org::org_view).patch(org::rename_org))
        .route("/{version}/billing", get(billing::billing_view))
        .route("/{version}/billing/webhook", post(billing::webhook))
        .route(
            "/{version}/jobs",
            post(jobs::create_job).get(jobs::list_jobs),
        )
        .route("/{version}/sync", post(jobs::create_sync_request))
        .route("/{version}/sync/{request}", get(jobs::sync_request_view))
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
        .route("/{version}/elections/items", get(resources::list_decisions))
        .route(
            "/{version}/elections/items/{item}/answer",
            post(resources::answer_decision),
        )
        .route(
            "/{version}/elections/items/{item}/withdraw",
            post(resources::withdraw_decision),
        )
        .route("/{version}/elections/rules", post(resources::upsert_rule))
        .route("/{version}/mappings", get(resources::list_mappings))
        .route(
            "/{version}/analytics/summary",
            get(analytics::analytics_summary),
        )
        .route("/{version}/status", get(resources::status))
        .route("/{version}/admin/signups", get(admin::signups))
        .route("/{version}/admin/orgs", get(admin::list_orgs))
        .route("/{version}/admin/orgs/{org}", get(admin::org_detail))
        .route("/{version}/admin/sync-health", get(admin::sync_health))
        .route("/{version}/admin/failed-writes", get(admin::failed_writes))
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
