//! The device registry: the seller's own machines, and the marketplaces each
//! one holds a session for.
//!
//! Decision D14 in `docs/notes/design/vendoo-for-teachers-rethink.md` puts
//! this beside better-auth rather than inside it, because better-auth's
//! session row carries `ipAddress` and `userAgent` and has no device-name
//! concept. Nothing here touches the `auth` schema; the "Your devices" page
//! joins the two planes in the browser, from the identity service's own client
//! SDK on one side and this repository on the other.
//!
//! Metadata only, on both tables. Neither carries a column a cookie or a token
//! could travel in, and migration 0042 records why: D1 keeps every no-API
//! marketplace session on the seller's device, so a server-side column able to
//! hold one would reintroduce exactly the custody the architecture removes.
//!
//! Revocation is a decision, not an effect. Marking a device revoked stops the
//! server answering it, and the device wipes its own sessions on its next
//! heartbeat; a device that never reconnects keeps its marketplace cookies
//! until the marketplace itself expires them. That limit is D14's and is
//! stated rather than engineered away.

use sqlx::PgPool;
use tam_types::{Marketplace, OrgId, Timestamp};

use crate::codec::{marketplace_to_db, timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::connections::marketplace_from_db;
use crate::{pin_org, StorageError};

/// What a device last reported about one marketplace session.
///
/// Closed, and every value is one the device can distinguish without making a
/// marketplace request. There is deliberately no `Expired`: nothing on the
/// device can tell a live session from a dead one without contacting the
/// marketplace, so a status meaning that would be a claim the data does not
/// support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceSessionStatus {
    /// The device holds a session it captured and has not discarded.
    Connected,
    /// The seller disconnected it on the device itself.
    SignedOut,
    /// The device discarded it because a heartbeat told it that it was
    /// revoked.
    Wiped,
}

impl DeviceSessionStatus {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::Connected, Self::SignedOut, Self::Wiped];

    /// The column text, which is also the token the wire and the generated
    /// client vocabulary carry: one word across all three, the way
    /// [`crate::ItemStateKind`] already does it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::SignedOut => "signed_out",
            Self::Wiped => "wiped",
        }
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == raw)
            .ok_or_else(|| StorageError::CorruptRow {
                reason: format!("unknown device session status {raw:?}"),
            })
    }
}

/// What a device says about itself when it registers or re-registers.
///
/// Borrowed rather than owned because every field arrives out of a request
/// body and is written straight through; nothing here is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRegistration<'a> {
    /// The identifier the device minted for itself and keeps in its own
    /// application data. Client-asserted by construction, and scoped by
    /// organisation, so a collision is only ever between one seller's own
    /// machines.
    pub id: &'a str,
    pub name: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub app_version: &'a str,
}

/// One marketplace session as a heartbeat reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceSessionReport<'a> {
    pub marketplace: Marketplace,
    /// Whatever the marketplace made cheaply visible at capture time; `None`
    /// is normal and is never worth a request to fill in.
    pub account_label: Option<&'a str>,
    pub status: DeviceSessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSessionRecord {
    pub marketplace: Marketplace,
    pub account_label: Option<String>,
    pub linked_at: Timestamp,
    pub last_used_at: Timestamp,
    pub status: DeviceSessionStatus,
}

/// One device and everything the registry holds about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    pub first_seen_at: Timestamp,
    pub last_seen_at: Timestamp,
    /// When the seller signed this device out. The instant we decided, not the
    /// instant the device complied: `last_seen_at` later than this is the only
    /// evidence of compliance.
    pub revoked_at: Option<Timestamp>,
    pub sessions: Vec<DeviceSessionRecord>,
}

/// What a heartbeat answers with: whether this device may keep working, and
/// since when it may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceHeartbeat {
    pub revoked_at: Option<Timestamp>,
}

impl DeviceHeartbeat {
    #[must_use]
    pub const fn revoked(self) -> bool {
        self.revoked_at.is_some()
    }
}

pub struct DeviceRepo {
    pool: PgPool,
}

