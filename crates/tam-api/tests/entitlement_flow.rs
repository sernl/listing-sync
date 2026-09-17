//! Decision D10's token over the wire: who is minted one, who is not, and what
//! it says.
//!
//! Every helper is defined here rather than shared, because what these assert
//! is the mint's own predicate and a fixture that drifted for another suite's
//! reason would move the answers silently.
//!
//! The token is verified with the same algorithm, audience, issuer and
//! `validate_exp = false` the desktop client's `Entitlement::verify` uses, and
//! decoded into the same `tam_domain::entitlement::Claims`. That is the wire
//! round trip: the server's bytes and the device's type, held together by one
//! definition rather than by two that agree today.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use sqlx::PgPool;
use std::collections::HashSet;
use tam_api::devices::{EntitlementKey, HeartbeatView};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::entitlement::{Claims, AUDIENCE, ISSUER};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{Marketplace, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const LAPTOP: &str = "11112222333344445555666677778888";
const NOW: Timestamp = Timestamp(1_756_000_000_000);
/// D11's numbers as seconds, spelled out so an assertion checks the constants
/// rather than restating them.
const VALIDITY_SECS: i64 = 3_600;
const GRACE_SECS: i64 = 86_400;

fn t0() -> Timestamp {
    NOW
}

/// The signing half and the verifying half of one throwaway key pair.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn key_pair() -> (EntitlementKey, Vec<u8>) {
    let random = ring::rand::SystemRandom::new();
    let pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&random).expect("the test key generates");
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .expect("the generated key parses");
    (
        EntitlementKey::new(pkcs8.as_ref().to_vec()),
        ring::signature::KeyPair::public_key(&pair)
            .as_ref()
            .to_vec(),
    )
}

/// A deployment that mints, and one that does not.
fn state(pool: PgPool, key: Option<EntitlementKey>) -> AppState {
    AppState {
        exchange_rates: None,
        pool,
        config: Config {
            entitlement_key: key,
            ..Config::default()
        },
        wall: t0,
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

/// One statement against one tenant's pinned connection.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn pinned(pool: &PgPool, org: OrgId, statement: &str) {
    let mut tx = pool.begin().await.expect("the fixture opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin applies");
    sqlx::query(statement)
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .execute(&mut *tx)
        .await
        .expect("the fixture writes");
    tx.commit().await.expect("the fixture commits");
}

/// A linked connection for one marketplace, which the mint requires.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn link(pool: &PgPool, org: OrgId, marketplace: &str, id: [u8; 16]) {
    let mut tx = pool.begin().await.expect("the fixture opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, $3, 'linked', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes(id))
    .bind(marketplace)
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    tx.commit().await.expect("the fixture commits");
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
    fn view(&self) -> HeartbeatView {
        serde_json::from_slice(&self.body).expect("the answer body parses")
    }
}

/// One request, as a value rather than as six positional arguments: four of
/// them are a key, a path, a token and a body, which is exactly the shape a
/// transposition survives compilation in. The same shape `devices_flow.rs`
/// uses, for the same reason.
struct Call<'a> {
    /// The signing key this deployment was started with, or none.
    key: Option<EntitlementKey>,
    method: Method,
    path: &'a str,
    token: &'a SessionToken,
    body: Option<serde_json::Value>,
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
    let response = router(state(pool, call.key))
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

async fn register(pool: &PgPool, key: &EntitlementKey, token: &SessionToken, id: &str) -> Answer {
    call(
        pool.clone(),
        Call {
            key: Some(key.clone()),
            method: Method::POST,
            path: "/v1/devices",
            token,
            body: Some(serde_json::json!({
                "id": id,
                "name": "founder-pc",
                "os": "windows",
                "arch": "x86_64",
                "app_version": "0.2.0",
            })),
        },
    )
    .await
}

async fn beat(
    pool: &PgPool,
    key: Option<EntitlementKey>,
    token: &SessionToken,
    id: &str,
) -> Answer {
    call(
        pool.clone(),
        Call {
            key,
            method: Method::POST,
            path: &format!("/v1/devices/{id}/heartbeat"),
            token,
            body: Some(serde_json::json!({ "sessions": [] })),
        },
    )
    .await
}

/// The claims a token carries, verified exactly as the desktop client verifies
/// them: same algorithm, same audience, same issuer, and `validate_exp` off,
/// because a token past `exp` but inside `grace` is still workable under D11.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn verified(token: &str, public: &[u8]) -> Claims {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_audience(&[AUDIENCE]);
    validation.set_issuer(&[ISSUER]);
    validation.required_spec_claims =
        HashSet::from(["exp".to_owned(), "aud".to_owned(), "iss".to_owned()]);
    validation.validate_exp = false;
    jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_ed_der(public), &validation)
        .expect("the minted token verifies against the key that signed it")
        .claims
}

