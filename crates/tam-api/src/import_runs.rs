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
//! no target mapping and runs no outbound projection: the outcome is resources
//! in the catalogue, each bound to the listing it was read from so the
//! catalogue says where it already is, and the seller decides afterwards
//! where else it goes.

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_engine_driver::import::{
    ImportClaim, ImportLease, ImportProgressReport, ImportRenewal, ImportStop, ImportStopAck,
    ObservedResource,
};
use tam_import::{AppliedResource, HeldFile, ImportRun, ImportedFile};
use tam_storage::{
    job_request_key, BatchRunOpening, BlobRepo, ClaimOutcome, EventScope, FenceOutcome,
    FingerprintWrite, ImportReasonCode, ImportRunHead, ImportRunItemRecord, ImportRunRepo,
    ImportStage, ItemOrder, ItemPageFilter, JobOrigin, JobRepo, MatchLayer, Minted, NewImportRun,
    NewJob, NewVerdict, ProgressReport, ReceiptOutcome, RunCounts, RunHistoryFilter, RunItemState,
    RunKind, RunOpening, RunState, Selection, TextSketchColumns, IMPORT_LEG, ITEMS_LISTED_MAX,
    RUNS_LISTED_MAX,
};
use tam_types::{
    Actor, ContentHash, FileBytes, FileKind, InventoryId, JobEventPayload, JobId, Marketplace,
    Money, Observation, OrgId, ProductId, ScanOutcome, Stamp, SystemComponent, Timestamp,
    TransportClass, Uuid,
};

use crate::entitlement::feature_refusal;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::{missing, storage_fault, validation, DeletionStatusView, JobDeletionView};
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

/// The displayed lifecycle of a run, decided by the server.
///
/// One vocabulary combining the run's own state with who holds it and how
/// fresh their contact is, because the alternative is what this replaces: a
/// browser inferring liveness from an event stream it happens to be holding
/// open, which cannot tell a working device from a phone that has been off
/// since Tuesday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportRunStage {
    /// Nothing has claimed it yet.
    Waiting,
    Discovering,
    Selecting,
    Reading,
    Reviewing,
    Committing,
    /// It was claimed, and its owner stopped answering. Resumable.
    Interrupted,
    Failed,
    Completed,
    Abandoned,
}

impl ImportRunStage {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 10] = [
        Self::Waiting,
        Self::Discovering,
        Self::Selecting,
        Self::Reading,
        Self::Reviewing,
        Self::Committing,
        Self::Interrupted,
        Self::Failed,
        Self::Completed,
        Self::Abandoned,
    ];
}

/// Why an import stopped, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportReasonCodeView {
    MissingSession,
    NotPermitted,
    UnsupportedSource,
    EnumerationFailed,
    DescriptionFailed,
    SubmissionFailed,
    ActivationExpired,
    LeaseExpired,
    Stopped,
    ClientUpdateRequired,
}

impl ImportReasonCodeView {
    pub const ALL: [Self; 10] = [
        Self::MissingSession,
        Self::NotPermitted,
        Self::UnsupportedSource,
        Self::EnumerationFailed,
        Self::DescriptionFailed,
        Self::SubmissionFailed,
        Self::ActivationExpired,
        Self::LeaseExpired,
        Self::Stopped,
        Self::ClientUpdateRequired,
    ];

    #[must_use]
    pub const fn of(code: ImportReasonCode) -> Self {
        match code {
            ImportReasonCode::MissingSession => Self::MissingSession,
            ImportReasonCode::NotPermitted => Self::NotPermitted,
            ImportReasonCode::UnsupportedSource => Self::UnsupportedSource,
            ImportReasonCode::EnumerationFailed => Self::EnumerationFailed,
            ImportReasonCode::DescriptionFailed => Self::DescriptionFailed,
            ImportReasonCode::SubmissionFailed => Self::SubmissionFailed,
            ImportReasonCode::ActivationExpired => Self::ActivationExpired,
            ImportReasonCode::LeaseExpired => Self::LeaseExpired,
            ImportReasonCode::Stopped => Self::Stopped,
            ImportReasonCode::ClientUpdateRequired => Self::ClientUpdateRequired,
        }
    }

    #[must_use]
    pub const fn into_storage(self) -> ImportReasonCode {
        match self {
            Self::MissingSession => ImportReasonCode::MissingSession,
            Self::NotPermitted => ImportReasonCode::NotPermitted,
            Self::UnsupportedSource => ImportReasonCode::UnsupportedSource,
            Self::EnumerationFailed => ImportReasonCode::EnumerationFailed,
            Self::DescriptionFailed => ImportReasonCode::DescriptionFailed,
            Self::SubmissionFailed => ImportReasonCode::SubmissionFailed,
            Self::ActivationExpired => ImportReasonCode::ActivationExpired,
            Self::LeaseExpired => ImportReasonCode::LeaseExpired,
            Self::Stopped => ImportReasonCode::Stopped,
            Self::ClientUpdateRequired => ImportReasonCode::ClientUpdateRequired,
        }
    }
}

/// Who is executing a run, and what the server knows about it.
///
/// Every field is read from a stored fact. `stage` is the one the console
/// renders: it is the run's lifecycle and its freshness combined here, on the
/// server, so "live" never means "a stream is open" and an interrupted run
/// says so.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportExecutionView {
    pub owner_device: Option<String>,
    pub attempt: u64,
    /// Unix milliseconds, which is every other timestamp on this wire.
    pub lease_expires_at: Option<Timestamp>,
    pub last_contact_at: Option<Timestamp>,
    /// When something last actually moved, which is not the same as when the
    /// owner last spoke.
    pub last_progress_at: Option<Timestamp>,
    pub stage: ImportRunStage,
    pub reason_code: Option<ImportReasonCodeView>,
    pub reason: Option<String>,
    pub discovered: u32,
    /// What the owner reported it had worked through, failures included.
    pub processed: u32,
    /// How many items were actually described, counted from the stored
    /// descriptions rather than reported: `observed` is written by the read
    /// and by nothing else. Listed and unselected rows have no description,
    /// a row skipped at selection was never read, and a row whose source read
    /// failed holds a reason instead — so this excludes all of those and
    /// includes every row that was described, whatever it has since become
    /// (matched, in review, imported, merged away, or failed at the commit).
    ///
    /// A device taking a run over has no journal of its own, and this is what
    /// it resumes from. `processed` cannot serve: it counts failures too.
    pub described: u32,
    pub enumeration_complete: bool,
    /// The frozen denominator of the reading stage.
    pub selected_total: Option<u32>,
    /// Whether a catalogue commit has been authorised: the seller's own
    /// confirmation, or the scheduler's approved rule. Finishing the
    /// descriptions is neither.
    pub commit_authorised: bool,
}

impl ImportExecutionView {
    /// `described` is read separately because it is a count over the run's
    /// items rather than a fact on the run's own row; every caller has it to
    /// hand or reads it beside the head.
    #[must_use]
    pub fn of(head: &ImportRunHead, described: u32) -> Self {
        let execution = &head.execution;
        Self {
            owner_device: execution.owner_device.clone(),
            attempt: execution.attempt,
            lease_expires_at: execution.lease_expires_at,
            last_contact_at: execution.last_contact_at,
            last_progress_at: execution.last_progress_at,
            stage: stage_of(head),
            reason_code: execution.reason_code.map(ImportReasonCodeView::of),
            reason: execution.reason.clone(),
            discovered: execution.discovered,
            processed: execution.processed,
            described,
            enumeration_complete: execution.enumeration_complete,
            selected_total: execution.selected_total,
            commit_authorised: execution.commit_authorised(head.scheduled),
        }
    }
}

/// The stage the console draws, decided here.
///
/// A settled run says what it settled as. An open one is described in this
/// order, and the order is the substance.
///
/// A device that reported an interruption said something true about the read,
/// and nothing derived overrides it. Next comes the selection: a closed
/// enumeration with no frozen selection is the seller's own step, and no
/// machine owes anything while they choose — the device's keeper ends with
/// its worker, so the lease lapses and the owner clears exactly when the run
/// is waiting for a person. Reading that as `waiting` or `interrupted` is the
/// defect this ordering exists to prevent, and the maintenance sweep exempts
/// the same runs for the same reason.
///
/// Only past the selection does a stage need a live machine. An ownerless run
/// that has done nothing is waiting for a device; one that has discovered or
/// described anything and then lost its owner stopped part way, which is an
/// interruption rather than a fresh wait. A live owner's own reported stage is
/// preferred over anything derived, because it is the only party that knows.
#[must_use]
fn stage_of(head: &ImportRunHead) -> ImportRunStage {
    let execution = &head.execution;
    match head.state {
        RunState::Complete => return ImportRunStage::Completed,
        RunState::Failed => return ImportRunStage::Failed,
        RunState::Abandoned => return ImportRunStage::Abandoned,
        RunState::Committing => return ImportRunStage::Committing,
        RunState::Reviewing => return ImportRunStage::Reviewing,
        RunState::Reading => {}
    }
    if matches!(execution.reported_stage, Some(ImportStage::Interrupted)) {
        return ImportRunStage::Interrupted;
    }
    // The seller's step, from the run's own authoritative facts rather than
    // from who is holding it.
    if execution.enumeration_complete && execution.selected_total.is_none() {
        return ImportRunStage::Selecting;
    }
    if execution.owner_device.is_none() {
        return if execution.enumeration_complete
            || execution.discovered > 0
            || execution.processed > 0
        {
            ImportRunStage::Interrupted
        } else {
            ImportRunStage::Waiting
        };
    }
    if !execution.lease_live {
        return ImportRunStage::Interrupted;
    }
    match execution.reported_stage {
        Some(ImportStage::Discovering) => ImportRunStage::Discovering,
        Some(ImportStage::Selecting) => ImportRunStage::Selecting,
        Some(ImportStage::Reading) => ImportRunStage::Reading,
        // Handled above, and named here rather than wildcarded so a stage
        // added to the device's vocabulary fails this match instead of
        // silently becoming a read.
        Some(ImportStage::Interrupted) => ImportRunStage::Interrupted,
        // Claimed, live, and nothing reported yet — or a failure reported
        // while the run is still open. Described by what the run itself has
        // reached: an open enumeration is discovery, and a frozen selection
        // is the read. The closed-enumeration-without-selection case is
        // already answered above.
        Some(ImportStage::Failed) | None => {
            if execution.enumeration_complete {
                ImportRunStage::Reading
            } else {
                ImportRunStage::Discovering
            }
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
    /// The settled run a seller asked to retry, where this run is that
    /// retry. The terminal run keeps everything it recorded; this is the
    /// lineage beside it.
    pub retry_of: Option<Uuid>,
    /// Where a Delete this import is still working through has got to, and
    /// nothing for an import nobody deleted. A listed run is never
    /// `deleted`: the tombstone is filtered in the storage query.
    pub deletion_status: Option<DeletionStatusView>,
    pub execution: ImportExecutionView,
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
    /// One page of the run's resources, under whatever filter and order was
    /// asked for. `counts`, `read_total` and `execution` above stay whole-run
    /// facts: a page narrows what is listed and never what is counted.
    pub items: Vec<ImportRunItemView>,
    /// How many resources the item filter matches across the whole run.
    pub items_total: u32,
    pub items_offset: u32,
    pub items_limit: u32,
    pub review_pairs: Vec<ReviewPairView>,
    pub created_at: Timestamp,
    pub settled_at: Option<Timestamp>,
    pub retry_of: Option<Uuid>,
    /// See [`ImportRunHeadView::deletion_status`].
    pub deletion_status: Option<DeletionStatusView>,
    pub execution: ImportExecutionView,
}

/// One page of the run history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRunsView {
    pub runs: Vec<ImportRunHeadView>,
    /// How many runs the filter matches across the whole history, which is
    /// what the pager states. Never inferred from the page's own length.
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
}

/// What confirming a run answered.
///
/// The confirmation is a durable request rather than a chunk of work: the
/// server's own drain creates the resources, so the answer is "accepted, and
/// here is the run" rather than "here is what one chunk did". That is what
/// removes the browser-owned commit loop — a seller who navigates away
/// mid-commit loses nothing, because nothing was being driven by their tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfirmAck {
    pub accepted: bool,
    pub run: ImportRunView,
}

/// Which shop to read, and which press of the button this is.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateRunBody {
    pub source: InventoryId,
    /// The client's own key for this intent, retained across its retries and
    /// across a native handoff. Required: without it a lost answer mints a
    /// second import, which is the defect it exists to close.
    pub start_key: Uuid,
    /// The settled run this start retries, where the seller pressed Retry on
    /// one. Validated against a terminal run of this organisation on the same
    /// shop; never sent by an ordinary start.
    #[serde(default)]
    pub retry_of: Option<Uuid>,
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

/// Opens a marketplace import run, or answers with the one this intent
/// already opened.
///
/// The start key is consulted before the open-run fence, and the order
/// matters: a client whose answer was lost retries with the same key, and a
/// key already spent has to reach the run it made even after that run has
/// settled or expired. Checking the fence first would mint a second run for
/// a completed one.
pub(crate) async fn create_run(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<CreateRunBody>,
) -> Result<(StatusCode, Json<ImportRunView>), APIError> {
    if !context.entitlement.caps.import_marketplace {
        return Err(feature_refusal(
            "import_marketplace",
            "Upgrade your plan to import from a marketplace.",
        ));
    }
    // A shop a device reads, which is what a marketplace run is. An official
    // API marketplace is imported server-side and has no run to start from a
    // console button.
    if body.source.marketplace().transport_class() != TransportClass::SellerDevice {
        return Err(validation(
            "Teachouse reads this marketplace for you, so you don't need to start an import here.",
        ));
    }
    // The seller's explicit permission, read before anything is minted: a
    // run is seller-device work from its first page.
    crate::consent::require_grant(&state, context.org, body.source.marketplace()).await?;
    let now = (state.wall)();
    let runs = ImportRunRepo::new(state.pool.clone());

    // The replay, before anything else is decided. A settled run is a
    // perfectly good answer here: the seller's client is asking what became
    // of the import it started, not for a new one.
    if let Some(held) = runs
        .start_key_run(context.org, body.start_key)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        // A key whose import the seller deleted is refused rather than
        // answered with the tombstone or spent a second time. The binding is
        // kept precisely so this answer exists: without it the replay would
        // find no key, open a second import of the same shop, and the seller
        // would have deleted an import that came straight back.
        if held.deleted {
            return Err(start_key_deleted());
        }
        if held.source != body.source || held.retry_of != body.retry_of {
            return Err(start_key_spent(held.source));
        }
        let view = view_of(&state, context.org, held.run).await?;
        return Ok((StatusCode::OK, Json(view)));
    }

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

    if let Some(parent) = body.retry_of {
        retryable(&state, context.org, parent, body.source).await?;
    }

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
                // A seller pressed Import. Only the pass sets this; see
                // migration 0071.
                scheduled: false,
                start_key: Some(body.start_key),
                retry_of: body.retry_of,
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match opening {
        RunOpening::Opened => {
            state.telemetry.capture(
                context.org,
                "import_run_started",
                serde_json::json!({
                    "kind": "marketplace",
                    "source_marketplace": body.source.marketplace(),
                    "is_first_ever": first_ever(&state, context.org).await,
                }),
            );
            let view = view_of(&state, context.org, run).await?;
            Ok((StatusCode::CREATED, Json(view)))
        }
        // Pressing the button twice on one shop is one import, and the answer
        // is that import rather than a refusal: the console's next move is to
        // show it.
        RunOpening::AlreadyOpen(open) => {
            let view = view_of(&state, context.org, open).await?;
            Ok((StatusCode::OK, Json(view)))
        }
        RunOpening::KeySpent { source } => Err(start_key_spent(source)),
    }
}

/// Whether the run just opened is this organisation's first ever.
///
/// Counted rather than joined later, because it is the property that makes
/// the activation funnel — signup, first import, first move — one query. The
/// filter asks for no rows, so this is a count on the tenant's own index; a
/// count that fails answers `false`, since an analytics property is never
/// worth failing an import the seller just started.
async fn first_ever(state: &AppState, org: OrgId) -> bool {
    let counted = ImportRunRepo::new(state.pool.clone())
        .history(
            org,
            &RunHistoryFilter {
                limit: 0,
                ..RunHistoryFilter::default()
            },
        )
        .await;
    matches!(counted, Ok(page) if page.total <= 1)
}

