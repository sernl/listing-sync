//! The spreadsheet import: a template workbook the seller fills, uploads, and
//! publishes from.
//!
//! A second way to fill the same form. Every write this surface eventually
//! performs is a call the console already makes from its own create form, and
//! nothing here invents an authoring path: a row becomes
//! `POST /{version}/products`, its labels become
//! `PUT /{version}/products/{product}/labels`, and a live row's publish becomes
//! `POST /{version}/jobs` for the seller's own device to claim. This module
//! parses, validates and holds; it creates nothing.
//!
//! Five routes in this phase. The template is generated from the registry on
//! request; an upload is parsed, validated and written as a batch with a row
//! per spreadsheet row and no product; the listing and the batch view serve the
//! report; and an abandon settles a batch the seller has given up on. The file
//! attachment and the commit are the phases after this one, and the row model
//! already carries the columns they fill.
//!
//! The spreadsheet is metadata only. No byte of a seller's resource travels
//! through it, and no request to a marketplace originates here or anywhere else
//! on this server for a no-API marketplace.

pub mod parse;
pub mod report;
pub mod sheet;
pub mod sweep;
pub mod template;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::{body::Bytes, Json};
use serde::{Deserialize, Serialize};
use tam_storage::{
    BatchState, BatchWrite, ConnectionRepo, ImportBatchRecord, ImportBatchRepo,
    ImportBatchRowRecord, LabelRepo, NewImportBatch, NewImportBatchRow, RowIntent, RowState,
    StorageError,
};
use tam_types::{InventoryId, Marketplace, Timestamp, Uuid};

use crate::blocking::spawn_supervised_blocking;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::RequestKey;
use crate::{AppState, OrgContext};

use self::parse::Malformed;
use self::report::{ParsedRow, Problem};

/// How long a batch survives unfinished, as milliseconds.
///
/// Derived from the founder's own figure in `tam_limits` rather than restated,
/// and computed once here because the batch stores its deadline as an instant:
/// the seller is told a date, and a date computed at render time would move
/// under them the day the constant moved.
fn expiry_of(created_at: Timestamp) -> Timestamp {
    const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1_000;
    Timestamp(
        created_at
            .0
            .saturating_add(tam_limits::import::BATCH_EXPIRY_DAYS.saturating_mul(MILLIS_PER_DAY)),
    )
}

/// The upload route's own body ceiling.
///
/// Its own rather than the resource upload's, and the reason is the format: an
/// xlsx is a zip, and a zip at the payload ceiling is a parse bomb whose
/// expansion the ingest pipeline's archive bounds never see, because this
/// upload deliberately does not go through the ingest route.
pub fn spreadsheet_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(
        usize::try_from(tam_limits::import::SPREADSHEET_BYTES_MAX).unwrap_or(usize::MAX),
    )
}

fn storage_fault(state: &AppState, error: &StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn missing() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("no such import")
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_| validation("the identifier is not a UUID"))
}

/// A sheet that could not be read into rows at all, as the caller's own fault.
///
/// Every arm of [`Malformed`] refuses the upload before a row exists to name,
/// so none of it can be reported per row and all of it is one entry. The code
/// is the upload's own rather than a new one: what the seller does about it is
/// the same thing they do about a refused resource upload, which is fix the
/// file and send it again.
fn malformed(state: &AppState, refusal: &Malformed) -> APIError {
    let entry = APIErrorEntry::new(&refusal.to_string())
        .code(APIErrorCode::UploadRejected)
        .kind(APIErrorKind::Validation);
    // The upstream crate's own words, where one was wrapped, behind the same
    // disclosure gate every other internal detail crosses. This crate holds no
    // logger, so the disclosure split is the mechanism it actually has: the
    // seller reads this module's sentence either way, and a development
    // deployment additionally reads calamine's or zip's.
    let entry = match (refusal.upstream(), state.config.disclosure) {
        (Some(detail), crate::error::Disclosure::Full) => entry.reason(detail),
        (Some(_) | None, crate::error::Disclosure::Full | crate::error::Disclosure::Redacted) => {
            entry
        }
    };
    APIError::new(StatusCode::UNPROCESSABLE_ENTITY, entry)
}

// ------------------------------------------------------------------- views

/// Where a batch stands, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStateView {
    Parsed,
    Attaching,
    Importing,
    Imported,
    Failed,
    Abandoned,
}

impl BatchStateView {
    /// The closed set, in a stable order, for the client's own exhaustive
    /// switch: the console's stage vocabulary fails its lane when a stage is
    /// added in Rust rather than rendering as whichever arm a list omitted.
    pub const ALL: [Self; 6] = [
        Self::Parsed,
        Self::Attaching,
        Self::Importing,
        Self::Imported,
        Self::Failed,
        Self::Abandoned,
    ];

