//! A spreadsheet the seller filled, parsed and held before anything is created.
//!
//! The batch is the unit the seller sees; the rows are what make it resumable.
//! Everything this repository writes is per row, for the reason
//! [`crate::sync_requests`] gives about its own: the commit that turns a row
//! into a product is not atomic internally and mints a fresh product id on
//! every pass, so a commit resumed after a closed browser has to know which
//! rows it already created or it inserts a second product no unique index
//! refuses.
//!
//! Nothing here creates a product, a label or a job. A parse writes the batch
//! and its rows, and that is the whole of its effect.

use sqlx::PgPool;
use tam_types::{InventoryId, MappingId, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    inventory_from_db, inventory_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db,
    uuid_to_db,
};
use crate::{pin_org, StorageError};

/// The partial unique index migration 0058 keeps one open batch under, named
/// here because its violation is a write's own answer rather than a fault.
const ONE_OPEN_PER_ORG: &str = "import_batch_one_open_per_org";

/// How many batches the listing answers with.
///
/// The Import page renders a panel of recent batches rather than a history, and
/// a batch is settled or swept within [`tam_limits::import::BATCH_EXPIRY_DAYS`],
/// so the panel is a working surface rather than an archive. Bounded because
/// the listing has no cursor: nothing above this is reachable from the console,
/// and adding a page to a panel nobody scrolls would be a cursor built for no
/// reader.
pub const BATCHES_LISTED_MAX: i64 = 50;

/// Where one batch stands.
///
/// `Parsed` holds a report and creates nothing. `Attaching` is a batch whose
/// live rows are collecting their files. `Importing` is a commit in progress,
/// which is chunked, so it is a state the seller can close the tab on.
/// `Imported`, `Failed` and `Abandoned` are settled; the last is what the
/// expiry sweep writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchState {
    Parsed,
    Attaching,
    Importing,
    Imported,
    Failed,
    Abandoned,
}

impl BatchState {
    /// Every state, so a reader can be exhaustive over the vocabulary the
    /// column's CHECK holds. Test-only scaffolding in this crate; the wire
    /// rendering in `tam-api` is the other consumer.
    pub const ALL: [Self; 6] = [
        Self::Parsed,
        Self::Attaching,
        Self::Importing,
        Self::Imported,
        Self::Failed,
        Self::Abandoned,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parsed => "parsed",
            Self::Attaching => "attaching",
            Self::Importing => "importing",
            Self::Imported => "imported",
            Self::Failed => "failed",
            Self::Abandoned => "abandoned",
        }
    }

    /// Whether a row of this batch may still be bound to bytes.
    ///
    /// Narrower than [`Self::is_open`] by exactly one state, and the state is
    /// `Importing`: a commit in flight has already passed the gate that every
    /// marketplace row holds a file, and a handle replaced under it would
    /// either be created twice or not at all depending on which chunk claimed
    /// the row first.
    #[must_use]
    pub const fn admits_attachment(self) -> bool {
        matches!(self, Self::Parsed | Self::Attaching)
    }

    /// Whether this state is one the "one open batch per organisation" index
    /// counts. Stated here rather than restated in SQL beside every query: the
    /// index's own predicate is the authority, and this is the reader's copy
    /// of it, held against the database by `the_open_states_match_the_index`.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Parsed | Self::Attaching | Self::Importing)
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "parsed" => Ok(Self::Parsed),
            "attaching" => Ok(Self::Attaching),
            "importing" => Ok(Self::Importing),
            "imported" => Ok(Self::Imported),
            "failed" => Ok(Self::Failed),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(StorageError::CorruptRow {
                reason: format!("import_batch.state holds {other}, which is not a batch state"),
            }),
        }
    }
}

/// Where one row of a batch stands.
///
/// `Creating` is the commit's own: a row whose product and mapping identifiers
/// are reserved and whose product does not exist yet. It is what makes a
/// resumed commit read whether it already created this row rather than mint a
/// second product for it, which is the hazard migration 0058's header names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Parsed,
    Attached,
    Creating,
    Created,
    Published,
    Failed,
    Skipped,
}

impl RowState {
    pub const ALL: [Self; 7] = [
        Self::Parsed,
        Self::Attached,
        Self::Creating,
        Self::Created,
        Self::Published,
        Self::Failed,
        Self::Skipped,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parsed => "parsed",
            Self::Attached => "attached",
            Self::Creating => "creating",
            Self::Created => "created",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    /// Whether this row may still be bound to bytes, or unbound from them.
    ///
    /// A row the parse refused can never be created, so binding to it would be
    /// work the seller loses; a row the commit has claimed already names the
    /// bytes it was created under.
    #[must_use]
    pub const fn admits_attachment(self) -> bool {
        matches!(self, Self::Parsed | Self::Attached)
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "parsed" => Ok(Self::Parsed),
            "attached" => Ok(Self::Attached),
            "creating" => Ok(Self::Creating),
            "created" => Ok(Self::Created),
            "published" => Ok(Self::Published),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            other => Err(StorageError::CorruptRow {
                reason: format!("import_batch_row.state holds {other}, which is not a row state"),
            }),
        }
    }
}

/// What the sheet's status column said this row should become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIntent {
    Draft,
    Live,
}

impl RowIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Live => "live",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "draft" => Ok(Self::Draft),
            "live" => Ok(Self::Live),
            other => Err(StorageError::CorruptRow {
                reason: format!("import_batch_row.intent holds {other}, which is not an intent"),
            }),
        }
    }
}

/// The bytes bound to one row, as `POST /{version}/uploads` returned them.
///
/// One value rather than three nullable fields, matching the
/// `import_batch_row_file_handle_total` constraint: a hash without its length
/// would make the commit guess at a number the create checks against the stored
/// blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFile {
    pub hash: tam_types::ContentHash,
    pub kind: String,
    pub byte_len: i64,
}

/// One batch as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBatchRecord {
    pub id: Uuid,
    pub source_name: String,
    pub state: BatchState,
    pub row_count: u32,
    pub live_count: u32,
    pub failed_count: u32,
    pub created_at: Timestamp,
    /// When the sweep will settle this batch and release the bytes its rows
    /// hold. Stated to the seller, which is why it is a stored instant rather
    /// than a sum computed at render time.
    pub expires_at: Timestamp,
    pub settled_at: Option<Timestamp>,
    pub failure_detail: Option<String>,
}

/// One row of a batch, without its draft.
///
/// The draft is absent rather than empty, for the reason
/// [`crate::resource_templates::ResourceTemplateSummary`] gives: the batch page
/// renders the report, which is the row numbers and their refusals, and a
/// five-hundred-row batch of drafts at the column's own ceiling would make one
/// page load serialise tens of megabytes nothing on it displays. The commit
/// reads drafts through [`ImportBatchRepo::draft_page`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBatchRowRecord {
    pub sheet: String,
    /// The seller's own spreadsheet row number.
    pub ordinal: u32,
    pub inventory: Option<InventoryId>,
    pub intent: RowIntent,
    /// Every refusal the parse raised against this row, as the report renders
    /// them.
    pub problems: serde_json::Value,
    /// The labels this row's draft names.
    ///
    /// Lifted out of the document rather than the document being shipped: the
    /// batch page warns about labels the sheet would newly create, and reading
    /// one array per row costs nothing where reading five hundred whole drafts
    /// would make one page load serialise tens of megabytes.
    pub labels: Vec<String>,
    pub file_name: Option<String>,
    pub file: Option<RowFile>,
    pub state: RowState,
    pub product_id: Option<ProductId>,
    pub mapping_id: Option<MappingId>,
    pub job_id: Option<Uuid>,
    pub failure_detail: Option<String>,
}

