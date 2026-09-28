//! Teachouse's own copy of a file an import left on the seller's device.
//!
//! The fixture is the state the founder met: a resource whose payload was
//! imported, so its row is marketplace-sourced and the bytes are on the
//! device only. The device's app asks which files the server lacks, sends
//! the one it holds, and from then on the console can view, range-read and
//! download it from any browser.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::library::{LibraryView, MissingView, ServerCopy};
use tam_api::resources::ProductView;
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
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
const TITLE: &str = "Full Unit: Whole Numbers";
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("server-copy-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("the store root is creatable");
    root
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn configured(pool: PgPool, root: &std::path::Path) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: Some(BlobStore::local(
            tam_secrets::Kek::from_bytes(&[0x5Du8; 32]).expect("a 32-byte key is a key"),
            root.to_path_buf(),
        )),
    }
}

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
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

    fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "one request's six parts, each varied by some caller; a struct would only rename them"
)]
async fn call(
    state: &AppState,
    method: Method,
    path: &str,
    token: &SessionToken,
    range: Option<&str>,
    body: Vec<u8>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    if let Some(range) = range {
        request = request.header(header::RANGE, range);
    }
    let response = router(state.clone())
        .oneshot(request.body(Body::from(body)).expect("the request builds"))
        .await
        .expect("the router serves");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer {
        status,
        headers,
        body,
    }
}

async fn get(state: &AppState, path: &str, range: Option<&str>) -> Answer {
    call(state, Method::GET, path, &TOKEN_A, range, Vec::new()).await
}

async fn put(state: &AppState, path: &str, body: Vec<u8>) -> Answer {
    call(state, Method::PUT, path, &TOKEN_A, None, body).await
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

/// Two tenants; the first has one resource whose payload was imported and
/// never reached the server, beside a cover the import did store.
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
        tam_storage::EntitlementRepo::new(pool.clone())
            .grant(
                org,
                &tam_storage::NewGrant {
                    id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                    plan: tam_limits::Plan::Studio,
                    rung: None,
                    granted_by: tam_storage::GrantedBy::Operator,
                    grantor_user: None,
                    reason: Some("the server copy fixture tenant"),
                    source_ref: None,
                    granted_at: Timestamp(1_000),
                    expires_at: None,
                },
            )
            .await
            .expect("the fixture grant seeds");
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
             VALUES ($1, $2, $3, 'body', 'markdown', 'free', 'unstated', now(), now()) \
             RETURNING org_id, id \
         ) INSERT INTO grade_declaration (org_id, product_id, source) \
           SELECT org_id, id, 'seller' FROM inserted",
    )
    .bind(org)
    .bind(product)
    .bind(TITLE)
    .execute(&mut *tx)
    .await
    .expect("the product inserts");
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
         VALUES ($1, 'device-1', 'laptop', 'windows', 'x86_64', '0.1.0', now(), now())",
    )
    .bind(org)
    .execute(&mut *tx)
    .await
    .expect("the device that read the file registers");
    let body = original();
    sqlx::query(
        "INSERT INTO product_file (org_id, id, product_id, position, role, kind, created_at, \
             source_marketplace, source_connection, source_resource, \
             observed_hash, observed_byte_len, asserted_scan_state, observed_by_device, \
             observed_at, recorded_at, payload_file_name, payload_content_type) \
         VALUES ($1, $2, $3, 0, 'payload', 'pdf', now(), 'tpt', $4, $5, $6, $7, 'pending', \
             'device-1', now(), now(), $8, 'application/pdf')",
    )
    .bind(org)
    .bind(uuid::Uuid::from_bytes(FILE))
    .bind(product)
    .bind(uuid::Uuid::from_bytes([0x33; 16]))
    .bind(RESOURCE)
    .bind(tam_pipeline::hash::content_hash(&body).0.as_slice())
    .bind(i64::try_from(body.len()).expect("small"))
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

