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
use tam_types::{FileBytes, Marketplace, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const LAPTOP: &str = "11112222333344445555666677778888";
/// A second machine of the same seller's, for the facts that are about there
/// being more than one: the link is existential over devices, and the
/// declaration outlives any of them.
const DESKTOP: &str = "99998888777766665555444433332222";
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

/// The heartbeat that leaves a device holding a session for both device-branch
/// marketplaces.
///
/// Registration alone no longer admits a claim: the claim serves a device only
/// work whose marketplace it holds a connected session for, which is what the
/// real device establishes on its first check-in.
async fn connected(pool: &PgPool, token: &SessionToken, id: &str) {
    let answered = beat(
        pool,
        token,
        id,
        serde_json::json!([
            { "marketplace": "Tes", "account_label": null, "status": "connected" },
            { "marketplace": "Tpt", "account_label": null, "status": "connected" },
        ]),
        t0,
    )
    .await;
    assert_eq!(
        answered.status,
        StatusCode::OK,
        "the fixture's check-in must land, or the claim it precedes proves nothing"
    );
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

/// The seller's declaration of authorship, over the wire.
async fn declare(
    pool: &PgPool,
    token: &SessionToken,
    marketplace: &str,
    name: &str,
    wall: WallClock,
) -> Answer {
    call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/connections/{marketplace}/authorship"),
            token,
            body: Some(serde_json::json!({ "name": name })),
            wall,
        },
    )
    .await
}

/// One device's whole reported session set, as a heartbeat body.
fn holding(marketplace: &str, status: &str) -> serde_json::Value {
    serde_json::json!([
        { "marketplace": marketplace, "account_label": null, "status": status },
    ])
}

/// The stored link state for one marketplace, or `None` where no connection
/// row exists at all -- which is a different fact from an unlinked one and is
/// what every test here starts from.
///
/// Read under the tenant pin deliberately. `connection` is under forced
/// row-level security, so an unpinned read answers `None` for a row that is
/// really there, and an assertion built on that would be green for the wrong
/// reason in exactly the way the step-11 fixture found.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn link_state(pool: &PgPool, org: OrgId, marketplace: &str) -> Option<String> {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let state: Option<String> =
        sqlx::query_scalar("SELECT state FROM connection WHERE org_id = $1 AND marketplace = $2")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(marketplace)
            .fetch_optional(&mut *tx)
            .await
            .expect("the connection reads");
    tx.commit().await.expect("the read commits");
    state
}

/// One tenant's connection lifecycle log, oldest first, as (event, actor kind,
/// actor id, detail).
///
/// Read under the tenant pin for the reason [`link_state`] is: `connection_audit`
/// carries forced row-level security, so an unpinned read answers an empty log
/// for a tenant that has one, and "no rows were written" would be indis-
/// tinguishable from "no rows were visible".
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn audit_log(
    pool: &PgPool,
    org: OrgId,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let rows = sqlx::query_as(
        "SELECT event, actor_kind, actor_id, detail FROM connection_audit \
         WHERE org_id = $1 ORDER BY id",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .fetch_all(&mut *tx)
    .await
    .expect("the log reads");
    tx.commit().await.expect("the read commits");
    rows
}

/// The attestation standing on one tenant's connection, and how many
/// connection rows that tenant has for that marketplace.
///
/// The count is half the assertion: "replaces" and "duplicates" both leave the
/// newest name readable, and only the count tells them apart.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn declared(pool: &PgPool, org: OrgId, marketplace: &str) -> (i64, Option<String>) {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let row: (i64, Option<String>) = sqlx::query_as(
        "SELECT count(*), max(authorship_name) FROM connection \
         WHERE org_id = $1 AND marketplace = $2",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(marketplace)
    .fetch_one(&mut *tx)
    .await
    .expect("the connection reads");
    tx.commit().await.expect("the read commits");
    row
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

/// The whole device loop over the wire, without a client existing yet: one
/// device claims, a second device is refused the settle, and the holder's own
/// settle is accepted.
///
/// The refusal is the point. The lease epoch is the fence and the device id is
/// the holder, so a settle naming a run the caller is not in is refused rather
/// than written — which is what stops a seller's second machine settling work
/// its sibling is still doing.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_settle_from_a_device_other_than_the_holder_is_refused(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    let second = "99998888777766665555444433332222";
    register(&pool, &TOKEN_A, second, "desktop").await;
    connected(&pool, &TOKEN_A, second).await;

    let claimed = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        claimed.status,
        StatusCode::OK,
        "the claim is served even with nothing queued: {}",
        String::from_utf8_lossy(&claimed.body)
    );

    // No item is queued in this fixture, so the claim is idle and there is no
    // live lease to settle. That is exactly the state a settle must refuse.
    let refused = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{second}/settle"),
            token: &TOKEN_A,
            // The settle carries the interpreter's own vocabulary now, so the
            // body is the lease it ran under and the verdict it reached.
            body: Some(serde_json::json!({
                "lease": {
                    "org": ORG_A,
                    "item": Uuid([0x11; 16]),
                    "lease_epoch": 1,
                },
                "verdict": {
                    "outcome": "succeeded",
                    "failure_code": null,
                    "failure_detail": null,
                },
                "at_ms": 1_756_000_042_000_i64,
            })),
            wall: t1,
        },
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::CONFLICT,
        "a settle naming a lease this device does not hold is refused: {}",
        String::from_utf8_lossy(&refused.body)
    );
}

/// A device belonging to one tenant reaches nothing through another tenant's
/// session, because the claim carries no organisation identifier at all: the
/// session decides the tenant and the SQL pins it again beneath that.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_session_cannot_claim_through_another_tenants_device_id(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;

    let across = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_B,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        across.status,
        StatusCode::OK,
        "the route answers rather than faulting"
    );
    let view: serde_json::Value =
        serde_json::from_slice(&across.body).expect("the claim view parses");
    assert_eq!(
        view["state"], "idle",
        "org B's session naming org A's device id claims nothing, because the device \
         predicate is read within the tenant the session speaks for: {view}"
    );
}

/// A ledger call naming a lease this device does not hold is refused before
/// anything is dispatched.
///
/// The fence is the whole point of the endpoint: the organisation comes from
/// the session, the holder from the lease, and a device that satisfies neither
/// reaches no ledger method at all.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_ledger_call_naming_a_lease_this_device_does_not_hold_is_refused(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;

    let refused = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/ledger"),
            token: &TOKEN_A,
            body: Some(serde_json::json!({
                "call": "preflight_succeeded",
                "lease": { "org": ORG_A, "item": Uuid([0x11; 16]), "lease_epoch": 1 },
            })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::CONFLICT,
        "no such live lease exists, so the call is refused rather than dispatched: {}",
        String::from_utf8_lossy(&refused.body)
    );
}

