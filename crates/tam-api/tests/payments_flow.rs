//! The operators' Payments page over the wire: the ledger the webhook and the
//! sync fill, and the refunds issued from it, against a loopback Stripe
//! double that holds one charge and records every call.
//!
//! What is proven is the money: a redelivered event is one row; a synced
//! object is one row however often the sync runs, and gives way to Stripe's
//! own event when that arrives; a refund never exceeds what Stripe says is
//! left on the charge, is made once however often it is asked for, names who
//! made it, and queues the customer's mail when asked or when the auto
//! setting says so.

#![cfg(feature = "pg-tests")]

use core::fmt::Write as _;
use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    routing::{get, post},
    Form, Json,
};
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::payments::{PaymentsAdminView, RefundView, SyncView};
use tam_api::{router, stripe, AppState, Config, SecretKey, WebhookSecret, SESSION_COOKIE};
use tam_storage::{OperatorRepo, PaymentRepo, RefundMailOutcome, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tokio::sync::Mutex;
use tower::ServiceExt;

const ORG_OPERATOR: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_SELLER: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);

const SECRET: &str = "whsec_01_development_only";
const NOW_SECS: i64 = 1_800_000_000;
const NOW: Timestamp = Timestamp(NOW_SECS * 1_000);

const CHARGE: &str = "ch_01";
const INTENT: &str = "pi_01";
const CUSTOMER: &str = "cus_01";
const CHARGED: i64 = 2_900;

const REQUEST_A: &str = "0f0f0f0f-0000-4000-8000-00000000000a";
const REQUEST_B: &str = "0f0f0f0f-0000-4000-8000-00000000000b";

/// What the double holds: the charge's running refund total, and every
/// refund POST it received with its idempotency key.
#[derive(Default)]
struct Double {
    refunded: i64,
    refunds: Vec<(Option<String>, Vec<(String, String)>)>,
    by_key: BTreeMap<String, serde_json::Value>,
}

type Shared = Arc<Mutex<Double>>;

fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn charge_object(refunded: i64) -> serde_json::Value {
    serde_json::json!({
        "id": CHARGE, "object": "charge", "amount": CHARGED, "amount_refunded": refunded,
        "refunded": refunded >= CHARGED, "currency": "usd", "status": "succeeded",
        "customer": CUSTOMER, "payment_intent": INTENT, "created": NOW_SECS - 3_600,
        "metadata": { "org": ORG_SELLER.0.to_hyphenated() }
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the double's lists read better built inline at the call"
)]
fn list(data: serde_json::Value) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "object": "list", "data": data, "has_more": false }))
}

