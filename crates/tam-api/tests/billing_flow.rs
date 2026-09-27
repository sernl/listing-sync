//! Billing over the wire: a correctly signed Stripe event driven into the
//! router, and the org-scoped read that shows what it changed.
//!
//! The webhook carries no session and names no organisation in its path, so
//! the tenancy statement provable at this surface is the one asserted here:
//! the organisation is the one the signed payload names, and a second
//! tenant's row is untouched by it. The signature is constructed in-process
//! from the same secret the router is configured with, which is what makes an
//! end-to-end test possible with no Stripe account in existence.
//!
//! One socket does open: `checkout.session.completed` is fulfilled by
//! re-reading the session with its line items expanded, which is Stripe's
//! own guidance, and the billing page's card, invoices, cancel and resume
//! read and write the subscription. The double below answers those calls on
//! a loopback port, so the fulfilment branch — the branch that credits a
//! seller's moves — and the cancellation round trip are exercised rather
//! than stubbed past.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    extract::{Path, Query},
    http::{header, Request, StatusCode},
    routing::get,
    Form, Json,
};
use core::fmt::Write as _;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::{
    router, stripe, AppState, BillingView, Config, InvoicesView, PaymentMethodView, PriceMap,
    SecretKey, WebhookSecret, ORG_METADATA_KEY, SESSION_COOKIE,
};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);

const SECRET: &str = "whsec_01_development_only";
/// The wall instant every request in this file is served at, and the instant
/// the signatures are stamped with, so the tolerance window is exercised by
/// choosing a timestamp rather than by waiting.
const NOW_SECS: i64 = 1_800_000_000;
const NOW: Timestamp = Timestamp(NOW_SECS * 1_000);

/// The period every subscription fixture renews at: a fortnight past `NOW`.
const PERIOD_END_SECS: i64 = NOW_SECS + 14 * 86_400;

const PACK_PRICE: &str = "price_pack_100";
const MONTHLY_PRICE: &str = "price_sync_monthly";
const STARTER_PRICE: &str = "price_starter_yearly";
const STUDIO_PRICE: &str = "price_studio_monthly";
const PACK_SESSION: &str = "cs_pack_01";
const SUBSCRIPTION_SESSION: &str = "cs_sub_01";
const STARTER_SESSION: &str = "cs_starter_01";
const SUBSCRIPTION: &str = "sub_01";
const CUSTOMER: &str = "cus_01";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn price_map() -> PriceMap {
    PriceMap::parse(&format!(
        r#"{{"{PACK_PRICE}":"pack_100","{MONTHLY_PRICE}":"sync_monthly","{STARTER_PRICE}":"starter_yearly","{STUDIO_PRICE}":"studio_monthly"}}"#
    ))
    .expect("the fixture price map parses")
}

