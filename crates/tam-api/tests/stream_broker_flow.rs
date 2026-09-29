//! An imported file opened in a browser, passed through from the seller's
//! device and never kept.
//!
//! The fixture is the state the founder meets: a resource whose payload was
//! imported, so its row is marketplace-sourced and the bytes are on one
//! device ("laptop"). The browser reads the file's content route; the test
//! plays the device's side of the channel through the same router: its long
//! poll, its check of the capability, and its answer.

#![cfg(feature = "pg-tests")]

use std::collections::HashSet;

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use sqlx::PgPool;
use tam_api::devices::EntitlementKey;
use tam_api::library::{FileCustody, HolderView, LibraryView};
use tam_api::resources::ProductView;
use tam_api::{broker, router, AppState, Config, SESSION_COOKIE};
use tam_domain::serve::{Claims, StreamRequests, AUDIENCE, CAPABILITY_HEADER, ISSUER, REFUSED_HEADER};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xC1; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xC2; 16]));
const TOKEN_A: SessionToken = SessionToken([0x61; 32]);
const TOKEN_B: SessionToken = SessionToken([0x62; 32]);
const PRODUCT: [u8; 16] = [0x31; 16];
const FILE: [u8; 16] = [0x32; 16];
const RESOURCE: &str = "13549794";
const DEVICE: &str = "device-1";
const NOW: Timestamp = Timestamp(1_756_000_000_000);

/// The imported original: a small PDF-shaped body, so a range and the whole
/// can be told apart.
fn original() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend((0..4_000u32).map(|n| u8::try_from(n % 251).unwrap_or(0)));
    bytes
}

fn hex(bytes: &[u8]) -> String {
    tam_secrets::hex_encode(&tam_pipeline::hash::content_hash(bytes).0)
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

fn configured(pool: PgPool, key: Option<EntitlementKey>, pickup_ms: u64) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            entitlement_key: key,
            broker_timeouts: broker::Timeouts {
                pickup: core::time::Duration::from_millis(pickup_ms),
                frame: core::time::Duration::from_secs(5),
            },
            ..Config::default()
        },
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    /// `Err` where the body broke off mid-way, which is how a browser sees a
    /// device that sent the wrong bytes.
    body: Result<Vec<u8>, String>,
}

impl Answer {
    fn bytes(&self) -> &[u8] {
        self.body.as_deref().unwrap_or_default()
    }

    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(self.bytes()).expect("the answer body parses")
    }

    fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(self.bytes()).into_owned()
    }
}

/// One request through the router, as the browser or the device sends it.
struct Call<'a> {
    method: Method,
    path: &'a str,
    token: &'a SessionToken,
    headers: Vec<(&'a str, String)>,
    body: Vec<u8>,
}

