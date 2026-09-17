//! The read leg of a sync, which is a request rather than a ledger item.
//!
//! A sync's first leg reads the source marketplace and writes the catalogue;
//! its second leg writes the target marketplace. Only the second is a job,
//! because a job carries one inventory and the item pump is deliberately
//! adapter-free. So the first leg lives here and the drain that runs it is a
//! sibling process under the tenant's own role.
//!
//! Everything this repo writes is per-resource, because `import_one` is not
//! atomic internally -- the product, the mapping, the raised queue items and
//! the blob writes are four commit points -- and it mints a fresh product id
//! on every pass. A request redrained after a crash therefore has to know
//! which resources it already canonicalised, or it inserts a second product
//! that no unique index refuses.

use sqlx::PgPool;
use tam_marketplace::{ListingState, RemoteListingId};
use tam_types::{
    InventoryId, MappingId, OrgId, ProductId, Stamp, SystemComponent, Timestamp, TransportClass,
    Uuid,
};

use crate::jobs::{CreatedJob, DeletionStatus, JobFence, NewJob, NewJobItem};

use crate::codec::{
    inventory_from_db, inventory_to_db, listing_state_from_db, listing_state_to_db,
    timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db, RemoteIdColumns,
};
use crate::mapping::remote_id_from_db;
use crate::{pin_org, StorageError};

/// The four counts as one measurement, which is how
/// `sync_request_resource_coverage_total` stores them: all present or all
/// absent. A partial set is a row that constraint should have refused, so it is
/// read as corrupt rather than as three counts and a guess.
fn coverage_from_db(
    seen: Option<i32>,
    mapped: Option<i32>,
    unmapped: Option<i32>,
    uncovered: Option<i32>,
) -> Result<Option<ResourceCoverage>, StorageError> {
    match (seen, mapped, unmapped, uncovered) {
        (None, None, None, None) => Ok(None),
        (Some(seen), Some(mapped), Some(unmapped), Some(uncovered)) => Ok(Some(ResourceCoverage {
            terms_seen: count_from_db(seen)?,
            terms_mapped: count_from_db(mapped)?,
            terms_unmapped: count_from_db(unmapped)?,
            terms_uncovered: count_from_db(uncovered)?,
        })),
        _ => Err(StorageError::CorruptRow {
            reason: "a coverage measurement is four counts or none, and this row holds some"
                .to_owned(),
        }),
    }
}

/// A count as the row holds it. Negative is impossible under
/// `sync_request_resource_counts`, so reading one back is a corrupt row rather
/// than a number to clamp.
fn count_from_db(raw: i32) -> Result<u32, StorageError> {
    u32::try_from(raw).map_err(|_| StorageError::CorruptRow {
        reason: format!("a coverage count is not negative and this is {raw}"),
    })
}

/// A SQL `sum` of per-resource counts. Postgres widens the sum to `bigint`,
/// and a total past `u32` is a catalogue no seller has, so it saturates for
/// the same reason [`count_to_db`] does rather than failing a page that has
/// otherwise been read.
fn sum_from_db(raw: i64) -> u32 {
    u32::try_from(raw).unwrap_or(u32::MAX)
}

/// The same in the other direction. A count beyond `i32` is a catalogue no
/// seller has, and saturating is the honest failure: the alternative is a
/// wrapped negative the CHECK would refuse at the far end of a page that
/// otherwise succeeded.
fn count_to_db(count: u32) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

pub struct SyncRequestRepo {
    pool: PgPool,
}

/// Whether the source listing survives the sync. The only difference between
/// the two dispositions in the drain is the second job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Sync,
    Migrate,
}

impl Disposition {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Migrate => "migrate",
        }
    }
}

/// What the seller asked the target to end up as. Lowered against the
/// mapping's own binding at the API boundary, never here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncIntent {
    Draft,
    Live,
}

impl SyncIntent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Live => "live",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSyncRequest {
    pub id: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: Disposition,
    pub intent: SyncIntent,
    pub requested_at: Timestamp,
    /// The seller's own addresses for the listings, in the order they gave
    /// them. The ordinal is the row's identity, so a redrain resumes against
    /// the same list the seller submitted.
    pub locators: Vec<String>,
}

/// One resource of a migration, already canonicalised.
///
/// Everything [`Canonicalised`] carries, before the request exists rather than
/// after a read produced it. The values come from the source mapping's own
/// binding, which is why a migration needs no marketplace read to start: the
/// catalogue already knows the product, and the binding already names the
/// listing the removal leg takes down and the state it takes it down from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalResource {
    /// How the source marketplace addresses this listing, rendered from the
    /// binding so the row reads the same as one a device wrote.
    pub locator: String,
    pub product: ProductId,
    /// The mapping on the request's *target*, which is what the create leg
    /// projects: `mapping_seeds` filters on the job's inventory, so a source
    /// mapping here would yield an itemless create job.
    pub mapping: MappingId,
    pub source: RemoteListingId,
    pub source_state: Option<ListingState>,
}

/// A migration between two marketplaces, resources and all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMigration {
    pub id: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: Disposition,
    pub intent: SyncIntent,
    pub requested_at: Timestamp,
    pub resources: Vec<CanonicalResource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequestRecord {
    pub id: Uuid,
    pub org: OrgId,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: Disposition,
    pub intent: SyncIntent,
    pub state: String,
    pub create_job: Option<Uuid>,
    pub remove_job: Option<Uuid>,
    pub failure_detail: Option<String>,
    pub requested_at: Timestamp,
    pub resources: Vec<SyncResourceRecord>,
}

/// What the import measured about one resource's taxonomy coverage.
///
/// The founder's kill gate, per resource. The definition is
/// `tam_import::uncovered_terms` and nothing else — Subject and Topic only,
/// distinct terms, a recorded no-counterpart omitted, an unclassifiable term
/// counted — because the number is compared across a series of migrations and
/// a second reckoning of it is how such a number stops meaning anything.
///
/// The absence of one of these is null in the row and `None` here, never a
/// zero: a resource the device skipped and every breadcrumb of a request that
/// measures nothing have not been through the taxonomy at all, and entering
/// them into the founder's average as perfect coverage is the one mistake this
/// number cannot afford.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceCoverage {
    pub terms_seen: u32,
    pub terms_mapped: u32,
    pub terms_unmapped: u32,
    pub terms_uncovered: u32,
}

/// What a completing page settles and mints, together.
#[derive(Debug, Clone)]
pub struct Completion<'a> {
    pub request: Uuid,
    pub at: Timestamp,
    /// The create job, or `None` for a catalogue that held nothing to publish.
    pub mint: Option<Mint<'a>>,
}

/// The create job a completing page earns.
#[derive(Debug, Clone)]
pub struct Mint<'a> {
    /// Derived from the request id, so a re-posted completing page is a replay
    /// of the same job rather than a second one.
    pub request_key: Uuid,
    pub job: &'a NewJob,
    pub items: &'a [NewJobItem],
}

/// What admitting one resource of a running page came to.
///
/// Three answers because the caller acts differently on each: a page carries
/// on with an admitted resource, skips one it has already described, and
/// stops without starting anything further when the seller has deleted the
/// request underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceAdmission {
    /// Admitted, at this ordinal. The breadcrumb is `pending` until the
    /// resource's commits finish, and the request reads `stopping` meanwhile.
    Admitted(i32),
    /// This locator already has a finished breadcrumb.
    AlreadyDescribed,
    /// Nothing was admitted: the request carries a deletion.
    Stopped,
}

/// One resource the device described, as the import route records it.
#[derive(Debug, Clone)]
pub struct Observed<'a> {
    /// How the seller's marketplace addresses it, and the identity a re-posted
    /// page is recognised by.
    pub locator: &'a str,
    pub product: ProductId,
    pub mapping: MappingId,
    pub source: &'a RemoteListingId,
    pub source_state: Option<ListingState>,
    pub coverage: ResourceCoverage,
}

