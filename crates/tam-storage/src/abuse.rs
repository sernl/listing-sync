//! Abuse prevention: the linkage ledger, the scorer that reads it, the
//! operator's decisions, and what a ban refuses afterwards (migrations 0105
//! and 0106; `docs/notes/design/abuse-prevention.md` is the long form).
//!
//! Every value this module stores or compares is a 32-byte keyed digest the
//! caller computed (`tam_secrets::abuse_digest`, or the shop's own exclusivity
//! digest). Nothing here sees an address, a device id or a card.
//!
//! [`AbuseRepo`] is the application pool's half: signals written from the
//! request path, the scorer, operator decisions, the gates' reads, the mail
//! outbox and the prune. [`AbuseBackofficeRepo`] is the operator page's reads
//! on the backoffice pool, which crosses every tenant by grant.

use std::net::IpAddr;

use sqlx::{PgConnection, PgPool};
use tam_types::{OrgId, Timestamp, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// How long a ban keeps refusing what it named, in months.
pub const BAN_MONTHS: u32 = 24;
/// How long an address or a browser is kept, in days.
pub const SHORT_SIGNAL_DAYS: i64 = 90;
/// How many times a warning or suspension mail is tried.
pub const ABUSE_MAIL_ATTEMPTS: i32 = 5;
/// How many sign-ups one address may make in a day before the next is refused.
pub const SIGNUPS_PER_IP_PER_DAY: i64 = 5;
/// How soon after its free moves were granted an unlink is a quick one.
pub const QUICK_UNLINK_DAYS: i64 = 7;

/// What a signal is a digest of. The closed set 0105's CHECK enumerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SignalKind {
    /// A shop, as `connection.platform_account_digest` names it.
    ShopDigest,
    /// An install of the desktop app: its self-minted device id.
    DeviceFingerprint,
    /// The address a sign-in came from.
    Ip,
    /// The part of an email address after the `@`.
    EmailDomain,
    /// Stripe's fingerprint of a card's number.
    PaymentFingerprint,
    /// The browser's user-agent string.
    UserAgentHash,
}

impl SignalKind {
    pub const ALL: [Self; 6] = [
        Self::ShopDigest,
        Self::DeviceFingerprint,
        Self::Ip,
        Self::EmailDomain,
        Self::PaymentFingerprint,
        Self::UserAgentHash,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ShopDigest => "shop_digest",
            Self::DeviceFingerprint => "device_fingerprint",
            Self::Ip => "ip",
            Self::EmailDomain => "email_domain",
            Self::PaymentFingerprint => "payment_fingerprint",
            Self::UserAgentHash => "user_agent_hash",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == raw)
    }

    /// Whether two organisations sharing this value says anything about them.
    ///
    /// A browser string and an email domain are shared by thousands of honest
    /// sellers (every Chrome on Windows, every Gmail address), so they are
    /// kept as context and never link two accounts on their own.
    #[must_use]
    pub const fn links(self) -> bool {
        matches!(
            self,
            Self::ShopDigest | Self::DeviceFingerprint | Self::Ip | Self::PaymentFingerprint
        )
    }
}

/// The rules the scorer and the request path raise. 0106's CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FlagKind {
    SharedShop,
    SharedDevice,
    SharedPayment,
    SignupBurst,
    DisposableEmail,
    QuickUnlink,
}

impl FlagKind {
    pub const ALL: [Self; 6] = [
        Self::SharedShop,
        Self::SharedDevice,
        Self::SharedPayment,
        Self::SignupBurst,
        Self::DisposableEmail,
        Self::QuickUnlink,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SharedShop => "shared_shop",
            Self::SharedDevice => "shared_device",
            Self::SharedPayment => "shared_payment",
            Self::SignupBurst => "signup_burst",
            Self::DisposableEmail => "disposable_email",
            Self::QuickUnlink => "quick_unlink",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == raw)
    }

    /// How much one flag of this kind weighs, out of 100. A shared shop is
    /// the farming loop itself; a shared card is often a family; a burst of
    /// sign-ups from one address is often a school.
    #[must_use]
    pub const fn score(self) -> i32 {
        match self {
            Self::SharedShop => 60,
            Self::SharedDevice | Self::QuickUnlink => 50,
            Self::SharedPayment | Self::DisposableEmail => 40,
            Self::SignupBurst => 30,
        }
    }
}

/// An organisation's standing, and the action an operator takes to set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbuseAction {
    None,
    Warn,
    Limit,
    Ban,
}

impl AbuseAction {
    pub const ALL: [Self; 4] = [Self::None, Self::Warn, Self::Limit, Self::Ban];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Warn => "warn",
            Self::Limit => "limit",
            Self::Ban => "ban",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.as_str() == raw)
    }

    fn from_db(raw: &str) -> Result<Self, StorageError> {
        Self::parse(raw).ok_or_else(|| StorageError::CorruptRow {
            reason: format!("unknown abuse action {raw:?}"),
        })
    }

    /// No free moves and no new connections: a limit, and a ban a fortiori.
    #[must_use]
    pub const fn withholds_free_moves(self) -> bool {
        matches!(self, Self::Limit | Self::Ban)
    }

    /// Whether this standing refuses every authenticated request.
    #[must_use]
    pub const fn suspends(self) -> bool {
        matches!(self, Self::Ban)
    }

    /// Whether deciding this queues a mail to the organisation's people.
    #[must_use]
    pub const fn mails(self) -> bool {
        matches!(self, Self::Warn | Self::Ban)
    }
}