/// The payload route refuses a device holding no live lease, and refuses a
/// file id that is not a uuid before it reaches the store.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_payload_route_refuses_a_device_with_no_live_lease(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;

    let forbidden = call(
        pool.clone(),
        Call {
            method: Method::GET,
            path: &format!("/v1/devices/{LAPTOP}/payload/11111111-1111-1111-1111-111111111111"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert!(
        matches!(
            forbidden.status,
            StatusCode::FORBIDDEN | StatusCode::SERVICE_UNAVAILABLE
        ),
        "a device with no live lease has no business fetching a seller's files; this \
         deployment answers 503 where it holds no object store at all: {}",
        forbidden.status
    );

    let malformed = call(
        pool.clone(),
        Call {
            method: Method::GET,
            path: &format!("/v1/devices/{LAPTOP}/payload/not-a-uuid"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert!(
        matches!(
            malformed.status,
            StatusCode::UNPROCESSABLE_ENTITY | StatusCode::SERVICE_UNAVAILABLE
        ),
        "a malformed file id is a validation answer rather than a lookup: {}",
        malformed.status
    );
}

/// One tenant's session cannot reach another tenant's device through either
/// new route, because neither carries an organisation identifier.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn neither_new_route_carries_an_organisation_a_caller_could_substitute(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;

    for path in [
        format!("/v1/devices/{LAPTOP}/ledger"),
        format!("/v1/devices/{LAPTOP}/settle"),
    ] {
        let across = call(
            pool.clone(),
            Call {
                method: Method::POST,
                path: &path,
                token: &TOKEN_B,
                body: Some(serde_json::json!({
                    "call": "preflight_succeeded",
                    "lease": { "org": ORG_A, "item": Uuid([0x11; 16]), "lease_epoch": 1 },
                })),
                wall: t0,
            },
        )
        .await;
        assert_ne!(
            across.status,
            StatusCode::OK,
            "{path}: org B's session naming org A's lease must not succeed, whatever the \
             body says the org is"
        );
    }
}

/// The work route accepts a marketplace filter and serves without one.
///
/// What the filter *does* is asserted where the fixtures can seed two
/// marketplaces of real work — `a_filtered_claim_returns_only_the_marketplace_it_asked_for`
/// in tam-storage. What this pins is the surface: the body parses, an absent
/// body keeps the old behaviour, and a client that sends the filter the desktop
/// stream sends is served rather than refused.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_work_route_accepts_a_marketplace_filter_and_serves_without_one(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;

    for body in [
        None,
        Some(serde_json::json!({ "marketplace": "Tes" })),
        Some(serde_json::json!({})),
    ] {
        let answer = call(
            pool.clone(),
            Call {
                method: Method::POST,
                path: &format!("/v1/devices/{LAPTOP}/work"),
                token: &TOKEN_A,
                body: body.clone(),
                wall: t0,
            },
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "the route serves with body {body:?}: {}",
            String::from_utf8_lossy(&answer.body)
        );
        let view: serde_json::Value =
            serde_json::from_slice(&answer.body).expect("the claim view parses");
        assert_eq!(
            view["state"], "idle",
            "nothing is queued in this fixture, so every shape answers idle rather than \
             faulting: {view}"
        );
    }
}

/// One queued Tes create, claimed through the endpoint, so the ledger routes
/// can be driven against a lease that really exists.
///
/// It costs a catalogue seed, and the cost is worth paying: the held-scope
/// defect this suite's sibling found showed up only because a test drove real
/// data through the real path. A fence asserted against a lease nobody holds
/// asserts nothing.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed_claimable(app: &PgPool) {
    use tam_types::Uuid as Id;
    let product = tam_types::ProductId(Id([0x01; 16]));
    let mapping = tam_types::MappingId(Id([0x02; 16]));
    tam_storage::ProductRepo::new(app.clone())
        .insert(
            ORG_A,
            &tam_domain::CanonicalProduct {
                id: product,
                org: ORG_A,
                title: tam_types::Title("Fixture".to_owned()),
                body: tam_types::ListingCopy {
                    body: "Fixture".to_owned(),
                    format: tam_types::CopyFormat::Markdown,
                },
                payload: tam_types::PayloadSet::new(
                    tam_types::ProductFile {
                        id: tam_types::FileId(Id([0x03; 16])),
                        role: tam_types::FileRole::Payload,
                        kind: tam_types::FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: tam_types::ContentHash([0x04; 32]),
                            byte_len: 4,
                            scan: tam_types::ScanOutcome::Pending,
                        },
                    },
                    vec![],
                ),
                cover: None,
                previews: vec![],
                subjects: vec![],
                grades: tam_domain::GradeDeclaration {
                    source: tam_domain::DeclarationSource::Seller,
                    raw: vec![],
                    derived: None,
                },
                price: tam_types::PriceIntent::Free,
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            NOW,
        )
        .await
        .expect("the product inserts");
    tam_storage::MappingRepo::new(app.clone())
        .insert(
            ORG_A,
            &tam_domain::Mapping {
                id: mapping,
                org: ORG_A,
                product,
                inventory: tam_types::InventoryId::TesGb,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: tam_types::PriceRule::Explicit(tam_types::PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x05; 16]))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    tx.commit().await.expect("the fixture commits");

    let engine = engine_pool(app).await;
    tam_storage::JobRepo::new(engine)
        .enqueue(
            ORG_A,
            &tam_storage::NewJob {
                job: tam_types::JobId(Id([0x06; 16])),
                inventory: tam_types::InventoryId::TesGb,
                stamp: tam_types::Stamp {
                    at: NOW,
                    actor: tam_types::Actor::System(tam_types::SystemComponent::Engine),
                },
            },
            &[tam_storage::NewJobItem {
                item: tam_domain::JobItemId(Id([0x07; 16])),
                mapping,
                idempotency_key: tam_marketplace::IdempotencyKey(Id([0x08; 16])),
                operation: tam_domain::ItemOperation::Create,
                requires_bound_on: None,
            }],
        )
        .await
        .expect("the job enqueues");
}

/// Seeds, registers and claims, answering the lease the device now holds.
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should stop the run"
)]
async fn claimed(pool: &PgPool) -> serde_json::Value {
    provision(pool).await;
    seed_claimable(pool).await;
    register(pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(pool, &TOKEN_A, LAPTOP).await;
    // The lease is taken through the same claim `/work` uses, rather than
    // through `/work` itself. These three tests are about the ledger endpoint,
    // and routing them through the work order would make them depend on the
    // fixture's product projecting cleanly — a second thing to go wrong that
    // has nothing to do with what they assert. `/work` has its own tests.
    let claimed = tam_storage::LeaseRepo::new(pool.clone())
        .claim_for_device(
            &tam_storage::DeviceRef {
                org: ORG_A,
                device: LAPTOP,
            },
            &tam_storage::ClaimPolicy {
                ttl_seconds: i64::from(tam_domain::LEASE_TTL_SECS),
                grace_hours: 24,
                marketplace: None,
                reconcile: true,
            },
            NOW,
        )
        .await
        .expect("the claim runs");
    let tam_storage::DeviceClaim::Leased(item) = claimed else {
        panic!("the fixture must actually lease, or every assertion built on it is vacuous: {claimed:?}");
    };
    let lease = serde_json::json!({
        "org": ORG_A,
        "item": item.item,
        "lease_epoch": item.lease_epoch,
    });
    // The fixture proves its own premise: the row it just claimed is the row
    // the endpoint's fence will look up, held by this device at this epoch.
    // Read through the repository, which pins the tenant: `job_item` is under
    // forced row-level security, so a bare read here would see nothing and the
    // premise would look false when it is true.
    let holder = tam_storage::LeaseRepo::new(pool.clone())
        .holder(ORG_A, item.item)
        .await
        .expect("the claimed row reads");
    assert_eq!(
        holder.as_deref(),
        Some(LAPTOP),
        "the fixture's lease must be the one the fence will find: {lease}"
    );
    lease
}

/// A ledger call from the device that holds the lease, over the wire.
async fn ledger_call(pool: &PgPool, body: serde_json::Value) -> Answer {
    call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/ledger"),
            token: &TOKEN_A,
            body: Some(body),
            wall: t1,
        },
    )
    .await
}

