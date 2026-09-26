//! The thumbnail of an already-imported resource, repaired by a later read.
//!
//! Two facts make this the only repair path there is. The server never holds
//! a no-API marketplace's original bytes, so nothing here can derive a
//! thumbnail from a payload; and nothing after a commit redraws a cover, so a
//! resource imported before one could be derived keeps none forever. The
//! device is therefore the only place a picture can come from, and the
//! enumeration skip is what used to stop it ever being asked again.
//!
//! Asserted end to end against the bytes the console would serve, because
//! every link in the chain is a place a thumbnail has previously been
//! dropped: the enumeration skip, the commit's bind-onto-the-survivor branch,
//! and the cover read the catalogue answers from.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::import_runs::{ImportRunItemState, ImportRunView, RunConfirmAck};
use tam_api::resources::ProductsPage;
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_engine_driver::import::{
    ContentType, Cover, FileName, ImportPage, ListedResource, Locator, ObservedFile,
    ObservedResource,
};
use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
use tam_storage::{DeviceRegistration, DeviceRepo, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, FileKind, ImportedPrice, OrgId, ProductId, ScanOutcome, Timestamp,
    UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xA7; 16]));
const USER_A: UserId = UserId(Uuid([0x07; 16]));
const TOKEN_A: SessionToken = SessionToken([0x57; 32]);
const NOW: Timestamp = Timestamp(5_000);
const DEVICE_A: &str = "11112222333344445555666677778888";
const CURRENT: &str = "0.9.0";
const ATTEMPT: u64 = 1;
const BIG: u64 = 120_000;
const LOCATOR: &str = "https://www.tes.com/teaching-resource/-1";

// ------------------------------------------------------------------ harness

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cover-{name}-{}", std::process::id()));
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
            tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
            root.to_path_buf(),
        )),
    }
}

