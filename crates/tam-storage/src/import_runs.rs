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

use sqlx::PgPool;
use tam_types::{ContentHash, InventoryId, JobId, Money, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    currency_from_db, currency_to_db, hash_from_db, hash_to_db, inventory_from_db, inventory_to_db,
    timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::{pin_org, StorageError};

/// How many runs one listing answers.
pub const RUNS_LISTED_MAX: i64 = 50;

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
}

/// What opening a run answered.
///
/// A branch rather than an error, because the two are different answers to the
/// seller: one import is open and this is which, or this one is now open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOpening {
    Opened,
    AlreadyOpen(Uuid),
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

/// The partial unique index one-open-run-per-org is stated by, named here
/// because its violation is the route's own refusal rather than a fault.
const ONE_OPEN_PER_ORG: &str = "import_run_one_open_per_org";

pub struct ImportRunRepo {
    pool: PgPool,
}

impl ImportRunRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Opens a run, or names the one already open.
    ///
    /// The refusal is the index's rather than a read before the write, so two
    /// submits racing cannot both be told the seller had nothing open.
    pub async fn create(&self, org: OrgId, new: &NewImportRun) -> Result<RunOpening, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let inserted = sqlx::query!(
            "INSERT INTO import_run \
               (org_id, id, kind, source, batch_id, target, state, anchor_job, created_at, \
                scheduled) \
             VALUES ($1, $2, $3, $4, $5, $6, 'reading', $7, $8, $9)",
            uuid_to_db(org.0),
            uuid_to_db(new.id),
            new.kind.as_str(),
            new.source.map(inventory_to_db),
            new.batch_id.map(uuid_to_db),
            new.target.map(inventory_to_db),
            uuid_to_db(new.anchor_job.0),
            timestamp_to_db(new.created_at)?,
            new.scheduled,
        )
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {
                tx.commit().await?;
                Ok(RunOpening::Opened)
            }
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some(ONE_OPEN_PER_ORG) =>
            {
                drop(tx);
                let open = self.open(org).await?.ok_or(StorageError::Inconsistent {
                    reason: "the one-open-run index refused an insert and no run is open"
                        .to_owned(),
                })?;
                Ok(RunOpening::AlreadyOpen(open.id))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The run still expecting work, if there is one.
    pub async fn open(&self, org: OrgId) -> Result<Option<ImportRunHead>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, kind, source, batch_id, target, state, anchor_job, read_total, \
                    created_at, settled_at, failure_detail, scheduled \
               FROM import_run \
              WHERE org_id = $1 AND state IN ('reading', 'reviewing', 'committing')",
            uuid_to_db(org.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| {
            head_of(
                row.id,
                &row.kind,
                row.source.as_deref(),
                row.batch_id,
                row.target.as_deref(),
                &row.state,
                row.anchor_job,
                row.read_total,
                row.created_at,
                row.settled_at,
                row.failure_detail,
                row.scheduled,
            )
        })
        .transpose()
    }

    /// The run that reviews one spreadsheet batch, if it exists.
    pub async fn by_batch(
        &self,
        org: OrgId,
        batch: Uuid,
    ) -> Result<Option<ImportRunHead>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, kind, source, batch_id, target, state, anchor_job, read_total, \
                    created_at, settled_at, failure_detail, scheduled \
               FROM import_run \
              WHERE org_id = $1 AND batch_id = $2 \
              ORDER BY created_at DESC LIMIT 1",
            uuid_to_db(org.0),
            uuid_to_db(batch),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| {
            head_of(
                row.id,
                &row.kind,
                row.source.as_deref(),
                row.batch_id,
                row.target.as_deref(),
                &row.state,
                row.anchor_job,
                row.read_total,
                row.created_at,
                row.settled_at,
                row.failure_detail,
                row.scheduled,
            )
        })
        .transpose()
    }

    /// The organisation's runs, newest first.
    pub async fn list(&self, org: OrgId) -> Result<Vec<ImportRunHead>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, kind, source, batch_id, target, state, anchor_job, read_total, \
                    created_at, settled_at, failure_detail, scheduled \
               FROM import_run WHERE org_id = $1 \
              ORDER BY created_at DESC, id DESC LIMIT $2",
            uuid_to_db(org.0),
            RUNS_LISTED_MAX,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                head_of(
                    row.id,
                    &row.kind,
                    row.source.as_deref(),
                    row.batch_id,
                    row.target.as_deref(),
                    &row.state,
                    row.anchor_job,
                    row.read_total,
                    row.created_at,
                    row.settled_at,
                    row.failure_detail,
                    row.scheduled,
                )
            })
            .collect()
    }

    /// One run and every row of it.
    pub async fn get(
        &self,
        org: OrgId,
        run: Uuid,
    ) -> Result<Option<ImportRunRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, kind, source, batch_id, target, state, anchor_job, read_total, \
                    created_at, settled_at, failure_detail, scheduled \
               FROM import_run WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let head = head_of(
            row.id,
            &row.kind,
            row.source.as_deref(),
            row.batch_id,
            row.target.as_deref(),
            &row.state,
            row.anchor_job,
            row.read_total,
            row.created_at,
            row.settled_at,
            row.failure_detail,
            row.scheduled,
        )?;
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
        let org_db = uuid_to_db(org.0);
        let run_db = uuid_to_db(run);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held: i64 = sqlx::query_scalar!(
            "SELECT COALESCE(MAX(ordinal), 0)::bigint FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2",
            org_db,
            run_db,
        )
        .fetch_one(&mut *tx)
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
            .execute(&mut *tx)
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
        .execute(&mut *tx)
        .await?;
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
    ) -> Result<(), StorageError> {
        let org_db = uuid_to_db(org.0);
        let run_db = uuid_to_db(run);
        let at_db = timestamp_to_db(at)?;
        let chosen: Vec<String> = match selection {
            Selection::All => Vec::new(),
            Selection::Locators(locators) => locators.to_vec(),
        };
        let all = matches!(selection, Selection::All);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE import_run_item SET state = 'selected' \
              WHERE org_id = $1 AND run_id = $2 AND state = 'listed' \
                AND ($3 OR locator = ANY($4))",
            org_db,
            run_db,
            all,
            &chosen,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE import_run_item \
                SET state = 'skipped', skip_reason = 'not chosen', settled_at = $3 \
              WHERE org_id = $1 AND run_id = $2 AND state = 'listed'",
            org_db,
            run_db,
            at_db,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
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
        let org_db = uuid_to_db(org.0);
        let run_db = uuid_to_db(run);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = sqlx::query!(
            "SELECT state FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 AND locator = $3",
            org_db,
            run_db,
            item.locator,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(held) = held.as_ref() {
            // Anything past `selected` already holds a description, and
            // overwriting it would move a product's input under a commit that
            // may already have read it.
            if !matches!(held.state.as_str(), "listed" | "selected") {
                tx.commit().await?;
                return Ok(false);
            }
        }
        let next_ordinal: i64 = sqlx::query_scalar!(
            "SELECT COALESCE(MAX(ordinal), 0)::bigint + 1 FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2",
            org_db,
            run_db,
        )
        .fetch_one(&mut *tx)
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
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
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
        let org_db = uuid_to_db(org.0);
        let run_db = uuid_to_db(run);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let next_ordinal: i64 = sqlx::query_scalar!(
            "SELECT COALESCE(MAX(ordinal), 0)::bigint + 1 FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2",
            org_db,
            run_db,
        )
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(1);
        sqlx::query!(
            "INSERT INTO import_run_item \
               (org_id, run_id, locator, ordinal, state, product_id, failure_detail, read_at, \
                settled_at) \
             VALUES ($1, $2, $3, $4, 'failed', $5, $6, $7, $7) \
             ON CONFLICT (org_id, run_id, locator) DO UPDATE SET \
               state = 'failed', failure_detail = EXCLUDED.failure_detail, \
               skip_reason = NULL, settled_at = EXCLUDED.settled_at",
            org_db,
            run_db,
            locator,
            i32::try_from(next_ordinal).unwrap_or(i32::MAX),
            uuid_to_db(fresh_uuid()),
            detail,
            at_db,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every state this run's rows are in.
    pub async fn counts(&self, org: OrgId, run: Uuid) -> Result<RunCounts, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT state, count(*)::bigint AS held FROM import_run_item \
              WHERE org_id = $1 AND run_id = $2 GROUP BY state",
            uuid_to_db(org.0),
            uuid_to_db(run),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
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
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant, run and locator, and the write carries its \
                  own value and its instant; a struct over those six would name the call"
    )]
    pub async fn set_state(
        &self,
        org: OrgId,
        run: Uuid,
        state: RunState,
        detail: Option<&str>,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let settled = (!state.open()).then_some(timestamp_to_db(at)).transpose()?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE import_run SET state = $3, settled_at = $4, failure_detail = $5 \
              WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(run),
            state.as_str(),
            settled,
            detail,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
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
}

#[allow(clippy::too_many_arguments)]
fn head_of(
    id: uuid::Uuid,
    kind: &str,
    source: Option<&str>,
    batch_id: Option<uuid::Uuid>,
    target: Option<&str>,
    state: &str,
    anchor_job: uuid::Uuid,
    read_total: Option<i32>,
    created_at: chrono::DateTime<chrono::Utc>,
    settled_at: Option<chrono::DateTime<chrono::Utc>>,
    failure_detail: Option<String>,
    scheduled: bool,
) -> Result<ImportRunHead, StorageError> {
    Ok(ImportRunHead {
        id: uuid_from_db(id),
        kind: RunKind::from_db(kind)?,
        source: source.map(inventory_from_db).transpose()?,
        batch_id: batch_id.map(uuid_from_db),
        target: target.map(inventory_from_db).transpose()?,
        state: RunState::from_db(state)?,
        anchor_job: JobId(uuid_from_db(anchor_job)),
        read_total: read_total.map(|total| u32::try_from(total).unwrap_or(0)),
        created_at: timestamp_from_db(created_at),
        settled_at: settled_at.map(timestamp_from_db),
        failure_detail,
        scheduled,
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
