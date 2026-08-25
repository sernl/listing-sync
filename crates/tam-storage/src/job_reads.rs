//! The ledger's read side, shaped for the API: keyset listing, the roll-up
//! snapshot that never gates on the first bad item, item pages, per-item
//! event timelines, and the org-scalar event queries the progress stream
//! projects. Everything here is a read; the write path stays in `jobs.rs`.

use sqlx::PgPool;
use tam_domain::{ItemOutcome, JobItemId};
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

    fn from_db(raw: &str) -> Result<Self, StorageError> {
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

fn item_outcome_from_db(raw: &str) -> Result<ItemOutcome, StorageError> {
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
            let n = u64::try_from(group.count).unwrap_or(0);
            counts.total += n;
            match ItemStateKind::from_db(&group.state)? {
                ItemStateKind::Queued => counts.queued += n,
                ItemStateKind::Leased => counts.leased += n,
                ItemStateKind::Running => counts.running += n,
                ItemStateKind::Blocked => counts.blocked += n,
                ItemStateKind::ParkedLive => counts.parked_live += n,
                ItemStateKind::ParkedCold => counts.parked_cold += n,
                ItemStateKind::Verifying => counts.verifying += n,
                ItemStateKind::Settled => counts.settled += n,
            }
            if let Some(outcome) = group.outcome.as_deref() {
                match item_outcome_from_db(outcome)? {
                    ItemOutcome::Succeeded => counts.succeeded += n,
                    ItemOutcome::Degraded => counts.degraded += n,
                    ItemOutcome::Failed => counts.failed += n,
                    ItemOutcome::Ambiguous => counts.ambiguous += n,
                    ItemOutcome::Skipped => counts.skipped += n,
                    ItemOutcome::Blocked => counts.outcome_blocked += n,
                }
            }
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
        cursor: Option<LedgerCursor>,
        limit: i64,
    ) -> Result<Vec<ItemRow>, StorageError> {
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
             ORDER BY created_at, id LIMIT $5",
            uuid_to_db(org.0),
            uuid_to_db(job.0),
            cursor_at,
            cursor_id,
            limit,
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
            "SELECT m.id AS mapping_id, m.product_id, f.hash \
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