/// One request as a list row: enough to render it and reach it, and no
/// breadcrumbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequestSummary {
    pub id: Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: Disposition,
    pub intent: SyncIntent,
    pub state: String,
    pub requested_at: Timestamp,
    pub resources_total: u32,
    /// How many the device could not describe. Shown beside the total rather
    /// than folded into it, because a migration that crossed forty of fifty
    /// resources is a different thing to a seller than one that crossed all
    /// forty it had.
    pub resources_failed: u32,
    /// Where a Delete this request is still working through has got to.
    /// Never `Deleted` on a listed row: the tombstone is filtered in SQL.
    pub deletion: Option<DeletionStatus>,
}

/// One page of the request list: what to narrow it to, where the last page
/// ended, and how many rows this one carries.
///
/// A struct rather than four arguments, because every caller has to state all
/// four and a positional `Option<Timestamp>` beside an `Option<String>` is a
/// pair of arguments nobody can read at the call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequestPage {
    /// The keyset the previous page ended on, as `(requested_at, id)`: the
    /// same pair the `ORDER BY` sorts by, so the walk cannot repeat or skip a
    /// request that shares an instant with the page boundary.
    pub after: Option<(Timestamp, Uuid)>,
    pub limit: i64,
    /// Which half of the list to answer, or both. A sync and a migration are
    /// one record under two dispositions, and the two screens that read this
    /// list each own one of them.
    pub disposition: Option<Disposition>,
    /// One request state, spelled as the column stores it. Held as a string
    /// rather than an enum because the state vocabulary is the drain's and
    /// this repo does not own it.
    pub state: Option<String>,
}

/// How many resources of a request stand in each state.
///
/// The whole request, counted in SQL, so a page of breadcrumbs never has to
/// be summed to say what the request as a whole is doing. A page-derived
/// tally would say "nothing imported" to a seller looking at page two of a
/// finished migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncResourceStateCount {
    pub state: String,
    pub count: u32,
}

/// One request, its whole-request figures, and one page of its breadcrumbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequestDetail {
    pub head: SyncRequestHead,
    /// Every state the request's resources stand in, with how many stand in
    /// it. Over the whole request, never over `resources`.
    pub counts: Vec<SyncResourceStateCount>,
    /// The coverage summed over the resources that carry a measurement, and
    /// `None` where no resource of the request carries one.
    ///
    /// Summed in SQL for the same reason the counts are, and over the
    /// measured rows only: a skipped resource never reached the taxonomy, and
    /// entering it as a row of zeros would put it into the founder's average
    /// as perfect coverage.
    pub coverage: Option<SyncCoverageTotals>,
    /// One page of breadcrumbs, by ordinal.
    pub resources: Vec<SyncResourceRecord>,
    /// The ordinal to ask after for the next page, or `None` at the end.
    pub next_ordinal: Option<i32>,
}

/// The request's own head, without its breadcrumbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRequestHead {
    pub id: Uuid,
    pub org: OrgId,
    pub source: InventoryId,
    pub target: InventoryId,
    pub disposition: Disposition,
    pub intent: SyncIntent,
    pub state: String,
    pub create_job: Option<Uuid>,
    pub remove_job: Option<Uuid>,
    pub failure_detail: Option<String>,
    pub requested_at: Timestamp,
    /// See [`SyncRequestSummary::deletion`].
    pub deletion: Option<DeletionStatus>,
}

/// The coverage of a whole request, and how many resources it is summed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncCoverageTotals {
    /// How many measured resources the sum is over, which is what makes the
    /// figures readable as an average rather than only as a total.
    pub rows: u32,
    pub terms_seen: u32,
    pub terms_mapped: u32,
    pub terms_unmapped: u32,
    pub terms_uncovered: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncResourceRecord {
    pub ordinal: i32,
    pub locator: String,
    pub state: String,
    /// What the import measured about this resource, and `None` where nothing
    /// measured it: a skipped resource, or any breadcrumb of a request whose
    /// source is not enumerated by a device.
    pub coverage: Option<ResourceCoverage>,
    pub product: Option<ProductId>,
    pub mapping: Option<MappingId>,
    /// What the read observed about the source listing, recorded so a redrain
    /// can rebuild this resource's removal item instead of dropping it.
    pub source: Option<RemoteListingId>,
    pub source_state: Option<ListingState>,
    pub failure_detail: Option<String>,
}

impl SyncResourceRecord {
    /// What the drain skips. A resource that already names a product, a
    /// mapping and the listing it was read from was canonicalised by an
    /// earlier pass, and running it again would mint a second product the
    /// catalogue has no way to refuse.
    #[must_use]
    pub const fn is_canonicalised(&self) -> bool {
        self.product.is_some() && self.mapping.is_some() && self.source.is_some()
    }
}

/// One resource's outcome, bundled because the values are meaningless apart:
/// the breadcrumb is the whole row or it is not a breadcrumb.
///
/// The source listing and its observed lifecycle are here for the same reason
/// the product and the mapping are. A migrate's removal item names the listing
/// it takes down and the state it takes it down from, both from the read that
/// already happened; a breadcrumb that omitted them would let a resumed pass
/// skip the work and lose its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canonicalised {
    pub request: Uuid,
    pub ordinal: i32,
    pub product: ProductId,
    pub mapping: MappingId,
    pub source: RemoteListingId,
    pub source_state: Option<ListingState>,
}

/// The jobs one request produced. A migrate names both; a sync names the
/// create alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enqueued {
    pub request: Uuid,
    pub create_job: Option<Uuid>,
    pub remove_job: Option<Uuid>,
    pub at: Timestamp,
}

