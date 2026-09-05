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
use tam_api::catalogue::{
    AddedFileView, CreatedProductView, DeletedProductView, RemovedFileView, ReplacedFileView,
    ThumbnailView, UploadedView,
};
use tam_api::resources::{FileView, ProductView, ProductsPage};
use tam_api::taxonomy::TermsView;
use tam_api::vocabulary::VocabularyView;
use tam_api::{router, APIError, APIErrorCode, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_domain::product::{AnswerKey, TaxCode};
use tam_storage::{
    ElectionRepo, MappingRepo, ProductRepo, SessionRepo, SessionToken, TaxonomyRepo, TptBaseRepo,
};
use tam_types::{
    CanonicalTermId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome, Timestamp,
    Title, UserId, Uuid,
};
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

/// A thumbnail hash naming bytes nobody uploaded is refused.
///
/// The sidecar's `thumbnail_hashes` travel as bare digests rather than as
/// `FileHandle`s, so they missed the create's own held-bytes check and reached
/// `product_tpt_base` unverified: four invented digests would have been stored
/// and the TPT write would then have pointed at objects that do not exist.
/// Nothing collected thumbnails when that gap opened, which is why it stood.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_thumbnail_hash_this_tenant_never_uploaded_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("thumb-unheld");
    let state = configured(pool, &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("thumbs"), "").await;
    let mut body = create_body(&uploaded, "Invented thumbnails", &["Tpt"]);
    let mut base = tpt_base();
    base["thumbnail_mode"] = serde_json::json!(2);
    base["thumbnail_hashes"] = serde_json::json!(["b".repeat(64)]);
    body["tpt_base"] = base;
    let (status, response) = json_call(state, &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&response);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::UploadRejected));
}

/// A fifth thumbnail is refused by the form's own cap, not by the database.
///
/// Migration 0040 carries `array_length(thumbnail_hashes, 1) <= 4` as a CHECK,
/// and reaching it would be a 500 rather than something a seller can act on.
/// The model refuses first, through `Picker::Thumbnails` and `OverCap`, so this
/// asserts the refusal arrives as a validation answer naming the control.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_fifth_thumbnail_is_refused_by_the_forms_own_cap(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("thumb-cap");
    let state = configured(pool, &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("cap"), "").await;
    let mut body = create_body(&uploaded, "Five thumbnails", &["Tpt"]);
    let mut base = tpt_base();
    base["thumbnail_mode"] = serde_json::json!(2);
    // Five distinct well-formed digests: the count is what is under test, so
    // none of them may collide or be malformed.
    base["thumbnail_hashes"] = serde_json::json!(["a", "b", "c", "d", "e"]
        .map(|mark| mark.repeat(64))
        .to_vec());
    body["tpt_base"] = base;
    let (status, response) = json_call(state, &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the database CHECK would have made this a 500"
    );
    let error: APIError = parse(&response);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::RequiredFieldMissing)
    );
    let refusals = error.errors[0]
        .detail
        .as_ref()
        .map(|detail| detail["refusals"].to_string())
        .unwrap_or_default();
    assert!(
        refusals.contains("Thumbnail"),
        "and it names the control the seller would look at: {refusals}"
    );
}

/// And a thumbnail hash that is not a digest at all never reaches the check
/// above, because the model refuses it first.
///
/// `record_of` reads the sidecar block through `UploadRef::new` before any
/// hash is looked up, so a malformed digest is an authoring refusal rather
/// than an upload one. The two layers refuse different things and the order
/// matters: this asserts the one that actually fires, so a change to either
/// shows up here rather than in a message nobody reads.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_malformed_thumbnail_hash_is_refused_by_the_model_before_the_lookup(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("thumb-malformed");
    let state = configured(pool, &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("thumbs2"), "").await;
    let mut body = create_body(&uploaded, "Malformed thumbnail", &["Tpt"]);
    let mut base = tpt_base();
    base["thumbnail_mode"] = serde_json::json!(2);
    base["thumbnail_hashes"] = serde_json::json!(["not-a-digest"]);
    body["tpt_base"] = base;
    let (status, response) = json_call(state, &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&response);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::RequiredFieldMissing),
        "the model refuses the shape before the create looks any hash up"
    );
    let refusals = error.errors[0]
        .detail
        .as_ref()
        .map(|detail| detail["refusals"].to_string())
        .unwrap_or_default();
    assert!(
        refusals.contains(r#""group":"files""#),
        "and it is raised against the group that holds the thumbnails rather than \
         another: {refusals}"
    );
}

