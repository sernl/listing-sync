//! Jobs as operation resources, end to end: the mandatory idempotency key,
//! replay returning the same job, the content-addressed 409, the roll-up
//! that never gates on a bad item, keyset item pages, and tenant isolation.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::jobs::{CreatedJobBody, ItemDetail, ItemsPage, JobPhase, JobView};
use tam_api::{router, APIError, APIErrorCode, AppState, Config, SESSION_COOKIE};
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{MappingRepo, ProductRepo, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, ListingCopy,
    MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);
const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));
const MAPPING_2: MappingId = MappingId(Uuid([0x32; 16]));

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

fn product(id: u8, hash: u8) -> tam_domain::CanonicalProduct {
    tam_domain::CanonicalProduct {
        id: ProductId(Uuid([id; 16])),
        org: ORG_A,
        title: Title(format!("Fixture {id}")),
        body: ListingCopy {
            body: "Fixture body.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([id.wrapping_add(0x10); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Held {
                    hash: ContentHash([hash; 32]),
                    byte_len: 4,
                    scan: ScanOutcome::Pending,
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
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
        consented(pool, org).await;
        // Copying and moving is a paid capability, so the fixture tenant is
        // a subscriber: without a grant every sync in this file would be
        // answered by the migration cap rather than by the job machinery it
        // is written to exercise.
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
                    source_ref: Some(name),
                    granted_at: Timestamp(1_000),
                    expires_at: None,
                },
            )
            .await
            .expect("the fixture grant seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_A, UserId(Uuid([0x0A; 16])), "a@example.test", TOKEN_A),
        (ORG_B, UserId(Uuid([0x0B; 16])), "b@example.test", TOKEN_B),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, Timestamp(100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
    let products = ProductRepo::new(pool.clone());
    let mappings = MappingRepo::new(pool.clone());
    for (mapping_id, product_id, hash) in [(MAPPING_1, 0x01u8, 0x51u8), (MAPPING_2, 0x02, 0x52)] {
        products
            .insert(ORG_A, &product(product_id, hash), Timestamp(1_000))
            .await
            .expect("the product inserts");
        mappings
            .insert(
                ORG_A,
                &Mapping {
                    id: mapping_id,
                    org: ORG_A,
                    product: ProductId(Uuid([product_id; 16])),
                    inventory: InventoryId::Tes,
                    binding: Binding::Unbound,
                    policies: FieldPolicies {
                        title: FieldPolicy::Managed,
                        description: FieldPolicy::Managed,
                        price: FieldPolicy::Managed,
                        taxonomy: FieldPolicy::Managed,
                        grades: FieldPolicy::Managed,
                        files: FieldPolicy::Managed,
                    },
                    price_rule: PriceRule::Explicit(PriceIntent::Free),
                    publish: PublishMode::DryRun,
                    lifecycle: RemoteLifecycle::Absent,
                },
                0,
                Timestamp(1_000),
            )
            .await
            .expect("the mapping inserts");
    }
}

struct Answer {
    status: StatusCode,
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
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "a test-local request helper; every call site reads the six labels in place"
)]
async fn call(
    pool: PgPool,
    method: Method,
    path: &str,
    token: &SessionToken,
    idempotency: Option<&str>,
    body: Option<serde_json::Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    if let Some(key) = idempotency {
        request = request.header("Idempotency-Key", key);
    }
    let request = match body {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer { status, body }
}

fn create_body() -> serde_json::Value {
    serde_json::json!({
        "inventory": "Tes",
        "mappings": [
            uuid::Uuid::from_bytes(MAPPING_1.0 .0).to_string(),
            uuid::Uuid::from_bytes(MAPPING_2.0 .0).to_string(),
        ],
    })
}

const KEY_1: &str = "11111111-1111-4111-8111-111111111111";
const KEY_2: &str = "22222222-2222-4222-8222-222222222222";

/// The one enqueue for sync, migrate and bulk. It writes the request and
/// returns: no marketplace is read here, because this process holds no
/// session and the broker lease belongs to the drain.
///
/// A `sync` rather than the `migrate` this used to post. C4's rule is that a
/// migrate from a marketplace with no official API names no resources here,
/// because the seller's own device is the only thing that can enumerate that
/// catalogue — so the old body is now a 422 by design. A sync is the shape
/// that still carries a seller-supplied list, which is what the assertions
/// below are about: that the list is written and polled back in the order it
/// was given. The migrate rule has its own test beside this one.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sync_request_is_accepted_written_and_polled_back(pool: PgPool) {
    provision(&pool).await;
    let body = serde_json::json!({
        "source": "Tes",
        "target": "Tpt",
        "disposition": "sync",
        "intent": "live",
        "resources": ["13549794", "13549795"],
    });
    let accepted = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(body.clone()),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);

    let replay = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(body),
    )
    .await;
    assert_eq!(
        replay.status,
        StatusCode::OK,
        "the idempotency key is the request's identity, so a retried submit is the same request"
    );

    let view = call(
        pool,
        Method::GET,
        &format!("/v1/sync/{KEY_1}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(view.status, StatusCode::OK);
    let record: serde_json::Value = view.json();
    assert_eq!(record["disposition"], "sync");
    assert_eq!(record["state"], "pending");
    assert_eq!(
        record["resources"].as_array().map(Vec::len),
        Some(2),
        "the seller's own list, in the order they gave it"
    );
    assert!(
        record["create_job"].is_null() && record["remove_job"].is_null(),
        "no job exists until the drain has read the source"
    );
}

/// Who supplies the resources, both ways.
///
/// A migrate from a marketplace with no official API is enumerated by the
/// seller's own device under D1, so naming a list here is refused rather than
/// silently ignored — the seller would otherwise submit a list, watch the
/// device walk the whole shop instead, and never learn the two were unrelated.
/// The same request with no list is the shape the console posts and is
/// accepted, which is what makes the refusal a rule about who enumerates
/// rather than a rule against empty requests.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_migrate_from_a_device_enumerated_source_names_no_resources(pool: PgPool) {
    provision(&pool).await;
    let named = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(serde_json::json!({
            "source": "Tes",
            "target": "Tpt",
            "disposition": "migrate",
            "intent": "draft",
            "resources": ["13549794"],
        })),
    )
    .await;
    assert_eq!(named.status, StatusCode::UNPROCESSABLE_ENTITY);
    let refusal: serde_json::Value = named.json();
    assert!(
        refusal.to_string().contains("enumerates that catalogue"),
        "the refusal says who does enumerate it, so the seller knows what to do instead: \
         {refusal}"
    );

    let empty = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_2),
        Some(serde_json::json!({
            "source": "Tes",
            "target": "Tpt",
            "disposition": "migrate",
            "intent": "draft",
            "resources": [],
        })),
    )
    .await;
    assert_eq!(
        empty.status,
        StatusCode::ACCEPTED,
        "and the same request with no list is accepted, because the device fills it"
    );
    let view = call(
        pool,
        Method::GET,
        &format!("/v1/sync/{KEY_2}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(view.status, StatusCode::OK);
    let record: serde_json::Value = view.json();
    assert_eq!(
        record["resources"].as_array().map(Vec::len),
        Some(0),
        "written with nothing, waiting for the device's first page"
    );
}

/// A source the drain cannot read is refused at submit, not by a request that
/// never settles.
///
/// `download_resource_bundle` is uncaptured on Etsy, and the endpoint
/// validated only that source and target differ. The row was written
/// `pending`, the drain built an adapter for it regardless, failed before it
/// could mark anything, and re-picked the same row every poll -- taking a
/// broker lease each pass while the seller polled `pending` forever.
///
/// TPT is the other half, and it is the same rule read the other way: its
/// own-file download was captured on 2026-09-13, so a TPT source is admitted
/// here rather than refused. The refusal is about what has been captured,
/// which is why the two live in one test.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_source_with_no_captured_read_is_refused_at_submit(pool: PgPool) {
    provision(&pool).await;
    let response = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(serde_json::json!({
            "source": "Etsy",
            "target": "Tes",
            "resources": ["1846029571"],
        })),
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let refusal: serde_json::Value = response.json();
    assert!(
        refusal
            .to_string()
            .contains("etsy.download_resource_bundle"),
        "the refusal names the capture the source is waiting on: {refusal}"
    );

    let view = call(
        pool.clone(),
        Method::GET,
        &format!("/v1/sync/{KEY_1}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(
        view.status,
        StatusCode::NOT_FOUND,
        "a refused submit writes no request for the drain to pick up"
    );

    let captured = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_2),
        Some(serde_json::json!({
            "source": "Tpt",
            "target": "Tes",
            "resources": ["13042099"],
        })),
    )
    .await;
    assert_eq!(
        captured.status,
        StatusCode::ACCEPTED,
        "a TPT source names a download the seller's own device can perform, so the request is \
         written: {}",
        String::from_utf8_lossy(&captured.body)
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sync_between_one_inventory_and_itself_is_refused(pool: PgPool) {
    provision(&pool).await;
    let response = call(
        pool,
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(serde_json::json!({
            "source": "Tes",
            "target": "Tes",
            "resources": ["13549794"],
        })),
    )
    .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Publish-to-both is this endpoint's headline flow, and from an unbound
/// mapping it is genuinely two writes: both adapters create a draft, so the
/// publish names whatever the create bound.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_live_intent_on_an_unbound_mapping_lowers_to_a_create_and_a_publish(pool: PgPool) {
    provision(&pool).await;
    let mut body = create_body();
    body["intent"] = serde_json::json!("live");
    let response = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(body),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let created: CreatedJobBody = response.json();

    let items = call(
        pool,
        Method::GET,
        &format!("/v1/jobs/{}/items", created.job.0.to_hyphenated()),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    let page: serde_json::Value = items.json();
    let rows = page["items"].as_array().expect("the page carries items");
    assert_eq!(
        rows.len(),
        4,
        "two mappings, each lowering to a create and the publish that follows it"
    );
}

/// A stated intent lowers against what the mapping actually is, and a
/// lifecycle the bind never wrote is refused rather than assumed. Every
/// mapping written before the bind recorded one reads absent, so this is the
/// common case for anything older than Phase 3 -- and the API would otherwise
/// have to assert a lower end of the transition it does not know.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_intent_against_an_unverifiable_lifecycle_is_refused_rather_than_guessed(pool: PgPool) {
    provision(&pool).await;
    // `mapping` carries FORCE ROW LEVEL SECURITY, so an unpinned update
    // matches nothing and reports it as a row count of zero rather than an
    // error. The pin and the update share one transaction because the pin is
    // transaction-local.
    let mut tx = pool.begin().await.expect("the fixture opens a transaction");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the fixture pins the tenant");
    let bound = sqlx::query(
        "UPDATE mapping SET binding_state = 'bound', remote_id_kind = 'tes', \
         remote_url = 'https://www.tes.com/teaching-resource/x-' || id::text, \
         first_seen_at = now(), \
         lifecycle_state = 'absent', lifecycle_since = NULL, lifecycle_reason = NULL, \
         verify_state = 'clean', verified_at = now(), verify_stale_since = NULL \
         WHERE org_id = $1",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .execute(&mut *tx)
    .await
    .expect("the fixture binds the mappings");
    assert!(
        bound.rows_affected() > 0,
        "an unpinned update would silently match nothing and the test would pass wrongly"
    );
    tx.commit().await.expect("the fixture commits");
    let response = call(
        pool,
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a bound mapping whose lifecycle nobody observed cannot be lowered against"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_retry_and_the_double_click_are_the_same_job(pool: PgPool) {
    provision(&pool).await;
    let first = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(first.status, StatusCode::CREATED);
    let created: CreatedJobBody = first.json();
    assert!(!created.replay, "the first carrier of a key creates");

    let replay = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(replay.status, StatusCode::OK);
    let replayed: CreatedJobBody = replay.json();
    assert_eq!(
        (replayed.job, replayed.replay),
        (created.job, true),
        "the same key returns the original job untouched"
    );

    let rerun = call(
        pool,
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_2),
        Some(create_body()),
    )
    .await;
    assert_eq!(
        rerun.status,
        StatusCode::CONFLICT,
        "a new key over unchanged content is the duplicate-upload storm the ledger refuses"
    );
    let error: APIError = rerun.json();
    assert_eq!(error.errors[0].code, Some(APIErrorCode::DuplicateSyncItem));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sync_without_an_idempotency_key_is_refused(pool: PgPool) {
    provision(&pool).await;
    let refused = call(
        pool,
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        None,
        Some(create_body()),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = refused.json();
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::IdempotencyKeyRequired),
        "no key, no job"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn unknown_mappings_are_named_in_the_refusal(pool: PgPool) {
    provision(&pool).await;
    let stranger = uuid::Uuid::from_bytes([0x99; 16]).to_string();
    let refused = call(
        pool,
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(serde_json::json!({ "inventory": "Tes", "mappings": [stranger.clone()] })),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = refused.json();
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::SyncMappingsInvalid)
    );
    assert_eq!(
        error.errors[0].detail,
        Some(serde_json::json!({ "mappings": [stranger] })),
        "the refusal names exactly the mappings it rejected"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_roll_up_never_gates_on_the_first_bad_item(pool: PgPool) {
    provision(&pool).await;
    let created: CreatedJobBody = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await
    .json();
    let job_path = format!("/v1/jobs/{}", created.job.0.to_hyphenated());

    let view: JobView = call(pool.clone(), Method::GET, &job_path, &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(
        (view.counts.total, view.counts.queued),
        (2, 2),
        "both mappings became items"
    );
    assert_eq!(view.phase, JobPhase::Active);

    // One item fails; the job stays active and reports the failure beside
    // the still-queued item rather than gating on it.
    settle_one(&pool, "failed").await;
    let view: JobView = call(pool.clone(), Method::GET, &job_path, &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(
        (view.counts.settled, view.counts.failed, view.counts.queued),
        (1, 1, 1),
        "the bad item is reported, not a verdict"
    );
    assert_eq!(view.phase, JobPhase::Active);

    settle_one(&pool, "succeeded").await;
    let view: JobView = call(pool.clone(), Method::GET, &job_path, &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(view.phase, JobPhase::Settled);
    assert_eq!(
        (view.counts.failed, view.counts.succeeded),
        (1, 1),
        "the roll-up is the outcome distribution, not a scalar"
    );
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn settle_one(pool: &PgPool, outcome: &str) {
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    sqlx::query(
        "UPDATE job_item SET state = 'settled', outcome = $1, settled_at = now() \
         WHERE id IN (SELECT id FROM job_item WHERE state = 'queued' ORDER BY id LIMIT 1)",
    )
    .bind(outcome)
    .execute(&mut *tx)
    .await
    .expect("the settle applies");
    tx.commit().await.expect("the settle commits");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn item_pages_walk_a_stable_keyset(pool: PgPool) {
    provision(&pool).await;
    let created: CreatedJobBody = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await
    .json();
    let base = format!("/v1/jobs/{}/items", created.job.0.to_hyphenated());

    let first: ItemsPage = call(
        pool.clone(),
        Method::GET,
        &format!("{base}?limit=1"),
        &TOKEN_A,
        None,
        None,
    )
    .await
    .json();
    assert_eq!(first.items.len(), 1);
    let cursor = first
        .next_cursor
        .clone()
        .expect("a full page mints a cursor");

    let second: ItemsPage = call(
        pool.clone(),
        Method::GET,
        &format!("{base}?limit=1&cursor={cursor}"),
        &TOKEN_A,
        None,
        None,
    )
    .await
    .json();
    assert_eq!(second.items.len(), 1);
    assert_ne!(
        first.items[0].item, second.items[0].item,
        "the cursor advanced past the first item"
    );

    let detail_path = format!(
        "/v1/jobs/{}/items/{}",
        created.job.0.to_hyphenated(),
        first.items[0].item.to_hyphenated()
    );
    let detail: ItemDetail = call(pool, Method::GET, &detail_path, &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(detail.item.state, "queued");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_tenant_sees_no_job(pool: PgPool) {
    provision(&pool).await;
    let created: CreatedJobBody = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await
    .json();
    let path = format!("/v1/jobs/{}", created.job.0.to_hyphenated());
    let foreign = call(pool, Method::GET, &path, &TOKEN_B, None, None).await;
    assert_eq!(
        foreign.status,
        StatusCode::NOT_FOUND,
        "a job is invisible across the tenant fence, not forbidden"
    );
}

/// An ignored filter must not read as an applied one.
///
/// `parse_page` is shared by the jobs list and the items page, and only the
/// items page forwarded `outcome`. The list validated it -- refusing a typo
/// with a 422 -- and then dropped it, so a client asking for its failures got
/// a well-formed page of every job, having just been told the parameter was
/// honoured.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_outcome_filter_on_the_jobs_list_is_refused_rather_than_dropped(pool: PgPool) {
    provision(&pool).await;
    let refused = call(
        pool.clone(),
        Method::GET,
        "/v1/jobs?outcome=failed",
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);

    let listed = call(pool, Method::GET, "/v1/jobs", &TOKEN_A, None, None).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "the page itself is unchanged; only the filter it never applied is refused"
    );
}

/// The migration cap, which is the one bound a seller can wait out: the
/// refusal names the allowance, what is left, and the day the counter
/// returns to zero.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_tenant_with_no_migration_allowance_is_refused_with_its_reset_date(pool: PgPool) {
    provision(&pool).await;
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    sqlx::query("DELETE FROM entitlement_grant WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(&mut *tx)
        .await
        .expect("the fixture grant is withdrawn");
    tx.commit().await.expect("the withdrawal commits");

    let answer = call(
        pool,
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some("11111111-1111-1111-1111-111111111111"),
        Some(serde_json::json!({
            "source": "Tes",
            "target": "Tpt",
            "disposition": "sync",
            "intent": "live",
            "resources": ["13549794"],
        })),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a plan that copies nothing refuses the copy rather than enqueuing it"
    );
    let refusal: APIError = serde_json::from_slice(&answer.body).expect("the refusal parses");
    let detail = refusal.errors[0]
        .detail
        .as_ref()
        .expect("the refusal names the bound it hit");
    assert_eq!(detail["quota"], "migrations_per_month");
    assert_eq!(detail["limit"], 0);
    assert!(
        refusal.errors[0].message.starts_with("Your plan"),
        "the sentence is the seller's: {}",
        refusal.errors[0].message
    );
}
