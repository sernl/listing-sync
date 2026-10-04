//! The refund policy over the wire: the quote route, the refund that records
//! the quote beside the amount, and a seller's "Ask for a refund" from the
//! billing page through an operator's Approve or Decline.
//!
//! The purchases are made the way production makes them, through signed
//! billing webhooks (billing_flow's harness): a Move Pack's completed
//! checkout credits its moves, and a yearly plan's checkout and paid invoice
//! grant the plan. A loopback Stripe double answers the reads the quote
//! makes (the charge, its invoice, the checkout session behind a pack) and
//! records every refund and every immediate cancellation.
//!
//! The clock is 2027-01-15 21:00 in Auckland. The yearly plan started on
//! 20 September 2026 at noon New Zealand time, so the refund is asked for in
//! month 4: months 5 to 12 are unused, and the terms' own example applies,
//! 7 × $240 ÷ 12 = $140.

#![cfg(feature = "pg-tests")]

use core::fmt::Write as _;
use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    routing::{delete, get, post},
    Form, Json,
};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::payments::{PaymentsAdminView, RefundView};
use tam_api::refund_quote::QuoteView;
use tam_api::refund_requests::{BillingRefundsView, RefundRequestView};
use tam_api::{
    router, stripe, AppState, Config, PriceMap, SecretKey, WebhookSecret, ORG_METADATA_KEY,
    SESSION_COOKIE,
};
use tam_storage::{EntitlementRepo, OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tokio::sync::Mutex;
use tower::ServiceExt;

const ORG_SELLER: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_OTHER: OrgId = OrgId(Uuid([0xBB; 16]));
const ORG_OPERATOR: OrgId = OrgId(Uuid([0xCC; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0A; 16]));
const USER_OTHER: UserId = UserId(Uuid([0x0B; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0C; 16]));
const TOKEN_SELLER: SessionToken = SessionToken([0x41; 32]);
const TOKEN_OTHER: SessionToken = SessionToken([0x42; 32]);
const TOKEN_OPERATOR: SessionToken = SessionToken([0x43; 32]);

const SECRET: &str = "whsec_01_development_only";
/// 2027-01-15T08:00:00Z, 21:00 in Auckland.
const NOW_SECS: i64 = 1_800_000_000;
const NOW: Timestamp = Timestamp(NOW_SECS * 1_000);
const DAY_SECS: i64 = 86_400;

/// 2026-09-20T00:00:00Z, noon in Auckland, and a year on.
const YEAR_START: i64 = 1_789_862_400;
const YEAR_END: i64 = 1_821_398_400;
/// 2027-01-12T00:00:00Z, a month that has started.
const MONTH_START: i64 = 1_799_712_000;
/// Two days before the clock.
const PACK_BOUGHT: i64 = NOW_SECS - 2 * DAY_SECS;

const CUSTOMER: &str = "cus_01";
const YEAR_CHARGE: &str = "ch_year";
const YEAR_INTENT: &str = "pi_year";
const YEAR_INVOICE: &str = "in_year";
const YEAR_SESSION: &str = "cs_year";
const YEAR_SUBSCRIPTION: &str = "sub_year";
const YEAR_PAID: i64 = 24_000;
const MONTH_CHARGE: &str = "ch_month";
const MONTH_INVOICE: &str = "in_month";
const PACK_CHARGE: &str = "ch_pack";
const PACK_INTENT: &str = "pi_pack";
const PACK_SESSION: &str = "cs_pack";
const PACK_PAID: i64 = 2_900;
const PACK_MOVES: i64 = 100;

const REQUEST_A: &str = "0f0f0f0f-0000-4000-8000-00000000000a";

/// What the double holds: each charge's running refund total, every refund
/// POST by idempotency key, and every subscription cancelled outright.
#[derive(Default)]
struct Double {
    refunded: BTreeMap<String, i64>,
    by_key: BTreeMap<String, serde_json::Value>,
    cancelled: Vec<String>,
}

type Shared = Arc<Mutex<Double>>;

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn price_map() -> PriceMap {
    let entries: serde_json::Map<String, serde_json::Value> = tam_limits::PriceKey::ALL
        .into_iter()
        .map(|key| (format!("price_{}", key.as_str()), key.as_str().into()))
        .collect();
    PriceMap::parse(&serde_json::Value::Object(entries).to_string())
        .expect("the fixture price map parses")
}

fn charge(id: &str, refunded: i64) -> Option<serde_json::Value> {
    let (amount, intent, invoice, created) = match id {
        YEAR_CHARGE => (YEAR_PAID, YEAR_INTENT, Some(YEAR_INVOICE), YEAR_START),
        MONTH_CHARGE => (2_900, "pi_month", Some(MONTH_INVOICE), MONTH_START),
        PACK_CHARGE => (PACK_PAID, PACK_INTENT, None, PACK_BOUGHT),
        _unknown => return None,
    };
    Some(serde_json::json!({
        "id": id, "object": "charge", "amount": amount, "amount_refunded": refunded,
        "refunded": refunded >= amount, "currency": "usd", "status": "succeeded",
        "customer": CUSTOMER, "payment_intent": intent, "invoice": invoice,
        "created": created, "metadata": { "org": ORG_SELLER.0.to_hyphenated() }
    }))
}

fn invoice_object(id: &str) -> Option<serde_json::Value> {
    let (subscription, price, start, end) = match id {
        YEAR_INVOICE => (YEAR_SUBSCRIPTION, "price_pro_yearly", YEAR_START, YEAR_END),
        MONTH_INVOICE => (
            "sub_month",
            "price_pro_monthly",
            MONTH_START,
            MONTH_START + 31 * DAY_SECS,
        ),
        _unknown => return None,
    };
    Some(serde_json::json!({
        "id": id, "object": "invoice", "customer": CUSTOMER, "subscription": subscription,
        "status": "paid", "period_start": start, "period_end": start,
        "subscription_details": { "metadata": { ORG_METADATA_KEY: ORG_SELLER.0.to_hyphenated() } },
        "lines": { "object": "list", "data": [{
            "id": "il_01", "price": { "id": price, "object": "price" },
            "period": { "start": start, "end": end }
        }]}
    }))
}

fn session(id: &str) -> serde_json::Value {
    let (mode, price, subscription, created) = if id == YEAR_SESSION {
        (
            "subscription",
            "price_pro_yearly",
            Some(YEAR_SUBSCRIPTION),
            YEAR_START,
        )
    } else {
        ("payment", "price_pack_100", None, PACK_BOUGHT)
    };
    serde_json::json!({
        "id": id, "object": "checkout.session", "mode": mode, "payment_status": "paid",
        "customer": CUSTOMER, "subscription": subscription, "created": created,
        "client_reference_id": ORG_SELLER.0.to_hyphenated(),
        "metadata": { ORG_METADATA_KEY: ORG_SELLER.0.to_hyphenated() },
        "line_items": { "object": "list", "data": [{
            "id": "li_01", "quantity": 1, "price": { "id": price, "object": "price" }
        }]}
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the double's lists read better built inline at the call"
)]
fn list(data: serde_json::Value) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "object": "list", "data": data, "has_more": false }))
}

