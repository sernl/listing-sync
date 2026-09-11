//! The connections surface for the API: the per-marketplace link states the
//! client's connections page renders, the four-state status derived from
//! them, and the lifecycle audit's writer.
//!
//! Revocation is a control-plane write and lives here. It used to travel
//! through the session broker, which held the credential vault and was the
//! only role that could tombstone it; D1 removed the vault along with every
//! server-side seller session, so what is left to revoke is the connection
//! row itself and the application role owns it.

use sqlx::PgPool;
use sqlx::{Postgres, Transaction};
use tam_types::{
    connection_status, ConnectionEvent, ConnectionHealth, ConnectionId, ConnectionState,
    ConnectionStatus, Marketplace, OrgId, Stamp, Timestamp,
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
    /// The market this connection authors into, ISO 3166-1 alpha-2, defaulted
    /// to `GB`. Read for Tes, where it selects the taxonomy tree and the age
    /// vocabulary, and ignored elsewhere. A column rather than an inventory
    /// variant, so a seller on a second Tes market is a data change
    /// (`docs/design/decisions.md`, 2026-09-12).
    pub country: String,
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
            "SELECT id, marketplace, state, country, created_at, updated_at, \
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
                    country: row.country,
                })
            })
            .collect()
    }

    /// Marks one connection revoked, answering whether a row moved.
    ///
    /// The guard makes a second revocation a no-op rather than a second audit
    /// row, and it is what lets a caller report a count without reading the
    /// row back. A connection the tenant does not have also answers `false`,
    /// because the pin means this statement cannot see another tenant's row
    /// and must not distinguish one from a row that is not there.
    ///
    /// Terminal by construction rather than by convention. The guard is on the
    /// check-in's *upgrade*, not its downgrade: `derive_link` lifts a
    /// connection to `linked` through an `ON CONFLICT ... DO UPDATE` whose
    /// `WHERE connection.state IN ('unlinked', 'linking', 'needs_reauth')`
    /// does not name `revoked`, so no live session a seller's machines report
    /// can restore it. The downgrade is guarded too, but only ever writes
    /// `needs_reauth` and so could not have lifted anything anyway.
    ///
    /// `ConnectionEvent::Revoked` is documented as a stored credential being
    /// tombstoned and nothing is tombstoned here, so the detail carries the
    /// distinction the vocabulary cannot — the same arrangement `derive_link`
    /// makes for `Linked`.
    pub async fn revoke(
        &self,
        org: OrgId,
        connection: ConnectionId,
        stamp: Stamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let moved = sqlx::query_scalar!(
            "UPDATE connection SET state = 'revoked', updated_at = now() \
             WHERE org_id = $1 AND id = $2 AND state <> 'revoked' \
             RETURNING id",
            uuid_to_db(org.0),
            uuid_to_db(connection.0),
        )
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if moved {
            record_connection_event(
                &mut tx,
                &ConnectionEventRecord {
                    org,
                    connection,
                    event: ConnectionEvent::Revoked,
                    detail: Some("seller-requested"),
                    stamp,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(moved)
    }

    /// Marks one connection unlinked at the seller's own request, answering
    /// whether a row moved.
    ///
    /// The reversible half of the pair. `revoke` above is terminal by
    /// construction, because `derive_link` will not lift a connection out of
    /// `revoked`; `unlinked` is one of the three states it does lift, so a
    /// seller who disconnects and connects again takes the same path they took
    /// the first time and nothing has to undo this write.
    ///
    /// Both terminal states are excluded from the guard rather than only
    /// `unlinked`. Writing `unlinked` over a `revoked` row would hand back the
    /// reconnect that revocation exists to withhold, so a disconnect finds
    /// nothing to move there and says so.
    ///
    /// Every lease stops without anything else being written: both branches of
    /// the claim scan require `c.state = 'linked'`, so the next claim passes
    /// this marketplace over. An item already leased runs to its lease expiry,
    /// which is the same posture device revocation takes.
    pub async fn unlink(
        &self,
        org: OrgId,
        connection: ConnectionId,
        stamp: Stamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let moved = sqlx::query_scalar!(
            "UPDATE connection SET state = 'unlinked', updated_at = now() \
             WHERE org_id = $1 AND id = $2 AND state NOT IN ('unlinked', 'revoked') \
             RETURNING id",
            uuid_to_db(org.0),
            uuid_to_db(connection.0),
        )
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if moved {
            record_connection_event(
                &mut tx,
                &ConnectionEventRecord {
                    org,
                    connection,
                    event: ConnectionEvent::Unlinked,
                    detail: Some("seller-requested"),
                    stamp,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(moved)
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
    pub detail: Option<&'a str>,
    pub stamp: Stamp,
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
        record.stamp.actor.kind(),
        record.stamp.actor.id(),
        record.detail,
        timestamp_to_db(record.stamp.at)?,
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
