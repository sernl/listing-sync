//! The operator backoffice: five reads over the whole platform rather than
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
use tam_storage::{BackofficeRepo, DailyCount, ItemCounts, SignupsRepo};
use tam_types::{FailureCode, InventoryId, MappingId, OrgId, Timestamp, Uuid};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::ConnectionView;
use crate::session::OperatorContext;
use crate::AppState;

/// How many failure rows one page carries. A bound on this answer's own size,
/// so it is a constant beside its caller rather than an entry in `tam-limits`.
const FAILED_WRITES_LIMIT: i64 = 100;

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

/// The backoffice pool, or the refusal a deployment without one answers.
///
/// Reached only after [`OperatorContext`] has already accepted the caller, so
/// a non-operator learns nothing from it: they are refused with the blank 401
/// before this runs. No closed error code names this condition, because the
/// code vocabulary is a cross-layer contract the client generates from, and
/// widening it is a change to the client's build rather than to this file.
fn backoffice(state: &AppState) -> Result<PgPool, APIError> {
    state.backoffice.clone().ok_or_else(|| {
        APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("no backoffice database is configured; nothing was read")
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

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgDetailView {
    pub org: OrgSummaryView,
    pub connections: Vec<ConnectionView>,
    pub halts: Vec<HaltView>,
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
                state: row.state,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
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
    pub failure_code: FailureCode,
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