/// One row with its draft, which is what a commit needs and a report does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBatchDraftRecord {
    pub sheet: String,
    pub ordinal: u32,
    pub inventory: Option<InventoryId>,
    pub intent: RowIntent,
    pub draft: serde_json::Value,
    pub file: Option<RowFile>,
    /// The thumbnail the upload generated beside the payload. Carried because
    /// the create writes it and a resource created without one has no
    /// thumbnail anywhere it is later listed.
    pub cover: Option<RowFile>,
    pub state: RowState,
}

/// What a new batch carries. The identifier, the instants and the counts are
/// the caller's: the identifier is the submit's idempotency key, and a test
/// drives the instants from a fixed clock rather than an ambient one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImportBatch<'a> {
    pub id: Uuid,
    pub source_name: &'a str,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub rows: &'a [NewImportBatchRow<'a>],
}

/// One parsed row, as the parser produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImportBatchRow<'a> {
    pub sheet: &'a str,
    pub ordinal: u32,
    pub inventory: Option<InventoryId>,
    pub intent: RowIntent,
    pub draft: &'a serde_json::Value,
    /// The refusals this row drew, as a JSON array. A non-empty array makes
    /// the row `failed`, which the column's own CHECK also holds.
    pub problems: &'a serde_json::Value,
    pub file_name: Option<&'a str>,
}

impl NewImportBatchRow<'_> {
    fn state(&self) -> RowState {
        match self.problems.as_array() {
            Some(problems) if problems.is_empty() => RowState::Parsed,
            // A `problems` value that is not an array at all is refused by the
            // column's own CHECK; reading it as a refused row here keeps the
            // two answers from disagreeing about a document neither accepts.
            _ => RowState::Failed,
        }
    }
}

/// What a create did. Neither alternative is an error variant, for the reason
/// [`crate::resource_templates::TemplateWrite`] states: an organisation that
/// already has an open batch, and a re-posted idempotency key, are answers a
/// seller acts on rather than faults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchWrite {
    Saved(ImportBatchRecord),
    /// This upload's idempotency key already named a batch, which is what a
    /// double-clicked submit looks like. The batch is the first submit's, and
    /// this upload's rows were not appended to it.
    Replay(ImportBatchRecord),
    /// This organisation already has an open batch. Carries it, because the
    /// refusal names the open batch and links to it rather than telling the
    /// seller to go and find it.
    AlreadyOpen(Box<ImportBatchRecord>),
}

/// One row's address inside a batch.
///
/// The tab it came off and the seller's own spreadsheet row number, which are
/// two thirds of `import_batch_row`'s primary key and never travel apart: a
/// sheet without its ordinal names a tab and an ordinal without its sheet
/// names a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowAddress<'a> {
    pub sheet: &'a str,
    pub ordinal: u32,
}

/// The two handles a bind writes.
///
/// One value because the upload answers both at once and a row holding one
/// without the other is a resource with a thumbnail of nothing, or none at
/// all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowFiles<'a> {
    pub payload: &'a RowFile,
    pub cover: &'a RowFile,
}

/// How much of a batch holds bytes, and how much still needs them.
///
/// Both counts are over the rows a commit could still take -- `parsed` and
/// `attached` -- so a settled batch's history does not move them. `awaiting`
/// is D32's rule counted: a row that passed the parse and names a marketplace
/// needs bytes whether it asked for draft or live, and a Teachouse row needs
/// none. It is therefore the commit's own gate as well as the panel's readout,
/// stated once so the two cannot disagree about which rows are outstanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AttachCounts {
    pub attached: u32,
    pub awaiting: u32,
}

/// One row after a bind, with everything the panel renders without re-reading
/// the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundRow {
    pub row: ImportBatchRowRecord,
    pub batch_state: BatchState,
    pub counts: AttachCounts,
}

/// What a bind did. None of the alternatives is an error variant, for the
/// reason [`BatchWrite`] gives: each is a sentence the seller acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindOutcome {
    Bound(Box<BoundRow>),
    /// This organisation holds no such batch, or the batch holds no such row.
    /// One answer for both, because telling the two apart would say whether a
    /// batch identifier a caller guessed exists.
    NoSuchRow,
    /// The batch takes no more files: it is settled, or a commit is already
    /// running over it.
    BatchClosed(BatchState),
    /// The row takes no handle in the state it is in.
    RowClosed(RowState),
}

/// What an unbind did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnbindOutcome {
    /// The row holds no handle now. Answered whether or not it held one
    /// before, because clearing what is already clear is what a double-clicked
    /// button sends rather than a fault.
    Cleared,
    NoSuchRow,
    BatchClosed(BatchState),
}

/// One row named by its address alone, as a refusal lists them.
///
/// Owned where [`RowAddress`] borrows, because this one travels out of the
/// statement that read it and into the sentence a seller reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowRef {
    pub sheet: String,
    pub ordinal: u32,
}

/// How many rows a refusal names before it stops naming them.
///
/// The seller fixes the first few and presses the button again, so a list
/// longer than a screen is a list nobody reads; the count beside it is the
/// whole answer to "how many are left".
const AWAITING_LISTED_MAX: i64 = 20;

/// What opening a commit did.
///
/// None of the alternatives is an error variant, for the reason [`BatchWrite`]
/// gives: each is a sentence the seller acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOpening {
    /// The batch is `importing` and its rows may be claimed. Answered for the
    /// chunk that moved it there and for every chunk after, because a commit
    /// is chunked and only the first transition is a transition.
    Open,
    NoSuchBatch,
    /// The batch is settled: imported, failed or abandoned. Carries the state,
    /// because what the seller does next differs by which one it is.
    BatchClosed(BatchState),
    /// D32's gate: a row that passed the parse and names a marketplace holds
    /// no bytes. The count is every such row and the list is the first
    /// [`AWAITING_LISTED_MAX`] of them.
    Awaiting {
        count: u32,
        rows: Vec<RowRef>,
    },
}

/// One row the commit has claimed, with the identifiers reserved for it.
///
/// The draft rather than the report's columns, because this is what a create
/// is built from; the two handles, because the create writes both; and the
/// identifiers, because they are reserved before the create runs and are what
/// makes a resumed pass finish this row rather than mint a second product for
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedRow {
    pub sheet: String,
    pub ordinal: u32,
    pub inventory: Option<InventoryId>,
    pub intent: RowIntent,
    pub draft: serde_json::Value,
    pub file: Option<RowFile>,
    pub cover: Option<RowFile>,
    pub product: ProductId,
    /// Reserved exactly where the row names an inventory, which is what
    /// `import_batch_row_mapping_follows_inventory` holds.
    pub mapping: Option<MappingId>,
}

/// Where a batch's rows stand part way through a commit.
///
/// `outstanding` is the commit's own completion test: zero is done, and it
/// counts the claimed-but-unfinished rows as well as the unclaimed ones, so a
/// pass that crashed between the claim and the create leaves a batch that
/// reads unfinished rather than one that reads complete with a row missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommitCounts {
    pub outstanding: u32,
    /// Rows this batch put in the catalogue.
    pub created: u32,
    /// Rows a pass claimed and did not create, because an earlier pass already
    /// had. Counted apart from `created` so a re-commit of a sheet already
    /// imported does not report itself as work done.
    pub skipped: u32,
    pub failed: u32,
}

/// What one expiry pass settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SweepReport {
    /// Batches settled to `abandoned`.
    pub abandoned: u64,
    /// Rows whose file handles were released, counted once per row rather
    /// than once per handle.
    pub released: u64,
}

