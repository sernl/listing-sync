//! The marketplace import as a run, end to end: a list, a selection, a
//! matcher, a review and a commit.
//!
//! The property every test here exists to hold is the one phase 2 adds: the
//! catalogue never gains a second copy of a resource it already holds, and it
//! never merges two real resources without being told to. Those pull in
//! opposite directions, which is why the matcher's outcomes are asserted
//! against real rows — a product count, a label, a tombstone — rather than
//! against a score.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::duplicates::DuplicatesView;
use tam_api::import_runs::{
    ImportRunItemState, ImportRunStage, ImportRunState, ImportRunView, ImportRunsView,
    RunConfirmAck,
};
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_engine_driver::import::{
    ContentType, Cover, FileName, ImportPage, ListedResource, Locator, ObservedFile,
    ObservedResource,
};
use tam_fingerprint::{Fingerprint, TextSketch};
use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
use tam_storage::{DeviceRegistration, DeviceRepo, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, FileKind, ImportedPrice, Marketplace, OrgId, ProductId, ScanOutcome,
    Timestamp, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xA1; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN_A: SessionToken = SessionToken([0x51; 32]);
const NOW: Timestamp = Timestamp(5_000);
const DEVICE_A: &str = "11112222333344445555666677778888";
const CURRENT: &str = "0.9.0";

/// Comfortably past the matcher's fifty-kilobyte floor, so an exact digest is
/// decisive rather than "as likely a licence note as a resource".
const BIG: u64 = 120_000;

// ------------------------------------------------------------------ harness

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("runs-{name}-{}", std::process::id()));
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
    // Reading a shop and reviewing duplicates are both paid capabilities, so
    // the fixture subscribes: without a grant every page here would be
    // answered by the plan gate rather than by the machinery under test.
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            ORG_A,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Subscriber,
                rung: None,
                granted_by: tam_storage::GrantedBy::Paddle,
                grantor_user: None,
                reason: None,
                source_ref: Some("sub_a1"),
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("the fixture grant seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG_A, USER_A, "a1@example.test", NOW)
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
         VALUES ($1, $2, 'tes', 'linked', $3, $3), ($1, $4, 'tpt', 'linked', $3, $3)",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0xC1; 16]))
    .bind(sqlx::types::chrono::DateTime::from_timestamp_millis(NOW.0).expect("a valid instant"))
    .bind(uuid::Uuid::from_bytes([0xC2; 16]))
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
async fn call(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> Answer {
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
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    Answer {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    }
}

/// The fence every page in this file is posted under.
///
/// One claim per run and one device, so the granted attempt is always the
/// first: asserted in [`claim_run`] rather than assumed, because a page under
/// the wrong attempt is exactly what the protocol refuses and a helper that
/// guessed it would make every test here pass for the wrong reason.
const ATTEMPT: u64 = 1;

async fn open_run(app: &axum::Router) -> Answer {
    open_run_on(app, "Tes").await
}

async fn open_run_on(app: &axum::Router, source: &str) -> Answer {
    open_run_keyed(app, source, fresh_uuid()).await
}

/// A start, under a key the caller chose: the same key twice is one import.
async fn open_run_keyed(app: &axum::Router, source: &str, start_key: Uuid) -> Answer {
    call(
        app,
        Method::POST,
        "/v1/imports/runs",
        Some(serde_json::json!({
            "source": source,
            "start_key": uuid_text(start_key),
        })),
    )
    .await
}

async fn started_run(app: &axum::Router) -> Uuid {
    started_run_on(app, "Tes").await
}

/// Opens a run and claims it for this device, which is the order the protocol
/// requires: the claim precedes any local preflight, and no page is accepted
/// without the attempt it grants.
async fn started_run_on(app: &axum::Router, source: &str) -> Uuid {
    let answer = open_run_on(app, source).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "the run opens: {}",
        answer.body
    );
    let view: ImportRunView = answer.json();
    assert_eq!(
        view.state,
        ImportRunState::Reading,
        "a fresh run is reading until the device lists the shop"
    );
    assert_eq!(
        view.execution.stage,
        ImportRunStage::Waiting,
        "an unclaimed run is waiting for a device rather than reading"
    );
    assert_eq!(
        claim_run(app, view.id).await,
        ATTEMPT,
        "the first claim grants the first fence"
    );
    view.id
}

/// Claims one run for this device, answering the fence it granted.
async fn claim_run(app: &axum::Router, run: Uuid) -> u64 {
    let answer = call(
        app,
        Method::POST,
        &format!("/v1/devices/{DEVICE_A}/import/{}/claim", uuid_text(run)),
        Some(serde_json::json!({ "takeover": false })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the claim: {}", answer.body);
    answer.body["attempt"].as_u64().unwrap_or_default()
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

/// The seller's confirmation that this run's resources are to be created.
///
/// Durable acceptance rather than a chunk of work: the server's own drain
/// creates them, which is why every test here confirms and then drains
/// instead of looping on a chunk acknowledgement.
async fn confirm(app: &axum::Router, run: Uuid) -> Answer {
    call(
        app,
        Method::POST,
        &format!("/v1/imports/runs/{}/commit", uuid_text(run)),
        None,
    )
    .await
}

/// One pass of the server's own worker, which is what creates the resources
/// an authorised run owes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn drain(state: &AppState) {
    tam_api::scheduler::pass(state, NOW)
        .await
        .expect("the pass runs");
}

/// Confirms and drains, and answers the run as it stands afterwards.
async fn confirm_and_drain(app: &axum::Router, state: &AppState, run: Uuid) -> ImportRunView {
    let accepted = confirm(app, run).await;
    assert_eq!(
        accepted.status,
        StatusCode::ACCEPTED,
        "the confirmation is accepted: {}",
        accepted.body
    );
    let ack: RunConfirmAck = accepted.json();
    assert!(ack.accepted, "the confirmation is recorded");
    assert!(
        ack.run.execution.commit_authorised,
        "and it is what authorises the commit"
    );
    drain(state).await;
    run_view(app, run).await
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

fn uuid_text(id: Uuid) -> String {
    uuid::Uuid::from_bytes(id.0).to_string()
}

fn product_text(id: ProductId) -> String {
    uuid_text(id.0)
}

// ----------------------------------------------------------------- fixtures

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn cover() -> Cover {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(b"IHDR a derived thumbnail");
    Cover::encode(&png).expect("the fixture is a PNG within the ceiling")
}

/// A sketch whose MinHash agrees with another in exactly `shared` of its 128
/// positions, which is what the estimator reads as a Jaccard.
///
/// Built by hand rather than from text, because the quantity under test is the
/// matcher's band boundary and deriving it from prose would make the test
/// depend on how many five-word shingles two paragraphs happen to share.
fn sketch(simhash: u64, shared: usize, tag: u32) -> TextSketch {
    let mut minhash = [0x1111_1111_u32; 128];
    for (slot, value) in minhash.iter_mut().enumerate() {
        if slot >= shared {
            *value = 0x2222_0000 | tag;
        }
    }
    TextSketch {
        simhash,
        minhash,
        shingle_count: 400,
        extracted_chars: 9_000,
    }
}

fn listed(locator: &str, title: &str) -> ListedResource {
    ListedResource {
        locator: Locator::new(locator).unwrap_or_else(|_| Locator::from_resource_id(1)),
        title: title.to_owned(),
        price_minor: None,
        currency: None,
        state: Some(ListingState::Live),
    }
}

/// One resource as a device describes it.
fn observed(
    locator: &str,
    title: &str,
    digest: Option<(u8, u64)>,
    text: Option<TextSketch>,
) -> ObservedResource {
    observed_on(Marketplace::Tes, locator, title, digest, text)
}

/// The same read from the shop named: a TPT locator is the product's number,
/// which is what its remote id carries.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn observed_on(
    shop: Marketplace,
    locator: &str,
    title: &str,
    digest: Option<(u8, u64)>,
    text: Option<TextSketch>,
) -> ObservedResource {
    let remote = match shop {
        Marketplace::Tes => RemoteListingId::Tes {
            url: locator.to_owned(),
        },
        Marketplace::Tpt => RemoteListingId::Tpt {
            product_id: locator.parse().expect("a TPT locator is a number"),
        },
        Marketplace::Etsy => RemoteListingId::Etsy {
            listing_id: locator.parse().expect("an Etsy locator is a number"),
        },
    };
    ObservedResource {
        locator: Locator::new(locator).expect("a bounded locator"),
        listing: ImportedListing {
            remote,
            title: title.to_owned(),
            body: "A worksheet.".to_owned(),
            body_format: CopyFormat::Markdown,
            native: Vec::new(),
            rights: None,
            price: ImportedPrice::Free,
            state: Some(ListingState::Live),
        },
        fingerprint: Some(Fingerprint {
            version: tam_fingerprint::FINGERPRINT_VERSION,
            text,
            page_count: None,
            cover_phash: None,
            title_norm: tam_fingerprint::normalise_title(title),
        }),
        file: digest.map(|(seed, byte_len)| ObservedFile {
            payload_file_name: FileName::new("worksheet-pack.pdf").expect("a plain name"),
            payload_content_type: ContentType::new("application/pdf").expect("a media type"),
            kind: FileKind::Pdf,
            hash: ContentHash([seed; 32]),
            byte_len,
            scan: ScanOutcome::Clean { at: NOW },
            entry: None,
        }),
        cover_png: Some(cover()),
    }
}

/// One page of descriptions, under this device's fence and with its own
/// receipt, which is what makes a resend safe.
fn page(run: Uuid, resources: Vec<ObservedResource>, complete: bool) -> ImportPage {
    ImportPage {
        run,
        request: None,
        attempt: Some(ATTEMPT),
        receipt: Some(fresh_uuid()),
        enumeration_complete: false,
        listed: None,
        resources,
        skipped: Vec::new(),
        complete,
        failed: None,
    }
}

/// The shop's own list, whole: the enumeration is closed by this page, which
/// is what lets the seller tick from it.
fn listing_page(run: Uuid, rows: Vec<ListedResource>) -> ImportPage {
    ImportPage {
        run,
        request: None,
        attempt: Some(ATTEMPT),
        receipt: Some(fresh_uuid()),
        enumeration_complete: true,
        listed: Some(rows),
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    }
}

/// Reads one shop of three resources into the catalogue, whole: list, select
/// all, describe, confirm, drain. The catalogue this leaves behind is what
/// the later runs are matched against.
async fn seed_catalogue(app: &axum::Router, state: &AppState) -> ImportRunView {
    let run = started_run(app).await;
    let answer = post_page(
        app,
        &listing_page(
            run,
            vec![
                listed("https://www.tes.com/teaching-resource/-1", "Fractions pack"),
                listed("https://www.tes.com/teaching-resource/-2", "Long division"),
                listed("https://www.tes.com/teaching-resource/-3", "Shape hunt"),
            ],
        ),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the list lands: {}",
        answer.body
    );

    let selected = call(
        app,
        Method::POST,
        &format!("/v1/imports/runs/{}/select", uuid_text(run)),
        Some(serde_json::json!({ "all": true })),
    )
    .await;
    assert_eq!(selected.status, StatusCode::OK, "{}", selected.body);

    let described = page(
        run,
        vec![
            observed(
                "https://www.tes.com/teaching-resource/-1",
                "Fractions pack",
                Some((0x5A, BIG)),
                None,
            ),
            observed(
                "https://www.tes.com/teaching-resource/-2",
                "Long division worksheet pack",
                Some((0x5B, BIG)),
                Some(sketch(0x0F0F_0F0F_0F0F_0F0F, 128, 1)),
            ),
            observed(
                "https://www.tes.com/teaching-resource/-3",
                "Shape hunt",
                Some((0x5C, BIG)),
                None,
            ),
        ],
        true,
    );
    let answer = post_page(app, &described).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the descriptions land: {}",
        answer.body
    );

    let view = run_view(app, run).await;
    assert_eq!(
        view.state,
        ImportRunState::Reviewing,
        "finishing the descriptions puts the run in front of the seller rather than \
         authorising anything"
    );
    assert!(
        !view.execution.commit_authorised,
        "nothing is authorised until the seller confirms"
    );
    // The drain creates nothing for an unconfirmed run, which is the whole
    // point of the confirmation being a fact of its own.
    drain(state).await;
    assert_eq!(
        run_view(app, run).await.counts.imported,
        0,
        "an unconfirmed run creates no resources"
    );

    let settled = confirm_and_drain(app, state, run).await;
    assert_eq!(
        (settled.state, settled.counts.imported),
        (ImportRunState::Complete, 3),
        "three resources, one drain, and the run settles"
    );
    settled
}

// -------------------------------------------------------------------- tests

/// The whole shape, in one pass: what the seller sees at every stage and what
/// the catalogue holds at the end.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_run_lists_selects_matches_reviews_and_commits(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("whole"));
    let app = router(state.clone());

    let seeded = seed_catalogue(&app, &state).await;
    let held: Vec<ProductId> = seeded
        .items
        .iter()
        .filter_map(|item| item.product_id)
        .collect();
    assert_eq!(held.len(), 3, "every item of the first run made a product");

    // The marketplace's own label, on every resource the first run created,
    // and outside the seller's own vocabulary.
    let labels: serde_json::Value = call(
        &app,
        Method::GET,
        &format!("/v1/products/{}/labels", product_text(held[0])),
        None,
    )
    .await
    .body;
    assert_eq!(
        labels,
        serde_json::json!({ "labels": [{ "name": "Tes", "colour": "blue", "system": true }] }),
        "an imported resource carries the shop's own label, in that shop's own colour"
    );

    // ---- the second run reads the seller's other shop: one byte-identical,
    // one similar, one new. The other shop and not the same one, because the
    // matcher compares across marketplaces only: every product the first run
    // made is bound to the Tes listing it came from, and a second Tes listing
    // by the same seller is a product they chose to have twice.
    let run = started_run_on(&app, "Tpt").await;
    let list = listing_page(
        run,
        vec![
            listed("11", "Fractions pack"),
            listed("12", "Long division worksheets pack"),
            listed("13", "Number bonds"),
            listed("14", "Left behind"),
        ],
    );
    assert_eq!(post_page(&app, &list).await.status, StatusCode::OK);

    let view = run_view(&app, run).await;
    assert_eq!(view.read_total, Some(4), "the list is the total");
    assert_eq!(view.counts.listed, 4);

    // A tick list rather than everything, so the unchosen one is skipped with
    // the reason the seller can read.
    let selected = call(
        &app,
        Method::POST,
        &format!("/v1/imports/runs/{}/select", uuid_text(run)),
        Some(serde_json::json!({
            "locators": ["11", "12", "13"]
        })),
    )
    .await;
    assert_eq!(selected.status, StatusCode::OK, "{}", selected.body);
    let view: ImportRunView = selected.json();
    assert_eq!((view.counts.selected, view.counts.skipped), (3, 1));
    let left = view
        .items
        .iter()
        .find(|item| item.locator.as_str() == "14")
        .expect("the unchosen row is on the run");
    assert_eq!(left.state, ImportRunItemState::Skipped);
    assert_eq!(left.skip_reason.as_deref(), Some("not chosen"));

    // What the device is to describe, read back the way the device reads it.
    let selection: serde_json::Value = call(
        &app,
        Method::GET,
        &format!("/v1/devices/{DEVICE_A}/import/{}/selection", uuid_text(run)),
        None,
    )
    .await
    .body;
    assert_eq!(
        selection["locators"].as_array().map(Vec::len),
        Some(3),
        "the device is handed exactly what the seller ticked"
    );

    let described = page(
        run,
        vec![
            // Byte for byte the first run's first resource: decisive, and the
            // seller is never asked.
            observed_on(
                Marketplace::Tpt,
                "11",
                "Fractions pack",
                Some((0x5A, BIG)),
                None,
            ),
            // The same text at ninety of a hundred and twenty-eight positions
            // and nearly the same title: two moderate signals, which is the
            // ask-the-seller band.
            observed_on(
                Marketplace::Tpt,
                "12",
                "Long division worksheets pack",
                Some((0x7B, BIG)),
                Some(sketch(0x0F0F_0F0F_0F0F_0F0F, 90, 2)),
            ),
            // Nothing in common with anything.
            observed_on(
                Marketplace::Tpt,
                "13",
                "Number bonds",
                Some((0x7C, BIG)),
                None,
            ),
        ],
        true,
    );
    let answer = post_page(&app, &described).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

    let view = run_view(&app, run).await;
    assert_eq!(
        view.state,
        ImportRunState::Reviewing,
        "a question owed puts the run in front of the seller"
    );
    let by = |suffix: &str| -> ImportRunItemState {
        view.items
            .iter()
            .find(|item| item.locator.as_str() == suffix)
            .map_or(ImportRunItemState::Failed, |item| item.state)
    };
    assert_eq!(
        by("11"),
        ImportRunItemState::Skipped,
        "an exact, large, rare file is the same resource and is not imported twice"
    );
    assert_eq!(
        by("12"),
        ImportRunItemState::Review,
        "two moderate signals are a question, not an answer"
    );
    assert_eq!(
        by("13"),
        ImportRunItemState::Matched,
        "silence is not a merge"
    );

    let merged = view
        .items
        .iter()
        .find(|item| item.locator.as_str() == "11")
        .expect("the merged row is on the run");
    assert_eq!(
        merged.skip_reason.as_deref(),
        Some("same as Fractions pack"),
        "the seller is told which resource this already was"
    );

    assert_eq!(
        view.review_pairs.len(),
        1,
        "one card, for the one pair nobody could decide"
    );
    let pair = &view.review_pairs[0];
    assert_eq!(
        pair.sentence, "The text of both PDFs is 70% the same.",
        "one sentence of evidence, and never a score"
    );

    // ---- the seller confirms, and the drain takes the matched item while the
    // question waits.
    let after = confirm_and_drain(&app, &state, run).await;
    assert_eq!(
        (after.state, after.counts.imported, after.counts.review),
        (ImportRunState::Committing, 1, 1),
        "the matched resource is created and the parked one waits"
    );
    assert_eq!(
        products_held(&pool).await,
        4,
        "four resources, not five: the byte-identical read created nothing"
    );

    // ---- the seller answers, and the answer is what unblocks the item.
    let answered = call(
        &app,
        Method::POST,
        format!(
            "/v1/duplicates/{}/{}",
            product_text(pair.product_lo),
            product_text(pair.product_hi)
        )
        .as_str(),
        Some(serde_json::json!({ "verdict": "different" })),
    )
    .await;
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.body);

    // No second confirmation: the run is already authorised, so the server's
    // own drain is what finishes it. A seller who answered and closed the tab
    // loses nothing.
    drain(&state).await;
    let view = run_view(&app, run).await;
    assert_eq!(
        (view.state, view.counts.imported),
        (ImportRunState::Complete, 2),
        "answering the question is what lets the run finish"
    );
    assert_eq!(products_held(&pool).await, 5);
    assert_eq!(
        view.execution.stage,
        ImportRunStage::Completed,
        "a settled run says what it settled as"
    );
    assert!(
        view.review_pairs.is_empty(),
        "an answered pair leaves the queue"
    );

    // ---- and the answer is remembered: the same pair is never asked again.
    let again = started_run(&app).await;
    let relisted = listing_page(
        again,
        vec![listed(
            "https://www.tes.com/teaching-resource/-22",
            "Long division worksheets pack",
        )],
    );
    assert_eq!(post_page(&app, &relisted).await.status, StatusCode::OK);
    let open: DuplicatesView = call(&app, Method::GET, "/v1/duplicates", None).await.json();
    assert!(
        open.pairs.is_empty(),
        "the pair the seller called different is not re-raised"
    );
}

