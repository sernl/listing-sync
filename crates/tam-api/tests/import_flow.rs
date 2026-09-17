//! The device-import route end to end: a page of what a seller's own device
//! observed, applied.
//!
//! The property every one of these exists to hold is D27's: the seller's file
//! bytes stay on the seller's machine, so a page describes a file and the
//! server stores nothing of it but the derived cover. That is asserted against
//! real blob rows under a real store root rather than against a fake sink,
//! because a fake would prove the handler's shape and none of the property.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::import::ImportAck;
use tam_api::jobs::{JobPage, SyncRequestListView, SyncRequestView};
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_engine_driver::import::{
    base64, ContentType, Cover, FileName, ImportPage, Locator, ObservedFile, ObservedResource,
    Reason, SkippedResource,
};
use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
use tam_storage::{DeviceRegistration, DeviceRepo, SessionRepo, SessionToken, TaxonomyRepo};
use tam_types::{
    CanonicalTermId, ContentHash, CopyFormat, FileKind, ImportedPrice, InventoryId, OrgId,
    ScanOutcome, Timestamp, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);
const DEVICE_A: &str = "11112222333344445555666677778888";
const DEVICE_B: &str = "99998888777766665555444433332222";
const REVOKED: &str = "aaaabbbbccccddddeeeeffff00001111";
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const TOPIC: CanonicalTermId = CanonicalTermId(Uuid([0x78; 16]));
/// At or past `tam_domain::SOURCED_PAYLOAD_MIN_VERSION`, so the claim gate
/// would hand this device a sourced item.
const CURRENT: &str = "0.9.0";
/// Below it, which is what makes the waiting reason appear.
const STALE: &str = "0.1.3";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("import-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("the store root is creatable");
    root
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn configured(pool: PgPool, root: &std::path::Path) -> AppState {
    AppState {
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

/// One tenant, its session, its device, its linked Tes connection and the
/// crosswalk `import_one` projects over.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool, org: OrgId, user: UserId, token: &SessionToken, device: &str) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(format!("org-{}", org.0 .0[0]))
        .execute(pool)
        .await
        .expect("the org seeds");
    consented(pool, org).await;
    // Reading a shop is a paid capability, so the fixture tenant subscribes:
    // without a grant every page in this file would be answered by the plan
    // gate rather than by the import machinery it is written to exercise.
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            org,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Subscriber,
                rung: None,
                granted_by: tam_storage::GrantedBy::Paddle,
                grantor_user: None,
                reason: None,
                source_ref: Some(&format!("sub_{}", org.0 .0[0])),
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("the fixture grant seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(org, user, &format!("{}@example.test", org.0 .0[0]), NOW)
        .await
        .expect("the user provisions");
    sessions
        .mint(token, user, Timestamp(100_000), NOW)
        .await
        .expect("the session mints");
    DeviceRepo::new(pool.clone())
        .register(
            org,
            &DeviceRegistration {
                id: device,
                name: "a test machine",
                os: "linux",
                arch: "x86_64",
                app_version: CURRENT,
            },
            NOW,
        )
        .await
        .expect("the device registers");
    // The connection a sourced file names. Written under a tenant pin because
    // `connection` carries forced row-level security.
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', $3, $3)",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes([org.0 .0[0]; 16]))
    .bind(sqlx::types::chrono::DateTime::from_timestamp_millis(NOW.0).expect("a valid instant"))
    .execute(&mut *tx)
    .await
    .expect("the connection seeds");
    tx.commit().await.expect("the fixture commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_crosswalk(pool: &PgPool) {
    let taxonomy = TaxonomyRepo::new(pool.clone());
    let terms = [
        CanonicalTerm {
            id: SUBJECT,
            kind: TermKind::Subject,
            parent: None,
            label: "Maths for early years".to_owned(),
        },
        CanonicalTerm {
            id: TOPIC,
            kind: TermKind::Topic,
            parent: Some(SUBJECT),
            label: "Time".to_owned(),
        },
    ];
    let edges = vec![
        edge(SUBJECT, InventoryId::Tes, TermKind::Subject, "1000454"),
        edge(TOPIC, InventoryId::Tes, TermKind::Topic, "1000732"),
    ];
    taxonomy
        .seed(&terms, &edges)
        .await
        .expect("the crosswalk seeds");
}

fn edge(
    from: CanonicalTermId,
    inventory: InventoryId,
    kind: TermKind,
    native: &str,
) -> ProjectionEdge {
    ProjectionEdge {
        from,
        to: VocabularyPath {
            vocabulary: VocabularyId(inventory, kind),
            segments: vec!["Maths for early years".to_owned()],
            native_id: Some(native.to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: NOW,
    }
}

/// A PNG that passes the cover check, and is nothing like a payload.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn cover() -> Cover {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(b"IHDR a derived thumbnail");
    Cover::encode(&png).expect("the fixture is a PNG within the ceiling")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn observed(resource: i64, digest: u8) -> ObservedResource {
    ObservedResource {
        locator: Locator::from_resource_id(resource),
        listing: ImportedListing {
            remote: RemoteListingId::Tes {
                url: format!("https://www.tes.com/teaching-resource/-{resource}"),
            },
            title: format!("Fractions practice {resource}"),
            body: "A worksheet.".to_owned(),
            body_format: CopyFormat::Markdown,
            native: vec![
                tam_types::ImportedTerm {
                    inventory: InventoryId::Tes,
                    kind: Some(TermKind::Subject),
                    segments: vec!["1000454".to_owned()],
                    native_id: Some("1000454".to_owned()),
                },
                tam_types::ImportedTerm {
                    inventory: InventoryId::Tes,
                    kind: Some(TermKind::Subject),
                    segments: vec!["1000732".to_owned()],
                    native_id: Some("1000732".to_owned()),
                },
            ],
            rights: None,
            price: ImportedPrice::Free,
            state: Some(ListingState::Live),
        },
        fingerprint: None,
        file: Some(ObservedFile {
            payload_file_name: FileName::new("worksheet.pdf").expect("a plain name"),
            payload_content_type: ContentType::new("application/pdf").expect("a media type"),
            kind: FileKind::Pdf,
            hash: ContentHash([digest; 32]),
            byte_len: 4_096,
            scan: ScanOutcome::Clean { at: NOW },
            entry: None,
        }),
        cover_png: Some(cover()),
    }
}

/// The migrate a device enumerates: `resources` empty, because the seller's
/// own machine is the only thing that can walk that catalogue.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn open_request(app: &axum::Router, token: &SessionToken, key: Uuid) -> Uuid {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/sync")
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", token.to_hex()),
                )
                .header("idempotency-key", uuid::Uuid::from_bytes(key.0).to_string())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "source": "Tes",
                        "target": "Tpt",
                        "disposition": "migrate",
                        "intent": "draft",
                        "resources": [],
                    })
                    .to_string(),
                ))
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert!(
        response.status().is_success(),
        "a migrate from a device-branch source opens with no resources named"
    );
    key
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn post_page(
    app: &axum::Router,
    token: &SessionToken,
    device: &str,
    page: &ImportPage,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/devices/{device}/import"))
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", token.to_hex()),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(page).expect("a page serialises"),
                ))
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body reads")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn view(app: &axum::Router, token: &SessionToken, request: Uuid) -> SyncRequestView {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!(
                    "/v1/sync/{}",
                    uuid::Uuid::from_bytes(request.0).as_hyphenated()
                ))
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", token.to_hex()),
                )
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body reads")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("the view decodes")
}

