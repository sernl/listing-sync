//! The ledger's read side, shaped for the API: keyset listing, the roll-up
//! snapshot that never gates on the first bad item, item pages, per-item
//! event timelines, and the org-scalar event queries the progress stream
//! projects. Everything here is a read; the write path stays in `jobs.rs`.

use sqlx::PgPool;
use tam_domain::{ItemOperation, ItemOutcome, JobItemId};
use tam_marketplace::RemoteListingId;
use tam_types::{FailureCode, InventoryId, JobId, MappingId, OrgId, Timestamp, Uuid};

use crate::codec::{
    failure_code_from_db, inventory_from_db, timestamp_from_db, timestamp_to_db, uuid_from_db,
    uuid_to_db,
};
use crate::{pin_org, StorageError};

/// The flat shape of a stored item state, for reading; the rich variants
/// with their leases and deadlines live in the domain and the write path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStateKind {
    Queued,
    Leased,
    Running,
    Blocked,
    ParkedLive,
    ParkedCold,
    Verifying,
    Settled,
}

impl ItemStateKind {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 8] = [
        Self::Queued,
        Self::Leased,
        Self::Running,
        Self::Blocked,
        Self::ParkedLive,
        Self::ParkedCold,
        Self::Verifying,
        Self::Settled,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::ParkedLive => "parked_live",
            Self::ParkedCold => "parked_cold",
            Self::Verifying => "verifying",
            Self::Settled => "settled",
        }
    }

    pub(crate) fn from_db(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "queued" => Ok(Self::Queued),
            "leased" => Ok(Self::Leased),
            "running" => Ok(Self::Running),
            "blocked" => Ok(Self::Blocked),
            "parked_live" => Ok(Self::ParkedLive),
            "parked_cold" => Ok(Self::ParkedCold),
            "verifying" => Ok(Self::Verifying),
            "settled" => Ok(Self::Settled),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown item state {other:?}"),
            }),
        }
    }
}

pub(crate) fn item_outcome_from_db(raw: &str) -> Result<ItemOutcome, StorageError> {
    match raw {
        "succeeded" => Ok(ItemOutcome::Succeeded),
        "degraded" => Ok(ItemOutcome::Degraded),
        "failed" => Ok(ItemOutcome::Failed),
        "ambiguous" => Ok(ItemOutcome::Ambiguous),
        "skipped" => Ok(ItemOutcome::Skipped),
        "blocked" => Ok(ItemOutcome::Blocked),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown item outcome {other:?}"),
        }),
    }
}

/// A keyset cursor over `(created_at, id)`; the API carries it opaquely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerCursor {
    pub created_at: Timestamp,
    pub id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobListRow {
    pub job: JobId,
    pub inventory: InventoryId,
    pub created_at: Timestamp,
}

/// The roll-up: raw counts, never a scalar verdict. Job status is computed
/// from these by the consumer and never gates on the first bad item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemCounts {
    pub total: u64,
    pub queued: u64,
    pub leased: u64,
    pub running: u64,
    pub blocked: u64,
    pub parked_live: u64,
    pub parked_cold: u64,
    pub verifying: u64,
    pub settled: u64,
    pub succeeded: u64,
    pub degraded: u64,
    pub failed: u64,
    pub ambiguous: u64,
    pub skipped: u64,
    pub outcome_blocked: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSnapshot {
    pub job: JobId,
    pub inventory: InventoryId,
    pub created_at: Timestamp,
    pub counts: ItemCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub item: JobItemId,
    pub mapping: MappingId,
    pub state: ItemStateKind,
    pub outcome: Option<ItemOutcome>,
    pub failure_code: Option<FailureCode>,
    pub failure_detail: Option<String>,
    pub blocked_on: Option<String>,
    pub attempt_count: i32,
    pub created_at: Timestamp,
    pub settled_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub org_seq: i64,
    pub job: JobId,
    pub item: Option<JobItemId>,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: Timestamp,
}