    const fn of(state: BatchState) -> Self {
        match state {
            BatchState::Parsed => Self::Parsed,
            BatchState::Attaching => Self::Attaching,
            BatchState::Importing => Self::Importing,
            BatchState::Imported => Self::Imported,
            BatchState::Failed => Self::Failed,
            BatchState::Abandoned => Self::Abandoned,
        }
    }
}

/// Where one row stands, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStateView {
    Parsed,
    Attached,
    Created,
    Published,
    Failed,
    Skipped,
}

impl RowStateView {
    pub const ALL: [Self; 6] = [
        Self::Parsed,
        Self::Attached,
        Self::Created,
        Self::Published,
        Self::Failed,
        Self::Skipped,
    ];

    const fn of(state: RowState) -> Self {
        match state {
            RowState::Parsed => Self::Parsed,
            RowState::Attached => Self::Attached,
            RowState::Created => Self::Created,
            RowState::Published => Self::Published,
            RowState::Failed => Self::Failed,
            RowState::Skipped => Self::Skipped,
        }
    }
}

/// What a row asked to become.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentView {
    Draft,
    Live,
}

impl IntentView {
    const fn of(intent: RowIntent) -> Self {
        match intent {
            RowIntent::Draft => Self::Draft,
            RowIntent::Live => Self::Live,
        }
    }
}

/// One batch as the listing and the batch page read it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchView {
    pub id: Uuid,
    pub source_name: String,
    pub state: BatchStateView,
    pub row_count: u32,
    pub live_count: u32,
    pub failed_count: u32,
    pub created_at: Timestamp,
    /// When an unfinished batch is swept and the files attached to it deleted.
    /// The console states this to the seller, which is why it travels rather
    /// than being recomputed from `created_at` and a constant the client would
    /// hold a second copy of.
    pub expires_at: Timestamp,
    pub settled_at: Option<Timestamp>,
    pub failure_detail: Option<String>,
}

impl ImportBatchView {
    fn of(record: ImportBatchRecord) -> Self {
        Self {
            id: record.id,
            source_name: record.source_name,
            state: BatchStateView::of(record.state),
            row_count: record.row_count,
            live_count: record.live_count,
            failed_count: record.failed_count,
            created_at: record.created_at,
            expires_at: record.expires_at,
            settled_at: record.settled_at,
            failure_detail: record.failure_detail,
        }
    }
}

/// One row of the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRowView {
    pub sheet: String,
    /// The seller's own spreadsheet row number, so the report cites what they
    /// see in the margin. Not an index: the off-by-one this feature would
    /// otherwise ship is held by a test on both sides.
    pub ordinal: u32,
    pub inventory: Option<InventoryId>,
    pub intent: IntentView,
    pub state: RowStateView,
    pub problems: Vec<Problem>,
    pub file_name: Option<String>,
    /// Whether the bytes for this row are held. Always false in this phase;
    /// the bind route is the next one.
    pub file_attached: bool,
    pub failure_detail: Option<String>,
}

impl ImportRowView {
    fn of(record: ImportBatchRowRecord) -> Self {
        Self {
            sheet: record.sheet,
            ordinal: record.ordinal,
            inventory: record.inventory,
            intent: IntentView::of(record.intent),
            state: RowStateView::of(record.state),
            // A document that will not deserialise is rendered as no problems
            // rather than as a fault: the column's own CHECK holds it to an
            // array, so this is unreachable, and a report that fails to load
            // is worse than one row of it reading empty.
            problems: serde_json::from_value(record.problems).unwrap_or_default(),
            file_name: record.file_name,
            file_attached: record.file.is_some(),
            failure_detail: record.failure_detail,
        }
    }
}

/// Something worth saying that does not refuse a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// A label the sheet names that this organisation does not hold yet. It
    /// will be created; the count is what makes a typo visible, because a
    /// one-row label beside a three-hundred-row near-identical one is the
    /// signature.
    NewLabel { name: String, rows: u32 },
    /// A live row on a marketplace this organisation holds no connection for.
    ///
    /// A warning rather than a refusal at parse time, because the seller may
    /// connect before committing. It becomes a refusal at commit, where a job
    /// minted for a marketplace with no connection would sit forever.
    NoConnection { marketplace: Marketplace, rows: u32 },
}

