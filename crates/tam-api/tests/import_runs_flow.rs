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
    ImportRunItemState, ImportRunState, ImportRunView, ImportRunsView, RunCommitAck,
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
    ContentHash, CopyFormat, FileKind, ImportedPrice, OrgId, ProductId, ScanOutcome, Timestamp,
    UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xA1; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN_A: SessionToken = SessionToken([0x51; 32]);
const NOW: Timestamp = Timestamp(5_000);
const DEVICE_A: &str = "11112222333344445555666677778888";
const CURRENT: &str = "0.2.0";

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
        blobs: Some(BlobStore {
            kek: tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
            root: root.to_path_buf(),
        }),
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

async fn open_run(app: &axum::Router) -> Answer {
    call(
        app,
        Method::POST,
        "/v1/imports/runs",
        Some(serde_json::json!({ "source": "Tes" })),
    )
    .await
}

async fn started_run(app: &axum::Router) -> Uuid {
    let answer = open_run(app).await;
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

async fn commit(app: &axum::Router, run: Uuid) -> Answer {
    call(
        app,
        Method::POST,
        &format!("/v1/imports/runs/{}/commit", uuid_text(run)),
        None,
    )
    .await
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
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn observed(
    locator: &str,
    title: &str,
    digest: Option<(u8, u64)>,
    text: Option<TextSketch>,
) -> ObservedResource {
    ObservedResource {
        locator: Locator::new(locator).expect("a bounded locator"),
        listing: ImportedListing {
            remote: RemoteListingId::Tes {
                url: locator.to_owned(),
            },
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

fn page(run: Uuid, resources: Vec<ObservedResource>, complete: bool) -> ImportPage {
    ImportPage {
        run,
        request: None,
        listed: None,
        resources,
        skipped: Vec::new(),
        complete,
        failed: None,
    }
}

/// Reads one shop of three resources into the catalogue, whole: list, select
/// all, describe, commit. The catalogue this leaves behind is what the later
/// runs are matched against.
async fn seed_catalogue(app: &axum::Router) -> ImportRunView {
    let run = started_run(app).await;
    let list = ImportPage {
        run,
        request: None,
        listed: Some(vec![
            listed("https://www.tes.com/teaching-resource/-1", "Fractions pack"),
            listed("https://www.tes.com/teaching-resource/-2", "Long division"),
            listed("https://www.tes.com/teaching-resource/-3", "Shape hunt"),
        ]),
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    let answer = post_page(app, &list).await;
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
        ImportRunState::Committing,
        "an empty shop of questions goes straight to the commit"
    );
    let ack = commit(app, run).await;
    assert_eq!(ack.status, StatusCode::OK, "{}", ack.body);
    let ack: RunCommitAck = ack.json();
    assert_eq!(
        (ack.applied, ack.complete, ack.run_state),
        (3, true, ImportRunState::Complete),
        "three resources, one chunk, and the run settles"
    );
    run_view(app, run).await
}

// -------------------------------------------------------------------- tests

/// The whole shape, in one pass: what the seller sees at every stage and what
/// the catalogue holds at the end.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_run_lists_selects_matches_reviews_and_commits(pool: PgPool) {
    provision(&pool).await;
    let app = router(configured(pool.clone(), &store_root("whole")));

    let seeded = seed_catalogue(&app).await;
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

    // ---- the second run: one byte-identical, one similar, one new.
    let run = started_run(&app).await;
    let list = ImportPage {
        run,
        request: None,
        listed: Some(vec![
            listed(
                "https://www.tes.com/teaching-resource/-11",
                "Fractions pack",
            ),
            listed(
                "https://www.tes.com/teaching-resource/-12",
                "Long division worksheets pack",
            ),
            listed("https://www.tes.com/teaching-resource/-13", "Number bonds"),
            listed("https://www.tes.com/teaching-resource/-14", "Left behind"),
        ]),
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
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
            "locators": [
                "https://www.tes.com/teaching-resource/-11",
                "https://www.tes.com/teaching-resource/-12",
                "https://www.tes.com/teaching-resource/-13",
            ]
        })),
    )
    .await;
    assert_eq!(selected.status, StatusCode::OK, "{}", selected.body);
    let view: ImportRunView = selected.json();
    assert_eq!((view.counts.selected, view.counts.skipped), (3, 1));
    let left = view
        .items
        .iter()
        .find(|item| item.locator.ends_with("-14"))
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
            observed(
                "https://www.tes.com/teaching-resource/-11",
                "Fractions pack",
                Some((0x5A, BIG)),
                None,
            ),
            // The same text at ninety of a hundred and twenty-eight positions
            // and nearly the same title: two moderate signals, which is the
            // ask-the-seller band.
            observed(
                "https://www.tes.com/teaching-resource/-12",
                "Long division worksheets pack",
                Some((0x7B, BIG)),
                Some(sketch(0x0F0F_0F0F_0F0F_0F0F, 90, 2)),
            ),
            // Nothing in common with anything.
            observed(
                "https://www.tes.com/teaching-resource/-13",
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
            .find(|item| item.locator.ends_with(suffix))
            .map_or(ImportRunItemState::Failed, |item| item.state)
    };
    assert_eq!(
        by("-11"),
        ImportRunItemState::Skipped,
        "an exact, large, rare file is the same resource and is not imported twice"
    );
    assert_eq!(
        by("-12"),
        ImportRunItemState::Review,
        "two moderate signals are a question, not an answer"
    );
    assert_eq!(
        by("-13"),
        ImportRunItemState::Matched,
        "silence is not a merge"
    );

    let merged = view
        .items
        .iter()
        .find(|item| item.locator.ends_with("-11"))
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

    // ---- the commit takes the matched item and leaves the question alone.
    let ack: RunCommitAck = commit(&app, run).await.json();
    assert_eq!(
        (ack.applied, ack.complete, ack.run_state),
        (1, false, ImportRunState::Committing),
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

    let ack: RunCommitAck = commit(&app, run).await.json();
    assert_eq!(
        (ack.applied, ack.complete, ack.run_state),
        (1, true, ImportRunState::Complete),
        "answering the question is what lets the run finish"
    );
    assert_eq!(products_held(&pool).await, 5);

    let view = run_view(&app, run).await;
    assert_eq!(view.state, ImportRunState::Complete);
    assert!(
        view.review_pairs.is_empty(),
        "an answered pair leaves the queue"
    );

    // ---- and the answer is remembered: the same pair is never asked again.
    let again = started_run(&app).await;
    let relisted = ImportPage {
        run: again,
        request: None,
        listed: Some(vec![listed(
            "https://www.tes.com/teaching-resource/-22",
            "Long division worksheets pack",
        )]),
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    assert_eq!(post_page(&app, &relisted).await.status, StatusCode::OK);
    let open: DuplicatesView = call(&app, Method::GET, "/v1/duplicates", None).await.json();
    assert!(
        open.pairs.is_empty(),
        "the pair the seller called different is not re-raised"
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
    let app = router(configured(pool.clone(), &store_root("common")));

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
    let ack: RunCommitAck = commit(&app, run).await.json();
    assert_eq!(ack.applied, 3, "nothing merged them on the way in");
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
    let app = router(configured(pool.clone(), &store_root("merge")));
    let seeded = seed_catalogue(&app).await;
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
    let app = router(configured(pool.clone(), &store_root("labels")));
    let seeded = seed_catalogue(&app).await;
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

/// One import at a time, and the refusal names the one that is open.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_second_run_is_refused_and_names_the_open_one(pool: PgPool) {
    provision(&pool).await;
    let app = router(configured(pool.clone(), &store_root("open")));
    let run = started_run(&app).await;

    let second = open_run(&app).await;
    assert_eq!(second.status, StatusCode::CONFLICT);
    assert_eq!(
        second.body["errors"][0]["code"], "import_run_open",
        "the code is the one the console branches on: {}",
        second.body
    );
    assert_eq!(
        second.body["errors"][0]["detail"]["run"],
        serde_json::Value::String(uuid_text(run)),
        "and it names the import already in progress"
    );

    // Stopping it is what makes room for the next one.
    let abandoned = call(
        &app,
        Method::POST,
        &format!("/v1/imports/runs/{}/abandon", uuid_text(run)),
        None,
    )
    .await;
    assert_eq!(abandoned.status, StatusCode::NO_CONTENT);
    assert_eq!(open_run(&app).await.status, StatusCode::CREATED);

    let runs: ImportRunsView = call(&app, Method::GET, "/v1/imports/runs", None)
        .await
        .json();
    assert_eq!(
        runs.runs.len(),
        2,
        "the listing holds both, past and present"
    );
    let stopped = runs
        .runs
        .iter()
        .find(|head| head.id == run)
        .expect("the stopped run is listed");
    assert_eq!(stopped.state, ImportRunState::Abandoned);
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
