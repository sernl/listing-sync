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
use tam_domain::{
    Binding, FieldPolicies, FieldPolicy, ItemOutcome, JobItemId, Mapping, PublishMode,
};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{
    ItemVerdict, LeaseRef, LeaseRepo, MappingRepo, ProductRepo, SessionRepo, SessionToken,
};
use tam_types::{
    Actor, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId,
    ScanOutcome, SystemComponent, Timestamp, Title, UserId, Uuid,
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
        exchange_rates: None,
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
const KEY_3: &str = "33333333-3333-4333-8333-333333333333";

/// Two attempt identities, because a second attempt row on one item needs an
/// id of its own and the item's would collide with the first.
const ATTEMPT_1: Uuid = Uuid([0xA1; 16]);
const ATTEMPT_2: Uuid = Uuid([0xA2; 16]);

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn deleting_a_queued_job_preserves_resources_and_prevents_replay(pool: PgPool) {
    provision(&pool).await;
    let created = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let receipt: serde_json::Value = created.json();
    let job = receipt["job"].as_str().expect("the created job has an id");
    let path = format!("/v1/jobs/{job}");
    let product_path = format!("/v1/products/{}", uuid::Uuid::from_bytes([0x01; 16]));
    let before = call(
        pool.clone(),
        Method::GET,
        &product_path,
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(before.status, StatusCode::OK);

    let other_tenant = call(pool.clone(), Method::DELETE, &path, &TOKEN_B, None, None).await;
    assert_eq!(other_tenant.status, StatusCode::NOT_FOUND);

    for _ in 0..2 {
        let deleted = call(pool.clone(), Method::DELETE, &path, &TOKEN_A, None, None).await;
        assert_eq!(deleted.status, StatusCode::OK);
        assert_eq!(deleted.json::<serde_json::Value>()["status"], "deleted");
    }
    let history = call(pool.clone(), Method::GET, "/v1/jobs", &TOKEN_A, None, None).await;
    assert_eq!(history.status, StatusCode::OK);
    assert_eq!(
        history.json::<serde_json::Value>()["jobs"],
        serde_json::json!([])
    );
    let after = call(
        pool.clone(),
        Method::GET,
        &product_path,
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(
        after.json::<serde_json::Value>(),
        before.json::<serde_json::Value>()
    );
    let replay = call(
        pool,
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(replay.status, StatusCode::CONFLICT);
}

/// Leases one of the job's items by hand and answers which item, under which
/// epoch.
///
/// The ledger's own `acquire` is cross-tenant and runs on the engine role,
/// which this file has no pool for; what the test needs is one item in a live
/// lease state, which is exactly what this writes. The epoch comes back
/// because a settle is fenced on it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn lease_one(pool: &PgPool) -> (JobItemId, i64) {
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let row: (uuid::Uuid, i64) = sqlx::query_as(
        "UPDATE job_item SET state = 'leased', lease_owner = 'a-test-worker', \
                lease_expires_at = now() + interval '5 minutes' \
          WHERE org_id = $1 \
            AND id = (SELECT id FROM job_item WHERE org_id = $1 \
                       ORDER BY created_at, id LIMIT 1) \
          RETURNING id, lease_epoch",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .fetch_one(&mut *tx)
    .await
    .expect("an item leases");
    tx.commit().await.expect("the lease commits");
    (JobItemId(Uuid(*row.0.as_bytes())), row.1)
}

/// A job whose item is being worked on right now is not hidden by a Delete.
///
/// The rule this is about: a leased item can still write to a marketplace, so
/// a console that removed the row would be telling the seller a publish had
/// been called off while the device was mid-submit. The Delete is accepted —
/// nothing further is served, and the item that had not started is settled —
/// and the row stays visible saying `stopping` until the work it is waiting
/// on is quiet. Settling that item is what retires it, in the settle's own
/// transaction, with nobody asking again.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn deleting_a_leased_job_reports_stopping_until_the_item_settles(pool: PgPool) {
    provision(&pool).await;
    let created = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let job = created.json::<serde_json::Value>()["job"]
        .as_str()
        .expect("the created job has an id")
        .to_owned();
    let path = format!("/v1/jobs/{job}");
    let (item, epoch) = lease_one(&pool).await;

    let stopping = call(pool.clone(), Method::DELETE, &path, &TOKEN_A, None, None).await;
    assert_eq!(
        stopping.status,
        StatusCode::ACCEPTED,
        "a job with a live lease is accepted for deletion, not deleted"
    );
    assert_eq!(
        stopping.json::<serde_json::Value>()["status"],
        "stopping",
        "and it says so rather than claiming the work is gone"
    );
    let listed = call(pool.clone(), Method::GET, "/v1/jobs", &TOKEN_A, None, None).await;
    let page: serde_json::Value = listed.json();
    assert_eq!(
        page["jobs"][0]["deletion_status"], "stopping",
        "the seller keeps a row that can still write, labelled: {page}"
    );

    LeaseRepo::new(pool.clone())
        .settle(
            &LeaseRef {
                org: ORG_A,
                item,
                lease_epoch: epoch,
            },
            &ItemVerdict {
                outcome: ItemOutcome::Succeeded,
                failure_code: None,
                failure_detail: None,
            },
            NOW,
        )
        .await
        .expect("the item the device was working on settles");

    let history = call(pool.clone(), Method::GET, "/v1/jobs", &TOKEN_A, None, None).await;
    assert_eq!(
        history.json::<serde_json::Value>()["jobs"],
        serde_json::json!([]),
        "settling the last live item retires the deletion without a second request"
    );
    assert_eq!(
        call(pool, Method::GET, &path, &TOKEN_A, None, None)
            .await
            .status,
        StatusCode::NOT_FOUND,
        "and its detail page goes with it"
    );
}

/// A write nobody can account for is never written off by a Delete.
///
/// An attempt still `in_flight` is the ledger saying it does not know what the
/// marketplace did with a write it issued. Deleting the job must not settle
/// that item — an outcome nobody observed is the one thing this ledger cannot
/// undo — so the job is retained, `needs_review`, for reconciliation to
/// finish. Everything that had not started is still stopped.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn deleting_a_job_with_an_unresolved_write_stays_visible_for_review(pool: PgPool) {
    provision(&pool).await;
    let created = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let job = created.json::<serde_json::Value>()["job"]
        .as_str()
        .expect("the created job has an id")
        .to_owned();
    let (item, epoch) = lease_one(&pool).await;
    open_attempt(&pool, item, epoch).await;

    let accepted = call(
        pool.clone(),
        Method::DELETE,
        &format!("/v1/jobs/{job}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    assert_eq!(
        accepted.json::<serde_json::Value>()["status"],
        "stopping",
        "the attempt is the live lease's own, and its holder may still settle it"
    );

    // The lease lapses with the attempt still standing, which is the state
    // nothing but evidence resolves: the holder is gone and the write is
    // undecided.
    expire_leases(&pool).await;
    let review = call(
        pool.clone(),
        Method::DELETE,
        &format!("/v1/jobs/{job}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(review.status, StatusCode::ACCEPTED);
    assert_eq!(
        review.json::<serde_json::Value>()["status"],
        "needs_review",
        "an issued write with no answer is not a deleted job"
    );
    let page: serde_json::Value = call(pool, Method::GET, "/v1/jobs", &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(
        page["jobs"][0]["deletion_status"], "needs_review",
        "and the seller can still see it: {page}"
    );
}

/// A stopped item whose write is undecided stops holding the marketplace.
///
/// The state this is about has no other way out: the holder is gone, the
/// attempt is in flight, and nothing may settle it. Left `leased` it keeps
/// the per-connection live-lease slot, so every unrelated job on that
/// marketplace waits behind a job the seller already deleted. The Delete
/// moves it to the park a reconciling claim reads, without settling the
/// attempt and without charging it — the evidence stays exactly where a
/// human or a later reconcile can find it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_stopped_undecided_write_releases_the_marketplace_slot(pool: PgPool) {
    provision(&pool).await;
    let created = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let job = created.json::<serde_json::Value>()["job"]
        .as_str()
        .expect("the created job has an id")
        .to_owned();
    let (item, epoch) = lease_one(&pool).await;
    open_attempt(&pool, item, epoch).await;
    expire_leases(&pool).await;

    let review = call(
        pool.clone(),
        Method::DELETE,
        &format!("/v1/jobs/{job}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(review.status, StatusCode::ACCEPTED);
    assert_eq!(review.json::<serde_json::Value>()["status"], "needs_review");

    let items: serde_json::Value = call(
        pool,
        Method::GET,
        &format!("/v1/jobs/{job}/items"),
        &TOKEN_A,
        None,
        None,
    )
    .await
    .json();
    let stopped = items["items"]
        .as_array()
        .expect("the items page is a list")
        .iter()
        .find(|row| row["item"] == uuid::Uuid::from_bytes(item.0 .0).to_string())
        .expect("the item that held the lease is still in the ledger")
        .clone();
    assert_eq!(
        stopped["state"], "parked_live",
        "the lapsed lease is released rather than held against the marketplace: {stopped}"
    );
    assert_eq!(
        stopped["blocked_on"], "awaiting_marketplace_answer",
        "and it parks where a reconciling claim looks, not where a clock does: {stopped}"
    );
    assert!(
        stopped["outcome"].is_null(),
        "nothing was settled on its behalf: {stopped}"
    );
}

/// A committed write receipt is never overwritten with `skipped`.
///
/// The driver settles the attempt and binds the mapping in one call and
/// settles the item in a later one. A device that died between the two leaves
/// a committed receipt over an unsettled item: the write happened. Reporting
/// "nothing was sent to the marketplace" over that is the invented outcome
/// this endpoint exists not to produce, so the item is retained for review
/// while everything genuinely unstarted is still stopped.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_committed_receipt_over_an_unsettled_item_is_not_written_off(pool: PgPool) {
    provision(&pool).await;
    let created = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let job = created.json::<serde_json::Value>()["job"]
        .as_str()
        .expect("the created job has an id")
        .to_owned();
    let (item, epoch) = lease_one(&pool).await;
    attempt_in_state(&pool, item, epoch, ATTEMPT_2, "committed").await;
    expire_leases(&pool).await;

    let review = call(
        pool.clone(),
        Method::DELETE,
        &format!("/v1/jobs/{job}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(
        review.status,
        StatusCode::ACCEPTED,
        "an item with a committed write behind it is not a deleted job"
    );
    assert_eq!(review.json::<serde_json::Value>()["status"], "needs_review");

    let items: serde_json::Value = call(
        pool,
        Method::GET,
        &format!("/v1/jobs/{job}/items"),
        &TOKEN_A,
        None,
        None,
    )
    .await
    .json();
    let rows = items["items"].as_array().expect("the items page is a list");
    let written = rows
        .iter()
        .find(|row| row["item"] == uuid::Uuid::from_bytes(item.0 .0).to_string())
        .expect("the item carrying the receipt is still in the ledger");
    assert!(
        written["outcome"].is_null(),
        "the receipt says a write was issued, so no outcome is invented for it: {written}"
    );
    assert!(
        rows.iter().any(|row| row["outcome"] == "skipped"),
        "and the item that never ran is still stopped: {items}"
    );
}

/// An ambiguous outcome keeps the job in the seller's history.
///
/// `ambiguous` is the ledger saying it does not know what the marketplace did
/// with a write it sent. Every item may be settled and every attempt closed,
/// and the external write is still unaccounted for — so the absence of an
/// `in_flight` row is not proof that anything was answered.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_ambiguous_outcome_keeps_a_deleted_job_visible_for_review(pool: PgPool) {
    provision(&pool).await;
    let created = call(
        pool.clone(),
        Method::POST,
        "/v1/jobs",
        &TOKEN_A,
        Some(KEY_1),
        Some(create_body()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let job = created.json::<serde_json::Value>()["job"]
        .as_str()
        .expect("the created job has an id")
        .to_owned();
    let (item, epoch) = lease_one(&pool).await;
    LeaseRepo::new(pool.clone())
        .settle(
            &LeaseRef {
                org: ORG_A,
                item,
                lease_epoch: epoch,
            },
            &ItemVerdict {
                outcome: ItemOutcome::Ambiguous,
                failure_code: None,
                failure_detail: None,
            },
            NOW,
        )
        .await
        .expect("the driver records what it could not determine");

    let review = call(
        pool.clone(),
        Method::DELETE,
        &format!("/v1/jobs/{job}"),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(review.status, StatusCode::ACCEPTED);
    assert_eq!(
        review.json::<serde_json::Value>()["status"],
        "needs_review",
        "an unresolved external write is not a deleted job, settled or not"
    );
    let page: serde_json::Value = call(pool, Method::GET, "/v1/jobs", &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(
        page["jobs"][0]["deletion_status"], "needs_review",
        "and the seller keeps the row saying so: {page}"
    );
}

/// Deleting one leg of a request stops the request and its sibling leg.
///
/// The window this is about is the drain's own: `drain_request` commits each
/// leg's job in its own transaction and writes `create_job_id`/`remove_job_id`
/// only afterwards, so a Delete arriving in between sees a request naming no
/// legs at all. Fencing on those columns would report the request gone while
/// a minted leg — a Copy's create, or a Move's source removal — stayed on the
/// queue, claimable. The legs are discovered through the link the job carries
/// from its first instant instead, and the Delete of either leg is the Delete
/// of the whole request.
///
/// The two legs are minted through the ordinary job route and then linked to
/// the request, because the drain that would mint them reads the source
/// marketplace and this process holds no session for one. What the pair
/// stands for is a migration's create and removal; what is reproduced exactly
/// is the state the interleaving leaves: two committed legs naming their
/// request, and a request naming neither.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn deleting_one_leg_stops_the_request_and_its_sibling(pool: PgPool) {
    provision(&pool).await;
    let accepted = call(
        pool.clone(),
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(serde_json::json!({
            "source": "Tes",
            "target": "Tpt",
            "disposition": "sync",
            "intent": "live",
            "resources": ["13549794"],
        })),
    )
    .await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    // The two legs a drain mints, before it has recorded either on the
    // request: this is the interleaving, and the second job is the one that
    // must not survive a Delete addressed to the first.
    let mut legs = Vec::new();
    for (key, mapping) in [(KEY_2, MAPPING_1), (KEY_3, MAPPING_2)] {
        let mut body = create_body();
        body["mappings"] = serde_json::json!([uuid::Uuid::from_bytes(mapping.0 .0).to_string()]);
        let leg = call(
            pool.clone(),
            Method::POST,
            "/v1/jobs",
            &TOKEN_A,
            Some(key),
            Some(body),
        )
        .await;
        assert_eq!(leg.status, StatusCode::CREATED);
        legs.push(
            leg.json::<serde_json::Value>()["job"]
                .as_str()
                .expect("the created job has an id")
                .to_owned(),
        );
    }
    own_by_request(&pool, KEY_1).await;

    let deleted = call(
        pool.clone(),
        Method::DELETE,
        &format!("/v1/jobs/{}", legs[0]),
        &TOKEN_A,
        None,
        None,
    )
    .await;
    assert_eq!(
        deleted.status,
        StatusCode::OK,
        "both legs were queued and nothing had run, so the whole request stops at once"
    );
    assert_eq!(deleted.json::<serde_json::Value>()["status"], "deleted");

    let history: serde_json::Value =
        call(pool.clone(), Method::GET, "/v1/jobs", &TOKEN_A, None, None)
            .await
            .json();
    assert_eq!(
        history["jobs"],
        serde_json::json!([]),
        "the sibling leg goes with the leg the seller addressed: {history}"
    );
    assert_eq!(
        call(
            pool.clone(),
            Method::GET,
            &format!("/v1/jobs/{}", legs[1]),
            &TOKEN_A,
            None,
            None,
        )
        .await
        .status,
        StatusCode::NOT_FOUND,
        "including its detail page"
    );
    assert_eq!(
        call(
            pool.clone(),
            Method::GET,
            &format!("/v1/sync/{KEY_1}"),
            &TOKEN_A,
            None,
            None,
        )
        .await
        .status,
        StatusCode::NOT_FOUND,
        "and the request the legs belonged to is stopped too, not only the leg"
    );
    let replay = call(
        pool,
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
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
        replay.status,
        StatusCode::CONFLICT,
        "and its key cannot start the move again"
    );
}

/// Links every unowned job of this tenant to one request, which is what a
/// drain's own mint does in the statement that inserts the job.
///
/// The fixture exists because the drain that would write these links reads
/// the source marketplace, and this process holds no session for one. What it
/// reproduces is the state after both legs are committed and before
/// `record_enqueued` names either — the interleaving under test.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn own_by_request(pool: &PgPool, request: &str) {
    let request: uuid::Uuid = request.parse().expect("the request id parses");
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "UPDATE job SET sync_request_id = $2 \
          WHERE org_id = $1 AND sync_request_id IS NULL",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(request)
    .execute(&mut *tx)
    .await
    .expect("the legs name their request");
    tx.commit().await.expect("the link commits");
}

/// Opens a write attempt by hand, in flight, against one leased item.
///
/// The fencing row is what "a write may have landed" is recorded as, and the
/// repository's own `open_asserted` needs a projected body this test has no
/// reason to build: the columns below are the fact under test.
async fn open_attempt(pool: &PgPool, item: JobItemId, epoch: i64) {
    attempt_in_state(pool, item, epoch, ATTEMPT_1, "in_flight").await;
}

/// One write-attempt row by hand, in the state named.
///
/// `actor_kind` and `actor_id` are not decoration: migration 0033 made
/// attribution mandatory on every audit-bearing row, so a fixture omitting
/// them is a NOT NULL violation rather than a row with an unknown author.
/// The author recorded is the one the real path records — the seller's own
/// machine is what issues these writes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn attempt_in_state(pool: &PgPool, item: JobItemId, epoch: i64, attempt: Uuid, state: &str) {
    let actor = Actor::System(SystemComponent::Device);
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO write_attempt \
             (org_id, id, job_item_id, mapping_id, lease_epoch, intent, intent_hash, \
              state, opened_at, actor_kind, actor_id) \
         SELECT $1, $2, ji.id, ji.mapping_id, $3, '{}'::jsonb, $4, $6, now(), $7, $8 \
           FROM job_item ji WHERE ji.org_id = $1 AND ji.id = $5",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(attempt.0))
    .bind(epoch)
    .bind(vec![0x11u8; 32])
    .bind(uuid::Uuid::from_bytes(item.0 .0))
    .bind(state)
    .bind(actor.kind())
    .bind(actor.id())
    .execute(&mut *tx)
    .await
    .expect("the attempt opens");
    tx.commit().await.expect("the attempt commits");
}

/// Ages every live lease out, without settling anything.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn expire_leases(pool: &PgPool) {
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "UPDATE job_item SET lease_expires_at = now() - interval '1 minute' \
          WHERE org_id = $1 AND state IN ('leased', 'running', 'verifying')",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .execute(&mut *tx)
    .await
    .expect("the leases lapse");
    tx.commit().await.expect("the expiry commits");
}

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

/// Deleting a sync stops it, hides it, and refuses its key.
///
/// The request's identity *is* its idempotency key, which is what makes the
/// last part load-bearing: the console's retry of a submit whose answer was
/// lost carries the same key, and answering it with the deleted request — or
/// worse, writing a second one under it — would resurrect exactly the work
/// the seller stopped. A pending request has no leg running, so the Delete
/// finishes immediately and repeating it says the same thing.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn deleting_a_sync_request_stops_it_and_refuses_the_replayed_key(pool: PgPool) {
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
    let path = format!("/v1/sync/{KEY_1}");

    let other_tenant = call(pool.clone(), Method::DELETE, &path, &TOKEN_B, None, None).await;
    assert_eq!(
        other_tenant.status,
        StatusCode::NOT_FOUND,
        "another tenant's request is not there to delete"
    );

    for _ in 0..2 {
        let deleted = call(pool.clone(), Method::DELETE, &path, &TOKEN_A, None, None).await;
        assert_eq!(
            deleted.status,
            StatusCode::OK,
            "a request with no leg running is stopped outright: {}",
            String::from_utf8_lossy(&deleted.body)
        );
        assert_eq!(deleted.json::<serde_json::Value>()["status"], "deleted");
    }

    assert_eq!(
        call(pool.clone(), Method::GET, &path, &TOKEN_A, None, None)
            .await
            .status,
        StatusCode::NOT_FOUND,
        "a deleted request has no page"
    );
    let list: serde_json::Value = call(pool.clone(), Method::GET, "/v1/sync", &TOKEN_A, None, None)
        .await
        .json();
    assert_eq!(
        list["requests"],
        serde_json::json!([]),
        "and the history it was in is filtered in SQL: {list}"
    );

    let replay = call(
        pool,
        Method::POST,
        "/v1/sync",
        &TOKEN_A,
        Some(KEY_1),
        Some(body),
    )
    .await;
    assert_eq!(
        replay.status,
        StatusCode::CONFLICT,
        "the key is spent and its request is gone, so the retry is refused rather than \
         re-enqueued: {}",
        String::from_utf8_lossy(&replay.body)
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
