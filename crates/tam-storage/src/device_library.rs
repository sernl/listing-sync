//! What each of the seller's machines holds in its own library, and where
//! another of their machines can reach it directly.
//!
//! Coordination only, as migration 0078 says: the server records a device's
//! node address and the digests it holds, and which files another device has
//! asked for. No byte of a seller's file passes through here, and nothing in
//! this module could carry one.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tam_types::{ContentHash, OrgId, Timestamp};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// One file a device reports holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldingReport {
    pub hash: ContentHash,
    pub byte_len: u64,
}

/// A device's whole library report: where it can be reached, and what it
/// holds. Replaces what the device last said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryReport<'a> {
    pub node_id: &'a str,
    pub direct_addrs: &'a [String],
    pub holdings: &'a [HoldingReport],
}

/// One file, as the console lists it: who holds it and who asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    pub hash: ContentHash,
    /// The name the catalogue records for these bytes, where a product file
    /// names them; the digest alone otherwise.
    pub file_name: Option<String>,
    pub byte_len: u64,
    pub holders: Vec<Holder>,
    pub wanted_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub device: String,
    pub name: String,
    pub last_seen_at: Timestamp,
}

/// Where one holder can be reached, for the device that wants the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub device: String,
    pub node_id: String,
    pub direct_addrs: Vec<String>,
}

fn hash_from_db(bytes: &[u8]) -> Result<ContentHash, StorageError> {
    <[u8; 32]>::try_from(bytes)
        .map(ContentHash)
        .map_err(|_| StorageError::CorruptRow {
            reason: "a library digest is not 32 bytes".to_owned(),
        })
}

pub struct DeviceLibraryRepo {
    pool: PgPool,
}