/// Folds one `GROUP BY state, outcome` row into the roll-up.
///
/// Shared by the per-job snapshot and the cross-tenant aggregate the operator
/// surface reads, so the two cannot disagree about which stored state feeds
/// which counter.
pub(crate) fn add_item_group(
    counts: &mut ItemCounts,
    state: &str,
    outcome: Option<&str>,
    n: u64,
) -> Result<(), StorageError> {
    counts.total += n;
    match ItemStateKind::from_db(state)? {
        ItemStateKind::Queued => counts.queued += n,
        ItemStateKind::Leased => counts.leased += n,
        ItemStateKind::Running => counts.running += n,
        ItemStateKind::Blocked => counts.blocked += n,
        ItemStateKind::ParkedLive => counts.parked_live += n,
        ItemStateKind::ParkedCold => counts.parked_cold += n,
        ItemStateKind::Verifying => counts.verifying += n,
        ItemStateKind::Settled => counts.settled += n,
    }
    if let Some(outcome) = outcome {
        match item_outcome_from_db(outcome)? {
            ItemOutcome::Succeeded => counts.succeeded += n,
            ItemOutcome::Degraded => counts.degraded += n,
            ItemOutcome::Failed => counts.failed += n,
            ItemOutcome::Ambiguous => counts.ambiguous += n,
            ItemOutcome::Skipped => counts.skipped += n,
            ItemOutcome::Blocked => counts.outcome_blocked += n,
        }
    }
    Ok(())
}

pub struct JobReadRepo {
    pool: PgPool,
}

impl JobReadRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Newest first, keyset on `(created_at, id)` strictly below the cursor.
    pub async fn list_jobs(
        &self,
        org: OrgId,
        cursor: Option<LedgerCursor>,
        limit: i64,
    ) -> Result<Vec<JobListRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let (cursor_at, cursor_id) = match cursor {
            Some(cursor) => (
                Some(timestamp_to_db(cursor.created_at)?),
                Some(uuid_to_db(cursor.id)),
            ),
            None => (None, None),
        };
        let rows = sqlx::query!(
            "SELECT id, inventory, created_at FROM job \
             WHERE org_id = $1 \
               AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3)) \
             ORDER BY created_at DESC, id DESC LIMIT $4",
            uuid_to_db(org.0),
            cursor_at,
            cursor_id,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(JobListRow {
                    job: JobId(uuid_from_db(row.id)),
                    inventory: inventory_from_db(&row.inventory)?,
                    created_at: timestamp_from_db(row.created_at),
                })
            })
            .collect()
    }

    /// The job and its raw counts, or `None` for a job the tenant does not
    /// hold. One grouped query; the roll-up is the caller's arithmetic.
    pub async fn snapshot(
        &self,
        org: OrgId,
        job: JobId,
    ) -> Result<Option<JobSnapshot>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(head) = sqlx::query!(
            "SELECT inventory, created_at FROM job WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(job.0),
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let groups = sqlx::query!(
            "SELECT state, outcome, count(*) AS \"count!\" FROM job_item \
             WHERE org_id = $1 AND job_id = $2 GROUP BY state, outcome",
            uuid_to_db(org.0),
            uuid_to_db(job.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut counts = ItemCounts::default();
        for group in groups {
            add_item_group(
                &mut counts,
                &group.state,
                group.outcome.as_deref(),
                u64::try_from(group.count).unwrap_or(0),
            )?;
        }
        Ok(Some(JobSnapshot {
            job,
            inventory: inventory_from_db(&head.inventory)?,
            created_at: timestamp_from_db(head.created_at),
            counts,
        }))
    }

    /// Oldest first, keyset strictly above the cursor, so a client walks a
    /// stable order while the engine settles items concurrently.
    pub async fn items_page(
        &self,
        org: OrgId,
        job: JobId,
        page: ItemsPageParams,
    ) -> Result<Vec<ItemRow>, StorageError> {
        let ItemsPageParams {
            cursor,
            limit,
            outcome,
        } = page;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let (cursor_at, cursor_id) = match cursor {
            Some(cursor) => (
                Some(timestamp_to_db(cursor.created_at)?),
                Some(uuid_to_db(cursor.id)),
            ),
            None => (None, None),
        };
        let rows = sqlx::query!(
            "SELECT id, mapping_id, state, outcome, failure_code, failure_detail, \
                    blocked_on, attempt_count, created_at, settled_at \
             FROM job_item \
             WHERE org_id = $1 AND job_id = $2 \
               AND ($3::timestamptz IS NULL OR (created_at, id) > ($3, $4)) \
               AND ($6::text IS NULL OR outcome = $6) \
             ORDER BY created_at, id LIMIT $5",
            uuid_to_db(org.0),
            uuid_to_db(job.0),
            cursor_at,
            cursor_id,
            limit,
            outcome,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ItemRow {
                    item: JobItemId(uuid_from_db(row.id)),
                    mapping: MappingId(uuid_from_db(row.mapping_id)),
                    state: ItemStateKind::from_db(&row.state)?,
                    outcome: row
                        .outcome
                        .as_deref()
                        .map(item_outcome_from_db)
                        .transpose()?,
                    failure_code: row
                        .failure_code
                        .as_deref()
                        .map(failure_code_from_db)
                        .transpose()?,
                    failure_detail: row.failure_detail,
                    blocked_on: row.blocked_on,
                    attempt_count: row.attempt_count,
                    created_at: timestamp_from_db(row.created_at),
                    settled_at: row.settled_at.map(timestamp_from_db),
                })
            })
            .collect()
    }

    /// One item's event timeline, oldest first: the downloadable per-item
    /// result and the client's step view.
    pub async fn item_events(
        &self,
        org: OrgId,
        item: JobItemId,
    ) -> Result<Vec<EventRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT org_seq, job_id, job_item_id, kind, payload, created_at \
             FROM job_event WHERE org_id = $1 AND job_item_id = $2 ORDER BY org_seq",
            uuid_to_db(org.0),
            uuid_to_db(item.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| EventRow {
                org_seq: row.org_seq,
                job: JobId(uuid_from_db(row.job_id)),
                item: row.job_item_id.map(|id| JobItemId(uuid_from_db(id))),
                kind: row.kind,
                payload: row.payload,
                created_at: timestamp_from_db(row.created_at),
            })
            .collect())
    }

    /// Every org event strictly after the cursor, oldest first: the stream's
    /// replay-then-poll source, scalar across every job the tenant watches.
    pub async fn events_after(
        &self,
        org: OrgId,
        after: i64,
        limit: i64,
    ) -> Result<Vec<EventRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT org_seq, job_id, job_item_id, kind, payload, created_at \
             FROM job_event WHERE org_id = $1 AND org_seq > $2 \
             ORDER BY org_seq LIMIT $3",
            uuid_to_db(org.0),
            after,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| EventRow {
                org_seq: row.org_seq,
                job: JobId(uuid_from_db(row.job_id)),
                item: row.job_item_id.map(|id| JobItemId(uuid_from_db(id))),
                kind: row.kind,
                payload: row.payload,
                created_at: timestamp_from_db(row.created_at),
            })
            .collect())
    }

    /// The pruning watermark the resume decision reads: a client's cursor
    /// below it gets a resync rather than a partial replay. A tenant with no
    /// counter row has watermark zero and nothing to prune.
    pub async fn watermark(&self, org: OrgId) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let watermark = sqlx::query_scalar!(
            "SELECT prune_watermark FROM org_event_counter WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(watermark.unwrap_or(0))
    }

    /// The newest allocated cursor, for the resync snapshot event.
    pub async fn latest_seq(&self, org: OrgId) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let latest = sqlx::query_scalar!(
            "SELECT max(org_seq) FROM job_event WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(latest.unwrap_or(0))
    }
}

