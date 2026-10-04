//! The operators' Payments page: every money event Stripe reports,
//! cross-referenced, and the refunds issued from it (migration 0102).
//!
//! Three writers fill the ledger. The billing webhook hands every event to
//! [`record_webhook`] before its own subscription logic runs, keyed on
//! Stripe's event id so a redelivery is a no-op. `POST …/payments/sync` reads
//! the last ninety days of charges, refunds, disputes and invoices from
//! Stripe's API, so the page is complete for money that moved before the
//! webhook was subscribed to these events. And a refund issued here records
//! itself, with who issued it, why, and whether the customer was told.
//!
//! Stripe stays the ledger of record. A refund's ceiling is read from the
//! charge in Stripe at the moment it is issued, never from these rows, so a
//! refund made in Stripe's dashboard and not yet delivered here still counts.
//!
//! Every route is an operator's, on the application pool, because both tables
//! are global (migration 0102). The one cross-tenant read — which
//! organisation a Stripe customer subscribes for — uses the backoffice pool
//! where the deployment has one, and does without it where it does not.

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    EntitlementRepo, MailRequest, ObservedRefund, PaymentBackofficeRepo, PaymentEvent, PaymentKind,
    PaymentRepo, Refund, RefundStatus, SiteSettingRepo, StorageError, AUTO_REFUND_MAIL_SETTING,
    SYNC_EVENT_PREFIX,
};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::billing::ORG_METADATA_KEY;
use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::mail_campaigns::long_date;
use crate::refund_policy::RefundBasis;
use crate::refund_quote::{self, QuoteView, Resolved};
use crate::refund_requests::{request_view, RefundRequestView};
use crate::session::OperatorContext;
use crate::stripe::{self, Charge, Listed, RefundReason, RefundRequest, StripeError};
use crate::AppState;

const MILLIS_PER_SEC: i64 = 1_000;
const MILLIS_PER_DAY: i64 = 86_400_000;

/// How far back the sync reads from Stripe.
pub const SYNC_DAYS: i64 = 90;

/// How far back the page lists, and how many rows at most.
const LISTED_DAYS: i64 = 400;
const LISTED_MAX: i64 = 5_000;

/// The longest note an operator may keep with a refund (migration 0102).
const NOTE_MAX: usize = 1_000;
const LABEL_MAX: usize = 120;

// -------------------------------------------------------------------- views

/// One ledger row as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentEventView {
    pub id: String,
    pub org_id: Option<String>,
    pub org_name: Option<String>,
    pub kind: String,
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
}

/// One refund as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundView {
    pub id: String,
    pub org_id: Option<String>,
    pub org_name: Option<String>,
    pub provider_refund_id: String,
    pub charge_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub status: String,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub issued_by: Option<String>,
    pub mail_requested_at: Option<Timestamp>,
    pub mail_sent_at: Option<Timestamp>,
    pub mail_error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The policy rule and amount the refund was issued against, and why the
    /// amount differs where it does (migration 0107). Absent on a refund
    /// issued before the policy was automated or made in Stripe's dashboard.
    pub policy_basis: Option<String>,
    pub quoted_cents: Option<i64>,
    pub override_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentsAdminView {
    pub events: Vec<PaymentEventView>,
    pub refunds: Vec<RefundView>,
    /// Sellers' "Ask for a refund" requests, the open ones first.
    pub requests: Vec<RefundRequestView>,
    pub auto_refund_mail: bool,
    pub stripe_configured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SyncView {
    pub charges: usize,
    pub refunds: usize,
    pub disputes: usize,
    pub invoices: usize,
    pub recorded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundBody {
    /// Minted once per open of the refund panel: our refund's id, and the
    /// key that makes a retried request answer the first refund.
    pub request_id: String,
    pub amount_cents: i64,
    pub reason: RefundReason,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub send_email: bool,
    /// The name the console showed for the operator, as the Mail page sends.
    #[serde(default)]
    pub issued_by_label: String,
    /// The policy quote the panel showed, which must still be the quote when
    /// the refund is made: a quote that changed underneath the operator (a
    /// month turned over, a refund landed elsewhere) is refused so they see
    /// the new one. Absent, the quote is recorded without that check.
    #[serde(default)]
    pub policy_basis: Option<RefundBasis>,
    #[serde(default)]
    pub quoted_cents: Option<i64>,
    /// Why the amount is not the quote. Required when they differ.
    #[serde(default)]
    pub override_reason: String,
    /// End the yearly plan this charge paid for today, as the Terms say a
    /// yearly refund does. Ignored for any other charge.
    #[serde(default)]
    pub end_plan: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentSettings {
    pub auto_refund_mail: bool,
}

// ------------------------------------------------------------------ money

/// An amount as a sentence says it: `$12.00`, `NZ$12.00`, or `12.00 EUR`.
#[must_use]
pub fn money(cents: i64, currency: &str) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let magnitude = cents.unsigned_abs();
    let whole = magnitude.div_euclid(100);
    let part = magnitude.rem_euclid(100);
    match currency.to_ascii_lowercase().as_str() {
        "usd" | "" => format!("{sign}${whole}.{part:02}"),
        "nzd" => format!("{sign}NZ${whole}.{part:02}"),
        "aud" => format!("{sign}A${whole}.{part:02}"),
        other => format!("{sign}{whole}.{part:02} {}", other.to_ascii_uppercase()),
    }
}

// ----------------------------------------------------------------- errors

pub(crate) fn invalid(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

pub(crate) fn conflict(message: &str) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

pub(crate) fn not_found(message: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(message).kind(APIErrorKind::NotFound),
    )
}

pub(crate) fn unconfigured() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new(
            "This deployment has no Stripe key, so it cannot read or refund payments.",
        )
        .kind(APIErrorKind::Internal),
    )
}

