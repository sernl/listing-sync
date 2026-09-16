//! Jobs as operation resources. A sync-starting request carries a mandatory
//! `Idempotency-Key`, so the retry and the double-click are the same job;
//! job status is a computed roll-up over item states that never gates on the
//! first bad item; listings page by opaque keyset cursor the server mints,
//! so a client cannot construct page state it was not issued.

use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{request::Parts, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::{ItemOperation, ItemOutcome, JobItemId};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::ListingState;
use tam_storage::{
    intent_digest, DeletionStatus, Disposition, EntitlementRepo, EventRow, ItemCounts, ItemRow,
    ItemsPageParams, JobOrigin, JobReadRepo, JobRepo, LedgerCursor, MappingSeed, NewJob,
    NewJobItem, NewSyncRequest, StorageError, SyncIntent, SyncRequestPage, SyncRequestRepo,
};
use tam_types::{Actor, FailureCode, InventoryId, JobId, MappingId, OrgId, Stamp, Timestamp, Uuid};

use crate::entitlement::migration_refusal;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// Every item minted by the API carries this intent version until the
/// duplication flow (M1j) starts versioning intents.
const INTENT_VERSION: u32 = 1;

const PAGE_LIMIT_DEFAULT: i64 = 50;
const PAGE_LIMIT_MAX: i64 = 200;

// ------------------------------------------------------------------ cursors

/// Encodes a keyset cursor as the opaque token the client carries back.
/// The format is a server detail; the contract is "opaque".
#[must_use]
pub fn encode_cursor(cursor: &LedgerCursor) -> String {
    use core::fmt::Write;
    let mut token = format!("{:x}.", cursor.created_at.0);
    for byte in cursor.id.0 {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(token, "{byte:02x}");
    }
    token
}

/// Parses a token this server minted; anything else is `None` and the
/// request is refused as validation, never guessed at.
#[must_use]
pub fn decode_cursor(raw: &str) -> Option<LedgerCursor> {
    let (millis_hex, id_hex) = raw.split_once('.')?;
    let millis = i64::from_str_radix(millis_hex, 16).ok()?;
    if id_hex.len() != 32 {
        return None;
    }
    let mut id = [0u8; 16];
    for (index, slot) in id.iter_mut().enumerate() {
        let pair = id_hex.get(index * 2..index * 2 + 2)?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(LedgerCursor {
        created_at: Timestamp(millis),
        id: Uuid(id),
    })
}

#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// Which outcome to show. A five-hundred-item bulk with four failures is
    /// the case this exists for: paging the other four hundred and ninety-six
    /// to find them is not a serious alternative.
    pub outcome: Option<String>,
}

struct Page {
    cursor: Option<LedgerCursor>,
    limit: i64,
    outcome: Option<String>,
}

fn parse_page(params: &PageParams) -> Result<Page, APIError> {
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(
            decode_cursor(raw)
                .ok_or_else(|| validation("the cursor is not one this server issued"))?,
        ),
    };
    let limit = params
        .limit
        .unwrap_or(PAGE_LIMIT_DEFAULT)
        .clamp(1, PAGE_LIMIT_MAX);
    let outcome = match params.outcome.as_deref() {
        None => None,
        Some(raw) => Some(
            ALL_OUTCOMES
                .into_iter()
                .map(outcome_str)
                .find(|name| *name == raw)
                .map(str::to_owned)
                .ok_or_else(|| validation("that is not an outcome an item can settle on"))?,
        ),
    };
    Ok(Page {
        cursor,
        limit,
        outcome,
    })
}

pub(crate) fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

pub(crate) fn missing(what: &str) -> APIError {
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
        .map_err(|_| validation("the identifier is not a UUID"))
}

// --------------------------------------------------------- idempotency key

/// The mandatory header on every sync-starting request, extracted before the
/// handler body so its absence is a structured 422 rather than a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestKey(pub Uuid);

impl<S: Send + Sync> FromRequestParts<S> for RequestKey {
    type Rejection = APIError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, APIError> {
        let refused = || {
            APIError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                APIErrorEntry::new(
                    "an Idempotency-Key header (a UUID) is required to start a sync",
                )
                .code(APIErrorCode::IdempotencyKeyRequired)
                .kind(APIErrorKind::Validation),
            )
        };
        let raw = parts
            .headers
            .get("idempotency-key")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(refused)?;
        let parsed = uuid::Uuid::parse_str(raw.trim()).map_err(|_| refused())?;
        Ok(Self(Uuid(*parsed.as_bytes())))
    }
}

// -------------------------------------------------------------------- DTOs

#[derive(Debug, Deserialize)]
pub struct CreateJobBody {
    pub inventory: InventoryId,
    pub mappings: Vec<MappingId>,
    /// Where the seller wants the listing to end up. Absent keeps the
    /// endpoint's original meaning, which is a create.
    ///
    /// Not `mapping.publish_mode`, which is a standing sync policy about
    /// whether to publish at all and is explicitly not a record of where a
    /// listing landed. The stated intent goes into the item's operation and
    /// nowhere else.
    #[serde(default)]
    pub intent: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatedJobBody {
    pub job: JobId,
    pub replay: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobPage {
    pub jobs: Vec<JobHead>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobHead {
    pub job: JobId,
    pub inventory: InventoryId,
    pub created_at: Timestamp,
    /// Where a Delete this job is still working through has got to, and
    /// nothing for a job nobody deleted.
    ///
    /// A listed job is never `deleted`: the tombstone is filtered in SQL, so
    /// what this carries is the two states a seller has to be able to see —
    /// work that is still stopping, and a write nobody can account for.
    pub deletion_status: Option<DeletionStatusView>,
}

/// The roll-up as the client reads it: the phase is derived, the counts are
/// the ledger's own, and no scalar verdict exists to gate on a bad item.
#[derive(Debug, Serialize, Deserialize)]
pub struct JobView {
    pub job: JobId,
    pub inventory: InventoryId,
    pub created_at: Timestamp,
    pub phase: JobPhase,
    pub counts: CountsView,
    /// See [`JobHead::deletion_status`].
    pub deletion_status: Option<DeletionStatusView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Active,
    Settled,
}

/// How far a Delete has got, as the wire carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionStatusView {
    /// Fenced, and something is still executing. The row stays visible.
    Stopping,
    /// Fenced, and a write already issued cannot be accounted for. Visible
    /// until evidence closes it; never closed by a clock.
    NeedsReview,
    /// Quiescent and removed from the seller's history.
    Deleted,
}

/// The closed set, in a stable order, for the vocabulary generator.
pub const ALL_DELETION_STATUSES: [DeletionStatusView; 3] = [
    DeletionStatusView::Stopping,
    DeletionStatusView::NeedsReview,
    DeletionStatusView::Deleted,
];

#[must_use]
pub const fn deletion_status_str(status: DeletionStatusView) -> &'static str {
    match status {
        DeletionStatusView::Stopping => "stopping",
        DeletionStatusView::NeedsReview => "needs_review",
        DeletionStatusView::Deleted => "deleted",
    }
}

impl DeletionStatusView {
    #[must_use]
    pub const fn of(status: DeletionStatus) -> Self {
        match status {
            DeletionStatus::Stopping => Self::Stopping,
            DeletionStatus::NeedsReview => Self::NeedsReview,
            DeletionStatus::Deleted => Self::Deleted,
        }
    }

