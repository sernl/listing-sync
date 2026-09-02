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

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use tam_domain::{ItemOperation, ItemOutcome, JobItemId, SellerEvent};
use tam_marketplace::{IdempotencyKey, RemoteLifecycle, RemoteListingId};
use tam_types::{
    Actor, FailureCode, FailureDetail, InventoryId, JobEventPayload, JobId, MappingId, OrgId,
    Stamp, SystemComponent, Timestamp, Uuid,
};

use crate::codec::{
    failure_code_to_db, inventory_from_db, inventory_to_db, marketplace_to_db, timestamp_to_db,
    uuid_from_db, uuid_to_db, OperationColumns, RemoteIdColumns, StoredOperation,
};
use crate::mapping::{remote_id_from_db, LifecycleColumns};
use crate::{pin_org, StorageError};

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

/// What opening a write attempt needs beyond the lease it runs under,
/// grouped for the same reason [`NewJob`] is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewAttempt<'a> {
    pub mapping: MappingId,
    pub intent: &'a AttemptIntent,
    pub stamp: Stamp,
}

/// The job-level half of an enqueue, grouped so call sites read as one
/// value rather than a parameter list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    pub job: JobId,
    pub inventory: InventoryId,
    /// When it was queued and who queued it. Job-level like the rest of this
    /// struct: a job has one author, and carrying it here rather than beside
    /// it keeps a caller from describing one job and attributing another.
    pub stamp: Stamp,
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
    /// What this item does to the listing its mapping names, and which
    /// listing the enqueuer asserts that is. Stored rather than derived: no
    /// column records which side of the draft line a listing sits on, and a
    /// subject the caller states is a divergence the engine can detect,
    /// where an unstated one silently retargets a rebound mapping.
    pub operation: ItemOperation,
    /// The inventory whose binding this item waits on, if any.
    ///
    /// A migrate's removal names the target: the source listing must not go
    /// until the target listing exists and we have seen it. A publish names
    /// its own inventory: the create binds the id the publish must address,
    /// and FIFO within a job would usually get that right but not when the
    /// create parks on an election and the publish leases first.
    pub requires_bound_on: Option<InventoryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedItem {
    pub org: OrgId,
    pub item: JobItemId,
    pub job: JobId,
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub idempotency_key: IdempotencyKey,
    pub operation: ItemOperation,
    pub lease_epoch: i64,
    pub attempt_count: i32,
    pub requires_bound_on: Option<InventoryId>,
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
        let NewJob {
            job,
            inventory,
            stamp,
        } = *new;
        let Stamp { at, actor } = stamp;
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO job \
             (org_id, id, inventory, marketplace, created_at, actor_kind, actor_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            org_db,
            uuid_to_db(job.0),
            inventory_to_db(inventory),
            marketplace_to_db(inventory.marketplace()),
            at_db,
            actor.kind(),
            actor.id(),
        )
        .execute(&mut *tx)
        .await?;
        for item in items {
            insert_job_item(&mut tx, org_db, uuid_to_db(job.0), item, at_db).await?;
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
            stamp,
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
        scope: &EventScope,
        payload: &JobEventPayload,
        stamp: Stamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, scope.org).await?;
        append_event(&mut tx, scope, payload, stamp).await?;
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

/// One inventory's recent terminal outcomes, for the fleet breaker: how many
/// items settled at all and how many of those say the marketplace is not
/// working. Cross-tenant by construction; runs on the engine role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryFailureWindow {
    pub inventory: InventoryId,
    pub settled: i64,
    /// `failed`, `ambiguous` and `blocked`. The third is the one that is easy
    /// to leave out and the one that hurts most when it is: a marketplace-wide
    /// condition that stops every tenant *before* the write — an expired
    /// session, an unclearable challenge, a preflight that will not complete —
    /// settles `blocked` on every tenant at once. Counted only in the
    /// denominator it does the opposite of its job, driving the ratio down
    /// precisely as the fleet-wide event this breaker exists for unfolds.
    pub adverse: i64,
}

impl JobRepo {
    pub async fn recent_outcomes(
        &self,
        since: Timestamp,
    ) -> Result<Vec<InventoryFailureWindow>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT j.inventory AS "inventory!",
                 count(*) AS "settled!",
                 count(*) FILTER (WHERE ji.outcome IN ('failed', 'ambiguous', 'blocked'))
                     AS "adverse!"
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
                    adverse: row.adverse,
                })
            })
            .collect()
    }
}

/// The one `INSERT INTO job_item`. Both enqueue paths route through it, so a
/// column the item grows cannot reach one site and miss the other: migration
/// 0019 drops `operation`'s default after backfilling, which turns such an
/// omission into a NOT NULL violation rather than a removal silently stored
/// and later run as a create.
async fn insert_job_item(
    tx: &mut Transaction<'_, Postgres>,
    org: uuid::Uuid,
    job: uuid::Uuid,
    item: &NewJobItem,
    at: DateTime<Utc>,
) -> Result<(), StorageError> {
    let operation = OperationColumns::encode(&item.operation)?;
    // `marketplace` is read from the mapping inside the statement rather than
    // supplied: the live-lease mutex is an index over it, so a caller-supplied
    // value that disagreed with the mapping would put the mutex on the wrong
    // marketplace account. Deriving it here makes that unrepresentable.
    let inserted = sqlx::query!(
        "INSERT INTO job_item \
         (org_id, id, job_id, mapping_id, idempotency_key, state, created_at, \
          operation, subject_kind, subject_url, subject_numeric_id, \
          state_from, state_to, requires_bound_on, marketplace) \
         SELECT $1, $2, $3, $4, $5, 'queued', $6, $7, $8, $9, $10, $11, $12, $13, \
                m.marketplace \
         FROM mapping m WHERE m.org_id = $1 AND m.id = $4",
        org,
        uuid_to_db(item.item.0),
        job,
        uuid_to_db(item.mapping.0),
        uuid_to_db(item.idempotency_key.0),
        at,
        operation.operation,
        operation.subject_kind,
        operation.subject_url,
        operation.subject_numeric_id,
        operation.state_from,
        operation.state_to,
        item.requires_bound_on.map(inventory_to_db),
    )
    .execute(&mut **tx)
    .await;
    map_unique(inserted, "job_item_idempotent", || {
        StorageError::DuplicateIdempotencyKey {
            key: uuid_to_db(item.idempotency_key.0),
        }
    })?;
    Ok(())
}