/// `open_attempt` is idempotent on the caller's id, and a different id while
/// one is in flight is refused — over the wire, against a lease the device
/// really holds.
///
/// The recovery this buys is the point: a device whose response was lost
/// re-offers the same id and finds the row it already opened, instead of
/// minting a second attempt and burning the item's budget. A second create,
/// which is a different id, still hits the fence.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn open_attempt_is_idempotent_on_the_id_and_refuses_a_second(pool: PgPool) {
    let lease = claimed(&pool).await;
    let open = |attempt: &str| {
        serde_json::json!({
            "call": "open_attempt",
            "lease": lease,
            "new": {
                "attempt": attempt,
                "mapping": "02020202-0202-0202-0202-020202020202",
                "intent": { "body": {}, "hash": [1] },
            },
            "at_ms": 1_756_000_010_000_i64,
        })
    };

    let first = ledger_call(&pool, open("7b7b7b7b-7b7b-7b7b-7b7b-7b7b7b7b7b7b")).await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "the first open succeeds: {}",
        String::from_utf8_lossy(&first.body)
    );
    let replayed = ledger_call(&pool, open("7b7b7b7b-7b7b-7b7b-7b7b-7b7b7b7b7b7b")).await;
    let answer: serde_json::Value =
        serde_json::from_slice(&replayed.body).expect("the answer parses");
    assert_eq!(
        answer["answer"], "done",
        "re-offering the id already standing is the lost-response recovery, so it \
         answers done rather than refusing: {answer}"
    );

    let second = ledger_call(&pool, open("7c7c7c7c-7c7c-7c7c-7c7c-7c7c7c7c7c7c")).await;
    let answer: serde_json::Value =
        serde_json::from_slice(&second.body).expect("the answer parses");
    assert_eq!(
        answer["answer"], "refused",
        "a different id while one is in flight is a second create, and the fence refuses \
         it: {answer}"
    );
    assert_eq!(
        answer["error"], "attempt_in_flight",
        "and it arrives as an answer the device can branch on rather than as a fault: \
         {answer}"
    );
}

/// The rate grant's window and ceiling are the server's, over the wire.
///
/// The call carries neither, by the shape of `LedgerCall::RequestGrant`, so
/// the assertion is behavioural: grants exhaust at a limit the device never
/// named.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn request_grant_exhausts_at_a_ceiling_the_device_never_named(pool: PgPool) {
    let lease = claimed(&pool).await;
    let connection = "05050505-0505-0505-0505-050505050505";
    let ceiling = tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX.get();
    let mut granted = 0_u32;
    for _ in 0..ceiling + 2 {
        let answer = ledger_call(
            &pool,
            serde_json::json!({
                "call": "request_grant",
                "lease": lease,
                "connection": connection,
                "kind": "write",
                "at_ms": 1_756_000_020_000_i64,
            }),
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "the grant call is served: {}",
            String::from_utf8_lossy(&answer.body)
        );
        let answer: serde_json::Value =
            serde_json::from_slice(&answer.body).expect("the answer parses");
        // `Granted` carries a count; `Exhausted` is a bare variant. Matching on
        // the shape rather than on absence, so an unexpected answer breaks the
        // loop instead of being counted as a grant.
        if answer["grant"].get("granted").is_some() {
            granted += 1;
        } else {
            assert_eq!(
                answer["grant"], "exhausted",
                "the only other answer this call has is exhaustion: {answer}"
            );
            break;
        }
    }
    assert_eq!(
        u64::from(granted),
        u64::from(ceiling),
        "the window closed at the server's own per-minute limit, which the call has no \
         field to carry"
    );
}

/// A device's ledger write records both instants, distinctly, over the wire.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_ledger_write_records_both_instants(pool: PgPool) {
    let lease = claimed(&pool).await;
    let asserted = 1_600_000_000_000_i64;
    let answer = ledger_call(
        &pool,
        serde_json::json!({
            "call": "record_event",
            "lease": lease,
            "payload": { "ItemLeased": { "worker": LAPTOP, "lease_epoch": 1 } },
            "at_ms": asserted,
        }),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the event records: {}",
        String::from_utf8_lossy(&answer.body)
    );

    let engine = engine_pool(&pool).await;
    let (created, recorded): (i64, Option<i64>) = sqlx::query_as(
        "SELECT (extract(epoch FROM created_at) * 1000)::bigint, \
                (extract(epoch FROM asserted_at) * 1000)::bigint \
         FROM job_event WHERE kind = 'ItemLeased' AND asserted_at IS NOT NULL \
         ORDER BY org_seq DESC LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the event row reads");
    assert_eq!(
        recorded.expect("a device row records what the device asserted"),
        asserted,
        "the seller's own instant is preserved exactly, as their assertion"
    );
    assert_ne!(
        created, asserted,
        "and our receipt is our own clock rather than theirs"
    );
}

/// The renewed lease is the server's own TTL, over the wire, under `tam_app`.
///
/// Two things at once, and both were unproven before. The renew reaches the
/// database through the device's own endpoint, where forced row-level security
/// is the tenancy, so a write that forgot to pin the tenant would be inert
/// here and silently green everywhere else. And the answer's span is the
/// server's constant: the call carries no duration at all, so there is nothing
/// for a device to name and nothing to saturate.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_renew_over_the_wire_moves_the_expiry_by_the_servers_own_ttl(pool: PgPool) {
    let lease = claimed(&pool).await;
    let answer = ledger_call(
        &pool,
        serde_json::json!({
            "call": "renew",
            "lease": lease,
        }),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the renew is served: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let answer: serde_json::Value =
        serde_json::from_slice(&answer.body).expect("the answer parses");
    assert_eq!(
        answer["answer"], "renewed",
        "a live lease renews rather than being refused, which is what an unpinned write \
         under forced row-level security would have produced: {answer}"
    );
    let now = answer["renewed"]["server_now_ms"]
        .as_i64()
        .expect("the server states its own now");
    let deadline = answer["renewed"]["server_deadline_ms"]
        .as_i64()
        .expect("the server states the new deadline");
    assert_eq!(
        deadline - now,
        i64::from(tam_domain::LEASE_TTL_SECS) * 1_000,
        "the span is the server's constant, and the device's asserted instant had no part \
         in it: {answer}"
    );
}

/// A settle naming an epoch the item has moved past is refused.
///
/// The holder check alone is not the fence. A run whose lease was stolen and
/// then handed back to the same device — a reaper steal followed by a re-claim
/// — would pass a holder check while settling for a run that is over. The
/// epoch is what distinguishes them, and it is read here rather than carried
/// and ignored.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_settle_naming_an_epoch_the_item_has_moved_past_is_refused(pool: PgPool) {
    let lease = claimed(&pool).await;
    let stale = lease["lease_epoch"]
        .as_i64()
        .expect("the fixture's lease states its epoch")
        + 1;
    let refused = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/settle"),
            token: &TOKEN_A,
            body: Some(serde_json::json!({
                "lease": {
                    "org": ORG_A,
                    "item": lease["item"],
                    "lease_epoch": stale,
                },
                "verdict": {
                    "outcome": "succeeded",
                    "failure_code": null,
                    "failure_detail": null,
                },
                "at_ms": 1_756_000_042_000_i64,
            })),
            wall: t2,
        },
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::CONFLICT,
        "the device holds the item but not at this epoch, and the epoch is the run: {}",
        String::from_utf8_lossy(&refused.body)
    );
}

