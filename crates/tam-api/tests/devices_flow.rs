//! The device registry over the wire: a device registers, checks in, is signed
//! out from the console, and learns of it on its next check-in.
//!
//! `/v1/devices` carries no organisation identifier, so the strongest tenancy
//! statement provable at this surface is the one asserted here: each session
//! lists and signs out its own machines, and one tenant naming another's device
//! id reaches nothing.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::devices::{DeviceView, DevicesView, HeartbeatView};
use tam_api::{
    router, APIError, APIErrorCode, APIErrorKind, AppState, Config, WallClock, SESSION_COOKIE,
};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{Marketplace, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const LAPTOP: &str = "11112222333344445555666677778888";
const NOW: Timestamp = Timestamp(1_756_000_000_000);

/// Three fixed instants, one per call that needs to be distinguishable from
/// the one before it. Separate functions rather than one mutable clock because
/// `wall` is a function pointer and cannot capture, and a static holding the
/// step would be state shared between tests.
fn t0() -> Timestamp {
    NOW
}

fn t1() -> Timestamp {
    Timestamp(NOW.0 + 60_000)
}

fn t2() -> Timestamp {
    Timestamp(NOW.0 + 120_000)
}

fn state(pool: PgPool, wall: WallClock) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall,
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
            .mint(&token, user, Timestamp(9_000_000_000_000), Timestamp(1_000))
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

/// One request, as a value rather than as five positional arguments: the
/// middle three are a path, a token and an optional body, which is exactly the
/// shape a transposition survives compilation in.
struct Call<'a> {
    method: Method,
    path: &'a str,
    token: &'a SessionToken,
    body: Option<serde_json::Value>,
    /// The instant this request is served at, so a revocation and the check-in
    /// after it are distinguishable.
    wall: WallClock,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(pool: PgPool, call: Call<'_>) -> Answer {
    let request = Request::builder()
        .method(call.method)
        .uri(call.path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", call.token.to_hex()),
        );
    let request = match call.body {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state(pool, call.wall))
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

fn registration(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "os": "windows",
        "arch": "x86_64",
        "app_version": "0.1.0",
    })
}

async fn register(pool: &PgPool, token: &SessionToken, id: &str, name: &str) -> Answer {
    call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: "/v1/devices",
            token,
            body: Some(registration(id, name)),
            wall: t0,
        },
    )
    .await
}

async fn beat(
    pool: &PgPool,
    token: &SessionToken,
    id: &str,
    sessions: serde_json::Value,
    wall: WallClock,
) -> Answer {
    call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{id}/heartbeat"),
            token,
            body: Some(serde_json::json!({ "sessions": sessions })),
            wall,
        },
    )
    .await
}

async fn revoke(pool: &PgPool, token: &SessionToken, id: &str, wall: WallClock) -> Answer {
    call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{id}/revoke"),
            token,
            body: None,
            wall,
        },
    )
    .await
}

