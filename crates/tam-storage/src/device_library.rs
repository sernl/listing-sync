//! What each of the seller's machines holds in its own library, and where
//! another of their machines can reach it directly.
//!
//! Coordination only, as migration 0078 says: the server records a device's
//! node address and the digests it holds, and which files another device has
//! asked for. No byte of a seller's file passes through here, and nothing in
//! this module could carry one.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tam_types::{ContentHash, OrgId, Timestamp, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
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

/// One file, as the file browser lists it: who holds it, who asked for it,
/// and which of the seller's resources it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    pub hash: ContentHash,
    /// The name the catalogue records for these bytes, where a product file
    /// names them; the digest alone otherwise.
    pub file_name: Option<String>,
    pub byte_len: u64,
    pub holders: Vec<Holder>,
    pub wanted_by: Vec<String>,
    /// Every live resource of this organisation whose files carry this
    /// digest. Empty for a file the seller keeps that no resource uses, and
    /// for one whose only resource was deleted.
    pub resources: Vec<LibraryResource>,
}

/// One resource a file belongs to: enough to name it and to link to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryResource {
    pub id: Uuid,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub device: String,
    pub name: String,
    pub last_seen_at: Timestamp,
}

/// Whether a file can be reached now, as the browser's filter asks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryAvailability {
    /// Some machine holding it checked in inside the online window.
    Online,
    /// Machines hold it, and none of them has checked in lately.
    Offline,
    /// The catalogue names the digest and no machine reports holding it.
    Missing,
}

impl LibraryAvailability {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Missing => "missing",
        }
    }
}

/// Whether a file belongs to a live resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryLinked {
    Linked,
    Unlinked,
}

impl LibraryLinked {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Unlinked => "unlinked",
        }
    }
}

/// The largest page the file browser may ask for, and the page it gets
/// without asking. A bound rather than a convention: the read walks every
/// digest the catalogue and the machines name between them, and handing all
/// of those over at once is what the whole-library read used to do.
pub const LIBRARY_LIMIT_MAX: u32 = 100;
pub const LIBRARY_LIMIT_DEFAULT: u32 = 25;

/// What the browser asked to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryFilter<'a> {
    /// Matched, case-insensitively, against the file's name, the titles of
    /// the resources using it, and its hex digest.
    pub search: Option<&'a str>,
    /// Only files this machine reports holding.
    pub device: Option<&'a str>,
    pub availability: Option<LibraryAvailability>,
    pub linked: Option<LibraryLinked>,
    pub offset: u32,
    pub limit: u32,
    /// A holder that checked in at or after this instant counts as online.
    /// Passed in rather than read here, so one page cannot disagree with
    /// itself about what "now" was.
    pub online_after: Timestamp,
}