/// What one pass of the stale-commit rule did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StaleReport {
    /// Batches a dead pass left `importing` with every row settled, now
    /// `imported` or `failed` with the seller's notification written.
    pub settled: u64,
    /// Batches a dead pass left `importing` with rows still to create,
    /// returned to the seller to finish.
    pub reopened: u64,
}

pub struct ImportBatchRepo {
    pool: PgPool,
}

impl ImportBatchRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes the batch and every parsed row in one transaction.
    ///
    /// One statement for the rows rather than one per row: the row ceiling is
    /// five hundred, and five hundred round trips inside one transaction is a
    /// slow answer to an upload the seller is watching. The batch's own counts
    /// are computed here from the rows it is handed, so a caller cannot state a
    /// `live_count` the rows do not support.
    pub async fn create(
        &self,
        org: OrgId,
        new: &NewImportBatch<'_>,
    ) -> Result<BatchWrite, StorageError> {
        let row_count = count_to_db(new.rows.len())?;
        let live_count = count_to_db(
            new.rows
                .iter()
                .filter(|row| row.intent == RowIntent::Live)
                .count(),
        )?;
        let failed_count = count_to_db(
            new.rows
                .iter()
                .filter(|row| row.state() == RowState::Failed)
                .count(),
        )?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO import_batch \
             (org_id, id, source_name, state, row_count, live_count, failed_count, \
              created_at, expires_at) \
             VALUES ($1, $2, $3, 'parsed', $4, $5, $6, $7, $8) \
             ON CONFLICT (org_id, id) DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(new.id),
            new.source_name,
            row_count,
            live_count,
            failed_count,
            timestamp_to_db(new.created_at)?,
            timestamp_to_db(new.expires_at)?,
        )
        .execute(&mut *tx)
        .await;
        let written = match written {
            Ok(written) => written,
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some(ONE_OPEN_PER_ORG) =>
            {
                drop(tx);
                let open = self.open(org).await?.ok_or_else(|| {
                    // The index refused because an open batch exists, so one
                    // exists; a read that then finds none means it was settled
                    // between the two statements, which is a race the caller
                    // resolves by retrying rather than a row to invent.
                    StorageError::Inconsistent {
                        reason: "the open-batch index refused a write and no open batch is \
                                 readable; retry the upload"
                            .to_owned(),
                    }
                })?;
                return Ok(BatchWrite::AlreadyOpen(Box::new(open)));
            }
            Err(error) => return Err(error.into()),
        };
        if written.rows_affected() == 0 {
            // The rows belong to the submit that created the batch, so a
            // replay writes none of them: appending would give the first
            // submit's batch a second copy of every row, and the primary key
            // would refuse only where the sheet and ordinal collided.
            drop(tx);
            let held = self
                .get(org, new.id)
                .await?
                .ok_or(StorageError::Inconsistent {
                    reason: "the batch key conflicted and the batch is not readable".to_owned(),
                })?;
            return Ok(BatchWrite::Replay(held));
        }

        let sheets: Vec<String> = new.rows.iter().map(|row| row.sheet.to_owned()).collect();
        let ordinals: Vec<i32> = new
            .rows
            .iter()
            .map(|row| ordinal_to_db(row.ordinal))
            .collect::<Result<_, _>>()?;
        // The absent inventory and the absent filename travel as the empty
        // string and are restored by `NULLIF` in the statement below, because
        // an array bound through the query macro is an array of non-null
        // elements. Neither column can hold an empty string for real: an
        // inventory code round-trips through `InventoryId`, and `file_name` is
        // CHECKed at one character or more.
        let inventories: Vec<String> = new
            .rows
            .iter()
            .map(|row| {
                row.inventory
                    .map_or_else(String::new, |id| inventory_to_db(id).to_owned())
            })
            .collect();
        let intents: Vec<String> = new
            .rows
            .iter()
            .map(|row| row.intent.as_str().to_owned())
            .collect();
        let drafts: Vec<serde_json::Value> = new.rows.iter().map(|row| row.draft.clone()).collect();
        let problems: Vec<serde_json::Value> =
            new.rows.iter().map(|row| row.problems.clone()).collect();
        let file_names: Vec<String> = new
            .rows
            .iter()
            .map(|row| row.file_name.unwrap_or_default().to_owned())
            .collect();
        let states: Vec<String> = new
            .rows
            .iter()
            .map(|row| row.state().as_str().to_owned())
            .collect();

        sqlx::query!(
            "INSERT INTO import_batch_row \
             (org_id, batch_id, sheet, ordinal, inventory, intent, draft, problems, \
              file_name, state) \
             SELECT $1, $2, r.sheet, r.ordinal, NULLIF(r.inventory, ''), r.intent, \
                    r.draft, r.problems, NULLIF(r.file_name, ''), r.state \
               FROM UNNEST($3::text[], $4::int[], $5::text[], $6::text[], $7::jsonb[], \
                           $8::jsonb[], $9::text[], $10::text[]) \
                 AS r(sheet, ordinal, inventory, intent, draft, problems, file_name, state)",
            uuid_to_db(org.0),
            uuid_to_db(new.id),
            &sheets,
            &ordinals,
            &inventories,
            &intents,
            &drafts,
            &problems,
            &file_names,
            &states,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(BatchWrite::Saved(ImportBatchRecord {
            id: new.id,
            source_name: new.source_name.to_owned(),
            state: BatchState::Parsed,
            row_count: count_from_db(row_count)?,
            live_count: count_from_db(live_count)?,
            failed_count: count_from_db(failed_count)?,
            created_at: new.created_at,
            expires_at: new.expires_at,
            settled_at: None,
            failure_detail: None,
        }))
    }

    /// This organisation's batches, newest first.
    pub async fn list(&self, org: OrgId) -> Result<Vec<ImportBatchRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, source_name, state, row_count, live_count, failed_count, \
                    created_at, expires_at, settled_at, failure_detail \
               FROM import_batch \
              WHERE org_id = $1 \
              ORDER BY created_at DESC, id \
              LIMIT $2",
            uuid_to_db(org.0),
            BATCHES_LISTED_MAX,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ImportBatchRecord {
                    id: uuid_from_db(row.id),
                    source_name: row.source_name,
                    state: BatchState::from_db(&row.state)?,
                    row_count: count_from_db(row.row_count)?,
                    live_count: count_from_db(row.live_count)?,
                    failed_count: count_from_db(row.failed_count)?,
                    created_at: timestamp_from_db(row.created_at),
                    expires_at: timestamp_from_db(row.expires_at),
                    settled_at: row.settled_at.map(timestamp_from_db),
                    failure_detail: row.failure_detail,
                })
            })
            .collect()
    }

    /// One batch, or none where this organisation holds no such batch.
    pub async fn get(
        &self,
        org: OrgId,
        id: Uuid,
    ) -> Result<Option<ImportBatchRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, source_name, state, row_count, live_count, failed_count, \
                    created_at, expires_at, settled_at, failure_detail \
               FROM import_batch WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ImportBatchRecord {
            id: uuid_from_db(row.id),
            source_name: row.source_name,
            state: BatchState::from_db(&row.state)?,
            row_count: count_from_db(row.row_count)?,
            live_count: count_from_db(row.live_count)?,
            failed_count: count_from_db(row.failed_count)?,
            created_at: timestamp_from_db(row.created_at),
            expires_at: timestamp_from_db(row.expires_at),
            settled_at: row.settled_at.map(timestamp_from_db),
            failure_detail: row.failure_detail,
        }))
    }

    /// This organisation's open batch, or none.
    ///
    /// At most one exists by `import_batch_one_open_per_org`, so this answers
    /// one row rather than a list, and the open states are the index's own
    /// predicate rather than a second opinion about it.
    pub async fn open(&self, org: OrgId) -> Result<Option<ImportBatchRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, source_name, state, row_count, live_count, failed_count, \
                    created_at, expires_at, settled_at, failure_detail \
               FROM import_batch \
              WHERE org_id = $1 AND state IN ('parsed', 'attaching', 'importing')",
            uuid_to_db(org.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ImportBatchRecord {
            id: uuid_from_db(row.id),
            source_name: row.source_name,
            state: BatchState::from_db(&row.state)?,
            row_count: count_from_db(row.row_count)?,
            live_count: count_from_db(row.live_count)?,
            failed_count: count_from_db(row.failed_count)?,
            created_at: timestamp_from_db(row.created_at),
            expires_at: timestamp_from_db(row.expires_at),
            settled_at: row.settled_at.map(timestamp_from_db),
            failure_detail: row.failure_detail,
        }))
    }

    /// Every row of one batch, without its draft, in sheet-and-row order.
    ///
    /// Unpaged, because `tam_limits::import::ROWS_PER_UPLOAD_MAX` bounds the
    /// batch and the report is read whole: a report that arrives in pages is
    /// not a report a seller can fix a sheet against.
    ///
    /// `COLLATE "C"` on the sheet name, because a linguistic collation orders
    /// "Teachouse" before "TES" and a byte collation orders them the other
    /// way — and which one a cluster uses is `db/ephemeral-postgres.sh`'s to
    /// state, not this query's, since initdb is passed no locale. Pinning it
    /// makes this repository's order a fact rather than a property of where it
    /// runs. It is not the order the seller reads: the caller re-orders by the
    /// workbook's own tab order, which is what a report is fixed against.
    pub async fn rows(
        &self,
        org: OrgId,
        batch: Uuid,
    ) -> Result<Vec<ImportBatchRowRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT sheet, ordinal, inventory, intent, problems, file_name, \
                    COALESCE( \
                        (SELECT array_agg(value #>> '{}' ORDER BY ordinality) \
                           FROM jsonb_array_elements(draft -> 'labels') \
                           WITH ORDINALITY AS named(value, ordinality) \
                          WHERE jsonb_typeof(draft -> 'labels') = 'array'), \
                        ARRAY[]::text[] \
                    ) AS \"labels!\", \
                    file_hash, file_kind, file_byte_len, state, product_id, mapping_id, \
                    job_id, failure_detail \
               FROM import_batch_row \
              WHERE org_id = $1 AND batch_id = $2 \
              ORDER BY sheet COLLATE \"C\", ordinal",
            uuid_to_db(org.0),
            uuid_to_db(batch),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ImportBatchRowRecord {
                    sheet: row.sheet,
                    ordinal: count_from_db(row.ordinal)?,
                    inventory: row
                        .inventory
                        .as_deref()
                        .map(inventory_from_db)
                        .transpose()?,
                    intent: RowIntent::from_db(&row.intent)?,
                    problems: row.problems,
                    labels: row.labels,
                    file_name: row.file_name,
                    file: file_from_db(row.file_hash, row.file_kind, row.file_byte_len)?,
                    state: RowState::from_db(&row.state)?,
                    product_id: row.product_id.map(|id| ProductId(uuid_from_db(id))),
                    mapping_id: row.mapping_id.map(|id| MappingId(uuid_from_db(id))),
                    job_id: row.job_id.map(uuid_from_db),
                    failure_detail: row.failure_detail,
                })
            })
            .collect()
    }

    /// One page of a batch's rows with their drafts, in sheet-and-row order,
    /// starting after the row the cursor names.
    ///
    /// Paged where [`Self::rows`] is not, because a draft is bounded at the
    /// column's own ceiling and the batch is bounded at five hundred rows: the
    /// product of the two is a response no reader wants whole. The commit takes
    /// it a chunk at a time anyway, which is what makes a browser closing lose
    /// only the current chunk.
    pub async fn draft_page(
        &self,
        org: OrgId,
        batch: Uuid,
        after: Option<(&str, u32)>,
        limit: i64,
    ) -> Result<Vec<ImportBatchDraftRecord>, StorageError> {
        let (after_sheet, after_ordinal) = match after {
            Some((sheet, ordinal)) => (sheet.to_owned(), ordinal_to_db(ordinal)?),
            None => (String::new(), 0),
        };
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT sheet, ordinal, inventory, intent, draft, \
                    file_hash, file_kind, file_byte_len, \
                    cover_hash, cover_kind, cover_byte_len, state \
               FROM import_batch_row \
              WHERE org_id = $1 AND batch_id = $2 \
                AND (sheet, ordinal) > ($3, $4) \
              ORDER BY sheet, ordinal \
              LIMIT $5",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            after_sheet,
            after_ordinal,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ImportBatchDraftRecord {
                    sheet: row.sheet,
                    ordinal: count_from_db(row.ordinal)?,
                    inventory: row
                        .inventory
                        .as_deref()
                        .map(inventory_from_db)
                        .transpose()?,
                    intent: RowIntent::from_db(&row.intent)?,
                    draft: row.draft,
                    file: file_from_db(row.file_hash, row.file_kind, row.file_byte_len)?,
                    cover: file_from_db(row.cover_hash, row.cover_kind, row.cover_byte_len)?,
                    state: RowState::from_db(&row.state)?,
                })
            })
            .collect()
    }

    /// Settles an open batch to `abandoned` at the seller's own request.
    ///
    /// Answers whether a batch moved: a second abandon of the same batch, which
    /// is what a double-clicked button sends, moves nothing and is not a fault.
    /// A settled batch is never re-settled, so the `state` predicate is the
    /// guard rather than a read taken before the write.
    pub async fn abandon(
        &self,
        org: OrgId,
        batch: Uuid,
        at: Timestamp,
        detail: &str,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let settled = sqlx::query!(
            "UPDATE import_batch \
                SET state = 'abandoned', settled_at = $3, failure_detail = $4 \
              WHERE org_id = $1 AND id = $2 \
                AND state IN ('parsed', 'attaching', 'importing')",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            timestamp_to_db(at)?,
            detail,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(settled.rows_affected() == 1)
    }

    /// Binds the bytes of one row: the payload the seller attached and the
    /// cover the upload generated beside it.
    ///
    /// One transaction, because the two writes are one fact. The row moves to
    /// `attached` and the batch moves `parsed -> attaching`, and a seller
    /// watching a half-applied pair would see a batch still asking for its
    /// first file beside a row that already holds one.
    ///
    /// Re-binding a row that already holds a handle replaces it, which is what
    /// "every binding is reversible before the commit" means in practice: the
    /// seller who matched the wrong file to a row fixes it by dropping the
    /// right one on top.
    ///
    /// The row is locked before the batch, while the expiry sweep takes both in
    /// one statement whose sibling CTEs Postgres orders as it likes, so a bind
    /// racing the sweep over one batch can deadlock in a narrow window.
    /// Postgres detects that and aborts one of the two; the caller's retry or
    /// the next sweep pass finishes the work. Both are locked because a
    /// concurrent abandon must settle either wholly before this bind or wholly
    /// after it. A row exists only under a batch, so a missing row answers for
    /// a missing batch too.
    pub async fn bind_file(
        &self,
        org: OrgId,
        batch: Uuid,
        at: RowAddress<'_>,
        files: RowFiles<'_>,
    ) -> Result<BindOutcome, StorageError> {
        let RowAddress { sheet, ordinal } = at;
        let RowFiles { payload, cover } = files;
        let ordinal = ordinal_to_db(ordinal)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let row_state = sqlx::query_scalar!(
            "SELECT state FROM import_batch_row \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4 \
              FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row_state) = row_state else {
            return Ok(BindOutcome::NoSuchRow);
        };
        let Some(batch_state) = locked_batch_state(&mut tx, org, batch).await? else {
            return Ok(BindOutcome::NoSuchRow);
        };
        // The batch's answer first, because it is the coarser fact: a seller
        // whose import has been settled reads that rather than a sentence
        // about one row of it.
        if !batch_state.admits_attachment() {
            return Ok(BindOutcome::BatchClosed(batch_state));
        }
        let row_state = RowState::from_db(&row_state)?;
        if !row_state.admits_attachment() {
            return Ok(BindOutcome::RowClosed(row_state));
        }

        let payload_hash = crate::codec::hash_to_db(payload.hash);
        let cover_hash = crate::codec::hash_to_db(cover.hash);
        sqlx::query!(
            "UPDATE import_batch_row \
                SET file_hash = $5, file_kind = $6, file_byte_len = $7, \
                    cover_hash = $8, cover_kind = $9, cover_byte_len = $10, \
                    state = 'attached' \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
            payload_hash.as_slice(),
            payload.kind,
            payload.byte_len,
            cover_hash.as_slice(),
            cover.kind,
            cover.byte_len,
        )
        .execute(&mut *tx)
        .await?;

        // Monotone, and deliberately so: a batch that has begun attaching has
        // begun, and never walks back to `parsed` when its last handle is
        // cleared. One fewer state transition is one fewer arm the console's
        // exhaustive switch has to mean something by.
        sqlx::query!(
            "UPDATE import_batch SET state = 'attaching' \
              WHERE org_id = $1 AND id = $2 AND state = 'parsed'",
            uuid_to_db(org.0),
            uuid_to_db(batch),
        )
        .execute(&mut *tx)
        .await?;

        let written = sqlx::query!(
            "SELECT sheet, ordinal, inventory, intent, problems, file_name, \
                    COALESCE( \
                        (SELECT array_agg(value #>> '{}' ORDER BY ordinality) \
                           FROM jsonb_array_elements(draft -> 'labels') \
                           WITH ORDINALITY AS named(value, ordinality) \
                          WHERE jsonb_typeof(draft -> 'labels') = 'array'), \
                        ARRAY[]::text[] \
                    ) AS \"labels!\", \
                    file_hash, file_kind, file_byte_len, state, product_id, mapping_id, \
                    job_id, failure_detail \
               FROM import_batch_row \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
        )
        .fetch_one(&mut *tx)
        .await?;
        let row = ImportBatchRowRecord {
            sheet: written.sheet,
            ordinal: count_from_db(written.ordinal)?,
            inventory: written
                .inventory
                .as_deref()
                .map(inventory_from_db)
                .transpose()?,
            intent: RowIntent::from_db(&written.intent)?,
            problems: written.problems,
            labels: written.labels,
            file_name: written.file_name,
            file: file_from_db(written.file_hash, written.file_kind, written.file_byte_len)?,
            state: RowState::from_db(&written.state)?,
            product_id: written.product_id.map(|id| ProductId(uuid_from_db(id))),
            mapping_id: written.mapping_id.map(|id| MappingId(uuid_from_db(id))),
            job_id: written.job_id.map(uuid_from_db),
            failure_detail: written.failure_detail,
        };
        let counts = counts_in(&mut tx, org, batch).await?;
        let batch_state = if batch_state == BatchState::Parsed {
            BatchState::Attaching
        } else {
            batch_state
        };
        tx.commit().await?;
        Ok(BindOutcome::Bound(Box::new(BoundRow {
            row,
            batch_state,
            counts,
        })))
    }

    /// Clears the handles one row holds and returns it to `parsed`.
    ///
    /// The batch does not follow it back, and the two rows are locked in
    /// [`Self::bind_file`]'s order. See there for both reasons.
    pub async fn unbind_file(
        &self,
        org: OrgId,
        batch: Uuid,
        at: RowAddress<'_>,
    ) -> Result<UnbindOutcome, StorageError> {
        let RowAddress { sheet, ordinal } = at;
        let ordinal = ordinal_to_db(ordinal)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let held = sqlx::query_scalar!(
            "SELECT state FROM import_batch_row \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4 \
              FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if held.is_none() {
            return Ok(UnbindOutcome::NoSuchRow);
        }
        let Some(batch_state) = locked_batch_state(&mut tx, org, batch).await? else {
            return Ok(UnbindOutcome::NoSuchRow);
        };
        if !batch_state.admits_attachment() {
            return Ok(UnbindOutcome::BatchClosed(batch_state));
        }

        // The state predicate rather than a refusal read off `held`: a row the
        // parse refused holds no handle to clear, so clearing nothing is the
        // honest answer rather than a second way to say the row is refused.
        sqlx::query!(
            "UPDATE import_batch_row \
                SET file_hash = NULL, file_kind = NULL, file_byte_len = NULL, \
                    cover_hash = NULL, cover_kind = NULL, cover_byte_len = NULL, \
                    state = 'parsed' \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4 \
                AND state IN ('parsed', 'attached')",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(UnbindOutcome::Cleared)
    }

    /// Settles up to `batch` expired batches to `abandoned` and releases the
    /// file handles their rows hold.
    ///
    /// Cross-tenant by construction, like the job-event pruner and the snapshot
    /// retention pass: one turn covers every organisation, so it runs on a
    /// `tam_engine` pool with the SELECT and UPDATE grants migration 0058 gives
    /// and sees nothing at all under `tam_app`'s forced row-level security.
    ///
    /// The release and the settle are one statement so they cannot be observed
    /// apart: a batch left open beside rows whose handles were cleared is a
    /// seller told to attach files to a batch that has already lost them.
    /// Releasing the handle is not yet deleting the bytes — a blob has no
    /// deletion path in this repository — so the pass answers what it released
    /// rather than what it erased. Both handles go together: a row holding a
    /// cover for a payload it no longer names is a thumbnail of nothing.
    ///
    /// A row past creation keeps both handles. They are the breadcrumb of what
    /// its product was created from, the product's own file rows hold the
    /// bytes, and `import_batch_row_live_creation_names_a_file` refuses to
    /// clear them on a live row — so a release that reached such a row would
    /// fail the whole statement and settle nothing for any organisation, on
    /// every pass, until someone fixed the batch by hand. The predicate below
    /// is that constraint's own state list.
    pub async fn sweep_pass(
        &self,
        cutoff: Timestamp,
        batch: i64,
        detail: &str,
    ) -> Result<SweepReport, StorageError> {
        let counted = sqlx::query!(
            r#"WITH doomed AS (
                   SELECT org_id, id FROM import_batch
                    WHERE expires_at < $1
                      AND state IN ('parsed', 'attaching', 'importing')
                    ORDER BY expires_at, org_id, id
                    LIMIT $2
               ),
               released AS (
                   UPDATE import_batch_row r
                      SET file_hash = NULL, file_kind = NULL, file_byte_len = NULL,
                          cover_hash = NULL, cover_kind = NULL, cover_byte_len = NULL
                     FROM doomed d
                    WHERE r.org_id = d.org_id AND r.batch_id = d.id
                      AND r.state NOT IN ('creating', 'created', 'published')
                      AND (r.file_hash IS NOT NULL OR r.cover_hash IS NOT NULL)
                   RETURNING r.org_id
               ),
               settled AS (
                   UPDATE import_batch b
                      SET state = 'abandoned', settled_at = $1, failure_detail = $3
                     FROM doomed d
                    WHERE b.org_id = d.org_id AND b.id = d.id
                   RETURNING b.org_id
               )
               SELECT (SELECT count(*) FROM settled)  AS "abandoned!",
                      (SELECT count(*) FROM released) AS "released!""#,
            timestamp_to_db(cutoff)?,
            batch,
            detail,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(SweepReport {
            abandoned: u64::try_from(counted.abandoned).unwrap_or(0),
            released: u64::try_from(counted.released).unwrap_or(0),
        })
    }

    /// Opens a commit over this batch, or answers why it will not run.
    ///
    /// The gate and the transition are one transaction because they are one
    /// decision: a seller who unbinds a row between the two would otherwise
    /// have a batch moved to `importing` — which admits no more files — with a
    /// marketplace row holding none. D32's rule is read through the same
    /// [`counts_in`] the bind route answers with, so the panel's "still
    /// waiting" readout and the commit's own gate cannot disagree about which
    /// rows are outstanding.
    ///
    /// A second chunk finds the batch already `importing` and opens too: the
    /// commit is chunked, and only the first transition is a transition.
    pub async fn open_commit(
        &self,
        org: OrgId,
        batch: Uuid,
    ) -> Result<CommitOpening, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(state) = locked_batch_state(&mut tx, org, batch).await? else {
            return Ok(CommitOpening::NoSuchBatch);
        };
        if !state.is_open() {
            return Ok(CommitOpening::BatchClosed(state));
        }
        let awaiting = counts_in(&mut tx, org, batch).await?.awaiting;
        if awaiting > 0 {
            let rows = awaiting_rows(&mut tx, org, batch).await?;
            return Ok(CommitOpening::Awaiting {
                count: awaiting,
                rows,
            });
        }
        sqlx::query!(
            "UPDATE import_batch SET state = 'importing' \
              WHERE org_id = $1 AND id = $2 AND state IN ('parsed', 'attaching')",
            uuid_to_db(org.0),
            uuid_to_db(batch),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CommitOpening::Open)
    }

    /// Claims the next page of rows, reserving a product and — where the row
    /// names an inventory — a mapping identifier for each.
    ///
    /// The reservation is the whole point and it is why this is a write rather
    /// than a read. A pass that created first and wrote a breadcrumb second
    /// would mint a fresh product on every attempt, so a browser closed between
    /// the two leaves a row a later pass creates again: a second charged
    /// product in the seller's catalogue for one spreadsheet row, which is the
    /// hazard migration 0058's own header names. With the identifier reserved,
    /// a later pass finds the row already `creating`, reads whether that
    /// product exists, and either records the breadcrumb or re-runs the create
    /// under the same identifier.
    ///
    /// `creating` is therefore in the predicate as well as `parsed` and
    /// `attached`: a row left claimed by a killed pass is finished by the next
    /// chunk rather than stranded, and `COALESCE` is what keeps its reserved
    /// identifier rather than replacing it. `SKIP LOCKED` is what keeps two
    /// chunks running at once — a seller with the page open in two tabs — from
    /// claiming one row twice. It does not cover the window between one
    /// chunk's claim committing and its create finishing; what covers that is
    /// the caller reading whether the reserved product exists before creating
    /// it, so the worst a doubled pickup produces is a refused second insert
    /// rather than a second product.
    ///
    /// The identifiers are minted here rather than in the statement so the
    /// column keeps taking values this workspace generated, and one page's
    /// worth is minted whether or not the page fills: an unused identifier is
    /// four words of stack, and asking the database how many rows it would
    /// claim before claiming them is the race this statement exists to avoid.
    ///
    /// `at` is stamped on the batch as `claimed_at`, in the claim's own
    /// transaction, and it is what [`Self::sweep_stale`] reads: a claim
    /// seconds old is a chunk at work, and one older than any chunk could run
    /// is a pass that died. Stamped whether or not the page holds a row,
    /// because an empty claim is still a chunk that reached the batch.
    pub async fn claim_page(
        &self,
        org: OrgId,
        batch: Uuid,
        limit: i64,
        at: Timestamp,
    ) -> Result<Vec<ClaimedRow>, StorageError> {
        let reserved = usize::try_from(limit.max(0)).unwrap_or(0);
        let products: Vec<uuid::Uuid> = (0..reserved).map(|_| uuid::Uuid::new_v4()).collect();
        let mappings: Vec<uuid::Uuid> = (0..reserved).map(|_| uuid::Uuid::new_v4()).collect();

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let claimed = sqlx::query!(
            r#"WITH page AS (
                   SELECT sheet, ordinal,
                          row_number() OVER (ORDER BY sheet, ordinal) AS slot
                     FROM (
                         SELECT sheet, ordinal
                           FROM import_batch_row
                          WHERE org_id = $1 AND batch_id = $2
                            AND state IN ('parsed', 'attached', 'creating')
                          ORDER BY sheet, ordinal
                          LIMIT $3
                            FOR UPDATE SKIP LOCKED
                     ) held
               )
               UPDATE import_batch_row r
                  SET state = 'creating',
                      product_id = COALESCE(r.product_id, ($4::uuid[])[p.slot::int]),
                      mapping_id = CASE WHEN r.inventory IS NULL THEN NULL
                                        ELSE COALESCE(r.mapping_id, ($5::uuid[])[p.slot::int])
                                   END
                 FROM page p
                WHERE r.org_id = $1 AND r.batch_id = $2
                  AND r.sheet = p.sheet AND r.ordinal = p.ordinal
            RETURNING r.sheet, r.ordinal, r.inventory, r.intent, r.draft,
                      r.file_hash, r.file_kind, r.file_byte_len,
                      r.cover_hash, r.cover_kind, r.cover_byte_len,
                      r.product_id AS "product_id!", r.mapping_id"#,
            uuid_to_db(org.0),
            uuid_to_db(batch),
            limit,
            &products,
            &mappings,
        )
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE import_batch SET claimed_at = $3 WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut rows: Vec<ClaimedRow> = claimed
            .into_iter()
            .map(|row| {
                Ok(ClaimedRow {
                    sheet: row.sheet,
                    ordinal: count_from_db(row.ordinal)?,
                    inventory: row
                        .inventory
                        .as_deref()
                        .map(inventory_from_db)
                        .transpose()?,
                    intent: RowIntent::from_db(&row.intent)?,
                    draft: row.draft,
                    file: file_from_db(row.file_hash, row.file_kind, row.file_byte_len)?,
                    cover: file_from_db(row.cover_hash, row.cover_kind, row.cover_byte_len)?,
                    product: ProductId(uuid_from_db(row.product_id)),
                    mapping: row.mapping_id.map(|id| MappingId(uuid_from_db(id))),
                })
            })
            .collect::<Result<_, StorageError>>()?;
        // An UPDATE promises nothing about the order it returns rows in, and
        // the seller's report is read in sheet-and-row order; sorting here is
        // what keeps a chunk's outcomes in the order the sheet was filled.
        rows.sort_by(|left, right| {
            left.sheet
                .cmp(&right.sheet)
                .then_with(|| left.ordinal.cmp(&right.ordinal))
        });
        Ok(rows)
    }

    /// Records that the product reserved for this row now exists.
    ///
    /// Answers whether a row moved. The `creating` predicate is the guard: a
    /// row two passes both claimed is recorded once and the second answer is
    /// `false`, which is a fact the caller counts rather than a fault.
    ///
    /// Takes no instant, because no column takes one: `import_batch_row`
    /// records what a row became and never when, and a parameter no statement
    /// reads is weight the next reader has to discharge.
    pub async fn record_created(
        &self,
        org: OrgId,
        batch: Uuid,
        at: RowAddress<'_>,
        skipped: bool,
    ) -> Result<bool, StorageError> {
        let RowAddress { sheet, ordinal } = at;
        let ordinal = ordinal_to_db(ordinal)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "UPDATE import_batch_row SET state = 'created', skipped = $5 \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4 \
                AND state = 'creating'",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
            skipped,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(written.rows_affected() == 1)
    }

    /// Records that this row's create was refused, in the words the seller
    /// reads on the report.
    ///
    /// The reserved identifiers go with it, because
    /// `import_batch_row_created_total` ties them to the three states that
    /// hold a product and a failed row holds none. Nothing was created under
    /// them — the caller only reaches this where the create refused — so
    /// releasing them strands nothing.
    pub async fn record_row_failed(
        &self,
        org: OrgId,
        batch: Uuid,
        at: RowAddress<'_>,
        detail: &str,
    ) -> Result<bool, StorageError> {
        let RowAddress { sheet, ordinal } = at;
        let ordinal = ordinal_to_db(ordinal)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "UPDATE import_batch_row \
                SET state = 'failed', failure_detail = $5, \
                    product_id = NULL, mapping_id = NULL \
              WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4 \
                AND state = 'creating'",
            uuid_to_db(org.0),
            uuid_to_db(batch),
            sheet,
            ordinal,
            detail,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(written.rows_affected() == 1)
    }

    /// Where this batch's rows stand, which is what a chunk answers with and
    /// what decides whether the batch is finished.
    pub async fn pending_counts(
        &self,
        org: OrgId,
        batch: Uuid,
    ) -> Result<CommitCounts, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let counts = commit_counts_in(&mut tx, org, batch).await?;
        tx.commit().await?;
        Ok(counts)
    }

    /// Settles a finished commit: `imported` where every row created, `failed`
    /// where any did not.
    ///
    /// The counts are read inside the settling transaction rather than handed
    /// in, so the sentence a failed batch carries counts the rows the database
    /// holds rather than the rows one chunk happened to see. A batch that is
    /// not `importing` is answered as it stands and settled again by nothing,
    /// which is what two chunks finishing the last row at once looks like.
    pub async fn settle(
        &self,
        org: OrgId,
        batch: Uuid,
        at: Timestamp,
    ) -> Result<BatchState, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(held) = locked_batch_state(&mut tx, org, batch).await? else {
            return Err(StorageError::Inconsistent {
                reason: "an import settled a batch that is no longer readable".to_owned(),
            });
        };
        if held != BatchState::Importing {
            tx.commit().await?;
            return Ok(held);
        }
        let counts = commit_counts_in(&mut tx, org, batch).await?;
        let settled = settle_in(&mut tx, org, batch, counts, at).await?;
        tx.commit().await?;
        Ok(settled)
    }

    /// The sweep's second rule: a batch a dead pass left `importing` is
    /// settled where every row has an outcome and returned to the seller
    /// where any row has none.
    ///
    /// A commit is chunked and each chunk is one request the seller's console
    /// sends, so a closed console or a server restart mid-chunk leaves a batch
    /// `importing` with nothing driving it — a state the console reads as
    /// being created right now, and one that refuses every bind. The evidence
    /// is the stamp [`Self::claim_page`] writes: `stale_before` is the instant
    /// a claim has to predate to count as dead, and the caller derives it from
    /// how long a chunk can run. A batch with no claim at all is judged from
    /// its upload, so a pass that died between opening the commit and claiming
    /// its first page is not exempt.
    ///
    /// Each batch is its own transaction, and the batch row is locked and
    /// re-read inside it: a chunk that claimed between the scan and the lock
    /// moved the stamp, and a batch whose stamp moved is left to that chunk.
    /// The settle is [`settle_in`], so a batch this rule finishes carries the
    /// notification the commit route's own settle would have written, in the
    /// transaction that settles it. An unfinished batch returns to the state a
    /// bind wrote — `attaching` where any of its rows holds bytes, `parsed`
    /// where none does, which is a batch no bind ever reached — and its
    /// claimed rows keep their state and their reserved identifiers, so the
    /// seller's next commit finishes them rather than minting a second product
    /// for any of them.
    ///
    /// Cross-tenant, on the engine pool, for the reason [`Self::sweep_pass`]
    /// gives.
    pub async fn sweep_stale(
        &self,
        stale_before: Timestamp,
        at: Timestamp,
        batch: i64,
    ) -> Result<StaleReport, StorageError> {
        let stale_db = timestamp_to_db(stale_before)?;
        let found = sqlx::query!(
            "SELECT org_id, id FROM import_batch \
              WHERE state = 'importing' AND COALESCE(claimed_at, created_at) < $1 \
              ORDER BY COALESCE(claimed_at, created_at), org_id, id \
              LIMIT $2",
            stale_db,
            batch,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut report = StaleReport::default();
        for candidate in found {
            let org = OrgId(uuid_from_db(candidate.org_id));
            let id = uuid_from_db(candidate.id);
            let mut tx = self.pool.begin().await?;
            let still_stale = sqlx::query_scalar!(
                r#"SELECT (COALESCE(claimed_at, created_at) < $3) AS "stale!"
                     FROM import_batch
                    WHERE org_id = $1 AND id = $2 AND state = 'importing'
                      FOR UPDATE"#,
                uuid_to_db(org.0),
                uuid_to_db(id),
                stale_db,
            )
            .fetch_optional(&mut *tx)
            .await?;
            if still_stale != Some(true) {
                tx.rollback().await?;
                continue;
            }
            let counts = commit_counts_in(&mut tx, org, id).await?;
            if counts.outstanding == 0 {
                settle_in(&mut tx, org, id, counts, at).await?;
                report.settled = report.settled.saturating_add(1);
            } else {
                sqlx::query!(
                    "UPDATE import_batch b \
                        SET state = CASE \
                                WHEN EXISTS (SELECT 1 FROM import_batch_row r \
                                              WHERE r.org_id = b.org_id AND r.batch_id = b.id \
                                                AND r.file_hash IS NOT NULL) \
                                THEN 'attaching' ELSE 'parsed' END \
                      WHERE b.org_id = $1 AND b.id = $2 AND b.state = 'importing'",
                    uuid_to_db(org.0),
                    uuid_to_db(id),
                )
                .execute(&mut *tx)
                .await?;
                report.reopened = report.reopened.saturating_add(1);
            }
            tx.commit().await?;
        }
        Ok(report)
    }
}

