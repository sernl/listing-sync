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
//! completes it or re-runs the create under the same identifier. No pass can
//! therefore mint a second product for one spreadsheet row, which is the
//! hazard migration 0058's own header names.
//!
//! The product is the first of a row's writes rather than the whole of them:
//! its mapping, its elections and its labels trail it, each in its own
//! transaction, so a pass killed after the insert and before those leaves a
//! product missing some of what its row named. The resume covers that too. A
//! row whose product exists has the trailing writes run again, and each of
//! them completes what is missing and re-writes nothing that stands, so the
//! row settles as `created` only once everything it named exists.
//!
//! What no chunk can see is a batch nobody drives: a row stays claimed and a
//! batch stays `importing` until a chunk returns, and the expiry sweep's stale
//! rule in [`super::sweep`] is what returns a batch a dead pass left there.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_limits::Capabilities;
use tam_storage::{
    BatchState, ClaimedRow, CommitOpening, ImportBatchRepo, ProductRepo, RowAddress, RowRef,
};
use tam_types::OrgId;

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
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
    if ImportBatchRepo::new(state.pool.clone())
        .get(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .is_none()
    {
        return Err(missing());
    }
    // The run this batch is reviewed through, opened on the first chunk. The
    // duplicate review is one thing for both sources -- one pair table, one
    // verdict, one card -- so a batch reaches it by having a run rather than
    // by growing a second review of its own.
    let run = crate::import_runs::run_for_batch(&state, context.org, batch).await?;
    // Pressing Commit on a batch is the seller's own confirmation that these
    // resources are to be created, recorded against the run with the actor
    // and the instant beside it. The same fact the marketplace path records,
    // so both sources reach the catalogue through one authorisation rule
    // rather than two — and it is what lets the server's own drain finish
    // this batch after the seller closes the tab.
    tam_storage::ImportRunRepo::new(state.pool.clone())
        .authorise_commit(
            context.org,
            run.id,
            &uuid::Uuid::from_bytes(context.user.0 .0).to_string(),
            (state.wall)(),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    chunk(&state, context.org, context.entitlement.caps, batch, &run).await
}

/// One chunk of a batch commit: claim a page of rows, decide each under the
/// organisation's guard, create what may be created, and settle when nothing
/// is left.
///
/// One implementation for the seller's own press of the button and for the
/// server's drain, because the guards are the point: a resumed batch must not
/// create from a decision the catalogue has moved under, and neither caller
/// may advance a run the seller stopped.
async fn chunk(
    state: &AppState,
    org: OrgId,
    caps: Capabilities,
    batch: tam_types::Uuid,
    run: &tam_storage::ImportRunHead,
) -> Result<Json<CommitAck>, APIError> {
    let batches = ImportBatchRepo::new(state.pool.clone());
    let held = batches
        .get(org, batch)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(missing)?;
    // The parse's own counts, and the reason they are read before anything is
    // claimed: `row_count` and `failed_count` are written once, at upload, and
    // never moved, so the total a seller watches is the same number on every
    // chunk.
    let total = held.row_count.saturating_sub(held.failed_count);

    match batches
        .open_commit(org, batch)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        CommitOpening::Open => {}
        CommitOpening::NoSuchBatch => return Err(missing()),
        CommitOpening::BatchClosed(settled) => return Err(batch_settled(settled)),
        CommitOpening::Awaiting { count, rows } => return Err(awaiting_files(count, &rows)),
    }

    let claimed = batches
        .claim_page(org, batch, ROWS_PER_CHUNK, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mut applied = 0_u32;
    let mut skipped = 0_u32;
    let mut failed = 0_u32;
    for row in &claimed {
        match apply_row(state, org, caps, run, batch, row).await {
            Ok(RowOutcome::Created) => applied = applied.saturating_add(1),
            // A row whose product an earlier pass had already created, and a
            // row another resource already stands for: neither is new work
            // and neither is a failure.
            Ok(RowOutcome::Completed | RowOutcome::Skipped) => {
                skipped = skipped.saturating_add(1);
            }
            // A question is owed about it. Left claimed on purpose:
            // `claim_page` re-claims a `creating` row, so the next chunk after
            // the seller answers picks it up without the batch having to
            // remember anything.
            Ok(RowOutcome::Held) => {}
            Err(refusal)
                if refusal
                    .errors
                    .iter()
                    .any(|entry| entry.code == Some(APIErrorCode::ImportRunSettled)) =>
            {
                return Err(refusal);
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
                // product identifier. The loser's whole row transaction is
                // refused rather than minting a second product, and the 500 it
                // raises is retriable here. That doubled pickup is the
                // accepted tradeoff, not a fault.
                if refusal.status_code().is_server_error() {
                    return Err(refusal);
                }
                batches
                    .record_row_failed(
                        org,
                        batch,
                        RowAddress {
                            sheet: &row.sheet,
                            ordinal: row.ordinal,
                        },
                        &reason_of(&refusal),
                    )
                    .await
                    .map_err(|error| storage_fault(state, &error))?;
                failed = failed.saturating_add(1);
            }
        }
    }

    let counts = batches
        .pending_counts(org, batch)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let batch_state = if counts.outstanding == 0 {
        let settled = batches
            .settle(org, batch, (state.wall)())
            .await
            .map_err(|error| storage_fault(state, &error))?;
        crate::import_runs::settle_run(state, org, run, tam_storage::RunState::Complete).await?;
        settled
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

/// How many chunks one drain pass runs for a batch.
///
/// The same reasoning as the marketplace drain's: bounded, because the pass
/// is shared with every other tenant's work, and a batch too large for one
/// pass is finished by the next.
const CHUNKS_PER_DRAIN: u32 = 8;

/// Finishes an authorised batch that nothing is driving.
///
/// This is what a closed browser used to cost: the batch stayed `importing`
/// with its rows claimed and its run authorised, and nothing picked it up
/// until the expiry sweep abandoned it days later. The seller's press of
/// Commit is the authorisation, and the server's own pass is what finishes
/// the work.
pub(crate) async fn drain_batch_run(
    state: &AppState,
    org: OrgId,
    run: &tam_storage::ImportRunHead,
) -> Result<u32, APIError> {
    if !run.execution.commit_authorised(run.scheduled) {
        return Ok(0);
    }
    let Some(batch) = run.batch_id else {
        return Ok(0);
    };
    let caps = crate::entitlement::Entitlement::of(
        tam_storage::EntitlementRepo::new(state.pool.clone())
            .current(org, (state.wall)())
            .await
            .map_err(|error| storage_fault(state, &error))?,
    )
    .caps;
    let mut created = 0_u32;
    for _ in 0..CHUNKS_PER_DRAIN {
        let ack = match chunk(state, org, caps, batch, run).await {
            Ok(ack) => ack,
            // A batch that has settled, or whose files are still missing, is
            // not this pass's to force: the seller's own page says what it is
            // waiting for, and a drain that failed the tenant's whole pass
            // over it would take every other run with it.
            Err(refusal) if !refusal.status_code().is_server_error() => return Ok(created),
            Err(refusal) => return Err(refusal),
        };
        created = created.saturating_add(ack.applied);
        if ack.complete || (ack.applied == 0 && ack.failed == 0) {
            return Ok(created);
        }
    }
    Ok(created)
}

/// One claimed row, decided and applied in one transaction.
///
/// Everything the row's outcome consists of is written under the
/// organisation's catalogue guard: the matcher's revalidated decision, the
/// product with its sidecar, its mappings, its elections, its labels and the
/// row's own breadcrumb. A pass that stops mid-row therefore leaves nothing —
/// no product whose row still reads `creating`, and no row marked created
/// against a product that was rolled back.
///
/// The existence read still decides which road a resumed row takes: a row
/// whose reserved product exists was created by an earlier pass, and creating
/// it again would charge the seller twice, so the trailing writes run instead
/// and each completes what is missing without rewriting what stands.
///
/// Answers what this pass did with the row.
#[expect(
    clippy::too_many_arguments,
    reason = "the state, the tenant, its capabilities, the run the row is reviewed through, the batch, the row itself and the plan's review capability; each comes from a different place"
)]
async fn apply_row(
    state: &AppState,
    org: OrgId,
    caps: Capabilities,
    run: &tam_storage::ImportRunHead,
    batch: tam_types::Uuid,
    row: &ClaimedRow,
) -> Result<RowOutcome, APIError> {
    let at = RowAddress {
        sheet: &row.sheet,
        ordinal: row.ordinal,
    };
    let locator = crate::import_runs::row_locator(&row.sheet, row.ordinal);
    // Everything slow and everything refusable, before the lock: the row's
    // own lowering, the create's validation and reads, and the matcher's
    // blocking reads.
    let already = ProductRepo::new(state.pool.clone())
        .get(org, row.product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .is_some();
    let Lowered { body, labels } = lower(row)?;
    let mappings: Vec<tam_types::MappingId> = row.mapping.into_iter().collect();
    let prepared = if already {
        None
    } else {
        Some(crate::catalogue::prepare_create(state, org, caps, &body, row.product).await?)
    };
    let matched = crate::import_runs::prepare_spreadsheet_match(row, caps.duplicate_review);

    let now = (state.wall)();
    let mut tx = crate::import_runs::begin_guarded(state, org).await?;
    // The run's own guard: a batch whose import the seller stopped advances
    // nothing, and the refusal is the answer rather than a silent no-op that
    // lets the create run anyway.
    let guard = tam_storage::guard_run(&mut tx, org, run.id)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(missing)?;
    if !guard.state.open() {
        tx.rollback()
            .await
            .map_err(|error| storage_fault_tx(state, &error))?;
        return Err(crate::import_runs::run_settled_refusal(guard.state));
    }

    // The decision, re-asked here against what is committed now, so a batch
    // that paused while another source committed the same resource does not
    // create a second one.
    if !already {
        let verdict = crate::import_runs::apply_spreadsheet_match(
            &mut tx, state, org, run, &locator, &matched, now,
        )
        .await?;
        match verdict {
            // Left claimed on purpose: `claim_page` re-claims a `creating` row, so
            // the next chunk after the seller answers picks it up without the
            // batch having to remember anything.
            tam_storage::RunItemState::Review => {
                tx.commit()
                    .await
                    .map_err(|error| storage_fault_tx(state, &error))?;
                return Ok(RowOutcome::Held);
            }
            // The seller answered that another resource already stands for this
            // row, so creating it is exactly what they said not to do.
            tam_storage::RunItemState::Skipped => {
                tam_storage::record_row_skipped_in(&mut tx, org, batch, at)
                    .await
                    .map_err(|error| storage_fault(state, &error))?;
                tx.commit()
                    .await
                    .map_err(|error| storage_fault_tx(state, &error))?;
                return Ok(RowOutcome::Skipped);
            }
            // Every other state is one the create is the next step for. Named
            // rather than wildcarded, so a state added to the run's own vocabulary
            // fails here rather than falling silently into a create.
            tam_storage::RunItemState::Listed
            | tam_storage::RunItemState::Selected
            | tam_storage::RunItemState::Read
            | tam_storage::RunItemState::Matched
            | tam_storage::RunItemState::Imported
            | tam_storage::RunItemState::Failed => {}
        }
    }

    let plan = crate::catalogue::CreatePlan {
        inventories: &body.inventories,
        elections: &body.elections,
        mappings: &mappings,
    };
    match prepared.as_ref() {
        Some(prepared) => {
            crate::catalogue::apply_create(&mut tx, state, org, prepared, &plan).await?;
        }
        // The product exists from an earlier pass, so only what trails it is
        // completed.
        None => {
            crate::catalogue::finish_existing(&mut tx, state, org, row.product, &body, &plan)
                .await?;
        }
    }
    // After the create, because `product_label` names a product. A label the
    // organisation does not hold yet is created by this write, which is what
    // the report's new-label warning told the seller it would do.
    crate::catalogue::set_labels(&mut tx, org, row.product, &labels, now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    tam_storage::record_row_created(&mut tx, org, batch, at, already)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    tx.commit()
        .await
        .map_err(|error| storage_fault_tx(state, &error))?;
    Ok(if already {
        RowOutcome::Completed
    } else {
        RowOutcome::Created
    })
}

/// What one row's pass did.
enum RowOutcome {
    /// A product was created for it.
    Created,
    /// Its product existed from an earlier pass; the trailing writes were
    /// completed.
    Completed,
    /// Another resource already stands for it.
    Skipped,
    /// A question is owed about it, so it stays claimed for the next chunk.
    Held,
}

/// A transaction the database refused, reported as ours.
fn storage_fault_tx(state: &AppState, error: &sqlx::Error) -> APIError {
    state.internal(&format!("the database refused a transaction: {error}"))
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
        APIErrorEntry::new("This import has already finished.")
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
        APIErrorEntry::new("Attach a file to every row that names a marketplace, then import.")
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({ "awaiting": count, "rows": named })),
    )
}
