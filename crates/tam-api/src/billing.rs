//! Billing: the two routes that open Stripe, the webhook that records what
//! Stripe did, and the org-scoped read that answers it.
//!
//! The routes are authenticated in two different ways, deliberately. The
//! read, the checkout and the portal are [`OrgContext`] like every other
//! tenant route, so a caller can only ever act on its own organisation. The
//! webhook has no session and no organisation extractor at all, because
//! Stripe calls it and Stripe holds no session: its signature over the raw
//! bytes *is* the authentication, and the organisation arrives inside the
//! signed payload rather than from the caller. That is why the handler takes
//! the body as [`axum::body::Bytes`] and verifies before it parses — a JSON
//! value re-serialised for hashing is a different byte string, and verifying
//! that one proves nothing about what arrived.
//!
//! Fulfilment is idempotent on the *object* identifier rather than on the
//! event identifier, which is the stronger of the two. Stripe retries a
//! delivery it did not hear 200 for, and it also describes one purchase with
//! more than one event: a completed subscription checkout produces both a
//! `checkout.session.completed` and an `invoice.paid`. Keying on the event
//! would make each delivery unique and let one purchase pay out twice; keying
//! on the session, the subscription and the billing period is what makes the
//! second arrival a no-op. Every write below therefore lands through a
//! `source_ref` the provider owns: the partial unique index migration 0086
//! makes vendor-agnostic is the fence under the grants, and
//! `EntitlementRepo::credit_moves` and `BillingRepo::record_booking` carry
//! their own.

use std::collections::BTreeMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_limits::{Plan, PriceKey, FOUNDING, PACKS};
use tam_storage::{
    Accrual, BillingRepo, EntitlementRepo, GrantedBy, MoveCredit, MoveSource, NewGrant,
    ServiceBooking, SubscriptionState,
};
use tam_types::{OrgId, Timestamp, Uuid};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::stripe::{self, CheckoutMode, CheckoutRequest};
use crate::version::APIVersion;
use crate::{AppState, OrgContext};

/// The key our checkout puts the organisation identifier under, in Stripe's
/// `metadata` and as the session's `client_reference_id`, and therefore the
/// only place a webhook may learn which tenant an event belongs to.
///
/// Named here rather than at the checkout, because this is the side that
/// cannot be changed unilaterally: a checkout that stopped setting it would
/// produce events this route ignores, silently, forever. One constant, two
/// readers.
pub const ORG_METADATA_KEY: &str = "org";

/// The events this route acts on. Every other event type Stripe sends is
/// acknowledged and ignored, so the endpoint can be subscribed to more than
/// it handles without either side changing.
const CHECKOUT_COMPLETED: &str = "checkout.session.completed";
const INVOICE_PAID: &str = "invoice.paid";
const INVOICE_PAYMENT_FAILED: &str = "invoice.payment_failed";
const SUBSCRIPTION_UPDATED: &str = "customer.subscription.updated";
const SUBSCRIPTION_DELETED: &str = "customer.subscription.deleted";

/// The Stripe subscription statuses that carry an entitlement.
///
/// Anything else — `past_due`, `paused`, `unpaid`, `incomplete`, `canceled`,
/// or a status Stripe adds later — grants nothing, which is the fail-closed
/// direction: a lapsed subscription stops entitling rather than entitling
/// indefinitely. This is the one place Stripe's vocabulary is interpreted
/// rather than recorded.
const ENTITLING: [&str; 2] = ["active", "trialing"];

/// How long a subscription keeps entitling past the period Stripe last named.
///
/// A renewal event arriving late must not take a paying seller's plan away
/// between the period ending and the webhook landing. A day is the same grace
/// the device entitlement token carries, and for the same reason: the cost of
/// a day of over-entitlement is one day, and the cost of under-entitlement is
/// a customer locked out of work they paid for. The same day is what a failed
/// payment buys while Stripe's dunning runs.
const SUBSCRIPTION_GRACE_HOURS: i64 = 24;
const MILLIS_PER_HOUR: i64 = 3_600_000;
const MILLIS_PER_SEC: i64 = 1_000;

/// How long a pack's moves stay spendable. Twelve months from purchase, which
/// is the pricing decision of 2026-09-20 and not a property of any plan.
const PACK_VALIDITY_DAYS: i64 = 365;
const MILLIS_PER_DAY: i64 = 86_400_000;

/// Where the browser lands after Stripe, when the request carried no origin
/// of its own. Same-origin by construction otherwise: the console posts from
/// the page the seller is standing on, and that page's origin is the one they
/// must come back to.
const CONSOLE_RETURN_PATH: &str = "/settings/subscription";

// -------------------------------------------------------------------- views

/// How often a subscription renews, which is the price a seller chose rather
/// than a property of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    Monthly,
    Yearly,
}

impl Cadence {
    #[must_use]
    const fn of(key: PriceKey) -> Option<Self> {
        match key {
            PriceKey::SyncMonthly => Some(Self::Monthly),
            PriceKey::SyncYearly | PriceKey::FoundingYearly => Some(Self::Yearly),
            PriceKey::Pack20
            | PriceKey::Pack50
            | PriceKey::Pack100
            | PriceKey::Pack250
            | PriceKey::Pack500
            | PriceKey::MoveWithMe => None,
        }
    }
}

