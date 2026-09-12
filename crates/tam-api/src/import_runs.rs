//! One import, as the console drives it: a run, a selection, a review and a
//! commit.
//!
//! The shape both sources now share. A spreadsheet batch has always paused
//! between the parse and the commit — migration 0058's own header says nothing
//! there creates anything — and a marketplace import used to create a product
//! the moment a page landed, which left no instant at which a duplicate could
//! be put to the seller. A run gives the marketplace source the same pause,
//! and the commit here is the same chunked loop `import_batch::commit` runs:
//! twenty-five at a time, each chunk a point the seller can close the tab at.
//!
//! Nothing is drafted anywhere. A run names no target, so `import_one` mints
//! no mapping and runs no outbound projection: the outcome is resources in the
//! catalogue and the seller decides afterwards where they go.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_engine_driver::import::ObservedResource;
use tam_import::{import_one, AppliedResource, HeldFile, ImportRun, ImportedFile};
use tam_storage::{
    job_request_key, BlobRepo, DuplicateRepo, EventScope, FingerprintRepo, FingerprintWrite,
    ImportRunHead, ImportRunItemRecord, ImportRunRepo, JobOrigin, JobRepo, LabelRepo, MatchLayer,
    NewImportRun, NewJob, NewVerdict, RunCounts, RunItemState, RunKind, RunOpening, RunState,
    Selection, TextSketchColumns, IMPORT_LEG,
};
use tam_types::{
    Actor, ContentHash, FileBytes, FileKind, InventoryId, JobEventPayload, JobId, Marketplace,
    Money, Observation, OrgId, ProductId, ScanOutcome, Stamp, SystemComponent, Timestamp,
    TransportClass, Uuid,
};

use crate::entitlement::feature_refusal;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::{missing, storage_fault, validation};
use crate::matcher::{self, Side, SideFile, TextFacts};
use crate::{AppState, OrgContext};

/// How many items one commit chunk creates.
///
/// The spreadsheet commit's own figure, and deliberately the same number: a
/// row and an item are the same amount of work — a product insert, its files,
/// its labels and its fingerprint — and two pacing constants for one wait
/// would drift apart for no reason a seller could see.
const ITEMS_PER_CHUNK: i64 = 25;

/// The sketch format this build writes and compares.
///
/// Read from the fingerprint crate rather than restated, because a sketch is
/// never compared across versions and a second copy of the number is how two
/// halves of one comparison end up disagreeing about which format they are in.
fn sketch_version() -> i16 {
    i16::try_from(tam_fingerprint::FINGERPRINT_VERSION).unwrap_or(i16::MAX)
}

// ------------------------------------------------------------------- views

/// Which machinery produced a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRunKind {
    Marketplace,
    Spreadsheet,
}

impl ImportRunKind {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 2] = [Self::Marketplace, Self::Spreadsheet];
}

/// Where a run stands, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRunState {
    Reading,
    Reviewing,
    Committing,
    Complete,
    Failed,
    Abandoned,
}

impl ImportRunState {
    pub const ALL: [Self; 6] = [
        Self::Reading,
        Self::Reviewing,
        Self::Committing,
        Self::Complete,
        Self::Failed,
        Self::Abandoned,
    ];
}

/// Where one resource of a run stands, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRunItemState {
    Listed,
    Selected,
    Read,
    Matched,
    Review,
    Imported,
    Skipped,
    Failed,
}

impl ImportRunItemState {
    pub const ALL: [Self; 8] = [
        Self::Listed,
        Self::Selected,
        Self::Read,
        Self::Matched,
        Self::Review,
        Self::Imported,
        Self::Skipped,
        Self::Failed,
    ];
}

/// Which layer of the matcher carried a pair, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchLayerView {
    L1,
    L1b,
    L2,
    L3,
    L4,
    L5,
}

impl MatchLayerView {
    pub const ALL: [Self; 6] = [Self::L1, Self::L1b, Self::L2, Self::L3, Self::L4, Self::L5];

    #[must_use]
    pub const fn of(layer: MatchLayer) -> Self {
        match layer {
            MatchLayer::L1 => Self::L1,
            MatchLayer::L1b => Self::L1b,
            MatchLayer::L2 => Self::L2,
            MatchLayer::L3 => Self::L3,
            MatchLayer::L4 => Self::L4,
            MatchLayer::L5 => Self::L5,
        }
    }
}

/// Every state a run's rows are in, which is what the live bar draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCountsView {
    pub listed: u32,
    pub selected: u32,
    pub read: u32,
    pub matched: u32,
    pub review: u32,
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
}

impl RunCountsView {
    #[must_use]
    pub const fn of(counts: RunCounts) -> Self {
        Self {
            listed: counts.listed,
            selected: counts.selected,
            read: counts.read,
            matched: counts.matched,
            review: counts.review,
            imported: counts.imported,
            skipped: counts.skipped,
            failed: counts.failed,
        }
    }
}

/// One resource of a run, as the item list renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRunItemView {
    pub state: ImportRunItemState,
    pub locator: String,
    pub ordinal: u32,
    pub title: Option<String>,
    pub price: Option<Money>,
    /// Where the cover is served from, or `null` where the read produced none.
    /// Keyed on the ordinal rather than the locator, because a locator is a URL
    /// and would need escaping in a path segment.
    pub cover_url: Option<String>,
    pub product_id: Option<ProductId>,
    pub skip_reason: Option<String>,
    pub failure_detail: Option<String>,
}

/// One side of a duplicate question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSideView {
    /// The product, where this side is one. `null` where it is a resource the
    /// run has read and not yet created.
    pub product_id: Option<ProductId>,
    /// The locator, where this side is a run item rather than a product.
    pub run_locator: Option<String>,
    pub marketplace: Option<Marketplace>,
    pub title: String,
    pub price: Option<Money>,
    /// The grade band as a phrase, where the read derived one. A list rather
    /// than a string because the console renders chips, and one derived
    /// interval is one chip.
    pub grades: Vec<String>,
    pub cover_url: Option<String>,
}