/// A page that lists and describes in one post keys its question to the
/// identifier the row actually holds.
///
/// The failure this closes leaves a resource unimportable forever. On a
/// combined page the list is appended first, which reserves the row's
/// identifier, and the description's insert conflicts and keeps it — so a
/// question raised under the identifier the preparation had minted names a
/// product no row carries. The duplicate card has a side nothing points at,
/// `item_of_product` finds nothing for it, and answering `different` cannot
/// unblock the row: the seller answers, and the item stays in review through
/// every later drain.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_combined_page_raises_its_question_against_the_row(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("combined"));
    let app = router(state.clone());
    seed_catalogue(&app, &state).await;

    // One post carrying both halves: the shop's list, and a description of
    // the listing in it. The description's subject is the review band against
    // the seeded catalogue — the same text at ninety of a hundred and
    // twenty-eight positions and nearly the same title.
    let run = started_run_on(&app, "Tpt").await;
    let combined = ImportPage {
        run,
        request: None,
        attempt: Some(ATTEMPT),
        receipt: Some(fresh_uuid()),
        enumeration_complete: true,
        listed: Some(vec![listed("12", "Long division worksheets pack")]),
        resources: vec![observed_on(
            Marketplace::Tpt,
            "12",
            "Long division worksheets pack",
            Some((0x7B, BIG)),
            Some(sketch(0x0F0F_0F0F_0F0F_0F0F, 90, 2)),
        )],
        skipped: Vec::new(),
        complete: true,
        failed: None,
    };
    let answer = post_page(&app, &combined).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

    let view = run_view(&app, run).await;
    let item = view
        .items
        .iter()
        .find(|item| item.locator.as_str() == "12")
        .expect("the described row is on the run");
    assert_eq!(
        item.state,
        ImportRunItemState::Review,
        "two moderate signals are a question"
    );
    assert_eq!(
        view.review_pairs.len(),
        1,
        "one card, for the one pair nobody could decide"
    );
    let pair = &view.review_pairs[0];
    assert!(
        pair.lo.run_locator.as_deref() == Some("12")
            || pair.hi.run_locator.as_deref() == Some("12"),
        "the question addresses the row the seller can answer"
    );

    // And the answer is what the whole thing is for: it has to reach the row.
    let answered = call(
        &app,
        Method::POST,
        format!(
            "/v1/duplicates/{}/{}",
            product_text(pair.product_lo),
            product_text(pair.product_hi)
        )
        .as_str(),
        Some(serde_json::json!({ "verdict": "different" })),
    )
    .await;
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.body);

    let settled = confirm_and_drain(&app, &state, run).await;
    assert_eq!(
        (
            settled.state,
            settled.counts.imported,
            settled.counts.review
        ),
        (ImportRunState::Complete, 1, 0),
        "the answered row is created rather than waiting in review forever"
    );
}