/// The run a retry may name: this organisation's, on the same shop, and
/// settled.
///
/// A terminal run is not reopened — it keeps everything it recorded — so the
/// retry is a new run that names it. An open parent would mean two live runs
/// for one shop, which the per-source fence refuses anyway; saying so here is
/// the answer the seller can act on.
async fn retryable(
    state: &AppState,
    org: OrgId,
    parent: Uuid,
    source: InventoryId,
) -> Result<(), APIError> {
    let head = head_or_missing(state, org, parent).await?;
    if head.source != Some(source) {
        return Err(validation(
            "Try again with the same shop as the first import.",
        ));
    }
    if head.state.open() {
        return Err(validation(
            "That import hasn't finished yet, so there is nothing to try again.",
        ));
    }
    Ok(())
}

/// How many runs the history answers when the caller names no size.
///
/// Ten, which is what the console shows: a seller reads their history to find
/// one import, and a page they can take in at a glance beats a page that
/// holds everything they have ever done.
const RUNS_PER_PAGE: i64 = 10;

/// How many resources of one run a page answers when the caller names no
/// size.
const ITEMS_PER_PAGE: i64 = 25;

/// Which page of the history, under which filter, in which direction.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunPageParams {
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// One run state, or `open` for every run still expecting work.
    #[serde(default)]
    pub state: Option<String>,
    /// One inventory, or `spreadsheet` for the runs that name no shop.
    #[serde(default)]
    pub source: Option<String>,
    /// `newest` (the default) or `oldest`.
    #[serde(default)]
    pub order: Option<String>,
}

/// Which page of one run's resources, under which filter, in which order.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemPageParams {
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    /// A plain substring of the title, matched over the whole run.
    #[serde(default)]
    pub q: Option<String>,
    /// One item state.
    #[serde(default)]
    pub state: Option<String>,
    /// `title` (the default) or `listed`.
    #[serde(default)]
    pub order: Option<String>,
}

/// What a page of a list is, once the request's own words have been read.
///
/// An unrecognised filter is refused rather than ignored, which is the rule
/// `jobs::parse_page` already states: a value we silently drop answers the
/// whole list, and a seller who asked for their failed imports and got all of
/// them has been told something untrue about what they are looking at.
fn window(offset: Option<i64>, limit: Option<i64>, fallback: i64, ceiling: i64) -> (i64, i64) {
    let offset = offset.unwrap_or(0).max(0);
    let limit = limit.unwrap_or(fallback).clamp(1, ceiling);
    (offset, limit)
}

fn run_history_filter(params: &RunPageParams) -> Result<RunHistoryFilter, APIError> {
    let (offset, limit) = window(params.offset, params.limit, RUNS_PER_PAGE, RUNS_LISTED_MAX);
    let mut filter = RunHistoryFilter {
        offset,
        limit,
        ..RunHistoryFilter::default()
    };
    if let Some(raw) = params.state.as_deref().filter(|raw| !raw.is_empty()) {
        match raw {
            "open" => filter.open_only = true,
            "reading" => filter.state = Some(RunState::Reading),
            "reviewing" => filter.state = Some(RunState::Reviewing),
            "committing" => filter.state = Some(RunState::Committing),
            "complete" => filter.state = Some(RunState::Complete),
            "failed" => filter.state = Some(RunState::Failed),
            "abandoned" => filter.state = Some(RunState::Abandoned),
            _ => return Err(validation("Choose a status from the list.")),
        }
    }
    if let Some(raw) = params.source.as_deref().filter(|raw| !raw.is_empty()) {
        if raw == "spreadsheet" {
            filter.spreadsheet_only = true;
        } else {
            filter.source = Some(
                crate::vocabulary::parse_inventory(raw)
                    .ok_or_else(|| validation("Choose a marketplace from the list."))?,
            );
        }
    }
    match params.order.as_deref().filter(|raw| !raw.is_empty()) {
        None | Some("newest") => {}
        Some("oldest") => filter.oldest = true,
        Some(_) => return Err(validation("Sort by newest or oldest.")),
    }
    Ok(filter)
}

fn item_window(params: &ItemPageParams) -> Result<ItemWindow, APIError> {
    let (offset, limit) = window(
        params.offset,
        params.limit,
        ITEMS_PER_PAGE,
        ITEMS_LISTED_MAX,
    );
    let state = match params.state.as_deref().filter(|raw| !raw.is_empty()) {
        None => None,
        Some("listed") => Some(RunItemState::Listed),
        Some("selected") => Some(RunItemState::Selected),
        Some("read") => Some(RunItemState::Read),
        Some("matched") => Some(RunItemState::Matched),
        Some("review") => Some(RunItemState::Review),
        Some("imported") => Some(RunItemState::Imported),
        Some("skipped") => Some(RunItemState::Skipped),
        Some("failed") => Some(RunItemState::Failed),
        Some(_) => return Err(validation("Choose a status from the list.")),
    };
    let order = match params.order.as_deref().filter(|raw| !raw.is_empty()) {
        None | Some("title") => ItemOrder::Title,
        Some("listed") => ItemOrder::Listed,
        Some(_) => return Err(validation("Sort by title or in listing order.")),
    };
    Ok(ItemWindow {
        state,
        search: params
            .q
            .as_deref()
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .map(ToOwned::to_owned),
        order,
        offset,
        limit,
    })
}

pub(crate) async fn list_runs(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<RunPageParams>,
) -> Result<Json<ImportRunsView>, APIError> {
    let filter = run_history_filter(&params)?;
    let repo = ImportRunRepo::new(state.pool.clone());
    let page = repo
        .history(context.org, &filter)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let mut runs = Vec::with_capacity(page.runs.len());
    for head in page.runs {
        let counts = repo
            .counts(context.org, head.id)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        let described = repo
            .described(context.org, head.id)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        runs.push(head_view(&head, counts, described));
    }
    Ok(Json(ImportRunsView {
        runs,
        total: u32::try_from(page.total).unwrap_or(u32::MAX),
        offset: u32::try_from(filter.offset).unwrap_or(u32::MAX),
        limit: u32::try_from(filter.limit).unwrap_or(u32::MAX),
    }))
}

pub(crate) async fn run_view(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
    Query(params): Query<ItemPageParams>,
) -> Result<Json<ImportRunView>, APIError> {
    let run = parse_id(&run)?;
    let window = item_window(&params)?;
    Ok(Json(
        view_of_windowed(&state, context.org, run, &window).await?,
    ))
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
    let selection = match &body {
        SelectBody::All { all } => {
            if !*all {
                return Err(validation(
                    "Choose all resources, or pick the ones you want.",
                ));
            }
            Selection::All
        }
        SelectBody::Named { locators } => Selection::Locators(locators),
    };
    // The run's own guard is inside the write's transaction rather than read
    // here first: a selection that passed a check and then landed after the
    // seller stopped the import would move a terminal run's rows, and one
    // taken over a half-walked shop would freeze a prefix of their catalogue
    // as the whole of it.
    match repo
        .select(context.org, run, selection, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        tam_storage::SelectionOutcome::Taken { .. } => {}
        tam_storage::SelectionOutcome::Discovering => {
            return Err(validation(
                "This import is still reading your shop, so the list isn't complete yet.",
            ))
        }
        tam_storage::SelectionOutcome::Settled(state) => return Err(run_settled(state)),
        tam_storage::SelectionOutcome::Missing => {
            return Err(missing("We can't find that import."))
        }
    }
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

/// One import this organisation has open, for a device to pick up.
///
/// A list rather than a singleton, which is the cutover: one open run per
/// source means a seller reading Tes can start TPT, and a device that can
/// only serve one of them picks the one it has a session for and leaves the
/// other alone rather than failing somebody else's run.
///
/// The two bools are the device's own two decisions, and the run's state
/// vocabulary stays the console's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRunView {
    pub run: Uuid,
    pub source: InventoryId,
    /// Whether the shop's own list is complete. False means carry on
    /// enumerating; true means never list again.
    pub listed: bool,
    /// Whether the seller -- or, on a scheduled run, the server -- has ticked
    /// what to describe.
    pub selected: bool,
    /// Whether the scheduler opened this rather than a seller. A device with
    /// no session skips a scheduled run quietly; a seller's own start is
    /// theirs to report a refusal against.
    pub scheduled: bool,
    /// Which device holds it, where one does. `null` is unclaimed work.
    pub owner_device: Option<String>,
}

pub(crate) async fn device_open_runs(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
) -> Result<Json<Vec<OpenRunView>>, APIError> {
    crate::import::admissible_device(&state, context.org, &device).await?;
    let repo = ImportRunRepo::new(state.pool.clone());
    let heads = repo
        .open_runs(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let mut open = Vec::with_capacity(heads.len());
    for head in &heads {
        // A run past `reading` is a review or a commit, and neither is device
        // work; a spreadsheet run names no shop to read. Both are omitted
        // rather than offered as work a device would not know what to do with.
        let (RunState::Reading, Some(source)) = (head.state, head.source) else {
            continue;
        };
        open.push(OpenRunView {
            run: head.id,
            source,
            // The device's own bit rather than the presence of a total: a
            // listed page may be partial now, so only the enumeration being
            // closed means the shop has been walked.
            listed: head.execution.enumeration_complete,
            // The frozen denominator, not a live count of rows still waiting
            // to be described. A run whose last description landed before its
            // completion did has no `selected` rows left, and reading that as
            // "not selected yet" left a resumed device with no phase it was
            // owed: it would not describe, because the list was empty, and it
            // would not report completion, because it believed the seller had
            // not chosen yet. The frozen total says the choice was made,
            // whatever has since been described.
            selected: head.execution.selected_total.is_some(),
            scheduled: head.scheduled,
            owner_device: head.execution.owner_device.clone(),
        });
    }
    Ok(Json(open))
}

/// Claims a run for one device, raising the fence.
///
/// Before any local preflight, deliberately: the device claims first and only
/// then looks for its own marketplace session, so "this phone is not signed
/// in to Tes" is a durable report against a run this device owns rather than
/// a refusal nobody recorded. Revocation and the organisation's own plan
/// still gate it.
pub(crate) async fn claim(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device, run)): Path<(String, String, String)>,
    Json(body): Json<ImportClaim>,
) -> Result<Json<ImportLease>, APIError> {
    let run = parse_id(&run)?;
    if !context.entitlement.caps.import_marketplace {
        return Err(feature_refusal(
            "import_marketplace",
            "Upgrade your plan to import from a marketplace.",
        ));
    }
    crate::import::admissible_device(&state, context.org, &device).await?;
    head_or_missing(&state, context.org, run).await?;
    let outcome = ImportRunRepo::new(state.pool.clone())
        .claim(context.org, run, &device, body.takeover)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    match outcome {
        ClaimOutcome::Granted(lease) => {
            // A claim changes who the console is watching and what stage it
            // shows, so the observers hear about it: this is the moment a
            // waiting run becomes a run a named device is executing.
            let head = head_or_missing(&state, context.org, run).await?;
            let counts = ImportRunRepo::new(state.pool.clone())
                .counts(context.org, run)
                .await
                .map_err(|error| storage_fault(&state, &error))?;
            progress_event(&state, context.org, &head, counts).await?;
            Ok(Json(wire_lease(lease)))
        }
        ClaimOutcome::HeldBy {
            device: owner,
            lease_expires_at,
        } => Err(held_by(&owner, lease_expires_at)),
        ClaimOutcome::Settled(state) => Err(run_settled(state)),
    }
}

/// Extends the owner's hold under its own fence.
pub(crate) async fn renew(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device, run)): Path<(String, String, String)>,
    Json(body): Json<ImportRenewal>,
) -> Result<Json<ImportLease>, APIError> {
    let run = parse_id(&run)?;
    crate::import::admissible_device(&state, context.org, &device).await?;
    let repo = ImportRunRepo::new(state.pool.clone());
    if let Some(lease) = repo
        .renew(context.org, run, &device, body.attempt)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        return Ok(Json(wire_lease(lease)));
    }
    Err(refused_fence(&state, context.org, run, &device, body.attempt).await?)
}

/// Records what the owner says it is doing.
///
/// No marketplace session is consulted and none is required: the whole point
/// of this route is that a device with no session for the shop can say so,
/// durably, against a run it owns.
pub(crate) async fn progress(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device, run)): Path<(String, String, String)>,
    Json(body): Json<ImportProgressReport>,
) -> Result<Json<ImportExecutionView>, APIError> {
    let run = parse_id(&run)?;
    crate::import::admissible_device(&state, context.org, &device).await?;
    let repo = ImportRunRepo::new(state.pool.clone());
    let reason = body
        .reason
        .as_ref()
        .map(|reason| reason.as_str().to_owned());
    let moved = repo
        .report_progress(
            context.org,
            run,
            &device,
            &ProgressReport {
                attempt: body.attempt,
                stage: storage_stage(body.stage),
                discovered: body.discovered,
                processed: body.processed,
                reason_code: body.reason_code.map(storage_reason),
                reason: reason.as_deref(),
            },
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if moved.is_none() {
        return Err(refused_fence(&state, context.org, run, &device, body.attempt).await?);
    }
    let head = head_or_missing(&state, context.org, run).await?;
    if head.state == RunState::Failed {
        settled_event(&state, context.org, &head, RunState::Failed).await?;
    }
    // The console learns what the device said from the ledger, and this is
    // the only thing that puts it there: a run whose worker reports per-item
    // progress sends no page between metadata pages, so without an event the
    // page a seller is watching keeps its stale counts, its stale stage and
    // its stale last contact for as long as the pass runs.
    let counts = ImportRunRepo::new(state.pool.clone())
        .counts(context.org, run)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    progress_event(&state, context.org, &head, counts).await?;
    let described = ImportRunRepo::new(state.pool.clone())
        .described(context.org, run)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ImportExecutionView::of(&head, described)))
}

/// The seller confirms that this run's resources are to be created.
///
/// Durable request rather than work: it records who confirmed and when, moves
/// the run to `committing`, and answers. The server's own drain creates the
/// resources, which is what lets the seller navigate away — and what stops a
/// finished description pass from adding anything nobody confirmed.
pub(crate) async fn confirm(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
) -> Result<(StatusCode, Json<RunConfirmAck>), APIError> {
    let run = parse_id(&run)?;
    let head = head_or_missing(&state, context.org, run).await?;
    if !head.state.open() {
        return Err(run_settled(head.state));
    }
    let actor = uuid_text(context.user.0);
    let accepted = ImportRunRepo::new(state.pool.clone())
        .authorise_commit(context.org, run, &actor, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !accepted {
        return Err(run_settled(head.state));
    }
    let view = view_of(&state, context.org, run).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(RunConfirmAck {
            accepted,
            run: view,
        }),
    ))
}

/// What one chunk of a commit did, and where the run stands after it.
pub(crate) struct ChunkReport {
    pub applied: u32,
    pub failed: u32,
    pub run_state: RunState,
}