/// A work route that cannot serve what it claimed hands the lease back.
///
/// The claim takes the item before anything downstream can fail, so every exit
/// that is not a work order has to give it back. A 500 that kept it would
/// strand the item until the reaper and, through the per-connection mutex,
/// strand every sibling item on that marketplace behind it — and the seller
/// would see a queue that had simply stopped.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_work_route_that_cannot_prepare_hands_the_lease_back(pool: PgPool) {
    provision(&pool).await;
    seed_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    let engine = engine_pool(&pool).await;
    break_preparation(&pool, &engine).await;
    let before: i32 = sqlx::query_scalar("SELECT attempt_count FROM job_item")
        .fetch_one(&engine)
        .await
        .expect("the item reads");

    let answer = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the caller is told the truth about the failure rather than being handed an idle \
         queue: {}",
        String::from_utf8_lossy(&answer.body)
    );

    let (state, attempts, owner): (String, i32, Option<String>) =
        sqlx::query_as("SELECT state, attempt_count, lease_owner FROM job_item")
            .fetch_one(&engine)
            .await
            .expect("the item reads");
    assert_eq!(
        (state.as_str(), owner),
        ("queued", None),
        "the item is back on the queue with no holder, so the next poll can take it"
    );
    assert_eq!(
        attempts,
        before + 1,
        "and it is charged exactly one attempt, which is what stops a deterministic \
         preparation failure from being served again on every poll for ever"
    );
}

/// Every write this endpoint serves lands, under the tenant pin.
///
/// The defect this exists for is silent: `job_item` and its neighbours are
/// under forced row-level security, so a repository method that reaches them
/// through `tam_app` without pinning the tenant sees no rows, writes nothing
/// and answers as though the fence had refused. Under the engine role, which
/// bypasses row-level security, the same method is fine — so every test that
/// drove these through the worker was green while the device path was inert.
///
/// The calls run in one order deliberately: `park` moves the item out of the
/// live states the others need, so it goes last.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn every_write_this_endpoint_serves_lands_under_the_tenant_pin(pool: PgPool) {
    let lease = claimed(&pool).await;
    let calls = [
        serde_json::json!({ "call": "preflight_failed", "lease": lease, "edge_class": true }),
        serde_json::json!({ "call": "preflight_succeeded", "lease": lease }),
        serde_json::json!({
            "call": "gate_connection", "lease": lease,
            "inventory": "TesGb", "at_ms": 1_756_000_030_000_i64,
        }),
        serde_json::json!({
            "call": "halt_this_tenant", "lease": lease, "inventory": "TesGb",
            "reason": "a device asked for it", "at_ms": 1_756_000_031_000_i64,
        }),
        serde_json::json!({
            "call": "park", "lease": lease,
            "blocked_on": tam_storage::REAUTH_REQUIRED,
        }),
    ];
    for body in calls {
        let name = body["call"].clone();
        let answer = ledger_call(&pool, body.clone()).await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{name} is served: {}",
            String::from_utf8_lossy(&answer.body)
        );
        let answer: serde_json::Value =
            serde_json::from_slice(&answer.body).expect("the answer parses");
        assert_ne!(
            answer["answer"], "refused",
            "{name} reached a row and changed it; a refusal here is the tenancy pin missing \
             rather than the epoch fence holding, because this device holds this lease: \
             {answer}"
        );
    }
}

/// A counterpart that will never bind is settled, not handed back.
///
/// The livelock this closes is quiet: `claim_for_device` orders by
/// `created_at` and a release advances nothing, so an item released from this
/// arm is the very item the next poll claims. It would be prepared, released
/// and claimed again for ever, and every sibling on that marketplace would
/// wait behind it at the per-connection mutex the whole time.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_lost_counterpart_settles_through_the_work_route_and_the_queue_moves_on(pool: PgPool) {
    provision(&pool).await;
    seed_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    let engine = engine_pool(&pool).await;
    // `counterpart_binding` reads never-going-to-bind from one state only:
    // an `ambiguous_create` binding on the mapping the item waits for. The
    // item is pointed at its own inventory's mapping and that mapping put in
    // that state, which is the shortest route to the arm under test; what is
    // being tested is the route's disposition, not how the counterpart got
    // there.
    sqlx::query(
        "UPDATE mapping SET binding_state = 'ambiguous_create', \
             binding_attempt = '00000000-0000-0000-0000-0000000000a1', \
             ambiguous_since = now()",
    )
    .execute(&engine)
    .await
    .expect("the counterpart is made unreachable");
    sqlx::query("UPDATE job_item SET requires_bound_on = 'tes_gb'")
        .execute(&engine)
        .await
        .expect("the item is made to wait on that mapping");

    let answer = work(&pool, t0).await;
    assert_eq!(
        answer["state"], "idle",
        "there is no work to hand out: {answer}"
    );

    let (state, outcome): (String, Option<String>) =
        sqlx::query_as("SELECT state, outcome FROM job_item")
            .fetch_one(&engine)
            .await
            .expect("the item reads");
    assert_eq!(
        (state.as_str(), outcome.as_deref()),
        ("settled", Some("skipped")),
        "the item is settled where it stands rather than released, so the next poll is \
         served whatever is behind it instead of this same item again"
    );
}

/// A blocked item is parked with its gate, and the next poll does not see it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_blocked_item_is_parked_through_the_work_route_and_the_queue_moves_on(pool: PgPool) {
    provision(&pool).await;
    seed_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    let engine = engine_pool(&pool).await;
    // A create against a bound mapping is what `admission` refuses, and the
    // gate it names is `binding`. The bound shape is the one
    // `mapping_binding_total` and `mapping_remote_id_shape` require of a Tes
    // mapping: the marketplace's own id kind, a URL, and no numeric id.
    sqlx::query(
        "UPDATE mapping SET binding_state = 'bound', remote_id_kind = 'tes', \
             remote_url = 'https://www.tes.com/teaching-resource/x-4242', \
             remote_numeric_id = NULL, first_seen_at = now(), \
             verify_stale_since = now()",
    )
    .execute(&engine)
    .await
    .expect("the mapping is bound");

    let first = work(&pool, t0).await;
    assert_eq!(
        first["state"], "idle",
        "a blocked item is not handed out: {first}"
    );

    let (state, blocked_on): (String, Option<String>) =
        sqlx::query_as("SELECT state, blocked_on FROM job_item")
            .fetch_one(&engine)
            .await
            .expect("the item reads");
    assert_eq!(
        (state.as_str(), blocked_on.as_deref()),
        ("parked_live", Some("binding")),
        "the item is parked under the gate that refused it, with a park expiry the reaper \
         reads, rather than released to be claimed again immediately"
    );
    let has_expiry: bool = sqlx::query_scalar("SELECT park_expires_at IS NOT NULL FROM job_item")
        .fetch_one(&engine)
        .await
        .expect("the park expiry reads");
    assert!(has_expiry, "a park with no expiry is a park nothing wakes");

    let second = work(&pool, t1).await;
    assert_eq!(
        second["state"], "idle",
        "and the next poll does not see it again, which is the livelock this closes: {second}"
    );
}

/// The claim route, as the device calls it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should stop the run"
)]
async fn work(pool: &PgPool, wall: WallClock) -> serde_json::Value {
    let answer = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall,
        },
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the claim is served: {}",
        String::from_utf8_lossy(&answer.body)
    );
    serde_json::from_slice(&answer.body).expect("the claim view parses")
}