/// A Stripe double answering the outbound calls this file's routes make.
///
/// It serves every session this file posts, keyed by the identifier in the
/// path, with the line items the real API would return for an expanded read;
/// the one subscription, whose `cancel_at_period_end` the update call flips
/// and the read reports, with its card expanded; and the customer's invoice
/// list, a draft among them. Nothing else is implemented, because nothing
/// else is called.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn stripe_double() -> String {
    let app = axum::Router::new().route(
        "/v1/checkout/sessions/{id}",
        get(|Path(id): Path<String>| async move {
            let (mode, price, subscription) = match id.as_str() {
                SUBSCRIPTION_SESSION => ("subscription", MONTHLY_PRICE, Some(SUBSCRIPTION)),
                STARTER_SESSION => ("subscription", STARTER_PRICE, Some(SUBSCRIPTION)),
                _pack => ("payment", PACK_PRICE, None),
            };
            Json(serde_json::json!({
                "id": id,
                "object": "checkout.session",
                "mode": mode,
                "payment_status": "paid",
                "customer": CUSTOMER,
                "subscription": subscription,
                "client_reference_id": ORG_A.0.to_hyphenated(),
                "metadata": { ORG_METADATA_KEY: ORG_A.0.to_hyphenated() },
                "line_items": {
                    "object": "list",
                    "data": [{
                        "id": "li_01",
                        "quantity": 1,
                        "price": { "id": price, "object": "price" }
                    }]
                }
            }))
        }),
    );
    let cancelling = Arc::new(AtomicBool::new(false));
    let subscription = |cancelling: bool| {
        serde_json::json!({
            "id": SUBSCRIPTION,
            "object": "subscription",
            "status": "active",
            "customer": CUSTOMER,
            "cancel_at_period_end": cancelling,
            "cancel_at": null,
            "items": { "object": "list", "data": [{ "current_period_end": PERIOD_END_SECS }] },
            "default_payment_method": {
                "id": "pm_01",
                "object": "payment_method",
                "card": { "brand": "visa", "last4": "3115", "exp_month": 8, "exp_year": 2029 }
            }
        })
    };
    let read = Arc::clone(&cancelling);
    let write = Arc::clone(&cancelling);
    let app = app
        .route(
            "/v1/subscriptions/{id}",
            get(move |Path(_id): Path<String>| {
                let read = Arc::clone(&read);
                async move { Json(subscription(read.load(Ordering::SeqCst))) }
            })
            .post(
                move |Path(_id): Path<String>, Form(form): Form<BTreeMap<String, String>>| {
                    let write = Arc::clone(&write);
                    async move {
                        let flag = form.get("cancel_at_period_end").map(String::as_str) == Some("true");
                        write.store(flag, Ordering::SeqCst);
                        Json(subscription(flag))
                    }
                },
            ),
        )
        .route(
            "/v1/invoices",
            get(|Query(query): Query<BTreeMap<String, String>>| async move {
                let mine = query.get("customer").map(String::as_str) == Some(CUSTOMER);
                let invoices = if mine {
                    serde_json::json!([
                        {
                            "id": "in_draft", "object": "invoice", "created": NOW_SECS + 60,
                            "total": 2900, "currency": "nzd", "status": "draft"
                        },
                        {
                            "id": "in_01", "object": "invoice", "created": NOW_SECS,
                            "total": 34783, "currency": "nzd", "status": "paid",
                            "hosted_invoice_url": "https://invoice.stripe.com/i/in_01",
                            "invoice_pdf": "https://pay.stripe.com/invoice/in_01/pdf",
                            "lines": { "object": "list", "data": [{ "description": "1 × Sync (monthly)" }] }
                        }
                    ])
                } else {
                    serde_json::json!([])
                };
                Json(serde_json::json!({ "object": "list", "data": invoices, "has_more": false }))
            }),
        );
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
    base
}