/// One chunk of a commit: up to [`ITEMS_PER_CHUNK`] authorised items decided
/// and created, the run moved, and the progress event emitted.
///
/// One definition for every caller — the seller's confirmed run, the
/// scheduler's own rule, the spreadsheet's batch — because the guards are the
/// point: two loops would be two answers to "may this resource be created".
///
/// Authorisation is checked here rather than assumed by the caller, and it is
/// not the run's state: a description pass that finished says nothing about
/// whether the seller asked for these resources.
pub(crate) async fn commit_chunk(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
) -> Result<ChunkReport, APIError> {
    let run = head.id;
    let repo = ImportRunRepo::new(state.pool.clone());
    let page = repo
        .commit_page(org, run, ITEMS_PER_CHUNK)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mut applied = 0_u32;
    let mut failed = 0_u32;
    // Whether one of the item transactions below is what moved the run to
    // `complete`. It travels out of [`commit_one`] rather than being inferred
    // from a re-read, because `set_run_state` moves an open run exactly once:
    // the caller it answered `true` is the only one that may announce the
    // settlement, and a state re-read here could not tell this chunk's own
    // write from somebody else's.
    let mut settled = false;
    for item in &page {
        match commit_one(state, org, head, item).await {
            Ok(committed) => {
                settled = settled || committed.settled;
                match committed.effect {
                    CommitEffect::Created => applied = applied.saturating_add(1),
                    // Bound onto a resource the catalogue already holds, or
                    // left for the seller's answer. Neither is a creation and
                    // neither is a failure.
                    CommitEffect::Bound | CommitEffect::Held => {}
                }
            }
            Err(refusal) => {
                // A refusal the seller can act on is recorded against the item
                // and the chunk carries on; anything else is ours and fails
                // the request, leaving the item `matched` for the next chunk.
                // Marking an item permanently failed because the database was
                // briefly unreachable would cost the seller a resource over a
                // fault that has already passed.
                if refusal.status_code().is_server_error() {
                    return Err(refusal);
                }
                if record_failure(
                    state,
                    org,
                    head,
                    item.locator.as_str(),
                    &reason_of(&refusal),
                )
                .await?
                {
                    failed = failed.saturating_add(1);
                }
            }
        }
    }

    // The recount, and the settlement it implies — under the run's own guard,
    // because the item transactions above may have created nothing at all.
    //
    // An import whose listings the catalogue already held, whose descriptions
    // all matched existing resources, or whose last committable row was
    // refused reaches zero outstanding without any successful create. Before
    // this it stayed `committing` forever, every drain pass returning without
    // progress, with its shop fenced against the seller's next import.
    let mut tx = begin_guarded(state, org).await?;
    let counts = tam_storage::counts_of(&mut tx, org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if counts.outstanding() == 0
        && tam_storage::set_run_state(&mut tx, org, run, RunState::Complete, None, (state.wall)())
            .await
            .map_err(|error| storage_fault(state, &error))?
    {
        // The recount is what moved it: every item transaction still saw work
        // outstanding, and what emptied the run was a refusal recorded above
        // rather than a create.
        settled = true;
    }
    let standing = tam_storage::guard_run(&mut tx, org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .map_or(head.state, |guard| guard.state);
    tx.commit()
        .await
        .map_err(|error| sql_fault(state, &error))?;
    if settled {
        settled_event(state, org, head, RunState::Complete).await?;
    }
    progress_event(state, org, head, counts).await?;
    Ok(ChunkReport {
        applied,
        failed,
        run_state: standing,
    })
}

/// Records one item's refusal under the run's own guard.
///
/// In a guarded transaction, and conditional on the item, because the item's
/// own transaction has already ended by the time the caller reaches here.
/// Between the two a stop can settle the run and a concurrent pass can create
/// or merge the very resource this refusal is about, and writing `failed`
/// over either would tell the seller that a resource they hold does not
/// exist, or advance a run they stopped. The run guard answers the first and
/// [`tam_storage::record_failed`]'s own condition answers the second.
///
/// Answers whether the refusal was recorded, so the chunk counts only what it
/// actually settled.
async fn record_failure(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    locator: &str,
    detail: &str,
) -> Result<bool, APIError> {
    let mut tx = begin_guarded(state, org).await?;
    let Some(guard) = tam_storage::guard_run(&mut tx, org, head.id)
        .await
        .map_err(|error| storage_fault(state, &error))?
    else {
        tx.rollback()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        return Ok(false);
    };
    if !guard.state.open() {
        // The seller stopped it, or it settled. Their ending stands; this
        // refusal is about work that no longer belongs to an open run.
        tx.rollback()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        return Ok(false);
    }
    let written =
        tam_storage::record_failed(&mut tx, org, head.id, locator, detail, (state.wall)())
            .await
            .map_err(|error| storage_fault(state, &error))?;
    tx.commit()
        .await
        .map_err(|error| sql_fault(state, &error))?;
    if written {
        item_settled_event(state, org, head, locator, "failed").await?;
    }
    Ok(written)
}

/// How many chunks one drain pass runs per run.
///
/// Bounded rather than "to the end", because the drain shares a pass with
/// every other tenant's work: a five-hundred-resource shop is finished across
/// consecutive passes rather than by holding this one for a minute. The run
/// stays `committing` in between, which is now a truthful state — the server
/// owns the work and the console says so.
const CHUNKS_PER_DRAIN: u32 = 8;

/// Creates what one authorised run still owes, in bounded chunks.
///
/// This is what replaces the browser's commit loop. A seller who confirms and
/// navigates away, and a server restarted mid-commit, both end at the same
/// place: the run is authorised, the drain picks it up, and the resources
/// exist exactly once.
pub(crate) async fn drain_run(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
) -> Result<u32, APIError> {
    if !head.execution.commit_authorised(head.scheduled) {
        return Ok(0);
    }
    let mut created = 0_u32;
    let mut standing = head.clone();
    for _ in 0..CHUNKS_PER_DRAIN {
        let chunk = commit_chunk(state, org, &standing).await?;
        created = created.saturating_add(chunk.applied);
        // Nothing moved: everything left is a question the seller owes an
        // answer to, so the run waits rather than spinning.
        if chunk.applied == 0 && chunk.failed == 0 {
            return Ok(created);
        }
        if chunk.run_state == RunState::Complete {
            return Ok(created);
        }
        standing.state = chunk.run_state;
    }
    Ok(created)
}

/// Settles an open run at the seller's own request.
///
/// The write is conditional on the run still being open, and the refusal it
/// can answer with is the point: if the last guarded commit completed between
/// the read above and this write, the stop does not overwrite `complete` with
/// `abandoned` and does not replace the instant or the sentence the seller was
/// already given. They are told what actually stands.
pub(crate) async fn abandon(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let run = parse_id(&run)?;
    let head = head_or_missing(&state, context.org, run).await?;
    if !head.state.open() {
        return Err(run_settled(head.state));
    }
    let stopped = ImportRunRepo::new(state.pool.clone())
        .set_state(
            context.org,
            run,
            RunState::Abandoned,
            Some("you stopped this import"),
            (state.wall)(),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !stopped {
        let standing = head_or_missing(&state, context.org, run).await?;
        return Err(run_settled(standing.state));
    }
    settled_event(&state, context.org, &head, RunState::Abandoned).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes one import, at any stage of its life.
///
/// Distinct from [`abandon`], which is the seller stopping an import they
/// intend to keep looking at: this removes it from their history, and stopping
/// it is the part of that it has to do first. So an open run is stopped here
/// too — a settled, failed, empty or already-abandoned one needs no stopping
/// and is admitted just the same, which is most of what a seller deletes.
///
/// `200`/`deleted` once no chunk and no device can still be working; `202`
/// with `stopping` while one can, and asking again is what learns that it has
/// finished. The resources earlier chunks committed stay in the catalogue
/// with their files and labels: they are the seller's own resources, and
/// deleting the record of how they arrived does not delete them. Nothing here
/// touches a marketplace.
pub(crate) async fn delete_run(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, run)): Path<(String, String)>,
) -> Result<Response, APIError> {
    let run = parse_id(&run)?;
    let status = ImportRunRepo::new(state.pool.clone())
        .delete(context.org, run, context.stamp((state.wall)()))
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    Ok(JobDeletionView::answer(status))
}

/// The device's own stop, replayable.
///
/// The console's [`abandon`] is the seller pressing stop in the browser and
/// answers 204. This is the same instruction arriving from the machine that
/// was running the import, and it differs in three ways the phone needs.
///
/// It is fenced: the attempt travels in the body, so a device whose run was
/// taken over while it was offline stops nothing when its queue drains.
/// A lapsed hold is admitted deliberately — the device that owned this
/// attempt may still stop it, because stopping takes no work from anyone and
/// refusing it would strand the instruction on a phone that can no longer
/// renew. A *later* attempt, or another device's, is refused.
///
/// It is idempotent: a run already abandoned answers the same 200 body rather
/// than a conflict, because the queued instruction is a replay of one that
/// may already have landed — from this device, or from the console. Only a
/// run that settled some *other* way refuses, because then the stop is a
/// claim about the ending that is not true.
///
/// And it answers a body: `{"abandoned": true}` is what tells the device its
/// queued stop was delivered by this server rather than answered by something
/// in between.
pub(crate) async fn device_stop(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device, run)): Path<(String, String, String)>,
    Json(body): Json<ImportStop>,
) -> Result<Json<ImportStopAck>, APIError> {
    let run = parse_id(&run)?;
    crate::import::admissible_device(&state, context.org, &device).await?;
    // One call, one transaction, one version of the row: the ownership test,
    // the attempt test and the transition are taken together under the run's
    // lock. A fence read here followed by a write afterwards would let a
    // takeover land between them, and this device's hour-old stop would
    // abandon the new owner's work.
    // The internal lookup, because this is a device's own post: a stop
    // replayed after the seller deleted the import must reach the repository,
    // where it is either the replay's own acknowledgement or the settled
    // run's conflict — never the not-found the phone would retry forever.
    let head = device_head(&state, context.org, run).await?;
    let outcome = ImportRunRepo::new(state.pool.clone())
        .stop_owned(context.org, run, &device, body.attempt, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    match outcome {
        tam_storage::StopOutcome::Stopped => {
            settled_event(&state, context.org, &head, RunState::Abandoned).await?;
            Ok(Json(ImportStopAck { abandoned: true }))
        }
        // The replay's own case: already stopped is what this caller asked
        // for, however many times it asks, and whoever stopped it.
        tam_storage::StopOutcome::Settled(RunState::Abandoned) => {
            Ok(Json(ImportStopAck { abandoned: true }))
        }
        tam_storage::StopOutcome::Settled(settled) => Err(run_settled(settled)),
        tam_storage::StopOutcome::Fenced(fence) => Err(fence_refusal(&fence)),
    }
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
        .map_err(|_| missing("We can't find that resource in this import."))?;
    let record = ImportRunRepo::new(state.pool.clone())
        .get(context.org, run)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    let hash = record
        .items
        .iter()
        .find(|item| item.ordinal == ordinal)
        .ok_or_else(|| missing("We can't find that resource in this import."))?
        .cover_hash
        .ok_or_else(|| missing("This resource has no cover."))?;
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
        .ok_or_else(|| missing("We can't find that import."))
}

/// The run's head for a device's own post, a deleted import included.
///
/// A device's queued page or stop is answered about the run rather than about
/// the seller's list, and the difference matters because of what the two
/// answers mean on the phone. The desktop transport reads a 404 as this
/// machine not being registered — an outage it offers again, ahead of every
/// other owed post — so a page replayed after the seller deleted the import
/// would sit at the head of that queue forever and hold the rest of it up. A
/// 409 naming a settled run is terminal: the post is retired and the queue
/// drains.
///
/// So a deleted run is not hidden here; it is refused with the answer it
/// already has for a run that is over. The seller's own history and detail
/// reads keep [`head_or_missing`], where a deleted import is a 404.
pub(crate) async fn device_head(
    state: &AppState,
    org: OrgId,
    run: Uuid,
) -> Result<ImportRunHead, APIError> {
    ImportRunRepo::new(state.pool.clone())
        .head_internal(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that import."))
}

/// The same lookup, refusing a deleted import outright.
///
/// What a page uses, because a page is a write: the run is over, nothing it
/// carries is applied, and the refusal is stated before any effect rather
/// than left to each branch's own fence. A stop uses [`device_head`]
/// instead — it takes work from nobody, and letting it reach the repository
/// is what lets a deleted run's retained hold be acknowledged.
pub(crate) async fn device_head_or_refusal(
    state: &AppState,
    org: OrgId,
    run: Uuid,
) -> Result<ImportRunHead, APIError> {
    let head = device_head(state, org, run).await?;
    if head.deletion.is_some() {
        return Err(run_settled(head.state));
    }
    Ok(head)
}

/// Which resources one run view carries with it.
///
/// The read is windowed rather than whole because a run of five hundred
/// resources used to arrive in full on every read, and the embedded copy on a
/// batch's own page arrived in full for a list that page never draws.
#[derive(Debug, Clone)]
pub(crate) struct ItemWindow {
    pub state: Option<RunItemState>,
    pub search: Option<String>,
    pub order: ItemOrder,
    pub offset: i64,
    pub limit: i64,
}

impl Default for ItemWindow {
    fn default() -> Self {
        Self {
            state: None,
            search: None,
            order: ItemOrder::Title,
            offset: 0,
            limit: ITEMS_PER_PAGE,
        }
    }
}

impl ItemWindow {
    /// The run's head and its open questions, and none of its resources.
    ///
    /// What a spreadsheet batch embeds: that page renders the review pairs and
    /// nothing else off the run, and its own report is the list of rows.
    /// `items_total` still states how many there are, so the absence reads as
    /// a window rather than as an empty run.
    pub(crate) const fn none() -> Self {
        Self {
            state: None,
            search: None,
            order: ItemOrder::Listed,
            offset: 0,
            limit: 0,
        }
    }
}

/// One run, with the first page of its resources.
pub(crate) async fn view_of(
    state: &AppState,
    org: OrgId,
    run: Uuid,
) -> Result<ImportRunView, APIError> {
    view_of_windowed(state, org, run, &ItemWindow::default()).await
}

/// One run, with the page of resources the caller asked for.
pub(crate) async fn view_of_windowed(
    state: &AppState,
    org: OrgId,
    run: Uuid,
    window: &ItemWindow,
) -> Result<ImportRunView, APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    let head = repo
        .head(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    let page = repo
        .items_page(
            org,
            run,
            &ItemPageFilter {
                state: window.state,
                search: window.search.as_deref(),
                order: window.order,
                offset: window.offset,
                limit: window.limit,
            },
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let counts = repo
        .counts(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let described = repo
        .described(org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let head = head_view(&head, counts, described);
    let pairs = crate::duplicates::pairs_of(state, org, Some(run)).await?;
    Ok(ImportRunView {
        id: head.id,
        kind: head.kind,
        source: head.source,
        batch_id: head.batch_id,
        state: head.state,
        read_total: head.read_total,
        counts: head.counts,
        items: page.items.iter().map(|item| item_view(run, item)).collect(),
        items_total: u32::try_from(page.total).unwrap_or(u32::MAX),
        items_offset: u32::try_from(window.offset).unwrap_or(u32::MAX),
        items_limit: u32::try_from(window.limit).unwrap_or(u32::MAX),
        review_pairs: pairs,
        created_at: head.created_at,
        settled_at: head.settled_at,
        retry_of: head.retry_of,
        deletion_status: head.deletion_status,
        execution: head.execution.clone(),
    })
}

#[must_use]
pub(crate) fn head_view(
    head: &ImportRunHead,
    counts: RunCounts,
    described: u32,
) -> ImportRunHeadView {
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
        retry_of: head.retry_of,
        deletion_status: head.deletion.map(DeletionStatusView::of),
        execution: ImportExecutionView::of(head, described),
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
/// Five things a page can carry, and it may carry several: a failure, part of
/// the shop's list, descriptions, refusals, and a completion. Two properties
/// hold over all of them.
///
/// It is fenced. A catalogue page names the attempt its device was granted,
/// and a page whose attempt is not the current one changes nothing — which is
/// what makes a taken-over phone harmless rather than a second writer.
///
/// It is atomic and replay-safe. Everything the page moves is written in one
/// transaction with the run row locked, and the acknowledgement is stored
/// under the page's own receipt: a device that never received an answer
/// resends the page and is told what it was told the first time, rather than
/// having it applied twice.
pub(crate) async fn run_page(
    state: &AppState,
    context: &OrgContext,
    device: &str,
    page: &tam_engine_driver::import::ImportPage,
    now: Timestamp,
) -> Result<(StatusCode, Json<crate::import::ImportAck>), APIError> {
    // Another organisation's run is missing rather than forbidden: the caller
    // learns nothing about whether the identifier exists, which is the posture
    // every other org-scoped read here takes.
    let head = device_head_or_refusal(state, context.org, page.run).await?;
    // An unfenced catalogue page is refused before any effect. `import.rs`
    // makes the same refusal at the boundary; this is it stated where the run
    // is known, so no path reaches the writes without an attempt.
    let Some(attempt) = page.attempt else {
        return Err(client_update_required());
    };

    // A page that says the import stopped. Fenced like every other write, and
    // settled before anything else is considered, because the device sends
    // nothing else with it and the seller's own run page is the record they
    // read.
    if let Some(why) = page.failed.as_ref() {
        let mut tx = begin_guarded(state, context.org).await?;
        let outcome = tam_storage::fenced(&mut tx, context.org, page.run, device, attempt)
            .await
            .map_err(|error| storage_fault(state, &error))?
            .ok_or_else(|| missing("We can't find that import."))?;
        // A terminal run is not reopened and its reason is not overwritten: a
        // late failure from a device that has already been taken over or
        // cancelled says nothing about the run as it stands.
        if outcome != FenceOutcome::Current {
            tx.rollback()
                .await
                .map_err(|error| sql_fault(state, &error))?;
            return Err(fence_refusal(&outcome));
        }
        tam_storage::set_run_state(
            &mut tx,
            context.org,
            page.run,
            RunState::Failed,
            Some(why.as_str()),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
        let counts = tam_storage::counts_of(&mut tx, context.org, page.run)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        tx.commit()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        settled_event(state, context.org, &head, RunState::Failed).await?;
        return Ok((StatusCode::OK, Json(run_ack(counts, 0, 0, true))));
    }

    if !head.state.open() {
        return Err(run_settled(head.state));
    }

    // Everything slow, before the lock: the cover bytes, the matcher's own
    // reads, and the catalogue lookup for listings this organisation already
    // holds. Nothing here writes to the run.
    let rows: Vec<tam_storage::ListedRow> = page
        .listed
        .as_ref()
        .map(|listed| {
            listed
                .iter()
                .map(|row| tam_storage::ListedRow {
                    locator: row.locator.as_str().to_owned(),
                    title: row.title.clone(),
                    price: listed_row_price(row, head.source),
                })
                .collect()
        })
        .unwrap_or_default();
    let held_bindings = match head.source {
        Some(source) if !rows.is_empty() => already_held(state, context.org, source, &rows).await?,
        _ => Vec::new(),
    };
    let mut prepared = Vec::with_capacity(page.resources.len());
    for resource in &page.resources {
        prepared.push(
            prepare_match(
                state,
                context.org,
                &head,
                resource,
                context.entitlement.caps.duplicate_review,
            )
            .await?,
        );
    }

    let identity = page_identity(page);
    let mut tx = begin_guarded(state, context.org).await?;
    let outcome = tam_storage::fenced(&mut tx, context.org, page.run, device, attempt)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    if outcome != FenceOutcome::Current {
        tx.rollback()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        return Err(fence_refusal(&outcome));
    }

    // The receipt, read under the same lock the writes take. A replay is
    // answered with the acknowledgement the first delivery was given, because
    // the device reads `applied` to decide whether it still owes us this page.
    if let Some(receipt) = page.receipt {
        match tam_storage::claim_receipt(&mut tx, context.org, page.run, receipt, &identity)
            .await
            .map_err(|error| storage_fault(state, &error))?
        {
            ReceiptOutcome::Fresh => {}
            ReceiptOutcome::Replay(ack) => {
                tx.rollback()
                    .await
                    .map_err(|error| sql_fault(state, &error))?;
                return Ok((StatusCode::OK, Json(replayed_ack(ack))));
            }
            ReceiptOutcome::Conflict => {
                tx.rollback()
                    .await
                    .map_err(|error| sql_fault(state, &error))?;
                return Err(receipt_conflict());
            }
        }
    }

    let mut listed_now = 0_u32;
    if !rows.is_empty() {
        listed_now = tam_storage::append_listed(&mut tx, context.org, page.run, &rows, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        // A listing the catalogue already holds is skipped as it lands,
        // before the seller can tick it and before the device reads it: an
        // imported resource is bound to the listing it came from, so a second
        // import of the same shop finds every one of them here. The matcher
        // could not have caught it — it compares across marketplaces only,
        // and this pair is the same listing on the same one.
        for (locator, title) in &held_bindings {
            tam_storage::record_skipped(
                &mut tx,
                context.org,
                tam_storage::ItemAddress {
                    run: page.run,
                    locator,
                },
                &format!("already in Resources as {title}"),
                now,
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
        }
    }

    // Discovery closes when the device says so and not when a page happens to
    // carry a list: a listed page may be partial, and selecting from half a
    // shop is the defect this bit exists to stop.
    if page.enumeration_complete {
        tam_storage::close_enumeration(&mut tx, context.org, page.run)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        // A scheduled run has no seller at the keyboard, so the server ticks
        // the list itself once the shop is whole: everything still listed,
        // which is everything the held rule above did not already skip. The
        // same write the seller's own "select everything" makes, so a
        // scheduled run and a ticked one reach the describe step identically.
        if head.scheduled {
            tam_storage::select_items(&mut tx, context.org, page.run, Selection::All, now)
                .await
                .map_err(|error| storage_fault(state, &error))?;
        }
    }

    let mut applied = 0_u32;
    for matched in &prepared {
        apply_matched(
            &mut tx,
            state,
            context.org,
            &head,
            matched,
            Some(device),
            now,
        )
        .await?;
        applied = applied.saturating_add(1);
    }

    let mut skipped = 0_u32;
    for skip in &page.skipped {
        tam_storage::record_skipped(
            &mut tx,
            context.org,
            tam_storage::ItemAddress {
                run: page.run,
                locator: skip.locator.as_str(),
            },
            skip.why.as_str(),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
        skipped = skipped.saturating_add(1);
    }

    let counts = tam_storage::counts_of(&mut tx, context.org, page.run)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    // Where the run stands after this page, decided here and written once.
    //
    // A closed enumeration with nothing outstanding is a finished import,
    // whatever the page said: an empty shop, or a shop every listing of which
    // the catalogue already holds, has nothing to describe, nothing to
    // confirm and nothing to create. Before this it sat `reading` forever
    // with its source fenced against the seller's next import, because the
    // only thing that ever moved a run on was a device sending a completion
    // it had no reason to send.
    let closed = page.enumeration_complete || head.execution.enumeration_complete;
    let next = if closed && counts.outstanding() == 0 {
        Some(RunState::Complete)
    } else if page.complete {
        // Completion closes the description and authorises nothing. A manual
        // run goes to the seller — that is where the confirmation comes from,
        // and an unconfirmed run creates no products however finished its
        // reading is. A scheduled run carries its own separately approved
        // rule, so it is the one that may go straight to committing.
        Some(if head.scheduled {
            RunState::Committing
        } else {
            RunState::Reviewing
        })
    } else {
        None
    };
    if let Some(next) = next {
        tam_storage::set_run_state(&mut tx, context.org, page.run, next, None, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    let ack = run_ack(counts, applied, skipped, page.complete);
    if let Some(receipt) = page.receipt {
        tam_storage::store_receipt(
            &mut tx,
            context.org,
            page.run,
            receipt,
            &tam_storage::StoredReceipt {
                identity: &identity,
                ack: tam_storage::ReceiptAck {
                    applied: ack.applied,
                    skipped: ack.skipped,
                    described_total: ack.described_total,
                    complete: ack.complete,
                },
                at: now,
            },
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    }
    // The owner spoke, and a page that carried anything moved something.
    tam_storage::note_contact(
        &mut tx,
        context.org,
        page.run,
        listed_now > 0 || applied > 0 || skipped > 0,
    )
    .await
    .map_err(|error| storage_fault(state, &error))?;
    tx.commit()
        .await
        .map_err(|error| sql_fault(state, &error))?;

    if listed_now > 0 {
        listed_event(state, context.org, &head, counts.listed).await?;
    }
    progress_event(state, context.org, &head, counts).await?;
    if next == Some(RunState::Complete) {
        settled_event(state, context.org, &head, RunState::Complete).await?;
    }
    Ok((StatusCode::OK, Json(ack)))
}

/// One page's content identity, with the attempt deliberately left out.
///
/// A device that resumed under a new fence and resent the page it never got
/// an answer for is sending the same page; a different page under the same
/// receipt is a client bug or a modified device, and conflicts.
fn page_identity(page: &tam_engine_driver::import::ImportPage) -> Vec<u8> {
    use sha2::Digest as _;
    // The page as it would be re-sent, with only the envelope cleared: the
    // fence, because a resume under a new attempt is the same page, and the
    // receipt, because it is the key rather than the content.
    //
    // Everything else is hashed through the vocabulary's own serialisation
    // rather than field by field, and that is the point: a title, a body, a
    // price, a listing binding, a fingerprint, a cover or a skip reason that
    // changed under one receipt is a different page, and a hash that read
    // only the locators would acknowledge it while applying nothing — the
    // device would then discard a description the server never accepted.
    // Serialisation is canonical here because every field of the vocabulary
    // is ordered by its own type, and the maps serde writes are struct
    // fields in declaration order.
    let mut identity_input = page.clone();
    identity_input.attempt = None;
    identity_input.receipt = None;
    let encoded = serde_json::to_vec(&identity_input).unwrap_or_default();
    let mut digest = sha2::Sha256::new();
    digest.update(&encoded);
    // A page that would not serialise cannot be given an identity, and
    // hashing an empty encoding for every such page would make two different
    // ones replay each other. The run's own identifier keeps that case
    // distinct per run, and a page this route accepted has already decoded,
    // so it serialises.
    digest.update(uuid_text(page.run).as_bytes());
    digest.finalize().to_vec()
}

/// The acknowledgement one stored receipt answers with.
fn replayed_ack(ack: tam_storage::ReceiptAck) -> crate::import::ImportAck {
    crate::import::ImportAck {
        // Zero applied, deliberately: nothing was applied by this delivery.
        // The device reads this to know the page is already ours.
        applied: 0,
        skipped: 0,
        described_total: ack.described_total,
        create_job: None,
        complete: ack.complete,
    }
}

/// A transaction with the tenant pinned and this organisation's catalogue
/// decisions serialised.
///
/// Everything slow happens before this is called: no marketplace request, no
/// file read and no fingerprint computation happens under the lock, and
/// nothing inside it opens a second transaction of its own.
pub(crate) async fn begin_guarded(
    state: &AppState,
    org: OrgId,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, APIError> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|error| sql_fault(state, &error))?;
    tam_storage::pin_tenant(&mut tx, org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    tam_storage::lock_org_catalogue(&mut tx, org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(tx)
}

fn sql_fault(state: &AppState, error: &sqlx::Error) -> APIError {
    state.internal(&format!("the database refused a transaction: {error}"))
}

/// Of the listed rows, those whose listing a product of this org is already
/// bound to on `source` *and* which that product holds a real picture for,
/// with the product's title.
///
/// One read of the tenant's mapping heads, one of its product summaries and
/// one of their covers, on the page that carries the list — which is the
/// first page of a run and no other — rather than a lookup per row.
///
/// Why the thumbnail decides. This skip is a saving: the resource is already
/// in the catalogue, so reading its file again buys nothing. That stops being
/// true for a resource whose thumbnail is missing, or is the generated card
/// that stood in for one — `tam_pipeline::render::is_generated_card` tells
/// the two apart by digest. The only place a picture can come from is the
/// resource's own bytes on the seller's device, the device reads those bytes
/// only for a row this rule did not skip, and nothing after the commit ever
/// redraws a cover — so skipping those rows is what made a historical
/// import's missing thumbnail permanent. Letting exactly them through costs
/// one bundle read each, once: the commit binds the listing onto the resource
/// the catalogue already holds rather than creating a second one, offers the
/// picture it now has, and the next run skips the row for good.
async fn already_held(
    state: &AppState,
    org: OrgId,
    source: InventoryId,
    rows: &[tam_storage::ListedRow],
) -> Result<Vec<(String, String)>, APIError> {
    let heads = tam_storage::MappingRepo::new(state.pool.clone())
        .list_heads(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let bound: Vec<(String, ProductId)> = heads
        .iter()
        .filter(|head| head.inventory == source)
        .filter_map(|head| {
            head.remote
                .as_ref()
                .map(|remote| (crate::migrations::locator_of(remote), head.product))
        })
        .collect();
    if bound.is_empty() {
        return Ok(Vec::new());
    }
    let repo = tam_storage::ProductRepo::new(state.pool.clone());
    let products = repo
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let held: Vec<ProductId> = bound.iter().map(|(_, product)| *product).collect();
    // Only a cover whose bytes this deployment holds counts, which is what
    // `covers` answers: a row naming a marketplace resource is a thumbnail no
    // console can draw, and treating it as one would leave the resource
    // unrepairable for exactly the reason above.
    let covers = repo
        .covers(org, &held)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let (_, product) = bound.iter().find(|(locator, _)| *locator == row.locator)?;
            let pictured = covers.iter().any(|cover| {
                cover.product == *product && !tam_pipeline::render::is_generated_card(cover.hash)
            });
            if !pictured {
                return None;
            }
            let title = products
                .iter()
                .find(|summary| summary.id == *product)
                .map_or_else(|| row.title.clone(), |summary| summary.title.0.clone());
            Some((row.locator.clone(), title))
        })
        .collect())
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

/// One described resource, ready to be matched and written.
///
/// The halves exist for the reason the commit's do: every slow read — the
/// cover's bytes into the blob store, the item's reserved identifier —
/// happens before the run's row is locked. The scorer is not among them. It
/// runs under the lock, in [`apply_matched`], because a verdict reached
/// against the catalogue as it was a moment ago is not a verdict about the
/// catalogue this write lands in: another source committing, a digest's
/// frequency moving, or this very shop's listing being bound to something
/// else all change what the scorer answers, and a merge is irreversible for
/// thirty days. So this struct carries the subject the scorer is asked about
/// rather than an answer about it.
pub(crate) struct MatchedRead {
    locator: String,
    observed: serde_json::Value,
    title: String,
    price: Option<Money>,
    cover_hash: Option<ContentHash>,
    /// How long those cover bytes are, measured where they were stored.
    ///
    /// Carried rather than read back, because the one branch that needs it
    /// runs under the run's lock and no file read happens there: a
    /// `product_file` row states its own length, and the only honest place to
    /// take it is beside the `put` that wrote the blob.
    cover_byte_len: Option<u64>,
    /// The identifier this resource's product will take: the row's reserved
    /// one where the run already listed it, and a fresh one otherwise. The
    /// matcher is asked about this identifier, because a question answered
    /// about one nothing took would be asked again on the next import.
    product: ProductId,
    /// What the scorer compares, built from the read.
    subject: Side,
    /// The subject's simhash bands, which are its blocking key.
    bands: Option<[i16; 4]>,
    /// Whether this plan includes the duplicate review.
    reviews: bool,
    source: InventoryId,
    /// The listing as read, for the one branch that writes a binding without
    /// creating a product: a resource the matcher decided the catalogue
    /// already holds binds this shop's listing onto the survivor.
    listing: tam_marketplace::ImportedListing,
    /// That listing's price, resolved by the source's own currency rule.
    /// Free where the rule cannot denominate it, which is the same absence
    /// the card already renders: this value only ever reaches a binding's
    /// price rule, and the commit that creates a product resolves it again
    /// through `import_one`, which refuses properly.
    price_intent: tam_types::PriceIntent,
}

/// Stores one description's cover and assembles what the matcher will be
/// asked, writing nothing to the run and asking nothing yet.
///
/// The one place both sources meet, so "what the matcher was given" cannot
/// differ between a device page and a spreadsheet row. The asking itself is
/// [`apply_matched`]'s, under the run's lock.
pub(crate) async fn prepare_match(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    resource: &ObservedResource,
    reviews: bool,
) -> Result<MatchedRead, APIError> {
    let now = (state.wall)();

    // The cover, stored as a held blob directly rather than through the ingest
    // pipeline, which would derive a cover from the cover. Its length is taken
    // here, where the bytes are, so no later branch has to read them back.
    let (cover_hash, cover_byte_len) = match resource.cover_png.as_ref() {
        Some(cover) => {
            let bytes = cover.bytes();
            (
                Some(store_cover(state, org, bytes, now).await?),
                Some(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
            )
        }
        None => (None, None),
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
    // The identifier the product will take. A row the enumeration already
    // listed carries a reserved one, and using a second would ask the matcher
    // about an identifier nothing takes.
    let product = ImportRunRepo::new(state.pool.clone())
        .item(org, head.id, resource.locator.as_str())
        .await
        .map_err(|error| storage_fault(state, &error))?
        .map_or_else(|| ProductId(fresh_uuid()), |item| item.product);

    let subject = side_of(resource, source);
    let bands = subject
        .text
        .map(|text| tam_fingerprint::simhash_bands(text.simhash))
        .map(|bands| bands.map(|band| i16::from_ne_bytes(band.to_ne_bytes())));

    Ok(MatchedRead {
        locator: resource.locator.as_str().to_owned(),
        observed,
        title: resource.listing.title.clone(),
        price,
        cover_hash,
        cover_byte_len,
        product,
        subject,
        bands,
        reviews,
        source,
        // The listing itself, kept for the one branch that has to write a
        // binding without a product of its own: a read the matcher decided is
        // a resource the catalogue already holds, which binds this shop's
        // listing onto the survivor.
        listing: resource.listing.clone(),
        price_intent: tam_import::resolve_price(source, &resource.listing.price)
            .unwrap_or(tam_types::PriceIntent::Free),
    })
}

/// Writes one matched description: the row, the scorer's decision under this
/// transaction's lock, the questions it raised, and the verdict it reached.
///
/// Inside the caller's transaction, because the row's state and the pairs
/// holding it in review are one fact: a review item whose questions were
/// written by a transaction that rolled back waits for an answer nobody was
/// asked for.
///
/// The scorer runs here rather than before the lock, and that is the whole
/// design of this call. A merge is a thirty-day-irreversible write, so the
/// evidence it rests on has to be the evidence that stands when it lands: a
/// digest's frequency moves as another source commits, this shop's listing
/// may have been bound to another product since the page was posted, and a
/// candidate may have been deleted or merged away. Re-resolving the survivor
/// alone was not enough — the verdict itself can change without the survivor
/// moving at all.
#[expect(
    clippy::too_many_arguments,
    reason = "the transaction, the state the scorer needs, the tenant, the run, the matched read, the device that described it and the instant; each comes from a different place and every caller passes all seven"
)]
pub(crate) async fn apply_matched(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    matched: &MatchedRead,
    device: Option<&str>,
    now: Timestamp,
) -> Result<RunItemState, APIError> {
    let at = tam_storage::ItemAddress {
        run: head.id,
        locator: matched.locator.as_str(),
    };
    let fresh = tam_storage::record_read(
        tx,
        org,
        head.id,
        &tam_storage::ReadItem {
            locator: matched.locator.as_str(),
            observed: &matched.observed,
            title: matched.title.as_str(),
            price: matched.price,
            cover_hash: matched.cover_hash,
            device,
            product: Some(matched.product),
        },
        now,
    )
    .await
    .map_err(|error| storage_fault_tx(org, &error))?;
    if !fresh {
        // Already described. A replayed page is told nothing changed rather
        // than having a product's input moved under a commit that may have
        // read it.
        return Ok(tam_storage::reserved_state(tx, org, at)
            .await
            .map_err(|error| storage_fault_tx(org, &error))?
            .unwrap_or(RunItemState::Read));
    }

    // The identifier the row actually holds, read back after the write. The
    // one this call was handed is a proposal: `prepare_match` mints a fresh
    // identifier for a locator the run does not yet hold, and a combined page
    // — a list and descriptions in one post — reserves the row's own
    // identifier by `append_listed` earlier in this very transaction. The
    // insert's conflict clause keeps the reserved one, so scoring, raising a
    // pair or settling under the proposed one would key the seller's question
    // to an identifier no row carries: `item_of_product` finds nothing, a
    // `different` answer cannot unblock the row, and it waits in review
    // forever. Everything below therefore uses this.
    let product = tam_storage::reserved_product(tx, org, at)
        .await
        .map_err(|error| storage_fault_tx(org, &error))?
        .unwrap_or(matched.product);

    // A pair this resource is already parked in is the seller's to answer,
    // and the scorer cannot see it: the never-ask-twice rule reads answered
    // pairs only. Honoured before the scorer is asked, so evidence that has
    // since fallen below the review floor cannot turn an open question into a
    // creation.
    if tam_storage::parked_for(tx, org, product)
        .await
        .map_err(|error| storage_fault_tx(org, &error))?
        > 0
    {
        tam_storage::record_verdict(tx, org, at, RunItemState::Review)
            .await
            .map_err(|error| storage_fault_tx(org, &error))?;
        return Ok(RunItemState::Review);
    }

    // The scorer, whole, against the catalogue as it stands inside this lock.
    let found = matcher::match_one_in(
        tx,
        &matcher::Asking {
            state,
            org,
            version: sketch_version(),
            subject: &matched.subject,
            subject_product: product,
            marketplace: Some(matched.source.marketplace()),
            bands: matched.bands,
            reviews: matched.reviews,
        },
    )
    .await?;

    // Even scored here, the survivor is resolved rather than assumed: the
    // candidate query and this write are one transaction, but a product
    // merged away by an earlier item of this very page is followed one hop to
    // what now stands, and one that stands nowhere is not merged onto.
    let survivor = match found.merged.first() {
        Some(raised) => survivor_now(tx, org, raised.other).await?,
        None => None,
    };
    let merge_withdrawn = !found.merged.is_empty() && survivor.is_none();

    for raised in found.merged.iter().chain(found.asked.iter()) {
        let merged = matches!(raised.verdict, tam_storage::Verdict::Same) && !merge_withdrawn;
        let other = match (merged, survivor.as_ref()) {
            (true, Some((product, _))) => *product,
            _ => raised.other,
        };
        let (lo, hi) = tam_storage::ordered_pair(product, other);
        // A withdrawn merge is parked, not decided — and a park is the
        // seller's question, never the system's claim. Migration 0070 states
        // that as a database fact: `duplicate_verdict_system_is_decisive`
        // admits `system` only on a `same` verdict from a decisive layer, so
        // parking with the scorer's own `system` voice would be refused by
        // the check and take the whole page's transaction with it.
        let (verdict, decided_by) =
            if merge_withdrawn && matches!(raised.verdict, tam_storage::Verdict::Same) {
                (tam_storage::Verdict::Parked, tam_storage::DecidedBy::Seller)
            } else {
                (raised.verdict, raised.decided_by)
            };
        tam_storage::raise_verdict(
            tx,
            org,
            &NewVerdict {
                lo,
                hi,
                verdict,
                decided_by,
                winning_layer: raised.layer,
                log_odds: raised.log_odds,
                fingerprint_version: sketch_version(),
                run: Some(head.id),
                kept: merged.then_some(other),
                raised_at: now,
                decided_at: merged.then_some(now),
                reversible_until: merged
                    .then_some(Timestamp(now.0.saturating_add(tam_storage::REVERSIBLE_MS))),
                evidence: &raised.evidence,
            },
        )
        .await
        .map_err(|error| storage_fault_tx(org, &error))?;
    }

    // A pair the system decided on its own is a merge, and the merge is the
    // skip: the kept product already holds this resource, so creating a second
    // one is exactly what the verdict says not to do.
    //
    // The survivor gains this shop's listing as well as its label, and the
    // binding is the half that used to go missing. A skipped item never
    // reaches the commit — `commit_page` takes `matched` rows only — so
    // without writing the mapping here the catalogue showed a Tes label on a
    // product with no Tes listing, and the next import of that shop could not
    // recognise the listing as one it already held and read it again.
    if let Some((product, title)) = survivor {
        // Which product ends up holding this listing, which is not always the
        // one the matcher named: another product of this organisation may
        // already bind it, and `bind_listing` leaves that claim standing and
        // answers with its owner. The label, the thumbnail and the sentence
        // all have to be about that product — offering them to the matcher's
        // choice instead would put this shop's chip and this read's picture
        // on a resource whose listing is somewhere else, which is the exact
        // mismatch the binding exists to prevent. `commit_one` reads the
        // holder the same way, through `bind_onto`.
        //
        // A survivor with no live payload is left unbound and unlabelled, for
        // the reason [`bind_onto`] states: 0061 refuses a claim on a resource
        // whose bytes are uncaptured, and its trigger is deferred, so writing
        // one would lose this whole page rather than this one row. The
        // resource still keeps the merge and still gains the thumbnail below.
        let holder = if tam_storage::has_live_payload(tx, org, product)
            .await
            .map_err(|error| storage_fault_tx(org, &error))?
        {
            let bound = tam_storage::bind_listing(
                tx,
                org,
                &tam_import::source_binding(
                    org,
                    product,
                    matched.source,
                    &matched.listing,
                    matched.price_intent,
                    now,
                ),
                0,
                now,
            )
            .await
            .map_err(|error| storage_fault_tx(org, &error))?;
            tam_storage::attach_system_label(tx, org, bound, matched.source.marketplace(), now)
                .await
                .map_err(|error| storage_fault_tx(org, &error))?;
            bound
        } else {
            product
        };
        // The thumbnail this read carried, offered to the resource that was
        // kept. A merge decided against a second product, not against the
        // picture: the survivor may be a metadata-only read from a source
        // that carries none, and dropping this one would leave a resource
        // with no thumbnail that nothing would ever redraw.
        let repaired = repair_cover(
            tx,
            org,
            holder,
            cover_offer(matched.cover_hash, matched.cover_byte_len),
            now,
        )
        .await
        .map_err(|error| storage_fault_tx(org, &error))?;
        // The title of the product the listing is actually on, for the same
        // reason: a sentence naming the matcher's choice would send the
        // seller to a resource this read did not touch. Read back rather than
        // reused where the holder is not the matcher's product.
        let named = if holder == product {
            title
        } else {
            tam_storage::title_of(tx, org, holder)
                .await
                .map_err(|error| storage_fault_tx(org, &error))?
                .unwrap_or(title)
        };
        tam_storage::record_skipped(
            tx,
            org,
            at,
            &skip_sentence(&named, Supplied::thumbnail(repaired)),
            now,
        )
        .await
        .map_err(|error| storage_fault_tx(org, &error))?;
        return Ok(RunItemState::Skipped);
    }

    // A withdrawn merge is a question owed, so the item waits for the seller
    // rather than being created behind a pair nobody answered.
    let next = if merge_withdrawn || found.needs_review() {
        RunItemState::Review
    } else {
        RunItemState::Matched
    };
    tam_storage::record_verdict(tx, org, at, next)
        .await
        .map_err(|error| storage_fault_tx(org, &error))?;
    Ok(next)
}

/// The product a candidate survivor has become, where it still stands.
///
/// Read inside the caller's guarded transaction, which is the whole point: a
/// candidate the matcher named before the lock may since have been deleted by
/// the seller or merged away by the duplicate review. A merge is followed one
/// hop to the product that now holds the resource; `None` means nothing does,
/// and nothing may be bound onto it.
async fn survivor_now(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    candidate: ProductId,
) -> Result<Option<(ProductId, String)>, APIError> {
    let standing = tam_storage::merged_into(tx, org, candidate)
        .await
        .map_err(|error| storage_fault_tx(org, &error))?
        .unwrap_or(candidate);
    // `title_of` reads live products only, so its absence is the tombstone.
    let title = tam_storage::title_of(tx, org, standing)
        .await
        .map_err(|error| storage_fault_tx(org, &error))?;
    Ok(title.map(|title| (standing, title)))
}

/// A storage fault raised inside a transaction, where the fault reporter's
/// own connection is not available.
///
/// Logged here for the reason `AppState::internal` logs: this is the other
/// constructor of a 500 in this crate, and a fault only the response carries
/// is a fault nobody can read after the response is gone.
fn storage_fault_tx(org: OrgId, error: &tam_storage::StorageError) -> APIError {
    let internals = format!(
        "the catalogue decision could not be written for {}: {error}",
        org.0.to_hyphenated()
    );
    eprintln!("tam-api: internal fault: {internals}");
    APIError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        APIErrorEntry::new(&internals).kind(APIErrorKind::Internal),
    )
}

/// The run that reviews one spreadsheet batch, found or opened, or the
/// refusal that this batch's import was deleted.
///
/// A run for the spreadsheet source too, and the reason is that the duplicate
/// review has to be one thing rather than two: the same pair table, the same
/// verdicts, the same never-ask-twice rule, and the same card. A batch that
/// held its own review would be a second implementation of the hardest part of
/// this feature.
///
/// The lookup is the batch's durable identity rather than the seller's
/// listing, and that distinction is the whole of the fence here. Deleting an
/// import does not destroy the spreadsheet's rows — they are the seller's own
/// upload, and the receipts of what was created from them — so a lookup that
/// could not see the tombstone found no run, opened a second one against
/// those preserved rows and created the rest of an import the seller had just
/// deleted. A deleted batch is refused instead, under the identity lock, so
/// the refusal cannot be raced by the deletion it is about.
pub(crate) async fn run_for_batch(
    state: &AppState,
    org: OrgId,
    batch: Uuid,
) -> Result<ImportRunHead, APIError> {
    let repo = ImportRunRepo::new(state.pool.clone());
    // The unlocked read first, which is the ordinary case and every resumed
    // chunk: a batch whose run exists needs no anchor job minted to be told
    // so. The locked decision below is what a first chunk reaches.
    if let Some(identity) = repo
        .batch_identity(org, batch)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        if let Some(deletion) = identity.deletion {
            return Err(batch_import_deleted(deletion));
        }
        return head_or_missing(state, org, identity.run).await;
    }
    let now = (state.wall)();
    let run = fresh_uuid();
    // A spreadsheet run names no shop, so its anchor job is filed against the
    // inventory the catalogue itself is: there is no marketplace read to
    // attribute it to, and the job carries no items either way.
    let anchor = anchor_job(state, org, run, InventoryId::Tes, now).await?;
    let opening = repo
        .open_for_batch(
            org,
            batch,
            &NewImportRun {
                id: run,
                kind: RunKind::Spreadsheet,
                source: None,
                batch_id: Some(batch),
                target: None,
                anchor_job: anchor,
                created_at: now,
                scheduled: false,
                // A batch is its own identity, so there is no start key to
                // mint, and a batch commit is never a retry of another run.
                start_key: None,
                retry_of: None,
            },
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    match opening {
        BatchRunOpening::Opened => {
            state.telemetry.capture(
                org,
                "import_run_started",
                serde_json::json!({
                    "kind": "spreadsheet",
                    "source_marketplace": serde_json::Value::Null,
                    "is_first_ever": first_ever(state, org).await,
                }),
            );
            head_or_missing(state, org, run).await
        }
        // A run for this batch appeared between the read above and this
        // write, which is two commit chunks racing on one batch: the answer
        // is that run, because it is the one this batch is reviewed through.
        BatchRunOpening::Existing(open) => head_or_missing(state, org, open).await,
        // And the deletion won that race. Refused for the same reason the
        // unlocked read refuses it, which is why both answers are one
        // sentence.
        BatchRunOpening::Deleted(deletion) => Err(batch_import_deleted(deletion)),
    }
}

/// This spreadsheet's import was deleted, so its rows create nothing more.
///
/// A conflict rather than a not-found: the batch is still there, and the
/// seller can still open it and read what it did. What is refused is
/// committing it again, and the deletion's own state is carried so a console
/// can say whether the stop has finished.
fn batch_import_deleted(deletion: tam_storage::DeletionStatus) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "You deleted the import for this spreadsheet, so its rows won't be created. Upload \
             it again to import it.",
        )
        .code(APIErrorCode::ImportRunSettled)
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({ "deletion_status": deletion.as_str() })),
    )
}

/// One claimed spreadsheet row, ready for the matcher.
///
/// The row's own draft is what the commit builds a product from, so it is
/// what the matcher is asked about — and the identifier it is asked under is
/// the one the claim reserved, because a question answered about an
/// identifier nothing took would be asked again on the next import.
pub(crate) struct SpreadsheetMatch {
    title: String,
    subject: Side,
    product: ProductId,
    draft: serde_json::Value,
    cover: Option<ContentHash>,
    reviews: bool,
}

/// Assembles what the matcher will be asked about one row, reading nothing
/// the decision transaction has to hold.
pub(crate) fn prepare_spreadsheet_match(
    row: &tam_storage::ClaimedRow,
    reviews: bool,
) -> SpreadsheetMatch {
    let title = row
        .draft
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
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
    SpreadsheetMatch {
        title,
        subject,
        product: row.product,
        draft: row.draft.clone(),
        cover: row.cover.as_ref().map(|cover| cover.hash),
        reviews,
    }
}

/// Decides one spreadsheet row inside the caller's guarded transaction.
///
/// The matcher runs here rather than before the lock, which is what makes a
/// resumed batch safe: a row matched an hour ago, against a catalogue another
/// source has since committed into, is decided against what stands now. The
/// row, the questions it raises and the verdict it reaches are one write with
/// the product the caller is about to create.
#[expect(
    clippy::too_many_arguments,
    reason = "the transaction, the tenant, the run, the row's handle, the prepared match and the instant; each comes from a different place and the one caller passes all six"
)]
pub(crate) async fn apply_spreadsheet_match(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    locator: &str,
    matched: &SpreadsheetMatch,
    now: Timestamp,
) -> Result<RunItemState, APIError> {
    let at = tam_storage::ItemAddress {
        run: head.id,
        locator,
    };
    // A row that has settled keeps its answer: an imported row created its
    // product, a skipped one was decided against, a failed one recorded why.
    // A row merely `matched` or in `review` is asked again, which is the whole
    // point of asking here.
    if let Some(held) = tam_storage::reserved_state(tx, org, at)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        if matches!(
            held,
            RunItemState::Imported | RunItemState::Skipped | RunItemState::Failed
        ) {
            return Ok(held);
        }
    }

    tam_storage::record_read(
        tx,
        org,
        head.id,
        &tam_storage::ReadItem {
            locator,
            // The row's own draft, which is the create form's shape: this is
            // the document the commit is about to build a product from, so it
            // is the honest record of what was matched.
            observed: &matched.draft,
            title: &matched.title,
            price: None,
            cover_hash: matched.cover,
            device: None,
            product: Some(matched.product),
        },
        now,
    )
    .await
    .map_err(|error| storage_fault(state, &error))?;

    // A pair this row is already parked in is a question the seller owes an
    // answer to, and the scorer cannot see it: `answered_pairs` excludes only
    // answered pairs from the never-ask-twice rule, so a parked pair is
    // re-scored from scratch. If the evidence has since fallen below the
    // review floor — another source committed, the frequency weighting moved
    // — the re-score returns no question at all and the row is created,
    // which is the duplicate the parked pair exists to prevent. So the park
    // is honoured before the scorer is asked.
    if tam_storage::parked_for(tx, org, matched.product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        > 0
    {
        tam_storage::record_verdict(tx, org, at, RunItemState::Review)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        return Ok(RunItemState::Review);
    }

    let found = matcher::match_one_in(
        tx,
        &matcher::Asking {
            state,
            org,
            version: sketch_version(),
            subject: &matched.subject,
            subject_product: matched.product,
            // A spreadsheet row is on no marketplace, so the
            // cross-marketplace rule admits every candidate: there is no
            // same-shop pair to exclude.
            marketplace: None,
            bands: None,
            reviews: matched.reviews,
        },
    )
    .await?;

    // The survivor a merge names, re-resolved under this same lock: the
    // matcher's candidate query and this decision are one transaction, but a
    // product merged away earlier in this very pass is still followed one hop
    // to what stands, and one that stands nowhere is not merged onto.
    let survivor = match found.merged.first() {
        Some(raised) => survivor_now(tx, org, raised.other).await?,
        None => None,
    };
    let merge_withdrawn = !found.merged.is_empty() && survivor.is_none();

    for raised in found.merged.iter().chain(found.asked.iter()) {
        let merged = matches!(raised.verdict, tam_storage::Verdict::Same) && !merge_withdrawn;
        let other = match (merged, survivor.as_ref()) {
            (true, Some((product, _))) => *product,
            _ => raised.other,
        };
        let (lo, hi) = tam_storage::ordered_pair(matched.product, other);
        // Parked, not decided — and a park is the seller's question, never
        // the system's claim. `duplicate_verdict_system_is_decisive` admits
        // `system` only on a `same` verdict from a decisive layer, so parking
        // in the scorer's own voice would be refused by the check and take
        // this row's whole transaction with it.
        let (verdict, decided_by) =
            if merge_withdrawn && matches!(raised.verdict, tam_storage::Verdict::Same) {
                (tam_storage::Verdict::Parked, tam_storage::DecidedBy::Seller)
            } else {
                (raised.verdict, raised.decided_by)
            };
        tam_storage::raise_verdict(
            tx,
            org,
            &NewVerdict {
                lo,
                hi,
                verdict,
                decided_by,
                winning_layer: raised.layer,
                log_odds: raised.log_odds,
                fingerprint_version: sketch_version(),
                run: Some(head.id),
                kept: merged.then_some(other),
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

    if let Some((_, title)) = survivor {
        tam_storage::record_skipped(tx, org, at, &format!("same as {title}"), now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        return Ok(RunItemState::Skipped);
    }
    let next = if merge_withdrawn || found.needs_review() {
        RunItemState::Review
    } else {
        RunItemState::Matched
    };
    tam_storage::record_verdict(tx, org, at, next)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(next)
}

/// The refusal a settled run answers a caller with.
#[must_use]
pub(crate) fn run_settled_refusal(state: RunState) -> APIError {
    run_settled(state)
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
    // The write is conditional on the run still being open, and a refusal is
    // not an error here: a batch finishing its last row after the seller
    // stopped the import does not overwrite that abandonment with a
    // completion, and the answer the seller was given stands. Nothing is
    // announced either, because nothing moved.
    let settled = ImportRunRepo::new(state.pool.clone())
        .set_state(org, head.id, run_state, None, (state.wall)())
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if !settled {
        return Ok(());
    }
    settled_event(state, org, head, run_state).await
}

/// What committing one item did.
pub(crate) enum CommitEffect {
    /// A product was created.
    Created,
    /// The resource was bound onto a product the catalogue already held, and
    /// the item settled as a skip. One resource, one product, both listings.
    Bound,
    /// Left where it is: a question is still owed about it, or the run's own
    /// guard refused the commit.
    Held,
}

/// What committing one item did, and whether its own transaction is what
/// settled the run.
///
/// The two travel together because they are one fact. The last item of a run
/// completes it inside the transaction that finished it — [`settle_if_done`]
/// — and the terminal event is appended after that transaction commits. A
/// chunk that re-read the run's state to decide whether to announce anything
/// would find `complete` and no way to tell whose write made it so, which is
/// how a finished import ended up with no `ImportRunSettled` event at all:
/// the chunk's own conditional transition found the run already settled and
/// read that as "nothing moved".
pub(crate) struct ItemCommit {
    pub effect: CommitEffect,
    pub settled: bool,
}

impl ItemCommit {
    /// An item that left the run exactly where it found it.
    const fn held() -> Self {
        Self {
            effect: CommitEffect::Held,
            settled: false,
        }
    }
}

/// Creates one authorised item: the product, its source binding, its label,
/// its sketch and the row's outcome — in one transaction, under this
/// organisation's catalogue lock.
///
/// The order is the whole design. Everything slow happens first and outside
/// the lock: the stored description is decoded, the seller's connection is
/// resolved, the cover's bytes are read back, and the catalogue write is
/// prepared. Then the lock is taken, and under it the decision is made
/// against what is committed *now* rather than against what the matcher saw
/// when the page landed.
///
/// Five things can be true under that lock, and each has one answer:
/// a question is still parked, so the item waits; the seller merged this
/// resource away, so it binds onto the survivor; this organisation already
/// has the resource this listing names — live, or deleted locally with the
/// listing's claim still standing — so that resource is reconciled against
/// what this read found rather than a second one being minted; that named
/// resource is itself a resource the seller merged away, so the survivor
/// standing for it answers the row and the tombstone is left where their
/// decision put it; a decisive digest twin exists, so the two sources end as
/// one product carrying both bindings. Only if none of them holds is a
/// product created.
#[expect(
    clippy::too_many_lines,
    reason = "the five revalidation answers and the create are one decision taken under one lock; splitting them would put half of what the lock protects outside the function that takes it"
)]
async fn commit_one(
    state: &AppState,
    org: OrgId,
    head: &ImportRunHead,
    item: &ImportRunItemRecord,
) -> Result<ItemCommit, APIError> {
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
        request: None,
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
        // A catalogue read names no file: the first-party-export read carries
        // what describes a listing, and the seller's own file arrives by the
        // separate download hop a migration's device performs. D32 and
        // migration 0061 make that a product with no payload rather than a
        // refusal, and a run drafts nowhere so no mapping exists for the
        // trigger to fire on.
        None => Vec::new(),
    };

    // The cover's length, read back from the blob it was stored as. Before the
    // lock, deliberately: this is a file read, and no file read happens under
    // the catalogue lock.
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
    let prepared = tam_import::prepare_one(&run, &applied)
        .await
        .map_err(|error| import_refusal(state, error))?;

    // The matcher's own subject, and the plan's review capability, read
    // before the lock: the subject is pure, and the entitlement is one
    // indexed read that has nothing to do with the catalogue decision.
    let subject = side_of(&resource, source);
    let bands = subject
        .text
        .map(|text| tam_fingerprint::simhash_bands(text.simhash))
        .map(|bands| bands.map(|band| i16::from_ne_bytes(band.to_ne_bytes())));
    let reviews = crate::entitlement::Entitlement::of(
        tam_storage::EntitlementRepo::new(state.pool.clone())
            .current(org, now)
            .await
            .map_err(|error| storage_fault(state, &error))?,
    )
    .caps
    .duplicate_review;

    let at = tam_storage::ItemAddress {
        run: head.id,
        locator: item.locator.as_str(),
    };
    let mut tx = begin_guarded(state, org).await?;
    let guard = tam_storage::guard_run(&mut tx, org, head.id)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    // The cancellation ordering, stated once: a cancellation that committed
    // before this transaction took the row's lock is visible here and rejects
    // the commit; one that arrives after this transaction commits finds the
    // resource created and stops the later work instead. Authorisation is
    // checked in the same read, so an unconfirmed manual run creates nothing
    // however finished its description pass is.
    if !guard.state.open() || !guard.commit_authorised {
        tx.rollback()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        return Ok(ItemCommit::held());
    }
    // Another commit page may have settled this item while we waited.
    // Its imported row can be a fileless resource's only source identity.
    if tam_storage::reserved_state(&mut tx, org, at)
        .await
        .map_err(|error| storage_fault(state, &error))?
        != Some(RunItemState::Matched)
    {
        tx.rollback()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        return Ok(ItemCommit::held());
    }

    // A question still owed about this resource. The item goes back to review
    // rather than being created behind the seller's answer.
    if tam_storage::parked_for(&mut tx, org, item.product)
        .await
        .map_err(|error| storage_fault(state, &error))?
        > 0
    {
        tam_storage::record_verdict(&mut tx, org, at, RunItemState::Review)
            .await
            .map_err(|error| storage_fault(state, &error))?;
        tx.commit()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        return Ok(ItemCommit::held());
    }

    // The revalidation, against the catalogue as it stands at this instant.
    //
    // Two answers that already exist are read first: a merge the seller
    // decided, and the resource this shop's own listing already names. Then
    // the matcher itself is re-asked — the whole scorer, with every layer,
    // the cross-marketplace rule, the frequency weighting, the negative
    // evidence and the never-ask-twice exclusions — because a decision
    // reached against an empty catalogue cannot be trusted after another
    // source has committed into it. A digest comparison standing in for the
    // scorer would miss exactly the pair the scorer exists to catch: matching
    // titles and text over different bytes.
    let merged = tam_storage::merged_into(&mut tx, org, item.product)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    // The resource this read *is*, where this organisation already has one:
    // the claim the shop's listing holds on a product, or — for a read that
    // carried no file, which migration 0061 admits no claim for — the run
    // item an earlier import of the same locator settled.
    //
    // Keyed on the listing's own identifier rather than on the row's locator.
    // They are not the same string on a shop that numbers its rows, and it is
    // the identifier the two partial unique indexes refuse a second claim on:
    // keying on the locator left the claim unseen, so the commit minted a
    // second product and the insert was refused with 23505 — a fault, which
    // the chunk retries, so the row stayed `matched` and the run stayed
    // `committing` for good.
    //
    // Tombstones included. A local delete leaves the claim standing, and a
    // seller who imports a listing again after deleting their copy is asking
    // for that resource rather than for a second one.
    let identity = match merged {
        Some(_) => None,
        None => {
            match tam_storage::claimed_product_for(&mut tx, org, source, &resource.listing.remote)
                .await
                .map_err(|error| storage_fault(state, &error))?
            {
                claimed @ Some(_) => claimed,
                None => {
                    tam_storage::imported_product_for(&mut tx, org, source, item.locator.as_str())
                        .await
                        .map_err(|error| storage_fault(state, &error))?
                }
            }
        }
    };

    let (twin, decided) = if let Some(kept) = merged {
        (Some(kept), None)
    } else if identity.is_some() {
        // The resource this listing already names. No score could outrank an
        // identity, so the matcher is not asked and no pair is raised about a
        // resource being compared with itself.
        (None, None)
    } else {
        let found = matcher::match_one_in(
            &mut tx,
            &matcher::Asking {
                state,
                org,
                version: sketch_version(),
                subject: &subject,
                subject_product: item.product,
                marketplace: Some(source.marketplace()),
                bands,
                reviews,
            },
        )
        .await?;
        let decided_merge = found.merged.first().map(|raised| raised.other);
        (decided_merge, Some(found))
    };

    // Whatever the revalidation raised is written here, in this transaction:
    // the merge it decided, and every question it now owes the seller. A pair
    // the machinery decided and did not store is a decision the next import
    // cannot read, and a review the commit held without a stored pair is a
    // resource waiting for a question nobody was asked.
    if let Some(found) = decided.as_ref() {
        for raised in found.merged.iter().chain(found.asked.iter()) {
            let (lo, hi) = tam_storage::ordered_pair(item.product, raised.other);
            let same = matches!(raised.verdict, tam_storage::Verdict::Same);
            tam_storage::raise_verdict(
                &mut tx,
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
                    kept: same.then_some(raised.other),
                    raised_at: now,
                    decided_at: same.then_some(now),
                    reversible_until: same
                        .then_some(Timestamp(now.0.saturating_add(tam_storage::REVERSIBLE_MS))),
                    evidence: &raised.evidence,
                },
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
        }
        // A question the revalidation raised is the seller's to answer, and
        // this item waits for it rather than being created behind it.
        if twin.is_none() && found.needs_review() {
            tam_storage::record_verdict(&mut tx, org, at, RunItemState::Review)
                .await
                .map_err(|error| storage_fault(state, &error))?;
            tx.commit()
                .await
                .map_err(|error| sql_fault(state, &error))?;
            item_settled_event(state, org, head, item.locator.as_str(), "review").await?;
            return Ok(ItemCommit::held());
        }
    }

    // The resource this listing already names, reconciled against what this
    // read found. Before the twin branch below because it is a different
    // question: there the catalogue holds a resource that *resembles* this
    // one and nothing about it moves, here it holds the very resource this
    // listing is, so this read's file is its file and a local delete of it
    // is undone.
    if let Some(held) = identity {
        // A merged listing names the final survivor, not its tombstoned
        // ancestor. Re-import may restore that survivor after a later local
        // delete, but must not undo merges or overwrite the survivor's own
        // metadata, files or listing claims.
        let merged_away = if held.live {
            None
        } else {
            survivor_of(state, &mut tx, org, held.product).await?
        };
        if let Some(survivor) = merged_away {
            let restored = tam_storage::restore_product(&mut tx, org, survivor, now)
                .await
                .map_err(|error| storage_fault(state, &error))?;
            let repaired = repair_cover(
                &mut tx,
                org,
                survivor,
                applied
                    .cover
                    .as_ref()
                    .map(|cover| (cover.hash, cover.byte_len)),
                now,
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
            let (settlement, effect) = if restored {
                tam_storage::record_imported(&mut tx, org, at, survivor, now)
                    .await
                    .map_err(|error| storage_fault(state, &error))?;
                ("imported", CommitEffect::Created)
            } else {
                let title = tam_storage::title_of(&mut tx, org, survivor)
                    .await
                    .map_err(|error| storage_fault(state, &error))?
                    .ok_or_else(|| {
                        validation("The resource this listing was merged into is unavailable.")
                    })?;
                tam_storage::record_skipped(
                    &mut tx,
                    org,
                    at,
                    &skip_sentence(&title, Supplied::thumbnail(repaired)),
                    now,
                )
                .await
                .map_err(|error| storage_fault(state, &error))?;
                ("skipped", CommitEffect::Bound)
            };
            let settled = settle_if_done(state, &mut tx, org, head.id, now).await?;
            tx.commit()
                .await
                .map_err(|error| sql_fault(state, &error))?;
            item_settled_event(state, org, head, item.locator.as_str(), settlement).await?;
            return Ok(ItemCommit { effect, settled });
        }
        let reconciled = reconcile_source(
            state,
            &mut tx,
            org,
            held,
            &prepared,
            applied
                .cover
                .as_ref()
                .map(|cover| (cover.hash, cover.byte_len)),
            now,
        )
        .await?;
        let title = if reconciled.restored {
            std::borrow::Cow::Borrowed(prepared.product.title.0.as_str())
        } else {
            std::borrow::Cow::Owned(
                tam_storage::title_of(&mut tx, org, reconciled.product)
                    .await
                    .map_err(|error| storage_fault(state, &error))?
                    .ok_or_else(|| validation("The imported resource is unavailable."))?,
            )
        };
        // Pair the accepted file's sketch with the canonical title, not
        // marketplace metadata this resource declined to adopt.
        if let Some(fingerprint) = resource
            .fingerprint
            .as_ref()
            .filter(|_| reconciled.describes_the_payload)
        {
            write_fingerprint(
                &mut tx,
                org,
                reconciled.product,
                fingerprint,
                &title,
                item.device.as_deref(),
                now,
            )
            .await?;
        }
        // A restore is a creation from the seller's side: the catalogue gained
        // a resource it did not have, and a row that said "skipped" would be
        // telling them it had been there all along. Anything else settles as
        // the skip it is, naming what this read supplied.
        let (settlement, effect) = if reconciled.restored {
            tam_storage::record_imported(&mut tx, org, at, reconciled.product, now)
                .await
                .map_err(|error| storage_fault(state, &error))?;
            ("imported", CommitEffect::Created)
        } else {
            tam_storage::record_skipped(
                &mut tx,
                org,
                at,
                &skip_sentence(&title, reconciled.supplied),
                now,
            )
            .await
            .map_err(|error| storage_fault(state, &error))?;
            ("skipped", CommitEffect::Bound)
        };
        let settled = settle_if_done(state, &mut tx, org, head.id, now).await?;
        tx.commit()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        item_settled_event(state, org, head, item.locator.as_str(), settlement).await?;
        return Ok(ItemCommit { effect, settled });
    }

    if let Some(twin) = twin {
        // One resource, one product, both bindings. The survivor gains this
        // shop's listing and this shop's label, and the item settles as a skip
        // rather than as a second product.
        // The product the listing ends up on, which is the twin unless
        // another product of this organisation already held that listing.
        let holder = bind_onto(state, &mut tx, org, twin, &prepared, now).await?;
        let title = tam_storage::title_of(&mut tx, org, holder)
            .await
            .map_err(|error| storage_fault(state, &error))?
            .unwrap_or_else(|| "a resource you already have".to_owned());
        // The repair for every resource imported before a cover could be
        // derived: this read carried a thumbnail drawn on the seller's own
        // device from the resource's own bytes, and the catalogue's copy of
        // that resource has none. It is offered here and refused where one
        // already lives, so a thumbnail the seller chose by hand outlives
        // every re-import. Nothing else about the kept resource moves: not
        // its payload, not its title, not who read it.
        let repaired = repair_cover(
            &mut tx,
            org,
            holder,
            applied
                .cover
                .as_ref()
                .map(|held| (held.hash, held.byte_len)),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
        tam_storage::record_skipped(
            &mut tx,
            org,
            at,
            &skip_sentence(&title, Supplied::thumbnail(repaired)),
            now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
        let settled = settle_if_done(state, &mut tx, org, head.id, now).await?;
        tx.commit()
            .await
            .map_err(|error| sql_fault(state, &error))?;
        item_settled_event(state, org, head, item.locator.as_str(), "skipped").await?;
        return Ok(ItemCommit {
            effect: CommitEffect::Bound,
            settled,
        });
    }

    let bound = tam_import::apply_prepared(&mut tx, org, &prepared, now)
        .await
        .map_err(|error| import_refusal(state, error))?;
    // The shop's label follows the binding. A read whose bytes are still
    // uncaptured lands as a product with no mapping — migration 0061's rule —
    // and labelling it would put a marketplace chip on a resource that shop's
    // listing does not hold, which is the mismatch the binding exists to
    // prevent. The capture writes both.
    if bound {
        tam_storage::attach_system_label(&mut tx, org, item.product, source.marketplace(), now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }
    if let Some(fingerprint) = resource.fingerprint.as_ref() {
        write_fingerprint(
            &mut tx,
            org,
            item.product,
            fingerprint,
            &prepared.product.title.0,
            item.device.as_deref(),
            now,
        )
        .await?;
    }
    tam_storage::record_imported(&mut tx, org, at, item.product, now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let settled = settle_if_done(state, &mut tx, org, head.id, now).await?;
    tx.commit()
        .await
        .map_err(|error| sql_fault(state, &error))?;
    item_settled_event(state, org, head, item.locator.as_str(), "imported").await?;
    Ok(ItemCommit {
        effect: CommitEffect::Created,
        settled,
    })
}

/// What a read supplied to the resource the catalogue already held.
///
/// Carried rather than inferred from the read, because what matters to the
/// seller is what actually landed: a read carrying a file supplies nothing to
/// a resource that already has one, and telling them otherwise would send
/// them looking for a change nobody made.
#[derive(Debug, Clone, Copy)]
struct Supplied {
    file: bool,
    thumbnail: bool,
}

impl Supplied {
    /// A thumbnail and nothing else, which is what a merge and a twin can
    /// offer: neither moves the kept resource's bytes.
    const fn thumbnail(repaired: bool) -> Self {
        Self {
            file: false,
            thumbnail: repaired,
        }
    }
}

/// What reconciling a read against the resource its own listing names did to
/// that resource.
struct Reconciled {
    /// The product the listing is on afterwards, which is what the sentence
    /// and the row's provenance are about.
    product: ProductId,
    /// Whether this transaction brought the resource back from a local
    /// delete. The one outcome that makes the row a creation: the catalogue
    /// gained a resource it did not have.
    restored: bool,
    /// Whether this read's sketch describes the payload the resource is left
    /// holding: its own capture was written, or the one payload it kept is
    /// byte for byte this capture, or it holds no payload for a sketch to
    /// disagree with. `false` where the resource kept other bytes — the
    /// seller's own file, an earlier read's, a bundle from another entry —
    /// and a sketch of what it declined would be a claim about a file it
    /// does not have.
    describes_the_payload: bool,
    supplied: Supplied,
}

/// Reconciles one read against the resource its own listing already names:
/// restores it where the seller had deleted it, offers the thumbnail and the
/// file this read captured, and binds the listing where the payload makes a
/// claim legal.
///
/// All of it in the caller's transaction, under the run's guard and this
/// organisation's catalogue lock, because a restore that committed without
/// the claim it needs — or a claim written onto a resource whose payload
/// arrived in a later transaction — is exactly the half-state the deferred
/// triggers and the partial unique indexes exist to refuse.
///
/// Why restore rather than mint a second product. The claim on the listing is
/// the tombstoned resource's, and the two indexes admit one: a second product
/// either loses to the index — the 23505 this repairs — or would have to
/// sever a claim on a listing that is still live on the shop, which would
/// tell the ledger a listing nobody removed is gone. Restoring keeps the
/// remote identity where it already is.
///
/// Safe under two schedulers. The restore is `WHERE deleted_at IS NOT NULL`,
/// the cover is offered only where the resource holds none, the payload
/// offer locks the product and decides against what it finds, and the bind
/// reads before it writes — so a pass that arrives second finds its work
/// done and reports the same resource rather than a second one.
#[expect(
    clippy::too_many_arguments,
    reason = "the transaction, the tenant and the instant bound the write; the claim, the \
              prepared read and its thumbnail are what is being reconciled, and a parameter \
              bag over them would only rename the call"
)]
async fn reconcile_source(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    held: tam_storage::BoundClaim,
    prepared: &tam_import::PreparedResource,
    cover: Option<(ContentHash, u64)>,
    now: Timestamp,
) -> Result<Reconciled, APIError> {
    // Back from the delete first, because everything below writes onto a live
    // resource: `update_product`, `offer_payload` and `offer_cover` all read
    // `deleted_at`, and so does every trigger that judges them. `false` means
    // another pass restored it between this transaction's read and its lock,
    // which is the same resource by the same identity rather than a conflict.
    let restoring = !held.live;
    let restored = restoring
        && tam_storage::restore_product(tx, org, held.product, now)
            .await
            .map_err(|error| storage_fault(state, &error))?;

    // What the shop says about the resource, written back only where this is
    // a resource coming back from a local delete.
    //
    // The product record owns the canonical fields, and drift on the shop is
    // the seller's to adopt or overwrite rather than ours to apply behind
    // them (`docs/notes/design/vendoo-for-teachers-rethink.md:75-77`). This
    // path is reached by every ordinary repair — a thumbnail that could not
    // be derived when the resource was created, a bundle captured by a later
    // read — so refreshing it unconditionally meant confirming a repair
    // destroyed the title and description the seller had written and then
    // reported the row as a skip. A resource they had deleted has no
    // authored copy left to protect, and what comes back is what the shop
    // says now.
    if restoring {
        tam_storage::update_product(tx, org, held.product, &refresh_of(&prepared.product), now)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }

    let thumbnail = repair_cover(tx, org, held.product, cover, now)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    // The file, where this read captured one and the resource has none: the
    // ordinary life of a draft read for its metadata first and its bundle
    // later. Offered before the bind and in this transaction, because the
    // payload is what migration 0061 requires of a resource a claim names and
    // the trigger judges what the transaction leaves behind.
    //
    // A restore offers more than that, and only a restore does. What a
    // tombstoned import leaves behind is a locator and the digest of the
    // bytes that shop served last time — not bytes this server keeps — so
    // the manifest sends that old commitment and the device's verification
    // rejects the file the shop serves now: the seller was told the restore
    // worked and left holding a resource nothing can publish, on the very
    // pass that had just captured usable bytes. `RestoreSource` refreshes
    // exactly that file: the resource's sole payload, source-backed, the
    // same source identity as this capture. An uploaded file, a file the
    // seller added beside the import's, or a bundle from another entry is
    // theirs and is kept.
    let policy = if restoring {
        tam_storage::PayloadOfferPolicy::RestoreSource
    } else {
        tam_storage::PayloadOfferPolicy::FillMissing
    };
    let offered = match prepared.product.payload_files().next() {
        Some(captured) => Some(
            tam_storage::offer_payload(tx, org, held.product, captured, None, policy, now)
                .await
                .map_err(|error| storage_fault(state, &error))?,
        ),
        None => None,
    };
    let file = matches!(offered, Some(tam_storage::PayloadOffer::Written));
    // Whether this read's sketch is this resource's to store. A read that
    // carried no bytes at all contradicts nothing where the resource holds
    // none either, which is the metadata-only draft read again and again.
    let describes_the_payload = match offered {
        Some(tam_storage::PayloadOffer::Written | tam_storage::PayloadOffer::Unchanged) => true,
        Some(tam_storage::PayloadOffer::AlreadyHeld | tam_storage::PayloadOffer::NoProduct) => {
            false
        }
        None => !tam_storage::has_live_payload(tx, org, held.product)
            .await
            .map_err(|error| storage_fault(state, &error))?,
    };

    let product = bind_onto(state, tx, org, held.product, prepared, now).await?;
    Ok(Reconciled {
        product,
        restored,
        describes_the_payload,
        supplied: Supplied { file, thumbnail },
    })
}

/// The product the seller's merges left standing in place of this one, or
/// `None` where this product is nobody's loser.
///
/// A survivor can lose a later merge, so follow the complete chain.
/// Cycles are refused rather than returning an arbitrary intermediate product.
///
/// `None` is the ordinary tombstone a local delete leaves, which is the one
/// the re-import restores.
async fn survivor_of(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
) -> Result<Option<ProductId>, APIError> {
    let mut walked = std::collections::HashSet::new();
    let mut standing = product;
    while let Some(kept) = tam_storage::merged_into(tx, org, standing)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        if !walked.insert(standing) {
            return Err(validation(
                "This resource couldn't be imported because its merge history is broken.",
            ));
        }
        standing = kept;
    }
    Ok((standing != product).then_some(standing))
}

/// What a re-read of one listing writes back to the resource it names.
///
/// The described fields, and only where the read actually carried them. A
/// read states a title, a body and a price, so those are refreshed. Subjects,
/// grades and a rights grant are absent from plenty of listings, and writing
/// an absence over what the catalogue holds would let a re-import delete the
/// seller's own taxonomy — so an empty one says nothing rather than saying
/// "none".
fn refresh_of(product: &tam_domain::CanonicalProduct) -> tam_storage::ProductEdit {
    tam_storage::ProductEdit {
        title: Some(product.title.clone()),
        body: Some(product.body.clone()),
        price: Some(product.price),
        subjects: (!product.subjects.is_empty()).then(|| product.subjects.clone()),
        grades: (!product.grades.raw.is_empty()).then(|| product.grades.clone()),
        rights: match &product.rights {
            tam_domain::RightsDeclaration::Unstated => None,
            declared @ tam_domain::RightsDeclaration::Declared { .. } => Some(declared.clone()),
        },
    }
}

/// Binds the listing this run read onto a product the catalogue already
/// holds, and gives that product this shop's label.
///
/// Through [`tam_storage::bind_listing`], which reads before it writes. The
/// distinction is not tidiness: PostgreSQL aborts the whole transaction on a
/// unique violation, so catching `ListingAlreadyBound` from the insert would
/// leave this caller carrying on inside a transaction the database had
/// already thrown away — the label below and the item's own settlement would
/// be silently discarded, and the item would be retried forever. A listing
/// this organisation has already bound is an ordinary outcome of a
/// re-imported shop, not a fault, so it must never reach the insert.
///
/// A resource with no live payload is left unbound, and unlabelled with it.
/// Migration 0061 moved the payload requirement from the product to the
/// mapping, so a claim on a resource whose bytes are still uncaptured raises
/// `mapping % names a product with no live payload file` — and that trigger
/// is deferred, so the whole transaction is lost at commit rather than the
/// one row, and the item is retried for good. The same rule the create path
/// follows: the claim and the shop's chip arrive with the file.
///
/// A permanent mapping conflict aborts this item's transaction.
/// [`commit_chunk`] records its failure in a separate transaction, so no
/// preflight query or partial catalogue write is needed to report it.
///
/// Answers which product holds the listing afterwards, which is the product
/// this shop's label belongs on: where another product already held it, that
/// one, because binding is what the label follows.
#[expect(
    clippy::too_many_arguments,
    reason = "the transaction, state and tenant bound the decision; the product, prepared source \
              and instant are the write itself, and a parameter bag would only rename them"
)]
async fn bind_onto(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
    prepared: &tam_import::PreparedResource,
    now: Timestamp,
) -> Result<ProductId, APIError> {
    let mut binding = prepared.source_mapping.clone();
    binding.id = tam_types::MappingId(fresh_uuid());
    binding.product = product;
    if !tam_storage::has_live_payload(tx, org, product)
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        return Ok(product);
    }
    let held = tam_storage::bind_listing(tx, org, &binding, 0, now)
        .await
        .map_err(|error| claim_refusal(state, &error))?;
    tam_storage::attach_system_label(tx, org, held, binding.inventory.marketplace(), now)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok(held)
}

/// Settles the run inside the same transaction that finished its last item,
/// answering whether this transaction is the one that moved it.
///
/// In the transaction rather than after it, so a run cannot be seen holding
/// nothing outstanding while still saying it is committing. The answer is
/// returned rather than discarded because the transition is what earns the
/// terminal event: `set_run_state` moves an open run once, and the caller it
/// answered `true` owes the announcement.
async fn settle_if_done(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    run: Uuid,
    now: Timestamp,
) -> Result<bool, APIError> {
    let counts = tam_storage::counts_of(tx, org, run)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    if counts.outstanding() != 0 {
        return Ok(false);
    }
    tam_storage::set_run_state(tx, org, run, RunState::Complete, None, now)
        .await
        .map_err(|error| storage_fault(state, &error))
}

/// What an import refusal is, on the way out.
///
/// Every arm the seller can act on is a validation answer about their own
/// listing; the rest are ours.
fn import_refusal(state: &AppState, error: tam_import::ImportError) -> APIError {
    match error {
        tam_import::ImportError::Storage(error) => claim_refusal(state, &error),
        refused @ (tam_import::ImportError::NoPayload
        | tam_import::ImportError::Price(_)
        | tam_import::ImportError::CurrencyUnknown { .. }) => validation(&refused.to_string()),
        // A run names no target, so nothing lowers an intent and nothing asks
        // for one.
        impossible @ (tam_import::ImportError::Lowering(_) | tam_import::ImportError::NoTarget) => {
            state.internal(&format!("the import answered {impossible}"))
        }
    }
}

/// A claim this commit could not reconcile.
///
/// A conflict rather than a fault of ours, and the distinction decides
/// whether the seller ever hears about it. Every claim this server can see is
/// answered by a read before the write — the listing's own claim, tombstone
/// included, and the provenance of a read that could carry none — so
/// reaching the insert means a claim no lookup here addresses, or one another
/// writer took between this transaction's read and its insert. Either way it
/// is permanent for this item: [`commit_chunk`] retries a fault, so reporting
/// this as one leaves the row `matched` and the run `committing` through
/// every later drain, which is precisely the healthy-looking stall this
/// repair exists to end. As a conflict the row settles `failed` carrying the
/// reason, the run finishes, and the seller can act.
///
/// Three permanent ones, and the third is a different fact from the first
/// two. `ListingAlreadyBound` and `MappingAlreadyBound` are about the
/// listing: somebody else's claim stands on it. The marketplace slot is
/// about the product: this resource already carries a mapping for this shop,
/// which is the seller's own cross-listing of a resource an earlier
/// metadata-only read created — `mapping_one_per_inventory` admits one, and
/// no retry will ever admit a second. The slot they filled is theirs: it is
/// not severed, not rebound and not taken to make this row land, because a
/// conflicting slot is not permission to steal one. The row fails, naming it.
///
/// Every other storage error stays ours and stays retried. A database that
/// was briefly unreachable, a deadlock, a serialisation failure: marking a
/// resource permanently failed over a fault that has already passed would
/// cost the seller the resource for nothing.
fn claim_refusal(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    match error {
        claimed @ (tam_storage::StorageError::ListingAlreadyBound
        | tam_storage::StorageError::MappingAlreadyBound) => {
            conflict(&claimed.to_string(), APIErrorCode::ListingAlreadyClaimed)
        }
        filled @ tam_storage::StorageError::InventoryMappingAlreadyExists => {
            conflict(&filled.to_string(), APIErrorCode::MappingAlreadyExists)
        }
        blocked @ tam_storage::StorageError::SellerRuleBlocked { .. } => {
            crate::jobs::storage_fault(state, blocked)
        }
        fault @ (tam_storage::StorageError::Db(_)
        | tam_storage::StorageError::TimestampOutOfRange { .. }
        | tam_storage::StorageError::CorruptRow { .. }
        | tam_storage::StorageError::OrgMismatch
        | tam_storage::StorageError::Inconsistent { .. }
        | tam_storage::StorageError::StaleLease
        | tam_storage::StorageError::DuplicateIdempotencyKey { .. }
        | tam_storage::StorageError::StorefrontBoundElsewhere { .. }
        | tam_storage::StorageError::AttemptInFlight) => storage_fault(state, fault),
    }
}

/// Stores one product's sketch, in the columns the matcher blocks on, inside
/// the transaction that created the product.
///
/// In the transaction because a sketch that outlived a rolled-back product is
/// a candidate the matcher would offer for a resource nobody has.
#[expect(
    clippy::too_many_arguments,
    reason = "the sketch is a device's assertion, so the write carries the machine and the instant beside the value; folding them into a struct would hide exactly the distinction 0052 exists to keep"
)]
pub(crate) async fn write_fingerprint(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
    fingerprint: &tam_fingerprint::Fingerprint,
    title: &str,
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
    tam_storage::put_fingerprint(
        tx,
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
            title_norm: &tam_fingerprint::normalise_title(title),
            observed_by_device: device,
            observed_at: now,
        },
        now,
    )
    .await
    .map_err(|error| storage_fault_tx(org, &error))?;
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

/// The cover a read carried, as the pair a write needs: its digest and its
/// length. `None` unless both are known, because a `product_file` row states
/// its own length and half an answer is not one.
const fn cover_offer(
    hash: Option<ContentHash>,
    byte_len: Option<u64>,
) -> Option<(ContentHash, u64)> {
    match (hash, byte_len) {
        (Some(hash), Some(byte_len)) => Some((hash, byte_len)),
        _ => None,
    }
}

/// Offers a thumbnail to the resource the catalogue kept, and answers whether
/// one arrived.
///
/// Both skip branches meet here, which is the point: a read the matcher
/// merged away and a read the commit found already bound are the same
/// situation for a thumbnail — this resource's picture was derived on the
/// seller's device, and the catalogue's copy of the resource either has no
/// thumbnail or has the generated card that stood in for one.
///
/// What may be replaced is the renderer's answer, not this function's:
/// `tam_pipeline::render::is_generated_card` recognises a card by the digest
/// it is stored under, so a card gives way to a picture and a picture — the
/// seller's own upload, or an earlier read's genuine preview — never gives
/// way to anything. The predicate rather than a flag, because the digests
/// belong to the renderer that draws them and a copy of them here would be a
/// second list to keep in step.
///
/// The bytes are already in this tenant's blob store: the page that carried
/// them stored them there. Nothing here reads a file, and nothing here
/// reaches a marketplace.
async fn repair_cover(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
    cover: Option<(ContentHash, u64)>,
    now: Timestamp,
) -> Result<bool, tam_storage::StorageError> {
    let Some((hash, byte_len)) = cover else {
        return Ok(false);
    };
    // A read that found no picture either carries a card or carries nothing,
    // and the two are the same answer to a repair: there is nothing to
    // supply. Refused here rather than offered, so a resource with no
    // thumbnail is never given a card by this path and a resource whose
    // thumbnail is a card never has it exchanged for another one — and, in
    // both cases, the seller is not told a thumbnail arrived when none did.
    if tam_pipeline::render::is_generated_card(hash) {
        return Ok(false);
    }
    let file = tam_types::ProductFile {
        id: tam_types::FileId(fresh_uuid()),
        role: tam_types::FileRole::Cover,
        kind: FileKind::Image,
        bytes: FileBytes::Held {
            hash,
            byte_len,
            // Rendered by the device from bytes it scanned there, which is the
            // same verdict the create path records for this cover.
            scan: ScanOutcome::Clean { at: now },
        },
    };
    let offered = tam_storage::offer_cover(
        tx,
        org,
        product,
        &file,
        tam_pipeline::render::is_generated_card,
        now,
    )
    .await?;
    Ok(matches!(
        offered,
        tam_storage::CoverOffer::Written | tam_storage::CoverOffer::Replaced
    ))
}

/// What the seller reads on a skipped row.
///
/// What the read supplied is said out loud. A row that reads only "same as
/// Fractions pack" after a re-import the seller ran to fix a missing
/// thumbnail — or to fetch the file a metadata-only read could not carry —
/// hides the one thing that changed, and the file is the change that decides
/// whether the resource can reach a marketplace at all, so it is named
/// first.
fn skip_sentence(title: &str, supplied: Supplied) -> String {
    match (supplied.file, supplied.thumbnail) {
        (true, true) => format!("same as {title}; this import added its file and thumbnail"),
        (true, false) => format!("same as {title}; this import added its file"),
        (false, true) => format!("same as {title}; this import added its thumbnail"),
        (false, false) => format!("same as {title}"),
    }
}

async fn store_cover(
    state: &AppState,
    org: OrgId,
    bytes: &[u8],
    now: Timestamp,
) -> Result<ContentHash, APIError> {
    let blobs = state.blobs.clone().ok_or_else(blob_store_unavailable)?;
    BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .put(org, bytes, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))
}

async fn cover_bytes(state: &AppState, org: OrgId, hash: ContentHash) -> Result<Vec<u8>, APIError> {
    let blobs = state.blobs.clone().ok_or_else(blob_store_unavailable)?;
    BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .get(org, hash)
        .await
        .map_err(|error| state.internal(&error.to_string()))
}

pub(crate) fn blob_store_unavailable() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new("Teachouse can't save or show covers right now. Try again later.")
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
    crate::consent::require_grant(state, org, source.marketplace()).await?;
    let created = JobRepo::new(state.pool.clone())
        .create_with_request_key(
            org,
            JobOrigin {
                request_key: job_request_key(run, IMPORT_LEG),
                run: None,
                // The anchor is minted before the run's own row exists — it
                // is what that row's `anchor_job` names — so it cannot carry
                // a link back to it. The deletion reaches it through
                // `import_run.anchor_job` instead, which is the direction
                // that is always available.
                import_run: None,
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
    // The anchor names no workflow — it *is* the run's own row, minted before
    // the run exists — so the mint has nothing to be refused by. A refusal
    // here would mean the origin carried a link this function does not set.
    let Minted::Job(created) = created else {
        return Err(state.internal("an import anchor was refused by a workflow it never named"));
    };
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
    // Completion is counted here rather than at each of the two call sites
    // that reach it, because this is the one funnel every settle passes
    // through, and a run that completes twice would be two rows on the
    // activation chart. The counts are re-read rather than threaded: they
    // are what the run actually did, and this runs once per run.
    if run_state == RunState::Complete {
        let counts = ImportRunRepo::new(state.pool.clone())
            .counts(org, head.id)
            .await
            .unwrap_or_default();
        let elapsed_ms = (state.wall)().0.saturating_sub(head.created_at.0);
        // Whole seconds, because the chart this feeds reads a duration in
        // seconds and a run measured to the millisecond is a run nobody
        // groups by.
        #[expect(
            clippy::integer_division,
            reason = "whole seconds is the resolution the chart reads"
        )]
        let duration_secs = elapsed_ms / 1_000;
        state.telemetry.capture(
            org,
            "import_run_completed",
            serde_json::json!({
                "kind": run_kind_word(head.kind),
                "resources_committed": counts.imported,
                "duplicates_merged": counts.matched,
                "duration_secs": duration_secs,
                "is_first_ever": first_ever(state, org).await,
            }),
        );
    }
    // Every abandonment reaching here is a stop — the seller's from the
    // console, or the owning device's own — so the reason is that one word
    // rather than a parameter every caller would have to be trusted to
    // spell. The stage is read from the head as it stood before the write,
    // which is where the run was given up.
    if run_state == RunState::Abandoned {
        let counts = ImportRunRepo::new(state.pool.clone())
            .counts(org, head.id)
            .await
            .unwrap_or_default();
        state.telemetry.capture(
            org,
            "import_run_abandoned",
            serde_json::json!({
                "stage": stage_of(head),
                "reason_code": "stopped",
                "items_listed": counts.listed,
            }),
        );
    }
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

/// How an import's two ways in are spelled on an event.
const fn run_kind_word(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Marketplace => "marketplace",
        RunKind::Spreadsheet => "spreadsheet",
    }
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

/// The lease, in the shape the device reads: milliseconds since the epoch,
/// which is every other timestamp on this wire.
fn wire_lease(lease: tam_storage::ImportLease) -> ImportLease {
    ImportLease {
        attempt: lease.attempt,
        lease_expires_at: lease.lease_expires_at.0,
    }
}

/// The stage a device reported, in the storage vocabulary.
const fn storage_stage(stage: tam_engine_driver::import::ImportStage) -> ImportStage {
    match stage {
        tam_engine_driver::import::ImportStage::Discovering => ImportStage::Discovering,
        tam_engine_driver::import::ImportStage::Selecting => ImportStage::Selecting,
        tam_engine_driver::import::ImportStage::Reading => ImportStage::Reading,
        tam_engine_driver::import::ImportStage::Interrupted => ImportStage::Interrupted,
        tam_engine_driver::import::ImportStage::Failed => ImportStage::Failed,
    }
}

/// The reason a device gave, in the storage vocabulary.
const fn storage_reason(code: tam_engine_driver::import::ImportReasonCode) -> ImportReasonCode {
    match code {
        tam_engine_driver::import::ImportReasonCode::MissingSession => {
            ImportReasonCode::MissingSession
        }
        tam_engine_driver::import::ImportReasonCode::NotPermitted => ImportReasonCode::NotPermitted,
        tam_engine_driver::import::ImportReasonCode::UnsupportedSource => {
            ImportReasonCode::UnsupportedSource
        }
        tam_engine_driver::import::ImportReasonCode::EnumerationFailed => {
            ImportReasonCode::EnumerationFailed
        }
        tam_engine_driver::import::ImportReasonCode::DescriptionFailed => {
            ImportReasonCode::DescriptionFailed
        }
        tam_engine_driver::import::ImportReasonCode::SubmissionFailed => {
            ImportReasonCode::SubmissionFailed
        }
        tam_engine_driver::import::ImportReasonCode::ActivationExpired => {
            ImportReasonCode::ActivationExpired
        }
        tam_engine_driver::import::ImportReasonCode::LeaseExpired => ImportReasonCode::LeaseExpired,
        tam_engine_driver::import::ImportReasonCode::Stopped => ImportReasonCode::Stopped,
        tam_engine_driver::import::ImportReasonCode::ClientUpdateRequired => {
            ImportReasonCode::ClientUpdateRequired
        }
    }
}

/// Why this device may not write, classified under the run's own lock.
///
/// Every one of these is a conflict rather than a generic refusal, and that
/// is the device's requirement rather than a preference: a device retries
/// from its own outbox on anything it reads as transport trouble, so an
/// outage and a cancellation sharing a status would have it keep reading a
/// run the seller stopped.
async fn refused_fence(
    state: &AppState,
    org: OrgId,
    run: Uuid,
    device: &str,
    attempt: u64,
) -> Result<APIError, APIError> {
    let outcome = ImportRunRepo::new(state.pool.clone())
        .fence(org, run, device, attempt)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("We can't find that import."))?;
    Ok(fence_refusal(&outcome))
}

fn fence_refusal(outcome: &FenceOutcome) -> APIError {
    match outcome {
        // Reachable when the row moved between the write and this read; the
        // honest answer is still "not yours to write".
        FenceOutcome::Current | FenceOutcome::Stale { .. } => conflict(
            "This import was restarted, so this attempt stops here.",
            APIErrorCode::ImportRunFenced,
        ),
        FenceOutcome::NotOwner { owner } => match owner {
            Some(owner) => conflict(
                &format!("Another device ({owner}) is running this import."),
                APIErrorCode::ImportRunFenced,
            ),
            None => conflict(
                "No device is running this import. Take it over first.",
                APIErrorCode::ImportRunFenced,
            ),
        },
        // The hold lapsed. Told apart from a takeover because the remedy is
        // its own: claim again, and the device's readiness is fresh rather
        // than assumed from before it stopped answering.
        FenceOutcome::Expired => conflict(
            "This device stopped running the import. Take it over again to continue.",
            APIErrorCode::ImportRunFenced,
        ),
        FenceOutcome::Settled(state) => run_settled(*state),
    }
}

/// Another device holds this import, and this is which.
fn held_by(device: &str, lease_expires_at: Option<Timestamp>) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(&format!(
            "Another device ({device}) is running this import. Take it over to continue here."
        ))
        .code(APIErrorCode::ImportRunFenced)
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({
            "owner_device": device,
            "lease_expires_at": lease_expires_at.map(|at| at.0),
        })),
    )
}