/// A deterministic preparation failure terminates when the budget runs out.
///
/// The route can neither hold the lease to expiry, which strands every sibling
/// on that marketplace, nor hand the item back uncharged, which would serve
/// this same item on every poll for ever. Charging is what separates the two
/// cases: this one never recovers, so it settles once the budget is spent.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_preparation_that_never_succeeds_settles_failed_once_the_budget_is_spent(pool: PgPool) {
    provision(&pool).await;
    seed_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    let engine = engine_pool(&pool).await;
    break_preparation(&pool, &engine).await;

    let budget = tam_limits::job::ATTEMPTS_MAX;
    for attempt in 1..budget {
        let answer = fault(&pool).await;
        assert_eq!(
            answer,
            StatusCode::INTERNAL_SERVER_ERROR,
            "poll {attempt} of {budget} still faults"
        );
        let (state, count): (String, i32) =
            sqlx::query_as("SELECT state, attempt_count FROM job_item")
                .fetch_one(&engine)
                .await
                .expect("the item reads");
        assert_eq!(
            (state.as_str(), count),
            ("queued", i32::try_from(attempt).unwrap_or(i32::MAX)),
            "inside the budget the item is charged and handed back, so a transient failure \
             would retry"
        );
    }

    assert_eq!(fault(&pool).await, StatusCode::INTERNAL_SERVER_ERROR);
    let (state, outcome, code): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT state, outcome, failure_code FROM job_item")
            .fetch_one(&engine)
            .await
            .expect("the item reads");
    assert_eq!(
        (state.as_str(), outcome.as_deref(), code.as_deref()),
        ("settled", Some("failed"), Some("Other")),
        "the last attempt settles it rather than queueing it a sixth time"
    );
}

/// One transient preparation failure costs one attempt and nothing else.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_transient_preparation_failure_costs_one_attempt_and_the_next_poll_claims_it_again(
    pool: PgPool,
) {
    provision(&pool).await;
    seed_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    let engine = engine_pool(&pool).await;
    let mapping: uuid::Uuid = sqlx::query_scalar("SELECT id FROM mapping")
        .fetch_one(&engine)
        .await
        .expect("the fixture's mapping reads");

    break_preparation(&pool, &engine).await;
    assert_eq!(fault(&pool).await, StatusCode::INTERNAL_SERVER_ERROR);

    // The fault clears, as a database blip would.
    sqlx::query("UPDATE job_item SET mapping_id = $1")
        .bind(mapping)
        .execute(&engine)
        .await
        .expect("the item points at its mapping again");

    // The item is claimed and prepared again rather than being stuck or
    // settled. What the preparation then decides is not this test's business:
    // this fixture's product does not project cleanly, so the honest assertion
    // is that the route reached a disposition rather than faulting again.
    let served = work(&pool, t1).await;
    assert_ne!(
        served["state"], "held",
        "the item is not still held by the run that failed: {served}"
    );
    let (state, count): (String, i32) = sqlx::query_as("SELECT state, attempt_count FROM job_item")
        .fetch_one(&engine)
        .await
        .expect("the item reads");
    assert_eq!(
        state, "parked_live",
        "the second poll claimed it and disposed of it, which is a transient failure \
         recovering rather than an item stranded"
    );
    assert_eq!(
        count, 1,
        "and the blip cost exactly one attempt: the park that followed charges none, so \
         a queue of transient failures still terminates rather than retrying for ever"
    );
}

/// Points the item at a mapping that is not there, which is the one
/// preparation failure reachable after the claim. The foreign keys go first;
/// this test owns its own database.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should stop the run"
)]
async fn break_preparation(pool: &PgPool, engine: &PgPool) {
    sqlx::query(
        "DO $$ DECLARE c text; BEGIN \
           FOR c IN SELECT conname FROM pg_constraint \
                    WHERE conrelid = 'job_item'::regclass AND contype = 'f' \
                      AND confrelid = 'mapping'::regclass \
           LOOP EXECUTE format('ALTER TABLE job_item DROP CONSTRAINT %I', c); END LOOP; \
         END $$",
    )
    .execute(pool)
    .await
    .expect("the test database drops its own constraints");
    sqlx::query("UPDATE job_item SET mapping_id = '00000000-0000-0000-0000-000000000009'")
        .execute(engine)
        .await
        .expect("the item is pointed at a mapping that does not exist");
}

/// One poll that is expected to fault, answering its status.
async fn fault(pool: &PgPool) -> StatusCode {
    call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await
    .status
}

/// A device cannot park an item on a gate the ledger does not know.
///
/// `blocked_on` carries no database constraint and the device names it, so an
/// unchecked park would put a gate in the ledger that no revive arm matches
/// and no console can label — a park nothing ever clears, written by the party
/// the park is holding.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_park_on_a_gate_outside_the_vocabulary_is_refused(pool: PgPool) {
    let lease = claimed(&pool).await;
    let answer = ledger_call(
        &pool,
        serde_json::json!({
            "call": "park",
            "lease": lease,
            "blocked_on": "a_gate_of_my_own_invention",
        }),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let answer: serde_json::Value =
        serde_json::from_slice(&answer.body).expect("the answer parses");
    assert_eq!(
        answer["answer"], "refused",
        "the gate is not one this ledger knows, so the park does not happen: {answer}"
    );

    let (state, gate): (String, Option<String>) =
        sqlx::query_as("SELECT state, blocked_on FROM job_item")
            .fetch_one(&engine_pool(&pool).await)
            .await
            .expect("the item reads");
    assert_ne!(
        state, "parked_live",
        "and the item is left where it was rather than parked on a gate nothing clears"
    );
    assert_eq!(gate, None, "with no gate written");
}

/// One canonical term's counterpart in one inventory's vocabulary.
fn path(
    inventory: tam_types::InventoryId,
    kind: tam_domain::TermKind,
    label: &str,
    native: &str,
) -> tam_domain::VocabularyPath {
    tam_domain::VocabularyPath {
        vocabulary: tam_domain::VocabularyId(inventory, kind),
        segments: vec![label.to_owned()],
        native_id: Some(native.to_owned()),
    }
}

fn crosswalk_edge(
    from: tam_types::CanonicalTermId,
    to: tam_domain::VocabularyPath,
) -> tam_domain::ProjectionEdge {
    tam_domain::ProjectionEdge {
        from,
        to,
        kind: tam_domain::EdgeKind::Exact,
        decided_by: tam_domain::Decider::Imported {
            source: "devices_flow fixture".to_owned(),
        },
        decided_at: NOW,
    }
}