/// A title the domain will not hold is refused on both routes that set one.
///
/// `ProductName` caps a title at 80 UTF-16 units, measured from TPT's own
/// form. The create checked only for blankness and reached the domain type
/// solely when a TPT-base block happened to be present, so a create without
/// one could store a title no marketplace would take — while the operator
/// import, which builds the same type, refused it. The two paths agree now.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_title_over_the_forms_own_cap_is_refused_on_create_and_on_edit(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("long-title");
    let state = configured(pool, &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("title"), "").await;

    let over = "a".repeat(81);
    let body = create_body(&uploaded, &over, &["Tpt"]);
    let (status, response) =
        json_call(state.clone(), &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "81 units is one over"
    );
    let error: APIError = parse(&response);
    assert!(
        error.errors[0].message.contains("80"),
        "the refusal states the cap the seller is against: {}",
        error.errors[0].message
    );

    // Eighty exactly is accepted, so the test pins the boundary rather than
    // only that something long is refused.
    let at_cap = create_body(&uploaded, &"a".repeat(80), &["Tpt"]);
    let (status, created) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &at_cap,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "eighty units is within the cap"
    );
    let made: CreatedProductView = parse(&created);

    let (status, _) = json_call(
        state,
        &TOKEN_A,
        Method::PATCH,
        &format!("/v1/products/{}", made.product.0.to_hyphenated()),
        &serde_json::json!({ "title": over }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "and the edit route holds the same cap the create does"
    );
}

/// D32 end to end: a resource is kept with no file, and pointing it at a
/// marketplace is refused until one is uploaded.
///
/// This is the test the `add_mapping` refusal was written for and could not
/// have until now — `CanonicalProduct.payload` was non-empty by construction,
/// so the guard was a check that could not fire.
///
/// It overlaps `a_payload_less_create_is_a_validation_answer_rather_than_a_fault`
/// on the create deliberately: that test holds both answers of the create rule
/// against each other, and this one carries a create through to the read-back
/// and the mapping refusal, which is the part no other test reaches.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_fileless_resource_is_kept_and_refuses_a_marketplace_until_a_file_arrives(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("fileless");
    let state = configured(pool, &root);

    // No payload, no marketplace: the Teachouse draft the founder asked for.
    let (status, response) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &serde_json::json!({
            "title": "Fractions, still being written",
            "price": "Free",
            "payload": [],
            "inventories": []
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a resource may be kept here with no file"
    );
    let created: CreatedProductView = parse(&response);
    assert!(created.mappings.is_empty());

    // It reads back as carrying no file rather than as a corrupt row.
    let path = format!("/v1/products/{}", created.product.0.to_hyphenated());
    let (status, read) = get(state.clone(), &TOKEN_A, &path).await;
    assert_eq!(status, StatusCode::OK, "and it reads back");
    let view: ProductView = parse(&read);
    assert!(
        view.files.is_empty(),
        "no file rows, rather than a fabricated one: {:?}",
        view.files
    );

    // Pointing it at a marketplace is refused by name until a file exists.
    let (status, refused) = json_call(
        state,
        &TOKEN_A,
        Method::POST,
        &format!("{path}/mappings"),
        &serde_json::json!({ "inventory": "Tpt" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a marketplace listing needs a file buyers can download"
    );
    let error: APIError = parse(&refused);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::PayloadMissing));
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

/// A payload-less create is a validation answer rather than a fault, both ways.
///
/// The name is older than D32 and still describes what it protects: the
/// deferred trigger must never reach a seller as a 500. What moved is the
/// answer on one side of it. A create naming no marketplace is now created —
/// a resource kept on Teachouse — and one naming a marketplace is refused 422.
/// Holding both in one test is what stops the rule being satisfied by a server
/// that refuses everything or one that accepts everything.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_payload_less_create_is_a_validation_answer_rather_than_a_fault(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("payloadless");
    let state = configured(pool, &root);

    let bare = |inventories: serde_json::Value| {
        serde_json::json!({
            "title": "No bytes",
            "price": "Free",
            "payload": [],
            "inventories": inventories
        })
    };

    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &bare(serde_json::json!(["Tpt"])),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a marketplace listing needs a file, and the deferred trigger would have \
         made this a 500"
    );
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::PayloadMissing));

    let (status, _) = json_call(
        state,
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &bare(serde_json::json!([])),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "and the same body naming no marketplace is a resource kept here (D32)"
    );
}

