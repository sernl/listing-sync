//! The operator backoffice: reads over the whole platform rather than one
//! tenant, and one write.
//!
//! Every route here takes [`OperatorContext`], so the marking is checked
//! before a handler runs and no route can forget it. Nothing here creates or
//! modifies an operator: the marking is granted by the `tam-admin` one-shot
//! on the box, so there is no self-elevation endpoint to attack.
//!
//! The cross-tenant queries run on a second pool connected as
//! `tam_backoffice`, whose whole reach is the SELECT grants and read
//! policies of migrations 0037, 0039, 0060, 0067 and 0069. A deployment that
//! passes no `--backoffice-db-url` serves no operator surface at all, rather
//! than half of one.
//!
//! The one write is the plan grant, and it does not go through that pool.
//! `tam_backoffice` holds SELECT and nothing else, deliberately: the reason
//! every other handler on this surface is safe is that the connection it
//! holds cannot write across the tenant fence, and granting it INSERT on one
//! table to shorten one handler would give that property away for all of
//! them. So `grant_plan` and `revoke_plan` open the application pool with
//! the target organisation pinned, exactly as a tenant's own write does —
//! [`OperatorContext`] names no organisation, so the pin is taken from the
//! path and the write is fenced to it. Read-then-write through two pools is
//! the visible cost: the organisation is read through the backoffice pool
//! before the write, so a grant cannot conjure a tenant by naming one.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tam_limits::Plan;
use tam_storage::{
    BackofficeRepo, DailyCount, EntitlementRepo, Grant, GrantRecord, GrantedBy, IdentityAuditRepo,
    ItemCounts, MoveCredit, MoveSource, NewGrant, SignupsRepo,
};
use tam_types::{FailureCode, InventoryId, MappingId, Marketplace, OrgId, Timestamp, UserId, Uuid};

use crate::billing::MoveBalance;
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

/// What the billing provider last said about this organisation's subscription.
///
/// Three fields and no identifier. The provider's subscription and customer ids are
/// what the tenant's own billing page needs to resume a checkout; an operator
/// reading every tenant is answering "is this one paying, and as of when",
/// which these three answer whole.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionStateView {
    /// The provider's own vocabulary, passed through rather than translated, for
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

/// The entitlement an organisation holds right now, as the operator reads
/// it.
///
/// Every field but `plan` is null for an organisation with no grant, which
/// is what most organisations are: nobody granted `free`, so naming a
/// grantor for it would state something false on the panel.
#[derive(Debug, Serialize, Deserialize)]
pub struct GrantView {
    pub plan: Plan,
    pub rung: Option<u32>,
    pub granted_by: Option<String>,
    pub granted_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub source_ref: Option<String>,
}

impl GrantView {
    fn of(grant: Grant) -> Self {
        Self {
            plan: grant.plan,
            rung: grant.rung,
            granted_by: grant.granted_by.map(|by| by.as_str().to_owned()),
            granted_at: grant.granted_at,
            expires_at: grant.expires_at,
            source_ref: grant.source_ref,
        }
    }
}

/// One grant as recorded, revocations included. The audit trail the design
/// asks a manual grant to leave.
#[derive(Debug, Serialize, Deserialize)]
pub struct GrantRecordView {
    pub id: Uuid,
    pub plan: Plan,
    pub rung: Option<u32>,
    pub granted_by: String,
    pub grantor_user: Option<Uuid>,
    pub reason: Option<String>,
    pub source_ref: Option<String>,
    pub granted_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
}

impl GrantRecordView {
    fn of(record: GrantRecord) -> Self {
        Self {
            id: record.id,
            plan: record.plan,
            rung: record.rung,
            granted_by: record.granted_by.as_str().to_owned(),
            grantor_user: record.grantor_user,
            reason: record.reason,
            source_ref: record.source_ref,
            granted_at: record.granted_at,
            expires_at: record.expires_at,
            revoked_at: record.revoked_at,
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
    /// What the organisation holds now, derived rather than stored.
    pub plan: GrantView,
    /// The moves it can spend now, read through the application pool with
    /// the organisation pinned: `move_ledger` grants `tam_backoffice`
    /// nothing, so the operator surface reads a balance the same fenced way
    /// it writes one.
    pub moves: MoveBalance,
    /// Every grant it has ever held, newest first.
    pub grants: Vec<GrantRecordView>,
}

pub(crate) async fn org_detail(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, org)): Path<(String, String)>,
) -> Result<Json<OrgDetailView>, APIError> {
    let org = OrgId(parse_id(&org)?);
    Ok(Json(detail_view(&state, org).await?))
}