/// A file every product carries is the seller's boilerplate, and agreeing on
/// it earns nothing.
///
/// The failure this exists to prevent is total: without frequency weighting,
/// one shared `Terms of Use.pdf` makes every product in the catalogue an exact
/// match for every other, and an unattended merge collapses the whole shop.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_digest_three_products_carry_decides_nothing(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("common"));
    let app = router(state.clone());

    // Three resources with one file between them, whose titles share nothing.
    let run = started_run(&app).await;
    let described = page(
        run,
        vec![
            observed("shared-a", "Autumn term planning", Some((0x33, BIG)), None),
            observed("shared-b", "Phonics flashcards", Some((0x33, BIG)), None),
            observed("shared-c", "Weather diary", Some((0x33, BIG)), None),
        ],
        true,
    );
    assert_eq!(post_page(&app, &described).await.status, StatusCode::OK);
    let settled = confirm_and_drain(&app, &state, run).await;
    assert_eq!(
        settled.counts.imported, 3,
        "nothing merged them on the way in, and nothing merged them at the commit either"
    );
    assert_eq!(products_held(&pool).await, 3);

    // A fourth read carrying the same file. The digest is now on three
    // products, so it says nothing about identity.
    let run = started_run(&app).await;
    let described = page(
        run,
        vec![observed(
            "shared-d",
            "Multiplication grids",
            Some((0x33, BIG)),
            None,
        )],
        true,
    );
    assert_eq!(post_page(&app, &described).await.status, StatusCode::OK);
    let view = run_view(&app, run).await;
    assert_eq!(
        view.items.first().map(|item| item.state),
        Some(ImportRunItemState::Matched),
        "a file the whole catalogue carries neither merges nor asks"
    );
    assert!(view.review_pairs.is_empty());
}

