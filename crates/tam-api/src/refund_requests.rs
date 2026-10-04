//! A seller's "Ask for a refund" (migration 0107), and an operator's answer.
//!
//! The billing page lists the seller's recent payments, quotes the one they
//! pick under the refund policy ([`crate::refund_quote`]), and records a
//! request for that quote with whatever they wrote. The operators are mailed
//! (`email.refund_requested`). On the Payments page an operator approves the
//! request, which issues the quoted refund through the same path the refund
//! panel uses (and, for a yearly plan, ends the plan today, as the Terms
//! say), or declines it with a reason the seller is mailed
//! (`email.refund_declined`).
//!
//! The seller's routes are [`OrgContext`] routes on the application pool:
//! the table is global, and every read and write here names the caller's
//! organisation from the session. A charge is the seller's only when it
//! resolves to their organisation; any other charge id answers as missing.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    BillingRepo, Decision, NewRefundRequest, PaymentRepo, RefundDecision, RefundRequest,
    RefundRequestStatus, RefundRequestWrite, StorageError,
};
use tam_types::{OrgId, Timestamp, Uuid};

use crate::billing::InvoiceObject;
use crate::error::APIError;
use crate::payments::{
    clean, conflict, invalid, issue_refund, not_found, storage, stripe_id, unconfigured, Issuer,
    RefundBody,
};
use crate::refund_policy::nz_long_date;
use crate::refund_quote::{self, label_for, QuoteView};
use crate::session::OperatorContext;
use crate::stripe::RefundReason;
use crate::version::APIVersion;
use crate::{AppState, OrgContext};

const MILLIS_PER_DAY: i64 = 86_400_000;

/// How far back the billing page offers payments to ask about: a yearly
/// plan's whole year, and a little over.
const OFFERED_DAYS: i64 = 400;

/// The longest note a seller or a decline reason may carry (migration 0107).
const NOTE_MAX: usize = 1_000;
const LABEL_MAX: usize = 120;

/// One refund request as the billing page and the Payments page list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundRequestView {
    pub id: String,
    pub org_id: String,
    pub org_name: Option<String>,
    pub charge_id: String,
    pub quoted_cents: i64,
    pub currency: String,
    pub policy_basis: String,
    /// `requested`, `approved` or `declined`.
    pub status: String,
    pub note: Option<String>,
    pub decided_by: Option<String>,
    pub decided_at: Option<Timestamp>,
    pub decline_reason: Option<String>,
    pub refund_id: Option<String>,
    pub created_at: Timestamp,
}

pub(crate) fn request_view(
    request: RefundRequest,
    names: &HashMap<OrgId, String>,
) -> RefundRequestView {
    RefundRequestView {
        id: request.id.to_hyphenated(),
        org_id: request.org.0.to_hyphenated(),
        org_name: names.get(&request.org).cloned(),
        charge_id: request.charge_id,
        quoted_cents: request.quoted_cents,
        currency: request.currency,
        policy_basis: request.policy_basis,
        status: request.status.as_str().to_owned(),
        note: request.note,
        decided_by: request.decided_by_label,
        decided_at: request.decided_at,
        decline_reason: request.decline_reason,
        refund_id: request.refund_id.map(|id| id.to_hyphenated()),
        created_at: request.created_at,
    }
}

/// One payment the billing page offers to ask about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundablePayment {
    pub charge_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub paid_at: Timestamp,
    /// What the refunds on record have returned on it already.
    pub refunded_cents: i64,
    /// The plan its invoice names, where the ledger holds that invoice:
    /// "Pro yearly plan". Absent for a one-off payment such as a Move Pack;
    /// the quote names it.
    pub what: Option<String>,
}

/// `GET /v1/billing/refund-requests`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingRefundsView {
    pub payments: Vec<RefundablePayment>,
    pub requests: Vec<RefundRequestView>,
}

/// What "Ask for a refund" sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskBody {
    pub charge_id: String,
    #[serde(default)]
    pub note: String,
}

/// What Approve and Decline send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionBody {
    /// Why it was declined; required to decline, ignored to approve.
    #[serde(default)]
    pub reason: String,
    /// The name the console showed for the operator.
    #[serde(default)]
    pub decided_by_label: String,
}

// ----------------------------------------------------------- the seller's

