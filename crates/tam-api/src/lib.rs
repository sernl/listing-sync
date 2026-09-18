//! The HTTP API as a library, so integration tests drive the router
//! in-process with no bound port.
//!
//! The crate owns the router, the extractors and the error mapping; a binary
//! that serves it owns a listener and nothing else. Every route mounted under
//! `/{version}` receives its version through the [`APIVersion`] extractor.
//! That parameter matches any first segment whatever, so a binary that mounts
//! anything behind this router asks [`claims_path`] which requests are the
//! API's before letting it answer: `/{version}/mappings` matches the console's
//! own `/automations/mappings`, and without that guard a browser asking for a
//! page was handed mapping JSON, or a 401. Where the guard is in place an
//! unknown but version-shaped first segment reaches whatever is mounted behind
//! the API rather than the structured refusal the extractor writes; that
//! refusal is what a deployment serving the API alone still answers, because
//! there nothing else could. The tenant boundary is the [`OrgContext`]
//! extractor; a handler that forgets it cannot name an organisation, because
//! no other source of one exists. The operator surface
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
pub mod collections;
pub mod consent;
pub mod devices;
pub mod duplicates;
pub mod entitlement;
pub mod error;
pub mod exchange_rates;
pub mod export;
pub mod guides;
pub mod import;
pub mod import_batch;
pub mod import_runs;
pub mod jobs;
pub mod library;
pub mod marketplace_requests;
pub mod matcher;
pub mod migrations;
pub mod notifications;
pub mod openapi;
pub mod org;
pub mod paddle;
pub mod product;
pub mod profile;
pub mod resource_templates;
pub mod resources;
pub mod scheduler;
pub mod schedules;
pub mod seller_rules;
pub mod session;
pub mod stream;
pub mod sync_activity;
pub mod sync_settings;
pub mod taxonomy;
pub mod template_apply;
pub mod text;
pub mod version;
pub mod vocabulary;
pub mod work;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, patch, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, UserId};

pub use crate::{
    auth::{
        AuthBridge, JwkSet, JwksFuture, JwksSource, JwksUnavailable, VerifiedSubject, AUDIENCE,
    },
    billing::{
        BillingView, PriceMap, PricedPlan, SubscriptionView, WebhookSecret, ORG_CUSTOM_DATA_KEY,
    },
    catalogue::{FileHandle, UploadedView},
    entitlement::{Entitlement, EntitlementView, PlansView, QuotaKind},
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
    /// Which Paddle price identifier sells which plan and rung.
    ///
    /// Empty by default, which is the fail-closed posture: a completed
    /// transaction naming a price this map does not know grants nothing and
    /// says so on the log, rather than being guessed into the largest rung.
    pub paddle_price_map: billing::PriceMap,
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
    /// Public reference data only; no marketplace credentials or requests.
    pub exchange_rates: Option<std::sync::Arc<dyn exchange_rates::ExchangeRateSource>>,
}

/// The two values an upload needs and no other route does: the key every
/// tenant's blob DEK is wrapped under, and the store the sealed objects are
/// written into. Held here rather than in [`Config`] because a key is not a
/// value that belongs in a `Debug` render of the configuration.
#[derive(Clone)]
pub struct BlobStore {
    pub kek: tam_secrets::Kek,
    pub backend: tam_blob_store::BlobBackend,
}

impl BlobStore {
    /// The store one request writes through. Every handler reaches the object
    /// store this way, so which backend a deployment took is decided once, at
    /// start-up, and read nowhere else.
    #[must_use]
    pub fn object_store(&self) -> tam_blob_store::AnyObjectStore {
        self.backend.object_store()
    }

    /// The local-directory form, which is what development, the tests and
    /// every deployment that has not moved its objects use.
    #[must_use]
    pub fn local(kek: tam_secrets::Kek, root: std::path::PathBuf) -> Self {
        Self {
            kek,
            backend: tam_blob_store::BlobBackend::Local(root),
        }
    }
}