/// The org detail, assembled once and answered by three routes: the read,
/// the grant and the revoke. A grant panel that re-rendered from a different
/// shape than the one it was loaded with would be two views of one fact.
async fn detail_view(state: &AppState, org: OrgId) -> Result<OrgDetailView, APIError> {
    let pool = backoffice(state)?;
    let detail = BackofficeRepo::new(pool.clone())
        .org(org, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such organisation"))?;
    // Through the backoffice pool, like every other read on this surface:
    // migration 0069 grants it SELECT and a read policy over the grant table
    // for exactly this panel.
    let entitlements = EntitlementRepo::new(pool);
    let held = entitlements
        .current(org, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let history = entitlements
        .history(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    // Not through the backoffice pool: migration 0084 grants that role
    // nothing on `move_ledger`, so the balance is read through the
    // application pool with this organisation pinned, exactly as the credit
    // below writes it.
    let moves = EntitlementRepo::new(state.pool.clone())
        .move_balance(org, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(OrgDetailView {
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
                // field is omitted rather than reported as `undeclared`,
                // which would state something false about every seller. It is
                // the seller's statement about their own work rather than a
                // fact about the link, so whether an operator sees it is a
                // founder decision rather than an oversight. Nothing stands
                // in the way of taking it: migration 0037 grants this pool
                // table-level SELECT on `connection` and the backoffice query
                // simply does not select the two authorship columns.
                authorship: None,
                country: (row.marketplace == Marketplace::Tes).then_some(row.country),
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
        plan: GrantView::of(held),
        moves: MoveBalance {
            available: moves.available,
            expiring_soonest: moves.expiring_soonest,
        },
        grants: history.into_iter().map(GrantRecordView::of).collect(),
    })
}

/// What an operator grant says.
///
/// `reason` is required and bounded, because an audit row whose reason is
/// blank answers none of the questions an audit row exists for.
#[derive(Debug, Deserialize)]
pub struct GrantPlanBody {
    pub plan: String,
    #[serde(default)]
    pub rung: Option<u32>,
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    pub reason: String,
}

/// How long a reason may be. A bound on this route's own input, so it is a
/// constant beside its caller rather than an entry in `tam-limits`.
const REASON_MAX_CHARS: usize = 500;

/// The operator sets an organisation's plan.
///
/// The write goes through `state.pool` — the application role — with the
/// target organisation pinned, not through the backoffice pool. The
/// backoffice role holds SELECT and nothing else by design, and the reason it
/// holds nothing else is that a connection able to write across tenants is
/// the one thing no other handler on this surface can accidentally acquire.
/// Pinning an organisation from a context that deliberately names none is the
/// price of that, and it is paid here, once, in the open.
pub(crate) async fn grant_plan(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, org)): Path<(String, String)>,
    Json(body): Json<GrantPlanBody>,
) -> Result<Json<OrgDetailView>, APIError> {
    let org = OrgId(parse_id(&org)?);
    // Through the backoffice pool, so a grant cannot create an organisation
    // by naming one that does not exist.
    let _known = detail_view(&state, org).await?;
    let plan = Plan::parse(&body.plan).ok_or_else(|| {
        validation(&format!(
            "{} is not a plan; the set is free, subscriber and studio",
            body.plan
        ))
    })?;
    let reason = body.reason.trim();
    if reason.is_empty() || reason.chars().count() > REASON_MAX_CHARS {
        return Err(validation(
            "a manual grant states its reason, in at most five hundred characters",
        ));
    }
    let now = (state.wall)();
    EntitlementRepo::new(state.pool.clone())
        .grant(
            org,
            &NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan,
                rung: body.rung,
                granted_by: GrantedBy::Operator,
                grantor_user: Some(operator.user.0),
                reason: Some(reason),
                source_ref: None,
                granted_at: now,
                expires_at: body.expires_at,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(detail_view(&state, org).await?))
}

/// What an operator credit of moves says.
///
/// `moves` is signed, because the reason an operator reaches for this is as
/// often a correction as a gift: a seller charged for a move that never
/// landed is given it back, and a balance credited twice is taken down. The
/// reason is required for the same reason a plan grant's is.
#[derive(Debug, Deserialize)]
pub struct CreditMovesBody {
    pub moves: i32,
    pub reason: String,
}

/// The operator moves an organisation's balance.
///
/// Through the application pool with the organisation pinned, exactly as
/// `grant_plan` writes: the backoffice role holds SELECT and nothing else,
/// and a second connection able to write across tenants is the thing no
/// handler on this surface may acquire.
///
/// The entry is idempotent on its reason, so a double-clicked form credits
/// once. That is deliberate rather than incidental: an operator who genuinely
/// means to give the same amount twice types a reason that says so, which is
/// exactly the audit trail this route exists to leave.
pub(crate) async fn credit_moves(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, org)): Path<(String, String)>,
    Json(body): Json<CreditMovesBody>,
) -> Result<Json<OrgDetailView>, APIError> {
    let org = OrgId(parse_id(&org)?);
    let _known = detail_view(&state, org).await?;
    let reason = body.reason.trim();
    if reason.is_empty() || reason.chars().count() > REASON_MAX_CHARS {
        return Err(validation(
            "an operator credit states its reason, in at most five hundred characters",
        ));
    }
    if body.moves == 0 {
        return Err(validation("a credit of no moves changes nothing"));
    }
    let now = (state.wall)();
    let reference = operator_reference(operator.user.0, reason);
    let written = EntitlementRepo::new(state.pool.clone())
        .credit_moves(
            org,
            MoveCredit {
                delta: body.moves,
                source: MoveSource::Operator,
                source_ref: Some(&reference),
                // No expiry. An operator correcting a balance is not selling
                // a pack, and a correction that quietly lapses is a
                // correction that has to be made again.
                expires_at: None,
                at: now,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !written {
        return Err(validation(
            "that operator has already credited this organisation with that reason; \
             say what is different about this one",
        ));
    }
    Ok(Json(detail_view(&state, org).await?))
}

/// What an operator credit is idempotent on: who made it and why. A hash
/// rather than the reason itself, so a five-hundred-character sentence does
/// not become a five-hundred-character index key.
fn operator_reference(user: Uuid, reason: &str) -> String {
    let digest = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, reason.as_bytes());
    let mut out = String::with_capacity(9 + 36 + 1 + 36);
    out.push_str("operator:");
    out.push_str(&uuid::Uuid::from_bytes(user.0).to_string());
    out.push(':');
    out.push_str(&digest.to_string());
    out
}

/// The operator withdraws a grant they or a purchase made.
pub(crate) async fn revoke_plan(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, org, grant)): Path<(String, String, String)>,
) -> Result<Json<OrgDetailView>, APIError> {
    let org = OrgId(parse_id(&org)?);
    let grant = parse_id(&grant)?;
    let _known = detail_view(&state, org).await?;
    let revoked = EntitlementRepo::new(state.pool.clone())
        .revoke(org, grant, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !revoked {
        return Err(missing("no such live grant on that organisation"));
    }
    Ok(Json(detail_view(&state, org).await?))
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

// --------------------------------------------------------------------- users

/// The organisation a user belongs to, as the user listing names it.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserOrgView {
    pub org: OrgId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

/// One user of the platform.
///
/// Three facts about the app plane and one about the identity plane, joined
/// on `auth_subject`: which tenant they are in, what that tenant holds, when
/// they last signed in, and the subject the console's own identity listing
/// keys on.
///
/// `auth_subject` and `last_sign_in_at` are both nullable, and separately.
/// A user provisioned before the identity plane existed carries no subject
/// and so appears here and not in the identity listing; a user who carries
/// one but has not signed in since the audit trail began carries no instant.
/// Neither is an error and neither is a zero: an operator reading this page
/// to find a dormant account needs "never seen" told apart from "seen at the
/// epoch".
///
/// No display name, because `app_user` has none. The name beside the address
/// is the identity plane's, and the console holds that list already.
#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformUserView {
    pub user: UserId,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_subject: Option<Uuid>,
    pub organisation: UserOrgView,
    pub plan: Plan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sign_in_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsersView {
    pub users: Vec<PlatformUserView>,
}

/// Every user of the platform, newest first.
///
/// Two pools, and the split is migration 0037's boundary rather than a
/// convenience: the organisation, the plan and the user row come through the
/// backoffice pool, which is granted exactly those three tables, and the last
/// sign-in comes through the application pool, because `auth.auth_event` is
/// the one object in the identity schema `tam_app` can read and
/// `tam_backoffice` holds not even USAGE there.
///
/// A database with no identity schema answers the listing whole with every
/// `last_sign_in_at` absent, rather than refusing. The two DDL sets are
/// applied by separate commands, so a deployment legitimately holds one and
/// not the other, and a user list that failed for it would take the
/// organisation and plan down with a column that is decoration beside them.
///
/// Active sessions are not here and cannot be: nothing grants any API role a
/// single column of `auth."session"`, which is where the session rows live,
/// and the console reads them from better-auth's own admin endpoints and
/// merges on `auth_subject`. The boundary `db/auth/0002_audit_event.sql`
/// states — the application reads the audit trail and nothing else in that
/// schema — is left standing.
pub(crate) async fn list_users(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<UsersView>, APIError> {
    let now = (state.wall)();
    let users = BackofficeRepo::new(backoffice(&state)?)
        .users(now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let seen: HashMap<Uuid, Timestamp> = IdentityAuditRepo::new(state.pool.clone())
        .last_sign_in()
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .unwrap_or_default()
        .into_iter()
        .collect();
    Ok(Json(UsersView {
        users: users
            .into_iter()
            .map(|user| PlatformUserView {
                last_sign_in_at: user
                    .auth_subject
                    .and_then(|subject| seen.get(&subject).copied()),
                user: user.user,
                email: user.email,
                auth_subject: user.auth_subject,
                organisation: UserOrgView {
                    org: user.org,
                    name: user.org_name,
                    slug: user.org_slug,
                },
                plan: user.plan,
                created_at: user.created_at,
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

/// Every measurement the read carried, and whether it carried them all.
///
/// `truncated` is reported rather than left to be inferred from `rows.len()`
/// against the limit. The ordering is by organisation name, so a read that
/// fills the limit drops whole tenants off the end of the alphabet and the
/// rows that survive look exactly like the complete platform; a page that
/// cannot say so claims a completeness it does not have, at precisely the
/// scale an operator would act on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDrainView {
    pub rows: Vec<ImportDrainRowView>,
    pub truncated: bool,
}

pub(crate) async fn import_drain(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<ImportDrainView>, APIError> {
    let page = BackofficeRepo::new(backoffice(&state)?)
        .import_drain(IMPORT_DRAIN_LIMIT)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ImportDrainView {
        rows: page
            .runs
            .into_iter()
            .map(|row| ImportDrainRowView {
                org: row.org,
                org_name: row.org_name,
                org_seq: row.org_seq,
                payload: row.payload,
            })
            .collect(),
        truncated: page.truncated,
    }))
}

// ------------------------------------------------------------- dead letters

/// Dead letters on one outbox topic, as the operator page reads them.
///
/// Two figures and no rows, because two figures answer the operator's
/// question: one organisation holding many is an address the relay refuses,
/// and many organisations holding one each is the relay or the key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterTopicView {
    pub topic: String,
    pub messages: i64,
    pub orgs: i64,
}

/// Every topic carrying a dead letter, alphabetically; empty where none does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLettersView {
    pub topics: Vec<DeadLetterTopicView>,
}

/// The outbox rows the drainer gave up on, counted rather than listed.
///
/// A message exhausts its attempt budget or is refused outright, is written
/// `state = 'dead'`, and is never retried. Until this read existed nothing
/// looked at those rows, so a completion mail that died was a line in the
/// drainer's log and nothing else.
pub(crate) async fn dead_letters(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<DeadLettersView>, APIError> {
    let rows = BackofficeRepo::new(backoffice(&state)?)
        .dead_letters()
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(DeadLettersView {
        topics: rows
            .into_iter()
            .map(|row| DeadLetterTopicView {
                topic: row.topic,
                messages: row.messages,
                orgs: row.orgs,
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