/// A merge of two real resources tombstones the loser, and the undo brings it
/// back inside the window the seller was told.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn merging_two_products_tombstones_the_loser_and_the_undo_restores_it(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("merge"));
    let app = router(state.clone());
    let seeded = seed_catalogue(&app, &state).await;
    let held: Vec<ProductId> = seeded
        .items
        .iter()
        .filter_map(|item| item.product_id)
        .collect();
    let (lo, hi) = tam_storage::ordered_pair(held[0], held[1]);

    // The question, raised through the repository rather than through a read.
    // Both sides of this pair are products, which is the branch a read cannot
    // reach on its own: the review runs before anything is created, so a pair
    // a read raises always has an uncreated side.
    tam_storage::DuplicateRepo::new(pool.clone())
        .raise(
            ORG_A,
            &tam_storage::NewVerdict {
                lo,
                hi,
                verdict: tam_storage::Verdict::Parked,
                decided_by: tam_storage::DecidedBy::Seller,
                winning_layer: tam_storage::MatchLayer::L4,
                log_odds: 3.5,
                fingerprint_version: 1,
                run: None,
                kept: None,
                raised_at: NOW,
                decided_at: None,
                reversible_until: None,
                evidence: &[tam_storage::Evidence {
                    layer: tam_storage::MatchLayer::L4,
                    polarity: tam_storage::Polarity::Positive,
                    measure: 0.9,
                    unit: tam_storage::EvidenceUnit::Jaccard,
                    observed_in: None,
                }],
            },
        )
        .await
        .expect("the question is raised");

    let open: DuplicatesView = call(&app, Method::GET, "/v1/duplicates", None).await.json();
    assert_eq!(open.pairs.len(), 1, "the card is drawn from the stored row");
    assert_eq!(open.pairs[0].sentence, "Nearly the same title.");

    let merged = call(
        &app,
        Method::POST,
        &format!("/v1/duplicates/{}/{}", product_text(lo), product_text(hi)),
        Some(serde_json::json!({
            "verdict": "same",
            "keep": product_text(lo),
            "fields": { "title": "hi" }
        })),
    )
    .await;
    assert_eq!(merged.status, StatusCode::OK, "{}", merged.body);
    assert_eq!(
        live_products(&pool).await,
        2,
        "the loser stops being a second resource"
    );
    assert_eq!(
        products_held(&pool).await,
        3,
        "and is tombstoned rather than erased, which is what makes the undo possible"
    );

    let undone = call(
        &app,
        Method::POST,
        &format!(
            "/v1/duplicates/{}/{}/undo",
            product_text(lo),
            product_text(hi)
        ),
        None,
    )
    .await;
    assert_eq!(undone.status, StatusCode::OK, "{}", undone.body);
    assert_eq!(
        live_products(&pool).await,
        3,
        "the reversal gives the seller their resource back"
    );
    let open: DuplicatesView = call(&app, Method::GET, "/v1/duplicates", None).await.json();
    assert_eq!(
        open.pairs.len(),
        1,
        "and puts the question back rather than answering it for them"
    );
}

/// The marketplace's label survives the seller editing their own labels.
///
/// Without this the auto-label lasts exactly until the first time a seller
/// uses the feature it was there to help: `set_for_product` is a replace, and
/// the sweep that follows it deletes a label nothing carries.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_marketplace_label_survives_the_seller_editing_their_own(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("labels"));
    let app = router(state.clone());
    let seeded = seed_catalogue(&app, &state).await;
    let product = seeded
        .items
        .iter()
        .find_map(|item| item.product_id)
        .expect("the run created a product");

    let set = call(
        &app,
        Method::PUT,
        &format!("/v1/products/{}/labels", product_text(product)),
        Some(serde_json::json!({ "labels": ["Autumn term"] })),
    )
    .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.body);
    assert_eq!(
        set.body,
        serde_json::json!({ "labels": [
            { "name": "Autumn term", "colour": tam_storage::Colour::of_name("Autumn term").as_str(), "system": false },
            { "name": "Tes", "colour": "blue", "system": true },
        ] }),
        "the seller's own label is added and the shop's own label stays"
    );

    // And the seller cannot spell it themselves, because the colour and the
    // flag are ours: a route that accepted it would write a label this one
    // does not own.
    let claimed = call(
        &app,
        Method::PUT,
        &format!("/v1/products/{}/labels", product_text(product)),
        Some(serde_json::json!({ "labels": ["tes"] })),
    )
    .await;
    assert_eq!(
        claimed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        claimed.body
    );

    // Nor rename it, nor delete it: both answer as a name nobody holds.
    let renamed = call(
        &app,
        Method::PATCH,
        "/v1/labels/Tes",
        Some(serde_json::json!({ "name": "Tes UK" })),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::NOT_FOUND);
    let deleted = call(&app, Method::DELETE, "/v1/labels/Tes", None).await;
    assert_eq!(deleted.status, StatusCode::NOT_FOUND);
}

