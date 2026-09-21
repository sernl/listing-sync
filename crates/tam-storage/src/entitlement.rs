//! What an organisation is entitled to, derived from its grants.
//!
//! The plan is a query rather than a column: migration 0069 records one row
//! per grant, and the plan an organisation holds is the strongest of the rows
//! that are neither revoked nor expired. An organisation with no rows holds
//! `free`, which is the same fail-closed direction the lapsed-subscription
//! read takes.
//!
//! Strongest rather than newest, because the two orders disagree exactly
//! where it matters: an organisation holding an operator grant of Studio
//! beside a live subscription would otherwise be served whichever webhook
//! landed last. `Plan::strength` is the order, declared in tam-limits beside
//! the capability table it selects.
//!
//! [`EntitlementRepo::current`] is on the request path — the `OrgContext`
//! extractor calls it before any handler runs — so it is one indexed read of
//! one organisation's live rows, and nothing else.
//!
//! Moves are the other half, and they are a ledger rather than a grant.
//! Migration 0084 says why; the short version is that a plan row can carry a
//! monthly allowance but not a balance, and the pricing model of 2026-09-20
//! sells a balance on any plan. The balance is `SUM(delta)` over the rows
//! that have not expired, which is one indexed read, and every writer is
//! idempotent on `source_ref`.

use chrono::{DateTime, Months, Utc};
use sqlx::PgPool;
use tam_limits::Plan;
use tam_types::{OrgId, Timestamp, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// Who made a grant, which decides how it is attributed and who may take it
/// back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantedBy {
    /// A Paddle purchase. Retired as a writer when the billing rail moved to
    /// Stripe, and kept as a value because the rows it wrote are real grants
    /// with real attribution: rewriting them would restate history to match
    /// a decision taken after it.
    Paddle,
    /// A Stripe purchase. Attributable to `source_ref`, which is the
    /// checkout session or the subscription Stripe named.
    Stripe,
    /// A backoffice grant. Attributable to `grantor_user` and to the reason
    /// the operator typed.
    Operator,
}

impl GrantedBy {
    pub const ALL: [Self; 3] = [Self::Paddle, Self::Stripe, Self::Operator];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paddle => "paddle",
            Self::Stripe => "stripe",
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
/// nobody granted `free`, and naming an operator or a purchase as its source
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

