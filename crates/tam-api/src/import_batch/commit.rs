//! Turning a held batch into resources, a chunk at a time.
//!
//! The commit stops at the catalogue. A row that asked to go live reaches
//! `created` and no further: no job is minted, `import_batch_row.job_id` stays
//! null and [`tam_storage::RowState::Published`] stays unreachable from here.
//! That is D1 rather than an omission — publishing to a no-API marketplace
//! originates on the seller's own device, so a server that enqueued one here
//! would be composing a marketplace request on their behalf.
//!
//! Resumability is a row breadcrumb rather than a request key, which is what
//! makes a closed browser lose only the chunk in flight. Each chunk claims a
//! page of rows and reserves their product and mapping identifiers in one short
//! transaction, then creates them one at a time outside it. A pass killed
//! between the two leaves rows in `creating` with identifiers already reserved,
//! and the next chunk reads whether the reserved product exists and either
//! records the breadcrumb or re-runs the create under the same identifier. No
//! pass can therefore mint a second product for one spreadsheet row, which is
//! the hazard migration 0058's own header names. That is the whole of it: the
//! resume proves the reserved product exists and re-runs none of the mapping,
//! election and label writes trailing it, so a pass killed between the product
//! insert and those writes leaves a row that settles as `created` with them
//! missing and nothing on the wire saying so. Widening the check to cover them
//! is recorded as a follow-up in the phase's scope note rather than taken here.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    BatchState, ClaimedRow, CommitOpening, ImportBatchRepo, LabelRepo, ProductRepo, RowAddress,
    RowRef,
};
use tam_types::OrgId;

use crate::catalogue::create_one;
use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

use super::lower::{lower, Lowered};
use super::{missing, parse_id, storage_fault, BatchStateView};

/// How many rows one chunk creates.
///
/// A pacing figure rather than a seller-visible ceiling, so it lives here
/// beside the code it paces, following `parse.rs`'s own bounds, rather than in
/// `tam-limits` where the founder-gated limits are.
///
/// Sized by how long a seller will wait for one answer, not by what the
/// database can take: a row is a whole create — the quota read, the held-bytes
/// check, a read-back of the cover's bytes to prove they are a picture, the
/// product insert, the mapping insert, the elections and the labels — so
/// twenty-five of them is a second or two of work. The batch ceiling is
/// `tam_limits::import::ROWS_PER_UPLOAD_MAX`, so a full import is a couple of
/// dozen requests, each one a point the seller can close the tab at.
const ROWS_PER_CHUNK: i64 = 25;

/// What one chunk did, and where the batch stands after it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitAck {
    /// Rows this chunk created.
    pub applied: u32,
    /// Rows this chunk claimed that an earlier pass had already created. A
    /// resumed commit reports these rather than counting them as new work, and
    /// on a first pass it is zero.
    pub skipped: u32,
    /// Rows this chunk refused, each carrying the reason on its own report row.
    pub failed: u32,
    /// Rows this commit will ever touch: every row the parse accepted. Read
    /// off the batch's own stored counts, which the parse wrote and nothing
    /// since has moved, so it does not shrink as rows are created.
    pub total: u32,
    /// Rows still to do. Answered rather than left to a client's running sum,
    /// because a seller who reloads the page mid-import has no sum to resume
    /// from and a progress bar that restarts at zero is a lie about the work.
    pub remaining: u32,
    pub complete: bool,
    pub batch_state: BatchStateView,
}

