//! The job ledger: enqueue, the cross-tenant lease scan, epoch-fenced writes,
//! the event stream with its per-organisation sequence, halts, the outbox and
//! the rate budget.
//!
//! Every method here runs correctly on either role, but the lease scan and
//! the stealer are inherently cross-tenant and see nothing under `tam_app`'s
//! forced row-level security; the engine constructs these repositories over a
//! `tam_engine` pool (BYPASSRLS, table privileges enumerated in migration
//! 0007), which is the one deliberate crossing. The governing axiom from the
//! design: a stalled queue is recoverable and a duplicate-upload storm is
//! not, so every refusal here biases toward stalling.

use sqlx::{PgPool, Postgres, Transaction};
use tam_domain::{ItemOutcome, JobItemId};
use tam_marketplace::{IdempotencyKey, RemoteListingId};
use tam_types::{
    FailureCode, FailureDetail, InventoryId, JobEventPayload, JobId, MappingId, OrgId, Timestamp,
    Uuid,
};

use crate::codec::{
    failure_code_to_db, inventory_from_db, inventory_to_db, marketplace_to_db, timestamp_to_db,
    uuid_from_db, uuid_to_db, RemoteIdColumns,
};
use crate::mapping::remote_id_from_db;
use crate::StorageError;

pub(crate) const fn item_outcome_to_db(outcome: ItemOutcome) -> &'static str {
    match outcome {
        ItemOutcome::Succeeded => "succeeded",
        ItemOutcome::Degraded => "degraded",
        ItemOutcome::Failed => "failed",
        ItemOutcome::Ambiguous => "ambiguous",
        ItemOutcome::Skipped => "skipped",
        ItemOutcome::Blocked => "blocked",
    }
}

/// The job-level half of an enqueue, grouped so call sites read as one
/// value rather than a parameter list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub job: JobId,
    pub inventory: InventoryId,
    pub at: Timestamp,
}

/// Which organisation, item and epoch a fenced write speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseRef {
    pub org: OrgId,
    pub item: JobItemId,
    pub lease_epoch: i64,
}

/// The scope an event is recorded against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventScope {
    pub org: OrgId,
    pub job: JobId,
    pub item: Option<JobItemId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJobItem {
    pub item: JobItemId,
    pub mapping: MappingId,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedItem {
    pub org: OrgId,
    pub item: JobItemId,
    pub job: JobId,
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub idempotency_key: IdempotencyKey,
    pub lease_epoch: i64,
    pub attempt_count: i32,
}

impl LeasedItem {
    #[must_use]
    pub const fn lease_ref(&self) -> LeaseRef {
        LeaseRef {
            org: self.org,
            item: self.item,
            lease_epoch: self.lease_epoch,
        }
    }
}

pub struct JobRepo {
    pool: PgPool,
}

impl JobRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The job, its items and the `JobQueued` event in one transaction. A
    /// reused idempotency key surfaces as `DuplicateIdempotencyKey` and
    /// nothing lands.
    pub async fn enqueue(
        &self,
        org: OrgId,
        new: &NewJob,
        items: &[NewJobItem],
    ) -> Result<(), StorageError> {
        let NewJob { job, inventory, at } = *new;
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO job (org_id, id, inventory, marketplace, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
            org_db,
            uuid_to_db(job.0),
            inventory_to_db(inventory),
            marketplace_to_db(inventory.marketplace()),
            at_db,
        )
        .execute(&mut *tx)
        .await?;
        for item in items {
            let inserted = sqlx::query!(
                "INSERT INTO job_item \
                 (org_id, id, job_id, mapping_id, idempotency_key, state, created_at) \
                 VALUES ($1, $2, $3, $4, $5, 'queued', $6)",
                org_db,
                uuid_to_db(item.item.0),
                uuid_to_db(job.0),
                uuid_to_db(item.mapping.0),
                uuid_to_db(item.idempotency_key.0),
                at_db,
            )
            .execute(&mut *tx)
            .await;
            map_unique(inserted, "job_item_idempotent", || {
                StorageError::DuplicateIdempotencyKey {
                    key: uuid_to_db(item.idempotency_key.0),
                }
            })?;
        }
        let item_count = u32::try_from(items.len()).map_err(|_| StorageError::Inconsistent {
            reason: format!("{} items exceed the event range", items.len()),
        })?;
        append_event(
            &mut tx,
            &EventScope {
                org,
                job,
                item: None,
            },
            &JobEventPayload::JobQueued { items: item_count },
            at,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One event against a job that already exists, in a transaction of its
    /// own. `enqueue` appends inside the transaction that creates the rows;
    /// a process recording a measurement after its work is finished has no
    /// such transaction to join, and `append_event` cannot pin the tenant
    /// itself.
    pub async fn record_event(
        &self,
        org: OrgId,
        scope: &EventScope,
        payload: &JobEventPayload,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        append_event(&mut tx, scope, payload, at).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The settled-outcome vector for the roll-up; deliberately no scalar
    /// verdict, per the design.
    pub async fn settled_outcomes(
        &self,
        org: OrgId,
        job: JobId,
    ) -> Result<Vec<(ItemOutcome, i64)>, StorageError> {
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            r#"SELECT outcome AS "outcome!", count(*) AS "count!" FROM job_item
             WHERE org_id = $1 AND job_id = $2 AND state = 'settled'
             GROUP BY outcome"#,
            uuid_to_db(org.0),
            uuid_to_db(job.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let outcome = match row.outcome.as_str() {
                    "succeeded" => ItemOutcome::Succeeded,
                    "degraded" => ItemOutcome::Degraded,
                    "failed" => ItemOutcome::Failed,
                    "ambiguous" => ItemOutcome::Ambiguous,
                    "skipped" => ItemOutcome::Skipped,
                    "blocked" => ItemOutcome::Blocked,
                    other => {
                        return Err(StorageError::CorruptRow {
                            reason: format!("unknown item outcome {other:?}"),
                        })
                    }
                };
                Ok((outcome, row.count))
            })
            .collect()
    }
}

/// One inventory's recent terminal outcomes, for the fleet breaker: how
/// many items settled at all and how many settled failed or ambiguous.
/// Cross-tenant by construction; runs on the engine role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryFailureWindow {
    pub inventory: InventoryId,
    pub settled: i64,
    pub failed_or_ambiguous: i64,
}

impl JobRepo {
    pub async fn recent_outcomes(
        &self,
        since: Timestamp,
    ) -> Result<Vec<InventoryFailureWindow>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT j.inventory AS "inventory!",
                 count(*) AS "settled!",
                 count(*) FILTER (WHERE ji.outcome IN ('failed', 'ambiguous'))
                     AS "failed_or_ambiguous!"
             FROM job_item ji
             JOIN job j ON j.org_id = ji.org_id AND j.id = ji.job_id
             WHERE ji.state = 'settled' AND ji.settled_at >= $1
             GROUP BY j.inventory"#,
            timestamp_to_db(since)?,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(InventoryFailureWindow {
                    inventory: inventory_from_db(&row.inventory)?,
                    settled: row.settled,
                    failed_or_ambiguous: row.failed_or_ambiguous,
                })
            })
            .collect()
    }
}