/// One duplicate question, as a review card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPairView {
    pub product_lo: ProductId,
    pub product_hi: ProductId,
    /// One sentence of evidence, never a score.
    pub sentence: String,
    pub layer: MatchLayerView,
    pub lo: ReviewSideView,
    pub hi: ReviewSideView,
}

/// One run without its rows, which is what "Your imports" draws.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRunHeadView {
    pub id: Uuid,
    pub kind: ImportRunKind,
    pub source: Option<InventoryId>,
    pub batch_id: Option<Uuid>,
    pub state: ImportRunState,
    pub read_total: Option<u32>,
    pub counts: RunCountsView,
    pub created_at: Timestamp,
    pub settled_at: Option<Timestamp>,
}

/// One run with its rows and its open questions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRunView {
    pub id: Uuid,
    pub kind: ImportRunKind,
    pub source: Option<InventoryId>,
    pub batch_id: Option<Uuid>,
    pub state: ImportRunState,
    pub read_total: Option<u32>,
    pub counts: RunCountsView,
    pub items: Vec<ImportRunItemView>,
    pub review_pairs: Vec<ReviewPairView>,
    pub created_at: Timestamp,
    pub settled_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRunsView {
    pub runs: Vec<ImportRunHeadView>,
}

/// What one commit chunk did, and where the run stands after it.
///
/// `CommitAck`'s shape, field for field, with `run_state` where that one says
/// `batch_state`: the console runs the same loop over both, and a second shape
/// would make it two loops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCommitAck {
    pub applied: u32,
    pub skipped: u32,
    pub failed: u32,
    pub total: u32,
    pub remaining: u32,
    pub complete: bool,
    pub run_state: ImportRunState,
}

/// Which shop to read.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateRunBody {
    pub source: InventoryId,
}

/// What the seller ticked.
///
/// Two spellings rather than one, because "everything" and "these four" are
/// different intents: a tick list that happened to name every resource is
/// still a list, and a seller who asked for all of them has not named any.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SelectBody {
    All { all: bool },
    Named { locators: Vec<String> },
}

/// The locators the device is to describe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSelectionView {
    pub locators: Vec<String>,
}

// ---------------------------------------------------------------- handlers

/// Opens a marketplace import run.
pub(crate) async fn create_run(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateRunBody>,
) -> Result<(StatusCode, Json<ImportRunView>), APIError> {
    if !context.entitlement.caps.import_marketplace {
        return Err(feature_refusal(
            "import_marketplace",
            "Your plan does not include reading your shop. Upgrade to import from a \
             marketplace.",
        ));
    }
    // A shop a device reads, which is what a marketplace run is. An official
    // API marketplace is imported server-side and has no run to start from a
    // console button.
    if body.source.marketplace().transport_class() != TransportClass::SellerDevice {
        return Err(validation(
            "this marketplace publishes an official API, so its catalogue is read on our own \
             infrastructure rather than by a run started here",
        ));
    }
    let now = (state.wall)();
    // Every file a run imports is fetched back through the seller's own
    // connection (D27), so a shop with no linked connection has nothing an
    // import could keep. Refused here, before the device reads anything,
    // rather than as a failed item after the seller has chosen and waited.
    if crate::import::source_connection(&state, context.org, body.source, now)
        .await
        .is_err()
    {
        return Err(validation(
            "Connect this marketplace on the Marketplaces page first, then import from it.",
        ));
    }
    let runs = ImportRunRepo::new(state.pool.clone());
    let run = fresh_uuid();
    let anchor = anchor_job(&state, context.org, run, body.source, now).await?;
    let opening = runs
        .create(
            context.org,
            &NewImportRun {
                id: run,
                kind: RunKind::Marketplace,
                source: Some(body.source),
                batch_id: None,
                // Nothing is drafted anywhere, which is what "no target"
                // means; see the module header.
                target: None,
                anchor_job: anchor,
                created_at: now,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match opening {
        RunOpening::Opened => {
            let view = view_of(&state, context.org, run).await?;
            Ok((StatusCode::CREATED, Json(view)))
        }
        RunOpening::AlreadyOpen(open) => Err(already_open(open)),
    }
}

pub(crate) async fn list_runs(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ImportRunsView>, APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    let heads = repo
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let mut runs = Vec::with_capacity(heads.len());
    for head in heads {
        let counts = repo
            .counts(context.org, head.id)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        runs.push(head_view(&head, counts));
    }
    Ok(Json(ImportRunsView { runs }))
}

pub(crate) async fn run_view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
) -> Result<Json<ImportRunView>, APIError> {
    let run = parse_id(&run)?;
    Ok(Json(view_of(&state, context.org, run).await?))
}

/// Records the seller's tick list.
pub(crate) async fn select(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
    Json(body): Json<SelectBody>,
) -> Result<Json<ImportRunView>, APIError> {
    let run = parse_id(&run)?;
    let repo = ImportRunRepo::new(state.pool.clone());
    let head = head_or_missing(&state, context.org, run).await?;
    if head.state != RunState::Reading {
        return Err(validation(
            "this import has finished reading, so its selection is settled",
        ));
    }
    let selection = match &body {
        SelectBody::All { all } => {
            if !*all {
                return Err(validation(
                    "a selection is either every resource or a list of them; `all: false` names \
                     neither",
                ));
            }
            Selection::All
        }
        SelectBody::Named { locators } => Selection::Locators(locators),
    };
    repo.select(context.org, run, selection, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(view_of(&state, context.org, run).await?))
}

/// The locators the device is to describe.
pub(crate) async fn device_selection(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device, run)): Path<(String, String, String)>,
) -> Result<Json<RunSelectionView>, APIError> {
    let run = parse_id(&run)?;
    crate::import::admissible_device(&state, context.org, &device).await?;
    head_or_missing(&state, context.org, run).await?;
    let locators = ImportRunRepo::new(state.pool.clone())
        .selection(context.org, run)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(RunSelectionView { locators }))
}