/// One page of a job's items, plus the outcome the caller wants to see.
///
/// The filter is a `WHERE` clause and not a client-side one: a seller paging
/// five hundred rows to find the four that failed is what the ledger's own
/// report is for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemsPageParams {
    pub cursor: Option<LedgerCursor>,
    pub limit: i64,
    pub outcome: Option<String>,
}

/// What a sync-starting request needs per mapping: the product it projects
/// and the ordered payload hashes whose digest content-addresses the item's
/// idempotency key. Only mappings of the requested inventory return; the
/// caller compares what came back against what was asked and refuses the
/// difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingSeed {
    pub mapping: MappingId,
    pub product: tam_types::ProductId,
    pub payload_hashes: Vec<tam_types::ContentHash>,
    /// How many times this mapping's listing has been severed, which the
    /// create's intent digest mixes so a migrate-back is a fresh key while an
    /// ordinary re-sync of unchanged content stays the no-op it was built to
    /// be.
    pub sever_generation: i32,
    /// What the mapping's binding and lifecycle read, as stored. The API
    /// lowers a seller's stated intent against them and reads no marketplace
    /// to do it; without these it could not evaluate a single row of the
    /// lowering table and would default every mapping to the create row,
    /// minting a second listing for one that is already bound.
    ///
    /// The stored spellings rather than the decoded values, because lowering
    /// is a table over exactly these two columns and hydrating a whole
    /// mapping per seed would be a join per row for two strings.
    pub binding_state: String,
    pub lifecycle_state: String,
    /// The listing a bound mapping names, which a revise must address. `None`
    /// for every other binding, where there is nothing to address yet.
    pub subject: Option<RemoteListingId>,
}