/// One queued TPT create, and deliberately no `connection` row.
///
/// Everything the seller has that a device does not: the taxonomy counterparts
/// TPT projects through, a product, a mapping and a job item. TPT is the
/// marketplace this has to be, because its product form is the one that makes
/// the seller declare authorship, so it is the only place where the attestation
/// is the difference between a write and a refusal.
///
/// There is no `connection` row here and none may be added. That row is what
/// the heartbeat and the declaration route exist to write, so a fixture that
/// inserted one would leave every test built on this green with both writers
/// deleted -- which is precisely the regression this fixture is here to catch.
/// `seed_claimable` above does insert one, deliberately and for a different
/// job: those tests are about the ledger and the fences, and they predate any
/// writer that could have made the row for them.
///
/// Unlike `seed_claimable` this product projects cleanly rather than parking on
/// a taxonomy election, which is what lets `/work` answer with an order at all.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed_tpt_claimable(app: &PgPool) {
    use tam_types::Uuid as Id;
    let product = tam_types::ProductId(Id([0x11; 16]));
    let mapping = tam_types::MappingId(Id([0x12; 16]));
    let subject = tam_types::CanonicalTermId(Id([0x77; 16]));
    let grade = tam_types::CanonicalTermId(Id([0x79; 16]));
    let declared = path(
        tam_types::InventoryId::TesUs,
        tam_domain::TermKind::Phase,
        "Kindergarten",
        "17",
    );
    tam_storage::TaxonomyRepo::new(app.clone())
        .seed(
            &[
                tam_domain::CanonicalTerm {
                    id: subject,
                    kind: tam_domain::TermKind::Subject,
                    parent: None,
                    label: "Maths for early years".to_owned(),
                },
                tam_domain::CanonicalTerm {
                    id: grade,
                    kind: tam_domain::TermKind::Phase,
                    parent: None,
                    label: "Kindergarten".to_owned(),
                },
            ],
            &[
                crosswalk_edge(
                    subject,
                    path(
                        tam_types::InventoryId::Tpt,
                        tam_domain::TermKind::Subject,
                        "Maths for early years",
                        "tpt-maths",
                    ),
                ),
                crosswalk_edge(
                    grade,
                    path(
                        tam_types::InventoryId::Tpt,
                        tam_domain::TermKind::Phase,
                        "Kindergarten",
                        "tpt-kindergarten",
                    ),
                ),
                // Seeded on the source side too. The product declares its grade
                // as a TesUs path, and the projection reaches TPT by ingesting
                // that into the canonical term and projecting out again; without
                // this edge the ingest finds nothing and the item parks on a
                // taxonomy election instead of producing an order.
                crosswalk_edge(grade, declared.clone()),
            ],
        )
        .await
        .expect("the crosswalk seeds");
    tam_storage::ProductRepo::new(app.clone())
        .insert(
            ORG_A,
            &tam_domain::CanonicalProduct {
                id: product,
                org: ORG_A,
                title: tam_types::Title("Fractions practice".to_owned()),
                body: tam_types::ListingCopy {
                    body: "A worksheet.".to_owned(),
                    format: tam_types::CopyFormat::Markdown,
                },
                payload: tam_types::PayloadSet::new(
                    tam_types::ProductFile {
                        id: tam_types::FileId(Id([0x13; 16])),
                        role: tam_types::FileRole::Payload,
                        kind: tam_types::FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: tam_types::ContentHash([0x14; 32]),
                            byte_len: 4,
                            scan: tam_types::ScanOutcome::Clean { at: NOW },
                        },
                    },
                    vec![],
                ),
                cover: Some(tam_types::ProductFile {
                    id: tam_types::FileId(Id([0x15; 16])),
                    role: tam_types::FileRole::Cover,
                    kind: tam_types::FileKind::Image,
                    bytes: FileBytes::Held {
                        hash: tam_types::ContentHash([0x16; 32]),
                        byte_len: 4,
                        scan: tam_types::ScanOutcome::Clean { at: NOW },
                    },
                }),
                previews: vec![],
                subjects: vec![subject],
                grades: tam_domain::GradeDeclaration {
                    source: tam_domain::DeclarationSource::Imported {
                        vocabulary: tam_domain::VocabularyId(
                            tam_types::InventoryId::TesUs,
                            tam_domain::TermKind::Phase,
                        ),
                    },
                    raw: vec![declared],
                    derived: Some(tam_domain::AgeInterval::new(5, 7).expect("a bounded range")),
                },
                price: tam_types::PriceIntent::Free,
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            NOW,
        )
        .await
        .expect("the product inserts");
    tam_storage::MappingRepo::new(app.clone())
        .insert(
            ORG_A,
            &tam_domain::Mapping {
                id: mapping,
                org: ORG_A,
                product,
                inventory: tam_types::InventoryId::Tpt,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: tam_types::PriceRule::Explicit(tam_types::PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
    tam_storage::JobRepo::new(engine_pool(app).await)
        .enqueue(
            ORG_A,
            &tam_storage::NewJob {
                job: tam_types::JobId(Id([0x17; 16])),
                inventory: tam_types::InventoryId::Tpt,
                stamp: tam_types::Stamp {
                    at: NOW,
                    actor: tam_types::Actor::System(tam_types::SystemComponent::Engine),
                },
            },
            &[tam_storage::NewJobItem {
                item: tam_domain::JobItemId(Id([0x18; 16])),
                mapping,
                idempotency_key: tam_marketplace::IdempotencyKey(Id([0x19; 16])),
                operation: tam_domain::ItemOperation::Create,
                requires_bound_on: None,
            }],
        )
        .await
        .expect("the job enqueues");
}

/// The whole link a TPT write needs, made only of what a seller and a device
/// actually do.
///
/// No `connection` row is seeded: the device's check-in writes the link and the
/// seller's declaration writes the attestation, and between them the item
/// becomes claimable and the order carries what the adapter refuses to write
/// without. This is the test the broker deletion is gated on, so it must fail
/// against a tree with no device-reported writer -- and it did, twice over,
/// because nothing wrote `connection.state` outside the vault and no work order
/// could carry an attestation under `tam_app` even where a row existed.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_and_a_declaration_are_the_whole_link_a_tpt_write_needs(pool: PgPool) {
    provision(&pool).await;
    seed_tpt_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;

    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await,
        None,
        "a registered device that has reported nothing links nothing: the row does not \
         exist yet, which is a different fact from an unlinked one"
    );
    let before = work(&pool, t0).await;
    assert_eq!(
        before["state"], "idle",
        "and nothing is claimable through it: {before}"
    );

    connected(&pool, &TOKEN_A, LAPTOP).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("linked"),
        "the check-in is the link: a device reporting a connected session is the only \
         evidence of one this server can ever hold"
    );
    let declared = declare(&pool, &TOKEN_A, "Tpt", "Ada Lovelace", t1).await;
    assert_eq!(
        declared.status,
        StatusCode::OK,
        "the declaration lands: {}",
        String::from_utf8_lossy(&declared.body)
    );

    let order = work(&pool, t2).await;
    assert_eq!(
        order["state"], "work",
        "the item is claimable, which is what the linked connection bought: {order}"
    );
    assert_eq!(
        order["attestation"]["attested_by"], "Ada Lovelace",
        "and the order carries the seller's own declaration, without which the TPT \
         adapter refuses every write before it reaches the transport: {order}"
    );
    assert_eq!(
        order["attestation"]["attested_at_ms"],
        serde_json::json!(t1().0),
        "stamped when the declaration was made rather than when the order was built, \
         which is {} here: {order}",
        t2().0
    );
}

/// A connection is linked while any live device holds it, and gated when the
/// last one stops.
///
/// The quantifier is what a wrong implementation gets wrong: deriving from the
/// reporting device alone would gate a seller who is still signed in on their
/// other machine, which is D14's whole point.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_connection_is_linked_while_any_device_holds_it_and_gated_when_the_last_stops(
    pool: PgPool,
) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    register(&pool, &TOKEN_A, DESKTOP, "desktop").await;

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t0).await;
    beat(&pool, &TOKEN_A, DESKTOP, holding("Tpt", "connected"), t0).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("linked"),
        "both machines hold it"
    );

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "signed_out"), t1).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("linked"),
        "one signing out is not the connection going: the desktop still holds a session, \
         and gating here would ask a signed-in seller to re-link"
    );

    beat(&pool, &TOKEN_A, DESKTOP, holding("Tpt", "signed_out"), t2).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("needs_reauth"),
        "and the last one stopping is, because nothing holds a session any more"
    );
}

/// A marketplace dropped from the report derives exactly like one reported
/// signed out.
///
/// The heartbeat deletes what a device stops naming, so the delete arm has to
/// derive as much as the upsert arm does; a derivation over the named set alone
/// leaves the connection linked on a session nothing reports at all.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_marketplace_dropped_from_a_report_gates_the_connection_it_held(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t0).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("linked")
    );

    beat(&pool, &TOKEN_A, LAPTOP, serde_json::json!([]), t1).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("needs_reauth"),
        "the device stopped naming it, which is how it says the session is gone"
    );
    let held = devices(&pool, &TOKEN_A).await;
    assert!(
        held.iter().all(|device| device.sessions.is_empty()),
        "and the session row went with it: {held:?}"
    );
}