fn missing() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": { "message": "No such object" } })),
    )
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "the refund handler holds the double's lock for the whole call, which is what makes the idempotency check and the write one step"
)]
async fn stripe_double() -> (String, Shared) {
    let shared: Shared = Arc::default();
    let app = axum::Router::new()
        .route(
            "/v1/charges/{id}",
            get(
                |State(shared): State<Shared>, Path(id): Path<String>| async move {
                    let refunded = shared.lock().await.refunded.get(&id).copied().unwrap_or(0);
                    charge(&id, refunded).map_or_else(missing, |found| (StatusCode::OK, Json(found)))
                },
            ),
        )
        .route(
            "/v1/invoices/{id}",
            get(|Path(id): Path<String>| async move {
                invoice_object(&id).map_or_else(missing, |found| (StatusCode::OK, Json(found)))
            }),
        )
        .route("/v1/invoice_payments", get(|| async { list(serde_json::json!([])) }))
        .route(
            "/v1/checkout/sessions/{id}",
            get(|Path(id): Path<String>| async move { Json(session(&id)) }),
        )
        .route(
            "/v1/checkout/sessions",
            get(|Query(query): Query<BTreeMap<String, String>>| async move {
                if query.get("payment_intent").map(String::as_str) == Some(PACK_INTENT) {
                    list(serde_json::json!([session(PACK_SESSION)]))
                } else {
                    list(serde_json::json!([]))
                }
            }),
        )
        .route(
            "/v1/subscriptions/{id}",
            delete(
                |State(shared): State<Shared>, Path(id): Path<String>| async move {
                    shared.lock().await.cancelled.push(id.clone());
                    Json(serde_json::json!({ "id": id, "object": "subscription", "status": "canceled" }))
                },
            ),
        )
        .route(
            "/v1/refunds",
            post(
                |State(shared): State<Shared>,
                 headers: HeaderMap,
                 Form(form): Form<Vec<(String, String)>>| async move {
                    let field = |name: &str| {
                        form.iter()
                            .find(|(key, _)| key == name)
                            .map(|(_, value)| value.clone())
                    };
                    let key = headers
                        .get("Idempotency-Key")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let mut held = shared.lock().await;
                    if let Some(seen) = key.as_ref().and_then(|key| held.by_key.get(key)) {
                        return Json(seen.clone());
                    }
                    let amount: i64 = field("amount")
                        .and_then(|raw| raw.parse().ok())
                        .unwrap_or(0);
                    let charge = field("charge").unwrap_or_default();
                    *held.refunded.entry(charge.clone()).or_insert(0) += amount;
                    let refund = serde_json::json!({
                        "id": format!("re_{}", held.by_key.len() + 1), "object": "refund",
                        "amount": amount, "currency": "usd", "charge": charge,
                        "status": "succeeded", "reason": field("reason"), "created": NOW_SECS
                    });
                    if let Some(key) = key {
                        held.by_key.insert(key, refund.clone());
                    }
                    Json(refund)
                },
            ),
        )
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the double binds a loopback port");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the double has an address")
    );
    tam_api::blocking::spawn_supervised("stripe double", async move {
        let _served: Result<(), std::io::Error> = axum::serve(listener, app).await;
    });
    (base, shared)
}

