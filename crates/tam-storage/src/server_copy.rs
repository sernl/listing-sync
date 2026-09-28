//! Teachouse's own copy of a file an import left on the seller's device.
//!
//! An imported original is a marketplace-sourced `product_file` row: the
//! device read the bytes and reported their digest, and the bytes stayed on
//! the device. The seller's app then copies those bytes here, into the blob
//! store, so they can be viewed, cut into a preview and downloaded wherever
//! the seller signs in. The row stays sourced — the marketplace remains where
//! the file came from — and the copy is a `blob` row under the same digest,
//! which is what every read below asks after.
//!
//! No new table: a copy is a blob like any other, charged against the plan's
//! storage like any other, and found by the digest the sourced row already
//! carries.

use std::collections::HashSet;

use sqlx::PgPool;
use tam_types::{ContentHash, OrgId};

use crate::codec::{hash_from_db, hash_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// One file a live resource names whose bytes this server does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingCopy {
    pub hash: ContentHash,
    /// What the device reported the bytes weigh.
    pub byte_len: u64,
}

/// What a live resource calls one file, and the type its bytes are served
/// under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedCopy {
    /// The seller's own name, the producer's where the file was imported, or
    /// `None` where neither was ever recorded.
    pub file_name: Option<String>,
    /// The producer's type where the file was imported, and the one its
    /// kind implies for a blob-backed file.
    pub content_type: String,
    /// What this server's copy weighs, or `None` where it holds none.
    pub stored_len: Option<u64>,
}

pub struct ServerCopyRepo {
    pool: PgPool,
}

fn length_of(raw: i64) -> u64 {
    u64::try_from(raw).unwrap_or(0)
}

impl ServerCopyRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every digest a live resource's imported file names that has no blob
    /// here, smallest first, so a device working down the list lands as many
    /// files as the plan has room for.
    ///
    /// Live only: a file of a deleted resource is not one the seller can
    /// open, and copying it would charge them for storage nobody reads.
    pub async fn missing(&self, org: OrgId) -> Result<Vec<MissingCopy>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            r#"SELECT f.observed_hash AS "hash!", max(f.observed_byte_len) AS "byte_len!"
                 FROM product_file f
                 JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id
                WHERE f.org_id = $1
                  AND f.hash IS NULL
                  AND f.observed_hash IS NOT NULL
                  AND f.deleted_at IS NULL
                  AND p.deleted_at IS NULL
                  AND NOT EXISTS (
                      SELECT 1 FROM blob b WHERE b.org_id = f.org_id AND b.hash = f.observed_hash
                  )
                GROUP BY f.observed_hash
                ORDER BY 2, 1"#,
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(MissingCopy {
                    hash: hash_from_db(&row.hash)?,
                    byte_len: length_of(row.byte_len),
                })
            })
            .collect()
    }

    /// Which of these digests this organisation holds bytes for.
    pub async fn held(
        &self,
        org: OrgId,
        hashes: &[ContentHash],
    ) -> Result<HashSet<ContentHash>, StorageError> {
        if hashes.is_empty() {
            return Ok(HashSet::new());
        }
        let raw: Vec<Vec<u8>> = hashes.iter().map(|hash| hash_to_db(*hash)).collect();
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_scalar!(
            "SELECT hash FROM blob WHERE org_id = $1 AND hash = ANY ($2)",
            uuid_to_db(org.0),
            &raw[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.iter().map(|hash| hash_from_db(hash)).collect()
    }

    /// The weight a live resource's imported file reports for these bytes,
    /// or `None` where no such file names them.
    ///
    /// What admits a copy: the route that takes one accepts only bytes an
    /// import already described, so it cannot become a way round the upload
    /// and its checks.
    pub async fn sourced_len(
        &self,
        org: OrgId,
        hash: ContentHash,
    ) -> Result<Option<u64>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = sqlx::query_scalar!(
            r#"SELECT max(f.observed_byte_len) AS "byte_len"
                 FROM product_file f
                 JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id
                WHERE f.org_id = $1
                  AND f.hash IS NULL
                  AND f.observed_hash = $2
                  AND f.deleted_at IS NULL
                  AND p.deleted_at IS NULL"#,
            uuid_to_db(org.0),
            hash_to_db(hash),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(found.map(length_of))
    }

    /// What a live resource of this organisation calls these bytes, or `None`
    /// where none of them uses the digest.
    ///
    /// A digest reused under two names answers the first by name, which is
    /// the order the file browser lists them in too.
    pub async fn named(
        &self,
        org: OrgId,
        hash: ContentHash,
    ) -> Result<Option<NamedCopy>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            r#"SELECT COALESCE(f.name, f.payload_file_name) AS file_name,
                      f.payload_content_type AS content_type,
                      f.kind AS "kind!",
                      (SELECT b.byte_len FROM blob b
                        WHERE b.org_id = f.org_id AND b.hash = $2) AS stored_len
                 FROM product_file f
                 JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id
                WHERE f.org_id = $1
                  AND COALESCE(f.hash, f.observed_hash) = $2
                  AND f.deleted_at IS NULL
                  AND p.deleted_at IS NULL
                ORDER BY 1 NULLS LAST
                LIMIT 1"#,
            uuid_to_db(org.0),
            hash_to_db(hash),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| NamedCopy {
            file_name: row.file_name,
            content_type: row
                .content_type
                .unwrap_or_else(|| crate::blobs::content_type(&row.kind)),
            stored_len: row.stored_len.map(length_of),
        }))
    }
}
