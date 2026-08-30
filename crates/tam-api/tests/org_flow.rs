//! The organisation settings surface over the wire: the read, the rename,
//! the refusals, and what a second tenant sees while the first renames itself.
//!
//! `/v1/org` carries no organisation identifier, so the strongest tenancy
//! statement provable at this surface is the one asserted here: each session
//! reads and writes its own row, and one tenant's rename leaves the other's
//! name exactly as it was.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::org::{OrgView, NAME_MAX_CHARS};
use tam_api::{router, APIError, APIErrorKind, AppState, Config, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
    }
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
            .expect("the org seeds");
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
            .mint(&token, user, Timestamp(100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).expect("the answer body parses")
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    method: Method,
    token: &SessionToken,
    body: Option<serde_json::Value>,
) -> Answer {
    let request = Request::builder().method(method).uri("/v1/org").header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
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
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer { status, body }
}

async fn name_of(pool: &PgPool, token: &SessionToken) -> String {
    let answer = call(pool.clone(), Method::GET, token, None).await;
    assert_eq!(answer.status, StatusCode::OK, "the settings read answers");
    answer.json::<OrgView>().name
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_read_answers_the_organisation_the_session_speaks_for(pool: PgPool) {
    provision(&pool).await;
    let answer = call(pool.clone(), Method::GET, &TOKEN_A, None).await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: OrgView = answer.json();
    assert_eq!(
        (view.id, view.name.as_str()),
        (ORG_A, "org-a"),
        "the read is scoped by the session, not by anything the request carried"
    );
    assert_eq!(
        name_of(&pool, &TOKEN_B).await,
        "org-b",
        "the second tenant's session reads its own row"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_rename_lands_and_the_other_tenant_is_untouched(pool: PgPool) {
    provision(&pool).await;
    let renamed = call(
        pool.clone(),
        Method::PATCH,
        &TOKEN_A,
        Some(serde_json::json!({ "name": "  Riverbend Resources  " })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK);
    assert_eq!(
        renamed.json::<OrgView>().name,
        "Riverbend Resources",
        "the answer states the trimmed name that was stored"
    );
    assert_eq!(
        name_of(&pool, &TOKEN_A).await,
        "Riverbend Resources",
        "the read reflects the rename"
    );
    assert_eq!(
        name_of(&pool, &TOKEN_B).await,
        "org-b",
        "one tenant's rename cannot reach another's row"
    );

    let other = call(
        pool.clone(),
        Method::PATCH,
        &TOKEN_B,
        Some(serde_json::json!({ "name": "Harbourview Teaching" })),
    )
    .await;
    assert_eq!(other.status, StatusCode::OK);
    assert_eq!(
        name_of(&pool, &TOKEN_A).await,
        "Riverbend Resources",
        "the second tenant's rename lands on its own row and no other"
    );
    assert_eq!(name_of(&pool, &TOKEN_B).await, "Harbourview Teaching");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refused_name_leaves_the_stored_one_standing(pool: PgPool) {
    provision(&pool).await;
    for name in [
        String::new(),
        "   \t ".to_owned(),
        "n".repeat(NAME_MAX_CHARS + 1),
    ] {
        let refused = call(
            pool.clone(),
            Method::PATCH,
            &TOKEN_A,
            Some(serde_json::json!({ "name": name })),
        )
        .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "a name of {} characters is the caller's error",
            name.chars().count()
        );
        let error: APIError = refused.json();
        assert_eq!(
            error.errors[0].kind,
            Some(APIErrorKind::Validation),
            "the refusal is the structured body's validation kind"
        );
    }
    assert_eq!(
        name_of(&pool, &TOKEN_A).await,
        "org-a",
        "nothing was written on any refused rename"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_is_closed_to_a_session_that_does_not_resolve(pool: PgPool) {
    provision(&pool).await;
    let unknown = SessionToken([0x43; 32]);
    for method in [Method::GET, Method::PATCH] {
        let body = (method == Method::PATCH).then(|| serde_json::json!({ "name": "anything" }));
        let refused = call(pool.clone(), method.clone(), &unknown, body).await;
        assert_eq!(
            refused.status,
            StatusCode::UNAUTHORIZED,
            "{method} without a live session is refused before any handler runs"
        );
    }
    assert_eq!(
        name_of(&pool, &TOKEN_A).await,
        "org-a",
        "the refused rename wrote nothing"
    );
}
