//! Sales, one-off discounts and codes over the wire, against a loopback
//! Stripe double that records every call.
//!
//! What is proven here is the contract with Stripe as well as with our own
//! pages: the admin write mints the coupon (and promotion code) Stripe will
//! later honour, with the terms the operator typed; the price list announces
//! a sale only inside its window; and a checkout opens with exactly one of a
//! coupon, a promotion code, or Stripe's own code field — never two, which
//! Stripe refuses.

#![cfg(feature = "pg-tests")]

use std::sync::Arc;

use tokio::sync::Mutex;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    routing::{get, post},
    Form, Json,
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::pricing::{DiscountView, PricingAdminView};
use tam_api::{router, stripe, AppState, Config, PlansView, PriceMap, SecretKey, SESSION_COOKIE};
use tam_storage::{OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_OPERATOR: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_SELLER: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);

/// 2026-09-27T12:00Z, before the Halloween sale opens.
const BEFORE: Timestamp = Timestamp(1_790_510_400_000);
/// 2026-10-15T12:00Z, inside it.
const DURING: Timestamp = Timestamp(1_792_065_600_000);
/// 2026-11-01T12:00Z, after it.
const AFTER: Timestamp = Timestamp(1_793_534_400_000);
/// The last second the sale's coupon may be redeemed: 2026-10-31T23:59:59Z.
const SALE_REDEEM_BY: i64 = 1_793_491_199;

const PLAN_KEY: &str = "sync_monthly";
const PACK_KEY: &str = "pack_100";
const PLAN_PRICE: &str = "price_sync_monthly";
const PACK_PRICE: &str = "price_pack_100";

/// One request the double received: method, path, the form it carried and
/// the Stripe-Version it was pinned to.
#[derive(Debug, Clone)]
struct Seen {
    method: &'static str,
    path: String,
    form: Vec<(String, String)>,
    version: Option<String>,
}

type Log = Arc<Mutex<Vec<Seen>>>;

async fn record(log: &Log, seen: Seen) {
    log.lock().await.push(seen);
}

fn version(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Stripe-Version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn field<'a>(form: &'a [(String, String)], name: &str) -> Option<&'a str> {
    form.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// A Stripe double answering the coupon, promotion-code, price and checkout
