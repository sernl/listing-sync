//! Both clocks, driven end to end: a schedule firing a label at a minute, and
//! a sync setting pulling a shop, committing what the device read, and
//! publishing it on.
//!
//! The property every test here exists to hold is that a pass is idempotent
//! on the *scheduled* instant rather than on its own: the loop runs every
//! sixty seconds, so every write it makes is made again a minute later unless
//! something stops it, and the things that stop it are `schedule_run`'s
//! primary key, `schedule.last_run_at`, `marketplace_sync_setting.last_pull_at`
//! and `auto_publish_run`'s primary key. Each test runs the pass twice and
//! asserts the second one changed nothing.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::import_runs::{ImportRunState, ImportRunView, OpenRunView};
use tam_api::schedules::{ScheduleRunsView, ScheduleView, SchedulesView};
use tam_api::sync_activity::ActivityView;
use tam_api::sync_settings::SyncSettingView;
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_engine_driver::import::{
    ContentType, Cover, FileName, ImportPage, ListedResource, Locator, ObservedFile,
    ObservedResource,
};
use tam_fingerprint::Fingerprint;
use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
use tam_storage::{
    DeviceRegistration, DeviceRepo, LabelRepo, ProductRepo, SessionRepo, SessionToken,
};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, ImportedPrice, ListingCopy,
    OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome, Timestamp, Title, UserId,
    Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xA1; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x51; 32]);
const DEVICE: &str = "11112222333344445555666677778888";
const CURRENT: &str = "0.9.0";

/// 2026-09-11, a Friday, at noon UTC. Named as an instant rather than derived
/// from a clock because the whole subject is "which minute has passed": a
/// fixture that read the real time would fire a nine-o'clock schedule on some
/// runs and not on others.
const NOW: Timestamp = Timestamp(1_789_128_000_000);

/// Nine in the morning, which at noon has passed today.
const NINE: u16 = 9 * 60;

/// The two labelled resources.
const FRACTIONS: u8 = 0x01;
const DECIMALS: u8 = 0x02;

/// The label the schedule selects by.
const READY: &str = "Ready";

/// Comfortably past the matcher's fifty-kilobyte floor.
const BIG: u64 = 120_000;

/// Subscriber's own floor, which is also the shortest interval the control
/// offers.
const SIX_HOURS: u32 = 6 * 3_600;