    /// 200 where the work is gone, 202 where the seller's Delete is accepted
    /// and not finished. Two codes rather than one because the difference is
    /// the whole contract: a console that rendered "deleted" over a 202 would
    /// be telling a seller a marketplace write had been called off when
    /// nobody knows that yet.
    #[must_use]
    pub const fn status_code(self) -> StatusCode {
        match self {
            Self::Deleted => StatusCode::OK,
            Self::Stopping | Self::NeedsReview => StatusCode::ACCEPTED,
        }
    }
}

/// What a Delete answers, for all three kinds of work.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct JobDeletionView {
    pub status: DeletionStatusView,
}

impl JobDeletionView {
    pub(crate) fn answer(status: DeletionStatus) -> Response {
        let status = DeletionStatusView::of(status);
        (status.status_code(), Json(Self { status })).into_response()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CountsView {
    pub total: u64,
    pub queued: u64,
    pub in_flight: u64,
    pub blocked: u64,
    pub parked: u64,
    pub settled: u64,
    pub succeeded: u64,
    pub degraded: u64,
    pub failed: u64,
    pub ambiguous: u64,
    pub skipped: u64,
    pub outcome_blocked: u64,
}

impl CountsView {
    fn from_counts(counts: ItemCounts) -> Self {
        Self {
            total: counts.total,
            queued: counts.queued,
            in_flight: counts.leased + counts.running + counts.verifying,
            blocked: counts.blocked,
            parked: counts.parked_live + counts.parked_cold,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct ItemsPage {
    pub items: Vec<ItemView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ItemView {
    pub item: Uuid,
    pub mapping: MappingId,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<FailureCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_on: Option<String>,
    pub attempt_count: i32,
    pub created_at: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<Timestamp>,
}

/// The closed outcome set, in a stable order, for the vocabulary generator.
pub const ALL_OUTCOMES: [ItemOutcome; 6] = [
    ItemOutcome::Succeeded,
    ItemOutcome::Degraded,
    ItemOutcome::Failed,
    ItemOutcome::Ambiguous,
    ItemOutcome::Skipped,
    ItemOutcome::Blocked,
];

#[must_use]
pub const fn outcome_str(outcome: ItemOutcome) -> &'static str {
    match outcome {
        ItemOutcome::Succeeded => "succeeded",
        ItemOutcome::Degraded => "degraded",
        ItemOutcome::Failed => "failed",
        ItemOutcome::Ambiguous => "ambiguous",
        ItemOutcome::Skipped => "skipped",
        ItemOutcome::Blocked => "blocked",
    }
}

impl ItemView {
    fn from_row(row: ItemRow) -> Self {
        Self {
            item: row.item.0,
            mapping: row.mapping,
            state: row.state.as_str().to_owned(),
            outcome: row.outcome.map(|outcome| outcome_str(outcome).to_owned()),
            failure_code: row.failure_code,
            failure_detail: row.failure_detail,
            blocked_on: row.blocked_on,
            attempt_count: row.attempt_count,
            created_at: row.created_at,
            settled_at: row.settled_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ItemDetail {
    #[serde(flatten)]
    pub item: ItemView,
    /// One page of the item's steps, oldest first.
    pub events: Vec<EventView>,
    /// The `org_seq` to ask after for the next page of steps, or nothing
    /// where this page is the last.
    pub events_next: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventView {
    pub org_seq: i64,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: Timestamp,
}

impl EventView {
    fn from_row(row: EventRow) -> Self {
        Self {
            org_seq: row.org_seq,
            kind: row.kind,
            payload: row.payload,
            created_at: row.created_at,
        }
    }
}

// ---------------------------------------------------------------- handlers

pub(crate) fn storage_fault(state: &AppState, error: &StorageError) -> APIError {
    state.internal(&error.to_string())
}

/// The one enqueue for sync, migrate and bulk. Bulk is not a separate verb;
/// it is this with more than one resource, which is why there is no third
/// endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequestBody {
    pub source: InventoryId,
    pub target: InventoryId,
    /// `sync` leaves the source listing alone; `migrate` removes it once the
    /// target is bound.
    #[serde(default)]
    pub disposition: Option<String>,
    #[serde(default)]
    pub intent: Option<String>,
    /// How the seller addresses each listing on the source, in their order.
    pub resources: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequestAck {
    pub request: Uuid,
}

/// Writes the request and returns. It performs no marketplace read: this
/// process holds no session, and the broker lease belongs to the drain.
pub(crate) async fn create_sync_request(
    State(state): State<AppState>,
    context: OrgContext,
    key: RequestKey,
    Json(body): Json<SyncRequestBody>,
) -> Result<Response, APIError> {
    if body.source == body.target {
        return Err(validation("a sync's source and target are two inventories"));
    }
    // The same registry `lower` refuses an uncaptured transition through, and
    // the same one the drain refuses on. Refusing here means the seller is
    // told at submit rather than by a request that sits `pending` forever
    // while the drain re-leases a gateway for it every poll.
    if let Some(capability) = tam_storage::uncaptured_source(body.source) {
        return Err(validation(&format!(
            "{:?} has no captured {capability}, so it cannot be a sync's source yet",
            body.source
        )));
    }
    let disposition = match body.disposition.as_deref() {
        None | Some("sync") => Disposition::Sync,
        Some("migrate") => Disposition::Migrate,
        Some(_) => return Err(validation("disposition is \"sync\" or \"migrate\"")),
    };
    let intent = match parse_intent(body.intent.as_deref())? {
        Intent::Draft => SyncIntent::Draft,
        Intent::Live => SyncIntent::Live,
    };
    // Who supplies the resources, decided by the source marketplace's transport
    // class rather than by a request kind of its own. Under D1 a device-branch
    // marketplace is enumerated only by the seller's own device, so a migrate
    // from one starts empty and its pages fill it; every other request names
    // its resources here, and an empty one would settle complete having moved
    // nothing.
    let device_enumerated = body.source.marketplace().transport_class()
        == tam_types::TransportClass::SellerDevice
        && disposition == Disposition::Migrate;
    if device_enumerated && !body.resources.is_empty() {
        return Err(validation(
            "a migrate from a marketplace with no official API names no resources here: the \
             seller's own device enumerates that catalogue and posts it a page at a time, so a \
             list named now would be silently ignored",
        ));
    }
    if !device_enumerated && body.resources.is_empty() {
        return Err(validation("a sync names at least one resource"));
    }
    // The month's migration allowance, counted in resources rather than in
    // requests: a per-request cap is gamed by batching, and the figure the
    // pricing page names is resources. A device-enumerated migrate names
    // none here, so it is admitted while the counter has any room at all and
    // its pages are counted as they land.
    let caps = context.entitlement.caps;
    let now = (state.wall)();
    let entitlements = EntitlementRepo::new(state.pool.clone());
    let used = entitlements
        .migrations_used_this_month(context.org, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let requested = i64::try_from(body.resources.len()).unwrap_or(i64::MAX);
    if used.saturating_add(requested.max(1)) > i64::from(caps.migrations_per_month) {
        let resets_at = entitlements
            .usage(context.org, now)
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .migrations_reset_at;
        return Err(migration_refusal(
            used,
            caps.migrations_per_month,
            requested,
            resets_at,
        ));
    }
    let new = NewSyncRequest {
        // The idempotency key is the request's identity, so a retried submit
        // is the same request rather than a second one.
        id: key.0,
        source: body.source,
        target: body.target,
        disposition,
        intent,
        requested_at: now,
        locators: body.resources,
    };
    // The insert decides the replay, rather than a read the two submits then
    // race: both saw no existing row, one INSERT won and the other violated
    // the primary key, so the double-click this endpoint exists to absorb
    // came back a fault. The request's identity is the key, so the ack is the
    // same either way.
    let requests = SyncRequestRepo::new(state.pool.clone());
    let written = requests
        .create(context.org, &new)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !written {
        // The key is spent. Ordinarily that is the double-click this
        // endpoint absorbs, and the answer is the request it already made —
        // but a request the seller deleted is not a request to be handed
        // back, and re-enqueueing under the same key would resurrect exactly
        // the work they stopped. So the replay of a deleted request is a
        // conflict, which is a thing a client can act on: start a new one.
        if let Some(deleted) = requests
            .deletion_status(context.org, key.0)
            .await
            .map_err(|error| storage_fault(&state, &error))?
        {
            return Err(deleted_key_conflict(deleted));
        }
    }
    let status = if written {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(SyncRequestAck { request: key.0 })).into_response())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequestView {
    pub request: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: String,
    pub intent: String,
    pub state: String,
    pub failure_detail: Option<String>,
    /// The jobs this request produced, once it has them. A migrate names
    /// both, because a job carries one inventory and the create and the
    /// removal are on two.
    pub create_job: Option<Uuid>,
    pub remove_job: Option<Uuid>,
    /// The founder's kill-gate number for this request, summed over its
    /// described resources.
    ///
    /// `None` rather than a struct of zeros on a request that measures no
    /// coverage, which is every request whose source is not enumerated by a
    /// device. The columns default to zero and are written only when a page
    /// applies a resource, so all-zeros would render "never measured" and
    /// "measured, and the answer was zero" as the same object — and this is
    /// the one number where confusing those two is expensive.
    pub coverage: Option<CoverageView>,
    /// Why nothing is running, when the reason is that no device can run it.
    ///
    /// Derived on read from the registered devices rather than stored, because
    /// it stops being true the moment a seller updates one. `None` means
    /// nothing is waiting on a version.
    pub waiting_for_device_version: Option<String>,
    /// See [`JobHead::deletion_status`].
    pub deletion_status: Option<DeletionStatusView>,
    /// How many of the request's resources stand in each state, over the
    /// whole request.
    ///
    /// Here rather than counted from `resources`, because `resources` is one
    /// page. Every sentence this page leads with — what the request is doing,
    /// how many listings arrived, how many the device skipped — is about the
    /// request and not about the rows on screen, and a tally taken from page
    /// two of a finished migration would say nothing had been imported.
    pub resource_counts: Vec<ResourceStateCountView>,
    /// One page of the request's resources, by ordinal.
    pub resources: Vec<SyncResourceView>,
    /// The ordinal to ask after for the next page of resources, or nothing
    /// where this page is the last. A plain ordinal rather than an opaque
    /// token: the order is the request's own ordinal, which the seller's
    /// submitted list already fixed, and there is nothing for a token to hide.
    pub resources_next: Option<i32>,
}

/// The coverage of one request, summed over the resources that carry it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CoverageView {
    /// How many resources the sum is over: the described ones, excluding
    /// anything the device skipped, because a skip measured nothing.
    pub rows: u32,
    pub terms_seen: u32,
    pub terms_mapped: u32,
    pub terms_unmapped: u32,
    pub terms_uncovered: u32,
}

/// How many of a request's resources stand in one state.
///
/// The state is the stored word rather than a closed enum, for the same
/// reason `SyncResourceView::state` is: the console already renders a state
/// it does not know by saying so, and a view that refused to carry one would
/// blank the page instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStateCountView {
    pub state: String,
    pub count: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncResourceView {
    pub ordinal: i32,
    pub locator: String,
    pub state: String,
    pub failure_detail: Option<String>,
    /// What the import measured about this resource, so the console and the
    /// review step can say which product carried the uncovered terms rather
    /// than only that some did.
    ///
    /// Absent where nothing measured it: a resource the device skipped, and
    /// every breadcrumb of a request that measures no coverage at all.
    pub coverage: Option<ResourceCoverageView>,
}

/// One resource's coverage, as the wire carries it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ResourceCoverageView {
    pub terms_seen: u32,
    pub terms_mapped: u32,
    pub terms_unmapped: u32,
    pub terms_uncovered: u32,
}

/// One page of the organisation's sync requests, newest first.
///
/// The console's only way back to a request it created and navigated away
/// from. A device-branch migrate mints no job until its completing page, so
/// before this it appeared in no list at all and the console's own copy told
/// the seller to return to a page nothing linked to.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequestListView {
    pub requests: Vec<SyncRequestSummaryView>,
    /// The token for the next page, or nothing when this page is the end.
    ///
    /// Opaque, and the same codec the ledger's own lists use, because it is
    /// the same kind of cursor: the keyset `(requested_at, id)` this list is
    /// ordered by.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequestSummaryView {
    pub request: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: String,
    pub intent: String,
    pub state: String,
    pub created_at: i64,
    pub resources_total: u32,
    pub resources_failed: u32,
    /// See [`JobHead::deletion_status`].
    pub deletion_status: Option<DeletionStatusView>,
}

/// How many requests one page carries at most, and by default.
///
/// It was a ceiling with no cursor, on the reasoning that a seller has a
/// handful of migrations rather than a feed. A seller who migrates a shop a
/// week reaches it inside a year, and past it the fifty-first migration was
/// not merely unpaged but unreachable — so the answer the comment named is
/// now the answer: a cursor. The figure stays as the ceiling on one page,
/// and as the default so that a caller which asks for no page size keeps the
/// answer it has always had.
const SYNC_LIST_MAX: i64 = 50;

/// What narrows the request list, and where the last page ended.
#[derive(Debug, Default, Deserialize)]
pub struct SyncListParams {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// `sync` or `migrate`. A sync and a migration are one record under two
    /// dispositions and the two screens that read this list each own one, so
    /// the narrowing is the server's `WHERE` clause rather than a filter the
    /// console applies to a page it was handed — which would answer "the
    /// migrations among the newest ten requests" and show a seller whose last
    /// ten requests were syncs no migrations at all.
    pub disposition: Option<String>,
    /// One request state, as the wire spells it elsewhere on this surface.
    pub state: Option<String>,
}

/// The dispositions a request can carry, for refusing anything else.
///
/// Refused rather than ignored, exactly as `parse_page` refuses an unknown
/// outcome: a filter that is silently dropped answers a well-formed page of
/// everything to a caller that believes it asked for one half.
fn parse_disposition(raw: Option<&str>) -> Result<Option<Disposition>, APIError> {
    match raw {
        None | Some("") => Ok(None),
        Some("sync") => Ok(Some(Disposition::Sync)),
        Some("migrate") => Ok(Some(Disposition::Migrate)),
        Some(_other) => Err(validation(
            "a request is either a sync or a migrate, and that is neither",
        )),
    }
}

pub(crate) async fn list_sync_requests(
    State(state): State<AppState>,
    context: OrgContext,
    Path(_version): Path<String>,
    Query(params): Query<SyncListParams>,
) -> Result<Json<SyncRequestListView>, APIError> {
    let after = match params.cursor.as_deref() {
        None | Some("") => None,
        Some(raw) => {
            let cursor = decode_cursor(raw)
                .ok_or_else(|| validation("the cursor is not one this server issued"))?;
            Some((cursor.created_at, cursor.id))
        }
    };
    let limit = params
        .limit
        .unwrap_or(SYNC_LIST_MAX)
        .clamp(1, SYNC_LIST_MAX);
    let page = SyncRequestPage {
        after,
        limit,
        disposition: parse_disposition(params.disposition.as_deref())?,
        state: params.state.filter(|state| !state.is_empty()),
    };
    let rows = SyncRequestRepo::new(state.pool.clone())
        .list(context.org, &page)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // A full page may have more behind it and a short one cannot, which is
    // the rule every other list on this surface pages by.
    let next_cursor = (i64::try_from(rows.len()).unwrap_or(i64::MAX) == limit)
        .then(|| {
            rows.last().map(|last| {
                encode_cursor(&LedgerCursor {
                    created_at: last.requested_at,
                    id: last.id,
                })
            })
        })
        .flatten();
    Ok(Json(SyncRequestListView {
        requests: rows
            .into_iter()
            .map(|row| SyncRequestSummaryView {
                request: row.id,
                source: row.source,
                target: row.target,
                disposition: row.disposition.as_str().to_owned(),
                intent: row.intent.as_str().to_owned(),
                state: row.state,
                created_at: row.requested_at.0,
                resources_total: row.resources_total,
                resources_failed: row.resources_failed,
                deletion_status: row.deletion.map(DeletionStatusView::of),
            })
            .collect(),
        next_cursor,
    }))
}

/// How many resources one page of a request's listing list carries.
///
/// A migration of a whole shop names every listing in it, and this page used
/// to answer all of them: five hundred rows on the wire and five hundred in
/// the document, for a screen that shows twenty-five. The figures beside the
/// list are the whole request's and are counted in SQL, so bounding the rows
/// costs the seller no fact.
const RESOURCE_PAGE_DEFAULT: i64 = 25;
const RESOURCE_PAGE_MAX: i64 = 100;

/// Where a page of the listing list starts, and how long it is.
#[derive(Debug, Default, Deserialize)]
pub struct ResourcePageParams {
    /// The ordinal of the last row already held. The request's ordinals are
    /// the order the seller submitted, so they are a total order already and
    /// there is nothing an opaque token would add.
    pub after: Option<i32>,
    pub limit: Option<i64>,
}

/// What the client polls between asking for a sync and the ledger having
/// something to show them.
pub(crate) async fn sync_request_view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, request)): Path<(String, String)>,
    Query(params): Query<ResourcePageParams>,
) -> Result<Json<SyncRequestView>, APIError> {
    let request = parse_id(&request)?;
    let limit = params
        .limit
        .unwrap_or(RESOURCE_PAGE_DEFAULT)
        .clamp(1, RESOURCE_PAGE_MAX);
    let detail = SyncRequestRepo::new(state.pool.clone())
        .detail(context.org, request, params.after, limit)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such sync request"))?;
    let head = detail.head;
    // Coverage is measured only where a device enumerated the catalogue, and
    // the source's transport class is what says so — the same predicate the
    // submit validated against, read here rather than restated.
    let measured = head.source.marketplace().transport_class()
        == tam_types::TransportClass::SellerDevice
        && head.disposition == Disposition::Migrate;
    // The sum is the storage layer's, over the breadcrumbs that carry a
    // measurement and only those: a skipped resource never reached the
    // taxonomy, so counting it as a row of zeros would enter it into the
    // founder's average as perfect coverage. It is a sum over the whole
    // request rather than over the page, which is the same reason the state
    // counts are. Storage answers `None` for "no row measured"; on a request
    // that does measure, that is a measurement whose answer is zero rows, so
    // it is rendered as zeros here — the distinction `None` carries on this
    // view is "never measured", which is the other branch.
    let coverage = measured.then(|| {
        let total = detail.coverage.unwrap_or_default();
        CoverageView {
            rows: total.rows,
            terms_seen: total.terms_seen,
            terms_mapped: total.terms_mapped,
            terms_unmapped: total.terms_unmapped,
            terms_uncovered: total.terms_uncovered,
        }
    });
    let waiting_for_device_version = if measured {
        waiting_for_a_device(&state, context.org).await?
    } else {
        None
    };
    Ok(Json(SyncRequestView {
        request: head.id,
        source: head.source,
        target: head.target,
        disposition: head.disposition.as_str().to_owned(),
        intent: head.intent.as_str().to_owned(),
        state: head.state,
        failure_detail: head.failure_detail,
        create_job: head.create_job,
        remove_job: head.remove_job,
        coverage,
        waiting_for_device_version,
        deletion_status: head.deletion.map(DeletionStatusView::of),
        resource_counts: detail
            .counts
            .into_iter()
            .map(|row| ResourceStateCountView {
                state: row.state,
                count: row.count,
            })
            .collect(),
        resources: detail
            .resources
            .into_iter()
            .map(|row| SyncResourceView {
                ordinal: row.ordinal,
                locator: row.locator,
                state: row.state,
                failure_detail: row.failure_detail,
                coverage: row.coverage.map(|measured| ResourceCoverageView {
                    terms_seen: measured.terms_seen,
                    terms_mapped: measured.terms_mapped,
                    terms_unmapped: measured.terms_unmapped,
                    terms_uncovered: measured.terms_uncovered,
                }),
            })
            .collect(),
        resources_next: detail.next_ordinal,
    }))
}

/// Deletes one sync or migration request, both legs together.
///
/// `200` with `deleted` where the work is gone, `202` with `stopping` or
/// `needs_review` where the Delete is accepted and not finished, and the same
/// `404` another organisation's request gets. Repeating it is the same answer,
/// recomputed: a Delete that said `stopping` says `deleted` once the legs are
/// quiet, which is how a console that asks again learns that it can stop
/// showing the row.
///
/// Nothing here touches a marketplace, a product, a mapping or a file. It
/// stops work and hides the request; the catalogue keeps everything the
/// migration had already canonicalised, because those are the seller's own
/// resources and not artefacts of the request.
pub(crate) async fn delete_sync_request(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, request)): Path<(String, String)>,
) -> Result<Response, APIError> {
    let request = parse_id(&request)?;
    let status = SyncRequestRepo::new(state.pool.clone())
        .delete(context.org, request, context.stamp((state.wall)()))
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such sync request"))?;
    Ok(JobDeletionView::answer(status))
}

/// The version a device needs before it can run a marketplace-sourced item,
/// stated only while the tenant has no device that can.
///
/// D6's surfacing half. The gate itself is the claim's, which hands a sourced
/// item only to a device at or past this version; what this answers is the
/// question that gate leaves a seller with — an item that waits in silence
/// costs a support ticket, and an item that says which version it needs costs
/// an update. Derived rather than stored, because it stops being true the
/// moment the seller updates a machine.
async fn waiting_for_a_device(state: &AppState, org: OrgId) -> Result<Option<String>, APIError> {
    let (major, minor, patch) = tam_domain::SOURCED_PAYLOAD_MIN_VERSION;
    let devices = tam_storage::DeviceRepo::new(state.pool.clone())
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let ready = devices.iter().any(|device| {
        device.revoked_at.is_none() && tam_domain::runs_sourced_payloads(&device.app_version)
    });
    Ok((!ready).then(|| format!("{major}.{minor}.{patch}")))
}

/// What the seller asked the listing to end up as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Draft,
    Live,
}

fn parse_intent(raw: Option<&str>) -> Result<Intent, APIError> {
    match raw {
        None | Some("draft") => Ok(Intent::Draft),
        Some("live") => Ok(Intent::Live),
        Some(_) => Err(validation("intent is \"draft\" or \"live\"")),
    }
}

/// The lowering, as `tam-storage` states it once for both enqueue paths, with
/// its refusals rendered as the validation answers they are.
///
/// The intent reaches it as the state the seller asked for rather than as a
/// second intent enum: `Draft` and `Live` are exactly `ListingState`, and one
/// vocabulary is one fewer thing to keep in step.
fn lower(
    intent: Intent,
    inventory: InventoryId,
    seed: &MappingSeed,
) -> Result<Vec<ItemOperation>, APIError> {
    let to = match intent {
        Intent::Draft => ListingState::Draft,
        Intent::Live => ListingState::Live,
    };
    tam_storage::lower(to, inventory, seed).map_err(|refusal| validation(&refusal.to_string()))
}

pub(crate) async fn create_job(
    State(state): State<AppState>,
    context: OrgContext,
    key: RequestKey,
    Json(body): Json<CreateJobBody>,
) -> Result<Response, APIError> {
    if body.mappings.is_empty() {
        return Err(validation("a sync needs at least one mapping"));
    }
    let reads = JobReadRepo::new(state.pool.clone());
    let seeds = reads
        .mapping_seeds(context.org, body.inventory, &body.mappings)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if seeds.len() != body.mappings.len() {
        let found: Vec<MappingId> = seeds.iter().map(|seed| seed.mapping).collect();
        let unknown: Vec<String> = body
            .mappings
            .iter()
            .filter(|mapping| !found.contains(mapping))
            .map(|mapping| uuid::Uuid::from_bytes(mapping.0 .0).to_string())
            .collect();
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("some mappings are unknown or not of the requested inventory")
                .code(APIErrorCode::SyncMappingsInvalid)
                .kind(APIErrorKind::Validation)
                .detail(serde_json::json!({ "mappings": unknown })),
        ));
    }

    let now = (state.wall)();
    let job = JobId(fresh_uuid());
    let intent = parse_intent(body.intent.as_deref())?;
    // The seller states draft-or-live per platform and the API lowers it
    // against the mapping's own binding, reading no marketplace to do it.
    let mut items: Vec<NewJobItem> = Vec::new();
    for seed in &seeds {
        items.extend(new_items(
            context.org,
            body.inventory,
            job,
            seed,
            lower(intent, body.inventory, seed)?,
        ));
    }
    let created = mint_job(
        &state,
        context.org,
        job,
        body.inventory,
        // The one write path with a seller genuinely behind it: the session
        // extractor already resolved who, and the repository boundary
        // discarded it until now.
        Actor::Person(context.user),
        now,
        key.0,
        &items,
        // A seller pressing Publish names no import; nothing derived this.
        None,
    )
    .await?;
    if created.replay {
        // The same rule the sync submit follows, at the other enqueue: a key
        // whose job the seller deleted is answered with a conflict rather
        // than with the tombstone, and nothing is enqueued. The ledger keeps
        // the key, which is what makes this answer possible at all.
        if let Some(deleted) = JobRepo::new(state.pool.clone())
            .deletion_status(context.org, created.job)
            .await
            .map_err(|error| storage_fault(&state, &error))?
        {
            return Err(deleted_key_conflict(deleted));
        }
    }
    let status = if created.replay {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(CreatedJobBody {
            job: created.job,
            replay: created.replay,
        }),
    )
        .into_response())
}