/// Commits the next chunk of a held batch.
pub(crate) async fn commit(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, batch)): Path<(String, String)>,
) -> Result<Json<CommitAck>, APIError> {
    let batch = parse_id(&batch)?;
    let batches = ImportBatchRepo::new(state.pool.clone());
    let held = batches
        .get(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    // The parse's own counts, and the reason they are read before anything is
    // claimed: `row_count` and `failed_count` are written once, at upload, and
    // never moved, so the total a seller watches is the same number on every
    // chunk.
    let total = held.row_count.saturating_sub(held.failed_count);

    match batches
        .open_commit(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        CommitOpening::Open => {}
        CommitOpening::NoSuchBatch => return Err(missing()),
        CommitOpening::BatchClosed(settled) => return Err(batch_settled(settled)),
        CommitOpening::Awaiting { count, rows } => return Err(awaiting_files(count, &rows)),
    }

    let claimed = batches
        .claim_page(context.org, batch, ROWS_PER_CHUNK)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let mut applied = 0_u32;
    let mut skipped = 0_u32;
    let mut failed = 0_u32;
    for row in &claimed {
        let at = RowAddress {
            sheet: &row.sheet,
            ordinal: row.ordinal,
        };
        match create_row(&state, context.org, row).await {
            Ok(true) => {
                batches
                    .record_created(context.org, batch, at)
                    .await
                    .map_err(|error| storage_fault(&state, &error))?;
                applied = applied.saturating_add(1);
            }
            Ok(false) => {
                batches
                    .record_created(context.org, batch, at)
                    .await
                    .map_err(|error| storage_fault(&state, &error))?;
                skipped = skipped.saturating_add(1);
            }
            Err(refusal) => {
                // A refusal the seller can act on is recorded against the row
                // and the chunk carries on; anything else is ours and fails
                // the request, leaving the row claimed for the next chunk to
                // retry. Marking a row permanently failed because the database
                // was briefly unreachable would cost the seller a resource
                // over a fault that has already passed. Two commit calls in
                // flight on one batch reach here the same way: a claim's lock
                // ends with its transaction and a row left `creating` by a
                // crash reads no differently from one a live chunk holds, so
                // both can claim it, and both are handed the same reserved
                // product identifier. The loser's insert is refused by the
                // primary key rather than minting a second product, and the
                // 500 it raises is retriable here — the row stays claimed for
                // the next chunk. That doubled pickup is the accepted
                // tradeoff, not a fault.
                if refusal.status_code().is_server_error() {
                    return Err(refusal);
                }
                batches
                    .record_row_failed(context.org, batch, at, &reason_of(&refusal))
                    .await
                    .map_err(|error| storage_fault(&state, &error))?;
                failed = failed.saturating_add(1);
            }
        }
    }

    let counts = batches
        .pending_counts(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let batch_state = if counts.outstanding == 0 {
        batches
            .settle(context.org, batch, (state.wall)())
            .await
            .map_err(|error| storage_fault(&state, &error))?
    } else {
        BatchState::Importing
    };

    Ok(Json(CommitAck {
        applied,
        skipped,
        failed,
        total,
        remaining: counts.outstanding,
        complete: counts.outstanding == 0,
        batch_state: BatchStateView::of(batch_state),
    }))
}

/// Creates one claimed row, answering whether this pass is what created it.
///
/// The existence read comes first and is the whole of the resume: a row whose
/// reserved product already exists was created by an earlier pass that died
/// before writing its breadcrumb, and creating it again would charge the
/// seller twice for one spreadsheet row. It proves the product and nothing
/// downstream of it, so the skip re-runs neither the mappings and elections
/// `create_one` writes nor the labels set below; the module doc says what that
/// leaves open.
async fn create_row(state: &AppState, org: OrgId, row: &ClaimedRow) -> Result<bool, APIError> {
    let already = ProductRepo::new(state.pool.clone())
        .get(org, row.product)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if already.is_some() {
        return Ok(false);
    }

    let Lowered { body, labels } = lower(row)?;
    let mappings: Vec<tam_types::MappingId> = row.mapping.into_iter().collect();
    create_one(state, org, &body, row.product, &mappings).await?;

    // After the create, because `product_label` names a product. A label the
    // organisation does not hold yet is created by this write, which is what
    // the report's new-label warning told the seller it would do; the names
    // reached the column through the label route's own rules at parse time,
    // so nothing is validated a second time here.
    LabelRepo::new(state.pool.clone())
        .set_for_product(org, row.product, &labels, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(true)
}

/// The sentence a refused row carries on the report.
fn reason_of(refusal: &APIError) -> String {
    refusal
        .errors
        .first()
        .map_or_else(|| refusal.to_string(), |entry| entry.message.clone())
}

/// The batch is settled and creates nothing more.
fn batch_settled(state: BatchState) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new("this import has already finished")
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "batch_state": BatchStateView::of(state),
            })),
    )
}

/// D32's gate: every row that names a marketplace holds bytes before any row
/// is created.
///
/// A refusal rather than a partial import, and the count and the first rows
/// travel with it so the panel can say which files are still missing instead
/// of sending the seller back to read the whole report.
fn awaiting_files(count: u32, rows: &[RowRef]) -> APIError {
    let named: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| serde_json::json!({ "sheet": row.sheet, "ordinal": row.ordinal }))
        .collect();
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "every row that names a marketplace needs its file attached before the import runs",
        )
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({ "awaiting": count, "rows": named })),
    )
}
