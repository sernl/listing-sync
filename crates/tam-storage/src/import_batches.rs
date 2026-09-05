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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Parsed,
    Attached,
    Created,
    Published,
    Failed,
    Skipped,
}

impl RowState {
    pub const ALL: [Self; 6] = [
        Self::Parsed,
        Self::Attached,
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
            Self::Created => "created",
            Self::Published => "published",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "parsed" => Ok(Self::Parsed),
            "attached" => Ok(Self::Attached),
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

/// What one sweep pass settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SweepReport {
    /// Batches settled to `abandoned`.
    pub abandoned: u64,
    /// Rows whose file handle was released. Zero until the bind route exists;
    /// counted from the start so the pass reports what it did rather than what
    /// it was expected to do.
    pub released: u64,
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
    /// "Teachouse" before "TES GB" and a byte collation orders them the other
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
                    file_hash, file_kind, file_byte_len, state \
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
    /// rather than what it erased.
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
                      SET file_hash = NULL, file_kind = NULL, file_byte_len = NULL
                     FROM doomed d
                    WHERE r.org_id = d.org_id AND r.batch_id = d.id
                      AND r.file_hash IS NOT NULL
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
