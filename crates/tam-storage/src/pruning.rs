//! Job-event retention: erase what has aged past the window and advance the
//! per-organisation watermark past it, so a client resuming below the
//! watermark receives a resync rather than a partial replay of a stream with
//! a hole in it.
//!
//! Cross-tenant by construction, like the lease scan: one pass covers every
//! organisation at once, so it runs on a `tam_engine` pool (BYPASSRLS, with
//! the DELETE privilege migration 0015 grants) and sees nothing at all under
//! `tam_app`'s forced row-level security.

use sqlx::PgPool;
use tam_types::Timestamp;

use crate::codec::timestamp_to_db;
use crate::StorageError;

/// What one pass erased, and how many organisations it moved forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PruneReport {
    pub deleted: u64,
    pub watermark_advances: u64,
}

pub struct PruneRepo {
    pool: PgPool,
}

impl PruneRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Deletes up to `batch` events created before `cutoff`, oldest first,
    /// then advances every affected organisation's watermark to the greatest
    /// sequence number it lost.
    ///
    /// One statement, so the delete and the advance cannot be observed apart:
    /// a watermark left behind its erased events is a client replaying a gap
    /// it believes is whole.
    pub async fn prune_pass(
        &self,
        cutoff: Timestamp,
        batch: i64,
    ) -> Result<PruneReport, StorageError> {
        let counted = sqlx::query!(
            r#"WITH doomed AS (
                   SELECT id FROM job_event
                    WHERE created_at < $1
                    ORDER BY created_at, id
                    LIMIT $2
               ),
               gone AS (
                   DELETE FROM job_event WHERE id IN (SELECT id FROM doomed)
                   RETURNING org_id, org_seq
               ),
               highest AS (
                   SELECT org_id, max(org_seq) AS max_seq FROM gone GROUP BY org_id
               ),
               advanced AS (
                   UPDATE org_event_counter c
                      SET prune_watermark = GREATEST(c.prune_watermark, h.max_seq)
                     FROM highest h
                    WHERE c.org_id = h.org_id AND h.max_seq > c.prune_watermark
                   RETURNING c.org_id
               )
               SELECT (SELECT count(*) FROM gone)     AS "deleted!",
                      (SELECT count(*) FROM advanced) AS "advances!""#,
            timestamp_to_db(cutoff)?,
            batch,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(PruneReport {
            deleted: u64::try_from(counted.deleted).unwrap_or(0),
            watermark_advances: u64::try_from(counted.advances).unwrap_or(0),
        })
    }

    /// Deletes up to `batch` metric snapshots observed before `cutoff`, oldest
    /// first, and answers how many were erased.
    ///
    /// No watermark travels with it, unlike the pass above: a snapshot stream
    /// has no resume position a client could fall behind, so erasing the tail
    /// of it costs a console the oldest point on a chart and nothing else.
    ///
    /// The batch is picked before it is erased so one pass cannot lock the
    /// whole table, and the ordering makes repeated passes converge from the
    /// oldest end rather than sampling the window at random.
    pub async fn prune_snapshots(
        &self,
        cutoff: Timestamp,
        batch: i64,
    ) -> Result<u64, StorageError> {
        let erased = sqlx::query!(
            r#"WITH doomed AS (
                   SELECT org_id, mapping_id, metric, observed_at
                     FROM listing_metric_snapshot
                    WHERE observed_at < $1
                    ORDER BY observed_at, org_id, mapping_id, metric
                    LIMIT $2
               )
               DELETE FROM listing_metric_snapshot s
                USING doomed d
                WHERE s.org_id = d.org_id
                  AND s.mapping_id = d.mapping_id
                  AND s.metric = d.metric
                  AND s.observed_at = d.observed_at"#,
            timestamp_to_db(cutoff)?,
            batch,
        )
        .execute(&self.pool)
        .await?;
        Ok(erased.rows_affected())
    }
}
