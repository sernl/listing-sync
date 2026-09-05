//! The operator backoffice: seven reads over the whole platform rather than
//! one tenant.
//!
//! Every route here takes [`OperatorContext`], so the marking is checked
//! before a handler runs and no route can forget it. Every route is a read.
//! Nothing here writes app data, and nothing here creates or modifies an
//! operator: the marking is granted by the `tam-admin` one-shot on the box,
//! so there is no self-elevation endpoint to attack.
//!
//! The cross-tenant queries run on a second pool connected as
//! `tam_backoffice`, whose whole reach is the SELECT grants and read policies
//! of migration 0037. A deployment that passes no `--backoffice-db-url`
//! serves no operator surface at all, rather than half of one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tam_storage::{BackofficeRepo, DailyCount, IdentityAuditRepo, ItemCounts, SignupsRepo};
use tam_types::{FailureCode, InventoryId, MappingId, OrgId, Timestamp, Uuid};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::ConnectionView;
use crate::session::OperatorContext;
use crate::AppState;

/// How many failure rows one page carries. A bound on this answer's own size,
/// so it is a constant beside its caller rather than an entry in `tam-limits`.
const FAILED_WRITES_LIMIT: i64 = 100;

/// How many impersonation rows one page carries, bounded for the reason the
/// failure listing above is.
const IMPERSONATIONS_LIMIT: i64 = 100;

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

/// The backoffice pool, or the refusal a deployment without one answers.
///
/// Reached only after [`OperatorContext`] has already accepted the caller, so
/// a non-operator learns nothing from it: they are refused with the blank 401
/// before this runs. [`APIErrorCode::BackofficeUnavailable`] names the
/// condition so the client can tell an unconfigured operator surface from a
/// fault, which is the difference between a page that explains itself and one
/// that reports an error nobody can act on.
///
/// Shared with `marketplace_requests`, whose operator listing is the one route
/// outside this module that reads across tenants: one refusal, so a deployment
/// without a backoffice database answers every operator route the same way.
pub(crate) fn backoffice(state: &AppState) -> Result<PgPool, APIError> {
    state.backoffice.clone().ok_or_else(|| {
        APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("no backoffice database is configured; nothing was read")
                .code(APIErrorCode::BackofficeUnavailable)
                .kind(APIErrorKind::Internal),
        )
    })
}

fn missing(what: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(what)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_| {
            APIError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                APIErrorEntry::new("the identifier is not a UUID").kind(APIErrorKind::Validation),
            )
        })
}

// ------------------------------------------------------------------ signups

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DayCountView {
    pub day: Timestamp,
    pub count: i64,
}

impl DayCountView {
    fn of(counted: DailyCount) -> Self {
        Self {
            day: counted.day,
            count: counted.count,
        }
    }
}

/// Signups from both planes, newest day first.
///
/// `identity` is absent rather than empty where this database holds no
/// identity schema. The two DDL sets are applied by separate commands, so
/// "nobody signed up" and "the identity audit trail is not visible from here"
/// are different facts and the operator is told which one they are looking at.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignupsView {
    pub provisioned: Vec<DayCountView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Vec<DayCountView>>,
}

pub(crate) async fn signups(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<SignupsView>, APIError> {
    let _configured = backoffice(&state)?;
    let repo = SignupsRepo::new(state.pool.clone());
    let provisioned = repo
        .provisioned_by_day()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let identity = repo
        .identity_by_day()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(SignupsView {
        provisioned: provisioned.into_iter().map(DayCountView::of).collect(),
        identity: identity.map(|days| days.into_iter().map(DayCountView::of).collect()),
    }))
}

// -------------------------------------------------------------- organisations

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgSummaryView {
    pub org: OrgId,
    pub name: String,
    pub slug: Option<String>,
    pub created_at: Timestamp,
    pub products: i64,
    pub mappings: i64,
    pub connections: i64,
    pub users: i64,
}