pub(crate) fn storage(state: &AppState, error: &StorageError) -> APIError {
    state.internal(&error.to_string())
}

pub(crate) fn stripe_fault(state: &AppState, error: &StripeError) -> APIError {
    state.internal(&error.to_string())
}

// ------------------------------------------------------------ the ledger

/// The ledger kind a Stripe event type records as, or `None` for an event
/// the ledger does not keep.
#[must_use]
pub fn ledger_kind(event_type: &str) -> Option<PaymentKind> {
    Some(match event_type {
        "charge.succeeded" => PaymentKind::PaymentSucceeded,
        "charge.failed" => PaymentKind::PaymentFailed,
        "refund.created" => PaymentKind::RefundCreated,
        "refund.updated" | "refund.failed" | "charge.refund.updated" => PaymentKind::RefundUpdated,
        "charge.dispute.created" => PaymentKind::DisputeOpened,
        "charge.dispute.closed" => PaymentKind::DisputeClosed,
        "invoice.paid" => PaymentKind::InvoicePaid,
        "invoice.payment_failed" => PaymentKind::InvoicePaymentFailed,
        "customer.subscription.created" => PaymentKind::SubscriptionCreated,
        "customer.subscription.deleted" => PaymentKind::SubscriptionCanceled,
        _other => return None,
    })
}

fn text<'a>(object: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    object.get(key)?.as_str().filter(|value| !value.is_empty())
}

/// A Stripe field that is an identifier or, expanded, the object carrying one.
fn id_of<'a>(object: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    let value = object.get(key)?;
    value
        .as_str()
        .or_else(|| value.get("id")?.as_str())
        .filter(|value| !value.is_empty())
}

fn owned(value: Option<&str>) -> Option<String> {
    value.map(str::to_owned)
}

fn org_in(metadata: Option<&serde_json::Value>) -> Option<OrgId> {
    let raw = metadata?.get(ORG_METADATA_KEY)?.as_str()?;
    Uuid::parse_hyphenated(raw).map(OrgId)
}

/// The organisation an object names in metadata of ours, in the first place
/// any object kind carries it.
fn metadata_org(object: &serde_json::Value) -> Option<OrgId> {
    org_in(object.get("metadata"))
        .or_else(|| org_in(object.get("subscription_details")?.get("metadata")))
        .or_else(|| {
            org_in(
                object
                    .get("parent")?
                    .get("subscription_details")?
                    .get("metadata"),
            )
        })
}

/// The first of an invoice's payments, which is where API versions from
/// 2025-03-31 put the payment intent and charge an invoice was paid by.
fn invoice_payment<'a>(object: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    let payment = object
        .get("payments")?
        .get("data")?
        .as_array()?
        .first()?
        .get("payment")?;
    id_of(payment, key)
}

