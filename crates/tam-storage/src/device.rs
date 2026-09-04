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
//!
//! The heartbeat is also where a connection becomes linked, which is why a
//! module about devices writes a `connection` row. D1 puts every no-API
//! marketplace session on the seller's own machine, so a device saying so is
//! the only evidence of one the server can ever hold: the check-in derives
//! `connection.state` from what this organisation's live devices report,
//! `linked` while at least one holds a connected session and `needs_reauth`
//! when the last one stops. It never writes `unlinked`, which is the state of
//! a connection no device has reported at all, and never writes over
//! `revoked`, because a revocation a device lifted by restarting would not be
//! the seller's decision any more.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tam_types::{
    ConnectionEvent, ConnectionId, Marketplace, OrgId, Stamp, SystemComponent, Timestamp,
};

use crate::codec::{
    marketplace_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::connections::{marketplace_from_db, record_connection_event, ConnectionEventRecord};
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

    /// Stamps the device seen, replaces what it holds, derives the link state
    /// of every marketplace that moves, and answers whether the device is
    /// revoked.
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

        for marketplace in replace_sessions(&mut tx, org, device, sessions, seen).await? {
            derive_link(&mut tx, org, &marketplace, at).await?;
        }
        tx.commit().await?;
        Ok(Some(DeviceHeartbeat {
            revoked_at: row.revoked_at.map(timestamp_from_db),
        }))
    }

    /// Whether this device is one the server will still answer, and holds a
    /// connected session for this marketplace.
    ///
    /// The same two conditions the work claim applies, asked as a question
    /// rather than embedded in a claim: the device is not revoked, and it
    /// reports that marketplace `connected` rather than merely recorded. A
    /// read the device branch needs because a capture is a marketplace request
    /// like any other and must not be handed to a machine that cannot make it.
    pub async fn holds_connected_session(
        &self,
        org: OrgId,
        device: &str,
        marketplace: Marketplace,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                 SELECT 1 FROM device_marketplace_session dms
                 JOIN device d ON d.org_id = dms.org_id AND d.id = dms.device_id
                 WHERE dms.org_id = $1 AND dms.device_id = $2
                   AND dms.marketplace = $3 AND dms.status = 'connected'
                   AND d.revoked_at IS NULL
               ) AS "held!""#,
            uuid_to_db(org.0),
            device,
            marketplace_to_db(marketplace),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(held)
    }

    /// Whether this tenant's entitlement still stands, in the sense D11 gives
    /// it and the work claim already applies.
    ///
    /// Not "has paid": an organisation that never subscribed has no row and is
    /// entitled, and what blocks is a plan that lapsed and stayed lapsed past
    /// the grace. Stated here as its own read because the claim expresses it
    /// only as a clause inside its own statement, and a second surface that
    /// spends a seller's marketplace budget needs the same gate rather than a
    /// second spelling of it.
    pub async fn entitlement_stands(
        &self,
        org: OrgId,
        grace_hours: i32,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let stands = sqlx::query_scalar!(
            r#"SELECT NOT EXISTS (
                 SELECT 1 FROM billing_subscription bs
                 WHERE bs.org_id = $1
                   AND bs.status NOT IN ('active', 'trialing')
                   AND bs.current_period_end IS NOT NULL
                   AND bs.current_period_end < now() - make_interval(hours => $2)
               ) AS "stands!""#,
            uuid_to_db(org.0),
            grace_hours,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stands)
    }

    /// The marketplaces this device may be granted right now, as decision D10's
    /// entitlement token names them.
    ///
    /// The work claim's own predicate asked as a question rather than embedded
    /// in a claim, and deliberately most of the same predicate. The grant set is
    /// a *superset* of what the claim will actually serve, never an equal set:
    /// the two conditions below are omitted, so a marketplace can be named here
    /// and still have every item refused.
    ///
    /// The over-approximation is the safe direction, and which direction is safe
    /// is a fact about this gate rather than a general principle. The gate on the
    /// device can only ever refuse — it grants nothing the server has not already
    /// decided, and the server re-decides on every claim — so a grant that is too
    /// wide costs a device one refused claim, while a grant that is too narrow
    /// stops a seller who is entitled from working at all. A later reader adding
    /// a condition to the claim should therefore ask whether omitting it here
    /// could ever make this set too *narrow*; if not, it does not belong here.
    ///
    /// Two of the claim's conditions are left out, each for a reason. A
    /// reported `connected` session is not one, because the device knows its
    /// own sessions better than its last report to us does, and refuses for
    /// want of one on its own — under a different reason, which the seller acts
    /// on differently; folding it in here would relabel "you are not logged in
    /// to that marketplace" as "you are not entitled". The per-item conditions
    /// are about an item rather than about entitlement.
    ///
    /// Halts are keyed on an inventory and a marketplace while the token's
    /// vocabulary is a marketplace alone, so a marketplace is granted when any
    /// of its inventories is unhalted. "Any" can never let a halted inventory
    /// be worked, because the claim still refuses per inventory; "all" would
    /// stop a seller working two healthy Tes inventories because a third was
    /// halted.
    pub async fn entitled_marketplaces(
        &self,
        org: OrgId,
        device: &str,
        grace_hours: i32,
    ) -> Result<Vec<Marketplace>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let granted = sqlx::query_scalar!(
            r#"SELECT DISTINCT mi.marketplace AS "marketplace!"
               FROM marketplace_inventory mi
               WHERE mi.transport_class = 'seller_device'
                 AND EXISTS (SELECT 1 FROM device d
                       WHERE d.org_id = $1 AND d.id = $2
                         AND d.revoked_at IS NULL)
                 AND NOT EXISTS (SELECT 1 FROM billing_subscription bs
                       WHERE bs.org_id = $1
                         AND bs.status NOT IN ('active', 'trialing')
                         AND bs.current_period_end IS NOT NULL
                         AND bs.current_period_end < now() - make_interval(hours => $3))
                 AND NOT EXISTS (SELECT 1 FROM org_halt oh WHERE oh.org_id = $1)
                 AND EXISTS (SELECT 1 FROM connection c
                       WHERE c.org_id = $1 AND c.marketplace = mi.marketplace
                         AND c.state = 'linked')
                 AND NOT EXISTS (SELECT 1 FROM inventory_halt ih
                       WHERE ih.inventory = mi.code AND ih.marketplace = mi.marketplace)
                 AND NOT EXISTS (SELECT 1 FROM org_inventory_halt oih
                       WHERE oih.org_id = $1 AND oih.inventory = mi.code
                         AND oih.marketplace = mi.marketplace)
               ORDER BY 1"#,
            uuid_to_db(org.0),
            device,
            grace_hours,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        granted.iter().map(|raw| marketplace_from_db(raw)).collect()
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

/// Replaces what one device holds, answering every marketplace whose link
/// state this check-in could have moved.
///
/// That is the ones it named together with the ones it stopped naming.
/// Dropping a marketplace from the report is how a device says it no longer
/// holds that session, so the delete arm has to derive as much as the upsert
/// arm does; a derivation over the named set alone would leave a connection
/// linked on a session nothing reports.
async fn replace_sessions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    device: &str,
    sessions: &[DeviceSessionReport<'_>],
    seen: DateTime<Utc>,
) -> Result<Vec<String>, StorageError> {
    let named: Vec<String> = sessions
        .iter()
        .map(|session| marketplace_to_db(session.marketplace).to_owned())
        .collect();
    let dropped = sqlx::query_scalar!(
        "DELETE FROM device_marketplace_session \
         WHERE org_id = $1 AND device_id = $2 AND marketplace <> ALL($3) \
         RETURNING marketplace",
        uuid_to_db(org.0),
        device,
        &named,
    )
    .fetch_all(&mut **tx)
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
        .execute(&mut **tx)
        .await?;
    }
    let mut touched = named;
    touched.extend(dropped);
    touched.sort_unstable();
    touched.dedup();
    Ok(touched)
}