/// One count, read under a tenant pin.
///
/// `blob` and `product_file` carry forced row-level security, so an unpinned
/// count returns zero whatever the table holds — and a test asserting "no
/// bytes were stored" against an unpinned read passes because it can see
/// nothing, not because nothing is there.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn pinned_count(pool: &PgPool, org: OrgId, sql: &str) -> i64 {
    let mut tx = pool.begin().await.expect("the count opens a transaction");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query_scalar(sql)
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("the count reads")
}

/// D9(2). A page of two resources becomes two breadcrumbs and two products,
/// and the only bytes we keep are the covers.
///
/// D27 on the server side. The seller's payloads are named, never stored, so
/// after this the blob table holds exactly one row per cover and nothing else —
/// and the positive control is the same pinned read finding those covers,
/// because a count that can see nothing passes this by being blind rather than
/// by being right.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_page_applies_its_resources_and_keeps_only_their_covers(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("kept");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x51; 16])).await;

    let page = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A), observed(13_549_795, 0x5B)],
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(status, StatusCode::OK, "the page applies: {body}");
    let ack: ImportAck = serde_json::from_value(body).expect("the ack decodes");
    assert_eq!((ack.applied, ack.skipped), (2, 0));

    let breadcrumbs = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM sync_request_resource WHERE org_id = $1",
    )
    .await;
    assert_eq!(breadcrumbs, 2, "one breadcrumb per described resource");
    let products = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM product WHERE org_id = $1",
    )
    .await;
    assert_eq!(products, 2, "and one product per breadcrumb");

    let sourced = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM product_file WHERE org_id = $1 AND hash IS NULL",
    )
    .await;
    assert_eq!(sourced, 2, "each payload is named rather than held");
    let held = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM product_file WHERE org_id = $1 AND hash IS NOT NULL",
    )
    .await;
    assert_eq!(held, 2, "and each cover is held: the positive control");

    let blobs = pinned_count(&pool, ORG_A, "SELECT count(*) FROM blob WHERE org_id = $1").await;
    assert_eq!(
        blobs, 1,
        "both covers are the same bytes, so content addressing stores one blob — and that one \
         blob is the whole of what this import kept. Anything beyond it is the seller's own \
         file on our servers, which is the one thing D27 forbids"
    );
    let payload_blobs = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM blob b JOIN product_file f \
         ON f.org_id = b.org_id AND f.observed_hash = b.hash \
         WHERE f.org_id = $1 AND f.hash IS NULL",
    )
    .await;
    assert_eq!(
        payload_blobs, 0,
        "and no blob carries a digest a device reported for a payload"
    );
}

