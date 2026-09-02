//! Authoring over the wire: the upload, the create, the edit, the delete and
//! the vocabulary the form is driven by.
//!
//! Every byte here travels the real pipeline into a real per-tenant sealed
//! blob under a temporary store root, because the point of the upload route
//! is that it runs the same ingest the operator import does; a fake sink
//! would prove the handler's shape and none of that.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::catalogue::{CreatedProductView, DeletedProductView, UploadedView};
use tam_api::resources::ProductView;
use tam_api::taxonomy::TermsView;
use tam_api::vocabulary::VocabularyView;
use tam_api::{router, APIError, APIErrorCode, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_domain::product::{AnswerKey, TaxCode};
use tam_storage::{
    ElectionRepo, MappingRepo, ProductRepo, SessionRepo, SessionToken, TaxonomyRepo, TptBaseRepo,
};
use tam_types::{CanonicalTermId, InventoryId, OrgId, ProductId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

/// The free tier's own listing ceiling, which is what the quota refusal binds
/// against. Read from `tam-limits` rather than restated, so a re-priced tier
/// moves the test with it.
fn free_listings_max() -> u32 {
    tam_limits::Tier::Free.quota().listings_max
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("catalogue-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("the store root is creatable");
    root
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn configured(pool: PgPool, root: &std::path::Path) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: Some(BlobStore {
            kek: tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
            root: root.to_path_buf(),
        }),
    }
}

fn unconfigured(pool: PgPool) -> AppState {
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

/// One request, bundled because the workspace argument limit is five and a
/// request genuinely carries more than that.
struct Call<'a> {
    method: Method,
    path: &'a str,
    body: Option<Body>,
    content_type: Option<&'a str>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(state: AppState, token: &SessionToken, call: Call<'_>) -> (StatusCode, Vec<u8>) {
    let Call {
        method,
        path,
        body,
        content_type,
    } = call;
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    if let Some(kind) = content_type {
        request = request.header(header::CONTENT_TYPE, kind);
    }
    let request = request
        .body(body.unwrap_or_else(Body::empty))
        .expect("the request builds");
    let response = router(state)
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

async fn get(state: AppState, token: &SessionToken, path: &str) -> (StatusCode, Vec<u8>) {
    call(
        state,
        token,
        Call {
            method: Method::GET,
            path,
            body: None,
            content_type: None,
        },
    )
    .await
}

async fn json_call(
    state: AppState,
    token: &SessionToken,
    method: Method,
    path: &str,
    body: &serde_json::Value,
) -> (StatusCode, Vec<u8>) {
    call(
        state,
        token,
        Call {
            method,
            path,
            body: Some(Body::from(body.to_string())),
            content_type: Some("application/json"),
        },
    )
    .await
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).expect("the body parses")
}

fn pdf(marker: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(marker.as_bytes());
    bytes
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write as _;
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("the entry starts");
        writer.write_all(bytes).expect("the entry writes");
    }
    writer.finish().expect("the archive finishes");
    buffer.into_inner()
}

async fn upload(
    state: AppState,
    token: &SessionToken,
    bytes: Vec<u8>,
    query: &str,
) -> UploadedView {
    let (status, body) = call(
        state,
        token,
        Call {
            method: Method::POST,
            path: &format!("/v1/uploads{query}"),
            body: Some(Body::from(bytes)),
            content_type: Some("application/octet-stream"),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the upload is accepted: {}",
        String::from_utf8_lossy(&body)
    );
    parse(&body)
}

fn create_body(uploaded: &UploadedView, title: &str, inventories: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "body": "A short description.",
        "body_format": "Markdown",
        "price": "Free",
        "payload": uploaded.payload,
        "cover": uploaded.cover,
        "inventories": inventories,
        "grades": [{
            "inventory": "TesGb",
            "kind": "phase",
            "segments": ["11-14"],
            "native_id": "3"
        }],
        "rights": {
            "inventory": "TesGb",
            "segments": ["CC-BY"],
            "native_id": "CC-BY"
        },
        "elections": [{
            "inventory": "TesGb",
            "axis": "licence",
            "trigger": "supply",
            "trigger_key": "free",
            "answers": [{"segments": ["CC-BY"], "native_id": "CC-BY"}]
        }]
    })
}