/// Writes the settled state of an `importing` batch the caller holds locked,
/// and the seller's notification beside it.
///
/// `counts` are the caller's own read under that lock, in this transaction,
/// so the sentence a failed batch carries counts the rows the database holds
/// rather than the rows one chunk happened to see. The notification is written
/// here rather than after the commit because this is the transaction that
/// owns the fact, which is the rule the sync path's `settle_if_complete`
/// already follows: after the state update and before the commit, so a batch
/// is never `imported` with no inbox row. A process that dies between the two
/// rolls both back and the next settle writes both again, where a separate
/// write would have left the seller a finished import nothing ever told them
/// about, with `CommitOpening::BatchClosed` refusing every later attempt to
/// revisit it.
async fn settle_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    batch: Uuid,
    counts: CommitCounts,
    at: Timestamp,
) -> Result<BatchState, StorageError> {
    let settled = if counts.failed == 0 {
        BatchState::Imported
    } else {
        BatchState::Failed
    };
    let detail = (counts.failed > 0).then(|| {
        format!(
            "{} of {} rows did not create",
            counts.failed,
            counts.failed.saturating_add(counts.created)
        )
    });
    sqlx::query!(
        "UPDATE import_batch SET state = $3, settled_at = $4, failure_detail = $5 \
          WHERE org_id = $1 AND id = $2 AND state = 'importing'",
        uuid_to_db(org.0),
        uuid_to_db(batch),
        settled.as_str(),
        timestamp_to_db(at)?,
        detail.as_deref(),
    )
    .execute(&mut **tx)
    .await?;
    crate::notifications::record(
        tx,
        org,
        &crate::notifications::Completion {
            kind: tam_types::NotificationKind::Import,
            subject: batch,
            inventory: None,
            counts: tam_types::NotificationCounts {
                succeeded: counts.created,
                skipped: counts.skipped,
                failed: counts.failed,
                ..tam_types::NotificationCounts::default()
            },
            at,
        },
    )
    .await?;
    Ok(settled)
}