/// D9(3). The same page twice is one product and one breadcrumb per resource.
///
/// `import_one` mints a fresh product every run, so the check that makes this
/// true has to happen before anything is canonicalised rather than after.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_same_page_twice_describes_each_resource_once(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("replay");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x52; 16])).await;

    let page = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    let (first, _) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(first, StatusCode::OK);
    let (second, body) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(second, StatusCode::OK, "a re-post is answered, not refused");
    let ack: ImportAck = serde_json::from_value(body).expect("the ack decodes");
    assert_eq!(
        ack.applied, 0,
        "and it applies nothing, which is how a device tells a retry from a first delivery"
    );

    let products = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM product WHERE org_id = $1",
    )
    .await;
    assert_eq!(products, 1, "one product, not two");
    let breadcrumbs = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM sync_request_resource WHERE org_id = $1",
    )
    .await;
    assert_eq!(breadcrumbs, 1, "and one breadcrumb");
}

/// D9(4). The completing page mints one create job, and a second one mints
/// nothing more.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_completing_page_mints_the_create_job_once(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("mint");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x53; 16])).await;

    let page = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A), observed(13_549_795, 0x5B)],
        skipped: Vec::new(),
        complete: true,
        failed: None,
    };
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the completing page applies: {body}"
    );
    let ack: ImportAck = serde_json::from_value(body).expect("the ack decodes");
    assert!(ack.complete);
    assert!(ack.create_job.is_some(), "two resources earn a create job");

    let jobs_before =
        pinned_count(&pool, ORG_A, "SELECT count(*) FROM job WHERE org_id = $1").await;
    let (again, _) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(
        again,
        StatusCode::OK,
        "the second completing page is a replay"
    );
    let jobs_after = pinned_count(&pool, ORG_A, "SELECT count(*) FROM job WHERE org_id = $1").await;
    assert_eq!(jobs_before, jobs_after, "and mints nothing more");

    let settled = view(&app, &TOKEN_A, request).await;
    assert_eq!(settled.state, "enqueued", "the request is settled");
    assert!(settled.create_job.is_some());
}