// ------------------------------------------------------------------ harness

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("schedules-{name}-{}", std::process::id()));
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
async fn provision(pool: &PgPool, plan: Option<tam_limits::Plan>) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .bind("a test org")
        .execute(pool)
        .await
        .expect("the org seeds");
    consented(pool, ORG).await;
    if let Some(plan) = plan {
        tam_storage::EntitlementRepo::new(pool.clone())
            .grant(
                ORG,
                &tam_storage::NewGrant {
                    id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                    plan,
                    rung: None,
                    granted_by: tam_storage::GrantedBy::Stripe,
                    grantor_user: None,
                    reason: None,
                    source_ref: Some("sub_a1"),
                    granted_at: Timestamp(1_000),
                    expires_at: None,
                },
            )
            .await
            .expect("the fixture grant seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG, USER, "a1@example.test", NOW)
        .await
        .expect("the user provisions");
    sessions
        .mint(
            &TOKEN,
            USER,
            Timestamp(NOW.0 + 86_400_000),
            Timestamp(1_000),
        )
        .await
        .expect("the session mints");
    DeviceRepo::new(pool.clone())
        .register(
            ORG,
            &DeviceRegistration {
                id: DEVICE,
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
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', $3, $3), ($1, $4, 'tpt', 'linked', $3, $3)",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes([0xC1; 16]))
    .bind(sqlx::types::chrono::DateTime::from_timestamp_millis(NOW.0).expect("a valid instant"))
    .bind(uuid::Uuid::from_bytes([0xC2; 16]))
    .execute(&mut *tx)
    .await
    .expect("the connection seeds");
    tx.commit().await.expect("the fixture commits");
}

/// Two resources with files, both carrying the label the schedule selects by.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn labelled_catalogue(pool: &PgPool) {
    let products = ProductRepo::new(pool.clone());
    let labels = LabelRepo::new(pool.clone());
    for tag in [FRACTIONS, DECIMALS] {
        products
            .insert(ORG, &product(tag), Timestamp(1_000))
            .await
            .expect("the product inserts");
        labels
            .set_for_product(
                ORG,
                ProductId(Uuid([tag; 16])),
                &[READY.to_owned()],
                Timestamp(1_000),
            )
            .await
            .expect("the label attaches");
    }
}

fn product(tag: u8) -> tam_domain::CanonicalProduct {
    tam_domain::CanonicalProduct {
        id: ProductId(Uuid([tag; 16])),
        org: ORG,
        title: Title(format!("Fixture {tag}")),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([tag.wrapping_add(0x60); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Held {
                    hash: ContentHash([tag; 32]),
                    byte_len: 4,
                    scan: ScanOutcome::Clean { at: NOW },
                },
            },
            vec![],
        )),
        cover: None,
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
    }
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
            format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
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

fn daily_body() -> serde_json::Value {
    serde_json::json!({
        "name": "Friday drop",
        "selection": { "label": READY },
        "inventories": ["Tpt"],
        "intent": "draft",
        "at_minute_of_day": NINE,
        "timezone": "UTC",
        "repeat": "daily",
        "republish_on_update": false,
        "enabled": true,
    })
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn counted(pool: &PgPool, sql: &str) -> i64 {
    // Under a tenant pin, because every table here carries forced row-level
    // security: an unpinned count matches no rows and would agree with itself
    // while asserting nothing.
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let held: i64 = sqlx::query_scalar(sql)
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("the count runs");
    tx.commit().await.expect("the read commits");
    held
}

// -------------------------------------------------------------------- tests

/// A daily schedule whose minute has passed sends its label once, and the
/// second pass of the same minute sends nothing.
///
/// The two halves are the feature and the thing that makes it safe to run
/// every sixty seconds. The first asserts the shape: one job on the named
/// marketplace carrying one create per member, a `schedule_run` row per
/// member, and a runs list that says two were sent. The second asserts that
/// the tick is the idempotency -- the pass runs again at the same instant and
/// the job count, the item count and the row count are all unchanged.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_daily_schedule_sends_its_label_once_per_tick(pool: PgPool) {
    provision(&pool, Some(tam_limits::Plan::Subscriber)).await;
    labelled_catalogue(&pool).await;
    let root = store_root("tick");
    let state = configured(pool.clone(), &root);
    let app = router(state.clone());

    let created = call(&app, Method::POST, "/v1/schedules", Some(daily_body())).await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "the schedule saves: {}",
        created.body
    );
    let view: ScheduleView = created.json();
    assert_eq!(
        view.next_run_at,
        Some(Timestamp(NOW.0 + 21 * 60 * 60 * 1_000)),
        "nine tomorrow, because today's nine is already behind us"
    );
    assert_eq!(view.last_run_at, None, "nothing has fired yet");

    let report = tam_api::scheduler::pass(&state, NOW)
        .await
        .expect("the pass runs");
    assert_eq!(report.ticks, 1, "one marketplace, one tick");
    assert_eq!(report.jobs, 1, "and one job for it");
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM job WHERE org_id = $1 AND inventory = 'tpt'"
        )
        .await,
        1,
        "one job on the marketplace the schedule named"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM job_item i JOIN job j ON j.org_id = i.org_id \
             AND j.id = i.job_id WHERE i.org_id = $1 AND i.operation = 'create'"
        )
        .await,
        2,
        "a draft intent on two unbound mappings is two creates and no publishes"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM schedule_run WHERE org_id = $1 AND state = 'sent'"
        )
        .await,
        2,
        "one row per member, which is the idempotency key"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM mapping WHERE org_id = $1 AND inventory = 'tpt'"
        )
        .await,
        2,
        "a member with no mapping on the target gets the one a seller ticking \
         the marketplace by hand would have got"
    );

    let runs: ScheduleRunsView = call(
        &app,
        Method::GET,
        &format!("/v1/schedules/{}/runs", view.id.to_hyphenated()),
        None,
    )
    .await
    .json();
    assert_eq!(runs.runs.len(), 1, "one tick, one marketplace, one line");
    assert_eq!(runs.runs[0].sent, 2);
    assert!(
        runs.runs[0].skipped.is_empty(),
        "nothing was refused: {:?}",
        runs.runs[0].skipped
    );
    assert!(runs.runs[0].job.is_some(), "the line links to its job");

    let listed: SchedulesView = call(&app, Method::GET, "/v1/schedules", None).await.json();
    assert_eq!(
        listed.schedules.first().and_then(|row| row.last_run_at),
        Some(Timestamp(NOW.0 - 3 * 60 * 60 * 1_000)),
        "the tick recorded is nine o'clock, not the instant the pass ran"
    );

    // The same minute again. The loop does this every sixty seconds, so
    // anything that is not fenced here reaches the seller's shop twice.
    let again = tam_api::scheduler::pass(&state, NOW)
        .await
        .expect("the second pass runs");
    assert_eq!(again.ticks, 0, "the tick was already recorded");
    assert_eq!(again.jobs, 0);
    assert_eq!(
        counted(&pool, "SELECT count(*) FROM job WHERE org_id = $1").await,
        1,
        "no second job"
    );
    assert_eq!(
        counted(&pool, "SELECT count(*) FROM job_item WHERE org_id = $1").await,
        2,
        "and no second pair of creates"
    );
    assert_eq!(
        counted(&pool, "SELECT count(*) FROM schedule_run WHERE org_id = $1").await,
        2,
    );

    let activity: ActivityView = call(&app, Method::GET, "/v1/sync/activity", None)
        .await
        .json();
    assert_eq!(
        activity.activity.first().map(|line| line.line.as_str()),
        Some("Schedule \"Friday drop\" sent 2 resources to TPT"),
        "the log says what the seller would say"
    );
}