/// A marketplace named without a file is refused as its own situation.
///
/// Same code as the bare payload-less create above and deliberately a
/// different sentence: one is a draft nobody asked to publish, the other is a
/// seller who has chosen where this goes and not yet uploaded what goes there,
/// and only the second can be answered with "add your file first".
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_marketplace_named_without_a_file_is_refused_by_that_name(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("marketplace-no-file");
    let state = configured(pool, &root);
    let (status, body) = json_call(
        state,
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &serde_json::json!({
            "title": "Bound for TPT",
            "price": "Free",
            "payload": [],
            "inventories": ["Tpt"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::PayloadMissing));
    assert!(
        error.errors[0].message.contains("marketplace"),
        "the refusal names the situation the seller is in, not the invariant: {}",
        error.errors[0].message
    );
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
    let missing = error.errors[0]
        .detail
        .as_ref()
        .map_or(serde_json::Value::Null, |detail| {
            detail["missing"][0].clone()
        });
    assert_eq!(
        (
            missing["inventory"].as_str(),
            missing["field"].as_str(),
            missing["label"].as_str()
        ),
        (Some("TesGb"), Some("licence"), Some("Licence")),
        "the refusal names the marketplace, the wire field a client anchors to, and the \
         words the seller reads, so the sentence can be built without a second lookup"
    );
}

/// An already-answered election satisfies the licence on its own.
///
/// `required_fields_answered` accepts either a rights declaration or a licence
/// election, and every test until now sent both, so the second half of that
/// `||` was never exercised: an election path that stopped satisfying the
/// requirement would have failed no test while the create form kept sending a
/// grant beside it. The create form composes both, which is exactly why this
/// one sends only the election.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_licence_election_alone_satisfies_the_field_without_a_rights_grant(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("election-only");
    let state = configured(pool, &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("election"), "").await;
    let mut body = create_body(&uploaded, "Election only", &["TesGb"]);
    body["rights"] = serde_json::Value::Null;
    let (status, response) = json_call(state, &TOKEN_A, Method::POST, "/v1/products", &body).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the election answers the required licence with no grant beside it"
    );
    let created: CreatedProductView = parse(&response);
    assert_eq!(created.elections_recorded, 1);
    assert_eq!(created.mappings.len(), 1);
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

// -------------------------------------------------------------- file edits

/// Creates one product from one upload and answers with its id and the id of
/// the payload file it landed with, which every file test below starts from.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn with_one_file(
    state: AppState,
    token: &SessionToken,
    marker: &str,
    inventories: &[&str],
) -> (ProductId, String) {
    let uploaded = upload(state.clone(), token, pdf(marker), "").await;
    let (status, body) = json_call(
        state.clone(),
        token,
        Method::POST,
        "/v1/products",
        &create_body(&uploaded, "A resource with files", inventories),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the create lands: {}",
        String::from_utf8_lossy(&body)
    );
    let created: CreatedProductView = parse(&body);
    let path = format!("/v1/products/{}", created.product.0.to_hyphenated());
    let (_, body) = get(state, token, &path).await;
    let view: ProductView = parse(&body);
    let payload = view
        .files
        .iter()
        .find(|file| file.role == "payload")
        .expect("the create landed a payload file");
    (created.product, payload.id.to_hyphenated())
}