/// The moves a tenant can spend, and when the soonest of them stop being
/// spendable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveBalance {
    pub available: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiring_soonest: Option<Timestamp>,
}

impl MoveBalance {
    fn of(balance: tam_storage::MoveBalance) -> Self {
        Self {
            available: balance.available,
            expiring_soonest: balance.expiring_soonest,
        }
    }
}

/// What a tenant's billing page reads.
///
/// Every field is an answer rather than a provider identifier. Stripe's
/// subscription and customer ids are the provider's bookkeeping and the
/// seller has no use for either; what they need is the plan they hold, when
/// it renews, what they can spend, and whether the portal will open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingView {
    pub plan: Plan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cadence: Option<Cadence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renews_at: Option<Timestamp>,
    pub moves: MoveBalance,
    /// Whether this tenant holds a founding-member price. It changes what
    /// they are charged for the next two years, so it is a fact the page
    /// states rather than infers.
    pub founding: bool,
    /// Whether "Manage billing" will open. False where this deployment holds
    /// no Stripe key, and where the tenant has never reached checkout and so
    /// has no customer to manage.
    pub portal_available: bool,
}

/// Where to send the browser. One field, because a checkout and a portal
/// answer the same thing and a client that handled two shapes would be
/// handling one fact twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedirectView {
    pub url: String,
}

/// What the console asks to buy: a price key from `tam-limits`, never a
/// Stripe price identifier.
///
/// The client naming our own vocabulary rather than Stripe's is what keeps
/// the price map on the server alone: a browser that could name a Stripe
/// price could name any Stripe price.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutBody {
    pub price_key: String,
}

pub(crate) async fn billing_view(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<BillingView>, APIError> {
    let stored = BillingRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let entitlements = EntitlementRepo::new(state.pool.clone());
    let held = entitlements
        .current(context.org, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let moves = entitlements
        .move_balance(context.org, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;

    let sold = stored
        .as_ref()
        .and_then(|state| state.provider_price_id.as_deref())
        .and_then(|price| state.config.stripe_price_map.key_for(price));
    Ok(Json(BillingView {
        plan: held.plan,
        cadence: sold.and_then(Cadence::of),
        renews_at: stored.as_ref().and_then(|state| state.current_period_end),
        moves: MoveBalance::of(moves),
        founding: sold == Some(PriceKey::FoundingYearly),
        portal_available: state.config.stripe.is_some()
            && stored.is_some_and(|state| !state.provider_customer_id.is_empty()),
    }))
}

// ------------------------------------------------------------------ outbound

/// Opens a Checkout Session for one price key and answers where to send the
/// browser.
///
/// The price key is ours and the price identifier is Stripe's, and the
/// translation happens here and only here. A key this deployment's map does
/// not carry is refused rather than guessed at: a checkout opened at the
/// wrong price is a charge nobody can undo without a refund.
pub(crate) async fn checkout(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
    headers: HeaderMap,
    Json(body): Json<CheckoutBody>,
) -> Result<Json<RedirectView>, APIError> {
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let Some(key) = PriceKey::parse(&body.price_key) else {
        return Err(unsellable(&body.price_key));
    };
    let Some(price) = state.config.stripe_price_map.price_for(key) else {
        return Err(unsellable(&body.price_key));
    };

    let held = BillingRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let customer = held
        .as_ref()
        .map(|state| state.provider_customer_id.as_str())
        .filter(|customer| !customer.is_empty());

    let origin = origin_of(&headers);
    let org = context.org.0.to_hyphenated();
    let session = client
        .create_checkout_session(&CheckoutRequest {
            price,
            mode: mode_for(key),
            org: &org,
            customer,
            success_url: &format!("{origin}{CONSOLE_RETURN_PATH}?checkout=success"),
            cancel_url: &format!("{origin}{CONSOLE_RETURN_PATH}?checkout=cancel"),
        })
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let Some(url) = session.url else {
        return Err(state.internal("the Stripe checkout session carried no url"));
    };
    Ok(Json(RedirectView { url }))
}

/// Opens the Stripe billing portal for this tenant's customer.
///
/// Minted per click rather than stored: an unused portal URL expires in five
/// minutes, so a cached one is a link that has already stopped working.
pub(crate) async fn portal(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
    headers: HeaderMap,
) -> Result<Json<RedirectView>, APIError> {
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let held = BillingRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let Some(customer) = held
        .as_ref()
        .map(|state| state.provider_customer_id.as_str())
        .filter(|customer| !customer.is_empty())
    else {
        return Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new("You have not bought a plan yet, so there is nothing to manage.")
                .kind(APIErrorKind::Validation),
        ));
    };
    let origin = origin_of(&headers);
    let url = client
        .create_portal_session(customer, &format!("{origin}{CONSOLE_RETURN_PATH}"))
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(RedirectView { url }))
}

