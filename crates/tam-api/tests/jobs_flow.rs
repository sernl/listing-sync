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
    ContentHash, CopyFormat, FileId, FileKind, FileRole, InventoryId, ListingCopy, MappingId,
    OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome, Timestamp,
    Title, UserId, Uuid,
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
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([id.wrapping_add(0x10); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([hash; 32]),
                byte_len: 4,
                scan: ScanOutcome::Pending,
            },
            vec![],
        ),
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
                    inventory: InventoryId::TesNz,
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
        "inventory": "TesNz",
        "mappings": [
            uuid::Uuid::from_bytes(MAPPING_1.0 .0).to_string(),
            uuid::Uuid::from_bytes(MAPPING_2.0 .0).to_string(),
        ],
    })
}

const KEY_1: &str = "11111111-1111-4111-8111-111111111111";
const KEY_2: &str = "22222222-2222-4222-8222-222222222222";

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
        Some(serde_json::json!({ "inventory": "TesNz", "mappings": [stranger.clone()] })),
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