/// One Stripe object as a ledger row of `kind`, or `None` where the object
/// has no id to key it by.
///
/// The organisation is the one the object's own metadata names; the caller
/// resolves the rest through [`resolve_org`].
#[must_use]
pub fn ledger_row(
    kind: PaymentKind,
    object: &serde_json::Value,
    provider_event_id: String,
    occurred_at: Timestamp,
    now: Timestamp,
) -> Option<PaymentEvent> {
    let id = text(object, "id")?;
    let mut row = PaymentEvent {
        id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
        org: metadata_org(object),
        provider_event_id,
        kind,
        amount_cents: None,
        currency: owned(text(object, "currency")),
        provider_object_id: id.to_owned(),
        charge_id: None,
        payment_intent_id: owned(id_of(object, "payment_intent")),
        invoice_id: None,
        customer_id: owned(id_of(object, "customer")),
        status: owned(text(object, "status")),
        reason: None,
        occurred_at,
        raw: object.clone(),
        created_at: now,
    };
    let amount = |key: &str| object.get(key).and_then(serde_json::Value::as_i64);
    match kind {
        PaymentKind::PaymentSucceeded | PaymentKind::PaymentFailed => {
            row.charge_id = Some(id.to_owned());
            row.invoice_id = owned(id_of(object, "invoice"));
            row.amount_cents = amount("amount");
            row.reason = owned(text(object, "failure_message"));
        }
        PaymentKind::RefundCreated | PaymentKind::RefundUpdated => {
            row.charge_id = owned(id_of(object, "charge"));
            row.amount_cents = amount("amount");
            row.reason = owned(text(object, "reason").or_else(|| text(object, "failure_reason")));
        }
        PaymentKind::DisputeOpened | PaymentKind::DisputeClosed => {
            row.charge_id = owned(id_of(object, "charge"));
            row.amount_cents = amount("amount");
            row.reason = owned(text(object, "reason"));
        }
        PaymentKind::InvoicePaid | PaymentKind::InvoicePaymentFailed => {
            row.invoice_id = Some(id.to_owned());
            row.charge_id =
                owned(id_of(object, "charge").or_else(|| invoice_payment(object, "charge")));
            row.payment_intent_id = owned(
                id_of(object, "payment_intent")
                    .or_else(|| invoice_payment(object, "payment_intent")),
            );
            row.amount_cents = if kind == PaymentKind::InvoicePaid {
                amount("amount_paid")
            } else {
                amount("amount_due")
            };
        }
        PaymentKind::SubscriptionCreated | PaymentKind::SubscriptionCanceled => {
            let price = object
                .get("items")
                .and_then(|items| items.get("data"))
                .and_then(serde_json::Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("price"));
            row.amount_cents = price
                .and_then(|price| price.get("unit_amount"))
                .and_then(serde_json::Value::as_i64);
            row.currency = owned(price.and_then(|price| text(price, "currency")));
            row.reason = owned(
                object
                    .get("cancellation_details")
                    .and_then(|details| text(details, "reason")),
            );
        }
    }
    Some(row)
}

/// The ids a row can be cross-referenced by.
fn keys_of(row: &PaymentEvent) -> Vec<String> {
    [
        Some(&row.provider_object_id),
        row.charge_id.as_ref(),
        row.payment_intent_id.as_ref(),
        row.invoice_id.as_ref(),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .collect()
}

/// Fills a row's organisation where its own metadata named none: from a
/// ledger row about the same charge, payment intent or invoice, and then
/// from the Stripe customer it belongs to.
async fn resolve_org(
    repo: &PaymentRepo,
    customers: &BTreeMap<String, OrgId>,
    row: &mut PaymentEvent,
) -> Result<(), StorageError> {
    if row.org.is_some() {
        return Ok(());
    }
    row.org = repo.org_for_objects(&keys_of(row)).await?;
    if row.org.is_none() {
        row.org = row
            .customer_id
            .as_ref()
            .and_then(|customer| customers.get(customer))
            .copied();
    }
    Ok(())
}

/// Stripe customer → organisation, from the ledger and, where this
/// deployment has a backoffice pool, from every subscription on record.
async fn customer_orgs(
    state: &AppState,
    repo: &PaymentRepo,
) -> Result<BTreeMap<String, OrgId>, StorageError> {
    let mut customers = repo.customer_orgs().await?;
    if let Some(backoffice) = &state.backoffice {
        customers.extend(
            PaymentBackofficeRepo::new(backoffice.clone())
                .customer_orgs()
                .await?,
        );
    }
    Ok(customers)
}

async fn auto_mail(state: &AppState) -> Result<bool, StorageError> {
    Ok(SiteSettingRepo::new(state.pool.clone())
        .get(AUTO_REFUND_MAIL_SETTING)
        .await?
        .and_then(|value| value.get("on").and_then(serde_json::Value::as_bool))
        .unwrap_or(false))
}

/// The refund a refund-kind row describes, for the refund table.
fn observed(row: &PaymentEvent, queue_mail: bool) -> Option<ObservedRefund<'_>> {
    Some(ObservedRefund {
        provider_refund_id: &row.provider_object_id,
        charge_id: row.charge_id.as_deref()?,
        org: row.org,
        amount_cents: row.amount_cents.filter(|amount| *amount > 0)?,
        currency: row.currency.as_deref().unwrap_or("usd"),
        status: RefundStatus::from_stripe(row.status.as_deref().unwrap_or("pending")),
        reason: row.reason.as_deref(),
        at: row.occurred_at,
        queue_mail,
    })
}

/// A Stripe event's envelope, as the ledger reads it: the event's own id
/// is the idempotency key here, unlike in the subscription logic.
#[derive(Debug, Deserialize)]
struct Envelope {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    created: i64,
    data: EnvelopeData,
}

#[derive(Debug, Deserialize)]
struct EnvelopeData {
    object: serde_json::Value,
}

