//! The platform-operator marking: who may read across tenants, answered from
//! a row in our own database rather than from anything a token asserts.
//!
//! Everything here runs on the application pool. The backoffice role is
//! granted nothing on `platform_operator` (migration 0037), so the pool that
//! performs the cross-tenant reads cannot read, let alone write, the list of
//! who is allowed to perform them.

use sqlx::PgPool;
use tam_types::{Timestamp, UserId};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::StorageError;

/// One operator as the one-shot's listing renders them, revoked ones
/// included: a withdrawn grant is part of the record, not an absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorRecord {
    pub user: UserId,
    pub email: String,
    pub granted_at: Timestamp,
    pub granted_by: String,
    pub revoked_at: Option<Timestamp>,
}

pub struct OperatorRepo {
    pool: PgPool,
}

impl OperatorRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Whether this user holds a grant nobody has withdrawn. The single
    /// question the request path asks, and the reason revocation takes effect
    /// on the next request rather than on the next login.
    pub async fn is_active(&self, user: UserId) -> Result<bool, StorageError> {
        let row = sqlx::query!(
            "SELECT 1 AS \"present!\" FROM platform_operator \
             WHERE user_id = $1 AND revoked_at IS NULL",
            uuid_to_db(user.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Grants the marking, reinstating a revoked one in place.
    ///
    /// One human keeps one row, so reinstatement clears `revoked_at` and
    /// restamps the grant rather than inserting a second marking whose
    /// predecessor would still read as revoked. The foreign key means a user
    /// id nobody has provisioned fails here instead of creating a marking
    /// that names nobody.
    pub async fn grant(
        &self,
        user: UserId,
        granted_by: &str,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO platform_operator (user_id, granted_at, granted_by) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (user_id) DO UPDATE \
                 SET granted_at = EXCLUDED.granted_at, \
                     granted_by = EXCLUDED.granted_by, \
                     revoked_at = NULL",
            uuid_to_db(user.0),
            timestamp_to_db(at)?,
            granted_by,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Withdraws a grant, answering whether there was an active one to
    /// withdraw. Revoking twice is not an error; it is simply `false` the
    /// second time, and the first revocation's instant survives.
    pub async fn revoke(&self, user: UserId, at: Timestamp) -> Result<bool, StorageError> {
        let done = sqlx::query!(
            "UPDATE platform_operator SET revoked_at = $2 \
             WHERE user_id = $1 AND revoked_at IS NULL",
            uuid_to_db(user.0),
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Every marking ever made, oldest grant first.
    pub async fn list(&self) -> Result<Vec<OperatorRecord>, StorageError> {
        let rows = sqlx::query!(
            "SELECT o.user_id, u.email, o.granted_at, o.granted_by, o.revoked_at \
             FROM platform_operator o JOIN app_user u ON u.id = o.user_id \
             ORDER BY o.granted_at, o.user_id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| OperatorRecord {
                user: UserId(uuid_from_db(row.user_id)),
                email: row.email,
                granted_at: timestamp_from_db(row.granted_at),
                granted_by: row.granted_by,
                revoked_at: row.revoked_at.map(timestamp_from_db),
            })
            .collect())
    }
}
