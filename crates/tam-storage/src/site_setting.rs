//! Site-wide switches, one jsonb value per key (migration 0087).
//!
//! The repository stores values and does not interpret them: the shape of
//! `maintenance`, `theme` and `banner` is parsed in `tam-api`'s `site`
//! module, and `sale.*` in the discounts surface, so a key's reader owns its
//! shape and this layer never has to learn a new one.
//!
//! Everything here runs on the application pool, like the guide corpus: the
//! table is global and the backoffice role is granted nothing on it.

use sqlx::PgPool;
use tam_types::{Timestamp, UserId};

use crate::codec::{timestamp_to_db, uuid_to_db};
use crate::StorageError;

pub struct SiteSettingRepo {
    pool: PgPool,
}

impl SiteSettingRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The value stored under `key`, or `None` where nobody has set it.
    pub async fn get(&self, key: &str) -> Result<Option<serde_json::Value>, StorageError> {
        let row = sqlx::query!("SELECT value FROM site_setting WHERE key = $1", key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.value))
    }

    /// Every stored value among `keys`, in key order. A key nobody has set is
    /// absent from the answer rather than present as null.
    pub async fn many(
        &self,
        keys: &[&str],
    ) -> Result<Vec<(String, serde_json::Value)>, StorageError> {
        let keys: Vec<String> = keys.iter().map(|key| (*key).to_owned()).collect();
        let rows = sqlx::query!(
            "SELECT key, value FROM site_setting WHERE key = ANY($1) ORDER BY key",
            &keys,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| (row.key, row.value)).collect())
    }

    /// Every stored value whose key begins with `prefix`, in key order.
    ///
    /// The prefix is matched literally: `starts_with` rather than `LIKE`, so
    /// an underscore in it is an underscore and not a wildcard.
    pub async fn with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, serde_json::Value)>, StorageError> {
        let rows = sqlx::query!(
            "SELECT key, value FROM site_setting WHERE starts_with(key, $1) ORDER BY key",
            prefix,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| (row.key, row.value)).collect())
    }

    /// Stores `value` under `key`, replacing whatever was there, and records
    /// who wrote it and when.
    pub async fn set(
        &self,
        key: &str,
        value: &serde_json::Value,
        by: UserId,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO site_setting (key, value, updated_at, updated_by) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (key) DO UPDATE \
                 SET value = EXCLUDED.value, \
                     updated_at = EXCLUDED.updated_at, \
                     updated_by = EXCLUDED.updated_by",
            key,
            value,
            timestamp_to_db(at)?,
            uuid_to_db(by.0),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Removes `key`, answering whether there was a row to remove. Removing
    /// twice is not an error; the key reads as unset either way.
    pub async fn delete(&self, key: &str) -> Result<bool, StorageError> {
        let done = sqlx::query!("DELETE FROM site_setting WHERE key = $1", key)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }
}