/// Appends one event inside the caller's transaction, allocating `org_seq`
/// by locking the per-organisation counter row — identity values are
/// allocated before commit and can appear out of order, which is exactly the
/// resume-query unsoundness the counter exists to repair.
pub async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    scope: &EventScope,
    payload: &JobEventPayload,
    at: Timestamp,
) -> Result<(), StorageError> {
    let EventScope { org, job, item } = *scope;
    let org_db = uuid_to_db(org.0);
    sqlx::query!(
        "INSERT INTO org_event_counter (org_id) VALUES ($1) ON CONFLICT (org_id) DO NOTHING",
        org_db,
    )
    .execute(&mut **tx)
    .await?;
    let seq = sqlx::query_scalar!(
        r#"UPDATE org_event_counter SET next_seq = next_seq + 1
         WHERE org_id = $1 RETURNING next_seq - 1 AS "seq!""#,
        org_db,
    )
    .fetch_one(&mut **tx)
    .await?;
    let encoded = serde_json::to_value(payload).map_err(|error| StorageError::Inconsistent {
        reason: format!("a job event payload must serialise: {error}"),
    })?;
    let body = encoded
        .get(payload.kind())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    sqlx::query!(
        "INSERT INTO job_event \
         (org_id, org_seq, job_id, job_item_id, kind, payload, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        org_db,
        seq,
        uuid_to_db(job.0),
        item.map(|item| uuid_to_db(item.0)),
        payload.kind(),
        body,
        timestamp_to_db(at)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// How an item settled in the ledger's own vocabulary: the outcome, its
/// failure code, and the adapter's free text beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemVerdict {
    pub outcome: ItemOutcome,
    pub failure_code: Option<FailureCode>,
    pub failure_detail: Option<FailureDetail>,
}

pub struct LeaseRepo {
    pool: PgPool,
}

impl LeaseRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The cross-tenant scan. Halts and the connection gate fail closed in
    /// the candidate filter; the per-tenant mutex is the partial unique index
    /// from migration 0008, so a concurrent second lease for one tenant fails
    /// structurally and reports as `None` rather than racing.
    pub async fn acquire(
        &self,
        worker: &str,
        now: Timestamp,
        ttl_seconds: i64,
    ) -> Result<Option<LeasedItem>, StorageError> {
        let expires = timestamp_to_db(Timestamp(now.0 + ttl_seconds * 1000))?;
        let mut tx = self.pool.begin().await?;
        let leased = sqlx::query!(
            r#"WITH candidate AS (
                 SELECT ji.org_id, ji.id
                 FROM job_item ji
                 JOIN job j ON j.org_id = ji.org_id AND j.id = ji.job_id
                 WHERE ji.state = 'queued'
                   AND NOT EXISTS (SELECT 1 FROM inventory_halt ih
                         WHERE ih.inventory = j.inventory
                           AND ih.marketplace = j.marketplace)
                   AND NOT EXISTS (SELECT 1 FROM org_halt oh
                         WHERE oh.org_id = ji.org_id)
                   AND NOT EXISTS (SELECT 1 FROM org_inventory_halt oih
                         WHERE oih.org_id = ji.org_id
                           AND oih.inventory = j.inventory
                           AND oih.marketplace = j.marketplace)
                   AND EXISTS (SELECT 1 FROM connection c
                         WHERE c.org_id = ji.org_id
                           AND c.marketplace = j.marketplace
                           AND c.state = 'linked')
                   AND NOT EXISTS (SELECT 1 FROM job_item live
                         WHERE live.org_id = ji.org_id
                           AND live.state IN ('leased', 'running', 'verifying'))
                 ORDER BY ji.created_at, ji.id
                 LIMIT 1
                 FOR UPDATE OF ji SKIP LOCKED
               )
               UPDATE job_item AS item
               SET state = 'leased', lease_owner = $1, lease_expires_at = $2
               FROM candidate, job j2
               WHERE item.org_id = candidate.org_id AND item.id = candidate.id
                 AND j2.org_id = item.org_id AND j2.id = item.job_id
               RETURNING item.org_id, item.id, item.job_id, item.mapping_id,
                 item.idempotency_key, item.lease_epoch, item.attempt_count,
                 j2.inventory AS "inventory!""#,
            worker,
            expires,
        )
        .fetch_optional(&mut *tx)
        .await;
        let leased = match map_unique(leased, "job_item_one_live_lease_per_org", || {
            StorageError::StaleLease
        }) {
            Ok(row) => row,
            // The mutex index fired: another worker holds this tenant.
            Err(StorageError::StaleLease) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        tx.commit().await?;
        leased
            .map(|row| {
                Ok(LeasedItem {
                    org: OrgId(uuid_from_db(row.org_id)),
                    item: JobItemId(uuid_from_db(row.id)),
                    job: JobId(uuid_from_db(row.job_id)),
                    mapping: MappingId(uuid_from_db(row.mapping_id)),
                    inventory: inventory_from_db(&row.inventory)?,
                    idempotency_key: IdempotencyKey(uuid_from_db(row.idempotency_key)),
                    lease_epoch: row.lease_epoch,
                    attempt_count: row.attempt_count,
                })
            })
            .transpose()
    }

    /// Every fenced write shares this shape: the epoch must still match, and
    /// a zero-row update is the stale worker finding out, not racing.
    pub async fn settle(
        &self,
        lease: &LeaseRef,
        verdict: &ItemVerdict,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let LeaseRef {
            org,
            item,
            lease_epoch,
        } = *lease;
        let ItemVerdict {
            outcome,
            failure_code,
            failure_detail,
        } = verdict;
        let updated = sqlx::query!(
            "UPDATE job_item \
             SET state = 'settled', outcome = $4, failure_code = $5, failure_detail = $6, \
                 settled_at = $7, lease_owner = NULL, lease_expires_at = NULL \
             WHERE org_id = $1 AND id = $2 AND lease_epoch = $3 \
               AND state IN ('leased', 'running', 'verifying')",
            uuid_to_db(org.0),
            uuid_to_db(item.0),
            lease_epoch,
            item_outcome_to_db(*outcome),
            failure_code.map(failure_code_to_db),
            failure_detail.as_ref().map(|detail| detail.0.as_str()),
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(StorageError::StaleLease);
        }
        Ok(())
    }

    pub async fn park(
        &self,
        lease: &LeaseRef,
        blocked_on: &str,
        park_expires: Timestamp,
    ) -> Result<(), StorageError> {
        let LeaseRef {
            org,
            item,
            lease_epoch,
        } = *lease;
        let updated = sqlx::query!(
            "UPDATE job_item \
             SET state = 'parked_live', blocked_on = $4, park_expires_at = $5, \
                 lease_owner = NULL, lease_expires_at = NULL \
             WHERE org_id = $1 AND id = $2 AND lease_epoch = $3 \
               AND state IN ('leased', 'running', 'verifying')",
            uuid_to_db(org.0),
            uuid_to_db(item.0),
            lease_epoch,
            blocked_on,
            timestamp_to_db(park_expires)?,
        )
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(StorageError::StaleLease);
        }
        Ok(())
    }

    /// Requeues expired leases with the epoch bumped so the previous holder's
    /// writes are fenced out, and settles items that exhausted their attempt
    /// budget as failed rather than requeueing them forever.
    pub async fn expire_and_steal(
        &self,
        now: Timestamp,
        attempts_max: i32,
    ) -> Result<u64, StorageError> {
        let now_db = timestamp_to_db(now)?;
        let failed = sqlx::query!(
            "UPDATE job_item \
             SET state = 'settled', outcome = 'failed', failure_code = 'Other', \
                 settled_at = $1, lease_owner = NULL, lease_expires_at = NULL, \
                 lease_epoch = lease_epoch + 1 \
             WHERE state IN ('leased', 'running', 'verifying') \
               AND lease_expires_at <= $1 AND attempt_count + 1 >= $2",
            now_db,
            attempts_max,
        )
        .execute(&self.pool)
        .await?;
        let stolen = sqlx::query!(
            "UPDATE job_item \
             SET state = 'queued', lease_owner = NULL, lease_expires_at = NULL, \
                 lease_epoch = lease_epoch + 1, attempt_count = attempt_count + 1 \
             WHERE state IN ('leased', 'running', 'verifying') \
               AND lease_expires_at <= $1",
            now_db,
        )
        .execute(&self.pool)
        .await?;
        Ok(failed.rows_affected() + stolen.rows_affected())
    }

    /// The tenant's connection for a marketplace, if one is linked.
    pub async fn connection_for(
        &self,
        org: OrgId,
        inventory: InventoryId,
    ) -> Result<Option<tam_types::ConnectionId>, StorageError> {
        let row = sqlx::query_scalar!(
            r#"SELECT id AS "id!" FROM connection
             WHERE org_id = $1 AND marketplace = $2 AND state = 'linked'"#,
            uuid_to_db(org.0),
            marketplace_to_db(inventory.marketplace()),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|id| tam_types::ConnectionId(uuid_from_db(id))))
    }

    /// Flips the tenant's connection to needs_reauth; the lease scan's
    /// linked-connection gate then holds every sibling item back, which is
    /// what RequeueBehindGate means.
    pub async fn gate_connection(
        &self,
        org: OrgId,
        inventory: InventoryId,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "UPDATE connection SET state = 'needs_reauth', updated_at = now() \
             WHERE org_id = $1 AND marketplace = $2 AND state = 'linked'",
            uuid_to_db(org.0),
            marketplace_to_db(inventory.marketplace()),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub struct HaltRepo {
    pool: PgPool,
}

impl HaltRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn raise_org_inventory(
        &self,
        org: OrgId,
        inventory: InventoryId,
        cause: &HaltCause,
    ) -> Result<(), StorageError> {
        let HaltCause {
            raised_by,
            reason,
            at,
        } = cause;
        sqlx::query!(
            "INSERT INTO org_inventory_halt \
             (org_id, inventory, marketplace, raised_by, reason, raised_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (org_id, inventory, marketplace) DO NOTHING",
            uuid_to_db(org.0),
            inventory_to_db(inventory),
            marketplace_to_db(inventory.marketplace()),
            raised_by,
            reason,
            timestamp_to_db(*at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn raise_org(&self, org: OrgId, cause: &HaltCause) -> Result<(), StorageError> {
        let HaltCause {
            raised_by,
            reason,
            at,
        } = cause;
        sqlx::query!(
            "INSERT INTO org_halt (org_id, raised_by, reason, raised_at)              VALUES ($1, $2, $3, $4) ON CONFLICT (org_id) DO NOTHING",
            uuid_to_db(org.0),
            raised_by,
            reason,
            timestamp_to_db(*at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn raise_fleet_inventory(
        &self,
        inventory: InventoryId,
        cause: &HaltCause,
    ) -> Result<(), StorageError> {
        let HaltCause {
            raised_by,
            reason,
            at,
        } = cause;
        sqlx::query!(
            "INSERT INTO inventory_halt (inventory, marketplace, raised_by, reason, raised_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (inventory, marketplace) DO NOTHING",
            inventory_to_db(inventory),
            marketplace_to_db(inventory.marketplace()),
            raised_by,
            reason,
            timestamp_to_db(*at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// What a write attempt intends: the projected body and its hash.
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptIntent {
    pub body: serde_json::Value,
    pub hash: Vec<u8>,
}

/// How an attempt settled in the ledger's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptVerdict {
    pub state: String,
    pub failure_code: Option<FailureCode>,
    /// The listing the write landed on, carried by the receipt a committed
    /// outcome holds. Without it the ledger cannot say what it created, so
    /// reconciliation, verification and dedup have nothing to address.
    pub landed: Option<RemoteListingId>,
}

/// The two rows a settlement writes: the fenced attempt, and the mapping a
/// landed write binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptRef {
    pub attempt: Uuid,
    pub mapping: MappingId,
}

/// The fencing token's ledger: the row is written before the click, because
/// the commit boundary is intent recorded rather than response received.
pub struct WriteAttemptRepo {
    pool: PgPool,
}

impl WriteAttemptRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Opens the in-flight row and mints its identifier. The partial unique
    /// index refuses a second in-flight attempt for the mapping, which is
    /// the duplicate-upload storm failing at the database.
    pub async fn open(
        &self,
        lease: &LeaseRef,
        mapping: MappingId,
        intent: &AttemptIntent,
        at: Timestamp,
    ) -> Result<tam_types::Uuid, StorageError> {
        let AttemptIntent { body, hash } = intent;
        let attempt = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
        let inserted = sqlx::query!(
            "INSERT INTO write_attempt              (org_id, id, job_item_id, mapping_id, lease_epoch, intent,               intent_hash, state, opened_at)              VALUES ($1, $2, $3, $4, $5, $6, $7, 'in_flight', $8)",
            uuid_to_db(lease.org.0),
            uuid_to_db(attempt),
            uuid_to_db(lease.item.0),
            uuid_to_db(mapping.0),
            lease.lease_epoch,
            body,
            hash.as_slice(),
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await;
        map_unique(inserted, "write_attempt_one_in_flight", || {
            StorageError::AttemptInFlight
        })?;
        Ok(attempt)
    }

    /// Epoch-fenced settlement of the attempt row, and — when the verdict
    /// carries a landed listing — the mapping bind, in one transaction. A
    /// separate bind call would leave a crash window where the attempt says
    /// committed while the mapping stays unbound.
    pub async fn settle(
        &self,
        lease: &LeaseRef,
        settling: AttemptRef,
        verdict: &AttemptVerdict,
        at: Timestamp,
    ) -> Result<BindDisposition, StorageError> {
        let AttemptRef { attempt, mapping } = settling;
        let AttemptVerdict {
            state,
            failure_code,
            landed,
        } = verdict;
        let remote = landed.as_ref().map(RemoteIdColumns::encode).transpose()?;
        let org_db = uuid_to_db(lease.org.0);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query!(
            "UPDATE write_attempt              SET state = $4, settled_at = $5, failure_code = $6,                  remote_id_kind = $7, remote_url = $8, remote_numeric_id = $9              WHERE org_id = $1 AND id = $2 AND lease_epoch = $3                AND state = 'in_flight'",
            org_db,
            uuid_to_db(attempt),
            lease.lease_epoch,
            state.as_str(),
            at_db,
            failure_code.map(failure_code_to_db),
            remote.as_ref().map(|remote| remote.kind),
            remote.as_ref().and_then(|remote| remote.url),
            remote.as_ref().and_then(|remote| remote.numeric_id),
        )
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(StorageError::StaleLease);
        }
        let (Some(landed), Some(remote)) = (landed.as_ref(), remote) else {
            tx.commit().await?;
            return Ok(BindDisposition::NotLanded);
        };
        let mapping_db = uuid_to_db(mapping.0);
        sqlx::query("SAVEPOINT bind").execute(&mut *tx).await?;
        let bound = sqlx::query!(
            "UPDATE mapping \
             SET binding_state = 'bound', \
                 remote_id_kind = $3, remote_url = $4, remote_numeric_id = $5, \
                 first_seen_at = $6, \
                 verify_state = 'stale', verified_at = NULL, verify_stale_since = $6, \
                 binding_attempt = NULL, binding_marker = NULL, \
                 ambiguous_since = NULL, severed_at = NULL, sever_cause = NULL, \
                 updated_at = $6 \
             WHERE org_id = $1 AND id = $2 \
               AND binding_state IN ('unbound', 'creating')",
            org_db,
            mapping_db,
            remote.kind,
            remote.url,
            remote.numeric_id,
            at_db,
        )
        .execute(&mut *tx)
        .await;
        let disposition = match bound {
            Ok(bound) if bound.rows_affected() > 0 => BindDisposition::Bound,
            Ok(_) => classify_bind(&mut tx, org_db, mapping_db, landed).await?,
            Err(clash) if is_bound_identity_clash(&clash) => {
                sqlx::query("ROLLBACK TO SAVEPOINT bind")
                    .execute(&mut *tx)
                    .await?;
                claimed_elsewhere(&mut tx, org_db, mapping_db, &remote).await?
            }
            Err(error) => return Err(error.into()),
        };
        tx.commit().await?;
        Ok(disposition)
    }
}

/// What the bind folded into an attempt settle did to the mapping.
///
/// A bind never overwrites: re-landing the same identifier is idempotent and
/// preserves `first_seen_at`, a different identifier against a bound row is
/// reported rather than written, and a listing another mapping in the same
/// inventory already holds is the cross-mapping form of the same refusal.
/// Every one of them still settles the attempt, because the second listing is
/// already queryable from the settled attempt's own remote-id columns and
/// refusing to record a write that landed would retry it into a third.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindDisposition {
    /// The verdict carried no landed listing, so there was nothing to bind.
    NotLanded,
    Bound,
    AlreadyBound,
    DivergentLanding {
        existing: RemoteListingId,
    },
    ClaimedElsewhere {
        existing_mapping: MappingId,
    },
    Refused {
        state: String,
    },
}

/// The partial unique indexes migration 0018 laid over a bound remote
/// identity. A violation of either is one mapping landing on the listing
/// another mapping in the same inventory already holds.
const BOUND_IDENTITY_INDEXES: [&str; 2] = ["mapping_one_bound_url", "mapping_one_bound_numeric_id"];

fn is_bound_identity_clash(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database) = error else {
        return false;
    };
    database
        .constraint()
        .is_some_and(|name| BOUND_IDENTITY_INDEXES.contains(&name))
}

/// Which mapping in the same inventory already holds the identifier the bind
/// tried to claim. The index that refused the bind guarantees at most one.
async fn claimed_elsewhere(
    tx: &mut Transaction<'_, Postgres>,
    org: uuid::Uuid,
    mapping: uuid::Uuid,
    remote: &RemoteIdColumns<'_>,
) -> Result<BindDisposition, StorageError> {
    let claimant = sqlx::query!(
        r#"SELECT claimant.id AS "id!"
           FROM mapping AS target
           JOIN mapping AS claimant
             ON claimant.org_id = target.org_id
            AND claimant.inventory = target.inventory
           WHERE target.org_id = $1 AND target.id = $2
             AND claimant.binding_state = 'bound'
             AND claimant.remote_id_kind = $3
             AND claimant.remote_url IS NOT DISTINCT FROM $4
             AND claimant.remote_numeric_id IS NOT DISTINCT FROM $5"#,
        org,
        mapping,
        remote.kind,
        remote.url,
        remote.numeric_id,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| StorageError::Inconsistent {
        reason: "the bound remote identity was refused by an index that names no claimant"
            .to_owned(),
    })?;
    Ok(BindDisposition::ClaimedElsewhere {
        existing_mapping: MappingId(uuid_from_db(claimant.id)),
    })
}

/// Why the fenced bind matched no row, read inside the settling transaction
/// so the answer is the state the bind was refused against.
async fn classify_bind(
    tx: &mut Transaction<'_, Postgres>,
    org: uuid::Uuid,
    mapping: uuid::Uuid,
    landed: &RemoteListingId,
) -> Result<BindDisposition, StorageError> {
    let row = sqlx::query!(
        "SELECT binding_state, remote_id_kind, remote_url, remote_numeric_id \
         FROM mapping WHERE org_id = $1 AND id = $2",
        org,
        mapping,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| StorageError::CorruptRow {
        reason: "the settled attempt names a mapping that does not exist".to_owned(),
    })?;
    if row.binding_state != "bound" {
        return Ok(BindDisposition::Refused {
            state: row.binding_state,
        });
    }
    let existing = remote_id_from_db(
        row.remote_id_kind
            .as_deref()
            .ok_or_else(|| StorageError::CorruptRow {
                reason: "bound binding without a remote id kind".to_owned(),
            })?,
        row.remote_url,
        row.remote_numeric_id,
    )?;
    Ok(if existing == *landed {
        BindDisposition::AlreadyBound
    } else {
        BindDisposition::DivergentLanding { existing }
    })
}

/// Who raised a halt, why, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaltCause {
    pub raised_by: String,
    pub reason: String,
    pub at: Timestamp,
}

pub struct OutboxRepo {
    pool: PgPool,
}

/// One pending message to append, in the causing transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct NewOutboxMessage {
    pub org: OrgId,
    pub id: tam_types::Uuid,
    pub topic: String,
    pub dedupe_key: String,
    pub payload: serde_json::Value,
    pub at: Timestamp,
}

/// The identity a drainer's compare-and-set speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef {
    pub org: OrgId,
    pub id: tam_types::Uuid,
    pub attempts_seen: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutboxMessage {
    pub org: OrgId,
    pub id: tam_types::Uuid,
    pub topic: String,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

impl OutboxMessage {
    #[must_use]
    pub const fn reference(&self) -> MessageRef {
        MessageRef {
            org: self.org,
            id: self.id,
            attempts_seen: self.attempts,
        }
    }
}

impl OutboxRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// In the caller's transaction, because the outbox row and the state
    /// change that caused it must commit together or not at all.
    pub async fn append(
        tx: &mut Transaction<'_, Postgres>,
        message: &NewOutboxMessage,
    ) -> Result<(), StorageError> {
        let NewOutboxMessage {
            org,
            id,
            topic,
            dedupe_key,
            payload,
            at,
        } = message;
        sqlx::query!(
            "INSERT INTO outbox_message \
             (org_id, id, topic, dedupe_key, payload, created_at, available_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $6) \
             ON CONFLICT (org_id, topic, dedupe_key) DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(*id),
            topic,
            dedupe_key,
            payload,
            timestamp_to_db(*at)?,
        )
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn claim_due(
        &self,
        now: Timestamp,
        limit: i64,
    ) -> Result<Vec<OutboxMessage>, StorageError> {
        let rows = sqlx::query!(
            "SELECT org_id, id, topic, payload, attempts FROM outbox_message \
             WHERE state = 'pending' AND available_at <= $1 \
             ORDER BY available_at LIMIT $2",
            timestamp_to_db(now)?,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| OutboxMessage {
                org: OrgId(uuid_from_db(row.org_id)),
                id: uuid_from_db(row.id),
                topic: row.topic,
                payload: row.payload,
                attempts: row.attempts,
            })
            .collect())
    }

    /// Compare-and-set on the attempt count, so two drainers cannot both
    /// account one delivery; no lock is held across the network call.
    pub async fn mark_delivered(
        &self,
        message: &MessageRef,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let MessageRef {
            org,
            id,
            attempts_seen,
        } = *message;
        let updated = sqlx::query!(
            "UPDATE outbox_message SET state = 'delivered', delivered_at = $4 \
             WHERE org_id = $1 AND id = $2 AND state = 'pending' AND attempts = $3",
            uuid_to_db(org.0),
            uuid_to_db(id),
            attempts_seen,
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn retry_later(
        &self,
        message: &MessageRef,
        next_at: Timestamp,
        error: &str,
    ) -> Result<bool, StorageError> {
        let MessageRef {
            org,
            id,
            attempts_seen,
        } = *message;
        let updated = sqlx::query!(
            "UPDATE outbox_message \
             SET attempts = attempts + 1, available_at = $4, last_error = $5 \
             WHERE org_id = $1 AND id = $2 AND state = 'pending' AND attempts = $3",
            uuid_to_db(org.0),
            uuid_to_db(id),
            attempts_seen,
            timestamp_to_db(next_at)?,
            error,
        )
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Dead-lettering is the poison-message handling: quarantined by attempt
    /// count alone, raised as an alert, never retried further.
    pub async fn mark_dead(&self, message: &MessageRef, error: &str) -> Result<bool, StorageError> {
        let MessageRef {
            org,
            id,
            attempts_seen,
        } = *message;
        let updated = sqlx::query!(
            "UPDATE outbox_message \
             SET state = 'dead', attempts = attempts + 1, last_error = $4 \
             WHERE org_id = $1 AND id = $2 AND state = 'pending' AND attempts = $3",
            uuid_to_db(org.0),
            uuid_to_db(id),
            attempts_seen,
            error,
        )
        .execute(&self.pool)
        .await?;
        Ok(updated.rows_affected() == 1)
    }
}

pub struct RateBudgetRepo {
    pool: PgPool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetGrant {
    Granted { used: i32 },
    Exhausted,
}

impl RateBudgetRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// One action against the per-connection window; refusal is the governor
    /// governing, not an error.
    pub async fn consume(
        &self,
        org: OrgId,
        connection: tam_types::ConnectionId,
        window_start: Timestamp,
        ceiling: i32,
    ) -> Result<BudgetGrant, StorageError> {
        let used = sqlx::query_scalar!(
            r#"INSERT INTO rate_budget (org_id, connection_id, window_start, actions_used)
             VALUES ($1, $2, $3, 1)
             ON CONFLICT (org_id, connection_id, window_start)
             DO UPDATE SET actions_used = rate_budget.actions_used + 1
             WHERE rate_budget.actions_used < $4
             RETURNING actions_used AS "used!""#,
            uuid_to_db(org.0),
            uuid_to_db(connection.0),
            timestamp_to_db(window_start)?,
            ceiling,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(match used {
            Some(used) => BudgetGrant::Granted { used },
            None => BudgetGrant::Exhausted,
        })
    }
}

pub(crate) fn map_unique<T>(
    outcome: Result<T, sqlx::Error>,
    constraint: &str,
    to_error: impl FnOnce() -> StorageError,
) -> Result<T, StorageError> {
    match outcome {
        Ok(value) => Ok(value),
        Err(sqlx::Error::Database(database)) if database.constraint() == Some(constraint) => {
            Err(to_error())
        }
        Err(error) => Err(error.into()),
    }
}

/// What the operation-resource create answered: the job the key names, and
/// whether this request had already run. The retry and the double-click are
/// the same request, so they get the same job back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedJob {
    pub job: JobId,
    pub replay: bool,
}

impl JobRepo {
    /// `enqueue` under a client-supplied request idempotency key. A key seen
    /// before returns the original job untouched; a race between two
    /// carriers of one key resolves at the unique constraint, and the loser
    /// reads the winner's job.
    pub async fn create_with_request_key(
        &self,
        org: OrgId,
        request_key: Uuid,
        new: &NewJob,
        items: &[NewJobItem],
    ) -> Result<CreatedJob, StorageError> {
        if let Some(existing) = self.job_for_request_key(org, request_key).await? {
            return Ok(CreatedJob {
                job: existing,
                replay: true,
            });
        }
        let NewJob { job, inventory, at } = *new;
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        let inserted = sqlx::query!(
            "INSERT INTO job \
             (org_id, id, inventory, marketplace, created_at, request_idempotency_key) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            org_db,
            uuid_to_db(job.0),
            inventory_to_db(inventory),
            marketplace_to_db(inventory.marketplace()),
            at_db,
            uuid_to_db(request_key),
        )
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {}
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some("job_request_idempotent") =>
            {
                drop(tx);
                let existing = self.job_for_request_key(org, request_key).await?.ok_or(
                    StorageError::Inconsistent {
                        reason: "the winning request's job must exist".to_owned(),
                    },
                )?;
                return Ok(CreatedJob {
                    job: existing,
                    replay: true,
                });
            }
            Err(error) => return Err(error.into()),
        }
        for item in items {
            let inserted = sqlx::query!(
                "INSERT INTO job_item \
                 (org_id, id, job_id, mapping_id, idempotency_key, state, created_at) \
                 VALUES ($1, $2, $3, $4, $5, 'queued', $6)",
                org_db,
                uuid_to_db(item.item.0),
                uuid_to_db(job.0),
                uuid_to_db(item.mapping.0),
                uuid_to_db(item.idempotency_key.0),
                at_db,
            )
            .execute(&mut *tx)
            .await;
            map_unique(inserted, "job_item_idempotent", || {
                StorageError::DuplicateIdempotencyKey {
                    key: uuid_to_db(item.idempotency_key.0),
                }
            })?;
        }
        let item_count = u32::try_from(items.len()).map_err(|_| StorageError::Inconsistent {
            reason: format!("{} items exceed the event range", items.len()),
        })?;
        append_event(
            &mut tx,
            &EventScope {
                org,
                job,
                item: None,
            },
            &JobEventPayload::JobQueued { items: item_count },
            at,
        )
        .await?;
        tx.commit().await?;
        Ok(CreatedJob { job, replay: false })
    }

    async fn job_for_request_key(
        &self,
        org: OrgId,
        request_key: Uuid,
    ) -> Result<Option<JobId>, StorageError> {
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        let found = sqlx::query_scalar!(
            "SELECT id FROM job WHERE org_id = $1 AND request_idempotency_key = $2",
            uuid_to_db(org.0),
            uuid_to_db(request_key),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(found.map(|id| JobId(uuid_from_db(id))))
    }
}

/// One inventory's halt, for the public status page: global reference state,
/// no tenant data, readable without a session by design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryHaltRow {
    pub inventory: InventoryId,
    pub raised_by: String,
    pub reason: String,
    pub raised_at: Timestamp,
}

impl HaltRepo {
    pub async fn inventory_halts(&self) -> Result<Vec<InventoryHaltRow>, StorageError> {
        let rows = sqlx::query!(
            "SELECT inventory, raised_by, reason, raised_at FROM inventory_halt \
             ORDER BY inventory",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(InventoryHaltRow {
                    inventory: inventory_from_db(&row.inventory)?,
                    raised_by: row.raised_by,
                    reason: row.reason,
                    raised_at: crate::codec::timestamp_from_db(row.raised_at),
                })
            })
            .collect()
    }
}