/// The run has settled, so nothing more is written to it.
fn run_settled(state: RunState) -> APIError {
    conflict(
        match state {
            RunState::Abandoned => "This import was stopped, so nothing more will be created.",
            RunState::Failed => "This import failed, so nothing more will be created.",
            RunState::Reading | RunState::Reviewing | RunState::Committing | RunState::Complete => {
                "This import has finished, so it can't take more."
            }
        },
        APIErrorCode::ImportRunSettled,
    )
}

/// The page carries no fence, which only an old client sends.
fn client_update_required() -> APIError {
    conflict(
        "Update the Teachouse app, then start the import again.",
        APIErrorCode::ImportClientUpdateRequired,
    )
}

/// The same receipt, different content.
fn receipt_conflict() -> APIError {
    conflict(
        "This part of the import was already received with different contents, so it was not \
         saved again.",
        APIErrorCode::ImportReceiptConflict,
    )
}

/// The start key was spent on a different intent.
fn start_key_spent(source: InventoryId) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "This import was started for a different shop. Start a new import instead.",
        )
        .code(APIErrorCode::ImportStartKeySpent)
        .kind(APIErrorKind::Validation)
        .detail(serde_json::json!({ "source": source })),
    )
}

/// The start key names an import the seller deleted.
///
/// The same code a spent key answers, because that is what this is from the
/// client's point of view — the key is used up and the run behind it is not
/// available — and the sentence is what differs.
fn start_key_deleted() -> APIError {
    conflict(
        "You deleted this import. Start a new one instead.",
        APIErrorCode::ImportStartKeySpent,
    )
}

fn conflict(message: &str, code: APIErrorCode) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(message)
            .code(code)
            .kind(APIErrorKind::Validation),
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
        .map_err(|_| missing("We can't find that import."))
}

fn uuid_text(id: Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}
