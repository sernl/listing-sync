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
use tam_domain::{ItemOutcome, JobItemId};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_storage::{
    payload_digest, EventRow, ItemCounts, ItemRow, JobReadRepo, JobRepo, LedgerCursor, NewJob,
    NewJobItem, StorageError,
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
}

struct Page {
    cursor: Option<LedgerCursor>,
    limit: i64,
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
    Ok(Page { cursor, limit })
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
    let items: Vec<NewJobItem> = seeds
        .iter()
        .map(|seed| NewJobItem {
            item: JobItemId(fresh_uuid()),
            mapping: seed.mapping,
            idempotency_key: derive_idempotency_key(
                context.org,
                body.inventory,
                seed.product,
                INTENT_VERSION,
                payload_digest(&seed.payload_hashes),
            ),
        })
        .collect();
    let new = NewJob {
        job: JobId(fresh_uuid()),
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
    let phase = if snapshot.counts.total > 0 && snapshot.counts.settled == snapshot.counts.total {
        JobPhase::Settled
    } else {
        JobPhase::Active
    };
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
        .items_page(context.org, job, page.cursor, page.limit)
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
        .items_page(context.org, job, None, i64::from(i32::MAX))
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
    use super::{decode_cursor, encode_cursor};
    use tam_storage::LedgerCursor;
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
    fn a_token_the_server_did_not_mint_is_refused() {
        assert_eq!(decode_cursor(""), None, "empty");
        assert_eq!(decode_cursor("zz.abcd"), None, "bad millis");
        assert_eq!(decode_cursor("10.short"), None, "bad id length");
        assert_eq!(decode_cursor("10"), None, "no separator");
    }
}