    /// The live provider grant for one subscription, session or transaction,
    /// if this organisation holds one.
    ///
    /// The webhook reads this before writing, so a `customer.subscription.*`
    /// stream renews one grant rather than accumulating a row per delivery.
    ///
    /// Keyed on `source_ref` alone rather than on the vendor beside it.
    /// Migration 0086 made the unique index on that column vendor-agnostic,
    /// so one live grant answers one reference across the deployment, and a
    /// lookup naming a vendor would have stopped finding the Paddle-era row
    /// a resumed subscription still holds.
    pub async fn provider_grant(
        &self,
        org: OrgId,
        source_ref: &str,
    ) -> Result<Option<Uuid>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_scalar!(
            "SELECT id FROM entitlement_grant WHERE org_id = $1 AND source_ref = $2 AND granted_by <> 'operator' AND revoked_at IS NULL",
            uuid_to_db(org.0),
            source_ref,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(uuid_from_db))
    }

    /// Moves a live grant's expiry, which is how a cancelled subscription
    /// stops entitling at its period end rather than at the moment the
    /// provider said so. Answers whether a row moved.
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
    pub async fn usage(&self, org: OrgId) -> Result<Usage, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            r#"SELECT
                 (SELECT count(*) FROM product
                   WHERE org_id = $1 AND deleted_at IS NULL)          AS "resources!",
                 (SELECT count(DISTINCT marketplace) FROM connection
                   WHERE org_id = $1 AND state = 'linked')            AS "marketplaces!",
                 (SELECT count(*) FROM resource_template
                   WHERE org_id = $1)                                 AS "templates!",
                 (SELECT count(*) FROM label
                   WHERE org_id = $1)                                 AS "labels!",
                 (SELECT count(*) FROM collection
                   WHERE org_id = $1)                                 AS "collections!",
                 (SELECT count(*) FROM device
                   WHERE org_id = $1 AND revoked_at IS NULL)          AS "devices!""#,
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Usage {
            resources: row.resources,
            marketplaces: row.marketplaces,
            templates: row.templates,
            labels: row.labels,
            collections: row.collections,
            devices: row.devices,
        })
    }

    /// How many moves this organisation can spend right now, and when the
    /// soonest part of that balance goes away.
    ///
    /// Both in one read, because every surface that shows the first shows the
    /// second beside it: a balance with no expiry date is a number a seller
    /// cannot plan against.
    pub async fn move_balance(
        &self,
        org: OrgId,
        now: Timestamp,
    ) -> Result<MoveBalance, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let at = timestamp_to_db(now)?;
        let row = sqlx::query!(
            r#"SELECT
                 COALESCE(SUM(delta), 0)::bigint          AS "available!",
                 MIN(expires_at) FILTER (WHERE delta > 0) AS "expiring_soonest"
               FROM move_ledger
              WHERE org_id = $1
                AND (expires_at IS NULL OR expires_at > $2)"#,
            uuid_to_db(org.0),
            at,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(MoveBalance {
            // Floored, because a balance is a thing a seller is told and
            // "minus two moves" is not one. A negative sum would be a writer
            // bug rather than a state the product has; the gate that reads
            // this refuses at zero either way.
            available: row.available.max(0),
            expiring_soonest: row.expiring_soonest.map(timestamp_from_db),
        })
    }

    /// Writes one ledger entry. Answers whether it was written, so a second
    /// delivery of the same webhook is a `false` rather than a second credit.
    ///
    /// `delta` is signed and the database checks it against `source`: the
    /// four purchase-shaped sources must credit, `commit` and `refund` must
    /// debit, and `operator` may do either because an operator correcting a
    /// mistake has to be able to correct it in both directions.
    pub async fn credit_moves(
        &self,
        org: OrgId,
        credit: MoveCredit<'_>,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = write_move(
            &mut tx,
            org,
            LedgerEntry {
                delta: credit.delta,
                source: credit.source,
                source_ref: credit.source_ref,
                expires_at: credit.expires_at.map(timestamp_to_db).transpose()?,
                at: timestamp_to_db(credit.at)?,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(written)
    }

    /// Spends one move against a committed job item.
    ///
    /// Idempotent on the item, which is what makes a retried settle free:
    /// the same item id names the same move however many times the device
    /// says so. Answers which move this was, counting from the
    /// organisation's first ever, or `None` when the item had already been
    /// charged.
    ///
    /// The ordinal is read in the same transaction as the write, because it
    /// is what the analytics event carries and a number read afterwards
    /// would count a concurrent settle's move as this one's.
    ///
    /// The debit inherits the expiry of the soonest-expiring credit it draws
    /// against, which is what keeps the balance honest when that credit
    /// lapses: the pair leaves together. Migration 0084's header is the long
    /// form of why.
    pub async fn debit_move(
        &self,
        org: OrgId,
        item: Uuid,
        at: Timestamp,
    ) -> Result<Option<i64>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let now = timestamp_to_db(at)?;
        let drawn = sqlx::query_scalar!(
            "SELECT MIN(expires_at) FROM move_ledger \
              WHERE org_id = $1 AND delta > 0 AND expires_at IS NOT NULL AND expires_at > $2",
            uuid_to_db(org.0),
            now,
        )
        .fetch_one(&mut *tx)
        .await?;
        let reference = item_reference(item);
        let written = write_move(
            &mut tx,
            org,
            LedgerEntry {
                delta: -1,
                source: MoveSource::Commit,
                source_ref: Some(&reference),
                expires_at: drawn,
                at: now,
            },
        )
        .await?;
        if !written {
            tx.commit().await?;
            return Ok(None);
        }
        let nth = sqlx::query_scalar!(
            r#"SELECT count(*) AS "nth!" FROM move_ledger
                WHERE org_id = $1 AND source = 'commit'"#,
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(nth))
    }

    /// Credits one billing period's moves to a subscription.
    ///
    /// Idempotent on the period, so a scheduler that runs twice or a webhook
    /// replayed a week later credits once. The amount is capped so the
    /// subscription's own unexpired credits never exceed `accrual_cap`: a
    /// seller who moved nothing for four months holds the cap rather than
    /// four months of allowance, which is what stops subscribe-hoard-cancel
    /// without touching a balance they bought outright.
    ///
    /// Each period's credit expires once the allowance has rolled as far as
    /// the cap permits, so the roll-up is a fact in the data rather than a
    /// sweep some job has to remember to run.
    pub async fn accrue_subscription_moves(
        &self,
        org: OrgId,
        accrual: Accrual,
    ) -> Result<bool, StorageError> {
        let Accrual {
            period_start,
            per_period,
            accrual_cap,
            at,
        } = accrual;
        if per_period == 0 {
            return Ok(false);
        }
        let opened = timestamp_to_db(period_start)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let now = timestamp_to_db(at)?;
        let held = sqlx::query_scalar!(
            r#"SELECT COALESCE(SUM(delta), 0)::bigint AS "held!" FROM move_ledger
                WHERE org_id = $1 AND source = 'subscription'
                  AND (expires_at IS NULL OR expires_at > $2)"#,
            uuid_to_db(org.0),
            now,
        )
        .fetch_one(&mut *tx)
        .await?;
        let room = i64::from(accrual_cap).saturating_sub(held.max(0));
        let delta = i64::from(per_period).min(room).max(0);
        if delta == 0 {
            tx.commit().await?;
            return Ok(false);
        }
        // How many periods the allowance may roll for before it lapses, which
        // is the cap expressed in the unit the cap is a multiple of. One
        // period at minimum, so a cap below a single period's allowance does
        // not mint a credit that expires the instant it is written.
        #[expect(
            clippy::integer_division,
            reason = "a count of whole periods; a remainder is allowance the \
                      cap does not stretch to another roll, and dropping it is \
                      the cap doing its job"
        )]
        let rolls = (accrual_cap / per_period).max(1);
        let expires = opened
            .checked_add_months(Months::new(rolls))
            .ok_or(StorageError::TimestampOutOfRange { millis: at.0 })?;
        let reference = period_reference(opened);
        let written = write_move(
            &mut tx,
            org,
            LedgerEntry {
                delta: i32::try_from(delta).unwrap_or(i32::MAX),
                source: MoveSource::Subscription,
                source_ref: Some(&reference),
                expires_at: Some(expires),
                at: now,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(written)
    }

    /// Binds a storefront to this organisation for the purpose of the free
    /// lifetime moves, and credits them when the binding is new.
    ///
    /// `true` exactly when this shop had never been claimed by anybody. The
    /// pool-owning form; the heartbeat that writes
    /// `connection.platform_account_digest` calls
    /// [`grant_storefront_allowance_in`] inside its own transaction instead,
    /// so the bind and the grant are one write.
    pub async fn grant_storefront_allowance(
        &self,
        org: OrgId,
        allowance: StorefrontAllowance<'_>,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let granted = grant_storefront_allowance_in(&mut tx, org, allowance).await?;
        tx.commit().await?;
        Ok(granted)
    }
}