/// D9(5). An empty completing page completes, mints nothing, and a skip shows
/// its reason on its own breadcrumb.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_empty_catalogue_completes_rather_than_failing(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("empty");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x54; 16])).await;

    let page = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: Vec::new(),
        skipped: vec![SkippedResource {
            locator: Locator::from_resource_id(13_549_796),
            why: Reason::truncating("the bundle download was refused"),
        }],
        complete: true,
        failed: None,
    };
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an empty catalogue completes: {body}"
    );
    let ack: ImportAck = serde_json::from_value(body).expect("the ack decodes");
    assert!(ack.complete);
    assert!(
        ack.create_job.is_none(),
        "an itemless job reads back settled, so a catalogue with nothing to publish mints none"
    );

    let settled = view(&app, &TOKEN_A, request).await;
    assert_eq!(
        settled.state, "enqueued",
        "complete rather than failed: an empty shop is not an error"
    );
    let skipped = settled
        .resources
        .iter()
        .find(|row| row.state == "failed")
        .expect("the skip is a breadcrumb");
    assert_eq!(
        skipped.failure_detail.as_deref(),
        Some("the bundle download was refused"),
        "carrying the device's own reason, so the console can say what did not cross"
    );
    assert!(
        skipped.coverage.is_none(),
        "and nothing measured it, which is null rather than a zero that reads as measured"
    );
    let coverage = settled
        .coverage
        .expect("a device-enumerated migrate measures coverage");
    assert_eq!(
        coverage.rows, 0,
        "the sum is over measured breadcrumbs, and a skip is not one"
    );
}

/// D9(6). Tenancy: another org's request, another org's device, and a revoked
/// device are each refused with nothing written.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_page_is_refused_across_a_tenant_and_for_a_revoked_device(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, DEVICE_B).await;
    seed_crosswalk(&pool).await;
    let root = store_root("tenancy");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x55; 16])).await;

    let page = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };

    // B's session against A's request: missing, not forbidden, so B learns
    // nothing about whether the id exists.
    let (status, _) = post_page(&app, &TOKEN_B, DEVICE_B, &page).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another org's request is not there"
    );

    // A's session naming B's device.
    let (status, _) = post_page(&app, &TOKEN_A, DEVICE_B, &page).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a device of another org is not this org's"
    );

    // A's own device, revoked.
    DeviceRepo::new(pool.clone())
        .register(
            ORG_A,
            &DeviceRegistration {
                id: REVOKED,
                name: "a signed-out machine",
                os: "linux",
                arch: "x86_64",
                app_version: CURRENT,
            },
            NOW,
        )
        .await
        .expect("the second device registers");
    DeviceRepo::new(pool.clone())
        .revoke(ORG_A, REVOKED, NOW)
        .await
        .expect("the device revokes");
    let (status, _) = post_page(&app, &TOKEN_A, REVOKED, &page).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a revoked device may not report a catalogue"
    );

    let products = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM product WHERE org_id = $1",
    )
    .await;
    assert_eq!(products, 0, "and none of the three wrote anything");
    let breadcrumbs = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM sync_request_resource WHERE org_id = $1",
    )
    .await;
    assert_eq!(breadcrumbs, 0);
}

/// D9(7). The waiting reason appears while no device can run a sourced item,
/// and clears when one can.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_waiting_reason_names_the_version_until_a_device_reaches_it(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("waiting");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x56; 16])).await;

    // Registering the same id again is how a device reports a version: the
    // upsert refreshes `app_version` and leaves `revoked_at` alone.
    DeviceRepo::new(pool.clone())
        .register(
            ORG_A,
            &DeviceRegistration {
                id: DEVICE_A,
                name: "a test machine",
                os: "linux",
                arch: "x86_64",
                app_version: STALE,
            },
            NOW,
        )
        .await
        .expect("the device re-registers below the version");
    let stale = view(&app, &TOKEN_A, request).await;
    assert_eq!(
        stale.waiting_for_device_version.as_deref(),
        Some("0.9.0"),
        "the seller is told which version their machine needs, rather than watching an item \
         wait in silence"
    );

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
        .expect("the device updates");
    let ready = view(&app, &TOKEN_A, request).await;
    assert_eq!(
        ready.waiting_for_device_version, None,
        "and it clears the moment one device can run the work"
    );
}