/// The TPT-base block that TPT's own form collects and `product` has no
/// column for. Only what the sidecar stores: the title, price, payload and
/// grades travel on the create body itself.
fn tpt_base() -> serde_json::Value {
    serde_json::json!({
        "subject_areas": ["math"],
        "tags": ["centers"],
        "formats": ["easel"],
        "custom_categories": ["Autumn unit"],
        "tax_code_id": 2,
        "additional_licence_minor_units": 405,
        "teaching_duration_id": 6,
        "pages_or_slides": 12,
        // Wire id 4 is "Included with Rubric", which sits at menu position 3.
        // Reading it back as 4 is what proves nothing derived it from a
        // position on the way through the wire and the column.
        "answer_key_id": 4,
        "copyright_declaration_id": 1,
        "status_user": 0
    })
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_create_persists_every_group_the_form_collects(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("tpt-base");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("base"), "").await;

    let mut body = create_body(&uploaded, "Fractions on a number line", &["Tpt"]);
    body["rights"] = serde_json::Value::Null;
    body["elections"] = serde_json::json!([]);
    body["tpt_base"] = tpt_base();
    let (status, created) =
        json_call(state.clone(), &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created)
    );
    let created: CreatedProductView =
        serde_json::from_slice(&created).expect("the create answers its own view");

    let held = TptBaseRepo::new(pool)
        .get(ORG_A, created.product)
        .await
        .expect("the sidecar reads")
        .expect("the create wrote a sidecar row");
    assert_eq!(
        held.details.answer_key.map(AnswerKey::wire_id),
        Some(4),
        "the answer key arrives as its wire id rather than its menu position"
    );
    assert_eq!(
        held.tax_code.map(TaxCode::wire_id),
        Some(2),
        "the tax code the seller designated, which is never chosen for them"
    );
    assert_eq!(
        (
            held.categories.subject_areas.len(),
            held.categories.tags.len(),
            held.categories.formats.len(),
            held.categories.custom_categories.as_slice()
        ),
        (1, 1, 1, ["Autumn unit".to_owned()].as_slice()),
        "each picker's chosen slugs survive verbatim, and the seller's own shelf beside them"
    );
    assert_eq!(held.additional_licence_minor_units, Some(405));
    assert_eq!(held.details.pages_or_slides, Some(12));
}

/// The attestation gate, at the route rather than only in the form: a client
/// that skipped the radio group is refused with the reason rather than
/// writing a product nobody attested to.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_create_without_an_attestation_is_refused_by_name(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("no-attestation");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("bare"), "").await;

    let mut body = create_body(&uploaded, "Unattested", &["Tpt"]);
    body["rights"] = serde_json::Value::Null;
    body["elections"] = serde_json::json!([]);
    let mut base = tpt_base();
    base["copyright_declaration_id"] = serde_json::Value::Null;
    body["tpt_base"] = base;

    let (status, refused) = json_call(state, &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let refused = String::from_utf8_lossy(&refused);
    assert!(
        refused.contains("copyright"),
        "the refusal names the group whose control is unanswered: {refused}"
    );
}