impl<'a> Call<'a> {
    fn get(path: &'a str) -> Self {
        Self {
            method: Method::GET,
            path,
            token: &TOKEN_A,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(state: &AppState, call: Call<'_>) -> Answer {
    let mut request = Request::builder()
        .method(call.method)
        .uri(call.path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", call.token.to_hex()),
        );
    for (name, value) in call.headers {
        request = request.header(name, value);
    }
    let response = router(state.clone())
        .oneshot(request.body(Body::from(call.body)).expect("the request builds"))
        .await
        .expect("the router serves");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .map(|collected| collected.to_bytes().to_vec())
        .map_err(|error| error.to_string());
    Answer {
        status,
        headers,
        body,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn pinned(pool: &PgPool, org: OrgId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    tx
}

/// Two tenants; the first has one resource whose payload was imported and is
/// held by one device.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_A, UserId(Uuid([0x0C; 16])), "a@example.test", TOKEN_A),
        (ORG_B, UserId(Uuid([0x0D; 16])), "b@example.test", TOKEN_B),
    ] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(email)
            .execute(pool)
            .await
            .expect("the org seeds");
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, Timestamp(9_000_000_000_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
    let org = uuid::Uuid::from_bytes(ORG_A.0 .0);
    let product = uuid::Uuid::from_bytes(PRODUCT);
    let body = original();
    let hash = tam_pipeline::hash::content_hash(&body).0;
    let len = i64::try_from(body.len()).expect("small");
    let mut tx = pinned(pool, ORG_A).await;
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tpt', 'linked', now(), now())",
    )
    .bind(org)
    .bind(uuid::Uuid::from_bytes([0x33; 16]))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    sqlx::query(
        "WITH inserted AS ( \
             INSERT INTO product (org_id, id, title, body, body_format, price_kind, \
                 rights_state, created_at, updated_at) \
             VALUES ($1, $2, 'Full Unit: Whole Numbers', 'body', 'markdown', 'free', \
                 'unstated', now(), now()) \
             RETURNING org_id, id \
         ) INSERT INTO grade_declaration (org_id, product_id, source) \
           SELECT org_id, id, 'seller' FROM inserted",
    )
    .bind(org)
    .bind(product)
    .execute(&mut *tx)
    .await
    .expect("the product inserts");
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'laptop', 'linux', 'x86_64', '0.16.0', \
             to_timestamp($3::float8 / 1000), to_timestamp($3::float8 / 1000))",
    )
    .bind(org)
    .bind(DEVICE)
    .bind(NOW.0)
    .execute(&mut *tx)
    .await
    .expect("the device that holds the file registers");
    sqlx::query(
        "INSERT INTO device_library_holding (org_id, device_id, hash, byte_len, reported_at) \
         VALUES ($1, $2, $3, $4, now())",
    )
    .bind(org)
    .bind(DEVICE)
    .bind(hash.as_slice())
    .bind(len)
    .execute(&mut *tx)
    .await
    .expect("the device reports holding the file");
    sqlx::query(
        "INSERT INTO product_file (org_id, id, product_id, position, role, kind, created_at, \
             source_marketplace, source_connection, source_resource, \
             observed_hash, observed_byte_len, asserted_scan_state, observed_by_device, \
             observed_at, recorded_at, payload_file_name, payload_content_type) \
         VALUES ($1, $2, $3, 0, 'payload', 'pdf', now(), 'tpt', $4, $5, $6, $7, 'pending', \
             $8, now(), now(), $9, 'application/pdf')",
    )
    .bind(org)
    .bind(uuid::Uuid::from_bytes(FILE))
    .bind(product)
    .bind(uuid::Uuid::from_bytes([0x33; 16]))
    .bind(RESOURCE)
    .bind(hash.as_slice())
    .bind(len)
    .bind(DEVICE)
    .bind(format!("{RESOURCE}.pdf"))
    .execute(&mut *tx)
    .await
    .expect("the imported payload inserts");
    tx.commit().await.expect("the fixture commits");
}

fn content_path() -> String {
    format!(
        "/v1/products/{}/files/{}/content",
        uuid::Uuid::from_bytes(PRODUCT),
        uuid::Uuid::from_bytes(FILE)
    )
}

fn poll_path(wait_ms: u64) -> String {
    format!("/v1/devices/{DEVICE}/streams?wait_ms={wait_ms}")
}

/// The device's long poll.
async fn poll(state: &AppState, wait_ms: u64) -> StreamRequests {
    let answer = call(state, Call::get(&poll_path(wait_ms))).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    answer.json()
}

/// The capability, verified as the device verifies it.
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
        .expect("the capability verifies against the key that signed it")
        .claims
}

/// What the device does once it has an ask: its answer, as the test chose.
enum Reply {
    Bytes(fn(&[u8]) -> Vec<u8>),
    Refuse(&'static str),
}

/// The device's whole turn: poll until an ask arrives, check it, answer it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn device_turn(state: &AppState, public: &[u8], reply: Reply) -> (Claims, Answer) {
    let mut asks = poll(state, 5_000).await;
    let ask = asks.requests.pop().expect("the browser's read reaches the device");
    let claims = verified(&ask.capability, public);
    let path = format!("/v1/devices/{DEVICE}/streams/{}", ask.stream);
    let (headers, body) = match reply {
        Reply::Bytes(cut) => (
            vec![(CAPABILITY_HEADER, ask.capability.clone())],
            cut(&original()),
        ),
        Reply::Refuse(word) => (
            vec![
                (CAPABILITY_HEADER, ask.capability.clone()),
                (REFUSED_HEADER, word.to_owned()),
            ],
            Vec::new(),
        ),
    };
    let answered = call(
        state,
        Call {
            method: Method::POST,
            path: &path,
            token: &TOKEN_A,
            headers,
            body,
        },
    )
    .await;
    (claims, answered)
}

