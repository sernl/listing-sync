//! The payments ledger: every money event Stripe reports, and every refund an
//! operator issues (migration 0102).
//!
//! Everything here runs on the application pool, which owns both tables by
//! owning the database. Neither is fenced: only operator routes and the
//! signed webhook reach them. The one read that crosses the tenant fence —
//! which organisation a Stripe customer belongs to — is
//! [`PaymentBackofficeRepo`], on the backoffice pool.
//!
//! A row lands three ways, and the keys keep them from doubling up. The
//! webhook writes one row per Stripe event under the event's own id. The sync
//! writes one row per Stripe object under `sync:<kind>:<object id>`, and
//! skips an object a webhook row already describes. A webhook row arriving
//! after a sync row for the same object replaces it, so the ledger holds the
//! event Stripe actually sent rather than our reconstruction of it.

use std::collections::{BTreeMap, HashMap};

use sqlx::{PgPool, Postgres, Transaction};
use tam_types::{OrgId, RefundDeclinedNotice, RefundRequestedNotice, Timestamp, UserId, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, NewOutboxMessage, OutboxRepo, StorageError};

/// The `site_setting` key the auto refund mail switch lives under.
pub const AUTO_REFUND_MAIL_SETTING: &str = "payments.auto_refund_mail";

/// The prefix a sync-written row's `provider_event_id` carries.
pub const SYNC_EVENT_PREFIX: &str = "sync:";

/// How many times a refund mail is tried before it is left failed.
pub const REFUND_MAIL_ATTEMPTS: i32 = 5;

/// The topic the operators' mail about a seller's refund request is drained
/// from, in the `email.` family beside the marketplace request's.
pub const REFUND_REQUESTED_TOPIC: &str = "email.refund_requested";

/// The topic the seller's mail about a declined refund request is drained
/// from.
pub const REFUND_DECLINED_TOPIC: &str = "email.refund_declined";

/// What one ledger row records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaymentKind {
    PaymentSucceeded,
    PaymentFailed,
    RefundCreated,
    RefundUpdated,
    DisputeOpened,
    DisputeClosed,
    InvoicePaid,
    InvoicePaymentFailed,
    SubscriptionCreated,
    SubscriptionCanceled,
}

impl PaymentKind {
    pub const ALL: [Self; 10] = [
        Self::PaymentSucceeded,
        Self::PaymentFailed,
        Self::RefundCreated,
        Self::RefundUpdated,
        Self::DisputeOpened,
        Self::DisputeClosed,
        Self::InvoicePaid,
        Self::InvoicePaymentFailed,
        Self::SubscriptionCreated,
        Self::SubscriptionCanceled,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PaymentSucceeded => "payment_succeeded",
            Self::PaymentFailed => "payment_failed",
            Self::RefundCreated => "refund_created",
            Self::RefundUpdated => "refund_updated",
            Self::DisputeOpened => "dispute_opened",
            Self::DisputeClosed => "dispute_closed",
            Self::InvoicePaid => "invoice_paid",
            Self::InvoicePaymentFailed => "invoice_payment_failed",
            Self::SubscriptionCreated => "subscription_created",
            Self::SubscriptionCanceled => "subscription_canceled",
        }
    }

    /// Whether Stripe describes one object with at most one event of this
    /// kind. Every kind is, except a refund's updates, which repeat as its
    /// status moves; only these may be replaced by a later webhook row.
    #[must_use]
    pub const fn once_per_object(self) -> bool {
        !matches!(self, Self::RefundUpdated)
    }

    /// The kind a stored string names.
    ///
    /// # Errors
    /// A string migration 0102's check would have refused.
    pub fn parse(raw: &str) -> Result<Self, StorageError> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == raw)
            .ok_or_else(|| StorageError::CorruptRow {
                reason: format!("payment_event.kind {raw:?} is not a known kind"),
            })
    }
}

/// One ledger row.
#[derive(Debug, Clone, PartialEq)]
pub struct PaymentEvent {
    pub id: Uuid,
    pub org: Option<OrgId>,
    pub provider_event_id: String,
    pub kind: PaymentKind,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
    pub provider_object_id: String,
    pub charge_id: Option<String>,
    pub payment_intent_id: Option<String>,
    pub invoice_id: Option<String>,
    pub customer_id: Option<String>,
    pub status: Option<String>,
    pub reason: Option<String>,
    pub occurred_at: Timestamp,
    /// The Stripe object as it arrived. Written, never listed: the page has
    /// no use for it and it is the bulk of the row.
    pub raw: serde_json::Value,
    pub created_at: Timestamp,
}

/// A refund's standing, Stripe's vocabulary narrowed to four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundStatus {
    Pending,
    Succeeded,
    Failed,
    Canceled,
}

impl RefundStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Stripe's status, with `requires_action` and anything unknown read as
    /// pending: neither is money returned or money kept yet.
    #[must_use]
    pub fn from_stripe(raw: &str) -> Self {
        match raw {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _pending => Self::Pending,
        }
    }

    fn parse(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "pending" => Ok(Self::Pending),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            other => Err(StorageError::CorruptRow {
                reason: format!("refund.status {other:?} is not a known status"),
            }),
        }
    }

    /// Whether the refund holds back part of its charge: a failed or
    /// cancelled refund returned nothing and leaves the amount refundable.
    #[must_use]
    pub const fn counts(self) -> bool {
        matches!(self, Self::Pending | Self::Succeeded)
    }
}

