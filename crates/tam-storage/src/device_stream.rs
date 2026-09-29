//! Asks for part of one file, waiting for the device that holds it.
//!
//! An imported file stays on the seller's devices (migration 0099). When the
//! seller opens one in a browser, the server process holding that request
//! writes a row here naming the device, the file and the bytes, and the
//! device's long poll -- which may reach another server process -- claims it
//! and answers with the bytes, which are passed through and never written
//! anywhere. This module carries the metadata only; nothing here could hold a
//! byte of a file.
//!
//! Also here: the reads the broker and the file views ask before any of that —
//! which devices hold a file and how recently each asked for streams, which
//! digests the seller uploaded themselves, and what a file is called.

use std::collections::{HashMap, HashSet};

use sqlx::PgPool;
use tam_types::{ContentHash, OrgId, Timestamp, Uuid};

use crate::codec::{hash_from_db, hash_to_db, timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// The channel a new ask is announced on, with `<org>:<device>` as payload,
/// so whichever server process holds that device's long poll wakes it.
pub const NOTIFY_CHANNEL: &str = "device_stream";

/// One of the seller's devices holding a file, as the broker chooses among
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamHolder {
    pub device: String,
    pub name: String,
    pub last_seen_at: Timestamp,
    /// When the device last asked for streams, or `None` where it never has.
    pub stream_polled_at: Option<Timestamp>,
}

/// One ask as the broker writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStream<'a> {
    pub id: Uuid,
    pub device: &'a str,
    pub hash: ContentHash,
    pub first: u64,
    pub last: u64,
    pub capability: &'a str,
    /// Where the server process holding the browser's request listens, or
    /// `None` where there is only one.
    pub pod: Option<&'a str>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// One ask as a device's poll or answer reads it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamRow {
    pub id: Uuid,
    pub device: String,
    pub hash: ContentHash,
    pub first: u64,
    pub last: u64,
    pub capability: String,
    pub pod: Option<String>,
}

/// What a live resource calls one file, and the type its bytes are served
/// under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedFile {
    /// The seller's own name, the producer's where the file was imported, or
    /// `None` where neither was ever recorded.
    pub file_name: Option<String>,
    pub content_type: String,
    /// The length of the bytes: the stored blob's for an upload, what the
    /// device reported for an import.
    pub byte_len: u64,
    /// Whether the seller uploaded these bytes, so Teachouse stores them.
    pub uploaded: bool,
}

fn length_of(raw: i64) -> u64 {
    u64::try_from(raw).unwrap_or(0)
}

fn offset_to_db(raw: u64) -> Result<i64, StorageError> {
    i64::try_from(raw).map_err(|_| StorageError::CorruptRow {
        reason: "a stream offset does not fit a bigint".to_owned(),
    })
}

pub struct DeviceStreamRepo {
    pool: PgPool,
}

