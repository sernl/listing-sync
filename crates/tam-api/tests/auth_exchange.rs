//! The identity bridge end to end: a login assertion signed by a test
//! Ed25519 key, served through a fake key set, exchanged for the same
//! `tam_session` cookie the break-glass token produces.
//!
//! Every refusal below is driven by a token that differs from the accepted
//! one in exactly one respect, so a verifier that skipped that one check
//! would pass its neighbours and fail here.

#![cfg(feature = "pg-tests")]

use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use base64::Engine as _;
use http_body_util::BodyExt;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use sqlx::PgPool;
use tam_api::auth::JWKS_REFETCH_COOLDOWN_MS;
use tam_api::{
    router, APIError, APIErrorCode, AppState, AuthBridge, Config, JwkSet, JwksFuture, JwksSource,
    JwksUnavailable, Whoami, AUDIENCE, SESSION_COOKIE,
};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

/// The issuer the bridge is configured with. Every accepted assertion names
/// exactly this.
const ISSUER: &str = "https://auth.example.test";
const KID: &str = "test-key-1";

const NOW_SECS: i64 = 1_700_000_000;
const NOW: Timestamp = Timestamp(NOW_SECS * 1_000);
const LIVE_EXP: i64 = NOW_SECS + 120;

fn now() -> Timestamp {
    NOW
}

fn after_the_cooldown() -> Timestamp {
    Timestamp(NOW.0 + JWKS_REFETCH_COOLDOWN_MS)
}

/// A signing key and the key set that publishes its public half. Generated
/// per test run rather than pinned: nothing here depends on the key's value,
/// only on the two halves agreeing.
struct TestKey {
    encoding: EncodingKey,
    public: Vec<u8>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn test_key() -> TestKey {
    let random = ring::rand::SystemRandom::new();
    let pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&random).expect("the test key generates");
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .expect("the generated key parses");
    TestKey {
        encoding: EncodingKey::from_ed_der(pkcs8.as_ref()),
        public: ring::signature::KeyPair::public_key(&pair)
            .as_ref()
            .to_vec(),
    }
}

/// The published key set, in the wire shape the identity service serves at
/// `/api/auth/jwks`.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn key_set(kid: &str, public: &[u8]) -> JwkSet {
    let document = serde_json::json!({
        "keys": [{
            "kty": "OKP",
            "crv": "Ed25519",
            "alg": "EdDSA",
            "use": "sig",
            "kid": kid,
            "x": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public),
        }]
    });
    serde_json::from_value(document).expect("the key set parses in the shape the bridge reads")
}

/// The key set as a value, counting how many times it was asked for. The
/// count is the whole point of the refetch-bound test: an unbounded
/// implementation reaches here once per request.
struct CountingJwks {
    document: JwkSet,
    fetches: Arc<AtomicUsize>,
}

impl JwksSource for CountingJwks {
    fn fetch(&self) -> JwksFuture<'_> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        let document = self.document.clone();
        Box::pin(async move { Ok(document) })
    }
}

/// A source that never answers, for the case where the identity service is
/// down: the exchange must refuse rather than fault.
struct DeadJwks;

impl JwksSource for DeadJwks {
    fn fetch(&self) -> JwksFuture<'_> {
        Box::pin(async { Err(JwksUnavailable("the test source is down".to_owned())) })
    }
}

fn bridge(key: &TestKey, fetches: &Arc<AtomicUsize>) -> Arc<AuthBridge> {
    Arc::new(AuthBridge::new(
        ISSUER.to_owned(),
        Box::new(CountingJwks {
            document: key_set(KID, &key.public),
            fetches: Arc::clone(fetches),
        }),
    ))
}

fn state(pool: PgPool, auth: Option<Arc<AuthBridge>>) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: now,
        auth,
        backoffice: None,
        blobs: None,
    }
}

/// The claim set the contract fixes, with every field a caller can vary so a
/// test can change exactly one of them.
fn claims(
    subject: uuid::Uuid,
    issuer: &str,
    audience: &str,
    exp: i64,
    email_verified: bool,
) -> serde_json::Value {
    serde_json::json!({
        "sub": subject.to_string(),
        "iss": issuer,
        "aud": audience,
        "iat": NOW_SECS,
        "exp": exp,
        "email_verified": email_verified,
    })
}