/// A create from a path that does not author on this form writes no sidecar
/// row, which is why every read of that table is an outer join.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_create_without_the_block_writes_no_sidecar_row(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("no-base");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("plain"), "").await;

    let (status, created) = json_call(
        state,
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "Imported elsewhere", &["TesGb"]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let created: CreatedProductView =
        serde_json::from_slice(&created).expect("the create answers its own view");
    assert_eq!(
        TptBaseRepo::new(pool)
            .get(ORG_A, created.product)
            .await
            .expect("the sidecar reads"),
        None,
        "no block is a create that did not author on this form, and absence is a fact"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_upload_then_a_create_lands_a_draft_and_enqueues_nothing(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("create");
    let state = configured(pool.clone(), &root);

    let uploaded = upload(state.clone(), &TOKEN_A, pdf("one"), "").await;
    assert_eq!(
        uploaded.payload.len(),
        1,
        "a single pdf is its own payload file"
    );
    assert_eq!(
        uploaded.cover.kind, "image",
        "the pipeline generated the cover Tes requires"
    );
    assert!(
        uploaded.stored_bytes > 0 && uploaded.storage_bytes_max > 0,
        "the response reports the tenant's headroom"
    );

    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "Fractions unit", &["TesGb", "Tpt"]),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the create lands: {}",
        String::from_utf8_lossy(&body)
    );
    let created: CreatedProductView = parse(&body);
    assert_eq!(
        created.mappings.len(),
        2,
        "one mapping per selected platform"
    );
    assert_eq!(
        created.elections_recorded, 1,
        "the licence the seller answered on the form is recorded before any mapping raised it"
    );

    let mappings = MappingRepo::new(pool.clone())
        .list_for_product(ORG_A, created.product)
        .await
        .unwrap_or_else(|error| panic!("the mappings read back: {error}"));
    for record in &mappings {
        assert_eq!(
            (
                matches!(record.mapping.binding, tam_domain::Binding::Unbound),
                record.mapping.publish
            ),
            (true, tam_domain::PublishMode::DryRun),
            "a created mapping is unbound and dry-run, exactly as the import writes one"
        );
    }

    let settled = ElectionRepo::new(pool.clone())
        .answered_for(ORG_A, created.product)
        .await
        .unwrap_or_else(|error| panic!("the settled elections read back: {error}"));
    assert_eq!(
        settled.len(),
        1,
        "the projection finds the form's answer already settled"
    );

    let path = format!("/v1/products/{}", created.product.0.to_hyphenated());
    let (status, body) = get(state.clone(), &TOKEN_A, &path).await;
    assert_eq!(status, StatusCode::OK);
    let view: ProductView = parse(&body);
    assert_eq!(
        (view.title.as_str(), view.files.len()),
        ("Fractions unit", 2),
        "the payload and the generated cover both read back"
    );
    assert_eq!(
        view.body_format,
        tam_types::CopyFormat::Markdown,
        "the declared body format round-trips, or the edit path would have to guess it"
    );
    assert_eq!(
        view.rights
            .as_ref()
            .and_then(|rights| rights.native_id.clone()),
        Some("CC-BY".to_owned()),
        "the stated rights grant round-trips"
    );
    assert_eq!(
        view.grades.raw.len(),
        1,
        "the seller's grade declaration is kept verbatim"
    );

    let (status, body) = get(state, &TOKEN_A, "/v1/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let jobs: serde_json::Value = parse(&body);
    assert_eq!(
        jobs["jobs"].as_array().map(Vec::len),
        Some(0),
        "the create enqueues nothing; publishing is a second, deliberate action"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_uploads_and_products_are_invisible_to_another(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    let root = store_root("tenancy");
    let state = configured(pool.clone(), &root);

    let uploaded = upload(state.clone(), &TOKEN_A, pdf("private"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "A's product", &["TesGb"]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let created: CreatedProductView = parse(&body);

    let (status, body) = get(state.clone(), &TOKEN_B, "/v1/products").await;
    assert_eq!(status, StatusCode::OK);
    let page: serde_json::Value = parse(&body);
    assert_eq!(
        page["products"].as_array().map(Vec::len),
        Some(0),
        "B's catalogue is empty although A has authored one"
    );

    let path = format!("/v1/products/{}", created.product.0.to_hyphenated());
    let (status, _) = get(state.clone(), &TOKEN_B, &path).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "A's product is not readable by B, not even by naming it"
    );

    // B naming A's bytes is refused: dedup is per tenant, so the hash resolves
    // to no blob of B's and the handle is rejected rather than borrowed.
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_B,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "B's theft", &["TesGb"]),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a handle for another tenant's bytes is refused"
    );
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::UploadRejected));

    let (status, _) = call(
        state.clone(),
        &TOKEN_B,
        Call {
            method: Method::DELETE,
            path: &path,
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "B cannot delete A's product either"
    );
    assert!(
        ProductRepo::new(pool)
            .get(ORG_A, created.product)
            .await
            .unwrap_or_else(|error| panic!("A's product still reads: {error}"))
            .is_some(),
        "and A's product is still there"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_payload_less_create_is_a_validation_answer_rather_than_a_fault(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("payloadless");
    let state = configured(pool, &root);
    let (status, body) = json_call(
        state,
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &serde_json::json!({
            "title": "No bytes",
            "price": "Free",
            "payload": [],
            "inventories": []
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the deferred payload trigger would have made this a 500"
    );
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::PayloadMissing));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_selected_platform_s_required_field_is_refused_by_name(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("required");
    let state = configured(pool, &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("required"), "").await;
    let mut body = create_body(&uploaded, "No licence", &["TesGb"]);
    body["rights"] = serde_json::Value::Null;
    body["elections"] = serde_json::json!([]);
    let (status, response) = json_call(state, &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&response);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::RequiredFieldMissing),
        "Tes declares its licence required and the create carries neither a grant nor an answer"
    );
    assert_eq!(
        error.errors[0]
            .detail
            .as_ref()
            .and_then(|detail| detail["missing"][0]["field"].as_str()),
        Some("licence"),
        "the refusal names the field the seller has to fill in"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_archive_kept_whole_is_one_payload_file_and_exploded_is_several(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("archive");
    let state = configured(pool, &root);
    let archive = zip_of(&[("a.pdf", pdf("a")), ("b.pdf", pdf("b"))]);

    let exploded = upload(state.clone(), &TOKEN_A, archive.clone(), "").await;
    assert_eq!(
        exploded.payload.len(),
        2,
        "the default explodes an archive, which is what the import does"
    );
    let whole = upload(state, &TOKEN_A, archive, "?archive=keep_whole").await;
    assert_eq!(
        whole.payload.len(),
        1,
        "kept whole, a bundle stays the single file a TPT create can carry"
    );
    assert_eq!(
        whole.payload[0].kind, "zip",
        "and it keeps the archive's own kind"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_upload_refuses_cleanly_where_no_store_is_configured(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let (status, body) = call(
        unconfigured(pool),
        &TOKEN_A,
        Call {
            method: Method::POST,
            path: "/v1/uploads",
            body: Some(Body::from(pdf("nowhere"))),
            content_type: Some("application/octet-stream"),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a deployment with no key and no root accepts no bytes rather than pretending"
    );
    let error: APIError = parse(&body);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::BlobStoreUnavailable)
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unrecognised_upload_is_the_seller_s_to_fix(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("unknown");
    let (status, body) = call(
        configured(pool, &root),
        &TOKEN_A,
        Call {
            method: Method::POST,
            path: "/v1/uploads",
            body: Some(Body::from(b"not a known magic at all".to_vec())),
            content_type: Some("application/octet-stream"),
        },
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::UploadRejected));
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_filler_products(pool: &PgPool, org: OrgId, count: i32) {
    let mut tx = pool.begin().await.expect("the fixture transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin applies");
    sqlx::query(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, 4, 'filler', 0, now())",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(vec![0x5Au8; 32])
    .execute(&mut *tx)
    .await
    .expect("the filler blob inserts");
    sqlx::query(
        "WITH made AS ( \
             INSERT INTO product \
             (org_id, id, title, body, body_format, price_kind, rights_state, \
              created_at, updated_at) \
             SELECT $1, gen_random_uuid(), 'filler', '', 'markdown', 'free', 'unstated', \
                    now(), now() \
             FROM generate_series(1, $2) \
             RETURNING id \
         ) \
         INSERT INTO product_file \
         (org_id, id, product_id, position, role, kind, hash, scan_state, created_at) \
         SELECT $1, gen_random_uuid(), made.id, 0, 'payload', 'pdf', $3, 'pending', now() \
         FROM made",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(count)
    .bind(vec![0x5Au8; 32])
    .execute(&mut *tx)
    .await
    .expect("the filler products insert");
    tx.commit().await.expect("the fixture commits");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_listing_quota_refuses_the_create_that_would_exceed_it(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("listings");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("quota"), "").await;

    let ceiling = i32::try_from(free_listings_max()).unwrap_or(i32::MAX);
    seed_filler_products(&pool, ORG_A, ceiling - 1).await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "The last one that fits", &["TesGb"]),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the product that exactly fills the quota is admitted: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, body) = json_call(
        state,
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "One too many", &["TesGb"]),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::QuotaExceeded));
    assert_eq!(
        error.errors[0]
            .detail
            .as_ref()
            .and_then(|detail| detail["quota"].as_str()),
        Some("listings_max"),
        "the refusal names which quota is full"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_storage_quota_refuses_the_upload_before_a_byte_is_sealed(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let mut tx = pool
        .begin()
        .await
        .unwrap_or_else(|e| panic!("tx begins: {e}"));
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("the pin applies: {e}"));
    let full = i64::try_from(tam_limits::Tier::Free.quota().storage_bytes_max).unwrap_or(i64::MAX);
    sqlx::query(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, $3, 'already-full', 0, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(vec![0x11u8; 32])
    .bind(full)
    .execute(&mut *tx)
    .await
    .unwrap_or_else(|e| panic!("the filler blob inserts: {e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("commit: {e}"));

    let root = store_root("storage");
    let (status, body) = call(
        configured(pool, &root),
        &TOKEN_A,
        Call {
            method: Method::POST,
            path: "/v1/uploads",
            body: Some(Body::from(pdf("over"))),
            content_type: Some("application/octet-stream"),
        },
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::QuotaExceeded));
    assert_eq!(
        error.errors[0]
            .detail
            .as_ref()
            .and_then(|detail| detail["quota"].as_str()),
        Some("storage_bytes_max"),
    );
}

/// Binds one of the product's mappings to a live listing, which is what makes
/// the edit refusal and the delete's removal leg reachable.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn bind_live(pool: &PgPool, org: OrgId, product: ProductId, inventory: InventoryId) {
    let mappings = MappingRepo::new(pool.clone())
        .list_for_product(org, product)
        .await
        .expect("the mappings read back");
    let target = mappings
        .into_iter()
        .find(|record| record.mapping.inventory == inventory)
        .expect("the product is mapped onto that inventory");
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    sqlx::query(
        "UPDATE mapping SET binding_state = 'bound', remote_id_kind = $3, remote_url = $4, \
         remote_numeric_id = $5, first_seen_at = now(), verify_state = 'clean', \
         verified_at = now(), lifecycle_state = 'live', lifecycle_since = now() \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes(target.mapping.id.0 .0))
    .bind(if inventory == InventoryId::Tpt {
        "tpt"
    } else {
        "tes"
    })
    .bind(if inventory == InventoryId::Tpt {
        None
    } else {
        Some("https://www.tes.com/teaching-resource/live-77")
    })
    .bind(if inventory == InventoryId::Tpt {
        Some(77i64)
    } else {
        None
    })
    .execute(&mut *tx)
    .await
    .expect("the mapping binds");
    tx.commit().await.expect("the bind commits");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_live_tes_listing_refuses_the_edit_with_the_capability_named(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("edit");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("edit"), "").await;
    let (_, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "Editable", &["TesNz"]),
    )
    .await;
    let created: CreatedProductView = parse(&body);
    let path = format!("/v1/products/{}", created.product.0.to_hyphenated());

    // Unbound, the edit lands.
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PATCH,
        &path,
        &serde_json::json!({"title": "Retitled while still a draft"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an unbound mapping edits freely: {}",
        String::from_utf8_lossy(&body)
    );
    let (_, body) = get(state.clone(), &TOKEN_A, &path).await;
    let view: ProductView = parse(&body);
    assert_eq!(view.title, "Retitled while still a draft");
    assert_eq!(
        view.rights.as_ref().and_then(|r| r.native_id.clone()),
        Some("CC-BY".to_owned()),
        "a field the patch did not name is left as stored rather than erased"
    );

    bind_live(&pool, ORG_A, created.product, InventoryId::TesNz).await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PATCH,
        &path,
        &serde_json::json!({"title": "Retitled while live"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "Tes serves no live-to-live transition, so the edit is refused before it is written"
    );
    let error: APIError = parse(&body);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::UncapturedTransition)
    );
    assert_eq!(
        error.errors[0]
            .detail
            .as_ref()
            .and_then(|detail| detail["blocked"][0]["capability"].as_str()),
        Some("tes.edit_published"),
        "the client can render which capability is missing"
    );
    let (_, body) = get(state, &TOKEN_A, &path).await;
    let view: ProductView = parse(&body);
    assert_eq!(
        view.title, "Retitled while still a draft",
        "and the refused edit wrote nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_delete_refuses_to_strand_a_live_listing_and_enqueues_the_removal_it_elects(
    pool: PgPool,
) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("delete");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("delete"), "").await;
    let (_, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "Deletable", &["TesNz"]),
    )
    .await;
    let created: CreatedProductView = parse(&body);
    let path = format!("/v1/products/{}", created.product.0.to_hyphenated());
    bind_live(&pool, ORG_A, created.product, InventoryId::TesNz).await;

    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::DELETE,
        &path,
        &serde_json::json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a local-only delete would leave a live listing nobody tracks"
    );
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::ListingStillBound));
    assert!(
        ProductRepo::new(pool.clone())
            .get(ORG_A, created.product)
            .await
            .unwrap_or_else(|e| panic!("the product still reads: {e}"))
            .is_some(),
        "and the refusal wrote nothing"
    );

    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::DELETE,
        &path,
        &serde_json::json!({"remove_from": ["TesNz"]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "electing the platform admits it: {}",
        String::from_utf8_lossy(&body)
    );
    let deleted: DeletedProductView = parse(&body);
    assert_eq!(
        deleted.removals.len(),
        1,
        "one removal leg per elected platform, on the ledger every other write travels"
    );
    assert!(deleted.left_live.is_empty(), "nothing was left standing");
    assert!(
        ProductRepo::new(pool.clone())
            .get(ORG_A, created.product)
            .await
            .unwrap_or_else(|e| panic!("the product read runs: {e}"))
            .is_none(),
        "the local delete landed"
    );

    let job = deleted.removals[0].job;
    let (status, body) = get(
        state.clone(),
        &TOKEN_A,
        &format!("/v1/jobs/{}/items", job.0.to_hyphenated()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items: serde_json::Value = parse(&body);
    assert_eq!(
        items["items"].as_array().map(Vec::len),
        Some(1),
        "the removal is one job item whose failures surface on the ledger, not on the delete"
    );

    // A repeated delete replays the first removal rather than enqueuing a
    // second write against a listing the first one already removed.
    let (status, body) = json_call(
        state,
        &TOKEN_A,
        Method::DELETE,
        &path,
        &serde_json::json!({"remove_from": ["TesNz"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let again: DeletedProductView = parse(&body);
    assert_eq!(
        again.removals[0].job, job,
        "the derived request key makes the second delete a replay of the first"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_delete_may_leave_a_live_listing_alone_when_the_seller_says_so(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("leave-live");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("leave"), "").await;
    let (_, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "Left alone", &["TesNz"]),
    )
    .await;
    let created: CreatedProductView = parse(&body);
    bind_live(&pool, ORG_A, created.product, InventoryId::TesNz).await;
    let (status, body) = json_call(
        state,
        &TOKEN_A,
        Method::DELETE,
        &format!("/v1/products/{}", created.product.0.to_hyphenated()),
        &serde_json::json!({"leave_live": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let deleted: DeletedProductView = parse(&body);
    assert_eq!(
        (deleted.removals.len(), deleted.left_live.as_slice()),
        (0, [InventoryId::TesNz].as_slice()),
        "nothing was removed remotely and the response names what was left standing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_vocabulary_endpoint_serves_one_marketplace_s_own_registry(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let state = unconfigured(pool);

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/vocabulary/TesGb").await;
    assert_eq!(status, StatusCode::OK);
    let view: VocabularyView = parse(&body);
    assert_eq!(view.inventory, InventoryId::TesGb);
    assert!(
        view.natives
            .iter()
            .any(|native| native.name == "licence" && native.required),
        "the one required field the registry declares reaches the form"
    );
    assert!(
        view.authoring.licence.is_some(),
        "and so does the free-versus-paid gate the write is subject to"
    );

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/vocabulary/Tpt").await;
    assert_eq!(status, StatusCode::OK);
    let view: VocabularyView = parse(&body);
    assert_eq!(
        view.absent_axes,
        vec![tam_domain::TermKind::Licence],
        "TPT's measured licence absence crosses as a disclosed loss"
    );

    let (status, _) = get(state.clone(), &TOKEN_A, "/v1/vocabulary/Nowhere").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an inventory this server does not serve is a not-found, not an empty registry"
    );

    let (status, _) = call(
        state,
        &SessionToken([0x00; 32]),
        Call {
            method: Method::GET,
            path: "/v1/vocabulary/TesGb",
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the vocabulary is tenant-scoped like every other catalogue read"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_taxonomy_endpoint_enumerates_the_terms_a_create_body_names(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let subject = CanonicalTermId(Uuid([0x51; 16]));
    let topic = CanonicalTermId(Uuid([0x52; 16]));
    let licence = CanonicalTermId(Uuid([0x53; 16]));
    TaxonomyRepo::new(pool.clone())
        .seed(
            &[
                tam_domain::CanonicalTerm {
                    id: subject,
                    kind: tam_domain::TermKind::Subject,
                    parent: None,
                    label: "Maths".to_owned(),
                },
                tam_domain::CanonicalTerm {
                    id: topic,
                    kind: tam_domain::TermKind::Topic,
                    parent: Some(subject),
                    label: "Time".to_owned(),
                },
                tam_domain::CanonicalTerm {
                    id: licence,
                    kind: tam_domain::TermKind::Licence,
                    parent: None,
                    label: "Teaching Resource Licence".to_owned(),
                },
            ],
            &[],
        )
        .await
        .expect("the terms seed");
    let state = unconfigured(pool);

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/taxonomy/terms?kind=subject").await;
    assert_eq!(status, StatusCode::OK);
    let view: TermsView = parse(&body);
    assert_eq!(
        view.terms
            .iter()
            .map(|term| (term.id, term.label.as_str(), term.parent))
            .collect::<Vec<_>>(),
        vec![(subject, "Maths", None)],
        "the subject picker offers the identifier the create body carries, under its own label"
    );

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/taxonomy/terms?kind=topic").await;
    assert_eq!(status, StatusCode::OK);
    let view: TermsView = parse(&body);
    assert_eq!(
        view.terms.first().map(|term| term.parent),
        Some(Some(subject)),
        "a topic names the subject it narrows, so a picker can group them"
    );

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/taxonomy/terms").await;
    assert_eq!(status, StatusCode::OK);
    let view: TermsView = parse(&body);
    assert_eq!(
        view.terms.len(),
        3,
        "no filter serves every kind the relation holds, each labelled by its own"
    );

    let (status, _) = get(
        state.clone(),
        &TOKEN_A,
        "/v1/taxonomy/terms?kind=resourceType",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a spelling this server never issues is refused rather than read as no filter"
    );

    let (status, _) = call(
        state,
        &SessionToken([0x00; 32]),
        Call {
            method: Method::GET,
            path: "/v1/taxonomy/terms",
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the canonical taxonomy is global reference data behind a session, like every other read"
    );
}