/// The listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportsView {
    pub imports: Vec<ImportBatchView>,
    /// The open batch, where one exists. Answered beside the listing because
    /// the Import page's card is enabled or disabled on exactly this, and
    /// making the client search the list for an open state would put the
    /// index's own predicate in a second place.
    pub open: Option<Uuid>,
}

/// One batch with its report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchDetailView {
    #[serde(flatten)]
    pub batch: ImportBatchView,
    pub rows: Vec<ImportRowView>,
    pub warnings: Vec<Warning>,
}

/// What an upload answered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedBatchView {
    #[serde(flatten)]
    pub detail: ImportBatchDetailView,
    /// Whether this upload created the batch, or found the idempotency key
    /// already used. A re-posted upload reports `false`, which is how a client
    /// tells a retry from a first delivery.
    pub created: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UploadParams {
    /// The uploaded file's own name.
    ///
    /// A parameter rather than a header, because the body is bytes and a
    /// `.csv` has no other way to say which tab it is: a comma-separated file
    /// carries one tab and no tab name, so the filename is the name. It is
    /// also what the listing shows, which is how a seller tells two batches
    /// apart.
    pub name: String,
}

// ---------------------------------------------------------------- handlers

/// The generated template workbook.
///
/// Named without a date, unlike the catalogue export beside it, and the
/// difference is what the two files are: an export is a snapshot of a
/// catalogue at an instant and two of them are different documents, while a
/// template is a blank form. A second download of the form replaces the first
/// in a downloads folder rather than accumulating beside it, which is what a
/// seller wants from a form and not from a snapshot.
pub(crate) async fn template(
    State(state): State<AppState>,
    _context: OrgContext,
) -> Result<impl IntoResponse, APIError> {
    let bytes = self::template::workbook()
        .map_err(|error| state.internal(&format!("the import template did not write: {error}")))?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_owned(),
            ),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"teachouse-import-template.xlsx\"".to_owned(),
            ),
        ],
        bytes,
    ))
}

/// Parses and validates an upload, and writes it as a batch that has created
/// nothing.
pub(crate) async fn upload(
    State(state): State<AppState>,
    context: OrgContext,
    RequestKey(key): RequestKey,
    Query(params): Query<UploadParams>,
    body: Bytes,
) -> Result<(StatusCode, Json<UploadedBatchView>), APIError> {
    let source_name = params.name.trim();
    if source_name.is_empty() {
        return Err(validation(
            "the upload states the name of the file it carries",
        ));
    }
    if body.is_empty() {
        return Err(validation("the upload carried no bytes"));
    }

    let batches = ImportBatchRepo::new(state.pool.clone());
    // Read before parsing rather than relying on the index alone: parsing a
    // five-hundred-row workbook to then refuse it for a reason known before
    // the first cell was read is work the seller waits through for nothing.
    // The index is still the arbiter, and the write below answers the same way.
    //
    // The open batch this key already named is the exception, and it is the
    // ordinary case rather than an edge one: a double-clicked submit and a
    // retried request both send the key again, and answering those with "you
    // already have one open" would refuse the seller their own import.
    if let Some(open) = batches
        .open(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        if open.id != key {
            return Err(already_open(&open));
        }
        let detail = detail_of(&state, context.org, open).await?;
        return Ok((
            StatusCode::OK,
            Json(UploadedBatchView {
                detail,
                created: false,
            }),
        ));
    }

    // Off the async worker, because both halves are CPU-bound over bytes a
    // stranger chose and neither yields. The bounds in `parse` make the work
    // finite; they do not make it short, and a five-hundred-row workbook parsed
    // on the thread serving the request holds that thread for the whole of it.
    // `spawn_supervised_blocking` puts it on the pool tokio keeps for exactly
    // this; the bare `spawn_blocking` is banned because a dropped handle
    // swallows a panic, and that wrapper is the crate's answer to it.
    let name = source_name.to_owned();
    let bytes = body.clone();
    let reported = spawn_supervised_blocking(move || {
        let grids = parse::grids(&name, &bytes)?;
        report::report(&grids)
    })
    .await
    .map_err(|error| state.internal(&format!("the spreadsheet parse did not run: {error}")))?
    .map_err(|refusal| malformed(&state, &refusal))?;

    let created_at = (state.wall)();
    // The two documents are serialised into owned vectors first and borrowed
    // from second, because the repository takes them by reference: building
    // them inside the mapping would hand it a reference to a temporary.
    let documents: Vec<(serde_json::Value, serde_json::Value)> = reported
        .rows
        .iter()
        .map(documents_of)
        .collect::<Result<_, _>>()?;
    let rows: Vec<NewImportBatchRow<'_>> = reported
        .rows
        .iter()
        .zip(documents.iter())
        .map(|(row, (draft, problems))| NewImportBatchRow {
            sheet: &row.sheet,
            ordinal: row.ordinal,
            inventory: row.inventory,
            intent: row.intent,
            draft,
            problems,
            file_name: row.file_name.as_deref(),
        })
        .collect();
    let written = batches
        .create(
            context.org,
            &NewImportBatch {
                id: key,
                source_name,
                created_at,
                expires_at: expiry_of(created_at),
                rows: &rows,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;

    let (record, created) = match written {
        BatchWrite::Saved(record) => (record, true),
        BatchWrite::Replay(record) => (record, false),
        BatchWrite::AlreadyOpen(open) => return Err(already_open(&open)),
    };
    let detail = detail_of(&state, context.org, record).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(UploadedBatchView { detail, created })))
}