impl OrgSummaryView {
    fn of(summary: tam_storage::OrgSummary) -> Self {
        Self {
            org: summary.org,
            name: summary.name,
            slug: summary.slug,
            created_at: summary.created_at,
            products: summary.products,
            mappings: summary.mappings,
            connections: summary.connections,
            users: summary.users,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgsView {
    pub orgs: Vec<OrgSummaryView>,
}

pub(crate) async fn list_orgs(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<OrgsView>, APIError> {
    let rows = BackofficeRepo::new(backoffice(&state)?)
        .orgs()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(OrgsView {
        orgs: rows.into_iter().map(OrgSummaryView::of).collect(),
    }))
}

/// A halt as recorded. `inventory` absent is the tenant-wide halt; present is
/// the one raised against a single inventory.
#[derive(Debug, Serialize, Deserialize)]
pub struct HaltView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory: Option<InventoryId>,
    pub reason: String,
    pub raised_by: String,
    pub raised_at: Timestamp,
}

/// What Paddle last said about this organisation's subscription.
///
/// Three fields and no identifier. Paddle's subscription and customer ids are
/// what the tenant's own billing page needs to resume a checkout; an operator
/// reading every tenant is answering "is this one paying, and as of when",
/// which these three answer whole.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionStateView {
    /// Paddle's own vocabulary, passed through rather than translated, for
    /// the reason `billing::SubscriptionView` gives.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_period_end: Option<Timestamp>,
    pub occurred_at: Timestamp,
}

impl SubscriptionStateView {
    fn of(record: tam_storage::SubscriptionRecord) -> Self {
        Self {
            status: record.status,
            current_period_end: record.current_period_end,
            occurred_at: record.occurred_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgDetailView {
    pub org: OrgSummaryView,
    pub connections: Vec<ConnectionView>,
    pub halts: Vec<HaltView>,
    /// Absent for a tenant that has never reached checkout, which is a
    /// different fact from a cancelled subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionStateView>,
}

pub(crate) async fn org_detail(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, org)): Path<(String, String)>,
) -> Result<Json<OrgDetailView>, APIError> {
    let org = OrgId(parse_id(&org)?);
    let detail = BackofficeRepo::new(backoffice(&state)?)
        .org(org, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such organisation"))?;
    Ok(Json(OrgDetailView {
        org: OrgSummaryView::of(detail.summary),
        connections: detail
            .connections
            .into_iter()
            .map(|row| ConnectionView {
                id: row.id,
                marketplace: row.marketplace,
                transport: row.marketplace.transport_class(),
                state: row.state,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
                // The operator surface does not serve the seller's own
                // authorship declaration, and `None` says exactly that: the
                // field is omitted rather than reported as `undeclared`, which
                // would state something false about every seller. It is the
                // seller's statement about their own work rather than a fact
                // about the link, so whether an operator sees it is a founder
                // decision rather than an oversight. Nothing stands in the way
                // of taking it: migration 0037 grants this pool table-level
                // SELECT on `connection` and the backoffice query simply does
                // not select the two authorship columns.
                authorship: None,
            })
            .collect(),
        halts: detail
            .halts
            .into_iter()
            .map(|halt| HaltView {
                inventory: halt.inventory,
                reason: halt.reason,
                raised_by: halt.raised_by,
                raised_at: halt.raised_at,
            })
            .collect(),
        subscription: detail.subscription.map(SubscriptionStateView::of),
    }))
}

// --------------------------------------------------------------- sync health

/// The ledger across every tenant, in the stored state vocabulary rather than
/// the client's collapsed one.
///
/// `jobs::CountsView` folds leased, running and verifying into one figure and
/// both park states into another, which is the right rendering for a seller
/// watching one job and the wrong one here: whether items are parked live or
/// parked cold is the distinction an operator is looking at the page to find.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SyncHealthView {
    pub jobs: i64,
    pub items: u64,
    pub queued: u64,
    pub leased: u64,
    pub running: u64,
    pub blocked: u64,
    pub parked_live: u64,
    pub parked_cold: u64,
    pub verifying: u64,
    pub settled: u64,
    pub succeeded: u64,
    pub degraded: u64,
    pub failed: u64,
    pub ambiguous: u64,
    pub skipped: u64,
    pub outcome_blocked: u64,
}

impl SyncHealthView {
    const fn of(jobs: i64, counts: ItemCounts) -> Self {
        Self {
            jobs,
            items: counts.total,
            queued: counts.queued,
            leased: counts.leased,
            running: counts.running,
            blocked: counts.blocked,
            parked_live: counts.parked_live,
            parked_cold: counts.parked_cold,
            verifying: counts.verifying,
            settled: counts.settled,
            succeeded: counts.succeeded,
            degraded: counts.degraded,
            failed: counts.failed,
            ambiguous: counts.ambiguous,
            skipped: counts.skipped,
            outcome_blocked: counts.outcome_blocked,
        }
    }
}

pub(crate) async fn sync_health(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<SyncHealthView>, APIError> {
    let health = BackofficeRepo::new(backoffice(&state)?)
        .sync_health()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(SyncHealthView::of(health.jobs, health.items)))
}

// ------------------------------------------------------------ failed writes

#[derive(Debug, Serialize, Deserialize)]
pub struct FailedWriteView {
    pub org: OrgId,
    pub attempt: Uuid,
    pub item: Uuid,
    pub mapping: MappingId,
    pub state: String,
    pub opened_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<Timestamp>,
    /// Absent where the attempt is still in flight past the lease that
    /// opened it, which is a row an operator needs and a failure code cannot
    /// describe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<FailureCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambiguity_cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_failure_code: Option<FailureCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_failure_detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FailedWritesView {
    pub writes: Vec<FailedWriteView>,
}

pub(crate) async fn failed_writes(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<FailedWritesView>, APIError> {
    let rows = BackofficeRepo::new(backoffice(&state)?)
        .failed_writes(FAILED_WRITES_LIMIT)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(FailedWritesView {
        writes: rows
            .into_iter()
            .map(|row| FailedWriteView {
                org: row.org,
                attempt: row.attempt,
                item: row.item.0,
                mapping: row.mapping,
                state: row.state,
                opened_at: row.opened_at,
                settled_at: row.settled_at,
                failure_code: row.failure_code,
                ambiguity_cause: row.ambiguity_cause,
                item_failure_code: row.item_failure_code,
                item_failure_detail: row.item_failure_detail,
            })
            .collect(),
    }))
}

// ------------------------------------------------------------- import drain

/// How many drain measurements one read may carry back. Thirty days of
/// retention across every tenant, which is what the page draws.
const IMPORT_DRAIN_LIMIT: i64 = 500;

/// One import-drain measurement as the operator page reads it.
///
/// `payload` is passed through unparsed rather than widened into named
/// fields. It is `jsonb` in the ledger and nothing constrains its shape at
/// rest, so a row written by an older build is a shape this binary may not
/// know; forwarding it lets the client drop that one row and draw the rest,
/// where parsing here would fail the whole read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDrainRowView {
    pub org: OrgId,
    pub org_name: String,
    pub org_seq: i64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDrainView {
    pub rows: Vec<ImportDrainRowView>,
}

pub(crate) async fn import_drain(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<ImportDrainView>, APIError> {
    let rows = BackofficeRepo::new(backoffice(&state)?)
        .import_drain(IMPORT_DRAIN_LIMIT)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ImportDrainView {
        rows: rows
            .into_iter()
            .map(|row| ImportDrainRowView {
                org: row.org,
                org_name: row.org_name,
                org_seq: row.org_seq,
                payload: row.payload,
            })
            .collect(),
    }))
}

// ---------------------------------------------------------- impersonations

/// One impersonation as the identity service recorded it.
///
/// `actor` and `target` are identity-plane subject ids, which is what
/// `auth.auth_event` stores. They are not `UserId`s and name no row in
/// `app_user` without the `auth_subject` join. Both are always present: the
/// identity schema refuses an impersonation row that names only one party.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImpersonationView {
    pub event: String,
    pub actor: Uuid,
    pub target: Uuid,
    pub at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

impl ImpersonationView {
    fn of(event: tam_storage::ImpersonationEvent) -> Self {
        Self {
            event: event.event,
            actor: event.actor,
            target: event.target,
            at: event.at,
            ip: event.ip_address,
        }
    }
}

/// `impersonations` is absent rather than empty where this database holds no
/// identity schema, for the reason [`SignupsView`] gives.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImpersonationsView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impersonations: Option<Vec<ImpersonationView>>,
}

/// Every impersonation the identity service recorded, newest first.
///
/// This read is the condition on which impersonation ships at all: an admin
/// signing in as a user leaves a record, and the record is readable by someone
/// other than the impersonator. Reading it needs the operator marking, which
/// the identity admin role does not confer -- the two roles are granted
/// separately and by hand, so the party who can impersonate and the party who
/// can read the trail are not the same party by construction.
pub(crate) async fn impersonations(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<ImpersonationsView>, APIError> {
    let _configured = backoffice(&state)?;
    let rows = IdentityAuditRepo::new(state.pool.clone())
        .impersonations(IMPERSONATIONS_LIMIT)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ImpersonationsView {
        impersonations: rows.map(|events| events.into_iter().map(ImpersonationView::of).collect()),
    }))
}