/// A plan without the timetable is refused the capability, not the count.
///
/// The two send the console to different places: one is an upgrade and the
/// other is a limit to work within, so the refusal names the `Capabilities`
/// field rather than only carrying a sentence.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_free_plan_is_refused_the_timetable(pool: PgPool) {
    provision(&pool, None).await;
    let root = store_root("free");
    let app = router(configured(pool.clone(), &root));

    let refused = call(&app, Method::POST, "/v1/schedules", Some(daily_body())).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused.body["errors"][0]["detail"]["feature"],
        serde_json::json!("scheduling"),
        "the refusal names the capability: {}",
        refused.body
    );
    let listing = call(&app, Method::GET, "/v1/schedules", None).await;
    assert_eq!(
        listing.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "and the read is gated too, so the page cannot render a list it may not have"
    );
}

/// The inbound clock, whole: a setting becomes due, the pass opens a run the
/// device finds, the list is ticked without a seller, the description
/// completes, the next pass commits it, and the seller's own rule publishes
/// the result on.
///
/// One test rather than six because the steps are only meaningful in
/// sequence: a run nobody selects is never described, and a commit that never
/// happened has nothing for a rule to publish. The assertions at each step
/// are the ones a plausible bug would break -- the `scheduled` flag, the two
/// bools the device branches on, and the idempotency of the second pass.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_due_sync_setting_pulls_commits_and_publishes(pool: PgPool) {
    provision(&pool, Some(tam_limits::Plan::Subscriber)).await;
    let root = store_root("pull");
    let state = configured(pool.clone(), &root);
    let app = router(state.clone());

    // Tes as the source, because it is the one marketplace with a captured
    // seller download; TPT as the rule's target.
    let saved = call(
        &app,
        Method::PUT,
        "/v1/sync/settings/Tes",
        Some(serde_json::json!({
            "enabled": true,
            "interval_secs": SIX_HOURS,
            "publish_to": ["Tpt"],
        })),
    )
    .await;
    assert_eq!(
        saved.status,
        StatusCode::OK,
        "the setting saves: {}",
        saved.body
    );
    let setting: SyncSettingView = saved.json();
    assert_eq!(setting.interval_secs, SIX_HOURS);
    assert_eq!(
        setting.minimum_secs,
        Some(SIX_HOURS),
        "the plan's floor travels with the row, so the control can disable what is below it"
    );
    assert_eq!(setting.publish_to, vec![tam_types::InventoryId::Tpt]);
    assert_eq!(setting.last_pull_at, None, "saving a cadence is not a read");

    // A shop that has never been read is due at once, which is what a seller
    // switching the toggle on expects.
    let report = tam_api::scheduler::pass(&state, NOW)
        .await
        .expect("the pass runs");
    assert_eq!(report.pulls, 1, "one shop was due: {:?}", report.failures);
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM import_run WHERE org_id = $1 AND scheduled = true \
             AND source = 'tes' AND state = 'reading'"
        )
        .await,
        1,
        "the run is flagged as the pass's own"
    );

    // The device asks at check-in and is told to enumerate. A list rather
    // than a nullable singleton: one open run per shop means the device is
    // handed every run this organisation has open and picks what it can serve.
    let open: Vec<OpenRunView> = call(
        &app,
        Method::GET,
        &format!("/v1/devices/{DEVICE}/import/open"),
        None,
    )
    .await
    .json();
    let open = open.into_iter().next().expect("a run is open");
    assert_eq!(open.source, tam_types::InventoryId::Tes);
    assert!(!open.listed, "the shop has not been enumerated yet");
    assert!(!open.selected, "so nothing has been ticked either");
    assert!(open.scheduled, "and the pass is what opened it");

    // The device claims it before it reads anything: the fence every page is
    // posted under, and the reason a report of "no session on this phone" has
    // a run to be recorded against.
    let claimed = call(
        &app,
        Method::POST,
        &format!(
            "/v1/devices/{DEVICE}/import/{}/claim",
            open.run.to_hyphenated()
        ),
        Some(serde_json::json!({ "takeover": false })),
    )
    .await;
    assert_eq!(claimed.status, StatusCode::OK, "{}", claimed.body);
    let attempt = claimed.body["attempt"].as_u64().unwrap_or_default();

    // The list lands, whole. Nobody is at the keyboard, so the server ticks
    // it once the enumeration is closed.
    let posted = call(
        &app,
        Method::POST,
        &format!("/v1/devices/{DEVICE}/import"),
        Some(
            serde_json::to_value(ImportPage {
                run: open.run,
                request: None,
                attempt: Some(attempt),
                receipt: Some(tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes())),
                enumeration_complete: true,
                listed: Some(vec![listed("https://www.tes.com/x/1", "Fractions Pack")]),
                resources: Vec::new(),
                skipped: Vec::new(),
                complete: false,
                failed: None,
            })
            .expect("the page serialises"),
        ),
    )
    .await;
    assert_eq!(posted.status, StatusCode::OK, "{}", posted.body);
    let after: Vec<OpenRunView> = call(
        &app,
        Method::GET,
        &format!("/v1/devices/{DEVICE}/import/open"),
        None,
    )
    .await
    .json();
    let after = after.into_iter().next().expect("the run is still open");
    assert!(
        after.listed,
        "the enumeration is closed, so it is not run twice"
    );
    assert!(
        after.selected,
        "and a scheduled run selects everything still listed itself"
    );
    assert_eq!(
        after.owner_device.as_deref(),
        Some(DEVICE),
        "and the run names the machine that holds it"
    );

    // The description completes with no duplicate owed. A scheduled run
    // carries its own separately approved rule, so it is the one kind of run
    // that may reach `committing` without a seller confirming anything.
    let described = call(
        &app,
        Method::POST,
        &format!("/v1/devices/{DEVICE}/import"),
        Some(
            serde_json::to_value(ImportPage {
                run: open.run,
                request: None,
                attempt: Some(attempt),
                receipt: Some(tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes())),
                enumeration_complete: false,
                listed: None,
                resources: vec![observed("https://www.tes.com/x/1", "Fractions Pack")],
                skipped: Vec::new(),
                complete: true,
                failed: None,
            })
            .expect("the page serialises"),
        ),
    )
    .await;
    assert_eq!(described.status, StatusCode::OK, "{}", described.body);
    let run: ImportRunView = call(
        &app,
        Method::GET,
        &format!("/v1/imports/runs/{}", open.run.to_hyphenated()),
        None,
    )
    .await
    .json();
    assert_eq!(
        run.state,
        ImportRunState::Committing,
        "nothing is owed to the seller, so the pass may finish it"
    );
    assert!(
        run.execution.commit_authorised,
        "and the rule that opened it is the authorisation, not a seller's confirmation"
    );

    // The next pass commits it and the rule publishes what it created.
    let finished = tam_api::scheduler::pass(&state, NOW)
        .await
        .expect("the second pass runs");
    assert_eq!(finished.committed, 1, "{:?}", finished.failures);
    assert_eq!(finished.pulls, 0, "the shop was read three hours ago");
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM product WHERE org_id = $1 AND deleted_at IS NULL"
        )
        .await,
        1,
        "the resource is in the catalogue"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM auto_publish_run WHERE org_id = $1 \
             AND target_inventory = 'tpt'"
        )
        .await,
        1,
        "the rule published it, once"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM job_item i JOIN job j ON j.org_id = i.org_id \
             AND j.id = i.job_id WHERE i.org_id = $1 AND j.inventory = 'tpt' \
             AND i.operation = 'create'"
        )
        .await,
        1,
        "a live intent on an unbound mapping is a create, and the publish waits on it"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM job_item i JOIN job j ON j.org_id = i.org_id \
             AND j.id = i.job_id WHERE i.org_id = $1 AND j.inventory = 'tpt' \
             AND i.operation = 'publish'"
        )
        .await,
        1,
    );

    // And again, at the same instant: the run is complete, the rule's row
    // exists, and neither is done twice.
    let third = tam_api::scheduler::pass(&state, NOW)
        .await
        .expect("the third pass runs");
    assert_eq!(third.committed, 0);
    assert_eq!(third.pulls, 0);
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM auto_publish_run WHERE org_id = $1"
        )
        .await,
        1,
        "the rule's primary key is what stops a second publish"
    );
    assert_eq!(
        counted(
            &pool,
            "SELECT count(*) FROM job WHERE org_id = $1 AND inventory = 'tpt'"
        )
        .await,
        1,
        "no second job on the target"
    );
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn listed(locator: &str, title: &str) -> ListedResource {
    ListedResource {
        locator: Locator::new(locator).expect("a bounded locator"),
        title: title.to_owned(),
        price_minor: None,
        currency: None,
        state: Some(ListingState::Live),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn observed(locator: &str, title: &str) -> ObservedResource {
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
            text: None,
            page_count: None,
            cover_phash: None,
            title_norm: tam_fingerprint::normalise_title(title),
        }),
        file: Some(ObservedFile {
            payload_file_name: FileName::new("worksheet-pack.pdf").expect("a plain name"),
            payload_content_type: ContentType::new("application/pdf").expect("a media type"),
            kind: FileKind::Pdf,
            hash: ContentHash([0x3A; 32]),
            byte_len: BIG,
            scan: ScanOutcome::Clean { at: NOW },
            entry: None,
        }),
        cover_png: Some(cover()),
    }
}