fn state(pool: PgPool, secret: Option<&str>, base: Option<&str>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            stripe_webhook_secret: secret.map(|raw| WebhookSecret::new(raw.to_owned())),
            stripe: base.map(|base| {
                stripe::Client::with_base(
                    SecretKey::new("sk_test_development_only".to_owned()),
                    base.to_owned(),
                )
            }),
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
fn sign(ts: i64, body: &str, secret: &str) -> String {
    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts a key of any length");
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let mut digest = String::new();
    for byte in mac.finalize().into_bytes() {
        write!(digest, "{byte:02x}").expect("writing to a String is infallible");
    }
    format!("t={ts},v1={digest}")
}

/// A completed Checkout Session as Stripe delivers it: no line items, which
/// is why the handler re-reads the session.
fn checkout_completed(org: OrgId, session: &str, mode: &str) -> String {
    serde_json::json!({
        "id": "evt_checkout_01",
        "type": "checkout.session.completed",
        "created": NOW_SECS,
        "data": { "object": {
            "id": session,
            "object": "checkout.session",
            "mode": mode,
            "payment_status": "paid",
            "customer": CUSTOMER,
            "subscription": if mode == "subscription" { Some(SUBSCRIPTION) } else { None },
            "client_reference_id": org.0.to_hyphenated(),
            "metadata": { ORG_METADATA_KEY: org.0.to_hyphenated() }
        }}
    })
    .to_string()
}

fn invoice(org: OrgId, event: &str) -> String {
    serde_json::json!({
        "id": "evt_invoice_01",
        "type": event,
        "created": NOW_SECS,
        "data": { "object": {
            "id": "in_01",
            "object": "invoice",
            "customer": CUSTOMER,
            "subscription": SUBSCRIPTION,
            "period_start": NOW_SECS,
            "period_end": PERIOD_END_SECS,
            "subscription_details": { "metadata": { ORG_METADATA_KEY: org.0.to_hyphenated() } },
            "lines": { "object": "list", "data": [{
                "id": "il_01",
                "price": { "id": MONTHLY_PRICE, "object": "price" },
                "period": { "start": NOW_SECS, "end": PERIOD_END_SECS }
            }]}
        }}
    })
    .to_string()
}

fn subscription_event(org: OrgId, event: &str, status: &str, price: &str) -> String {
    serde_json::json!({
        "id": "evt_subscription_01",
        "type": event,
        "created": NOW_SECS,
        "data": { "object": {
            "id": SUBSCRIPTION,
            "object": "subscription",
            "customer": CUSTOMER,
            "status": status,
            "current_period_end": PERIOD_END_SECS,
            "metadata": { ORG_METADATA_KEY: org.0.to_hyphenated() },
            "items": { "object": "list", "data": [{
                "id": "si_01",
                "current_period_end": PERIOD_END_SECS,
                "price": { "id": price, "object": "price" }
            }]}
        }}
    })
    .to_string()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the fixture org inserts");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_A, UserId(Uuid([0x0A; 16])), "a@example.test", TOKEN_A),
        (ORG_B, UserId(Uuid([0x0B; 16])), "b@example.test", TOKEN_B),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, Timestamp(NOW.0 + 100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn post_webhook(
    pool: PgPool,
    base: Option<&str>,
    signature: Option<&str>,
    body: &str,
) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(signature) = signature {
        request = request.header("Stripe-Signature", signature);
    }
    let request = request
        .body(Body::from(body.to_owned()))
        .expect("the request builds");
    router(state(pool, Some(SECRET), base))
        .oneshot(request)
        .await
        .expect("the router serves")
        .status()
}

/// The signed form of one event, posted with the double reachable.
async fn deliver(pool: &PgPool, base: &str, body: &str) -> StatusCode {
    post_webhook(
        pool.clone(),
        Some(base),
        Some(&sign(NOW_SECS, body, SECRET)),
        body,
    )
    .await
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn read_billing(pool: PgPool, token: &SessionToken) -> BillingView {
    let request = Request::builder()
        .uri("/v1/billing")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    // Configured with a client, because `portal_available` says whether this
    // deployment holds a Stripe key as well as whether the tenant holds a
    // customer. No outbound call is made by the read itself.
    let base = "http://127.0.0.1:1";
    let response = router(state(pool, Some(SECRET), Some(base)))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the billing read answers the session that asked"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    serde_json::from_slice(&body).expect("the answer is the billing shape")
}

/// One session-authenticated call to a billing route with the double
/// reachable, answering the status and the body's bytes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: &PgPool,
    base: &str,
    method: &str,
    uri: &str,
    token: &SessionToken,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    if method == "POST" {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let body = if method == "POST" { "{}" } else { "" };
    let response = router(state(pool.clone(), Some(SECRET), Some(base)))
        .oneshot(request.body(Body::from(body)).expect("the request builds"))
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

/// A subscriber on the monthly price, renewing at `PERIOD_END_SECS`.
async fn subscribe(pool: &PgPool, base: &str) {
    assert_eq!(
        deliver(
            pool,
            base,
            &checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription")
        )
        .await,
        StatusCode::OK,
        "the subscription checkout is accepted"
    );
    assert_eq!(
        deliver(pool, base, &invoice(ORG_A, "invoice.paid")).await,
        StatusCode::OK,
        "the first invoice is accepted"
    );
}

/// One pinned read of a fenced table.
///
/// Every table this file inspects directly forces row-level security, and
/// the migration role is not exempt from it, so an unpinned statement here
/// matches nothing and reads as an absent row rather than as a refusal. The
/// pin is transaction-local, which is the same shape every repository read
/// takes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn pinned_scalar<T>(pool: &PgPool, org: OrgId, sql: &str, bind: &str) -> T
where
    for<'r> T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    let mut tx = pool.begin().await.expect("the pinned read opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(org.0.to_hyphenated())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin applies");
    let value: T = sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(&mut *tx)
        .await
        .expect("the pinned read answers");
    tx.commit().await.expect("the pinned read closes");
    value
}

/// When one live grant stops entitling, in epoch seconds.
///
/// Read as epoch seconds rather than as a date type: this crate holds no
/// date library, and the assertion is about an instant either way.
async fn grant_expiry(pool: &PgPool, source_ref: &str) -> Option<i64> {
    pinned_scalar(
        pool,
        ORG_A,
        "SELECT extract(epoch FROM expires_at)::bigint FROM entitlement_grant \
          WHERE source_ref = $1 AND revoked_at IS NULL",
        source_ref,
    )
    .await
}

// -------------------------------------------------------- the five events

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_paid_pack_becomes_moves_the_organisation_can_spend(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    let body = checkout_completed(ORG_A, PACK_SESSION, "payment");
    assert_eq!(
        deliver(&pool, &base, &body).await,
        StatusCode::OK,
        "Stripe is acknowledged"
    );

    let view = read_billing(pool, &TOKEN_A).await;
    assert_eq!(
        view.moves.available, 100,
        "the pack the expanded line item named is what landed in the balance"
    );
    assert!(
        view.moves.expiring_soonest.is_some(),
        "a pack's moves carry the twelve-month expiry the pricing decision gives them"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_redelivered_pack_purchase_credits_once(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    let body = checkout_completed(ORG_A, PACK_SESSION, "payment");
    for _delivery in 0..3 {
        assert_eq!(deliver(&pool, &base, &body).await, StatusCode::OK);
    }
    assert_eq!(
        read_billing(pool, &TOKEN_A).await.moves.available,
        100,
        "Stripe retries a delivery it did not hear 200 for, and a retried \
         purchase must not pay out twice"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_completed_subscription_checkout_entitles_and_accrues(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    let body = checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription");
    assert_eq!(deliver(&pool, &base, &body).await, StatusCode::OK);

    let view = read_billing(pool, &TOKEN_A).await;
    assert_eq!(
        view.plan.as_str(),
        "subscriber",
        "the plan crossed the whole path from the signed bytes to the read"
    );
    assert_eq!(
        view.cadence,
        Some(tam_api::Cadence::Monthly),
        "the price the session named is what says how often it renews"
    );
    assert_eq!(
        view.moves.available, 25,
        "the first period's moves accrue with the subscription rather than \
         waiting for the first renewal"
    );
    assert!(
        view.portal_available,
        "a tenant with a Stripe customer can be sent to the portal"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_paid_invoice_moves_the_renewal_forward(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    assert_eq!(
        deliver(
            &pool,
            &base,
            &checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription")
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        deliver(&pool, &base, &invoice(ORG_A, "invoice.paid")).await,
        StatusCode::OK
    );

    let view = read_billing(pool, &TOKEN_A).await;
    assert_eq!(
        view.renews_at,
        Some(Timestamp(PERIOD_END_SECS * 1_000)),
        "the period the invoice covered is the one the page states"
    );
    assert_eq!(
        view.moves.available, 25,
        "one period's accrual is one period's moves: the session and the \
         invoice describe the same period and must not accrue twice"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_failed_payment_keeps_the_seller_working_for_a_day(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    deliver(
        &pool,
        &base,
        &checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription"),
    )
    .await;
    assert_eq!(
        deliver(&pool, &base, &invoice(ORG_A, "invoice.payment_failed")).await,
        StatusCode::OK,
        "a dunning notice is acknowledged"
    );
    assert_eq!(
        read_billing(pool, &TOKEN_A).await.plan.as_str(),
        "subscriber",
        "Stripe retries a failed card for weeks and most retries succeed; \
         the first failure must not lock a paying seller out"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_deleted_subscription_stops_entitling_at_the_period_end(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    deliver(
        &pool,
        &base,
        &checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription"),
    )
    .await;
    assert_eq!(
        deliver(
            &pool,
            &base,
            &subscription_event(
                ORG_A,
                "customer.subscription.deleted",
                "canceled",
                MONTHLY_PRICE
            )
        )
        .await,
        StatusCode::OK
    );

    let view = read_billing(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        view.plan.as_str(),
        "subscriber",
        "a cancellation runs to the period end rather than taking the plan \
         away the moment Stripe says so"
    );
    assert_eq!(
        grant_expiry(&pool, SUBSCRIPTION).await,
        Some(PERIOD_END_SECS),
        "the period end without the grace: a cancellation is the seller's own \
         decision, and a day past it is a day nobody asked for"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_updated_subscription_keeps_a_day_of_grace_past_the_period(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    deliver(
        &pool,
        &base,
        &checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription"),
    )
    .await;
    assert_eq!(
        deliver(
            &pool,
            &base,
            &subscription_event(
                ORG_A,
                "customer.subscription.updated",
                "active",
                MONTHLY_PRICE
            )
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        grant_expiry(&pool, SUBSCRIPTION).await,
        Some(PERIOD_END_SECS + 24 * 3_600),
        "a renewal event arriving late must not take a paying seller's plan \
         away between the period ending and the webhook landing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn each_tier_grants_its_own_plan_and_its_own_moves(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    assert_eq!(
        deliver(
            &pool,
            &base,
            &checkout_completed(ORG_A, STARTER_SESSION, "subscription")
        )
        .await,
        StatusCode::OK
    );
    let view = read_billing(pool, &TOKEN_A).await;
    assert_eq!(
        view.plan.as_str(),
        "starter",
        "the Starter price grants Starter, not the plan every subscription used to grant"
    );
    assert_eq!(view.cadence, Some(tam_api::Cadence::Yearly));
    assert_eq!(
        view.moves.available, 10,
        "the first period accrues Starter's allowance, not Sync's"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_price_change_in_the_portal_moves_the_grant_to_the_new_tier(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    deliver(
        &pool,
        &base,
        &checkout_completed(ORG_A, SUBSCRIPTION_SESSION, "subscription"),
    )
    .await;
    for _delivery in 0..2 {
        assert_eq!(
            deliver(
                &pool,
                &base,
                &subscription_event(
                    ORG_A,
                    "customer.subscription.updated",
                    "active",
                    STUDIO_PRICE
                )
            )
            .await,
            StatusCode::OK
        );
    }
    let view = read_billing(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        view.plan.as_str(),
        "studio",
        "switching Sync to Studio in the billing portal changes the plan the seller holds"
    );
    let live: i64 = pinned_scalar(
        &pool,
        ORG_A,
        "SELECT count(*) FROM entitlement_grant WHERE source_ref = $1 AND revoked_at IS NULL",
        SUBSCRIPTION,
    )
    .await;
    assert_eq!(
        live, 1,
        "one live grant per subscription, however often the switch is redelivered"
    );
    assert_eq!(
        grant_expiry(&pool, SUBSCRIPTION).await,
        Some(PERIOD_END_SECS + 24 * 3_600),
        "the new tier's grant runs to the same period end, with the same grace"
    );
}

// ----------------------------------------------------------- the boundary

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_event_for_one_organisation_leaves_the_other_with_nothing(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    deliver(
        &pool,
        &base,
        &checkout_completed(ORG_A, PACK_SESSION, "payment"),
    )
    .await;
    let view = read_billing(pool, &TOKEN_B).await;
    assert_eq!(
        view.moves.available, 0,
        "the organisation the payload did not name reads the honest zero"
    );
    assert!(
        !view.portal_available,
        "and has no customer to manage, so the portal is not offered"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unsigned_body_changes_nothing(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    let body = checkout_completed(ORG_A, PACK_SESSION, "payment");
    assert_eq!(
        post_webhook(pool.clone(), Some(&base), None, &body).await,
        StatusCode::UNAUTHORIZED,
        "the signature is the whole authentication of this route"
    );
    assert_eq!(
        post_webhook(
            pool.clone(),
            Some(&base),
            Some(&sign(NOW_SECS, &body, "whsec_somebody_elses_secret")),
            &body
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "and a digest under another secret is not this endpoint's signature"
    );
    assert_eq!(
        post_webhook(
            pool.clone(),
            Some(&base),
            Some(&sign(
                NOW_SECS - stripe::SIGNATURE_TOLERANCE_SECS - 1,
                &body,
                SECRET
            )),
            &body
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "a captured signature stops being useful once the window closes"
    );
    let tampered = body.replace(PACK_SESSION, "cs_somebody_elses_session");
    assert_eq!(
        post_webhook(
            pool.clone(),
            Some(&base),
            Some(&sign(NOW_SECS, &body, SECRET)),
            &tampered
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "the digest covers the body, so one changed byte refuses"
    );
    assert_eq!(
        read_billing(pool, &TOKEN_A).await.moves.available,
        0,
        "and none of the four wrote anything"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_endpoint_with_no_secret_refuses_rather_than_trusting_the_caller(pool: PgPool) {
    provision(&pool).await;
    let body = checkout_completed(ORG_A, PACK_SESSION, "payment");
    let request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header(header::CONTENT_TYPE, "application/json")
        .header("Stripe-Signature", sign(NOW_SECS, &body, SECRET))
        .body(Body::from(body))
        .expect("the request builds");
    let status = router(state(pool, None, None))
        .oneshot(request)
        .await
        .expect("the router serves")
        .status();
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "there is no unauthenticated mode of this route to fall back to"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_event_type_this_route_does_not_handle_is_acknowledged(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    let body = serde_json::json!({
        "id": "evt_other_01",
        "type": "payment_intent.succeeded",
        "created": NOW_SECS,
        "data": { "object": { "id": "pi_01" } }
    })
    .to_string();
    assert_eq!(
        deliver(&pool, &base, &body).await,
        StatusCode::OK,
        "the endpoint may be subscribed to more than it handles, and an \
         unhandled delivery is ordinary rather than a fault"
    );
}

// ------------------------------------------------------ the billing page

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cancelled_plan_runs_to_the_period_end_and_can_be_kept(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    subscribe(&pool, &base).await;

    let (status, body) = call(&pool, &base, "POST", "/v1/billing/cancel", &TOKEN_A).await;
    assert_eq!(status, StatusCode::OK);
    let answered: BillingView = serde_json::from_slice(&body).expect("the billing shape");
    assert!(
        answered.cancel_at_period_end,
        "the answer is the page as it now stands"
    );

    let view = read_billing(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        view.plan.as_str(),
        "subscriber",
        "cancelling ends the plan at the period's close, not now"
    );
    assert_eq!(
        (view.renews_at, view.ends_at),
        (None, Some(Timestamp(PERIOD_END_SECS * 1_000))),
        "a cancelled plan says when it ends and never that it renews"
    );
    assert_eq!(
        grant_expiry(&pool, SUBSCRIPTION).await,
        Some(PERIOD_END_SECS + 24 * 3_600),
        "the paid period and its grace are untouched by the cancel request"
    );

    let (status, body) = call(&pool, &base, "POST", "/v1/billing/resume", &TOKEN_A).await;
    assert_eq!(status, StatusCode::OK);
    let kept: BillingView = serde_json::from_slice(&body).expect("the billing shape");
    assert!(!kept.cancel_at_period_end);
    assert_eq!(
        (kept.renews_at, kept.ends_at),
        (Some(Timestamp(PERIOD_END_SECS * 1_000)), None),
        "a kept plan renews on the date it would have"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cancellation_made_in_the_portal_reaches_the_page(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    subscribe(&pool, &base).await;

    let mut event: serde_json::Value = serde_json::from_str(&subscription_event(
        ORG_A,
        "customer.subscription.updated",
        "active",
        MONTHLY_PRICE,
    ))
    .expect("the fixture reads");
    event["data"]["object"]["cancel_at_period_end"] = serde_json::json!(true);
    assert_eq!(
        deliver(&pool, &base, &event.to_string()).await,
        StatusCode::OK
    );
    let view = read_billing(pool.clone(), &TOKEN_A).await;
    assert!(view.cancel_at_period_end);
    assert_eq!(view.ends_at, Some(Timestamp(PERIOD_END_SECS * 1_000)));

    // The renewal invoice that follows a portal "undo" says nothing about the
    // instruction; only the subscription event may clear it.
    assert_eq!(
        deliver(&pool, &base, &invoice(ORG_A, "invoice.paid")).await,
        StatusCode::OK
    );
    assert!(
        read_billing(pool, &TOKEN_A).await.cancel_at_period_end,
        "an invoice must not quietly renew a plan the seller cancelled"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_page_lists_bills_and_names_the_card(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    subscribe(&pool, &base).await;

    let (status, body) = call(&pool, &base, "GET", "/v1/billing/invoices", &TOKEN_A).await;
    assert_eq!(status, StatusCode::OK);
    let listed: InvoicesView = serde_json::from_slice(&body).expect("the invoices shape");
    assert_eq!(
        listed
            .invoices
            .iter()
            .map(|invoice| invoice.id.as_str())
            .collect::<Vec<_>>(),
        ["in_01"],
        "a draft is not yet a bill and is not listed"
    );
    let bill = &listed.invoices[0];
    assert_eq!(
        (bill.total, bill.currency.as_str(), bill.status.as_str()),
        (34_783, "nzd", "paid")
    );
    assert_eq!(bill.created_at, NOW);
    assert_eq!(
        bill.hosted_url.as_deref(),
        Some("https://invoice.stripe.com/i/in_01")
    );
    assert_eq!(bill.description.as_deref(), Some("1 × Sync (monthly)"));

    let (status, body) = call(&pool, &base, "GET", "/v1/billing/payment-method", &TOKEN_A).await;
    assert_eq!(status, StatusCode::OK);
    let card = serde_json::from_slice::<PaymentMethodView>(&body)
        .expect("the payment-method shape")
        .card
        .expect("the subscription's own card is named");
    assert_eq!((card.brand.as_str(), card.last4.as_str()), ("visa", "3115"));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_tenant_that_never_paid_has_no_bills_and_nothing_to_cancel(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    subscribe(&pool, &base).await;

    let (status, body) = call(&pool, &base, "GET", "/v1/billing/invoices", &TOKEN_B).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        serde_json::from_slice::<InvoicesView>(&body)
            .expect("the invoices shape")
            .invoices
            .is_empty(),
        "the other tenant's bills are not this one's"
    );
    let (status, _) = call(&pool, &base, "POST", "/v1/billing/cancel", &TOKEN_B).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        !read_billing(pool, &TOKEN_A).await.cancel_at_period_end,
        "a refused cancel from one tenant leaves the other's plan renewing"
    );
}
