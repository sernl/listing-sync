//! The organisation row itself: the name the tenant carries, which the
//! settings surface reads and renames.
//!
//! `organisation` is the tenancy root and carries no row-level-security
//! policy, unlike every table that references it. It cannot: the signup path
//! inserts the row before any `app.current_org` exists to pin, and the
//! engine's cross-tenant scan reads the table to enumerate tenants. So the
//! `WHERE id = $1` clause below is the whole tenant fence here rather than
//! the redundant filter it is in the repositories the policies also guard.

use sqlx::PgPool;
use tam_types::OrgId;

use crate::codec::{uuid_from_db, uuid_to_db};
use crate::StorageError;

/// One organisation as the settings surface sees it. The id is read back
/// from the row rather than echoed from the argument, so a read that somehow
/// answered with another tenant's row would be visible rather than masked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgRecord {
    pub id: OrgId,
    pub name: String,
}

pub struct OrgRepo {
    pool: PgPool,
}

impl OrgRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The calling organisation's own row, or `None` when no such row exists.
    pub async fn get(&self, org: OrgId) -> Result<Option<OrgRecord>, StorageError> {
        let row = sqlx::query!(
            "SELECT id, name FROM organisation WHERE id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| OrgRecord {
            id: OrgId(uuid_from_db(row.id)),
            name: row.name,
        }))
    }

    /// Renames one organisation, answering whether a row was there to rename.
    /// The name is the caller's to validate; storage stores what it is given.
    pub async fn rename(&self, org: OrgId, name: &str) -> Result<bool, StorageError> {
        let done = sqlx::query!(
            "UPDATE organisation SET name = $2 WHERE id = $1",
            uuid_to_db(org.0),
            name,
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }
}