/// A registered device of ORG_A with every marketplace linked, Etsy included.
///
/// Etsy is linked deliberately, and it is what makes T1's assertion severe
/// rather than decorative. Without the row, Etsy is absent from the grant set
/// because the tenant has no `linked` connection for it, so the assertion would
/// hold with the `transport_class = 'seller_device'` filter deleted from
/// `entitled_marketplaces` — and deleting that filter is exactly how a token
/// comes to name a marketplace whose requests no device may ever compose, which
/// is the first non-negotiable in CLAUDE.md failing silently.
async fn entitled_device(pool: &PgPool, key: &EntitlementKey) {
    provision(pool).await;
    register(pool, key, &TOKEN_A, LAPTOP).await;
    link(pool, ORG_A, "tpt", [0x05; 16]).await;
    link(pool, ORG_A, "tes", [0x06; 16]).await;
    link(pool, ORG_A, "etsy", [0x07; 16]).await;
}

// T1
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_entitled_device_is_minted_a_token_naming_itself_and_its_marketplaces(pool: PgPool) {
    let (key, public) = key_pair();
    entitled_device(&pool, &key).await;

    let answer = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await;
    assert_eq!(answer.status, StatusCode::OK);
    let view = answer.view();
    assert!(!view.revoked);

    let claims = verified(
        view.entitlement
            .as_deref()
            .expect("an entitled device is minted a token"),
        &public,
    );
    assert_eq!(
        claims.device, LAPTOP,
        "a token names the device that asked, so it is useless on any other machine"
    );
    assert_eq!(
        claims.sub,
        Uuid([0xAA; 16]).to_hyphenated(),
        "and the organisation it was minted under, for attribution"
    );
    let mut granted = claims.marketplaces.clone();
    granted.sort_by_key(|marketplace| format!("{marketplace:?}"));
    assert_eq!(
        granted,
        vec![Marketplace::Tes, Marketplace::Tpt],
        "both seller-device marketplaces are linked, and Etsy is never granted: its \
         automation runs server-side and no device composes its requests"
    );
    assert_eq!(
        claims.exp,
        NOW.0.div_euclid(1_000) + VALIDITY_SECS,
        "one hour of validity, from the server's own clock"
    );
    assert_eq!(
        claims.grace,
        claims.exp + GRACE_SECS,
        "and twenty-four hours of grace after it"
    );
}

// T2
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_revoked_device_is_told_so_and_granted_nothing(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;

    call(
        pool.clone(),
        Call {
            key: Some(key.clone()),
            method: Method::POST,
            path: &format!("/v1/devices/{LAPTOP}/revoke"),
            token: &TOKEN_A,
            body: None,
        },
    )
    .await;

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    assert!(view.revoked, "the sign-out reaches the device");
    assert_eq!(
        view.entitlement, None,
        "and it is granted nothing in the same answer, so a client that ignored `revoked` \
         still cannot work"
    );
}