/// calls this feature makes, and recording each one.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn stripe_double() -> (String, Log) {
    let log: Log = Arc::default();
    let app = axum::Router::new()
        .route(
            "/v1/coupons",
            post(
                |State(log): State<Log>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>| async move {
                    let id = field(&form, "id").unwrap_or("coupon").to_owned();
                    record(&log, Seen { method: "POST", path: "/v1/coupons".to_owned(), form, version: version(&headers) }).await;
                    Json(serde_json::json!({ "id": id, "object": "coupon", "valid": true, "times_redeemed": 0 }))
                },
            )
            .get(|State(log): State<Log>| async move {
                let created: Vec<serde_json::Value> = log
                    .lock()
                    .await
                    .iter()
                    .filter(|seen| seen.method == "POST" && seen.path == "/v1/coupons")
                    .filter_map(|seen| field(&seen.form, "id"))
                    .map(|id| serde_json::json!({ "id": id, "valid": true }))
                    .collect();
                Json(serde_json::json!({ "object": "list", "data": created, "has_more": false }))
            }),
        )
        .route(
            "/v1/coupons/{id}",
            axum::routing::delete(|State(log): State<Log>, Path(id): Path<String>| async move {
                record(&log, Seen { method: "DELETE", path: format!("/v1/coupons/{id}"), form: Vec::new(), version: None }).await;
                Json(serde_json::json!({ "id": id, "deleted": true }))
            }),
        )
        .route(
            "/v1/promotion_codes",
            post(
                |State(log): State<Log>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>| async move {
                    let code = field(&form, "code").unwrap_or("CODE").to_owned();
                    record(&log, Seen { method: "POST", path: "/v1/promotion_codes".to_owned(), form, version: version(&headers) }).await;
                    Json(serde_json::json!({ "id": format!("promo_{code}"), "code": code, "active": true }))
                },
            ),
        )
        .route(
            "/v1/promotion_codes/{id}",
            post(
                |State(log): State<Log>, Path(id): Path<String>, Form(form): Form<Vec<(String, String)>>| async move {
                    record(&log, Seen { method: "POST", path: format!("/v1/promotion_codes/{id}"), form, version: None }).await;
                    Json(serde_json::json!({ "id": id, "active": false }))
                },
            ),
        )
        .route(
            "/v1/prices/{id}",
            get(|Path(id): Path<String>| async move {
                Json(serde_json::json!({ "id": id, "object": "price", "product": format!("prod_{id}") }))
            }),
        )
        .route(
            "/v1/checkout/sessions",
            post(
                |State(log): State<Log>, Form(form): Form<Vec<(String, String)>>| async move {
                    record(&log, Seen { method: "POST", path: "/v1/checkout/sessions".to_owned(), form, version: None }).await;
                    Json(serde_json::json!({ "id": "cs_test_01", "url": "https://checkout.stripe.test/cs_test_01" }))
                },
            ),
        )
        .with_state(log.clone());
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
    (base, log)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn state(pool: PgPool, base: &str, wall: fn() -> Timestamp) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            stripe: Some(stripe::Client::with_base(
                SecretKey::new("sk_test_development_only".to_owned()),
                base.to_owned(),
            )),
            stripe_price_map: PriceMap::parse(&format!(
                r#"{{"{PLAN_PRICE}":"{PLAN_KEY}","{PACK_PRICE}":"{PACK_KEY}"}}"#
            ))
            .expect("the fixture price map parses"),
            ..Config::default()
        },
        wall,
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
    for (org, name) in [(ORG_OPERATOR, "org-operator"), (ORG_SELLER, "org-seller")] {
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
                Timestamp(AFTER.0 + 86_400_000),
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
) -> (StatusCode, HeaderMap, Vec<u8>) {
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
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    serde_json::from_slice(bytes).expect("the answer body parses")
}

fn halloween() -> serde_json::Value {
    serde_json::json!({
        "name": "Halloween 2026",
        "percent_off": 25,
        "from": "2026-10-01",
        "until": "2026-10-31",
        "banner": "Halloween sale: 25% off every plan until 31 October",
        "theme": "halloween"
    })
}

async fn seen(log: &Log, method: &str, path: &str) -> Vec<Seen> {
    log.lock()
        .await
        .iter()
        .filter(|seen| seen.method == method && seen.path == path)
        .cloned()
        .collect()
}

