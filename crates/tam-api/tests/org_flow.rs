//! The organisation settings surface over the wire: the read, the rename, the
//! slug claim, the refusals, and what a second tenant sees while the first
//! renames itself.
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
use tam_api::org::{OrgView, SlugAvailability, SlugPrompt, NAME_MAX_CHARS};
use tam_api::{
    router, APIError, APIErrorCode, APIErrorKind, AppState, Config, Whoami, SESSION_COOKIE,
};
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn probe(pool: PgPool, token: &SessionToken, slug: &str) -> Answer {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/v1/org/slug/{slug}"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
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
    Answer { status, body }
}

async fn view_of(pool: &PgPool, token: &SessionToken) -> OrgView {
    let answer = call(pool.clone(), Method::GET, token, None).await;
    assert_eq!(answer.status, StatusCode::OK, "the settings read answers");
    answer.json()
}

async fn claim(pool: &PgPool, token: &SessionToken, slug: &str) -> Answer {
    call(
        pool.clone(),
        Method::PATCH,
        token,
        Some(serde_json::json!({ "slug": slug })),
    )
    .await
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_claim_stores_the_slug_and_settles_the_prompt(pool: PgPool) {
    provision(&pool).await;
    let before = view_of(&pool, &TOKEN_A).await;
    assert_eq!(before.slug, None, "a fresh organisation carries no slug");
    assert_eq!(
        before.slug_prompt,
        SlugPrompt::Claim,
        "an organisation created since the slug existed meets the gate"
    );

    let claimed = claim(&pool, &TOKEN_A, "  RiverBend-Resources  ").await;
    assert_eq!(claimed.status, StatusCode::OK);
    let view: OrgView = claimed.json();
    assert_eq!(
        view.slug.as_deref(),
        Some("riverbend-resources"),
        "the answer states the normalised slug that was stored, not the one typed"
    );
    assert_eq!(
        view.slug_prompt,
        SlugPrompt::Settled,
        "a claimed slug asks the console for nothing further"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_A).await.slug.as_deref(),
        Some("riverbend-resources"),
        "the read reflects the claim"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_B).await.slug,
        None,
        "one tenant's claim cannot reach another's row"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_slug_another_tenant_holds_is_refused_and_nothing_is_written(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend").await.status,
        StatusCode::OK
    );

    let refused = claim(&pool, &TOKEN_B, "riverbend").await;
    assert_eq!(
        refused.status,
        StatusCode::CONFLICT,
        "a collision is not the caller's malformed input; it is someone else being first"
    );
    let error: APIError = refused.json();
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::OrgSlugTaken),
        "the console renders its own sentence from this code"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_A).await.slug.as_deref(),
        Some("riverbend"),
        "the refusal left the holder's slug exactly as it was"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_B).await.slug,
        None,
        "the refused claim wrote nothing on the refused tenant either"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_uniqueness_is_case_insensitive(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend").await.status,
        StatusCode::OK
    );
    // Without the index being on `lower(slug)` this passes: the stored value
    // is lowercased on the way in, so a plain unique index would compare
    // "riverbend" against "riverbend" and never see the difference this
    // assertion exists to catch -- a row written by any other path.
    sqlx::query("UPDATE organisation SET slug = 'RiverBend' WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(&pool)
        .await
        .expect_err("the shape CHECK refuses an uppercase slug written directly");

    let refused = claim(&pool, &TOKEN_B, "RIVERBEND").await;
    assert_eq!(
        refused.status,
        StatusCode::CONFLICT,
        "a slug differing only in case is the same slug"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_reserved_slug_is_a_policy_refusal_rather_than_a_collision(pool: PgPool) {
    provision(&pool).await;
    let refused = claim(&pool, &TOKEN_A, "admin").await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a reserved word is refused for what it is, not for who holds it"
    );
    let error: APIError = refused.json();
    assert_eq!(
        error.errors[0].kind,
        Some(APIErrorKind::Validation),
        "the console renders a reserved word beside the field, as it does a malformed one"
    );
    assert_eq!(
        error.errors[0].code, None,
        "no collision code, because nothing collided"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_A).await.slug,
        None,
        "the refused claim wrote nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refused_slug_does_not_take_the_name_with_it(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend").await.status,
        StatusCode::OK
    );

    let refused = call(
        pool.clone(),
        Method::PATCH,
        &TOKEN_B,
        Some(serde_json::json!({ "name": "Harbourview Teaching", "slug": "riverbend" })),
    )
    .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(
        name_of(&pool, &TOKEN_B).await,
        "org-b",
        "the name and the slug are one statement: a refused slug rolls the name back with it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_change_naming_nothing_is_refused(pool: PgPool) {
    provision(&pool).await;
    let refused = call(
        pool.clone(),
        Method::PATCH,
        &TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a body naming neither field is the caller's error rather than a silent no-op"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_released_slug_is_claimable_by_another_tenant(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend").await.status,
        StatusCode::OK
    );
    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend-two").await.status,
        StatusCode::OK
    );
    assert_eq!(
        claim(&pool, &TOKEN_B, "riverbend").await.status,
        StatusCode::OK,
        "the old slug is not reserved, which is the decision made testable"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_A).await.slug.as_deref(),
        Some("riverbend-two"),
        "the renaming tenant keeps its new slug"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_availability_route_is_closed_to_a_caller_with_no_session(pool: PgPool) {
    provision(&pool).await;
    let refused = probe(pool.clone(), &SessionToken([0x43; 32]), "riverbend").await;
    assert_eq!(
        refused.status,
        StatusCode::UNAUTHORIZED,
        "the verdict is a directory of every tenant's slug, so it is behind the session"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn availability_answers_the_holder_and_the_free_name_apart(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend").await.status,
        StatusCode::OK
    );

    let taken = probe(pool.clone(), &TOKEN_B, "riverbend").await;
    assert_eq!(taken.status, StatusCode::OK);
    let verdict: SlugAvailability = taken.json();
    assert!(
        !verdict.available,
        "a slug another tenant holds is not available"
    );
    assert_eq!(
        verdict.slug, "riverbend",
        "the verdict names the normalised slug it judged"
    );

    let free = probe(pool.clone(), &TOKEN_B, "Harbourview").await;
    assert_eq!(free.status, StatusCode::OK);
    let verdict: SlugAvailability = free.json();
    assert!(verdict.available, "an unclaimed slug is available");
    assert_eq!(
        verdict.slug, "harbourview",
        "the console can show the seller the lowercased form it would get"
    );

    let own = probe(pool.clone(), &TOKEN_A, "riverbend").await;
    let verdict: SlugAvailability = own.json();
    assert!(
        verdict.available,
        "a seller re-typing the slug they already hold is told it is free, which is \
         what writing it again would do"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn availability_refuses_a_malformed_slug_rather_than_calling_it_taken(pool: PgPool) {
    provision(&pool).await;
    for slug in ["ab", "ab_c", "admin"] {
        let refused = probe(pool.clone(), &TOKEN_A, slug).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{slug:?} is refused for its shape or for policy, never answered unavailable"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_organisation_that_predates_the_slug_meets_a_banner_rather_than_a_gate(pool: PgPool) {
    provision(&pool).await;
    // What migration 0057's one-shot UPDATE wrote for every row that already
    // existed. A test database is created by replaying the migrations against
    // an empty schema, so that UPDATE touches nothing here and the state it
    // produces has to be seeded to be exercised at all.
    sqlx::query("UPDATE organisation SET slug_deferred = true WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await
        .expect("the deferral seeds");

    assert_eq!(
        view_of(&pool, &TOKEN_A).await.slug_prompt,
        SlugPrompt::Claim,
        "an organisation created since the slug existed meets the gate"
    );
    assert_eq!(
        view_of(&pool, &TOKEN_B).await.slug_prompt,
        SlugPrompt::Banner,
        "one that predates it meets a banner it can dismiss"
    );

    assert_eq!(
        claim(&pool, &TOKEN_B, "harbourview").await.status,
        StatusCode::OK
    );
    assert_eq!(
        view_of(&pool, &TOKEN_B).await.slug_prompt,
        SlugPrompt::Settled,
        "claiming settles the prompt whichever state it was in"
    );
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn whoami(pool: PgPool, token: &SessionToken) -> Whoami {
    let request = Request::builder()
        .method(Method::GET)
        .uri("/v1/whoami")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(response.status(), StatusCode::OK, "whoami answers");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    serde_json::from_slice(&body).expect("the body is whoami")
}

/// The console reads `/v1/whoami` on its route load, before it renders
/// anything, so this is where its gate is answered. A slug carried only by
/// `/v1/org` would be read after the console had already drawn.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn whoami_carries_the_slug_and_the_prompt_it_calls_for(pool: PgPool) {
    provision(&pool).await;
    let before = whoami(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        (before.slug, before.slug_prompt),
        (None, SlugPrompt::Claim),
        "an unclaimed organisation created since the slug existed meets the gate"
    );

    assert_eq!(
        claim(&pool, &TOKEN_A, "riverbend").await.status,
        StatusCode::OK
    );

    let after = whoami(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        (after.slug.as_deref(), after.slug_prompt),
        (Some("riverbend"), SlugPrompt::Settled),
        "the claim is visible at the same route the gate is read from"
    );
    let other = whoami(pool.clone(), &TOKEN_B).await;
    assert_eq!(
        (other.slug, other.slug_prompt),
        (None, SlugPrompt::Claim),
        "one tenant's claim says nothing about another's session"
    );
}
