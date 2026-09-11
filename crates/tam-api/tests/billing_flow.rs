//! Billing over the wire: a correctly signed Paddle notification driven into
//! the router, and the org-scoped read that shows what it changed.
//!
//! The webhook carries no session and names no organisation in its path, so
//! the tenancy statement provable at this surface is the one asserted here:
//! the organisation is the one the signed payload names, and a second tenant's
//! row is untouched by it. Nothing in this file opens a socket — the signature
//! is constructed in-process from the same secret the router is configured
//! with, which is what makes an end-to-end test possible with no Paddle
//! account in existence.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use core::fmt::Write as _;

use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::{
    paddle::SIGNATURE_TOLERANCE_SECS, router, AppState, BillingView, Config, WebhookSecret,
    ORG_CUSTOM_DATA_KEY, SESSION_COOKIE,
};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);

const SECRET: &str = "pdl_ntfset_01_development_only";
/// The wall instant every request in this file is served at, and the instant
/// the signatures are stamped with, so the tolerance window is exercised by
/// choosing a timestamp rather than by waiting.
const NOW_SECS: i64 = 1_800_000_000;
const NOW: Timestamp = Timestamp(NOW_SECS * 1_000);

fn state(pool: PgPool, secret: Option<&str>) -> AppState {
    AppState {
        pool,
        config: Config {
            paddle_webhook_secret: secret.map(|raw| WebhookSecret::new(raw.to_owned())),
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
    mac.update(b":");
    mac.update(body.as_bytes());
    let mut digest = String::new();
    for byte in mac.finalize().into_bytes() {
        write!(digest, "{byte:02x}").expect("writing to a String is infallible");
    }
    format!("ts={ts};h1={digest}")
}

/// One subscription notification as Paddle renders it, carrying the
/// organisation under the key the checkout agrees on.
fn notification(org: OrgId, event: &str, status: &str, occurred_at: &str) -> String {
    serde_json::json!({
        "event_id": "evt_01",
        "event_type": event,
        "occurred_at": occurred_at,
        "notification_id": "ntf_01",
        "data": {
            "id": format!("sub_for_{}", org.0.to_hyphenated()),
            "customer_id": "ctm_01",
            "status": status,
            "current_billing_period": {
                "starts_at": "2027-01-15T00:00:00.000000Z",
                "ends_at": "2027-02-15T00:00:00.000000Z"
            },
            "custom_data": { ORG_CUSTOM_DATA_KEY: org.0.to_hyphenated() }
        }
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
async fn post_webhook(pool: PgPool, signature: Option<&str>, body: &str) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(signature) = signature {
        request = request.header("Paddle-Signature", signature);
    }
    let request = request
        .body(Body::from(body.to_owned()))
        .expect("the request builds");
    router(state(pool, Some(SECRET)))
        .oneshot(request)
        .await
        .expect("the router serves")
        .status()
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
    let response = router(state(pool, Some(SECRET)))
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

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_signed_notification_becomes_state_the_organisation_can_read(pool: PgPool) {
    provision(&pool).await;
    let body = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00.000000Z",
    );
    assert_eq!(
        post_webhook(pool.clone(), Some(&sign(NOW_SECS, &body, SECRET)), &body).await,
        StatusCode::OK,
        "Paddle is acknowledged"
    );

    let view = read_billing(pool, &TOKEN_A).await;
    let subscription = view
        .subscription
        .expect("the organisation the payload named now has a subscription");
    assert_eq!(
        subscription.status, "active",
        "the status crossed the whole path from the signed bytes to the read"
    );
    assert_eq!(
        subscription.paddle_customer_id, "ctm_01",
        "and so did the customer Paddle named"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_notification_for_one_organisation_leaves_the_other_with_nothing(pool: PgPool) {
    provision(&pool).await;
    let body = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00.000000Z",
    );
    post_webhook(pool.clone(), Some(&sign(NOW_SECS, &body, SECRET)), &body).await;
    assert!(
        read_billing(pool, &TOKEN_B).await.subscription.is_none(),
        "the organisation the payload did not name reads the honest none-shape"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_redelivered_older_notification_does_not_regress_what_the_tenant_reads(pool: PgPool) {
    provision(&pool).await;
    for (status, occurred_at) in [
        ("canceled", "2027-01-10T00:00:00.000000Z"),
        ("active", "2027-01-15T00:00:00.000000Z"),
    ]
    .into_iter()
    .rev()
    {
        // Newest first, then the older redelivery behind it: the order Paddle
        // can produce and the order the fence exists for.
        let body = notification(ORG_A, "subscription.updated", status, occurred_at);
        assert_eq!(
            post_webhook(pool.clone(), Some(&sign(NOW_SECS, &body, SECRET)), &body).await,
            StatusCode::OK,
            "a stale delivery is acknowledged rather than made Paddle's problem"
        );
    }
    assert_eq!(
        read_billing(pool, &TOKEN_A)
            .await
            .subscription
            .map(|subscription| subscription.status),
        Some("active".to_owned()),
        "the tenant reads the state Paddle last described, not the last one delivered"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unsigned_or_wrongly_signed_notification_changes_nothing(pool: PgPool) {
    provision(&pool).await;
    let body = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00.000000Z",
    );
    let stale = NOW_SECS - SIGNATURE_TOLERANCE_SECS - 1;
    for (signature, why) in [
        (None, "no signature header at all"),
        (
            Some(sign(NOW_SECS, &body, "another-secret")),
            "another secret",
        ),
        (
            Some(sign(stale, &body, SECRET)),
            "a timestamp outside tolerance",
        ),
        (Some("ts=nonsense".to_owned()), "a malformed header"),
    ] {
        assert_eq!(
            post_webhook(pool.clone(), signature.as_deref(), &body).await,
            StatusCode::UNAUTHORIZED,
            "{why} is refused"
        );
    }
    assert!(
        read_billing(pool, &TOKEN_A).await.subscription.is_none(),
        "and none of them wrote a row"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_signed_body_that_was_altered_in_flight_is_refused(pool: PgPool) {
    provision(&pool).await;
    let signed = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00.000000Z",
    );
    let delivered = signed.replace("\"active\"", "\"canceled\"");
    assert_eq!(
        post_webhook(
            pool.clone(),
            Some(&sign(NOW_SECS, &signed, SECRET)),
            &delivered
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "the signature covers the raw bytes, so one altered field refuses"
    );
    assert!(
        read_billing(pool, &TOKEN_A).await.subscription.is_none(),
        "and nothing was recorded"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_event_type_this_milestone_does_not_handle_is_acknowledged(pool: PgPool) {
    provision(&pool).await;
    let body = notification(
        ORG_A,
        "transaction.completed",
        "completed",
        "2027-01-15T00:00:00.000000Z",
    );
    assert_eq!(
        post_webhook(pool.clone(), Some(&sign(NOW_SECS, &body, SECRET)), &body).await,
        StatusCode::OK,
        "an unhandled event must not be answered with a status Paddle retries"
    );
    assert!(
        read_billing(pool, &TOKEN_A).await.subscription.is_none(),
        "acknowledged is not recorded"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_notification_naming_no_organisation_is_acknowledged_and_ignored(pool: PgPool) {
    provision(&pool).await;
    let body = serde_json::json!({
        "event_id": "evt_02",
        "event_type": "subscription.created",
        "occurred_at": "2027-01-15T00:00:00.000000Z",
        "data": {
            "id": "sub_from_somewhere_else",
            "customer_id": "ctm_02",
            "status": "active"
        }
    })
    .to_string();
    assert_eq!(
        post_webhook(pool.clone(), Some(&sign(NOW_SECS, &body, SECRET)), &body).await,
        StatusCode::OK,
        "a subscription created outside our checkout carries no org and is not a fault"
    );
    assert!(
        read_billing(pool, &TOKEN_A).await.subscription.is_none(),
        "and it attached itself to no tenant"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_handled_event_missing_the_state_it_describes_is_refused_for_replay(pool: PgPool) {
    provision(&pool).await;
    let body = serde_json::json!({
        "event_id": "evt_03",
        "event_type": "subscription.updated",
        "occurred_at": "2027-01-15T00:00:00.000000Z",
        "data": {
            "id": "sub_01",
            "custom_data": { ORG_CUSTOM_DATA_KEY: ORG_A.0.to_hyphenated() }
        }
    })
    .to_string();
    assert_eq!(
        post_webhook(pool.clone(), Some(&sign(NOW_SECS, &body, SECRET)), &body).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a subscription event we cannot read must stay visible in Paddle's \
         failed-notification list rather than be dropped silently"
    );
    assert!(
        read_billing(pool, &TOKEN_A).await.subscription.is_none(),
        "and it wrote no half-row"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_deployment_with_no_secret_refuses_the_webhook_rather_than_accepting_it(pool: PgPool) {
    provision(&pool).await;
    let body = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00.000000Z",
    );
    let request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header("Paddle-Signature", sign(NOW_SECS, &body, SECRET))
        .body(Body::from(body))
        .expect("the request builds");
    let response = router(state(pool.clone(), None))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unconfigured surface refuses the way the operator surface does, and \
         with a status Paddle retries once the secret is supplied"
    );
    assert!(
        read_billing(pool, &TOKEN_A).await.subscription.is_none(),
        "nothing was recorded"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_read_is_closed_to_a_session_that_does_not_resolve(pool: PgPool) {
    provision(&pool).await;
    let unknown = SessionToken([0x44; 32]);
    let request = Request::builder()
        .uri("/v1/billing")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", unknown.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool, Some(SECRET)))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the read is org-scoped like every sibling, so no session is no answer"
    );
}

// ------------------------------------------------------- grants from Paddle

/// How many grant rows one organisation holds, counted through the tenant
/// pin the policies demand: an unpinned statement matches nothing, which
/// would make every count in this file a vacuous zero.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn grants_held(pool: &PgPool, org: OrgId) -> i64 {
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM entitlement_grant WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("the count runs");
    rows
}

/// The price map a deployment would be started with: one recurring price and
/// one rung of the Catalogue Import ladder.
fn price_map() -> tam_api::PriceMap {
    tam_api::PriceMap::parse(
        r#"{
             "pri_subscriber_monthly": { "plan": "subscriber" },
             "pri_rung_50": { "plan": "migration_only", "rung": 50 }
           }"#,
    )
    .unwrap_or_default()
}

fn priced_state(pool: PgPool) -> AppState {
    let mut built = state(pool, Some(SECRET));
    built.config.paddle_price_map = price_map();
    built
}

/// One `transaction.completed` as Paddle v2 renders it: the price lives on
/// the line item, the organisation in `custom_data`, and the transaction's
/// own identifier is what the grant is attributed to.
fn transaction(org: OrgId, price: &str) -> String {
    serde_json::json!({
        "event_id": "evt_02",
        "event_type": "transaction.completed",
        "occurred_at": "2027-01-15T00:00:00.000000Z",
        "data": {
            "id": "txn_01",
            "status": "completed",
            "items": [{ "price": { "id": price }, "quantity": 1 }],
            "custom_data": { ORG_CUSTOM_DATA_KEY: org.0.to_hyphenated() }
        }
    })
    .to_string()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn post_priced_webhook(pool: PgPool, body: &str) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/billing/webhook")
        .header(header::CONTENT_TYPE, "application/json")
        .header("Paddle-Signature", sign(NOW_SECS, body, SECRET))
        .body(Body::from(body.to_owned()))
        .expect("the request builds");
    router(priced_state(pool))
        .oneshot(request)
        .await
        .expect("the router serves")
        .status()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn read_entitlement(pool: PgPool, token: &SessionToken) -> serde_json::Value {
    let request = Request::builder()
        .uri("/v1/entitlement")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(priced_state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the entitlement read answers the session that asked"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    serde_json::from_slice(&body).expect("the answer is the entitlement shape")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_entitling_subscription_grants_the_subscriber_plan(pool: PgPool) {
    provision(&pool).await;
    let body = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00Z",
    );
    assert_eq!(
        post_priced_webhook(pool.clone(), &body).await,
        StatusCode::OK
    );

    let held = read_entitlement(pool.clone(), &TOKEN_A).await;
    assert_eq!(held["plan"], "subscriber");
    assert_eq!(held["granted_by"], "paddle");
    assert_eq!(
        held["capabilities"]["resources_max"], 400,
        "the plan's capabilities are what the Account page reads back"
    );
    assert_eq!(
        read_entitlement(pool, &TOKEN_B).await["plan"],
        "free",
        "the grant lands on the organisation the signed payload named and no other"
    );
}

/// A renewal stream must renew one grant rather than pile up a row per
/// delivery, because `subscription.updated` arrives on every card change.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_repeated_subscription_event_renews_one_grant(pool: PgPool) {
    provision(&pool).await;
    for event in ["subscription.created", "subscription.updated"] {
        let body = notification(ORG_A, event, "active", "2027-01-15T00:00:00Z");
        assert_eq!(
            post_priced_webhook(pool.clone(), &body).await,
            StatusCode::OK
        );
    }
    let rows = grants_held(&pool, ORG_A).await;
    assert_eq!(rows, 1, "two deliveries of one subscription are one grant");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cancellation_lets_the_grant_run_to_the_period_end(pool: PgPool) {
    provision(&pool).await;
    let live = notification(
        ORG_A,
        "subscription.created",
        "active",
        "2027-01-15T00:00:00Z",
    );
    assert_eq!(
        post_priced_webhook(pool.clone(), &live).await,
        StatusCode::OK
    );
    let gone = notification(
        ORG_A,
        "subscription.canceled",
        "canceled",
        "2027-01-16T00:00:00Z",
    );
    assert_eq!(
        post_priced_webhook(pool.clone(), &gone).await,
        StatusCode::OK
    );

    let held = read_entitlement(pool, &TOKEN_A).await;
    assert_eq!(
        held["plan"], "subscriber",
        "a cancelled subscription keeps entitling until the period it was paid for ends"
    );
    assert_eq!(
        held["expires_at"],
        serde_json::json!(1_802_649_600_000_i64),
        "and the expiry is the period end Paddle named, with no grace added to a \
         cancellation the seller chose"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_completed_one_off_grants_the_rung_its_price_names(pool: PgPool) {
    provision(&pool).await;
    let body = transaction(ORG_A, "pri_rung_50");
    assert_eq!(
        post_priced_webhook(pool.clone(), &body).await,
        StatusCode::OK
    );

    let held = read_entitlement(pool.clone(), &TOKEN_A).await;
    assert_eq!(held["plan"], "migration_only");
    assert_eq!(held["rung"], 50);
    assert_eq!(held["expires_at"], serde_json::Value::Null);
    assert_eq!(
        held["capabilities"]["resources_max"], 50,
        "the rung bought is the resource allowance"
    );
    assert_eq!(
        held["capabilities"]["edit_days_after_purchase"], 30,
        "and the thirty-day edit window rides on the plan rather than on the grant"
    );

    // A retried delivery of one purchase must not grant twice.
    assert_eq!(
        post_priced_webhook(pool.clone(), &body).await,
        StatusCode::OK
    );
    let rows = grants_held(&pool, ORG_A).await;
    assert_eq!(rows, 1, "a replayed transaction is the same purchase");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_price_the_map_does_not_know_grants_nothing(pool: PgPool) {
    provision(&pool).await;
    let body = transaction(ORG_A, "pri_a_rung_nobody_configured");
    assert_eq!(
        post_priced_webhook(pool.clone(), &body).await,
        StatusCode::OK,
        "Paddle is acknowledged, because refusing would have it retry forever"
    );
    assert_eq!(
        read_entitlement(pool, &TOKEN_A).await["plan"],
        "free",
        "and nothing is granted, rather than the largest rung being guessed at"
    );
}