/// Takes the per-organisation event lock without spending a sequence number.
///
/// The same row `allocate_org_seq` locks, held from wherever the caller needs
/// serialisation to begin rather than from the append. A reader that decides
/// something from a count and then appends on the strength of it has to take
/// it before the count, or two transactions each read a snapshot without the
/// other's uncommitted write and both decide not to append.
async fn lock_org_counter(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
) -> Result<(), StorageError> {
    let org_db = uuid_to_db(org.0);
    sqlx::query!(
        "INSERT INTO org_event_counter (org_id) VALUES ($1) ON CONFLICT (org_id) DO NOTHING",
        org_db,
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "SELECT next_seq FROM org_event_counter WHERE org_id = $1 FOR UPDATE",
        org_db,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(())
}

/// Allocates the next `org_seq` by locking the per-organisation counter row.
/// The lock is what serialises event appends within one organisation, which
/// the conditional append below relies on: a second transaction evaluating
/// its own existence check has already waited on this row, so it sees the
/// first one's committed event.
async fn allocate_org_seq(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
) -> Result<i64, StorageError> {
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
    Ok(seq)
}

/// Appends one event inside the caller's transaction, allocating `org_seq`
/// by locking the per-organisation counter row — identity values are
/// allocated before commit and can appear out of order, which is exactly the
/// resume-query unsoundness the counter exists to repair.
pub async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    scope: &EventScope,
    payload: &JobEventPayload,
    stamp: Stamp,
) -> Result<(), StorageError> {
    let Stamp { at, actor } = stamp;
    let EventScope { org, job, item } = *scope;
    let org_db = uuid_to_db(org.0);
    let seq = allocate_org_seq(tx, org).await?;
    let encoded = serde_json::to_value(payload).map_err(|error| StorageError::Inconsistent {
        reason: format!("a job event payload must serialise: {error}"),
    })?;
    let body = encoded
        .get(payload.kind())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    sqlx::query!(
        "INSERT INTO job_event \
         (org_id, org_seq, job_id, job_item_id, kind, payload, created_at, \
          actor_kind, actor_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        org_db,
        seq,
        uuid_to_db(job.0),
        item.map(|item| uuid_to_db(item.0)),
        payload.kind(),
        body,
        timestamp_to_db(at)?,
        actor.kind(),
        actor.id(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The gates a projection or an election park writes, and the only ones an
/// expiry revive may touch.
///
/// A challenge park is a different animal: `SyncMachine::park` advances with
/// the write attempt still `in_flight`, and `write_attempt_one_in_flight` then
/// refuses `AttemptRepo::open` on the revived run, so the item burns its
/// attempt budget and settles `failed`/`Other` a day later with nothing in the
/// ledger to explain it. `blocked_on` is what tells the two apart: these are
/// written by the seed gate, and a challenge park writes the challenge's own
/// debug form.
pub const REVIVABLE_GATES: [&str; 10] = [
    "reconciliation",
    ELECTION,
    "currency_unknown",
    "cover_missing",
    "scan_incomplete",
    AWAITING_COUNTERPART,
    // The four `admission` writes. They are here for the give-up arm rather
    // than for the revive: the arm filters on this same list, so a gate
    // outside it parks for a day, re-parks forever and never settles, and
    // the job it belongs to reads active with no event and no seller
    // notification ever. Reviving them costs one lease a day until the
    // attempt budget runs out, which is what makes the give-up reachable.
    "binding",
    "unbound",
    "subject_diverged",
    "lifecycle_diverged",
];

/// The gate a counterpart-bound item waits on. Stated here because both the
/// gate that writes it and the revive that clears it read it, and a second
/// spelling of it is a park nothing ever wakes.
pub const AWAITING_COUNTERPART: &str = "awaiting_counterpart";

/// The gate an open seller election writes, for the same reason: the
/// projection that raises it, the answer that clears it and the worker's
/// own re-check after parking all name it.
pub const ELECTION: &str = "election";

/// The gate a session that died mid-submit parks on.
///
/// Deliberately not in `REVIVABLE_GATES`: that list is time-gated, and a
/// re-link is the only thing that clears this one, so the clock must not.
/// `LeaseRepo::revive_expired` reads it back on its own arm.
///
/// The driver writes it as `ChallengeKind::ReauthRequired`'s `Debug` form
/// rather than through a codec, so this spelling is pinned against that
/// derivation in this crate's tests; a second spelling of it is a park
/// nothing ever wakes.
pub const REAUTH_REQUIRED: &str = "ReauthRequired";

/// The gap queue's revive. Keyed on the gate rather than on the mapping,
/// because one `reconciliation_item` row stands for every product that hit
/// it: `reconciliation_item_open_dedup` collapses a five-hundred-product
/// batch into one question, so resolving it must un-block all five hundred
/// items and not the one whose mapping happens to be named as provenance.
///
/// Requeuing an item whose gap is still open is harmless — it re-projects,
/// re-parks, and costs one lease.
pub async fn revive_by_gap(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    gate: &str,
    at: Timestamp,
) -> Result<u64, StorageError> {
    pin_org(tx, org).await?;
    let rows = sqlx::query!(
        "UPDATE job_item \
         SET state = 'queued', blocked_on = NULL, park_expires_at = NULL \
         WHERE org_id = $1 AND state = 'parked_live' AND blocked_on = $2 \
         RETURNING org_id, job_id, id",
        uuid_to_db(org.0),
        gate,
    )
    .fetch_all(&mut **tx)
    .await?;
    let revived = revived(rows.into_iter().map(|row| (row.org_id, row.job_id, row.id)));
    record_resumptions(tx, &revived, at).await?;
    Ok(count_of(&revived))
}

/// The election queue's revive. Keyed on the mapping, because
/// `election_item_open_dedup` makes an election one-to-one with a product's
/// mapping and there is nothing to fan out to.
///
/// Both revives take the answering transaction rather than their own pool
/// handle: a crash between recording the seller's answer and requeueing the
/// item leaves it parked for the full day, which is exactly the latency this
/// exists to remove. And both pin, because `job_item` carries FORCE ROW LEVEL
/// SECURITY — the clause that removes the owner's exemption — so an unpinned
/// `tam_app` session updates nothing and reports it as a row count of zero.
pub async fn revive_on(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    mapping: MappingId,
    gate: &str,
    at: Timestamp,
) -> Result<u64, StorageError> {
    pin_org(tx, org).await?;
    let rows = sqlx::query!(
        "UPDATE job_item \
         SET state = 'queued', blocked_on = NULL, park_expires_at = NULL \
         WHERE org_id = $1 AND mapping_id = $2 AND state = 'parked_live' \
           AND blocked_on = $3 \
         RETURNING org_id, job_id, id",
        uuid_to_db(org.0),
        uuid_to_db(mapping.0),
        gate,
    )
    .fetch_all(&mut **tx)
    .await?;
    let revived = revived(rows.into_iter().map(|row| (row.org_id, row.job_id, row.id)));
    record_resumptions(tx, &revived, at).await?;
    Ok(count_of(&revived))
}

/// The binding's own revive: every item of this tenant waiting for this
/// product to be bound on this inventory, released by the binding rather than
/// by the clock.
///
/// The two items a live intent lowers to share a `created_at`, so `acquire`'s
/// FIFO tie-breaks on a fresh uuid and roughly half of publish-to-live
/// enqueues lease the publish first. It parks on `awaiting_counterpart`, and
/// until this existed only the day-long park expiry cleared it -- a seller
/// who asked for a live listing got one the next day, for a create that
/// landed seconds later. A migrate's removal waits on the same gate.
///
/// Keyed on the product and the inventory rather than on the mapping,
/// because the waiter is a different mapping: `requires_bound_on` names the
/// inventory whose binding it waits for, and `counterpart_binding` resolves
/// that to the product's mapping there. The publish that waits on its own
/// create is the degenerate case of the same join and needs no second rule.
///
/// Pinned like the other two revives: `job_item` carries FORCE ROW LEVEL
/// SECURITY, so an unpinned `tam_app` update reports its miss as `Ok(0)`.
pub async fn revive_counterparts(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    bound: MappingId,
    at: Timestamp,
) -> Result<u64, StorageError> {
    pin_org(tx, org).await?;
    let rows = sqlx::query!(
        "UPDATE job_item ji \
         SET state = 'queued', blocked_on = NULL, park_expires_at = NULL \
         FROM mapping waiting, mapping landed \
         WHERE ji.org_id = $1 \
           AND waiting.org_id = $1 AND waiting.id = ji.mapping_id \
           AND landed.org_id = $1 AND landed.id = $2 \
           AND waiting.product_id = landed.product_id \
           AND ji.state = 'parked_live' \
           AND ji.blocked_on = $3 \
           AND ji.requires_bound_on = landed.inventory \
         RETURNING ji.org_id, ji.job_id, ji.id",
        uuid_to_db(org.0),
        uuid_to_db(bound.0),
        AWAITING_COUNTERPART,
    )
    .fetch_all(&mut **tx)
    .await?;
    let revived = revived(rows.into_iter().map(|row| (row.org_id, row.job_id, row.id)));
    record_resumptions(tx, &revived, at).await?;
    Ok(count_of(&revived))
}

fn revived(
    rows: impl Iterator<Item = (uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
) -> Vec<(OrgId, JobId, JobItemId)> {
    rows.map(|(org, job, item)| {
        (
            OrgId(uuid_from_db(org)),
            JobId(uuid_from_db(job)),
            JobItemId(uuid_from_db(item)),
        )
    })
    .collect()
}

fn count_of(revived: &[(OrgId, JobId, JobItemId)]) -> u64 {
    u64::try_from(revived.len()).unwrap_or(u64::MAX)
}

/// Records the resumption of every item a revive requeued, in the same
/// transaction, so the ledger explains why a parked item is running again.
async fn record_resumptions(
    tx: &mut Transaction<'_, Postgres>,
    revived: &[(OrgId, JobId, JobItemId)],
    at: Timestamp,
) -> Result<(), StorageError> {
    for &(org, job, item) in revived {
        append_event(
            tx,
            &EventScope {
                org,
                job,
                item: Some(item),
            },
            &JobEventPayload::ItemResumed,
            // The revive sweep is the engine's own timer firing; no request
            // and no seller is behind an item resuming.
            Stamp::system(SystemComponent::Engine, at),
        )
        .await?;
    }
    Ok(())
}

/// Emits `JobSettled` and the seller's notification once every item of a job
/// has settled, and does nothing otherwise.
///
/// This hangs off the ledger write rather than off the worker's control flow
/// because the commonest bulk failure never passes through the worker at all:
/// `expire_and_steal` settles an attempt-exhausted item `failed`/`Other` in
/// its own cross-tenant statement, from the maintenance loop, with no job
/// context. A job whose last item dies there would flip to `Settled` with no
/// event to explain it.
///
/// Serialised per organisation before the count, not after it. The count
/// decides whether to append and the append is conditional on the count, so
/// under READ COMMITTED two transactions settling a job's last two items each
/// saw the other's item unsettled, both returned early, and the job finished
/// with no `JobSettled` and no seller notification at all --
/// `job_item_one_live_lease_per_org` does not serialise them, because it
/// covers the live states a parked row does not have and `revive_expired`
/// settles parked rows from the maintenance loop. Taking the event lock first
/// makes the second transaction's count run on a snapshot that includes the
/// first's settle.
///
/// Idempotent at the database in the other direction too: `WHERE NOT EXISTS`
/// with `job_event_one_settled_per_job` behind it, so a duplicate is refused
/// by the index however the caller races.
pub async fn settle_if_complete(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    job: JobId,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let org_db = uuid_to_db(org.0);
    let job_db = uuid_to_db(job.0);
    lock_org_counter(tx, org).await?;
    let counts = sqlx::query!(
        r#"SELECT
             count(*) FILTER (WHERE state <> 'settled')          AS "unsettled!",
             count(*) FILTER (WHERE outcome = 'succeeded')       AS "succeeded!",
             count(*) FILTER (WHERE outcome = 'degraded')        AS "degraded!",
             count(*) FILTER (WHERE outcome = 'failed')          AS "failed!",
             count(*) FILTER (WHERE outcome = 'ambiguous')       AS "ambiguous!",
             count(*) FILTER (WHERE outcome = 'skipped')         AS "skipped!",
             count(*) FILTER (WHERE outcome = 'blocked')         AS "blocked!",
             count(*)                                            AS "total!"
           FROM job_item WHERE org_id = $1 AND job_id = $2"#,
        org_db,
        job_db,
    )
    .fetch_one(&mut **tx)
    .await?;
    if counts.total == 0 || counts.unsettled > 0 {
        return Ok(false);
    }

    let payload = JobEventPayload::JobSettled {
        succeeded: count_u32(counts.succeeded)?,
        degraded: count_u32(counts.degraded)?,
        failed: count_u32(counts.failed)?,
        ambiguous: count_u32(counts.ambiguous)?,
        skipped: count_u32(counts.skipped)?,
        blocked: count_u32(counts.blocked)?,
    };
    let seq = allocate_org_seq(tx, org).await?;
    let encoded = serde_json::to_value(&payload).map_err(|error| StorageError::Inconsistent {
        reason: format!("a job event payload must serialise: {error}"),
    })?;
    let body = encoded
        .get(payload.kind())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let inserted = sqlx::query!(
        "INSERT INTO job_event \
         (org_id, org_seq, job_id, job_item_id, kind, payload, created_at, \
          actor_kind, actor_id) \
         SELECT $1, $2, $3, NULL, $4, $5, $6, $7, $8 \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM job_event \
             WHERE org_id = $1 AND job_id = $3 AND kind = $4 \
         ) \
         ON CONFLICT DO NOTHING",
        org_db,
        seq,
        job_db,
        payload.kind(),
        body,
        timestamp_to_db(at)?,
        // A job settles when its last item settles, which the engine
        // observes rather than anyone requesting.
        Actor::System(SystemComponent::Engine).kind(),
        Actor::System(SystemComponent::Engine).id(),
    )
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 0 {
        return Ok(false);
    }

    OutboxRepo::append(
        tx,
        &NewOutboxMessage {
            org,
            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
            topic: "email.job_settled".to_owned(),
            dedupe_key: format!("{:?}:{:02x?}", SellerEvent::JobSettled, job.0 .0),
            payload: serde_json::json!({ "event": format!("{:?}", SellerEvent::JobSettled) }),
            at,
        },
    )
    .await?;
    Ok(true)
}

/// The ledger counts rows and the event vocabulary counts items; the widths
/// differ and the conversion is arithmetic rather than a design choice.
fn count_u32(count: i64) -> Result<u32, StorageError> {
    u32::try_from(count).map_err(|_| StorageError::Inconsistent {
        reason: format!("a job item count does not fit the event vocabulary: {count}"),
    })
}

/// How an item settled in the ledger's own vocabulary: the outcome, its
/// failure code, and the adapter's free text beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemVerdict {
    pub outcome: ItemOutcome,
    pub failure_code: Option<FailureCode>,
    pub failure_detail: Option<FailureDetail>,
}

/// Where an item's run of failed preflights stands: how many in a row, and
/// whether every one of them was the marketplace's edge rather than something
/// the seller could act on.
///
/// The second field travels with the first because the caller's verdict is
/// about the streak and not about its last member. A streak that mixed a
/// Cloudflare block with a lapsed session would otherwise be judged by
/// whichever arrived last, which is arrival order deciding whether a seller is
/// told to re-link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreflightStreak {
    pub failures: u32,
    pub edge_only: bool,
}

/// Who is claiming: the tenant the claim speaks for and the device asking.
///
/// A pair rather than two parameters, because neither is meaningful alone —
/// a device id is only ever read against the organisation that registered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRef<'a> {
    pub org: OrgId,
    pub device: &'a str,
}

/// What a device's claim came to.
///
/// `Empty` and `HeldByAnotherDevice` are deliberately different answers. Under
/// the per-connection mutex a seller's second device loses at the index, and
/// returning it the same `None` an empty queue returns would have it hot-poll
/// a queue it can never win. Told the slot is taken, it backs off until the
/// holder settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceClaim {
    Leased(Box<LeasedItem>),
    Empty,
    HeldByAnotherDevice,
}