/// Which Checkout mode a price key needs. A recurring price refuses
/// `payment` and a one-off price refuses `subscription`, so this is a fact
/// about the price list rather than a preference.
const fn mode_for(key: PriceKey) -> CheckoutMode {
    match key {
        PriceKey::SyncMonthly | PriceKey::SyncYearly | PriceKey::FoundingYearly => {
            CheckoutMode::Subscription
        }
        PriceKey::Pack20
        | PriceKey::Pack50
        | PriceKey::Pack100
        | PriceKey::Pack250
        | PriceKey::Pack500
        | PriceKey::MoveWithMe => CheckoutMode::Payment,
    }
}

/// The absolute origin to send the seller back to.
///
/// Read from the request rather than configured, because the console posts
/// from the page the seller is standing on and that is by construction the
/// origin they must return to. A request carrying neither header is not a
/// browser, and the relative path it gets is refused by Stripe rather than
/// landing anyone anywhere wrong.
fn origin_of(headers: &HeaderMap) -> String {
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|origin| origin.starts_with("http"))
    {
        return origin.to_owned();
    }
    headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map_or_else(String::new, |host| format!("https://{host}"))
}

// ------------------------------------------------------------------ webhook

/// One event's envelope, parsed only after its signature has been verified
/// over the bytes it arrived as.
///
/// Stripe's `id` is not read. Fulfilment is keyed on the object the event
/// describes — the session, the subscription, the billing period — because
/// one purchase produces more than one event and keying on the event would
/// make each of them unique enough to pay out again.
#[derive(Debug, Deserialize)]
struct Event {
    /// Stripe's own field is `type`, which is a keyword here. `kind` rather
    /// than `event_type`, because a field naming its own struct reads as
    /// `event.kind` at every call site.
    #[serde(rename = "type")]
    kind: String,
    /// Stripe stamps Unix seconds rather than an RFC 3339 instant, which is
    /// why this module parses no dates.
    #[serde(default)]
    created: i64,
    data: EventData,
}

#[derive(Debug, Deserialize)]
struct EventData {
    object: serde_json::Value,
}

/// A `customer.subscription.*` object, narrowed to what is recorded.
#[derive(Debug, Default, Deserialize)]
struct SubscriptionObject {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    customer: Option<serde_json::Value>,
    #[serde(default)]
    status: Option<String>,
    /// Present on the older API shape. Stripe moved the period onto the
    /// subscription's items, so both are read and the item wins nothing: the
    /// top-level value is preferred where it exists and the item is the
    /// fallback, which is what makes one reader serve both shapes.
    #[serde(default)]
    current_period_end: Option<i64>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    items: SubscriptionItems,
}

#[derive(Debug, Default, Deserialize)]
struct SubscriptionItems {
    #[serde(default)]
    data: Vec<SubscriptionItem>,
}

#[derive(Debug, Default, Deserialize)]
struct SubscriptionItem {
    #[serde(default)]
    current_period_end: Option<i64>,
    #[serde(default)]
    price: Option<stripe::PriceRef>,
}

impl SubscriptionObject {
    fn period_end(&self) -> Option<i64> {
        self.current_period_end.or_else(|| {
            self.items
                .data
                .iter()
                .filter_map(|item| item.current_period_end)
                .max()
        })
    }

    fn price(&self) -> Option<&str> {
        self.items
            .data
            .iter()
            .find_map(|item| item.price.as_ref()?.id.as_deref())
    }
}

