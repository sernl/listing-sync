//! Product files whose bytes are the seller's marketplace resource rather than
//! a blob we hold.
//!
//! D27 keeps a migrated file's bytes on the seller's own device: it is fetched
//! from the source marketplace under the seller's own session and uploaded to
//! the target in the same run, and never reaches our servers. So the row that
//! stands for such a file cannot carry a digest we computed, and carries a
//! locator and a digest a device asserted instead.
//!
//! This module owns the observation history alone. The file's own row is
//! written by `ProductRepo::insert` like every other product file, because a
//! product and its first payload must land in one transaction —
//! `product_payload_nonempty` is a deferred constraint trigger and refuses a
//! product that commits without one — and a device-imported product carries a
//! sourced payload beside a blob-backed cover, so one insert has to write
//! both. A separate creation path here could not compose with that and was
//! removed rather than worked around.
//!
//! What is left is the part that is genuinely its own: the file's source group
//! records the first observation and is never overwritten, while every
//! observation including the first is appended here. A second device reporting
//! a different digest for one resource is recorded rather than refused and
//! rather than silently replacing what the first saw, and
//! [`ProductFileSourceRepo::disputed`] is what the item view asks to find
//! out.

use sqlx::PgPool;
use tam_types::{FileId, Observation, OrgId, Timestamp};

use crate::codec::{hash_to_db, timestamp_to_db, uuid_to_db, ScanColumns};
use crate::{pin_org, StorageError};

pub struct ProductFileSourceRepo {
    pool: PgPool,
}

impl ProductFileSourceRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Appends an observation without touching the file's own row.
    ///
    /// The first observation stands as what the file says about itself. A
    /// later one that disagrees is a fact to surface, not a correction to
    /// apply: we cannot tell which device saw the truth, and overwriting would
    /// destroy the evidence that they differ.
    pub async fn observe(
        &self,
        org: OrgId,
        file: FileId,
        observation: &Observation,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        append(&mut tx, org, file, observation, at).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether this file's observations disagree about its bytes.
    ///
    /// More than one distinct digest across the history, which is the item
    /// view's question. One device observing the same file twice answers
    /// `false`, because the same digest twice is agreement however many rows
    /// it took.
    pub async fn disputed(&self, org: OrgId, file: FileId) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let distinct = sqlx::query_scalar!(
            "SELECT count(DISTINCT observed_hash) FROM product_file_observation \
             WHERE org_id = $1 AND file_id = $2",
            uuid_to_db(org.0),
            uuid_to_db(file.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(distinct.unwrap_or_default() > 1)
    }
}

/// Appends one observation inside a caller's transaction.
///
/// `pub(crate)` because `ProductRepo::insert` calls it for a sourced file's
/// first observation, in the same transaction as the row itself: the history
/// and the file's own copy of it must land together or the first observation
/// is missing from the log that is supposed to hold every one.
pub(crate) async fn append(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    file: FileId,
    observation: &Observation,
    at: Timestamp,
) -> Result<(), StorageError> {
    let scan = ScanColumns::from_outcome(&observation.scan)?;
    let byte_len = i64::try_from(observation.byte_len).map_err(|_| StorageError::Inconsistent {
        reason: format!("observed byte_len {} exceeds i64", observation.byte_len),
    })?;
    sqlx::query!(
        "INSERT INTO product_file_observation \
         (org_id, file_id, observed_by_device, observed_at, recorded_at, \
          observed_hash, observed_byte_len, scan_state, scan_signature, \
          scan_failure_code, scanned_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        uuid_to_db(org.0),
        uuid_to_db(file.0),
        observation.device,
        timestamp_to_db(observation.observed_at)?,
        timestamp_to_db(at)?,
        hash_to_db(observation.hash),
        byte_len,
        scan.state,
        scan.signature,
        scan.failure_code,
        scan.scanned_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