/// One import per shop, and both shops advance together.
///
/// The organisation-wide fence this replaces is the reason a seller reading
/// Tes could not start TPT. Pressing the button twice on one shop is one
/// import and answers with it; the same start key twice is the same import
/// again, whatever happened to it in between.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_import_per_shop_and_a_start_key_is_one_import(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("open"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    // The same shop again: the answer is the import already open on it,
    // rather than a second one and rather than a refusal the seller has to
    // read a list to act on.
    let second = open_run(&app).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    let linked: ImportRunView = second.json();
    assert_eq!(linked.id, run, "a second start on one shop links to it");

    // The other shop, at the same time.
    let other = open_run_on(&app, "Tpt").await;
    assert_eq!(
        other.status,
        StatusCode::CREATED,
        "another source remains startable while the first is in flight: {}",
        other.body
    );

    // Both are offered to the device, which decides for itself which it has a
    // session for.
    let open: serde_json::Value = call(
        &app,
        Method::GET,
        &format!("/v1/devices/{DEVICE_A}/import/open"),
        None,
    )
    .await
    .body;
    assert_eq!(
        open.as_array().map(Vec::len),
        Some(2),
        "the device is offered every open run rather than one: {open}"
    );

    // One key, one import: replayed after the run has settled, it still
    // reaches the run it made rather than minting another.
    let key = fresh_uuid();
    let keyed = open_run_keyed(&app, "Etsy", key).await;
    assert_eq!(
        keyed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an official-API marketplace is read on our own infrastructure: {}",
        keyed.body
    );

    let abandoned = call(
        &app,
        Method::POST,
        &format!("/v1/imports/runs/{}/abandon", uuid_text(run)),
        None,
    )
    .await;
    assert_eq!(abandoned.status, StatusCode::NO_CONTENT);

    let retried = open_run_keyed(&app, "Tes", key).await;
    assert_eq!(retried.status, StatusCode::CREATED, "{}", retried.body);
    let minted: ImportRunView = retried.json();
    let replayed = open_run_keyed(&app, "Tes", key).await;
    assert_eq!(replayed.status, StatusCode::OK, "{}", replayed.body);
    assert_eq!(
        replayed.json::<ImportRunView>().id,
        minted.id,
        "a replayed start key reaches the run it already made"
    );

    let spent = open_run_keyed(&app, "Tpt", key).await;
    assert_eq!(spent.status, StatusCode::CONFLICT);
    assert_eq!(
        spent.body["errors"][0]["code"], "import_start_key_spent",
        "the same key on a different shop is a different intent: {}",
        spent.body
    );

    let runs: ImportRunsView = call(&app, Method::GET, "/v1/imports/runs", None)
        .await
        .json();
    assert_eq!(
        (runs.runs.len(), runs.total),
        (3, 3),
        "the first page holds these three, past and present, and says so is all there is"
    );
    let stopped = runs
        .runs
        .iter()
        .find(|head| head.id == run)
        .expect("the stopped run is listed");
    assert_eq!(stopped.state, ImportRunState::Abandoned);
    assert_eq!(
        stopped.execution.stage,
        ImportRunStage::Abandoned,
        "and says so as its displayed stage"
    );
}

/// A run waiting for the seller to choose stays `selecting` after its
/// device's keeper ends.
///
/// The keeper ends with the worker, and the worker's work is finished at the
/// moment the enumeration closes: nothing is owed by any machine while a
/// person reads the list. So the lease lapses and the owner clears exactly
/// when the run is waiting for the seller, and before this the console read
/// that as `interrupted` — or as `waiting`, once the owner cleared — and told
/// a seller their import had died at the one moment it was waiting for them.
/// The maintenance pass is driven here too, because interrupting the row is
/// the other half of the same defect.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_run_awaiting_selection_survives_its_keeper(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("selecting"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    let list = listing_page(
        run,
        vec![
            listed("https://www.tes.com/x/1", "Fractions pack"),
            listed("https://www.tes.com/x/2", "Long division"),
        ],
    );
    assert_eq!(post_page(&app, &list).await.status, StatusCode::OK);

    let walked: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(
        walked.execution.stage,
        ImportRunStage::Selecting,
        "a closed enumeration with nothing ticked is the seller's own step"
    );

    // First half: the hold lapses while the device is still named, which is
    // what the maintenance sweep looks for. Written by statement because no
    // route leaves this state on purpose and the lease is compared against
    // the database's own clock.
    keeper_ends(&pool, run, false).await;
    drain(&state).await;

    let lapsed: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(
        lapsed.execution.stage,
        ImportRunStage::Selecting,
        "a lapsed hold does not interrupt a run nobody owes work on"
    );
    assert!(
        lapsed.execution.reason_code.is_none(),
        "and the maintenance pass wrote no lapse against it"
    );

    // Second half: the owner stops being named at all.
    keeper_ends(&pool, run, true).await;
    let after: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(
        after.execution.stage,
        ImportRunStage::Selecting,
        "and an ownerless run mid-selection is not waiting for a device"
    );
    assert_eq!(
        after.state,
        ImportRunState::Reading,
        "the run itself is untouched"
    );

    // And the seller's choice still lands, which is what the stage promised.
    let selected = call(
        &app,
        Method::POST,
        &format!("/v1/imports/runs/{}/select", uuid_text(run)),
        Some(serde_json::json!({ "all": true })),
    )
    .await;
    assert_eq!(
        selected.status,
        StatusCode::OK,
        "the selection is accepted: {}",
        selected.body
    );
}

/// Ends a run's keeper the way the device does: the hold lapses, and then the
/// owner stops being named. By statement, because no route leaves this state
/// on purpose and the lease is compared against the database's own clock.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn keeper_ends(pool: &PgPool, run: Uuid, release: bool) {
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    sqlx::query(
        "UPDATE import_run \
            SET lease_expires_at = CASE WHEN $3 THEN NULL \
                    ELSE now() - make_interval(secs => 600) END, \
                owner_device = CASE WHEN $3 THEN NULL ELSE owner_device END, \
                attempt = CASE WHEN $3 THEN 0 ELSE attempt END \
          WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(run.0))
    .bind(release)
    .execute(&mut *tx)
    .await
    .expect("the lease lapses");
    tx.commit().await.expect("the fixture commits");
}

/// A described resource whose bytes are not captured yet lands, without its
/// source binding.
///
/// Every TPT read on file is this resource: the metadata is read from the
/// shop and the file arrives later, by a supervised capture. Migration 0061
/// moved the payload requirement from the product to the mapping, so binding
/// the listing onto a payload-less product raises a check violation at
/// commit — and the commit's transaction carries the whole item, so the item
/// rolled back and every drain pass retried it forever. The resource lands
/// and the binding waits for the file.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_resource_whose_file_is_not_captured_yet_still_lands(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("uncaptured"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    // No file: the read carried metadata only.
    let described = page(
        run,
        vec![observed(
            "https://www.tes.com/teaching-resource/-77",
            "Uncaptured pack",
            None,
            None,
        )],
        true,
    );
    assert_eq!(
        post_page(&app, &described).await.status,
        StatusCode::OK,
        "the description is accepted"
    );

    let settled = confirm_and_drain(&app, &state, run).await;
    assert_eq!(
        (settled.state, settled.counts.imported),
        (ImportRunState::Complete, 1),
        "the item finished rather than being retried forever"
    );
    assert_eq!(
        products_held(&pool).await,
        1,
        "the resource is in the catalogue"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM mapping WHERE org_id = $1 AND inventory = 'tes'",
        )
        .await,
        0,
        "and no listing is bound to it, because 0061 admits no mapping without a payload"
    );
}

/// A page from a device that has been taken over changes nothing, and says so
/// as a conflict rather than as something the device should retry.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_taken_over_device_cannot_extend_the_run(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("fence"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    // The same device claims again — a resume — which raises the fence.
    let resumed = call(
        &app,
        Method::POST,
        &format!("/v1/devices/{DEVICE_A}/import/{}/claim", uuid_text(run)),
        Some(serde_json::json!({ "takeover": false })),
    )
    .await;
    assert_eq!(resumed.status, StatusCode::OK, "{}", resumed.body);
    assert_eq!(
        resumed.body["attempt"].as_u64(),
        Some(ATTEMPT + 1),
        "a reclaim is a new attempt: {}",
        resumed.body
    );

    // A page under the old attempt, which is what a device still finishing
    // its previous pass posts.
    let late = post_page(
        &app,
        &page(
            run,
            vec![observed("late-1", "Late arrival", Some((0x44, BIG)), None)],
            false,
        ),
    )
    .await;
    assert_eq!(late.status, StatusCode::CONFLICT, "{}", late.body);
    assert_eq!(
        late.body["errors"][0]["code"], "import_run_fenced",
        "the device stops on this rather than retrying it forever: {}",
        late.body
    );
    assert_eq!(
        run_view(&app, run).await.counts.read,
        0,
        "and the page wrote nothing"
    );

    // An unfenced page — an app from before the fence — is refused the same
    // way, before any effect.
    let mut old = page(run, Vec::new(), false);
    old.attempt = None;
    let refused = post_page(&app, &old).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(
        refused.body["errors"][0]["code"], "import_client_update_required",
        "{}",
        refused.body
    );
}

