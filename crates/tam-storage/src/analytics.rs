//! Per-listing marketplace analytics: the capture pass's insert and the
//! newest-per-metric read a console renders.
//!
//! The table is append-only. The all-time read a capture takes is a running
//! total, so a rate over any period is the difference between two snapshots;
//! a row that overwrote its predecessor would destroy the only way to compute
//! one. Nothing here updates, and the one erasure — retention — lives in
//! [`crate::pruning`] with the other cross-tenant pass.

use sqlx::PgPool;
use tam_types::{InventoryId, MappingId, OrgId, Timestamp};

use crate::codec::{
    inventory_from_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::{pin_org, StorageError};

/// One metric total for one listing at one instant, as a capture writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricSnapshot {
    pub mapping: MappingId,
    /// The stored metric name. Open rather than closed: the metrics captured
    /// today are a shortlist off the twelve the adapter names, and widening it
    /// must not need a migration or a redeploy of the read side.
    pub metric: String,
    pub observed_at: Timestamp,
    /// The number as the adapter read it — a count for sales, an amount for
    /// earnings — kept untyped for the reason the adapter keeps it that way.
    pub total_value: f64,
}

/// The newest snapshot of one metric for one listing, carrying the inventory
/// its mapping names rather than one of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestMetric {
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub metric: String,
    pub observed_at: Timestamp,
    pub total_value: f64,
}

pub struct AnalyticsRepo {
    pool: PgPool,
}

impl AnalyticsRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records one pass's snapshots, answering how many rows were new.
    ///
    /// A conflict on the primary key is ignored rather than raised. The key is
    /// (tenant, mapping, metric, instant) and a pass stamps every row it writes
    /// with one instant, so the only way to collide is to write a pass twice —
    /// a timer that double-fired, or a pass resumed after a partial write. Both
    /// should be no-ops, and neither is a fault worth failing a capture over.
    pub async fn record(
        &self,
        org: OrgId,
        snapshots: &[MetricSnapshot],
    ) -> Result<u64, StorageError> {
        if snapshots.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let mut written = 0;
        for snapshot in snapshots {
            written += sqlx::query!(
                "INSERT INTO listing_metric_snapshot \
                 (org_id, mapping_id, metric, observed_at, total_value) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
                uuid_to_db(org.0),
                uuid_to_db(snapshot.mapping.0),
                snapshot.metric,
                timestamp_to_db(snapshot.observed_at)?,
                snapshot.total_value,
            )
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(written)
    }

    /// The newest snapshot of every metric this tenant has captured, one row
    /// per (mapping, metric).
    ///
    /// The join to `mapping` is what supplies the inventory the table
    /// deliberately does not carry, and it is also the fence against a
    /// snapshot outliving the mapping it describes: such a row answers nothing
    /// rather than answering with an inventory nobody can name.
    pub async fn latest(&self, org: OrgId) -> Result<Vec<LatestMetric>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT DISTINCT ON (s.mapping_id, s.metric) \
                    s.mapping_id, s.metric, s.observed_at, s.total_value, m.inventory \
               FROM listing_metric_snapshot s \
               JOIN mapping m ON m.org_id = s.org_id AND m.id = s.mapping_id \
              WHERE s.org_id = $1 \
              ORDER BY s.mapping_id, s.metric, s.observed_at DESC",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(LatestMetric {
                    mapping: MappingId(uuid_from_db(row.mapping_id)),
                    inventory: inventory_from_db(&row.inventory)?,
                    metric: row.metric,
                    observed_at: timestamp_from_db(row.observed_at),
                    total_value: row.total_value,
                })
            })
            .collect()
    }
}