/// Creates the next chunk of a run's matched items.
pub(crate) async fn commit(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
) -> Result<Json<RunCommitAck>, APIError> {
    let run = parse_id(&run)?;
    let head = head_or_missing(&state, context.org, run).await?;
    if !head.state.open() {
        return Err(validation(
            "this import has settled and creates nothing more",
        ));
    }
    let repo = ImportRunRepo::new(state.pool.clone());
    repo.set_state(context.org, run, RunState::Committing, None, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;

    let page = repo
        .commit_page(context.org, run, ITEMS_PER_CHUNK)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let mut applied = 0_u32;
    let mut failed = 0_u32;
    for item in &page {
        match commit_one(&state, context.org, &head, item).await {
            Ok(()) => applied = applied.saturating_add(1),
            Err(refusal) => {
                // A refusal the seller can act on is recorded against the item
                // and the chunk carries on; anything else is ours and fails
                // the request, leaving the item `matched` for the next chunk.
                // Marking an item permanently failed because the database was
                // briefly unreachable would cost the seller a resource over a
                // fault that has already passed. This is `commit.rs`'s own
                // rule, restated because the consequence is the same.
                if refusal.status_code().is_server_error() {
                    return Err(refusal);
                }
                repo.record_failed(
                    context.org,
                    run,
                    item.locator.as_str(),
                    &reason_of(&refusal),
                    (state.wall)(),
                )
                .await
                .map_err(|error| storage_fault(&state, &error))?;
                failed = failed.saturating_add(1);
            }
        }
    }

    let counts = repo
        .counts(context.org, run)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let outstanding = counts.outstanding();
    let run_state = if outstanding == 0 {
        repo.set_state(context.org, run, RunState::Complete, None, (state.wall)())
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        settled_event(&state, context.org, &head, RunState::Complete).await?;
        RunState::Complete
    } else {
        RunState::Committing
    };
    progress_event(&state, context.org, &head, counts).await?;
    Ok(Json(RunCommitAck {
        applied,
        skipped: counts.skipped,
        failed,
        total: counts
            .imported
            .saturating_add(counts.matched)
            .saturating_add(counts.review)
            .saturating_add(counts.read),
        remaining: outstanding,
        complete: outstanding == 0,
        run_state: state_view(run_state),
    }))
}

/// Settles an open run at the seller's own request.
pub(crate) async fn abandon(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let run = parse_id(&run)?;
    let head = head_or_missing(&state, context.org, run).await?;
    if !head.state.open() {
        return Err(validation("this import has already settled"));
    }
    ImportRunRepo::new(state.pool.clone())
        .set_state(
            context.org,
            run,
            RunState::Abandoned,
            Some("you stopped this import"),
            (state.wall)(),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    settled_event(&state, context.org, &head, RunState::Abandoned).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The cover one read produced, served to the browser that renders the card.
pub(crate) async fn item_cover(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run, ordinal)): Path<(String, String, String)>,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), APIError> {
    let run = parse_id(&run)?;
    let ordinal: u32 = ordinal
        .parse()
        .map_err(|_| missing("no such item of this import"))?;
    let record = ImportRunRepo::new(state.pool.clone())
        .get(context.org, run)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such import"))?;
    let hash = record
        .items
        .iter()
        .find(|item| item.ordinal == ordinal)
        .ok_or_else(|| missing("no such item of this import"))?
        .cover_hash
        .ok_or_else(|| missing("this resource produced no cover"))?;
    let bytes = cover_bytes(&state, context.org, hash).await?;
    crate::resources::image_answer(bytes)
}

// ----------------------------------------------------------------- shared

/// The run's head, or the same not-found another organisation's run gets.
pub(crate) async fn head_or_missing(
    state: &AppState,
    org: OrgId,
    run: Uuid,
) -> Result<ImportRunHead, APIError> {
    ImportRunRepo::new(state.pool.clone())
        .get(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .map(|record| record.head)
        .ok_or_else(|| missing("no such import"))
}

/// One run, whole.
pub(crate) async fn view_of(
    state: &AppState,
    org: OrgId,
    run: Uuid,
) -> Result<ImportRunView, APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    let record = repo
        .get(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such import"))?;
    let counts = repo
        .counts(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let head = head_view(&record.head, counts);
    let pairs = crate::duplicates::pairs_of(state, org, Some(run)).await?;
    Ok(ImportRunView {
        id: head.id,
        kind: head.kind,
        source: head.source,
        batch_id: head.batch_id,
        state: head.state,
        read_total: head.read_total,
        counts: head.counts,
        items: record
            .items
            .iter()
            .map(|item| item_view(run, item))
            .collect(),
        review_pairs: pairs,
        created_at: head.created_at,
        settled_at: head.settled_at,
    })
}

#[must_use]
pub(crate) fn head_view(head: &ImportRunHead, counts: RunCounts) -> ImportRunHeadView {
    ImportRunHeadView {
        id: head.id,
        kind: match head.kind {
            RunKind::Marketplace => ImportRunKind::Marketplace,
            RunKind::Spreadsheet => ImportRunKind::Spreadsheet,
        },
        source: head.source,
        batch_id: head.batch_id,
        state: state_view(head.state),
        read_total: head.read_total,
        counts: RunCountsView::of(counts),
        created_at: head.created_at,
        settled_at: head.settled_at,
    }
}

#[must_use]
pub(crate) const fn state_view(state: RunState) -> ImportRunState {
    match state {
        RunState::Reading => ImportRunState::Reading,
        RunState::Reviewing => ImportRunState::Reviewing,
        RunState::Committing => ImportRunState::Committing,
        RunState::Complete => ImportRunState::Complete,
        RunState::Failed => ImportRunState::Failed,
        RunState::Abandoned => ImportRunState::Abandoned,
    }
}

#[must_use]
pub(crate) const fn item_state_view(state: RunItemState) -> ImportRunItemState {
    match state {
        RunItemState::Listed => ImportRunItemState::Listed,
        RunItemState::Selected => ImportRunItemState::Selected,
        RunItemState::Read => ImportRunItemState::Read,
        RunItemState::Matched => ImportRunItemState::Matched,
        RunItemState::Review => ImportRunItemState::Review,
        RunItemState::Imported => ImportRunItemState::Imported,
        RunItemState::Skipped => ImportRunItemState::Skipped,
        RunItemState::Failed => ImportRunItemState::Failed,
    }
}

fn item_view(run: Uuid, item: &ImportRunItemRecord) -> ImportRunItemView {
    ImportRunItemView {
        state: item_state_view(item.state),
        locator: item.locator.clone(),
        ordinal: item.ordinal,
        title: item.title.clone(),
        price: item.price,
        cover_url: item.cover_hash.map(|_| item_cover_url(run, item.ordinal)),
        // Only an imported item names a product the seller can open. The
        // reserved identifier every row carries is not one: following it would
        // 404, because the product exists only once the commit has run.
        product_id: matches!(item.state, RunItemState::Imported).then_some(item.product),
        skip_reason: item.skip_reason.clone(),
        failure_detail: item.failure_detail.clone(),
    }
}

#[must_use]
pub(crate) fn item_cover_url(run: Uuid, ordinal: u32) -> String {
    format!("/v1/imports/runs/{}/items/{ordinal}/cover", uuid_text(run))
}

/// One page of a run, as the device posts it.
///
/// Four things a page can carry, and it may carry several: a failure, a list,
/// descriptions, refusals, and a completion. The order below is the order they
/// have to happen in — a terminal failure settles the run before anything else
/// is considered, because a device that stopped mid-pass sends nothing else and
/// the seller's own page is the record they read.
pub(crate) async fn run_page(
    state: &AppState,
    context: &OrgContext,
    device: &str,
    page: &tam_engine_driver::import::ImportPage,
    now: Timestamp,
) -> Result<(StatusCode, Json<crate::import::ImportAck>), APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    // Another organisation's run is missing rather than forbidden: the caller
    // learns nothing about whether the identifier exists, which is the posture
    // every other org-scoped read here takes.
    let head = head_or_missing(state, context.org, page.run).await?;

    if let Some(why) = page.failed.as_ref() {
        if head.state.open() {
            repo.set_state(
                context.org,
                page.run,
                RunState::Failed,
                Some(why.as_str()),
                now,
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
            settled_event(state, context.org, &head, RunState::Failed).await?;
        }
        let counts = repo
            .counts(context.org, page.run)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        return Ok((StatusCode::OK, Json(run_ack(counts, 0, 0, true))));
    }

    if !head.state.open() {
        return Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(
                "this import has already settled, and a settled import is not extended",
            )
            .kind(APIErrorKind::Validation),
        ));
    }

    // The list. `Some(vec![])` is a shop that holds nothing and `None` is an
    // ordinary page of descriptions, which is why the field is an option: an
    // empty vector on every page would reset the total to whatever the last
    // page happened to say.
    if let Some(listed) = page.listed.as_ref() {
        let rows: Vec<tam_storage::ListedRow> = listed
            .iter()
            .map(|row| tam_storage::ListedRow {
                locator: row.locator.as_str().to_owned(),
                title: row.title.clone(),
                price: listed_row_price(row, head.source),
            })
            .collect();
        repo.append_listed(context.org, page.run, &rows, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        let counts = repo
            .counts(context.org, page.run)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        listed_event(state, context.org, &head, counts.listed).await?;
    }

    let mut applied = 0_u32;
    for resource in &page.resources {
        record_and_match(
            state,
            context.org,
            &head,
            resource,
            Some(device),
            context.entitlement.caps.duplicate_review,
        )
        .await?;
        applied = applied.saturating_add(1);
    }

    let mut skipped = 0_u32;
    for skip in &page.skipped {
        repo.record_skipped(
            context.org,
            page.run,
            skip.locator.as_str(),
            skip.why.as_str(),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
        skipped = skipped.saturating_add(1);
    }

    let counts = repo
        .counts(context.org, page.run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    progress_event(state, context.org, &head, counts).await?;

    if page.complete {
        // A run with a question owed goes to the seller; one with none is
        // ready for the console to commit. Not committed here: the chunk loop
        // is the client's, so that a closed browser loses only the chunk in
        // flight.
        let next = if counts.review > 0 {
            RunState::Reviewing
        } else {
            RunState::Committing
        };
        repo.set_state(context.org, page.run, next, None, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    Ok((
        StatusCode::OK,
        Json(run_ack(counts, applied, skipped, page.complete)),
    ))
}

/// The list row's price, denominated by the source's own rule.
fn listed_row_price(
    row: &tam_engine_driver::import::ListedResource,
    source: Option<InventoryId>,
) -> Option<Money> {
    let (minor, currency, source) = (row.price_minor?, row.currency.as_ref()?, source?);
    let tam_types::CurrencyRule::Fixed(fixed) = source.currency_rule() else {
        return None;
    };
    if fixed.code() != currency {
        return None;
    }
    Money::new(minor, fixed).ok()
}

/// The page acknowledgement, in the shape the device already reads.
///
/// `create_job` is always absent: a run drafts nowhere, so there is no create
/// job for a completing page to mint. The field stays rather than forking the
/// type, because one device posts both kinds of page and a second ack shape
/// would be a second decoder for no gain.
fn run_ack(
    counts: RunCounts,
    applied: u32,
    skipped: u32,
    complete: bool,
) -> crate::import::ImportAck {
    crate::import::ImportAck {
        applied,
        skipped,
        described_total: counts
            .read
            .saturating_add(counts.matched)
            .saturating_add(counts.review)
            .saturating_add(counts.imported),
        create_job: None,
        complete,
    }
}

/// Stores one described resource on its item and runs the matcher over it.
///
/// The one place both sources meet. It is called per resource of a device page
/// and per row of a spreadsheet commit, so "what the matcher was given" cannot
/// differ between them.
#[expect(
    clippy::too_many_arguments,
    reason = "the state, the tenant, the run, the resource, its device and the plan's review capability; a struct over them would name this call and nothing else, and both callers pass all six"
)]
pub(crate) async fn record_and_match(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    resource: &ObservedResource,
    device: Option<&str>,
    reviews: bool,
) -> Result<RunItemState, APIError> {
    let now = (state.wall)();
    let repo = ImportRunRepo::new(state.pool.clone());

    // The cover, stored as a held blob directly rather than through the ingest
    // pipeline, which would derive a cover from the cover.
    let cover_hash = match resource.cover_png.as_ref() {
        Some(cover) => Some(store_cover(state, org, cover.bytes(), now).await?),
        None => None,
    };

    // The description as posted, minus the cover bytes: those are a blob named
    // by `cover_hash`, and a page's worth of base64 in a jsonb column would be
    // the one copy of a file this design exists to avoid.
    let mut stored = resource.clone();
    stored.cover_png = None;
    let observed = serde_json::to_value(&stored).map_err(|error| {
        state.internal(&format!(
            "an observed resource would not serialise: {error}"
        ))
    })?;

    let source = head
        .source
        .ok_or_else(|| state.internal("a marketplace run names no source"))?;
    let price = listed_price(resource, source);
    let fresh = repo
        .record_read(
            org,
            head.id,
            &tam_storage::ReadItem {
                locator: resource.locator.as_str(),
                observed: &observed,
                title: resource.listing.title.as_str(),
                price,
                cover_hash,
                device,
                product: None,
            },
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let item = repo
        .item(org, head.id, resource.locator.as_str())
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| state.internal("an item just written reads back missing"))?;
    if !fresh {
        return Ok(item.state);
    }

    let subject = side_of(resource, source);
    let bands = subject
        .text
        .map(|text| tam_fingerprint::simhash_bands(text.simhash))
        .map(|bands| bands.map(|band| i16::from_ne_bytes(band.to_ne_bytes())));
    let found = matcher::match_one(
        state,
        org,
        sketch_version(),
        &subject,
        item.product,
        Some(source.marketplace()),
        bands,
        reviews,
    )
    .await?;

    let duplicates = DuplicateRepo::new(state.pool.clone());
    for raised in found.merged.iter().chain(found.asked.iter()) {
        let (lo, hi) = tam_storage::ordered_pair(item.product, raised.other);
        let kept = matches!(raised.verdict, tam_storage::Verdict::Same).then_some(raised.other);
        duplicates
            .raise(
                org,
                &NewVerdict {
                    lo,
                    hi,
                    verdict: raised.verdict,
                    decided_by: raised.decided_by,
                    winning_layer: raised.layer,
                    log_odds: raised.log_odds,
                    fingerprint_version: sketch_version(),
                    run: Some(head.id),
                    kept,
                    raised_at: now,
                    decided_at: matches!(raised.verdict, tam_storage::Verdict::Same).then_some(now),
                    reversible_until: matches!(raised.verdict, tam_storage::Verdict::Same)
                        .then_some(Timestamp(now.0.saturating_add(tam_storage::REVERSIBLE_MS))),
                    evidence: &raised.evidence,
                },
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    // A pair the system decided on its own is a merge, and the merge is the
    // skip: the kept product already holds this resource, so creating a second
    // one is exactly what the verdict says not to do. The kept product gains
    // the source's label, because it now carries a listing from that shop.
    if let Some(merged) = found.merged.first() {
        LabelRepo::new(state.pool.clone())
            .attach_system_label(org, merged.other, source.marketplace(), now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        let title = held_title(state, org, merged.other).await?;
        repo.record_skipped(
            org,
            head.id,
            resource.locator.as_str(),
            &format!("same as {title}"),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
        item_settled_event(state, org, head, resource.locator.as_str(), "skipped").await?;
        return Ok(RunItemState::Skipped);
    }

    let next = if found.needs_review() {
        RunItemState::Review
    } else {
        RunItemState::Matched
    };
    repo.record_verdict(org, head.id, resource.locator.as_str(), next)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(next)
}

/// The run that reviews one spreadsheet batch, found or opened.
///
/// A run for the spreadsheet source too, and the reason is that the duplicate
/// review has to be one thing rather than two: the same pair table, the same
/// verdicts, the same never-ask-twice rule, and the same card. A batch that
/// held its own review would be a second implementation of the hardest part of
/// this feature.
pub(crate) async fn run_for_batch(
    state: &AppState,
    org: OrgId,
    batch: Uuid,
) -> Result<ImportRunHead, APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    if let Some(head) = repo
        .by_batch(org, batch)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        return Ok(head);
    }
    let now = (state.wall)();
    let run = fresh_uuid();
    // A spreadsheet run names no shop, so its anchor job is filed against the
    // inventory the catalogue itself is: there is no marketplace read to
    // attribute it to, and the job carries no items either way.
    let anchor = anchor_job(state, org, run, InventoryId::Tes, now).await?;
    let opening = repo
        .create(
            org,
            &NewImportRun {
                id: run,
                kind: RunKind::Spreadsheet,
                source: None,
                batch_id: Some(batch),
                target: None,
                anchor_job: anchor,
                created_at: now,
            },
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    match opening {
        RunOpening::Opened => head_or_missing(state, org, run).await,
        // Another import is open. A batch commit is not refused for it: the
        // batch has its own one-open-per-org index and the seller reached this
        // by pressing a button on a batch that was already theirs, so the
        // honest answer is the refusal the run index gives, naming what is
        // open.
        RunOpening::AlreadyOpen(open) => Err(already_open(open)),
    }
}

/// Puts one claimed spreadsheet row through the matcher.
///
/// Called after the claim rather than before it, because the claim is what
/// reserves the row's product identifier and the pair has to be keyed on the
/// identifier the product will actually take: a question answered about an
/// identifier nothing took would be asked again on the next import, which is
/// the one failure a stored `different` exists to prevent.
#[expect(
    clippy::too_many_arguments,
    reason = "the state, the tenant, the run, the row's handle, the row and the plan's review capability; the same six as the marketplace path, deliberately"
)]
pub(crate) async fn match_spreadsheet_row(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    locator: &str,
    row: &tam_storage::ClaimedRow,
    reviews: bool,
) -> Result<RunItemState, APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    if let Some(held) = repo
        .item(org, head.id, locator)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        // Already described and decided. A resumed commit reads the verdict
        // rather than asking the matcher a second question about one row.
        if !matches!(held.state, RunItemState::Listed | RunItemState::Selected) {
            return Ok(held.state);
        }
    }
    let now = (state.wall)();
    let title = row
        .draft
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    // The write's own answer is not read: a row reaching this twice is a
    // resumed commit, and the state read above is what decided whether to ask
    // the matcher again.
    let _fresh = repo
        .record_read(
            org,
            head.id,
            &tam_storage::ReadItem {
                locator,
                // The row's own draft, which is the create form's shape: this
                // is the document the commit is about to build a product from,
                // so it is the honest record of what was matched.
                observed: &row.draft,
                title: &title,
                price: None,
                cover_hash: row.cover.as_ref().map(|cover| cover.hash),
                device: None,
                product: Some(row.product),
            },
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;

    let subject = Side {
        title: title.clone(),
        title_norm: tam_fingerprint::normalise_title(&title),
        // A spreadsheet row carries no device-computed sketch: the bytes came
        // to us through the upload route, and the fingerprint stage over a
        // server-held file is not part of this phase. So the row is matched on
        // its digest and its title, which is exactly what it has.
        price: None,
        text: None,
        page_count: None,
        cover_phash: None,
        subjects: Vec::new(),
        grade_low: None,
        grade_high: None,
        files: row
            .file
            .as_ref()
            .map(|file| {
                vec![SideFile {
                    digest: file.hash,
                    byte_len: u64::try_from(file.byte_len).unwrap_or(0),
                    name: None,
                }]
            })
            .unwrap_or_default(),
    };
    let found = matcher::match_one(
        state,
        org,
        sketch_version(),
        &subject,
        row.product,
        // A spreadsheet row is on no marketplace, so the cross-marketplace
        // rule admits every candidate: there is no same-shop pair to exclude.
        None,
        None,
        reviews,
    )
    .await?;

    let duplicates = DuplicateRepo::new(state.pool.clone());
    for raised in found.merged.iter().chain(found.asked.iter()) {
        let (lo, hi) = tam_storage::ordered_pair(row.product, raised.other);
        let merged = matches!(raised.verdict, tam_storage::Verdict::Same);
        duplicates
            .raise(
                org,
                &NewVerdict {
                    lo,
                    hi,
                    verdict: raised.verdict,
                    decided_by: raised.decided_by,
                    winning_layer: raised.layer,
                    log_odds: raised.log_odds,
                    fingerprint_version: sketch_version(),
                    run: Some(head.id),
                    kept: merged.then_some(raised.other),
                    raised_at: now,
                    decided_at: merged.then_some(now),
                    reversible_until: merged
                        .then_some(Timestamp(now.0.saturating_add(tam_storage::REVERSIBLE_MS))),
                    evidence: &raised.evidence,
                },
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    if let Some(merged) = found.merged.first() {
        let title = held_title(state, org, merged.other).await?;
        repo.record_skipped(org, head.id, locator, &format!("same as {title}"), now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        return Ok(RunItemState::Skipped);
    }
    let next = if found.needs_review() {
        RunItemState::Review
    } else {
        RunItemState::Matched
    };
    repo.record_verdict(org, head.id, locator, next)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(next)
}

/// One spreadsheet row's handle inside the run, which is how migration 0070's
/// own header spells it.
#[must_use]
pub(crate) fn row_locator(sheet: &str, ordinal: u32) -> String {
    format!("{sheet}#{ordinal}")
}

/// Settles a run whose batch has finished.
pub(crate) async fn settle_run(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    run_state: RunState,
) -> Result<(), APIError> {
    if !head.state.open() {
        return Ok(());
    }
    ImportRunRepo::new(state.pool.clone())
        .set_state(org, head.id, run_state, None, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?;
    settled_event(state, org, head, run_state).await
}

/// Creates one matched item: the product, its label and its sketch.
async fn commit_one(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    item: &ImportRunItemRecord,
) -> Result<(), APIError> {
    let now = (state.wall)();
    let observed = item
        .observed
        .clone()
        .ok_or_else(|| state.internal("a matched item holds no description"))?;
    let resource: ObservedResource = serde_json::from_value(observed).map_err(|error| {
        state.internal(&format!("a stored description would not decode: {error}"))
    })?;
    let source = head
        .source
        .ok_or_else(|| state.internal("a marketplace run names no source"))?;

    let run = ImportRun {
        pool: state.pool.clone(),
        org,
        source,
        // Nothing is drafted anywhere.
        target: None,
        now,
    };

    let payload = match resource.file.as_ref() {
        Some(file) => {
            let connection = crate::import::source_connection(state, org, source, now).await?;
            vec![ImportedFile {
                kind: file.kind,
                bytes: FileBytes::Sourced {
                    marketplace: source.marketplace(),
                    connection,
                    resource: resource.locator.as_str().to_owned(),
                    entry: file.entry.as_ref().map(|entry| entry.as_str().to_owned()),
                    payload_file_name: file.payload_file_name.as_str().to_owned(),
                    payload_content_type: file.payload_content_type.as_str().to_owned(),
                    observed: Observation {
                        device: item.device.clone().unwrap_or_default(),
                        hash: file.hash,
                        byte_len: file.byte_len,
                        scan: file.scan.clone(),
                        observed_at: now,
                    },
                },
            }]
        }
        // A TPT read names no file, because `tpt.download_resource_bundle` is
        // uncaptured. D32 and migration 0061 make that a product with no
        // payload rather than a refusal, and a run drafts nowhere so no
        // mapping exists for the trigger to fire on.
        None => Vec::new(),
    };

    // The cover's length, read back from the blob it was stored as. One
    // decrypt of one 512x384 PNG per item, which is cheaper than carrying a
    // length in the stored document and having two places that could disagree
    // about it.
    let cover = match item.cover_hash {
        Some(hash) => {
            let bytes = cover_bytes(state, org, hash).await?;
            Some(HeldFile {
                kind: FileKind::Image,
                hash,
                byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                scan: ScanOutcome::Clean { at: now },
            })
        }
        None => None,
    };

    let applied = AppliedResource {
        // A locator is not inherently numeric -- a Tes resource is a URL -- so
        // an unparseable one reports zero rather than refusing an import over
        // a display value.
        resource: resource.locator.as_str().parse().unwrap_or(0),
        product: item.product,
        listing: resource.listing.clone(),
        payload,
        cover,
    };
    let report = import_one(&run, &applied)
        .await
        .map_err(|error| match error {
            tam_import::ImportError::Storage(error) => storage_fault(state, &error),
            // Every one of these is something about the seller's own listing:
            // a resource with nothing to sell, a price that will not
            // denominate, a currency nobody has measured. Each is theirs to
            // act on, which is what makes them validation rather than faults.
            refused @ (tam_import::ImportError::NoPayload
            | tam_import::ImportError::Price(_)
            | tam_import::ImportError::CurrencyUnknown { .. }) => validation(&refused.to_string()),
            // A run names no target, so nothing lowers an intent and nothing
            // asks for one.
            impossible @ (tam_import::ImportError::Lowering(_)
            | tam_import::ImportError::NoTarget) => {
                state.internal(&format!("the import answered {impossible}"))
            }
        })?;

    LabelRepo::new(state.pool.clone())
        .attach_system_label(org, report.product, source.marketplace(), now)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    if let Some(fingerprint) = resource.fingerprint.as_ref() {
        write_fingerprint(
            state,
            org,
            report.product,
            fingerprint,
            item.device.as_deref(),
            now,
        )
        .await?;
    }

    ImportRunRepo::new(state.pool.clone())
        .record_imported(org, head.id, item.locator.as_str(), report.product, now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    item_settled_event(state, org, head, item.locator.as_str(), "imported").await?;
    Ok(())
}

/// Stores one product's sketch, in the columns the matcher blocks on.
#[expect(
    clippy::too_many_arguments,
    reason = "the sketch is a device's assertion, so the write carries the machine and the instant beside the value; folding them into a struct would hide exactly the distinction 0052 exists to keep"
)]
pub(crate) async fn write_fingerprint(
    state: &AppState,
    org: OrgId,
    product: ProductId,
    fingerprint: &tam_fingerprint::Fingerprint,
    device: Option<&str>,
    now: Timestamp,
) -> Result<(), APIError> {
    let text = fingerprint.text.as_ref().map(|text| TextSketchColumns {
        simhash: i64::from_ne_bytes(text.simhash.to_ne_bytes()),
        minhash: tam_fingerprint::minhash_bytes(&text.minhash).to_vec(),
        shingle_count: i32::try_from(text.shingle_count).unwrap_or(i32::MAX),
        extracted_chars: i32::try_from(text.extracted_chars).unwrap_or(i32::MAX),
        bands: tam_fingerprint::simhash_bands(text.simhash)
            .map(|band| i16::from_ne_bytes(band.to_ne_bytes())),
    });
    FingerprintRepo::new(state.pool.clone())
        .put(
            org,
            &FingerprintWrite {
                product,
                version: i16::try_from(fingerprint.version).unwrap_or(i16::MAX),
                text,
                page_count: fingerprint
                    .page_count
                    .map(|count| i32::try_from(count).unwrap_or(i32::MAX)),
                cover_phash: fingerprint
                    .cover_phash
                    .map(|phash| i64::from_ne_bytes(phash.to_ne_bytes())),
                title_norm: fingerprint.title_norm.as_str(),
                observed_by_device: device,
                observed_at: now,
            },
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(())
}

/// One described resource, in the matcher's own shape.
#[must_use]
pub(crate) fn side_of(resource: &ObservedResource, source: InventoryId) -> Side {
    let fingerprint = resource.fingerprint.as_ref();
    Side {
        title: resource.listing.title.clone(),
        title_norm: fingerprint.map_or_else(
            || tam_fingerprint::normalise_title(&resource.listing.title),
            |fingerprint| fingerprint.title_norm.clone(),
        ),
        price: listed_price(resource, source),
        text: fingerprint.and_then(|fingerprint| {
            fingerprint.text.as_ref().map(|text| TextFacts {
                simhash: text.simhash,
                minhash: text.minhash,
                extracted_chars: text.extracted_chars,
            })
        }),
        page_count: fingerprint.and_then(|fingerprint| fingerprint.page_count),
        cover_phash: fingerprint.and_then(|fingerprint| fingerprint.cover_phash),
        // The read's own taxonomy is native and untagged until `import_one`
        // maps it, so the corroboration layer has nothing to compare on the
        // subject axis for an item that is not yet a product. Stated as an
        // absence rather than guessed at: a wrong subject would earn half a
        // point for nothing.
        subjects: Vec::new(),
        grade_low: None,
        grade_high: None,
        files: resource
            .file
            .as_ref()
            .map(|file| {
                vec![SideFile {
                    digest: file.hash,
                    byte_len: file.byte_len,
                    name: Some(file.payload_file_name.as_str().to_owned()),
                }]
            })
            .unwrap_or_default(),
    }
}

/// What the read said the resource costs, denominated by the source's own
/// rule.
///
/// The same resolution `tam_import::resolve_price` performs, and refusing
/// rather than guessing for the same reason: a seller-scoped inventory renders
/// a bare symbol, and reading that as a currency would put an unmeasured
/// denomination inside `Money` where nothing downstream can tell it from a
/// measured one. Here the answer to an unresolvable price is an absent one,
/// because this value is a number on a card rather than a price anything is
/// published at -- the commit resolves it again through `import_one`, which
/// refuses properly.
fn listed_price(resource: &ObservedResource, source: InventoryId) -> Option<Money> {
    let tam_types::ImportedPrice::Paid {
        minor_units,
        denomination,
    } = &resource.listing.price
    else {
        return None;
    };
    let tam_types::CurrencyRule::Fixed(currency) = source.currency_rule() else {
        return None;
    };
    if currency.code() != denomination {
        return None;
    }
    Money::new(*minor_units, currency).ok()
}

async fn held_title(state: &AppState, org: OrgId, product: ProductId) -> Result<String, APIError> {
    let held = tam_storage::ProductRepo::new(state.pool.clone())
        .get(org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(held.map_or_else(
        || "a resource you already have".to_owned(),
        |record| record.product.title.0.clone(),
    ))
}

async fn store_cover(
    state: &AppState,
    org: OrgId,
    bytes: &[u8],
    now: Timestamp,
) -> Result<ContentHash, APIError> {
    let blobs = state.blobs.clone().ok_or_else(blob_store_unavailable)?;
    BlobRepo::new(
        state.pool.clone(),
        tam_pipeline::store::LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    )
    .put(org, bytes, now)
    .await
    .map_err(|error| state.internal(&error.to_string()))
}

async fn cover_bytes(state: &AppState, org: OrgId, hash: ContentHash) -> Result<Vec<u8>, APIError> {
    let blobs = state.blobs.clone().ok_or_else(blob_store_unavailable)?;
    BlobRepo::new(
        state.pool.clone(),
        tam_pipeline::store::LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    )
    .get(org, hash)
    .await
    .map_err(|error| state.internal(&error.to_string()))
}

pub(crate) fn blob_store_unavailable() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new(
            "this deployment holds no key-encryption key or object-store root, so no cover \
             could be stored or read",
        )
        .code(APIErrorCode::BlobStoreUnavailable)
        .kind(APIErrorKind::Internal),
    )
}

// ----------------------------------------------------------------- events

/// The itemless job a run's events hang from, found or created.
///
/// Keyed on the run, so every page of one import reaches the same job rather
/// than minting one each. It carries no items, and so cannot become a publish:
/// a `queued` item is what the lease scan claims and it has none.
pub(crate) async fn anchor_job(
    state: &AppState,
    org: OrgId,
    run: Uuid,
    source: InventoryId,
    now: Timestamp,
) -> Result<JobId, APIError> {
    let created = JobRepo::new(state.pool.clone())
        .create_with_request_key(
            org,
            JobOrigin {
                request_key: job_request_key(run, IMPORT_LEG),
                run: None,
            },
            &NewJob {
                job: JobId(fresh_uuid()),
                inventory: source,
                stamp: Stamp {
                    at: now,
                    actor: Actor::System(SystemComponent::Import),
                },
            },
            &[],
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(created.job)
}

pub(crate) async fn listed_event(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    listed: u32,
) -> Result<(), APIError> {
    emit(
        state,
        org,
        head,
        &JobEventPayload::ImportRunListed {
            run: head.id,
            listed,
        },
    )
    .await
}

pub(crate) async fn progress_event(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    counts: RunCounts,
) -> Result<(), APIError> {
    emit(
        state,
        org,
        head,
        &JobEventPayload::ImportRunProgress {
            run: head.id,
            listed: counts.listed,
            read: counts.read,
            matched: counts.matched,
            review: counts.review,
            imported: counts.imported,
            skipped: counts.skipped,
            failed: counts.failed,
        },
    )
    .await
}

pub(crate) async fn item_settled_event(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    locator: &str,
    item_state: &str,
) -> Result<(), APIError> {
    emit(
        state,
        org,
        head,
        &JobEventPayload::ImportRunItemSettled {
            run: head.id,
            locator: locator.to_owned(),
            state: item_state.to_owned(),
        },
    )
    .await
}

pub(crate) async fn settled_event(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    run_state: RunState,
) -> Result<(), APIError> {
    emit(
        state,
        org,
        head,
        &JobEventPayload::ImportRunSettled {
            run: head.id,
            state: run_state.as_str().to_owned(),
        },
    )
    .await
}

async fn emit(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    payload: &JobEventPayload,
) -> Result<(), APIError> {
    JobRepo::new(state.pool.clone())
        .record_event(
            &EventScope {
                org,
                job: head.anchor_job,
                item: None,
            },
            payload,
            Stamp::system(SystemComponent::Import, (state.wall)()),
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(())
}

// --------------------------------------------------------------- refusals

/// One import at a time, and this is which one.
///
/// The open run's identifier travels on the entry, because the console's next
/// move is to show it: a seller who pressed the button twice wants the import
/// they started, not a list to find it in.
fn already_open(run: Uuid) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "you already have an import in progress. Finish or stop it before starting another.",
        )
        .code(APIErrorCode::ImportRunOpen)
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({ "run": uuid_text(run) })),
    )
}

fn reason_of(refusal: &APIError) -> String {
    refusal
        .errors
        .first()
        .map_or_else(|| refusal.to_string(), |entry| entry.message.clone())
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw)
        .map(|id| Uuid(*id.as_bytes()))
        .map_err(|_| missing("no such import"))
}

fn uuid_text(id: Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}
