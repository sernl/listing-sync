//! What an organisation is entitled to, derived from its grants.
//!
//! The plan is a query rather than a column: migration 0069 records one row
//! per grant, and the plan an organisation holds is the strongest of the rows
//! that are neither revoked nor expired. An organisation with no rows holds
//! `free`, which is the same fail-closed direction the lapsed-subscription
//! read takes.
//!
//! Strongest rather than newest, because the two orders disagree exactly
//! where it matters: a seller who bought a one-off Catalogue Import and then
//! subscribed holds two live grants, and serving the newer one would answer
//! whichever webhook happened to land last. `Plan::strength` is the order,
//! declared in tam-limits beside the capability table it selects.
//!
//! [`EntitlementRepo::current`] is on the request path — the `OrgContext`
//! extractor calls it before any handler runs — so it is one indexed read of
//! one organisation's live rows, and nothing else.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::PgPool;
use tam_limits::Plan;
use tam_types::{OrgId, Timestamp, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// Who made a grant, which decides how it is attributed and who may take it
/// back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantedBy {
    /// A purchase. Attributable to `source_ref`, which is Paddle's own
    /// subscription or transaction identifier.
    Paddle,
    /// A backoffice grant. Attributable to `grantor_user` and to the reason
    /// the operator typed.
    Operator,
}

impl GrantedBy {
    pub const ALL: [Self; 2] = [Self::Paddle, Self::Operator];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paddle => "paddle",
            Self::Operator => "operator",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == raw)
    }
}

/// The entitlement one organisation holds right now.
///
/// Every field except `plan` is absent for an organisation with no grant at
/// all, which is the ordinary state of a free account that has never bought
/// anything. `granted_by` is therefore an `Option` rather than a default:
/// nobody granted `free`, and naming an operator or Paddle as its source
/// would state something false on the Account page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub plan: Plan,
    pub rung: Option<u32>,
    pub granted_by: Option<GrantedBy>,
    pub granted_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub source_ref: Option<String>,
}

impl Grant {
    /// What an organisation with no live grant holds.
    #[must_use]
    pub const fn free() -> Self {
        Self {
            plan: Plan::Free,
            rung: None,
            granted_by: None,
            granted_at: None,
            expires_at: None,
            source_ref: None,
        }
    }
}

/// One grant as recorded, revocations and expiries included. What the
/// backoffice reads; the request path reads [`Grant`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRecord {
    pub id: Uuid,
    pub plan: Plan,
    pub rung: Option<u32>,
    pub granted_by: GrantedBy,
    pub grantor_user: Option<Uuid>,
    pub reason: Option<String>,
    pub source_ref: Option<String>,
    pub granted_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
}

/// A grant about to be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGrant<'a> {
    pub id: Uuid,
    pub plan: Plan,
    pub rung: Option<u32>,
    pub granted_by: GrantedBy,
    pub grantor_user: Option<Uuid>,
    pub reason: Option<&'a str>,
    pub source_ref: Option<&'a str>,
    pub granted_at: Timestamp,
    pub expires_at: Option<Timestamp>,
}

pub struct EntitlementRepo {
    pool: PgPool,
}