/// The items one mapping's lowered operations become.
///
/// Extracted from `create_job` rather than restated, because the scheduler
/// mints jobs from the same lowering and a second copy of the idempotency
/// derivation is a second answer to "is this the same write": the key mixes
/// the organisation, the inventory, the product, the intent version and the
/// operation's own digest, and a caller that assembled four of those five
/// would mint items that never dedupe against the seller's own.
pub(crate) fn new_items(
    org: OrgId,
    inventory: InventoryId,
    job: JobId,
    seed: &MappingSeed,
    operations: Vec<ItemOperation>,
) -> Vec<NewJobItem> {
    operations
        .into_iter()
        .map(|operation| NewJobItem {
            item: JobItemId(fresh_uuid()),
            mapping: seed.mapping,
            idempotency_key: derive_idempotency_key(
                org,
                inventory,
                seed.product,
                INTENT_VERSION,
                intent_digest(&operation, job, &seed.payload_hashes, seed.sever_generation),
            ),
            requires_bound_on: tam_storage::requires_bound_on(&operation, inventory),
            operation,
        })
        .collect()
}

/// One job under a request key, with the duplicate-item conflict rendered as
/// the validation answer it is.
///
/// The actor is the caller's, because the two callers differ in exactly that:
/// a seller pressed Publish, or the scheduler's pass reached a minute nobody
/// was present for.
///
/// `import_run` is the import whose products this job publishes, where one
/// asked for it. It is written in the transaction that inserts the job, under
/// that import's own row lock, so a publication either precedes the import's
/// deletion or is refused by it — and the job it writes carries the link that
/// lets the deletion fence it. Every caller with no import behind it passes
/// `None`, which is every caller but the scheduler's post-import
/// publication.
#[expect(
    clippy::too_many_arguments,
    reason = "one job is its identity, its marketplace, its author, its instant, its request \
              key, the import it publishes for and its items; a struct over those would be \
              this signature with a name"
)]
pub(crate) async fn mint_job(
    state: &AppState,
    org: OrgId,
    job: JobId,
    inventory: InventoryId,
    actor: Actor,
    now: Timestamp,
    request_key: Uuid,
    items: &[NewJobItem],
    import_run: Option<Uuid>,
) -> Result<tam_storage::CreatedJob, APIError> {
    crate::consent::require_grant(state, org, inventory.marketplace()).await?;
    let minted = JobRepo::new(state.pool.clone())
        .create_with_request_key(
            org,
            JobOrigin {
                request_key,
                run: None,
                import_run,
            },
            &NewJob {
                job,
                inventory,
                stamp: Stamp { at: now, actor },
            },
            items,
        )
        .await
        .map_err(|error| {
            if matches!(error, StorageError::DuplicateIdempotencyKey { .. }) {
                APIError::new(
                    StatusCode::CONFLICT,
                    APIErrorEntry::new(
                        "an identical sync item is already in the ledger; unchanged content \
                         does not need re-uploading",
                    )
                    .code(APIErrorCode::DuplicateSyncItem)
                    .kind(APIErrorKind::Validation),
                )
            } else {
                storage_fault(state, &error)
            }
        })?;
    match minted {
        tam_storage::Minted::Job(created) => Ok(created),
        tam_storage::Minted::WorkflowDeleted(workflow) => Err(workflow_deleted(workflow)),
    }
}