/// Records one verified webhook body in the ledger, idempotently by Stripe's
/// event id. Called by the billing webhook before its own handling, and
/// harmless for every event the ledger does not keep.
///
/// A refund Stripe reports that this page did not issue — one made in the
/// dashboard — is recorded too, and its customer's mail queued when the
/// auto setting is on.
pub(crate) async fn record_webhook(state: &AppState, body: &[u8]) -> Result<(), APIError> {
    let Ok(envelope) = serde_json::from_slice::<Envelope>(body) else {
        return Ok(());
    };
    let Some(kind) = ledger_kind(&envelope.kind) else {
        return Ok(());
    };
    let now = (state.wall)();
    let occurred_at = Timestamp(envelope.created.saturating_mul(MILLIS_PER_SEC));
    let Some(mut row) = ledger_row(kind, &envelope.data.object, envelope.id, occurred_at, now)
    else {
        return Ok(());
    };
    let repo = PaymentRepo::new(state.pool.clone());
    let fault = |error: &StorageError| storage(state, error);
    let customers = repo.customer_orgs().await.map_err(|e| fault(&e))?;
    resolve_org(&repo, &customers, &mut row)
        .await
        .map_err(|e| fault(&e))?;
    repo.record_event(&row).await.map_err(|e| fault(&e))?;
    if matches!(
        kind,
        PaymentKind::RefundCreated | PaymentKind::RefundUpdated
    ) {
        let queue = auto_mail(state).await.map_err(|e| fault(&e))?;
        if let Some(refund) = observed(&row, queue) {
            repo.observe_refund(&refund).await.map_err(|e| fault(&e))?;
        }
    }
    Ok(())
}

// --------------------------------------------------------------- the read

pub(crate) fn refund_view(refund: Refund, names: &HashMap<OrgId, String>) -> RefundView {
    RefundView {
        id: refund.id.to_hyphenated(),
        org_id: refund.org.map(|org| org.0.to_hyphenated()),
        org_name: refund.org.and_then(|org| names.get(&org).cloned()),
        provider_refund_id: refund.provider_refund_id,
        charge_id: refund.charge_id,
        amount_cents: refund.amount_cents,
        currency: refund.currency,
        status: refund.status.as_str().to_owned(),
        reason: refund.reason,
        note: refund.note,
        issued_by: refund
            .issued_by_label
            .or_else(|| refund.issued_by.map(|_| "an admin".to_owned())),
        mail_requested_at: refund.mail_requested_at,
        mail_sent_at: refund.mail_sent_at,
        mail_error: refund.mail_error,
        created_at: refund.created_at,
        updated_at: refund.updated_at,
        policy_basis: refund.policy_basis,
        quoted_cents: refund.quoted_cents,
        override_reason: refund.override_reason,
    }
}

/// Fills every org-less row and refund from a row about the same object or
/// the same customer, so the page's Org column and search reach money the
/// webhook could not attribute when it arrived.
fn fill_orgs(
    events: &mut [PaymentEvent],
    refunds: &mut [Refund],
    customers: &BTreeMap<String, OrgId>,
) {
    let mut by_key: BTreeMap<String, OrgId> = BTreeMap::new();
    for row in events.iter() {
        if let Some(org) = row.org {
            for key in keys_of(row) {
                by_key.entry(key).or_insert(org);
            }
        }
    }
    for row in events.iter_mut().filter(|row| row.org.is_none()) {
        row.org = keys_of(row)
            .iter()
            .find_map(|key| by_key.get(key))
            .or_else(|| row.customer_id.as_ref().and_then(|c| customers.get(c)))
            .copied();
    }
    for refund in refunds.iter_mut().filter(|refund| refund.org.is_none()) {
        refund.org = by_key.get(&refund.charge_id).copied();
    }
}