/// One parsed row's two documents, in the shape the columns hold them.
///
/// Serialised here rather than in the repository, because the draft's shape is
/// the create form's and this crate owns that shape; `tam-storage` guarantees
/// the tenancy and the ceilings and takes the document as opaque, which is
/// migration 0056's own division.
fn documents_of(row: &ParsedRow) -> Result<(serde_json::Value, serde_json::Value), APIError> {
    let draft = serde_json::to_value(&row.draft).map_err(|error| {
        validation(&format!(
            "row {} of {} could not be recorded: {error}",
            row.ordinal, row.sheet
        ))
    })?;
    let problems = serde_json::to_value(&row.problems).map_err(|error| {
        validation(&format!(
            "the refusals for row {} of {} could not be recorded: {error}",
            row.ordinal, row.sheet
        ))
    })?;
    Ok((draft, problems))
}

fn already_open(open: &ImportBatchRecord) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "you already have an import open; finish it or abandon it before starting another",
        )
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({
            "open_batch": open.id,
            "source_name": open.source_name,
        })),
    )
}

pub(crate) async fn list(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ImportsView>, APIError> {
    let held = ImportBatchRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // Found in the page rather than by a second statement, which is sound
    // because at most one batch is open and no batch can be created while one
    // is: the open batch is therefore always the newest, and the listing is
    // newest first.
    let open = held
        .iter()
        .find(|record| record.state.is_open())
        .map(|record| record.id);
    Ok(Json(ImportsView {
        imports: held.into_iter().map(ImportBatchView::of).collect(),
        open,
    }))
}

pub(crate) async fn view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, batch)): Path<(String, String)>,
) -> Result<Json<ImportBatchDetailView>, APIError> {
    let batch = parse_id(&batch)?;
    let record = ImportBatchRepo::new(state.pool.clone())
        .get(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(detail_of(&state, context.org, record).await?))
}

