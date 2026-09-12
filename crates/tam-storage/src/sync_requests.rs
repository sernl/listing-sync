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
use tam_types::{InventoryId, MappingId, OrgId, ProductId, Timestamp, TransportClass, Uuid};

use crate::jobs::{CreatedJob, NewJob, NewJobItem};

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
        tx.commit().await?;
        Ok(true)
    }

    /// The organisation's sync requests, newest first.
    ///
    /// The console's only way to reach a request it created and navigated away
    /// from. A device-branch migrate mints no job until its completing page, so
    /// until then it appears in no job list and, without this, nowhere at all —
    /// which left the console telling a seller to go back to a page it could
    /// not link to.
    ///
    /// The two counts are computed here rather than by loading every
    /// breadcrumb, because a list of fifty requests over a five-hundred-resource
    /// shop is twenty-five thousand rows to answer a question about two numbers.
    pub async fn list(
        &self,
        org: OrgId,
        limit: i64,
    ) -> Result<Vec<SyncRequestSummary>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            r#"SELECT r.id, r.source, r.target, r.disposition, r.intent, r.state, r.requested_at,
                      (SELECT count(*) FROM sync_request_resource s
                        WHERE s.org_id = r.org_id AND s.request_id = r.id) AS "total!",
                      (SELECT count(*) FROM sync_request_resource s
                        WHERE s.org_id = r.org_id AND s.request_id = r.id
                          AND s.state = 'failed') AS "failed!"
               FROM sync_request r
               WHERE r.org_id = $1
               ORDER BY r.requested_at DESC, r.id DESC
               LIMIT $2"#,
            uuid_to_db(org.0),
            limit,
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
            "SELECT source, target, disposition, intent, state, create_job_id, remove_job_id, \
                    failure_detail, requested_at \
             FROM sync_request WHERE org_id = $1 AND id = $2",
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

    /// The drain's own work list. Ordered oldest first so a backlog drains in
    /// the order sellers asked, and scoped to one tenant because this repo is
    /// only ever reached through a pinned connection.
    pub async fn pending(&self, org: OrgId, limit: i64) -> Result<Vec<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id FROM sync_request WHERE org_id = $1 AND state IN ('pending', 'draining') \
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
                 source_state = $9, failure_detail = NULL \
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

    /// Appends one described resource to a device import, at the next ordinal.
    ///
    /// `Ok(false)` means the request already carries a breadcrumb for this
    /// locator and nothing was written. Identity is the marketplace resource
    /// id within the request rather than the ordinal, because a device that
    /// re-posts a page it already sent must not mint a second product: the
    /// caller checks this before it canonicalises, and this is the write-side
    /// half of the same rule.
    ///
    /// The ordinal is computed and inserted in one statement so two pages
    /// cannot read the same maximum; the primary key refuses the remainder of
    /// that race rather than the two silently sharing an ordinal.
    pub async fn append_observed(
        &self,
        org: OrgId,
        request: Uuid,
        observed: &Observed<'_>,
    ) -> Result<bool, StorageError> {
        let columns = RemoteIdColumns::encode(observed.source)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO sync_request_resource \
             (org_id, request_id, ordinal, locator, state, product_id, mapping_id, \
              source_kind, source_url, source_numeric_id, source_state, \
              terms_seen, terms_mapped, terms_unmapped, terms_uncovered) \
             SELECT $1, $2, \
                    COALESCE((SELECT MAX(ordinal) FROM sync_request_resource \
                              WHERE org_id = $1 AND request_id = $2), -1) + 1, \
                    $3, 'canonicalised', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13 \
             WHERE NOT EXISTS (SELECT 1 FROM sync_request_resource \
                               WHERE org_id = $1 AND request_id = $2 AND locator = $3)",
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
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(written == 1)
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
                }
            }
        };
        sqlx::query!(
            "UPDATE sync_request \
             SET state = 'enqueued', create_job_id = $3, settled_at = $4 \
             WHERE org_id = $1 AND id = $2 AND state <> 'enqueued'",
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

    pub async fn record_resource_failure(
        &self,
        org: OrgId,
        request: Uuid,
        ordinal: i32,
        detail: &str,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE sync_request_resource SET state = 'failed', failure_detail = $4 \
             WHERE org_id = $1 AND request_id = $2 AND ordinal = $3",
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
             WHERE org_id = $1 AND id = $2",
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