/// One batch's state, locked, inside the caller's own transaction.
///
/// Read after the row it belongs to rather than before, which is the lock
/// order [`ImportBatchRepo::bind_file`] states its reason for.
async fn locked_batch_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    batch: Uuid,
) -> Result<Option<BatchState>, StorageError> {
    let held = sqlx::query_scalar!(
        "SELECT state FROM import_batch WHERE org_id = $1 AND id = $2 FOR UPDATE",
        uuid_to_db(org.0),
        uuid_to_db(batch),
    )
    .fetch_optional(&mut **tx)
    .await?;
    held.as_deref().map(BatchState::from_db).transpose()
}

/// How many of a batch's outstanding rows hold bytes, and how many still need
/// them, read inside the caller's own transaction so a bind's answer counts
/// the write it just made.
async fn counts_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    batch: Uuid,
) -> Result<AttachCounts, StorageError> {
    let counted = sqlx::query!(
        r#"SELECT
               count(*) FILTER (
                   WHERE state IN ('parsed', 'attached') AND file_hash IS NOT NULL
               ) AS "attached!",
               count(*) FILTER (
                   WHERE state IN ('parsed', 'attached')
                     AND file_hash IS NULL
                     AND inventory IS NOT NULL
               ) AS "awaiting!"
             FROM import_batch_row
            WHERE org_id = $1 AND batch_id = $2"#,
        uuid_to_db(org.0),
        uuid_to_db(batch),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(AttachCounts {
        attached: attach_count_from_db(counted.attached)?,
        awaiting: attach_count_from_db(counted.awaiting)?,
    })
}

/// The first few rows that still need bytes, as a refusal names them.
///
/// The predicate is [`counts_in`]'s `awaiting` filter, and the two are read in
/// the same transaction, so the count a seller is told and the rows they are
/// shown are one answer rather than two taken a moment apart.
async fn awaiting_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    batch: Uuid,
) -> Result<Vec<RowRef>, StorageError> {
    let named = sqlx::query!(
        "SELECT sheet, ordinal FROM import_batch_row \
          WHERE org_id = $1 AND batch_id = $2 \
            AND state IN ('parsed', 'attached') \
            AND file_hash IS NULL \
            AND inventory IS NOT NULL \
          ORDER BY sheet, ordinal \
          LIMIT $3",
        uuid_to_db(org.0),
        uuid_to_db(batch),
        AWAITING_LISTED_MAX,
    )
    .fetch_all(&mut **tx)
    .await?;
    named
        .into_iter()
        .map(|row| {
            Ok(RowRef {
                sheet: row.sheet,
                ordinal: count_from_db(row.ordinal)?,
            })
        })
        .collect()
}