/// The serialized view, as the console will actually receive it.
///
/// Produced by serialising what the handler returns rather than by hand, so
/// the console's types are checked against bytes. Covers a measured resource,
/// a skipped one, the request-level sum, and the shapes that are absent.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_serialized_view_is_what_the_console_receives(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("shape");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x57; 16])).await;

    let page = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: vec![SkippedResource {
            locator: Locator::from_resource_id(13_549_796),
            why: Reason::truncating("the bundle download was refused"),
        }],
        complete: true,
        failed: None,
    };
    let (status, _) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(status, StatusCode::OK);

    let settled = view(&app, &TOKEN_A, request).await;
    let json = serde_json::to_value(&settled).expect("the view serialises");
    eprintln!(
        "SyncRequestView as the console receives it:\n{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );

    let object = json.as_object().expect("the view is an object");
    assert!(object.contains_key("coverage"));
    assert!(object.contains_key("waiting_for_device_version"));
    let resources = object["resources"]
        .as_array()
        .expect("resources is an array");
    let measured = resources
        .iter()
        .find(|row| row["state"] == "canonicalised")
        .expect("the described resource");
    assert!(
        measured["coverage"].is_object(),
        "a measured resource carries its four counts as one object"
    );
    let skipped = resources
        .iter()
        .find(|row| row["state"] == "failed")
        .expect("the skipped resource");
    assert!(
        skipped["coverage"].is_null(),
        "and a skipped one carries null, not zeros"
    );
    assert_eq!(
        object["coverage"]["rows"], 1,
        "the request-level sum counts the measured breadcrumb and not the skip"
    );
}

/// A page from a device that is not this seller's is refused before the
/// vocabulary's own bounds are even reached, and a payload smuggled into a
/// bounded field is refused by the type.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_field_that_carries_a_payload_is_refused_at_the_wire(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("smuggle");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x58; 16])).await;

    // A media type carrying an encoded payload, posted as raw json so the
    // client-side constructor cannot refuse it first.
    let mut body = serde_json::to_value(&ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: Vec::new(),
        complete: false,
        failed: None,
    })
    .expect("the page serialises");
    body["resources"][0]["file"]["payload_content_type"] =
        serde_json::Value::String(base64(b"%PDF-1.7 the seller's own worksheet"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/v1/devices/{DEVICE_A}/import"))
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", TOKEN_A.to_hex()),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert!(
        response.status().is_client_error(),
        "the media type is a type and a subtype, so a payload encoded into it never decodes"
    );
    let products = pinned_count(
        &pool,
        ORG_A,
        "SELECT count(*) FROM product WHERE org_id = $1",
    )
    .await;
    assert_eq!(
        products, 0,
        "and nothing was written on the way to refusing it"
    );
}

/// The list a console needs to reach a request it navigated away from.
///
/// r-c7's M2: a device-branch migrate mints no job until its completing page,
/// so before this it appeared in no list at all and the console's own copy told
/// the seller to return to a page nothing linked to. The counts are shown apart
/// because a migration that crossed forty of fifty resources is a different
/// thing to a seller than one that crossed all forty it had.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_sync_list_reaches_a_request_that_minted_no_job(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, DEVICE_B).await;
    seed_crosswalk(&pool).await;
    let root = store_root("list");
    let app = router(configured(pool.clone(), &root));

    let first = open_request(&app, &TOKEN_A, Uuid([0x61; 16])).await;
    let second = open_request(&app, &TOKEN_A, Uuid([0x62; 16])).await;
    // One page against the second, with a skip, so the counts differ.
    let page = ImportPage {
        run: tam_types::Uuid([0; 16]),
        request: Some(second),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: vec![SkippedResource {
            locator: Locator::from_resource_id(13_549_796),
            why: Reason::truncating("the bundle download was refused"),
        }],
        complete: false,
        failed: None,
    };
    let (status, _) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(status, StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/sync")
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", TOKEN_A.to_hex()),
                )
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body reads")
        .to_bytes();
    let listed: SyncRequestListView = serde_json::from_slice(&bytes).expect("the list decodes");

    assert_eq!(listed.requests.len(), 2, "both of this org's requests");
    let described = listed
        .requests
        .iter()
        .find(|row| row.request == second)
        .expect("the request that was posted to");
    assert_eq!(
        (described.resources_total, described.resources_failed),
        (2, 1),
        "two breadcrumbs, one of them the skip, counted apart rather than folded together"
    );
    let untouched = listed
        .requests
        .iter()
        .find(|row| row.request == first)
        .expect("the request nothing was posted to");
    assert_eq!(
        (untouched.resources_total, untouched.resources_failed),
        (0, 0),
        "a request with no pages yet is listed with nothing, which is how the console reaches \
         one that has minted no job"
    );
    assert_eq!(described.disposition, "migrate");
    assert_eq!(described.state, "draining");

    // Another organisation's list is its own.
    let theirs = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/sync")
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", TOKEN_B.to_hex()),
                )
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    let bytes = theirs
        .into_body()
        .collect()
        .await
        .expect("the body reads")
        .to_bytes();
    let theirs: SyncRequestListView = serde_json::from_slice(&bytes).expect("the list decodes");
    assert!(
        theirs.requests.is_empty(),
        "a tenant sees its own requests and no other's"
    );
}