/// `GET /v1/admin/payments`.
pub(crate) async fn admin_view(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<PaymentsAdminView>, APIError> {
    let now = (state.wall)();
    let repo = PaymentRepo::new(state.pool.clone());
    let fault = |error: &StorageError| storage(&state, error);
    let mut events = repo
        .events_since(
            Timestamp(now.0.saturating_sub(LISTED_DAYS * MILLIS_PER_DAY)),
            LISTED_MAX,
        )
        .await
        .map_err(|e| fault(&e))?;
    let mut refunds = repo.refunds().await.map_err(|e| fault(&e))?;
    let requests = repo.requests().await.map_err(|e| fault(&e))?;
    let customers = customer_orgs(&state, &repo).await.map_err(|e| fault(&e))?;
    fill_orgs(&mut events, &mut refunds, &customers);
    let orgs: Vec<OrgId> = events
        .iter()
        .filter_map(|row| row.org)
        .chain(refunds.iter().filter_map(|refund| refund.org))
        .chain(requests.iter().map(|request| request.org))
        .collect::<HashSet<OrgId>>()
        .into_iter()
        .collect();
    let names = repo.org_names(&orgs).await.map_err(|e| fault(&e))?;
    Ok(Json(PaymentsAdminView {
        events: events
            .into_iter()
            .map(|row| PaymentEventView {
                id: row.id.to_hyphenated(),
                org_id: row.org.map(|org| org.0.to_hyphenated()),
                org_name: row.org.and_then(|org| names.get(&org).cloned()),
                kind: row.kind.as_str().to_owned(),
                amount_cents: row.amount_cents,
                currency: row.currency,
                provider_object_id: row.provider_object_id,
                charge_id: row.charge_id,
                payment_intent_id: row.payment_intent_id,
                invoice_id: row.invoice_id,
                customer_id: row.customer_id,
                status: row.status,
                reason: row.reason,
                occurred_at: row.occurred_at,
            })
            .collect(),
        refunds: refunds
            .into_iter()
            .map(|refund| refund_view(refund, &names))
            .collect(),
        requests: requests
            .into_iter()
            .map(|request| request_view(request, &names))
            .collect(),
        auto_refund_mail: auto_mail(&state).await.map_err(|e| fault(&e))?,
        stripe_configured: state.config.stripe.is_some(),
    }))
}

// --------------------------------------------------------------- the sync

/// The ledger rows one listed object becomes. A pending charge, an unpaid
/// invoice and anything without an id become none.
fn synced_rows(listed: Listed, object: &serde_json::Value, now: Timestamp) -> Vec<PaymentEvent> {
    let created = object
        .get("created")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    let at = |secs: i64| Timestamp(secs.saturating_mul(MILLIS_PER_SEC));
    let kind = match listed {
        Listed::Charges => match text(object, "status") {
            Some("succeeded") => PaymentKind::PaymentSucceeded,
            Some("failed") => PaymentKind::PaymentFailed,
            _pending => return Vec::new(),
        },
        Listed::Refunds => PaymentKind::RefundCreated,
        Listed::Disputes => PaymentKind::DisputeOpened,
        Listed::Invoices => match text(object, "status") {
            Some("paid") => PaymentKind::InvoicePaid,
            _unpaid => return Vec::new(),
        },
    };
    let occurred = if kind == PaymentKind::InvoicePaid {
        object
            .get("status_transitions")
            .and_then(|transitions| transitions.get("paid_at"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(created)
    } else {
        created
    };
    let Some(id) = text(object, "id") else {
        return Vec::new();
    };
    let event_id = format!("{SYNC_EVENT_PREFIX}{}:{id}", kind.as_str());
    ledger_row(kind, object, event_id, at(occurred), now)
        .into_iter()
        .collect()
}

/// `POST /v1/admin/payments/sync`: the last ninety days from Stripe.
///
/// Charges first, so the refunds and disputes read after them can inherit
/// the organisation of the charge they are about. A refund found here never
/// queues a mail, whatever the auto setting says: it is history, and the
/// customer heard about it when it happened.
pub(crate) async fn sync(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<SyncView>, APIError> {
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let now = (state.wall)();
    let since = now
        .0
        .saturating_sub(SYNC_DAYS * MILLIS_PER_DAY)
        .div_euclid(MILLIS_PER_SEC);
    let repo = PaymentRepo::new(state.pool.clone());
    let fault = |error: &StorageError| storage(&state, error);
    let customers = customer_orgs(&state, &repo).await.map_err(|e| fault(&e))?;
    let mut view = SyncView::default();
    for listed in [
        Listed::Charges,
        Listed::Invoices,
        Listed::Refunds,
        Listed::Disputes,
    ] {
        let objects = client
            .list_created_since(listed, since)
            .await
            .map_err(|error| stripe_fault(&state, &error))?;
        let count = objects.len();
        match listed {
            Listed::Charges => view.charges = count,
            Listed::Refunds => view.refunds = count,
            Listed::Disputes => view.disputes = count,
            Listed::Invoices => view.invoices = count,
        }
        for object in &objects {
            for mut row in synced_rows(listed, object, now) {
                resolve_org(&repo, &customers, &mut row)
                    .await
                    .map_err(|e| fault(&e))?;
                if repo.record_synced(&row).await.map_err(|e| fault(&e))? {
                    view.recorded += 1;
                }
                if listed == Listed::Refunds {
                    if let Some(refund) = observed(&row, false) {
                        repo.observe_refund(&refund).await.map_err(|e| fault(&e))?;
                    }
                }
            }
        }
    }
    Ok(Json(view))
}

// ------------------------------------------------------------ the refund

/// A Stripe object id as a path segment may carry one: letters, digits and
/// underscores, which is every id Stripe mints and nothing that could turn
/// the outbound URL into a different request.
pub(crate) fn stripe_id(raw: &str) -> bool {
    (3..=255).contains(&raw.len())
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(crate) fn clean(raw: &str, max: usize) -> String {
    raw.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect()
}

/// The organisation a charge belongs to: the one its own metadata names,
/// else the one a ledger row about the same charge or payment intent names,
/// else the one its Stripe customer subscribes for.
pub(crate) async fn charge_org(
    state: &AppState,
    repo: &PaymentRepo,
    held: &Charge,
) -> Result<Option<OrgId>, StorageError> {
    if let Some(org) = org_in(serde_json::to_value(&held.metadata).ok().as_ref()) {
        return Ok(Some(org));
    }
    let mut keys = vec![held.id.clone()];
    keys.extend(held.payment_intent.clone());
    if let Some(org) = repo.org_for_objects(&keys).await? {
        return Ok(Some(org));
    }
    let Some(customer) = &held.customer else {
        return Ok(None);
    };
    Ok(customer_orgs(state, repo).await?.get(customer).copied())
}

/// `GET /v1/admin/payments/charges/{charge}/quote`: what the refund policy
/// gives on one charge today, which the refund panel opens with.
pub(crate) async fn quote(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, charge)): Path<(String, String)>,
) -> Result<Json<QuoteView>, APIError> {
    if !stripe_id(&charge) {
        return Err(not_found("There is no payment with that id."));
    }
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let resolved = refund_quote::resolve(&state, client, &charge, (state.wall)()).await?;
    Ok(Json(resolved.view))
}

/// Who a refund is issued by: the operator, and the name the console showed
/// for them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Issuer<'a> {
    pub(crate) user: UserId,
    pub(crate) label: &'a str,
}

/// `POST /v1/admin/payments/charges/{charge}/refunds`.
pub(crate) async fn create_refund(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, charge)): Path<(String, String)>,
    Json(body): Json<RefundBody>,
) -> Result<(StatusCode, Json<RefundView>), APIError> {
    let label = clean(&body.issued_by_label, LABEL_MAX);
    let issuer = Issuer {
        user: operator.user,
        label: &label,
    };
    let (status, view) = issue_refund(&state, issuer, &charge, &body).await?;
    Ok((status, Json(view)))
}

