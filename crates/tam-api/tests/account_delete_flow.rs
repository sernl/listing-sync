//! A seller deletes their own account over the wire: `DELETE /v1/account`.
//!
//! Stripe is a loopback double that answers the subscription read and the
//! immediate cancellation, and records that the cancellation was asked for.
//! The identity service and the relay are an in-process [`Offboarding`] that
//! records every call in order, because the order is the contract: the
//! goodbye goes before the identity is deleted, and nothing irreversible
//! happens before the proof is checked. Each test reads the database back
//! rather than trusting the status.

#![cfg(feature = "pg-tests")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use axum::{
    body::Body,
    extract::Path,
    http::{header, Method, Request, StatusCode},
    routing::get,
    Json,
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::account::{
    email_hash, AccountDeletedView, Departing, Offboarding, OffboardingFault, OffboardingFuture,
    OffboardingPort, Proof, Reauthentication,
};
use tam_api::{router, stripe, APIError, AppState, Config, SecretKey, SESSION_COOKIE};
use tam_storage::{BillingRepo, OperatorRepo, SessionRepo, SessionToken, SubscriptionState};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xBB; 16]));
const OTHER_ORG: OrgId = OrgId(Uuid([0xCC; 16]));
const USER: UserId = UserId(Uuid([0x0B; 16]));
const COLLEAGUE: UserId = UserId(Uuid([0x0C; 16]));
const OTHER_USER: UserId = UserId(Uuid([0x0D; 16]));
const TOKEN: SessionToken = SessionToken([0x42; 32]);
const SUBJECT: [u8; 16] = [0xB1; 16];
const OTHER_SUBJECT: [u8; 16] = [0xD1; 16];
const SLUG: &str = "maths-corner";
const PASSWORD: &str = "correct horse battery staple";
const ADDRESS: &str = "Aroha@Example.test";
const SUBSCRIPTION: &str = "sub_seller";
const NOW: Timestamp = Timestamp(1_800_000_000_000);

/// The identity service and the relay, recording each call in order.
#[derive(Default)]
struct FakeOffboarding {
    calls: RwLock<Vec<String>>,
}

impl FakeOffboarding {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a poisoned record should panic"
    )]
    fn note(&self, call: String) {
        self.calls
            .write()
            .expect("the record is writable")
            .push(call);
    }

    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a poisoned record should panic"
    )]
    fn calls(&self) -> Vec<String> {
        self.calls.read().expect("the record is readable").clone()
    }
}

impl Offboarding for FakeOffboarding {
    fn reauthenticate<'a>(
        &'a self,
        subject: Uuid,
        proof: Proof<'a>,
    ) -> OffboardingFuture<'a, Reauthentication> {
        self.note(format!("reauth {}", subject.to_hyphenated()));
        let verdict = match proof.password {
            Some(PASSWORD) => Reauthentication::Confirmed(Departing {
                email: ADDRESS.to_owned(),
                name: Some("Aroha".to_owned()),
            }),
            Some(_) => Reauthentication::PasswordWrong,
            None => Reauthentication::PasswordRequired,
        };
        Box::pin(core::future::ready(Ok(verdict)))
    }

    fn farewell<'a>(&'a self, to: &'a Departing) -> OffboardingFuture<'a, ()> {
        self.note(format!("farewell {}", to.email));
        Box::pin(core::future::ready(Ok(())))
    }

    fn delete_identity(&self, subject: Uuid) -> OffboardingFuture<'_, ()> {
        self.note(format!("delete {}", subject.to_hyphenated()));
        Box::pin(core::future::ready(Ok::<(), OffboardingFault>(())))
    }
}