impl EntitlementRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The plan this organisation holds at `now`.
    ///
    /// A row counts when it is unrevoked and either has no expiry or has one
    /// still ahead. Among those, the strongest plan wins, and ties are broken
    /// by the later grant so a re-grant of the same plan carries the newer
    /// rung and expiry.
    ///
    /// The ordering is done in Rust rather than in SQL because the precedence
    /// lives in `Plan::strength`, and a `CASE` restating it in the query
    /// would be the second copy of an ordering whose whole purpose is that
    /// there is one. The row set is one organisation's live grants, which is
    /// a handful.
    pub async fn current(&self, org: OrgId, now: Timestamp) -> Result<Grant, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT plan, rung, granted_by, granted_at, expires_at, source_ref \
               FROM entitlement_grant \
              WHERE org_id = $1 \
                AND revoked_at IS NULL \
                AND (expires_at IS NULL OR expires_at > $2)",
            uuid_to_db(org.0),
            timestamp_to_db(now)?,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut strongest = Grant::free();
        let mut best: Option<(u8, Timestamp)> = None;
        for row in rows {
            let plan = Plan::parse(&row.plan).ok_or_else(|| StorageError::CorruptRow {
                reason: format!("entitlement_grant.plan holds the unknown plan {}", row.plan),
            })?;
            let granted_by =
                GrantedBy::parse(&row.granted_by).ok_or_else(|| StorageError::CorruptRow {
                    reason: format!(
                        "entitlement_grant.granted_by holds the unknown source {}",
                        row.granted_by
                    ),
                })?;
            let granted_at = timestamp_from_db(row.granted_at);
            let rank = (plan.strength(), granted_at);
            if best.is_some_and(|held| held >= rank) {
                continue;
            }
            best = Some(rank);
            strongest = Grant {
                plan,
                rung: row.rung.map(rung_from_db),
                granted_by: Some(granted_by),
                granted_at: Some(granted_at),
                expires_at: row.expires_at.map(timestamp_from_db),
                source_ref: row.source_ref,
            };
        }
        Ok(strongest)
    }

    /// Every grant this organisation has ever held, newest first.
    pub async fn history(&self, org: OrgId) -> Result<Vec<GrantRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, plan, rung, granted_by, grantor_user, reason, source_ref, \
                    granted_at, expires_at, revoked_at \
               FROM entitlement_grant WHERE org_id = $1 \
              ORDER BY granted_at DESC, id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(GrantRecord {
                    id: uuid_from_db(row.id),
                    plan: Plan::parse(&row.plan).ok_or_else(|| StorageError::CorruptRow {
                        reason: format!(
                            "entitlement_grant.plan holds the unknown plan {}",
                            row.plan
                        ),
                    })?,
                    rung: row.rung.map(rung_from_db),
                    granted_by: GrantedBy::parse(&row.granted_by).ok_or_else(|| {
                        StorageError::CorruptRow {
                            reason: format!(
                                "entitlement_grant.granted_by holds the unknown source {}",
                                row.granted_by
                            ),
                        }
                    })?,
                    grantor_user: row.grantor_user.map(uuid_from_db),
                    reason: row.reason,
                    source_ref: row.source_ref,
                    granted_at: timestamp_from_db(row.granted_at),
                    expires_at: row.expires_at.map(timestamp_from_db),
                    revoked_at: row.revoked_at.map(timestamp_from_db),
                })
            })
            .collect()
    }

    /// Records a grant.
    pub async fn grant(&self, org: OrgId, new: &NewGrant<'_>) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO entitlement_grant \
             (org_id, id, plan, rung, granted_by, grantor_user, reason, source_ref, \
              granted_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            uuid_to_db(org.0),
            uuid_to_db(new.id),
            new.plan.as_str(),
            new.rung.map(rung_to_db),
            new.granted_by.as_str(),
            new.grantor_user.map(uuid_to_db),
            new.reason,
            new.source_ref,
            timestamp_to_db(new.granted_at)?,
            new.expires_at.map(timestamp_to_db).transpose()?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The live Paddle grant for one subscription or transaction, if this
    /// organisation holds one.
    ///
    /// The webhook reads this before writing, so a `subscription.updated`
    /// stream renews one grant rather than accumulating a row per delivery.
    pub async fn paddle_grant(
        &self,
        org: OrgId,
        source_ref: &str,
    ) -> Result<Option<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_scalar!(
            "SELECT id FROM entitlement_grant \
              WHERE org_id = $1 AND granted_by = 'paddle' AND source_ref = $2 \
                AND revoked_at IS NULL",
            uuid_to_db(org.0),
            source_ref,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(uuid_from_db))
    }

    /// Moves a live grant's expiry, which is how a cancelled subscription
    /// stops entitling at its period end rather than at the moment Paddle
    /// said so. Answers whether a row moved.
    pub async fn set_expiry(
        &self,
        org: OrgId,
        grant: Uuid,
        expires_at: Option<Timestamp>,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let moved = sqlx::query!(
            "UPDATE entitlement_grant SET expires_at = $3 \
              WHERE org_id = $1 AND id = $2 AND revoked_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(grant),
            expires_at.map(timestamp_to_db).transpose()?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(moved == 1)
    }

    /// Revokes a grant. Answers whether one was revoked, so a second revoke
    /// of the same grant is a not-found rather than a silent success.
    pub async fn revoke(
        &self,
        org: OrgId,
        grant: Uuid,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let revoked = sqlx::query!(
            "UPDATE entitlement_grant SET revoked_at = $3 \
              WHERE org_id = $1 AND id = $2 AND revoked_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(grant),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(revoked == 1)
    }

    /// What this organisation has used of what its plan allows, as one read.
    ///
    /// One query rather than six repositories, because this answers one page
    /// — the Account page's plan panel — and a round trip per counter would
    /// make that page six.
    pub async fn usage(&self, org: OrgId, now: Timestamp) -> Result<Usage, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let (opened, resets) = month_bounds(now)?;
        let row = sqlx::query!(
            r#"SELECT
                 (SELECT count(*) FROM product
                   WHERE org_id = $1 AND deleted_at IS NULL)          AS "resources!",
                 (SELECT count(DISTINCT marketplace) FROM connection
                   WHERE org_id = $1 AND state = 'linked')            AS "marketplaces!",
                 (SELECT count(*) FROM sync_request_resource r
                    JOIN sync_request s
                      ON s.org_id = r.org_id AND s.id = r.request_id
                   WHERE r.org_id = $1
                     AND s.disposition IN ('sync', 'migrate')
                     AND s.requested_at >= $2)                        AS "migrations!",
                 (SELECT count(*) FROM resource_template
                   WHERE org_id = $1)                                 AS "templates!",
                 (SELECT count(*) FROM label
                   WHERE org_id = $1)                                 AS "labels!",
                 (SELECT count(*) FROM collection
                   WHERE org_id = $1)                                 AS "collections!",
                 (SELECT count(*) FROM device
                   WHERE org_id = $1 AND revoked_at IS NULL)          AS "devices!""#,
            uuid_to_db(org.0),
            opened,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Usage {
            resources: row.resources,
            marketplaces: row.marketplaces,
            migrations_this_month: row.migrations,
            migrations_reset_at: resets,
            templates: row.templates,
            labels: row.labels,
            collections: row.collections,
            devices: row.devices,
        })
    }

    /// How many resources this organisation's copies and moves have named
    /// since the first of the current UTC month.
    ///
    /// The cap counts resources rather than requests, because a per-request
    /// cap is gamed by batching. A request whose resources the device has not
    /// enumerated yet contributes the rows it has, which is the honest
    /// running total: the gate re-reads it on the next submit.
    pub async fn migrations_used_this_month(
        &self,
        org: OrgId,
        now: Timestamp,
    ) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let used = sqlx::query_scalar!(
            r#"SELECT count(*) AS "used!" FROM sync_request_resource r
                 JOIN sync_request s ON s.org_id = r.org_id AND s.id = r.request_id
                WHERE r.org_id = $1
                  AND s.disposition IN ('sync', 'migrate')
                  AND s.requested_at >= $2"#,
            uuid_to_db(org.0),
            month_bounds(now)?.0,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(used)
    }
}