/// Settles an open batch at the seller's own request.
pub(crate) async fn abandon(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, batch)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let batch = parse_id(&batch)?;
    let batches = ImportBatchRepo::new(state.pool.clone());
    let held = batches
        .get(context.org, batch)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    if !held.state.is_open() {
        // Already settled, by the seller, by a commit or by the sweep. A
        // second abandon is what a double-clicked button sends, and answering
        // 204 rather than a refusal is what makes it one action.
        return Ok(StatusCode::NO_CONTENT);
    }
    let _settled = batches
        .abandon(
            context.org,
            batch,
            (state.wall)(),
            "abandoned by the seller",
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// One batch with its rows and the warnings its rows raise.
///
/// The warnings are computed here rather than stored with the parse, because
/// both are answers about the state of the world now: a label the sheet would
/// newly create may have been created since, and a marketplace with no
/// connection at parse time may have one by the time the seller commits.
async fn detail_of(
    state: &AppState,
    org: tam_types::OrgId,
    record: ImportBatchRecord,
) -> Result<ImportBatchDetailView, APIError> {
    let batch = record.id;
    let mut rows = ImportBatchRepo::new(state.pool.clone())
        .rows(org, batch)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    rows.sort_by_key(|row| (tab_position(&row.sheet), row.ordinal));
    let warnings = warnings_of(state, org, &rows).await?;
    Ok(ImportBatchDetailView {
        batch: ImportBatchView::of(record),
        rows: rows.into_iter().map(ImportRowView::of).collect(),
        warnings,
    })
}

/// Where one tab sits in the workbook, so the report reads in the order the
/// seller filled it rather than in whatever order the sheet names sort in.
///
/// The repository answers in a byte-collation order, which is deterministic and
/// is not the workbook's; a tab name the workbook does not write sorts after
/// every one it does, which is where a row off a renamed tab belongs.
fn tab_position(sheet: &str) -> usize {
    sheet::TABS
        .iter()
        .position(|tab| tab.title == sheet)
        .unwrap_or(sheet::TABS.len())
}

async fn warnings_of(
    state: &AppState,
    org: tam_types::OrgId,
    rows: &[ImportBatchRowRecord],
) -> Result<Vec<Warning>, APIError> {
    let mut warnings = Vec::new();

    let held: Vec<String> = LabelRepo::new(state.pool.clone())
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .map(|record| record.name.to_lowercase())
        .collect();
    let mut counted: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for row in rows {
        for label in &row.labels {
            if held.contains(&label.to_lowercase()) {
                continue;
            }
            *counted.entry(label.clone()).or_insert(0) += 1;
        }
    }
    let mut new_labels: Vec<(String, u32)> = counted.into_iter().collect();
    new_labels.sort_by(|(left_name, left), (right_name, right)| {
        right.cmp(left).then_with(|| left_name.cmp(right_name))
    });
    warnings.extend(
        new_labels
            .into_iter()
            .map(|(name, rows)| Warning::NewLabel { name, rows }),
    );

    // A list rather than a map: `Marketplace` is not ordered, there are three
    // of them, and giving the type an ordering to satisfy a counter here would
    // be a trait implementation earned by nothing.
    let mut live: Vec<(Marketplace, u32)> = Vec::new();
    for row in rows {
        if row.intent != RowIntent::Live {
            continue;
        }
        let Some(inventory) = row.inventory else {
            continue;
        };
        let marketplace = inventory.marketplace();
        match live.iter_mut().find(|(held, _)| *held == marketplace) {
            Some((_, counted)) => *counted = counted.saturating_add(1),
            None => live.push((marketplace, 1)),
        }
    }
    if !live.is_empty() {
        let connected: Vec<Marketplace> = ConnectionRepo::new(state.pool.clone())
            .list(org, (state.wall)())
            .await
            .map_err(|error| storage_fault(state, &error))?
            .into_iter()
            .map(|row| row.marketplace)
            .collect();
        warnings.extend(
            live.into_iter()
                .filter(|(marketplace, _)| !connected.contains(marketplace))
                .map(|(marketplace, rows)| Warning::NoConnection { marketplace, rows }),
        );
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::{expiry_of, BatchStateView, IntentView, RowStateView, Warning};
    use tam_storage::{BatchState, RowIntent, RowState};
    use tam_types::{Marketplace, Timestamp};

    /// The wire vocabulary and the stored vocabulary are the same closed set.
    ///
    /// The `of` conversions are exhaustive matches, so a state added in
    /// `tam-storage` fails to compile here; these hold that the two sets are
    /// also the same size, which a compile error cannot say.
    #[test]
    fn every_stored_state_has_exactly_one_wire_state() {
        assert_eq!(
            BatchState::ALL.len(),
            BatchStateView::ALL.len(),
            "a batch state added in storage is a state the console must render"
        );
        assert_eq!(
            RowState::ALL.len(),
            RowStateView::ALL.len(),
            "a row state added in storage is a state the console must render"
        );
        for state in BatchState::ALL {
            let _crossed = BatchStateView::of(state);
        }
        for state in RowState::ALL {
            let _crossed = RowStateView::of(state);
        }
        for intent in [RowIntent::Draft, RowIntent::Live] {
            let _crossed = IntentView::of(intent);
        }
    }

    /// A batch's deadline is the founder's own figure past its creation, and
    /// the arithmetic is the one a seller reads as a date.
    #[test]
    fn the_deadline_is_the_expiry_window_past_creation() {
        const DAY: i64 = 24 * 60 * 60 * 1_000;
        let created = Timestamp(1_700_000_000_000);
        assert_eq!(
            expiry_of(created).0 - created.0,
            tam_limits::import::BATCH_EXPIRY_DAYS * DAY,
            "the stored deadline is the constant's own number of days"
        );
    }

    /// The two warnings serialise under a tag the console switches on, so a
    /// third one added in Rust is a case the client's exhaustive switch fails
    /// on rather than one it renders as whichever arm it happened to omit.
    #[test]
    fn a_warning_names_its_own_kind_on_the_wire() {
        let rendered = serde_json::to_value(Warning::NoConnection {
            marketplace: Marketplace::Tes,
            rows: 3,
        });
        let Ok(rendered) = rendered else {
            panic!("a warning serialises");
        };
        assert_eq!(
            rendered.get("kind").and_then(serde_json::Value::as_str),
            Some("no_connection"),
            "the tag is what the console switches on"
        );
    }
}