/// One refund row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refund {
    pub id: Uuid,
    pub org: Option<OrgId>,
    pub provider_refund_id: String,
    pub charge_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub status: RefundStatus,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub issued_by: Option<UserId>,
    pub issued_by_label: Option<String>,
    pub mail_requested_at: Option<Timestamp>,
    pub mail_sent_at: Option<Timestamp>,
    pub mail_error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The rule of the refund policy that quoted this refund, as tam-api's
    /// `RefundBasis` spells it, where it was issued against a quote
    /// (migration 0107).
    pub policy_basis: Option<String>,
    /// What that rule gave, beside `amount_cents`, which is what was refunded.
    pub quoted_cents: Option<i64>,
    /// Why the operator refunded other than the quote. Required by the
    /// database whenever the two differ.
    pub override_reason: Option<String>,
}

/// Where a seller's refund request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundRequestStatus {
    Requested,
    Approved,
    Declined,
}

impl RefundRequestStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Approved => "approved",
            Self::Declined => "declined",
        }
    }

    fn parse(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "requested" => Ok(Self::Requested),
            "approved" => Ok(Self::Approved),
            "declined" => Ok(Self::Declined),
            other => Err(StorageError::CorruptRow {
                reason: format!("refund_request.status {other:?} is not a known status"),
            }),
        }
    }
}

/// One seller's refund request (migration 0107).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundRequest {
    pub id: Uuid,
    pub org: OrgId,
    pub charge_id: String,
    pub quoted_cents: i64,
    pub currency: String,
    pub policy_basis: String,
    pub status: RefundRequestStatus,
    pub note: Option<String>,
    pub requested_by: Option<UserId>,
    pub decided_by_label: Option<String>,
    pub decided_at: Option<Timestamp>,
    pub decline_reason: Option<String>,
    pub refund_id: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A new refund request, as the seller's route composes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRefundRequest<'a> {
    pub id: Uuid,
    pub org: OrgId,
    pub charge_id: &'a str,
    pub quoted_cents: i64,
    pub currency: &'a str,
    pub policy_basis: &'a str,
    /// The policy's sentence about the quote, which the operators' mail
    /// carries. Not stored: the quote route recomputes it on demand.
    pub explanation: &'a str,
    pub note: &'a str,
    pub requested_by: UserId,
    pub at: Timestamp,
}

/// What asking for a refund did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefundRequestWrite {
    Recorded(Box<RefundRequest>),
    /// The charge already has a request waiting for an operator.
    AlreadyOpen,
}

/// Who decided a request, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision<'a> {
    pub by: UserId,
    pub label: &'a str,
    pub at: Timestamp,
}

/// What deciding a request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefundDecision {
    Decided(Box<RefundRequest>),
    /// It was already approved or declined; the row as it stands.
    AlreadyDecided(Box<RefundRequest>),
    NoRequest,
}

/// A refund Stripe reports that this page may not have issued: from the
/// webhook or the sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedRefund<'a> {
    pub provider_refund_id: &'a str,
    pub charge_id: &'a str,
    pub org: Option<OrgId>,
    pub amount_cents: i64,
    pub currency: &'a str,
    pub status: RefundStatus,
    pub reason: Option<&'a str>,
    pub at: Timestamp,
    /// Queue the customer's mail if the refund is new to us: the auto
    /// setting is on and the refund names an organisation to mail.
    pub queue_mail: bool,
}

/// One claimed refund mail: what the drainer needs to compose and address it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedRefundMail {
    pub refund: Uuid,
    pub org_name: String,
    pub amount_cents: i64,
    pub currency: String,
    pub attempts: i32,
    /// The platform subjects of the organisation's people: the addresses the
    /// identity service holds for them are where the mail goes.
    pub subjects: Vec<Uuid>,
}

/// What became of one refund mail attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefundMailOutcome {
    Sent,
    Retry { error: String, at: Timestamp },
    Failed { error: String },
}

/// What asking for a refund's mail did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailRequest {
    Queued(Box<Refund>),
    AlreadySent(Timestamp),
    NoOrganisation,
    NoRefund,
}

pub struct PaymentRepo {
    pool: PgPool,
}

struct EventRow {
    id: uuid::Uuid,
    org_id: Option<uuid::Uuid>,
    provider_event_id: String,
    kind: String,
    amount_cents: Option<i64>,
    currency: Option<String>,
    provider_object_id: String,
    charge_id: Option<String>,
    payment_intent_id: Option<String>,
    invoice_id: Option<String>,
    customer_id: Option<String>,
    status: Option<String>,
    reason: Option<String>,
    occurred_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<EventRow> for PaymentEvent {
    type Error = StorageError;

    fn try_from(row: EventRow) -> Result<Self, StorageError> {
        Ok(Self {
            id: uuid_from_db(row.id),
            org: row.org_id.map(|id| OrgId(uuid_from_db(id))),
            provider_event_id: row.provider_event_id,
            kind: PaymentKind::parse(&row.kind)?,
            amount_cents: row.amount_cents,
            currency: row.currency,
            provider_object_id: row.provider_object_id,
            charge_id: row.charge_id,
            payment_intent_id: row.payment_intent_id,
            invoice_id: row.invoice_id,
            customer_id: row.customer_id,
            status: row.status,
            reason: row.reason,
            occurred_at: timestamp_from_db(row.occurred_at),
            raw: serde_json::Value::Null,
            created_at: timestamp_from_db(row.created_at),
        })
    }
}

struct RefundRow {
    id: uuid::Uuid,
    org_id: Option<uuid::Uuid>,
    provider_refund_id: String,
    charge_id: String,
    amount_cents: i64,
    currency: String,
    status: String,
    reason: Option<String>,
    note: Option<String>,
    issued_by: Option<uuid::Uuid>,
    issued_by_label: Option<String>,
    mail_requested_at: Option<chrono::DateTime<chrono::Utc>>,
    mail_sent_at: Option<chrono::DateTime<chrono::Utc>>,
    mail_error: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    policy_basis: Option<String>,
    quoted_cents: Option<i64>,
    override_reason: Option<String>,
}

impl TryFrom<RefundRow> for Refund {
    type Error = StorageError;