/// Where one ledger entry came from. The closed set migration 0084's CHECK
/// enumerates, and the sign rule it enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveSource {
    /// The five a storefront is given once, ever.
    Free,
    /// A pack, valid twelve months from purchase.
    Pack,
    /// One billing period's allowance on a subscription.
    Subscription,
    /// The founding member's bonus, on top of the subscription's own.
    Founding,
    /// A correction, in either direction, with a reason.
    Operator,
    /// One resource committed to the other marketplace. The only ordinary
    /// debit.
    Commit,
    /// A purchase taken back.
    Refund,
}

impl MoveSource {
    pub const ALL: [Self; 7] = [
        Self::Free,
        Self::Pack,
        Self::Subscription,
        Self::Founding,
        Self::Operator,
        Self::Commit,
        Self::Refund,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pack => "pack",
            Self::Subscription => "subscription",
            Self::Founding => "founding",
            Self::Operator => "operator",
            Self::Commit => "commit",
            Self::Refund => "refund",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|source| source.as_str() == raw)
    }
}

/// What an organisation can spend, and when the soonest of it lapses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveBalance {
    pub available: i64,
    /// The expiry of the earliest-expiring credit still standing. `None`
    /// where nothing in the balance expires, which is the free grant's shape.
    pub expiring_soonest: Option<Timestamp>,
}

/// One ledger entry a caller asks for, as [`EntitlementRepo::credit_moves`]
/// takes it. Named rather than positional because `delta`, `source` and the
/// two optional stamps are four ways to write the same call wrongly.
#[derive(Debug, Clone, Copy)]
pub struct MoveCredit<'a> {
    /// Signed, and checked against `source` by the database.
    pub delta: i32,
    pub source: MoveSource,
    /// What the write is idempotent on. `None` writes unconditionally.
    pub source_ref: Option<&'a str>,
    /// When the credit lapses. `None` never lapses.
    pub expires_at: Option<Timestamp>,
    pub at: Timestamp,
}

/// One billing period's accrual, as
/// [`EntitlementRepo::accrue_subscription_moves`] takes it. `per_period` and
/// `accrual_cap` are both move counts and would otherwise be adjacent `u32`s.
#[derive(Debug, Clone, Copy)]
pub struct Accrual {
    /// The period this credit belongs to, which it is idempotent on.
    pub period_start: Timestamp,
    /// The allowance one period carries.
    pub per_period: u32,
    /// The ceiling the subscription's own unexpired credits never exceed.
    pub accrual_cap: u32,
    pub at: Timestamp,
}

