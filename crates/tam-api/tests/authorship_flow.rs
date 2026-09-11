//! The seller's authorship declaration over the wire: what the write stores,
//! what the connections list serves back, and the fence between tenants.
//!
//! The fence is the half worth testing hardest, and for a specific reason.
//! `ConnectionFactsRepo::authorship_for` once read without a tenant pin —
//! correctly for the cross-tenant lease scan it was written for, wrongly once
//! the API path called it — and nothing caught it because no test had driven
//! that path. The console read added beside it must not repeat that, so this
//! file asserts a second tenant sees none of the first's declaration through
//! the surface a seller actually reads.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resources::{AuthorshipView, ConnectionsView};
use tam_api::{router, APIError, AppState, Config, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const LAPTOP: &str = "11112222333344445555666677778888";
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

async fn declare(pool: PgPool, token: &SessionToken, name: &str) -> StatusCode {
    call(
        pool,
        token,
        Method::POST,
        "/v1/connections/Tpt/authorship",
        Some(serde_json::json!({ "name": name })),
    )
    .await
    .0
}

/// A registered device reporting one connected session for `marketplace`.
///
/// This is how the file builds a connection row no declaration was made
/// against: the check-in derives the link, creating the row where none stood.
/// Without such a row the undeclared case cannot be asserted at all, because
/// the list wraps every row it finds in `Some` — so a file holding only
/// declared rows stays green with the `Undeclared` arm broken.
async fn connected(pool: &PgPool, token: &SessionToken, marketplace: &str) {
    let (registered, _body) = call(
        pool.clone(),
        token,
        Method::POST,
        "/v1/devices",
        Some(serde_json::json!({
            "id": LAPTOP,
            "name": "staffroom-laptop",
            "os": "windows",
            "arch": "x86_64",
            "app_version": "0.1.0",
        })),
    )
    .await;
    assert_eq!(
        registered,
        StatusCode::OK,
        "the fixture's device registers, or the check-in after it reaches nothing"
    );
    let (checked_in, _body) = call(
        pool.clone(),
        token,
        Method::POST,
        &format!("/v1/devices/{LAPTOP}/heartbeat"),
        Some(serde_json::json!({
            "sessions": [
                { "marketplace": marketplace, "account_label": null, "status": "connected" },
            ]
        })),
    )
    .await;
    assert_eq!(
        checked_in,
        StatusCode::OK,
        "the fixture's check-in must land, or the connection row it derives never exists"
    );
}

async fn connections(pool: PgPool, token: &SessionToken) -> ConnectionsView {
    let (status, body) = call(pool, token, Method::GET, "/v1/connections", None).await;
    assert_eq!(status, StatusCode::OK, "the connections list reads");
    parse(&body)
}

/// The name a row declares, or `None` for a row that is served and undeclared.
///
/// Panics on the not-served value, which belongs to the operator surface and
/// must never reach this endpoint. Distinguishing the two is the whole point of
/// the three-state shape, so a helper that quietly treated them alike would
/// defeat it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn declared_name(view: &ConnectionsView) -> Option<String> {
    view.connections.iter().find_map(|row| {
        let held = row
            .authorship
            .as_ref()
            .expect("the seller's own surface always says which state a declaration is in");
        match held {
            AuthorshipView::Declared { name, .. } => Some(name.clone()),
            AuthorshipView::Undeclared => None,
        }
    })
}

/// The declaration a seller made is the one the page shows them back.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_declaration_is_served_back_on_the_connection_it_was_made_for(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    assert_eq!(
        declare(pool.clone(), &TOKEN_A, "A Teacher").await,
        StatusCode::OK
    );

    let view = connections(pool, &TOKEN_A).await;
    let tpt = view
        .connections
        .iter()
        .find(|row| row.marketplace == tam_types::Marketplace::Tpt)
        .expect("declaring mints the connection row it is stored against");
    match tpt
        .authorship
        .as_ref()
        .expect("the seller's own surface always says which state a declaration is in")
    {
        AuthorshipView::Declared { name, attested_at } => {
            assert_eq!(name, "A Teacher");
            assert_eq!(*attested_at, NOW, "the instant is the server's own");
        }
        AuthorshipView::Undeclared => panic!("the declaration reaches the page that shows it"),
    }
}