/// `GET /v1/billing/refund-requests`: the seller's payments of the last 400
/// days with something left to refund, and the requests they have made.
pub(crate) async fn billing_refunds(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<BillingRefundsView>, APIError> {
    let fault = |error: &StorageError| storage(&state, error);
    let customers: Vec<String> = BillingRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|e| fault(&e))?
        .map(|held| held.provider_customer_id)
        .filter(|customer| !customer.is_empty())
        .into_iter()
        .collect();
    let repo = PaymentRepo::new(state.pool.clone());
    let since = Timestamp(
        (state.wall)()
            .0
            .saturating_sub(OFFERED_DAYS * MILLIS_PER_DAY),
    );
    let paid = repo
        .org_payments(context.org, &customers, since)
        .await
        .map_err(|e| fault(&e))?;
    let charges: Vec<String> = paid
        .iter()
        .filter_map(|row| row.charge_id.clone())
        .collect();
    let refunded = repo.refunded_on(&charges).await.map_err(|e| fault(&e))?;
    let mut payments = Vec::with_capacity(paid.len());
    for row in paid {
        let (Some(charge_id), Some(amount_cents)) = (row.charge_id, row.amount_cents) else {
            continue;
        };
        let refunded_cents = refunded.get(&charge_id).copied().unwrap_or(0);
        if amount_cents.saturating_sub(refunded_cents) <= 0 {
            continue;
        }
        let mut keys = vec![charge_id.clone()];
        keys.extend(row.payment_intent_id);
        let what = repo
            .invoice_paid_by(&keys)
            .await
            .map_err(|e| fault(&e))?
            .and_then(|raw| serde_json::from_value::<InvoiceObject>(raw).ok())
            .and_then(|invoice| {
                let price = invoice.price()?;
                state.config.stripe_price_map.key_for(price).map(label_for)
            });
        payments.push(RefundablePayment {
            charge_id,
            amount_cents,
            currency: row.currency.unwrap_or_else(|| "usd".to_owned()),
            paid_at: row.occurred_at,
            refunded_cents,
            what,
        });
    }
    let requests = repo
        .org_requests(context.org)
        .await
        .map_err(|e| fault(&e))?
        .into_iter()
        .map(|request| request_view(request, &HashMap::new()))
        .collect();
    Ok(Json(BillingRefundsView { payments, requests }))
}

/// One of the caller's charges, quoted; any other charge is missing.
async fn own_quote(state: &AppState, org: OrgId, charge: &str) -> Result<QuoteView, APIError> {
    let missing = || not_found("There is no payment with that id.");
    if !stripe_id(charge) {
        return Err(missing());
    }
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let resolved = refund_quote::resolve(state, client, charge, (state.wall)()).await?;
    if resolved.org != Some(org) {
        return Err(missing());
    }
    Ok(resolved.view)
}

/// `GET /v1/billing/payments/{charge}/refund-quote`: what the refund policy
/// gives on one of the caller's payments today.
pub(crate) async fn billing_quote(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, charge)): Path<(String, String)>,
) -> Result<Json<QuoteView>, APIError> {
    own_quote(&state, context.org, &charge).await.map(Json)
}

/// `POST /v1/billing/refund-requests`: asks for the policy's refund on one
/// of the caller's payments, and tells the operators.
///
/// The amount is the server's quote, never the client's. A payment the
/// policy refunds nothing on is refused with the policy's own sentence, and
/// a payment with a request already waiting answers that it is waiting.
pub(crate) async fn ask(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<AskBody>,
) -> Result<(StatusCode, Json<RefundRequestView>), APIError> {
    let quote = own_quote(&state, context.org, &body.charge_id).await?;
    if quote.amount_cents <= 0 {
        return Err(invalid(&format!(
            "{} So there's nothing to ask for on this payment.",
            quote.explanation
        )));
    }
    let note = clean(&body.note, NOTE_MAX);
    let written = PaymentRepo::new(state.pool.clone())
        .create_request(&NewRefundRequest {
            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
            org: context.org,
            charge_id: &quote.charge_id,
            quoted_cents: quote.amount_cents,
            currency: &quote.currency,
            policy_basis: quote.basis.as_str(),
            explanation: &quote.explanation,
            note: &note,
            requested_by: context.user,
            at: (state.wall)(),
        })
        .await
        .map_err(|error| storage(&state, &error))?;
    match written {
        RefundRequestWrite::Recorded(request) => {
            state.telemetry.capture(
                context.org,
                "refund_requested",
                serde_json::json!({ "basis": quote.basis.as_str() }),
            );
            Ok((
                StatusCode::CREATED,
                Json(request_view(*request, &HashMap::new())),
            ))
        }
        RefundRequestWrite::AlreadyOpen => Err(conflict(
            "You've already asked for a refund on this payment. We'll email you when it's \
             decided.",
        )),
    }
}