/// What one organisation has used of each metered capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub resources: i64,
    pub marketplaces: i64,
    pub migrations_this_month: i64,
    /// When the migration counter next returns to zero: the first instant of
    /// the following UTC month. Carried so a refusal can say when the seller
    /// may try again rather than only that they may not now.
    pub migrations_reset_at: Timestamp,
    pub templates: i64,
    pub labels: i64,
    pub collections: i64,
    pub devices: i64,
}

/// The current UTC month's opening instant, and the instant the counter
/// resets at.
///
/// UTC rather than the seller's zone, and stated as such wherever a refusal
/// quotes the reset: a per-tenant month would need a per-tenant zone nobody
/// has asked for, and a boundary that moves with whoever is reading is worse
/// than one that is the same everywhere.
fn month_bounds(now: Timestamp) -> Result<(DateTime<Utc>, Timestamp), StorageError> {
    let at = timestamp_to_db(now)?;
    let date = at.date_naive();
    let opened = NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
        .and_then(|first| first.and_hms_opt(0, 0, 0))
        .ok_or_else(|| StorageError::TimestampOutOfRange { millis: now.0 })?
        .and_utc();
    let (year, month) = if date.month() == 12 {
        (date.year().saturating_add(1), 1)
    } else {
        (date.year(), date.month() + 1)
    };
    let resets = NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.and_hms_opt(0, 0, 0))
        .ok_or_else(|| StorageError::TimestampOutOfRange { millis: now.0 })?
        .and_utc();
    Ok((opened, timestamp_from_db(resets)))
}

fn rung_from_db(raw: i32) -> u32 {
    u32::try_from(raw).unwrap_or(0)
}

fn rung_to_db(rung: u32) -> i32 {
    i32::try_from(rung).unwrap_or(i32::MAX)
}
