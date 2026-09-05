//! Renaming and removing a label over the wire.
//!
//! The label routes that write through a product -- `GET` and `PUT
//! /v1/products/{product}/labels` -- are exercised in `mappings_flow.rs`,
//! where the product fixture they need already lives. These two address a
//! label by its own text, which is the only identifier this surface gives a
//! client for one, so what they prove that the storage suite cannot is that
//! the text survives the path: a label with a space in it is reached under its
//! percent-encoded name and not by some other spelling.
//!
//! Two tenants, because the fence around a name is the interesting refusal: a
//! label another organisation holds must be not-found rather than forbidden,
//! or the route is an oracle for what other sellers call things.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resources::{LabelView, LabelsView};
use tam_api::{router, APIError, APIErrorCode, APIErrorKind, AppState, Config, SESSION_COOKIE};
use tam_storage::{Colour, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
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

/// One label, written the way the label writer writes it: the colour is the
/// one the name earns, so a fixture cannot seed a label the API could not have
/// made.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_label(pool: &PgPool, org: OrgId, name: &str, mark: u8) {
    let mut tx = pool.begin().await.expect("the fixture transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO label (org_id, id, name, colour, created_at) \
         VALUES ($1, $2, $3, $4, now())",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes([mark; 16]))
    .bind(name)
    .bind(Colour::of_name(name).as_str())
    .execute(&mut *tx)
    .await
    .expect("the label inserts");
    tx.commit().await.expect("the fixture transaction commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    token: Option<&SessionToken>,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
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

async fn names(pool: &PgPool, token: &SessionToken) -> Vec<String> {
    let (status, body) = call(pool.clone(), Some(token), Method::GET, "/v1/labels", None).await;
    assert_eq!(status, StatusCode::OK, "the vocabulary reads");
    parse::<LabelsView>(&body)
        .labels
        .into_iter()
        .map(|label| label.name)
        .collect()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_rename_answers_the_stored_label_and_refuses_a_name_already_in_use(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_label(&pool, ORG_A, "Autumn term", 0x11).await;
    seed_label(&pool, ORG_A, "Bundle", 0x12).await;

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::PATCH,
        "/v1/labels/Autumn%20term",
        Some(serde_json::json!({ "name": "  Term one  " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the rename lands");
    let renamed: LabelView = parse(&body);
    assert_eq!(
        (renamed.name.as_str(), renamed.colour.as_str()),
        ("Term one", Colour::of_name("Term one").as_str()),
        "the answer is the label as stored: the name trimmed, and the colour \
         the new name earns rather than the one the old name had"
    );
    assert_eq!(
        names(&pool, &TOKEN_A).await,
        vec!["Bundle".to_owned(), "Term one".to_owned()],
        "the vocabulary carries the new name and nothing of the old one"
    );

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::PATCH,
        "/v1/labels/Term%20one",
        Some(serde_json::json!({ "name": "bundle" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a name another label holds, in any case, is the seller's to change"
    );
    assert_eq!(
        parse::<APIError>(&body).errors[0].kind,
        Some(APIErrorKind::Validation),
        "the refusal carries the kind the dialog branches on"
    );

    let at_the_bound = "x".repeat(60);
    let over_the_bound = "x".repeat(61);
    for (what, path, sent, expected) in [
        (
            "a label this organisation does not have",
            "/v1/labels/Nothing%20like%20it",
            "Term two",
            StatusCode::NOT_FOUND,
        ),
        (
            "a new name that is blank",
            "/v1/labels/Term%20one",
            "   ",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "a new name over the sixty-character bound",
            "/v1/labels/Term%20one",
            over_the_bound.as_str(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        // A zero byte passes trimming and the length bound, and the column
        // cannot hold one, so without a check at the boundary this is a fault
        // rather than a refusal.
        (
            "a new name carrying a zero byte",
            "/v1/labels/Term%20one",
            "Term\u{0}two",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        // The address path is the reason: a label is reached as one path
        // segment, so a name a client could only send as %2F is a label that
        // could be made and then never renamed or deleted.
        (
            "a new name carrying a slash",
            "/v1/labels/Term%20one",
            "Term 1/2",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (status, _body) = call(
            pool.clone(),
            Some(&TOKEN_A),
            Method::PATCH,
            path,
            Some(serde_json::json!({ "name": sent })),
        )
        .await;
        assert_eq!(status, expected, "{what} is refused as such");
    }
    assert_eq!(
        names(&pool, &TOKEN_A).await,
        vec!["Bundle".to_owned(), "Term one".to_owned()],
        "no refused rename moved anything"
    );

    // The bound is pinned on both sides: one character too many is refused
    // above, and exactly sixty is accepted here, so lowering the bound fails
    // this and raising it fails that.
    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::PATCH,
        "/v1/labels/Term%20one",
        Some(serde_json::json!({ "name": at_the_bound })),
    )
    .await;
    assert_eq!(
        (status, parse::<LabelView>(&body).name.chars().count()),
        (StatusCode::OK, 60),
        "a name of exactly the bound is accepted and stored whole"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_delete_removes_one_label_and_stops_at_the_tenant_fence(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    seed_label(&pool, ORG_A, "Autumn term", 0x11).await;
    seed_label(&pool, ORG_A, "Bundle", 0x12).await;
    // Org B holds the same word, so a delete that ignored the pin and removed
    // the label by name across every organisation is caught by this test
    // rather than passing as a delete that found nothing.
    seed_label(&pool, ORG_B, "Autumn term", 0x21).await;

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_B),
        Method::DELETE,
        "/v1/labels/Bundle",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a label only another organisation holds is not found rather than \
         forbidden, so the route is no oracle for what other sellers call things"
    );
    assert_eq!(
        parse::<APIError>(&body).errors[0].code,
        Some(APIErrorCode::ResourceMissing),
        "in the same words a name nobody holds gets"
    );
    assert_eq!(
        names(&pool, &TOKEN_A).await,
        vec!["Autumn term".to_owned(), "Bundle".to_owned()],
        "the other tenant's call left this one's vocabulary alone"
    );

    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::DELETE,
        "/v1/labels/autumn%20TERM",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the label is addressed the way the unique index folds it, and nothing \
         of it survives to answer with"
    );
    assert_eq!(
        (names(&pool, &TOKEN_A).await, names(&pool, &TOKEN_B).await),
        (vec!["Bundle".to_owned()], vec!["Autumn term".to_owned()]),
        "the named label is gone from the organisation that asked, the rest of \
         its vocabulary stands, and the other tenant's label of the same name \
         is untouched"
    );

    let (status, _body) = call(
        pool,
        Some(&TOKEN_A),
        Method::DELETE,
        "/v1/labels/Autumn%20term",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "asking again is answered rather than reported as a success that did nothing"
    );
}