/// Reading the same shop twice creates nothing twice: every resource the
/// first run made is bound to the listing it came from, so the second run
/// skips those listings as they land, with the resource named, and only the
/// one the shop gained is left to tick.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_second_read_of_the_same_shop_skips_what_the_catalogue_holds(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("again"));
    let app = router(state.clone());
    seed_catalogue(&app, &state).await;
    assert_eq!(products_held(&pool).await, 3);

    let run = started_run(&app).await;
    let list = listing_page(
        run,
        vec![
            listed("https://www.tes.com/teaching-resource/-1", "Fractions pack"),
            listed(
                "https://www.tes.com/teaching-resource/-2",
                "Long division worksheet pack",
            ),
            listed("https://www.tes.com/teaching-resource/-4", "New this week"),
        ],
    );
    assert_eq!(post_page(&app, &list).await.status, StatusCode::OK);

    let view = run_view(&app, run).await;
    assert_eq!(
        (view.counts.listed, view.counts.skipped),
        (1, 2),
        "the two the catalogue holds are settled before anything is ticked"
    );
    let held = view
        .items
        .iter()
        .find(|item| item.locator.ends_with("-1"))
        .expect("the held listing is on the run");
    assert_eq!(held.state, ImportRunItemState::Skipped);
    assert_eq!(
        held.skip_reason.as_deref(),
        Some("already in Resources as Fractions pack"),
        "the seller is told which resource it already is"
    );

    // Ticking everything ticks only what is still open, and freezes the
    // denominator the reading stage is measured against.
    let selected = call(
        &app,
        Method::POST,
        &format!("/v1/imports/runs/{}/select", uuid_text(run)),
        Some(serde_json::json!({ "all": true })),
    )
    .await;
    assert_eq!(selected.status, StatusCode::OK);
    let view: ImportRunView = selected.json();
    assert_eq!((view.counts.selected, view.counts.skipped), (1, 2));
    assert_eq!(
        view.execution.selected_total,
        Some(1),
        "the frozen total counts the rows this selection actually took, not the listings \
         discovery had already settled as held"
    );
}

/// A shop with nothing left to describe finishes itself.
///
/// Every listing of it is one the catalogue already holds, so there is
/// nothing to tick, nothing to confirm and nothing to create. Before this the
/// run sat `reading` forever — no caller had any reason to send a completion
/// — and its shop stayed fenced against the seller's next import.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_shop_with_no_work_left_completes_itself(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("nowork"));
    let app = router(state.clone());
    seed_catalogue(&app, &state).await;

    let run = started_run(&app).await;
    let list = listing_page(
        run,
        vec![
            listed("https://www.tes.com/teaching-resource/-1", "Fractions pack"),
            listed("https://www.tes.com/teaching-resource/-2", "Long division"),
            listed("https://www.tes.com/teaching-resource/-3", "Shape hunt"),
        ],
    );
    assert_eq!(post_page(&app, &list).await.status, StatusCode::OK);

    let view = run_view(&app, run).await;
    assert_eq!(
        (view.state, view.execution.stage),
        (ImportRunState::Complete, ImportRunStage::Completed),
        "a walked shop with nothing outstanding is a finished import"
    );
    assert_eq!(products_held(&pool).await, 3, "and it created nothing");

    // And the shop is free again, which is what the fence was holding.
    let again = open_run(&app).await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
}

/// An ordinary successful import announces its own settlement exactly once.
///
/// The completion is written inside the transaction that commits the last
/// outstanding item, so the chunk's own conditional transition afterwards
/// finds the run already `complete` and moves nothing. Reading that as
/// "nothing settled" left every finished import with no terminal event at
/// all: the console refetches on an event of the organisation's stream and
/// runs no poller, so a seller watched a live bar over an import that had
/// finished until they reloaded the page.
///
/// The count is asserted rather than the presence, because announcing from
/// both writers is the other way for the transition and the event to
/// disagree, and a duplicate terminal event is what tells the console a
/// settled run settled twice.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn committing_the_last_item_announces_one_settlement(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("announce"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    // Three resources, distinct bytes and distinct titles, so nothing is held
    // for review and the third item's own commit is what empties the run.
    let described = page(
        run,
        vec![
            observed(
                "https://www.tes.com/teaching-resource/-1",
                "Fractions pack",
                Some((0x5A, BIG)),
                None,
            ),
            observed(
                "https://www.tes.com/teaching-resource/-2",
                "Long division",
                Some((0x5B, BIG)),
                None,
            ),
            observed(
                "https://www.tes.com/teaching-resource/-3",
                "Shape hunt",
                Some((0x5C, BIG)),
                None,
            ),
        ],
        true,
    );
    let answer = post_page(&app, &described).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

    let settled = confirm_and_drain(&app, &state, run).await;
    assert_eq!(
        (
            settled.state,
            settled.counts.imported,
            settled.counts.review
        ),
        (ImportRunState::Complete, 3, 0),
        "the last item's own commit is what completes the run"
    );
    assert_eq!(
        settled_events(&pool, run).await,
        1,
        "a completed run is announced once: not never, because the transition happened \
         inside the last item's transaction, and not twice, because only one writer moved it"
    );
}

/// A run the commit refuses every row of announces its settlement once too.
///
/// Nothing succeeded here, so no item transaction completed the run and the
/// chunk's own recount is what settles it. Both roads to `complete` are
/// asserted because the fix makes one writer answer for the announcement, and
/// a fix that moved the announcement onto the item would leave this run — the
/// one where no item finished — silent.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_run_whose_every_row_is_refused_announces_one_settlement(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("refused"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    // Tes fixes GBP, so a row read in dollars has no resolvable price and the
    // commit refuses it. The refusal is the seller's to act on, so it is
    // recorded against the row rather than failing the drain.
    let mut priced = observed(
        "https://www.tes.com/teaching-resource/-9",
        "Priced in the wrong money",
        None,
        None,
    );
    priced.listing.price = ImportedPrice::Paid {
        minor_units: 500,
        denomination: "USD".to_owned(),
    };
    let answer = post_page(&app, &page(run, vec![priced], true)).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);

    let settled = confirm_and_drain(&app, &state, run).await;
    assert_eq!(
        (
            settled.state,
            settled.counts.failed,
            settled.counts.imported
        ),
        (ImportRunState::Complete, 1, 0),
        "a run with nothing outstanding is finished, however little it created"
    );
    assert_eq!(
        settled_events(&pool, run).await,
        1,
        "and the recount that settled it is the writer that announces it"
    );
    assert_eq!(
        products_held(&pool).await,
        0,
        "the refusal created nothing, counted rather than trusted"
    );
}

