//! Binding one spreadsheet row to the bytes the seller matched to it.
//!
//! The bytes never travel here. `POST /{version}/uploads` already took them,
//! sealed them and answered a handle, and this route takes the handle: the
//! panel that matches a file to a row is doing to the import exactly what the
//! create form does to one resource, one step earlier. So the body is two
//! handles and the ceiling is the default one, not the spreadsheet's.
//!
//! Two handles rather than one because the upload answers two. It generates a
//! cover during the ingest and the create form sends it back, so a row that
//! carried only its payload would create a resource with no thumbnail — a
//! listing the seller has to open and fix one at a time, which is the whole of
//! what a bulk import exists to avoid.
//!
//! The panel sends `archive=keep_whole` on the upload for every tab. The row
//! model holds one payload handle, so an exploded archive's several handles
//! have nowhere to go; the bound is the row rather than the marketplace, which
//! is why this departs from the phase-0 note's per-tab recommendation.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    BatchState, BindOutcome, ImportBatchRepo, ProductRepo, RowAddress, RowFile, RowFiles, RowState,
    UnbindOutcome,
};
use tam_types::ContentHash;

use crate::catalogue::{parse_hash, FileHandle};
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::{kind_from_str, kind_str};
use crate::{AppState, OrgContext};

use super::{missing, parse_id, storage_fault, validation, BatchStateView, ImportRowView};

/// The two handles one row holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindBody {
    pub payload: FileHandle,
    pub cover: FileHandle,
}

/// One row after a bind, with the counts the panel renders beside it.
///
/// The counts travel with the row so the panel does not re-read the whole
/// batch after every file: a seller dropping thirty files on the drop zone
/// would otherwise fetch a thirty-row report thirty times.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundRowView {
    pub row: ImportRowView,
    pub batch_state: BatchStateView,
    /// Rows of this batch that hold bytes.
    pub attached: u32,
    /// Rows that must hold bytes before the commit will run and do not. D32 is
    /// the rule: a row that passed the parse and names a marketplace needs
    /// bytes whether it asked for draft or live, and a Teachouse row needs
    /// none. Zero is the commit's own gate.
    pub awaiting: u32,
}

