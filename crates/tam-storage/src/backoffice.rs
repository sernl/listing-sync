//! The operator backoffice's reads: the whole platform rather than one
//! tenant.
//!
//! [`BackofficeRepo`] runs on a pool connected as `tam_backoffice`, whose
//! reach is the grant list and the read policies of migration 0037 and
//! nothing else. It pins no organisation, deliberately -- the pin is what
//! makes the application's own pool a tenant fence, and every query here
//! exists precisely to aggregate across tenants. Nothing here writes, and the
//! role holds no privilege that would let it.
//!
//! [`SignupsRepo`] is the exception and runs on the application pool, because
//! everything it reads is global already: `app_user` carries no policy, and
//! `auth.auth_event` is the one object in the identity schema `tam_app` holds
//! SELECT on.

use sqlx::PgPool;
use tam_domain::JobItemId;
use tam_types::{
    connection_status, ConnectionHealth, ConnectionId, ConnectionState, FailureCode, InventoryId,
    MappingId, OrgId, Timestamp, Uuid,
};

use crate::codec::{
    failure_code_from_db, inventory_from_db, timestamp_from_db, uuid_from_db, uuid_to_db,
};
use crate::connections::{marketplace_from_db, ConnectionRow};
use crate::job_reads::add_item_group;
use crate::{ItemCounts, StorageError};

/// How many rows a signup series or a failure listing may carry back. A bound
/// on one caller's own answer rather than a shared resource, so it lives here
/// beside the query rather than in `tam-limits`.
const MAX_ROWS: i64 = 500;

/// One day and what was counted on it. The instant is the day's start in UTC,
/// which is what `date_trunc` returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyCount {
    pub day: Timestamp,
    pub count: i64,
}

/// Signups from both planes, read on the application pool.
pub struct SignupsRepo {
    pool: PgPool,
}

impl SignupsRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// App-side provisioning per day: one row per `app_user`, which the
    /// session exchange writes on a subject's first login.
    pub async fn provisioned_by_day(&self) -> Result<Vec<DailyCount>, StorageError> {
        let rows = sqlx::query!(
            "SELECT date_trunc('day', created_at) AS \"day!\", count(*) AS \"count!\" \
             FROM app_user GROUP BY 1 ORDER BY 1 DESC LIMIT $1",
            MAX_ROWS,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DailyCount {
                day: timestamp_from_db(row.day),
                count: row.count,
            })
            .collect())
    }

    /// Identity-plane signups per day, or `None` where the identity schema is
    /// not present in this database.
    ///
    /// The two DDL sets are applied by separate commands against separate
    /// roles -- `just db-migrate` for `crates/tam-storage/migrations` and
    /// `just auth-migrate` for `db/auth` -- so a database can legitimately
    /// hold one and not the other. Answering zero there would report "nobody
    /// signed up" for what is really "this database cannot see the identity
    /// audit trail", and those are the two readings an operator looking at an
    /// empty chart needs told apart.
    pub async fn identity_by_day(&self) -> Result<Option<Vec<DailyCount>>, StorageError> {
        let present =
            sqlx::query!("SELECT to_regclass('auth.auth_event') IS NOT NULL AS \"present!\"",)
                .fetch_one(&self.pool)
                .await?
                .present;
        if !present {
            return Ok(None);
        }
        let rows = sqlx::query!(
            "SELECT date_trunc('day', at) AS \"day!\", count(*) AS \"count!\" \
             FROM auth.auth_event WHERE event = 'user_signed_up' \
             GROUP BY 1 ORDER BY 1 DESC LIMIT $1",
            MAX_ROWS,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(
            rows.into_iter()
                .map(|row| DailyCount {
                    day: timestamp_from_db(row.day),
                    count: row.count,
                })
                .collect(),
        ))
    }
}

/// One impersonation as the identity service recorded it.
///
/// `actor` and `target` are identity-plane subject ids -- `auth."user".id`,
/// which `app_user.auth_subject` joins to -- and not `UserId`s. The two planes
/// number their users separately, so rendering one as the other would name a
/// different person.
///
/// Neither party is optional, though the columns holding them are.
/// `auth_event_impersonation_parties_identified` in
/// `db/auth/0003_impersonation_event.sql` demands both of exactly the two
/// events this read selects, so a row reaching here without them is one the
/// database would not have accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpersonationEvent {
    pub event: String,
    pub actor: Uuid,
    pub target: Uuid,
    pub at: Timestamp,
    pub ip_address: Option<String>,
}