/// A page replayed with changed content is refused rather than acknowledged.
///
/// The receipt is the device's key for one page, and its identity is the
/// whole of what that page would do. A hash over locators alone would
/// acknowledge a changed title, price or file while applying none of it — and
/// the device, reading the acknowledgement, would discard a description the
/// server never accepted.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_receipt_answers_only_for_the_page_it_accepted(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("receipt"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    let first = page(
        run,
        vec![observed(
            "https://www.tes.com/x/9",
            "Fractions pack",
            Some((0x61, BIG)),
            None,
        )],
        false,
    );
    assert_eq!(post_page(&app, &first).await.status, StatusCode::OK);

    // The same page again: answered from the stored acknowledgement, applying
    // nothing.
    let replay = post_page(&app, &first).await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.body);
    assert_eq!(
        replay.body["applied"], 0,
        "a replay is answered rather than applied: {}",
        replay.body
    );

    // The same receipt, a different description.
    let mut changed = first.clone();
    changed.resources = vec![observed(
        "https://www.tes.com/x/9",
        "Fractions pack, second edition",
        Some((0x61, BIG)),
        None,
    )];
    let refused = post_page(&app, &changed).await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.body);
    assert_eq!(
        refused.body["errors"][0]["code"], "import_receipt_conflict",
        "{}",
        refused.body
    );
    let view = run_view(&app, run).await;
    assert_eq!(
        view.items
            .iter()
            .find(|item| item.locator.ends_with("/9"))
            .and_then(|item| item.title.clone())
            .as_deref(),
        Some("Fractions pack"),
        "and the description the server accepted is the one it kept"
    );
}

/// A resource another source committed after this run matched is not created
/// twice, and the seller is asked rather than merged behind.
///
/// The matcher is re-asked under the organisation's catalogue lock at the
/// moment of creation. Two runs that both matched against an empty catalogue
/// used to create two products: the second's own decision was already
/// "new", and a digest comparison could not see a pair that agrees on text
/// and title over different bytes.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_commit_re_asks_the_matcher_against_what_is_committed(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("reval"));
    let app = router(state.clone());

    // Two runs, one per shop, both describing the same resource: the same
    // text and the same title, different bytes. Both finish matching against
    // a catalogue that holds neither.
    let tes = started_run_on(&app, "Tes").await;
    let tpt = started_run_on(&app, "Tpt").await;
    let sketched = sketch(0x0F0F_0F0F_0F0F_0F0F, 128, 7);
    assert_eq!(
        post_page(
            &app,
            &page(
                tes,
                vec![observed(
                    "https://www.tes.com/x/11",
                    "Long division worksheets pack",
                    Some((0x71, BIG)),
                    Some(sketched.clone()),
                )],
                true,
            )
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        post_page(
            &app,
            &page(
                tpt,
                vec![observed_on(
                    Marketplace::Tpt,
                    "12",
                    "Long division worksheets pack",
                    Some((0x72, BIG)),
                    Some(sketched),
                )],
                true,
            )
        )
        .await
        .status,
        StatusCode::OK
    );

    // The first confirmation creates its resource.
    let first = confirm_and_drain(&app, &state, tes).await;
    assert_eq!(
        (first.state, first.counts.imported),
        (ImportRunState::Complete, 1)
    );
    assert_eq!(products_held(&pool).await, 1);

    // The second is revalidated against that product before it creates
    // anything, and the pair it now finds is a question rather than a second
    // resource.
    let second = confirm_and_drain(&app, &state, tpt).await;
    assert_eq!(
        products_held(&pool).await,
        1,
        "the second source created nothing behind the first"
    );
    assert_eq!(
        second.counts.review, 1,
        "and the seller is asked: {:?}",
        second.counts
    );
    let open: DuplicatesView = call(&app, Method::GET, "/v1/duplicates", None).await.json();
    assert_eq!(
        open.pairs.len(),
        1,
        "with the question stored rather than implied"
    );
}

/// A read the matcher merged away still binds its listing to the survivor.
///
/// The skip settles the row, and a settled row never reaches the commit — so
/// without writing the binding here the catalogue showed the shop's label on
/// a product with no listing from that shop, and the next import of it read
/// the same listing again as though it were new.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_merged_read_binds_its_listing_to_the_survivor(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("bindmerge"));
    let app = router(state.clone());
    seed_catalogue(&app, &state).await;

    // The other shop, byte for byte the first run's first resource: decisive,
    // so the system merges it without asking.
    let run = started_run_on(&app, "Tpt").await;
    assert_eq!(
        post_page(
            &app,
            &page(
                run,
                vec![observed_on(
                    Marketplace::Tpt,
                    "31",
                    "Fractions pack",
                    Some((0x5A, BIG)),
                    None,
                )],
                true,
            )
        )
        .await
        .status,
        StatusCode::OK
    );
    let view = run_view(&app, run).await;
    assert_eq!(
        view.items.first().map(|item| item.state),
        Some(ImportRunItemState::Skipped),
        "an exact, large, rare file is the same resource"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM mapping WHERE org_id = $1 AND inventory = 'tpt' \
             AND binding_state = 'bound'"
        )
        .await,
        1,
        "and the survivor carries the listing it was read from, not just its label"
    );
}

/// The device's queued stop: fenced, idempotent, and answered with a body it
/// can tell apart from a gateway's 200.
///
/// Three failures this holds shut. A stop carrying an attempt that is no
/// longer this device's must settle nothing — a phone draining its queue
/// after a takeover would otherwise stop the run the seller is watching on
/// another machine. A replayed stop must not turn into a conflict the device
/// treats as a refusal, or the instruction is queued forever. And the success
/// body must say `abandoned`, because that field is the device's only proof
/// that this server answered rather than something in between.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_stop_is_fenced_idempotent_and_answered(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("stop"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    let stop = format!("/v1/devices/{DEVICE_A}/import/{}/stop", uuid_text(run));

    let stale = call(
        &app,
        Method::POST,
        &stop,
        Some(serde_json::json!({ "attempt": ATTEMPT + 1 })),
    )
    .await;
    assert_eq!(
        stale.status,
        StatusCode::CONFLICT,
        "an attempt this device does not hold stops nothing: {}",
        stale.body
    );
    assert_eq!(
        stale.body["errors"][0]["code"], "import_run_fenced",
        "and the code is the structured one the device matches on"
    );
    let standing: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(
        standing.state,
        ImportRunState::Reading,
        "the run the refused stop named is still open"
    );

    let stopped = call(
        &app,
        Method::POST,
        &stop,
        Some(serde_json::json!({ "attempt": ATTEMPT })),
    )
    .await;
    assert_eq!(
        stopped.status,
        StatusCode::OK,
        "the owner's own stop is accepted: {}",
        stopped.body
    );
    assert_eq!(
        stopped.body,
        serde_json::json!({ "abandoned": true }),
        "and the body is exactly what the device validates"
    );

    let replayed = call(
        &app,
        Method::POST,
        &stop,
        Some(serde_json::json!({ "attempt": ATTEMPT })),
    )
    .await;
    assert_eq!(
        (replayed.status, replayed.body),
        (StatusCode::OK, serde_json::json!({ "abandoned": true })),
        "a replay of a delivered stop answers the same thing rather than a refusal"
    );

    let settled: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(settled.state, ImportRunState::Abandoned);
    assert_eq!(
        products_held(&pool).await,
        0,
        "and a stop creates nothing, counted rather than trusted"
    );
}