impl JobReadRepo {
    pub async fn mapping_seeds(
        &self,
        org: OrgId,
        inventory: InventoryId,
        mappings: &[MappingId],
    ) -> Result<Vec<MappingSeed>, StorageError> {
        let ids: Vec<uuid::Uuid> = mappings.iter().map(|m| uuid_to_db(m.0)).collect();
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT m.id AS mapping_id, m.product_id, m.sever_generation, \
                    m.binding_state, m.lifecycle_state, \
                    m.remote_id_kind, m.remote_url, m.remote_numeric_id, \
                    COALESCE(f.hash, f.observed_hash) AS \"hash!\" \
             FROM mapping m \
             JOIN product_file f \
               ON f.org_id = m.org_id AND f.product_id = m.product_id \
             WHERE m.org_id = $1 AND m.inventory = $2 AND m.id = ANY($3) \
               AND f.role = 'payload' AND f.deleted_at IS NULL \
             ORDER BY m.id, f.position",
            uuid_to_db(org.0),
            crate::codec::inventory_to_db(inventory),
            &ids,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        // A payload file is blob-backed or marketplace-sourced, and the
        // idempotency key covers the bytes that will be uploaded either way,
        // so the coalesce takes whichever digest describes them. The two are
        // deliberately distinct everywhere else — one we verified, one a
        // device asserted — and this is the one place the distinction does not
        // change the answer, because the key asks what is being sent rather
        // than who vouched for it. `product_file_blob_or_source` is what makes
        // the coalesce total.
        //
        // The consequence worth knowing: the key is no longer purely
        // server-derived, because on a sourced file the digest is one a device
        // asserted. That is Q-a's decision doing its work rather than a new
        // exposure: the value is read from our own row rather than from a
        // request. What it is not is responsive to a later disagreement — the
        // file's digest records the first observation and is never updated, so
        // a second device reporting different bytes changes nothing here. The
        // key follows the recorded digest; the disagreement is recorded beside
        // it and surfaced, not applied.
        let mut seeds: Vec<MappingSeed> = Vec::new();
        for row in rows {
            let mapping = MappingId(uuid_from_db(row.mapping_id));
            let hash = crate::codec::hash_from_db(&row.hash)?;
            match seeds.last_mut() {
                Some(seed) if seed.mapping == mapping => seed.payload_hashes.push(hash),
                _ => seeds.push(MappingSeed {
                    mapping,
                    product: tam_types::ProductId(uuid_from_db(row.product_id)),
                    payload_hashes: vec![hash],
                    sever_generation: row.sever_generation,
                    binding_state: row.binding_state.clone(),
                    lifecycle_state: row.lifecycle_state.clone(),
                    subject: match row.remote_id_kind.as_deref() {
                        Some(kind) => Some(crate::mapping::remote_id_from_db(
                            kind,
                            row.remote_url.clone(),
                            row.remote_numeric_id,
                        )?),
                        None => None,
                    },
                }),
            }
        }
        Ok(seeds)
    }
}

/// The digest that content-addresses a sync item: BLAKE3 over the ordered
/// payload hashes, so the same files in the same order name the same intent
/// and a changed file changes the key.
#[must_use]
pub fn payload_digest(hashes: &[tam_types::ContentHash]) -> tam_types::ContentHash {
    let mut concatenated = Vec::with_capacity(hashes.len() * 32);
    for hash in hashes {
        concatenated.extend_from_slice(&hash.0);
    }
    tam_pipeline::hash::content_hash(&concatenated)
}

/// Domain separator for a non-create intent, so no encoding here can ever be
/// read as a concatenation of payload hashes.
const NON_CREATE_INTENT_DOMAIN: &[u8] = b"tam.item.intent.v1\x00";

/// The separator for a create whose mapping has been severed at least once.
/// Its own domain so a severed create can never collide with a non-create
/// intent that happened to hash the same bytes.
const SEVERED_CREATE_DOMAIN: &[u8] = b"tam.item.intent.create.severed.v1\x00";