/// The identity plane's impersonation record, read on the application pool.
///
/// The same pool and the same reason as [`SignupsRepo`]: `auth.auth_event` is
/// the one object in the identity schema `tam_app` holds SELECT on, and the
/// cross-tenant `tam_backoffice` role holds nothing there at all.
pub struct IdentityAuditRepo {
    pool: PgPool,
}

impl IdentityAuditRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Impersonation events newest first, or `None` where the identity schema
    /// is not present in this database.
    ///
    /// The `None` draws the distinction [`SignupsRepo::identity_by_day`] draws
    /// and for the same reason: an operator looking at an empty list needs
    /// "nobody impersonated anyone" told apart from "the identity audit trail
    /// is not visible from here".
    pub async fn impersonations(
        &self,
        limit: i64,
    ) -> Result<Option<Vec<ImpersonationEvent>>, StorageError> {
        let present =
            sqlx::query!("SELECT to_regclass('auth.auth_event') IS NOT NULL AS \"present!\"",)
                .fetch_one(&self.pool)
                .await?
                .present;
        if !present {
            return Ok(None);
        }
        // The two non-null overrides are the filter's own consequence: the
        // check constraint that names both parties applies to exactly the two
        // events selected here, so neither column can arrive null.
        let rows = sqlx::query!(
            "SELECT event, user_id AS \"actor!\", target_user_id AS \"target!\", \
                    ip_address, at \
             FROM auth.auth_event \
             WHERE event IN ('user_impersonated', 'user_impersonation_stopped') \
             ORDER BY at DESC, id DESC LIMIT $1",
            limit.clamp(1, MAX_ROWS),
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(
            rows.into_iter()
                .map(|row| ImpersonationEvent {
                    event: row.event,
                    actor: uuid_from_db(row.actor),
                    target: uuid_from_db(row.target),
                    at: timestamp_from_db(row.at),
                    ip_address: row.ip_address,
                })
                .collect(),
        ))
    }
}

/// One organisation and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgSummary {
    pub org: OrgId,
    pub name: String,
    /// The handle the tenant claimed, or `None` while they have claimed none.
    /// What lets an operator find a tenant from a slug quoted in a support
    /// email, which the provisional `org-{uuid}` name never allowed.
    pub slug: Option<String>,
    pub created_at: Timestamp,
    pub products: i64,
    pub mappings: i64,
    pub connections: i64,
    pub users: i64,
}

/// A tenant-wide halt as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaltRecord {
    pub inventory: Option<InventoryId>,
    pub reason: String,
    pub raised_by: String,
    pub raised_at: Timestamp,
}

/// What Paddle last said about one organisation's subscription, narrowed to
/// the three facts an operator reads.
///
/// Deliberately not [`crate::SubscriptionState`]. That carries Paddle's
/// subscription and customer identifiers, which the tenant's own billing page
/// needs and a cross-tenant read has no use for; the smallest row that
/// answers the question is the one this surface carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionRecord {
    /// Paddle's own vocabulary, stored verbatim (migration 0038) and passed
    /// through here rather than translated.
    pub status: String,
    pub current_period_end: Option<Timestamp>,
    pub occurred_at: Timestamp,
}

/// One organisation rendered whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgDetail {
    pub summary: OrgSummary,
    pub connections: Vec<ConnectionRow>,
    pub halts: Vec<HaltRecord>,
    /// Absent for a tenant that has never reached checkout, which is a
    /// different fact from a cancelled subscription: that one is present and
    /// carries Paddle's cancelled status.
    pub subscription: Option<SubscriptionRecord>,
}

/// The ledger across every tenant: how many jobs exist, and their items by
/// state with the settled outcomes beside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncHealth {
    pub jobs: i64,
    pub items: ItemCounts,
}