async fn rows(pool: &PgPool, table: &str) -> i64 {
    let mut tx = pinned(pool, ORG_A).await;
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&mut *tx)
        .await
        .unwrap_or(-1)
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_serving_device_answers_a_range_through_the_broker(pool: PgPool) {
    provision(&pool).await;
    let (key, public) = key_pair();
    let state = configured(pool.clone(), Some(key), 5_000);
    assert!(poll(&state, 0).await.requests.is_empty(), "nothing is asked yet");

    let content = content_path();
    let browser = call(
        &state,
        Call {
            headers: vec![("range", "bytes=100-1099".to_owned())],
            ..Call::get(&content)
        },
    );
    let device = device_turn(&state, &public, Reply::Bytes(|all| all[100..=1099].to_vec()));
    let (browser, (claims, answered)) = tokio::join!(browser, device);

    assert_eq!(answered.status, StatusCode::NO_CONTENT, "{}", answered.text());
    assert_eq!(browser.status, StatusCode::PARTIAL_CONTENT, "{}", browser.text());
    let body = original();
    assert_eq!(browser.bytes(), &body[100..=1099], "the range the device sent is the range served");
    assert_eq!(
        browser.header(header::CONTENT_RANGE),
        Some(format!("bytes 100-1099/{}", body.len()).as_str()),
        "the range names the whole length the catalogue records"
    );
    assert_eq!(browser.header(header::CACHE_CONTROL), Some("private, no-store"));
    assert_eq!(
        (claims.device.as_str(), claims.hash.as_str(), claims.first, claims.last),
        (DEVICE, hex(&body).as_str(), 100, 1099),
        "the capability binds the device, the file and the bytes"
    );
    assert_eq!(rows(&pool, "device_stream").await, 0, "the ask is forgotten once answered");
    assert_eq!(rows(&pool, "blob").await, 0, "no byte of the file was kept");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_whole_file_is_one_answer_and_a_download_names_it(pool: PgPool) {
    provision(&pool).await;
    let (key, public) = key_pair();
    let state = configured(pool, Some(key), 5_000);
    poll(&state, 0).await;

    let download = format!("{}?download=1", content_path());
    let browser = call(&state, Call::get(&download));
    let device = device_turn(&state, &public, Reply::Bytes(<[u8]>::to_vec));
    let (browser, (claims, _)) = tokio::join!(browser, device);

    assert_eq!(browser.status, StatusCode::OK, "{}", browser.text());
    assert_eq!(browser.bytes(), original().as_slice());
    assert_eq!((claims.first, claims.last), (0, 4_008), "the whole file is asked for");
    assert!(
        browser
            .header(header::CONTENT_DISPOSITION)
            .is_some_and(|value| value.starts_with("attachment")),
        "a download is an attachment"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_that_is_not_polling_is_offline_by_name(pool: PgPool) {
    provision(&pool).await;
    let (key, _public) = key_pair();
    let state = configured(pool, Some(key), 5_000);

    for path in [content_path(), format!("{}?probe=1", content_path())] {
        let answer = call(&state, Call::get(&path)).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{}", answer.text());
        let body: serde_json::Value = answer.json();
        assert_eq!(body["errors"][0]["code"], "device_offline");
        assert_eq!(body["errors"][0]["detail"]["device"], "laptop");
        assert_eq!(
            body["errors"][0]["message"],
            "Your file is on laptop, which is offline. Open the Teachouse app there."
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_probe_answers_no_content_while_a_device_serves(pool: PgPool) {
    provision(&pool).await;
    let (key, _public) = key_pair();
    let state = configured(pool.clone(), Some(key), 5_000);
    poll(&state, 0).await;

    let probe = call(&state, Call::get(&format!("{}?probe=1", content_path()))).await;
    assert_eq!(probe.status, StatusCode::NO_CONTENT, "{}", probe.text());
    assert_eq!(rows(&pool, "device_stream").await, 0, "a probe asks the device nothing");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_that_never_picks_up_is_offline(pool: PgPool) {
    provision(&pool).await;
    let (key, _public) = key_pair();
    let state = configured(pool.clone(), Some(key), 300);
    poll(&state, 0).await;

    let answer = call(&state, Call::get(&content_path())).await;
    assert_eq!(answer.status, StatusCode::CONFLICT, "{}", answer.text());
    let body: serde_json::Value = answer.json();
    assert_eq!(body["errors"][0]["code"], "device_offline");
    assert_eq!(rows(&pool, "device_stream").await, 0, "the unanswered ask is withdrawn");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_file_the_device_no_longer_has_is_nowhere(pool: PgPool) {
    provision(&pool).await;
    let (key, public) = key_pair();
    let state = configured(pool, Some(key), 5_000);
    poll(&state, 0).await;

    let content = content_path();
    let browser = call(&state, Call::get(&content));
    let device = device_turn(&state, &public, Reply::Refuse("missing"));
    let (browser, (_, answered)) = tokio::join!(browser, device);
    assert_eq!(answered.status, StatusCode::NO_CONTENT, "a refusal is received");
    assert_eq!(browser.status, StatusCode::NOT_FOUND, "{}", browser.text());
    assert_eq!(
        browser.json::<serde_json::Value>()["errors"][0]["message"],
        broker::NOWHERE
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_file_no_device_reports_is_nowhere(pool: PgPool) {
    provision(&pool).await;
    let (key, _public) = key_pair();
    let state = configured(pool.clone(), Some(key), 5_000);
    let mut tx = pinned(&pool, ORG_A).await;
    sqlx::query("DELETE FROM device_library_holding")
        .execute(&mut *tx)
        .await
        .unwrap_or_default();
    tx.commit().await.unwrap_or_default();

    let answer = call(&state, Call::get(&content_path())).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.text());
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_answer_short_of_the_range_breaks_the_download(pool: PgPool) {
    provision(&pool).await;
    let (key, public) = key_pair();
    let state = configured(pool, Some(key), 5_000);
    poll(&state, 0).await;

    let content = content_path();
    let browser = call(&state, Call::get(&content));
    let device = device_turn(&state, &public, Reply::Bytes(|all| all[..10].to_vec()));
    let (browser, (_, answered)) = tokio::join!(browser, device);
    assert_eq!(
        answered.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the device is told its answer was wrong"
    );
    assert_eq!(browser.status, StatusCode::OK, "the headers were already sent");
    assert!(browser.body.is_err(), "the browser sees a broken download, not a short file");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_tenant_can_neither_poll_nor_answer(pool: PgPool) {
    provision(&pool).await;
    let (key, _public) = key_pair();
    let state = configured(pool, Some(key), 5_000);
    poll(&state, 0).await;

    let foreign = call(
        &state,
        Call {
            token: &TOKEN_B,
            ..Call::get(&poll_path(0))
        },
    )
    .await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND, "the device is not theirs");

    let content = content_path();
    let browser = call(&state, Call::get(&content));
    let device = async {
        let mut asks = poll(&state, 5_000).await;
        let ask = asks.requests.pop().expect("the ask arrives");
        let path = format!("/v1/devices/{DEVICE}/streams/{}", ask.stream);
        let stolen = call(
            &state,
            Call {
                method: Method::POST,
                path: &path,
                token: &TOKEN_B,
                headers: vec![(CAPABILITY_HEADER, ask.capability.clone())],
                body: b"not the file".to_vec(),
            },
        )
        .await;
        assert_eq!(stolen.status, StatusCode::GONE, "another tenant's answer is refused");
        let forged = call(
            &state,
            Call {
                method: Method::POST,
                path: &path,
                token: &TOKEN_A,
                headers: vec![(CAPABILITY_HEADER, "forged".to_owned())],
                body: b"not the file".to_vec(),
            },
        )
        .await;
        assert_eq!(forged.status, StatusCode::GONE, "an answer without the capability is refused");
        call(
            &state,
            Call {
                method: Method::POST,
                path: &path,
                token: &TOKEN_A,
                headers: vec![(CAPABILITY_HEADER, ask.capability.clone())],
                body: original(),
            },
        )
        .await
    };
    let (browser, answered) = tokio::join!(browser, device);
    assert_eq!(answered.status, StatusCode::NO_CONTENT, "{}", answered.text());
    assert_eq!(browser.bytes(), original().as_slice(), "the real device's bytes are served");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_views_say_where_an_imported_file_is(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool, None, 5_000);

    let product: ProductView = call(
        &state,
        Call::get(&format!("/v1/products/{}", uuid::Uuid::from_bytes(PRODUCT))),
    )
    .await
    .json();
    let payload = product
        .files
        .iter()
        .find(|file| file.role == "payload")
        .map(|file| file.custody.clone());
    assert_eq!(
        payload,
        Some(FileCustody::Devices {
            holders: vec![HolderView {
                device: DEVICE.to_owned(),
                name: "laptop".to_owned(),
                online: true,
            }]
        }),
        "an imported file is on the device that holds it"
    );

    let library: LibraryView = call(&state, Call::get("/v1/library")).await.json();
    assert_eq!(
        library
            .files
            .iter()
            .map(|file| (file.hash.clone(), file.uploaded))
            .collect::<Vec<_>>(),
        vec![(hex(&original()), false)],
        "an imported file is not one the seller uploaded"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_deployment_without_a_signing_key_says_streaming_is_unavailable(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool, None, 5_000);
    poll(&state, 0).await;
    let answer = call(&state, Call::get(&content_path())).await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE, "{}", answer.text());
    assert_eq!(
        answer.json::<serde_json::Value>()["errors"][0]["code"],
        "streaming_unavailable"
    );
}