/// A Stripe double: the subscription is active until the immediate
/// cancellation arrives, and the cancellation is recorded.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn stripe_double(cancelled: Arc<AtomicBool>) -> String {
    let subscription = |cancelled: bool| {
        serde_json::json!({
            "id": SUBSCRIPTION,
            "object": "subscription",
            "status": if cancelled { "canceled" } else { "active" },
            "customer": "cus_seller",
            "cancel_at_period_end": false,
            "cancel_at": null,
        })
    };
    let read = Arc::clone(&cancelled);
    let write = Arc::clone(&cancelled);
    let app = axum::Router::new().route(
        "/v1/subscriptions/{id}",
        get(move |Path(_id): Path<String>| {
            let read = Arc::clone(&read);
            async move { Json(subscription(read.load(Ordering::SeqCst))) }
        })
        .delete(move |Path(_id): Path<String>| {
            let write = Arc::clone(&write);
            async move {
                write.store(true, Ordering::SeqCst);
                Json(subscription(true))
            }
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

fn state(pool: PgPool, stripe_base: &str, offboarding: Option<Arc<FakeOffboarding>>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            stripe: Some(stripe::Client::with_base(
                SecretKey::new("sk_test_development_only".to_owned()),
                stripe_base.to_owned(),
            )),
            offboarding: offboarding.map(|fake| OffboardingPort(fake)),
            ..Config::default()
        },
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

/// A seller alone in a claimed organisation, signed in, paying monthly; and a
/// second tenant nothing here may touch.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name, slug) in [
        (ORG, "Maths Corner", Some(SLUG)),
        (OTHER_ORG, "Science Shed", None),
    ] {
        sqlx::query(
            "INSERT INTO organisation (id, name, slug, created_at) VALUES ($1, $2, $3, now())",
        )
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(name)
        .bind(slug)
        .execute(pool)
        .await
        .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, subject) in [
        (ORG, USER, "seller@example.test", SUBJECT),
        (OTHER_ORG, OTHER_USER, "other@example.test", OTHER_SUBJECT),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sqlx::query("UPDATE app_user SET auth_subject = $2 WHERE id = $1")
            .bind(uuid::Uuid::from_bytes(user.0 .0))
            .bind(uuid::Uuid::from_bytes(subject))
            .execute(pool)
            .await
            .expect("the subject links");
    }
    sessions
        .mint(&TOKEN, USER, Timestamp(NOW.0 + 100_000), Timestamp(1_000))
        .await
        .expect("the session mints");
    BillingRepo::new(pool.clone())
        .apply(
            ORG,
            &SubscriptionState {
                provider_subscription_id: SUBSCRIPTION.to_owned(),
                provider_customer_id: "cus_seller".to_owned(),
                status: "active".to_owned(),
                provider_price_id: None,
                current_period_end: None,
                cancel_at_period_end: None,
                occurred_at: Timestamp(4_000),
            },
            NOW,
        )
        .await
        .expect("the subscription state applies");
}

struct Answer {
    status: StatusCode,
    set_cookie: Option<String>,
    body: Vec<u8>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn delete_account(state: AppState, body: &serde_json::Value) -> Answer {
    let request = Request::builder()
        .method(Method::DELETE)
        .uri("/v1/account")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!(
                "{SESSION_COOKIE}={}; better-auth.session_token=tok.sig",
                TOKEN.to_hex()
            ),
        )
        .body(Body::from(body.to_string()))
        .expect("the request builds");
    let response = router(state)
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer {
        status,
        set_cookie,
        body,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn refusal(body: &[u8]) -> String {
    let error: APIError = serde_json::from_slice(body).expect("the refusal is an APIError");
    let entry = error.errors.into_iter().next().expect("one entry");
    entry
        .detail
        .and_then(|detail| {
            detail
                .get("refusal")
                .and_then(|word| word.as_str())
                .map(str::to_owned)
        })
        .expect("the refusal names itself")
}

/// The seller's organisation, user and subscription rows, read under its pin.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seller_rows(pool: &PgPool) -> [i64; 3] {
    let org = uuid::Uuid::from_bytes(ORG.0 .0);
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(org.to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let mut counts = [0; 3];
    for (slot, table) in [
        "organisation WHERE id = $1",
        "app_user WHERE org_id = $1",
        "billing_subscription WHERE org_id = $1",
    ]
    .iter()
    .enumerate()
    {
        counts[slot] = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .bind(org)
            .fetch_one(&mut *tx)
            .await
            .expect("the count reads");
    }
    counts
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn deletions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM account_deletion")
        .fetch_one(pool)
        .await
        .expect("the count reads")
}

fn confirmed(confirm: &str, password: &str) -> serde_json::Value {
    serde_json::json!({
        "confirm": confirm,
        "password": password,
        "reason": "  Retiring at the end of term.  ",
    })
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_deletes_their_account_and_everything_goes_in_order(pool: PgPool) {
    provision(&pool).await;
    let cancelled = Arc::new(AtomicBool::new(false));
    let base = stripe_double(Arc::clone(&cancelled)).await;
    let fake = Arc::new(FakeOffboarding::default());

    let answer = delete_account(
        state(pool.clone(), &base, Some(Arc::clone(&fake))),
        &confirmed("Maths-Corner", PASSWORD),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    let view: AccountDeletedView = serde_json::from_slice(&answer.body).expect("the answer parses");
    assert_eq!(
        view,
        AccountDeletedView {
            subscription_cancelled: true,
            goodbye_sent: true,
        }
    );
    assert!(
        answer
            .set_cookie
            .as_deref()
            .is_some_and(|cookie| cookie.starts_with(&format!("{SESSION_COOKIE}=;"))
                && cookie.contains("Max-Age=0")),
        "the API's cookie is expired with the account: {:?}",
        answer.set_cookie
    );
    assert!(
        cancelled.load(Ordering::SeqCst),
        "Stripe was told to cancel now"
    );

    let subject = Uuid(SUBJECT).to_hyphenated();
    assert_eq!(
        fake.calls(),
        vec![
            format!("reauth {subject}"),
            format!("farewell {ADDRESS}"),
            format!("delete {subject}"),
        ],
        "the proof first, the goodbye while the address is known, the identity last"
    );

    assert_eq!(
        seller_rows(&pool).await,
        [0; 3],
        "the organisation, the user and the subscription record are gone"
    );
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM user_session WHERE user_id = $1")
        .bind(uuid::Uuid::from_bytes(USER.0 .0))
        .fetch_one(&pool)
        .await
        .expect("the count reads");
    assert_eq!(sessions, 0, "their session cannot outlive them");
    let others: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(OTHER_ORG.0 .0))
        .fetch_one(&pool)
        .await
        .expect("the count reads");
    assert_eq!(others, 1, "the other tenant is untouched");

    let row: (
        uuid::Uuid,
        String,
        uuid::Uuid,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT subject, email_hash, org_id, stripe_subscription_id, reason FROM account_deletion",
    )
    .fetch_one(&pool)
    .await
    .expect("one audit row");
    assert_eq!(row.0, uuid::Uuid::from_bytes(SUBJECT));
    assert_eq!(
        row.1,
        email_hash("aroha@example.test"),
        "the address is kept as a digest only"
    );
    assert_eq!(row.2, uuid::Uuid::from_bytes(ORG.0 .0));
    assert_eq!(row.3.as_deref(), Some(SUBSCRIPTION));
    assert_eq!(row.4.as_deref(), Some("Retiring at the end of term."));
    let requested: i64 = sqlx::query_scalar(
        "SELECT (extract(epoch FROM requested_at) * 1000)::bigint FROM account_deletion",
    )
    .fetch_one(&pool)
    .await
    .expect("the instant reads");
    assert_eq!(requested, NOW.0);

    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("the test database names itself");
    let backoffice = PgPoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "postgres://tam_backoffice:tam_backoffice_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the backoffice role connects");
    let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM account_deletion")
        .fetch_one(&backoffice)
        .await
        .expect("the backoffice role reads the deletions");
    assert_eq!(seen, 1, "support can see that the account was deleted");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_steps_down_before_deleting_and_nothing_moves(pool: PgPool) {
    provision(&pool).await;
    OperatorRepo::new(pool.clone())
        .grant(USER, "the test fixture", NOW)
        .await
        .expect("the operator marking lands");
    let cancelled = Arc::new(AtomicBool::new(false));
    let base = stripe_double(Arc::clone(&cancelled)).await;
    let fake = Arc::new(FakeOffboarding::default());

    let answer = delete_account(
        state(pool.clone(), &base, Some(Arc::clone(&fake))),
        &confirmed(SLUG, PASSWORD),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refusal(&answer.body), "operator");
    assert!(
        fake.calls().is_empty(),
        "the identity service is not asked anything"
    );
    assert!(!cancelled.load(Ordering::SeqCst), "Stripe is not touched");
    assert_eq!(seller_rows(&pool).await, [1; 3], "nothing was deleted");
    assert_eq!(deletions(&pool).await, 0);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_wrong_password_or_confirmation_deletes_nothing(pool: PgPool) {
    provision(&pool).await;
    let cancelled = Arc::new(AtomicBool::new(false));
    let base = stripe_double(Arc::clone(&cancelled)).await;

    for (body, status, word) in [
        (confirmed(SLUG, "guess"), StatusCode::FORBIDDEN, "password"),
        (
            serde_json::json!({ "confirm": "DELETE" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "password_required",
        ),
        (
            confirmed("delete", PASSWORD),
            StatusCode::UNPROCESSABLE_ENTITY,
            "confirmation",
        ),
        (
            confirmed("science-shed", PASSWORD),
            StatusCode::UNPROCESSABLE_ENTITY,
            "confirmation",
        ),
    ] {
        let fake = Arc::new(FakeOffboarding::default());
        let answer =
            delete_account(state(pool.clone(), &base, Some(Arc::clone(&fake))), &body).await;
        assert_eq!(answer.status, status, "{body}");
        assert_eq!(refusal(&answer.body), word, "{body}");
        assert!(
            fake.calls().iter().all(|call| call.starts_with("reauth")),
            "only the proof was checked: {:?}",
            fake.calls()
        );
    }
    assert!(!cancelled.load(Ordering::SeqCst), "Stripe is not touched");
    assert_eq!(seller_rows(&pool).await, [1; 3], "nothing was deleted");
    assert_eq!(deletions(&pool).await, 0);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_shared_organisation_is_not_erased_and_a_deployment_without_the_port_refuses(
    pool: PgPool,
) {
    provision(&pool).await;
    let cancelled = Arc::new(AtomicBool::new(false));
    let base = stripe_double(Arc::clone(&cancelled)).await;

    let unconfigured =
        delete_account(state(pool.clone(), &base, None), &confirmed(SLUG, PASSWORD)).await;
    assert_eq!(unconfigured.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refusal(&unconfigured.body), "unavailable");

    SessionRepo::new(pool.clone())
        .create_user(ORG, COLLEAGUE, "colleague@example.test", Timestamp(1_000))
        .await
        .expect("the colleague provisions");
    let fake = Arc::new(FakeOffboarding::default());
    let shared = delete_account(
        state(pool.clone(), &base, Some(Arc::clone(&fake))),
        &confirmed(SLUG, PASSWORD),
    )
    .await;
    assert_eq!(shared.status, StatusCode::CONFLICT);
    assert_eq!(refusal(&shared.body), "shared_organisation");
    assert!(fake.calls().is_empty());
    assert!(!cancelled.load(Ordering::SeqCst), "Stripe is not touched");
    assert_eq!(deletions(&pool).await, 0);
}