/// A device that stopped tells the request, and the seller's own page says so.
///
/// r-c5b2's O1. Everything the pass posts is the request's, but a terminal
/// failure that posted NOTHING — a first page that could not be sent, a
/// sign-out mid-pass — left the request holding exactly nothing, and the
/// console's request page watched a state that never changed. This is that
/// failure arriving where the seller is already looking.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_that_stopped_settles_the_request_with_its_reason(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("stopped");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x71; 16])).await;

    let stopped = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: true,
        failed: Some(Reason::truncating(
            "your catalogue could not be read: the session has expired",
        )),
    };
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &stopped).await;
    assert_eq!(status, StatusCode::OK, "the report is accepted: {body}");

    let settled = view(&app, &TOKEN_A, request).await;
    assert_eq!(
        settled.state, "failed",
        "a stopped import is over, so the request is terminal rather than left expecting pages"
    );
    assert_eq!(
        settled.failure_detail.as_deref(),
        Some("your catalogue could not be read: the session has expired"),
        "carrying the device's own sentence, which is what the request page renders"
    );

    // A late second report does not overwrite the first reason, and does not
    // reopen anything.
    let again = ImportPage {
        // The migrate leg names a request; `run` is the field a phase 2 import
        // names instead, and a page names one or the other. This one is the
        // request's, and the run identifier it carries is never read.
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        // The migration path names a request rather than a run, and is fenced
        // by that request rather than by a device attempt.
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: true,
        failed: Some(Reason::truncating("a different reason entirely")),
    };
    let (status, _) = post_page(&app, &TOKEN_A, DEVICE_A, &again).await;
    assert_eq!(status, StatusCode::OK);
    let after = view(&app, &TOKEN_A, request).await;
    assert_eq!(
        after.failure_detail.as_deref(),
        Some("your catalogue could not be read: the session has expired"),
        "the first reason stands: a report arriving after the request is terminal must not \
         replace what the seller was already told"
    );
}

/// The request's own page, as the console addresses it.
fn request_path(request: Uuid) -> String {
    format!(
        "/v1/sync/{}",
        uuid::Uuid::from_bytes(request.0).as_hyphenated()
    )
}

