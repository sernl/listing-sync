//! The seller's own projection overrides over the wire: what the route records,
//! what it refuses before writing, and the tenant fence around the listing.
//!
//! The refusals are the interesting half. Two of them exist because a value
//! that reached the database would be wrong in a way nothing downstream could
//! correct: a licence override the CHECK would reject anyway, and a native
//! identifier of a shape the marketplace does not issue, which would be
//! discovered only when a listing carrying it was refused.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resources::OverridesView;
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::{CanonicalTerm, TermKind};
use tam_storage::{SessionRepo, SessionToken, TaxonomyRepo};
use tam_types::{CanonicalTermId, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const TERM: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const NOW: Timestamp = Timestamp(5_000);

const PATH: &str = "/v1/mappings/overrides";

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

/// The canonical taxonomy an override points out of.
///
/// `projection_override.from_term` carries a foreign key onto `canonical_term`,
/// which is global rather than per tenant, so it is seeded once per test
/// database rather than per organisation.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_term(pool: &PgPool) {
    TaxonomyRepo::new(pool.clone())
        .seed(
            &[CanonicalTerm {
                id: TERM,
                kind: TermKind::Subject,
                parent: None,
                label: "Fractions".to_owned(),
            }],
            &[],
        )
        .await
        .expect("the term seeds");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    token: Option<&SessionToken>,
    method: Method,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(PATH);
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

fn override_body(kind: &str) -> serde_json::Value {
    serde_json::json!({
        "inventory": "Tpt",
        "axis": "subject",
        "from_term": TERM.0.to_hyphenated(),
        "to": { "segments": ["Maths"], "native_id": "math-elementary" },
        "kind": kind,
    })
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_override_records_and_lists_back(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        Some(override_body("broader")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    assert_eq!(status, StatusCode::OK);
    let view: OverridesView = parse(&body);
    assert_eq!(view.overrides.len(), 1);
    assert_eq!(view.overrides[0].from_term, TERM);
    assert_eq!(view.overrides[0].segments, vec!["Maths".to_owned()]);
    assert_eq!(
        view.overrides[0].kind, "broader",
        "the kind the seller chose survives the round trip"
    );
}

/// A licence is a legal statement about the work rather than a mapping choice.
/// The constructor refuses it and the database CHECK refuses it; the seller is
/// told which, rather than meeting a five-hundred from the second.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_licence_override_is_refused_as_validation(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["axis"] = serde_json::json!("licence");
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "the refused override wrote nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_empty_path_is_refused_as_validation(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["to"] = serde_json::json!({ "segments": [] });
    let (status, _body) = call(pool, Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// The check that runs before the write rather than at the listing: TPT
/// addresses its tag namespace by slug, and an identifier of another shape
/// would be discovered only when a listing carrying it was refused.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_native_identifier_of_the_wrong_shape_is_refused_before_the_write(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    for native in ["1234", "Math-Elementary", "math elementary", ""] {
        let mut body = override_body("exact");
        body["to"] = serde_json::json!({ "segments": ["Maths"], "native_id": native });
        let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "not a slug TPT issues: {native:?}"
        );
    }

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "every refusal happened before anything was written"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_withdrawal_removes_one_and_forgives_a_miss(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;
    call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        Some(override_body("exact")),
    )
    .await;

    let withdraw = serde_json::json!({
        "inventory": "Tpt",
        "axis": "subject",
        "from_term": TERM.0.to_hyphenated(),
    });
    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::DELETE,
        Some(withdraw.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_status, body) = call(pool.clone(), Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(view.overrides.is_empty(), "the override is gone");

    let (status, _body) = call(pool, Some(&TOKEN_A), Method::DELETE, Some(withdraw)).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "withdrawing what is already absent is the state the seller asked for, not an error"
    );
}

/// The organisation comes from the session and from nowhere else, so one
/// tenant's overrides are invisible to another and neither can write the
/// other's.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_overrides_are_invisible_to_another(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;

    call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        Some(override_body("exact")),
    )
    .await;

    let (status, body) = call(pool.clone(), Some(&TOKEN_B), Method::GET, None).await;
    assert_eq!(status, StatusCode::OK);
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "tenant b sees none of tenant a's overrides"
    );

    // The same term, from the other tenant: one override each, not one shared.
    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_B),
        Method::POST,
        Some(override_body("broader")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert_eq!(
        view.overrides[0].kind, "exact",
        "tenant b's write did not touch tenant a's answer for the same term"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_is_closed_without_a_session(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    for (method, body) in [
        (Method::POST, Some(override_body("exact"))),
        (Method::GET, None),
        (
            Method::DELETE,
            Some(serde_json::json!({
                "inventory": "Tpt",
                "axis": "subject",
                "from_term": TERM.0.to_hyphenated(),
            })),
        ),
    ] {
        let (status, _body) = call(pool.clone(), None, method.clone(), body).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} without a session"
        );
    }
}

/// The bound on what a stranger may make this server write. The domain refuses
/// an empty path because it names nothing; this refuses an enormous one because
/// nobody's vocabulary is that deep and the array is otherwise the caller's to
/// fill.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_oversized_path_is_refused_before_the_write(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut deep = override_body("exact");
    deep["to"] = serde_json::json!({
        "segments": (0..9).map(|n| format!("level {n}")).collect::<Vec<_>>(),
        "native_id": "math-elementary",
    });
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(deep)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "nine segments is deeper than any marketplace publishes"
    );

    let mut long = override_body("exact");
    long["to"] = serde_json::json!({
        "segments": ["x".repeat(201)],
        "native_id": "math-elementary",
    });
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(long)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "one segment longer than two hundred characters"
    );

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "both refusals happened before anything was written"
    );
}

/// A term the taxonomy does not hold reaches the foreign key, and the seller is
/// told to pick again rather than shown a fault of ours. Reachable from an
/// ordinary client: the screen picks from a cached list, so a tab open across a
/// taxonomy change can name a term that has since gone.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_term_the_taxonomy_does_not_hold_is_refused_rather_than_faulting(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["from_term"] = serde_json::json!(Uuid([0x66; 16]).to_hyphenated());
    let (status, _body) = call(pool, Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown term is the caller's to correct, not a five-hundred"
    );
}

/// An axis the marketplace does not bind is refused by the constructor, not by
/// the database: Etsy carries no equivalence axis at all, so an override for
/// one would be a durable answer to a question that marketplace never asks.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_axis_the_marketplace_does_not_bind_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["inventory"] = serde_json::json!("Etsy");
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "the refusal happened before anything was written"
    );
}