fn good_claims(subject: uuid::Uuid) -> serde_json::Value {
    claims(subject, ISSUER, AUDIENCE, LIVE_EXP, true)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn assertion(key: &EncodingKey, kid: &str, claims: &serde_json::Value) -> String {
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(kid.to_owned());
    encode(&header, claims, key).expect("the test assertion signs")
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
    set_cookie: Option<String>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn post_session(state: AppState, token: &str) -> Answer {
    let request = Request::builder()
        .method(axum::http::Method::POST)
        .uri("/v1/session")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "token": token }).to_string(),
        ))
        .expect("the request builds");
    let response = router(state)
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
    Answer {
        status,
        body,
        set_cookie,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn refusal_is_the_closed_vocabulary(answer: &Answer, what: &str) {
    assert_eq!(
        answer.status,
        StatusCode::UNAUTHORIZED,
        "{what} must be refused"
    );
    let error: APIError = serde_json::from_slice(&answer.body).expect("the body is the error");
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::SessionRequired),
        "{what} is the same refusal as every other, so it is no oracle"
    );
    assert!(
        answer.set_cookie.is_none(),
        "{what} must not set a session cookie"
    );
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn tenant_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM organisation")
        .fetch_one(pool)
        .await
        .expect("the organisations count")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn user_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM app_user")
        .fetch_one(pool)
        .await
        .expect("the users count")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_first_assertion_provisions_a_tenant_and_mints_a_session(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    let token = assertion(&key.encoding, KID, &good_claims(subject));

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "a valid assertion mints a session: {}",
        String::from_utf8_lossy(&answer.body)
    );

    let whoami: Whoami =
        serde_json::from_slice(&answer.body).expect("the exchange echoes the identity");
    assert_eq!(
        tenant_count(&pool).await,
        1,
        "an unknown subject creates exactly one organisation"
    );
    assert_eq!(user_count(&pool).await, 1, "and exactly one user under it");

    let linked = SessionRepo::new(pool.clone())
        .user_by_auth_subject(Uuid(*subject.as_bytes()))
        .await
        .expect("the link reads back")
        .expect("the subject is linked to the provisioned user");
    assert_eq!(
        linked,
        (whoami.org, whoami.user),
        "the row the exchange wrote is the identity it answered with, keyed on auth_subject"
    );

    // The cookie it set authenticates the ordinary session path, which is the
    // whole point of exchanging rather than carrying the assertion onward.
    let cookie = answer.set_cookie.expect("the exchange sets the cookie");
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && cookie.contains("Secure"),
        "the minted cookie carries its protections: {cookie}"
    );
    let pair = cookie.split(';').next().expect("the cookie has a pair");
    let request = Request::builder()
        .uri("/v1/whoami")
        .header(header::COOKIE, pair)
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool, None))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the exchanged cookie authenticates like any other session"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_known_subject_reuses_its_tenant_rather_than_creating_a_second(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    let token = assertion(&key.encoding, KID, &good_claims(subject));
    let shared = bridge(&key, &fetches);

    let first = post_session(state(pool.clone(), Some(Arc::clone(&shared))), &token).await;
    assert_eq!(first.status, StatusCode::OK, "the first login provisions");
    let first_identity: Whoami =
        serde_json::from_slice(&first.body).expect("the first exchange echoes the identity");

    let second = post_session(state(pool.clone(), Some(shared)), &token).await;
    assert_eq!(second.status, StatusCode::OK, "the second login resolves");
    let second_identity: Whoami =
        serde_json::from_slice(&second.body).expect("the second exchange echoes the identity");

    assert_eq!(
        (second_identity.org, second_identity.user),
        (first_identity.org, first_identity.user),
        "the second login lands in the tenant the first created"
    );
    assert_eq!(
        tenant_count(&pool).await,
        1,
        "a returning subject must not provision a second organisation"
    );
    assert_eq!(user_count(&pool).await, 1, "nor a second user");
    assert_ne!(
        first.set_cookie, second.set_cookie,
        "each exchange mints its own session token rather than re-issuing one"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_expired_assertion_is_refused(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    // One second before the injected clock, so a verifier reading the machine
    // clock instead would wrongly accept it.
    let expired = claims(subject, ISSUER, AUDIENCE, NOW_SECS - 1, true);
    let token = assertion(&key.encoding, KID, &expired);

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an expired assertion");
    assert_eq!(
        tenant_count(&pool).await,
        0,
        "a refused assertion provisions nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_assertion_from_another_issuer_is_refused(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    let wrong = claims(
        subject,
        "https://auth.attacker.test",
        AUDIENCE,
        LIVE_EXP,
        true,
    );
    let token = assertion(&key.encoding, KID, &wrong);

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion naming another issuer");
    assert_eq!(tenant_count(&pool).await, 0, "and provisions nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_assertion_for_another_audience_is_refused(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    // Minted by the right issuer, under the right key, for a different
    // service: only the audience check stands between this and a session.
    let wrong = claims(subject, ISSUER, "some-other-service", LIVE_EXP, true);
    let token = assertion(&key.encoding, KID, &wrong);

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion minted for another audience");
    assert_eq!(tenant_count(&pool).await, 0, "and provisions nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_assertion_signed_by_another_key_is_refused(pool: PgPool) {
    let published = test_key();
    let impostor = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    // Every claim is right and the `kid` names the published key; only the
    // signature was made with a key the identity service never published.
    let token = assertion(&impostor.encoding, KID, &good_claims(subject));

    let answer = post_session(
        state(pool.clone(), Some(bridge(&published, &fetches))),
        &token,
    )
    .await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion signed by an unpublished key");
    assert_eq!(tenant_count(&pool).await, 0, "and provisions nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unverified_address_is_refused(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    let unverified = claims(subject, ISSUER, AUDIENCE, LIVE_EXP, false);
    let token = assertion(&key.encoding, KID, &unverified);

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion whose address is unverified");
    assert_eq!(
        tenant_count(&pool).await,
        0,
        "signup is gated on verification, so an unverified subject creates no tenant"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_assertion_missing_the_verification_claim_is_refused(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    // The claim absent rather than false: a verifier that read it as an
    // `Option` defaulting to `None` and treated that as acceptable would let
    // an issuer misconfiguration through.
    let silent = serde_json::json!({
        "sub": subject.to_string(),
        "iss": ISSUER,
        "aud": AUDIENCE,
        "iat": NOW_SECS,
        "exp": LIVE_EXP,
    });
    let token = assertion(&key.encoding, KID, &silent);

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion that omits email_verified");
    assert_eq!(tenant_count(&pool).await, 0, "and provisions nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_symmetric_assertion_is_refused_before_any_key_is_looked_up(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    // The classic substitution: the token declares HS256 and is signed with
    // the published public key as the shared secret. A verifier that trusted
    // the header's algorithm would compute exactly this MAC and accept.
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(KID.to_owned());
    let token = encode(
        &header,
        &good_claims(subject),
        &EncodingKey::from_secret(&key.public),
    )
    .expect("the substituted assertion signs");

    let answer = post_session(state(pool.clone(), Some(bridge(&key, &fetches))), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion declaring a symmetric algorithm");
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        0,
        "the algorithm is settled before the key set is even consulted"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unknown_kid_refetches_once_per_cooldown_rather_than_once_per_request(pool: PgPool) {
    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let subject = uuid::Uuid::new_v4();
    // Signed by the published key but naming a `kid` the set does not carry,
    // which is what a rotation looks like and what a flood would forge.
    let token = assertion(&key.encoding, "rotated-away", &good_claims(subject));
    let shared = bridge(&key, &fetches);

    for attempt in 1..=5_u32 {
        let answer = post_session(state(pool.clone(), Some(Arc::clone(&shared))), &token).await;
        refusal_is_the_closed_vocabulary(&answer, &format!("attempt {attempt} at an unknown kid"));
    }
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        1,
        "five requests naming an unknown kid reach the identity service once, not five times"
    );

    // The cooldown expires rather than latching, or a genuine key rotation
    // would never be picked up.
    let mut later = state(pool, Some(Arc::clone(&shared)));
    later.wall = after_the_cooldown;
    let answer = post_session(later, &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an unknown kid after the cooldown");
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        2,
        "once the cooldown has passed the key set is fetched again"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unreachable_identity_service_refuses_rather_than_faults(pool: PgPool) {
    let key = test_key();
    let subject = uuid::Uuid::new_v4();
    let token = assertion(&key.encoding, KID, &good_claims(subject));
    let dead = Arc::new(AuthBridge::new(ISSUER.to_owned(), Box::new(DeadJwks)));

    let answer = post_session(state(pool.clone(), Some(dead)), &token).await;
    refusal_is_the_closed_vocabulary(&answer, "an assertion the key set could not be read for");
    assert_eq!(tenant_count(&pool).await, 0, "and provisions nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_assertion_is_refused_where_no_identity_service_is_configured(pool: PgPool) {
    let key = test_key();
    let subject = uuid::Uuid::new_v4();
    let token = assertion(&key.encoding, KID, &good_claims(subject));

    let answer = post_session(state(pool.clone(), None), &token).await;
    refusal_is_the_closed_vocabulary(
        &answer,
        "an assertion presented to a deployment with no identity service",
    );
    assert_eq!(tenant_count(&pool).await, 0, "and provisions nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_break_glass_token_still_exchanges_alongside_the_bridge(pool: PgPool) {
    const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
    const USER: UserId = UserId(Uuid([0x0A; 16]));
    const TOKEN: SessionToken = SessionToken([0x42; 32]);

    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(&pool)
        .await
        .expect("the org seeds");
    let repo = SessionRepo::new(pool.clone());
    repo.create_user(ORG, USER, "founder@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    repo.mint(&TOKEN, USER, Timestamp(NOW.0 + 95_000), Timestamp(1_000))
        .await
        .expect("the session mints");

    let key = test_key();
    let fetches = Arc::new(AtomicUsize::new(0));
    let answer = post_session(
        state(pool, Some(bridge(&key, &fetches))),
        &format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
    )
    .await;

    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the mint tool's token still exchanges with an identity service configured"
    );
    let whoami: Whoami = serde_json::from_slice(&answer.body).expect("the exchange echoes whoami");
    assert_eq!(
        (whoami.org, whoami.user),
        (ORG, USER),
        "and speaks for exactly the identity it was minted for"
    );
    let cookie = answer.set_cookie.expect("the exchange sets the cookie");
    assert!(
        cookie.contains(&TOKEN.to_hex()),
        "the break-glass path re-uses its own token rather than minting a second: {cookie}"
    );
    assert!(
        cookie.contains("Max-Age=95"),
        "Max-Age stays honest to that session's own expiry: {cookie}"
    );
    assert_eq!(
        fetches.load(Ordering::SeqCst),
        0,
        "a token with no dots never reaches the identity service"
    );
}