/// The refusal a mint answers when the workflow behind it has been stopped.
///
/// A conflict rather than a fault: nothing is wrong with the request, and the
/// state it conflicts with is one the seller created deliberately. The code is
/// what makes it actionable from outside this module — the scheduler skips
/// this product's publication and carries on with the rest of its pass rather
/// than failing the tenant's whole tick over one deleted import.
pub(crate) fn workflow_deleted(workflow: tam_storage::WorkflowKind) -> APIError {
    let entry = match workflow {
        // Coded, because the scheduler matches on it: a tenant's pass skips
        // the publication of a deleted import and carries on with the rest
        // rather than failing the whole tick. `ImportRunSettled` is the
        // existing name for "this import is over, a late write changes
        // nothing", which is exactly what a deletion makes it.
        tam_storage::WorkflowKind::ImportRun => APIErrorEntry::new(
            "this import has been deleted, so nothing further is published from it",
        )
        .code(APIErrorCode::ImportRunSettled),
        // Uncoded: nothing branches on it. The drain reads the refusal as the
        // end of the request and stops, and no client reaches this arm,
        // because the enqueue routes that take a request key fence
        // themselves before they mint.
        tam_storage::WorkflowKind::SyncRequest => {
            APIErrorEntry::new("this request has been deleted, so no further leg is queued for it")
        }
    };
    APIError::new(StatusCode::CONFLICT, entry.kind(APIErrorKind::Validation))
}