fn product_path() -> String {
    format!("/v1/products/{}", uuid::Uuid::from_bytes(PRODUCT))
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn payload_view(state: &AppState) -> tam_api::resources::FileView {
    let answer = get(state, &product_path(), None).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    answer
        .json::<ProductView>()
        .files
        .into_iter()
        .find(|file| file.role == "payload")
        .expect("the payload is listed")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_missing_route_names_the_imported_file_until_the_device_copies_it(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool, &store_root("missing"));
    let body = original();

    let missing: MissingView = get(&state, "/v1/library/missing", None).await.json();
    assert_eq!(
        missing
            .files
            .iter()
            .map(|file| (file.hash.clone(), file.byte_len))
            .collect::<Vec<_>>(),
        vec![(hex(&body), u64::try_from(body.len()).unwrap_or(0))],
        "the imported payload is the one file the server lacks"
    );
    assert_eq!(missing.copy_bytes_max, tam_api::library::COPY_BYTES_MAX);
    assert_eq!(
        missing.storage_bytes_max,
        tam_limits::Plan::Studio
            .capabilities(None)
            .storage_bytes_max
    );
    let other: MissingView = call(
        &state,
        Method::GET,
        "/v1/library/missing",
        &TOKEN_B,
        None,
        Vec::new(),
    )
    .await
    .json();
    assert!(
        other.files.is_empty(),
        "another tenant's import is not this seller's to copy"
    );

    let before = payload_view(&state).await;
    assert_eq!(before.server_copy, ServerCopy::DeviceOnly);
    assert_eq!(
        before.name.as_deref(),
        Some("Full Unit: Whole Numbers.pdf"),
        "a file the device named by the marketplace's number reads as the resource's title"
    );
    let waiting = get(&state, &content_path(), None).await;
    assert_eq!(waiting.status, StatusCode::NOT_FOUND);
    assert!(
        waiting.text().contains("on your device only"),
        "{}",
        waiting.text()
    );

    let copied = put(
        &state,
        &format!("/v1/library/files/{}", hex(&body)),
        body.clone(),
    )
    .await;
    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.text());
    let again = put(
        &state,
        &format!("/v1/library/files/{}", hex(&body)),
        body.clone(),
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "a repeated copy writes nothing"
    );

    let after: MissingView = get(&state, "/v1/library/missing", None).await.json();
    assert!(
        after.files.is_empty(),
        "the copy landed, so nothing is missing"
    );
    assert_eq!(after.stored_bytes, u64::try_from(body.len()).unwrap_or(0));
    assert_eq!(payload_view(&state).await.server_copy, ServerCopy::Stored);

    let library: LibraryView = get(&state, "/v1/library?linked=linked", None).await.json();
    assert_eq!(
        library
            .files
            .iter()
            .find(|file| file.hash == hex(&body))
            .map(|file| file.server_copy),
        Some(ServerCopy::Stored),
        "the file browser says Teachouse holds it too"
    );

    let whole = get(&state, &content_path(), None).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(
        whole.body, body,
        "the console reads back the device's bytes"
    );
    let by_digest = get(
        &state,
        &format!("/v1/library/files/{}/content?download=1", hex(&body)),
        None,
    )
    .await;
    assert_eq!(by_digest.status, StatusCode::OK);
    assert_eq!(by_digest.body, body);
    assert!(by_digest
        .header(header::CONTENT_DISPOSITION)
        .is_some_and(|value| value.starts_with("attachment;")));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_copy_is_refused_for_other_bytes_unknown_files_and_a_full_plan(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("refused"));
    let body = original();
    let path = format!("/v1/library/files/{}", hex(&body));

    let mut forged = body.clone();
    forged.push(0);
    let wrong = put(&state, &path, forged).await;
    assert_eq!(
        wrong.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        wrong.text()
    );

    let stray = b"%PDF-1.7 not any resource's file".to_vec();
    let unknown = put(&state, &format!("/v1/library/files/{}", hex(&stray)), stray).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{}", unknown.text());

    let other = call(&state, Method::PUT, &path, &TOKEN_B, None, body.clone()).await;
    assert_eq!(
        other.status,
        StatusCode::NOT_FOUND,
        "another tenant cannot copy bytes into a resource it does not have"
    );

    // Fill the plan to within a few bytes of its ceiling.
    let cap = tam_limits::Plan::Studio
        .capabilities(None)
        .storage_bytes_max;
    let mut tx = pinned(&pool, ORG_A).await;
    sqlx::query(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, $3, 'fixture', 1, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind([0xEEu8; 32].as_slice())
    .bind(i64::try_from(cap - 10).unwrap_or(i64::MAX))
    .execute(&mut *tx)
    .await
    .expect("the filler blob inserts");
    tx.commit().await.expect("the filler commits");

    assert_eq!(
        payload_view(&state).await.server_copy,
        ServerCopy::StorageFull
    );
    let full = put(&state, &path, body).await;
    assert_eq!(full.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(full.text().contains("quota_exceeded"), "{}", full.text());
    let missing: MissingView = get(&state, "/v1/library/missing", None).await.json();
    assert_eq!(missing.files.len(), 1, "nothing was stored");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_content_route_answers_ranges_and_downloads(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool, &store_root("ranges"));
    let body = original();
    let len = body.len();
    let copied = put(
        &state,
        &format!("/v1/library/files/{}", hex(&body)),
        body.clone(),
    )
    .await;
    assert_eq!(copied.status, StatusCode::CREATED, "{}", copied.text());

    let whole = get(&state, &content_path(), None).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(whole.header(header::ACCEPT_RANGES), Some("bytes"));
    assert_eq!(whole.header(header::CONTENT_TYPE), Some("application/pdf"));
    assert_eq!(
        whole.header(header::CONTENT_LENGTH),
        Some(len.to_string().as_str())
    );
    assert!(whole
        .header(header::CONTENT_DISPOSITION)
        .is_some_and(
            |value| value.starts_with("inline;") && value.contains("Full Unit: Whole Numbers.pdf")
        ));

    let head = get(&state, &content_path(), Some("bytes=0-3")).await;
    assert_eq!(head.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(head.body, b"%PDF");
    let expected = format!("bytes 0-3/{len}");
    assert_eq!(head.header(header::CONTENT_RANGE), Some(expected.as_str()));
    assert_eq!(head.header(header::CONTENT_LENGTH), Some("4"));

    let open = get(
        &state,
        &content_path(),
        Some(&format!("bytes={}-", len - 5)),
    )
    .await;
    assert_eq!(open.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(open.body, body[len - 5..].to_vec());

    let tail = get(&state, &content_path(), Some("bytes=-7")).await;
    assert_eq!(tail.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(tail.body, body[len - 7..].to_vec());

    let past = get(&state, &content_path(), Some(&format!("bytes={len}-"))).await;
    assert_eq!(past.status, StatusCode::RANGE_NOT_SATISFIABLE);
    let unsatisfied = format!("bytes */{len}");
    assert_eq!(
        past.header(header::CONTENT_RANGE),
        Some(unsatisfied.as_str())
    );

    let several = get(&state, &content_path(), Some("bytes=0-1,4-5")).await;
    assert_eq!(
        several.status,
        StatusCode::OK,
        "several ranges are answered whole"
    );
    assert_eq!(several.body, body);

    let saved = get(&state, &format!("{}?download=1", content_path()), None).await;
    assert_eq!(saved.status, StatusCode::OK);
    assert!(saved
        .header(header::CONTENT_DISPOSITION)
        .is_some_and(|value| value.starts_with("attachment;")));
    assert_eq!(saved.body, body);
}