/// An `invoice.*` object, narrowed to what is read.
#[derive(Debug, Default, Deserialize)]
struct InvoiceObject {
    #[serde(default)]
    subscription: Option<serde_json::Value>,
    #[serde(default)]
    customer: Option<serde_json::Value>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    /// Where an invoice carries the subscription's own metadata. Stripe has
    /// spelled this two ways across API versions, and both are read because
    /// the account's version is a dashboard setting rather than ours.
    #[serde(default)]
    subscription_details: Option<MetadataHolder>,
    #[serde(default)]
    parent: Option<InvoiceParent>,
    #[serde(default)]
    lines: InvoiceLines,
    /// The invoice's own window, which for a subscription's first invoice is
    /// the instant it was created at both ends. The service period lives on
    /// the lines; these are read only when no line carries one.
    #[serde(default)]
    period_start: Option<i64>,
    #[serde(default)]
    period_end: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct InvoiceParent {
    #[serde(default)]
    subscription_details: Option<SubscriptionDetails>,
}

#[derive(Debug, Default, Deserialize)]
struct SubscriptionDetails {
    #[serde(default)]
    subscription: Option<serde_json::Value>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct MetadataHolder {
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct InvoiceLines {
    #[serde(default)]
    data: Vec<InvoiceLine>,
}

/// One invoice line. The price rides in two places across API versions,
/// like the subscription above: `price` before 2025-03-31, `pricing.
/// price_details.price` from it, and both are read because the account's
/// version is a dashboard setting rather than ours.
#[derive(Debug, Default, Deserialize)]
struct InvoiceLine {
    #[serde(default)]
    price: Option<stripe::PriceRef>,
    #[serde(default)]
    pricing: Option<LinePricing>,
    #[serde(default)]
    period: Option<LinePeriod>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct LinePricing {
    #[serde(default)]
    price_details: Option<PriceDetails>,
}

#[derive(Debug, Default, Deserialize)]
struct PriceDetails {
    #[serde(default)]
    price: Option<String>,
}

impl InvoiceLine {
    fn price(&self) -> Option<&str> {
        self.price
            .as_ref()
            .and_then(|price| price.id.as_deref())
            .or_else(|| {
                self.pricing
                    .as_ref()?
                    .price_details
                    .as_ref()?
                    .price
                    .as_deref()
            })
    }
}

#[derive(Debug, Default, Deserialize)]
struct LinePeriod {
    #[serde(default)]
    start: Option<i64>,
    #[serde(default)]
    end: Option<i64>,
}

impl InvoiceObject {
    fn subscription_id(&self) -> Option<&str> {
        identifier(self.subscription.as_ref()).or_else(|| {
            identifier(
                self.parent
                    .as_ref()?
                    .subscription_details
                    .as_ref()?
                    .subscription
                    .as_ref(),
            )
        })
    }

    /// The service period this invoice paid for: the widest span the lines
    /// cover, and the header only where no line states one. The header of a
    /// subscription's first invoice reads created-to-created, and a grant
    /// expiring on that would lock the seller out a day after paying.
    fn period(&self) -> (Option<i64>, Option<i64>) {
        let periods = || {
            self.lines
                .data
                .iter()
                .filter_map(|line| line.period.as_ref())
        };
        let start = periods().filter_map(|period| period.start).min();
        let end = periods().filter_map(|period| period.end).max();
        (start.or(self.period_start), end.or(self.period_end))
    }

    fn price(&self) -> Option<&str> {
        self.lines.data.iter().find_map(InvoiceLine::price)
    }
}

/// A Stripe field that is either an identifier or, expanded, the object
/// carrying one.
fn identifier(value: Option<&serde_json::Value>) -> Option<&str> {
    let value = value?;
    value.as_str().or_else(|| value.get("id")?.as_str())
}

/// The organisation an event speaks for, from the first place that names one
/// we can read.
///
/// Absent covers every way that can fail — no metadata, no `org` key, a value
/// that is not a UUID — because the route's answer is the same for all of
/// them and distinguishing them here would only invite a log line keyed on
/// something a payload controls.
fn org_from<'a>(candidates: impl IntoIterator<Item = Option<&'a str>>) -> Option<OrgId> {
    candidates
        .into_iter()
        .flatten()
        .find_map(|raw| Uuid::parse_hyphenated(raw).map(OrgId))
}

fn unconfigured() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new("Payments are not available right now. Nothing was charged.")
            .kind(APIErrorKind::Internal),
    )
}

/// A price key the console named that this deployment cannot sell. The key is
/// echoed because it is the console's own vocabulary and not a secret, and
/// because a deployment missing one line of its price map is exactly what
/// this says.
fn unsellable(key: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(&format!("{key} is not for sale.")).kind(APIErrorKind::Validation),
    )
}

/// One refusal for every way a signature fails, saying nothing about which
/// check it failed: the caller here is either Stripe, which never sees this,
/// or somebody probing, who learns nothing they can work with.
fn unsigned() -> APIError {
    APIError::new(
        StatusCode::UNAUTHORIZED,
        APIErrorEntry::new("the event is not signed for this endpoint")
            .kind(APIErrorKind::Unauthenticated),
    )
}

/// A body that carried a valid signature and still could not be read as the
/// event it claims to be.
///
/// Refused rather than acknowledged, and refused only for the event types
/// this route acts on. Stripe records a non-2xx delivery as failed and
/// retries it, so this is the answer that keeps a schema drift visible and
/// recoverable; acknowledging it would drop a real purchase silently, which
/// is the one outcome nothing downstream could detect.
fn unreadable() -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new("the event did not carry the state its type describes")
            .kind(APIErrorKind::Validation),
    )
}

fn at(unix_secs: i64) -> Timestamp {
    Timestamp(unix_secs.saturating_mul(MILLIS_PER_SEC))
}