/// A bodyless call as the seller, which is every read and every Delete the
/// console makes against a migration.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn send(
    app: &axum::Router,
    token: &SessionToken,
    method: Method,
    path: &str,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(
                    header::COOKIE,
                    format!("{SESSION_COOKIE}={}", token.to_hex()),
                )
                .body(Body::empty())
                .expect("the request builds"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body reads")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// A resource the deletion interrupted is finished by the replay, and
/// nothing else is.
///
/// Storage admits a resource before the first of the four commits its apply
/// makes, so a process that died between two of them leaves a `pending`
/// breadcrumb carrying `admitted_at`. That marker is work in progress: it is
/// what keeps the request's Delete answering `stopping`, and the device's
/// replay of that page is the only thing that can retire it. The replay
/// arrives at a request the Delete has already settled, carrying the admitted
/// locator beside the next one the dead pass had reached.
///
/// Three things therefore hold at once: the admitted resource finishes, the
/// locator nothing admitted never starts, and a migration the seller stopped
/// mints no create leg. The deletion then converges and the request leaves the
/// seller's history, rather than sitting on a marker the sweep can only
/// reread for ever.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_replay_finishes_the_resource_a_deletion_interrupted(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("interrupted");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x74; 16])).await;
    let opening = ImportPage {
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    let (opened, body) = post_page(&app, &TOKEN_A, DEVICE_A, &opening).await;
    assert_eq!(
        opened,
        StatusCode::OK,
        "the page creates its anchor before admission: {body}"
    );

    // The admission an apply that died mid-write leaves behind, raised
    // through the same call the endpoint's own loop makes.
    let admitted = Locator::from_resource_id(13_549_794);
    let requests = tam_storage::SyncRequestRepo::new(pool.clone());
    assert_eq!(
        requests
            .admit_resource(ORG_A, request, admitted.as_str(), NOW)
            .await
            .expect("the admission answers"),
        tam_storage::ResourceAdmission::Admitted(0),
        "the interrupted page's own admission is the fixture"
    );

    let (stopping, body) = send(&app, &TOKEN_A, Method::DELETE, &request_path(request)).await;
    assert_eq!(
        stopping,
        StatusCode::ACCEPTED,
        "a request holding a resource mid-apply is stopping rather than gone: {body}"
    );
    assert_eq!(body["status"], "stopping");

    let replay = ImportPage {
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A), observed(13_549_795, 0x5B)],
        skipped: Vec::new(),
        complete: true,
        failed: None,
    };
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &replay).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the replay is accepted, because the resource the deletion admitted can only be \
         finished by the page that was applying it: {body}"
    );
    let ack: ImportAck = serde_json::from_value(body).expect("the ack decodes");
    assert_eq!(
        ack.applied, 1,
        "exactly the admitted resource finishes, and the locator nothing admitted does not start"
    );
    assert!(
        ack.create_job.is_none(),
        "and a migration the seller stopped mints no create leg"
    );
    assert_eq!(
        pinned_count(
            &pool,
            ORG_A,
            "SELECT count(*) FROM product WHERE org_id = $1",
        )
        .await,
        1,
        "one product, for the one resource that was already being written"
    );

    let (converged, body) = send(&app, &TOKEN_A, Method::DELETE, &request_path(request)).await;
    assert_eq!(
        converged,
        StatusCode::OK,
        "with nothing left mid-apply the deletion completes: {body}"
    );
    assert_eq!(body["status"], "deleted");
    let (page, _) = send(&app, &TOKEN_A, Method::GET, &request_path(request)).await;
    assert_eq!(
        page,
        StatusCode::NOT_FOUND,
        "and the stopped migration leaves the seller's history"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cancelled_page_does_not_duplicate_its_partly_committed_resource(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("interrupted-commit");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x76; 16])).await;
    sqlx::raw_sql(
        "CREATE FUNCTION pause_canonical_receipt() RETURNS trigger LANGUAGE plpgsql AS $$
           BEGIN
             IF NEW.state = 'canonicalised' THEN
               PERFORM pg_advisory_xact_lock(763294);
             END IF;
             RETURN NEW;
           END $$;
         CREATE TRIGGER pause_canonical_receipt
           BEFORE UPDATE ON sync_request_resource
           FOR EACH ROW EXECUTE FUNCTION pause_canonical_receipt();",
    )
    .execute(&pool)
    .await
    .expect("the crash gate is installed in this test database");
    let mut gate = pool.begin().await.expect("the gate opens");
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .expect("the gate identifies itself");
    sqlx::query("SELECT pg_advisory_xact_lock(763294)")
        .execute(&mut *gate)
        .await
        .expect("the gate holds the receipt boundary");
    let mut page = ImportPage {
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    // Selecting the gate drops the unfinished request future at this scope's
    // boundary, modelling the caller disappearing before its receipt commits.
    let writer_pid = tokio::select! {
        response = post_page(&app, &TOKEN_A, DEVICE_A, &page) => {
            panic!("the page finished before its closed receipt gate: {response:?}");
        }
        waiter = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let blocked: Option<i32> = sqlx::query_scalar(
                    "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
                     AND $1 = ANY(pg_blocking_pids(pid))",
                )
                .bind(gate_pid)
                .fetch_optional(&pool)
                .await
                .expect("the gate's waiter is observable");
                if let Some(pid) = blocked {
                    break pid;
                }
                tokio::task::yield_now().await;
            }
        }) => waiter.expect("the page reaches its canonical receipt"),
    };
    // Terminate only the identified waiter in this isolated test database:
    // this rolls back the interrupted transaction before the replay starts.
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1, 5000)")
        .bind(writer_pid)
        .fetch_one(&pool)
        .await
        .expect("the interrupted writer is disconnected");
    assert!(terminated);
    gate.rollback().await.expect("the gate releases");
    let (status, _) = send(&app, &TOKEN_A, Method::DELETE, &request_path(request)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    page.complete = true;
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &page).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["create_job"].is_null());
    assert_eq!(
        pinned_count(
            &pool,
            ORG_A,
            "SELECT count(*) FROM product WHERE org_id = $1"
        )
        .await,
        1,
        "a replay must not leave both the interrupted product and a fresh replacement"
    );
    let (status, body) = send(&app, &TOKEN_A, Method::DELETE, &request_path(request)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "deleted");
}

