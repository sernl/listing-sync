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
//! One socket does open, and only one: `checkout.session.completed` is
//! fulfilled by re-reading the session with its line items expanded, which is
//! Stripe's own guidance and the only outbound call on any path here. The
//! double below answers that read on a loopback port, so the fulfilment
//! branch — the branch that credits a seller's moves — is exercised rather
//! than stubbed past.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    extract::Path,
    http::{header, Request, StatusCode},
    routing::get,
    Json,
};
use core::fmt::Write as _;

use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::{
    router, stripe, AppState, BillingView, Config, PriceMap, SecretKey, WebhookSecret,
    ORG_METADATA_KEY, SESSION_COOKIE,
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
const SERVICE_PRICE: &str = "price_move_with_me";
const PACK_SESSION: &str = "cs_pack_01";
const SUBSCRIPTION_SESSION: &str = "cs_sub_01";
const SERVICE_SESSION: &str = "cs_svc_01";
const SUBSCRIPTION: &str = "sub_01";
const CUSTOMER: &str = "cus_01";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn price_map() -> PriceMap {
    PriceMap::parse(&format!(
        r#"{{"{PACK_PRICE}":"pack_100","{MONTHLY_PRICE}":"sync_monthly","{SERVICE_PRICE}":"move_with_me"}}"#
    ))
    .expect("the fixture price map parses")
}

/// A Stripe double answering the one outbound read the webhook makes.
///
/// It serves every session this file posts, keyed by the identifier in the
/// path, with the line items the real API would return for an expanded read.
/// Nothing else is implemented, because nothing else is called.
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
                SERVICE_SESSION => ("payment", SERVICE_PRICE, None),
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

fn subscription_event(org: OrgId, event: &str, status: &str) -> String {
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
                "price": { "id": MONTHLY_PRICE, "object": "price" }
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
    assert!(
        !view.founding,
        "the monthly price is not the founding price"
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
            &subscription_event(ORG_A, "customer.subscription.deleted", "canceled")
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
            &subscription_event(ORG_A, "customer.subscription.updated", "active")
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
async fn the_service_purchase_books_time_and_grants_nothing(pool: PgPool) {
    provision(&pool).await;
    let base = stripe_double().await;
    assert_eq!(
        deliver(
            &pool,
            &base,
            &checkout_completed(ORG_A, SERVICE_SESSION, "payment")
        )
        .await,
        StatusCode::OK
    );

    let view = read_billing(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        view.plan.as_str(),
        "free",
        "\"Move with me\" buys 45 minutes of somebody's time, not a capability"
    );
    assert_eq!(view.moves.available, 0, "and not a move either");
    let booked: i64 = pinned_scalar(
        &pool,
        ORG_A,
        "SELECT count(*) FROM service_booking WHERE key = $1",
        "move_with_me",
    )
    .await;
    assert_eq!(booked, 1, "the booking is what was recorded");
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