    fn try_from(row: RefundRow) -> Result<Self, StorageError> {
        Ok(Self {
            id: uuid_from_db(row.id),
            org: row.org_id.map(|id| OrgId(uuid_from_db(id))),
            provider_refund_id: row.provider_refund_id,
            charge_id: row.charge_id,
            amount_cents: row.amount_cents,
            currency: row.currency,
            status: RefundStatus::parse(&row.status)?,
            reason: row.reason,
            note: row.note,
            issued_by: row.issued_by.map(|id| UserId(uuid_from_db(id))),
            issued_by_label: row.issued_by_label,
            mail_requested_at: row.mail_requested_at.map(timestamp_from_db),
            mail_sent_at: row.mail_sent_at.map(timestamp_from_db),
            mail_error: row.mail_error,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
            policy_basis: row.policy_basis,
            quoted_cents: row.quoted_cents,
            override_reason: row.override_reason,
        })
    }
}

struct RequestRow {
    id: uuid::Uuid,
    org_id: uuid::Uuid,
    charge_id: String,
    quoted_cents: i64,
    currency: String,
    policy_basis: String,
    status: String,
    note: Option<String>,
    requested_by: Option<uuid::Uuid>,
    decided_by_label: Option<String>,
    decided_at: Option<chrono::DateTime<chrono::Utc>>,
    decline_reason: Option<String>,
    refund_id: Option<uuid::Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<RequestRow> for RefundRequest {
    type Error = StorageError;

    fn try_from(row: RequestRow) -> Result<Self, StorageError> {
        Ok(Self {
            id: uuid_from_db(row.id),
            org: OrgId(uuid_from_db(row.org_id)),
            charge_id: row.charge_id,
            quoted_cents: row.quoted_cents,
            currency: row.currency,
            policy_basis: row.policy_basis,
            status: RefundRequestStatus::parse(&row.status)?,
            note: row.note,
            requested_by: row.requested_by.map(|id| UserId(uuid_from_db(id))),
            decided_by_label: row.decided_by_label,
            decided_at: row.decided_at.map(timestamp_from_db),
            decline_reason: row.decline_reason,
            refund_id: row.refund_id.map(uuid_from_db),
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        })
    }
}

fn fresh_uuid() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

impl PaymentRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records one webhook event, answering whether it was new.
    ///
    /// A redelivery of the same Stripe event is a no-op on the unique
    /// `provider_event_id`. A sync row describing the same object is replaced
    /// in the same transaction, for the kinds Stripe sends once per object.
    pub async fn record_event(&self, event: &PaymentEvent) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        let inserted = insert_event(&mut tx, event).await?;
        if inserted && event.kind.once_per_object() {
            sqlx::query!(
                "DELETE FROM payment_event \
                 WHERE kind = $1 AND provider_object_id = $2 \
                   AND starts_with(provider_event_id, $3) AND id <> $4",
                event.kind.as_str(),
                event.provider_object_id,
                SYNC_EVENT_PREFIX,
                uuid_to_db(event.id),
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(inserted)
    }

    /// Records one object the sync read, answering whether a row was written
    /// or refreshed.
    ///
    /// An object a webhook row already describes under the same kind is left
    /// alone: Stripe's own event is the better record. A sync row written
    /// before is refreshed in place, so a charge's later status — refunded,
    /// disputed — reaches the page on the next sync.
    pub async fn record_synced(&self, event: &PaymentEvent) -> Result<bool, StorageError> {
        let described = sqlx::query_scalar!(
            "SELECT count(*) AS \"count!\" FROM payment_event \
             WHERE kind = $1 AND provider_object_id = $2 \
               AND NOT starts_with(provider_event_id, $3)",
            event.kind.as_str(),
            event.provider_object_id,
            SYNC_EVENT_PREFIX,
        )
        .fetch_one(&self.pool)
        .await?;
        if described > 0 {
            return Ok(false);
        }
        let written = sqlx::query!(
            "INSERT INTO payment_event \
                 (id, org_id, provider_event_id, kind, amount_cents, currency, \
                  provider_object_id, charge_id, payment_intent_id, invoice_id, customer_id, \
                  status, reason, occurred_at, raw, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
             ON CONFLICT (provider_event_id) DO UPDATE SET \
                 org_id = coalesce(EXCLUDED.org_id, payment_event.org_id), \
                 amount_cents = EXCLUDED.amount_cents, \
                 currency = EXCLUDED.currency, \
                 charge_id = coalesce(EXCLUDED.charge_id, payment_event.charge_id), \
                 payment_intent_id = \
                     coalesce(EXCLUDED.payment_intent_id, payment_event.payment_intent_id), \
                 invoice_id = coalesce(EXCLUDED.invoice_id, payment_event.invoice_id), \
                 customer_id = coalesce(EXCLUDED.customer_id, payment_event.customer_id), \
                 status = EXCLUDED.status, \
                 reason = EXCLUDED.reason, \
                 occurred_at = EXCLUDED.occurred_at, \
                 raw = EXCLUDED.raw",
            uuid_to_db(event.id),
            event.org.map(|org| uuid_to_db(org.0)),
            event.provider_event_id,
            event.kind.as_str(),
            event.amount_cents,
            event.currency,
            event.provider_object_id,
            event.charge_id,
            event.payment_intent_id,
            event.invoice_id,
            event.customer_id,
            event.status,
            event.reason,
            timestamp_to_db(event.occurred_at)?,
            event.raw,
            timestamp_to_db(event.created_at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(written.rows_affected() > 0)
    }

    /// Every ledger row that occurred at or after `since`, newest first, up
    /// to `limit`. `raw` is not read.
    pub async fn events_since(
        &self,
        since: Timestamp,
        limit: i64,
    ) -> Result<Vec<PaymentEvent>, StorageError> {
        let rows = sqlx::query_as!(
            EventRow,
            "SELECT id, org_id, provider_event_id, kind, amount_cents, currency, \
                    provider_object_id, charge_id, payment_intent_id, invoice_id, customer_id, \
                    status, reason, occurred_at, created_at \
             FROM payment_event WHERE occurred_at >= $1 \
             ORDER BY occurred_at DESC, id LIMIT $2",
            timestamp_to_db(since)?,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(PaymentEvent::try_from).collect()
    }

    /// The organisation the ledger already links to any of these ids — a
    /// charge, payment intent or invoice — where one row names one. How a
    /// refund or dispute, whose own object carries no metadata of ours,
    /// inherits the organisation of the charge it is about.
    pub async fn org_for_objects(&self, ids: &[String]) -> Result<Option<OrgId>, StorageError> {
        if ids.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query_scalar!(
            "SELECT org_id FROM payment_event \
             WHERE org_id IS NOT NULL \
               AND (provider_object_id = ANY($1) OR charge_id = ANY($1) \
                    OR payment_intent_id = ANY($1) OR invoice_id = ANY($1)) \
             ORDER BY occurred_at LIMIT 1",
            ids,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.flatten().map(|id| OrgId(uuid_from_db(id))))
    }

    /// Which organisation each Stripe customer the ledger has linked belongs
    /// to, from the rows that name both.
    pub async fn customer_orgs(&self) -> Result<BTreeMap<String, OrgId>, StorageError> {
        let rows = sqlx::query!(
            "SELECT DISTINCT ON (customer_id) customer_id AS \"customer!\", \
                    org_id AS \"org!\" \
             FROM payment_event \
             WHERE customer_id IS NOT NULL AND org_id IS NOT NULL \
             ORDER BY customer_id, occurred_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.customer, OrgId(uuid_from_db(row.org))))
            .collect())
    }

    /// Organisation names by id, for the page's Org column. An id with no
    /// organisation left (an erased seller) is absent.
    pub async fn org_names(&self, ids: &[OrgId]) -> Result<HashMap<OrgId, String>, StorageError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<uuid::Uuid> = ids.iter().map(|org| uuid_to_db(org.0)).collect();
        let rows = sqlx::query!("SELECT id, name FROM organisation WHERE id = ANY($1)", &ids,)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| (OrgId(uuid_from_db(row.id)), row.name))
            .collect())
    }

    /// Every refund, newest first.
    pub async fn refunds(&self) -> Result<Vec<Refund>, StorageError> {
        let rows = sqlx::query_as!(
            RefundRow,
            "SELECT id, org_id, provider_refund_id, charge_id, amount_cents, currency, status, \
                    reason, note, issued_by, issued_by_label, mail_requested_at, mail_sent_at, \
                    mail_error, created_at, updated_at, policy_basis, quoted_cents, override_reason \
             FROM refund ORDER BY created_at DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Refund::try_from).collect()
    }

    /// One refund by our id.
    pub async fn refund(&self, id: Uuid) -> Result<Option<Refund>, StorageError> {
        let row = sqlx::query_as!(
            RefundRow,
            "SELECT id, org_id, provider_refund_id, charge_id, amount_cents, currency, status, \
                    reason, note, issued_by, issued_by_label, mail_requested_at, mail_sent_at, \
                    mail_error, created_at, updated_at, policy_basis, quoted_cents, override_reason \
             FROM refund WHERE id = $1",
            uuid_to_db(id),
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(Refund::try_from).transpose()
    }

    /// Records a refund issued from the console, and the operators' trail
    /// line naming who issued it, in one transaction. A second call with the
    /// same id — a retried request — writes nothing and answers the first.
    ///
    /// Stripe can deliver `refund.created` before this runs, in which case
    /// the webhook's row is adopted: our id, the operator, the note, and the
    /// operator's own choice about the mail replace what the webhook guessed,
    /// unless that mail already went.
    pub async fn issue(&self, refund: &Refund) -> Result<Refund, StorageError> {
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query!(
            "INSERT INTO refund \
                 (id, org_id, provider_refund_id, charge_id, amount_cents, currency, status, \
                  reason, note, issued_by, issued_by_label, mail_requested_at, mail_due_at, \
                  created_at, updated_at, policy_basis, quoted_cents, override_reason) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12, $13, $13, \
                     $14, $15, $16) \
             ON CONFLICT (provider_refund_id) DO UPDATE SET \
                 id = EXCLUDED.id, \
                 org_id = coalesce(refund.org_id, EXCLUDED.org_id), \
                 reason = coalesce(refund.reason, EXCLUDED.reason), \
                 note = EXCLUDED.note, \
                 issued_by = EXCLUDED.issued_by, \
                 issued_by_label = EXCLUDED.issued_by_label, \
                 policy_basis = EXCLUDED.policy_basis, \
                 quoted_cents = EXCLUDED.quoted_cents, \
                 override_reason = EXCLUDED.override_reason, \
                 mail_requested_at = CASE WHEN refund.mail_sent_at IS NULL \
                     THEN EXCLUDED.mail_requested_at ELSE refund.mail_requested_at END, \
                 mail_due_at = CASE WHEN refund.mail_sent_at IS NULL \
                     THEN EXCLUDED.mail_due_at ELSE refund.mail_due_at END \
             WHERE refund.issued_by IS NULL",
            uuid_to_db(refund.id),
            refund.org.map(|org| uuid_to_db(org.0)),
            refund.provider_refund_id,
            refund.charge_id,
            refund.amount_cents,
            refund.currency,
            refund.status.as_str(),
            refund.reason,
            refund.note,
            refund.issued_by.map(|user| uuid_to_db(user.0)),
            refund.issued_by_label,
            refund.mail_requested_at.map(timestamp_to_db).transpose()?,
            timestamp_to_db(refund.created_at)?,
            refund.policy_basis,
            refund.quoted_cents,
            refund.override_reason,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if inserted {
            if let Some(operator) = refund.issued_by {
                sqlx::query!(
                    "INSERT INTO platform_operator_event (user_id, action, actor, at, refund_id) \
                     VALUES ($1, 'refund', $2, $3, $4)",
                    uuid_to_db(operator.0),
                    format!("operator:{}", operator.0.to_hyphenated()),
                    timestamp_to_db(refund.created_at)?,
                    uuid_to_db(refund.id),
                )
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        if let Some(stored) = self.refund(refund.id).await? {
            return Ok(stored);
        }
        // The id was new and Stripe's was not: the webhook recorded this
        // refund first. That row is the one to answer.
        self.refund_by_provider(&refund.provider_refund_id)
            .await?
            .ok_or_else(|| StorageError::Inconsistent {
                reason: "a refund just written cannot be read back".to_owned(),
            })
    }

    async fn refund_by_provider(&self, provider: &str) -> Result<Option<Refund>, StorageError> {
        let row = sqlx::query_as!(
            RefundRow,
            "SELECT id, org_id, provider_refund_id, charge_id, amount_cents, currency, status, \
                    reason, note, issued_by, issued_by_label, mail_requested_at, mail_sent_at, \
                    mail_error, created_at, updated_at, policy_basis, quoted_cents, override_reason \
             FROM refund WHERE provider_refund_id = $1",
            provider,
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(Refund::try_from).transpose()
    }

    /// Records a refund Stripe reports, answering whether it was new to us.
    ///
    /// Known (by Stripe's id) means its status and amount are refreshed and
    /// nothing else: who issued it, the note and the mail are ours. New means
    /// it was made somewhere else — Stripe's dashboard — and is recorded with
    /// no operator, its mail queued when [`ObservedRefund::queue_mail`] says.
    pub async fn observe_refund(&self, seen: &ObservedRefund<'_>) -> Result<bool, StorageError> {
        let at = timestamp_to_db(seen.at)?;
        let requested = (seen.queue_mail && seen.org.is_some()).then_some(at);
        let row = sqlx::query_scalar!(
            "INSERT INTO refund \
                 (id, org_id, provider_refund_id, charge_id, amount_cents, currency, status, \
                  reason, mail_requested_at, mail_due_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $10, $10) \
             ON CONFLICT (provider_refund_id) DO UPDATE SET \
                 status = EXCLUDED.status, \
                 amount_cents = EXCLUDED.amount_cents, \
                 org_id = coalesce(refund.org_id, EXCLUDED.org_id), \
                 reason = coalesce(refund.reason, EXCLUDED.reason), \
                 updated_at = EXCLUDED.updated_at \
             RETURNING (xmax = 0) AS \"inserted!\"",
            fresh_uuid(),
            seen.org.map(|org| uuid_to_db(org.0)),
            seen.provider_refund_id,
            seen.charge_id,
            seen.amount_cents,
            seen.currency,
            seen.status.as_str(),
            seen.reason,
            requested,
            at,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Queues one refund's mail to its customer, or says why not.
    ///
    /// A mail already sent is not queued again. A queued or failed one is
    /// queued afresh, with its attempts reset: the operator pressing the
    /// button is the retry.
    pub async fn request_mail(&self, id: Uuid, at: Timestamp) -> Result<MailRequest, StorageError> {
        let Some(refund) = self.refund(id).await? else {
            return Ok(MailRequest::NoRefund);
        };
        if let Some(sent) = refund.mail_sent_at {
            return Ok(MailRequest::AlreadySent(sent));
        }
        if refund.org.is_none() {
            return Ok(MailRequest::NoOrganisation);
        }
        let at_db = timestamp_to_db(at)?;
        sqlx::query!(
            "UPDATE refund SET mail_requested_at = $2, mail_due_at = $2, mail_attempts = 0, \
                 mail_error = NULL, updated_at = $2 \
             WHERE id = $1 AND mail_sent_at IS NULL",
            uuid_to_db(id),
            at_db,
        )
        .execute(&self.pool)
        .await?;
        let refreshed = self.refund(id).await?.ok_or(StorageError::Inconsistent {
            reason: "a refund vanished while its mail was queued".to_owned(),
        })?;
        Ok(MailRequest::Queued(Box::new(refreshed)))
    }

    /// Claims the next due refund mail, leasing it until `lease_until`.
    pub async fn claim_mail(
        &self,
        now: Timestamp,
        lease_until: Timestamp,
    ) -> Result<Option<ClaimedRefundMail>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let claimed = sqlx::query!(
            "UPDATE refund SET mail_due_at = $2, mail_attempts = mail_attempts + 1 \
             WHERE id = ( \
                 SELECT id FROM refund \
                 WHERE mail_requested_at IS NOT NULL AND mail_sent_at IS NULL \
                   AND mail_due_at <= $1 AND org_id IS NOT NULL AND mail_attempts < $3 \
                 ORDER BY mail_due_at LIMIT 1 FOR UPDATE SKIP LOCKED) \
             RETURNING id, org_id AS \"org_id!\", amount_cents, currency, mail_attempts",
            timestamp_to_db(now)?,
            timestamp_to_db(lease_until)?,
            REFUND_MAIL_ATTEMPTS,
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
        Ok(Some(ClaimedRefundMail {
            refund: uuid_from_db(claimed.id),
            org_name,
            amount_cents: claimed.amount_cents,
            currency: claimed.currency,
            attempts: claimed.mail_attempts,
            subjects: subjects.into_iter().map(uuid_from_db).collect(),
        }))
    }

    /// Records what one claimed mail came to.
    pub async fn settle_mail(
        &self,
        id: Uuid,
        outcome: &RefundMailOutcome,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let now = timestamp_to_db(now)?;
        match outcome {
            RefundMailOutcome::Sent => {
                sqlx::query!(
                    "UPDATE refund SET mail_sent_at = $2, mail_due_at = NULL, mail_error = NULL, \
                         updated_at = $2 WHERE id = $1",
                    uuid_to_db(id),
                    now,
                )
                .execute(&self.pool)
                .await?;
            }
            RefundMailOutcome::Retry { error, at } => {
                sqlx::query!(
                    "UPDATE refund SET mail_due_at = $2, mail_error = $3, updated_at = $4 \
                     WHERE id = $1",
                    uuid_to_db(id),
                    timestamp_to_db(*at)?,
                    error,
                    now,
                )
                .execute(&self.pool)
                .await?;
            }
            RefundMailOutcome::Failed { error } => {
                sqlx::query!(
                    "UPDATE refund SET mail_due_at = NULL, mail_error = $2, updated_at = $3 \
                     WHERE id = $1",
                    uuid_to_db(id),
                    error,
                    now,
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }
}

/// The refund policy's reads on the ledger and the seller's refund requests
/// (migration 0107).
impl PaymentRepo {
    /// One organisation's successful payments since `since`, newest first:
    /// the ledger rows naming it, and the rows of the Stripe customers it
    /// pays through, which a row the webhook could not attribute still names.
    /// `raw` is not read.
    pub async fn org_payments(
        &self,
        org: OrgId,
        customers: &[String],
        since: Timestamp,
    ) -> Result<Vec<PaymentEvent>, StorageError> {
        let rows = sqlx::query_as!(
            EventRow,
            "SELECT id, org_id, provider_event_id, kind, amount_cents, currency, \
                    provider_object_id, charge_id, payment_intent_id, invoice_id, customer_id, \
                    status, reason, occurred_at, created_at \
             FROM payment_event \
             WHERE kind = 'payment_succeeded' AND occurred_at >= $3 \
               AND (org_id = $1 OR customer_id = ANY($2)) \
             ORDER BY occurred_at DESC, id LIMIT 100",
            uuid_to_db(org.0),
            customers,
            timestamp_to_db(since)?,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(PaymentEvent::try_from).collect()
    }

    /// The invoice a charge paid, as Stripe sent it, from the ledger's
    /// `invoice_paid` rows naming the charge or its payment intent.
    pub async fn invoice_paid_by(
        &self,
        ids: &[String],
    ) -> Result<Option<serde_json::Value>, StorageError> {
        if ids.is_empty() {
            return Ok(None);
        }
        Ok(sqlx::query_scalar!(
            "SELECT raw FROM payment_event \
             WHERE kind = 'invoice_paid' \
               AND (charge_id = ANY($1) OR payment_intent_id = ANY($1)) \
             ORDER BY occurred_at DESC LIMIT 1",
            ids,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    /// What the refunds on record that count (pending or succeeded) have
    /// returned on each charge.
    pub async fn refunded_on(
        &self,
        charges: &[String],
    ) -> Result<HashMap<String, i64>, StorageError> {
        if charges.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query!(
            "SELECT charge_id, COALESCE(SUM(amount_cents), 0)::bigint AS \"refunded!\" \
             FROM refund WHERE charge_id = ANY($1) AND status IN ('pending', 'succeeded') \
             GROUP BY charge_id",
            charges,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.charge_id, row.refunded))
            .collect())
    }

    /// Records a seller's refund request and the operators' mail about it,
    /// in one transaction, or answers that the charge already has one open.
    pub async fn create_request(
        &self,
        new: &NewRefundRequest<'_>,
    ) -> Result<RefundRequestWrite, StorageError> {
        let mut tx = self.pool.begin().await?;
        let at = timestamp_to_db(new.at)?;
        let row = sqlx::query_as!(
            RequestRow,
            "INSERT INTO refund_request \
                 (id, org_id, charge_id, quoted_cents, currency, policy_basis, note, \
                  requested_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) \
             ON CONFLICT (charge_id) WHERE status = 'requested' DO NOTHING \
             RETURNING id, org_id, charge_id, quoted_cents, currency, policy_basis, status, \
                       note, requested_by, decided_by_label, decided_at, decline_reason, \
                       refund_id, created_at, updated_at",
            uuid_to_db(new.id),
            uuid_to_db(new.org.0),
            new.charge_id,
            new.quoted_cents,
            new.currency,
            new.policy_basis,
            (!new.note.is_empty()).then_some(new.note),
            uuid_to_db(new.requested_by.0),
            at,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(RefundRequestWrite::AlreadyOpen);
        };
        let requester_subject = sqlx::query_scalar!(
            "SELECT auth_subject FROM app_user WHERE id = $1",
            uuid_to_db(new.requested_by.0),
        )
        .fetch_optional(&mut *tx)
        .await?
        .flatten()
        .map(uuid_from_db);
        let notice = RefundRequestedNotice {
            request: new.id,
            requester_subject,
            org: new.org,
            org_name: org_name(&mut tx, new.org).await?,
            charge_id: new.charge_id.to_owned(),
            quoted_cents: new.quoted_cents,
            currency: new.currency.to_owned(),
            explanation: new.explanation.to_owned(),
            note: new.note.to_owned(),
        };
        enqueue(
            &mut tx,
            Mailing {
                org: new.org,
                topic: REFUND_REQUESTED_TOPIC,
                request: new.id,
                payload: serde_json::to_value(&notice),
                at: new.at,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(RefundRequestWrite::Recorded(Box::new(
            RefundRequest::try_from(row)?,
        )))
    }

    /// One organisation's refund requests, newest first.
    pub async fn org_requests(&self, org: OrgId) -> Result<Vec<RefundRequest>, StorageError> {
        let rows = sqlx::query_as!(
            RequestRow,
            "SELECT id, org_id, charge_id, quoted_cents, currency, policy_basis, status, note, \
                    requested_by, decided_by_label, decided_at, decline_reason, refund_id, \
                    created_at, updated_at \
             FROM refund_request WHERE org_id = $1 ORDER BY created_at DESC, id LIMIT 50",
            uuid_to_db(org.0),
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(RefundRequest::try_from).collect()
    }

    /// Every refund request, the open ones first, then newest first.
    pub async fn requests(&self) -> Result<Vec<RefundRequest>, StorageError> {
        let rows = sqlx::query_as!(
            RequestRow,
            "SELECT id, org_id, charge_id, quoted_cents, currency, policy_basis, status, note, \
                    requested_by, decided_by_label, decided_at, decline_reason, refund_id, \
                    created_at, updated_at \
             FROM refund_request \
             ORDER BY (status = 'requested') DESC, created_at DESC, id LIMIT 500",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(RefundRequest::try_from).collect()
    }

    /// One refund request by id.
    pub async fn refund_request(&self, id: Uuid) -> Result<Option<RefundRequest>, StorageError> {
        let row = sqlx::query_as!(
            RequestRow,
            "SELECT id, org_id, charge_id, quoted_cents, currency, policy_basis, status, note, \
                    requested_by, decided_by_label, decided_at, decline_reason, refund_id, \
                    created_at, updated_at \
             FROM refund_request WHERE id = $1",
            uuid_to_db(id),
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(RefundRequest::try_from).transpose()
    }

    /// Marks an open request approved by the refund `refund`.
    pub async fn approve_request(
        &self,
        id: Uuid,
        refund: Uuid,
        decision: Decision<'_>,
    ) -> Result<RefundDecision, StorageError> {
        let row = sqlx::query_as!(
            RequestRow,
            "UPDATE refund_request SET status = 'approved', refund_id = $2, decided_by = $3, \
                 decided_by_label = $4, decided_at = $5, updated_at = $5 \
             WHERE id = $1 AND status = 'requested' \
             RETURNING id, org_id, charge_id, quoted_cents, currency, policy_basis, status, \
                       note, requested_by, decided_by_label, decided_at, decline_reason, \
                       refund_id, created_at, updated_at",
            uuid_to_db(id),
            uuid_to_db(refund),
            uuid_to_db(decision.by.0),
            decision.label,
            timestamp_to_db(decision.at)?,
        )
        .fetch_optional(&self.pool)
        .await?;
        self.decided(id, row).await
    }

    /// Marks an open request declined for `reason`, and queues the mail
    /// telling the organisation why, in one transaction.
    pub async fn decline_request(
        &self,
        id: Uuid,
        reason: &str,
        decision: Decision<'_>,
    ) -> Result<RefundDecision, StorageError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as!(
            RequestRow,
            "UPDATE refund_request SET status = 'declined', decline_reason = $2, \
                 decided_by = $3, decided_by_label = $4, decided_at = $5, updated_at = $5 \
             WHERE id = $1 AND status = 'requested' \
             RETURNING id, org_id, charge_id, quoted_cents, currency, policy_basis, status, \
                       note, requested_by, decided_by_label, decided_at, decline_reason, \
                       refund_id, created_at, updated_at",
            uuid_to_db(id),
            reason,
            uuid_to_db(decision.by.0),
            decision.label,
            timestamp_to_db(decision.at)?,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            let org = OrgId(uuid_from_db(row.org_id));
            let subjects = sqlx::query_scalar!(
                "SELECT auth_subject AS \"subject!\" FROM app_user \
                 WHERE org_id = $1 AND auth_subject IS NOT NULL ORDER BY id",
                row.org_id,
            )
            .fetch_all(&mut *tx)
            .await?;
            let notice = RefundDeclinedNotice {
                org,
                org_name: org_name(&mut tx, org).await?,
                quoted_cents: row.quoted_cents,
                currency: row.currency.clone(),
                reason: reason.to_owned(),
                subjects: subjects.into_iter().map(uuid_from_db).collect(),
            };
            enqueue(
                &mut tx,
                Mailing {
                    org,
                    topic: REFUND_DECLINED_TOPIC,
                    request: id,
                    payload: serde_json::to_value(&notice),
                    at: decision.at,
                },
            )
            .await?;
        }
        tx.commit().await?;
        self.decided(id, row).await
    }

    /// What a decision's UPDATE came to: the row it changed, or why none.
    async fn decided(
        &self,
        id: Uuid,
        changed: Option<RequestRow>,
    ) -> Result<RefundDecision, StorageError> {
        if let Some(row) = changed {
            return Ok(RefundDecision::Decided(Box::new(RefundRequest::try_from(
                row,
            )?)));
        }
        Ok(match self.refund_request(id).await? {
            Some(standing) => RefundDecision::AlreadyDecided(Box::new(standing)),
            None => RefundDecision::NoRequest,
        })
    }
}

async fn org_name(tx: &mut Transaction<'_, Postgres>, org: OrgId) -> Result<String, StorageError> {
    Ok(sqlx::query_scalar!(
        "SELECT name FROM organisation WHERE id = $1",
        uuid_to_db(org.0),
    )
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or_default())
}

/// One refund-request mail, as the outbox will carry it.
struct Mailing {
    org: OrgId,
    topic: &'static str,
    request: Uuid,
    payload: Result<serde_json::Value, serde_json::Error>,
    at: Timestamp,
}

/// One refund-request mail as an outbox message, deduplicated on the
/// request: one request, one mail of each kind, whatever retries the write.
async fn enqueue(tx: &mut Transaction<'_, Postgres>, mailing: Mailing) -> Result<(), StorageError> {
    let payload = mailing
        .payload
        .map_err(|error| StorageError::Inconsistent {
            reason: format!("a refund request notice must serialise: {error}"),
        })?;
    pin_org(tx, mailing.org).await?;
    OutboxRepo::append(
        tx,
        &NewOutboxMessage {
            org: mailing.org,
            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
            topic: mailing.topic.to_owned(),
            dedupe_key: uuid_to_db(mailing.request).to_string(),
            payload,
            at: mailing.at,
        },
    )
    .await
}

async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &PaymentEvent,
) -> Result<bool, StorageError> {
    let done = sqlx::query!(
        "INSERT INTO payment_event \
             (id, org_id, provider_event_id, kind, amount_cents, currency, provider_object_id, \
              charge_id, payment_intent_id, invoice_id, customer_id, status, reason, \
              occurred_at, raw, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
         ON CONFLICT (provider_event_id) DO NOTHING",
        uuid_to_db(event.id),
        event.org.map(|org| uuid_to_db(org.0)),
        event.provider_event_id,
        event.kind.as_str(),
        event.amount_cents,
        event.currency,
        event.provider_object_id,
        event.charge_id,
        event.payment_intent_id,
        event.invoice_id,
        event.customer_id,
        event.status,
        event.reason,
        timestamp_to_db(event.occurred_at)?,
        event.raw,
        timestamp_to_db(event.created_at)?,
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// The one payments read that crosses the tenant fence: which organisation
/// each Stripe customer subscribes for. On the backoffice pool, whose read
/// policy on `billing_subscription` is migration 0039's.
pub struct PaymentBackofficeRepo {
    pool: PgPool,
}

impl PaymentBackofficeRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Stripe customer id → organisation, for every subscription on record.
    pub async fn customer_orgs(&self) -> Result<BTreeMap<String, OrgId>, StorageError> {
        let rows = sqlx::query!(
            "SELECT provider_customer_id, org_id FROM billing_subscription \
             WHERE provider_customer_id <> ''",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.provider_customer_id, OrgId(uuid_from_db(row.org_id))))
            .collect())
    }
}