/// The storefront a free grant is claimed against, as
/// [`EntitlementRepo::grant_storefront_allowance`] and
/// [`grant_storefront_allowance_in`] take it.
#[derive(Debug, Clone, Copy)]
pub struct StorefrontAllowance<'a> {
    pub marketplace: &'a str,
    /// The opaque platform account digest the claim is unique on.
    pub digest: &'a [u8],
    /// How many moves the claim credits. Zero grants nothing.
    pub moves: u32,
    pub at: Timestamp,
}

/// The allowance write, in a caller's transaction.
///
/// Separate from the repository method so the heartbeat can bind a storefront
/// and grant its moves in one transaction: a row here with no credit, or a
/// credit with no row here, would each be a way to lose or repeat the grant.
/// The caller has already pinned `org`.
pub(crate) async fn grant_storefront_allowance_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    allowance: StorefrontAllowance<'_>,
) -> Result<bool, StorageError> {
    let StorefrontAllowance {
        marketplace,
        digest,
        moves,
        at,
    } = allowance;
    let now = timestamp_to_db(at)?;
    // The primary key crosses the tenant fence even though the policy does
    // not: a unique violation is raised on rows the pin cannot see, so a
    // second organisation naming the same shop inserts nothing and is never
    // told whose shop it was.
    let claimed = sqlx::query!(
        "INSERT INTO storefront_allowance \
         (marketplace, platform_account_digest, org_id, granted_at) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (marketplace, platform_account_digest) DO NOTHING",
        marketplace,
        digest,
        uuid_to_db(org.0),
        now,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if claimed == 0 || moves == 0 {
        return Ok(false);
    }
    let reference = storefront_reference(marketplace, digest);
    write_move(
        tx,
        org,
        LedgerEntry {
            delta: i32::try_from(moves).unwrap_or(i32::MAX),
            source: MoveSource::Free,
            source_ref: Some(&reference),
            // No expiry. The five are the free tier's whole offer, and an
            // offer that quietly lapses is one the seller finds out about by
            // losing it.
            expires_at: None,
            at: now,
        },
    )
    .await?;
    Ok(true)
}

/// One ledger entry, in a caller's transaction, idempotent on its reference.
async fn write_move(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    entry: LedgerEntry<'_>,
) -> Result<bool, StorageError> {
    let written = sqlx::query!(
        "INSERT INTO move_ledger \
         (org_id, id, delta, source, source_ref, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (org_id, source, source_ref) WHERE source_ref IS NOT NULL DO NOTHING",
        uuid_to_db(org.0),
        uuid::Uuid::new_v4(),
        entry.delta,
        entry.source.as_str(),
        entry.source_ref,
        entry.expires_at,
        entry.at,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(written == 1)
}

/// One ledger row as [`write_move`] writes it: the caller-facing
/// [`MoveCredit`] with its stamps already in the database's type.
#[derive(Debug, Clone, Copy)]
struct LedgerEntry<'a> {
    delta: i32,
    source: MoveSource,
    source_ref: Option<&'a str>,
    expires_at: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
}

/// The reference a commit's debit is idempotent on.
fn item_reference(item: Uuid) -> String {
    let mut out = String::with_capacity(5 + 36);
    out.push_str("item:");
    out.push_str(&uuid_to_db(item).to_string());
    out
}

/// The reference a period's accrual is idempotent on: the day the period
/// opened, in UTC, which is the boundary the scheduler works in.
fn period_reference(opened: DateTime<Utc>) -> String {
    let mut out = String::with_capacity(7 + 10);
    out.push_str("period:");
    out.push_str(&opened.date_naive().to_string());
    out
}

/// The reference a storefront's free grant is idempotent on. The digest is
/// already opaque, so it travels as hex rather than as anything a reader
/// could turn back into a shop.
fn storefront_reference(marketplace: &str, digest: &[u8]) -> String {
    let mut out = String::with_capacity(11 + marketplace.len() + digest.len() * 2);
    out.push_str("storefront:");
    out.push_str(marketplace);
    out.push(':');
    for byte in digest {
        out.push(hex_nibble(byte >> 4));
        out.push(hex_nibble(byte & 0x0f));
    }
    out
}

fn hex_nibble(nibble: u8) -> char {
    char::from(match nibble {
        0..=9 => b'0' + nibble,
        _ => b'a' + nibble - 10,
    })
}

/// What one organisation has used of each metered capability.
///
/// Moves are not here. They are a balance rather than a count against a
/// monthly ceiling, and [`EntitlementRepo::move_balance`] is the read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub resources: i64,
    pub marketplaces: i64,
    pub templates: i64,
    pub labels: i64,
    pub collections: i64,
    pub devices: i64,
}

fn rung_from_db(raw: i32) -> u32 {
    u32::try_from(raw).unwrap_or(0)
}

fn rung_to_db(rung: u32) -> i32 {
    i32::try_from(rung).unwrap_or(i32::MAX)
}