/// The declaration outlives the device that was registered when it was made.
///
/// It is keyed on the marketplace connection, so a seller who replaces a laptop
/// declares nothing again -- which is the difference between this and anything
/// stored per device.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_declaration_outlives_the_device_that_was_registered_when_it_was_made(pool: PgPool) {
    provision(&pool).await;
    seed_tpt_claimable(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    declare(&pool, &TOKEN_A, "Tpt", "Ada Lovelace", t0).await;
    revoke(&pool, &TOKEN_A, LAPTOP, t1).await;

    register(&pool, &TOKEN_A, DESKTOP, "desktop").await;
    connected(&pool, &TOKEN_A, DESKTOP).await;
    let order = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{DESKTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t2,
        },
    )
    .await;
    assert_eq!(
        order.status,
        StatusCode::OK,
        "the new machine is served: {}",
        String::from_utf8_lossy(&order.body)
    );
    let order: serde_json::Value =
        serde_json::from_slice(&order.body).expect("the claim view parses");
    assert_eq!(
        order["state"], "work",
        "the replacement machine's own check-in relinks the connection: {order}"
    );
    assert_eq!(
        order["attestation"]["attested_by"], "Ada Lovelace",
        "and it writes under the declaration the seller made on the machine they no \
         longer have, which is what keying it on the connection buys: {order}"
    );
}

/// A declaration reaches its own tenant's connection and no other's.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_declaration_reaches_only_the_tenant_that_made_it(pool: PgPool) {
    provision(&pool).await;
    let declared = declare(&pool, &TOKEN_B, "Tpt", "Grace Hopper", t0).await;
    assert_eq!(declared.status, StatusCode::OK);

    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await,
        None,
        "the other tenant has no connection at all, and a route that took the \
         organisation from anywhere but the session would have given it one"
    );
    assert_eq!(
        link_state(&pool, ORG_B, "tpt").await.as_deref(),
        Some("unlinked"),
        "and the declaring tenant's row is unlinked rather than linked: declaring is a \
         fact about the seller, not a session any device reported"
    );

    declare(&pool, &TOKEN_A, "Tpt", "Ada Lovelace", t1).await;
    let sanctioned = declare(&pool, &TOKEN_A, "Etsy", "Ada Lovelace", t1).await;
    assert_eq!(
        sanctioned.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a marketplace with an official API composes its writes server-side, so no \
         device ever reads a declaration for it back: {}",
        String::from_utf8_lossy(&sanctioned.body)
    );
    assert_eq!(
        link_state(&pool, ORG_A, "etsy").await,
        None,
        "and the refusal wrote nothing"
    );
}

/// A check-in never lifts a revoked connection.
///
/// The same reading `register` already applies to a revoked device: a
/// revocation any machine could undo by restarting would not be the seller's
/// decision any more.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_check_in_never_lifts_a_revoked_connection(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t0).await;

    // What the console's revoke does, written directly. The route is a plain
    // write now that the broker is gone, so a test could call it; this stays
    // direct because the subject here is the beat, and reaching the state
    // through another route would put that route's behaviour in the way of it.
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query("UPDATE connection SET state = 'revoked' WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(&mut *tx)
        .await
        .expect("the revocation writes");
    tx.commit().await.expect("the revocation commits");

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t1).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("revoked"),
        "the device still reports a session and it changes nothing here"
    );
    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "signed_out"), t2).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("revoked"),
        "and the gating arm leaves it alone too, so neither direction rewrites the \
         seller's decision"
    );
}

/// A revoked device's session keeps nothing linked, even when it is the only
/// one that ever reported.
///
/// The `d.revoked_at IS NULL` join in `derive_link` is what excludes it, and
/// nothing until here could tell whether that clause was doing anything.
/// Revoking a device leaves its `device_marketplace_session` row exactly as it
/// was -- `revoke` writes only `device.revoked_at` -- and a revoked device can
/// still check in, which is how it learns it was revoked. So a stale
/// `connected` row genuinely reaches the existence probe, and the last step
/// below flips on that clause alone: with it, no live device reports and the
/// connection gates; without it, two disowned machines' stale rows would hold
/// it open for ever.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_revoked_devices_session_holds_no_connection_open(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;
    register(&pool, &TOKEN_A, DESKTOP, "desktop").await;
    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t0).await;
    beat(&pool, &TOKEN_A, DESKTOP, holding("Tpt", "connected"), t0).await;

    revoke(&pool, &TOKEN_A, DESKTOP, t1).await;
    let held = devices(&pool, &TOKEN_A).await;
    let stale = held
        .iter()
        .find(|device| device.id == DESKTOP)
        .expect("the revoked device is still listed");
    assert_eq!(
        stale.sessions.len(),
        1,
        "revoking writes only revoked_at, so the session row it reported is still here: \
         {stale:?}"
    );
    assert_eq!(
        stale.sessions[0].status, "connected",
        "and still says connected, which is the row the exclusion has to see past"
    );

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t1).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("linked"),
        "the premise: one machine is still ours and still reports it"
    );

    revoke(&pool, &TOKEN_A, LAPTOP, t2).await;
    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t2).await;
    assert_eq!(
        link_state(&pool, ORG_A, "tpt").await.as_deref(),
        Some("needs_reauth"),
        "and now every machine reporting it has been disowned. Both still say connected \
         and neither is ours, so the connection is not held open by what a machine we \
         have asked to forget its sessions still claims"
    );
}

/// The lifecycle log records the transitions and not the check-ins.
///
/// Two claims, and neither was asserted anywhere: that a device-derived link
/// names the device as the actor rather than the broker, and that a device
/// beating every thirty seconds does not write a row every thirty seconds.
/// `link_state` reads `connection.state` and never the log, so both were
/// conclusions from reading the SQL rather than from running it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_lifecycle_log_records_the_transition_and_not_the_check_in(pool: PgPool) {
    provision(&pool).await;
    register(&pool, &TOKEN_A, LAPTOP, "laptop").await;

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t0).await;
    assert_eq!(
        audit_log(&pool, ORG_A).await,
        vec![(
            "linked".to_owned(),
            "system".to_owned(),
            Some("device".to_owned()),
            Some("device-reported".to_owned()),
        )],
        "one row, naming the device rather than the broker. The vocabulary's `linked` was \
         written for a credential being sealed and nothing is sealed here, so the detail \
         is what separates the two writers inside one log"
    );

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "connected"), t1).await;
    assert_eq!(
        audit_log(&pool, ORG_A).await.len(),
        1,
        "the same session reported again is not a second link. A device checks in on a \
         timer, so a log that recorded check-ins would bury every real transition"
    );

    beat(&pool, &TOKEN_A, LAPTOP, holding("Tpt", "signed_out"), t2).await;
    let log = audit_log(&pool, ORG_A).await;
    assert_eq!(
        log.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        vec!["linked", "needs_reauth"],
        "and the gating is a transition too, recorded in the order it happened: {log:?}"
    );
}