/// What a ban refuses. 0106's CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BannedKind {
    Email,
    ShopDigest,
    DeviceFingerprint,
    PaymentFingerprint,
}

impl BannedKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::ShopDigest => "shop_digest",
            Self::DeviceFingerprint => "device_fingerprint",
            Self::PaymentFingerprint => "payment_fingerprint",
        }
    }
}

/// One sighting of one signal.
#[derive(Debug, Clone, Copy)]
pub struct Signal<'a> {
    pub org: OrgId,
    pub kind: SignalKind,
    pub value: &'a [u8; 32],
    pub at: Timestamp,
}

/// An operator's decision about the organisation a flag names.
#[derive(Debug, Clone, Copy)]
pub struct Decision<'a> {
    pub flag: Uuid,
    pub action: AbuseAction,
    pub reason: Option<&'a str>,
    pub by: Uuid,
    pub at: Timestamp,
    /// What a ban refuses beyond what the database already holds as digests:
    /// the caller's keyed digests of the organisation's email addresses and
    /// device ids. Ignored unless `action` is a ban.
    pub identities: &'a [(BannedKind, [u8; 32])],
}

/// One claimed warning or suspension mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedAbuseMail {
    pub flag: Uuid,
    pub action: AbuseAction,
    pub org_name: String,
    pub attempts: i32,
    /// The identity subjects of the organisation's people.
    pub subjects: Vec<Uuid>,
}

/// What became of one mail attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbuseMailOutcome {
    Sent,
    Retry { error: String, at: Timestamp },
    Failed { error: String },
}

/// What the scorer raised in one pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scored {
    pub raised: u64,
}

/// What the nightly prune removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    pub signals: u64,
    pub bans: u64,
}

/// The standing of `org`, in a caller's connection.
pub(crate) async fn standing_in(
    conn: &mut PgConnection,
    org: OrgId,
) -> Result<AbuseAction, StorageError> {
    let raw = sqlx::query_scalar!(
        r#"SELECT abuse_standing($1) AS "standing!""#,
        uuid_to_db(org.0),
    )
    .fetch_one(conn)
    .await?;
    AbuseAction::from_db(&raw)
}

/// Writes one sighting, in a caller's connection: the first writes the row,
/// every later one moves `last_seen` forward.
pub(crate) async fn record_signal_in(
    conn: &mut PgConnection,
    signal: Signal<'_>,
) -> Result<(), StorageError> {
    let at = timestamp_to_db(signal.at)?;
    sqlx::query!(
        "INSERT INTO account_link_signal (org_id, kind, value_hash, first_seen, last_seen) \
         VALUES ($1, $2, $3, $4, $4) \
         ON CONFLICT (org_id, kind, value_hash) DO UPDATE \
             SET last_seen = GREATEST(account_link_signal.last_seen, EXCLUDED.last_seen)",
        uuid_to_db(signal.org.0),
        signal.kind.as_str(),
        &signal.value[..],
        at,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether a ban still refuses this value, in a caller's connection.
pub(crate) async fn is_banned_in(
    conn: &mut PgConnection,
    kind: BannedKind,
    value: &[u8],
    now: Timestamp,
) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM banned_identity
              WHERE kind = $1 AND value_hash = $2 AND expires_at > $3
           ) AS "banned!""#,
        kind.as_str(),
        value,
        timestamp_to_db(now)?,
    )
    .fetch_one(conn)
    .await?)
}