/// A Stripe double: one charge, its refunds, and the four lists the sync
/// reads. A refund POST honours the idempotency key the way Stripe does —
/// the same key answers the first refund and moves no more money.
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
                    if id != CHARGE {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(serde_json::json!({ "error": { "message": "No such charge" } })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(charge_object(shared.lock().await.refunded)),
                    )
                },
            ),
        )
        .route(
            "/v1/refunds",
            post(
                |State(shared): State<Shared>,
                 headers: HeaderMap,
                 Form(form): Form<Vec<(String, String)>>| async move {
                    let key = headers
                        .get("Idempotency-Key")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let mut held = shared.lock().await;
                    held.refunds.push((key.clone(), form.clone()));
                    if let Some(seen) = key.as_ref().and_then(|key| held.by_key.get(key)) {
                        return Json(seen.clone());
                    }
                    let amount: i64 = field(&form, "amount")
                        .and_then(|raw| raw.parse().ok())
                        .unwrap_or(0);
                    held.refunded += amount;
                    let refund = serde_json::json!({
                        "id": format!("re_{}", held.by_key.len() + 1), "object": "refund",
                        "amount": amount, "currency": "usd", "charge": CHARGE,
                        "status": "pending", "reason": field(&form, "reason"),
                        "created": NOW_SECS
                    });
                    if let Some(key) = key {
                        held.by_key.insert(key, refund.clone());
                    }
                    Json(refund)
                },
            )
            .get(|| async {
                list(serde_json::json!([{
                    "id": "re_dash", "object": "refund", "amount": 500, "currency": "usd",
                    "charge": CHARGE, "status": "succeeded", "reason": "requested_by_customer",
                    "created": NOW_SECS - 1_800
                }]))
            }),
        )
        .route(
            "/v1/charges",
            get(|State(shared): State<Shared>| async move {
                let refunded = shared.lock().await.refunded;
                list(serde_json::json!([
                    charge_object(refunded),
                    { "id": "ch_pending", "object": "charge", "status": "pending", "amount": 100,
                      "currency": "usd", "created": NOW_SECS }
                ]))
            }),
        )
        .route(
            "/v1/disputes",
            get(|| async {
                list(serde_json::json!([{
                    "id": "dp_01", "object": "dispute", "amount": CHARGED, "currency": "usd",
                    "charge": CHARGE, "payment_intent": INTENT, "status": "needs_response",
                    "reason": "fraudulent", "created": NOW_SECS - 600
                }]))
            }),
        )
        .route(
            "/v1/invoices",
            get(|| async {
                list(serde_json::json!([{
                    "id": "in_01", "object": "invoice", "status": "paid", "amount_paid": 1_900,
                    "currency": "usd", "customer": CUSTOMER, "created": NOW_SECS - 7_200,
                    "status_transitions": { "paid_at": NOW_SECS - 7_000 },
                    "subscription_details": { "metadata": { "org": ORG_SELLER.0.to_hyphenated() } }
                }]))
            }),
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

fn state(pool: PgPool, base: Option<&str>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            stripe_webhook_secret: Some(WebhookSecret::new(SECRET.to_owned())),
            stripe: base.map(|base| {
                stripe::Client::with_base(
                    SecretKey::new("sk_test_development_only".to_owned()),
                    base.to_owned(),
                )
            }),
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
    for (org, name) in [(ORG_OPERATOR, "Teachouse"), (ORG_SELLER, "Kauri Room")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (
            ORG_OPERATOR,
            USER_OPERATOR,
            "operator@example.test",
            TOKEN_OPERATOR,
        ),
        (ORG_SELLER, USER_SELLER, "seller@example.test", TOKEN_SELLER),
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
async fn call(
    state: AppState,
    token: Option<&SessionToken>,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let response = router(state)
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
async fn deliver(pool: &PgPool, event: &serde_json::Value) -> StatusCode {
    let body = event.to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header(header::CONTENT_TYPE, "application/json")
        .header("Stripe-Signature", sign(&body))
        .body(Body::from(body))
        .expect("the request builds");
    router(state(pool.clone(), None))
        .oneshot(request)
        .await
        .expect("the router serves")
        .status()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the fixtures read better built inline at the call"
)]
fn event(id: &str, kind: &str, object: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": id, "type": kind, "created": NOW_SECS, "data": { "object": object } })
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn ledger(pool: &PgPool) -> Vec<(String, String, String, Option<Uuid>)> {
    let rows: Vec<(String, String, String, Option<uuid::Uuid>)> = sqlx::query_as(
        "SELECT provider_event_id, kind, provider_object_id, org_id FROM payment_event \
         ORDER BY kind, provider_object_id, provider_event_id",
    )
    .fetch_all(pool)
    .await
    .expect("the ledger reads");
    rows.into_iter()
        .map(|(event, kind, object, org)| {
            (event, kind, object, org.map(|org| Uuid(*org.as_bytes())))
        })
        .collect()
}

async fn view(pool: &PgPool, base: &str) -> PaymentsAdminView {
    let (status, body) = call(
        state(pool.clone(), Some(base)),
        Some(&TOKEN_OPERATOR),
        Method::GET,
        "/v1/admin/payments",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    json(&body)
}

async fn refund(
    pool: &PgPool,
    base: &str,
    request: &str,
    amount: i64,
    send_email: bool,
) -> (StatusCode, Vec<u8>) {
    call(
        state(pool.clone(), Some(base)),
        Some(&TOKEN_OPERATOR),
        Method::POST,
        &format!("/v1/admin/payments/charges/{CHARGE}/refunds"),
        // The double's charge traces to no plan or pack, so the policy quotes
        // nothing and every amount here is an override that must say why.
        Some(serde_json::json!({
            "request_id": request, "amount_cents": amount, "reason": "requested_by_customer",
            "note": "Charged twice in September", "send_email": send_email,
            "issued_by_label": "Sam", "override_reason": "Charged twice in September"
        })),
    )
    .await
}

async fn set_auto(pool: &PgPool, on: bool) {
    let (status, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Method::PUT,
        "/v1/admin/payments/settings",
        Some(serde_json::json!({ "auto_refund_mail": on })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

// ------------------------------------------------------------------ webhook

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_redelivered_event_is_one_ledger_row(pool: PgPool) {
    provision(&pool).await;
    let paid = event("evt_charge_01", "charge.succeeded", charge_object(0));
    assert_eq!(deliver(&pool, &paid).await, StatusCode::OK);
    assert_eq!(deliver(&pool, &paid).await, StatusCode::OK);
    assert_eq!(
        ledger(&pool).await,
        vec![(
            "evt_charge_01".to_owned(),
            "payment_succeeded".to_owned(),
            CHARGE.to_owned(),
            Some(ORG_SELLER.0)
        )],
        "Stripe's retry of one event is one row, attributed by the charge's metadata"
    );

    // A dispute carries no metadata of ours, and inherits the organisation of
    // the charge it disputes.
    let disputed = event(
        "evt_dispute_01",
        "charge.dispute.created",
        serde_json::json!({ "id": "dp_01", "object": "dispute", "amount": CHARGED,
            "currency": "usd", "charge": CHARGE, "status": "needs_response",
            "reason": "fraudulent" }),
    );
    assert_eq!(deliver(&pool, &disputed).await, StatusCode::OK);
    let rows = ledger(&pool).await;
    assert_eq!(rows.len(), 2);
    assert!(rows.contains(&(
        "evt_dispute_01".to_owned(),
        "dispute_opened".to_owned(),
        "dp_01".to_owned(),
        Some(ORG_SELLER.0)
    )));

    // An event the ledger does not keep changes nothing and is acknowledged.
    let ignored = event(
        "evt_customer_01",
        "customer.updated",
        serde_json::json!({ "id": CUSTOMER }),
    );
    assert_eq!(deliver(&pool, &ignored).await, StatusCode::OK);
    assert_eq!(ledger(&pool).await.len(), 2);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refund_made_in_the_dashboard_mails_the_customer_only_when_the_setting_is_on(
    pool: PgPool,
) {
    provision(&pool).await;
    let paid = event("evt_charge_01", "charge.succeeded", charge_object(0));
    assert_eq!(deliver(&pool, &paid).await, StatusCode::OK);
    let dashboard = |id: &str, refund: &str| {
        event(
            id,
            "refund.created",
            serde_json::json!({ "id": refund, "object": "refund", "amount": 500,
                "currency": "usd", "charge": CHARGE, "status": "pending" }),
        )
    };

    assert_eq!(
        deliver(&pool, &dashboard("evt_refund_01", "re_quiet")).await,
        StatusCode::OK
    );
    set_auto(&pool, true).await;
    assert_eq!(
        deliver(&pool, &dashboard("evt_refund_02", "re_told")).await,
        StatusCode::OK
    );

    let refunds = PaymentRepo::new(pool.clone())
        .refunds()
        .await
        .expect("the refunds read");
    let queued: BTreeMap<String, bool> = refunds
        .iter()
        .map(|refund| {
            (
                refund.provider_refund_id.clone(),
                refund.mail_requested_at.is_some(),
            )
        })
        .collect();
    assert_eq!(
        queued,
        BTreeMap::from([("re_quiet".to_owned(), false), ("re_told".to_owned(), true)]),
        "only the refund that arrived with the switch on queues the customer's mail"
    );
    assert!(
        refunds
            .iter()
            .all(|refund| refund.org == Some(ORG_SELLER) && refund.issued_by.is_none()),
        "a dashboard refund inherits the charge's organisation and names no operator"
    );

    // Stripe settles it: refund.updated moves the status and nothing else.
    let settled = event(
        "evt_refund_03",
        "refund.updated",
        serde_json::json!({ "id": "re_told", "object": "refund", "amount": 500,
            "currency": "usd", "charge": CHARGE, "status": "succeeded" }),
    );
    assert_eq!(deliver(&pool, &settled).await, StatusCode::OK);
    let refunds = PaymentRepo::new(pool.clone())
        .refunds()
        .await
        .expect("the refunds read");
    assert_eq!(refunds.len(), 2, "an update is not a second refund");
    assert!(refunds
        .iter()
        .any(|refund| refund.provider_refund_id == "re_told"
            && refund.status == tam_storage::RefundStatus::Succeeded));
}

// --------------------------------------------------------------------- sync

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sync_upserts_and_gives_way_to_stripes_own_event(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    let sync = || {
        call(
            state(pool.clone(), Some(&base)),
            Some(&TOKEN_OPERATOR),
            Method::POST,
            "/v1/admin/payments/sync",
            None,
        )
    };
    let (status, body) = sync().await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let first: SyncView = json(&body);
    assert_eq!(
        first,
        SyncView {
            charges: 2,
            refunds: 1,
            disputes: 1,
            invoices: 1,
            recorded: 4
        },
        "the pending charge is read and not recorded"
    );
    let (_, body) = sync().await;
    let second: SyncView = json(&body);
    assert_eq!(second.recorded, 4, "a second sync refreshes the same rows");
    let rows = ledger(&pool).await;
    assert_eq!(
        rows.iter()
            .map(|(event, ..)| event.as_str())
            .collect::<Vec<_>>(),
        vec![
            "sync:dispute_opened:dp_01",
            "sync:invoice_paid:in_01",
            "sync:payment_succeeded:ch_01",
            "sync:refund_created:re_dash",
        ],
        "one row per object however often the sync runs"
    );
    assert!(
        rows.iter().all(|(.., org)| *org == Some(ORG_SELLER.0)),
        "the refund and dispute inherit the charge's organisation: {rows:?}"
    );
    let refunds = PaymentRepo::new(pool.clone())
        .refunds()
        .await
        .expect("the refunds read");
    assert_eq!(
        refunds.len(),
        1,
        "a synced refund lands in the refund table"
    );
    assert!(
        refunds
            .iter()
            .all(|refund| refund.mail_requested_at.is_none()),
        "history never mails anybody"
    );

    // Stripe's own event for the same charge replaces the sync's row.
    let paid = event("evt_charge_01", "charge.succeeded", charge_object(0));
    assert_eq!(deliver(&pool, &paid).await, StatusCode::OK);
    let charges: Vec<String> = ledger(&pool)
        .await
        .into_iter()
        .filter(|(_, kind, ..)| kind == "payment_succeeded")
        .map(|(event, ..)| event)
        .collect();
    assert_eq!(charges, vec!["evt_charge_01".to_owned()]);
    let (_, body) = sync().await;
    let third: SyncView = json(&body);
    assert_eq!(
        third.recorded, 3,
        "the sync leaves alone an object Stripe's event already describes"
    );
    assert_eq!(ledger(&pool).await.len(), 4);
}

// ------------------------------------------------------------------ refunds

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_refunds_part_of_a_charge_once(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = stripe_double().await;
    let (status, body) = refund(&pool, &base, REQUEST_A, 1_200, true).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let made: RefundView = json(&body);
    assert_eq!(made.id, REQUEST_A);
    assert_eq!(made.amount_cents, 1_200);
    assert_eq!(made.status, "pending");
    assert_eq!(made.org_name.as_deref(), Some("Kauri Room"));
    assert_eq!(made.issued_by.as_deref(), Some("Sam"));
    assert_eq!(made.note.as_deref(), Some("Charged twice in September"));
    assert_eq!(
        made.mail_requested_at,
        Some(NOW),
        "send_email queues the mail"
    );
    assert_eq!(made.mail_sent_at, None);

    // A retried request — the same id — answers the same refund and moves
    // no more money.
    let (status, body) = refund(&pool, &base, REQUEST_A, 1_200, true).await;
    assert_eq!(status, StatusCode::OK);
    let again: RefundView = json(&body);
    assert_eq!(again.provider_refund_id, made.provider_refund_id);
    let (calls, refunded) = {
        let held = double.lock().await;
        (held.refunds.clone(), held.refunded)
    };
    assert_eq!(calls.len(), 1, "Stripe was asked once");
    let (key, form) = calls.first().expect("one refund call");
    assert_eq!(key.as_deref(), Some(format!("refund-{REQUEST_A}").as_str()));
    assert_eq!(field(form, "charge"), Some(CHARGE));
    assert_eq!(field(form, "amount"), Some("1200"));
    assert_eq!(field(form, "reason"), Some("requested_by_customer"));
    assert_eq!(field(form, "metadata[refund]"), Some(REQUEST_A));
    assert_eq!(refunded, 1_200);

    // Who issued it is on the operators' trail.
    let trail: Vec<(String, uuid::Uuid)> = sqlx::query_as(
        "SELECT action, refund_id FROM platform_operator_event \
         WHERE user_id = $1 AND refund_id IS NOT NULL",
    )
    .bind(uuid::Uuid::from_bytes(USER_OPERATOR.0 .0))
    .fetch_all(&pool)
    .await
    .expect("the trail reads");
    assert_eq!(
        trail,
        vec![(
            "refund".to_owned(),
            uuid::Uuid::parse_str(REQUEST_A).expect("the fixture id parses")
        )]
    );
    assert_eq!(
        OperatorRepo::new(pool.clone())
            .events(USER_OPERATOR)
            .await
            .expect("the marking's trail reads")
            .len(),
        1,
        "the marking's own story is still one grant"
    );

    // The page lists it at once, against the charge it came from.
    let page = view(&pool, &base).await;
    assert_eq!(page.refunds.len(), 1);
    assert!(page.events.iter().any(|row| row.kind == "refund_created"
        && row.charge_id.as_deref() == Some(CHARGE)
        && row.org_name.as_deref() == Some("Kauri Room")));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refund_past_what_is_left_is_refused_with_the_sentence(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = stripe_double().await;
    let (status, body) = refund(&pool, &base, REQUEST_A, CHARGED + 1, false).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        String::from_utf8_lossy(&body).contains("You can refund at most $29.00 on this payment."),
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (status, _) = refund(&pool, &base, REQUEST_A, CHARGED, false).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the whole charge is refundable"
    );
    let (status, body) = refund(&pool, &base, REQUEST_B, 1, false).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        String::from_utf8_lossy(&body).contains("This payment has already been refunded in full."),
        "{}",
        String::from_utf8_lossy(&body)
    );
    let (status, _) = refund(&pool, &base, REQUEST_B, 0, false).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        double.lock().await.refunds.len(),
        1,
        "no refused request reached Stripe"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_mail_is_sent_once_and_the_page_says_when(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    let (status, body) = refund(&pool, &base, REQUEST_A, 500, false).await;
    assert_eq!(status, StatusCode::CREATED);
    let made: RefundView = json(&body);
    assert_eq!(made.mail_requested_at, None, "not asked for, not queued");

    let path = format!("/v1/admin/payments/refunds/{REQUEST_A}/mail");
    let mail = || {
        call(
            state(pool.clone(), Some(&base)),
            Some(&TOKEN_OPERATOR),
            Method::POST,
            &path,
            None,
        )
    };
    let (status, body) = mail().await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let queued: RefundView = json(&body);
    assert_eq!(queued.mail_requested_at, Some(NOW));

    // The drainer claims it and records the send.
    let repo = PaymentRepo::new(pool.clone());
    let claimed = repo
        .claim_mail(NOW, Timestamp(NOW.0 + 60_000))
        .await
        .expect("the claim runs")
        .expect("the queued mail is due");
    assert_eq!(claimed.amount_cents, 500);
    assert_eq!(claimed.org_name, "Kauri Room");
    assert_eq!(
        repo.claim_mail(NOW, Timestamp(NOW.0 + 60_000))
            .await
            .expect("the claim runs"),
        None,
        "a leased mail is not claimed twice"
    );
    repo.settle_mail(claimed.refund, &RefundMailOutcome::Sent, NOW)
        .await
        .expect("the send records");

    let page = view(&pool, &base).await;
    let listed = page.refunds.first().expect("the refund is listed");
    assert_eq!(listed.mail_sent_at, Some(NOW));
    let (status, body) = mail().await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        String::from_utf8_lossy(&body).contains("This email was already sent on"),
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_auto_setting_is_on_the_page_and_off_until_turned_on(pool: PgPool) {
    provision(&pool).await;
    let (base, _double) = stripe_double().await;
    assert!(!view(&pool, &base).await.auto_refund_mail);
    set_auto(&pool, true).await;
    let page = view(&pool, &base).await;
    assert!(page.auto_refund_mail);
    assert!(page.stripe_configured);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_is_refused_every_payments_route(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = stripe_double().await;
    for (method, path, body) in [
        (Method::GET, "/v1/admin/payments".to_owned(), None),
        (Method::POST, "/v1/admin/payments/sync".to_owned(), None),
        (
            Method::POST,
            format!("/v1/admin/payments/charges/{CHARGE}/refunds"),
            Some(
                serde_json::json!({ "request_id": REQUEST_A, "amount_cents": 100,
                "reason": "duplicate" }),
            ),
        ),
        (
            Method::POST,
            format!("/v1/admin/payments/refunds/{REQUEST_A}/mail"),
            None,
        ),
        (
            Method::PUT,
            "/v1/admin/payments/settings".to_owned(),
            Some(serde_json::json!({ "auto_refund_mail": true })),
        ),
    ] {
        for token in [Some(&TOKEN_SELLER), None] {
            let (status, _) = call(
                state(pool.clone(), Some(&base)),
                token,
                method.clone(),
                &path,
                body.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
    }
    assert!(double.lock().await.refunds.is_empty());
}