/// One failed write attempt, with the item that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedWrite {
    pub org: OrgId,
    pub attempt: Uuid,
    pub item: JobItemId,
    pub mapping: MappingId,
    pub state: String,
    pub opened_at: Timestamp,
    pub settled_at: Option<Timestamp>,
    /// `None` where the attempt is still in flight: it has recorded no
    /// failure and may yet record none, which is exactly why it needs an
    /// operator.
    pub failure_code: Option<FailureCode>,
    pub ambiguity_cause: Option<String>,
    pub item_failure_code: Option<FailureCode>,
    pub item_failure_detail: Option<String>,
}

/// One import-drain measurement, with the tenant that recorded it.
///
/// `payload` travels as it was written. It is `jsonb` at rest and nothing
/// constrains its shape there, so parsing it here would turn one malformed
/// historical row into a failed read of the whole series; the client narrows
/// it instead and drops the row it cannot read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDrainRun {
    pub org: OrgId,
    pub org_name: String,
    pub org_seq: i64,
    pub payload: serde_json::Value,
}

/// One read of the drain series, and whether the limit cut it short.
///
/// `truncated` travels beside the runs rather than being left to be inferred
/// from their count, because it cannot be inferred correctly: the ordering is
/// by organisation name, so a full page drops whole tenants off the end of the
/// alphabet and the rows that survive look like the complete answer. A caller
/// comparing the length against its own limit would also be guessing at the
/// clamp this repository applies to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDrainPage {
    pub runs: Vec<ImportDrainRun>,
    pub truncated: bool,
}

pub struct BackofficeRepo {
    pool: PgPool,
}