/// Raises one flag unless one of its kind is already open for the
/// organisation. Answers whether it was raised.
pub(crate) async fn raise_in(
    conn: &mut PgConnection,
    org: OrgId,
    kind: FlagKind,
    reason: &str,
    at: Timestamp,
) -> Result<bool, StorageError> {
    let raised = sqlx::query!(
        "INSERT INTO abuse_flag (id, org_id, kind, score, reason, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (org_id, kind) WHERE resolved_at IS NULL DO NOTHING",
        uuid::Uuid::new_v4(),
        uuid_to_db(org.0),
        kind.as_str(),
        kind.score(),
        reason,
        timestamp_to_db(at)?,
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(raised == 1)
}

/// The scorer's shared-value rules, in a caller's connection: every
/// organisation holding a shop, a device or a card some other organisation
/// also holds is flagged once for it.
///
/// `scope` narrows the pass to clusters touching one organisation, which is
/// the on-event form; `None` is the nightly pass over everything.
///
/// Evidence already decided is not raised again: a flag of the same kind
/// resolved after the sharing began means an operator has seen this, and only
/// a sharing that began after their decision is new. That is what keeps a
/// dismissed family card from being flagged every night.
pub(crate) async fn score_in(
    conn: &mut PgConnection,
    scope: Option<OrgId>,
    at: Timestamp,
) -> Result<Scored, StorageError> {
    let now = timestamp_to_db(at)?;
    let scope = scope.map(|org| uuid_to_db(org.0));
    let mut raised = 0;
    for (signal, flag, noun) in [
        (SignalKind::ShopDigest, FlagKind::SharedShop, "a shop"),
        (
            SignalKind::DeviceFingerprint,
            FlagKind::SharedDevice,
            "a device",
        ),
        (
            SignalKind::PaymentFingerprint,
            FlagKind::SharedPayment,
            "a card",
        ),
    ] {
        raised += sqlx::query!(
            r#"INSERT INTO abuse_flag (id, org_id, kind, score, reason, created_at)
               SELECT gen_random_uuid(), c.org_id, $3, $4,
                      'Shares ' || $5 || ' with ' || c.others
                        || CASE WHEN c.others = 1 THEN ' other account.' ELSE ' other accounts.' END,
                      $6
                 FROM (SELECT a.org_id,
                              count(DISTINCT b.org_id) AS others,
                              max(GREATEST(a.first_seen, b.first_seen)) AS evidence_at
                         FROM account_link_signal a
                         JOIN account_link_signal b
                           ON b.kind = a.kind AND b.value_hash = a.value_hash
                          AND b.org_id <> a.org_id
                        WHERE a.kind = $1
                          AND ($2::uuid IS NULL OR a.org_id = $2 OR b.org_id = $2)
                        GROUP BY a.org_id) c
                WHERE NOT EXISTS (
                        SELECT 1 FROM abuse_flag f
                         WHERE f.org_id = c.org_id AND f.kind = $3
                           AND (f.resolved_at IS NULL OR f.resolved_at >= c.evidence_at))
               ON CONFLICT (org_id, kind) WHERE resolved_at IS NULL DO NOTHING"#,
            signal.as_str(),
            scope,
            flag.as_str(),
            flag.score(),
            noun,
            now,
        )
        .execute(&mut *conn)
        .await?
        .rows_affected();
    }
    // Sign-up bursts: three or more organisations whose sign-up address (the
    // address seen within the hour the organisation was made) is the same,
    // made within a day of one another. One address is often a school, which
    // is why this weighs least and is never acted on unseen.
    raised += sqlx::query!(
        r#"WITH signup AS (
               SELECT s.org_id, s.value_hash, s.first_seen
                 FROM account_link_signal s
                 JOIN organisation o ON o.id = s.org_id
                WHERE s.kind = 'ip' AND s.first_seen <= o.created_at + interval '1 hour'
           ), burst AS (
               SELECT org_id, value_hash, first_seen,
                      count(*) OVER (PARTITION BY value_hash ORDER BY first_seen
                                     RANGE BETWEEN interval '24 hours' PRECEDING
                                               AND interval '24 hours' FOLLOWING) AS n
                 FROM signup
           )
           INSERT INTO abuse_flag (id, org_id, kind, score, reason, created_at)
           SELECT gen_random_uuid(), b.org_id, $2, $3,
                  'One of ' || max(b.n) || ' accounts made from the same address within a day.',
                  $4
             FROM burst b
            WHERE b.n >= 3
              AND ($1::uuid IS NULL OR b.value_hash IN
                     (SELECT value_hash FROM signup WHERE org_id = $1))
              AND NOT EXISTS (
                    SELECT 1 FROM abuse_flag f
                     WHERE f.org_id = b.org_id AND f.kind = $2
                       AND (f.resolved_at IS NULL OR f.resolved_at >= b.first_seen))
            GROUP BY b.org_id
           ON CONFLICT (org_id, kind) WHERE resolved_at IS NULL DO NOTHING"#,
        scope,
        FlagKind::SignupBurst.as_str(),
        FlagKind::SignupBurst.score(),
        now,
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(Scored { raised })
}

pub struct AbuseRepo {
    pool: PgPool,
}

impl AbuseRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes one sighting and, for a value that can link two accounts, runs
    /// the scorer over the clusters it touches.
    pub async fn record_signal(&self, signal: Signal<'_>) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        record_signal_in(&mut tx, signal).await?;
        if signal.kind.links() {
            score_in(&mut tx, Some(signal.org), signal.at).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Raises one flag the request path concluded on its own (a throwaway
    /// email domain). Answers whether it was raised.
    pub async fn raise(
        &self,
        org: OrgId,
        kind: FlagKind,
        reason: &str,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut conn = self.pool.acquire().await?;
        raise_in(&mut conn, org, kind, reason, at).await
    }

    /// The scorer over everything, or over the clusters one organisation
    /// touches.
    pub async fn score(&self, scope: Option<OrgId>, at: Timestamp) -> Result<Scored, StorageError> {
        let mut tx = self.pool.begin().await?;
        let scored = score_in(&mut tx, scope, at).await?;
        tx.commit().await?;
        Ok(scored)
    }

    /// The standing of one organisation.
    pub async fn standing(&self, org: OrgId) -> Result<AbuseAction, StorageError> {
        let mut conn = self.pool.acquire().await?;
        standing_in(&mut conn, org).await
    }

    /// Whether a ban still refuses this value.
    pub async fn is_banned(
        &self,
        kind: BannedKind,
        value: &[u8; 32],
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut conn = self.pool.acquire().await?;
        is_banned_in(&mut conn, kind, value, now).await
    }

    /// How many accounts were made from this address since `since`: the
    /// identity subjects whose first agreement (migration 0103, written the
    /// moment an account is made) came from it.
    pub async fn signups_from(&self, ip: IpAddr, since: Timestamp) -> Result<i64, StorageError> {
        Ok(sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM (
                   SELECT DISTINCT ON (subject) subject, ip_address, accepted_at
                     FROM account_consent
                    ORDER BY subject, accepted_at
               ) first
              WHERE first.ip_address = $1::text::inet AND first.accepted_at >= $2"#,
            ip.to_string(),
            timestamp_to_db(since)?,
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// The email addresses one organisation's people agreed to the terms
    /// with, which is the only place the domain database holds them.
    pub async fn org_emails(&self, org: OrgId) -> Result<Vec<String>, StorageError> {
        Ok(sqlx::query_scalar!(
            r#"SELECT DISTINCT c.email AS "email!"
                 FROM account_consent c
                 JOIN app_user u ON u.auth_subject = c.subject
                WHERE u.org_id = $1 AND c.email IS NOT NULL AND c.email <> ''"#,
            uuid_to_db(org.0),
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// The email address a subject signed up with, where one was recorded.
    pub async fn signup_email(&self, subject: Uuid) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar!(
            "SELECT email FROM account_consent \
              WHERE subject = $1 AND email IS NOT NULL \
              ORDER BY accepted_at LIMIT 1",
            uuid_to_db(subject),
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// An unlink soon after a shop's free moves were granted, by an
    /// organisation that has spent moves: what is left of its free moves is
    /// taken back and the organisation is flagged.
    ///
    /// The farming loop's own shape (connect, spend, unlink, next account).
    /// The shop's five are already once-ever, so this does not protect them;
    /// it is the evidence that somebody is trying, raised where they try it.
    /// Answers whether the unlink was a quick one.
    pub async fn quick_unlink(
        &self,
        org: OrgId,
        connection: Uuid,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let quick = sqlx::query_scalar!(
            r#"SELECT c.marketplace AS "marketplace!"
                 FROM connection c
                 JOIN storefront_allowance a
                   ON a.org_id = c.org_id AND a.marketplace = c.marketplace
                WHERE c.org_id = $1 AND c.id = $2
                  AND a.granted_at > $3::timestamptz - make_interval(days => $4::int)
                  AND EXISTS (SELECT 1 FROM move_ledger l
                               WHERE l.org_id = c.org_id AND l.source = 'commit')
                LIMIT 1"#,
            uuid_to_db(org.0),
            uuid_to_db(connection),
            timestamp_to_db(at)?,
            i32::try_from(QUICK_UNLINK_DAYS).unwrap_or(i32::MAX),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(marketplace) = quick else {
            tx.commit().await?;
            return Ok(false);
        };
        let reference = format!("abuse:quick-unlink:{marketplace}");
        crate::entitlement::zero_free_moves_in(&mut tx, org, &reference, at).await?;
        raise_in(
            &mut tx,
            org,
            FlagKind::QuickUnlink,
            &format!(
                "Unlinked a {marketplace} shop within {QUICK_UNLINK_DAYS} days of its free moves, after spending moves."
            ),
            at,
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The organisation a flag names, where the flag exists.
    pub async fn flag_org(&self, flag: Uuid) -> Result<Option<OrgId>, StorageError> {
        Ok(sqlx::query_scalar!(
            "SELECT org_id FROM abuse_flag WHERE id = $1",
            uuid_to_db(flag),
        )
        .fetch_optional(&self.pool)
        .await?
        .map(|org| OrgId(uuid_from_db(org))))
    }

    /// An operator's decision. Answers the organisation it was about, or
    /// `None` for a flag that does not exist.
    ///
    /// One transaction: the flag and every other open flag of the
    /// organisation take the action; a warning or a ban queues its mail on
    /// the flag acted on; a limit or a ban takes back what is left of the free
    /// moves; a ban ends every session the organisation holds and writes what
    /// it refuses; any other decision about a banned organisation lifts the
    /// ban and deletes what it refused.
    pub async fn decide(&self, decision: Decision<'_>) -> Result<Option<OrgId>, StorageError> {
        let Decision {
            flag,
            action,
            reason,
            by,
            at,
            identities: _,
        } = decision;
        let now = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        let Some(org) = sqlx::query_scalar!(
            "SELECT org_id FROM abuse_flag WHERE id = $1 FOR UPDATE",
            uuid_to_db(flag),
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let org = OrgId(uuid_from_db(org));
        let before = standing_in(&mut tx, org).await?;
        let mails = action.mails();
        sqlx::query!(
            "UPDATE abuse_flag \
                SET resolved_at = $2::timestamptz, resolved_by = $3, action = $4, action_reason = $5, \
                    mail_requested_at = CASE WHEN $6::bool THEN $2::timestamptz END, \
                    mail_due_at = CASE WHEN $6::bool THEN $2::timestamptz END, \
                    mail_attempts = 0, mail_sent_at = NULL, mail_error = NULL \
              WHERE id = $1",
            uuid_to_db(flag),
            now,
            uuid_to_db(by),
            action.as_str(),
            reason,
            mails,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE abuse_flag \
                SET resolved_at = $3, resolved_by = $4, action = $5, action_reason = $6 \
              WHERE org_id = $1 AND id <> $2 AND resolved_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(flag),
            now,
            uuid_to_db(by),
            action.as_str(),
            reason,
        )
        .execute(&mut *tx)
        .await?;
        if action == AbuseAction::Ban {
            ban_in(&mut tx, org, &decision).await?;
        } else if before == AbuseAction::Ban {
            sqlx::query!(
                "DELETE FROM banned_identity WHERE origin_org = $1",
                uuid_to_db(org.0),
            )
            .execute(&mut *tx)
            .await?;
        }
        if action.withholds_free_moves() {
            pin_org(&mut tx, org).await?;
            let reference = format!("abuse:{}:{}", action.as_str(), uuid_to_db(flag));
            crate::entitlement::zero_free_moves_in(&mut tx, org, &reference, at).await?;
        }
        tx.commit().await?;
        Ok(Some(org))
    }

    /// Claims the next due warning or suspension mail.
    pub async fn claim_mail(
        &self,
        now: Timestamp,
        lease_until: Timestamp,
    ) -> Result<Option<ClaimedAbuseMail>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let claimed = sqlx::query!(
            "UPDATE abuse_flag SET mail_due_at = $2, mail_attempts = mail_attempts + 1 \
             WHERE id = ( \
                 SELECT id FROM abuse_flag \
                 WHERE mail_requested_at IS NOT NULL AND mail_sent_at IS NULL \
                   AND mail_due_at <= $1 AND mail_attempts < $3 \
                 ORDER BY mail_due_at LIMIT 1 FOR UPDATE SKIP LOCKED) \
             RETURNING id, org_id, action, mail_attempts",
            timestamp_to_db(now)?,
            timestamp_to_db(lease_until)?,
            ABUSE_MAIL_ATTEMPTS,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(claimed) = claimed else {
            tx.commit().await?;
            return Ok(None);
        };
        let org_name = sqlx::query_scalar!(
            "SELECT name FROM organisation WHERE id = $1",
            claimed.org_id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_default();
        let subjects = sqlx::query_scalar!(
            "SELECT auth_subject AS \"subject!\" FROM app_user \
             WHERE org_id = $1 AND auth_subject IS NOT NULL ORDER BY id",
            claimed.org_id,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(ClaimedAbuseMail {
            flag: uuid_from_db(claimed.id),
            action: AbuseAction::from_db(&claimed.action)?,
            org_name,
            attempts: claimed.mail_attempts,
            subjects: subjects.into_iter().map(uuid_from_db).collect(),
        }))
    }

    /// Records what one claimed mail came to.
    pub async fn settle_mail(
        &self,
        flag: Uuid,
        outcome: &AbuseMailOutcome,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let now = timestamp_to_db(now)?;
        match outcome {
            AbuseMailOutcome::Sent => {
                sqlx::query!(
                    "UPDATE abuse_flag SET mail_sent_at = $2, mail_due_at = NULL, mail_error = NULL \
                     WHERE id = $1",
                    uuid_to_db(flag),
                    now,
                )
                .execute(&self.pool)
                .await?;
            }
            AbuseMailOutcome::Retry { error, at } => {
                sqlx::query!(
                    "UPDATE abuse_flag SET mail_due_at = $2, mail_error = $3 WHERE id = $1",
                    uuid_to_db(flag),
                    timestamp_to_db(*at)?,
                    error,
                )
                .execute(&self.pool)
                .await?;
            }
            AbuseMailOutcome::Failed { error } => {
                sqlx::query!(
                    "UPDATE abuse_flag SET mail_due_at = NULL, mail_error = $2 WHERE id = $1",
                    uuid_to_db(flag),
                    error,
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    /// The nightly retention: addresses and browsers older than
    /// [`SHORT_SIGNAL_DAYS`], and bans past their expiry.
    pub async fn prune(&self, now: Timestamp) -> Result<Pruned, StorageError> {
        let at = timestamp_to_db(now)?;
        let signals = sqlx::query!(
            "DELETE FROM account_link_signal \
              WHERE kind IN ('ip', 'user_agent_hash') \
                AND last_seen < $1::timestamptz - make_interval(days => $2::int)",
            at,
            i32::try_from(SHORT_SIGNAL_DAYS).unwrap_or(i32::MAX),
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        let bans = sqlx::query!("DELETE FROM banned_identity WHERE expires_at <= $1", at)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(Pruned { signals, bans })
    }

    /// Every card fingerprint the payments ledger (migration 0102) has seen
    /// on a successful charge it could attribute, newest first, for the
    /// caller to digest and record. `charge` narrows to one charge.
    pub async fn card_fingerprints(
        &self,
        charge: Option<&str>,
    ) -> Result<Vec<(OrgId, String, Timestamp)>, StorageError> {
        let rows = sqlx::query!(
            r#"SELECT org_id AS "org!",
                      raw -> 'payment_method_details' -> 'card' ->> 'fingerprint' AS "fingerprint!",
                      max(occurred_at) AS "at!"
                 FROM payment_event
                WHERE kind = 'payment_succeeded' AND org_id IS NOT NULL
                  AND raw -> 'payment_method_details' -> 'card' ->> 'fingerprint' IS NOT NULL
                  AND ($1::text IS NULL OR provider_object_id = $1)
                GROUP BY 1, 2"#,
            charge,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    OrgId(uuid_from_db(row.org)),
                    row.fingerprint,
                    timestamp_from_db(row.at),
                )
            })
            .collect())
    }
}

/// A ban's own writes, inside [`AbuseRepo::decide`]'s transaction.
///
/// Every session the organisation holds ends here, so the next request meets
/// the suspension rather than a console. What is refused afterwards is the
/// caller's digests (addresses, device ids) and whatever the database already
/// holds as one: every shop the organisation ever claimed or bound, and every
/// device and card the ledger saw. The shop rows need the pin; the rest is
/// global.
async fn ban_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    decision: &Decision<'_>,
) -> Result<(), StorageError> {
    let Decision {
        flag,
        reason,
        at,
        identities,
        ..
    } = *decision;
    let now = timestamp_to_db(at)?;
    let until = now
        .checked_add_months(chrono::Months::new(BAN_MONTHS))
        .ok_or(StorageError::TimestampOutOfRange { millis: at.0 })?;
    let reason = reason
        .filter(|said| !said.is_empty())
        .unwrap_or("Suspended");
    sqlx::query!(
        "DELETE FROM user_session WHERE org_id = $1",
        uuid_to_db(org.0)
    )
    .execute(&mut **tx)
    .await?;
    for (kind, value) in identities {
        sqlx::query!(
            "INSERT INTO banned_identity \
             (kind, value_hash, origin_org, flag_id, reason, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (kind, value_hash) DO NOTHING",
            kind.as_str(),
            &value[..],
            uuid_to_db(org.0),
            uuid_to_db(flag),
            reason,
            now,
            until,
        )
        .execute(&mut **tx)
        .await?;
    }
    pin_org(tx, org).await?;
    sqlx::query!(
        r#"INSERT INTO banned_identity
               (kind, value_hash, origin_org, flag_id, reason, created_at, expires_at)
           SELECT DISTINCT
                  CASE s.kind WHEN 'shop_digest' THEN 'shop_digest'
                              WHEN 'device_fingerprint' THEN 'device_fingerprint'
                              ELSE 'payment_fingerprint' END,
                  s.value_hash, $1::uuid, $2::uuid, $3::text, $4::timestamptz, $5::timestamptz
             FROM (SELECT kind, value_hash FROM account_link_signal
                    WHERE org_id = $1
                      AND kind IN ('shop_digest', 'device_fingerprint', 'payment_fingerprint')
                   UNION
                   SELECT 'shop_digest', platform_account_digest FROM storefront_allowance
                    WHERE org_id = $1
                   UNION
                   SELECT 'shop_digest', platform_account_digest FROM connection
                    WHERE org_id = $1 AND platform_account_digest IS NOT NULL
                      AND octet_length(platform_account_digest) = 32) s
           ON CONFLICT (kind, value_hash) DO NOTHING"#,
        uuid_to_db(org.0),
        uuid_to_db(flag),
        reason,
        now,
        until,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The counters at the top of the Abuse page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AbuseCounters {
    pub open_orgs: i64,
    pub open_flags: i64,
    pub warned: i64,
    pub limited: i64,
    pub banned: i64,
    pub banned_identities: i64,
    pub signals: i64,
}

/// One flagged organisation, as the Abuse page's table lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlaggedOrg {
    pub org: OrgId,
    pub name: String,
    pub slug: Option<String>,
    pub score: i64,
    pub kinds: Vec<FlagKind>,
    pub signal_kinds: Vec<SignalKind>,
    pub linked_orgs: i64,
    pub standing: AbuseAction,
    pub open_flags: i64,
    pub last_flagged_at: Timestamp,
    pub flag: Uuid,
}

/// One flag, as an organisation's cluster view lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagRecord {
    pub id: Uuid,
    pub kind: FlagKind,
    pub score: i32,
    pub reason: String,
    pub created_at: Timestamp,
    pub resolved_at: Option<Timestamp>,
    pub resolved_by: Option<Uuid>,
    pub action: AbuseAction,
    pub action_reason: Option<String>,
    pub mail_sent_at: Option<Timestamp>,
    pub mail_error: Option<String>,
}

/// One signal an organisation holds, and how many others hold it too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRecord {
    pub kind: SignalKind,
    pub value: Vec<u8>,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub shared_with: i64,
}

/// Another organisation sharing a linking signal with this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedOrg {
    pub org: OrgId,
    pub name: String,
    pub standing: AbuseAction,
    pub shared: Vec<SignalKind>,
}

/// One organisation's cluster view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbuseOrgDetail {
    pub org: OrgId,
    pub name: String,
    pub slug: Option<String>,
    pub created_at: Timestamp,
    pub standing: AbuseAction,
    pub flags: Vec<FlagRecord>,
    pub signals: Vec<SignalRecord>,
    pub linked: Vec<LinkedOrg>,
}

/// What a search names on the Abuse page, once the caller has read it.
#[derive(Debug, Clone, Copy)]
pub enum AbuseSearch<'a> {
    /// Every organisation a person who agreed with this address belongs to.
    Email(&'a str),
    /// Every organisation holding this digest, of these kinds.
    Digest(&'a [u8; 32], &'a [SignalKind]),
    /// Every organisation holding a digest whose hex starts with this.
    HexPrefix(&'a str),
    /// Organisation name or slug.
    Name(&'a str),
}

/// The Abuse page's reads, on the backoffice pool.
pub struct AbuseBackofficeRepo {
    pool: PgPool,
}

fn kinds_from_db<T>(raw: &[String], parse: fn(&str) -> Option<T>) -> Result<Vec<T>, StorageError> {
    raw.iter()
        .map(|kind| {
            parse(kind).ok_or_else(|| StorageError::CorruptRow {
                reason: format!("unknown abuse kind {kind:?}"),
            })
        })
        .collect()
}

impl AbuseBackofficeRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn counters(&self) -> Result<AbuseCounters, StorageError> {
        let row = sqlx::query!(
            r#"WITH standing AS (
                   SELECT abuse_standing(org_id) AS action
                     FROM (SELECT DISTINCT org_id FROM abuse_flag
                            WHERE resolved_at IS NOT NULL) decided
               )
               SELECT
                 (SELECT count(DISTINCT org_id) FROM abuse_flag
                   WHERE resolved_at IS NULL)                              AS "open_orgs!",
                 (SELECT count(*) FROM abuse_flag WHERE resolved_at IS NULL) AS "open_flags!",
                 (SELECT count(*) FROM standing WHERE action = 'warn')     AS "warned!",
                 (SELECT count(*) FROM standing WHERE action = 'limit')    AS "limited!",
                 (SELECT count(*) FROM standing WHERE action = 'ban')      AS "banned!",
                 (SELECT count(*) FROM banned_identity)                    AS "banned_identities!",
                 (SELECT count(*) FROM account_link_signal)                AS "signals!""#
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(AbuseCounters {
            open_orgs: row.open_orgs,
            open_flags: row.open_flags,
            warned: row.warned,
            limited: row.limited,
            banned: row.banned,
            banned_identities: row.banned_identities,
            signals: row.signals,
        })
    }

    /// The organisations a search names. Answers them whether or not they
    /// have been flagged; the caller narrows.
    pub async fn search(&self, search: AbuseSearch<'_>) -> Result<Vec<OrgId>, StorageError> {
        let rows = match search {
            AbuseSearch::Email(email) => {
                sqlx::query_scalar!(
                    r#"SELECT DISTINCT u.org_id AS "org!"
                         FROM account_consent c
                         JOIN app_user u ON u.auth_subject = c.subject
                        WHERE lower(c.email) = lower($1)"#,
                    email.trim(),
                )
                .fetch_all(&self.pool)
                .await?
            }
            AbuseSearch::Digest(value, kinds) => {
                let kinds: Vec<String> =
                    kinds.iter().map(|kind| kind.as_str().to_owned()).collect();
                sqlx::query_scalar!(
                    r#"SELECT DISTINCT org_id AS "org!" FROM account_link_signal
                        WHERE value_hash = $1 AND kind = ANY($2)
                       UNION
                       SELECT DISTINCT org_id FROM connection
                        WHERE platform_account_digest = $1 AND 'shop_digest' = ANY($2)"#,
                    &value[..],
                    &kinds,
                )
                .fetch_all(&self.pool)
                .await?
            }
            AbuseSearch::HexPrefix(prefix) => {
                sqlx::query_scalar!(
                    r#"SELECT DISTINCT org_id AS "org!" FROM account_link_signal
                        WHERE encode(value_hash, 'hex') LIKE $1 || '%'"#,
                    prefix.to_ascii_lowercase(),
                )
                .fetch_all(&self.pool)
                .await?
            }
            AbuseSearch::Name(name) => {
                sqlx::query_scalar!(
                    r#"SELECT id AS "org!" FROM organisation
                        WHERE strpos(lower(name), lower($1)) > 0
                           OR strpos(lower(COALESCE(slug, '')), lower($1)) > 0
                        LIMIT 200"#,
                    name.trim(),
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        Ok(rows
            .into_iter()
            .map(|org| OrgId(uuid_from_db(org)))
            .collect())
    }

    /// The flagged organisations: those with an open flag, or every one that
    /// ever had a flag when `all`, narrowed to `only` when given. Highest
    /// open score first, then the newest flag.
    pub async fn flagged(
        &self,
        all: bool,
        only: Option<&[OrgId]>,
    ) -> Result<Vec<FlaggedOrg>, StorageError> {
        let only: Option<Vec<uuid::Uuid>> =
            only.map(|orgs| orgs.iter().map(|org| uuid_to_db(org.0)).collect());
        let rows = sqlx::query!(
            r#"SELECT o.id AS "org!", o.name AS "name!", o.slug,
                      COALESCE(sum(f.score) FILTER (WHERE f.resolved_at IS NULL), 0)::bigint
                                                                          AS "score!",
                      COALESCE(array_agg(DISTINCT f.kind) FILTER (WHERE f.resolved_at IS NULL),
                               '{}')                                      AS "open_kinds!",
                      array_agg(DISTINCT f.kind)                          AS "all_kinds!",
                      count(*) FILTER (WHERE f.resolved_at IS NULL)       AS "open_flags!",
                      max(f.created_at)                                   AS "last_flagged_at!",
                      (array_agg(f.id ORDER BY f.created_at DESC, f.id DESC))[1] AS "flag!",
                      abuse_standing(o.id)                                AS "standing!"
                 FROM abuse_flag f
                 JOIN organisation o ON o.id = f.org_id
                WHERE ($1 OR EXISTS (SELECT 1 FROM abuse_flag x
                                      WHERE x.org_id = f.org_id AND x.resolved_at IS NULL))
                  AND ($2::uuid[] IS NULL OR o.id = ANY($2))
                GROUP BY o.id
                ORDER BY 4 DESC, 8 DESC
                LIMIT 500"#,
            all,
            only.as_deref(),
        )
        .fetch_all(&self.pool)
        .await?;
        let orgs: Vec<uuid::Uuid> = rows.iter().map(|row| row.org).collect();
        let links = sqlx::query!(
            r#"SELECT a.org_id AS "org!",
                      array_agg(DISTINCT a.kind) AS "kinds!",
                      count(DISTINCT b.org_id)   AS "linked!"
                 FROM account_link_signal a
                 JOIN account_link_signal b
                   ON b.kind = a.kind AND b.value_hash = a.value_hash AND b.org_id <> a.org_id
                WHERE a.org_id = ANY($1)
                  AND a.kind IN ('shop_digest', 'device_fingerprint', 'ip', 'payment_fingerprint')
                GROUP BY a.org_id"#,
            &orgs,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let link = links.iter().find(|link| link.org == row.org);
                let kinds = if row.open_kinds.is_empty() {
                    row.all_kinds
                } else {
                    row.open_kinds
                };
                Ok(FlaggedOrg {
                    org: OrgId(uuid_from_db(row.org)),
                    name: row.name,
                    slug: row.slug,
                    score: row.score,
                    kinds: kinds_from_db(&kinds, FlagKind::parse)?,
                    signal_kinds: match link {
                        Some(link) => kinds_from_db(&link.kinds, SignalKind::parse)?,
                        None => Vec::new(),
                    },
                    linked_orgs: link.map_or(0, |link| link.linked),
                    standing: AbuseAction::from_db(&row.standing)?,
                    open_flags: row.open_flags,
                    last_flagged_at: timestamp_from_db(row.last_flagged_at),
                    flag: uuid_from_db(row.flag),
                })
            })
            .collect()
    }

    /// One organisation's cluster: its flags, its signals and who shares
    /// them. `None` for an organisation with no row.
    pub async fn org(&self, org: OrgId) -> Result<Option<AbuseOrgDetail>, StorageError> {
        let id = uuid_to_db(org.0);
        let Some(head) = sqlx::query!(
            r#"SELECT name, slug, created_at, abuse_standing(id) AS "standing!"
                 FROM organisation WHERE id = $1"#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let flags = sqlx::query!(
            "SELECT id, kind, score, reason, created_at, resolved_at, resolved_by, action, \
                    action_reason, mail_sent_at, mail_error \
               FROM abuse_flag WHERE org_id = $1 \
              ORDER BY created_at DESC, id DESC",
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        let signals = sqlx::query!(
            r#"SELECT s.kind, s.value_hash, s.first_seen, s.last_seen,
                      (SELECT count(DISTINCT o.org_id) FROM account_link_signal o
                        WHERE o.kind = s.kind AND o.value_hash = s.value_hash
                          AND o.org_id <> s.org_id) AS "shared_with!"
                 FROM account_link_signal s
                WHERE s.org_id = $1
                ORDER BY s.kind, s.last_seen DESC"#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        let linked = sqlx::query!(
            r#"SELECT b.org_id AS "org!", o.name AS "name!",
                      abuse_standing(b.org_id) AS "standing!",
                      array_agg(DISTINCT b.kind) AS "kinds!"
                 FROM account_link_signal a
                 JOIN account_link_signal b
                   ON b.kind = a.kind AND b.value_hash = a.value_hash AND b.org_id <> a.org_id
                 JOIN organisation o ON o.id = b.org_id
                WHERE a.org_id = $1
                  AND a.kind IN ('shop_digest', 'device_fingerprint', 'ip', 'payment_fingerprint')
                GROUP BY b.org_id, o.name
                ORDER BY o.name"#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(AbuseOrgDetail {
            org,
            name: head.name,
            slug: head.slug,
            created_at: timestamp_from_db(head.created_at),
            standing: AbuseAction::from_db(&head.standing)?,
            flags: flags
                .into_iter()
                .map(|row| {
                    Ok(FlagRecord {
                        id: uuid_from_db(row.id),
                        kind: FlagKind::parse(&row.kind).ok_or_else(|| {
                            StorageError::CorruptRow {
                                reason: format!("unknown flag kind {:?}", row.kind),
                            }
                        })?,
                        score: row.score,
                        reason: row.reason,
                        created_at: timestamp_from_db(row.created_at),
                        resolved_at: row.resolved_at.map(timestamp_from_db),
                        resolved_by: row.resolved_by.map(uuid_from_db),
                        action: AbuseAction::from_db(&row.action)?,
                        action_reason: row.action_reason,
                        mail_sent_at: row.mail_sent_at.map(timestamp_from_db),
                        mail_error: row.mail_error,
                    })
                })
                .collect::<Result<_, StorageError>>()?,
            signals: signals
                .into_iter()
                .map(|row| {
                    Ok(SignalRecord {
                        kind: SignalKind::parse(&row.kind).ok_or_else(|| {
                            StorageError::CorruptRow {
                                reason: format!("unknown signal kind {:?}", row.kind),
                            }
                        })?,
                        value: row.value_hash,
                        first_seen: timestamp_from_db(row.first_seen),
                        last_seen: timestamp_from_db(row.last_seen),
                        shared_with: row.shared_with,
                    })
                })
                .collect::<Result<_, StorageError>>()?,
            linked: linked
                .into_iter()
                .map(|row| {
                    Ok(LinkedOrg {
                        org: OrgId(uuid_from_db(row.org)),
                        name: row.name,
                        standing: AbuseAction::from_db(&row.standing)?,
                        shared: kinds_from_db(&row.kinds, SignalKind::parse)?,
                    })
                })
                .collect::<Result<_, StorageError>>()?,
        }))
    }
}
