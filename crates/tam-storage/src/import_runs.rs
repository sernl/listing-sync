//! One import, as a run with a row per resource.
//!
//! Both sources reach the same rows. A marketplace run is filled by the
//! seller's own device — a list first, then a description per selected
//! resource — and a spreadsheet run is filled from the batch the parse already
//! holds. What the two share is the pause: an item sits at `matched` or
//! `review` having created nothing, and the commit is what mints products.
//!
//! Nothing here creates a product. `product_id` is a reserved identifier
//! minted when the row is written, exactly as `import_batch_row.product_id`
//! is, and for two reasons rather than one: a resumed commit reads whether
//! that product exists and so cannot mint a second, and the matcher needs a
//! stable name for a side of a pair before either side is a product.

use sqlx::{PgPool, Postgres, Transaction};
use tam_types::{
    Actor, ContentHash, InventoryId, JobId, Money, OrgId, ProductId, Stamp, SystemComponent,
    Timestamp, Uuid,
};

use crate::codec::{
    currency_from_db, currency_to_db, hash_from_db, hash_to_db, inventory_from_db, inventory_to_db,
    timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::jobs::{DeletionStatus, JobFence};
use crate::{pin_org, BoundClaim, StorageError};

/// How many runs one page of the history may answer at most.
///
/// The ceiling on what a caller may ask for, not the size a page normally
/// is: the console asks for ten, and this is what stops a hand-written query
/// string asking for the whole history back.
pub const RUNS_LISTED_MAX: i64 = 50;

/// How many resources of one run a page may answer at most. The console asks
/// for twenty-five.
pub const ITEMS_LISTED_MAX: i64 = 200;

/// Which runs one page of the history is about, and which end of it.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunHistoryFilter {
    /// One shop. `None` is every shop rather than none.
    pub source: Option<InventoryId>,
    /// Only the runs that name no shop, which is the spreadsheet way in.
    pub spreadsheet_only: bool,
    pub state: Option<RunState>,
    /// Every run still expecting work, whichever of the three open states it
    /// stands in.
    pub open_only: bool,
    pub oldest: bool,
    pub offset: i64,
    pub limit: i64,
}

/// One page of run history, and the size of the history it was cut from.
pub struct RunHistoryPage {
    pub runs: Vec<ImportRunHead>,
    /// How many runs the filter matches altogether. The figure the pager
    /// states, and never an inference from the page's own length.
    pub total: i64,
}

/// The order one run's resources are read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ItemOrder {
    /// Title A–Z, with the read order as the tie-break, so a page boundary
    /// falls in the same place on every read.
    #[default]
    Title,
    /// The order the shop listed them in.
    Listed,
}

/// Which resources of one run a page is about.
#[derive(Debug, Clone, Copy, Default)]
pub struct ItemPageFilter<'a> {
    pub state: Option<RunItemState>,
    /// A plain substring of the title, or of the locator where the
    /// enumeration named no title. Matched over the whole run, never over the
    /// page.
    pub search: Option<&'a str>,
    pub order: ItemOrder,
    pub offset: i64,
    pub limit: i64,
}

/// One page of a run's resources, and how many the filter matched.
pub struct ItemPage {
    pub items: Vec<ImportRunItemRecord>,
    pub total: i64,
}

/// Which machinery produced a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    Marketplace,
    Spreadsheet,
}

impl RunKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Marketplace => "marketplace",
            Self::Spreadsheet => "spreadsheet",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "marketplace" => Ok(Self::Marketplace),
            "spreadsheet" => Ok(Self::Spreadsheet),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown import run kind {other:?}"),
            }),
        }
    }
}

/// Where a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Reading,
    Reviewing,
    Committing,
    Complete,
    Failed,
    Abandoned,
}

impl RunState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Reviewing => "reviewing",
            Self::Committing => "committing",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }

    /// Whether the run is still expecting work. The same predicate the partial
    /// unique index uses, stated here so the route and the column agree on
    /// what "open" means.
    #[must_use]
    pub const fn open(self) -> bool {
        matches!(self, Self::Reading | Self::Reviewing | Self::Committing)
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "reading" => Ok(Self::Reading),
            "reviewing" => Ok(Self::Reviewing),
            "committing" => Ok(Self::Committing),
            "complete" => Ok(Self::Complete),
            "failed" => Ok(Self::Failed),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown import run state {other:?}"),
            }),
        }
    }
}

/// Where one item of a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunItemState {
    Listed,
    Selected,
    Read,
    Matched,
    Review,
    Imported,
    Skipped,
    Failed,
}

impl RunItemState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Listed => "listed",
            Self::Selected => "selected",
            Self::Read => "read",
            Self::Matched => "matched",
            Self::Review => "review",
            Self::Imported => "imported",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "listed" => Ok(Self::Listed),
            "selected" => Ok(Self::Selected),
            "read" => Ok(Self::Read),
            "matched" => Ok(Self::Matched),
            "review" => Ok(Self::Review),
            "imported" => Ok(Self::Imported),
            "skipped" => Ok(Self::Skipped),
            "failed" => Ok(Self::Failed),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown import run item state {other:?}"),
            }),
        }
    }
}

/// How often an owner is expected to renew, in seconds.
///
/// The founder-approved figure, held here beside the lease it paces. Four
/// renewals inside one lease, so three lost polls on a phone that has the
/// screen off do not cost the seller their import.
pub const RENEWAL_SECS: i64 = 15;

/// How long one claim holds a run without a renewal, in seconds.
pub const LEASE_SECS: i64 = 60;

/// How long a manual run waits for a device to claim it, in seconds.
///
/// A seller pressed Import and is looking at the page, so this is measured in
/// how long they will watch nothing happen before the answer has to be a
/// sentence they can act on. A scheduled run is not paced by it at all: that
/// one truthfully awaits a device, and expiring it would abandon work nobody
/// asked to abandon.
pub const MANUAL_ACTIVATION_SECS: i64 = 120;

/// A renewal window fits inside a lease several times over, checked where the
/// compiler checks it rather than in a test that could not fail for any input.
const _: () = assert!(
    RENEWAL_SECS * 3 < LEASE_SECS,
    "a lease has to survive several missed renewals"
);

/// What the owner says it is doing.
///
/// The device's own vocabulary, which is not the console's: the displayed
/// stage is this combined with the run's state and its freshness by the
/// server, so a browser never infers liveness from a stream it happens to
/// hold open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Discovering,
    Selecting,
    Reading,
    Interrupted,
    Failed,
}

impl ImportStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovering => "discovering",
            Self::Selecting => "selecting",
            Self::Reading => "reading",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }

    /// Whether this stage ends the run. `failed` does; `interrupted` is a run
    /// whose owner stopped answering and which another attempt may resume.
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Failed)
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "discovering" => Ok(Self::Discovering),
            "selecting" => Ok(Self::Selecting),
            "reading" => Ok(Self::Reading),
            "interrupted" => Ok(Self::Interrupted),
            "failed" => Ok(Self::Failed),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown import stage {other:?}"),
            }),
        }
    }
}

/// Why an import stopped, as a closed code.
///
/// Closed rather than prose because the console renders a next action from it
/// -- "sign in to Tes on this phone" is a different answer from "update the
/// app" -- and because the sentence beside it is the device's own words and
/// must not be parsed by anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportReasonCode {
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

impl ImportReasonCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingSession => "missing_session",
            Self::NotPermitted => "not_permitted",
            Self::UnsupportedSource => "unsupported_source",
            Self::EnumerationFailed => "enumeration_failed",
            Self::DescriptionFailed => "description_failed",
            Self::SubmissionFailed => "submission_failed",
            Self::ActivationExpired => "activation_expired",
            Self::LeaseExpired => "lease_expired",
            Self::Stopped => "stopped",
            Self::ClientUpdateRequired => "client_update_required",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "missing_session" => Ok(Self::MissingSession),
            "not_permitted" => Ok(Self::NotPermitted),
            "unsupported_source" => Ok(Self::UnsupportedSource),
            "enumeration_failed" => Ok(Self::EnumerationFailed),
            "description_failed" => Ok(Self::DescriptionFailed),
            "submission_failed" => Ok(Self::SubmissionFailed),
            "activation_expired" => Ok(Self::ActivationExpired),
            "lease_expired" => Ok(Self::LeaseExpired),
            "stopped" => Ok(Self::Stopped),
            "client_update_required" => Ok(Self::ClientUpdateRequired),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown import reason code {other:?}"),
            }),
        }
    }
}

/// Who is executing a run and what they have reported.
///
/// Every field is a stored fact rather than an inference, which is the whole
/// point of the table change behind it: before this, `reading` was said by a
/// run from the instant the seller pressed the button, and nothing could tell
/// a device at work from a phone that had been off since Tuesday.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunExecution {
    pub owner_device: Option<String>,
    /// The fence. Zero means nothing has ever claimed this run.
    pub attempt: u64,
    pub lease_expires_at: Option<Timestamp>,
    pub last_contact_at: Option<Timestamp>,
    pub last_progress_at: Option<Timestamp>,
    pub reported_stage: Option<ImportStage>,
    pub reason_code: Option<ImportReasonCode>,
    pub reason: Option<String>,
    pub discovered: u32,
    pub processed: u32,
    pub enumeration_complete: bool,
    /// The frozen denominator of the reading stage, once the selection is
    /// taken.
    pub selected_total: Option<u32>,
    pub commit_authorised_at: Option<Timestamp>,
    pub commit_authorised_by: Option<String>,
    /// Whether the lease was live at the instant this row was read, by the
    /// database's own clock. Read there rather than compared here, because the
    /// two servers that could disagree about a wall clock are exactly the ones
    /// a takeover decides between.
    pub lease_live: bool,
}

impl RunExecution {
    /// Whether a catalogue commit for this run is authorised.
    ///
    /// A scheduled run carries its own separately approved rule; a manual one
    /// needs the seller's stored confirmation. Description completion is
    /// neither, which is why an old `committing` row authorises nothing.
    #[must_use]
    pub const fn commit_authorised(&self, scheduled: bool) -> bool {
        scheduled || self.commit_authorised_at.is_some()
    }
}

/// What a claim or a renewal granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLease {
    pub attempt: u64,
    pub lease_expires_at: Timestamp,
}

/// What a claim answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    Granted(ImportLease),
    /// Another device holds it. Answered rather than refused silently,
    /// because "your other phone is reading this shop" is what the seller
    /// needs to be told.
    HeldBy {
        device: String,
        lease_expires_at: Option<Timestamp>,
    },
    /// The run has settled and is not executed again.
    Settled(RunState),
}

/// Whether a fenced write may proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenceOutcome {
    /// This device holds the run under this attempt, and the run is open.
    Current,
    /// A newer attempt exists, which is what a taken-over device meets.
    Stale { attempt: u64 },
    /// Another device owns the run, or none does.
    NotOwner { owner: Option<String> },
    /// This device holds the run under this attempt, and its lease has
    /// lapsed. A refusal of its own because the remedy is its own: claim
    /// again, which raises the fence and makes the device's readiness fresh
    /// rather than assumed from an hour ago.
    Expired,
    /// The run has settled; a late page changes nothing.
    Settled(RunState),
}

/// What a device's own stop settled, decided under the run's lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopOutcome {
    /// This call abandoned the run.
    Stopped,
    /// The run had already settled. `Abandoned` is the replay's own case and
    /// the caller answers it as success; any other ending is distinct.
    Settled(RunState),
    /// Not this device's attempt to stop. Never `Current` or `Expired`:
    /// both of those proceed.
    Fenced(FenceOutcome),
}

/// What an owner reports about its own progress.
#[derive(Debug, Clone)]
pub struct ProgressReport<'a> {
    pub attempt: u64,
    pub stage: ImportStage,
    pub discovered: u32,
    pub processed: u32,
    pub reason_code: Option<ImportReasonCode>,
    pub reason: Option<&'a str>,
}

/// What one start key was spent on.
///
/// The shop and the retried run together are the key's identity: "start Tes"
/// and "retry that failed Tes run" are different intents, and a replay of one
/// key carrying the other must not be answered with the wrong run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartKeyBinding {
    pub run: Uuid,
    pub source: InventoryId,
    pub retry_of: Option<Uuid>,
    /// Whether the run this key was spent on has been deleted.
    ///
    /// Carried on the binding rather than read separately, because the two
    /// facts are always wanted together: a replayed start has to be answered
    /// with the run its key made, and a key whose run the seller deleted has
    /// to be refused rather than answered with a tombstone or spent again on
    /// a second import of the same shop.
    pub deleted: bool,
}

/// The acknowledgement one page was given, stored so a replay is answered
/// with what the first delivery was told rather than with a recount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptAck {
    pub applied: u32,
    pub skipped: u32,
    pub described_total: u32,
    pub complete: bool,
}

/// What a receipt lookup answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptOutcome {
    /// Never seen. Apply it, then store the acknowledgement.
    Fresh,
    /// Seen, with the same content. Answer with the original acknowledgement
    /// and apply nothing.
    Replay(ReceiptAck),
    /// Seen, with different content under the same key.
    Conflict,
}

/// A run to open.
#[derive(Debug, Clone)]
pub struct NewImportRun {
    pub id: Uuid,
    pub kind: RunKind,
    pub source: Option<InventoryId>,
    pub batch_id: Option<Uuid>,
    pub target: Option<InventoryId>,
    pub anchor_job: JobId,
    pub created_at: Timestamp,
    /// Whether the scheduler's pass opened this run rather than a seller.
    ///
    /// It decides who finishes the run: a scheduled one selects every listed
    /// row itself and is committed by the pass, and a seller's waits for the
    /// seller. See migration 0071 for why that is one bit rather than a
    /// second `kind`.
    pub scheduled: bool,
    /// The client's own key for this press of the button, where the caller
    /// has one.
    ///
    /// Retained across the client's retries and across a native handoff;
    /// only an explicit new import intent mints another. `None` for a
    /// spreadsheet run, whose batch is already its identity, and for the
    /// scheduler's pull, whose interval is.
    pub start_key: Option<Uuid>,
    /// The settled run a seller explicitly asked to retry, where this start
    /// is that retry.
    ///
    /// Never set by the scheduler, by a batch or by an ordinary start: a
    /// terminal run stays terminal and keeps what it recorded, and the retry
    /// is a new run that names its parent.
    pub retry_of: Option<Uuid>,
}