/// The history is a page of ten, and the eleventh import is reachable.
///
/// The listing used to answer up to fifty runs with no cursor, no filter and
/// no total, which made a seller's older imports reachable only by scrolling
/// and their fifty-first not at all. The property under test is that the
/// page is a window on the whole history rather than a truncation of it: the
/// total counts every match, the offset reaches past the first page, and a
/// state filter is applied in SQL rather than to the page.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_history_pages_past_the_tenth_run_and_filters_the_whole_of_it(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("history"));
    let app = router(state.clone());

    // Eleven imports, each settled before the next opens: one run per shop is
    // the fence, so a history of eleven is eleven abandoned runs.
    //
    // Every run is aged to a distinct minute afterwards. The harness clock is
    // a constant, so without this all eleven share one `created_at` and the
    // order falls to the tie-break over identifiers nobody chose — which
    // would make "the oldest import" a claim about random UUIDs rather than
    // about the order the seller made them in.
    let mut opened = Vec::new();
    for minute in 0..11 {
        let answer = open_run(&app).await;
        assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.body);
        let run: ImportRunView = answer.json();
        opened.push(run.id);
        let abandoned = call(
            &app,
            Method::POST,
            &format!("/v1/imports/runs/{}/abandon", uuid_text(run.id)),
            None,
        )
        .await;
        assert_eq!(abandoned.status, StatusCode::NO_CONTENT);
        // The first import opened is the furthest back, so "newest first"
        // ends at it and "oldest first" begins there.
        aged(&pool, run.id, 11 - minute).await;
    }

    let first: ImportRunsView = call(&app, Method::GET, "/v1/imports/runs", None)
        .await
        .json();
    assert_eq!(first.runs.len(), 10, "the page is ten");
    assert_eq!(
        first.total, 11,
        "and it says how many there are, rather than how many it sent"
    );

    let second: ImportRunsView = call(&app, Method::GET, "/v1/imports/runs?offset=10", None)
        .await
        .json();
    assert_eq!(
        second.runs.len(),
        1,
        "the eleventh run is reachable: {:?}",
        second.runs.len()
    );
    assert_eq!(
        second.runs[0].id, opened[0],
        "newest first, so the oldest import is the one past the first page"
    );

    let oldest: ImportRunsView = call(&app, Method::GET, "/v1/imports/runs?order=oldest", None)
        .await
        .json();
    assert_eq!(
        oldest.runs[0].id, opened[0],
        "oldest first turns the history round rather than filtering it"
    );
    assert_eq!(oldest.total, 11, "and counts the same history");

    let settled: ImportRunsView = call(
        &app,
        Method::GET,
        "/v1/imports/runs?state=abandoned&limit=50",
        None,
    )
    .await
    .json();
    assert_eq!(
        (settled.runs.len(), settled.total),
        (11, 11),
        "the filter matches over the whole history, not over one page"
    );

    let open: ImportRunsView = call(&app, Method::GET, "/v1/imports/runs?state=open", None)
        .await
        .json();
    assert_eq!(
        (open.runs.len(), open.total),
        (0, 0),
        "and nothing is open once every run has been given up"
    );

    let refused = call(&app, Method::GET, "/v1/imports/runs?state=nonsense", None).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unrecognised filter is refused rather than ignored: {}",
        refused.body
    );
}

/// A run's resources page at twenty-five, search covers the whole run, and a
/// selection made across two pages imports exactly what was ticked.
///
/// The last of those is the defect this paging could have shipped: the
/// console used to send `{all: true}` whenever the ticked count equalled the
/// listed count, which with a page of twenty-five would have turned "these
/// two" into "the whole shop". The wire asserts it here — two locators in,
/// two selected, the rest skipped — so the shortcut cannot come back without
/// this failing.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn items_page_search_the_whole_run_and_a_selection_spans_pages(pool: PgPool) {
    provision(&pool).await;
    let state = configured(pool.clone(), &store_root("itempages"));
    let app = router(state.clone());
    let run = started_run(&app).await;

    let rows: Vec<_> = (1..=26)
        .map(|n| listed(&format!("res-{n:02}"), &format!("Resource {n:02}")))
        .collect();
    assert_eq!(
        post_page(&app, &listing_page(run, rows)).await.status,
        StatusCode::OK
    );

    let first = run_view(&app, run).await;
    assert_eq!(first.items.len(), 25, "a page of twenty-five");
    assert_eq!(
        (first.items_total, first.items_limit, first.items_offset),
        (26, 25, 0),
        "and the window says what it is a window on"
    );
    assert_eq!(
        first.read_total,
        Some(26),
        "the whole-run total is unaffected by the page"
    );
    assert_eq!(
        first.items[0].title.as_deref(),
        Some("Resource 01"),
        "title A-Z by default"
    );

    let second: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}?offset=25", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(
        second.items.len(),
        1,
        "the twenty-sixth resource is reachable"
    );
    assert_eq!(second.items[0].title.as_deref(), Some("Resource 26"));

    let found: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}?q=resource%2026", uuid_text(run)),
        None,
    )
    .await
    .json();
    assert_eq!(
        (found.items.len(), found.items_total),
        (1, 1),
        "the search reads the whole run, not the page in hand"
    );
    assert_eq!(found.items[0].locator.as_str(), "res-26");

    // One from the first page and one from the second: exactly what a seller
    // who paged and ticked sends.
    let selected = call(
        &app,
        Method::POST,
        &format!("/v1/imports/runs/{}/select", uuid_text(run)),
        Some(serde_json::json!({ "locators": ["res-02", "res-26"] })),
    )
    .await;
    assert_eq!(selected.status, StatusCode::OK, "{}", selected.body);
    let view: ImportRunView = selected.json();
    assert_eq!(
        (view.counts.selected, view.counts.skipped),
        (2, 24),
        "exactly the two ticked, and the shop is not imported wholesale"
    );

    let selection: serde_json::Value = call(
        &app,
        Method::GET,
        &format!("/v1/devices/{DEVICE_A}/import/{}/selection", uuid_text(run)),
        None,
    )
    .await
    .body;
    assert_eq!(
        selection["locators"],
        serde_json::json!(["res-02", "res-26"]),
        "and the device is handed those two, in the shop's own order"
    );
}

// ----------------------------------------------------------- fixture clock

/// Moves one run's `created_at` back by whole minutes, so a fixture of runs
/// made inside one constant-clock test has a real order to be listed in.
///
/// Written directly rather than through a route, because no route exists to
/// backdate a run and none should: this is the fixture's own clock, not a
/// capability of the product.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn aged(pool: &PgPool, run: Uuid, minutes: i32) {
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    sqlx::query(
        "UPDATE import_run SET created_at = created_at - make_interval(mins => $1) \
          WHERE org_id = $2 AND id = $3",
    )
    .bind(minutes)
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(run.0))
    .execute(&mut *tx)
    .await
    .expect("the backdate runs");
    tx.commit().await.expect("the backdate commits");
}

// ------------------------------------------------------------------ counted

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn counted(pool: &PgPool, sql: &str) -> i64 {
    // Under a tenant pin, because `product` carries forced row-level security:
    // an unpinned count matches no rows and would agree with itself while
    // asserting nothing.
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let held: i64 = sqlx::query_scalar(sql)
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("the count runs");
    tx.commit().await.expect("the read commits");
    held
}

async fn products_held(pool: &PgPool) -> i64 {
    counted(pool, "SELECT count(*) FROM product WHERE org_id = $1").await
}

async fn live_products(pool: &PgPool) -> i64 {
    counted(
        pool,
        "SELECT count(*) FROM product WHERE org_id = $1 AND deleted_at IS NULL",
    )
    .await
}

/// How many terminal events this run has in the ledger.
///
/// Counted from `job_event` rather than read off the view, because the view
/// answers what the row says and this is about what the console's stream is
/// told: a settled run whose event never landed reads `complete` on a refetch
/// nobody had a reason to make.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn settled_events(pool: &PgPool, run: Uuid) -> i64 {
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_event \
          WHERE org_id = $1 AND kind = 'ImportRunSettled' AND payload->>'run' = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid_text(run))
    .fetch_one(&mut *tx)
    .await
    .expect("the count runs");
    tx.commit().await.expect("the read commits");
    events
}