/// The quote the body names against the one the policy gives now, and the
/// reason an amount other than the quote needs.
fn check_quote(
    quote: &QuoteView,
    body: &RefundBody,
    override_reason: &str,
) -> Result<(), APIError> {
    match (body.policy_basis, body.quoted_cents) {
        (None, None) => {}
        (Some(basis), Some(quoted)) => {
            if basis != quote.basis || quoted != quote.amount_cents {
                return Err(conflict(&format!(
                    "The policy amount for this payment changed since the panel opened: it is \
                     now {}. Check it and try again.",
                    money(quote.amount_cents, &quote.currency)
                )));
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(invalid(
                "The refund names half of a policy quote. Reload the page and try again.",
            ))
        }
    }
    if body.amount_cents != quote.amount_cents && override_reason.is_empty() {
        return Err(invalid(&format!(
            "Say why you're refunding {} rather than the policy's {}.",
            money(body.amount_cents, &quote.currency),
            money(quote.amount_cents, &quote.currency)
        )));
    }
    Ok(())
}

/// Makes the refund in Stripe, under our row's id as its idempotency key.
async fn make_refund(
    state: &AppState,
    client: &stripe::Client,
    request: &RefundRequest<'_>,
) -> Result<stripe::Refund, APIError> {
    match client.create_refund(request).await {
        Ok(made) => Ok(made),
        Err(StripeError::Api { status, message }) if (400..500).contains(&status) => {
            eprintln!(
                "tam-api: Stripe refused a refund on {}: {message}",
                request.charge
            );
            Err(invalid(
                "Stripe refused this refund. Open the payment in Stripe's dashboard to see why.",
            ))
        }
        Err(error) => Err(stripe_fault(state, &error)),
    }
}

/// The ledger row the webhook's `refund.created` will replace, so the page
/// shows the refund the moment it is made rather than when Stripe calls.
async fn record_made(
    repo: &PaymentRepo,
    made: &stripe::Refund,
    refund: &Refund,
    held: &Charge,
    now: Timestamp,
) -> Result<(), StorageError> {
    let object = serde_json::json!({
        "id": made.id,
        "object": "refund",
        "amount": made.amount,
        "currency": refund.currency,
        "charge": refund.charge_id,
        "payment_intent": held.payment_intent,
        "status": made.status,
        "reason": made.reason,
        "created": made.created,
    });
    if let Some(mut row) = ledger_row(
        PaymentKind::RefundCreated,
        &object,
        format!(
            "{SYNC_EVENT_PREFIX}{}:{}",
            PaymentKind::RefundCreated.as_str(),
            made.id
        ),
        if made.created > 0 {
            Timestamp(made.created.saturating_mul(MILLIS_PER_SEC))
        } else {
            now
        },
        now,
    ) {
        row.org = refund.org;
        repo.record_synced(&row).await?;
    }
    Ok(())
}

/// What a refund does beyond the money: a Move Pack refunded in full takes
/// its moves back out of the balance, and a yearly plan whose refund asks
/// for it ends today, as the Terms say.
async fn settle_purchase(
    state: &AppState,
    client: &stripe::Client,
    resolved: &Resolved,
    refunded: i64,
    end_plan: bool,
) -> Result<(), APIError> {
    let now = (state.wall)();
    if let (Some(session), Some(org)) = (resolved.pack_session(), resolved.org) {
        if resolved.view.remaining_cents.saturating_sub(refunded) <= 0 {
            let _taken: bool = EntitlementRepo::new(state.pool.clone())
                .refund_pack(org, session, now)
                .await
                .map_err(|error| storage(state, &error))?;
        }
    }
    if end_plan {
        if let Some(subscription) = resolved.yearly_subscription() {
            crate::billing::end_plan_now(state, client, resolved.org, subscription, now).await?;
        }
    }
    Ok(())
}

/// One refund, issued from the refund panel or by approving a seller's
/// request.
///
/// The ceiling is Stripe's: the charge is re-read and its own
/// `amount_refunded` decides what is left, so a refund made elsewhere counts
/// even before its webhook lands. The policy's quote is taken on the same
/// read and recorded beside the amount, with the operator's reason where the
/// two differ. The refund is made with our row's id as its idempotency key,
/// then recorded, with a line in the operators' trail naming who issued it.
/// A request id seen before answers the refund it made.
pub(crate) async fn issue_refund(
    state: &AppState,
    issuer: Issuer<'_>,
    charge: &str,
    body: &RefundBody,
) -> Result<(StatusCode, RefundView), APIError> {
    if !stripe_id(charge) {
        return Err(not_found("There is no payment with that id."));
    }
    let Some(id) = Uuid::parse_hyphenated(&body.request_id) else {
        return Err(invalid(
            "The refund request is missing its id. Reload the page and try again.",
        ));
    };
    if body.amount_cents <= 0 {
        return Err(invalid("Enter an amount greater than zero."));
    }
    let note = clean(&body.note, NOTE_MAX);
    let override_reason = clean(&body.override_reason, NOTE_MAX);
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let repo = PaymentRepo::new(state.pool.clone());
    let fault = |error: &StorageError| storage(state, error);

    if let Some(existing) = repo.refund(id).await.map_err(|e| fault(&e))? {
        if existing.charge_id != charge {
            return Err(conflict("That refund request was for a different payment."));
        }
        let names = match existing.org {
            Some(org) => repo.org_names(&[org]).await.map_err(|e| fault(&e))?,
            None => HashMap::new(),
        };
        return Ok((StatusCode::OK, refund_view(existing, &names)));
    }

    let resolved = refund_quote::resolve(state, client, charge, (state.wall)()).await?;
    let quote = &resolved.view;
    let held = &resolved.charge;
    if held.refunded || quote.remaining_cents <= 0 {
        return Err(invalid("This payment has already been refunded in full."));
    }
    if body.amount_cents > quote.remaining_cents {
        return Err(invalid(&format!(
            "You can refund at most {} on this payment.",
            money(quote.remaining_cents, &held.currency)
        )));
    }
    check_quote(quote, body, &override_reason)?;

    let org = resolved.org;
    let org_text = org.map(|org| org.0.to_hyphenated());
    let id_text = id.to_hyphenated();
    let made = make_refund(
        state,
        client,
        &RefundRequest {
            id: &id_text,
            charge,
            amount_cents: body.amount_cents,
            reason: body.reason,
            org: org_text.as_deref(),
        },
    )
    .await?;

    let now = (state.wall)();
    let refund = Refund {
        id,
        org,
        provider_refund_id: made.id.clone(),
        charge_id: charge.to_owned(),
        amount_cents: made.amount,
        currency: if made.currency.is_empty() {
            held.currency.clone()
        } else {
            made.currency.clone()
        },
        status: RefundStatus::from_stripe(made.status.as_deref().unwrap_or("pending")),
        reason: Some(body.reason.as_str().to_owned()),
        note: (!note.is_empty()).then_some(note),
        issued_by: Some(issuer.user),
        issued_by_label: Some(if issuer.label.is_empty() {
            "an admin".to_owned()
        } else {
            issuer.label.to_owned()
        }),
        mail_requested_at: (body.send_email && org.is_some()).then_some(now),
        mail_sent_at: None,
        mail_error: None,
        created_at: now,
        updated_at: now,
        policy_basis: Some(quote.basis.as_str().to_owned()),
        quoted_cents: Some(quote.amount_cents),
        override_reason: (made.amount != quote.amount_cents).then(|| {
            if override_reason.is_empty() {
                "Stripe refunded a different amount from the one asked for.".to_owned()
            } else {
                override_reason
            }
        }),
    };
    let stored = repo.issue(&refund).await.map_err(|e| fault(&e))?;
    record_made(&repo, &made, &refund, held, now)
        .await
        .map_err(|e| fault(&e))?;
    settle_purchase(state, client, &resolved, made.amount, body.end_plan).await?;

    let names = match stored.org {
        Some(org) => repo.org_names(&[org]).await.map_err(|e| fault(&e))?,
        None => HashMap::new(),
    };
    Ok((StatusCode::CREATED, refund_view(stored, &names)))
}

/// `POST /v1/admin/payments/refunds/{id}/mail`: queues the refund mail to
/// the customer, for a refund issued without it or whose send failed.
pub(crate) async fn mail_refund(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<Json<RefundView>, APIError> {
    let Some(id) = Uuid::parse_hyphenated(&id) else {
        return Err(not_found("There is no refund with that id."));
    };
    let repo = PaymentRepo::new(state.pool.clone());
    let fault = |error: &StorageError| storage(&state, error);
    match repo
        .request_mail(id, (state.wall)())
        .await
        .map_err(|e| fault(&e))?
    {
        MailRequest::Queued(refund) => {
            let names = match refund.org {
                Some(org) => repo.org_names(&[org]).await.map_err(|e| fault(&e))?,
                None => HashMap::new(),
            };
            Ok(Json(refund_view(*refund, &names)))
        }
        MailRequest::AlreadySent(at) => Err(conflict(&format!(
            "This email was already sent on {}.",
            long_date(at)
        ))),
        MailRequest::NoOrganisation => Err(invalid(
            "This payment isn't linked to an organisation, so there is nobody to email.",
        )),
        MailRequest::NoRefund => Err(not_found("There is no refund with that id.")),
    }
}

/// `PUT /v1/admin/payments/settings`: whether new refunds email the customer
/// without being asked.
pub(crate) async fn update_settings(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<PaymentSettings>,
) -> Result<Json<PaymentSettings>, APIError> {
    SiteSettingRepo::new(state.pool.clone())
        .set(
            AUTO_REFUND_MAIL_SETTING,
            &serde_json::json!({ "on": body.auto_refund_mail }),
            operator.user,
            (state.wall)(),
        )
        .await
        .map_err(|error| storage(&state, &error))?;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::{ledger_kind, ledger_row, money, stripe_id, synced_rows};
    use crate::stripe::Listed;
    use tam_storage::PaymentKind;
    use tam_types::Timestamp;

    #[test]
    fn amounts_read_as_a_sentence_says_them() {
        assert_eq!(money(1_200, "usd"), "$12.00");
        assert_eq!(money(5, "usd"), "$0.05");
        assert_eq!(money(34_783, "nzd"), "NZ$347.83");
        assert_eq!(money(990, "eur"), "9.90 EUR");
    }

    #[test]
    fn only_a_stripe_shaped_id_reaches_the_outbound_path() {
        assert!(stripe_id("ch_3PQx9z2eZvKYlo2C1abcdEFG"));
        assert!(!stripe_id("ch_1/../../v1/customers"));
        assert!(!stripe_id("ch_1?expand[]=x"));
        assert!(!stripe_id(""));
    }

    #[test]
    fn the_ledger_keeps_money_events_and_ignores_the_rest() {
        assert_eq!(
            ledger_kind("charge.succeeded"),
            Some(PaymentKind::PaymentSucceeded)
        );
        assert_eq!(
            ledger_kind("refund.failed"),
            Some(PaymentKind::RefundUpdated)
        );
        assert_eq!(
            ledger_kind("charge.dispute.closed"),
            Some(PaymentKind::DisputeClosed)
        );
        assert_eq!(ledger_kind("checkout.session.completed"), None);
        assert_eq!(ledger_kind("customer.subscription.updated"), None);
    }

    #[test]
    fn a_new_api_versions_invoice_still_names_its_payment_intent() {
        let invoice = serde_json::json!({
            "id": "in_1", "object": "invoice", "status": "paid", "amount_paid": 2900,
            "currency": "usd", "customer": "cus_1", "created": 100,
            "status_transitions": { "paid_at": 160 },
            "payments": { "data": [{ "payment": { "type": "payment_intent", "payment_intent": "pi_1" } }] }
        });
        let rows = synced_rows(Listed::Invoices, &invoice, Timestamp(0));
        assert_eq!(rows.len(), 1, "a paid invoice is one row");
        let row = rows.first().expect("the row is there");
        assert_eq!(row.kind, PaymentKind::InvoicePaid);
        assert_eq!(row.payment_intent_id.as_deref(), Some("pi_1"));
        assert_eq!(row.occurred_at, Timestamp(160_000), "paid, not created");
        assert_eq!(row.provider_event_id, "sync:invoice_paid:in_1");
    }

    #[test]
    fn a_pending_charge_and_a_draft_invoice_are_not_money_yet() {
        let charge = serde_json::json!({ "id": "ch_1", "status": "pending", "amount": 100 });
        assert!(synced_rows(Listed::Charges, &charge, Timestamp(0)).is_empty());
        let invoice = serde_json::json!({ "id": "in_1", "status": "draft" });
        assert!(synced_rows(Listed::Invoices, &invoice, Timestamp(0)).is_empty());
    }

    #[test]
    fn a_dispute_is_about_the_charge_it_disputes() {
        let dispute = serde_json::json!({
            "id": "dp_1", "charge": { "id": "ch_1", "object": "charge" }, "amount": 2900,
            "currency": "usd", "status": "needs_response", "reason": "fraudulent"
        });
        let row = ledger_row(
            PaymentKind::DisputeOpened,
            &dispute,
            "evt_1".to_owned(),
            Timestamp(0),
            Timestamp(0),
        );
        let row = row.as_ref();
        assert_eq!(row.and_then(|row| row.charge_id.as_deref()), Some("ch_1"));
        assert_eq!(
            row.and_then(|row| row.reason.as_deref()),
            Some("fraudulent")
        );
    }
}