/// What opening a run answered.
///
/// A branch rather than an error, because these are different answers to the
/// seller: this one is now open, one is already open on this shop and this is
/// which, or this start key has already been spent on a different shop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOpening {
    Opened,
    AlreadyOpen(Uuid),
    KeySpent { source: InventoryId },
}

/// What one spreadsheet batch's durable identity says, tombstone included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchIdentity {
    /// The run that reviews this batch.
    pub run: Uuid,
    /// Where a deletion of that run got to, and `None` where nobody deleted
    /// it. A batch whose identity carries one has been imported and removed,
    /// and is not imported again.
    pub deletion: Option<DeletionStatus>,
}

/// What opening a spreadsheet batch's run answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchRunOpening {
    /// This call opened it.
    Opened,
    /// The batch already has a run, and this is which.
    Existing(Uuid),
    /// The batch's import was deleted, so it is not reviewed again.
    Deleted(DeletionStatus),
}

/// One run without its rows, which is what the listing draws.
#[derive(Debug, Clone)]
pub struct ImportRunHead {
    pub id: Uuid,
    pub kind: RunKind,
    pub source: Option<InventoryId>,
    pub batch_id: Option<Uuid>,
    pub target: Option<InventoryId>,
    pub state: RunState,
    pub anchor_job: JobId,
    pub read_total: Option<u32>,
    pub created_at: Timestamp,
    pub settled_at: Option<Timestamp>,
    pub failure_detail: Option<String>,
    /// See [`NewImportRun::scheduled`].
    pub scheduled: bool,
    /// The settled run this one retries, where a seller asked for one.
    pub retry_of: Option<Uuid>,
    /// Where a Delete this run is still working through has got to. Never
    /// `Deleted` on a head a seller can read: the tombstone is filtered in
    /// the head query itself.
    pub deletion: Option<DeletionStatus>,
    /// Who holds this run and what they have reported. See [`RunExecution`].
    pub execution: RunExecution,
}

/// One run with its rows.
#[derive(Debug, Clone)]
pub struct ImportRunRecord {
    pub head: ImportRunHead,
    pub items: Vec<ImportRunItemRecord>,
}

/// One resource of a run, at whatever stage it has reached.
#[derive(Debug, Clone)]
pub struct ImportRunItemRecord {
    pub locator: String,
    pub ordinal: u32,
    pub state: RunItemState,
    /// The reserved product identifier. Present from the row's first write;
    /// the product it names exists only once the commit has run.
    pub product: ProductId,
    /// The `ObservedResource` the device posted, minus the cover bytes.
    pub observed: Option<serde_json::Value>,
    pub title: Option<String>,
    /// What the read said the resource costs. Absent where the read carried no
    /// amount and where the amount was nothing: `Money` is positive by
    /// construction, so a free listing and an unread price are one absence
    /// here, which is the same shape the wire already has.
    pub price: Option<Money>,
    pub cover_hash: Option<ContentHash>,
    /// Which machine described it, which the fingerprint write carries onward
    /// as the assertion it is.
    pub device: Option<String>,
    pub failure_detail: Option<String>,
    pub skip_reason: Option<String>,
}

/// One row of the list the device posts before it describes anything.
#[derive(Debug, Clone)]
pub struct ListedRow {
    pub locator: String,
    pub title: String,
    pub price: Option<Money>,
}

/// One described resource, as the route hands it over.
#[derive(Debug, Clone)]
pub struct ReadItem<'a> {
    pub locator: &'a str,
    pub observed: &'a serde_json::Value,
    pub title: &'a str,
    pub price: Option<Money>,
    pub cover_hash: Option<ContentHash>,
    /// Which machine described it, where one did.
    pub device: Option<&'a str>,
    /// The identifier the product will have, where the caller has already
    /// reserved one.
    ///
    /// A spreadsheet row reserves its own when the commit claims it, and the
    /// matcher has to name that identifier rather than a second one: a pair
    /// answered `different` is remembered by the pair, so a question asked
    /// about an identifier the product never took would be asked again on the
    /// next import. `None` mints one, which is the marketplace read's case --
    /// there the run item's reserved identifier is what `import_one` is given.
    pub product: Option<ProductId>,
}

/// What the seller ticked.
#[derive(Debug, Clone, Copy)]
pub enum Selection<'a> {
    All,
    Locators(&'a [String]),
}

/// Every state a run holds, counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunCounts {
    pub listed: u32,
    pub selected: u32,
    pub read: u32,
    pub matched: u32,
    pub review: u32,
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
}

impl RunCounts {
    /// Items the commit still has to reach: everything described that has not
    /// settled. A `review` item is outstanding, which is what makes a run with
    /// an undecided pair stay `committing` rather than settle behind the
    /// seller's back.
    #[must_use]
    pub const fn outstanding(self) -> u32 {
        self.listed
            .saturating_add(self.selected)
            .saturating_add(self.read)
            .saturating_add(self.matched)
            .saturating_add(self.review)
    }

    /// Items the commit can act on now.
    #[must_use]
    pub const fn committable(self) -> u32 {
        self.matched
    }
}

/// The partial unique indexes the open-run fences are stated by, named here
/// because their violation is the route's own refusal rather than a fault.
///
/// Two of them since migration 0074, which is the point of that change: one
/// open run per marketplace source and one per spreadsheet batch, rather than
/// one per organisation. A seller reading Tes can start TPT, and a scheduled
/// pull for one shop no longer waits a pass for the other.
const ONE_OPEN_PER_SOURCE: &str = "import_run_one_open_per_source";
const ONE_OPEN_PER_BATCH: &str = "import_run_one_open_per_batch";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Tombstones {
    #[default]
    Exclude,
    Include,
}

/// Which runs a head read is about.
///
/// One filter and one query rather than a statement per caller, because every
/// head read now carries twenty-seven columns and five copies of that column
/// list is five places for the execution facts to go missing from one of them.
#[derive(Debug, Clone, Copy, Default)]
struct HeadFilter {
    run: Option<Uuid>,
    source: Option<InventoryId>,
    /// Only the runs that name no shop, which is the spreadsheet way in.
    /// Distinct from `source: None`, which means "any shop": one is a filter
    /// the seller asked for and the other is the absence of one.
    spreadsheet_only: bool,
    batch: Option<Uuid>,
    state: Option<RunState>,
    open_only: bool,
    /// Oldest first rather than newest first. The history's own default is
    /// newest, and this is the seller asking for the other end of it.
    oldest: bool,
    /// Internal reconciliation includes tombstones; seller-facing reads do not.
    tombstones: Tombstones,
    offset: i64,
    limit: i64,
}

/// One run's row, as the one head query answers it.
struct HeadRow {
    id: uuid::Uuid,
    kind: String,
    source: Option<String>,
    batch_id: Option<uuid::Uuid>,
    target: Option<String>,
    state: String,
    anchor_job: uuid::Uuid,
    read_total: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
    settled_at: Option<chrono::DateTime<chrono::Utc>>,
    failure_detail: Option<String>,
    scheduled: bool,
    retry_of: Option<uuid::Uuid>,
    owner_device: Option<String>,
    attempt: i64,
    lease_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    last_contact_at: Option<chrono::DateTime<chrono::Utc>>,
    last_progress_at: Option<chrono::DateTime<chrono::Utc>>,
    reported_stage: Option<String>,
    reason_code: Option<String>,
    reason: Option<String>,
    discovered: i32,
    processed: i32,
    enumeration_complete: bool,
    selected_total: Option<i32>,
    commit_authorised_at: Option<chrono::DateTime<chrono::Utc>>,
    commit_authorised_by: Option<String>,
    /// Whether the lease is live, by the database's own clock.
    lease_live: bool,
    deletion_state: Option<String>,
}

pub struct ImportRunRepo {
    pool: PgPool,
}