/// Deleting a migration's event anchor stops the migration, not just the row.
///
/// A legacy migration's anchor is that request's own itemless ledger row, and
/// the seller's publishing history offers it like any other job. Deleting it
/// as an independent job is instantaneous and changes nothing about the
/// migration: the device's next page reaches the same anchor under the same
/// key, goes on importing resources, and its completing page mints the create
/// leg the seller thought they had stopped.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn deleting_a_migrations_event_anchor_stops_the_migration(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, DEVICE_A).await;
    seed_crosswalk(&pool).await;
    let root = store_root("anchor-delete");
    let app = router(configured(pool.clone(), &root));
    let request = open_request(&app, &TOKEN_A, Uuid([0x75; 16])).await;

    let first = ImportPage {
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_794, 0x5A)],
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    let (status, body) = post_page(&app, &TOKEN_A, DEVICE_A, &first).await;
    assert_eq!(status, StatusCode::OK, "the first page applies: {body}");

    let (listed, body) = send(&app, &TOKEN_A, Method::GET, "/v1/jobs").await;
    assert_eq!(listed, StatusCode::OK, "the job history reads: {body}");
    let history: JobPage = serde_json::from_value(body).expect("the job page decodes");
    assert_eq!(
        history.jobs.len(),
        1,
        "a migration mid-read has minted nothing but its event anchor: {:?}",
        history.jobs
    );
    let anchor = history.jobs[0].job;

    let (deleted, body) = send(
        &app,
        &TOKEN_A,
        Method::DELETE,
        &format!(
            "/v1/jobs/{}",
            uuid::Uuid::from_bytes(anchor.0 .0).as_hyphenated()
        ),
    )
    .await;
    assert!(
        matches!(deleted, StatusCode::OK | StatusCode::ACCEPTED),
        "the Delete is answered: {deleted} {body}"
    );

    let (view_status, body) = send(&app, &TOKEN_A, Method::GET, &request_path(request)).await;
    assert_eq!(
        view_status,
        StatusCode::NOT_FOUND,
        "the anchor is the migration's own ledger row, so deleting it stops the migration \
         rather than leaving the request open behind it: {body}"
    );

    // The device's next page, which reaches the same anchor under the same
    // key. It must import nothing and mint nothing.
    let next = ImportPage {
        run: tam_types::Uuid([0; 16]),
        request: Some(request),
        attempt: None,
        receipt: None,
        enumeration_complete: false,
        listed: None,
        resources: vec![observed(13_549_796, 0x5C)],
        skipped: Vec::new(),
        complete: true,
        failed: None,
    };
    let (refused, body) = post_page(&app, &TOKEN_A, DEVICE_A, &next).await;
    assert!(
        matches!(refused, StatusCode::NOT_FOUND | StatusCode::CONFLICT),
        "a page for a stopped migration is refused rather than applied: {refused} {body}"
    );
    assert_eq!(
        pinned_count(
            &pool,
            ORG_A,
            "SELECT count(*) FROM product WHERE org_id = $1",
        )
        .await,
        1,
        "the page after the deletion creates no resource"
    );
    let (_, body) = send(&app, &TOKEN_A, Method::GET, "/v1/jobs").await;
    let after: JobPage = serde_json::from_value(body).expect("the job page decodes");
    assert!(
        after.jobs.is_empty(),
        "and no create leg is minted for a migration whose anchor the seller deleted: {:?}",
        after.jobs
    );
}
