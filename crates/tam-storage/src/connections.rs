//! The connections surface for the API: the per-marketplace link states the
//! client's connections page renders, the four-state status derived from
//! them, and the lifecycle audit's writer.
//!
//! Revocation itself travels through the broker — the only role that can
//! tombstone the vault — so no connection write lives here. The audit is the
//! exception and only half of one: this is the writer the application and
//! engine roles use, and the broker writes its own rows with its own role
//! rather than depending on this crate across the privilege boundary.

use sqlx::PgPool;
use sqlx::{Postgres, Transaction};
use tam_types::{
    connection_status, Actor, ConnectionEvent, ConnectionHealth, ConnectionId, ConnectionState,
    ConnectionStatus, Marketplace, OrgId, Timestamp,
};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRow {
    pub id: ConnectionId,
    pub marketplace: Marketplace,
    pub state: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// What the seller is told, derived from `state` and the verification
    /// columns together rather than from `state` alone. Derived here, at the
    /// one place that reads the row, so no caller can render the link state
    /// as though it were a health check.
    pub status: ConnectionStatus,
}

pub struct ConnectionRepo {
    pool: PgPool,
}

impl ConnectionRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The tenant's connections, each carrying its derived status.
    ///
    /// `now` is passed in rather than read here, for the reason the rest of
    /// this workspace passes clocks in: a status that depends on a clock the
    /// function reads itself cannot be tested at a boundary.
    pub async fn list(
        &self,
        org: OrgId,
        now: Timestamp,
    ) -> Result<Vec<ConnectionRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, marketplace, state, created_at, updated_at, \
                    session_verified_at, session_refresh_after, refresh_failures \
             FROM connection WHERE org_id = $1 ORDER BY marketplace",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let state = ConnectionState::from_db(&row.state).ok_or_else(|| {
                    StorageError::CorruptRow {
                        reason: format!("unknown connection state {:?}", row.state),
                    }
                })?;
                let status = connection_status(
                    &ConnectionHealth {
                        state,
                        session_verified_at: row.session_verified_at.map(timestamp_from_db),
                        session_refresh_after: row.session_refresh_after.map(timestamp_from_db),
                        refresh_failures: row.refresh_failures,
                    },
                    now,
                );
                Ok(ConnectionRow {
                    id: ConnectionId(uuid_from_db(row.id)),
                    marketplace: marketplace_from_db(&row.marketplace)?,
                    state: row.state,
                    created_at: timestamp_from_db(row.created_at),
                    updated_at: timestamp_from_db(row.updated_at),
                    status,
                })
            })
            .collect()
    }
}

/// One lifecycle event, as the writer states it.
///
/// A struct rather than six positional arguments: four of them are an id, an
/// enum, an optional string and an instant, which is exactly the shape a
/// transposition survives compilation in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionEventRecord<'a> {
    pub org: OrgId,
    pub connection: ConnectionId,
    pub event: ConnectionEvent,
    pub actor: Actor,
    pub detail: Option<&'a str>,
    pub at: Timestamp,
}

/// Appends one row to the connection lifecycle audit, inside a caller's
/// transaction so the record and the state change it describes land or fail
/// together. A lifecycle audit written in its own transaction would drift
/// from the row it audits at exactly the moment worth auditing: a crash
/// between the two.
pub async fn record_connection_event(
    tx: &mut Transaction<'_, Postgres>,
    record: &ConnectionEventRecord<'_>,
) -> Result<(), StorageError> {
    sqlx::query!(
        "INSERT INTO connection_audit \
         (org_id, connection_id, event, actor_kind, actor_id, detail, at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        uuid_to_db(record.org.0),
        uuid_to_db(record.connection.0),
        record.event.as_str(),
        record.actor.kind(),
        record.actor.id(),
        record.detail,
        timestamp_to_db(record.at)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The lifecycle audit as a repository, for callers that hold a pool rather
/// than a transaction.
pub struct ConnectionAudit {
    pool: PgPool,
}

impl ConnectionAudit {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records one event under the tenant pin.
    pub async fn record(&self, record: &ConnectionEventRecord<'_>) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, record.org).await?;
        record_connection_event(&mut tx, record).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every recorded event for one connection, newest first.
    pub async fn history(
        &self,
        org: OrgId,
        connection: ConnectionId,
    ) -> Result<Vec<ConnectionAuditRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT event, actor_kind, actor_id, detail, at FROM connection_audit \
             WHERE org_id = $1 AND connection_id = $2 ORDER BY at DESC, id DESC",
            uuid_to_db(org.0),
            uuid_to_db(connection.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| ConnectionAuditRow {
                event: row.event,
                actor_kind: row.actor_kind,
                actor_id: row.actor_id,
                detail: row.detail,
                at: timestamp_from_db(row.at),
            })
            .collect())
    }
}

/// One recorded lifecycle event, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionAuditRow {
    pub event: String,
    pub actor_kind: String,
    pub actor_id: Option<String>,
    pub detail: Option<String>,
    pub at: Timestamp,
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
