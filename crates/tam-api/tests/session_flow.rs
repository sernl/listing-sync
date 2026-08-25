//! The session floor end to end: a minted cookie authenticates whoami, an
//! expired one refuses with the closed vocabulary, and the whole flow runs
//! in-process against a per-test database.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::{router, APIError, APIErrorCode, AppState, Config, Whoami, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool, expires_at: Timestamp) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let repo = SessionRepo::new(pool.clone());
    repo.create_user(ORG, USER, "founder@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    repo.mint(&TOKEN, USER, expires_at, Timestamp(1_000))
        .await
        .expect("the session mints");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn get_with_cookie(pool: PgPool, path: &str) -> (StatusCode, Vec<u8>) {
    let request = Request::builder()
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, body)
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_live_session_authenticates_whoami(pool: PgPool) {
    provision(&pool, Timestamp(10_000)).await;
    let (status, body) = get_with_cookie(pool, "/v1/whoami").await;
    assert_eq!(status, StatusCode::OK);
    let whoami: Whoami = serde_json::from_slice(&body).expect("the body is whoami");
    assert_eq!(
        (whoami.org, whoami.user),
        (ORG, USER),
        "the cookie speaks for exactly the minted identity"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_expired_session_refuses_with_the_closed_vocabulary(pool: PgPool) {
    provision(&pool, Timestamp(4_999)).await;
    let (status, body) = get_with_cookie(pool, "/v1/whoami").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let error: APIError = serde_json::from_slice(&body).expect("the body is the error");
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::SessionRequired),
        "expiry and absence are the same refusal"
    );
}