/// The seller-device consent, granted for every marketplace that needs it,
/// so the mints under test are answered by the machinery rather than by the
/// consent gate; `consent_flow.rs` is where that gate is exercised.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn consented(pool: &PgPool, org: OrgId) {
    let consents = tam_storage::ConsentRepo::new(pool.clone());
    for marketplace in tam_types::Marketplace::ALL {
        if marketplace.transport_class() == tam_types::TransportClass::SellerDevice {
            consents
                .grant(
                    org,
                    marketplace,
                    tam_types::CONSENT_NOTICE_VERSION,
                    Uuid([0xC0; 16]),
                    Timestamp(1_000),
                )
                .await
                .expect("the fixture consent grants");
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .bind("a test org")
        .execute(pool)
        .await
        .expect("the org seeds");
    consented(pool, ORG_A).await;
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            ORG_A,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Subscriber,
                rung: None,
                granted_by: tam_storage::GrantedBy::Stripe,
                grantor_user: None,
                reason: None,
                source_ref: Some("sub_a7"),
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("the fixture grant seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG_A, USER_A, "a7@example.test", NOW)
        .await
        .expect("the user provisions");
    sessions
        .mint(&TOKEN_A, USER_A, Timestamp(100_000), NOW)
        .await
        .expect("the session mints");
    DeviceRepo::new(pool.clone())
        .register(
            ORG_A,
            &DeviceRegistration {
                id: DEVICE_A,
                name: "a test machine",
                os: "linux",
                arch: "x86_64",
                app_version: CURRENT,
            },
            NOW,
        )
        .await
        .expect("the device registers");
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', $3, $3)",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0xC1; 16]))
    .bind(sqlx::types::chrono::DateTime::from_timestamp_millis(NOW.0).expect("a valid instant"))
    .execute(&mut *tx)
    .await
    .expect("the connection seeds");
    tx.commit().await.expect("the fixture commits");
}

struct Answer {
    status: StatusCode,
    body: serde_json::Value,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_value(self.body.clone()).expect("the answer decodes")
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn raw(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", TOKEN_A.to_hex()),
        )
        .header(header::CONTENT_TYPE, "application/json");
    if body.is_none() {
        request = request.header(header::CONTENT_LENGTH, "0");
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(body.map_or_else(Body::empty, |body| {
                    Body::from(serde_json::to_vec(&body).expect("a body serialises"))
                }))
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    (status, content_type, bytes.to_vec())
}

async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> Answer {
    let (status, _, bytes) = raw(app, method, uri, body).await;
    Answer {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    }
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

fn uuid_text(id: Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}

async fn started_run(app: &axum::Router) -> Uuid {
    let answer = call(
        app,
        Method::POST,
        "/v1/imports/runs",
        Some(serde_json::json!({
            "source": "Tes",
            "start_key": uuid_text(fresh_uuid()),
        })),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "the run opens: {}",
        answer.body
    );
    let view: ImportRunView = answer.json();
    let claim = call(
        app,
        Method::POST,
        &format!("/v1/devices/{DEVICE_A}/import/{}/claim", uuid_text(view.id)),
        Some(serde_json::json!({ "takeover": false })),
    )
    .await;
    assert_eq!(claim.status, StatusCode::OK, "the claim: {}", claim.body);
    assert_eq!(
        claim.body["attempt"].as_u64(),
        Some(ATTEMPT),
        "the first claim grants the first fence"
    );
    view.id
}

async fn post_page(app: &axum::Router, page: &ImportPage) -> Answer {
    call(
        app,
        Method::POST,
        &format!("/v1/devices/{DEVICE_A}/import"),
        Some(serde_json::to_value(page).unwrap_or(serde_json::Value::Null)),
    )
    .await
}

async fn run_view(app: &axum::Router, run: Uuid) -> ImportRunView {
    call(
        app,
        Method::GET,
        &format!("/v1/imports/runs/{}", uuid_text(run)),
        None,
    )
    .await
    .json()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn confirm_and_drain(app: &axum::Router, state: &AppState, run: Uuid) -> ImportRunView {
    let accepted = call(
        app,
        Method::POST,
        &format!("/v1/imports/runs/{}/commit", uuid_text(run)),
        None,
    )
    .await;
    assert_eq!(
        accepted.status,
        StatusCode::ACCEPTED,
        "the confirmation is accepted: {}",
        accepted.body
    );
    let ack: RunConfirmAck = accepted.json();
    assert!(ack.accepted, "the confirmation is recorded");
    tam_api::scheduler::pass(state, NOW)
        .await
        .expect("the pass runs");
    run_view(app, run).await
}

async fn select_all(app: &axum::Router, run: Uuid) {
    let selected = call(
        app,
        Method::POST,
        &format!("/v1/imports/runs/{}/select", uuid_text(run)),
        Some(serde_json::json!({ "all": true })),
    )
    .await;
    assert_eq!(
        selected.status,
        StatusCode::OK,
        "the selection lands: {}",
        selected.body
    );
}

// ----------------------------------------------------------------- fixtures

fn listing_page(run: Uuid) -> ImportPage {
    ImportPage {
        run,
        request: None,
        attempt: Some(ATTEMPT),
        receipt: Some(fresh_uuid()),
        enumeration_complete: true,
        listed: Some(vec![ListedResource {
            locator: Locator::new(LOCATOR).unwrap_or_else(|_| Locator::from_resource_id(1)),
            title: "Fractions pack".to_owned(),
            price_minor: None,
            currency: None,
            state: Some(ListingState::Live),
        }]),
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    }
}

/// The thumbnail a device derived from the resource's own bytes, as distinct
/// bytes per read so that which one the catalogue serves is decidable.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn drawn_cover(marker: u8) -> (Cover, Vec<u8>) {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(b"IHDR a preview from inside the resource ");
    png.push(marker);
    (
        Cover::encode(&png).expect("the fixture is a PNG within the ceiling"),
        png,
    )
}

/// Exact bytes, as the wire carries them: used to post the renderer's own
/// generated card, which is what every Tes bundle imported so far holds.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn as_cover(png: &[u8]) -> Cover {
    Cover::encode(png).expect("the card is a PNG within the ceiling")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn described(run: Uuid, cover: Option<Cover>) -> ImportPage {
    ImportPage {
        run,
        request: None,
        attempt: Some(ATTEMPT),
        receipt: Some(fresh_uuid()),
        enumeration_complete: false,
        listed: None,
        resources: vec![ObservedResource {
            locator: Locator::new(LOCATOR).expect("a bounded locator"),
            listing: ImportedListing {
                remote: RemoteListingId::Tes {
                    url: LOCATOR.to_owned(),
                },
                title: "Fractions pack".to_owned(),
                body: "A worksheet.".to_owned(),
                body_format: CopyFormat::Markdown,
                native: Vec::new(),
                rights: None,
                price: ImportedPrice::Free,
                state: Some(ListingState::Live),
            },
            fingerprint: None,
            file: Some(ObservedFile {
                payload_file_name: FileName::new("worksheet-pack.zip").expect("a plain name"),
                payload_content_type: ContentType::new("application/zip").expect("a media type"),
                kind: FileKind::Zip,
                hash: ContentHash([0x5A; 32]),
                byte_len: BIG,
                scan: ScanOutcome::Clean { at: NOW },
                entry: None,
            }),
            cover_png: cover,
        }],
        skipped: Vec::new(),
        complete: true,
        failed: None,
    }
}

/// The resources the seller's catalogue actually lists, read through the
/// route the console reads.
///
/// Counted from the answer rather than from `product` directly: a row count
/// taken outside a request has no tenant pinned, so it measures what row-level
/// security hides rather than what the catalogue holds, and it would pass or
/// fail for reasons that have nothing to do with the import.
async fn catalogue(app: &axum::Router) -> Vec<ProductId> {
    let answer = call(app, Method::GET, "/v1/products", None).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the catalogue lists: {}",
        answer.body
    );
    let page: ProductsPage = answer.json();
    page.products.iter().map(|head| head.id).collect()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn imported_product(view: &ImportRunView) -> ProductId {
    view.items
        .iter()
        .find_map(|item| item.product_id)
        .expect("the run names the product it created")
}

// -------------------------------------------------------------------- tests

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_resource_imported_without_a_thumbnail_gains_one_on_the_next_read(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("repair"));
    let app = router(state.clone());

    // ---- the historical import: a read that carried no cover, which is every
    // resource imported before one could be derived.
    let first = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(first)).await.status,
        StatusCode::OK
    );
    select_all(&app, first).await;
    assert_eq!(
        post_page(&app, &described(first, None)).await.status,
        StatusCode::OK
    );
    let done = confirm_and_drain(&app, &state, first).await;
    let product = imported_product(&done);
    assert_eq!(
        catalogue(&app).await,
        vec![product],
        "one resource in the catalogue, and it is the one the run created"
    );
    let (missing, _, _) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(
        missing,
        StatusCode::NOT_FOUND,
        "it has no thumbnail, which is the state a seller reports as a blank card"
    );

    // ---- the repair: the seller reads the shop again. The listing is one the
    // catalogue already holds, and before this repair that was the end of it.
    let second = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(second)).await.status,
        StatusCode::OK
    );
    let listed = run_view(&app, second).await;
    assert_eq!(
        (listed.counts.listed, listed.counts.skipped),
        (1, 0),
        "a resource with no thumbnail is offered again rather than skipped, because reading \
         its file is the only way one can ever be derived"
    );

    select_all(&app, second).await;
    let (cover, bytes) = drawn_cover(0xA1);
    assert_eq!(
        post_page(&app, &described(second, Some(cover)))
            .await
            .status,
        StatusCode::OK
    );
    let repaired = confirm_and_drain(&app, &state, second).await;
    let row = repaired
        .items
        .iter()
        .find(|item| item.locator == LOCATOR)
        .unwrap_or_else(|| panic!("the re-read row is on the run"));
    assert_eq!(
        row.state,
        ImportRunItemState::Skipped,
        "the resource is not imported a second time"
    );
    assert_eq!(
        row.skip_reason.as_deref(),
        Some("same as Fractions pack; this import added its thumbnail"),
        "and the seller is told the one thing that changed"
    );
    assert_eq!(
        catalogue(&app).await,
        vec![product],
        "no second copy of the resource: the catalogue still lists exactly the one the first \
         run created"
    );

    let (status, content_type, served) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the catalogue now serves a picture");
    assert_eq!(
        content_type.as_deref(),
        Some("image/png"),
        "sniffed from the bytes, as the cover route always does"
    );
    assert_eq!(
        served, bytes,
        "the bytes served are the picture the device derived from the resource, byte for byte, \
         rather than anything this server generated"
    );

    // ---- and it is self-terminating: the next read of the same shop skips
    // the resource again, and a second offer cannot overwrite the picture.
    let third = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(third)).await.status,
        StatusCode::OK
    );
    let after = run_view(&app, third).await;
    assert_eq!(
        (after.counts.listed, after.counts.skipped),
        (0, 1),
        "a resource that has its thumbnail is skipped as it lands, as it always was"
    );
    assert_eq!(
        after
            .items
            .iter()
            .find(|item| item.locator == LOCATOR)
            .and_then(|item| item.skip_reason.as_deref()),
        Some("already in Resources as Fractions pack"),
        "with the sentence the saving has always carried"
    );
    let (_, _, unchanged) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(
        unchanged, bytes,
        "and the thumbnail it holds is still the one it was given"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_read_that_found_no_picture_repairs_nothing_and_says_nothing(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("nocard"));
    let app = router(state.clone());

    // An import with no thumbnail at all.
    let first = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(first)).await.status,
        StatusCode::OK
    );
    select_all(&app, first).await;
    assert_eq!(
        post_page(&app, &described(first, None)).await.status,
        StatusCode::OK
    );
    let product = imported_product(&confirm_and_drain(&app, &state, first).await);

    // The re-read finds no picture inside the resource either — a PDF, or a
    // bundle of documents that store no thumbnail — so what it carries is the
    // generated card.
    let card = tam_pipeline::render::cover(FileKind::Pdf, b"%PDF-1.7 one page nobody rendered")
        .unwrap_or_else(|error| panic!("the card renders: {error}"))
        .image
        .png;
    let second = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(second)).await.status,
        StatusCode::OK
    );
    select_all(&app, second).await;
    assert_eq!(
        post_page(&app, &described(second, Some(as_cover(&card))))
            .await
            .status,
        StatusCode::OK
    );
    let after = confirm_and_drain(&app, &state, second).await;
    assert_eq!(
        after
            .items
            .iter()
            .find(|item| item.locator == LOCATOR)
            .and_then(|item| item.skip_reason.as_deref()),
        Some("same as Fractions pack"),
        "a card is not a thumbnail, so the seller is not told one arrived"
    );
    let (status, _, _) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "and the resource is left with no thumbnail rather than given a picture of nothing: \
         the repair stays owed until a read finds a real preview"
    );
    assert_eq!(
        catalogue(&app).await,
        vec![product],
        "and the resource itself is untouched: one copy, the one the first run created"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_generated_card_gives_way_to_a_picture_and_a_picture_never_does(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("card"));
    let app = router(state.clone());

    // The state the live catalogue is actually in: a Tes bundle imported with
    // the generated zip card as its thumbnail, which is a picture of nothing.
    let card = tam_pipeline::render::cover(FileKind::Zip, b"not an archive at all")
        .unwrap_or_else(|error| panic!("the card renders: {error}"))
        .image
        .png;
    let first = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(first)).await.status,
        StatusCode::OK
    );
    select_all(&app, first).await;
    assert_eq!(
        post_page(&app, &described(first, Some(as_cover(&card))))
            .await
            .status,
        StatusCode::OK
    );
    let done = confirm_and_drain(&app, &state, first).await;
    let product = imported_product(&done);
    let (status, _, stored) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stored, card,
        "the resource starts with the card, which is what a seller reads as a blank tile"
    );

    // ---- the refresh: a card is not a picture, so the listing is offered
    // again and the read that finds a real preview replaces it.
    let second = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(second)).await.status,
        StatusCode::OK
    );
    let offered = run_view(&app, second).await;
    assert_eq!(
        (offered.counts.listed, offered.counts.skipped),
        (1, 0),
        "a resource whose thumbnail is the generated card is offered again: the card is \
         exactly the evidence that no picture was ever found in its bytes"
    );
    select_all(&app, second).await;
    let (preview, preview_bytes) = drawn_cover(0xB2);
    assert_eq!(
        post_page(&app, &described(second, Some(preview)))
            .await
            .status,
        StatusCode::OK
    );
    let repaired = confirm_and_drain(&app, &state, second).await;
    assert_eq!(
        repaired
            .items
            .iter()
            .find(|item| item.locator == LOCATOR)
            .and_then(|item| item.skip_reason.as_deref()),
        Some("same as Fractions pack; this import added its thumbnail"),
        "the resource is bound onto rather than created twice, and the repair is said out loud"
    );
    assert_eq!(
        catalogue(&app).await,
        vec![product],
        "no second copy of the resource"
    );
    let (_, _, served) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(
        served, preview_bytes,
        "the catalogue now serves the picture the device found inside the resource"
    );

    // ---- and now that it holds a picture, nothing offers to change it: the
    // listing is skipped as it lands and the run never reads its file.
    let third = started_run(&app).await;
    assert_eq!(
        post_page(&app, &listing_page(third)).await.status,
        StatusCode::OK
    );
    let after = run_view(&app, third).await;
    assert_eq!(
        (after.counts.listed, after.counts.skipped),
        (0, 1),
        "a real picture ends the refresh: a thumbnail the seller can see is never re-derived"
    );
    let (_, _, unchanged) = raw(
        &app,
        Method::GET,
        &format!("/v1/products/{}/cover", uuid_text(product.0)),
        None,
    )
    .await;
    assert_eq!(
        unchanged, preview_bytes,
        "and the picture it holds is untouched"
    );
}