/// The one place the API mints row identity; v4 via the generator the
/// workspace already trusts for it.
fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

pub(crate) async fn list_jobs(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<PageParams>,
) -> Result<Json<JobPage>, APIError> {
    let page = parse_page(&params)?;
    // Refused rather than ignored. `outcome` is an item's, and this page
    // carries jobs; accepting it here and dropping it returned a well-formed
    // page of every job to a client the 422-on-typo behaviour had just told
    // the parameter was honoured.
    if page.outcome.is_some() {
        return Err(validation(
            "outcome is a filter on a job's items; ask for it on that job's items page",
        ));
    }
    let rows = JobReadRepo::new(state.pool.clone())
        .list_jobs(context.org, page.cursor, page.limit)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let next_cursor = (i64::try_from(rows.len()).unwrap_or(i64::MAX) == page.limit)
        .then(|| {
            rows.last().map(|last| {
                encode_cursor(&LedgerCursor {
                    created_at: last.created_at,
                    id: last.job.0,
                })
            })
        })
        .flatten();
    Ok(Json(JobPage {
        jobs: rows
            .into_iter()
            .map(|row| JobHead {
                job: row.job,
                inventory: row.inventory,
                created_at: row.created_at,
                deletion_status: row.deletion.map(DeletionStatusView::of),
            })
            .collect(),
        next_cursor,
    }))
}