/// Stripe's event endpoint. No session, no [`OrgContext`]: the signature is
/// the authentication and the organisation travels in the signed payload.
///
/// `Bytes` is the last argument because it consumes the body, and it is the
/// body extractor rather than `Json` because the signature covers the bytes
/// as received. Reading them any other way would verify a re-rendering.
///
/// Acknowledged with 200 and no action: an event type this route does not
/// handle, and an event whose payload names no organisation we can resolve.
/// Both are ordinary — the endpoint may be subscribed to more event types
/// than the five acted on, and a subscription created outside our checkout
/// carries no metadata of ours. Neither is logged: the payload is Stripe's to
/// shape, and a log line per unhandled delivery is a disk-filling primitive
/// handed to whoever can cause one.
///
/// A purchase naming a price this deployment's map does not know is the one
/// ignored case that *is* logged, because it is the one caused by our own
/// configuration rather than by Stripe's traffic: a price added in the
/// dashboard and not in `--stripe-price-map` is a seller who paid and
/// received nothing, and it must not be silent.
pub(crate) async fn webhook(
    _version: APIVersion,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, APIError> {
    let secret = state
        .config
        .stripe_webhook_secret
        .as_ref()
        .ok_or_else(unconfigured)?;
    let signature = headers
        .get(stripe::SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(unsigned)?;
    stripe::verify(signature, &body, secret.expose(), (state.wall)())
        .map_err(|_refusal| unsigned())?;

    let Ok(event) = serde_json::from_slice::<Event>(&body) else {
        return Err(unreadable());
    };
    match event.kind.as_str() {
        CHECKOUT_COMPLETED => checkout_completed(&state, &event).await,
        INVOICE_PAID => invoice_paid(&state, &event).await,
        INVOICE_PAYMENT_FAILED => invoice_failed(&state, &event).await,
        SUBSCRIPTION_UPDATED | SUBSCRIPTION_DELETED => subscription_changed(&state, &event).await,
        _unhandled => Ok(StatusCode::OK),
    }
}

/// A completed Checkout Session: the primary fulfilment trigger for packs,
/// for the service booking, and for a new subscription.
///
/// The session is re-read from Stripe with its line items expanded, because
/// the webhook's own copy carries none and the line items are what name the
/// price a seller chose. Reading the amount instead would be reading a
/// currency, a discount and a tax decision.
async fn checkout_completed(state: &AppState, event: &Event) -> Result<StatusCode, APIError> {
    let Ok(session) = serde_json::from_value::<stripe::CheckoutSession>(event.data.object.clone())
    else {
        return Err(unreadable());
    };
    // Stripe's own fulfilment guidance: act on anything that is not `unpaid`,
    // because `no_payment_required` is a legitimate zero-amount completion.
    if session.payment_status.as_deref() == Some("unpaid") {
        return Ok(StatusCode::OK);
    }
    let Some(org) = org_from([
        session.client_reference_id.as_deref(),
        session.metadata.get(ORG_METADATA_KEY).map(String::as_str),
    ]) else {
        return Ok(StatusCode::OK);
    };
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let expanded = client
        .retrieve_checkout_session(&session.id)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;

    let occurred_at = at(event.created);
    let entitlements = EntitlementRepo::new(state.pool.clone());
    for (price, quantity) in expanded.purchased() {
        let Some(key) = state.config.stripe_price_map.key_for(price) else {
            eprintln!(
                "tam-api: stripe price {price} is not in the price map, so the completed \
                 checkout session granted nothing"
            );
            continue;
        };
        match key {
            PriceKey::MoveWithMe => {
                let booked = BillingRepo::new(state.pool.clone())
                    .record_booking(
                        org,
                        &ServiceBooking {
                            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                            key: key.as_str().to_owned(),
                            provider_ref: expanded.id.clone(),
                            created_at: occurred_at,
                        },
                    )
                    .await
                    .map_err(|error| state.internal(&error.to_string()))?;
                if booked {
                    state.telemetry.capture(
                        org,
                        "service_booked",
                        serde_json::json!({ "price_key": key.as_str() }),
                    );
                }
            }
            PriceKey::Pack20
            | PriceKey::Pack50
            | PriceKey::Pack100
            | PriceKey::Pack250
            | PriceKey::Pack500 => {
                let Some(pack) = PACKS.iter().find(|pack| pack.key == key) else {
                    return Err(state.internal("a pack price key names no pack"));
                };
                // Quantity is honoured rather than assumed: Checkout allows an
                // adjustable quantity and a seller who bought two packs has
                // paid for two packs.
                let moves = i32::try_from(quantity.max(1).saturating_mul(i64::from(pack.moves)))
                    .unwrap_or(i32::MAX);
                let credited = entitlements
                    .credit_moves(
                        org,
                        MoveCredit {
                            delta: moves,
                            source: MoveSource::Pack,
                            source_ref: Some(&expanded.id),
                            expires_at: Some(Timestamp(
                                occurred_at
                                    .0
                                    .saturating_add(PACK_VALIDITY_DAYS * MILLIS_PER_DAY),
                            )),
                            at: occurred_at,
                        },
                    )
                    .await
                    .map_err(|error| state.internal(&error.to_string()))?;
                if credited {
                    state.telemetry.capture(
                        org,
                        "pack_purchased",
                        serde_json::json!({ "price_key": key.as_str(), "moves": moves }),
                    );
                }
            }
            PriceKey::SyncMonthly | PriceKey::SyncYearly | PriceKey::FoundingYearly => {
                let Some(subscription) = expanded.subscription.as_deref() else {
                    // A subscription mode session with no subscription is a
                    // shape Stripe does not send; refusing keeps the drift
                    // visible and replayable.
                    return Err(unreadable());
                };
                subscription_started(
                    state,
                    org,
                    subscription,
                    expanded.customer.as_deref().unwrap_or_default(),
                    price,
                    key,
                    occurred_at,
                )
                .await?;
            }
        }
    }
    Ok(StatusCode::OK)
}

/// The first period of a new subscription: the grant, the founding bonus and
/// the first month's moves.
///
/// Every write is keyed on the subscription identifier or the period, so a
/// redelivery of the session and the `invoice.paid` that follows it land the
/// same facts once.
#[allow(clippy::too_many_arguments)]
async fn subscription_started(
    state: &AppState,
    org: OrgId,
    subscription: &str,
    customer: &str,
    price: &str,
    key: PriceKey,
    occurred_at: Timestamp,
) -> Result<(), APIError> {
    let entitlements = EntitlementRepo::new(state.pool.clone());
    let held = entitlements
        .provider_grant(org, subscription)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if held.is_none() {
        entitlements
            .grant(
                org,
                &NewGrant {
                    id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                    plan: Plan::Subscriber,
                    rung: None,
                    granted_by: GrantedBy::Stripe,
                    grantor_user: None,
                    reason: None,
                    source_ref: Some(subscription),
                    granted_at: occurred_at,
                    // The period end arrives with `customer.subscription.*`
                    // and `invoice.paid`, both of which follow within
                    // seconds. Until one does, the grace is what keeps the
                    // seller working for the day they just paid for.
                    expires_at: Some(Timestamp(
                        occurred_at
                            .0
                            .saturating_add(SUBSCRIPTION_GRACE_HOURS * MILLIS_PER_HOUR),
                    )),
                },
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        state.telemetry.capture(
            org,
            "subscription_started",
            serde_json::json!({ "price_key": key.as_str() }),
        );
    }
    if key == PriceKey::FoundingYearly {
        let _credited: bool = entitlements
            .credit_moves(
                org,
                MoveCredit {
                    delta: i32::try_from(FOUNDING.extra_moves).unwrap_or(i32::MAX),
                    source: MoveSource::Founding,
                    source_ref: Some(&format!("{subscription}:founding")),
                    expires_at: None,
                    at: occurred_at,
                },
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
    }
    accrue(state, org, occurred_at, occurred_at).await?;

    // The subscription row is recorded last, because it is the record of what
    // Stripe said and the grant beside it is what the product reads. Two
    // facts, two rows: the operator can see Stripe reporting `past_due` while
    // the grant still runs to the period end, and neither answer has to be
    // reconstructed from the other.
    //
    // The session carries no period, and `invoice.paid` for the same
    // subscription routinely lands a second before it: a period end already
    // recorded is kept rather than blanked by the later-stamped event.
    let billing = BillingRepo::new(state.pool.clone());
    let current_period_end = billing
        .get(org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .filter(|stored| stored.provider_subscription_id == subscription)
        .and_then(|stored| stored.current_period_end);
    let _applied: bool = billing
        .apply(
            org,
            &SubscriptionState {
                provider_subscription_id: subscription.to_owned(),
                provider_customer_id: customer.to_owned(),
                status: "active".to_owned(),
                provider_price_id: Some(price.to_owned()),
                current_period_end,
                occurred_at,
            },
            (state.wall)(),
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(())
}

/// One billing period's moves, capped so a balance accumulated from
/// subscription periods never climbs past the plan's accrual ceiling.
async fn accrue(
    state: &AppState,
    org: OrgId,
    period_start: Timestamp,
    at: Timestamp,
) -> Result<(), APIError> {
    let capabilities = Plan::Subscriber.capabilities(None);
    let _accrued: bool = EntitlementRepo::new(state.pool.clone())
        .accrue_subscription_moves(
            org,
            Accrual {
                period_start,
                per_period: capabilities.moves_per_month,
                accrual_cap: capabilities.moves_accrual_cap,
                at,
            },
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(())
}

/// A paid invoice: the renewal signal. The grant's expiry moves forward and
/// the period's moves accrue.
async fn invoice_paid(state: &AppState, event: &Event) -> Result<StatusCode, APIError> {
    let Ok(invoice) = serde_json::from_value::<InvoiceObject>(event.data.object.clone()) else {
        return Err(unreadable());
    };
    let Some(org) = org_from([
        invoice.metadata.get(ORG_METADATA_KEY).map(String::as_str),
        invoice
            .subscription_details
            .as_ref()
            .and_then(|held| held.metadata.get(ORG_METADATA_KEY))
            .map(String::as_str),
        invoice
            .parent
            .as_ref()
            .and_then(|parent| parent.subscription_details.as_ref())
            .and_then(|held| held.metadata.get(ORG_METADATA_KEY))
            .map(String::as_str),
        invoice
            .lines
            .data
            .iter()
            .find_map(|line| line.metadata.get(ORG_METADATA_KEY))
            .map(String::as_str),
    ]) else {
        return Ok(StatusCode::OK);
    };
    // A one-off invoice carries no subscription and is nothing to renew; the
    // pack it paid for was fulfilled by the session.
    let Some(subscription) = invoice.subscription_id() else {
        return Ok(StatusCode::OK);
    };
    let (period_start, period_end) = invoice.period();
    let occurred_at = at(event.created);

    let entitlements = EntitlementRepo::new(state.pool.clone());
    let expires_at = period_end.map(|end| {
        Timestamp(
            at(end)
                .0
                .saturating_add(SUBSCRIPTION_GRACE_HOURS * MILLIS_PER_HOUR),
        )
    });
    match entitlements
        .provider_grant(org, subscription)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        Some(grant) => {
            let _moved: bool = entitlements
                .set_expiry(org, grant, expires_at)
                .await
                .map_err(|error| state.internal(&error.to_string()))?;
        }
        // A renewal for a subscription we never recorded is a subscription
        // started outside our checkout or one whose session event was lost.
        // Granting here is what keeps a paying seller entitled either way.
        None => entitlements
            .grant(
                org,
                &NewGrant {
                    id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                    plan: Plan::Subscriber,
                    rung: None,
                    granted_by: GrantedBy::Stripe,
                    grantor_user: None,
                    reason: None,
                    source_ref: Some(subscription),
                    granted_at: occurred_at,
                    expires_at,
                },
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?,
    }
    accrue(
        state,
        org,
        period_start.map_or(occurred_at, at),
        occurred_at,
    )
    .await?;

    let _applied: bool = BillingRepo::new(state.pool.clone())
        .apply(
            org,
            &SubscriptionState {
                provider_subscription_id: subscription.to_owned(),
                provider_customer_id: identifier(invoice.customer.as_ref())
                    .unwrap_or_default()
                    .to_owned(),
                status: "active".to_owned(),
                provider_price_id: invoice.price().map(str::to_owned),
                current_period_end: period_end.map(at),
                occurred_at,
            },
            (state.wall)(),
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(StatusCode::OK)
}

/// A failed payment: a day of grace while Stripe's dunning runs, and nothing
/// else.
///
/// The grant is not revoked here. Stripe retries a failed card for weeks and
/// most of those retries succeed; taking the plan away on the first failure
/// would lock a paying seller out of work they have already paid for. The
/// subscription's own status event is what eventually lapses it.
async fn invoice_failed(state: &AppState, event: &Event) -> Result<StatusCode, APIError> {
    let Ok(invoice) = serde_json::from_value::<InvoiceObject>(event.data.object.clone()) else {
        return Err(unreadable());
    };
    let Some(org) = org_from([
        invoice.metadata.get(ORG_METADATA_KEY).map(String::as_str),
        invoice
            .subscription_details
            .as_ref()
            .and_then(|held| held.metadata.get(ORG_METADATA_KEY))
            .map(String::as_str),
        invoice
            .parent
            .as_ref()
            .and_then(|parent| parent.subscription_details.as_ref())
            .and_then(|held| held.metadata.get(ORG_METADATA_KEY))
            .map(String::as_str),
    ]) else {
        return Ok(StatusCode::OK);
    };
    let Some(subscription) = invoice.subscription_id() else {
        return Ok(StatusCode::OK);
    };
    let entitlements = EntitlementRepo::new(state.pool.clone());
    if let Some(grant) = entitlements
        .provider_grant(org, subscription)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        let _moved: bool = entitlements
            .set_expiry(
                org,
                grant,
                Some(Timestamp(
                    at(event.created)
                        .0
                        .saturating_add(SUBSCRIPTION_GRACE_HOURS * MILLIS_PER_HOUR),
                )),
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
    }
    Ok(StatusCode::OK)
}

/// A subscription's status changed, or it ended.
///
/// An entitling status renews one grant rather than accumulating a row per
/// delivery: this event arrives on every renewal and every card change, and a
/// grant per delivery would make the history unreadable within a month. A
/// deletion moves the same grant's expiry to the period end with no grace, so
/// the seller keeps what they paid for until it runs out and not a day past
/// it.
async fn subscription_changed(state: &AppState, event: &Event) -> Result<StatusCode, APIError> {
    let Ok(object) = serde_json::from_value::<SubscriptionObject>(event.data.object.clone()) else {
        return Err(unreadable());
    };
    let Some(org) = org_from([object.metadata.get(ORG_METADATA_KEY).map(String::as_str)]) else {
        return Ok(StatusCode::OK);
    };
    let (Some(subscription), Some(status)) = (object.id.as_deref(), object.status.as_deref())
    else {
        return Err(unreadable());
    };
    let occurred_at = at(event.created);
    let period_end = object.period_end().map(at);
    let ended = event.kind == SUBSCRIPTION_DELETED;
    let entitling = !ended && ENTITLING.contains(&status);

    let entitlements = EntitlementRepo::new(state.pool.clone());
    let held = entitlements
        .provider_grant(org, subscription)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let expires_at = if entitling {
        period_end.map(|end| {
            Timestamp(
                end.0
                    .saturating_add(SUBSCRIPTION_GRACE_HOURS * MILLIS_PER_HOUR),
            )
        })
    } else {
        // The period end without the grace: a cancellation is the seller's own
        // decision, and extending it by a day would keep entitling them for a
        // day they did not ask for.
        period_end
    };
    match held {
        Some(grant) => {
            let _moved: bool = entitlements
                .set_expiry(org, grant, expires_at)
                .await
                .map_err(|error| state.internal(&error.to_string()))?;
        }
        None if entitling => {
            entitlements
                .grant(
                    org,
                    &NewGrant {
                        id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                        plan: Plan::Subscriber,
                        rung: None,
                        granted_by: GrantedBy::Stripe,
                        grantor_user: None,
                        reason: None,
                        source_ref: Some(subscription),
                        granted_at: occurred_at,
                        expires_at,
                    },
                )
                .await
                .map_err(|error| state.internal(&error.to_string()))?;
        }
        None => {}
    }
    if ended {
        state.telemetry.capture(
            org,
            "subscription_canceled",
            serde_json::json!({ "status": status }),
        );
    }

    let _applied: bool = BillingRepo::new(state.pool.clone())
        .apply(
            org,
            &SubscriptionState {
                provider_subscription_id: subscription.to_owned(),
                provider_customer_id: identifier(object.customer.as_ref())
                    .unwrap_or_default()
                    .to_owned(),
                status: status.to_owned(),
                provider_price_id: object.price().map(str::to_owned),
                current_period_end: period_end,
                occurred_at,
            },
            (state.wall)(),
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::{identifier, org_from, Cadence, Event, InvoiceObject, ORG_METADATA_KEY};
    use tam_limits::PriceKey;

    const ORG: &str = "0a8f6c2e-1d4b-4f3a-9c77-2b5e8d1a4c60";

    #[test]
    fn the_organisation_is_read_from_the_first_candidate_that_names_one() {
        assert!(
            org_from([None, Some("not-a-uuid"), Some(ORG)]).is_some(),
            "an unreadable candidate is skipped rather than ending the search"
        );
        assert!(
            org_from([None, Some("not-a-uuid")]).is_none(),
            "no readable candidate is no organisation, and the route ignores the event"
        );
    }

    #[test]
    fn an_expanded_field_reads_the_same_identifier_as_a_bare_one() {
        let bare = serde_json::json!("sub_01");
        let expanded = serde_json::json!({ "id": "sub_01", "object": "subscription" });
        assert_eq!(identifier(Some(&bare)), Some("sub_01"));
        assert_eq!(
            identifier(Some(&expanded)),
            Some("sub_01"),
            "expanding a field must not change which subscription it names"
        );
    }

    #[test]
    fn the_envelope_reads_stripes_own_shape() {
        let event: Event = serde_json::from_str(
            r#"{"id":"evt_01","type":"invoice.paid","created":1800000000,
                "data":{"object":{"id":"in_01"}}}"#,
        )
        .expect("Stripe's envelope reads");
        assert_eq!(event.kind, "invoice.paid");
        assert_eq!(event.created, 1_800_000_000);
        assert_eq!(event.data.object["id"], "in_01");
    }

    #[test]
    fn an_invoice_line_names_its_price_in_either_api_version() {
        let acacia: InvoiceObject = serde_json::from_value(serde_json::json!({
            "lines": { "data": [ { "price": { "id": "price_01" } } ] }
        }))
        .expect("the pre-basil line reads");
        let basil: InvoiceObject = serde_json::from_value(serde_json::json!({
            "lines": { "data": [ { "pricing": { "price_details": { "price": "price_01" } } } ] }
        }))
        .expect("the basil line reads");
        assert_eq!(acacia.price(), Some("price_01"));
        assert_eq!(
            basil.price(),
            Some("price_01"),
            "a renewal on a newer account version must not blank the price the page reads"
        );
    }

    #[test]
    fn the_service_period_is_the_lines_not_the_invoice_header() {
        // A first subscription invoice as Stripe sends it: the header window
        // is the creation instant twice, the line carries the month paid for.
        let first: InvoiceObject = serde_json::from_value(serde_json::json!({
            "period_start": 1_790_085_745, "period_end": 1_790_085_745,
            "lines": { "data": [ { "period": { "start": 1_790_085_745, "end": 1_792_677_745 } } ] }
        }))
        .expect("the invoice reads");
        assert_eq!(
            first.period(),
            (Some(1_790_085_745), Some(1_792_677_745)),
            "expiring on the header would lock the seller out a day after paying"
        );

        let bare: InvoiceObject = serde_json::from_value(serde_json::json!({
            "period_start": 1, "period_end": 2, "lines": { "data": [ {} ] }
        }))
        .expect("the invoice reads");
        assert_eq!(
            bare.period(),
            (Some(1), Some(2)),
            "no line period, the header stands"
        );
    }

    #[test]
    fn only_the_recurring_keys_carry_a_cadence() {
        assert_eq!(Cadence::of(PriceKey::SyncMonthly), Some(Cadence::Monthly));
        assert_eq!(Cadence::of(PriceKey::SyncYearly), Some(Cadence::Yearly));
        assert_eq!(
            Cadence::of(PriceKey::FoundingYearly),
            Some(Cadence::Yearly),
            "founding is an annual price, and the page says so"
        );
        assert_eq!(
            Cadence::of(PriceKey::Pack100),
            None,
            "a pack renews nothing, so it has no cadence to state"
        );
    }

    #[test]
    fn the_metadata_key_is_the_one_the_checkout_writes() {
        assert_eq!(
            ORG_METADATA_KEY, "org",
            "one constant, two readers: changing it here changes the checkout"
        );
    }
}