/// A create's digest: the payload, plus the number of times this mapping has
/// been severed.
///
/// The payload alone is what makes re-uploading unchanged content a no-op,
/// and that property is deliberate. But `mapping_one_per_inventory` is
/// unpredicated, so a migrate reversed lands on the severed row rather than
/// minting a new one, and a re-create of the same product with the same files
/// then reproduces the first create's key exactly. `job_item_idempotent` is
/// table-wide with no job scoping and `job_item` rows are never deleted, so
/// the second round trip is refused forever with a message about unchanged
/// content that is false: the listing no longer exists on that platform.
///
/// Mixing the job would fix it and delete the no-op property for every
/// ordinary sync. The sever counter separates keys across a sever and nowhere
/// else, which is exactly the boundary that matters. Generation zero hashes
/// to the payload digest unchanged, so no in-flight create is re-keyed.
fn create_digest(
    hashes: &[tam_types::ContentHash],
    sever_generation: i32,
) -> tam_types::ContentHash {
    let payload = payload_digest(hashes);
    if sever_generation == 0 {
        return payload;
    }
    let mut encoded = Vec::with_capacity(48);
    encoded.extend_from_slice(SEVERED_CREATE_DOMAIN);
    encoded.extend_from_slice(&payload.0);
    encoded.extend_from_slice(&sever_generation.to_be_bytes());
    tam_pipeline::hash::content_hash(&encoded)
}

/// A publish's digest: the job, and nothing else it could name. The create
/// that binds its subject sits in the same job, and the pair is separated by
/// this function's own domain tag rather than by a subject neither has.
fn publish_digest(job: JobId) -> tam_types::ContentHash {
    let mut encoded = Vec::with_capacity(48);
    encoded.extend_from_slice(NON_CREATE_INTENT_DOMAIN);
    encoded.extend_from_slice(b"publish");
    encoded.push(0);
    encoded.extend_from_slice(&job.0 .0);
    tam_pipeline::hash::content_hash(&encoded)
}

/// The digest that identifies one item's intent.
///
/// A create is content-addressed: the same files in the same order name the
/// same intent, which is what makes re-uploading unchanged content a no-op.
/// Nothing else is. A revise carries no payload change at all -- a price fix
/// and a title fix hash identically -- and a removal followed by a re-create
/// reproduces the first create's digest, so under a content-addressed key the
/// second of any such pair is refused by `job_item_idempotent` forever, since
/// `job_item` rows are never deleted and the uniqueness is org-scoped.
/// Those operations are therefore identified by the job that asked for them,
/// which keeps duplicate protection within a job and drops it across jobs,
/// where it was never wanted.
///
/// `Create` delegates to [`payload_digest`] rather than re-deriving it: two
/// implementations of one digest would drift silently and re-key every
/// in-flight create.
#[must_use]
pub fn intent_digest(
    operation: &ItemOperation,
    job: JobId,
    hashes: &[tam_types::ContentHash],
    sever_generation: i32,
) -> tam_types::ContentHash {
    let (tag, subject) = match operation {
        ItemOperation::Create => return create_digest(hashes, sever_generation),
        // A publish names no subject at enqueue time, so it is keyed on the
        // job alone, like the other non-create arms and for the same reason:
        // duplicate protection within a job, and none across jobs, where it
        // was never wanted.
        ItemOperation::Publish { .. } => return publish_digest(job),
        ItemOperation::Revise { subject, .. } => (&b"revise"[..], subject),
        ItemOperation::Remove { subject, .. } => (&b"remove"[..], subject),
    };
    let mut encoded = Vec::with_capacity(64);
    encoded.extend_from_slice(NON_CREATE_INTENT_DOMAIN);
    encoded.extend_from_slice(tag);
    encoded.push(0);
    encoded.extend_from_slice(&job.0 .0);
    match subject {
        RemoteListingId::Tes { url } => {
            encoded.extend_from_slice(b"tes\x00");
            encoded.extend_from_slice(url.as_bytes());
        }
        RemoteListingId::Tpt { product_id } => {
            encoded.extend_from_slice(b"tpt\x00");
            encoded.extend_from_slice(&product_id.to_be_bytes());
        }
        RemoteListingId::Etsy { listing_id } => {
            encoded.extend_from_slice(b"etsy\x00");
            encoded.extend_from_slice(&listing_id.to_be_bytes());
        }
    }
    tam_pipeline::hash::content_hash(&encoded)
}

#[cfg(test)]
mod tests {
    use super::{intent_digest, payload_digest};
    use tam_domain::ItemOperation;
    use tam_marketplace::idempotency::derive_idempotency_key;
    use tam_marketplace::{IdempotencyKey, LifecycleTransition, ListingState, RemoteListingId};
    use tam_types::{ContentHash, InventoryId, JobId, OrgId, ProductId, Uuid};