// T3. The lapse is read off `entitlement_grant` rather than off Paddle's
// recorded status: the grace is folded into the grant's own expiry when the
// webhook writes it, so what the device gate asks is "does this organisation
// hold a live grant", and an organisation that has held one and holds none
// now is the lapsed case.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_plan_that_lapsed_is_granted_nothing(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;
    pinned(
        &pool,
        ORG_A,
        "INSERT INTO entitlement_grant \
         (org_id, id, plan, granted_by, source_ref, granted_at, expires_at) \
         VALUES ($1, gen_random_uuid(), 'subscriber', 'paddle', 'sub_1', \
                 now() - interval '40 days', now() - interval '1 hour')",
    )
    .await;

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    assert_eq!(
        view.entitlement, None,
        "a subscription whose grant has run out stops granting the device any work"
    );
}

// T3, the other half: the fact that an organisation which never subscribed is
// entitled is the one most easily broken by a predicate written the other way
// round, and T1 already asserts it -- ORG_A has no grant at all there.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_plan_still_inside_its_expiry_grants(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;
    pinned(
        &pool,
        ORG_A,
        "INSERT INTO entitlement_grant \
         (org_id, id, plan, granted_by, source_ref, granted_at, expires_at) \
         VALUES ($1, gen_random_uuid(), 'subscriber', 'paddle', 'sub_1', \
                 now() - interval '10 days', now() + interval '20 days')",
    )
    .await;

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    assert!(
        view.entitlement.is_some(),
        "a live grant is what lets the device work between check-ins"
    );
}

// T4
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_tenant_wide_halt_grants_nothing(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;
    pinned(
        &pool,
        ORG_A,
        "INSERT INTO org_halt (org_id, raised_by, reason, raised_at) \
         VALUES ($1, 'operator', 'under investigation', now())",
    )
    .await;

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    assert_eq!(
        view.entitlement, None,
        "a halted tenant works nothing, and the token must not say otherwise"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_write_only_tpt_halt_keeps_the_source_in_the_signed_read_grant(pool: PgPool) {
    let (key, public) = key_pair();
    entitled_device(&pool, &key).await;
    pinned(
        &pool,
        ORG_A,
        "INSERT INTO org_inventory_halt \
         (org_id, inventory, marketplace, raised_by, reason, raised_at, write_only) \
         VALUES ($1, 'tpt', 'tpt', 'operator', 'read-only source', now(), true)",
    )
    .await;

    let answer = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await;
    assert_eq!(answer.status, StatusCode::OK);
    let view = answer.view();
    let claims = verified(
        view.entitlement
            .as_deref()
            .expect("a read-only source still receives a signed read grant"),
        &public,
    );
    assert!(
        claims.marketplaces.contains(&Marketplace::Tpt),
        "the native importer must be able to read TPT without permitting TPT writes"
    );
}

// T4, the fleet kill switch, at marketplace granularity.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn halting_every_inventory_of_one_marketplace_drops_it_from_the_grant(pool: PgPool) {
    let (key, public) = key_pair();
    entitled_device(&pool, &key).await;
    // Tes runs disjoint inventories; the marketplace leaves the grant only
    // when none of them is workable, which is what "any" means.
    for inventory in ["tes"] {
        sqlx::query(
            "INSERT INTO inventory_halt (inventory, marketplace, raised_by, reason, raised_at) \
             VALUES ($1, 'tes', 'operator', 'cease and desist', now())",
        )
        .bind(inventory)
        .execute(&pool)
        .await
        .expect("the halt inserts");
    }

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    let claims = verified(
        view.entitlement.as_deref().expect("Tpt still grants"),
        &public,
    );
    assert_eq!(
        claims.marketplaces,
        vec![Marketplace::Tpt],
        "dropping one marketplace from the grant set is how a cease-and-desist is answered \
         across the installed fleet within one revalidation window"
    );
}

// T4, the "any" rule itself: a marketplace is granted while any of its
// inventories is open. Tes has exactly one since 2026-09-12, so halting it is
// halting the marketplace, and the claim must drop it while TPT stays.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn halting_the_only_tes_inventory_drops_the_marketplace_from_the_grant(pool: PgPool) {
    let (key, public) = key_pair();
    entitled_device(&pool, &key).await;
    sqlx::query(
        "INSERT INTO inventory_halt (inventory, marketplace, raised_by, reason, raised_at) \
         VALUES ('tes', 'tes', 'operator', 'one storefront only', now())",
    )
    .execute(&pool)
    .await
    .expect("the halt inserts");

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    let claims = verified(
        view.entitlement.as_deref().expect("a token is minted"),
        &public,
    );
    assert_eq!(
        claims.marketplaces,
        vec![Marketplace::Tpt],
        "with one Tes inventory a halt on it is a halt on the marketplace, so the device \
         may not work Tes until it lifts; TPT is untouched"
    );
}

