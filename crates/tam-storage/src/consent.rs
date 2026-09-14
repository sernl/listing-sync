//! The seller's explicit permission for a marketplace that publishes no
//! official API.
//!
//! D1 puts every no-API marketplace request on the seller's own device under
//! the seller's own sign-in. The founder's rule is that this happens only
//! after the seller has read what the connection does and agreed to it, so
//! the record here is the agreement: one standing grant per organisation and
//! marketplace, carrying the version of the notice the seller read.
//!
//! Per organisation rather than per device, deliberately. A seller with two
//! machines agrees once and both read the same answer; a device could hold a
//! private "yes" in its own storage, and a device is exactly the party whose
//! word this record exists to stop being the only evidence.
//!
//! Append-only. A withdrawal fills `withdrawn_*` on the standing row, and a
//! later grant is a new row, so the Account page can show the seller every
//! decision they made and when.

use sqlx::PgPool;
use tam_types::{Marketplace, OrgId, Timestamp, Uuid};

use crate::codec::{
    marketplace_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::connections::marketplace_from_db;
use crate::{pin_org, StorageError};

/// One grant, standing or withdrawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentRecord {
    pub marketplace: Marketplace,
    pub notice_version: String,
    pub granted_by: Uuid,
    pub granted_at: Timestamp,
    pub withdrawn_at: Option<Timestamp>,
}

struct Row {
    marketplace: String,
    notice_version: String,
    granted_by: uuid::Uuid,
    granted_at: chrono::DateTime<chrono::Utc>,
    withdrawn_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Row {
    fn record(self) -> Result<ConsentRecord, StorageError> {
        Ok(ConsentRecord {
            marketplace: marketplace_from_db(&self.marketplace)?,
            notice_version: self.notice_version,
            granted_by: uuid_from_db(self.granted_by),
            granted_at: timestamp_from_db(self.granted_at),
            withdrawn_at: self.withdrawn_at.map(timestamp_from_db),
        })
    }
}

pub struct ConsentRepo {
    pool: PgPool,
}

impl ConsentRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The grant standing on `notice_version` exactly, else `None`.
    ///
    /// A grant on any other version answers `None` too: the seller agreed to
    /// a notice that no longer says what the current one says, and the
    /// console asks them to read the current one.
    pub async fn standing(
        &self,
        org: OrgId,
        marketplace: Marketplace,
        notice_version: &str,
    ) -> Result<Option<ConsentRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_as!(
            Row,
            "SELECT marketplace, notice_version, granted_by, granted_at, withdrawn_at \
             FROM marketplace_consent \
             WHERE org_id = $1 AND marketplace = $2 AND notice_version = $3 \
               AND withdrawn_at IS NULL",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
            notice_version,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(Row::record).transpose()
    }

    /// Every row this organisation holds, newest grant first.
    pub async fn history(&self, org: OrgId) -> Result<Vec<ConsentRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            Row,
            "SELECT marketplace, notice_version, granted_by, granted_at, withdrawn_at \
             FROM marketplace_consent \
             WHERE org_id = $1 \
             ORDER BY granted_at DESC, marketplace",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter().map(Row::record).collect()
    }

    /// Records a standing grant.
    ///
    /// Idempotent on the version: a grant standing on the same version is
    /// returned unchanged. A grant standing on another version is withdrawn
    /// by the same actor in the same transaction, so the standing index never
    /// sees two.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant and marketplace, and the grant carries the \
                  notice it was read on, who made it and when; a struct over those five would \
                  name the call"
    )]
    pub async fn grant(
        &self,
        org: OrgId,
        marketplace: Marketplace,
        notice_version: &str,
        by: Uuid,
        at: Timestamp,
    ) -> Result<ConsentRecord, StorageError> {
        let instant = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let standing = sqlx::query_as!(
            Row,
            "SELECT marketplace, notice_version, granted_by, granted_at, withdrawn_at \
             FROM marketplace_consent \
             WHERE org_id = $1 AND marketplace = $2 AND withdrawn_at IS NULL \
             FOR UPDATE",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = standing {
            if row.notice_version == notice_version {
                tx.commit().await?;
                return row.record();
            }
            sqlx::query!(
                "UPDATE marketplace_consent SET withdrawn_by = $3, withdrawn_at = $4 \
                 WHERE org_id = $1 AND marketplace = $2 AND withdrawn_at IS NULL",
                uuid_to_db(org.0),
                marketplace_to_db(marketplace),
                uuid_to_db(by),
                instant,
            )
            .execute(&mut *tx)
            .await?;
        }
        let row = sqlx::query_as!(
            Row,
            "INSERT INTO marketplace_consent \
             (org_id, marketplace, notice_version, granted_by, granted_at) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING marketplace, notice_version, granted_by, granted_at, withdrawn_at",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
            notice_version,
            uuid_to_db(by),
            instant,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        row.record()
    }

    /// Withdraws the standing grant, whatever version it carries.
    ///
    /// `Ok(None)` when none stands, which a second press of the same button
    /// reaches and which is not a fault.
    pub async fn withdraw(
        &self,
        org: OrgId,
        marketplace: Marketplace,
        by: Uuid,
        at: Timestamp,
    ) -> Result<Option<ConsentRecord>, StorageError> {
        let instant = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_as!(
            Row,
            "UPDATE marketplace_consent SET withdrawn_by = $3, withdrawn_at = $4 \
             WHERE org_id = $1 AND marketplace = $2 AND withdrawn_at IS NULL \
             RETURNING marketplace, notice_version, granted_by, granted_at, withdrawn_at",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
            uuid_to_db(by),
            instant,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(Row::record).transpose()
    }
}
