//! An operator deletes a seller over the wire: `DELETE /v1/admin/users/{subject}`.
//!
//! The severity is in what else goes. A seller alone in their organisation
//! takes it with them, with every tenant row it owns — the append-only audit
//! tables included, which is the one legitimate way their rows ever leave.
//! Three cases are refused with nothing deleted: someone else is in the
//! organisation, the seller is an operator, or Stripe still holds a live
//! subscription. Each test reads the database back rather than trusting the
//! status, because a refusal that deleted half a tenant would still be a 409.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::admin::DeletedUserView;
use tam_api::{router, APIError, AppState, Config, SESSION_COOKIE};
use tam_limits::Plan;
use tam_storage::{
    BillingRepo, ConnectionAudit, ConnectionEventRecord, EntitlementRepo, GrantedBy, NewGrant,
    OperatorRepo, SessionRepo, SessionToken, SubscriptionState,
};
use tam_types::{
    Actor, ConnectionEvent, ConnectionId, OrgId, Stamp, SystemComponent, Timestamp, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_OPERATOR: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_SELLER: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const USER_COLLEAGUE: UserId = UserId(Uuid([0x0C; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
const SUBJECT_OPERATOR: [u8; 16] = [0xA1; 16];
const SUBJECT_SELLER: [u8; 16] = [0xB1; 16];
const CONNECTION: ConnectionId = ConnectionId(Uuid([0xC1; 16]));
const NOW: Timestamp = Timestamp(5_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn backoffice_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the test database names itself");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_backoffice:tam_backoffice_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the backoffice role connects")
}

fn state(pool: PgPool, backoffice: PgPool) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: Some(backoffice),
        blobs: None,
    }
}

/// An operator in one organisation, and a seller alone in another who holds
/// a plan grant, a signed-in session, a cancelled subscription and a row in
/// the append-only connection audit.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_OPERATOR, "Teachouse"), (ORG_SELLER, "Maths Corner")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token, subject) in [
        (
            ORG_OPERATOR,
            USER_OPERATOR,
            "operator@example.test",
            TOKEN_OPERATOR,
            SUBJECT_OPERATOR,
        ),
        (
            ORG_SELLER,
            USER_SELLER,
            "seller@example.test",
            TOKEN_SELLER,
            SUBJECT_SELLER,
        ),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, Timestamp(100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
        sqlx::query("UPDATE app_user SET auth_subject = $2 WHERE id = $1")
            .bind(uuid::Uuid::from_bytes(user.0 .0))
            .bind(uuid::Uuid::from_bytes(subject))
            .execute(pool)
            .await
            .expect("the subject links");
    }
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", NOW)
        .await
        .expect("the operator marking lands");

    EntitlementRepo::new(pool.clone())
        .grant(
            ORG_SELLER,
            &NewGrant {
                id: Uuid([0xE1; 16]),
                plan: Plan::Subscriber,
                rung: None,
                granted_by: GrantedBy::Operator,
                grantor_user: Some(USER_OPERATOR.0),
                reason: Some("the test fixture"),
                source_ref: None,
                granted_at: Timestamp(2_000),
                expires_at: None,
            },
        )
        .await
        .expect("the grant lands");
    subscribe(pool, "canceled").await;
    ConnectionAudit::new(pool.clone())
        .record(&ConnectionEventRecord {
            org: ORG_SELLER,
            connection: CONNECTION,
            event: ConnectionEvent::Revoked,
            detail: None,
            stamp: Stamp {
                at: Timestamp(3_000),
                actor: Actor::System(SystemComponent::Broker),
            },
        })
        .await
        .expect("the audit row appends");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn subscribe(pool: &PgPool, status: &str) {
    BillingRepo::new(pool.clone())
        .apply(
            ORG_SELLER,
            &SubscriptionState {
                provider_subscription_id: "sub_seller".to_owned(),
                provider_customer_id: "cus_seller".to_owned(),
                status: status.to_owned(),
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn delete(pool: &PgPool, token: &SessionToken, subject: [u8; 16]) -> (StatusCode, Vec<u8>) {
    let backoffice = backoffice_pool(pool).await;
    let path = format!(
        "/v1/admin/users/{}",
        uuid::Uuid::from_bytes(subject).hyphenated()
    );
    let request = Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool.clone(), backoffice))
        .oneshot(request)
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

/// Rows the seller's organisation still holds in the tables this file seeds,
/// read under its pin so forced row-level security is satisfied.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seller_rows(pool: &PgPool) -> [i64; 5] {
    let org = uuid::Uuid::from_bytes(ORG_SELLER.0 .0);
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(org.to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let mut counts = [0; 5];
    for (slot, table) in [
        "organisation WHERE id = $1",
        "app_user WHERE org_id = $1",
        "entitlement_grant WHERE org_id = $1",
        "billing_subscription WHERE org_id = $1",
        "connection_audit WHERE org_id = $1",
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
fn refusal(body: &[u8]) -> (String, serde_json::Value) {
    let error: APIError = serde_json::from_slice(body).expect("the refusal is an APIError");
    let entry = error.errors.into_iter().next().expect("one entry");
    (
        entry.message,
        entry.detail.expect("the refusal says which one it is"),
    )
}

const UNTOUCHED: [i64; 5] = [1, 1, 1, 1, 1];

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_alone_in_their_organisation_is_deleted_with_it(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        seller_rows(&pool).await,
        UNTOUCHED,
        "the fixture holds a whole tenant"
    );

    let (status, body) = delete(&pool, &TOKEN_OPERATOR, SUBJECT_SELLER).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let deleted: DeletedUserView = serde_json::from_slice(&body).expect("the answer parses");
    assert_eq!(deleted.user, USER_SELLER);
    assert_eq!(deleted.organisation.org, ORG_SELLER);
    assert_eq!(deleted.organisation.name, "Maths Corner");

    assert_eq!(
        seller_rows(&pool).await,
        [0; 5],
        "the organisation, its user, its grant, its subscription record and its \
         append-only audit rows are all gone"
    );
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM user_session WHERE user_id = $1")
        .bind(uuid::Uuid::from_bytes(USER_SELLER.0 .0))
        .fetch_one(&pool)
        .await
        .expect("the count reads");
    assert_eq!(sessions, 0, "their session cannot outlive them");
    let operator_still: i64 = sqlx::query_scalar("SELECT count(*) FROM app_user WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_OPERATOR.0 .0))
        .fetch_one(&pool)
        .await
        .expect("the count reads");
    assert_eq!(operator_still, 1, "the other tenant is untouched");

    let (again, _) = delete(&pool, &TOKEN_OPERATOR, SUBJECT_SELLER).await;
    assert_eq!(
        again,
        StatusCode::NOT_FOUND,
        "a second delete finds nobody, so the console goes on to the sign-in account"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_who_shares_their_organisation_is_not_deleted(pool: PgPool) {
    provision(&pool).await;
    SessionRepo::new(pool.clone())
        .create_user(
            ORG_SELLER,
            USER_COLLEAGUE,
            "colleague@example.test",
            Timestamp(1_500),
        )
        .await
        .expect("the colleague provisions");

    let (status, body) = delete(&pool, &TOKEN_OPERATOR, SUBJECT_SELLER).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (message, detail) = refusal(&body);
    assert_eq!(detail["refusal"], "shared_organisation");
    assert_eq!(detail["other_members"], 1);
    assert!(
        message.contains("1 other member"),
        "the operator is told why: {message}"
    );
    let rows = seller_rows(&pool).await;
    assert_eq!(rows[0], 1, "the organisation stands");
    assert_eq!(rows[1], 2, "both members stand");
    assert_eq!(rows[2..], UNTOUCHED[2..], "nothing in the tenant moved");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_still_paying_stripe_is_not_deleted(pool: PgPool) {
    provision(&pool).await;
    subscribe(&pool, "past_due").await;

    let (status, body) = delete(&pool, &TOKEN_OPERATOR, SUBJECT_SELLER).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (message, detail) = refusal(&body);
    assert_eq!(detail["refusal"], "live_subscription");
    assert_eq!(detail["status"], "past_due");
    assert!(
        message.contains("Cancel it there first"),
        "the operator is told what to do: {message}"
    );
    assert_eq!(seller_rows(&pool).await, UNTOUCHED);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_cannot_be_deleted_from_the_console(pool: PgPool) {
    provision(&pool).await;

    let (status, body) = delete(&pool, &TOKEN_OPERATOR, SUBJECT_OPERATOR).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, detail) = refusal(&body);
    assert_eq!(detail["refusal"], "operator");
    let still: i64 = sqlx::query_scalar("SELECT count(*) FROM organisation WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_OPERATOR.0 .0))
        .fetch_one(&pool)
        .await
        .expect("the count reads");
    assert_eq!(
        still, 1,
        "an operator deleting themselves is the same refusal"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_cannot_delete_anybody(pool: PgPool) {
    provision(&pool).await;

    let (status, _) = delete(&pool, &TOKEN_SELLER, SUBJECT_SELLER).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "not an operator");
    assert_eq!(seller_rows(&pool).await, UNTOUCHED);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn outside_an_erasure_the_audit_tables_stay_append_only(pool: PgPool) {
    provision(&pool).await;
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_SELLER.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    for statement in [
        "DELETE FROM connection_audit",
        "UPDATE connection_audit SET event = 'linked'",
    ] {
        let refused = sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect_err("an ordinary statement cannot rewrite or erase the audit");
        let sqlx::Error::Database(refusal) = &refused else {
            panic!("the refusal must come from Postgres: {refused:?}");
        };
        assert_eq!(
            refusal.code().as_deref(),
            Some("42501"),
            "insufficient_privilege, from the guard: {statement}"
        );
        drop(tx);
        tx = pool.begin().await.expect("transaction begins");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(ORG_SELLER.0 .0).to_string())
            .execute(&mut *tx)
            .await
            .expect("tenant pin applies");
    }
    drop(tx);
    assert_eq!(seller_rows(&pool).await, UNTOUCHED);
}