    const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
    const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
    const JOB: JobId = JobId(Uuid([0x0B; 16]));
    const OTHER_JOB: JobId = JobId(Uuid([0x0C; 16]));
    const HASHES: [ContentHash; 2] = [ContentHash([0x51; 32]), ContentHash([0x52; 32])];

    fn key(operation: &ItemOperation, job: JobId) -> IdempotencyKey {
        key_at(operation, job, 0)
    }

    fn key_at(operation: &ItemOperation, job: JobId, sever_generation: i32) -> IdempotencyKey {
        derive_idempotency_key(
            ORG,
            InventoryId::TesGb,
            PRODUCT,
            1,
            intent_digest(operation, job, &HASHES, sever_generation),
        )
    }

    fn listing() -> RemoteListingId {
        RemoteListingId::Tes {
            url: "https://www.tes.com/teaching-resource/fractions-9001".to_owned(),
        }
    }

    /// The golden vector. The four tests beside `derive_idempotency_key` pin
    /// determinism and inequality and no bytes at all, so every one of them
    /// would pass through a change to the canonical encoding that re-keys
    /// every create in flight. This one would not.
    #[test]
    fn a_create_key_is_the_bytes_it_has_always_been() {
        assert_eq!(
            key(&ItemOperation::Create, JOB).0 .0,
            [
                0xD4, 0xE0, 0x6F, 0xE6, 0xAE, 0x48, 0x5F, 0xD0, 0xB7, 0xA2, 0xE2, 0x8C, 0xD2, 0x3C,
                0xFE, 0x4A,
            ],
            "changing a create's key strands every in-flight create against job_item_idempotent"
        );
    }

    #[test]
    fn a_create_delegates_rather_than_re_deriving() {
        assert_eq!(
            intent_digest(&ItemOperation::Create, JOB, &HASHES, 0),
            payload_digest(&HASHES),
            "two implementations of one digest would drift silently"
        );
        assert_eq!(
            key(&ItemOperation::Create, JOB),
            key(&ItemOperation::Create, OTHER_JOB),
            "a create is content-addressed, so the job it was asked for cannot enter its key"
        );
    }

    /// Migrate-back. `mapping_one_per_inventory` is unpredicated, so a
    /// re-create of the same product with the same files lands on the severed
    /// row and would otherwise reproduce the first create's key exactly --
    /// which `job_item_idempotent` refuses forever, with a message about
    /// unchanged content that is false for a listing that no longer exists.
    #[test]
    fn a_create_after_a_sever_is_a_fresh_key_and_an_unsevered_one_is_not() {
        assert_eq!(
            key_at(&ItemOperation::Create, JOB, 0),
            key(&ItemOperation::Create, JOB),
            "no in-flight create is re-keyed: generation zero is the digest it always was"
        );
        assert_ne!(
            key_at(&ItemOperation::Create, JOB, 1),
            key_at(&ItemOperation::Create, JOB, 0),
            "a migrate-back is a new intent, not a duplicate of the create that was removed"
        );
        assert_eq!(
            key_at(&ItemOperation::Create, JOB, 1),
            key_at(&ItemOperation::Create, OTHER_JOB, 1),
            "and it stays content-addressed: the job still cannot enter a create's key"
        );
    }

    #[test]
    fn every_non_create_intent_is_distinct_and_carries_its_job() {
        let revise = ItemOperation::Revise {
            subject: listing(),
            transition: LifecycleTransition {
                from: ListingState::Draft,
                to: ListingState::Live,
            },
        };
        let remove = ItemOperation::Remove {
            subject: listing(),
            state: ListingState::Live,
        };
        assert_ne!(
            key(&revise, JOB),
            key(&ItemOperation::Create, JOB),
            "a re-create after a removal must not reproduce the first create's key"
        );
        assert_ne!(
            key(&revise, JOB),
            key(&remove, JOB),
            "a revise and a removal of one listing are two different asks"
        );
        assert_ne!(
            key(&revise, JOB),
            key(&revise, OTHER_JOB),
            "a second job may legitimately revise the same listing again"
        );
        assert_eq!(
            key(&remove, JOB),
            key(&remove, JOB),
            "a requeued removal must recompute the same key or lose its identity"
        );
    }
}