fn files_path(product: ProductId) -> String {
    format!("/v1/products/{}/files", product.0.to_hyphenated())
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_second_payload_file_is_added_and_reads_back_beside_the_first(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-add");
    let state = configured(pool.clone(), &root);
    let (product, _) = with_one_file(state.clone(), &TOKEN_A, "add", &["TesGb"]).await;

    let second = upload(state.clone(), &TOKEN_A, pdf("answer-key"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": second.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the add lands: {}",
        String::from_utf8_lossy(&body)
    );
    let added: AddedFileView = parse(&body);
    assert_eq!(
        (added.file.role.as_str(), added.file.kind.as_str()),
        ("payload", "pdf"),
        "the row is written in the role the body named"
    );
    assert_eq!(
        added.reaches,
        vec![InventoryId::TesGb],
        "the response names the marketplace this reaches on the next send rather than \
         implying it has already reached it"
    );

    let (status, body) = get(
        state.clone(),
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: ProductView = parse(&body);
    assert_eq!(
        view.files
            .iter()
            .filter(|file| file.role == "payload")
            .count(),
        2,
        "both payload files read back"
    );

    let (status, body) = get(state, &TOKEN_A, "/v1/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let jobs: serde_json::Value = parse(&body);
    assert_eq!(
        jobs["jobs"].as_array().map(Vec::len),
        Some(0),
        "a file change enqueues nothing; the marketplace's copy waits for the next send"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_client_named_cover_is_refused_on_every_write_that_could_place_one(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-cover-role");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "sellable", &["TesGb"]).await;
    let cover = cover_id(state.clone(), &TOKEN_A, product).await;
    let secret = upload(state.clone(), &TOKEN_A, pdf("the-paid-worksheet"), "").await;

    // The console reads a cover back as an image, so a row whose role is
    // `cover` is a row whose bytes a browser fetches. Naming that role against
    // a payload hash is what would make a sellable file browser-readable.
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "cover", "handle": secret.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a cover is drawn rather than uploaded: {}",
        String::from_utf8_lossy(&body)
    );

    // The same hole through the other door: replacing the cover row puts the
    // client's chosen hash behind the same role.
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{cover}", files_path(product)),
        &serde_json::json!({"handle": secret.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the cover row is not a target: {}",
        String::from_utf8_lossy(&body)
    );

    // The third door is shut by not existing: the redraw takes no handle at
    // all, so a `cover` field in the body reaches nothing. It is sent here
    // anyway, exactly as the old exploit did, to prove it is ignored rather
    // than honoured — a replacement that quietly accepted it would still pass
    // a test that only read the status code.
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{first}", files_path(product)),
        &serde_json::json!({"handle": secret.payload[0], "cover": secret.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the replacement itself is ordinary: {}",
        String::from_utf8_lossy(&body)
    );
    let replaced: ReplacedFileView = parse(&body);
    let drawn = replaced
        .cover
        .as_ref()
        .unwrap_or_else(|| panic!("replacing the first payload file redraws the thumbnail"));
    assert_eq!(
        drawn.kind, "image",
        "the thumbnail this server drew is an image, not the PDF the body named"
    );
    assert_ne!(
        drawn.byte_len, secret.payload[0].byte_len,
        "and it is not the payload's bytes wearing the thumbnail's role"
    );

    let after = files_of(state, &TOKEN_A, product).await;
    assert_ne!(
        cover_of(&after).map(|one| one.id.to_hyphenated()),
        Some(cover),
        "the thumbnail was redrawn, so it is not the one the create generated"
    );
    assert_eq!(
        cover_of(&after).map(|one| one.byte_len),
        Some(drawn.byte_len),
        "and the one standing is the one this server drew"
    );
    assert_eq!(
        after.iter().filter(|file| file.role == "cover").count(),
        1,
        "one thumbnail, as product_file_one_cover requires"
    );
    assert_eq!(
        after.iter().filter(|file| file.role == "payload").count(),
        1,
        "and the two refusals above wrote nothing"
    );
}

/// The refusal above, read as a seller reads it.
///
/// Separate from the test that proves the three doors are shut, because a
/// refusal that is correct and unreadable is still a defect: this one asserts
/// the seller is told what to do instead, and that a genuinely generated cover
/// handle is refused exactly as a payload hash is — the role is what is
/// refused, not the bytes behind it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_cover_refusal_is_a_sentence_and_holds_even_for_a_real_cover(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-cover");
    let state = configured(pool.clone(), &root);
    let (product, _) = with_one_file(state.clone(), &TOKEN_A, "cover", &["TesGb"]).await;

    let second = upload(state.clone(), &TOKEN_A, pdf("another"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "cover", "handle": second.cover}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a cover the pipeline really did generate is refused too: {}",
        String::from_utf8_lossy(&body)
    );
    let error: APIError = parse(&body);
    assert!(
        error.errors[0].message.contains("thumbnail")
            && error.errors[0].message.contains("first file"),
        "and the seller is told where a thumbnail comes from rather than only that this failed: {}",
        error.errors[0].message
    );

    let (_, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    let view: ProductView = parse(&body);
    assert_eq!(
        view.files
            .iter()
            .filter(|file| file.role == "cover")
            .count(),
        1,
        "the cover the create generated is still the only one"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_replace_swaps_the_bytes_keeps_the_role_and_retires_the_old_row(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-replace");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "v1", &["TesGb"]).await;

    let revised = upload(state.clone(), &TOKEN_A, pdf("v2-with-more-pages"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{first}", files_path(product)),
        &serde_json::json!({"handle": revised.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the replace lands: {}",
        String::from_utf8_lossy(&body)
    );
    let replaced: ReplacedFileView = parse(&body);
    assert_eq!(
        replaced.removed.to_hyphenated(),
        first,
        "the response names the row that stopped being this resource's"
    );
    assert_eq!(
        replaced.file.role, "payload",
        "a replacement keeps the role of the file it replaced"
    );
    assert_ne!(
        replaced.file.id.to_hyphenated(),
        first,
        "the replacement is a new row, so a client keying on the old id is told"
    );

    let (_, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    let view: ProductView = parse(&body);
    let payloads: Vec<_> = view
        .files
        .iter()
        .filter(|file| file.role == "payload")
        .collect();
    assert_eq!(payloads.len(), 1, "one payload file, not two");
    assert_eq!(
        payloads[0].byte_len, revised.payload[0].byte_len,
        "the bytes the seller uploaded are the ones the resource now names"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_takes_one_file_and_leaves_the_rest(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-remove");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "keep", &["TesGb"]).await;

    let second = upload(state.clone(), &TOKEN_A, pdf("spare"), "").await;
    let (status, _) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": second.payload[0]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{first}", files_path(product)),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the removal lands: {}",
        String::from_utf8_lossy(&body)
    );
    let removed: RemovedFileView = parse(&body);
    assert_eq!(removed.file.to_hyphenated(), first);

    let (_, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    let view: ProductView = parse(&body);
    assert_eq!(
        view.files
            .iter()
            .filter(|file| file.role == "payload")
            .count(),
        1,
        "the other payload file stands"
    );
    assert!(
        !view
            .files
            .iter()
            .any(|file| file.id.to_hyphenated() == first),
        "the removed file is gone from every read of the resource"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn removing_the_only_payload_file_is_refused_by_name_and_the_file_stands(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-last");
    let state = configured(pool.clone(), &root);
    let (product, only) = with_one_file(state.clone(), &TOKEN_A, "only", &["TesGb"]).await;

    let (status, body) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{only}", files_path(product)),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a resource keeps at least one file"
    );
    let error: APIError = parse(&body);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::PayloadMissing),
        "refused under the code a payload-less create is refused under, not a bare validation"
    );

    let (_, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    let view: ProductView = parse(&body);
    assert!(
        view.files
            .iter()
            .any(|file| file.id.to_hyphenated() == only),
        "the refusal left the file exactly where it was"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_live_tes_listing_refuses_every_file_change_with_the_capability_named(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-uncaptured");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "live", &["TesNz"]).await;
    let spare = upload(state.clone(), &TOKEN_A, pdf("spare"), "").await;
    bind_live(&pool, ORG_A, product, InventoryId::TesNz).await;

    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": spare.payload[0]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::UncapturedTransition),
        "a file change reaches a marketplace as a revise, so a listing that cannot be \
         revised cannot have its files changed through us"
    );

    let (status, _) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{first}", files_path(product)),
        &serde_json::json!({"handle": spare.payload[0]}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "so is a replace");

    let (status, _) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{first}", files_path(product)),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "and so is a removal"
    );

    let (_, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    let view: ProductView = parse(&body);
    assert_eq!(
        view.files
            .iter()
            .filter(|file| file.role == "payload")
            .count(),
        1,
        "nothing was written before the refusal"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_handle_this_tenant_never_uploaded_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-unheld");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "held", &["TesGb"]).await;

    let invented = serde_json::json!({
        "hash": "0".repeat(64),
        "kind": "pdf",
        "byte_len": 1024
    });
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": invented}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::UploadRejected));

    let (status, _) = json_call(
        state,
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{first}", files_path(product)),
        &serde_json::json!({"handle": invented}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the replace is held to the same check as the add"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_file_is_not_reachable_from_another(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    let root = store_root("file-tenancy");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "theirs", &["TesGb"]).await;
    let mine = upload(state.clone(), &TOKEN_B, pdf("mine"), "").await;

    let (status, _) = json_call(
        state.clone(),
        &TOKEN_B,
        Method::PUT,
        &format!("{}/{first}", files_path(product)),
        &serde_json::json!({"handle": mine.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another tenant's product is not there to be edited"
    );

    let (_, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    let view: ProductView = parse(&body);
    assert!(
        view.files
            .iter()
            .any(|file| file.id.to_hyphenated() == first),
        "the owner's file is untouched"
    );
}

/// The product's files as the console reads them, in the order it renders.
async fn files_of(state: AppState, token: &SessionToken, product: ProductId) -> Vec<FileView> {
    let (status, body) = get(
        state,
        token,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the resource reads back");
    let view: ProductView = parse(&body);
    view.files
}

/// The product's cover, or a failed test: every caller of this reads a
/// resource whose create generated one, so its absence is a broken fixture
/// rather than a case to handle.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn cover_id(state: AppState, token: &SessionToken, product: ProductId) -> String {
    let files = files_of(state, token, product).await;
    cover_of(&files)
        .expect("the create generated a cover")
        .id
        .to_hyphenated()
}

fn cover_of(files: &[FileView]) -> Option<&FileView> {
    files.iter().find(|file| file.role == "cover")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn replacing_the_first_payload_file_redraws_the_cover_it_was_drawn_from(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-cover-redraw");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "drawn", &["TesGb"]).await;
    let before = cover_id(state.clone(), &TOKEN_A, product).await;

    let revised = upload(state.clone(), &TOKEN_A, pdf("redrawn-from-this"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{first}", files_path(product)),
        &serde_json::json!({"handle": revised.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the replace lands: {}",
        String::from_utf8_lossy(&body)
    );
    let replaced: ReplacedFileView = parse(&body);
    let drawn = replaced
        .cover
        .as_ref()
        .unwrap_or_else(|| panic!("the response names the cover it redrew"));
    assert_eq!(
        drawn.role, "cover",
        "the redrawn row occupies the cover's role"
    );

    let after = files_of(state, &TOKEN_A, product).await;
    let live = cover_of(&after).unwrap_or_else(|| panic!("the resource still has a cover"));
    assert_eq!(
        live.id.to_hyphenated(),
        drawn.id.to_hyphenated(),
        "the cover the resource reads back is the one the response named"
    );
    assert_ne!(
        live.id.to_hyphenated(),
        before,
        "the cover drawn from the replaced file is not the one left standing"
    );
    assert_eq!(
        after.iter().filter(|file| file.role == "cover").count(),
        1,
        "one cover, which product_file_one_cover would have refused otherwise"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn replacing_a_later_payload_file_leaves_the_cover_alone(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-cover-kept");
    let state = configured(pool.clone(), &root);
    let (product, _) = with_one_file(state.clone(), &TOKEN_A, "kept-cover", &["TesGb"]).await;
    let before = cover_id(state.clone(), &TOKEN_A, product).await;

    let second = upload(state.clone(), &TOKEN_A, pdf("answer-key"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": second.payload[0]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let added: AddedFileView = parse(&body);

    // A replacement of the file the thumbnail was not drawn from. Nothing in
    // the body could ask for a redraw even if it wanted one; the server
    // decides, and here it decides not to.
    let third = upload(state.clone(), &TOKEN_A, pdf("answer-key-v2"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!("{}/{}", files_path(product), added.file.id.to_hyphenated()),
        &serde_json::json!({"handle": third.payload[0]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the replace lands: {}",
        String::from_utf8_lossy(&body)
    );
    let replaced: ReplacedFileView = parse(&body);
    assert!(
        replaced.cover.is_none(),
        "the response says no cover was redrawn, so the page does not claim one was"
    );

    let after = files_of(state, &TOKEN_A, product).await;
    let live = cover_of(&after).unwrap_or_else(|| panic!("the resource still has a cover"));
    assert_eq!(
        live.id.to_hyphenated(),
        before,
        "the cover drawn from the first payload file is untouched"
    );
}

/// The create body with each payload handle carrying the name a seller's file
/// picker would have given it.
fn named_create(uploaded: &UploadedView, title: &str, names: &[&str]) -> serde_json::Value {
    let mut body = create_body(uploaded, title, &["TesGb"]);
    let payload: Vec<serde_json::Value> = uploaded
        .payload
        .iter()
        .zip(names)
        .map(|(handle, name)| {
            serde_json::json!({
                "hash": handle.hash,
                "kind": handle.kind,
                "byte_len": handle.byte_len,
                "name": name,
            })
        })
        .collect();
    body["payload"] = serde_json::Value::Array(payload);
    body
}

fn name_of<'a>(files: &'a [FileView], id: &str) -> Option<&'a str> {
    files
        .iter()
        .find(|file| file.id.to_hyphenated() == id)
        .and_then(|file| file.name.as_deref())
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_s_own_filenames_survive_the_create_and_the_two_writes(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-names");
    let state = configured(pool.clone(), &root);

    let uploaded = upload(state.clone(), &TOKEN_A, pdf("named"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        "/v1/products",
        &named_create(&uploaded, "Named files", &["task-cards.pdf"]),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the create lands: {}",
        String::from_utf8_lossy(&body)
    );
    let created: CreatedProductView = parse(&body);
    let files = files_of(state.clone(), &TOKEN_A, created.product).await;
    let payload = files
        .iter()
        .find(|file| file.role == "payload")
        .unwrap_or_else(|| panic!("the create landed a payload file"));
    assert_eq!(
        payload.name.as_deref(),
        Some("task-cards.pdf"),
        "the name the seller's picker gave the file reaches the row"
    );
    assert_eq!(
        cover_of(&files).and_then(|cover| cover.name.as_deref()),
        None,
        "the generated cover carries no name, because nobody chose it"
    );

    // An add names its own file.
    let second = upload(state.clone(), &TOKEN_A, pdf("answers"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(created.product),
        &serde_json::json!({
            "role": "payload",
            "handle": {
                "hash": second.payload[0].hash,
                "kind": second.payload[0].kind,
                "byte_len": second.payload[0].byte_len,
                "name": "answer-key.pdf",
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let added: AddedFileView = parse(&body);
    assert_eq!(
        added.file.name.as_deref(),
        Some("answer-key.pdf"),
        "the response names the file it wrote"
    );

    // A replacement takes the new file's name, not the old one's.
    let third = upload(state.clone(), &TOKEN_A, pdf("answers-v2"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::PUT,
        &format!(
            "{}/{}",
            files_path(created.product),
            added.file.id.to_hyphenated()
        ),
        &serde_json::json!({
            "handle": {
                "hash": third.payload[0].hash,
                "kind": third.payload[0].kind,
                "byte_len": third.payload[0].byte_len,
                "name": "answer-key-corrected.pdf",
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let replaced: ReplacedFileView = parse(&body);
    assert_eq!(
        replaced.file.name.as_deref(),
        Some("answer-key-corrected.pdf")
    );

    let after = files_of(state, &TOKEN_A, created.product).await;
    assert_eq!(
        name_of(&after, &payload.id.to_hyphenated()),
        Some("task-cards.pdf"),
        "the file nobody touched keeps its name"
    );
    assert_eq!(
        name_of(&after, &replaced.file.id.to_hyphenated()),
        Some("answer-key-corrected.pdf"),
        "and the replacement reads back under the name the seller chose for it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_file_nobody_named_reads_back_without_a_name_rather_than_with_a_guess(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-unnamed");
    let state = configured(pool.clone(), &root);
    // The same body every client sent before names existed: handles with no
    // `name` at all, which is what every stored row looks like today.
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "unnamed", &["TesGb"]).await;

    let files = files_of(state, &TOKEN_A, product).await;
    assert_eq!(
        name_of(&files, &first),
        None,
        "an unnamed file stays unnamed rather than acquiring its kind as a name"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_name_longer_than_the_column_is_refused_before_the_insert(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-long-name");
    let state = configured(pool.clone(), &root);
    let (product, _) = with_one_file(state.clone(), &TOKEN_A, "long", &["TesGb"]).await;

    let second = upload(state.clone(), &TOKEN_A, pdf("verbose"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({
            "role": "payload",
            "handle": {
                "hash": second.payload[0].hash,
                "kind": second.payload[0].kind,
                "byte_len": second.payload[0].byte_len,
                "name": "x".repeat(256),
            }
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "refused as the seller's to fix rather than reaching the CHECK as a 500: {}",
        String::from_utf8_lossy(&body)
    );

    let files = files_of(state, &TOKEN_A, product).await;
    assert_eq!(
        files.iter().filter(|file| file.role == "payload").count(),
        1,
        "and nothing was written"
    );
}

/// The read a browser makes to draw a thumbnail, and the two fences on it.
///
/// The catalogue names the URL rather than the client composing it, so this
/// asserts the exact string a row is handed; and the bytes are the PNG the
/// ingest generated, read back through the same sealed store the upload wrote
/// them to, so a cover that only existed as a database row would fail here.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cover_is_named_on_the_catalogue_and_served_only_to_its_own_tenant(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    let root = store_root("cover-route");
    let state = configured(pool.clone(), &root);
    let (product, _payload) = with_one_file(state.clone(), &TOKEN_A, "covered", &["TesGb"]).await;

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/products").await;
    assert_eq!(status, StatusCode::OK, "the catalogue reads");
    let page: ProductsPage = parse(&body);
    let head = page
        .products
        .iter()
        .find(|entry| entry.id == product)
        .expect("the created resource is on the page");
    let path = format!("/v1/products/{}/cover", product.0.to_hyphenated());
    assert_eq!(
        head.cover.as_deref(),
        Some(path.as_str()),
        "a resource whose ingest generated a cover names where to fetch it"
    );

    let (status, bytes) = get(state.clone(), &TOKEN_A, &path).await;
    assert_eq!(status, StatusCode::OK, "the owner reads their own cover");
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "the bytes are the PNG the ingest encoded, not an error document"
    );

    let (status, _) = get(state, &TOKEN_B, &path).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another tenant's cover answers exactly as a resource that is not there"
    );
}

/// The read the create form makes before a product exists.
///
/// A cover is generated during the upload, so the form holds a handle and no
/// product; this proves the handle alone reaches the bytes inside the tenant
/// that sealed them, that it does not reach them from outside it, and that a
/// payload handle is refused rather than streamed through an image route.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_uploaded_image_reads_by_handle_inside_its_own_tenant_only(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    let root = store_root("upload-handle");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("preview"), "").await;

    let cover = format!("/v1/uploads/{}", uploaded.cover.hash);
    let (status, bytes) = get(state.clone(), &TOKEN_A, &cover).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the uploader reads the cover it just made"
    );
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "and the bytes are the PNG the render encoded"
    );

    let payload = format!("/v1/uploads/{}", uploaded.payload[0].hash);
    let (status, _) = get(state.clone(), &TOKEN_A, &payload).await;
    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "a payload handle is refused by name rather than streamed through an image route"
    );

    let (status, _) = get(state, &TOKEN_B, &cover).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another tenant's handle answers exactly as one that was never sealed"
    );
}

/// The 32 bytes a lowercase-hex handle names.
fn content_hash(hex: &str) -> ContentHash {
    let mut bytes = [0u8; 32];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let pair = hex.get(index * 2..index * 2 + 2).unwrap_or("00");
        *slot = u8::from_str_radix(pair, 16).unwrap_or(0);
    }
    ContentHash(bytes)
}

/// A cover row pointing at bytes that are not an image is refused, not served.
///
/// The row is written through the repository rather than through a write
/// route, deliberately: the write side may come to refuse a client-named cover
/// of the wrong kind, and this route has to refuse on its own either way, so a
/// test that reached the state through the write side would stop testing this
/// one the moment that refusal landed.
///
/// The bytes are a real sealed PDF from a real upload, so what is refused is a
/// blob this organisation genuinely holds — the fence being proved is the
/// route's, not the store's.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cover_naming_bytes_that_are_not_an_image_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("cover-not-an-image");
    let state = configured(pool.clone(), &root);
    let uploaded = upload(state.clone(), &TOKEN_A, pdf("not-an-image"), "").await;
    let hash = content_hash(&uploaded.payload[0].hash);

    let product = ProductId(Uuid([0x77; 16]));
    let held = |id: u8, role: FileRole| ProductFile {
        id: FileId(Uuid([id; 16])),
        role,
        kind: FileKind::Pdf,
        bytes: FileBytes::Held {
            hash,
            byte_len: uploaded.payload[0].byte_len,
            scan: ScanOutcome::Pending,
        },
    };
    ProductRepo::new(pool.clone())
        .insert(
            ORG_A,
            &tam_domain::CanonicalProduct {
                id: product,
                org: ORG_A,
                title: Title("A resource whose cover is a PDF".to_owned()),
                body: ListingCopy {
                    body: "Fixture body.".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: Some(PayloadSet::new(held(0x71, FileRole::Payload), vec![])),
                cover: Some(held(0x72, FileRole::Cover)),
                previews: vec![],
                subjects: vec![],
                grades: tam_domain::GradeDeclaration {
                    source: tam_domain::DeclarationSource::Seller,
                    raw: vec![],
                    derived: None,
                },
                price: PriceIntent::Free,
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            NOW,
        )
        .await
        .expect("the fixture product inserts");

    let path = format!("/v1/products/{}/cover", product.0.to_hyphenated());
    let (status, _) = get(state, &TOKEN_A, &path).await;
    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "a seller's PDF is not streamed through the route a browser draws images from"
    );
}

/// A deleted resource has no cover, on the catalogue or on the route.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_deleted_resource_stops_naming_and_stops_serving_its_cover(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("cover-after-delete");
    let state = configured(pool.clone(), &root);
    let (product, _payload) = with_one_file(state.clone(), &TOKEN_A, "doomed", &["TesGb"]).await;
    let path = format!("/v1/products/{}/cover", product.0.to_hyphenated());

    let (status, _) = get(state.clone(), &TOKEN_A, &path).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the cover serves while the resource stands"
    );

    let (status, _) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::DELETE,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
        &serde_json::json!({"leave_live": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the resource deletes");

    let (status, body) = get(state.clone(), &TOKEN_A, "/v1/products").await;
    assert_eq!(status, StatusCode::OK);
    let page: ProductsPage = parse(&body);
    assert!(
        page.products.is_empty(),
        "a deleted resource leaves the catalogue"
    );

    let (status, _) = get(state, &TOKEN_A, &path).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "and its cover goes with it, rather than outliving the row that named it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn removing_the_file_the_thumbnail_was_drawn_from_redraws_it_from_the_next(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-remove-redraw");
    let state = configured(pool.clone(), &root);
    let (product, first) = with_one_file(state.clone(), &TOKEN_A, "drawn-from", &["TesGb"]).await;
    let before = cover_id(state.clone(), &TOKEN_A, product).await;

    // A second payload file, so the removal is permitted at all and there is
    // something for the thumbnail to be redrawn from.
    let second = upload(state.clone(), &TOKEN_A, pdf("answer-key"), "").await;
    let (status, _) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": second.payload[0]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{first}", files_path(product)),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the removal lands: {}",
        String::from_utf8_lossy(&body)
    );
    let removed: RemovedFileView = parse(&body);
    let ThumbnailView::Redrawn { file: drawn } = &removed.thumbnail else {
        panic!(
            "the response names the thumbnail it redrew, not {:?}",
            removed.thumbnail
        )
    };
    assert_eq!(
        drawn.role, "cover",
        "the redrawn row occupies the same role"
    );

    let after = files_of(state, &TOKEN_A, product).await;
    assert_ne!(
        cover_of(&after).map(|one| one.id.to_hyphenated()),
        Some(before),
        "the thumbnail drawn from the removed file is not the one left standing"
    );
    assert_eq!(
        cover_of(&after).map(|one| one.id.to_hyphenated()),
        Some(drawn.id.to_hyphenated()),
        "the one standing is the one this server drew"
    );
    assert_eq!(
        after.iter().filter(|file| file.role == "cover").count(),
        1,
        "one thumbnail, as product_file_one_cover requires"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn removing_a_later_payload_file_leaves_the_thumbnail_alone(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-remove-keeps");
    let state = configured(pool.clone(), &root);
    let (product, _) = with_one_file(state.clone(), &TOKEN_A, "keeps", &["TesGb"]).await;
    let before = cover_id(state.clone(), &TOKEN_A, product).await;

    let second = upload(state.clone(), &TOKEN_A, pdf("spare"), "").await;
    let (status, body) = json_call(
        state.clone(),
        &TOKEN_A,
        Method::POST,
        &files_path(product),
        &serde_json::json!({"role": "payload", "handle": second.payload[0]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let added: AddedFileView = parse(&body);

    let (status, body) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{}", files_path(product), added.file.id.to_hyphenated()),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let removed: RemovedFileView = parse(&body);
    assert!(
        matches!(removed.thumbnail, ThumbnailView::Untouched),
        "the response says the thumbnail was left alone, so the page does not claim otherwise"
    );

    let after = files_of(state, &TOKEN_A, product).await;
    assert_eq!(
        cover_of(&after).map(|one| one.id.to_hyphenated()),
        Some(before),
        "the thumbnail drawn from the first payload file is untouched"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unmapped_resource_may_lose_its_last_file_and_its_thumbnail_with_it(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-unmapped-last");
    let state = configured(pool.clone(), &root);
    // No inventories: a draft kept here and carried by no marketplace, which is
    // the case migration 0061 relaxed the payload requirement for.
    let (product, only) = with_one_file(state.clone(), &TOKEN_A, "draft-only", &[]).await;

    let (status, body) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{only}", files_path(product)),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a resource no marketplace carries may have no file: {}",
        String::from_utf8_lossy(&body)
    );
    let removed: RemovedFileView = parse(&body);
    assert!(
        matches!(removed.thumbnail, ThumbnailView::Retired),
        "the thumbnail goes with the file it was drawn from, because nothing is left to \
         draw a new one from: {:?}",
        removed.thumbnail
    );

    // The resource is still there and still readable, which is the half a
    // relaxed invariant most easily breaks.
    let (status, body) = get(
        state,
        &TOKEN_A,
        &format!("/v1/products/{}", product.0.to_hyphenated()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the resource still reads back");
    let view: ProductView = parse(&body);
    assert!(
        view.files.is_empty(),
        "with no files at all, which is the state D32 admits"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_listed_resource_still_keeps_its_last_file(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("file-mapped-last");
    let state = configured(pool.clone(), &root);
    let (product, only) = with_one_file(state.clone(), &TOKEN_A, "listed", &["TesGb"]).await;

    let (status, body) = call(
        state.clone(),
        &TOKEN_A,
        Call {
            method: Method::DELETE,
            path: &format!("{}/{only}", files_path(product)),
            body: None,
            content_type: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a resource a marketplace carries keeps at least one file"
    );
    let error: APIError = parse(&body);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::PayloadMissing),
        "refused by name, under the code the same invariant uses at create"
    );

    let after = files_of(state, &TOKEN_A, product).await;
    assert!(
        after.iter().any(|file| file.id.to_hyphenated() == only),
        "and the refusal left the file where it was"
    );
}
