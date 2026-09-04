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
pub mod blocking;
pub mod catalogue;
pub mod devices;
pub mod error;
pub mod import;
pub mod jobs;
pub mod openapi;
pub mod org;
pub mod paddle;
pub mod product;
pub mod quota;
pub mod resources;
pub mod session;
pub mod stream;
pub mod taxonomy;
pub mod version;
pub mod vocabulary;
pub mod work;

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
    catalogue::{FileHandle, UploadedView},
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
    /// Paddle's notification-webhook secret, which is the only thing that
    /// authenticates the billing webhook. Absent, that route answers 503:
    /// there is no unauthenticated mode of it to fall back to.
    pub paddle_webhook_secret: Option<billing::WebhookSecret>,
    /// The Ed25519 signing key for the entitlement tokens the heartbeat mints
    /// under decision D10. Absent in development, and the heartbeat then
    /// answers without a token: the client gate reads that as closed.
    pub entitlement_key: Option<devices::EntitlementKey>,
    /// When the standards crawl that vouches for TPT's node ids was taken.
    ///
    /// A node id is served only where a capture inside this window stands
    /// behind it. Absent means every id is withheld, which is the fail-closed
    /// reading: without a window there is no evidence any id still resolves,
    /// and posting one TPT has since rebuilt puts a listing under a standard
    /// nobody chose.
    pub standards_crawl_window: Option<tam_standards::crawl::CrawlWindow>,
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
    /// Where an upload's bytes are sealed and where the sealed objects live.
    ///
    /// Absent means this deployment was started without a key-encryption key
    /// and an object-store root, and `POST /{version}/uploads` refuses with
    /// 503 rather than accepting bytes it cannot seal — the same posture the
    /// operator surface takes without its backoffice pool, and the same one
    /// revocation takes without the broker socket.
    pub blobs: Option<BlobStore>,
}

/// The two values an upload needs and no other route does: the key every
/// tenant's blob DEK is wrapped under, and the root the sealed objects are
/// written beneath. Held here rather than in [`Config`] because a key is not
/// a value that belongs in a `Debug` render of the configuration.
#[derive(Clone)]
pub struct BlobStore {
    pub kek: tam_secrets::Kek,
    pub root: std::path::PathBuf,
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
        .route(
            "/{version}/sync",
            post(jobs::create_sync_request).get(jobs::list_sync_requests),
        )
        .route("/{version}/sync/{request}", get(jobs::sync_request_view))
        .route("/{version}/jobs/{job}", get(jobs::job_view))
        .route("/{version}/jobs/{job}/items", get(jobs::job_items))
        .route("/{version}/jobs/{job}/items/{item}", get(jobs::item_detail))
        .route("/{version}/events/stream", get(stream::events_stream))
        .route(
            "/{version}/uploads",
            post(catalogue::upload).layer(catalogue::upload_body_limit()),
        )
        .route(
            "/{version}/products",
            get(resources::list_products).post(catalogue::create_product),
        )
        .route(
            "/{version}/products/{product}",
            get(resources::product_view)
                .patch(catalogue::patch_product)
                .delete(catalogue::delete_product),
        )
        .route(
            "/{version}/products/{product}/mappings",
            post(catalogue::add_mapping),
        )
        .route(
            "/{version}/products/{product}/labels",
            get(resources::product_labels).put(resources::set_product_labels),
        )
        .route("/{version}/labels", get(resources::list_labels))
        .route(
            "/{version}/vocabulary/{inventory}",
            get(vocabulary::vocabulary_view),
        )
        .route(
            "/{version}/authoring/vocabulary",
            get(product::form_vocabulary_view),
        )
        .route("/{version}/authoring/check", post(product::check_draft))
        .route(
            "/{version}/standards/search",
            get(product::standards_search),
        )
        .route("/{version}/taxonomy/terms", get(taxonomy::list_terms))
        .route(
            "/{version}/devices",
            get(devices::list_devices).post(devices::register),
        )
        .route(
            "/{version}/devices/{device}/heartbeat",
            post(devices::heartbeat),
        )
        .route(
            "/{version}/devices/{device}/revoke",
            post(devices::revoke_device),
        )
        // Beside the device routes because it is what makes them able to
        // write, and keyed on the marketplace rather than on a device because
        // the declaration outlives every machine that carries it.
        .route(
            "/{version}/connections/{marketplace}/authorship",
            post(devices::declare_authorship),
        )
        // D1's declarative-intent surface: the device asks what is due and
        // reports what it did. The server never says now.
        .route("/{version}/devices/{device}/work", post(work::claim))
        .route("/{version}/devices/{device}/settle", post(work::settle))
        .route("/{version}/devices/{device}/ledger", post(work::ledger))
        // The migration read's server half. The body limit is the upload
        // route's own constant rather than a new number: a page carries up to
        // twenty-five covers, which the device bounds, and inventing a second
        // ceiling here would be a second thing to keep in step with the first.
        .route(
            "/{version}/devices/{device}/import",
            post(import::import_page).layer(catalogue::upload_body_limit()),
        )
        .route(
            "/{version}/devices/{device}/payload/{file}",
            get(work::payload),
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
        .route(
            "/{version}/mappings/overrides",
            post(resources::upsert_override)
                .get(resources::list_overrides)
                .delete(resources::withdraw_override),
        )
        .route("/{version}/mappings", get(resources::list_mappings))
        .route(
            "/{version}/mappings/{mapping}/bind",
            post(resources::bind_mapping),
        )
        .route(
            "/{version}/analytics/summary",
            get(analytics::analytics_summary),
        )
        // The capture's own two, device-scoped: what to read, and the reading
        // sent back. Analytics rather than device routes because that is where
        // a reader looks for them, and the device is named in the path.
        .route(
            "/{version}/devices/{device}/reads",
            get(analytics::read_order).post(analytics::record_capture),
        )
        .route("/{version}/status", get(resources::status))
        .route("/{version}/admin/signups", get(admin::signups))
        .route("/{version}/admin/orgs", get(admin::list_orgs))
        .route("/{version}/admin/orgs/{org}", get(admin::org_detail))
        .route("/{version}/admin/sync-health", get(admin::sync_health))
        .route("/{version}/admin/failed-writes", get(admin::failed_writes))
        .route(
            "/{version}/admin/impersonations",
            get(admin::impersonations),
        )
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