/// One marketplace's declaration as the seller's own surface serves it.
///
/// Panics where the list carries no row for the marketplace, and where a row
/// carries no declaration state at all: the first is the fixture failing to
/// build the case under test and the second is the not-served value, which
/// belongs to the operator surface. Reading either as "not declared" would
/// make an assertion about the `Undeclared` arm pass without reaching it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn authorship_of(view: &ConnectionsView, marketplace: tam_types::Marketplace) -> &AuthorshipView {
    view.connections
        .iter()
        .find(|row| row.marketplace == marketplace)
        .expect("the connection row this case is about is listed")
        .authorship
        .as_ref()
        .expect("the seller's own surface always says which state a declaration is in")
}

/// A marketplace the seller has not declared for is `undeclared` rather than
/// absent: absent is what the operator surface serves, and the two must not
/// read alike.
///
/// Both arms are driven against rows that exist. The device's check-in creates
/// a Tes connection nothing has been declared against, beside the Tpt one the
/// declaration mints, so the `Undeclared` arm is reached rather than assumed.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_undeclared_marketplace_says_so_rather_than_saying_nothing(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    // Two marketplaces, so the tenant needs a plan that connects more than
    // one: the free allowance is a catalogue rather than a crosslister, and
    // adopting a second marketplace on it is refused by the plan gate.
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            ORG_A,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Subscriber,
                rung: None,
                granted_by: tam_storage::GrantedBy::Paddle,
                grantor_user: None,
                reason: None,
                source_ref: Some("sub_authorship"),
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("the fixture grant seeds: {error}"));
    connected(&pool, &TOKEN_A, "Tes").await;
    assert_eq!(
        declare(pool.clone(), &TOKEN_A, "A Teacher").await,
        StatusCode::OK
    );

    let view = connections(pool, &TOKEN_A).await;
    let tes = authorship_of(&view, tam_types::Marketplace::Tes);
    assert!(
        matches!(tes, AuthorshipView::Undeclared),
        "a connection nothing was declared against says so: {tes:?}"
    );
    let tpt = authorship_of(&view, tam_types::Marketplace::Tpt);
    assert!(
        matches!(tpt, AuthorshipView::Declared { .. }),
        "and the declared row beside it still reads declared: {tpt:?}"
    );
    assert!(
        view.connections.iter().all(|row| row.authorship.is_some()),
        "every row on the seller's own surface states which of the two it is"
    );
}

/// The console read must not repeat the unpinned read that once served the
/// claim: a second tenant sees none of the first's declaration.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_declaration_is_invisible_to_another(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    declare(pool.clone(), &TOKEN_A, "A Teacher").await;

    let theirs = connections(pool.clone(), &TOKEN_B).await;
    assert_eq!(
        declared_name(&theirs),
        None,
        "tenant b sees no declaration of tenant a's, whatever rows it holds"
    );

    declare(pool.clone(), &TOKEN_B, "Another Teacher").await;
    assert_eq!(
        declared_name(&connections(pool.clone(), &TOKEN_B).await).as_deref(),
        Some("Another Teacher"),
        "tenant b's own declaration reaches tenant b"
    );
    assert_eq!(
        declared_name(&connections(pool, &TOKEN_A).await).as_deref(),
        Some("A Teacher"),
        "tenant b's write did not reach tenant a's row"
    );
}

/// Declaring again replaces, so the page shows the replacement rather than two
/// declarations or the first one.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn declaring_again_replaces_what_stands(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    declare(pool.clone(), &TOKEN_A, "First Name").await;
    declare(pool.clone(), &TOKEN_A, "Second Name").await;

    assert_eq!(
        declared_name(&connections(pool, &TOKEN_A).await).as_deref(),
        Some("Second Name"),
        "one declaration, the newest"
    );
}

/// A marketplace whose automation runs server-side under our own token refuses
/// the declaration: no device composes a write for it and nothing would read
/// the declaration back.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sanctioned_marketplace_refuses_a_declaration(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let (status, _body) = call(
        pool.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/connections/Etsy/authorship",
        Some(serde_json::json!({ "name": "A Teacher" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        declared_name(&connections(pool, &TOKEN_A).await),
        None,
        "the refused declaration stored nothing"
    );
}

/// The path segment is the marketplace's own serde name, and any other
/// spelling is not found rather than quietly matched.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_marketplace_spelled_any_other_way_is_not_found(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    for path in [
        "/v1/connections/tpt/authorship",
        "/v1/connections/TPT/authorship",
        "/v1/connections/teacherspayteachers/authorship",
    ] {
        let (status, body) = call(
            pool.clone(),
            &TOKEN_A,
            Method::POST,
            path,
            Some(serde_json::json!({ "name": "A Teacher" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        let error: APIError = parse(&body);
        assert_eq!(
            error.errors[0].code,
            Some(tam_api::APIErrorCode::ResourceMissing),
            "{path}"
        );
    }
}