impl DeviceStreamRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every unrevoked device of this organisation reporting one file, the
    /// one that asked for streams most recently first, then the one that
    /// checked in most recently.
    pub async fn holders(
        &self,
        org: OrgId,
        hash: ContentHash,
    ) -> Result<Vec<StreamHolder>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT d.id, d.name, d.last_seen_at, d.stream_polled_at \
               FROM device_library_holding h \
               JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id \
              WHERE h.org_id = $1 AND h.hash = $2 AND d.revoked_at IS NULL \
              ORDER BY d.stream_polled_at DESC NULLS LAST, d.last_seen_at DESC, d.id",
            uuid_to_db(org.0),
            hash_to_db(hash),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| StreamHolder {
                device: row.id,
                name: row.name,
                last_seen_at: timestamp_from_db(row.last_seen_at),
                stream_polled_at: row.stream_polled_at.map(timestamp_from_db),
            })
            .collect())
    }

    /// Every unrevoked holder of each of these digests, by name, for the file
    /// views.
    pub async fn holders_of(
        &self,
        org: OrgId,
        hashes: &[ContentHash],
    ) -> Result<HashMap<ContentHash, Vec<StreamHolder>>, StorageError> {
        let mut found: HashMap<ContentHash, Vec<StreamHolder>> = HashMap::new();
        if hashes.is_empty() {
            return Ok(found);
        }
        let raw: Vec<Vec<u8>> = hashes.iter().map(|hash| hash_to_db(*hash)).collect();
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT h.hash, d.id, d.name, d.last_seen_at, d.stream_polled_at \
               FROM device_library_holding h \
               JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id \
              WHERE h.org_id = $1 AND h.hash = ANY ($2) AND d.revoked_at IS NULL \
              ORDER BY h.hash, d.name, d.id",
            uuid_to_db(org.0),
            &raw[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        for row in rows {
            found
                .entry(hash_from_db(&row.hash)?)
                .or_default()
                .push(StreamHolder {
                    device: row.id,
                    name: row.name,
                    last_seen_at: timestamp_from_db(row.last_seen_at),
                    stream_polled_at: row.stream_polled_at.map(timestamp_from_db),
                });
        }
        Ok(found)
    }

    /// Which of these digests a live file the seller uploaded names: bytes
    /// Teachouse stores on purpose.
    pub async fn uploaded(
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
            r#"SELECT DISTINCT f.hash AS "hash!"
                 FROM product_file f
                WHERE f.org_id = $1 AND f.hash = ANY ($2) AND f.deleted_at IS NULL"#,
            uuid_to_db(org.0),
            &raw[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.iter().map(|hash| hash_from_db(hash)).collect()
    }

    /// What a live resource of this organisation calls these bytes, or `None`
    /// where none of them uses the digest. A digest reused under two names
    /// answers the first by name, the order the file browser lists them in.
    pub async fn named(
        &self,
        org: OrgId,
        hash: ContentHash,
    ) -> Result<Option<NamedFile>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            r#"SELECT COALESCE(f.name, f.payload_file_name) AS file_name,
                      f.payload_content_type AS content_type,
                      f.kind AS "kind!",
                      f.hash IS NOT NULL AS "uploaded!",
                      COALESCE((SELECT b.byte_len FROM blob b
                                 WHERE b.org_id = f.org_id AND b.hash = f.hash),
                               f.observed_byte_len, 0) AS "byte_len!"
                 FROM product_file f
                 JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id
                WHERE f.org_id = $1
                  AND COALESCE(f.hash, f.observed_hash) = $2
                  AND f.deleted_at IS NULL
                  AND p.deleted_at IS NULL
                ORDER BY f.hash IS NULL, 1 NULLS LAST
                LIMIT 1"#,
            uuid_to_db(org.0),
            hash_to_db(hash),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| NamedFile {
            file_name: row.file_name,
            content_type: row
                .content_type
                .unwrap_or_else(|| crate::blobs::content_type(&row.kind)),
            byte_len: length_of(row.byte_len),
            uploaded: row.uploaded,
        }))
    }

    /// Stamps a device as asking for streams now. `false` where the device is
    /// unknown or signed out, which is not a device that may serve.
    pub async fn polled(
        &self,
        org: OrgId,
        device: &str,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let stamped = sqlx::query_scalar!(
            r#"UPDATE device SET stream_polled_at = $3
                WHERE org_id = $1 AND id = $2 AND revoked_at IS NULL
            RETURNING 1 AS "one!""#,
            uuid_to_db(org.0),
            device,
            timestamp_to_db(at)?,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stamped.is_some())
    }

    /// Writes one ask and announces it on [`NOTIFY_CHANNEL`], in one
    /// transaction so a listener never wakes before the row is visible. Asks
    /// of this organisation past their expiry are cleared on the way.
    pub async fn open(&self, org: OrgId, stream: &NewStream<'_>) -> Result<(), StorageError> {
        let created = timestamp_to_db(stream.created_at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "DELETE FROM device_stream WHERE org_id = $1 AND expires_at < $2",
            uuid_to_db(org.0),
            created,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO device_stream \
                 (org_id, id, device_id, hash, first_byte, last_byte, capability, pod, \
                  created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            uuid_to_db(org.0),
            uuid_to_db(stream.id),
            stream.device,
            hash_to_db(stream.hash),
            offset_to_db(stream.first)?,
            offset_to_db(stream.last)?,
            stream.capability,
            stream.pod,
            created,
            timestamp_to_db(stream.expires_at)?,
        )
        .execute(&mut *tx)
        .await?;
        let payload = format!("{}:{}", uuid_to_db(org.0), stream.device);
        sqlx::query!("SELECT pg_notify($1, $2)", NOTIFY_CHANNEL, payload)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Hands every unexpired, unclaimed ask for one device to that device's
    /// poll, once: a claimed ask is not handed over again.
    pub async fn claim(
        &self,
        org: OrgId,
        device: &str,
        at: Timestamp,
    ) -> Result<Vec<StreamRow>, StorageError> {
        let instant = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "UPDATE device_stream SET claimed_at = $3 \
              WHERE org_id = $1 AND device_id = $2 AND claimed_at IS NULL AND expires_at > $3 \
          RETURNING id, device_id, hash, first_byte, last_byte, capability, pod",
            uuid_to_db(org.0),
            device,
            instant,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(StreamRow {
                    id: crate::codec::uuid_from_db(row.id),
                    device: row.device_id,
                    hash: hash_from_db(&row.hash)?,
                    first: length_of(row.first_byte),
                    last: length_of(row.last_byte),
                    capability: row.capability,
                    pod: row.pod,
                })
            })
            .collect()
    }

    /// One ask by id, where it has not expired.
    pub async fn find(
        &self,
        org: OrgId,
        id: Uuid,
        at: Timestamp,
    ) -> Result<Option<StreamRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, device_id, hash, first_byte, last_byte, capability, pod \
               FROM device_stream WHERE org_id = $1 AND id = $2 AND expires_at > $3",
            uuid_to_db(org.0),
            uuid_to_db(id),
            timestamp_to_db(at)?,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| {
            Ok(StreamRow {
                id: crate::codec::uuid_from_db(row.id),
                device: row.device_id,
                hash: hash_from_db(&row.hash)?,
                first: length_of(row.first_byte),
                last: length_of(row.last_byte),
                capability: row.capability,
                pod: row.pod,
            })
        })
        .transpose()
    }

    /// Forgets one ask: answered, refused, abandoned or timed out.
    pub async fn close(&self, org: OrgId, id: Uuid) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "DELETE FROM device_stream WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