/// Derives one marketplace's link state from what this organisation's live
/// devices report, and records the transition where there was one.
///
/// The quantifier is existential deliberately: a seller with two machines has
/// a linked connection while either holds a session, and loses it when the
/// last one stops. A revoked device is not a live one -- it has been asked to
/// forget its sessions, so counting what it still reports would hold a
/// connection open on a machine we have disowned, which is the reading the
/// claim's own device predicate already takes.
async fn derive_link(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    marketplace: &str,
    at: Timestamp,
) -> Result<(), StorageError> {
    let connected = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM device_marketplace_session dms
             JOIN device d ON d.org_id = dms.org_id AND d.id = dms.device_id
             WHERE dms.org_id = $1 AND dms.marketplace = $2
               AND dms.status = 'connected' AND d.revoked_at IS NULL
           ) AS "connected!""#,
        uuid_to_db(org.0),
        marketplace,
    )
    .fetch_one(&mut **tx)
    .await?;
    // Each statement answers a row only where it moved one, so the audit
    // records transitions rather than check-ins: a device reporting the same
    // session every thirty seconds writes one row, not one per beat.
    let moved = if connected {
        // The only path that may create a connection. A seller who declared
        // authorship before connecting anything left an `unlinked` row here,
        // and a live session is what turns it into one the claim will serve.
        sqlx::query_scalar!(
            "INSERT INTO connection \
             (org_id, id, marketplace, state, created_at, updated_at) \
             VALUES ($1, $2, $3, 'linked', now(), now()) \
             ON CONFLICT (org_id, marketplace) DO UPDATE \
                 SET state = 'linked', updated_at = now() \
                 WHERE connection.state IN ('unlinked', 'linking', 'needs_reauth') \
             RETURNING id",
            uuid_to_db(org.0),
            uuid::Uuid::new_v4(),
            marketplace,
        )
        .fetch_optional(&mut **tx)
        .await?
    } else {
        // Only ever the downgrade of a link that stood. A marketplace the
        // seller never linked stays `unlinked`, because asking them to re-link
        // something they never linked names the wrong remedy, and `revoked` is
        // a decision no check-in may lift.
        sqlx::query_scalar!(
            "UPDATE connection SET state = 'needs_reauth', updated_at = now() \
             WHERE org_id = $1 AND marketplace = $2 AND state IN ('linking', 'linked') \
             RETURNING id",
            uuid_to_db(org.0),
            marketplace,
        )
        .fetch_optional(&mut **tx)
        .await?
    };
    let Some(connection) = moved else {
        return Ok(());
    };
    record_connection_event(
        tx,
        &ConnectionEventRecord {
            org,
            connection: ConnectionId(uuid_from_db(connection)),
            event: if connected {
                ConnectionEvent::Linked
            } else {
                ConnectionEvent::NeedsReauth
            },
            // The vocabulary's `linked` was written for the vault sealing a
            // credential, and nothing is sealed here. The detail is what
            // separates the two writers inside one log.
            detail: Some("device-reported"),
            stamp: Stamp::system(SystemComponent::Device, at),
        },
    )
    .await
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