impl AppState {
    /// The one way a storage or infrastructure fault becomes a response:
    /// through the disclosure split. Trace identifiers arrive with the
    /// observability milestone; until then the entry is deliberately
    /// untraced rather than pseudo-traced.
    ///
    /// And the one place the internals are written down. `APIErrorEntry`
    /// holds no logger and says so: under `Disclosure::Redacted` — which is
    /// every deployment a seller reaches — the text it is handed is dropped,
    /// so a caller that does not log it has destroyed the only copy. That is
    /// how nine migrations refused by a stale idempotency key produced a 500
    /// with nothing in `kubectl logs` to say which key or which resource.
    #[must_use]
    pub fn internal(&self, internals: &str) -> APIError {
        eprintln!("tam-api: internal fault: {internals}");
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
///
/// It carries the organisation's slug and the prompt that slug calls for
/// because the console's own gate is answered here and nowhere earlier: the
/// client's route load calls this before the console renders, so a seller who
/// has claimed no slug meets the claim screen rather than a console they would
/// have to leave again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Whoami {
    pub org: OrgId,
    pub user: UserId,
    pub slug: Option<String>,
    pub slug_prompt: org::SlugPrompt,
}

/// Every route this build serves outside a version prefix.
///
/// Listed rather than derived, because nothing can derive it: axum's table is
/// not introspectable, so this is the one place [`claims_path`] can learn that
/// `/healthz` is the API's. A route added to [`router`] without a version and
/// not named here is a route whatever is mounted behind the API swallows.
const UNVERSIONED_ROUTES: [&str; 1] = ["/healthz"];

/// Whether the API owns `path`, which is the question a binary mounting
/// anything behind this router has to ask before it lets the router answer.
///
/// The route table cannot answer it. Every versioned route begins with a
/// `{version}` parameter, and a parameter matches any segment:
/// `/{version}/mappings` claims the console's `/automations/mappings` with
/// `automations` for a version, and `/{version}/status`, `/{version}/library`
/// and every other one-word collection claim a console page of the same name.
/// The closed version set is what separates the two, so ownership is decided
/// from [`APIVersion::from_path_prefix`] and from the list above, and from
/// nothing else.
#[must_use]
pub fn claims_path(path: &str) -> bool {
    APIVersion::from_path_prefix(path).is_some() || UNVERSIONED_ROUTES.contains(&path)
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
        .route("/{version}/org", get(org::org_view).patch(org::update_org))
        .route("/{version}/org/slug/{slug}", get(org::slug_availability))
        .route("/{version}/billing", get(billing::billing_view))
        .route("/{version}/billing/webhook", post(billing::webhook))
        // The price list, unauthenticated: the pricing page is public, and a
        // price a seller cannot read before signing up is not a price list.
        .route("/{version}/plans", get(entitlement::plans_view))
        // What this tenant holds, where it came from, and what it has used.
        .route("/{version}/entitlement", get(entitlement::entitlement_view))
        .route(
            "/{version}/jobs",
            post(jobs::create_job).get(jobs::list_jobs),
        )
        .route(
            "/{version}/sync",
            post(jobs::create_sync_request).get(jobs::list_sync_requests),
        )
        // The seller's own two clocks. Both literal segments are listed
        // before `/{version}/sync/{request}`, because `settings`, `activity`
        // and `multi` are words and a request identifier is a parameter: a
        // sync request is never called "settings", and the order is what
        // makes that true of the router rather than only of the vocabulary.
        .route(
            "/{version}/sync/settings",
            get(sync_settings::list_settings),
        )
        .route(
            "/{version}/sync/settings/{inventory}",
            put(sync_settings::update_setting),
        )
        .route("/{version}/sync/activity", get(sync_activity::activity))
        .route("/{version}/sync/multi", get(sync_activity::multi_listed))
        .route(
            "/{version}/sync/{request}",
            get(jobs::sync_request_view).delete(jobs::delete_sync_request),
        )
        // Publishing on a timetable. A schedule is a row these routes write
        // and `scheduler::pass` reads; nothing here fires one.
        .route(
            "/{version}/schedules",
            get(schedules::list_schedules).post(schedules::create_schedule),
        )
        .route(
            "/{version}/schedules/{schedule}",
            put(schedules::update_schedule).delete(schedules::delete_schedule),
        )
        .route(
            "/{version}/schedules/{schedule}/runs",
            get(schedules::schedule_runs),
        )
        // Copying and moving between two marketplaces. The plan is a read
        // that writes nothing and is listed before the collection so the
        // literal segment matches ahead of any identifier the route family
        // grows later.
        .route(
            "/{version}/migrations/plan",
            post(migrations::plan_migration),
        )
        .route("/{version}/migrations", post(migrations::create_migration))
        .route(
            "/{version}/seller-rules",
            get(seller_rules::list).post(seller_rules::create),
        )
        .route(
            "/{version}/seller-rules/presets",
            get(seller_rules::presets),
        )
        .route(
            "/{version}/seller-rules/reference",
            get(seller_rules::reference),
        )
        .route(
            "/{version}/seller-rules/preview",
            post(seller_rules::preview),
        )
        .route(
            "/{version}/seller-rules/previews/{preview}",
            get(seller_rules::read_preview),
        )
        .route(
            "/{version}/seller-rules/previews/{preview}/decision",
            post(seller_rules::decide),
        )
        .route(
            "/{version}/seller-rules/{rule}",
            put(seller_rules::update).delete(seller_rules::delete),
        )
        .route(
            "/{version}/jobs/{job}",
            get(jobs::job_view).delete(jobs::delete_job),
        )
        .route("/{version}/jobs/{job}/items", get(jobs::job_items))
        .route("/{version}/jobs/{job}/items/{item}", get(jobs::item_detail))
        .route("/{version}/events/stream", get(stream::events_stream))
        .route(
            "/{version}/uploads",
            post(catalogue::upload).layer(catalogue::upload_body_limit()),
        )
        // The create form's own preview: a cover is generated during the
        // upload, before any product exists to address it through, so the
        // handle the upload answered is what the form has to read it by.
        .route(
            "/{version}/uploads/{handle}",
            get(resources::uploaded_image),
        )
        .route(
            "/{version}/products",
            get(resources::list_products).post(catalogue::create_product),
        )
        .route("/{version}/products/export", get(export::export_catalogue))
        // The spreadsheet import. The template is generated from the
        // registry; an upload is parsed and held and creates nothing. The
        // upload's body ceiling is its own rather than the resource upload's,
        // because an xlsx is a zip parsed into cells and not a payload.
        .route("/{version}/imports/template", get(import_batch::template))
        // The marketplace import, as a run. Listed before `/imports/{batch}`
        // in the file and matched before it by the router, because `runs` is a
        // literal segment and a batch identifier is a parameter: a run is
        // never a batch, and the one-letter-apart paths are why the nav claims
        // `/imports` by name.
        .route(
            "/{version}/imports/runs",
            post(import_runs::create_run).get(import_runs::list_runs),
        )
        .route(
            "/{version}/imports/runs/{run}",
            get(import_runs::run_view).delete(import_runs::delete_run),
        )
        .route(
            "/{version}/imports/runs/{run}/select",
            post(import_runs::select),
        )
        // This records confirmation; the server's drain owns catalogue writes.
        .route(
            "/{version}/imports/runs/{run}/commit",
            post(import_runs::confirm),
        )
        .route(
            "/{version}/imports/runs/{run}/abandon",
            post(import_runs::abandon),
        )
        // The cover a read produced, before any product exists to address it
        // through. Keyed on the ordinal rather than the locator, because a
        // locator is a URL and a path segment is not where one goes.
        .route(
            "/{version}/imports/runs/{run}/items/{ordinal}/cover",
            get(import_runs::item_cover),
        )
        // The duplicate review. Addressed by the pair rather than by a
        // question identifier, because the pair *is* the identity: one pair is
        // one row, and "have we asked already" is a primary-key lookup.
        .route("/{version}/duplicates", get(duplicates::list_duplicates))
        .route("/{version}/duplicates/{lo}/{hi}", post(duplicates::decide))
        .route(
            "/{version}/duplicates/{lo}/{hi}/undo",
            post(duplicates::undo),
        )
        // The body ceiling is applied before the listing is added, because
        // `MethodRouter::layer` reaches the handlers already on the router and
        // not the ones added after it: the upload gets the larger limit and
        // the listing keeps the default, which is what each needs.
        .route(
            "/{version}/imports",
            post(import_batch::upload)
                .layer(import_batch::spreadsheet_body_limit())
                .get(import_batch::list),
        )
        .route(
            "/{version}/imports/{batch}",
            get(import_batch::view).delete(import_batch::abandon),
        )
        // The bind carries two handles and never bytes, so it keeps the
        // default body ceiling rather than either upload's: the bytes reached
        // `POST /{version}/uploads` already and this names what they were for.
        .route(
            "/{version}/imports/{batch}/rows/{sheet}/{ordinal}/file",
            post(import_batch::attach::bind).delete(import_batch::attach::unbind),
        )
        // The commit takes no body and creates a page of rows per call, so it
        // keeps the default ceiling too: the chunking is the server's, and a
        // client asks for the next page by asking again.
        .route(
            "/{version}/imports/{batch}/commit",
            post(import_batch::commit::commit),
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
        // A sub-resource rather than a `files` array on the PATCH above: that
        // route's contract is that an absent field is left as stored, so a
        // whole-array replace could not tell a removal from a form that did
        // not render the field. Bytes still arrive only at `POST /uploads`;
        // these three name a handle it already returned.
        .route(
            "/{version}/products/{product}/files",
            post(catalogue::add_file),
        )
        .route(
            "/{version}/products/{product}/files/{file}",
            put(catalogue::replace_file).delete(catalogue::remove_file),
        )
        // The one read that hands a browser a resource's picture. Its own
        // route rather than a field of bytes on the product view: a thumbnail
        // is fetched by the img element itself, cached by the browser, and
        // asked for once per row.
        .route(
            "/{version}/products/{product}/cover",
            get(resources::product_cover),
        )
        .route(
            "/{version}/products/{product}/labels",
            get(resources::product_labels).put(resources::set_product_labels),
        )
        // Which collections a resource is in, which is the resource page's
        // own panel. A sub-resource of the product rather than a query on the
        // collection list, because the question is about this resource.
        .route(
            "/{version}/products/{product}/collections",
            get(collections::for_product),
        )
        // The Template Manager's second tab: a named partial draft of the
        // create form, saved to prefill the next one.
        .route(
            "/{version}/templates",
            get(resource_templates::list).post(resource_templates::create),
        )
        .route(
            "/{version}/templates/{template}",
            get(resource_templates::get)
                .patch(resource_templates::update)
                .delete(resource_templates::delete),
        )
        // Applying one to resources the catalogue already holds. The plan is
        // a read that writes nothing and is listed before the confirm, so the
        // longer literal path matches ahead of the shorter one.
        .route(
            "/{version}/templates/{template}/apply/plan",
            post(template_apply::plan_apply),
        )
        .route(
            "/{version}/templates/{template}/apply",
            post(template_apply::apply),
        )
        // A named, ordered set of resources the seller acts on together. The
        // two publish paths are listed before the collection's own routes for
        // the reason the migration plan is listed before the migration: a
        // literal segment has to match ahead of any identifier the family
        // grows later.
        .route(
            "/{version}/collections/{collection}/publish/plan",
            post(collections::plan_publish),
        )
        .route(
            "/{version}/collections/{collection}/publish",
            post(collections::publish),
        )
        .route(
            "/{version}/collections/{collection}/members",
            put(collections::set_members),
        )
        .route(
            "/{version}/collections/{collection}/labels",
            put(collections::add_labels),
        )
        .route(
            "/{version}/collections",
            get(collections::list).post(collections::create),
        )
        .route(
            "/{version}/collections/{collection}",
            get(collections::get)
                .put(collections::update)
                .delete(collections::delete),
        )
        .route("/{version}/labels", get(resources::list_labels))
        // Named by the label's own text, because that is how a label is
        // identified everywhere else on this surface: the client never learns
        // an identifier for one.
        .route(
            "/{version}/labels/{name}",
            patch(resources::rename_label).delete(resources::delete_label),
        )
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
        // The way back from that one, and a route rather than something the
        // heartbeat does: a machine returns to the fleet because a seller
        // signed in at it asked, not because it restarted.
        .route(
            "/{version}/devices/{device}/restore",
            post(devices::restore_device),
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
        // What the seller ticked, for the device to describe. A read rather
        // than a field on the work claim, because an import is started by the
        // console and the device asks what it is for.
        // What is open for this device to read, asked at every check-in.
        // Listed before the selection read because `open` is a literal
        // segment and a run identifier is a parameter: a run is never called
        // "open".
        .route(
            "/{version}/devices/{device}/import/open",
            get(import_runs::device_open_runs),
        )
        .route(
            "/{version}/devices/{device}/import/{run}/claim",
            post(import_runs::claim),
        )
        .route(
            "/{version}/devices/{device}/import/{run}/renew",
            post(import_runs::renew),
        )
        .route(
            "/{version}/devices/{device}/import/{run}/progress",
            post(import_runs::progress),
        )
        .route(
            "/{version}/devices/{device}/import/{run}/stop",
            post(import_runs::device_stop),
        )
        .route(
            "/{version}/devices/{device}/import/{run}/selection",
            get(import_runs::device_selection),
        )
        .route(
            "/{version}/devices/{device}/payload/{file}",
            get(work::payload),
        )
        .route("/{version}/connections", get(resources::list_connections))
        // The seller's explicit permission for a no-API marketplace, per
        // organisation: read by every device, granted at Connect or on the
        // Account page, and what every mint of seller-device work checks.
        .route("/{version}/consents", get(consent::list_consents))
        .route(
            "/{version}/consents/{marketplace}",
            post(consent::grant_consent),
        )
        .route(
            "/{version}/consents/{marketplace}/withdraw",
            post(consent::withdraw_consent),
        )
        // The seller's own files, on the seller's own machines: who holds
        // what, and where one machine can reach another. Coordination only;
        // no route here carries a byte of a file.
        .route("/{version}/library", get(library::list_library))
        .route(
            "/{version}/devices/{device}/library/want",
            post(library::want).delete(library::unwant),
        )
        .route(
            "/{version}/devices/{device}/library/wants",
            get(library::wants),
        )
        .route(
            "/{version}/devices/{device}/library/peers",
            get(library::peers),
        )
        .route(
            "/{version}/connections/{connection}/revoke",
            post(resources::revoke_connection),
        )
        // The seller's own disconnect, beside the operator's revoke and
        // deliberately not the same write: this one is reversible, and the
        // next check-in from a machine still holding the login lifts it back.
        .route(
            "/{version}/connections/{connection}/disconnect",
            post(resources::disconnect_connection),
        )
        // Beside the connections it is about: what a seller asks for when the
        // marketplace they sell on is not one of the three there is a
        // connection for.
        .route(
            "/{version}/marketplace-requests",
            post(marketplace_requests::create),
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
        // The best-fit tick, per marketplace rather than per axis, because
        // that is the control: one checkbox at the head of a marketplace tab.
        // It writes a durable rule per delegable axis and the untick withdraws
        // them, so the delegation is auditable and revocable rather than a
        // preference held in a browser.
        .route(
            "/{version}/elections/delegation",
            get(resources::list_delegations).put(resources::set_delegation),
        )
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
        .route("/{version}/notifications", get(notifications::list))
        .route(
            "/{version}/notifications/read",
            post(notifications::mark_read),
        )
        .route(
            "/{version}/notifications/preferences",
            get(notifications::preferences).patch(notifications::update_preferences),
        )
        // The seller's own picture, beside the other per-user setting. No
        // route here takes a user: the session's is the only one any of them
        // can read or write, and the bytes reach `POST /{version}/uploads`
        // first.
        .route("/{version}/profile", get(profile::profile_view))
        .route(
            "/{version}/profile/avatar",
            get(profile::avatar)
                .put(profile::set_avatar)
                .delete(profile::clear_avatar),
        )
        // The help corpus, read by every seller through the session gate.
        // `/{version}/guides/images/{handle}` is listed before
        // `/{version}/guides/{slug}` because `images` is a literal segment
        // and a slug is a parameter: no guide is called "images", and the
        // order is what makes that true of the router rather than only of the
        // vocabulary.
        .route("/{version}/guides", get(guides::published_guides))
        .route("/{version}/guides/_taxonomy", get(guides::guide_taxonomy))
        .route(
            "/{version}/guides/images/{handle}",
            get(guides::guide_image),
        )
        .route("/{version}/guides/{slug}", get(guides::published_guide))
        .route("/{version}/admin/signups", get(admin::signups))
        .route("/{version}/admin/orgs", get(admin::list_orgs))
        .route("/{version}/admin/orgs/{org}", get(admin::org_detail))
        // The one write on the operator surface. It goes through the
        // application pool with the target organisation pinned, because the
        // backoffice role is SELECT-only by grant and widening it would
        // create a second, unfenced way to change what a tenant holds.
        .route("/{version}/admin/orgs/{org}/plan", post(admin::grant_plan))
        .route(
            "/{version}/admin/orgs/{org}/plan/{grant}/revoke",
            post(admin::revoke_plan),
        )
        .route("/{version}/admin/sync-health", get(admin::sync_health))
        .route("/{version}/admin/failed-writes", get(admin::failed_writes))
        .route("/{version}/admin/import-drain", get(admin::import_drain))
        .route("/{version}/admin/dead-letters", get(admin::dead_letters))
        .route(
            "/{version}/admin/impersonations",
            get(admin::impersonations),
        )
        .route(
            "/{version}/admin/marketplace-requests",
            get(marketplace_requests::list_all),
        )
        // The operator's half of the same corpus: the listing and the create,
        // then one guide, then the pictures a guide body points at. The
        // picture upload carries its own body ceiling, as the seller's upload
        // does, because a route that accepts bytes without one accepts any
        // number of them.
        .route(
            "/{version}/admin/guides",
            get(guides::list_guides).post(guides::create_guide),
        )
        .route(
            "/{version}/admin/guides/images",
            post(guides::upload_guide_image).layer(guides::image_body_limit()),
        )
        .route(
            "/{version}/admin/guides/_preview",
            post(guides::preview_guide),
        )
        .route(
            "/{version}/admin/guides/_taxonomy",
            get(guides::admin_guide_taxonomy),
        )
        .route(
            "/{version}/admin/guides/_taxonomy/{kind}",
            post(guides::create_guide_taxon),
        )
        .route(
            "/{version}/admin/guides/_taxonomy/{kind}/{id}",
            put(guides::update_guide_taxon),
        )
        .route(
            "/{version}/admin/guides/{slug}",
            get(guides::guide_detail)
                .put(guides::save_guide)
                .delete(guides::delete_guide),
        )
        .route(
            "/{version}/admin/guides/{slug}/publish",
            post(guides::publish_guide),
        )
        .route(
            "/{version}/admin/guides/{slug}/unpublish",
            post(guides::unpublish_guide),
        )
        .route("/{version}/admin/users", get(admin::list_users))
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

async fn whoami(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<Whoami>, APIError> {
    let (slug, slug_prompt) = org::slug_state(&state, context.org).await?;
    Ok(Json(Whoami {
        org: context.org,
        user: context.user,
        slug,
        slug_prompt,
    }))
}

#[cfg(test)]
mod tests {
    use super::{claims_path, APIVersion, UNVERSIONED_ROUTES};

    /// What the guard in front of this router is allowed to hand on, and what
    /// it has to hand back.
    #[test]
    fn the_api_claims_its_versions_and_its_unversioned_probe() {
        for version in APIVersion::SUPPORTED {
            let path = format!("/{version}/openapi.json");
            assert!(
                claims_path(&path),
                "{path} is a route this build serves and must reach it"
            );
        }
        for path in UNVERSIONED_ROUTES {
            assert!(
                claims_path(path),
                "{path} is mounted without a version, so only this list can save it"
            );
        }
        for path in [
            "/automations/mappings",
            "/settings/notifications",
            "/resources",
            "/library",
            "/login",
            "/",
        ] {
            assert!(
                !claims_path(path),
                "{path} is a console page, whatever the route table matches"
            );
        }
    }
}