impl ImportRunRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Opens a run, or names the one the seller already has for this intent.
    ///
    /// Three answers rather than one, and each is a different thing to tell
    /// the seller. `Opened` is a new import. `AlreadyOpen` is the run they
    /// already have on this shop — which is now a per-source fence rather
    /// than an organisation-wide one, so Tes and TPT advance together.
    /// `KeySpent` is a start key already spent on a different shop, which is
    /// a different intent wearing an answered key.
    ///
    /// The start key is bound in this transaction rather than after it, which
    /// is what makes a retried start one import: the run insert and the
    /// binding stand or fall together, so a lost acknowledgement replayed
    /// after settlement or after an expiry finds the key spent and reaches
    /// the run it already made instead of minting a second one.
    pub async fn create(&self, org: OrgId, new: &NewImportRun) -> Result<RunOpening, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        match insert_run(&mut tx, org, new).await? {
            Inserted::Done => {}
            // The failed insert has aborted this transaction, so the
            // linking — and the key binding that has to go with it —
            // happens in one of its own.
            Inserted::AlreadyOpen => {
                drop(tx);
                return self.link_existing(org, new).await;
            }
        }
        let (Some(start_key), Some(source)) = (new.start_key, new.source) else {
            tx.commit().await?;
            return Ok(RunOpening::Opened);
        };
        match bind_start_key(&mut tx, org, start_key, new, new.id).await? {
            KeyBinding::Bound => {
                tx.commit().await?;
                Ok(RunOpening::Opened)
            }
            // The key was spent while this transaction was opening the run.
            // The rollback is what makes that harmless: the run this call
            // inserted never existed, so there is nothing to abandon and
            // nothing for the seller to see.
            KeyBinding::Held(run) => {
                tx.rollback().await?;
                Ok(RunOpening::AlreadyOpen(run))
            }
            KeyBinding::Spent { source: spent_on } => {
                tx.rollback().await?;
                let _ = source;
                Ok(RunOpening::KeySpent { source: spent_on })
            }
        }
    }

    /// Links this start to the run already open on its shop, and spends its
    /// key on that run.
    ///
    /// The binding matters as much as the answer. Without it a start that was
    /// answered with an existing run held no record of the key it was
    /// answered under: a lost acknowledgement retried after that run settled
    /// found no binding and opened a second import, and the same key could
    /// then be spent on another shop. The key is bound to whatever run the
    /// seller was told about, which is what makes "this key is this import"
    /// true for a linked start too.
    async fn link_existing(
        &self,
        org: OrgId,
        new: &NewImportRun,
    ) -> Result<RunOpening, StorageError> {
        let held = self
            .heads(
                org,
                &HeadFilter {
                    source: new.source,
                    batch: new.batch_id,
                    open_only: true,
                    limit: 1,
                    ..HeadFilter::default()
                },
            )
            .await?;
        let open = held
            .first()
            .ok_or(StorageError::Inconsistent {
                reason: "an open-run index refused an insert and no run is open".to_owned(),
            })?
            .id;
        let Some(start_key) = new.start_key else {
            return Ok(RunOpening::AlreadyOpen(open));
        };
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let outcome = bind_start_key(&mut tx, org, start_key, new, open).await?;
        tx.commit().await?;
        Ok(match outcome {
            KeyBinding::Bound => RunOpening::AlreadyOpen(open),
            // Already spent on this same intent, possibly on an older run of
            // this shop: the key's own answer is the one to give, because that
            // is the run the client was told about.
            KeyBinding::Held(run) => RunOpening::AlreadyOpen(run),
            KeyBinding::Spent { source } => RunOpening::KeySpent { source },
        })
    }

    /// Every run matching one filter, newest first unless the filter asks
    /// otherwise.
    ///
    /// The order is expressed as a pair of `CASE`s over one boolean rather
    /// than as two statements, because two statements would be two copies of
    /// the twenty-seven column list this function exists to have one of. It
    /// costs the `import_run_by_age` index on the oldest-first read, which an
    /// organisation's own run history — bounded at `RUNS_LISTED_MAX` a page
    /// and some hundreds in total — sorts in memory without noticing.
    async fn heads(
        &self,
        org: OrgId,
        filter: &HeadFilter,
    ) -> Result<Vec<ImportRunHead>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            HeadRow,
            "SELECT id, kind, source, batch_id, target, state, anchor_job, read_total, \
                    created_at, settled_at, failure_detail, scheduled, retry_of, owner_device, \
                    attempt, lease_expires_at, last_contact_at, last_progress_at, \
                    reported_stage, reason_code, reason, discovered, processed, \
                    enumeration_complete, selected_total, commit_authorised_at, \
                    commit_authorised_by, \
                    COALESCE(lease_expires_at > now(), false) AS \"lease_live!\", \
                    deletion_state \
               FROM import_run \
              WHERE org_id = $1 \
                AND ($2::uuid IS NULL OR id = $2) \
                AND ($3::text IS NULL OR source = $3) \
                AND (NOT $4 OR source IS NULL) \
                AND ($5::uuid IS NULL OR batch_id = $5) \
                AND ($6::text IS NULL OR state = $6) \
                AND (NOT $7 OR state IN ('reading', 'reviewing', 'committing')) \
                AND ($11 OR deletion_state IS DISTINCT FROM 'deleted') \
              ORDER BY CASE WHEN $8 THEN created_at END ASC, \
                       CASE WHEN NOT $8 THEN created_at END DESC, \
                       id DESC \
              LIMIT $9 OFFSET $10",
            uuid_to_db(org.0),
            filter.run.map(uuid_to_db),
            filter.source.map(inventory_to_db),
            filter.spreadsheet_only,
            filter.batch.map(uuid_to_db),
            filter.state.map(RunState::as_str),
            filter.open_only,
            filter.oldest,
            filter.limit,
            filter.offset,
            filter.tombstones == Tombstones::Include,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.iter().map(head_of).collect()
    }

    /// How many runs one filter matches, across the whole history rather than
    /// one page of it.
    ///
    /// Read beside the page rather than inferred from it, because a pager that
    /// guessed at a total would be inventing the one figure the seller uses to
    /// decide whether the run they are looking for is further back.
    async fn head_total(&self, org: OrgId, filter: &HeadFilter) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let total = sqlx::query_scalar!(
            "SELECT count(*) AS \"total!\" FROM import_run \
              WHERE org_id = $1 \
                AND ($2::text IS NULL OR source = $2) \
                AND (NOT $3 OR source IS NULL) \
                AND ($4::text IS NULL OR state = $4) \
                AND (NOT $5 OR state IN ('reading', 'reviewing', 'committing')) \
                AND ($6 OR deletion_state IS DISTINCT FROM 'deleted')",
            uuid_to_db(org.0),
            filter.source.map(inventory_to_db),
            filter.spreadsheet_only,
            filter.state.map(RunState::as_str),
            filter.open_only,
            filter.tombstones == Tombstones::Include,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(total)
    }

    /// The runs still expecting work, one per source at most.
    ///
    /// A vector rather than an option, which is the cutover migration 0074
    /// states: the organisation-wide singleton is what stopped a seller
    /// reading two shops, and every caller that used to ask for "the" open
    /// run now names the source it means or handles each in turn.
    pub async fn open_runs(&self, org: OrgId) -> Result<Vec<ImportRunHead>, StorageError> {
        self.heads(
            org,
            &HeadFilter {
                open_only: true,
                limit: RUNS_LISTED_MAX,
                ..HeadFilter::default()
            },
        )
        .await
    }

    /// The run still expecting work on one shop, if there is one.
    pub async fn open_for_source(
        &self,
        org: OrgId,
        source: InventoryId,
    ) -> Result<Option<ImportRunHead>, StorageError> {
        Ok(self
            .heads(
                org,
                &HeadFilter {
                    source: Some(source),
                    open_only: true,
                    limit: 1,
                    ..HeadFilter::default()
                },
            )
            .await?
            .into_iter()
            .next())
    }

    /// The run that reviews one spreadsheet batch, as the seller sees it: a
    /// deleted run is not one of them. [`Self::batch_identity`] is the
    /// durable question.
    pub async fn by_batch(
        &self,
        org: OrgId,
        batch: Uuid,
    ) -> Result<Option<ImportRunHead>, StorageError> {
        Ok(self
            .heads(
                org,
                &HeadFilter {
                    batch: Some(batch),
                    limit: 1,
                    ..HeadFilter::default()
                },
            )
            .await?
            .into_iter()
            .next())
    }

    /// What this batch's identity says, tombstone included.
    ///
    /// Separate from [`Self::by_batch`] because the two questions are not the
    /// same question. A seller's page asks "is there a run to show", and a
    /// deleted one is not. A commit asks "has this batch been imported
    /// before", and a deleted one emphatically has: the batch's rows are
    /// preserved, so a lookup that could not see the tombstone opened a
    /// second run against them and recreated the import the seller had just
    /// deleted.
    ///
    /// A deletion anywhere in this batch's history is what is answered,
    /// rather than merely the newest run's state, so no ordering of historic
    /// rows can hide one.
    pub async fn batch_identity(
        &self,
        org: OrgId,
        batch: Uuid,
    ) -> Result<Option<BatchIdentity>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let identity = batch_identity_in_tx(&mut tx, org, batch).await?;
        tx.commit().await?;
        Ok(identity)
    }

    /// Opens the run that reviews one spreadsheet batch, refusing a batch
    /// whose import was deleted.
    ///
    /// One transaction under the batch's identity lock, which is what makes
    /// the refusal sound: the read that decides and the insert that acts on
    /// it cannot have a deletion land between them, so a recommit of a
    /// deleted import's surviving batch meets the tombstone however the two
    /// requests are interleaved.
    pub async fn open_for_batch(
        &self,
        org: OrgId,
        batch: Uuid,
        new: &NewImportRun,
    ) -> Result<BatchRunOpening, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        lock_batch(&mut tx, batch).await?;
        if let Some(identity) = batch_identity_in_tx(&mut tx, org, batch).await? {
            tx.commit().await?;
            return Ok(match identity.deletion {
                Some(deletion) => BatchRunOpening::Deleted(deletion),
                None => BatchRunOpening::Existing(identity.run),
            });
        }
        // Under the lock, with no run for this batch, the only unique index
        // this insert can meet is the one open run per batch — which is the
        // row just read as absent. A conflict here is therefore a lock that
        // did not hold, and is reported rather than papered over with a
        // second lookup.
        match insert_run(&mut tx, org, new).await? {
            Inserted::Done => {
                tx.commit().await?;
                Ok(BatchRunOpening::Opened)
            }
            Inserted::AlreadyOpen => {
                drop(tx);
                Err(StorageError::Inconsistent {
                    reason: "a batch with no run refused its own run under the batch lock"
                        .to_owned(),
                })
            }
        }
    }

    /// One page of the organisation's run history, and how many runs the
    /// filter matches in total.
    ///
    /// Replaces the whole-history read this used to be. Fifty runs was a
    /// bound on the query rather than on the page: a seller's eleventh import
    /// was reachable only by scrolling, and their five-hundredth not at all.
    pub async fn history(
        &self,
        org: OrgId,
        filter: &RunHistoryFilter,
    ) -> Result<RunHistoryPage, StorageError> {
        let heads = HeadFilter {
            source: filter.source,
            spreadsheet_only: filter.spreadsheet_only,
            state: filter.state,
            open_only: filter.open_only,
            oldest: filter.oldest,
            offset: filter.offset.max(0),
            limit: filter.limit.clamp(0, RUNS_LISTED_MAX),
            ..HeadFilter::default()
        };
        Ok(RunHistoryPage {
            runs: self.heads(org, &heads).await?,
            total: self.head_total(org, &heads).await?,
        })
    }

    /// One page of a run's resources, and how many the filter matches.
    ///
    /// The search is a plain substring over the title, falling back to the
    /// locator where the enumeration named nothing else — which is exactly
    /// what the list renders, so a seller searching for what they can see
    /// finds it. `position` rather than `ILIKE` because a title holding a
    /// percent sign is a title, not a wildcard.
    pub async fn items_page(
        &self,
        org: OrgId,
        run: Uuid,
        filter: &ItemPageFilter<'_>,
    ) -> Result<ItemPage, StorageError> {
        let state = filter.state.map(RunItemState::as_str);
        let search = filter.search.filter(|term| !term.trim().is_empty());
        let by_title = filter.order == ItemOrder::Title;
        let limit = filter.limit.clamp(0, ITEMS_LISTED_MAX);
        let offset = filter.offset.max(0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT locator, ordinal, state, product_id, observed, title, price_minor, \
                    price_currency, cover_hash, observed_by_device, failure_detail, \
                    skip_reason \
               FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 \
                AND ($3::text IS NULL OR state = $3) \
                AND ($4::text IS NULL \
                     OR position(lower($4) in lower(COALESCE(title, locator))) > 0) \
              ORDER BY CASE WHEN $5 THEN lower(COALESCE(title, locator)) END ASC, ordinal ASC \
              LIMIT $6 OFFSET $7",
            uuid_to_db(org.0),
            uuid_to_db(run),
            state,
            search,
            by_title,
            limit,
            offset,
        )
        .fetch_all(&mut *tx)
        .await?;
        let total = sqlx::query_scalar!(
            "SELECT count(*) AS \"total!\" FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 \
                AND ($3::text IS NULL OR state = $3) \
                AND ($4::text IS NULL \
                     OR position(lower($4) in lower(COALESCE(title, locator))) > 0)",
            uuid_to_db(org.0),
            uuid_to_db(run),
            state,
            search,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let items = rows
            .into_iter()
            .map(|row| {
                item_of(
                    row.locator,
                    row.ordinal,
                    &row.state,
                    row.product_id,
                    row.observed,
                    row.title,
                    row.price_minor,
                    row.price_currency.as_deref(),
                    row.cover_hash.as_deref(),
                    row.observed_by_device,
                    row.failure_detail,
                    row.skip_reason,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ItemPage { items, total })
    }

    /// The run's head alone, without its rows.
    pub async fn head(&self, org: OrgId, run: Uuid) -> Result<Option<ImportRunHead>, StorageError> {
        Ok(self
            .heads(
                org,
                &HeadFilter {
                    run: Some(run),
                    limit: 1,
                    ..HeadFilter::default()
                },
            )
            .await?
            .into_iter()
            .next())
    }

    /// The run's head, deleted runs included, for a caller whose question is
    /// not the seller's.
    ///
    /// Two readers, and both need the tombstone rather than a not-found. A
    /// device replaying a queued page or stop has to be told the run is over
    /// — a 404 reads to the desktop transport as "this machine is not
    /// registered", which is an outage it retries behind every other owed
    /// post, so the queue never drains. Reconciliation has to be able to
    /// read what it is reconciling. The seller's own history and detail
    /// reads keep using [`Self::head`], which does not see tombstones.
    pub async fn head_internal(
        &self,
        org: OrgId,
        run: Uuid,
    ) -> Result<Option<ImportRunHead>, StorageError> {
        Ok(self
            .heads(
                org,
                &HeadFilter {
                    run: Some(run),
                    limit: 1,
                    tombstones: Tombstones::Include,
                    ..HeadFilter::default()
                },
            )
            .await?
            .into_iter()
            .next())
    }

    /// One run and every row of it.
    pub async fn get(
        &self,
        org: OrgId,
        run: Uuid,
    ) -> Result<Option<ImportRunRecord>, StorageError> {
        let Some(head) = self.head(org, run).await? else {
            return Ok(None);
        };
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let items = sqlx::query!(
            "SELECT locator, ordinal, state, product_id, observed, title, price_minor, \
                    price_currency, cover_hash, observed_by_device, failure_detail, \
                    skip_reason \
               FROM import_run_item WHERE org_id = $1 AND run_id = $2 ORDER BY ordinal",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let items = items
            .into_iter()
            .map(|row| {
                item_of(
                    row.locator,
                    row.ordinal,
                    &row.state,
                    row.product_id,
                    row.observed,
                    row.title,
                    row.price_minor,
                    row.price_currency.as_deref(),
                    row.cover_hash.as_deref(),
                    row.observed_by_device,
                    row.failure_detail,
                    row.skip_reason,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(ImportRunRecord { head, items }))
    }

    /// Writes the list the source named, and the total that list is.
    ///
    /// Idempotent per locator, because a device that resent its list must not
    /// double the run: the insert conflicts on the row's own key and leaves
    /// the first write standing. `read_total` is set from the run's own row
    /// count afterwards, which is what makes a resent list not inflate it.
    pub async fn append_listed(
        &self,
        org: OrgId,
        run: Uuid,
        listed: &[ListedRow],
        at: Timestamp,
    ) -> Result<u32, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = append_listed(&mut tx, org, run, listed, at).await?;
        tx.commit().await?;
        Ok(written)
    }

    /// Records the seller's tick list: what they chose becomes `selected`, and
    /// what they left becomes `skipped` with the reason they left it.
    ///
    /// One statement per direction rather than a read-and-branch, so a
    /// selection naming a locator the run does not hold moves nothing instead
    /// of refusing the whole tick list.
    pub async fn select(
        &self,
        org: OrgId,
        run: Uuid,
        selection: Selection<'_>,
        at: Timestamp,
    ) -> Result<SelectionOutcome, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        lock_org_catalogue(&mut tx, org).await?;
        let outcome = select_items(&mut tx, org, run, selection, at).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// The locators the device is to describe.
    pub async fn selection(&self, org: OrgId, run: Uuid) -> Result<Vec<String>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_scalar!(
            "SELECT locator FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 AND state = 'selected' ORDER BY ordinal",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows)
    }

    /// Stores what one read said, moving the item to `read`.
    ///
    /// Answers whether this was the first delivery. A device that reposts a
    /// page is told zero applied rather than having its description written
    /// twice, which is the replay rule the page route has always had.
    ///
    /// The row is created where the source named no list — a spreadsheet run
    /// has no enumeration step — so this is the one write both sources share.
    pub async fn record_read(
        &self,
        org: OrgId,
        run: Uuid,
        item: &ReadItem<'_>,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let fresh = record_read(&mut tx, org, run, item, at).await?;
        tx.commit().await?;
        Ok(fresh)
    }

    /// The verdict the matcher reached for one described item.
    pub async fn record_verdict(
        &self,
        org: OrgId,
        run: Uuid,
        locator: &str,
        state: RunItemState,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE import_run_item SET state = $4 \
              WHERE org_id = $1 AND run_id = $2 AND locator = $3 \
                AND state IN ('read', 'matched', 'review')",
            uuid_to_db(org.0),
            uuid_to_db(run),
            locator,
            state.as_str(),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One item created its product.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant, run and locator, and the write carries its \
                  own value and its instant; a struct over those six would name the call"
    )]
    pub async fn record_imported(
        &self,
        org: OrgId,
        run: Uuid,
        locator: &str,
        product: ProductId,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE import_run_item \
                SET state = 'imported', product_id = $4, settled_at = $5 \
              WHERE org_id = $1 AND run_id = $2 AND locator = $3",
            uuid_to_db(org.0),
            uuid_to_db(run),
            locator,
            uuid_to_db(product.0),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One item is not being imported, and this is why the seller was told.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant, run and locator, and the write carries its \
                  own value and its instant; a struct over those six would name the call"
    )]
    pub async fn record_skipped(
        &self,
        org: OrgId,
        run: Uuid,
        locator: &str,
        why: &str,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE import_run_item \
                SET state = 'skipped', skip_reason = $4, settled_at = $5, \
                    failure_detail = NULL \
              WHERE org_id = $1 AND run_id = $2 AND locator = $3",
            uuid_to_db(org.0),
            uuid_to_db(run),
            locator,
            why,
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One item could not be described or created.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant, run and locator, and the write carries its \
                  own value and its instant; a struct over those six would name the call"
    )]
    pub async fn record_failed(
        &self,
        org: OrgId,
        run: Uuid,
        locator: &str,
        detail: &str,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        record_failed(&mut tx, org, run, locator, detail, at).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every state this run's rows are in.
    pub async fn counts(&self, org: OrgId, run: Uuid) -> Result<RunCounts, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let counts = counts_of(&mut tx, org, run).await?;
        tx.commit().await?;
        Ok(counts)
    }

    /// The next page of items the commit can act on.
    ///
    /// `matched` only. A `review` item is left where it is until its pair is
    /// decided, which is what makes a parked question stop this item rather
    /// than the import.
    pub async fn commit_page(
        &self,
        org: OrgId,
        run: Uuid,
        limit: i64,
    ) -> Result<Vec<ImportRunItemRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT locator, ordinal, state, product_id, observed, title, price_minor, \
                    price_currency, cover_hash, observed_by_device, failure_detail, \
                    skip_reason \
               FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 AND state = 'matched' \
                AND EXISTS (SELECT 1 FROM import_run r \
                      WHERE r.org_id = import_run_item.org_id \
                        AND r.id = import_run_item.run_id \
                        AND r.deletion_requested_at IS NULL) \
              ORDER BY ordinal LIMIT $3",
            uuid_to_db(org.0),
            uuid_to_db(run),
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                item_of(
                    row.locator,
                    row.ordinal,
                    &row.state,
                    row.product_id,
                    row.observed,
                    row.title,
                    row.price_minor,
                    row.price_currency.as_deref(),
                    row.cover_hash.as_deref(),
                    row.observed_by_device,
                    row.failure_detail,
                    row.skip_reason,
                )
            })
            .collect()
    }

    /// One row of a run, by the locator the source addresses it with.
    pub async fn item(
        &self,
        org: OrgId,
        run: Uuid,
        locator: &str,
    ) -> Result<Option<ImportRunItemRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT locator, ordinal, state, product_id, observed, title, price_minor, \
                    price_currency, cover_hash, observed_by_device, failure_detail, \
                    skip_reason \
               FROM import_run_item WHERE org_id = $1 AND run_id = $2 AND locator = $3",
            uuid_to_db(org.0),
            uuid_to_db(run),
            locator,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| {
            item_of(
                row.locator,
                row.ordinal,
                &row.state,
                row.product_id,
                row.observed,
                row.title,
                row.price_minor,
                row.price_currency.as_deref(),
                row.cover_hash.as_deref(),
                row.observed_by_device,
                row.failure_detail,
                row.skip_reason,
            )
        })
        .transpose()
    }

    /// The run and locator a reserved product identifier belongs to, if any.
    ///
    /// The duplicate routes address a pair by two product identifiers, and one
    /// side of a pair raised during a read is an item whose product does not
    /// exist yet. This is the lookup that turns that identifier back into the
    /// row to skip.
    pub async fn item_by_product(
        &self,
        org: OrgId,
        product: ProductId,
    ) -> Result<Option<(Uuid, String)>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT run_id, locator FROM import_run_item \
              WHERE org_id = $1 AND product_id = $2",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| (uuid_from_db(row.run_id), row.locator)))
    }

    /// Moves the run, settling it where the state is terminal.
    ///
    /// Conditional on the run still being open, and that is not defensive
    /// tidiness: a stop request read an unlocked head a moment ago, and if
    /// the last guarded commit completed in between, an unconditional write
    /// would overwrite `complete` with `abandoned` and replace the instant
    /// and the reason the seller was already given. History a decision was
    /// taken against is not rewritten; the caller is told what actually
    /// stands instead.
    ///
    /// Answers whether this transition was the one applied.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant and run, and the write carries its state, the \
                  sentence a settled run holds and its instant; a struct over those five would \
                  name the call"
    )]
    pub async fn set_state(
        &self,
        org: OrgId,
        run: Uuid,
        state: RunState,
        detail: Option<&str>,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let moved = set_run_state(&mut tx, org, run, state, detail, at).await?;
        tx.commit().await?;
        Ok(moved)
    }

    /// How many of this run's items carry a description, which is what a
    /// taking-over device resumes from. See [`described_of`].
    pub async fn described(&self, org: OrgId, run: Uuid) -> Result<u32, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = described_of(&mut tx, org, run).await?;
        tx.commit().await?;
        Ok(held)
    }

    /// Stops a run this device owns, deciding and writing under one lock.
    ///
    /// One transaction, and that is the whole point of the call existing: a
    /// fence read in one transaction is not authority for a write in
    /// another. Between the two, a second device can take the run over and
    /// raise the attempt, and the stop a phone queued an hour ago would then
    /// abandon work it does not own. [`guard_run`] takes the row `FOR
    /// UPDATE`, so the ownership test, the attempt test and the transition
    /// are one decision against one version of the row.
    ///
    /// A lapsed lease is admitted where a page write would be refused: the
    /// device that owns this attempt may stop it, because stopping takes work
    /// from nobody, and refusing would strand the instruction on a phone that
    /// can no longer renew.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant and run, the fence by device and attempt, and \
                  the settlement carries its instant; a struct over those five would name the \
                  call"
    )]
    pub async fn stop_owned(
        &self,
        org: OrgId,
        run: Uuid,
        device: &str,
        attempt: u64,
        at: Timestamp,
    ) -> Result<Option<StopOutcome>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(guard) = guard_run(&mut tx, org, run).await? else {
            tx.rollback().await?;
            return Ok(None);
        };
        if !guard.state.open() {
            // A settled run this device owned, whose import the seller
            // deleted: this stop is the acknowledgement that closes the
            // deletion's remaining evidence. The retained lease is what said
            // a phone might still be mid-read; the phone has just said it is
            // not, so the hold is released and the deletion reclassified
            // here rather than waiting out the lease's clock in the sweep.
            if guard.deletion_requested_at.is_some()
                && guard.owner_device.as_deref() == Some(device)
                && guard.attempt == attempt
            {
                release_lease(&mut tx, org, run).await?;
                let owned = fence_owned_jobs(
                    &mut tx,
                    org,
                    run,
                    guard.anchor_job,
                    JobFence::Reconcile(Stamp {
                        at,
                        actor: Actor::System(SystemComponent::Device),
                    }),
                )
                .await?;
                let status = run_deletion_status(&mut tx, org, run, owned).await?;
                set_run_deletion(&mut tx, org, run, status).await?;
                tx.commit().await?;
                return Ok(Some(StopOutcome::Settled(guard.state)));
            }
            tx.rollback().await?;
            return Ok(Some(StopOutcome::Settled(guard.state)));
        }
        if guard.owner_device.as_deref() != Some(device) {
            tx.rollback().await?;
            return Ok(Some(StopOutcome::Fenced(FenceOutcome::NotOwner {
                owner: guard.owner_device,
            })));
        }
        if guard.attempt != attempt {
            tx.rollback().await?;
            return Ok(Some(StopOutcome::Fenced(FenceOutcome::Stale {
                attempt: guard.attempt,
            })));
        }
        let moved = set_run_state(
            &mut tx,
            org,
            run,
            RunState::Abandoned,
            Some("you stopped this import"),
            at,
        )
        .await?;
        tx.commit().await?;
        // The row was locked and open when the transition was written, so it
        // applied. A `false` here would mean the lock did not hold, which is
        // ours to report rather than a seller's refusal.
        if moved {
            Ok(Some(StopOutcome::Stopped))
        } else {
            Err(StorageError::Inconsistent {
                reason: "a locked open run refused its own stop".to_owned(),
            })
        }
    }

    /// The resources this run created, in read order.
    ///
    /// What an auto-publish rule acts on. `imported` only: a skipped row
    /// created nothing to publish, and a row still under review has not been
    /// decided, so publishing either would send a resource the seller has not
    /// got.
    pub async fn imported_products(
        &self,
        org: OrgId,
        run: Uuid,
    ) -> Result<Vec<tam_types::ProductId>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT product_id FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 AND state = 'imported' \
                AND product_id IS NOT NULL \
              ORDER BY ordinal",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.product_id)
            .map(|id| tam_types::ProductId(uuid_from_db(id)))
            .collect())
    }

    // ------------------------------------------------------- execution

    /// Claims a run for one device, raising the fence.
    ///
    /// Every comparison is the database's own: the lease is written as
    /// `now() + interval` and read back against `now()`, so two servers whose
    /// wall clocks disagree cannot disagree about whose claim stands. That is
    /// also why a claim raises `attempt` even when the same device reclaims —
    /// a page still in flight from the previous attempt has to fail after a
    /// resume, and a fence that only moved on a change of owner would let it
    /// land.
    ///
    /// A claim is not a marketplace operation and deliberately precedes one:
    /// the device claims first and only then looks for its own shop session,
    /// so "this phone is not signed in to Tes" is a durable report against a
    /// run this device owns rather than a refusal nobody recorded.
    pub async fn claim(
        &self,
        org: OrgId,
        run: Uuid,
        device: &str,
        takeover: bool,
    ) -> Result<ClaimOutcome, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let granted = sqlx::query!(
            "UPDATE import_run \
                SET owner_device = $3, \
                    attempt = attempt + 1, \
                    lease_expires_at = now() + make_interval(secs => $4), \
                    last_contact_at = now() \
              WHERE org_id = $1 AND id = $2 \
                AND state IN ('reading', 'reviewing', 'committing') \
                AND (owner_device IS NULL OR owner_device = $3 OR $5) \
              RETURNING attempt, lease_expires_at AS \"lease_expires_at!\"",
            uuid_to_db(org.0),
            uuid_to_db(run),
            device,
            lease_window(),
            takeover,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = granted {
            tx.commit().await?;
            return Ok(ClaimOutcome::Granted(ImportLease {
                attempt: u64::try_from(row.attempt).unwrap_or(0),
                lease_expires_at: timestamp_from_db(row.lease_expires_at),
            }));
        }
        // Refused, and which refusal it is is what the seller is told: a
        // settled run is over, and a held one names the other phone.
        let held = sqlx::query!(
            "SELECT state, owner_device, lease_expires_at FROM import_run \
              WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let held = held.ok_or(StorageError::Inconsistent {
            reason: "a claim was refused for a run that does not exist".to_owned(),
        })?;
        let state = RunState::from_db(&held.state)?;
        if !state.open() {
            return Ok(ClaimOutcome::Settled(state));
        }
        Ok(ClaimOutcome::HeldBy {
            device: held.owner_device.unwrap_or_default(),
            lease_expires_at: held.lease_expires_at.map(timestamp_from_db),
        })
    }

    /// Extends the owner's hold, under its own fence.
    ///
    /// A lapsed lease is not renewed: the owner has to claim again, which is
    /// what makes a resumed device visible as a new attempt rather than as an
    /// hour of silence nobody can see.
    pub async fn renew(
        &self,
        org: OrgId,
        run: Uuid,
        device: &str,
        attempt: u64,
    ) -> Result<Option<ImportLease>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "UPDATE import_run \
                SET lease_expires_at = now() + make_interval(secs => $5), \
                    last_contact_at = now() \
              WHERE org_id = $1 AND id = $2 AND owner_device = $3 \
                AND attempt = $4 \
                AND state IN ('reading', 'reviewing', 'committing') \
                AND lease_expires_at IS NOT NULL AND lease_expires_at > now() \
              RETURNING attempt, lease_expires_at AS \"lease_expires_at!\"",
            uuid_to_db(org.0),
            uuid_to_db(run),
            device,
            fence_of(attempt),
            lease_window(),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| ImportLease {
            attempt: u64::try_from(row.attempt).unwrap_or(0),
            lease_expires_at: timestamp_from_db(row.lease_expires_at),
        }))
    }

    /// Records what the owner says it is doing, under its own fence.
    ///
    /// Counts never regress and a terminal run never reopens, both in the
    /// statement rather than in a read-then-write: a device that restarted its
    /// own counter is not a shop that shrank, and a page that arrives after a
    /// cancellation must change nothing.
    ///
    /// Answers the run's state after the report, or `None` where the fence,
    /// the owner or the run's own state refused it.
    pub async fn report_progress(
        &self,
        org: OrgId,
        run: Uuid,
        device: &str,
        report: &ProgressReport<'_>,
    ) -> Result<Option<RunState>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "UPDATE import_run \
                SET reported_stage = CASE \
                        WHEN $8::text IS NULL AND reason_code IS NOT NULL \
                            AND $6 <= discovered AND $7 <= processed AND NOT $10 \
                        THEN reported_stage ELSE $5 END, \
                    discovered = GREATEST(discovered, $6), \
                    processed = GREATEST(processed, $7), \
                    last_contact_at = now(), \
                    last_progress_at = CASE \
                        WHEN $6 > discovered OR $7 > processed THEN now() \
                        ELSE last_progress_at END, \
                    reason_code = CASE WHEN $8 IS NOT NULL THEN $8 \
                        WHEN $6 > discovered OR $7 > processed THEN NULL \
                        ELSE reason_code END, \
                    reason = CASE WHEN $8 IS NOT NULL THEN $9 \
                        WHEN $6 > discovered OR $7 > processed THEN NULL \
                        ELSE reason END, \
                    state = CASE WHEN $10 THEN 'failed' ELSE state END, \
                    settled_at = CASE WHEN $10 THEN now() ELSE settled_at END, \
                    failure_detail = CASE WHEN $10 THEN $9 ELSE failure_detail END, \
                    lease_expires_at = CASE WHEN $10 THEN NULL \
                        ELSE now() + make_interval(secs => $11) END \
              WHERE org_id = $1 AND id = $2 AND owner_device = $3 \
                AND attempt = $4 \
                AND state IN ('reading', 'reviewing', 'committing') \
                AND lease_expires_at IS NOT NULL AND lease_expires_at > now() \
              RETURNING state",
            uuid_to_db(org.0),
            uuid_to_db(run),
            device,
            fence_of(report.attempt),
            report.stage.as_str(),
            i32::try_from(report.discovered).unwrap_or(i32::MAX),
            i32::try_from(report.processed).unwrap_or(i32::MAX),
            report.reason_code.map(ImportReasonCode::as_str),
            report.reason,
            report.stage.terminal(),
            lease_window(),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| RunState::from_db(&row.state)).transpose()
    }

    /// Whether this device may still write to this run.
    ///
    /// The same classification the page path performs inside its own
    /// transaction, served here for the routes that answer a device before
    /// they do anything. `None` is a run that does not exist.
    pub async fn fence(
        &self,
        org: OrgId,
        run: Uuid,
        device: &str,
        attempt: u64,
    ) -> Result<Option<FenceOutcome>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let outcome = fenced(&mut tx, org, run, device, attempt).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// The seller's own confirmation that this run's resources are to be
    /// created, with the actor beside the instant.
    ///
    /// Stored separately from the state for the reason migration 0074 gives:
    /// finishing the descriptions is not a decision to add anything, and the
    /// run reaching `committing` used to be read as one.
    pub async fn authorise_commit(
        &self,
        org: OrgId,
        run: Uuid,
        actor: &str,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let moved = sqlx::query!(
            "UPDATE import_run \
                SET state = 'committing', \
                    commit_authorised_at = COALESCE(commit_authorised_at, $4), \
                    commit_authorised_by = COALESCE(commit_authorised_by, $3) \
              WHERE org_id = $1 AND id = $2 \
                AND state IN ('reading', 'reviewing', 'committing')",
            uuid_to_db(org.0),
            uuid_to_db(run),
            actor,
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(moved.rows_affected() > 0)
    }

    /// The runs a server-side drain may commit now.
    ///
    /// A run is drained because it is authorised, not because it stopped
    /// reading: a scheduled run carries its own approved rule and a manual one
    /// carries the seller's stored confirmation. An old `committing` row with
    /// neither is left alone, which is exactly what the migration's backfill
    /// leaves behind.
    pub async fn drainable(&self, org: OrgId) -> Result<Vec<ImportRunHead>, StorageError> {
        Ok(self
            .open_runs(org)
            .await?
            .into_iter()
            .filter(|head| {
                head.state == RunState::Committing
                    && head.execution.commit_authorised(head.scheduled)
            })
            .collect())
    }

    /// The run one start key was spent on, with the shop it was spent for.
    ///
    /// Retained across settlement: a lost acknowledgement replayed after the
    /// run completed or after its activation expired has to reach the run it
    /// already made rather than mint a second one.
    pub async fn start_key_run(
        &self,
        org: OrgId,
        start_key: Uuid,
    ) -> Result<Option<StartKeyBinding>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT k.run_id, k.source, k.retry_of, \
                    r.deletion_requested_at IS NOT NULL AS \"deleted!\" \
               FROM import_run_start_key k \
               JOIN import_run r ON r.org_id = k.org_id AND r.id = k.run_id \
              WHERE k.org_id = $1 AND k.start_key = $2",
            uuid_to_db(org.0),
            uuid_to_db(start_key),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok(StartKeyBinding {
                run: uuid_from_db(row.run_id),
                source: inventory_from_db(&row.source)?,
                retry_of: row.retry_of.map(uuid_from_db),
                deleted: row.deleted,
            })
        })
        .transpose()
    }

    // ----------------------------------------------------- maintenance

    /// Fails every manual run nothing was ever recorded against inside the
    /// activation window, and answers which.
    ///
    /// A scheduled run is untouched however long it waits: it truthfully
    /// awaits a device, and expiring it would abandon work nobody asked to
    /// abandon.
    ///
    /// `attempt = 0` is not the whole of "nobody has done anything here", and
    /// on its own it is a misreading of every run older than the fence. The
    /// attempt counter arrived with 0074, so a pre-fence import carries zero
    /// however much of the seller's shop it holds — and migration 0076 exists
    /// precisely to hand those runs back, frozen selection and described rows
    /// and all, still at zero because the recovery raises no fence. Expiring
    /// on the counter alone settles that work irreversibly on the next pass,
    /// with `activation_expired` against a run nobody was waiting for.
    ///
    /// So the window is decided against what the run has to show instead: a
    /// row that has moved past `listed` is a resource the seller ticked or a
    /// device described, which is work rather than an unanswered start. A
    /// posted list alone is not — it is a shop walked for a seller who never
    /// chose from it, whose `import_run_one_open_per_source` fence is exactly
    /// what this pass exists to lift.
    pub async fn expire_activations(
        &self,
        org: OrgId,
        detail: &str,
    ) -> Result<Vec<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "UPDATE import_run \
                SET state = 'failed', \
                    settled_at = now(), \
                    failure_detail = $3, \
                    reason_code = 'activation_expired', \
                    reason = $3, \
                    reported_stage = 'failed' \
              WHERE org_id = $1 \
                AND state = 'reading' \
                AND NOT scheduled \
                AND attempt = 0 \
                AND created_at <= now() - make_interval(secs => $2) \
                AND NOT EXISTS ( \
                    SELECT 1 FROM import_run_item item \
                     WHERE item.org_id = import_run.org_id \
                       AND item.run_id = import_run.id \
                       AND item.state <> 'listed' \
                ) \
              RETURNING id",
            uuid_to_db(org.0),
            activation_window(),
            detail,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(|row| uuid_from_db(row.id)).collect())
    }

    /// Marks every run whose owner stopped answering as interrupted, and
    /// answers which.
    ///
    /// Not terminal: the resources this run already described are still the
    /// seller's, and another attempt — the same phone coming back, or their
    /// other one taking over — resumes it. What changes is that the console
    /// stops saying a dead device is reading.
    ///
    /// A run whose enumeration closed and whose selection is not yet frozen
    /// is exempt, and that exemption is load-bearing: the seller is choosing
    /// what to import, nothing is owed by any machine while they do, and the
    /// device's keeper ends with its worker. Interrupting there would tell a
    /// seller their import had died at the moment it was waiting for them.
    pub async fn interrupt_stale_leases(&self, org: OrgId) -> Result<Vec<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "UPDATE import_run \
                SET reported_stage = 'interrupted', \
                    reason_code = COALESCE(reason_code, 'lease_expired'), \
                    lease_expires_at = NULL \
              WHERE org_id = $1 \
                AND state IN ('reading', 'reviewing') \
                AND owner_device IS NOT NULL \
                AND lease_expires_at IS NOT NULL \
                AND lease_expires_at <= now() \
                AND NOT (enumeration_complete AND selected_total IS NULL) \
              RETURNING id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(|row| uuid_from_db(row.id)).collect())
    }

    // -------------------------------------------------------- deletion

    /// Stops an import and answers how far the removal got.
    ///
    /// An open run is settled `abandoned` with the seller's own reason, and
    /// that transition is the fence rather than a second mechanism: every
    /// write this run can still receive — a claim, a renewal, a progress
    /// report, a page, a selection, a commit authorisation — is conditional
    /// on the run being `reading`, `reviewing` or `committing`, so a device
    /// mid-walk stops landing anything the moment this commits.
    ///
    /// What it deliberately does not touch is the catalogue. Resources an
    /// earlier chunk already committed are products the seller has, with
    /// their files, labels and fingerprints; they are not artefacts of the
    /// import and deleting the import does not delete them. The run's rows
    /// keep saying which resources those were, which is what a later import
    /// of the same shop reads to recognise them.
    ///
    /// The anchor job goes with it, because it is this run's own ledger row
    /// and would otherwise sit in the seller's publishing history naming an
    /// import they deleted. It carries no items, so it is quiet by
    /// construction — and it is not the only job this import owns. A
    /// scheduled run publishes what it imported, and each publishing job it
    /// minted names it in `job.import_run_id`; those carry items, they can
    /// be claimed, and they are what would otherwise go on writing to a
    /// marketplace after this call answered. Every one of them is fenced
    /// here, under this run's lock, in this transaction.
    ///
    /// What the fence does *not* do is erase the evidence of work in flight.
    /// The run's lease is left exactly where it was: the state transition is
    /// what refuses the device's next claim, renewal, page and report, so
    /// keeping the hold costs nothing — and it is the only record that a
    /// phone is at this moment reading the shop, which is the difference
    /// between answering `stopping` and telling a seller their import is
    /// gone while it is still running. The hold is closed by the device
    /// acknowledging the stop, or by lapsing; the sweep converges it either
    /// way.
    ///
    /// Only where the fence actually interrupted something, mind: a run that
    /// had already settled *before the first Delete* keeps no hold, because a
    /// lease left over from its last page is a clock that has not run down
    /// rather than a phone doing work. That is the `CASE` on
    /// `lease_expires_at` below, and it is what keeps deleting a finished
    /// import the immediate `deleted` it should be.
    ///
    /// Which is why the second press is not the first. A Delete that caught a
    /// device mid-walk has already moved the run to `abandoned`, so a repeat
    /// arriving before the device has acknowledged or lapsed would read its
    /// own transition as "this was settled all along" and clear the very hold
    /// the first press deliberately kept — reporting the import gone while
    /// the phone is still reading it. A run that already carries a deletion
    /// therefore keeps whatever hold it has, and nothing but the device's
    /// stop or the clock retires it. The run terminal before its first Delete
    /// had its hold cleared by that first press and has none to keep.
    ///
    /// `None` is a run this organisation does not have.
    pub async fn delete(
        &self,
        org: OrgId,
        run: Uuid,
        stamp: Stamp,
    ) -> Result<Option<DeletionStatus>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        // The batch's identity lock first, where this run reviews one, so a
        // commit deciding whether that batch already has a run cannot decide
        // it against a run this call is tombstoning. Taken before the row
        // lock and never the other way round, which is the whole of the
        // ordering rule here.
        if let Some(batch) = batch_of(&mut tx, org, run).await? {
            lock_batch(&mut tx, batch).await?;
        }
        // The run's own lock, taken in the order the commit and a device's
        // page take it, so a chunk in flight either completes before this
        // fence or sees it. Held rather than discarded: what the row said
        // *before* the fence is the only account of what was executing when
        // the seller pressed Delete, and the transition below overwrites it.
        let standing = guard_run(&mut tx, org, run).await?;
        let Some(row) = sqlx::query!(
            "UPDATE import_run \
                SET deletion_requested_at = COALESCE(deletion_requested_at, $3), \
                    deletion_actor_kind = COALESCE(deletion_actor_kind, $4), \
                    deletion_actor_id = COALESCE(deletion_actor_id, $5), \
                    deletion_state = COALESCE(deletion_state, 'stopping'), \
                    state = CASE WHEN state IN ('reading', 'reviewing', 'committing') \
                                 THEN 'abandoned' ELSE state END, \
                    settled_at = CASE WHEN state IN ('reading', 'reviewing', 'committing') \
                                      THEN now() ELSE settled_at END, \
                    failure_detail = CASE WHEN state IN ('reading', 'reviewing', 'committing') \
                                          THEN $6 ELSE failure_detail END, \
                    reason_code = CASE WHEN state IN ('reading', 'reviewing', 'committing') \
                                       THEN 'stopped' ELSE reason_code END, \
                    reason = CASE WHEN state IN ('reading', 'reviewing', 'committing') \
                                  THEN $6 ELSE reason END, \
                    lease_expires_at = CASE \
                        WHEN state IN ('reading', 'reviewing', 'committing') \
                             OR deletion_requested_at IS NOT NULL \
                        THEN lease_expires_at ELSE NULL END \
              WHERE org_id = $1 AND id = $2 \
              RETURNING anchor_job",
            uuid_to_db(org.0),
            uuid_to_db(run),
            timestamp_to_db(stamp.at)?,
            stamp.actor.kind(),
            stamp.actor.id(),
            DELETED_IMPORT,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        let owned = fence_owned_jobs(
            &mut tx,
            org,
            run,
            JobId(uuid_from_db(row.anchor_job)),
            JobFence::Delete(stamp),
        )
        .await?;
        let reached = run_deletion_status(&mut tx, org, run, owned).await?;
        // A run the fence caught mid-commit. The chunk that authorisation had
        // already let start may finish the resource it holds — that is what
        // the contract means by an admitted write finishing — and nothing in
        // the row says so afterwards, because the fence is the state it
        // overwrote. So it is carried from the locked read above, and the
        // sweep answers `deleted` once the chunk is done.
        let status = if reached == DeletionStatus::Deleted
            && standing.is_some_and(|guard| guard.state == RunState::Committing)
        {
            DeletionStatus::Stopping
        } else {
            reached
        };
        set_run_deletion(&mut tx, org, run, status).await?;
        tx.commit().await?;
        Ok(Some(status))
    }

    /// Re-reads one import's deletion as it now stands. `None` where nobody
    /// deleted it.
    pub async fn finalise_deletion(
        &self,
        org: OrgId,
        run: Uuid,
        stamp: Stamp,
    ) -> Result<Option<DeletionStatus>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let _locked = guard_run(&mut tx, org, run).await?;
        let Some(row) = sqlx::query!(
            "SELECT anchor_job FROM import_run \
              WHERE org_id = $1 AND id = $2 AND deletion_requested_at IS NOT NULL",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        let owned = fence_owned_jobs(
            &mut tx,
            org,
            run,
            JobId(uuid_from_db(row.anchor_job)),
            JobFence::Reconcile(stamp),
        )
        .await?;
        let status = run_deletion_status(&mut tx, org, run, owned).await?;
        set_run_deletion(&mut tx, org, run, status).await?;
        tx.commit().await?;
        Ok(Some(status))
    }

    /// Sweeps this tenant's open import deletions, and answers how many
    /// reached the tombstone.
    pub async fn finalise_deletions(&self, org: OrgId, stamp: Stamp) -> Result<u64, StorageError> {
        let open = {
            let mut tx = self.pool.begin().await?;
            pin_org(&mut tx, org).await?;
            let rows = sqlx::query_scalar!(
                "SELECT id FROM import_run \
                  WHERE org_id = $1 AND deletion_state IN ('stopping', 'needs_review') \
                  ORDER BY created_at, id",
                uuid_to_db(org.0),
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            rows
        };
        let mut finalised = 0_u64;
        for id in open {
            if self.finalise_deletion(org, uuid_from_db(id), stamp).await?
                == Some(DeletionStatus::Deleted)
            {
                finalised = finalised.saturating_add(1);
            }
        }
        Ok(finalised)
    }

    /// Whether this run carries a deletion, tombstone included. The replay
    /// check a start key needs where no binding is read.
    pub async fn deletion_status(
        &self,
        org: OrgId,
        run: Uuid,
    ) -> Result<Option<DeletionStatus>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let raw = sqlx::query_scalar!(
            "SELECT deletion_state FROM import_run WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        DeletionStatus::parse(raw.flatten().as_deref())
    }
}