/// A job carrying no items is settled, not active. `create_job` refuses an
/// empty mapping list and both storage constructors insert a job's items in
/// the transaction that creates it, so an itemless job is only ever an
/// import run recording its drain measurement — work that is over, and would
/// otherwise read as in flight forever.
const fn phase_of(counts: &ItemCounts) -> JobPhase {
    if counts.settled == counts.total {
        JobPhase::Settled
    } else {
        JobPhase::Active
    }
}

pub(crate) async fn job_view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, job)): Path<(String, String)>,
) -> Result<Json<JobView>, APIError> {
    let job = JobId(parse_id(&job)?);
    let snapshot = JobReadRepo::new(state.pool.clone())
        .snapshot(context.org, job)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such job"))?;
    let phase = phase_of(&snapshot.counts);
    Ok(Json(JobView {
        job: snapshot.job,
        inventory: snapshot.inventory,
        created_at: snapshot.created_at,
        phase,
        counts: CountsView::from_counts(snapshot.counts),
        deletion_status: snapshot.deletion.map(DeletionStatusView::of),
    }))
}

/// Deletes one publishing job, or the workflow that owns it.
///
/// Everything the module header says about the ledger applies to stopping it:
/// the fence goes up before any row is hidden, and a write already issued is
/// never written off. So this answers `200`/`deleted` only for a job whose
/// items are all settled and whose attempts are all decided; a job with a
/// live lease answers `202`/`stopping`, and one with an attempt nobody can
/// account for answers `202`/`needs_review` and stays in the seller's history
/// saying so.
///
/// Ownership is resolved before anything is fenced, because most jobs in the
/// seller's history are not independent. One leg of a migration is deleted by
/// deleting the migration: fencing the target create alone leaves the source
/// removal claimable, and the create's own settle then revives it — which is
/// the seller's original listing taken down after they stopped the move. An
/// import's event anchor is worse: it has no items, so stopping it is
/// instantaneous and changes nothing about the import whose device and
/// scheduler go on creating resources behind it. Only a job nothing owns is
/// stopped on its own.
///
/// It removes no listing, no product, no mapping and no file. The items it
/// cancels are items that had not run, and they keep their receipts.
pub(crate) async fn delete_job(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, job)): Path<(String, String)>,
) -> Result<Response, APIError> {
    let job = JobId(parse_id(&job)?);
    let jobs = JobRepo::new(state.pool.clone());
    let stamp = context.stamp((state.wall)());
    let owner = jobs
        .owner(context.org, job)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such job"))?;
    let status = match owner {
        tam_storage::JobOwner::SyncRequest(request) => {
            tam_storage::SyncRequestRepo::new(state.pool.clone())
                .delete(context.org, request, stamp)
                .await
        }
        tam_storage::JobOwner::ImportRun(run) => {
            tam_storage::ImportRunRepo::new(state.pool.clone())
                .delete(context.org, run, stamp)
                .await
        }
        tam_storage::JobOwner::Standalone => jobs.delete(context.org, job, stamp).await,
    }
    .map_err(|error| storage_fault(&state, &error))?
    // The owning workflow was read in this request and the job it owns
    // exists, so its absence here is not a 404 the seller can act on.
    .ok_or_else(|| {
        state.internal("the workflow owning this job could not be read back to stop it")
    })?;
    Ok(JobDeletionView::answer(status))
}