fn state(pool: PgPool, base: &str) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            stripe_webhook_secret: Some(WebhookSecret::new(SECRET.to_owned())),
            stripe: Some(stripe::Client::with_base(
                SecretKey::new("sk_test_development_only".to_owned()),
                base.to_owned(),
            )),
            stripe_price_map: price_map(),
            ..Config::default()
        },
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [
        (ORG_SELLER, "Kauri Room"),
        (ORG_OTHER, "Tui Teaching"),
        (ORG_OPERATOR, "Teachouse"),
    ] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_SELLER, USER_SELLER, "seller@example.test", TOKEN_SELLER),
        (ORG_OTHER, USER_OTHER, "other@example.test", TOKEN_OTHER),
        (
            ORG_OPERATOR,
            USER_OPERATOR,
            "operator@example.test",
            TOKEN_OPERATOR,
        ),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(
                &token,
                user,
                Timestamp(NOW.0 + 86_400_000),
                Timestamp(1_000),
            )
            .await
            .expect("the session mints");
    }
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", Timestamp(1_000))
        .await
        .expect("the operator marking lands");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn sign(body: &str) -> String {
    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(SECRET.as_bytes())
        .expect("HMAC accepts a key of any length");
    mac.update(NOW_SECS.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let mut digest = String::new();
    for byte in mac.finalize().into_bytes() {
        write!(digest, "{byte:02x}").expect("writing to a String is infallible");
    }
    format!("t={NOW_SECS},v1={digest}")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn deliver(pool: &PgPool, base: &str, event: &serde_json::Value) {
    let body = event.to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header(header::CONTENT_TYPE, "application/json")
        .header("Stripe-Signature", sign(&body))
        .body(Body::from(body))
        .expect("the request builds");
    let status = router(state(pool.clone(), base))
        .oneshot(request)
        .await
        .expect("the router serves")
        .status();
    assert_eq!(status, StatusCode::OK, "the webhook takes {event}");
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the fixtures read better built inline at the call"
)]
fn event(id: &str, kind: &str, created: i64, object: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": id, "type": kind, "created": created, "data": { "object": object } })
}

/// The seller's purchases, as Stripe tells us about them: a Move Pack two
/// days ago, and a Pro yearly plan since 20 September; and a charge event
/// for each payment the billing page lists.
async fn purchase(pool: &PgPool, base: &str) {
    for (id, session_id, mode) in [
        ("evt_pack", PACK_SESSION, "payment"),
        ("evt_year", YEAR_SESSION, "subscription"),
    ] {
        let created = if mode == "payment" {
            PACK_BOUGHT
        } else {
            YEAR_START
        };
        let mut object = session(session_id);
        object["line_items"] = serde_json::Value::Null;
        deliver(
            pool,
            base,
            &event(id, "checkout.session.completed", created, object),
        )
        .await;
    }
    let mut paid = invoice_object(YEAR_INVOICE).unwrap_or_default();
    paid["charge"] = YEAR_CHARGE.into();
    paid["payment_intent"] = YEAR_INTENT.into();
    deliver(
        pool,
        base,
        &event("evt_invoice", "invoice.paid", YEAR_START, paid),
    )
    .await;
    for (id, charge_id) in [
        ("evt_ch_year", YEAR_CHARGE),
        ("evt_ch_month", MONTH_CHARGE),
        ("evt_ch_pack", PACK_CHARGE),
    ] {
        let object = charge(charge_id, 0).unwrap_or_default();
        let created = object["created"].as_i64().unwrap_or(NOW_SECS);
        deliver(pool, base, &event(id, "charge.succeeded", created, object)).await;
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    (pool, base): (&PgPool, &str),
    token: &SessionToken,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let response = router(state(pool.clone(), base))
        .oneshot(
            request
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .expect("the request builds"),
        )
        .await
        .expect("the router serves");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    serde_json::from_slice(bytes).expect("the answer body parses")
}

async fn admin_quote(pool: &PgPool, base: &str, charge_id: &str) -> QuoteView {
    let (status, body) = call(
        (pool, base),
        &TOKEN_OPERATOR,
        Method::GET,
        &format!("/v1/admin/payments/charges/{charge_id}/quote"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    json(&body)
}

/// One pinned scalar read on a fenced table.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn pinned<T>(pool: &PgPool, sql: &str) -> Vec<T>
where
    T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(ORG_SELLER.0.to_hyphenated())
        .execute(&mut *tx)
        .await
        .expect("the pin lands");
    let rows: Vec<(T,)> = sqlx::query_as(sql)
        .fetch_all(&mut *tx)
        .await
        .expect("the read runs");
    tx.commit().await.expect("the read commits");
    rows.into_iter().map(|(value,)| value).collect()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_yearly_charge_is_quoted_by_the_terms_formula(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    purchase(&pool, &base).await;

    let quote = admin_quote(&pool, &base, YEAR_CHARGE).await;
    assert_eq!(quote.amount_cents, 14_000, "{quote:?}");
    assert_eq!(quote.basis.as_str(), "yearly_unused_months");
    assert_eq!(quote.what.as_deref(), Some("Pro yearly plan"));
    assert_eq!(quote.org_id, Some(ORG_SELLER.0.to_hyphenated()));
    assert!(quote.ends_plan, "a yearly refund ends the plan");
    assert_eq!(
        quote.explanation,
        "This is month 4 of the yearly plan, so months 5 to 12 are unused: 8 whole months. \
         Less one month's fee, the refund is 7 × $240.00 ÷ 12 = $140.00."
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_monthly_charge_and_an_unknown_charge_quote_nothing(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    purchase(&pool, &base).await;

    let monthly = admin_quote(&pool, &base, MONTH_CHARGE).await;
    assert_eq!(monthly.amount_cents, 0);
    assert_eq!(monthly.basis.as_str(), "monthly_started");
    assert!(!monthly.ends_plan, "only a yearly refund ends the plan");

    let (status, _body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::GET,
        "/v1/admin/payments/charges/ch_nobody/quote",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "Stripe has no such charge");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_pack_is_refundable_in_full_until_a_move_is_used(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    purchase(&pool, &base).await;

    let unused = admin_quote(&pool, &base, PACK_CHARGE).await;
    assert_eq!(unused.amount_cents, PACK_PAID);
    assert_eq!(unused.basis.as_str(), "pack_unused");
    assert_eq!(unused.what.as_deref(), Some("Move Pack of 100 moves"));

    let spent = EntitlementRepo::new(pool.clone())
        .debit_move(ORG_SELLER, Uuid([0x77; 16]), NOW)
        .await
        .expect("the move debits");
    assert!(spent.is_some(), "one move is committed");
    let used = admin_quote(&pool, &base, PACK_CHARGE).await;
    assert_eq!(used.amount_cents, 0);
    assert_eq!(used.basis.as_str(), "pack_used");
    assert_eq!(
        used.explanation,
        "1 move from this Move Pack has been used, so it isn't refunded."
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refund_records_the_policy_beside_the_amount(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    purchase(&pool, &base).await;
    let path = format!("/v1/admin/payments/charges/{YEAR_CHARGE}/refunds");
    let body = |amount: i64, quoted: i64, reason: &str| {
        serde_json::json!({
            "request_id": REQUEST_A, "amount_cents": amount, "reason": "requested_by_customer",
            "issued_by_label": "Sam", "policy_basis": "yearly_unused_months",
            "quoted_cents": quoted, "override_reason": reason
        })
    };

    let (stale, stale_body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &path,
        Some(body(15_000, 15_000, "")),
    )
    .await;
    assert_eq!(stale, StatusCode::CONFLICT, "a quote that moved is refused");
    assert!(String::from_utf8_lossy(&stale_body).contains("$140.00"));

    let (unexplained, _body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &path,
        Some(body(20_000, 14_000, "")),
    )
    .await;
    assert_eq!(
        unexplained,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an amount other than the quote must say why"
    );

    let (made, made_body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &path,
        Some(body(20_000, 14_000, "Our sync was down for a fortnight")),
    )
    .await;
    assert_eq!(
        made,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&made_body)
    );
    let refund: RefundView = json(&made_body);
    assert_eq!(refund.amount_cents, 20_000);
    assert_eq!(refund.quoted_cents, Some(14_000));
    assert_eq!(refund.policy_basis.as_deref(), Some("yearly_unused_months"));
    assert_eq!(
        refund.override_reason.as_deref(),
        Some("Our sync was down for a fortnight")
    );

    let (status, body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::GET,
        "/v1/admin/payments",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: PaymentsAdminView = json(&body);
    assert_eq!(
        view.refunds
            .iter()
            .find(|row| row.id == REQUEST_A)
            .and_then(|row| row.quoted_cents),
        Some(14_000),
        "the audit lists the policy beside the actual"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_asks_and_an_approval_refunds_and_ends_the_yearly_plan(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = stripe_double().await;
    purchase(&pool, &base).await;

    let (status, body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::GET,
        "/v1/billing/refund-requests",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let offered: BillingRefundsView = json(&body);
    let charges: Vec<&str> = offered
        .payments
        .iter()
        .map(|payment| payment.charge_id.as_str())
        .collect();
    assert_eq!(
        charges,
        [PACK_CHARGE, MONTH_CHARGE, YEAR_CHARGE],
        "newest first"
    );
    assert_eq!(
        offered
            .payments
            .iter()
            .find(|payment| payment.charge_id == YEAR_CHARGE)
            .and_then(|payment| payment.what.as_deref()),
        Some("Pro yearly plan")
    );

    let (quoted, quote_body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::GET,
        &format!("/v1/billing/payments/{YEAR_CHARGE}/refund-quote"),
        None,
    )
    .await;
    assert_eq!(quoted, StatusCode::OK);
    assert_eq!(json::<QuoteView>(&quote_body).amount_cents, 14_000);

    let ask = serde_json::json!({ "charge_id": YEAR_CHARGE, "note": "We've moved schools." });
    let (asked, asked_body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::POST,
        "/v1/billing/refund-requests",
        Some(ask.clone()),
    )
    .await;
    assert_eq!(
        asked,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&asked_body)
    );
    let request: RefundRequestView = json(&asked_body);
    assert_eq!(request.quoted_cents, 14_000);
    assert_eq!(request.status, "requested");
    assert_eq!(request.note.as_deref(), Some("We've moved schools."));

    let (again, _body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::POST,
        "/v1/billing/refund-requests",
        Some(ask),
    )
    .await;
    assert_eq!(again, StatusCode::CONFLICT, "asking twice is asking once");

    let mailed: Vec<serde_json::Value> = pinned(
        &pool,
        "SELECT payload FROM outbox_message WHERE topic = 'email.refund_requested'",
    )
    .await;
    assert_eq!(mailed.len(), 1, "the operators are mailed once");
    assert_eq!(mailed[0]["quoted_cents"], 14_000);
    assert_eq!(mailed[0]["note"], "We've moved schools.");

    let (status, body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::GET,
        "/v1/admin/payments",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: PaymentsAdminView = json(&body);
    assert_eq!(view.requests.len(), 1);
    assert_eq!(view.requests[0].org_name.as_deref(), Some("Kauri Room"));

    let approve = format!("/v1/admin/payments/refund-requests/{}/approve", request.id);
    let decided = serde_json::json!({ "decided_by_label": "Sam" });
    let (approved, approved_body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &approve,
        Some(decided.clone()),
    )
    .await;
    assert_eq!(
        approved,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&approved_body)
    );
    let approved: RefundRequestView = json(&approved_body);
    assert_eq!(approved.status, "approved");
    assert_eq!(approved.refund_id.as_deref(), Some(request.id.as_str()));
    assert_eq!(approved.decided_by.as_deref(), Some("Sam"));

    let refunded = double.lock().await.refunded.get(YEAR_CHARGE).copied();
    assert_eq!(refunded, Some(14_000), "the quoted refund went out");
    let cancelled = double.lock().await.cancelled.clone();
    assert_eq!(
        cancelled,
        [YEAR_SUBSCRIPTION],
        "the yearly plan is cancelled now"
    );
    let expiry: Vec<chrono::DateTime<chrono::Utc>> = pinned(
        &pool,
        "SELECT expires_at FROM entitlement_grant WHERE source_ref = 'sub_year' AND revoked_at IS NULL",
    )
    .await;
    assert_eq!(
        expiry.first().map(chrono::DateTime::timestamp_millis),
        Some(NOW.0),
        "the plan ends on the day of the refund"
    );

    let (twice, _body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &approve,
        Some(decided),
    )
    .await;
    assert_eq!(
        twice,
        StatusCode::CONFLICT,
        "an approved request is not approved again"
    );
    assert_eq!(
        double.lock().await.refunded.get(YEAR_CHARGE),
        Some(&14_000),
        "and no more money moves"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_declined_request_mails_the_seller_the_reason(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = stripe_double().await;
    purchase(&pool, &base).await;

    let (asked, asked_body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::POST,
        "/v1/billing/refund-requests",
        Some(serde_json::json!({ "charge_id": PACK_CHARGE })),
    )
    .await;
    assert_eq!(
        asked,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&asked_body)
    );
    let request: RefundRequestView = json(&asked_body);
    assert_eq!(request.quoted_cents, PACK_PAID);
    let decline = format!("/v1/admin/payments/refund-requests/{}/decline", request.id);

    let (silent, _body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &decline,
        Some(serde_json::json!({ "reason": "  " })),
    )
    .await;
    assert_eq!(
        silent,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a decline says why"
    );

    let reason = "This pack was bought for a second shop that's already covered.";
    let (declined, declined_body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &decline,
        Some(serde_json::json!({ "reason": reason, "decided_by_label": "Sam" })),
    )
    .await;
    assert_eq!(
        declined,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&declined_body)
    );
    let declined: RefundRequestView = json(&declined_body);
    assert_eq!(declined.status, "declined");
    assert_eq!(declined.decline_reason.as_deref(), Some(reason));
    assert!(double.lock().await.refunded.is_empty(), "no money moves");

    let mailed: Vec<serde_json::Value> = pinned(
        &pool,
        "SELECT payload FROM outbox_message WHERE topic = 'email.refund_declined'",
    )
    .await;
    assert_eq!(mailed.len(), 1);
    assert_eq!(mailed[0]["reason"], reason);
    assert_eq!(mailed[0]["quoted_cents"], PACK_PAID);

    let (status, body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::GET,
        "/v1/billing/refund-requests",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mine: BillingRefundsView = json(&body);
    assert_eq!(mine.requests.len(), 1);
    assert_eq!(mine.requests[0].status, "declined");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_approved_pack_refund_takes_the_moves_back(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    purchase(&pool, &base).await;
    let entitlements = EntitlementRepo::new(pool.clone());
    let before = entitlements
        .move_balance(ORG_SELLER, NOW)
        .await
        .map(|balance| balance.available);

    let (asked, asked_body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::POST,
        "/v1/billing/refund-requests",
        Some(serde_json::json!({ "charge_id": PACK_CHARGE })),
    )
    .await;
    assert_eq!(asked, StatusCode::CREATED);
    let request: RefundRequestView = json(&asked_body);
    let (approved, approved_body) = call(
        (&pool, &base),
        &TOKEN_OPERATOR,
        Method::POST,
        &format!("/v1/admin/payments/refund-requests/{}/approve", request.id),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        approved,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&approved_body)
    );

    let after = entitlements
        .move_balance(ORG_SELLER, NOW)
        .await
        .map(|balance| balance.available);
    assert_eq!(
        before.as_ref().ok().map(|moves| moves - PACK_MOVES),
        after.ok(),
        "the refunded pack's moves leave the balance"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_reaches_only_their_own_payments_and_only_a_refund_the_policy_gives(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    purchase(&pool, &base).await;

    let (quoted, _body) = call(
        (&pool, &base),
        &TOKEN_OTHER,
        Method::GET,
        &format!("/v1/billing/payments/{YEAR_CHARGE}/refund-quote"),
        None,
    )
    .await;
    assert_eq!(
        quoted,
        StatusCode::NOT_FOUND,
        "another organisation's charge is missing"
    );
    let (asked, _body) = call(
        (&pool, &base),
        &TOKEN_OTHER,
        Method::POST,
        "/v1/billing/refund-requests",
        Some(serde_json::json!({ "charge_id": YEAR_CHARGE })),
    )
    .await;
    assert_eq!(asked, StatusCode::NOT_FOUND);
    let (status, body) = call(
        (&pool, &base),
        &TOKEN_OTHER,
        Method::GET,
        "/v1/billing/refund-requests",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let theirs: BillingRefundsView = json(&body);
    assert!(
        theirs.payments.is_empty(),
        "nothing of the seller's is listed to another tenant"
    );

    let (nothing, nothing_body) = call(
        (&pool, &base),
        &TOKEN_SELLER,
        Method::POST,
        "/v1/billing/refund-requests",
        Some(serde_json::json!({ "charge_id": MONTH_CHARGE })),
    )
    .await;
    assert_eq!(nothing, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        String::from_utf8_lossy(&nothing_body).contains("a month that has started isn't refunded"),
        "the refusal is the policy's own sentence"
    );

    for (token, path) in [
        (
            &TOKEN_SELLER,
            format!("/v1/admin/payments/charges/{YEAR_CHARGE}/quote"),
        ),
        (
            &TOKEN_SELLER,
            format!("/v1/admin/payments/refund-requests/{REQUEST_A}/approve"),
        ),
    ] {
        let method = if path.ends_with("approve") {
            Method::POST
        } else {
            Method::GET
        };
        let (status, _body) = call(
            (&pool, &base),
            token,
            method,
            &path,
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a seller is not an operator: {path}"
        );
    }
}