async fn devices(pool: &PgPool, token: &SessionToken) -> Vec<DeviceView> {
    let answer = call(
        pool.clone(),
        Call {
            method: Method::GET,
            path: "/v1/devices",
            token,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the listing answers");
    answer.json::<DevicesView>().devices
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_registers_checks_in_and_appears_in_its_own_tenants_listing(pool: PgPool) {
    provision(&pool).await;

    let registered = register(&pool, &TOKEN_A, LAPTOP, "founder-pc").await;
    assert_eq!(registered.status, StatusCode::OK);
    let view: DeviceView = registered.json();
    assert_eq!(
        (view.id.as_str(), view.name.as_str()),
        (LAPTOP, "founder-pc")
    );
    assert!(
        !view.wipe_outstanding && view.revoked_at.is_none(),
        "a device nobody signed out has nothing outstanding"
    );

    let checked_in = beat(
        &pool,
        &TOKEN_A,
        LAPTOP,
        serde_json::json!([
            { "marketplace": "Tpt", "account_label": "Founder's Classroom", "status": "connected" },
            { "marketplace": "Tes", "account_label": null, "status": "connected" },
        ]),
        t0,
    )
    .await;
    assert_eq!(checked_in.status, StatusCode::OK);
    assert!(
        !checked_in.json::<HeartbeatView>().revoked,
        "the device is told it may keep working"
    );

    let listed = devices(&pool, &TOKEN_A).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0]
            .sessions
            .iter()
            .map(|session| (session.marketplace, session.status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (Marketplace::Tes, "connected"),
            (Marketplace::Tpt, "connected")
        ],
        "the page renders what the device reported, ordered by marketplace"
    );
    assert_eq!(
        listed[0].sessions[1].account_label.as_deref(),
        Some("Founder's Classroom")
    );

    assert!(
        devices(&pool, &TOKEN_B).await.is_empty(),
        "the second tenant's session lists its own machines, and it has none"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn signing_a_device_out_reaches_it_on_its_next_check_in(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "founder-pc").await;
    beat(
        &pool,
        &TOKEN_A,
        LAPTOP,
        serde_json::json!([{ "marketplace": "Tpt", "account_label": null, "status": "connected" }]),
        t0,
    )
    .await;

    let revoked = revoke(&pool, &TOKEN_A, LAPTOP, t1).await;
    assert_eq!(revoked.status, StatusCode::OK);
    let view: DeviceView = revoked.json();
    assert_eq!(view.revoked_at, Some(t1()));
    assert!(
        view.wipe_outstanding,
        "the device has not been heard from since, so it may still hold its cookies"
    );

    let told = beat(
        &pool,
        &TOKEN_A,
        LAPTOP,
        serde_json::json!([{ "marketplace": "Tpt", "account_label": null, "status": "wiped" }]),
        t2,
    )
    .await;
    assert_eq!(told.status, StatusCode::OK);
    let answer: HeartbeatView = told.json();
    assert!(
        answer.revoked,
        "the heartbeat's answer is how the device learns to wipe"
    );
    assert_eq!(answer.revoked_at, Some(t1()));

    let listed = devices(&pool, &TOKEN_A).await;
    assert!(
        !listed[0].wipe_outstanding,
        "the check-in after the sign-out is the evidence the device complied"
    );
    assert_eq!(listed[0].sessions[0].status, "wiped");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenant_cannot_reach_another_tenants_device(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "founder-pc").await;
    beat(
        &pool,
        &TOKEN_A,
        LAPTOP,
        serde_json::json!([{ "marketplace": "Tes", "account_label": null, "status": "connected" }]),
        t0,
    )
    .await;

    let refused = revoke(&pool, &TOKEN_B, LAPTOP, t1).await;
    assert_eq!(
        refused.status,
        StatusCode::NOT_FOUND,
        "another tenant's device id names nothing this session can reach"
    );
    assert_eq!(
        refused.json::<APIError>().errors[0].code,
        Some(APIErrorCode::ResourceMissing)
    );

    let stolen = beat(&pool, &TOKEN_B, LAPTOP, serde_json::json!([]), t1).await;
    assert_eq!(
        stolen.status,
        StatusCode::NOT_FOUND,
        "nor can it heartbeat as that device and clear what the device holds"
    );

    let listed = devices(&pool, &TOKEN_A).await;
    assert_eq!(listed[0].revoked_at, None, "A's device was not signed out");
    assert_eq!(
        listed[0].sessions.len(),
        1,
        "and what it holds was not replaced"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_refuses_what_it_cannot_represent(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "founder-pc").await;

    let unknown_status = beat(
        &pool,
        &TOKEN_A,
        LAPTOP,
        serde_json::json!([{ "marketplace": "Tpt", "account_label": null, "status": "expired" }]),
        t0,
    )
    .await;
    assert_eq!(unknown_status.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unknown_status.json::<APIError>().errors[0].kind,
        Some(APIErrorKind::Validation)
    );

    let sanctioned = beat(
        &pool,
        &TOKEN_A,
        LAPTOP,
        serde_json::json!([{ "marketplace": "Etsy", "account_label": null, "status": "connected" }]),
        t0,
    )
    .await;
    assert_eq!(
        sanctioned.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a device claiming a session for a marketplace whose automation is server-side \
         is the two-branch rule being violated, not a row to record"
    );

    let unregistered = beat(&pool, &TOKEN_A, "not-a-device", serde_json::json!([]), t0).await;
    assert_eq!(
        unregistered.status,
        StatusCode::NOT_FOUND,
        "a heartbeat is not a registration"
    );

    assert_eq!(
        devices(&pool, &TOKEN_A).await[0].sessions.len(),
        0,
        "no refused report left a row behind"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_is_closed_to_a_session_that_does_not_resolve(pool: PgPool) {
    provision(&pool).await;
    let unknown = SessionToken([0x43; 32]);
    for (method, path, body) in [
        (Method::GET, "/v1/devices".to_owned(), None),
        (
            Method::POST,
            "/v1/devices".to_owned(),
            Some(registration(LAPTOP, "founder-pc")),
        ),
        (
            Method::POST,
            format!("/v1/devices/{LAPTOP}/heartbeat"),
            Some(serde_json::json!({ "sessions": [] })),
        ),
        (Method::POST, format!("/v1/devices/{LAPTOP}/revoke"), None),
    ] {
        let refused = call(
            pool.clone(),
            Call {
                method: method.clone(),
                path: &path,
                token: &unknown,
                body,
                wall: t0,
            },
        )
        .await;
        assert_eq!(
            refused.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} without a live session is refused before any handler runs"
        );
    }
    assert!(
        devices(&pool, &TOKEN_A).await.is_empty(),
        "the refused registration created nothing"
    );
}