/// Where a batch's rows stand, read inside the caller's own transaction so a
/// chunk's answer counts the writes it just made.
///
/// A failed row is one this commit failed, not one the parse refused: a
/// refusal is stored in `problems` and writes no `failure_detail`, and
/// `import_batch_row_failure_detail` holds that detail to failed rows, so the
/// column is the discriminator rather than a state list that cannot tell the
/// two apart. That matters for the settle: a batch whose only failures came
/// off the parse is `imported`, because every row the commit was given did
/// create.
async fn commit_counts_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    batch: Uuid,
) -> Result<CommitCounts, StorageError> {
    let counted = sqlx::query!(
        r#"SELECT
               count(*) FILTER (
                   WHERE state IN ('parsed', 'attached', 'creating')
               ) AS "outstanding!",
               count(*) FILTER (
                   WHERE state IN ('created', 'published') AND NOT skipped
               ) AS "created!",
               count(*) FILTER (
                   WHERE state IN ('created', 'published') AND skipped
               ) AS "skipped!",
               count(*) FILTER (
                   WHERE state = 'failed' AND failure_detail IS NOT NULL
               ) AS "failed!"
             FROM import_batch_row
            WHERE org_id = $1 AND batch_id = $2"#,
        uuid_to_db(org.0),
        uuid_to_db(batch),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(CommitCounts {
        outstanding: attach_count_from_db(counted.outstanding)?,
        created: attach_count_from_db(counted.created)?,
        skipped: attach_count_from_db(counted.skipped)?,
        failed: attach_count_from_db(counted.failed)?,
    })
}

/// A count of rows as Postgres answers one. Bounded by
/// `tam_limits::import::ROWS_PER_UPLOAD_MAX` long before `u32` binds, so a
/// value past it is a row set no upload could have written.
fn attach_count_from_db(raw: i64) -> Result<u32, StorageError> {
    u32::try_from(raw).map_err(|_| StorageError::CorruptRow {
        reason: format!("an import batch holds {raw} rows, which no upload can have written"),
    })
}

/// The three file columns as one value, matching
/// `import_batch_row_file_handle_total`: all present or all absent. A partial
/// set is a row that constraint should have refused, so it is read as corrupt
/// rather than as two facts and a guess.
fn file_from_db(
    hash: Option<Vec<u8>>,
    kind: Option<String>,
    byte_len: Option<i64>,
) -> Result<Option<RowFile>, StorageError> {
    match (hash, kind, byte_len) {
        (None, None, None) => Ok(None),
        (Some(hash), Some(kind), Some(byte_len)) => Ok(Some(RowFile {
            hash: crate::codec::hash_from_db(&hash)?,
            kind,
            byte_len,
        })),
        _ => Err(StorageError::CorruptRow {
            reason: "an import row's file handle is three columns or none, and this row holds some"
                .to_owned(),
        }),
    }
}

/// A count as the column holds it. Bounded by
/// `tam_limits::import::ROWS_PER_UPLOAD_MAX` long before `i32` binds, so a
/// count past it is a caller that skipped the route's own bound rather than a
/// number to saturate.
fn count_to_db(count: usize) -> Result<i32, StorageError> {
    i32::try_from(count).map_err(|_| StorageError::Inconsistent {
        reason: format!("an import batch holds {count} rows, which no column can state"),
    })
}

fn count_from_db(raw: i32) -> Result<u32, StorageError> {
    u32::try_from(raw).map_err(|_| StorageError::CorruptRow {
        reason: format!("an import count is not negative and this is {raw}"),
    })
}

/// The seller's own spreadsheet row number, which
/// `import_batch_row_ordinal_positive` holds above zero.
fn ordinal_to_db(ordinal: u32) -> Result<i32, StorageError> {
    i32::try_from(ordinal).map_err(|_| StorageError::Inconsistent {
        reason: format!("spreadsheet row {ordinal} is past what the column can state"),
    })
}
