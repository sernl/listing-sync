//! The connections read side for the API: the per-marketplace link states
//! the client's connections page renders. Revocation itself travels through
//! the broker — the only role that can tombstone the vault — so no write
//! lives here.

use sqlx::PgPool;
use tam_types::{ConnectionId, Marketplace, OrgId, Timestamp};

use crate::codec::{timestamp_from_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRow {
    pub id: ConnectionId,
    pub marketplace: Marketplace,
    pub state: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub struct ConnectionRepo {
    pool: PgPool,
}

impl ConnectionRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, org: OrgId) -> Result<Vec<ConnectionRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, marketplace, state, created_at, updated_at \
             FROM connection WHERE org_id = $1 ORDER BY marketplace",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ConnectionRow {
                    id: ConnectionId(uuid_from_db(row.id)),
                    marketplace: marketplace_from_db(&row.marketplace)?,
                    state: row.state,
                    created_at: timestamp_from_db(row.created_at),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }
}

pub(crate) fn marketplace_from_db(raw: &str) -> Result<Marketplace, StorageError> {
    match raw {
        "tes" => Ok(Marketplace::Tes),
        "etsy" => Ok(Marketplace::Etsy),
        "tpt" => Ok(Marketplace::Tpt),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown marketplace {other:?}"),
        }),
    }
}