impl BackofficeRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every organisation with its per-tenant counts, newest first.
    ///
    /// One query rather than a listing plus a count per tenant, which is what
    /// the grant on `organisation` and `app_user` buys: both are readable
    /// across tenants by the application role already, so naming them here
    /// moves an existing read onto this connection instead of reaching
    /// anything new.
    pub async fn orgs(&self) -> Result<Vec<OrgSummary>, StorageError> {
        let rows = sqlx::query!(
            "SELECT o.id, o.name, o.slug, o.created_at, \
                    (SELECT count(*) FROM product p \
                      WHERE p.org_id = o.id AND p.deleted_at IS NULL) AS \"products!\", \
                    (SELECT count(*) FROM mapping m WHERE m.org_id = o.id) AS \"mappings!\", \
                    (SELECT count(*) FROM connection c WHERE c.org_id = o.id) AS \"connections!\", \
                    (SELECT count(*) FROM app_user u WHERE u.org_id = o.id) AS \"users!\" \
             FROM organisation o ORDER BY o.created_at DESC, o.id LIMIT $1",
            MAX_ROWS,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| OrgSummary {
                org: OrgId(uuid_from_db(row.id)),
                name: row.name,
                slug: row.slug,
                created_at: timestamp_from_db(row.created_at),
                products: row.products,
                mappings: row.mappings,
                connections: row.connections,
                users: row.users,
            })
            .collect())
    }

    /// One organisation: its counts, its connections carrying the same
    /// derived status the seller's own page renders, and its halts.
    pub async fn org(&self, org: OrgId, now: Timestamp) -> Result<Option<OrgDetail>, StorageError> {
        let Some(head) = sqlx::query!(
            "SELECT o.id, o.name, o.slug, o.created_at, \
                    (SELECT count(*) FROM product p \
                      WHERE p.org_id = o.id AND p.deleted_at IS NULL) AS \"products!\", \
                    (SELECT count(*) FROM mapping m WHERE m.org_id = o.id) AS \"mappings!\", \
                    (SELECT count(*) FROM connection c WHERE c.org_id = o.id) AS \"connections!\", \
                    (SELECT count(*) FROM app_user u WHERE u.org_id = o.id) AS \"users!\" \
             FROM organisation o WHERE o.id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };

        let connection_rows = sqlx::query!(
            "SELECT id, marketplace, state, country, created_at, updated_at, \
                    session_verified_at, session_refresh_after, refresh_failures \
             FROM connection WHERE org_id = $1 ORDER BY marketplace",
            uuid_to_db(org.0),
        )
        .fetch_all(&self.pool)
        .await?;
        let connections = connection_rows
            .into_iter()
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
            .collect::<Result<Vec<_>, StorageError>>()?;

        let tenant_halt = sqlx::query!(
            "SELECT reason, raised_by, raised_at FROM org_halt WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        let inventory_halts = sqlx::query!(
            "SELECT inventory, reason, raised_by, raised_at FROM org_inventory_halt \
             WHERE org_id = $1 ORDER BY inventory",
            uuid_to_db(org.0),
        )
        .fetch_all(&self.pool)
        .await?;

        let mut halts = Vec::with_capacity(inventory_halts.len() + 1);
        if let Some(row) = tenant_halt {
            halts.push(HaltRecord {
                inventory: None,
                reason: row.reason,
                raised_by: row.raised_by,
                raised_at: timestamp_from_db(row.raised_at),
            });
        }
        for row in inventory_halts {
            halts.push(HaltRecord {
                inventory: Some(inventory_from_db(&row.inventory)?),
                reason: row.reason,
                raised_by: row.raised_by,
                raised_at: timestamp_from_db(row.raised_at),
            });
        }

        // Unpinned, like every other read on this pool: migration 0039's
        // read policy is what admits it past the tenant fence migration 0038
        // established, and pinning here would defeat the surface's purpose.
        let subscription = sqlx::query!(
            "SELECT status, current_period_end, occurred_at FROM billing_subscription \
             WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?
        .map(|row| SubscriptionRecord {
            status: row.status,
            current_period_end: row.current_period_end.map(timestamp_from_db),
            occurred_at: timestamp_from_db(row.occurred_at),
        });

        Ok(Some(OrgDetail {
            summary: OrgSummary {
                org: OrgId(uuid_from_db(head.id)),
                name: head.name,
                slug: head.slug,
                created_at: timestamp_from_db(head.created_at),
                products: head.products,
                mappings: head.mappings,
                connections: head.connections,
                users: head.users,
            },
            connections,
            halts,
            subscription,
        }))
    }

    /// The ledger's shape across every tenant. Raw counts, never a scalar
    /// verdict, for the reason the per-job roll-up gives: a health figure
    /// that collapses states hides the one that matters.
    pub async fn sync_health(&self) -> Result<SyncHealth, StorageError> {
        let jobs = sqlx::query!("SELECT count(*) AS \"jobs!\" FROM job")
            .fetch_one(&self.pool)
            .await?
            .jobs;
        let groups = sqlx::query!(
            "SELECT state, outcome, count(*) AS \"count!\" FROM job_item GROUP BY state, outcome",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut items = ItemCounts::default();
        for group in groups {
            add_item_group(
                &mut items,
                &group.state,
                group.outcome.as_deref(),
                u64::try_from(group.count).unwrap_or(0),
            )?;
        }
        Ok(SyncHealth { jobs, items })
    }

    /// Write attempts an operator needs to see: those that recorded a
    /// failure, and those stranded in flight.
    ///
    /// The second half is not decoration. A create's attempt is deliberately
    /// left standing when a run cannot prove what happened — releasing it is
    /// the only fence there is against a second live listing — so it is
    /// invisible to a view keyed on `failure_code`, mapping-scoped, and
    /// permanent until someone reconciles it.
    ///
    /// Stranded is defined against the lease rather than against the clock.
    /// An elapsed time cannot say it: the heartbeat renews a lease for as
    /// long as a device keeps working, so a healthy run holds its attempt
    /// open well past one TTL and a clock-keyed view would report it as a
    /// fault. The epoch does say it. An attempt whose `lease_epoch` is behind
    /// its item's current one belonged to a run that has been superseded, and
    /// one whose item is no longer in a live state belonged to a run that has
    /// ended; either way the run that could have settled it is gone.
    ///
    /// Stranded rows sort first, because they are the oldest by construction
    /// and would otherwise fall off the end of a newest-first page.
    pub async fn failed_writes(&self, limit: i64) -> Result<Vec<FailedWrite>, StorageError> {
        let rows = sqlx::query!(
            "SELECT w.org_id, w.id, w.job_item_id, w.mapping_id, w.state, \
                    w.opened_at, w.settled_at, w.failure_code, \
                    w.ambiguity_cause, i.failure_code AS item_failure_code, \
                    i.failure_detail AS item_failure_detail \
             FROM write_attempt w \
             JOIN job_item i ON i.org_id = w.org_id AND i.id = w.job_item_id \
             WHERE w.failure_code IS NOT NULL \
                OR (w.state = 'in_flight' \
                    AND (w.lease_epoch < i.lease_epoch \
                         OR i.state NOT IN ('leased', 'running', 'verifying'))) \
             ORDER BY (w.state = 'in_flight' AND w.failure_code IS NULL) DESC, \
                      w.opened_at DESC, w.id DESC LIMIT $1",
            limit.clamp(1, MAX_ROWS),
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(FailedWrite {
                    org: OrgId(uuid_from_db(row.org_id)),
                    attempt: uuid_from_db(row.id),
                    item: JobItemId(uuid_from_db(row.job_item_id)),
                    mapping: MappingId(uuid_from_db(row.mapping_id)),
                    state: row.state,
                    opened_at: timestamp_from_db(row.opened_at),
                    settled_at: row.settled_at.map(timestamp_from_db),
                    failure_code: row
                        .failure_code
                        .as_deref()
                        .map(failure_code_from_db)
                        .transpose()?,
                    ambiguity_cause: row.ambiguity_cause,
                    item_failure_code: row
                        .item_failure_code
                        .as_deref()
                        .map(failure_code_from_db)
                        .transpose()?,
                    item_failure_detail: row.item_failure_detail,
                })
            })
            .collect()
    }

    /// Every tenant's import-drain series, ordered so the client can group it
    /// without re-sorting: by organisation name, then by ledger position.
    ///
    /// Ordering by `org_seq` within a tenant is not cosmetic. The kill gate
    /// compares a tenant's first migration against its tenth, so the position
    /// of a row in its own tenant's series is the whole meaning of "first";
    /// a global ordering would interleave tenants and make that meaningless.
    pub async fn import_drain(&self, limit: i64) -> Result<ImportDrainPage, StorageError> {
        let cap = limit.clamp(1, MAX_ROWS);
        let mut rows = sqlx::query!(
            // The `!` assertions are needed because the columns come through a
            // view, and sqlx cannot carry NOT NULL inference across one. Every
            // one of them is NOT NULL on `job_event` itself (migration 0005).
            "SELECT d.org_id AS \"org_id!\", d.org_seq AS \"org_seq!\", \
                    d.payload AS \"payload!\", o.name AS \"org_name!\" \
             FROM import_drain_measurement d \
             JOIN organisation o ON o.id = d.org_id \
             ORDER BY o.name, d.org_seq LIMIT $1",
            // One past the cap, so a page that exactly fills the limit is told
            // from one the limit cut short. The extra row is a probe and is
            // dropped below rather than served.
            cap.saturating_add(1),
        )
        .fetch_all(&self.pool)
        .await?;
        let width = usize::try_from(cap).unwrap_or(usize::MAX);
        let truncated = rows.len() > width;
        rows.truncate(width);
        Ok(ImportDrainPage {
            runs: rows
                .into_iter()
                .map(|row| ImportDrainRun {
                    org: OrgId(uuid_from_db(row.org_id)),
                    org_name: row.org_name,
                    org_seq: row.org_seq,
                    payload: row.payload,
                })
                .collect(),
            truncated,
        })
    }

    /// Every topic with a dead letter, alphabetically, counted rather than
    /// listed.
    ///
    /// Two figures per topic and no rows, because two figures are the whole
    /// operator question: one organisation holding many is an address the
    /// relay refuses, and many organisations holding one each is the relay or
    /// the key. The role reads three columns of this table and only the dead
    /// rows (migration 0067), which is exactly what this asks for.
    pub async fn dead_letters(&self) -> Result<Vec<DeadLetterTopic>, StorageError> {
        let rows = sqlx::query!(
            "SELECT topic, count(*) AS \"messages!\", count(DISTINCT org_id) AS \"orgs!\" \
             FROM outbox_message WHERE state = 'dead' \
             GROUP BY topic ORDER BY topic LIMIT $1",
            MAX_ROWS,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DeadLetterTopic {
                topic: row.topic,
                messages: row.messages,
                orgs: row.orgs,
            })
            .collect())
    }
}

/// Dead letters on one outbox topic: how many, and across how many
/// organisations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetterTopic {
    pub topic: String,
    pub messages: i64,
    pub orgs: i64,
}