pub struct LeaseRepo {
    pool: PgPool,
}

impl LeaseRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The cross-tenant scan for the branch this process is allowed to run.
    ///
    /// Halts and the connection gate fail closed in the candidate filter.
    ///
    /// The transport predicate is what keeps the two branches off one queue:
    /// this scan sees `official_api` items only, so a no-API marketplace
    /// cannot be drained by a server process however the Rust side is wired.
    /// `claim_for_device` drains the other branch, and the test binding the
    /// column to `Marketplace::transport_class()` is what stops the two
    /// disagreeing.
    ///
    /// The live-lease mutex is the partial unique index from migration 0044,
    /// scoped to the connection, so a seller's second device contends only
    /// with a sibling working the same marketplace account.
    ///
    /// The lease expiry is computed by the database rather than by the
    /// claimant, because it is read back by the reaper against the database's
    /// own clock: a claimant running fast would otherwise wedge its tenant's
    /// queue past a TTL the server thinks it granted, and one running slow
    /// would have its claim stolen mid-submit.
    pub async fn acquire(
        &self,
        worker: &str,
        ttl_seconds: i64,
    ) -> Result<Option<LeasedItem>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let leased = sqlx::query!(
            r#"WITH candidate AS (
                 SELECT ji.org_id, ji.id
                 FROM job_item ji
                 JOIN job j ON j.org_id = ji.org_id AND j.id = ji.job_id
                 JOIN marketplace_inventory mi
                   ON mi.code = j.inventory AND mi.marketplace = j.marketplace
                 WHERE ji.state = 'queued'
                   AND mi.transport_class = 'official_api'
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
                           AND live.marketplace = ji.marketplace
                           AND live.state IN ('leased', 'running', 'verifying'))
                 ORDER BY ji.created_at, ji.id
                 LIMIT 1
                 FOR UPDATE OF ji SKIP LOCKED
               )
               UPDATE job_item AS item
               SET state = 'leased', lease_owner = $1,
                   lease_expires_at = now() + make_interval(secs => $2)
               FROM candidate, job j2
               WHERE item.org_id = candidate.org_id AND item.id = candidate.id
                 AND j2.org_id = item.org_id AND j2.id = item.job_id
               RETURNING item.org_id, item.id, item.job_id, item.mapping_id,
                 item.idempotency_key, item.lease_epoch, item.attempt_count,
                 item.operation, item.subject_kind, item.subject_url,
                 item.subject_numeric_id, item.state_from, item.state_to,
                 item.requires_bound_on,
                 j2.inventory AS "inventory!""#,
            worker,
            f64::from(i32::try_from(ttl_seconds).unwrap_or(i32::MAX)),
        )
        .fetch_optional(&mut *tx)
        .await;
        let leased = match map_unique(leased, "job_item_one_live_lease_per_connection", || {
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
                    operation: StoredOperation {
                        operation: row.operation,
                        subject_kind: row.subject_kind,
                        subject_url: row.subject_url,
                        subject_numeric_id: row.subject_numeric_id,
                        state_from: row.state_from,
                        state_to: row.state_to,
                    }
                    .decode()?,
                    lease_epoch: row.lease_epoch,
                    attempt_count: row.attempt_count,
                    requires_bound_on: row
                        .requires_bound_on
                        .as_deref()
                        .map(inventory_from_db)
                        .transpose()?,
                })
            })
            .transpose()
    }

    /// The seller's own device claiming its own tenant's work.
    ///
    /// Four things separate this from [`Self::acquire`], and each is a
    /// capability the caller must not hold. The statement is org-pinned, so
    /// forced row-level security fixes the tenant rather than trusting an
    /// argument. It sees `seller_device` items only, which is the other half
    /// of the transport split. It admits only a registered, unrevoked device.
    /// And it carries the entitlement predicate beside the halts, so a forged
    /// token still selects zero rows.
    ///
    /// Entitlement here is not "has paid". It is whether this organisation and
    /// this device may work a marketplace right now, which is what D10 and D11
    /// decide: a Free organisation is entitled within its quotas exactly as
    /// `tam-limits` says, and what blocks is a plan that lapsed and stayed
    /// lapsed past the grace. An organisation that never subscribed has no
    /// row and is never blocked by this; only one that had a paid plan and let
    /// it go stale is.
    pub async fn claim_for_device(
        &self,
        claimant: &DeviceRef<'_>,
        ttl_seconds: i64,
        grace_hours: i64,
        at: Timestamp,
    ) -> Result<DeviceClaim, StorageError> {
        let DeviceRef { org, device } = *claimant;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let leased = sqlx::query!(
            r#"WITH candidate AS (
                 SELECT ji.org_id, ji.id
                 FROM job_item ji
                 JOIN job j ON j.org_id = ji.org_id AND j.id = ji.job_id
                 JOIN marketplace_inventory mi
                   ON mi.code = j.inventory AND mi.marketplace = j.marketplace
                 WHERE ji.state = 'queued'
                   -- Defence in depth beside the row-level security this
                   -- statement is meant to run under. Forced RLS pins the
                   -- tenant for free under `tam_app`, but the pin is the only
                   -- tenancy this statement has, and a pool wired to a
                   -- BYPASSRLS role would silently claim another tenant's
                   -- work. Reading the pinned setting rather than an argument
                   -- keeps the tenant the server's, never the caller's.
                   -- The two-argument form answers NULL rather than raising
                   -- when the setting is absent, and a NULL comparison selects
                   -- nothing: an unpinned caller claims no work at all, which
                   -- is the fail-closed direction.
                   AND ji.org_id = nullif(current_setting('app.current_org', true), '')::uuid
                   AND mi.transport_class = 'seller_device'
                   AND EXISTS (SELECT 1 FROM device d
                         WHERE d.org_id = ji.org_id AND d.id = $1
                           AND d.revoked_at IS NULL)
                   AND NOT EXISTS (SELECT 1 FROM billing_subscription bs
                         WHERE bs.org_id = ji.org_id
                           AND bs.status NOT IN ('active', 'trialing')
                           AND bs.current_period_end IS NOT NULL
                           AND bs.current_period_end < now() - make_interval(hours => $3))
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
                           AND live.marketplace = ji.marketplace
                           AND live.state IN ('leased', 'running', 'verifying'))
                 ORDER BY ji.created_at, ji.id
                 LIMIT 1
                 FOR UPDATE OF ji SKIP LOCKED
               )
               UPDATE job_item AS item
               SET state = 'leased', lease_owner = $1,
                   lease_expires_at = now() + make_interval(secs => $2)
               FROM candidate, job j2
               WHERE item.org_id = candidate.org_id AND item.id = candidate.id
                 AND j2.org_id = item.org_id AND j2.id = item.job_id
               RETURNING item.org_id, item.id, item.job_id, item.mapping_id,
                 item.idempotency_key, item.lease_epoch, item.attempt_count,
                 item.operation, item.subject_kind, item.subject_url,
                 item.subject_numeric_id, item.state_from, item.state_to,
                 item.requires_bound_on,
                 j2.inventory AS "inventory!""#,
            device,
            f64::from(i32::try_from(ttl_seconds).unwrap_or(i32::MAX)),
            i32::try_from(grace_hours).unwrap_or(i32::MAX),
        )
        .fetch_optional(&mut *tx)
        .await;
        let leased = match map_unique(leased, "job_item_one_live_lease_per_connection", || {
            StorageError::StaleLease
        }) {
            Ok(row) => row,
            // The mutex index fired between the candidate read and the write:
            // a sibling device took the slot, which is the same answer the
            // read below gives and never an empty queue.
            Err(StorageError::StaleLease) => return Ok(DeviceClaim::HeldByAnotherDevice),
            Err(error) => return Err(error),
        };
        let Some(row) = leased else {
            // Nothing claimable. Whether that is an empty queue or a sibling
            // device holding the slot is the difference between polling again
            // soon and backing off, so it is read rather than assumed.
            let held: Option<String> = sqlx::query_scalar!(
                "SELECT lease_owner FROM job_item \
                 WHERE org_id = $1 AND state IN ('leased', 'running', 'verifying') \
                   AND lease_owner IS DISTINCT FROM $2 LIMIT 1",
                uuid_to_db(org.0),
                device,
            )
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
            tx.commit().await?;
            return Ok(if held.is_some() {
                DeviceClaim::HeldByAnotherDevice
            } else {
                DeviceClaim::Empty
            });
        };
        let item = JobItemId(uuid_from_db(row.id));
        let job = JobId(uuid_from_db(row.job_id));
        // In the claim's own transaction, so progress names the device doing
        // the work the moment the work is its to do.
        append_event(
            &mut tx,
            &EventScope {
                org,
                job,
                item: Some(item),
            },
            &JobEventPayload::ItemLeased {
                worker: device.to_owned(),
                lease_epoch: row.lease_epoch,
            },
            Stamp::system(SystemComponent::Device, at),
        )
        .await?;
        tx.commit().await?;
        Ok(DeviceClaim::Leased(Box::new(LeasedItem {
            org: OrgId(uuid_from_db(row.org_id)),
            item,
            job,
            mapping: MappingId(uuid_from_db(row.mapping_id)),
            inventory: inventory_from_db(&row.inventory)?,
            idempotency_key: IdempotencyKey(uuid_from_db(row.idempotency_key)),
            operation: StoredOperation {
                operation: row.operation,
                subject_kind: row.subject_kind,
                subject_url: row.subject_url,
                subject_numeric_id: row.subject_numeric_id,
                state_from: row.state_from,
                state_to: row.state_to,
            }
            .decode()?,
            lease_epoch: row.lease_epoch,
            attempt_count: row.attempt_count,
            requires_bound_on: row
                .requires_bound_on
                .as_deref()
                .map(inventory_from_db)
                .transpose()?,
        })))
    }

    /// Every fenced write shares this shape: the epoch must still match, and
    /// a zero-row update is the stale worker finding out, not racing.
    ///
    /// The job-level settle hangs off this write rather than off the caller's
    /// control flow, so `JobSettled` is a consequence of the last item
    /// settling however it settled.
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
        let mut tx = self.pool.begin().await?;
        let settled = sqlx::query!(
            "UPDATE job_item \
             SET state = 'settled', outcome = $4, failure_code = $5, failure_detail = $6, \
                 settled_at = $7, lease_owner = NULL, lease_expires_at = NULL \
             WHERE org_id = $1 AND id = $2 AND lease_epoch = $3 \
               AND state IN ('leased', 'running', 'verifying') \
             RETURNING job_id",
            uuid_to_db(org.0),
            uuid_to_db(item.0),
            lease_epoch,
            item_outcome_to_db(*outcome),
            failure_code.map(failure_code_to_db),
            failure_detail.as_ref().map(|detail| detail.0.as_str()),
            timestamp_to_db(at)?,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = settled else {
            return Err(StorageError::StaleLease);
        };
        settle_if_complete(&mut tx, org, JobId(uuid_from_db(row.job_id)), at).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Parks the item for a duration the caller states, resolved against the
    /// database's clock for the same reason the lease expiry is: the reaper
    /// reads `park_expires_at` against `now()`, so an instant minted anywhere
    /// else is two clocks being compared.
    pub async fn park(
        &self,
        lease: &LeaseRef,
        blocked_on: &str,
        park_for_seconds: i64,
    ) -> Result<(), StorageError> {
        let LeaseRef {
            org,
            item,
            lease_epoch,
        } = *lease;
        let updated = sqlx::query!(
            "UPDATE job_item \
             SET state = 'parked_live', blocked_on = $4, \
                 park_expires_at = now() + make_interval(secs => $5), \
                 lease_owner = NULL, lease_expires_at = NULL \
             WHERE org_id = $1 AND id = $2 AND lease_epoch = $3 \
               AND state IN ('leased', 'running', 'verifying')",
            uuid_to_db(org.0),
            uuid_to_db(item.0),
            lease_epoch,
            blocked_on,
            f64::from(i32::try_from(park_for_seconds).unwrap_or(i32::MAX)),
        )
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(StorageError::StaleLease);
        }
        Ok(())
    }

    /// Advances this item's consecutive-preflight-failure streak and answers
    /// what it now stands at.
    ///
    /// Separate from `attempt_count`, which counts leases rather than
    /// preflights and which only the reapers advance: the caller's bound is
    /// on failures *in a row*, so it needs a counter a healthy preflight can
    /// return to zero. Fenced on the epoch like every other lease write, so a
    /// stolen item's former holder cannot move a counter its new holder is
    /// also moving.
    pub async fn preflight_failed(
        &self,
        lease: &LeaseRef,
        edge_class: bool,
    ) -> Result<PreflightStreak, StorageError> {
        let LeaseRef {
            org,
            item,
            lease_epoch,
        } = *lease;
        let row = sqlx::query!(
            r#"UPDATE job_item
               SET preflight_failures = preflight_failures + 1,
                   preflight_challenge_only = preflight_challenge_only AND $4
               WHERE org_id = $1 AND id = $2 AND lease_epoch = $3
                 AND state IN ('leased', 'running', 'verifying')
               RETURNING preflight_failures AS "failures!",
                 preflight_challenge_only AS "edge_only!""#,
            uuid_to_db(org.0),
            uuid_to_db(item.0),
            lease_epoch,
            edge_class,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::StaleLease)?;
        Ok(PreflightStreak {
            failures: count_u32(i64::from(row.failures))?,
            edge_only: row.edge_only,
        })
    }

    /// Returns the streak to zero, so the bound the caller holds is on
    /// consecutive failures rather than on a lifetime tally.
    ///
    /// A zero row count is either a row already at zero or a lease the
    /// stealer took, and neither is something this caller can act on: the
    /// writes that follow in the same run are fenced on the same epoch and
    /// report the steal themselves.
    pub async fn preflight_succeeded(&self, lease: &LeaseRef) -> Result<(), StorageError> {
        let LeaseRef {
            org,
            item,
            lease_epoch,
        } = *lease;
        sqlx::query!(
            "UPDATE job_item \
             SET preflight_failures = 0, preflight_challenge_only = true \
             WHERE org_id = $1 AND id = $2 AND lease_epoch = $3 \
               AND state IN ('leased', 'running', 'verifying') \
               AND (preflight_failures <> 0 OR NOT preflight_challenge_only)",
            uuid_to_db(org.0),
            uuid_to_db(item.0),
            lease_epoch,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Requeues expired leases with the epoch bumped so the previous holder's
    /// writes are fenced out, and settles items that exhausted their attempt
    /// budget as failed rather than requeueing them forever.
    ///
    /// The failed branch is the commonest way a bulk job ends and it runs
    /// here, cross-tenant, with no job context at all, so it carries the
    /// job-level settle with it: otherwise a job whose last item dies of
    /// attempt exhaustion flips to `Settled` with no event explaining it.
    pub async fn expire_and_steal(
        &self,
        now: Timestamp,
        attempts_max: i32,
    ) -> Result<u64, StorageError> {
        let now_db = timestamp_to_db(now)?;
        let mut tx = self.pool.begin().await?;
        let failed = sqlx::query!(
            "UPDATE job_item \
             SET state = 'settled', outcome = 'failed', failure_code = 'Other', \
                 settled_at = $1, lease_owner = NULL, lease_expires_at = NULL, \
                 lease_epoch = lease_epoch + 1 \
             WHERE state IN ('leased', 'running', 'verifying') \
               AND lease_expires_at <= now() AND attempt_count + 1 >= $2 \
             RETURNING org_id, job_id",
            now_db,
            attempts_max,
        )
        .fetch_all(&mut *tx)
        .await?;
        let stolen = sqlx::query!(
            "UPDATE job_item \
             SET state = 'queued', lease_owner = NULL, lease_expires_at = NULL, \
                 lease_epoch = lease_epoch + 1, attempt_count = attempt_count + 1 \
             WHERE state IN ('leased', 'running', 'verifying') \
               AND lease_expires_at <= now()",
        )
        .execute(&mut *tx)
        .await?;

        let mut seen: Vec<(OrgId, JobId)> = Vec::new();
        for row in &failed {
            let pair = (
                OrgId(uuid_from_db(row.org_id)),
                JobId(uuid_from_db(row.job_id)),
            );
            if !seen.contains(&pair) {
                seen.push(pair);
            }
        }
        for (org, job) in seen {
            settle_if_complete(&mut tx, org, job, now).await?;
        }
        let expired = u64::try_from(failed.len()).unwrap_or(u64::MAX);
        tx.commit().await?;
        Ok(expired.saturating_add(stolen.rows_affected()))
    }

    /// Requeues items whose park has expired, and only those parked on a gate
    /// a drained queue can actually clear, plus items parked on
    /// [`REAUTH_REQUIRED`] whose connection has since been re-linked.
    ///
    /// The two are separate arms because their releasing event differs: the
    /// first is the clock, the second is the seller re-linking, and a gate the
    /// clock cannot clear must never be revived by it. The answer is a count
    /// of everything requeued by either.
    ///
    /// Cross-tenant and unpinned on purpose: this runs under `tam_engine`,
    /// which is BYPASSRLS, so `job_item`'s FORCE ROW LEVEL SECURITY does not
    /// apply. The lease epoch is deliberately not bumped — `park` and
    /// `acquire` both leave it alone, the parking worker has already
    /// returned, and every fenced write additionally requires a live lease
    /// state.
    ///
    /// `attempts_max` is the caller's, because the budget is a limit the
    /// worker owns and this crate holds no limits of its own.
    pub async fn revive_expired(
        &self,
        now: Timestamp,
        attempts_max: i32,
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        // The give-up arm, first, mirroring `expire_and_steal`'s two-statement
        // shape. A park cycling forever is invisible to every existing reaper
        // -- `expire_and_steal` filters on the live states a parked row does
        // not have, and the wall-clock bound is per run inside the driver --
        // so without this an item whose gate never clears re-parks every day
        // and the job it belongs to reads active forever.
        let exhausted = sqlx::query!(
            "UPDATE job_item \
             SET state = 'settled', outcome = 'skipped', failure_code = $4, \
                 failure_detail = 'the gate ' || COALESCE(blocked_on, 'unknown') \
                     || ' did not clear within the attempt budget', \
                 blocked_on = NULL, park_expires_at = NULL, settled_at = $1 \
             WHERE state = 'parked_live' AND park_expires_at <= now() \
               AND blocked_on = ANY($2) \
               AND attempt_count + 1 >= $3 \
             RETURNING org_id, job_id, id",
            timestamp_to_db(now)?,
            &REVIVABLE_GATES.map(str::to_owned)[..],
            attempts_max,
            // Bound through the encoder rather than spelled here. `job_item`
            // carries no CHECK on the column, so a literal that drifts from
            // the codec is written happily and then fails to decode forever
            // -- and `items_page` collects into one Result, so a single such
            // row makes every page of that job's items a fault.
            failure_code_to_db(FailureCode::Other),
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut settled: Vec<(uuid::Uuid, uuid::Uuid)> = Vec::new();
        for row in &exhausted {
            let pair = (row.org_id, row.job_id);
            if !settled.contains(&pair) {
                settled.push(pair);
            }
        }
        let rows = sqlx::query!(
            "UPDATE job_item \
             SET state = 'queued', blocked_on = NULL, park_expires_at = NULL, \
                 attempt_count = attempt_count + 1 \
             WHERE state = 'parked_live' AND park_expires_at <= now() \
               AND blocked_on = ANY($1) \
             RETURNING org_id, job_id, id",
            &REVIVABLE_GATES.map(str::to_owned)[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        let revived = revived(rows.into_iter().map(|row| (row.org_id, row.job_id, row.id)));
        record_resumptions(&mut tx, &revived, now).await?;

        // The re-link arm, conditioned on the connection rather than on the
        // clock. `park_expires_at` is deliberately not read here: a re-link is
        // the event that clears this gate, and until it happens no amount of
        // waiting should.
        //
        // The stranded attempt is released first, and that ordering is the
        // whole of why this arm is two statements. `SyncMachine::park`
        // advances with the write attempt still `in_flight`, and nothing on
        // the parked path settles it -- the driver returns `Parked` and the
        // worker only logs it. A revive that left the row standing would meet
        // `write_attempt_one_in_flight` at `AttemptRepo::open`, abandon, and
        // repeat that on every pass until the attempt budget settled the item
        // `failed`/`Other` with nothing in the ledger: the exact failure
        // `REVIVABLE_GATES` is written to avoid.
        //
        // `create` is excluded and stays parked. Settling its attempt would
        // release the only fence there is against a second listing on the
        // seller's store -- the engine's `may_settle_unverified` holds a
        // create's attempt standing for that reason, and neither adapter
        // offers an idempotent create. A submit that died on `SessionExpired`
        // may or may not have landed, so the resolution a create needs is a
        // read-back that settles on what is actually there; that read is not
        // available to this scan, and building it is the intended next step
        // rather than an omission.
        // Both writes hang off one statement, and that is the whole reason
        // this is a CTE rather than the two statements it reads as. Postgres
        // runs every arm of a data-modifying `WITH` against a single snapshot,
        // so the connection predicate is evaluated once for both. Split into
        // two statements under READ COMMITTED each takes its own snapshot, and
        // a re-link committing between them revives an item whose attempt the
        // first statement had already declined to settle -- putting back the
        // `AttemptInFlight` deadlock this arm exists to prevent.
        //
        // `eligible` re-states `parked_live` and `in_flight` on the updates
        // themselves as well: the snapshot settles which rows are in scope,
        // and those predicates keep a concurrent pass that already revived a
        // row from advancing its `attempt_count` a second time.
        let at = timestamp_to_db(now)?;
        let relinked = sqlx::query!(
            "WITH eligible AS ( \
                 SELECT ji.org_id, ji.id, ji.subject_kind, ji.subject_url, \
                        ji.subject_numeric_id \
                 FROM job_item ji \
                 JOIN job j ON j.org_id = ji.org_id AND j.id = ji.job_id \
                 WHERE ji.state = 'parked_live' \
                   AND ji.blocked_on = $2 \
                   AND ji.operation <> 'create' \
                   AND EXISTS (SELECT 1 FROM connection c \
                         WHERE c.org_id = ji.org_id \
                           AND c.marketplace = j.marketplace \
                           AND c.state = 'linked') \
             ), released AS ( \
                 UPDATE write_attempt wa \
                 SET state = 'abandoned', settled_at = $1, \
                     remote_id_kind = e.subject_kind, \
                     remote_url = e.subject_url, \
                     remote_numeric_id = e.subject_numeric_id \
                 FROM eligible e \
                 WHERE wa.org_id = e.org_id AND wa.job_item_id = e.id \
                   AND wa.state = 'in_flight' \
                 RETURNING wa.id \
             ) \
             UPDATE job_item ji \
             SET state = 'queued', blocked_on = NULL, park_expires_at = NULL, \
                 attempt_count = attempt_count + 1 \
             FROM eligible e \
             WHERE ji.org_id = e.org_id AND ji.id = e.id \
               AND ji.state = 'parked_live' \
             RETURNING ji.org_id, ji.job_id, ji.id",
            at,
            REAUTH_REQUIRED,
        )
        .fetch_all(&mut *tx)
        .await?;
        let relinked = self::revived(
            relinked
                .into_iter()
                .map(|row| (row.org_id, row.job_id, row.id)),
        );
        record_resumptions(&mut tx, &relinked, now).await?;

        for (org, job) in settled {
            settle_if_complete(
                &mut tx,
                OrgId(uuid_from_db(org)),
                JobId(uuid_from_db(job)),
                now,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(count_of(&revived).saturating_add(count_of(&relinked)))
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
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        // RETURNING rather than a separate read: the audit must record the
        // connection this statement actually gated, and a row that was
        // already gated matches nothing and is not recorded twice.
        let gated = sqlx::query!(
            "UPDATE connection SET state = 'needs_reauth', updated_at = now() \
             WHERE org_id = $1 AND marketplace = $2 AND state = 'linked' \
             RETURNING id",
            uuid_to_db(org.0),
            marketplace_to_db(inventory.marketplace()),
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = gated {
            crate::connections::record_connection_event(
                &mut tx,
                &crate::connections::ConnectionEventRecord {
                    org,
                    connection: tam_types::ConnectionId(uuid_from_db(row.id)),
                    event: tam_types::ConnectionEvent::NeedsReauth,
                    // The inventory, not the marketplace: the connection is
                    // per-marketplace, so which of its inventories failed is
                    // the part the row does not already carry.
                    detail: Some(inventory_to_db(inventory)),
                    // The gate is the engine classifying a failure as an
                    // authentication problem; no seller asked for it.
                    stamp: Stamp::system(SystemComponent::Engine, at),
                },
            )
            .await?;
        }
        tx.commit().await?;
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

/// What a settling attempt did to the mapping it names.
///
/// A create and a revise land; a removal severs; an attempt that addressed a
/// listing and changed nothing still says which listing that was. Splitting
/// these is what stops a committed removal binding the mapping to a listing
/// that no longer exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandingEffect {
    /// The verdict named no listing; there is nothing to record.
    None,
    /// The attempt addressed this listing and changed no binding -- a revise
    /// or a removal that did not take, or one whose fate is unsettled. The
    /// attempt row records what it addressed; the mapping is untouched.
    /// Without it a failed removal would not say what it failed to remove.
    Addressed { id: RemoteListingId },
    /// The write landed here: bind the mapping to it, and record the
    /// lifecycle the verification read actually observed rather than the one
    /// the adapter's create convention would imply.
    Landed {
        id: RemoteListingId,
        lifecycle: RemoteLifecycle,
    },
    /// The write removed it: sever the binding, releasing the bound claim
    /// both of migration 0018's partial unique indexes hold.
    Severed { id: RemoteListingId },
}

impl LandingEffect {
    /// The listing the attempt addressed, whatever it did to it. Every
    /// variant but `None` writes the `write_attempt` remote-id columns, so
    /// the settled row always names what the write was about.
    const fn addressed(&self) -> Option<&RemoteListingId> {
        match self {
            Self::None => Option::None,
            Self::Addressed { id } | Self::Landed { id, .. } | Self::Severed { id } => Some(id),
        }
    }
}

/// How an attempt settled in the ledger's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptVerdict {
    pub state: String,
    pub failure_code: Option<FailureCode>,
    /// What the write did to the listing it addressed. Without it the ledger
    /// cannot say what it created, so reconciliation, verification and dedup
    /// have nothing to address.
    pub landing: LandingEffect,
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
    /// The attempt id is the caller's rather than minted here, and the write
    /// is idempotent on it: a response lost in flight is recovered by
    /// re-offering the same id, which answers `Ok` against the row already
    /// standing instead of spending another of the item's attempts.
    ///
    /// `ON CONFLICT (org_id, id) DO NOTHING` is what makes the replay safe,
    /// and the `RETURNING`-less row count is what tells the two cases apart
    /// from the fence firing: a different id arriving while one is in flight
    /// still violates `write_attempt_one_in_flight` and still answers
    /// `AttemptInFlight`, because that is a second create rather than a
    /// retry of the first.
    pub async fn open(
        &self,
        lease: &LeaseRef,
        attempt: tam_types::Uuid,
        new: &NewAttempt<'_>,
    ) -> Result<(), StorageError> {
        let NewAttempt {
            mapping,
            intent,
            stamp,
        } = *new;
        let Stamp { at, actor } = stamp;
        let AttemptIntent { body, hash } = intent;
        let inserted = sqlx::query!(
            "INSERT INTO write_attempt              (org_id, id, job_item_id, mapping_id, lease_epoch, intent,               intent_hash, state, opened_at, actor_kind, actor_id)              VALUES ($1, $2, $3, $4, $5, $6, $7, 'in_flight', $8, $9, $10)              ON CONFLICT (org_id, id) DO NOTHING",
            uuid_to_db(lease.org.0),
            uuid_to_db(attempt),
            uuid_to_db(lease.item.0),
            uuid_to_db(mapping.0),
            lease.lease_epoch,
            body,
            hash.as_slice(),
            timestamp_to_db(at)?,
            actor.kind(),
            actor.id(),
        )
        .execute(&self.pool)
        .await;
        map_unique(inserted, "write_attempt_one_in_flight", || {
            StorageError::AttemptInFlight
        })?;
        Ok(())
    }

    /// Epoch-fenced settlement of the attempt row, and — when the verdict
    /// carries a landing or a sever — the mapping write, in one transaction.
    /// A separate call would leave a crash window where the attempt says
    /// committed while the mapping stays unbound, or stays bound to a listing
    /// the removal took down.
    ///
    /// What authorises the sever is `write_attempt_one_in_flight`, not
    /// `write_attempt.lease_epoch`: the epoch is written at open from the
    /// run's own `LeaseRef` and compared here against the same one, so it can
    /// never mismatch within a run, and `expire_and_steal` bumps
    /// `job_item.lease_epoch` rather than the attempt's. The clause that
    /// bites is `state = 'in_flight'`. The sever's remote-id predicate is
    /// defence in depth against fixture-level writes and a future concurrent
    /// binder, not a live guard: while a removal is in flight no second
    /// attempt on the mapping can open, so nothing can rebind it underneath.
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
            landing,
        } = verdict;
        let remote = landing
            .addressed()
            .map(RemoteIdColumns::encode)
            .transpose()?;
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
        let mapping_db = uuid_to_db(mapping.0);
        // The savepoint guards the identity clash and nothing else: every
        // other database error still aborts the whole settle. A sever cannot
        // raise one, because releasing a claim contends with no index.
        let disposition = match landing {
            LandingEffect::None => BindDisposition::NotLanded,
            LandingEffect::Addressed { .. } => BindDisposition::Addressed,
            LandingEffect::Landed { id, lifecycle } => {
                let remote = RemoteIdColumns::encode(id)?;
                let lifecycle = LifecycleColumns::encode(lifecycle)?;
                sqlx::query("SAVEPOINT bind").execute(&mut *tx).await?;
                let bound = sqlx::query!(
                    "UPDATE mapping \
                     SET binding_state = 'bound', \
                         remote_id_kind = $3, remote_url = $4, remote_numeric_id = $5, \
                         first_seen_at = $6, \
                         verify_state = 'stale', verified_at = NULL, verify_stale_since = $6, \
                         binding_attempt = NULL, binding_marker = NULL, \
                         ambiguous_since = NULL, severed_at = NULL, sever_cause = NULL, \
                         lifecycle_state = $7, lifecycle_since = $8, lifecycle_reason = $9, \
                         updated_at = $6 \
                     WHERE org_id = $1 AND id = $2 \
                       AND binding_state IN ('unbound', 'creating', 'severed')",
                    org_db,
                    mapping_db,
                    remote.kind,
                    remote.url,
                    remote.numeric_id,
                    at_db,
                    lifecycle.state,
                    lifecycle.since,
                    lifecycle.reason,
                )
                .execute(&mut *tx)
                .await;
                match bound {
                    Ok(bound) if bound.rows_affected() > 0 => BindDisposition::Bound,
                    Ok(_) => {
                        let classified = classify_bind(&mut tx, org_db, mapping_db, id).await?;
                        if classified == BindDisposition::AlreadyBound {
                            relanded(
                                &mut tx,
                                MappingWrite {
                                    org: org_db,
                                    mapping: mapping_db,
                                    at: at_db,
                                },
                                &remote,
                                &lifecycle,
                            )
                            .await?;
                        }
                        classified
                    }
                    Err(clash) if is_bound_identity_clash(&clash) => {
                        sqlx::query("ROLLBACK TO SAVEPOINT bind")
                            .execute(&mut *tx)
                            .await?;
                        claimed_elsewhere(&mut tx, org_db, mapping_db, &remote).await?
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            LandingEffect::Severed { id } => {
                let remote = RemoteIdColumns::encode(id)?;
                let lifecycle = LifecycleColumns::encode(&RemoteLifecycle::Absent)?;
                let severed = sqlx::query!(
                    "UPDATE mapping \
                     SET binding_state = 'severed', \
                         sever_generation = sever_generation + 1, \
                         severed_at = $6, sever_cause = 'removed_by_seller', \
                         verify_state = 'stale', verified_at = NULL, \
                         verify_stale_since = NULL, \
                         lifecycle_state = $7, lifecycle_since = $8, lifecycle_reason = $9, \
                         updated_at = $6 \
                     WHERE org_id = $1 AND id = $2 \
                       AND binding_state = 'bound' \
                       AND remote_id_kind = $3 \
                       AND remote_url IS NOT DISTINCT FROM $4 \
                       AND remote_numeric_id IS NOT DISTINCT FROM $5",
                    org_db,
                    mapping_db,
                    remote.kind,
                    remote.url,
                    remote.numeric_id,
                    at_db,
                    lifecycle.state,
                    lifecycle.since,
                    lifecycle.reason,
                )
                .execute(&mut *tx)
                .await?;
                if severed.rows_affected() > 0 {
                    BindDisposition::Severed
                } else {
                    classify_sever(&mut tx, org_db, mapping_db, id).await?
                }
            }
        };
        // The gate's answer arrives here or a day later. An item waiting on
        // this product's binding is queued by the binding itself, in the
        // transaction that wrote it, so the park expiry stays the backstop it
        // was meant to be rather than the only way out.
        if matches!(
            disposition,
            BindDisposition::Bound | BindDisposition::AlreadyBound
        ) {
            revive_counterparts(&mut tx, lease.org, mapping, at).await?;
        }
        tx.commit().await?;
        Ok(disposition)
    }
}

/// The three columns every mapping write inside a settle is keyed and
/// stamped by. A struct because the writer below would otherwise run past
/// `too-many-arguments-threshold`, and because they always travel together.
#[derive(Clone, Copy)]
struct MappingWrite {
    org: uuid::Uuid,
    mapping: uuid::Uuid,
    at: DateTime<Utc>,
}

/// A landing on a mapping already bound to the very listing it landed on.
///
/// The bind's own fence excludes `'bound'` on purpose — re-landing must not
/// overwrite `first_seen_at` or reclaim an identity — but the lifecycle the
/// verification read observed is new information every time, and it is the
/// only record of which side of the draft line the listing now sits on.
/// Without this write a committed publish left the mapping reading
/// `'draft'`, and `admission`'s `lifecycle_diverged` gate then parked every
/// later item on that mapping permanently.
///
/// Fenced on the binding it is updating, so a mapping rebound underneath the
/// settle keeps the divergent disposition `classify_bind` reported rather
/// than having a foreign listing's lifecycle written over it.
async fn relanded(
    tx: &mut Transaction<'_, Postgres>,
    target: MappingWrite,
    remote: &RemoteIdColumns<'_>,
    lifecycle: &LifecycleColumns,
) -> Result<(), StorageError> {
    sqlx::query!(
        "UPDATE mapping \
         SET lifecycle_state = $4, lifecycle_since = $5, lifecycle_reason = $6, \
             updated_at = $7 \
         WHERE org_id = $1 AND id = $2 \
           AND binding_state = 'bound' \
           AND remote_id_kind = $3 \
           AND remote_url IS NOT DISTINCT FROM $8 \
           AND remote_numeric_id IS NOT DISTINCT FROM $9",
        target.org,
        target.mapping,
        remote.kind,
        lifecycle.state,
        lifecycle.since,
        lifecycle.reason,
        target.at,
        remote.url,
        remote.numeric_id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
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
///
/// `#[must_use]` because the three anomalous dispositions are the only record
/// that a landed listing went unbound: dropping one loses the listing, since
/// the item outcome and the run verdict are deliberately unchanged by it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum BindDisposition {
    /// The verdict carried no landed listing, so there was nothing to bind.
    NotLanded,
    /// The attempt named the listing it addressed and asked for no change to
    /// the binding, so the `write_attempt` row records it and the mapping is
    /// untouched. A failed removal settles here, saying what it failed to
    /// remove.
    Addressed,
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
    /// The removal took: the binding is released and both partial unique
    /// indexes stop holding the claim, so the mapping can be re-created
    /// through the same row.
    Severed,
    /// The sever's fence matched no row because the mapping was not bound.
    SeverRefused {
        state: String,
    },
    /// The mapping is bound, to a different listing than the one this
    /// removal took down. A separate variant rather than `Refused`, whose
    /// documented meaning is that the fence matched no row *in this binding
    /// state*: a rebound sever fails on the remote-id predicate while
    /// `binding_state` is still `'bound'`, so `Refused { binding_state:
    /// "bound" }` would be a self-contradiction an operator cannot act on.
    SeverDiverged {
        existing: RemoteListingId,
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

/// Why the fenced sever matched no row, read inside the settling
/// transaction. Two answers only: the mapping is not bound, or it is bound to
/// a listing other than the one this removal took down.
async fn classify_sever(
    tx: &mut Transaction<'_, Postgres>,
    org: uuid::Uuid,
    mapping: uuid::Uuid,
    severed: &RemoteListingId,
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
        return Ok(BindDisposition::SeverRefused {
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
    if existing == *severed {
        return Err(StorageError::Inconsistent {
            reason: "the sever matched no row against the identity the mapping holds".to_owned(),
        });
    }
    Ok(BindDisposition::SeverDiverged { existing })
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
        let NewJob {
            job,
            inventory,
            stamp,
        } = *new;
        let Stamp { at, actor } = stamp;
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        let inserted = sqlx::query!(
            "INSERT INTO job \
             (org_id, id, inventory, marketplace, created_at, \
              request_idempotency_key, actor_kind, actor_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            org_db,
            uuid_to_db(job.0),
            inventory_to_db(inventory),
            marketplace_to_db(inventory.marketplace()),
            at_db,
            uuid_to_db(request_key),
            actor.kind(),
            actor.id(),
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
            insert_job_item(&mut tx, org_db, uuid_to_db(job.0), item, at_db).await?;
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
            stamp,
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

#[cfg(test)]
mod tests {
    use super::{REAUTH_REQUIRED, REVIVABLE_GATES};
    use tam_marketplace::ChallengeKind;

    /// The driver parks a challenge under `format!("{challenge:?}")`, so this
    /// constant is bound to a `Debug` derivation rather than to a codec. A
    /// rename of the variant would otherwise leave the revive reading a gate
    /// nothing writes, and the park would go back to being unreachable.
    #[test]
    fn the_reauth_gate_matches_the_debug_form_the_driver_writes() {
        assert_eq!(
            format!("{:?}", ChallengeKind::ReauthRequired),
            REAUTH_REQUIRED
        );
    }

    /// The re-link arm and the expiry arm must stay disjoint: `REVIVABLE_GATES`
    /// is time-gated, so a `REAUTH_REQUIRED` entry there would requeue the item
    /// on the clock while the connection was still unusable.
    #[test]
    fn the_reauth_gate_is_not_time_revivable() {
        assert!(!REVIVABLE_GATES.contains(&REAUTH_REQUIRED));
    }
}
