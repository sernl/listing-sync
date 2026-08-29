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
    intent_digest, Disposition, EventRow, ItemCounts, ItemRow, ItemsPageParams, JobReadRepo,
    JobRepo, LedgerCursor, MappingSeed, NewJob, NewJobItem, NewSyncRequest, StorageError,
    SyncIntent, SyncRequestRepo,
};
use tam_types::{FailureCode, InventoryId, JobId, MappingId, OrgId, Timestamp, Uuid};

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

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    Active,
    Settled,
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
    pub events: Vec<EventView>,
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

fn storage_fault(state: &AppState, error: &StorageError) -> APIError {
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
    if body.resources.is_empty() {
        return Err(validation("a sync names at least one resource"));
    }
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
    let new = NewSyncRequest {
        // The idempotency key is the request's identity, so a retried submit
        // is the same request rather than a second one.
        id: key.0,
        source: body.source,
        target: body.target,
        disposition,
        intent,
        requested_at: (state.wall)(),
        locators: body.resources,
    };
    // The insert decides the replay, rather than a read the two submits then
    // race: both saw no existing row, one INSERT won and the other violated
    // the primary key, so the double-click this endpoint exists to absorb
    // came back a fault. The request's identity is the key, so the ack is the
    // same either way.
    let written = SyncRequestRepo::new(state.pool.clone())
        .create(context.org, &new)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
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
    pub resources: Vec<SyncResourceView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SyncResourceView {
    pub ordinal: i32,
    pub locator: String,
    pub state: String,
    pub failure_detail: Option<String>,
}

/// What the client polls between asking for a sync and the ledger having
/// something to show them.
pub(crate) async fn sync_request_view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, request)): Path<(String, String)>,
) -> Result<Json<SyncRequestView>, APIError> {
    let request = parse_id(&request)?;
    let record = SyncRequestRepo::new(state.pool.clone())
        .get(context.org, request)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such sync request"))?;
    Ok(Json(SyncRequestView {
        request: record.id,
        source: record.source,
        target: record.target,
        disposition: record.disposition.as_str().to_owned(),
        intent: record.intent.as_str().to_owned(),
        state: record.state,
        failure_detail: record.failure_detail,
        create_job: record.create_job,
        remove_job: record.remove_job,
        resources: record
            .resources
            .into_iter()
            .map(|row| SyncResourceView {
                ordinal: row.ordinal,
                locator: row.locator,
                state: row.state,
                failure_detail: row.failure_detail,
            })
            .collect(),
    }))
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
        for operation in lower(intent, body.inventory, seed)? {
            items.push(NewJobItem {
                item: JobItemId(fresh_uuid()),
                mapping: seed.mapping,
                idempotency_key: derive_idempotency_key(
                    context.org,
                    body.inventory,
                    seed.product,
                    INTENT_VERSION,
                    intent_digest(&operation, job, &seed.payload_hashes, seed.sever_generation),
                ),
                requires_bound_on: tam_storage::requires_bound_on(&operation, body.inventory),
                operation,
            });
        }
    }
    let new = NewJob {
        job,
        inventory: body.inventory,
        at: now,
    };
    let created = JobRepo::new(state.pool.clone())
        .create_with_request_key(context.org, key.0, &new, &items)
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
                storage_fault(&state, &error)
            }
        })?;
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
    }))
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

pub(crate) async fn item_detail(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, job, item)): Path<(String, String, String)>,
) -> Result<Json<ItemDetail>, APIError> {
    let job = JobId(parse_id(&job)?);
    let item = JobItemId(parse_id(&item)?);
    let reads = JobReadRepo::new(state.pool.clone());
    let rows = reads
        .items_page(
            context.org,
            job,
            ItemsPageParams {
                cursor: None,
                limit: i64::from(i32::MAX),
                outcome: None,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let row = rows
        .into_iter()
        .find(|row| row.item == item)
        .ok_or_else(|| missing("no such item"))?;
    let events = reads
        .item_events(context.org, item)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ItemDetail {
        item: ItemView::from_row(row),
        events: events.into_iter().map(EventView::from_row).collect(),
    }))
}

impl OrgContext {
    /// Convenience for tests asserting who a view was computed for.
    #[must_use]
    pub const fn organisation(&self) -> OrgId {
        self.org
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_cursor, encode_cursor, phase_of, JobPhase};
    use tam_storage::{ItemCounts, LedgerCursor};
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
}