impl DeviceLibraryRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Replaces what one device says about itself. The device must be
    /// registered; a report for an unknown device is a no-op that answers
    /// `false`, because a heartbeat is not a registration.
    pub async fn report(
        &self,
        org: OrgId,
        device: &str,
        report: &LibraryReport<'_>,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let instant = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let known = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM device WHERE org_id = $1 AND id = $2) AS "known!""#,
            uuid_to_db(org.0),
            device,
        )
        .fetch_one(&mut *tx)
        .await?;
        if !known {
            tx.commit().await?;
            return Ok(false);
        }
        let addrs = serde_json::to_value(report.direct_addrs).map_err(|why| {
            StorageError::Inconsistent {
                reason: format!("the direct addresses did not encode: {why}"),
            }
        })?;
        sqlx::query!(
            "INSERT INTO device_node_addr (org_id, device_id, node_id, direct_addrs, reported_at) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (org_id, device_id) DO UPDATE \
                 SET node_id = EXCLUDED.node_id, \
                     direct_addrs = EXCLUDED.direct_addrs, \
                     reported_at = EXCLUDED.reported_at",
            uuid_to_db(org.0),
            device,
            report.node_id,
            addrs,
            instant,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM device_library_holding WHERE org_id = $1 AND device_id = $2",
            uuid_to_db(org.0),
            device,
        )
        .execute(&mut *tx)
        .await?;
        for holding in report.holdings {
            sqlx::query!(
                "INSERT INTO device_library_holding (org_id, device_id, hash, byte_len, reported_at) \
                 VALUES ($1, $2, $3, $4, $5)",
                uuid_to_db(org.0),
                device,
                &holding.hash.0[..],
                i64::try_from(holding.byte_len).unwrap_or(i64::MAX),
                instant,
            )
            .execute(&mut *tx)
            .await?;
        }
        // A want this device now holds is satisfied.
        sqlx::query!(
            "DELETE FROM device_library_want w \
             WHERE w.org_id = $1 AND w.device_id = $2 \
               AND EXISTS (SELECT 1 FROM device_library_holding h \
                           WHERE h.org_id = w.org_id AND h.device_id = w.device_id AND h.hash = w.hash)",
            uuid_to_db(org.0),
            device,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Every file some device of this organisation holds, with who holds
    /// it and who asked for it. Names come from the catalogue where a
    /// product file carries the same digest.
    pub async fn files(&self, org: OrgId) -> Result<Vec<LibraryFile>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let holdings = sqlx::query!(
            r#"SELECT h.hash, h.byte_len, h.device_id, d.name AS device_name, d.last_seen_at,
                      (SELECT COALESCE(f.name, f.payload_file_name) FROM product_file f
                        WHERE f.org_id = h.org_id
                          AND (f.observed_hash = h.hash OR f.hash = h.hash)
                        ORDER BY f.name IS NULL, f.id LIMIT 1) AS "file_name?"
               FROM device_library_holding h
               JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id
              WHERE h.org_id = $1 AND d.revoked_at IS NULL
              ORDER BY h.hash, d.name"#,
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        let wants = sqlx::query!(
            "SELECT hash, device_id FROM device_library_want WHERE org_id = $1 ORDER BY hash, device_id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut files: Vec<LibraryFile> = Vec::new();
        for row in holdings {
            let hash = hash_from_db(&row.hash)?;
            let holder = Holder {
                device: row.device_id,
                name: row.device_name,
                last_seen_at: timestamp_from_db(row.last_seen_at),
            };
            match files.iter_mut().find(|file| file.hash == hash) {
                Some(file) => file.holders.push(holder),
                None => files.push(LibraryFile {
                    hash,
                    file_name: row.file_name,
                    byte_len: u64::try_from(row.byte_len).unwrap_or(0),
                    holders: vec![holder],
                    wanted_by: Vec::new(),
                }),
            }
        }
        for want in wants {
            let hash = hash_from_db(&want.hash)?;
            if let Some(file) = files.iter_mut().find(|file| file.hash == hash) {
                file.wanted_by.push(want.device_id);
            }
        }
        Ok(files)
    }

    /// Records that one device wants one file. Idempotent.
    pub async fn want(
        &self,
        org: OrgId,
        device: &str,
        hash: ContentHash,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let instant = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO device_library_want (org_id, device_id, hash, requested_at) \
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            uuid_to_db(org.0),
            device,
            &hash.0[..],
            instant,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Cancels a want. Cancelling what was never asked for is success.
    pub async fn unwant(
        &self,
        org: OrgId,
        device: &str,
        hash: ContentHash,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "DELETE FROM device_library_want WHERE org_id = $1 AND device_id = $2 AND hash = $3",
            uuid_to_db(org.0),
            device,
            &hash.0[..],
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The files one device has asked for and does not yet hold.
    pub async fn wants_of(
        &self,
        org: OrgId,
        device: &str,
    ) -> Result<Vec<ContentHash>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT hash FROM device_library_want WHERE org_id = $1 AND device_id = $2 ORDER BY requested_at",
            uuid_to_db(org.0),
            device,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.iter().map(|row| hash_from_db(&row.hash)).collect()
    }

    /// The other devices holding one file that have checked in since
    /// `seen_after`, with where each can be reached.
    pub async fn peers(
        &self,
        org: OrgId,
        asking: &str,
        hash: ContentHash,
        seen_after: Timestamp,
    ) -> Result<Vec<Peer>, StorageError> {
        let since: DateTime<Utc> = timestamp_to_db(seen_after)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT h.device_id, a.node_id, a.direct_addrs \
               FROM device_library_holding h \
               JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id \
               JOIN device_node_addr a ON a.org_id = h.org_id AND a.device_id = h.device_id \
              WHERE h.org_id = $1 AND h.hash = $2 AND h.device_id <> $3 \
                AND d.revoked_at IS NULL AND d.last_seen_at >= $4 \
              ORDER BY d.last_seen_at DESC",
            uuid_to_db(org.0),
            &hash.0[..],
            asking,
            since,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let direct_addrs: Vec<String> =
                    serde_json::from_value(row.direct_addrs).map_err(|why| {
                        StorageError::CorruptRow {
                            reason: format!("a node address list did not decode: {why}"),
                        }
                    })?;
                Ok(Peer {
                    device: row.device_id,
                    node_id: row.node_id,
                    direct_addrs,
                })
            })
            .collect()
    }

    /// Every node id this organisation's live devices report, which is the
    /// set a device accepts a direct connection from.
    pub async fn node_ids(&self, org: OrgId) -> Result<Vec<String>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_scalar!(
            "SELECT a.node_id FROM device_node_addr a \
               JOIN device d ON d.org_id = a.org_id AND d.id = a.device_id \
              WHERE a.org_id = $1 AND d.revoked_at IS NULL \
              ORDER BY a.node_id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows)
    }
}