/// Binds the payload and cover the seller matched to one row.
pub(crate) async fn bind(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, batch, sheet, ordinal)): Path<(String, String, String, String)>,
    Json(body): Json<BindBody>,
) -> Result<Json<BoundRowView>, APIError> {
    let batch = parse_id(&batch)?;
    let ordinal = parse_ordinal(&ordinal)?;
    let batches = ImportBatchRepo::new(state.pool.clone());

    let held = batches
        .get(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    if !held.state.admits_attachment() {
        return Err(batch_closed(held.state));
    }

    let payload = handle_of(&body.payload)?;
    let cover = handle_of(&body.cover)?;
    held_bytes(&state, context.org, &[&body.payload, &body.cover]).await?;

    let outcome = batches
        .bind_file(
            context.org,
            batch,
            RowAddress {
                sheet: &sheet,
                ordinal,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match outcome {
        BindOutcome::Bound(bound) => Ok(Json(BoundRowView {
            row: ImportRowView::of(bound.row),
            batch_state: BatchStateView::of(bound.batch_state),
            attached: bound.counts.attached,
            awaiting: bound.counts.awaiting,
        })),
        BindOutcome::NoSuchRow => Err(missing()),
        BindOutcome::BatchClosed(state) => Err(batch_closed(state)),
        BindOutcome::RowClosed(state) => Err(row_closed(state)),
    }
}

/// Clears the handles one row holds.
///
/// Answers 204 whether or not there was one to clear, because a double-clicked
/// remove is one action.
pub(crate) async fn unbind(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, batch, sheet, ordinal)): Path<(String, String, String, String)>,
) -> Result<StatusCode, APIError> {
    let batch = parse_id(&batch)?;
    let ordinal = parse_ordinal(&ordinal)?;
    let outcome = ImportBatchRepo::new(state.pool.clone())
        .unbind_file(
            context.org,
            batch,
            RowAddress {
                sheet: &sheet,
                ordinal,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match outcome {
        UnbindOutcome::Cleared => Ok(StatusCode::NO_CONTENT),
        UnbindOutcome::NoSuchRow => Err(missing()),
        UnbindOutcome::BatchClosed(state) => Err(batch_closed(state)),
    }
}

/// The seller's own spreadsheet row number, as the path names it.
///
/// Read through `i32` rather than `u32` because that is the column's own
/// width: an ordinal past it names no row, and refusing it here is what keeps
/// a hostile path from reaching the repository as a fault.
fn parse_ordinal(raw: &str) -> Result<u32, APIError> {
    let parsed: i32 = raw
        .parse()
        .map_err(|_| validation("a row is addressed by its spreadsheet row number"))?;
    u32::try_from(parsed)
        .ok()
        .filter(|ordinal| *ordinal > 0)
        .ok_or_else(|| validation("a spreadsheet row number counts from one"))
}

/// One handle as the row holds it, refusing what the create route refuses.
///
/// The hash and the kind are read through the same two conversions
/// `POST /{version}/products` reads them through, so a handle this route
/// accepts is a handle the commit's create will take: a row bound here and
/// refused there would be a seller told twice about one file, once too late to
/// fix it. The length is checked against the column rather than against the
/// stored blob, which is the create route's own position on it.
fn handle_of(handle: &FileHandle) -> Result<RowFile, APIError> {
    let hash = parse_hash(&handle.hash)
        .ok_or_else(|| validation("a file handle's hash is not a 64-character hex digest"))?;
    let kind = kind_from_str(&handle.kind)
        .ok_or_else(|| validation("a file handle names a kind this server does not store"))?;
    let byte_len = i64::try_from(handle.byte_len)
        .map_err(|_| validation("a file handle states a length no stored file can have"))?;
    Ok(RowFile {
        hash,
        // The vocabulary's own spelling rather than the caller's, so the column
        // holds what the product view reads back.
        kind: kind_str(kind).to_owned(),
        byte_len,
    })
}

/// Refuses a handle naming bytes this tenant has never uploaded.
///
/// The create route's own check, applied to the two handles a bind carries,
/// and not merely the foreign key's: the key would refuse the write as a
/// database fault, and this is the same refusal as a sentence the seller can
/// act on.
async fn held_bytes(
    state: &AppState,
    org: tam_types::OrgId,
    handles: &[&FileHandle],
) -> Result<(), APIError> {
    let claimed: Vec<ContentHash> = handles
        .iter()
        .filter_map(|handle| parse_hash(&handle.hash))
        .collect();
    let known: Vec<ContentHash> = ProductRepo::new(state.pool.clone())
        .stored_hashes(org, &claimed)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .map(|(hash, _length)| hash)
        .collect();
    let unknown: Vec<String> = handles
        .iter()
        .filter(|handle| !parse_hash(&handle.hash).is_some_and(|hash| known.contains(&hash)))
        .map(|handle| handle.hash.clone())
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new("a file handle names bytes this organisation has not uploaded")
            .code(APIErrorCode::UploadRejected)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({ "hashes": unknown })),
    ))
}

/// The batch takes no more files.
///
/// Carries the state rather than only saying so, because what the seller does
/// next differs by which one it is: a settled batch is read, and an importing
/// one is waited on.
fn batch_closed(state: BatchState) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new("this import is no longer taking files")
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "batch_state": BatchStateView::of(state),
            })),
    )
}

fn row_closed(state: RowState) -> APIError {
    let sentence = match state {
        RowState::Failed => {
            "this row was refused by the parse and cannot be created, so it takes no file"
        }
        RowState::Skipped => "this row was left out of this import, so it takes no file",
        RowState::Creating | RowState::Created | RowState::Published => {
            "this row has already been created, so it takes no more files"
        }
        // Screened out by `RowState::admits_attachment` before this is
        // reached. Stated rather than wildcarded, so a state added in storage
        // is a compile error here rather than a sentence chosen by omission.
        RowState::Parsed | RowState::Attached => "this row takes no file",
    };
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(sentence).kind(APIErrorKind::Validation),
    )
}
