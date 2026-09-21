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
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call_json(
    pool: PgPool,
    method: axum::http::Method,
    path: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>, Option<String>) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let request = match body {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state(pool))
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
    (status, body, set_cookie)
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_exchange_turns_the_minted_token_into_the_cookie_and_logout_clears_it(pool: PgPool) {
    provision(&pool, Timestamp(100_000)).await;

    // A bad token is the same refusal as no session.
    let (status, _body, _cookie) = call_json(
        pool.clone(),
        axum::http::Method::POST,
        "/v1/session",
        None,
        Some(serde_json::json!({ "token": "zz".repeat(32) })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The mint tool prints "tam_session=<hex>"; the exchange accepts that
    // form verbatim, so the founder pastes the whole line.
    let (status, body, set_cookie) = call_json(
        pool.clone(),
        axum::http::Method::POST,
        "/v1/session",
        None,
        Some(serde_json::json!({ "token": format!("tam_session={}", TOKEN.to_hex()) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let whoami: Whoami = serde_json::from_slice(&body).expect("the exchange echoes whoami");
    assert_eq!(whoami.org, ORG);
    let cookie = set_cookie.expect("the exchange sets the cookie");
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && cookie.contains("Secure"),
        "the cookie carries its protections: {cookie}"
    );
    assert!(
        cookie.contains("Max-Age=95"),
        "Max-Age is honest to the session's expiry (95s left): {cookie}"
    );

    // The set cookie authenticates; logout expires it server-side and the
    // same cookie is then refused.
    let pair = cookie.split(';').next().expect("the cookie has a pair");
    let (status, _body, _cookie) = call_json(
        pool.clone(),
        axum::http::Method::GET,
        "/v1/whoami",
        Some(pair),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the exchanged cookie authenticates");

    let (status, _body, clearing) = call_json(
        pool.clone(),
        axum::http::Method::DELETE,
        "/v1/session",
        Some(pair),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        clearing.is_some_and(|cookie| cookie.contains("Max-Age=0")),
        "logout clears the browser's copy"
    );
    let (status, _body, _cookie) = call_json(
        pool,
        axum::http::Method::GET,
        "/v1/whoami",
        Some(pair),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the expired session is gone server-side, not merely from the browser"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_status_page_needs_no_session(pool: PgPool) {
    provision(&pool, Timestamp(100_000)).await;
    let (status, body, _cookie) =
        call_json(pool, axum::http::Method::GET, "/v1/status", None, None).await;
    assert_eq!(status, StatusCode::OK, "the status page is public");
    let view: serde_json::Value = serde_json::from_slice(&body).expect("the status parses");
    let inventories = view["inventories"]
        .as_array()
        .expect("the inventories array");
    assert_eq!(inventories.len(), 3, "every inventory in the closed set");
    assert!(
        inventories.iter().all(|entry| entry["halted"] == false),
        "a fresh fleet has no halts"
    );
    // D1's badge, on the one surface that needs no session: the status page
    // renders a row per inventory whether or not a connection stands behind
    // it, so the branch must be readable there and not only from a
    // connection listing.
    let branches: Vec<(&str, &str)> = inventories
        .iter()
        .filter_map(|entry| Some((entry["inventory"].as_str()?, entry["transport"].as_str()?)))
        .collect();
    assert_eq!(
        branches,
        vec![
            ("Tes", "SellerDevice"),
            ("Etsy", "OfficialApi"),
            ("Tpt", "SellerDevice"),
        ],
        "every row carries the branch its marketplace falls in, and Etsy is \
         the only one of them that runs under a token we hold"
    );
}
