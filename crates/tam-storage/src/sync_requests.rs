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
use tam_types::{InventoryId, MappingId, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    inventory_from_db, inventory_to_db, listing_state_from_db, listing_state_to_db,
    timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db, RemoteIdColumns,
};
use crate::mapping::remote_id_from_db;
use crate::{pin_org, StorageError};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncResourceRecord {
    pub ordinal: i32,
    pub locator: String,
    pub state: String,
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
    pub async fn create(&self, org: OrgId, new: &NewSyncRequest) -> Result<(), StorageError> {
        if new.locators.is_empty() {
            return Err(StorageError::Inconsistent {
                reason: "a sync request names at least one resource".to_owned(),
            });
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO sync_request \
             (org_id, id, source, target, disposition, intent, state, requested_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7)",
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
        Ok(())
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
                    source_kind, source_url, source_numeric_id, source_state, failure_detail \
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