/// One page of the file browser, with the figure its pager needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPage {
    pub files: Vec<LibraryFile>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
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

    /// One bounded page of the seller's files: what their machines hold,
    /// plus the digests their live resources name that no machine reports,
    /// with who holds each, who asked for it, and which resources use it.
    ///
    /// Page enrichment uses three set-based reads, never one per file.
    /// Filtering and an exact total still scan the tenant's matching metadata.
    pub async fn page(
        &self,
        org: OrgId,
        filter: &LibraryFilter<'_>,
    ) -> Result<LibraryPage, StorageError> {
        let limit = filter.limit.clamp(1, LIBRARY_LIMIT_MAX);
        let online_after = timestamp_to_db(filter.online_after)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        pin_org(&mut tx, org).await?;
        // `totals LEFT JOIN page ON true` rather than a window count: a
        // page past the end has no rows to carry the figure on, and a pager
        // that reads the total as zero there sends the seller nowhere.
        let rows = sqlx::query!(
            r#"WITH held AS (
                   SELECT h.hash AS hash,
                          max(h.byte_len) AS byte_len,
                          bool_or(d.last_seen_at >= $2) AS online,
                          array_agg(h.device_id) AS devices
                     FROM device_library_holding h
                     JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id
                    WHERE h.org_id = $1 AND d.revoked_at IS NULL
                    GROUP BY h.hash
               ),
               named AS (
                   SELECT COALESCE(f.hash, f.observed_hash) AS hash,
                          min(COALESCE(f.name, f.payload_file_name)) AS file_name
                     FROM product_file f
                    WHERE f.org_id = $1
                    GROUP BY COALESCE(f.hash, f.observed_hash)
               ),
               linked AS (
                   SELECT COALESCE(f.hash, f.observed_hash) AS hash,
                          count(DISTINCT p.id) AS resources,
                          max(COALESCE(b.byte_len, f.observed_byte_len)) AS byte_len,
                          bool_or(f.role = 'payload') AS payload,
                          bool_or($3::text IS NOT NULL AND position(lower($3) in lower(p.title)) > 0) AS title_matches
                     FROM product_file f
                     JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id
                     LEFT JOIN blob b ON b.org_id = f.org_id AND b.hash = f.hash
                    WHERE f.org_id = $1
                      AND f.deleted_at IS NULL
                      AND p.deleted_at IS NULL
                    GROUP BY COALESCE(f.hash, f.observed_hash)
               ),
               universe AS (
                   SELECT COALESCE(h.hash, l.hash) AS hash,
                          COALESCE(h.byte_len, l.byte_len, 0) AS byte_len,
                          COALESCE(h.online, false) AS online,
                          h.hash IS NOT NULL AS present,
                          COALESCE(l.resources, 0) AS resources,
                          COALESCE(h.devices, ARRAY[]::text[]) AS devices,
                          l.title_matches AS title_matches
                     FROM held h
                     FULL OUTER JOIN linked l ON l.hash = h.hash
                    WHERE h.hash IS NOT NULL OR l.payload
               ),
               filtered AS (
                   SELECT u.hash AS hash,
                          u.byte_len AS byte_len,
                          n.file_name AS file_name,
                          u.online AS online,
                          u.present AS present,
                          u.resources AS resources
                     FROM universe u
                     LEFT JOIN named n ON n.hash = u.hash
                    WHERE ($3::text IS NULL
                           OR position(lower($3) in lower(COALESCE(n.file_name, ''))) > 0
                           OR COALESCE(u.title_matches, false)
                           OR position(lower($3) in encode(u.hash, 'hex')) > 0)
                      AND ($4::text IS NULL OR $4 = ANY (u.devices))
                      AND ($5::text IS NULL
                           OR ($5 = 'online' AND u.present AND u.online)
                           OR ($5 = 'offline' AND u.present AND NOT u.online)
                           OR ($5 = 'missing' AND NOT u.present))
                      AND ($6::text IS NULL
                           OR ($6 = 'linked' AND u.resources > 0)
                           OR ($6 = 'unlinked' AND u.resources = 0))
               ),
               totals AS (SELECT count(*) AS total FROM filtered),
               chosen AS (
                   SELECT * FROM filtered
                    ORDER BY file_name NULLS LAST, hash
                    OFFSET $7 LIMIT $8
               )
               SELECT t.total AS "total!",
                      c.hash AS "hash?",
                      c.byte_len AS "byte_len?",
                      c.file_name AS "file_name?"
                 FROM totals t
                 LEFT JOIN chosen c ON true
                ORDER BY c.file_name NULLS LAST, c.hash"#,
            uuid_to_db(org.0),
            online_after,
            filter.search,
            filter.device,
            filter.availability.map(LibraryAvailability::as_db),
            filter.linked.map(LibraryLinked::as_db),
            i64::from(filter.offset),
            i64::from(limit),
        )
        .fetch_all(&mut *tx)
        .await?;
        let total = u64::try_from(rows.first().map_or(0, |row| row.total)).unwrap_or(0);
        let mut files: Vec<LibraryFile> = Vec::new();
        for row in rows {
            // A page past the end answers with one row: the total, and no
            // file beside it.
            let Some(raw) = row.hash else {
                continue;
            };
            files.push(LibraryFile {
                hash: hash_from_db(&raw)?,
                file_name: row.file_name,
                byte_len: row
                    .byte_len
                    .and_then(|len| u64::try_from(len).ok())
                    .unwrap_or(0),
                holders: Vec::new(),
                wanted_by: Vec::new(),
                resources: Vec::new(),
            });
        }
        let hashes: Vec<Vec<u8>> = files.iter().map(|file| file.hash.0.to_vec()).collect();
        let holders = sqlx::query!(
            "SELECT h.hash, h.device_id, d.name AS device_name, d.last_seen_at \
               FROM device_library_holding h \
               JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id \
              WHERE h.org_id = $1 AND d.revoked_at IS NULL AND h.hash = ANY ($2) \
              ORDER BY h.hash, d.name",
            uuid_to_db(org.0),
            &hashes[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        let wants = sqlx::query!(
            "SELECT hash, device_id FROM device_library_want \
              WHERE org_id = $1 AND hash = ANY ($2) ORDER BY hash, device_id",
            uuid_to_db(org.0),
            &hashes[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        // Every live resource carrying one of this page's digests, deleted
        // products and deleted file rows excluded, and the tenant pinned:
        // one file reused by three resources names all three, and a file
        // whose only resource was deleted names none.
        let links = sqlx::query!(
            r#"SELECT DISTINCT COALESCE(f.hash, f.observed_hash) AS "hash!", p.id AS "id!", p.title AS "title!"
                 FROM product_file f
                 JOIN product p ON p.org_id = f.org_id AND p.id = f.product_id
                WHERE f.org_id = $1
                  AND f.deleted_at IS NULL
                  AND p.deleted_at IS NULL
                  AND COALESCE(f.hash, f.observed_hash) = ANY ($2)
                ORDER BY p.title, p.id"#,
            uuid_to_db(org.0),
            &hashes[..],
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        for row in holders {
            let hash = hash_from_db(&row.hash)?;
            if let Some(file) = files.iter_mut().find(|file| file.hash == hash) {
                file.holders.push(Holder {
                    device: row.device_id,
                    name: row.device_name,
                    last_seen_at: timestamp_from_db(row.last_seen_at),
                });
            }
        }
        for want in wants {
            let hash = hash_from_db(&want.hash)?;
            if let Some(file) = files.iter_mut().find(|file| file.hash == hash) {
                file.wanted_by.push(want.device_id);
            }
        }
        for link in links {
            let hash = hash_from_db(&link.hash)?;
            if let Some(file) = files.iter_mut().find(|file| file.hash == hash) {
                file.resources.push(LibraryResource {
                    id: uuid_from_db(link.id),
                    title: link.title,
                });
            }
        }
        Ok(LibraryPage {
            files,
            total,
            offset: filter.offset,
            limit,
        })
    }

    /// Whether a live machine of this organisation other than `asking`
    /// reports holding one file. One digest, one row: the refusal in front
    /// of a want does not need the library listed to answer it.
    pub async fn held_elsewhere(
        &self,
        org: OrgId,
        asking: &str,
        hash: ContentHash,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM device_library_holding h
                     JOIN device d ON d.org_id = h.org_id AND d.id = h.device_id
                    WHERE h.org_id = $1 AND h.hash = $2 AND h.device_id <> $3
                      AND d.revoked_at IS NULL
               ) AS "held!""#,
            uuid_to_db(org.0),
            &hash.0[..],
            asking,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(held)
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