impl DeviceRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records this device, or refreshes what a known one says about itself.
    ///
    /// `revoked_at` is deliberately not cleared. A revoked device
    /// re-registering is the exact case the mark exists for, and a
    /// registration that lifted it would let any device undo its own
    /// revocation by restarting.
    pub async fn register(
        &self,
        org: OrgId,
        device: &DeviceRegistration<'_>,
        at: Timestamp,
    ) -> Result<DeviceRecord, StorageError> {
        let seen = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO device \
             (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             ON CONFLICT (org_id, id) DO UPDATE \
                 SET name = EXCLUDED.name, \
                     os = EXCLUDED.os, \
                     arch = EXCLUDED.arch, \
                     app_version = EXCLUDED.app_version, \
                     last_seen_at = EXCLUDED.last_seen_at",
            uuid_to_db(org.0),
            device.id,
            device.name,
            device.os,
            device.arch,
            device.app_version,
            seen,
        )
        .execute(&mut *tx)
        .await?;
        let records = load(&mut tx, org, Some(device.id)).await?;
        tx.commit().await?;
        records
            .into_iter()
            .next()
            .ok_or_else(|| StorageError::Inconsistent {
                reason: "the device just registered does not read back".to_owned(),
            })
    }

    /// Stamps the device seen, replaces what it holds, and answers whether it
    /// is revoked.
    ///
    /// The report is the device's whole session set rather than a delta, so a
    /// marketplace the device no longer names is deleted here. A delta would
    /// leave a stale row standing after a disconnect the device performed
    /// while offline, and the page's whole job is saying what a device holds
    /// now.
    ///
    /// `Ok(None)` means no such device for this tenant, which the API answers
    /// as not-found: a heartbeat is not a registration and must not create one.
    pub async fn heartbeat(
        &self,
        org: OrgId,
        device: &str,
        sessions: &[DeviceSessionReport<'_>],
        at: Timestamp,
    ) -> Result<Option<DeviceHeartbeat>, StorageError> {
        let seen = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let Some(row) = sqlx::query!(
            "UPDATE device SET last_seen_at = $3 \
             WHERE org_id = $1 AND id = $2 \
             RETURNING revoked_at",
            uuid_to_db(org.0),
            device,
            seen,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };

        let named: Vec<String> = sessions
            .iter()
            .map(|session| marketplace_to_db(session.marketplace).to_owned())
            .collect();
        sqlx::query!(
            "DELETE FROM device_marketplace_session \
             WHERE org_id = $1 AND device_id = $2 AND marketplace <> ALL($3)",
            uuid_to_db(org.0),
            device,
            &named,
        )
        .execute(&mut *tx)
        .await?;

        for session in sessions {
            sqlx::query!(
                "INSERT INTO device_marketplace_session \
                 (org_id, device_id, marketplace, account_label, linked_at, last_used_at, status) \
                 VALUES ($1, $2, $3, $4, $5, $5, $6) \
                 ON CONFLICT (org_id, device_id, marketplace) DO UPDATE \
                     SET account_label = EXCLUDED.account_label, \
                         status = EXCLUDED.status, \
                         last_used_at = CASE WHEN EXCLUDED.status = 'connected' \
                                             THEN EXCLUDED.last_used_at \
                                             ELSE device_marketplace_session.last_used_at END",
                uuid_to_db(org.0),
                device,
                marketplace_to_db(session.marketplace),
                session.account_label,
                seen,
                session.status.as_str(),
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Some(DeviceHeartbeat {
            revoked_at: row.revoked_at.map(timestamp_from_db),
        }))
    }

    /// Every device this organisation has registered, revoked ones included: a
    /// device the seller signed out is part of the record, and hiding it would
    /// hide the one row whose wipe is still outstanding.
    pub async fn list(&self, org: OrgId) -> Result<Vec<DeviceRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let records = load(&mut tx, org, None).await?;
        tx.commit().await?;
        Ok(records)
    }

    /// Signs one device out, answering the revocation instant now standing.
    ///
    /// Revoking twice keeps the first instant rather than restamping, because
    /// the first is when the seller decided and the wipe is measured against
    /// it. `Ok(None)` is no such device, which is a different fact from an
    /// already-revoked one and answers not-found.
    pub async fn revoke(
        &self,
        org: OrgId,
        device: &str,
        at: Timestamp,
    ) -> Result<Option<Timestamp>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "UPDATE device SET revoked_at = COALESCE(revoked_at, $3) \
             WHERE org_id = $1 AND id = $2 \
             RETURNING revoked_at AS \"revoked_at!\"",
            uuid_to_db(org.0),
            device,
            timestamp_to_db(at)?,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| timestamp_from_db(row.revoked_at)))
    }
}

/// The devices and their sessions, folded into one record each. `only` narrows
/// to a single device without a second query shape.
async fn load(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    only: Option<&str>,
) -> Result<Vec<DeviceRecord>, StorageError> {
    let devices = sqlx::query!(
        "SELECT id, name, os, arch, app_version, first_seen_at, last_seen_at, revoked_at \
         FROM device \
         WHERE org_id = $1 AND ($2::text IS NULL OR id = $2) \
         ORDER BY first_seen_at, id",
        uuid_to_db(org.0),
        only,
    )
    .fetch_all(&mut **tx)
    .await?;

    let sessions = sqlx::query!(
        "SELECT device_id, marketplace, account_label, linked_at, last_used_at, status \
         FROM device_marketplace_session \
         WHERE org_id = $1 AND ($2::text IS NULL OR device_id = $2) \
         ORDER BY device_id, marketplace",
        uuid_to_db(org.0),
        only,
    )
    .fetch_all(&mut **tx)
    .await?;

    devices
        .into_iter()
        .map(|device| {
            let held = sessions
                .iter()
                .filter(|session| session.device_id == device.id)
                .map(|session| {
                    Ok(DeviceSessionRecord {
                        marketplace: marketplace_from_db(&session.marketplace)?,
                        account_label: session.account_label.clone(),
                        linked_at: timestamp_from_db(session.linked_at),
                        last_used_at: timestamp_from_db(session.last_used_at),
                        status: DeviceSessionStatus::from_db(&session.status)?,
                    })
                })
                .collect::<Result<Vec<_>, StorageError>>()?;
            Ok(DeviceRecord {
                id: device.id,
                name: device.name,
                os: device.os,
                arch: device.arch,
                app_version: device.app_version,
                first_seen_at: timestamp_from_db(device.first_seen_at),
                last_seen_at: timestamp_from_db(device.last_seen_at),
                revoked_at: device.revoked_at.map(timestamp_from_db),
                sessions: held,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::DeviceSessionStatus;

    #[test]
    fn every_stored_status_decodes_back_to_itself() {
        for status in DeviceSessionStatus::ALL {
            assert_eq!(
                DeviceSessionStatus::from_db(status.as_str()).ok(),
                Some(status)
            );
        }
        assert!(
            DeviceSessionStatus::from_db("expired").is_err(),
            "a status nothing on the device can distinguish must not decode"
        );
    }

    #[test]
    fn the_tokens_are_lowercase_and_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for status in DeviceSessionStatus::ALL {
            let token = status.as_str();
            assert_eq!(
                token,
                token.to_lowercase(),
                "the column CHECK in migration 0042 lists these verbatim: {token}"
            );
            assert!(
                seen.insert(token),
                "two statuses spelled {token} would make the column ambiguous"
            );
        }
    }
}