/// The replay of a creation key whose work was deleted.
///
/// A conflict rather than a 404 or a fresh enqueue, and the distinction is
/// what a client can act on: the key is spent and the work behind it is
/// deliberately gone, so the next move is a new request under a new key
/// rather than a retry of this one.
pub(crate) fn deleted_key_conflict(status: DeletionStatus) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "this request was deleted, so it cannot be started again under the same \
             idempotency key; start a new one",
        )
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({ "deletion_status": status.as_str() })),
    )
}

pub(crate) async fn job_items(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, job)): Path<(String, String)>,
    Query(params): Query<PageParams>,
) -> Result<Json<ItemsPage>, APIError> {
    let job = JobId(parse_id(&job)?);
    let page = parse_page(&params)?;
    let reads = JobReadRepo::new(state.pool.clone());
    reads
        .snapshot(context.org, job)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such job"))?;
    let rows = reads
        .items_page(
            context.org,
            job,
            ItemsPageParams {
                cursor: page.cursor,
                limit: page.limit,
                outcome: page.outcome,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let next_cursor = (i64::try_from(rows.len()).unwrap_or(i64::MAX) == page.limit)
        .then(|| {
            rows.last().map(|last| {
                encode_cursor(&LedgerCursor {
                    created_at: last.created_at,
                    id: last.item.0,
                })
            })
        })
        .flatten();
    Ok(Json(ItemsPage {
        items: rows.into_iter().map(ItemView::from_row).collect(),
        next_cursor,
    }))
}

/// How many steps one page of an item's timeline carries.
///
/// A retried item records an event per step per attempt, so the timeline is
/// the largest thing on an item's disclosure. Twenty-five is what the panel
/// shows; the cursor is `org_seq`, which is the total order the stream
/// already pages by.
const EVENT_PAGE_DEFAULT: i64 = 25;
const EVENT_PAGE_MAX: i64 = 200;

/// Where a page of one item's timeline starts.
#[derive(Debug, Default, Deserialize)]
pub struct EventPageParams {
    /// The `org_seq` of the last step already held.
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

pub(crate) async fn item_detail(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, job, item)): Path<(String, String, String)>,
    Query(params): Query<EventPageParams>,
) -> Result<Json<ItemDetail>, APIError> {
    let job = JobId(parse_id(&job)?);
    let item = JobItemId(parse_id(&item)?);
    let limit = params
        .limit
        .unwrap_or(EVENT_PAGE_DEFAULT)
        .clamp(1, EVENT_PAGE_MAX);
    let reads = JobReadRepo::new(state.pool.clone());
    // The one item, read as one row. This used to page every item of the job
    // with a limit of `i32::MAX` and then search the result in memory, so
    // opening one item of a five-hundred-item bulk read the whole bulk.
    let row = reads
        .item(context.org, job, item)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such item"))?;
    let events = reads
        .item_events(context.org, item, params.after, limit)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let events_next = (i64::try_from(events.len()).unwrap_or(i64::MAX) == limit)
        .then(|| events.last().map(|last| last.org_seq))
        .flatten();
    Ok(Json(ItemDetail {
        item: ItemView::from_row(row),
        events: events.into_iter().map(EventView::from_row).collect(),
        events_next,
    }))
}

impl OrgContext {
    /// Convenience for tests asserting who a view was computed for.
    #[must_use]
    pub const fn organisation(&self) -> OrgId {
        self.org
    }

    /// Who is asking and when, for a write that records both.
    ///
    /// A Delete is the seller's decision and the receipt has to say so: the
    /// deletion columns carry the actor beside the instant, and the session
    /// extractor is the only thing that knows which seller it was.
    #[must_use]
    pub const fn stamp(&self, at: Timestamp) -> Stamp {
        Stamp {
            at,
            actor: Actor::Person(self.user),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_cursor, deletion_status_str, encode_cursor, phase_of, DeletionStatusView, JobPhase,
        ALL_DELETION_STATUSES,
    };
    use tam_storage::{ItemCounts, LedgerCursor, ALL_DELETION_STATUSES as STORED_STATUSES};
    use tam_types::{Timestamp, Uuid};

    #[test]
    fn the_cursor_round_trips() {
        let cursor = LedgerCursor {
            created_at: Timestamp(1_732_000_123_456),
            id: Uuid([0x3D; 16]),
        };
        assert_eq!(
            decode_cursor(&encode_cursor(&cursor)),
            Some(cursor),
            "a minted token parses back to the same keyset position"
        );
    }

    #[test]
    fn a_job_carrying_no_items_is_settled_rather_than_active_forever() {
        assert_eq!(
            phase_of(&ItemCounts::default()),
            JobPhase::Settled,
            "an import run records its measurement against an itemless job"
        );
        assert_eq!(
            phase_of(&ItemCounts {
                total: 2,
                settled: 1,
                ..ItemCounts::default()
            }),
            JobPhase::Active,
            "a partly settled job is still in flight"
        );
        assert_eq!(
            phase_of(&ItemCounts {
                total: 2,
                settled: 2,
                ..ItemCounts::default()
            }),
            JobPhase::Settled,
            "every item settled settles the job"
        );
    }

    #[test]
    fn a_token_the_server_did_not_mint_is_refused() {
        assert_eq!(decode_cursor(""), None, "empty");
        assert_eq!(decode_cursor("zz.abcd"), None, "bad millis");
        assert_eq!(decode_cursor("10.short"), None, "bad id length");
        assert_eq!(decode_cursor("10"), None, "no separator");
    }

    /// The stored deletion vocabulary and the wire's are one vocabulary.
    ///
    /// Two enums, because the storage layer owns no wire format and the API
    /// owns no column — and one spelling drifting from the other is a state
    /// the console renders as an unknown string or refuses to decode at all.
    /// Iterated over both closed sets rather than spot-checked, so a fourth
    /// state added to either side fails here.
    #[test]
    fn the_wire_deletion_vocabulary_is_the_stored_one() {
        assert_eq!(
            STORED_STATUSES.len(),
            ALL_DELETION_STATUSES.len(),
            "one state per state"
        );
        for (stored, wire) in STORED_STATUSES.into_iter().zip(ALL_DELETION_STATUSES) {
            assert_eq!(
                DeletionStatusView::of(stored),
                wire,
                "the two sets are in the same order"
            );
            assert_eq!(
                deletion_status_str(wire),
                stored.as_str(),
                "and each state is spelled the same on the wire as in the column"
            );
            assert_eq!(
                serde_json::to_value(wire).ok(),
                Some(serde_json::Value::String(stored.as_str().to_owned())),
                "which is what the client's own union is generated from"
            );
        }
    }
}