// T5
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_marketplace_with_no_linked_connection_is_not_granted(pool: PgPool) {
    let (key, public) = key_pair();
    provision(&pool).await;
    register(&pool, &key, &TOKEN_A, LAPTOP).await;
    link(&pool, ORG_A, "tpt", [0x05; 16]).await;

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    let claims = verified(view.entitlement.as_deref().expect("Tpt is linked"), &public);
    assert_eq!(
        claims.marketplaces,
        vec![Marketplace::Tpt],
        "a marketplace the seller has not linked is not one this device may work"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_with_nothing_linked_is_granted_nothing(pool: PgPool) {
    let (key, _public) = key_pair();
    provision(&pool).await;
    register(&pool, &key, &TOKEN_A, LAPTOP).await;

    let view = beat(&pool, Some(key), &TOKEN_A, LAPTOP).await.view();
    assert_eq!(
        view.entitlement, None,
        "an empty grant set is no token at all, which the device reads as a closed gate"
    );
}

// T6
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_deployment_with_no_signing_key_still_answers_the_check_in(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;

    let answer = beat(&pool, None, &TOKEN_A, LAPTOP).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "a heartbeat's first job is delivering a revocation, so it must not refuse for want \
         of a signing key"
    );
    let raw: serde_json::Value =
        serde_json::from_slice(&answer.body).expect("the answer body parses");
    assert_eq!(
        raw.as_object().map(serde_json::Map::len),
        Some(2),
        "and the bytes are exactly what this route answered before the mint existed, which \
         is what keeps the published 0.1.3 client working: {raw}"
    );
    assert!(raw.get("entitlement").is_none());
}

// S2's second half: the `d.org_id = $1` clause inside `entitled_marketplaces`
// is defence in depth behind the route's own 404, and no route-level test would
// notice its removal. This calls the repository directly.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_repository_grants_nothing_for_another_tenants_device(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;
    let devices = tam_storage::DeviceRepo::new(pool.clone());

    let mine = devices
        .entitled_marketplaces(ORG_A, LAPTOP)
        .await
        .expect("the read runs");
    assert!(
        !mine.is_empty(),
        "the fixture's premise: this device is entitled under its own tenant, or the \
         assertion below is vacuous"
    );

    let theirs = devices
        .entitled_marketplaces(ORG_B, LAPTOP)
        .await
        .expect("the read runs");
    assert!(
        theirs.is_empty(),
        "org B naming org A's device id must reach nothing even at the repository, where no \
         route has already refused it: {theirs:?}"
    );
}

// T7
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenant_cannot_obtain_a_token_for_another_tenants_device(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;

    let answer = beat(&pool, Some(key), &TOKEN_B, LAPTOP).await;
    assert_eq!(
        answer.status,
        StatusCode::NOT_FOUND,
        "org B naming org A's device reaches nothing, so there is no answer to mint into"
    );
    assert!(
        !String::from_utf8_lossy(&answer.body).contains("entitlement"),
        "and nothing token-shaped leaves in the refusal"
    );
}

// T7, the unauthenticated case.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unauthenticated_check_in_is_minted_nothing(pool: PgPool) {
    let (key, _public) = key_pair();
    entitled_device(&pool, &key).await;

    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/devices/{LAPTOP}/heartbeat"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"sessions":[]}"#))
        .expect("the request builds");
    let response = router(state(pool, Some(key)))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