impl SyncRequestRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The request and its resources in one transaction, because a request
    /// with no resources is a request the drain would settle as finished.
    ///
    /// Returns whether this call is the one that wrote it. The request's
    /// identity is the seller's idempotency key, so a second submit under one
    /// key is the retry the endpoint exists to absorb, and the conflict is
    /// resolved here rather than by a read the caller then races: two submits
    /// both saw no existing row and the loser's INSERT violated the primary
    /// key, turning the double-click into a fault.
    pub async fn create(&self, org: OrgId, new: &NewSyncRequest) -> Result<bool, StorageError> {
        // A request whose source is a device-branch marketplace starts empty
        // and is filled by the pages the seller's own device posts, because
        // under D1 the device is the only thing that can enumerate that
        // catalogue. Every other request names its resources up front, and an
        // empty one there is a migration that would settle complete having
        // moved nothing.
        //
        // The guard is here rather than only at the API because it is a
        // property of the row: the transport class is a fact about the source
        // marketplace, so the write can decide it without being told.
        if new.locators.is_empty()
            && new.source.marketplace().transport_class() != TransportClass::SellerDevice
        {
            return Err(StorageError::Inconsistent {
                reason: "a sync request on a server-branch source names at least one resource"
                    .to_owned(),
            });
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO sync_request \
             (org_id, id, source, target, disposition, intent, state, requested_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7) \
             ON CONFLICT (org_id, id) DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(new.id),
            inventory_to_db(new.source),
            inventory_to_db(new.target),
            new.disposition.as_str(),
            new.intent.as_str(),
            timestamp_to_db(new.requested_at)?,
        )
        .execute(&mut *tx)
        .await?;
        if written.rows_affected() == 0 {
            // The resources are the request's, so a replay writes none of
            // them: appending them to the first submit's row would give it a
            // second copy of every locator.
            return Ok(false);
        }
        for (ordinal, locator) in new.locators.iter().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(|_| StorageError::Inconsistent {
                reason: "a sync request holds fewer resources than this".to_owned(),
            })?;
            sqlx::query!(
                "INSERT INTO sync_request_resource \
                 (org_id, request_id, ordinal, locator, state) \
                 VALUES ($1, $2, $3, $4, 'pending')",
                uuid_to_db(org.0),
                uuid_to_db(new.id),
                ordinal,
                locator,
            )
            .execute(&mut *tx)
            .await?;
        }
        crate::rule_capture::confirm_request_snapshot(
            &mut tx,
            org,
            &crate::rule_capture::RequestCapture {
                request: new.id,
                source: new.source,
                target: new.target,
                use_: crate::rule_capture::scope_of(new.disposition),
                products: None,
                at: new.requested_at,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// A request whose resources are canonicalised as they are written.
    ///
    /// [`create`] writes locators the drain has yet to resolve, because a
    /// device-enumerated import is the only thing that can turn a seller's
    /// marketplace address into a product. A migration between two
    /// marketplaces moves resources the catalogue already holds: the product
    /// exists, the source mapping is bound, and the listing identifier the
    /// removal leg needs is already in that binding. There is nothing left for
    /// a read to discover, so the breadcrumb is written up front and the drain
    /// skips straight to minting the jobs.
    ///
    /// One transaction for the head and its resources, and the same
    /// `ON CONFLICT DO NOTHING` idempotency [`create`] has: the request's
    /// identity is the seller's key, so a double-clicked confirm is one
    /// migration.
    ///
    /// [`create`]: Self::create
    pub async fn create_canonicalised(
        &self,
        org: OrgId,
        new: &NewMigration,
    ) -> Result<bool, StorageError> {
        // A migration with no resources would settle complete having moved
        // nothing, which is `create`'s rule for a server-branch source and is
        // this path's rule unconditionally: nothing here is device-enumerated,
        // because the selection is the catalogue's own.
        if new.resources.is_empty() {
            return Err(StorageError::Inconsistent {
                reason: "a migration names at least one resource".to_owned(),
            });
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO sync_request \
             (org_id, id, source, target, disposition, intent, state, requested_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7) \
             ON CONFLICT (org_id, id) DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(new.id),
            inventory_to_db(new.source),
            inventory_to_db(new.target),
            new.disposition.as_str(),
            new.intent.as_str(),
            timestamp_to_db(new.requested_at)?,
        )
        .execute(&mut *tx)
        .await?;
        if written.rows_affected() == 0 {
            return Ok(false);
        }
        for (ordinal, resource) in new.resources.iter().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(|_| StorageError::Inconsistent {
                reason: "a migration holds fewer resources than this".to_owned(),
            })?;
            let columns = RemoteIdColumns::encode(&resource.source)?;
            sqlx::query!(
                "INSERT INTO sync_request_resource \
                 (org_id, request_id, ordinal, locator, state, product_id, mapping_id, \
                  source_kind, source_url, source_numeric_id, source_state) \
                 VALUES ($1, $2, $3, $4, 'canonicalised', $5, $6, $7, $8, $9, $10)",
                uuid_to_db(org.0),
                uuid_to_db(new.id),
                ordinal,
                resource.locator,
                uuid_to_db(resource.product.0),
                uuid_to_db(resource.mapping.0),
                columns.kind,
                columns.url,
                columns.numeric_id,
                resource.source_state.map(listing_state_to_db),
            )
            .execute(&mut *tx)
            .await?;
        }
        let products: Vec<_> = new
            .resources
            .iter()
            .map(|resource| resource.product)
            .collect();
        crate::rule_capture::confirm_request_snapshot(
            &mut tx,
            org,
            &crate::rule_capture::RequestCapture {
                request: new.id,
                source: new.source,
                target: new.target,
                use_: crate::rule_capture::scope_of(new.disposition),
                products: Some(&products),
                at: new.requested_at,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// One page of the organisation's sync requests, newest first.
    ///
    /// The console's only way to reach a request it created and navigated away
    /// from. A device-branch migrate mints no job until its completing page, so
    /// until then it appears in no job list and, without this, nowhere at all —
    /// which left the console telling a seller to go back to a page it could
    /// not link to.
    ///
    /// The two counts are computed here rather than by loading every
    /// breadcrumb, because a page of requests over a five-hundred-resource
    /// shop is thousands of rows to answer a question about two numbers.
    ///
    /// The narrowing is a `WHERE` clause and the keyset is applied with it,
    /// before the `LIMIT`. A page filtered after the limit answers "the
    /// migrations among the newest ten requests", which is empty for a seller
    /// whose last ten requests were syncs — and reads as "you have never
    /// migrated anything".
    pub async fn list(
        &self,
        org: OrgId,
        page: &SyncRequestPage,
    ) -> Result<Vec<SyncRequestSummary>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let (cursor_at, cursor_id) = match page.after {
            Some((at, id)) => (Some(timestamp_to_db(at)?), Some(uuid_to_db(id))),
            None => (None, None),
        };
        let disposition = page.disposition.map(Disposition::as_str);
        let rows = sqlx::query!(
            r#"SELECT r.id, r.source, r.target, r.disposition, r.intent, r.state, r.requested_at,
                      r.deletion_state,
                      (SELECT count(*) FROM sync_request_resource s
                        WHERE s.org_id = r.org_id AND s.request_id = r.id) AS "total!",
                      (SELECT count(*) FROM sync_request_resource s
                        WHERE s.org_id = r.org_id AND s.request_id = r.id
                          AND s.state = 'failed') AS "failed!"
               FROM sync_request r
               WHERE r.org_id = $1
                 AND r.deletion_state IS DISTINCT FROM 'deleted'
                 AND ($3::timestamptz IS NULL OR (r.requested_at, r.id) < ($3, $4))
                 AND ($5::text IS NULL OR r.disposition = $5)
                 AND ($6::text IS NULL OR r.state = $6)
               ORDER BY r.requested_at DESC, r.id DESC
               LIMIT $2"#,
            uuid_to_db(org.0),
            page.limit,
            cursor_at,
            cursor_id,
            disposition,
            page.state.as_deref(),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(SyncRequestSummary {
                    id: uuid_from_db(row.id),
                    source: inventory_from_db(&row.source)?,
                    target: inventory_from_db(&row.target)?,
                    disposition: disposition_from_db(&row.disposition)?,
                    intent: intent_from_db(&row.intent)?,
                    state: row.state,
                    requested_at: timestamp_from_db(row.requested_at),
                    resources_total: count_from_db(i32::try_from(row.total).unwrap_or(i32::MAX))?,
                    deletion: DeletionStatus::parse(row.deletion_state.as_deref())?,
                    resources_failed: count_from_db(i32::try_from(row.failed).unwrap_or(i32::MAX))?,
                })
            })
            .collect()
    }

    pub async fn get(
        &self,
        org: OrgId,
        request: Uuid,
    ) -> Result<Option<SyncRequestRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let head = sqlx::query!(
            // A tombstoned request is gone as far as every reader of this
            // repository is concerned, and that includes the drain: a late
            // pass finds no request, mints nothing and says so, which is
            // exactly the fence a deleted migration needs.
            "SELECT source, target, disposition, intent, state, create_job_id, remove_job_id, \
                    failure_detail, requested_at \
             FROM sync_request WHERE org_id = $1 AND id = $2 \
               AND deletion_state IS DISTINCT FROM 'deleted'",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(head) = head else {
            tx.commit().await?;
            return Ok(None);
        };
        let rows = sqlx::query!(
            "SELECT ordinal, locator, state, product_id, mapping_id, \
                    source_kind, source_url, source_numeric_id, source_state, failure_detail, \
                    terms_seen, terms_mapped, terms_unmapped, terms_uncovered \
             FROM sync_request_resource WHERE org_id = $1 AND request_id = $2 ORDER BY ordinal",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(SyncRequestRecord {
            id: request,
            org,
            source: inventory_from_db(&head.source)?,
            target: inventory_from_db(&head.target)?,
            disposition: disposition_from_db(&head.disposition)?,
            intent: intent_from_db(&head.intent)?,
            state: head.state,
            create_job: head.create_job_id.map(uuid_from_db),
            remove_job: head.remove_job_id.map(uuid_from_db),
            failure_detail: head.failure_detail,
            requested_at: timestamp_from_db(head.requested_at),
            resources: rows
                .into_iter()
                .map(|row| {
                    Ok(SyncResourceRecord {
                        ordinal: row.ordinal,
                        locator: row.locator,
                        state: row.state,
                        coverage: coverage_from_db(
                            row.terms_seen,
                            row.terms_mapped,
                            row.terms_unmapped,
                            row.terms_uncovered,
                        )?,
                        product: row.product_id.map(|id| ProductId(uuid_from_db(id))),
                        mapping: row.mapping_id.map(|id| MappingId(uuid_from_db(id))),
                        source: row
                            .source_kind
                            .map(|kind| {
                                remote_id_from_db(&kind, row.source_url, row.source_numeric_id)
                            })
                            .transpose()?,
                        source_state: row
                            .source_state
                            .as_deref()
                            .map(listing_state_from_db)
                            .transpose()?,
                        failure_detail: row.failure_detail,
                    })
                })
                .collect::<Result<Vec<_>, StorageError>>()?,
        }))
    }

    /// One request's head, its whole-request figures, and one page of its
    /// breadcrumbs.
    ///
    /// Separate from [`SyncRequestRepo::get`] rather than a limit on it,
    /// because the two reads answer different questions. The drain and the
    /// device-apply route need every breadcrumb — a redrain that saw a page
    /// would canonicalise a resource twice — while the console needs a
    /// request's state, its coverage and the twenty-five rows on screen. The
    /// figures are counted in SQL over the whole request for exactly that
    /// reason: the page they are shown beside is not what they are about.
    pub async fn detail(
        &self,
        org: OrgId,
        request: Uuid,
        after_ordinal: Option<i32>,
        limit: i64,
    ) -> Result<Option<SyncRequestDetail>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let request_db = uuid_to_db(request);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let head = sqlx::query!(
            // Hidden for the reason `get` gives.
            "SELECT source, target, disposition, intent, state, create_job_id, remove_job_id, \
                    failure_detail, requested_at, deletion_state \
             FROM sync_request WHERE org_id = $1 AND id = $2 \
               AND deletion_state IS DISTINCT FROM 'deleted'",
            org_db,
            request_db,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(head) = head else {
            tx.commit().await?;
            return Ok(None);
        };
        let counted = sqlx::query!(
            "SELECT state, count(*) AS \"count!\" FROM sync_request_resource \
             WHERE org_id = $1 AND request_id = $2 GROUP BY state",
            org_db,
            request_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        // The measured rows only, and `rows` counts them rather than every
        // breadcrumb: the four sums are null when nothing measured, which is
        // what tells "never measured" from "measured, and the answer was
        // zero".
        let summed = sqlx::query!(
            "SELECT count(*) FILTER (WHERE terms_seen IS NOT NULL) AS \"rows!\", \
                    COALESCE(sum(terms_seen), 0) AS \"seen!\", \
                    COALESCE(sum(terms_mapped), 0) AS \"mapped!\", \
                    COALESCE(sum(terms_unmapped), 0) AS \"unmapped!\", \
                    COALESCE(sum(terms_uncovered), 0) AS \"uncovered!\" \
             FROM sync_request_resource WHERE org_id = $1 AND request_id = $2",
            org_db,
            request_db,
        )
        .fetch_one(&mut *tx)
        .await?;
        // One row more than the page, so a page that filled exactly is told
        // apart from one that ended there. `next_ordinal` minted from a full
        // page alone would offer an empty page at every exact boundary.
        let rows = sqlx::query!(
            "SELECT ordinal, locator, state, product_id, mapping_id, \
                    source_kind, source_url, source_numeric_id, source_state, failure_detail, \
                    terms_seen, terms_mapped, terms_unmapped, terms_uncovered \
             FROM sync_request_resource \
             WHERE org_id = $1 AND request_id = $2 \
               AND ($3::integer IS NULL OR ordinal > $3) \
             ORDER BY ordinal LIMIT $4",
            org_db,
            request_db,
            after_ordinal,
            limit.saturating_add(1),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let wanted = usize::try_from(limit).unwrap_or(usize::MAX);
        let more = rows.len() > wanted;
        let mut resources = Vec::with_capacity(rows.len().min(wanted));
        for row in rows.into_iter().take(wanted) {
            resources.push(SyncResourceRecord {
                ordinal: row.ordinal,
                locator: row.locator,
                state: row.state,
                coverage: coverage_from_db(
                    row.terms_seen,
                    row.terms_mapped,
                    row.terms_unmapped,
                    row.terms_uncovered,
                )?,
                product: row.product_id.map(|id| ProductId(uuid_from_db(id))),
                mapping: row.mapping_id.map(|id| MappingId(uuid_from_db(id))),
                source: row
                    .source_kind
                    .map(|kind| remote_id_from_db(&kind, row.source_url, row.source_numeric_id))
                    .transpose()?,
                source_state: row
                    .source_state
                    .as_deref()
                    .map(listing_state_from_db)
                    .transpose()?,
                failure_detail: row.failure_detail,
            });
        }
        let next_ordinal = more
            .then(|| resources.last().map(|last| last.ordinal))
            .flatten();
        let measured_rows = count_from_db(i32::try_from(summed.rows).unwrap_or(i32::MAX))?;
        Ok(Some(SyncRequestDetail {
            head: SyncRequestHead {
                id: request,
                org,
                source: inventory_from_db(&head.source)?,
                target: inventory_from_db(&head.target)?,
                disposition: disposition_from_db(&head.disposition)?,
                intent: intent_from_db(&head.intent)?,
                state: head.state,
                create_job: head.create_job_id.map(uuid_from_db),
                remove_job: head.remove_job_id.map(uuid_from_db),
                failure_detail: head.failure_detail,
                requested_at: timestamp_from_db(head.requested_at),
                deletion: DeletionStatus::parse(head.deletion_state.as_deref())?,
            },
            counts: counted
                .into_iter()
                .map(|row| {
                    Ok(SyncResourceStateCount {
                        state: row.state,
                        count: count_from_db(i32::try_from(row.count).unwrap_or(i32::MAX))?,
                    })
                })
                .collect::<Result<Vec<_>, StorageError>>()?,
            coverage: (measured_rows > 0).then(|| SyncCoverageTotals {
                rows: measured_rows,
                terms_seen: sum_from_db(summed.seen),
                terms_mapped: sum_from_db(summed.mapped),
                terms_unmapped: sum_from_db(summed.unmapped),
                terms_uncovered: sum_from_db(summed.uncovered),
            }),
            resources,
            next_ordinal,
        }))
    }

    /// Every tenant, because the drain runs as `tam_app` under forced RLS and
    /// cannot scan `sync_request` across organisations at all.
    ///
    /// That is the posture rather than a limitation to route around: the
    /// drain pins one organisation per request and never reads two tenants'
    /// rows in one statement, which is a stronger guarantee than the item
    /// pump's BYPASSRLS scan gives. `organisation` is the tenant root and
    /// carries no `org_id`, so listing it needs no pin.
    pub async fn tenants(&self) -> Result<Vec<OrgId>, StorageError> {
        let rows = sqlx::query!("SELECT id FROM organisation ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| OrgId(uuid_from_db(row.id)))
            .collect())
    }

    /// Links a legacy migration's event anchor to the request it belongs to,
    /// and answers how many anchors this pass linked.
    ///
    /// A migration's anchor is the itemless job its `job_event` rows hang
    /// from, minted by the first page under [`IMPORT_LEG`]. Anchors minted
    /// before that leg named its request carry no origin link at all, so
    /// `JobRepo::owner` reads them as standalone work: deleting one reports
    /// `deleted` without fencing its request, and the device's next page
    /// reuses the same idempotency key, imports resources and mints the
    /// create leg over a request the seller was told is gone.
    ///
    /// The link is derived rather than guessed. [`job_request_key`] is a
    /// function of the request and the leg, so the anchor of a given request
    /// is the job holding exactly that key -- the same derivation the page
    /// that minted it used, run in reverse. No prose, no instant window and
    /// no shape heuristic takes part.
    ///
    /// One maintenance pass, run once per tenant at deployment, not a
    /// fallback any read consults: after it, ownership is a column again.
    /// Bounded and restartable by construction -- the requests are walked in
    /// `id` order in fixed batches, each batch is its own transaction, and
    /// the anchors of a batch are locked in `id` order before anything is
    /// written, which is `fence_owned_jobs`' own ordering. The pass appends
    /// no event, so it never holds the organisation's event counter and
    /// cannot deadlock with a settling lease.
    ///
    /// What it refuses to touch, and what it refuses outright:
    ///
    ///   * a job already naming this request is left exactly as it is, which
    ///     is what makes a second run move zero rows;
    ///   * an anchor naming an import -- `import_run_id`, or a run naming it
    ///     as `anchor_job` -- is a native device import's own anchor and
    ///     keeps that ownership, because reassigning it would move the fence
    ///     off the import that actually owns the work;
    ///   * a job holding a request's import key while naming a *different*
    ///     request, carrying items, or attributed to something other than
    ///     the import component is not an anchor this pass can explain, and
    ///     it is reported rather than rewritten. Linking it would fence
    ///     real work against a workflow that did not produce it.
    pub async fn normalize_migration_anchors(&self, org: OrgId) -> Result<u64, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut linked = 0_u64;
        let mut after: Option<uuid::Uuid> = None;
        loop {
            let mut tx = self.pool.begin().await?;
            pin_org(&mut tx, org).await?;
            // Every request this tenant retains, tombstoned ones included: a
            // deleted request's anchor is exactly the one whose next page
            // must be refused, so skipping it would leave the hole open on
            // the rows that need it most.
            let requests = sqlx::query_scalar!(
                "SELECT id FROM sync_request \
                  WHERE org_id = $1 AND ($2::uuid IS NULL OR id > $2) \
                  ORDER BY id LIMIT $3",
                org_db,
                after,
                ANCHOR_BATCH,
            )
            .fetch_all(&mut *tx)
            .await?;
            let Some(last) = requests.last().copied() else {
                tx.commit().await?;
                return Ok(linked);
            };
            after = Some(last);
            let mut owner_of: std::collections::HashMap<uuid::Uuid, uuid::Uuid> =
                std::collections::HashMap::with_capacity(requests.len());
            for request in &requests {
                owner_of.insert(
                    uuid_to_db(job_request_key(uuid_from_db(*request), IMPORT_LEG)),
                    *request,
                );
            }
            let keys: Vec<uuid::Uuid> = owner_of.keys().copied().collect();
            // Callers quiesce API and worker writers before this maintenance
            // pass. Lock matching jobs in identity order within each batch.
            let candidates = sqlx::query!(
                "SELECT id, request_idempotency_key AS \"key!\", sync_request_id, \
                        import_run_id, actor_kind, actor_id, \
                        EXISTS (SELECT 1 FROM job_item item \
                                 WHERE item.org_id = job.org_id AND item.job_id = job.id) \
                            AS \"has_items!\", \
                        EXISTS (SELECT 1 FROM import_run run \
                                 WHERE run.org_id = job.org_id AND run.anchor_job = job.id) \
                            AS \"anchors_import!\" \
                   FROM job \
                  WHERE job.org_id = $1 AND job.request_idempotency_key = ANY($2) \
                  ORDER BY job.id \
                    FOR UPDATE",
                org_db,
                &keys,
            )
            .fetch_all(&mut *tx)
            .await?;
            let mut anchors: Vec<uuid::Uuid> = Vec::new();
            let mut owners: Vec<uuid::Uuid> = Vec::new();
            for row in candidates {
                let request =
                    owner_of
                        .get(&row.key)
                        .copied()
                        .ok_or_else(|| StorageError::Inconsistent {
                            reason: format!("anchor {} returned an unrequested import key", row.id),
                        })?;
                if row.has_items
                    || row.actor_kind != "system"
                    || row.actor_id.as_deref() != Some(SystemComponent::Import.as_str())
                {
                    return Err(StorageError::Inconsistent {
                        reason: format!(
                            "job {} holds the import key of sync request {request} but is not \
                             an itemless import anchor",
                            row.id,
                        ),
                    });
                }
                if row.import_run_id.is_some() || row.anchors_import {
                    continue;
                }
                match row.sync_request_id {
                    Some(held) if held == request => {}
                    Some(held) => {
                        return Err(StorageError::Inconsistent {
                            reason: format!(
                                "anchor {} holds the import key of sync request {request} but \
                                 names sync request {held}",
                                row.id,
                            ),
                        })
                    }
                    None => {
                        anchors.push(row.id);
                        owners.push(request);
                    }
                }
            }
            if !anchors.is_empty() {
                linked += sqlx::query!(
                    "UPDATE job SET sync_request_id = link.request_id \
                       FROM unnest($2::uuid[], $3::uuid[]) AS link(anchor, request_id) \
                      WHERE job.org_id = $1 AND job.id = link.anchor \
                        AND job.sync_request_id IS NULL",
                    org_db,
                    &anchors,
                    &owners,
                )
                .execute(&mut *tx)
                .await?
                .rows_affected();
            }
            tx.commit().await?;
            if i64::try_from(requests.len()).unwrap_or(ANCHOR_BATCH) < ANCHOR_BATCH {
                return Ok(linked);
            }
        }
    }

    /// The drain's own work list. Ordered oldest first so a backlog drains in
    /// the order sellers asked, and scoped to one tenant because this repo is
    /// only ever reached through a pinned connection.
    pub async fn pending(&self, org: OrgId, limit: i64) -> Result<Vec<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            // A deleted request is not drainable, whatever state it was in
            // when the seller stopped it. The Delete settles a pending or
            // draining request as well, so this predicate is belt beside
            // braces — and it is the belt that holds if a future state is
            // added to the drain's list.
            "SELECT id FROM sync_request WHERE org_id = $1 AND state IN ('pending', 'draining') \
               AND deletion_requested_at IS NULL \
             ORDER BY requested_at, id LIMIT $2",
            uuid_to_db(org.0),
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(|row| uuid_from_db(row.id)).collect())
    }

    pub async fn mark_draining(&self, org: OrgId, request: Uuid) -> Result<(), StorageError> {
        self.set_state(org, request, "draining", None).await
    }

    /// The breadcrumb, written in the same transaction that marks the
    /// resource done. Two statements would leave a window in which the
    /// product exists and nothing records that it does, which is the orphan
    /// this column exists to remove.
    pub async fn record_canonicalised(
        &self,
        org: OrgId,
        done: &Canonicalised,
    ) -> Result<(), StorageError> {
        let Canonicalised {
            request,
            ordinal,
            product,
            mapping,
            source,
            source_state,
        } = done;
        let columns = RemoteIdColumns::encode(source)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let affected = sqlx::query!(
            "UPDATE sync_request_resource \
             SET state = 'canonicalised', product_id = $4, mapping_id = $5, \
                 source_kind = $6, source_url = $7, source_numeric_id = $8, \
                 source_state = $9, failure_detail = NULL, admitted_at = NULL \
             WHERE org_id = $1 AND request_id = $2 AND ordinal = $3",
            uuid_to_db(org.0),
            uuid_to_db(*request),
            ordinal,
            uuid_to_db(product.0),
            uuid_to_db(mapping.0),
            columns.kind,
            columns.url,
            columns.numeric_id,
            source_state.map(listing_state_to_db),
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if affected == 0 {
            return Err(StorageError::Inconsistent {
                reason: "no such sync request resource".to_owned(),
            });
        }
        Ok(())
    }

    /// Admits one resource of a running page before any of its catalogue
    /// writes begin.
    ///
    /// `import_one` commits four times internally, so the fence has to go up
    /// before the first of those rather than after the last: a Delete that
    /// saw nothing outstanding would report the request gone while the page
    /// it raced carried on minting products for it. The admission is that
    /// fence in both directions — nothing is admitted once the request
    /// carries a deletion, and an admitted resource is what makes that
    /// deletion answer `stopping` until the one it let through is finished.
    ///
    /// `admitted_at` rather than the row's `'pending'` state, because that
    /// state already means something else: the drain's own requests
    /// pre-create a pending row per resource the seller listed, and reading
    /// those as work in progress would leave every unstarted sync's deletion
    /// reporting `stopping` for ever. The instant says an apply is holding
    /// this row right now; the state says the resource is not finished.
    ///
    /// Idempotent on the locator, which is what keeps a page recoverable: a
    /// re-post whose earlier attempt died mid-apply is re-admitted against
    /// the row it left behind, rather than being refused as a new start or
    /// stopping the request for ever.
    pub async fn admit_resource(
        &self,
        org: OrgId,
        request: Uuid,
        locator: &str,
        at: Timestamp,
    ) -> Result<ResourceAdmission, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let admission = Self::admit_resource_in(&mut tx, org, request, locator, at).await?;
        tx.commit().await?;
        Ok(admission)
    }

    /// Rechecks admission while holding the owning request until the caller
    /// commits its resource and receipt together. The transaction must already
    /// be pinned to `org`; preparation must precede it.
    pub async fn admit_resource_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: OrgId,
        request: Uuid,
        locator: &str,
        at: Timestamp,
    ) -> Result<ResourceAdmission, StorageError> {
        let at_db = timestamp_to_db(at)?;
        // The request row exclusively, and first: Delete takes the same row
        // the same way, so one of the two waits and then reads the other's
        // committed decision instead of both deciding on a stale snapshot.
        let Some(deleted) = sqlx::query_scalar!(
            "SELECT deletion_requested_at IS NOT NULL AS \"deleted!\" FROM sync_request \
             WHERE org_id = $1 AND id = $2 FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .fetch_optional(&mut **tx)
        .await?
        else {
            return Ok(ResourceAdmission::Stopped);
        };
        // A retry may finish an earlier admission after deletion. Merely
        // appearing in the request's original resource list is not admission.
        let standing = sqlx::query!(
            "UPDATE sync_request_resource SET admitted_at = $4 \
             WHERE org_id = $1 AND request_id = $2 AND locator = $3 \
               AND state = 'pending' \
               AND (NOT $5 OR admitted_at IS NOT NULL) \
             RETURNING ordinal",
            uuid_to_db(org.0),
            uuid_to_db(request),
            locator,
            at_db,
            deleted,
        )
        .fetch_optional(&mut **tx)
        .await?;
        if let Some(standing) = standing {
            return Ok(ResourceAdmission::Admitted(standing.ordinal));
        }
        if deleted {
            return Ok(ResourceAdmission::Stopped);
        }
        let finished = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM sync_request_resource \
                             WHERE org_id = $1 AND request_id = $2 AND locator = $3) \
                    AS \"finished!\"",
            uuid_to_db(org.0),
            uuid_to_db(request),
            locator,
        )
        .fetch_one(&mut **tx)
        .await?;
        if finished {
            return Ok(ResourceAdmission::AlreadyDescribed);
        }
        let ordinal = sqlx::query_scalar!(
            "INSERT INTO sync_request_resource \
             (org_id, request_id, ordinal, locator, state, admitted_at) \
             SELECT $1, $2, \
                    COALESCE((SELECT MAX(ordinal) FROM sync_request_resource \
                              WHERE org_id = $1 AND request_id = $2), -1) + 1, \
                    $3, 'pending', $4 \
             RETURNING ordinal",
            uuid_to_db(org.0),
            uuid_to_db(request),
            locator,
            at_db,
        )
        .fetch_one(&mut **tx)
        .await?;
        Ok(ResourceAdmission::Admitted(ordinal))
    }

    /// Records an admitted resource in the same tenant-pinned transaction as
    /// its product and mappings. Call `admit_resource_in` first so concurrent
    /// replays cannot commit another product for this locator.
    pub async fn append_observed(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: OrgId,
        request: Uuid,
        observed: &Observed<'_>,
    ) -> Result<(), StorageError> {
        let columns = RemoteIdColumns::encode(observed.source)?;
        let upgraded = sqlx::query!(
            "UPDATE sync_request_resource \
             SET state = 'canonicalised', product_id = $4, mapping_id = $5, \
                 source_kind = $6, source_url = $7, source_numeric_id = $8, \
                 source_state = $9, failure_detail = NULL, admitted_at = NULL, \
                 terms_seen = $10, terms_mapped = $11, terms_unmapped = $12, \
                 terms_uncovered = $13 \
             WHERE org_id = $1 AND request_id = $2 AND locator = $3 \
               AND state = 'pending' AND admitted_at IS NOT NULL",
            uuid_to_db(org.0),
            uuid_to_db(request),
            observed.locator,
            uuid_to_db(observed.product.0),
            uuid_to_db(observed.mapping.0),
            columns.kind,
            columns.url,
            columns.numeric_id,
            observed.source_state.map(listing_state_to_db),
            count_to_db(observed.coverage.terms_seen),
            count_to_db(observed.coverage.terms_mapped),
            count_to_db(observed.coverage.terms_unmapped),
            count_to_db(observed.coverage.terms_uncovered),
        )
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if upgraded != 1 {
            return Err(StorageError::Inconsistent {
                reason: format!(
                    "resource {} has no pending admission in sync request {}",
                    observed.locator,
                    uuid_to_db(request),
                ),
            });
        }
        Ok(())
    }

    /// Appends one resource the device could not describe, as a failed
    /// breadcrumb carrying the device's own reason.
    ///
    /// A skip is recorded rather than dropped because completion is what mints
    /// the write jobs: a request that completed without saying what did not
    /// cross would report success for a partial catalogue, and the seller
    /// would find out by noticing something missing from their own shop. It is
    /// never a product, so no create item is ever minted for it.
    pub async fn append_skipped(
        &self,
        org: OrgId,
        request: Uuid,
        locator: &str,
        why: &str,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO sync_request_resource \
             (org_id, request_id, ordinal, locator, state, failure_detail) \
             SELECT $1, $2, \
                    COALESCE((SELECT MAX(ordinal) FROM sync_request_resource \
                              WHERE org_id = $1 AND request_id = $2), -1) + 1, \
                    $3, 'failed', $4 \
             WHERE NOT EXISTS (SELECT 1 FROM sync_request_resource \
                               WHERE org_id = $1 AND request_id = $2 AND locator = $3)",
            uuid_to_db(org.0),
            uuid_to_db(request),
            locator,
            why,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(written == 1)
    }

    /// Settles a device import and mints its create job in ONE transaction.
    ///
    /// The two halves are one fact and are written as one. Doing them
    /// separately — `create_with_request_key` then `record_enqueued`, which is
    /// what the cron drain does — leaves a window where the job exists and the
    /// request still reads `draining`. The drain survives that by re-running on
    /// its next tick; a device does not, because it gets one answer to its
    /// completing page and a 200 is not retried, so a crash in the window would
    /// leave the seller's request permanently mid-flight beside a job nobody
    /// had told it about.
    ///
    /// `mint` is `None` where the catalogue held nothing to publish. That is a
    /// completion rather than a failure, and it deliberately mints no job: an
    /// itemless job reads back settled, because zero settled of zero is
    /// complete, so creating one would report a migration finished when there
    /// was never anything in it.
    pub async fn complete_with_create_job(
        &self,
        org: OrgId,
        completion: &Completion<'_>,
    ) -> Result<Option<CreatedJob>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let created = match completion.mint.as_ref() {
            None => None,
            Some(mint) => {
                match crate::jobs::create_job_in_tx(
                    &mut tx,
                    org,
                    crate::jobs::JobOrigin {
                        request_key: mint.request_key,
                        run: Some(completion.request),
                        import_run: None,
                    },
                    mint.job,
                    mint.items,
                )
                .await?
                {
                    crate::jobs::JobWrite::Created => Some(CreatedJob {
                        job: mint.job.job,
                        replay: false,
                    }),
                    // The key was already spent, which means an earlier
                    // completing page minted this job and settled this request.
                    // The failed insert aborted the transaction, so nothing here
                    // is written and the caller is told it is a replay.
                    crate::jobs::JobWrite::KeyAlreadyUsed => {
                        drop(tx);
                        return Ok(Some(CreatedJob {
                            job: mint.job.job,
                            replay: true,
                        }));
                    }
                    // The seller deleted the migration while its device was
                    // still describing the shop. Nothing is minted and the
                    // request is left exactly as the Delete settled it: the
                    // device is answered with no job, which is the truth —
                    // the catalogue keeps what earlier pages committed and
                    // nothing will be published from it.
                    crate::jobs::JobWrite::RunDeleted => {
                        tx.rollback().await?;
                        return Ok(None);
                    }
                    crate::jobs::JobWrite::ImportDeleted => {
                        return Err(StorageError::Inconsistent {
                            reason: "a sync-request mint without an import origin returned an import deletion".into(),
                        });
                    }
                }
            }
        };
        sqlx::query!(
            "UPDATE sync_request \
             SET state = 'enqueued', create_job_id = $3, settled_at = $4 \
             WHERE org_id = $1 AND id = $2 AND state <> 'enqueued' \
               AND deletion_requested_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(completion.request),
            created.map(|job| uuid_to_db(job.job.0)),
            timestamp_to_db(completion.at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(created)
    }

    /// Moves a request from `pending` to `draining` on its first page, and
    /// leaves it alone once it is already there.
    pub async fn mark_draining_if_pending(
        &self,
        org: OrgId,
        request: Uuid,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE sync_request SET state = 'draining' \
             WHERE org_id = $1 AND id = $2 AND state = 'pending'",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One resource's failure, which is also the release of whatever
    /// admission was holding it: a failed resource is finished, and a
    /// deletion waiting on it has nothing further to wait for.
    pub async fn record_resource_failure(
        &self,
        org: OrgId,
        request: Uuid,
        ordinal: i32,
        detail: &str,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        // Serialize with an applying replay before deciding whether the
        // failed attempt still owns an unfinished admission.
        sqlx::query_scalar!(
            "SELECT id FROM sync_request WHERE org_id = $1 AND id = $2 FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .fetch_optional(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE sync_request_resource \
             SET state = 'failed', failure_detail = $4, admitted_at = NULL \
             WHERE org_id = $1 AND request_id = $2 AND ordinal = $3 \
               AND state = 'pending' AND admitted_at IS NOT NULL",
            uuid_to_db(org.0),
            uuid_to_db(request),
            ordinal,
            detail,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The jobs the request produced, recorded with the terminal state so a
    /// seller polling the request is never told it finished without being
    /// told where to look next.
    ///
    /// Never over a deleted request. This runs after the legs have been
    /// minted and committed, so a Delete landing in between has already
    /// settled the request `failed` and stamped its tombstone — writing
    /// `enqueued` over that would resurrect the row in the seller's list and
    /// clear the reason it carries. The legs themselves are not lost by the
    /// refusal: a job names its request as it is minted, so the deletion
    /// finds them through `job.sync_request_id` whether or not these columns
    /// were ever written.
    pub async fn record_enqueued(
        &self,
        org: OrgId,
        enqueued: &Enqueued,
    ) -> Result<(), StorageError> {
        let Enqueued {
            request,
            create_job,
            remove_job,
            at,
        } = *enqueued;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE sync_request \
             SET state = 'enqueued', create_job_id = $3, remove_job_id = $4, settled_at = $5 \
             WHERE org_id = $1 AND id = $2 AND deletion_requested_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(request),
            create_job.map(uuid_to_db),
            remove_job.map(uuid_to_db),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn record_failure(
        &self,
        org: OrgId,
        request: Uuid,
        detail: &str,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        self.set_state(org, request, "failed", Some((detail, at)))
            .await
    }

    async fn set_state(
        &self,
        org: OrgId,
        request: Uuid,
        state: &str,
        failure: Option<(&str, Timestamp)>,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let (detail, settled) = match failure {
            Some((detail, at)) => (Some(detail), Some(timestamp_to_db(at)?)),
            None => (None, None),
        };
        sqlx::query!(
            "UPDATE sync_request SET state = $3, failure_detail = $4, settled_at = $5 \
             WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(request),
            state,
            detail,
            settled,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Stops a sync or a migration, both legs together, and answers how far
    /// the removal got.
    ///
    /// One transaction for the request and every job it owns, which is the
    /// property a migration needs: the removal leg takes a listing down on
    /// the *source* marketplace, so fencing the create and leaving the
    /// removal for a second statement would let a drain or a claim start the
    /// leg that removes the seller's original listing after they asked for
    /// the whole migration to stop.
    ///
    /// An unsettled request is settled `failed` with the seller's own reason,
    /// which is the existing terminal transition rather than a new state: a
    /// failed request is not in the drain's work list, mints nothing, and
    /// already reads correctly everywhere a state is rendered.
    ///
    /// `None` is a request this organisation does not have.
    pub async fn delete(
        &self,
        org: OrgId,
        request: Uuid,
        stamp: Stamp,
    ) -> Result<Option<DeletionStatus>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(legs) = sqlx::query!(
            "UPDATE sync_request \
             SET deletion_requested_at = COALESCE(deletion_requested_at, $3), \
                 deletion_actor_kind = COALESCE(deletion_actor_kind, $4), \
                 deletion_actor_id = COALESCE(deletion_actor_id, $5), \
                 deletion_state = COALESCE(deletion_state, 'stopping'), \
                 state = CASE WHEN state IN ('pending', 'draining') THEN 'failed' \
                              ELSE state END, \
                 settled_at = CASE WHEN state IN ('pending', 'draining') THEN $3 \
                                   ELSE settled_at END, \
                 failure_detail = CASE WHEN state IN ('pending', 'draining') THEN $6 \
                                       ELSE failure_detail END \
             WHERE org_id = $1 AND id = $2 \
             RETURNING create_job_id, remove_job_id",
            uuid_to_db(org.0),
            uuid_to_db(request),
            timestamp_to_db(stamp.at)?,
            stamp.actor.kind(),
            stamp.actor.id(),
            DELETED_BY_THE_SELLER,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        let status = classify_legs(
            &mut tx,
            org,
            request,
            [legs.create_job_id, legs.remove_job_id],
            JobFence::Delete(stamp),
        )
        .await?;
        set_request_deletion(&mut tx, org, request, status).await?;
        tx.commit().await?;
        Ok(Some(status))
    }

    /// Re-reads one request's deletion against its legs as they now stand.
    /// `None` where nobody deleted it.
    pub async fn finalise_deletion(
        &self,
        org: OrgId,
        request: Uuid,
        stamp: Stamp,
    ) -> Result<Option<DeletionStatus>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(legs) = sqlx::query!(
            "SELECT create_job_id, remove_job_id FROM sync_request \
             WHERE org_id = $1 AND id = $2 AND deletion_requested_at IS NOT NULL \
             FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        let status = classify_legs(
            &mut tx,
            org,
            request,
            [legs.create_job_id, legs.remove_job_id],
            JobFence::Reconcile(stamp),
        )
        .await?;
        set_request_deletion(&mut tx, org, request, status).await?;
        tx.commit().await?;
        Ok(Some(status))
    }

    /// Sweeps this tenant's open request deletions, and answers how many
    /// reached the tombstone. The sibling of `JobRepo::finalise_deletions`,
    /// and run beside it: a request is retired by its legs going quiet, which
    /// is an event on the jobs rather than on the request.
    pub async fn finalise_deletions(&self, org: OrgId, stamp: Stamp) -> Result<u64, StorageError> {
        let open = {
            let mut tx = self.pool.begin().await?;
            pin_org(&mut tx, org).await?;
            let rows = sqlx::query_scalar!(
                "SELECT id FROM sync_request \
                 WHERE org_id = $1 AND deletion_state IN ('stopping', 'needs_review') \
                 ORDER BY requested_at, id",
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

    /// Whether this request carries a deletion, tombstone included.
    ///
    /// The replay check for `POST /{v}/sync` and `POST /{v}/migrations`: the
    /// request's identity is its idempotency key, so a resubmitted key whose
    /// request was deleted has to answer a conflict rather than be handed
    /// back a request the seller cannot see.
    pub async fn deletion_status(
        &self,
        org: OrgId,
        request: Uuid,
    ) -> Result<Option<DeletionStatus>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let raw = sqlx::query_scalar!(
            "SELECT deletion_state FROM sync_request WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(request),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        DeletionStatus::parse(raw.flatten().as_deref())
    }
}

/// The sentence a deleted request records where it had not settled yet.
const DELETED_BY_THE_SELLER: &str = "you deleted this request, so nothing further was queued";

/// Fences every leg this request owns and answers the request's own status.
///
/// The legs are deleted rather than merely read, and in this transaction: a
/// request whose Delete had fenced the row but not its jobs is exactly the
/// window in which a claim starts the removal leg.
///
/// Discovered through `job.sync_request_id` rather than trusted from the
/// request's own `create_job_id`/`remove_job_id`, because those two are
/// written *after* both legs are minted and committed. A Delete arriving in
/// between reads two nulls, and a request that reported `deleted` on the
/// strength of them leaves a minted, unfenced leg on the queue — a Copy's
/// create, or worse a Move's source removal. The job names its request in the
/// same statement that inserts it, so this link exists from the leg's first
/// instant. The recorded columns are still folded in: they are the only
/// record of a leg whose job row has since gone, and one the ledger cannot
/// produce is reported rather than passed over.
///
/// A resource admitted by a running migration page counts as executing even
/// where no leg does. `apply_one` commits the product, the files and the
/// mapping separately, so a page holding an admitted resource — an unfinished
/// breadcrumb carrying `admitted_at` — is work in progress in exactly the
/// sense `Stopping` names, and reporting the request gone while it finishes is
/// how resources appear after a deletion has returned. An unfinished resource
/// nobody is applying is not executing: the drain pre-creates one per listed
/// resource, and a stopped request's drain will never run.
///
/// `Stopping` beats `NeedsReview` beats `Deleted`, in that order, because a
/// request is only as finished as its least finished part — and a request
/// with nothing outstanding at all is finished, which is the pending-request
/// case and the commonest Delete there is.
async fn classify_legs(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    request: Uuid,
    recorded: [Option<uuid::Uuid>; 2],
    fence: JobFence,
) -> Result<DeletionStatus, StorageError> {
    let mut legs = sqlx::query_scalar!(
        "SELECT id FROM job WHERE org_id = $1 AND sync_request_id = $2 \
         ORDER BY created_at, id",
        uuid_to_db(org.0),
        uuid_to_db(request),
    )
    .fetch_all(&mut **tx)
    .await?;
    for leg in recorded.into_iter().flatten() {
        if !legs.contains(&leg) {
            legs.push(leg);
        }
    }
    // Every leg's row taken before the first of them is fenced, because
    // fencing one appends events and holds the organisation's event counter
    // to commit. A settle takes its own job row before that counter, so a
    // transaction that held the counter from leg one and then reached for
    // leg two would deadlock against a settle holding it — and the victim
    // Postgres picks could be the settle of a write that already reached the
    // marketplace. Ordered, so two of these serialise rather than cross.
    legs.sort_unstable();
    sqlx::query_scalar!(
        "SELECT id FROM job WHERE org_id = $1 AND id = ANY($2) \
         ORDER BY id FOR UPDATE",
        uuid_to_db(org.0),
        &legs[..],
    )
    .fetch_all(&mut **tx)
    .await?;
    crate::jobs::lock_job_items(tx, org, &legs).await?;
    let mut worst = DeletionStatus::Deleted;
    for leg in legs {
        let job = tam_types::JobId(uuid_from_db(leg));
        let status = fence.apply(tx, org, job).await?;
        // A leg the request names and the ledger does not have is not a
        // reason to call the request finished: it is a row that cannot be
        // reconciled, and saying so is the only honest answer.
        let status = status.unwrap_or(DeletionStatus::NeedsReview);
        worst = worse_of(worst, status);
    }
    let applying = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM sync_request_resource \
                         WHERE org_id = $1 AND request_id = $2 \
                           AND state = 'pending' AND admitted_at IS NOT NULL) \
                AS \"applying!\"",
        uuid_to_db(org.0),
        uuid_to_db(request),
    )
    .fetch_one(&mut **tx)
    .await?;
    if applying {
        worst = worse_of(worst, DeletionStatus::Stopping);
    }
    Ok(worst)
}

/// The less finished of two states, which is the one a request reports.
const fn worse_of(left: DeletionStatus, right: DeletionStatus) -> DeletionStatus {
    match (left, right) {
        (DeletionStatus::Stopping, _) | (_, DeletionStatus::Stopping) => DeletionStatus::Stopping,
        (DeletionStatus::NeedsReview, _) | (_, DeletionStatus::NeedsReview) => {
            DeletionStatus::NeedsReview
        }
        (DeletionStatus::Deleted, DeletionStatus::Deleted) => DeletionStatus::Deleted,
    }
}

async fn set_request_deletion(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    request: Uuid,
    status: DeletionStatus,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE sync_request SET deletion_state = $3 WHERE org_id = $1 AND id = $2",
        uuid_to_db(org.0),
        uuid_to_db(request),
        status.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The two request keys one request needs.
///
/// A migrate is two jobs and `job_request_idempotent` admits one job per key
/// per organisation, so passing the request's own id twice makes the second
/// creation a replay of the first: `create_with_request_key` returns the
/// first job with `replay: true` and drops the second job's items on the
/// floor. The removal would then never exist, the drain would report success,
/// and the migrate would silently degrade to a plain sync. Deriving one key
/// per leg is what stops that.
#[must_use]
pub fn job_request_key(request: Uuid, leg: &str) -> Uuid {
    let derived = uuid::Uuid::new_v5(&uuid::Uuid::from_bytes(request.0), leg.as_bytes());
    Uuid(*derived.as_bytes())
}

pub const CREATE_LEG: &str = "create";
pub const REMOVE_LEG: &str = "remove";
/// The leg an import's own events hang from.
///
/// A `job_event` row requires a job, and a device import has none until its
/// completing page mints the create leg — so progress would be invisible for
/// however long the seller's shop takes to walk. This leg names an itemless
/// job created with the first page purely to anchor those events, which is the
/// same device `record_drain_report` already uses. It can never become a
/// publish: a `queued` item is what the lease scan claims, and it has none.
pub const IMPORT_LEG: &str = "import";

/// How many requests one normalization transaction reads and locks.
///
/// A deployment-time pass over a tenant's whole history, so the number only
/// has to keep any one transaction short: a batch is the unit of locked
/// anchors and of restartable progress. Small enough that a tenant with tens
/// of thousands of migrations never holds a long-running lock, large enough
/// that the common tenant is one round trip.
const ANCHOR_BATCH: i64 = 256;

fn disposition_from_db(raw: &str) -> Result<Disposition, StorageError> {
    match raw {
        "sync" => Ok(Disposition::Sync),
        "migrate" => Ok(Disposition::Migrate),
        other => Err(StorageError::Inconsistent {
            reason: format!("unknown sync disposition {other}"),
        }),
    }
}

fn intent_from_db(raw: &str) -> Result<SyncIntent, StorageError> {
    match raw {
        "draft" => Ok(SyncIntent::Draft),
        "live" => Ok(SyncIntent::Live),
        other => Err(StorageError::Inconsistent {
            reason: format!("unknown sync intent {other}"),
        }),
    }
}