/// Declaring again replaces the declaration rather than adding one.
///
/// The upsert is keyed on `(org_id, marketplace)`, so a second declaration
/// structurally cannot make a second row -- but "replaces" and "duplicates"
/// both leave the newest name readable, so the count is half of what is
/// asserted here and the name alone would not have caught it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn declaring_again_replaces_the_declaration_rather_than_adding_one(pool: PgPool) {
    provision(&pool).await;
    let first = declare(&pool, &TOKEN_A, "Tpt", "Ada Lovelace", t0).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(
        declared(&pool, ORG_A, "tpt").await,
        (1, Some("Ada Lovelace".to_owned()))
    );

    let second = declare(&pool, &TOKEN_A, "Tpt", "Grace Hopper", t1).await;
    assert_eq!(
        second.status,
        StatusCode::OK,
        "a seller may correct what they declared: {}",
        String::from_utf8_lossy(&second.body)
    );
    let view: serde_json::Value = second.json();
    assert_eq!(
        view["name"], "Grace Hopper",
        "the answer is the declaration as it now stands rather than what was sent: {view}"
    );
    assert_eq!(
        view["attested_at"],
        serde_json::json!(t1().0),
        "stamped when this declaration was made, not when the first one was: {view}"
    );

    assert_eq!(
        declared(&pool, ORG_A, "tpt").await,
        (1, Some("Grace Hopper".to_owned())),
        "one connection, carrying the second name. Two rows would each be a connection \
         for the same marketplace, which is the shape the unique index exists to refuse"
    );
}

/// Makes the fixture product's payload a marketplace-sourced file.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn source_the_payload(pool: &PgPool) {
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "UPDATE product_file SET hash = NULL, scan_state = NULL, scan_signature = NULL, \
                scanned_at = NULL, scan_failure_code = NULL, \
                source_marketplace = 'tes', \
                source_connection = (SELECT id FROM connection WHERE org_id = $1 LIMIT 1), \
                source_resource = '13549794', source_entry = 'worksheet.pdf', \
                observed_hash = decode(repeat('5a', 32), 'hex'), observed_byte_len = 493000, \
                asserted_scan_state = 'pending', observed_by_device = $2, \
                observed_at = now(), recorded_at = now(), \
                payload_file_name = 'worksheet.pdf', payload_content_type = 'application/pdf' \
         WHERE org_id = $1 AND role = 'payload'",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(LAPTOP)
    .execute(&mut *tx)
    .await
    .expect("the payload becomes marketplace-sourced");
    tx.commit().await.expect("the fixture commits");
}

/// Registers the device again at a stated version, which is how a client
/// reports an upgrade.
///
/// No `expect_used` attribute: this helper asserts rather than unwrapping, and
/// an unfulfilled expectation is itself a denied lint.
async fn register_at(pool: &PgPool, version: &str) -> Answer {
    let answer = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: "/v1/devices",
            token: &TOKEN_A,
            body: Some(serde_json::json!({
                "id": LAPTOP,
                "name": "laptop",
                "os": "linux",
                "arch": "x86_64",
                "app_version": version,
            })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the fixture's registration must succeed or every claim below is vacuous: {}",
        String::from_utf8_lossy(&answer.body)
    );
    answer
}

/// The one item's state, read as the engine so RLS does not hide it.
///
/// The claim view cannot answer what this asks. `seed_claimable`'s product is
/// not projectable, so an item that *is* claimed is prepared, found blocked
/// and parked, and the route answers idle — the same word it answers for an
/// item the gate never made a candidate. The difference is in the ledger: a
/// gated item is untouched at `queued`, a claimed one has moved.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn item_state(app: &PgPool) -> String {
    let engine = engine_pool(app).await;
    // Keyed on the tenant and counted, because the engine role is BYPASSRLS
    // and an unfiltered `LIMIT 1` would read whichever row came first — right
    // only while exactly one exists, and silently wrong the day a fixture
    // enqueues a second or a sibling tenant appears.
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT state FROM job_item ji \
         JOIN job j ON j.org_id = ji.org_id AND j.id = ji.job_id \
         WHERE ji.org_id = $1",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .fetch_all(&engine)
    .await
    .expect("the item reads");
    assert_eq!(
        rows.len(),
        1,
        "this probe answers for one item and the fixture must hold exactly one: {rows:?}"
    );
    rows.into_iter().next().expect("the one row")
}

/// The gate, through the route a device actually calls.
///
/// The three claim tests in `tam-storage` drive `claim_for_device` directly,
/// which leaves the route's own half — that the version compared is the one on
/// the device's row — asserted nowhere. This is that half: nothing here hands
/// the claim a version, so the only way the right item is served is if the
/// route read it from the row it registered.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_work_route_gates_a_sourced_item_on_the_devices_reported_version(pool: PgPool) {
    provision(&pool).await;
    // `seed_claimable` inserts the connection and `connected` derives one from
    // the heartbeat, so the seed goes first: the other order trips
    // `connection_one_per_marketplace`, which is the constraint doing its job
    // rather than a fixture problem to work around.
    seed_claimable(&pool).await;
    register_at(&pool, "0.1.3").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    source_the_payload(&pool).await;

    let idle = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(idle.status, StatusCode::OK);
    assert_eq!(
        item_state(&pool).await,
        "queued",
        "a client at 0.1.3 cannot decode a marketplace source, so the item was never a \
         candidate and the ledger has not moved"
    );

    // The same device reporting an upgrade is served the same item.
    register_at(&pool, "0.2.0").await;
    let served = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(served.status, StatusCode::OK);
    assert_ne!(
        item_state(&pool).await,
        "queued",
        "the upgrade is what the gate reads: the item is now a candidate and has been claimed, \
         whatever the preparation then made of it"
    );
}

/// A version claimed in the request body changes nothing.
///
/// This is the test that catches version-from-request, which is the wrong
/// implementation and the tempting one: the device is the governed party, so a
/// gate that asked it what it may run would be asking the thing being gated.
/// The body below says 0.2.0 while the row says 0.1.3, and the item must still
/// wait.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_version_claimed_in_the_request_body_is_ignored(pool: PgPool) {
    provision(&pool).await;
    // `seed_claimable` inserts the connection and `connected` derives one from
    // the heartbeat, so the seed goes first: the other order trips
    // `connection_one_per_marketplace`, which is the constraint doing its job
    // rather than a fixture problem to work around.
    seed_claimable(&pool).await;
    register_at(&pool, "0.1.3").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;
    source_the_payload(&pool).await;

    let answer = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: Some(serde_json::json!({ "app_version": "0.2.0", "marketplace": "Tes" })),
            wall: t0,
        },
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    // The ledger, not the view. `idle` is also what a claimed-then-parked item
    // produces, so asserting it would pass whether or not the body was read —
    // which is the whole of what this test exists to falsify.
    assert_eq!(
        item_state(&pool).await,
        "queued",
        "the gate reads the device's row, never its request: a client that could ask for work \
         it cannot decode by saying so would be the governed party deciding what it is allowed"
    );
}

/// A blob-backed item is still served to a client below the shim.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_work_route_still_serves_a_blob_backed_item_to_an_old_client(pool: PgPool) {
    provision(&pool).await;
    // `seed_claimable` inserts the connection and `connected` derives one from
    // the heartbeat, so the seed goes first: the other order trips
    // `connection_one_per_marketplace`, which is the constraint doing its job
    // rather than a fixture problem to work around.
    seed_claimable(&pool).await;
    register_at(&pool, "0.1.3").await;
    connected(&pool, &TOKEN_A, LAPTOP).await;

    let answer = call(
        pool.clone(),
        Call {
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/work"),
            token: &TOKEN_A,
            body: None,
            wall: t0,
        },
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_ne!(
        item_state(&pool).await,
        "queued",
        "the gate is about the item's payload, not the client's age: a blob-backed item is a \
         candidate for an old client and was claimed"
    );
}