// ---------------------------------------------------------- the operator's

fn already(request: &RefundRequest) -> APIError {
    let word = match request.status {
        RefundRequestStatus::Approved => "approved",
        RefundRequestStatus::Declined => "declined",
        RefundRequestStatus::Requested => "decided",
    };
    conflict(&format!(
        "This request was already {word}{}.",
        request
            .decided_at
            .map(|at| format!(" on {}", nz_long_date(at)))
            .unwrap_or_default()
    ))
}

async fn named(
    state: &AppState,
    request: RefundRequest,
) -> Result<Json<RefundRequestView>, APIError> {
    let names = PaymentRepo::new(state.pool.clone())
        .org_names(&[request.org])
        .await
        .map_err(|error| storage(state, &error))?;
    Ok(Json(request_view(request, &names)))
}

fn request_id(raw: &str) -> Result<Uuid, APIError> {
    Uuid::parse_hyphenated(raw).ok_or_else(|| not_found("There is no refund request with that id."))
}

/// `POST /v1/admin/payments/refund-requests/{id}/approve`: issues the
/// refund quoted when the seller asked, under the request's own id so a
/// retry answers the same refund, emails the customer, and, for a yearly
/// plan, ends the plan today.
///
/// The amount is the quote on the day they asked, which is what they were
/// shown; where today's quote is lower (a month turned over while the
/// request waited), the refund records that as its reason.
pub(crate) async fn approve(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
    Json(body): Json<DecisionBody>,
) -> Result<Json<RefundRequestView>, APIError> {
    let id = request_id(&id)?;
    let repo = PaymentRepo::new(state.pool.clone());
    let fault = |error: &StorageError| storage(&state, error);
    let Some(request) = repo.refund_request(id).await.map_err(|e| fault(&e))? else {
        return Err(not_found("There is no refund request with that id."));
    };
    if request.status != RefundRequestStatus::Requested {
        return Err(already(&request));
    }
    let label = clean(&body.decided_by_label, LABEL_MAX);
    let refund = RefundBody {
        request_id: id.to_hyphenated(),
        amount_cents: request.quoted_cents,
        reason: RefundReason::RequestedByCustomer,
        note: "Approved the seller's refund request.".to_owned(),
        send_email: true,
        issued_by_label: label.clone(),
        policy_basis: None,
        quoted_cents: None,
        override_reason: format!(
            "The policy's quote on {}, when the seller asked.",
            nz_long_date(request.created_at)
        ),
        end_plan: true,
    };
    let issuer = Issuer {
        user: operator.user,
        label: &label,
    };
    let (_status, made) = issue_refund(&state, issuer, &request.charge_id, &refund).await?;
    let refund_id = Uuid::parse_hyphenated(&made.id).unwrap_or(id);
    let decision = Decision {
        by: operator.user,
        label: &label,
        at: (state.wall)(),
    };
    match repo
        .approve_request(id, refund_id, decision)
        .await
        .map_err(|e| fault(&e))?
    {
        RefundDecision::Decided(request) => named(&state, *request).await,
        RefundDecision::AlreadyDecided(request) => Err(already(&request)),
        RefundDecision::NoRequest => Err(not_found("There is no refund request with that id.")),
    }
}

/// `POST /v1/admin/payments/refund-requests/{id}/decline`: declines the
/// request and emails the seller the reason.
pub(crate) async fn decline(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
    Json(body): Json<DecisionBody>,
) -> Result<Json<RefundRequestView>, APIError> {
    let id = request_id(&id)?;
    let reason = clean(&body.reason, NOTE_MAX);
    if reason.is_empty() {
        return Err(invalid("Say why, in a sentence the seller will read."));
    }
    let label = clean(&body.decided_by_label, LABEL_MAX);
    let decision = Decision {
        by: operator.user,
        label: &label,
        at: (state.wall)(),
    };
    match PaymentRepo::new(state.pool.clone())
        .decline_request(id, &reason, decision)
        .await
        .map_err(|error| storage(&state, &error))?
    {
        RefundDecision::Decided(request) => named(&state, *request).await,
        RefundDecision::AlreadyDecided(request) => Err(already(&request)),
        RefundDecision::NoRequest => Err(not_found("There is no refund request with that id.")),
    }
}