/// Whether the run insert landed, or met the index that says one is already
/// open for this shop or this batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inserted {
    Done,
    AlreadyOpen,
}

/// Writes one run's row. One copy of the column list, shared by the ordinary
/// opening and by the batch's own serialised one.
///
/// A unique-index conflict is an answer rather than a fault, and it aborts
/// the transaction: every caller has to abandon this one and decide what the
/// standing run means to it, which is why the branch is returned rather than
/// handled here.
async fn insert_run(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    new: &NewImportRun,
) -> Result<Inserted, StorageError> {
    let inserted = sqlx::query!(
        "INSERT INTO import_run \
           (org_id, id, kind, source, batch_id, target, state, anchor_job, created_at, \
            scheduled, retry_of) \
         VALUES ($1, $2, $3, $4, $5, $6, 'reading', $7, $8, $9, $10)",
        uuid_to_db(org.0),
        uuid_to_db(new.id),
        new.kind.as_str(),
        new.source.map(inventory_to_db),
        new.batch_id.map(uuid_to_db),
        new.target.map(inventory_to_db),
        uuid_to_db(new.anchor_job.0),
        timestamp_to_db(new.created_at)?,
        new.scheduled,
        new.retry_of.map(uuid_to_db),
    )
    .execute(&mut **tx)
    .await;
    match inserted {
        Ok(_) => Ok(Inserted::Done),
        Err(sqlx::Error::Database(database))
            if database.constraint() == Some(ONE_OPEN_PER_SOURCE)
                || database.constraint() == Some(ONE_OPEN_PER_BATCH) =>
        {
            Ok(Inserted::AlreadyOpen)
        }
        Err(error) => Err(error.into()),
    }
}