async fn create_sale(pool: &PgPool, base: &str) -> DiscountView {
    let (status, _headers, body) = call(
        state(pool.clone(), base, || BEFORE),
        Some(&TOKEN_OPERATOR),
        Method::POST,
        "/v1/admin/pricing/sales",
        Some(halloween()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    json(&body)
}

async fn plans(pool: &PgPool, base: &str, wall: fn() -> Timestamp) -> (HeaderMap, PlansView) {
    let (status, headers, body) = call(
        state(pool.clone(), base, wall),
        None,
        Method::GET,
        "/v1/plans",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    (headers, json(&body))
}

async fn checkout(
    pool: &PgPool,
    base: &str,
    price_key: &str,
    code: Option<&str>,
) -> (StatusCode, Vec<u8>) {
    let (status, _headers, body) = call(
        state(pool.clone(), base, || DURING),
        Some(&TOKEN_SELLER),
        Method::POST,
        "/v1/billing/checkout",
        Some(serde_json::json!({ "price_key": price_key, "code": code })),
    )
    .await;
    (status, body)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn last_checkout(log: &Log) -> Vec<(String, String)> {
    seen(log, "POST", "/v1/checkout/sessions")
        .await
        .pop()
        .expect("a checkout session was opened")
        .form
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_saved_sale_mints_its_coupon_with_the_terms_typed(pool: PgPool) {
    provision(&pool).await;
    let (base, log) = stripe_double().await;
    let sale = create_sale(&pool, &base).await;

    let coupons = seen(&log, "POST", "/v1/coupons").await;
    assert_eq!(coupons.len(), 1, "one coupon per sale");
    let form = &coupons[0].form;
    assert_eq!(field(form, "id"), Some(sale.stripe_coupon_id.as_str()));
    assert_eq!(field(form, "percent_off"), Some("25"));
    assert_eq!(field(form, "duration"), Some("once"));
    assert_eq!(
        field(form, "redeem_by"),
        Some(SALE_REDEEM_BY.to_string().as_str()),
        "the coupon stops being redeemable at the end of the last day"
    );
    assert!(
        form.iter().all(|(key, _)| !key.starts_with("applies_to")),
        "a sale is applied by our checkout alone and needs no product restriction"
    );
    assert_eq!(sale.from, "2026-10-01");
    assert_eq!(sale.until, "2026-10-31");
    assert_eq!(sale.theme.as_deref(), Some("halloween"));

    // The admin read shows it back with its banner, and Stripe still holding it.
    let (status, _headers, body) = call(
        state(pool.clone(), &base, || BEFORE),
        Some(&TOKEN_OPERATOR),
        Method::GET,
        "/v1/admin/pricing",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: PricingAdminView = json(&body);
    assert_eq!(view.discounts.len(), 1);
    assert_eq!(
        view.discounts[0].banner.as_deref(),
        Some("Halloween sale: 25% off every plan until 31 October")
    );
    assert_eq!(view.discounts[0].in_stripe, Some(true));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_price_list_announces_a_sale_only_inside_its_window(pool: PgPool) {
    provision(&pool).await;
    let (base, _log) = stripe_double().await;
    create_sale(&pool, &base).await;

    let (_headers, before) = plans(&pool, &base, || BEFORE).await;
    assert_eq!(before.sale, None, "scheduled is not open");

    let (headers, during) = plans(&pool, &base, || DURING).await;
    let sale = during.sale.expect("the sale is announced while open");
    assert_eq!(sale.percent_off, 25);
    assert_eq!(sale.until, "2026-10-31");
    assert_eq!(
        sale.banner,
        "Halloween sale: 25% off every plan until 31 October"
    );
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("public, max-age=60")
    );

    let (_headers, after) = plans(&pool, &base, || AFTER).await;
    assert_eq!(after.sale, None, "the sale closes after its last day");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_checkout_during_a_sale_carries_its_coupon_on_plans_only(pool: PgPool) {
    provision(&pool).await;
    let (base, log) = stripe_double().await;
    let sale = create_sale(&pool, &base).await;

    let (status, body) = checkout(&pool, &base, PLAN_KEY, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let form = last_checkout(&log).await;
    assert_eq!(
        field(&form, "discounts[0][coupon]"),
        Some(sale.stripe_coupon_id.as_str())
    );
    assert_eq!(
        field(&form, "allow_promotion_codes"),
        None,
        "Stripe refuses a session with both a discount and the code field"
    );

    let (status, _body) = checkout(&pool, &base, PACK_KEY, None).await;
    assert_eq!(status, StatusCode::OK);
    let form = last_checkout(&log).await;
    assert_eq!(
        field(&form, "discounts[0][coupon]"),
        None,
        "a pack is not a plan"
    );
    assert_eq!(field(&form, "allow_promotion_codes"), Some("true"));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_typed_code_reaches_only_the_prices_it_names(pool: PgPool) {
    provision(&pool).await;
    let (base, log) = stripe_double().await;
    let (status, _headers, body) = call(
        state(pool.clone(), &base, || BEFORE),
        Some(&TOKEN_OPERATOR),
        Method::POST,
        "/v1/admin/pricing/codes",
        Some(serde_json::json!({
            "code": "PACKS20",
            "name": "Packs, twenty off",
            "percent_off": 20,
            "price_keys": [PACK_KEY],
            "from": "2026-10-01",
            "until": "2026-10-31",
            "max_redemptions": 50
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let created: DiscountView = json(&body);

    let coupon = seen(&log, "POST", "/v1/coupons").await.remove(0).form;
    assert_eq!(
        field(&coupon, "applies_to[products][0]"),
        Some(format!("prod_{PACK_PRICE}").as_str()),
        "a typed coupon is restricted in Stripe too, because Stripe's own field skips our check"
    );
    let promotion = seen(&log, "POST", "/v1/promotion_codes").await.remove(0);
    assert_eq!(field(&promotion.form, "promotion[type]"), Some("coupon"));
    assert_eq!(
        field(&promotion.form, "promotion[coupon]"),
        Some(created.stripe_coupon_id.as_str())
    );
    assert_eq!(field(&promotion.form, "code"), Some("PACKS20"));
    assert_eq!(field(&promotion.form, "max_redemptions"), Some("50"));
    assert_eq!(
        promotion.version.as_deref(),
        Some(stripe::PROMOTION_CODE_API_VERSION),
        "the promotion-code shape is pinned to the version it was written for"
    );

    // Typed in any case, it opens the pack checkout with the promotion code.
    let (status, body) = checkout(&pool, &base, PACK_KEY, Some("packs20")).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let form = last_checkout(&log).await;
    assert_eq!(
        field(&form, "discounts[0][promotion_code]"),
        Some("promo_PACKS20")
    );
    assert_eq!(field(&form, "allow_promotion_codes"), None);

    // It does not reach a plan, and an unknown code is refused the same way.
    for (key, code) in [(PLAN_KEY, "PACKS20"), (PACK_KEY, "NOPE123")] {
        let opened = seen(&log, "POST", "/v1/checkout/sessions").await.len();
        let (status, _body) = checkout(&pool, &base, key, Some(code)).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{key} with {code}"
        );
        assert_eq!(
            seen(&log, "POST", "/v1/checkout/sessions").await.len(),
            opened,
            "a refused code opens no session"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn overlapping_sales_are_refused_and_ending_one_withdraws_its_coupon(pool: PgPool) {
    provision(&pool).await;
    let (base, log) = stripe_double().await;
    let sale = create_sale(&pool, &base).await;

    let mut overlapping = halloween();
    overlapping["from"] = "2026-10-20".into();
    overlapping["until"] = "2026-11-05".into();
    let (status, _headers, _body) = call(
        state(pool.clone(), &base, || BEFORE),
        Some(&TOKEN_OPERATOR),
        Method::POST,
        "/v1/admin/pricing/sales",
        Some(overlapping),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        seen(&log, "POST", "/v1/coupons").await.len(),
        1,
        "a refused sale mints nothing"
    );

    let (status, _headers, body) = call(
        state(pool.clone(), &base, || DURING),
        Some(&TOKEN_OPERATOR),
        Method::POST,
        &format!("/v1/admin/pricing/{}/end", sale.id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        seen(
            &log,
            "DELETE",
            &format!("/v1/coupons/{}", sale.stripe_coupon_id)
        )
        .await
        .len(),
        1
    );
    let (_headers, during) = plans(&pool, &base, || DURING).await;
    assert_eq!(during.sale, None, "an ended sale is announced nowhere");
    let (status, _body) = checkout(&pool, &base, PLAN_KEY, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        field(&last_checkout(&log).await, "discounts[0][coupon]"),
        None
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_cannot_reach_the_pricing_surface(pool: PgPool) {
    provision(&pool).await;
    let (base, log) = stripe_double().await;
    let (status, _headers, _body) = call(
        state(pool.clone(), &base, || BEFORE),
        Some(&TOKEN_SELLER),
        Method::POST,
        "/v1/admin/pricing/sales",
        Some(halloween()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(seen(&log, "POST", "/v1/coupons").await.is_empty());
}
