//! The seller-device consent over the wire: what a grant records, what the
//! list serves back, what is refused as not an agreement, and the gate every
//! mint of seller-device work reads through.
//!
//! The gate is driven through the import-run start rather than through a
//! repository call, because the whole point is that a request which would
//! start work on the seller's device is answered `consent_required` before
//! anything is written.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::consent::{ConsentView, ConsentsView};
use tam_api::{router, APIError, APIErrorCode, AppState, Config, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid, CONSENT_NOTICE_VERSION};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

fn state(pool: PgPool) -> AppState {
    AppState {
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
async fn provision(pool: &PgPool, org: OrgId, user: UserId, token: &SessionToken, name: &str) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(name)
        .execute(pool)
        .await
        .expect("the org seeds");
    // Reading a shop is a paid capability; the fixture subscribes so the
    // import start is answered by the consent gate rather than the plan gate.
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            org,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Subscriber,
                rung: None,
                granted_by: tam_storage::GrantedBy::Paddle,
                grantor_user: None,
                reason: None,
                source_ref: Some(name),
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("the fixture grant seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(org, user, &format!("{name}@example.test"), Timestamp(1_000))
        .await
        .expect("the user provisions");
    sessions
        .mint(token, user, Timestamp(100_000), Timestamp(1_000))
        .await
        .expect("the session mints");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    token: &SessionToken,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    let request = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            request.body(Body::from(json.to_string()))
        }
        None => request.body(Body::empty()),
    }
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).expect("the body parses")
}

async fn grant(
    pool: PgPool,
    token: &SessionToken,
    marketplace: &str,
    agreed: bool,
) -> (StatusCode, Vec<u8>) {
    call(
        pool,
        token,
        Method::POST,
        &format!("/v1/consents/{marketplace}"),
        Some(serde_json::json!({ "notice_version": CONSENT_NOTICE_VERSION, "agreed": agreed })),
    )
    .await
}

async fn consents(pool: PgPool, token: &SessionToken) -> ConsentsView {
    let (status, body) = call(pool, token, Method::GET, "/v1/consents", None).await;
    assert_eq!(status, StatusCode::OK, "the consent record lists");
    parse(&body)
}

/// A start of a Tpt shop read, which is seller-device work from its first
/// page and the mint the gate is asserted through.
async fn start_tpt_run(pool: PgPool, token: &SessionToken) -> (StatusCode, Vec<u8>) {
    call(
        pool,
        token,
        Method::POST,
        "/v1/imports/runs",
        Some(serde_json::json!({
            "source": "Tpt",
            "start_key": uuid::Uuid::new_v4().hyphenated().to_string(),
        })),
    )
    .await
}

fn code_of(body: &[u8]) -> Option<APIErrorCode> {
    parse::<APIError>(body)
        .errors
        .first()
        .and_then(|entry| entry.code)
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_grant_is_listed_standing_and_gates_the_mint(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;

    let (status, body) = start_tpt_run(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        code_of(&body),
        Some(APIErrorCode::ConsentRequired),
        "the start without a grant is refused by the consent gate"
    );
    let refusal: APIError = parse(&body);
    assert_eq!(
        refusal.errors[0].detail,
        Some(serde_json::json!({ "marketplace": "Tpt", "notice_version": CONSENT_NOTICE_VERSION }))
    );

    let (status, body) = grant(pool.clone(), &TOKEN_A, "Tpt", true).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: ConsentView = parse(&body);
    assert!(view.standing, "a grant on the current notice stands");
    assert_eq!(
        view.notice_version, CONSENT_NOTICE_VERSION,
        "the grant carries the version it was read on"
    );
    assert_eq!(view.granted_at, NOW, "the grant is stamped by the server");

    let listed = consents(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        listed.notice_version, CONSENT_NOTICE_VERSION,
        "the list names the version a grant must carry"
    );
    assert_eq!(listed.consents, vec![view], "the list shows the grant");

    // Past the consent gate, the start meets the next one: no machine of
    // this fixture's holds a Tpt login. That refusal is the proof the grant
    // was read, without standing up a device and a heartbeat here.
    let (status, body) = start_tpt_run(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "after the grant the start is refused for want of a login, not of consent: {}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        code_of(&body),
        None,
        "that refusal is not the consent gate's"
    );
    assert!(
        String::from_utf8_lossy(&body).contains("Connect this marketplace"),
        "the refusal is the missing login"
    );

    // Per organisation: the second tenant sees no grant and is refused.
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    assert!(
        consents(pool.clone(), &TOKEN_B).await.consents.is_empty(),
        "a grant is per organisation"
    );
    let (status, body) = start_tpt_run(pool.clone(), &TOKEN_B).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the other tenant is refused");
    assert_eq!(
        code_of(&body),
        Some(APIErrorCode::ConsentRequired),
        "by the consent gate"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn only_an_explicit_agreement_on_the_current_notice_is_recorded(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;

    let (status, _) = grant(pool.clone(), &TOKEN_A, "Tpt", false).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "agreed:false is not an agreement"
    );

    let (status, _) = call(
        pool.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/consents/Tpt",
        Some(serde_json::json!({ "notice_version": "2020-01-01", "agreed": true })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an out-of-date notice is not the current one"
    );

    let (status, _) = grant(pool.clone(), &TOKEN_A, "Etsy", true).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an official-API marketplace needs no grant"
    );

    let (status, _) = grant(pool.clone(), &TOKEN_A, "tpt", true).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the path spells the marketplace as the wire does"
    );

    assert!(
        consents(pool.clone(), &TOKEN_A).await.consents.is_empty(),
        "none of those wrote a row"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_withdrawal_closes_the_gate_again(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let (status, _) = grant(pool.clone(), &TOKEN_A, "Tpt", true).await;
    assert_eq!(status, StatusCode::OK, "the grant lands");

    let (status, body) = call(
        pool.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/consents/Tpt/withdraw",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: ConsentView = parse(&body);
    assert!(!view.standing, "a withdrawn grant does not stand");
    assert_eq!(view.withdrawn_at, Some(NOW), "the withdrawal is stamped");

    let (status, body) = start_tpt_run(pool.clone(), &TOKEN_A).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "the gate is closed again");
    assert_eq!(
        code_of(&body),
        Some(APIErrorCode::ConsentRequired),
        "by the consent gate"
    );

    // A second withdrawal is idempotent and still says nothing stands.
    let (status, body) = call(
        pool.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/consents/Tpt/withdraw",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a second withdrawal is not a fault");
    assert!(
        !parse::<ConsentView>(&body).standing,
        "and still says nothing stands"
    );

    let listed = consents(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        listed.consents.len(),
        1,
        "the withdrawn grant stays in the record"
    );
    assert!(!listed.consents[0].standing, "as withdrawn");
}