/// One batch's durable identity, decided inside the caller's transaction.
///
/// A deletion anywhere in this batch's run history is preferred over the
/// newest row, which is the `ORDER BY`'s only job: the question is whether
/// this batch has ever been imported and removed, and an older tombstone
/// answers it as well as a newer one.
async fn batch_identity_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    batch: Uuid,
) -> Result<Option<BatchIdentity>, StorageError> {
    let row = sqlx::query!(
        "SELECT id, deletion_state FROM import_run \
          WHERE org_id = $1 AND batch_id = $2 \
          ORDER BY (deletion_state IS NOT NULL) DESC, created_at DESC, id DESC \
          LIMIT 1",
        uuid_to_db(org.0),
        uuid_to_db(batch),
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        Ok(BatchIdentity {
            run: uuid_from_db(row.id),
            deletion: DeletionStatus::parse(row.deletion_state.as_deref())?,
        })
    })
    .transpose()
}

/// Releases the hold a deleted run retained, its work having been accounted
/// for.
///
/// The counterpart of the lease this fence deliberately does not clear: the
/// hold is outstanding-execution evidence, so it is closed by the device
/// saying it has stopped, or by its own expiry, and never by the act of
/// deleting.
async fn release_lease(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run SET lease_expires_at = NULL WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The sentence a deleted import records where it was still open.
const DELETED_IMPORT: &str = "you deleted this import";

/// What a deleted import is still doing.
///
/// An import writes to this catalogue and to nobody else's marketplace, so
/// the only thing that can hold a deletion open here is work still in
/// progress: a device holding a live lease, or a server commit chunk that
/// authorisation had already let start. `commit_page` is fenced, so no
/// further chunk begins; the one that is running may finish the resource it
/// holds, which is exactly what `stopping` says.
///
/// The status of every job this import owns is folded in, so a run whose
/// ledger rows are not all quiet does not read as tidied away.
async fn run_deletion_status(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    owned: Option<DeletionStatus>,
) -> Result<DeletionStatus, StorageError> {
    let standing = sqlx::query!(
        r#"SELECT
             COALESCE(lease_expires_at > now(), false) AS "held!",
             state = 'committing'                      AS "committing!"
           FROM import_run WHERE org_id = $1 AND id = $2"#,
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .fetch_one(&mut **tx)
    .await?;
    if standing.held || standing.committing {
        return Ok(DeletionStatus::Stopping);
    }
    Ok(match owned {
        None | Some(DeletionStatus::Deleted) => DeletionStatus::Deleted,
        Some(open) => open,
    })
}

async fn set_run_deletion(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    status: DeletionStatus,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run SET deletion_state = $3 WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(run),
        status.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Fences, or re-reads, every job this import owns, and answers the worst
/// state any of them is in.
///
/// Two kinds of job, and neither may be left out. The anchor is the run's own
/// ledger row, itemless by construction. The others are the publishing jobs a
/// scheduled run's publication pass minted, which name this run in
/// `job.import_run_id` — they carry items, they are claimable, and a
/// deletion that fenced only the anchor left them to go on writing to a
/// marketplace after the seller had been told their import was gone.
///
/// Enumerated under this run's lock, in the caller's transaction, against the
/// same column the mint writes while holding that lock: a publication either
/// commits before this enumeration and is in it, or meets the fence and is
/// never minted. There is no third outcome, which is the only reason this can
/// be a plain `SELECT` rather than a retry loop.
///
/// The whole set is then locked in one statement, in `id` order, before any
/// of it is fenced — and that ordering is load-bearing rather than tidy.
/// Fencing a job appends an event, which holds this organisation's event
/// counter to commit; a settling lease takes its own job row first and the
/// counter after. A transaction that fenced one job and then reached for the
/// next would hold the counter and want a row a settle holds, and the
/// deadlock victim could be the settle of a write that had already reached a
/// marketplace. Taking every row this transaction will need up front, in a
/// fixed order, removes the cycle; each fence re-locks its own row, so the
/// pre-lock costs nothing.
async fn fence_owned_jobs(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    anchor: JobId,
    fence: JobFence,
) -> Result<Option<DeletionStatus>, StorageError> {
    let derived = sqlx::query_scalar!(
        "SELECT id FROM job \
          WHERE org_id = $1 AND import_run_id = $2 AND id <> $3",
        uuid_to_db(org.0),
        uuid_to_db(run),
        uuid_to_db(anchor.0),
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut owned: Vec<_> = std::iter::once(uuid_to_db(anchor.0))
        .chain(derived)
        .collect();
    owned.sort_unstable();
    let locked = sqlx::query_scalar!(
        "SELECT id FROM job \
          WHERE org_id = $1 AND id = ANY($2) \
          ORDER BY id FOR UPDATE",
        uuid_to_db(org.0),
        &owned,
    )
    .fetch_all(&mut **tx)
    .await?;
    crate::jobs::lock_job_items(tx, org, &locked).await?;
    let mut worst = None;
    for id in locked {
        let job = JobId(uuid_from_db(id));
        let reached = fence.apply(tx, org, job).await?;
        worst = worse(worst, reached);
    }
    Ok(worst)
}

/// The less finished of two deletion states, `None` being nothing to report.
///
/// `needs_review` outranks `stopping` because an unaccounted write is the one
/// thing a clock must never close, and both outrank `deleted`: an import is
/// removed from a seller's history when *every* job it owns is quiet, not
/// when the quietest one is.
const fn worse(
    held: Option<DeletionStatus>,
    reached: Option<DeletionStatus>,
) -> Option<DeletionStatus> {
    match (held, reached) {
        (Some(DeletionStatus::NeedsReview), _) | (_, Some(DeletionStatus::NeedsReview)) => {
            Some(DeletionStatus::NeedsReview)
        }
        (Some(DeletionStatus::Stopping), _) | (_, Some(DeletionStatus::Stopping)) => {
            Some(DeletionStatus::Stopping)
        }
        (Some(DeletionStatus::Deleted), _) | (_, Some(DeletionStatus::Deleted)) => {
            Some(DeletionStatus::Deleted)
        }
        (None, None) => None,
    }
}

/// The batch one run reviews, without locking it. `None` on a marketplace
/// run, which reviews none.
async fn batch_of(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<Option<Uuid>, StorageError> {
    Ok(sqlx::query_scalar!(
        "SELECT batch_id FROM import_run WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .fetch_optional(&mut **tx)
    .await?
    .flatten()
    .map(uuid_from_db))
}

/// Serialises the decisions made about one spreadsheet batch's identity for
/// the rest of the transaction.
///
/// The decision this protects is "does this batch already have a run", and
/// the two transactions that must not interleave on it are a commit asking
/// the question and a deletion tombstoning the answer. Without it a commit
/// reads no usable run a moment before the delete commits, and opens a second
/// run against the same preserved rows: the seller deletes a half-finished
/// spreadsheet import and recommitting the batch creates the rest of it.
///
/// Advisory and transaction-scoped, for the reason [`lock_org_catalogue`]
/// gives: what has to be serialised is a decision spread over the run table
/// and the batch's own rows, not one row. Taken before [`guard_run`] wherever
/// both are held, which is the ordering rule that keeps the pair deadlock-free.
async fn lock_batch(tx: &mut Transaction<'_, Postgres>, batch: Uuid) -> Result<(), StorageError> {
    let key = uuid_to_db(batch).to_string();
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('import-batch:' || $1, 0))",
        key,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

fn head_of(row: &HeadRow) -> Result<ImportRunHead, StorageError> {
    Ok(ImportRunHead {
        id: uuid_from_db(row.id),
        kind: RunKind::from_db(&row.kind)?,
        source: row.source.as_deref().map(inventory_from_db).transpose()?,
        batch_id: row.batch_id.map(uuid_from_db),
        target: row.target.as_deref().map(inventory_from_db).transpose()?,
        state: RunState::from_db(&row.state)?,
        anchor_job: JobId(uuid_from_db(row.anchor_job)),
        read_total: row
            .read_total
            .map(|total| u32::try_from(total).unwrap_or(0)),
        created_at: timestamp_from_db(row.created_at),
        settled_at: row.settled_at.map(timestamp_from_db),
        failure_detail: row.failure_detail.clone(),
        scheduled: row.scheduled,
        retry_of: row.retry_of.map(uuid_from_db),
        deletion: DeletionStatus::parse(row.deletion_state.as_deref())?,
        execution: RunExecution {
            owner_device: row.owner_device.clone(),
            attempt: u64::try_from(row.attempt).unwrap_or(0),
            lease_expires_at: row.lease_expires_at.map(timestamp_from_db),
            last_contact_at: row.last_contact_at.map(timestamp_from_db),
            last_progress_at: row.last_progress_at.map(timestamp_from_db),
            reported_stage: row
                .reported_stage
                .as_deref()
                .map(ImportStage::from_db)
                .transpose()?,
            reason_code: row
                .reason_code
                .as_deref()
                .map(ImportReasonCode::from_db)
                .transpose()?,
            reason: row.reason.clone(),
            discovered: u32::try_from(row.discovered).unwrap_or(0),
            processed: u32::try_from(row.processed).unwrap_or(0),
            enumeration_complete: row.enumeration_complete,
            selected_total: row
                .selected_total
                .map(|total| u32::try_from(total).unwrap_or(0)),
            commit_authorised_at: row.commit_authorised_at.map(timestamp_from_db),
            commit_authorised_by: row.commit_authorised_by.clone(),
            lease_live: row.lease_live,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn item_of(
    locator: String,
    ordinal: i32,
    state: &str,
    product_id: Option<uuid::Uuid>,
    observed: Option<serde_json::Value>,
    title: Option<String>,
    price_minor: Option<i64>,
    price_currency: Option<&str>,
    cover_hash: Option<&[u8]>,
    device: Option<String>,
    failure_detail: Option<String>,
    skip_reason: Option<String>,
) -> Result<ImportRunItemRecord, StorageError> {
    let price = match (price_minor, price_currency) {
        (Some(minor), Some(currency)) => {
            Some(Money::new(minor, currency_from_db(currency)?).map_err(|_| {
                StorageError::CorruptRow {
                    reason: format!("non-positive import run item price {minor}"),
                }
            })?)
        }
        _ => None,
    };
    Ok(ImportRunItemRecord {
        locator,
        ordinal: u32::try_from(ordinal).unwrap_or(0),
        state: RunItemState::from_db(state)?,
        product: ProductId(uuid_from_db(product_id.ok_or(
            StorageError::CorruptRow {
                reason: "an import run item holds no reserved product identifier".to_owned(),
            },
        )?)),
        observed,
        title,
        price,
        cover_hash: cover_hash.map(hash_from_db).transpose()?,
        device,
        failure_detail,
        skip_reason,
    })
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

/// The lease window, in the units `make_interval` takes.
fn lease_window() -> f64 {
    f64::from(i32::try_from(LEASE_SECS).unwrap_or(60))
}

/// The manual activation window, in the units `make_interval` takes.
fn activation_window() -> f64 {
    f64::from(i32::try_from(MANUAL_ACTIVATION_SECS).unwrap_or(120))
}

/// One attempt, in the column's own width. A fence beyond the column's range
/// cannot equal any stored value, which is the refusal it should be.
fn fence_of(attempt: u64) -> i64 {
    i64::try_from(attempt).unwrap_or(i64::MAX)
}

// -------------------------------------------------- the decision transaction

/// Everything the commit guard reads about a run before it decides anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunGuard {
    pub state: RunState,
    pub scheduled: bool,
    pub source: Option<InventoryId>,
    pub anchor_job: JobId,
    /// Whether a catalogue commit for this run is authorised: the scheduler's
    /// own rule, or the seller's stored confirmation.
    pub commit_authorised: bool,
    pub owner_device: Option<String>,
    pub attempt: u64,
    /// Whether the owner's lease is live, by the database's own clock. Read
    /// there rather than compared against a server's wall clock, because the
    /// two servers that could disagree are exactly the ones a takeover
    /// decides between.
    pub lease_live: bool,
    /// When this import's deletion was requested, at any stage of it.
    ///
    /// Read under the same `FOR UPDATE` as everything else here, which is
    /// what makes it usable as an admission test: a caller that mints work
    /// against this run — a publishing job the scheduler derives from it —
    /// holds the row until its own insert commits, so the deletion either
    /// precedes the mint and refuses it or follows it and fences what it
    /// made.
    pub deletion_requested_at: Option<Timestamp>,
}

/// Serialises this organisation's catalogue decisions for the rest of the
/// transaction.
///
/// An advisory lock rather than a row lock over the catalogue, because what
/// has to be serialised is the *decision* — "is this resource one we already
/// hold" — and the rows that answer it are spread over products, mappings,
/// files and verdicts. It is taken inside the transaction that makes the
/// decision and released by its commit, and nothing slow happens under it:
/// the blob read, the fingerprint and every marketplace fact are settled
/// before the caller opens the transaction. One organisation's imports
/// serialise against each other and against nobody else's.
pub async fn lock_org_catalogue(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
) -> Result<(), StorageError> {
    let key = uuid_to_db(org.0).to_string();
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('import-commit:' || $1, 0))",
        key,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

/// Locks one run's row and answers what it permits.
///
/// `FOR UPDATE` rather than a plain read: cancellation, the drain and a
/// device's page all decide against this row, and the ordering the design
/// requires — a cancellation acknowledged first rejects the commit, a commit
/// that ran first stays visible and the cancellation stops later work — is
/// exactly the order two transactions take this lock in.
pub async fn guard_run(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<Option<RunGuard>, StorageError> {
    let row = sqlx::query!(
        "SELECT state, scheduled, source, anchor_job, owner_device, attempt, \
                commit_authorised_at, \
                COALESCE(lease_expires_at > now(), false) AS \"lease_live!\", \
                deletion_requested_at \
           FROM import_run WHERE org_id = $1 AND id = $2 FOR UPDATE",
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(RunGuard {
        state: RunState::from_db(&row.state)?,
        scheduled: row.scheduled,
        source: row.source.as_deref().map(inventory_from_db).transpose()?,
        anchor_job: JobId(uuid_from_db(row.anchor_job)),
        commit_authorised: row.scheduled || row.commit_authorised_at.is_some(),
        owner_device: row.owner_device,
        attempt: u64::try_from(row.attempt).unwrap_or(0),
        lease_live: row.lease_live,
        deletion_requested_at: row.deletion_requested_at.map(timestamp_from_db),
    }))
}

/// Whether this device may write to this run, deciding against the locked row.
///
/// One implementation for the fence, used by the page path inside its own
/// transaction and by [`ImportRunRepo::fence`] for the routes that answer a
/// device before doing anything.
///
/// A lapsed lease refuses as surely as a stale attempt, and for the same
/// reason: the hold is what says this device is still the one doing the work,
/// and a worker that stopped answering for a minute has to say so again
/// before it writes. Without this a suspended phone could wake an hour later
/// and settle a run the console had already shown as interrupted.
pub async fn fenced(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    device: &str,
    attempt: u64,
) -> Result<Option<FenceOutcome>, StorageError> {
    let Some(guard) = guard_run(tx, org, run).await? else {
        return Ok(None);
    };
    if !guard.state.open() {
        return Ok(Some(FenceOutcome::Settled(guard.state)));
    }
    if guard.owner_device.as_deref() != Some(device) {
        return Ok(Some(FenceOutcome::NotOwner {
            owner: guard.owner_device,
        }));
    }
    if guard.attempt != attempt {
        return Ok(Some(FenceOutcome::Stale {
            attempt: guard.attempt,
        }));
    }
    if !guard.lease_live {
        return Ok(Some(FenceOutcome::Expired));
    }
    Ok(Some(FenceOutcome::Current))
}

/// Notes that the owner spoke, and that something moved.
pub async fn note_contact(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    progressed: bool,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run \
            SET last_contact_at = now(), \
                last_progress_at = CASE WHEN $3 THEN now() ELSE last_progress_at END, \
                reported_stage = CASE WHEN $3 THEN NULL ELSE reported_stage END, \
                reason_code = CASE WHEN $3 THEN NULL ELSE reason_code END, \
                reason = CASE WHEN $3 THEN NULL ELSE reason END \
          WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(run),
        progressed,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Closes the discovery. Only the device saying so closes it, which is why a
/// partial listed page no longer means the shop has been walked.
pub async fn close_enumeration(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run SET enumeration_complete = true \
          WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Whether this receipt has been seen, and with what.
///
/// The attempt is deliberately absent from the identity the caller digests: a
/// device that resumed under a new fence and resent the page it never got an
/// answer for is sending the same page, and telling it otherwise would cost
/// the seller those resources twice.
pub async fn claim_receipt(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    receipt: Uuid,
    identity: &[u8],
) -> Result<ReceiptOutcome, StorageError> {
    let row = sqlx::query!(
        "SELECT identity, applied, skipped, described_total, complete \
           FROM import_run_receipt \
          WHERE org_id = $1 AND run_id = $2 AND receipt = $3",
        uuid_to_db(org.0),
        uuid_to_db(run),
        uuid_to_db(receipt),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(ReceiptOutcome::Fresh);
    };
    if row.identity != identity {
        return Ok(ReceiptOutcome::Conflict);
    }
    Ok(ReceiptOutcome::Replay(ReceiptAck {
        applied: u32::try_from(row.applied).unwrap_or(0),
        skipped: u32::try_from(row.skipped).unwrap_or(0),
        described_total: u32::try_from(row.described_total).unwrap_or(0),
        complete: row.complete,
    }))
}

/// Stores the acknowledgement one page was given, in the transaction that
/// applied it.
pub async fn store_receipt(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    receipt: Uuid,
    stored: &StoredReceipt<'_>,
) -> Result<(), StorageError> {
    sqlx::query!(
        "INSERT INTO import_run_receipt \
           (org_id, run_id, receipt, identity, applied, skipped, described_total, complete, \
            accepted_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        uuid_to_db(org.0),
        uuid_to_db(run),
        uuid_to_db(receipt),
        stored.identity,
        i32::try_from(stored.ack.applied).unwrap_or(i32::MAX),
        i32::try_from(stored.ack.skipped).unwrap_or(i32::MAX),
        i32::try_from(stored.ack.described_total).unwrap_or(i32::MAX),
        stored.ack.complete,
        timestamp_to_db(stored.at)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// One acknowledgement to store, with the content identity it answers for.
#[derive(Debug, Clone, Copy)]
pub struct StoredReceipt<'a> {
    pub identity: &'a [u8],
    pub ack: ReceiptAck,
    pub at: Timestamp,
}

/// The product an earlier import of this shop created for this locator,
/// whether or not it is still in the catalogue.
///
/// The durable provenance every import leaves behind, and the only record of
/// a metadata-only read's identity: migration 0061 admits no mapping onto a
/// product with no live payload, so a resource whose bytes are still
/// uncaptured has no claim on its listing for [`claimed_product_for`] to
/// find. Without this, a second read of the same draft creates a second copy
/// of it, and a re-import after a local delete can never find the first.
///
/// Keyed on the locator the item row holds, which is the address the device
/// reads that shop by, and scoped to the shop through the item's own run: two
/// shops that number their rows the same way are two identities.
///
/// The live one first, then the most recently settled, so a tombstone is
/// answered only when nothing live carries the identity.
///
/// [`claimed_product_for`]: crate::claimed_product_for
pub async fn imported_product_for(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    source: InventoryId,
    locator: &str,
) -> Result<Option<BoundClaim>, StorageError> {
    let row = sqlx::query!(
        "SELECT i.product_id, (p.deleted_at IS NULL) AS \"live!\" \
           FROM import_run_item i \
           JOIN import_run r ON r.org_id = i.org_id AND r.id = i.run_id \
           JOIN product p ON p.org_id = i.org_id AND p.id = i.product_id \
          WHERE i.org_id = $1 AND r.source = $2 AND i.locator = $3 \
            AND i.state = 'imported' \
          ORDER BY p.deleted_at NULLS FIRST, i.settled_at DESC \
          LIMIT 1",
        uuid_to_db(org.0),
        inventory_to_db(source),
        locator,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| {
        let product = row.product_id.ok_or_else(|| StorageError::CorruptRow {
            reason: "an imported run item carries no product".to_owned(),
        })?;
        Ok(BoundClaim {
            product: ProductId(uuid_from_db(product)),
            live: row.live,
        })
    })
    .transpose()
}

/// The survivor of a merge this product was decided into, if any.
pub async fn merged_into(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    product: ProductId,
) -> Result<Option<ProductId>, StorageError> {
    let row = sqlx::query!(
        "SELECT kept_product FROM duplicate_verdict \
          WHERE org_id = $1 AND verdict = 'same' \
            AND $2 IN (product_lo, product_hi) \
            AND kept_product IS NOT NULL AND kept_product <> $2 \
          LIMIT 1",
        uuid_to_db(org.0),
        uuid_to_db(product.0),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row
        .and_then(|row| row.kept_product)
        .map(|kept| ProductId(uuid_from_db(kept))))
}

/// How many questions about this product are still unanswered, read inside
/// the decision transaction.
pub async fn parked_for(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    product: ProductId,
) -> Result<u32, StorageError> {
    let held: i64 = sqlx::query_scalar!(
        "SELECT count(*)::bigint FROM duplicate_verdict \
          WHERE org_id = $1 AND verdict = 'parked' AND $2 IN (product_lo, product_hi)",
        uuid_to_db(org.0),
        uuid_to_db(product.0),
    )
    .fetch_one(&mut **tx)
    .await?
    .unwrap_or(0);
    Ok(u32::try_from(held).unwrap_or(u32::MAX))
}

/// One item created its product, in the transaction that created it.
pub async fn record_imported(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    at: ItemAddress<'_>,
    product: ProductId,
    when: Timestamp,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run_item \
            SET state = 'imported', product_id = $4, settled_at = $5 \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3",
        uuid_to_db(org.0),
        uuid_to_db(at.run),
        at.locator,
        uuid_to_db(product.0),
        timestamp_to_db(when)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// One item is not being imported, in the transaction that decided so.
pub async fn record_skipped(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    at: ItemAddress<'_>,
    why: &str,
    when: Timestamp,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run_item \
            SET state = 'skipped', skip_reason = $4, settled_at = $5, failure_detail = NULL \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3",
        uuid_to_db(org.0),
        uuid_to_db(at.run),
        at.locator,
        why,
        timestamp_to_db(when)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// One item's verdict, in the transaction that reached it.
pub async fn record_verdict(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    at: ItemAddress<'_>,
    state: RunItemState,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE import_run_item SET state = $4 \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3 \
            AND state IN ('read', 'matched', 'review')",
        uuid_to_db(org.0),
        uuid_to_db(at.run),
        at.locator,
        state.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Which row of which run, for the writes a decision transaction makes.
#[derive(Debug, Clone, Copy)]
pub struct ItemAddress<'a> {
    pub run: Uuid,
    pub locator: &'a str,
}

/// Moves the run, in the transaction that moved its items.
#[expect(
    clippy::too_many_arguments,
    reason = "the row is addressed by tenant and run, and the write carries its state, the sentence a settled run holds and its instant; a struct over those five would name this call"
)]
pub async fn set_run_state(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    state: RunState,
    detail: Option<&str>,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let settled = (!state.open()).then_some(timestamp_to_db(at)).transpose()?;
    // Only from an open state. A settled run's state, its instant and the
    // sentence the seller was given are what a decision was already taken
    // against, and a later writer — a stop request that read an unlocked
    // head, a batch settling a run the seller had abandoned — does not get to
    // rewrite them. The caller is told the write did not apply and answers
    // with what stands.
    let moved = sqlx::query!(
        "UPDATE import_run SET state = $3, settled_at = $4, \
                failure_detail = COALESCE($5, failure_detail) \
          WHERE org_id = $1 AND id = $2 \
            AND state IN ('reading', 'reviewing', 'committing')",
        uuid_to_db(org.0),
        uuid_to_db(run),
        state.as_str(),
        settled,
        detail,
    )
    .execute(&mut **tx)
    .await?;
    Ok(moved.rows_affected() > 0)
}

/// What a selection answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// The tick list was taken, and this many rows moved.
    Taken {
        selected: u32,
    },
    /// The shop has not been walked yet, so a selection over it would freeze
    /// a prefix of the seller's own catalogue.
    Discovering,
    /// The run is no longer taking a selection.
    Settled(RunState),
    Missing,
}

/// What binding one start key answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyBinding {
    /// The key is now this run's.
    Bound,
    /// Already spent on this same intent, and this is the run it answered
    /// with.
    Held(Uuid),
    /// Already spent on a different intent.
    Spent { source: InventoryId },
}

/// Spends one start key on one run, in the caller's transaction.
///
/// One write for both starts that reach it — a new run, and a start linked to
/// the run already open on its shop — because the property it holds is the
/// same for both: every accepted start keeps the answer it was given, so a
/// retry after settlement reaches that run rather than minting another.
///
/// The key's identity is the shop and the retried run together: "start Tes"
/// and "retry that failed Tes run" are different intents, and answering one
/// with the other's run would hide a retry the seller asked for.
async fn bind_start_key(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    start_key: Uuid,
    new: &NewImportRun,
    run: Uuid,
) -> Result<KeyBinding, StorageError> {
    let Some(source) = new.source else {
        return Ok(KeyBinding::Bound);
    };
    let bound = sqlx::query!(
        "INSERT INTO import_run_start_key \
           (org_id, start_key, source, run_id, retry_of, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (org_id, start_key) DO NOTHING",
        uuid_to_db(org.0),
        uuid_to_db(start_key),
        inventory_to_db(source),
        uuid_to_db(run),
        new.retry_of.map(uuid_to_db),
        timestamp_to_db(new.created_at)?,
    )
    .execute(&mut **tx)
    .await?;
    if bound.rows_affected() > 0 {
        return Ok(KeyBinding::Bound);
    }
    let held = sqlx::query!(
        "SELECT run_id, source, retry_of FROM import_run_start_key \
          WHERE org_id = $1 AND start_key = $2",
        uuid_to_db(org.0),
        uuid_to_db(start_key),
    )
    .fetch_one(&mut **tx)
    .await?;
    let spent_on = inventory_from_db(&held.source)?;
    if spent_on == source && held.retry_of.map(uuid_from_db) == new.retry_of {
        return Ok(KeyBinding::Held(uuid_from_db(held.run_id)));
    }
    Ok(KeyBinding::Spent { source: spent_on })
}

/// Writes the list the source named, in a transaction the caller owns.
///
/// Idempotent per locator, because a device that resent its list must not
/// double the run: the insert conflicts on the row's own key and leaves the
/// first write standing. `read_total` follows the run's own row count, which
/// is what makes a resent list not inflate it — and, since a listed page may
/// be partial, what makes the total grow as discovery proceeds rather than
/// claiming the shop after one page.
pub async fn append_listed(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    listed: &[ListedRow],
    at: Timestamp,
) -> Result<u32, StorageError> {
    let org_db = uuid_to_db(org.0);
    let run_db = uuid_to_db(run);
    let at_db = timestamp_to_db(at)?;
    let held: i64 = sqlx::query_scalar!(
        "SELECT COALESCE(MAX(ordinal), 0)::bigint FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2",
        org_db,
        run_db,
    )
    .fetch_one(&mut **tx)
    .await?
    .unwrap_or(0);
    let mut ordinal = held;
    let mut written = 0_u32;
    for row in listed {
        ordinal = ordinal.saturating_add(1);
        let inserted = sqlx::query!(
            "INSERT INTO import_run_item \
               (org_id, run_id, locator, ordinal, state, product_id, title, \
                price_minor, price_currency, read_at) \
             VALUES ($1, $2, $3, $4, 'listed', $5, $6, $7, $8, $9) \
             ON CONFLICT (org_id, run_id, locator) DO NOTHING",
            org_db,
            run_db,
            row.locator.as_str(),
            i32::try_from(ordinal).unwrap_or(i32::MAX),
            uuid_to_db(fresh_uuid()),
            row.title.as_str(),
            row.price.map(Money::minor_units),
            row.price.map(|money| currency_to_db(money.currency())),
            at_db,
        )
        .execute(&mut **tx)
        .await?;
        if inserted.rows_affected() == 0 {
            ordinal = ordinal.saturating_sub(1);
        } else {
            written = written.saturating_add(1);
        }
    }
    sqlx::query!(
        "UPDATE import_run SET read_total = \
           (SELECT count(*)::int FROM import_run_item WHERE org_id = $1 AND run_id = $2) \
          WHERE org_id = $1 AND id = $2",
        org_db,
        run_db,
    )
    .execute(&mut **tx)
    .await?;
    Ok(written)
}

/// Records the tick list, in a transaction the caller owns.
///
/// One statement per direction rather than a read-and-branch, so a selection
/// naming a locator the run does not hold moves nothing instead of refusing
/// the whole tick list.
pub async fn select_items(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    selection: Selection<'_>,
    at: Timestamp,
) -> Result<SelectionOutcome, StorageError> {
    let org_db = uuid_to_db(org.0);
    let run_db = uuid_to_db(run);
    let at_db = timestamp_to_db(at)?;
    // The run's own guard, under the lock this transaction holds. Two
    // refusals rather than one, because they are different answers: a
    // selection over a half-discovered shop would freeze a prefix and leave
    // later rows listed forever, and a selection that arrives after the
    // seller stopped the import must not move a terminal run's rows.
    let Some(guard) = guard_run(tx, org, run).await? else {
        return Ok(SelectionOutcome::Missing);
    };
    if guard.state != RunState::Reading {
        return Ok(SelectionOutcome::Settled(guard.state));
    }
    let closed: bool = sqlx::query_scalar!(
        "SELECT enumeration_complete FROM import_run WHERE org_id = $1 AND id = $2",
        org_db,
        run_db,
    )
    .fetch_one(&mut **tx)
    .await?;
    if !closed {
        return Ok(SelectionOutcome::Discovering);
    }
    let chosen: Vec<String> = match selection {
        Selection::All => Vec::new(),
        Selection::Locators(locators) => locators.to_vec(),
    };
    let all = matches!(selection, Selection::All);
    let taken = sqlx::query!(
        "UPDATE import_run_item SET state = 'selected' \
          WHERE org_id = $1 AND run_id = $2 AND state = 'listed' \
            AND ($3 OR locator = ANY($4))",
        org_db,
        run_db,
        all,
        &chosen,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    sqlx::query!(
        "UPDATE import_run_item \
            SET state = 'skipped', skip_reason = 'not chosen', settled_at = $3 \
          WHERE org_id = $1 AND run_id = $2 AND state = 'listed'",
        org_db,
        run_db,
        at_db,
    )
    .execute(&mut **tx)
    .await?;
    // The denominator the reading stage is measured against, frozen here and
    // nowhere else: the selection is the last moment the number can change,
    // and a progress bar whose denominator moves under it is the defect this
    // column exists to stop.
    //
    // It counts the rows this selection actually took, and nothing else. A
    // listing discovery had already skipped as held was never selected, never
    // described and is not part of what the reading stage is measured
    // against; counting it made a one-resource pass read as one of two. And
    // `COALESCE` keeps the first accepted number on a replay, because a
    // resent selection is the same decision rather than a smaller one.
    let accepted = i32::try_from(taken).unwrap_or(i32::MAX);
    sqlx::query!(
        "UPDATE import_run SET selected_total = COALESCE(selected_total, $3) \
          WHERE org_id = $1 AND id = $2",
        org_db,
        run_db,
        accepted,
    )
    .execute(&mut **tx)
    .await?;
    // A selection that owes nothing settles here, in the same transaction
    // that froze it. The seller who ticked nothing, and the shop whose every
    // listing discovery had already skipped as held, both reach this: there
    // is no description for any device to do and no completion any device
    // will send, so a run left open would sit in `selecting` forever and the
    // device's own list would keep offering it work it cannot do.
    let counts = counts_of(tx, org, run).await?;
    if counts.outstanding() == 0 {
        set_run_state(tx, org, run, RunState::Complete, None, at).await?;
    }
    Ok(SelectionOutcome::Taken {
        selected: u32::try_from(taken).unwrap_or(u32::MAX),
    })
}

/// Returns an item a merge skipped to the review it came from.
///
/// The undo's own transition, and it needs its own statement: a merged item
/// is `skipped` and settled, which every other verdict write refuses to move
/// — so without this the reversal reopened the pair and left the resource
/// skipped forever, unreachable by any commit however the seller then
/// answered.
///
/// Only from `skipped`, so nothing else is dragged back out of a terminal
/// state, and only where the caller has already decided the run permits work.
pub async fn reopen_skipped(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    at: ItemAddress<'_>,
) -> Result<bool, StorageError> {
    let moved = sqlx::query!(
        "UPDATE import_run_item \
            SET state = 'review', skip_reason = NULL, settled_at = NULL \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3 AND state = 'skipped'",
        uuid_to_db(org.0),
        uuid_to_db(at.run),
        at.locator,
    )
    .execute(&mut **tx)
    .await?;
    Ok(moved.rows_affected() > 0)
}

/// Stores what one read said, in a transaction the caller owns.
///
/// Answers whether this was the first delivery. A device that reposts a page
/// is told zero applied rather than having its description written twice.
///
/// The row is created where the source named no list — a spreadsheet run has
/// no enumeration step — so this is the one write both sources share.
pub async fn record_read(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    item: &ReadItem<'_>,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let org_db = uuid_to_db(org.0);
    let run_db = uuid_to_db(run);
    let at_db = timestamp_to_db(at)?;
    let held = sqlx::query!(
        "SELECT state FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3",
        org_db,
        run_db,
        item.locator,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(held) = held.as_ref() {
        // Anything past `selected` already holds a description, and
        // overwriting it would move a product's input under a commit that may
        // already have read it.
        if !matches!(held.state.as_str(), "listed" | "selected") {
            return Ok(false);
        }
    }
    let next_ordinal: i64 = sqlx::query_scalar!(
        "SELECT COALESCE(MAX(ordinal), 0)::bigint + 1 FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2",
        org_db,
        run_db,
    )
    .fetch_one(&mut **tx)
    .await?
    .unwrap_or(1);
    sqlx::query!(
        "INSERT INTO import_run_item \
           (org_id, run_id, locator, ordinal, state, product_id, observed, title, \
            price_minor, price_currency, cover_hash, observed_by_device, observed_at, \
            read_at) \
         VALUES ($1, $2, $3, $4, 'read', $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         ON CONFLICT (org_id, run_id, locator) DO UPDATE SET \
           state = 'read', observed = EXCLUDED.observed, title = EXCLUDED.title, \
           price_minor = EXCLUDED.price_minor, \
           price_currency = EXCLUDED.price_currency, \
           cover_hash = EXCLUDED.cover_hash, \
           observed_by_device = EXCLUDED.observed_by_device, \
           observed_at = EXCLUDED.observed_at, read_at = EXCLUDED.read_at",
        org_db,
        run_db,
        item.locator,
        i32::try_from(next_ordinal).unwrap_or(i32::MAX),
        uuid_to_db(item.product.map_or_else(fresh_uuid, |product| product.0)),
        item.observed,
        item.title,
        item.price.map(Money::minor_units),
        item.price.map(|money| currency_to_db(money.currency())),
        item.cover_hash.map(hash_to_db),
        item.device,
        item.device.map(|_| at_db),
        at_db,
    )
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

/// One item's reserved product identifier, read inside a transaction.
///
/// The matcher has to be asked about the identifier the product will actually
/// take, so a page that is describing a row the run already listed uses that
/// row's reserved identifier rather than minting a second one.
pub async fn reserved_product(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    at: ItemAddress<'_>,
) -> Result<Option<ProductId>, StorageError> {
    let row = sqlx::query!(
        "SELECT product_id FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3",
        uuid_to_db(org.0),
        uuid_to_db(at.run),
        at.locator,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row
        .and_then(|row| row.product_id)
        .map(|id| ProductId(uuid_from_db(id))))
}

/// One item's state, read inside a transaction.
pub async fn reserved_state(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    at: ItemAddress<'_>,
) -> Result<Option<RunItemState>, StorageError> {
    let row = sqlx::query!(
        "SELECT state FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2 AND locator = $3",
        uuid_to_db(org.0),
        uuid_to_db(at.run),
        at.locator,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|row| RunItemState::from_db(&row.state)).transpose()
}

/// The run and locator a reserved product identifier belongs to, read inside
/// a transaction.
///
/// One side of a duplicate pair raised during a read is an item whose product
/// does not exist yet; this is the lookup that turns that identifier back
/// into the row a verdict acts on, taken under the same lock as the verdict.
pub async fn item_of_product(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    product: ProductId,
) -> Result<Option<(Uuid, String)>, StorageError> {
    let row = sqlx::query!(
        "SELECT run_id, locator FROM import_run_item \
          WHERE org_id = $1 AND product_id = $2",
        uuid_to_db(org.0),
        uuid_to_db(product.0),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| (uuid_from_db(row.run_id), row.locator)))
}

/// Every state this run's rows are in, read inside a transaction.
pub async fn counts_of(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<RunCounts, StorageError> {
    let rows = sqlx::query!(
        "SELECT state, count(*)::bigint AS held FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2 GROUP BY state",
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut counts = RunCounts::default();
    for row in rows {
        let held = u32::try_from(row.held.unwrap_or(0)).unwrap_or(u32::MAX);
        match RunItemState::from_db(&row.state)? {
            RunItemState::Listed => counts.listed = held,
            RunItemState::Selected => counts.selected = held,
            RunItemState::Read => counts.read = held,
            RunItemState::Matched => counts.matched = held,
            RunItemState::Review => counts.review = held,
            RunItemState::Imported => counts.imported = held,
            RunItemState::Skipped => counts.skipped = held,
            RunItemState::Failed => counts.failed = held,
        }
    }
    Ok(counts)
}

/// How many of this run's items were actually described.
///
/// The description fact itself, not a state and not a reported number.
/// `observed` is written by the read and by nothing else, and
/// `import_run_item_observed_follows_state` is what makes its presence
/// trustworthy: a listed or selected row has nothing to hold, a row skipped
/// at selection was never read, and a row whose source read failed holds a
/// reason instead. So a row carrying `observed` was described, whatever it
/// has since become - matched, in review, imported, merged away, or failed at
/// the commit afterwards.
///
/// A taking-over device has no journal, and this is what tells it how much of
/// the shop is already done. The reported `processed` counter cannot: it
/// counts failures too, so a run that described twenty-five of thirty and
/// failed five reads as thirty done, and a resumed pass whose last five fail
/// reads as having described nothing at all.
pub async fn described_of(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
) -> Result<u32, StorageError> {
    let held = sqlx::query_scalar!(
        "SELECT count(*)::bigint FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2 AND observed IS NOT NULL",
        uuid_to_db(org.0),
        uuid_to_db(run),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(u32::try_from(held.unwrap_or(0)).unwrap_or(u32::MAX))
}

/// One item could not be described or created, written in a transaction the
/// caller owns.
///
/// Conditional on the row not already standing settled, and that condition is
/// the point. The commit records a refusal against an item whose own
/// transaction has already finished, so between the two a stop, a merge or a
/// successful create can land; an unconditional write would overwrite an
/// `imported` row with `failed`, and the seller would be told a resource they
/// hold does not exist. An absent row is inserted, which is the read path's
/// case: a resource that failed before it was ever described has no row yet.
///
/// Answers whether this call settled the row.
#[expect(
    clippy::too_many_arguments,
    reason = "the transaction, the tenant, the run and the locator address the row, and the write carries its sentence and its instant; every caller passes all six"
)]
pub async fn record_failed(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    run: Uuid,
    locator: &str,
    detail: &str,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let org_db = uuid_to_db(org.0);
    let run_db = uuid_to_db(run);
    let at_db = timestamp_to_db(at)?;
    let next_ordinal: i64 = sqlx::query_scalar!(
        "SELECT COALESCE(MAX(ordinal), 0)::bigint + 1 FROM import_run_item \
          WHERE org_id = $1 AND run_id = $2",
        org_db,
        run_db,
    )
    .fetch_one(&mut **tx)
    .await?
    .unwrap_or(1);
    let written = sqlx::query!(
        "INSERT INTO import_run_item \
           (org_id, run_id, locator, ordinal, state, product_id, failure_detail, read_at, \
            settled_at) \
         VALUES ($1, $2, $3, $4, 'failed', $5, $6, $7, $7) \
         ON CONFLICT (org_id, run_id, locator) DO UPDATE SET \
           state = 'failed', failure_detail = EXCLUDED.failure_detail, \
           skip_reason = NULL, settled_at = EXCLUDED.settled_at \
         WHERE import_run_item.state NOT IN ('imported', 'skipped', 'failed')",
        org_db,
        run_db,
        locator,
        i32::try_from(next_ordinal).unwrap_or(i32::MAX),
        uuid_to_db(fresh_uuid()),
        detail,
        at_db,
    )
    .execute(&mut **tx)
    .await?;
    Ok(written.rows_affected() == 1)
}
