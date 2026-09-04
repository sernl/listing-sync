//! The engine-side guarantees, proven against the live database over the
//! tam_engine role: the per-tenant mutex is structural, a stolen lease fences
//! its previous holder, halts and the connection gate fail closed, the
//! attempt budget settles rather than loops, the outbox deduplicates and
//! dead-letters, and org_seq is consecutive per organisation.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{
    attempt_budget_spent, Binding, FieldPolicies, FieldPolicy, ItemOperation, ItemOutcome,
    JobItemId, Mapping, PublishMode, LEASE_TTL_SECS,
};
use tam_marketplace::{
    IdempotencyKey, LifecycleTransition, ListingState, RemoteLifecycle, RemoteListingId,
};
use tam_storage::{
    revive_by_gap, revive_on, settle_if_complete, AttemptIntent, AttemptRef, AttemptVerdict,
    BudgetGrant, Charged, ClaimPolicy, ConnectionAudit, DeviceClaim, DeviceRef, HaltCause,
    HaltRepo, ItemVerdict, JobReadRepo, JobRepo, LandingEffect, LeaseRepo, LeasedItem, MappingRepo,
    NewAttempt, NewJob, NewJobItem, NewOutboxMessage, OutboxRepo, ProductRepo, RateBudgetRepo,
    StorageError, WriteAttemptRepo, AWAITING_MARKETPLACE_ANSWER, AWAITING_SELLER_SIGNIN,
    REAUTH_REQUIRED,
};
use tam_types::{
    Actor, CanonicalTermId, ConnectionId, ContentHash, CopyFormat, FailureCode, FailureDetail,
    FieldKey, FileId, FileKind, FileRole, InventoryId, JobId, ListingCopy, MappingId, Marketplace,
    OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome, Stamp,
    SystemComponent, Timestamp, Title, TransportClass, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
/// The worker's own budget, restated here because the revive takes it as a
/// parameter rather than holding a limit this crate does not own.
const ATTEMPTS_MAX: i32 = 5;

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

/// The engine connects as its own role; the per-test database name comes from
/// the app pool. Host and port are the dev database's, same as DATABASE_URL.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects to the test database")
}

struct Tenant {
    org: OrgId,
    product: ProductId,
    mapping: MappingId,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed_tenant(app: &PgPool, seed: u8, linked: bool) -> Tenant {
    let org = OrgId(Uuid([seed; 16]));
    let product = ProductId(Uuid([seed.wrapping_add(1); 16]));
    let mapping = MappingId(Uuid([seed.wrapping_add(2); 16]));
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(db_uuid(org.0))
        .bind(format!("org-{seed}"))
        .execute(app)
        .await
        .expect("organisation row inserts");
    let full_product = tam_domain::CanonicalProduct {
        id: product,
        org,
        title: Title("Fixture".to_owned()),
        body: ListingCopy {
            body: "Fixture".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([seed.wrapping_add(3); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([seed; 32]),
                byte_len: 4,
                scan: ScanOutcome::Pending,
            },
            vec![],
        ),
        cover: None,
        previews: vec![],
        subjects: Vec::<CanonicalTermId>::new(),
        grades: tam_domain::GradeDeclaration {
            source: tam_domain::DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: tam_domain::RightsDeclaration::Unstated,
        native_residue: vec![],
    };
    ProductRepo::new(app.clone())
        .insert(org, &full_product, T0)
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(app.clone())
        .insert(
            org,
            &Mapping {
                id: mapping,
                org,
                product,
                inventory: InventoryId::TesGb,
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
            T0,
        )
        .await
        .expect("the fixture mapping inserts");
    if linked {
        let mut tx = app.begin().await.expect("transaction begins");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(db_uuid(org.0).to_string())
            .execute(&mut *tx)
            .await
            .expect("tenant pin applies");
        sqlx::query(
            "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
             VALUES ($1, $2, 'tes', 'linked', now(), now())",
        )
        .bind(db_uuid(org.0))
        .bind(db_uuid(Uuid([seed.wrapping_add(4); 16])))
        .execute(&mut *tx)
        .await
        .expect("the fixture connection inserts");
        tx.commit().await.expect("the fixture connection commits");
    }
    register_device(app, org, DEVICE).await;
    Tenant {
        org,
        product,
        mapping,
    }
}

/// The seller's device, which the seller-device claim admits only if it is
/// registered and unrevoked.
const DEVICE: &str = "fixture-device";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn register_device(app: &PgPool, org: OrgId, device: &str) {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, \
                             first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'fixture', 'linux', 'x86_64', '0.0.0', now(), now()) \
         ON CONFLICT (org_id, id) DO NOTHING",
    )
    .bind(db_uuid(org.0))
    .bind(device)
    .execute(&mut *tx)
    .await
    .expect("the fixture device registers");
    tx.commit().await.expect("the fixture device commits");
    // Registration alone no longer admits a claim: the device must also hold
    // a session for the job's marketplace. Both device-branch marketplaces are
    // seeded because the fixture tenant enqueues onto either, and `DO NOTHING`
    // so a test that has flipped a status keeps it across the next claim.
    for marketplace in ["tes", "tpt"] {
        connect_session(app, org, device, marketplace, "connected").await;
    }
}

/// One marketplace session for a device, at a stated status.
///
/// Written over the app role behind a tenant pin, the way every other fixture
/// row on an RLS-forced table is.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn connect_session(app: &PgPool, org: OrgId, device: &str, marketplace: &str, status: &str) {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO device_marketplace_session \
             (org_id, device_id, marketplace, linked_at, last_used_at, status) \
         VALUES ($1, $2, $3, now(), now(), $4) \
         ON CONFLICT (org_id, device_id, marketplace) DO NOTHING",
    )
    .bind(db_uuid(org.0))
    .bind(device)
    .bind(marketplace)
    .bind(status)
    .execute(&mut *tx)
    .await
    .expect("the fixture session inserts");
    tx.commit().await.expect("the fixture session commits");
}

/// Moves an already-seeded session to a stated status, which the seeding
/// helper deliberately will not do.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn set_session_status(
    app: &PgPool,
    org: OrgId,
    device: &str,
    marketplace: &str,
    status: &str,
) {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "UPDATE device_marketplace_session SET status = $4 \
         WHERE org_id = $1 AND device_id = $2 AND marketplace = $3",
    )
    .bind(db_uuid(org.0))
    .bind(device)
    .bind(marketplace)
    .bind(status)
    .execute(&mut *tx)
    .await
    .expect("the session status moves");
    tx.commit().await.expect("the session status commits");
}

/// The seller-device claim, as the tests take it. `acquire` is the other
/// branch's scan and no longer sees a Tes or Tpt item at all, so a fixture
/// that means to lease one claims as a device.
async fn claim(app: &PgPool, org: OrgId, device: &str, ttl: i64) -> Option<LeasedItem> {
    claim_with_reconcile(app, org, device, ttl, true).await
}

/// The same claim, told whether a reconcile can run.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn claim_with_reconcile(
    app: &PgPool,
    org: OrgId,
    device: &str,
    ttl: i64,
    reconcile: bool,
) -> Option<LeasedItem> {
    // Each worker name is a device now, so the fixture registers whichever one
    // is claiming. Registration is what these tests assume rather than what
    // they are about; the tests that are about it revoke explicitly.
    register_device(app, org, device).await;
    match LeaseRepo::new(app.clone())
        .claim_for_device(
            &DeviceRef { org, device },
            &ClaimPolicy {
                ttl_seconds: ttl,
                grace_hours: 24,
                marketplace: None,
                reconcile,
            },
            T0,
        )
        .await
        .expect("the claim runs")
    {
        DeviceClaim::Leased(item) => Some(*item),
        DeviceClaim::Empty | DeviceClaim::HeldByAnotherDevice => None,
    }
}

/// An outcome with no failure code and no detail beside it.
fn verdict(outcome: ItemOutcome) -> ItemVerdict {
    ItemVerdict {
        outcome,
        failure_code: None,
        failure_detail: None,
    }
}

fn item(seed: u8) -> NewJobItem {
    NewJobItem {
        item: JobItemId(Uuid([seed; 16])),
        mapping: MappingId(Uuid([0; 16])),
        idempotency_key: IdempotencyKey(Uuid([seed.wrapping_add(0x40); 16])),
        operation: ItemOperation::Create,
        requires_bound_on: None,
    }
}

async fn enqueue_one(engine: &PgPool, tenant: &Tenant, job_seed: u8, item_seed: u8) -> JobItemId {
    enqueue_operation(engine, tenant, job_seed, item_seed, ItemOperation::Create).await
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn enqueue_operation(
    engine: &PgPool,
    tenant: &Tenant,
    job_seed: u8,
    item_seed: u8,
    operation: ItemOperation,
) -> JobItemId {
    let mut new_item = item(item_seed);
    new_item.mapping = tenant.mapping;
    new_item.operation = operation;
    JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job: JobId(Uuid([job_seed; 16])),
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            std::slice::from_ref(&new_item),
        )
        .await
        .expect("the fixture job enqueues");
    new_item.item
}

/// The listing an item asserts it acts on, distinct per seed so a subject
/// that survives the round trip is provably this item's.
fn subject(seed: u8) -> RemoteListingId {
    RemoteListingId::Tes {
        url: format!("https://www.tes.com/teaching-resource/fixture-{seed}"),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_tenant_mutex_holds_and_parallelism_is_inter_tenant(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant_a = seed_tenant(&app, 0xA0, true).await;
    let tenant_b = seed_tenant(&app, 0xB0, true).await;
    enqueue_one(&engine, &tenant_a, 0x11, 0x21).await;
    enqueue_one(&engine, &tenant_a, 0x12, 0x22).await;
    enqueue_one(&engine, &tenant_b, 0x13, 0x23).await;
    let expected_orgs = [tenant_a.org, tenant_b.org];

    let first = claim(&app, tenant_a.org, "w1", 60)
        .await
        .expect("something is leasable");
    let second = claim(&app, tenant_b.org, "w2", 60)
        .await
        .expect("the other tenant is leasable");
    assert_ne!(
        first.org, second.org,
        "one live lease per tenant: the second lease must come from the other org"
    );
    let third = claim(&app, tenant_a.org, "w3", 60).await;
    assert!(
        third.is_none(),
        "both tenants hold a live lease, so nothing is leasable"
    );
    for org in [first.org, second.org] {
        assert!(
            expected_orgs.contains(&org),
            "every lease belongs to a seeded tenant"
        );
    }
}

/// Parks the item under a lease and returns nothing but the park's effect, so
/// a revive test reads only what it set up.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn park_leased(
    app: &PgPool,
    engine: &PgPool,
    claimant: DeviceRef<'_>,
    gate: &str,
    park_for: i64,
) {
    // The claim is org-pinned and runs as the app role; the park is the
    // engine's fenced write and runs as the engine role, which needs no pin.
    let leases = LeaseRepo::new(engine.clone());
    let lease = claim(app, claimant.org, claimant.device, 60)
        .await
        .expect("the item leases");
    leases
        .park(&lease.lease_ref(), gate, park_for)
        .await
        .expect("the park is fenced on a live lease");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn states(pool: &PgPool, org: OrgId) -> Vec<(String, Option<String>)> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT state, blocked_on FROM job_item WHERE org_id = $1 ORDER BY id",
    )
    .bind(db_uuid(org.0))
    .fetch_all(pool)
    .await
    .expect("the ledger is readable")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn event_kinds(pool: &PgPool, org: OrgId) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT kind FROM job_event WHERE org_id = $1 ORDER BY org_seq")
        .bind(db_uuid(org.0))
        .fetch_all(pool)
        .await
        .expect("the event stream is readable")
}

#[sqlx::test(migrations = "./migrations")]
async fn an_expired_park_revives_on_a_gate_a_drained_queue_clears(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC1, true).await;
    enqueue_one(&engine, &tenant, 0x51, 0x52).await;
    let leases = LeaseRepo::new(engine.clone());
    park_leased(
        &app,
        &engine,
        DeviceRef {
            org: tenant.org,
            device: "w1",
        },
        "reconciliation",
        1,
    )
    .await;

    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 500), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        0,
        "a park that has not expired is not the unparker's business"
    );
    // The park expiry is the database's own fact now, so a fixture that means
    // to expire one ages the row rather than naming a later instant.
    sqlx::query("UPDATE job_item SET park_expires_at = now() - interval '1 second'")
        .execute(&engine)
        .await
        .expect("the park ages");
    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 2_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        1,
        "an expired projection park requeues into a clean retry"
    );
    assert_eq!(
        states(&engine, tenant.org).await,
        vec![("queued".to_owned(), None)],
        "the gate is cleared with the state, so the item does not re-park on a stale reason"
    );
    assert!(
        event_kinds(&engine, tenant.org)
            .await
            .contains(&"ItemResumed".to_owned()),
        "the ledger says why a parked item is running again"
    );
}

/// The unparker's give-up arm settles the item, and the row it writes must be
/// readable afterwards.
///
/// `job_item` carries no CHECK on `failure_code`, so a spelling the codec does
/// not know is written happily and then fails to decode forever; `items_page`
/// collects the whole page into one `Result`, so a single such row turns every
/// page of that job's items into a fault, for the job the seller is most
/// likely to be inspecting. Reading back through the page is the assertion,
/// because reading the column with raw SQL passes either way.
#[sqlx::test(migrations = "./migrations")]
async fn a_gate_that_never_clears_settles_into_a_row_the_item_page_can_still_read(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC5, true).await;
    let job = JobId(Uuid([0x59; 16]));
    let item = enqueue_operation(&engine, &tenant, 0x59, 0x5A, ItemOperation::Create).await;
    let leases = LeaseRepo::new(engine.clone());
    park_leased(
        &app,
        &engine,
        DeviceRef {
            org: tenant.org,
            device: "w1",
        },
        "reconciliation",
        1,
    )
    .await;

    // The park expiry is the database's own fact now, so a fixture that means
    // to expire one ages the row rather than naming a later instant.
    sqlx::query("UPDATE job_item SET park_expires_at = now() - interval '1 second'")
        .execute(&engine)
        .await
        .expect("the park ages");
    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 2_000), 1)
            .await
            .expect("the unparker runs")
            .requeued,
        0,
        "the give-up arm settles rather than resumes, so nothing is counted as revived"
    );
    let rows = JobReadRepo::new(app.clone())
        .items_page(
            tenant.org,
            job,
            tam_storage::ItemsPageParams {
                cursor: None,
                limit: 10,
                outcome: None,
            },
        )
        .await
        .expect("the item page reads, which is the whole assertion");
    let settled = rows
        .iter()
        .find(|row| row.item == item)
        .expect("the given-up item is on the page");
    assert_eq!(
        (settled.outcome, settled.failure_code),
        (Some(ItemOutcome::Skipped), Some(FailureCode::Other)),
        "the give-up arm's code decodes back to the one the encoder wrote"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_expired_challenge_park_is_left_where_the_driver_put_it(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC2, true).await;
    enqueue_one(&engine, &tenant, 0x53, 0x54).await;
    let leases = LeaseRepo::new(engine.clone());
    // The driver writes the challenge's own debug form here, never a gate.
    park_leased(
        &app,
        &engine,
        DeviceRef {
            org: tenant.org,
            device: "w1",
        },
        "Captcha",
        1,
    )
    .await;

    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 2_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        0,
        "a challenge park still holds an in-flight write attempt, so reviving it would burn \
         the attempt budget and settle the item failed a day later with nothing to explain it"
    );
    assert_eq!(
        states(&engine, tenant.org).await,
        vec![("parked_live".to_owned(), Some("Captcha".to_owned()))],
        "the park survives untouched"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn resolving_one_gap_revives_every_item_parked_behind_it(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC3, true).await;
    // One queue row stands for every product that hit the gap, so the revive
    // has to be keyed on the gate rather than on a mapping.
    for seed in [0x61_u8, 0x63, 0x65] {
        enqueue_one(&engine, &tenant, seed, seed.wrapping_add(1)).await;
    }
    for worker in ["w1", "w2", "w3"] {
        park_leased(
            &app,
            &engine,
            DeviceRef {
                org: tenant.org,
                device: worker,
            },
            "reconciliation",
            86_400,
        )
        .await;
    }

    let mut tx = app.begin().await.expect("the answering transaction opens");
    let revived = revive_by_gap(&mut tx, tenant.org, "reconciliation", T0)
        .await
        .expect("the gap revive runs");
    tx.commit().await.expect("the answer commits");
    assert_eq!(
        revived, 3,
        "a five-hundred-item bulk behind one queue row must not revive one item and leave \
         the other four hundred and ninety-nine to the day-long timer"
    );
    assert!(
        states(&engine, tenant.org)
            .await
            .iter()
            .all(|(state, gate)| state == "queued" && gate.is_none()),
        "every item behind the answered gap is queued"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_election_revive_touches_only_its_own_mapping(app: PgPool) {
    let engine = engine_pool(&app).await;
    let first = seed_tenant(&app, 0xC4, true).await;
    let second = seed_tenant(&app, 0xC8, true).await;
    enqueue_one(&engine, &first, 0x71, 0x72).await;
    enqueue_one(&engine, &second, 0x73, 0x74).await;
    park_leased(
        &app,
        &engine,
        DeviceRef {
            org: first.org,
            device: "w1",
        },
        "election",
        86400,
    )
    .await;
    park_leased(
        &app,
        &engine,
        DeviceRef {
            org: second.org,
            device: "w2",
        },
        "election",
        86400,
    )
    .await;

    let mut tx = app.begin().await.expect("the answering transaction opens");
    let revived = revive_on(&mut tx, first.org, first.mapping, "election", T0)
        .await
        .expect("the election revive runs");
    tx.commit().await.expect("the answer commits");
    assert_eq!(
        revived, 1,
        "an election is one-to-one with a product's mapping"
    );
    assert_eq!(
        states(&engine, second.org).await,
        vec![("parked_live".to_owned(), Some("election".to_owned()))],
        "another tenant's park is untouched, which the pin is what guarantees"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_last_item_to_settle_finishes_the_job_once(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC5, true).await;
    let job = JobId(Uuid([0x81; 16]));
    let mut first = item(0x82);
    first.mapping = tenant.mapping;
    let mut second = item(0x83);
    second.mapping = tenant.mapping;
    JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job,
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[first, second],
        )
        .await
        .expect("the two-item job enqueues");

    let leases = LeaseRepo::new(engine.clone());
    let one = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the first item leases");
    leases
        .settle(&one.lease_ref(), &verdict(ItemOutcome::Succeeded), T0)
        .await
        .expect("the first item settles");
    assert!(
        !event_kinds(&engine, tenant.org)
            .await
            .contains(&"JobSettled".to_owned()),
        "a job with an unsettled item has not finished"
    );

    let two = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the second item leases");
    leases
        .settle(&two.lease_ref(), &verdict(ItemOutcome::Failed), T0)
        .await
        .expect("the second item settles");
    let kinds = event_kinds(&engine, tenant.org).await;
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "JobSettled").count(),
        1,
        "the ledger says a job finished exactly once, without resting on a tenant mutex the \
         design elsewhere names as removable"
    );

    let topics =
        sqlx::query_scalar::<_, String>("SELECT topic FROM outbox_message WHERE org_id = $1")
            .bind(db_uuid(tenant.org.0))
            .fetch_all(&engine)
            .await
            .expect("the outbox is readable");
    assert_eq!(
        topics,
        vec!["email.job_settled".to_owned()],
        "the seller is told, which nothing did before"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_job_whose_last_item_exhausts_its_attempts_still_says_it_finished(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC6, true).await;
    enqueue_one(&engine, &tenant, 0x91, 0x92).await;
    let leases = LeaseRepo::new(engine.clone());
    claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the item leases");

    // The commonest bulk failure: the item dies in the maintenance loop's own
    // cross-tenant statement, with no job context and no worker involved.
    // The lease expiry is the database's own fact now, so a fixture that means
    // to expire one ages the row rather than naming a later instant.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the lease ages");
    let touched = leases
        .expire_and_steal(Timestamp(T0.0 + 61_000), 1)
        .await
        .expect("the stealer runs");
    assert_eq!(
        touched, 1,
        "the attempt budget is exhausted, so the item settles failed"
    );
    assert!(
        event_kinds(&engine, tenant.org)
            .await
            .contains(&"JobSettled".to_owned()),
        "a job that ended this way flipped to Settled with no event to explain it"
    );
}

/// The attempt ledger is fenced by the same steal, at both of its writes.
///
/// `a_stale_worker_is_fenced_after_a_steal` below covers the *item* settle,
/// which compares `job_item.lease_epoch` and always did. These two cover the
/// *attempt* writes, which did not: `write_attempt.lease_epoch` is written at
/// open from the run's own lease and compared at settle against that same one,
/// so it agrees with itself no matter how stale the caller is, and the clause
/// could never bite. Until the epoch was read from the item, a worker whose
/// lease had been stolen could still take the in-flight slot from the device
/// that now owned the work, and could still settle its own standing attempt --
/// binding the mapping on evidence from a run that had already lost the item.
#[sqlx::test(migrations = "./migrations")]
async fn a_stale_holder_cannot_open_an_attempt_after_a_steal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x81, true).await;
    enqueue_one(&engine, &tenant, 0x82, 0x83).await;
    let stale = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the item leases");
    steal_the_lease(&engine).await;

    let refused = WriteAttemptRepo::new(engine.clone())
        .open(
            &stale.lease_ref(),
            Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await;
    assert!(
        matches!(refused, Err(StorageError::StaleLease)),
        "the previous holder must not open an attempt on work it no longer holds: {refused:?}"
    );

    // The positive control, and it is what keeps the predicate from being a
    // refusal of everyone: the device that actually holds the item now opens
    // normally at the bumped epoch.
    let current = claim(&app, tenant.org, "w2", 60)
        .await
        .expect("the stolen item re-leases");
    assert_eq!(
        current.lease_epoch,
        stale.lease_epoch + 1,
        "the steal bumped the epoch, or this proves nothing"
    );
    WriteAttemptRepo::new(engine.clone())
        .open(
            &current.lease_ref(),
            Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the current holder opens");
}

/// And the settle, where the damage would be a bound mapping rather than a
/// taken slot.
///
/// A removal rather than a create, and the reason is worth stating because the
/// first draft of this test used a create and passed for the wrong reason. The
/// reaper's third arm takes a create whose attempt is in flight and whose
/// mapping is unbound and *parks* it rather than stealing it, charged nothing —
/// and a park does not bump `lease_epoch`. So a create in this shape is never
/// stolen at all and there is no stale epoch to fence. A removal falls through
/// to the steal arm, which is the situation this test is about.
#[sqlx::test(migrations = "./migrations")]
async fn a_stale_holder_cannot_settle_its_attempt_after_a_steal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x84, true).await;
    enqueue_operation(
        &engine,
        &tenant,
        0x85,
        0x86,
        ItemOperation::Remove {
            subject: subject(0x86),
            state: ListingState::Live,
        },
    )
    .await;
    let stale = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the item leases");
    let attempt = Uuid(*uuid::Uuid::new_v4().as_bytes());
    let attempts = WriteAttemptRepo::new(engine.clone());
    attempts
        .open(
            &stale.lease_ref(),
            attempt,
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the attempt opens while the lease is still current");
    steal_the_lease(&engine).await;

    let refused = attempts
        .settle(
            &stale.lease_ref(),
            AttemptRef {
                attempt,
                mapping: tenant.mapping,
            },
            &AttemptVerdict {
                state: "committed".to_owned(),
                failure_code: None,
                landing: LandingEffect::Landed {
                    id: RemoteListingId::Tes {
                        url: "https://www.tes.com/teaching-resource/stale-9".to_owned(),
                    },
                    lifecycle: RemoteLifecycle::Draft,
                },
            },
            T0,
        )
        .await;
    assert!(
        matches!(refused, Err(StorageError::StaleLease)),
        "a run that lost its lease must not settle the attempt it left behind: {refused:?}"
    );

    let bound = binding_state(&engine, tenant.org, tenant.mapping).await;
    assert_ne!(
        bound.as_deref(),
        Some("bound"),
        "and the mapping is still unbound, which is the damage rather than the refusal: a \
         binding written here would name a listing on the word of a run that had already \
         lost the item"
    );

    // The control D asked for cannot exist, and the reason is worth asserting
    // rather than leaving as an absence: the device that re-leases the item
    // after the steal is refused too. `write_attempt.lease_epoch` is the epoch
    // the attempt was opened under, and `settle` compares it against the
    // caller's own, so only a caller at that same epoch can settle that row --
    // which the steal has just made impossible for everyone. That is
    // pre-existing and by design, not something the epoch check introduced:
    // the clause predates it and refused the new holder before it existed too.
    //
    // What settles a stranded attempt is therefore not this call at all. It is
    // the reaper's own arms, which update `write_attempt` in SQL without a
    // lease -- `expire_and_steal`'s settle arm and `revive_expired`'s re-link
    // arm both do it, and they are the only things that can.
    let current = claim(&app, tenant.org, "w2", 60)
        .await
        .expect("the stolen item re-leases");
    assert_eq!(
        current.lease_epoch,
        stale.lease_epoch + 1,
        "the steal bumped the epoch, or nothing below means anything"
    );
    let also_refused = attempts
        .settle(
            &current.lease_ref(),
            AttemptRef {
                attempt,
                mapping: tenant.mapping,
            },
            &AttemptVerdict {
                state: "committed".to_owned(),
                failure_code: None,
                landing: LandingEffect::Landed {
                    id: RemoteListingId::Tes {
                        url: "https://www.tes.com/teaching-resource/stale-9".to_owned(),
                    },
                    lifecycle: RemoteLifecycle::Draft,
                },
            },
            T0,
        )
        .await;
    assert!(
        matches!(also_refused, Err(StorageError::StaleLease)),
        "the new holder cannot settle the old attempt either, so the fence refuses a stale \
         caller rather than choosing between two live ones: {also_refused:?}"
    );
    assert_eq!(
        binding_state(&engine, tenant.org, tenant.mapping)
            .await
            .as_deref(),
        Some("unbound"),
        "and the mapping stays unbound throughout, which is the property the whole fence \
         exists for"
    );
}

/// One mapping's binding state, named rather than read as whichever row comes
/// first: a fixture that grows a second mapping would otherwise start
/// asserting about the wrong one without failing.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn binding_state(engine: &PgPool, org: OrgId, mapping: MappingId) -> Option<String> {
    sqlx::query_scalar("SELECT binding_state FROM mapping WHERE org_id = $1 AND id = $2")
        .bind(db_uuid(org.0))
        .bind(db_uuid(mapping.0))
        .fetch_one(engine)
        .await
        .expect("the mapping row reads")
}

/// Ages the live lease past its expiry and runs the stealer, which is what
/// bumps `job_item.lease_epoch` out from under whoever held it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn steal_the_lease(engine: &PgPool) {
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(engine)
        .await
        .expect("the lease ages");
    let touched = LeaseRepo::new(engine.clone())
        .expire_and_steal(Timestamp(T0.0 + 61_000), 5)
        .await
        .expect("the stealer runs");
    assert_eq!(
        touched, 1,
        "the expired lease is stolen, or the fixture proves nothing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_stale_worker_is_fenced_after_a_steal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    enqueue_one(&engine, &tenant, 0x11, 0x21).await;

    let leases = LeaseRepo::new(engine.clone());
    let lease = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the item leases");
    steal_the_lease(&engine).await;
    let after_expiry = Timestamp(T0.0 + 61_000);

    let stale = leases
        .settle(
            &lease.lease_ref(),
            &verdict(ItemOutcome::Succeeded),
            after_expiry,
        )
        .await;
    assert!(
        matches!(stale, Err(StorageError::StaleLease)),
        "the previous holder's write must be fenced out, not raced"
    );

    let release = claim(&app, tenant.org, "w2", 60)
        .await
        .expect("the stolen item re-leases");
    assert_eq!(
        release.lease_epoch,
        lease.lease_epoch + 1,
        "the steal bumped the epoch"
    );
    leases
        .settle(
            &release.lease_ref(),
            &verdict(ItemOutcome::Succeeded),
            after_expiry,
        )
        .await
        .expect("the current epoch settles");
}

#[sqlx::test(migrations = "./migrations")]
async fn halts_and_the_connection_gate_fail_closed(app: PgPool) {
    let engine = engine_pool(&app).await;
    let halted = seed_tenant(&app, 0xA0, true).await;
    let unlinked = seed_tenant(&app, 0xB0, false).await;
    enqueue_one(&engine, &halted, 0x11, 0x21).await;
    enqueue_one(&engine, &unlinked, 0x12, 0x22).await;

    HaltRepo::new(engine.clone())
        .raise_org_inventory(
            halted.org,
            InventoryId::TesGb,
            &HaltCause {
                raised_by: "test".to_owned(),
                reason: "ambiguity".to_owned(),
                at: T0,
            },
        )
        .await
        .expect("the halt raises");

    let leases = LeaseRepo::new(engine.clone());
    let nothing = claim(&app, halted.org, "w1", 60).await;
    assert!(
        nothing.is_none(),
        "one tenant is halted and the other has no linked connection; both must be refused"
    );

    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(unlinked.org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(db_uuid(Uuid([0xB0; 16])))
    .bind(db_uuid(Uuid([0xB4; 16])))
    .execute(&mut *tx)
    .await
    .expect("linking the connection");
    tx.commit().await.expect("the link commits");
    // The device claim is org-pinned, so each tenant is asked separately
    // rather than one cross-tenant scan choosing between them. The gate's
    // meaning is unchanged: the halted tenant is still refused, and the one
    // whose connection was just linked is not.
    assert!(
        claim(&app, halted.org, "w1", 60).await.is_none(),
        "the halt still stands, so that tenant is still refused"
    );
    let leased = claim(&app, unlinked.org, "w2", 60)
        .await
        .expect("the linked tenant now leases");
    assert_eq!(leased.org, unlinked.org, "only the linked tenant leases");

    leases
        .gate_connection(leased.org, InventoryId::TesGb, T0)
        .await
        .expect("the gate flips");
    // F10: the gate is a lifecycle event, and the connection row alone cannot
    // say it happened, only that it is currently gated.
    let gate_trail = ConnectionAudit::new(engine.clone())
        .history(leased.org, connection_of(&engine, leased.org).await)
        .await
        .expect("the lifecycle audit reads back");
    assert_eq!(
        gate_trail
            .iter()
            .map(|row| (
                row.event.as_str(),
                row.actor_kind.as_str(),
                row.actor_id.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![("needs_reauth", "system", Some("engine"))],
        "the gate records itself, attributed to the engine that closed it"
    );
    assert_eq!(
        gate_trail[0].detail.as_deref(),
        Some("tes_gb"),
        "and names the inventory whose failure caused it"
    );
    leases
        .settle(&leased.lease_ref(), &verdict(ItemOutcome::Blocked), T0)
        .await
        .expect("the item settles");
    enqueue_one(&engine, &unlinked, 0x13, 0x23).await;
    let gated = claim(&app, halted.org, "w1", 60).await;
    assert!(
        gated.is_none(),
        "needs_reauth is the gate: nothing leases behind it"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_attempt_budget_settles_failed_rather_than_looping(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    let item_id = enqueue_one(&engine, &tenant, 0x11, 0x21).await;

    let leases = LeaseRepo::new(engine.clone());
    claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the item leases");
    // The lease expiry is the database's own fact now, so a fixture that means
    // to expire one ages the row rather than naming a later instant.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the lease ages");
    let touched = leases
        .expire_and_steal(Timestamp(T0.0 + 61_000), 1)
        .await
        .expect("the stealer runs");
    assert_eq!(touched, 1, "the item is at its attempt cap");

    let state: (String, Option<String>) =
        sqlx::query_as("SELECT state, outcome FROM job_item WHERE org_id = $1 AND id = $2")
            .bind(db_uuid(tenant.org.0))
            .bind(db_uuid(item_id.0))
            .fetch_one(&engine)
            .await
            .expect("the item row reads");
    assert_eq!(
        (state.0.as_str(), state.1.as_deref()),
        ("settled", Some("failed")),
        "an exhausted attempt budget settles as failed, it does not requeue forever"
    );
}

/// The adapter's free text is the only human-readable account of a refusal
/// the operator ever sees; a settle that drops it leaves the item page with
/// a code and nothing else.
#[sqlx::test(migrations = "./migrations")]
async fn a_rejected_settle_carries_its_failure_detail(app: PgPool) {
    const DETAIL: &str = "the upload was refused: the file exceeds the size cap";
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    let job = JobId(Uuid([0x11; 16]));
    let mut first_item = item(0x21);
    first_item.mapping = tenant.mapping;
    let mut second_item = item(0x22);
    second_item.mapping = tenant.mapping;
    let (rejected, succeeded) = (first_item.item, second_item.item);
    JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job,
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[first_item, second_item],
        )
        .await
        .expect("the fixture job enqueues");

    let leases = LeaseRepo::new(engine.clone());
    let first = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the first item leases");
    assert_eq!(first.item, rejected, "the oldest item leases first");
    leases
        .settle(
            &first.lease_ref(),
            &ItemVerdict {
                outcome: ItemOutcome::Failed,
                failure_code: Some(FailureCode::UploadRejected),
                failure_detail: Some(FailureDetail(DETAIL.to_owned())),
            },
            T0,
        )
        .await
        .expect("the rejected item settles");

    let second = claim(&app, tenant.org, "w2", 60)
        .await
        .expect("the second item leases");
    assert_eq!(
        second.item, succeeded,
        "the tenant's next item leases once the first has settled"
    );
    leases
        .settle(&second.lease_ref(), &verdict(ItemOutcome::Succeeded), T0)
        .await
        .expect("the succeeded item settles");

    let rows = JobReadRepo::new(app.clone())
        .items_page(
            tenant.org,
            job,
            tam_storage::ItemsPageParams {
                cursor: None,
                limit: 10,
                outcome: None,
            },
        )
        .await
        .expect("the item page reads");
    let failed = rows
        .iter()
        .find(|row| row.item == rejected)
        .expect("the rejected item is on the page");
    assert_eq!(
        failed.failure_detail.as_deref(),
        Some(DETAIL),
        "the adapter's free text must reach the column the API surfaces"
    );
    let clean = rows
        .iter()
        .find(|row| row.item == succeeded)
        .expect("the succeeded item is on the page");
    assert_eq!(
        clean.failure_detail, None,
        "a settle with no rejection leaves the detail null"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_reused_idempotency_key_is_named(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    enqueue_one(&engine, &tenant, 0x11, 0x21).await;

    let mut duplicate = item(0x21);
    duplicate.item = JobItemId(Uuid([0x99; 16]));
    duplicate.mapping = tenant.mapping;
    let refused = JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job: JobId(Uuid([0x14; 16])),
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            std::slice::from_ref(&duplicate),
        )
        .await;
    assert!(
        matches!(refused, Err(StorageError::DuplicateIdempotencyKey { .. })),
        "the database backstop names the duplicate rather than racing it"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_outbox_deduplicates_backs_off_and_dead_letters(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    let outbox = OutboxRepo::new(engine.clone());

    for id_byte in [0x61u8, 0x62] {
        let mut tx = engine.begin().await.expect("transaction begins");
        tam_storage::OutboxRepo::append(
            &mut tx,
            &NewOutboxMessage {
                org: tenant.org,
                id: Uuid([id_byte; 16]),
                topic: "email.parked_job".to_owned(),
                dedupe_key: "job-1".to_owned(),
                payload: serde_json::json!({"job": "1"}),
                at: T0,
            },
        )
        .await
        .expect("the append runs");
        tx.commit().await.expect("the append commits");
    }
    let due = outbox.claim_due(T0, 10).await.expect("the claim runs");
    assert_eq!(
        due.len(),
        1,
        "two appends with one dedupe key are one message"
    );
    let message = &due[0];

    let retried = outbox
        .retry_later(
            &message.reference(),
            Timestamp(T0.0 + 30_000),
            "relay refused",
        )
        .await
        .expect("the retry records");
    assert!(
        retried,
        "the compare-and-set matches the seen attempt count"
    );
    assert!(
        outbox
            .claim_due(T0, 10)
            .await
            .expect("the claim runs")
            .is_empty(),
        "a backed-off message is not due until its instant"
    );

    let due_later = outbox
        .claim_due(Timestamp(T0.0 + 31_000), 10)
        .await
        .expect("the claim runs");
    assert_eq!(due_later.len(), 1, "the backoff elapsed");
    let dead = outbox
        .mark_dead(&due_later[0].reference(), "poison")
        .await
        .expect("the dead-letter records");
    assert!(dead, "the message quarantines by attempt count");
    assert!(
        outbox
            .claim_due(Timestamp(T0.0 + 60_000), 10)
            .await
            .expect("the claim runs")
            .is_empty(),
        "a dead message is never claimed again"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn org_seq_is_consecutive_per_organisation(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant_a = seed_tenant(&app, 0xA0, true).await;
    let tenant_b = seed_tenant(&app, 0xB0, true).await;
    enqueue_one(&engine, &tenant_a, 0x11, 0x21).await;
    enqueue_one(&engine, &tenant_b, 0x12, 0x22).await;
    enqueue_one(&engine, &tenant_a, 0x13, 0x23).await;

    let seqs: Vec<(uuid::Uuid, i64)> =
        sqlx::query_as("SELECT org_id, org_seq FROM job_event ORDER BY org_id, org_seq")
            .fetch_all(&engine)
            .await
            .expect("the events read");
    for org in [tenant_a.org, tenant_b.org] {
        let own: Vec<i64> = seqs
            .iter()
            .filter(|(event_org, _)| *event_org == db_uuid(org.0))
            .map(|(_, seq)| *seq)
            .collect();
        let count = i64::try_from(own.len()).expect("event counts fit i64");
        let expected: Vec<i64> = (1..=count).collect();
        assert_eq!(
            own, expected,
            "org_seq must be consecutive from one within each organisation"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_rate_budget_grants_then_exhausts(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    let connection: uuid::Uuid = sqlx::query_scalar("SELECT id FROM connection WHERE org_id = $1")
        .bind(db_uuid(tenant.org.0))
        .fetch_one(&engine)
        .await
        .expect("the fixture connection reads");
    let budget = RateBudgetRepo::new(engine.clone());
    let window = T0;
    let connection = ConnectionId(Uuid(*connection.as_bytes()));
    for expected in 1..=2 {
        let grant = budget
            .consume(tenant.org, connection, window, 2)
            .await
            .expect("the consume runs");
        assert_eq!(
            grant,
            BudgetGrant::Granted { used: expected },
            "the window counts up to its ceiling"
        );
    }
    let refused = budget
        .consume(tenant.org, connection, window, 2)
        .await
        .expect("the consume runs");
    assert_eq!(
        refused,
        BudgetGrant::Exhausted,
        "the governor refuses rather than errors at the ceiling"
    );
}

/// The whole operation survives the ledger: not only which of the three it
/// is, but the listing it names and the transition it states. A `RETURNING`
/// list that stops at `operation` would read a revise back with no subject
/// and no ends, and the lift would have nothing to validate against the
/// binding.
#[sqlx::test(migrations = "./migrations")]
async fn an_enqueued_removal_leases_as_a_removal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let remover = seed_tenant(&app, 0xA0, true).await;
    let reviser = seed_tenant(&app, 0xB0, true).await;
    let removal = ItemOperation::Remove {
        subject: subject(0x21),
        state: ListingState::Live,
    };
    let revision = ItemOperation::Revise {
        subject: subject(0x22),
        transition: LifecycleTransition {
            from: ListingState::Draft,
            to: ListingState::Live,
        },
    };
    enqueue_operation(&engine, &remover, 0x11, 0x21, removal.clone()).await;
    enqueue_operation(&engine, &reviser, 0x12, 0x22, revision.clone()).await;

    let first = claim(&app, remover.org, "w1", 60)
        .await
        .expect("something is leasable");
    let second = claim(&app, reviser.org, "w2", 60)
        .await
        .expect("the other tenant is leasable");
    for leased in [first, second] {
        let expected = if leased.org == remover.org {
            &removal
        } else {
            &revision
        };
        assert_eq!(
            &leased.operation, expected,
            "the leased item must carry the operation, subject and transition it was enqueued with"
        );
    }
}

/// The request-idempotency-key path is the one the API actually enqueues
/// through, and it is a second `INSERT INTO job_item`. Both route through one
/// helper; were they to drift, a removal enqueued here would fail the NOT
/// NULL that migration 0019 leaves behind when it drops the default, rather
/// than lease as a create and run as a second listing.
#[sqlx::test(migrations = "./migrations")]
async fn a_removal_enqueued_under_a_request_key_leases_as_a_removal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    let removal = ItemOperation::Remove {
        subject: subject(0x31),
        state: ListingState::Draft,
    };
    let mut new_item = item(0x31);
    new_item.mapping = tenant.mapping;
    new_item.operation = removal.clone();
    let created = JobRepo::new(engine.clone())
        .create_with_request_key(
            tenant.org,
            Uuid([0x71; 16]),
            &NewJob {
                job: JobId(Uuid([0x15; 16])),
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            std::slice::from_ref(&new_item),
        )
        .await
        .expect("the request-keyed job enqueues");
    assert!(
        !created.replay,
        "the first carrier of the key creates a job"
    );

    let leased = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the enqueued removal is leasable");
    assert_eq!(
        leased.operation, removal,
        "an item enqueued through the request-key path leases as what it was enqueued as"
    );
}

/// One settler's own view of the job, taken with the item it settled already
/// written and not yet committed -- which is what the other settler sees a
/// snapshot of.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn settle_row(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: OrgId, item: JobItemId) {
    sqlx::query(
        "UPDATE job_item SET state = 'settled', outcome = 'succeeded', settled_at = now() \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(item.0))
    .execute(&mut **tx)
    .await
    .expect("the item settles");
}

/// The completeness count runs behind the organisation's event lock.
///
/// It used to run in front of it. `settle_if_complete` returns before
/// `allocate_org_seq` when it counts an unsettled sibling, so two
/// transactions settling a job's last two items each counted a snapshot
/// without the other's uncommitted write, both returned early, and the job
/// finished with every item settled, no `JobSettled` in the ledger and no
/// `email.job_settled` for the seller -- while `phase_of` read it as settled.
/// Two worker processes per tenant is the designed configuration, and
/// `job_item_one_live_lease_per_org` does not serialise them: it covers the
/// live states a parked row does not have, and `revive_expired` settles
/// parked rows from the maintenance loop.
///
/// Witnessed rather than raced, so the assertion is deterministic: a third
/// connection asking for the lock without waiting is refused for as long as
/// the first settler's transaction is open, and being refused is precisely
/// what makes the second settler's count see the first's settle.
#[sqlx::test(migrations = "./migrations")]
async fn the_completeness_count_holds_the_organisation_event_lock(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xD5, true).await;
    let job = JobId(Uuid([0x91; 16]));
    let mut first = item(0x92);
    first.mapping = tenant.mapping;
    let mut second = item(0x93);
    second.mapping = tenant.mapping;
    JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job,
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[first, second],
        )
        .await
        .expect("the two-item job enqueues");

    let mut settler = engine.begin().await.expect("the first settler opens");
    settle_row(&mut settler, tenant.org, JobItemId(Uuid([0x92; 16]))).await;
    assert!(
        !settle_if_complete(&mut settler, tenant.org, job, T0)
            .await
            .expect("the first settle runs"),
        "a job with an unsettled sibling has not finished"
    );

    let refused =
        sqlx::query("SELECT next_seq FROM org_event_counter WHERE org_id = $1 FOR UPDATE NOWAIT")
            .bind(db_uuid(tenant.org.0))
            .fetch_optional(&engine)
            .await
            .map(|_| ());
    assert!(
        refused.is_err(),
        "a settler that decided nothing still holds the lock the other settler must take \
         before its own count, or both count a snapshot missing the other's write: {refused:?}"
    );
    settler.commit().await.expect("the first settle commits");

    let mut last = engine.begin().await.expect("the second settler opens");
    settle_row(&mut last, tenant.org, JobItemId(Uuid([0x93; 16]))).await;
    assert!(
        settle_if_complete(&mut last, tenant.org, job, T0)
            .await
            .expect("the second settle runs"),
        "the settler that takes the lock last is the one that finishes the job"
    );
    last.commit().await.expect("the second settle commits");

    let kinds = event_kinds(&engine, tenant.org).await;
    assert_eq!(
        kinds.iter().filter(|kind| *kind == "JobSettled").count(),
        1,
        "exactly one, and never none"
    );
}

/// A second inventory for a product this org already lists, which is the
/// cross-listing shape rather than a second fixture: `mapping_one_per_inventory`
/// is keyed on the product and the inventory, so one product carries one
/// mapping per marketplace.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed_mapping_on(
    app: &PgPool,
    tenant: &Tenant,
    seed: u8,
    inventory: InventoryId,
) -> MappingId {
    let mapping = MappingId(Uuid([seed; 16]));
    MappingRepo::new(app.clone())
        .insert(
            tenant.org,
            &Mapping {
                id: mapping,
                org: tenant.org,
                product: tenant.product,
                inventory,
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
            T0,
        )
        .await
        .expect("the second-inventory mapping inserts");
    mapping
}

/// A linked connection for a marketplace the fixture tenant did not seed one
/// for. Written over the app role behind a tenant pin, the way `seed_tenant`
/// does: `tam_engine` may update `connection` -- that is what
/// `gate_connection` is -- but it holds no INSERT on the table.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn link_connection(app: &PgPool, org: OrgId, marketplace: &str, seed: u8) {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, $3, 'linked', now(), now())",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(Uuid([seed; 16])))
    .bind(marketplace)
    .execute(&mut *tx)
    .await
    .expect("the fixture connection inserts");
    tx.commit().await.expect("the fixture connection commits");
}

/// The re-link itself, which no repository method models: the broker owns
/// that write, and this test stands in for it with the state change it makes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn relink(engine: &PgPool, org: OrgId, marketplace: &str) {
    let flipped = sqlx::query(
        "UPDATE connection SET state = 'linked', updated_at = now() \
         WHERE org_id = $1 AND marketplace = $2",
    )
    .bind(db_uuid(org.0))
    .bind(marketplace)
    .execute(engine)
    .await
    .expect("the re-link applies");
    assert_eq!(
        flipped.rows_affected(),
        1,
        "the fixture must re-link exactly the connection under test"
    );
}

/// One fixture enqueue: which listing, seeded how, onto which inventory, doing
/// what. Grouped because the cross-marketplace tests vary all of them together.
#[derive(Debug, Clone)]
struct EnqueueOnto {
    mapping: MappingId,
    job_seed: u8,
    item_seed: u8,
    inventory: InventoryId,
    operation: ItemOperation,
}

/// Enqueues onto a stated inventory, which `enqueue_operation` cannot: it
/// fixes `InventoryId::TesGb`, and the marketplace the job carries is what
/// the re-link arm joins the connection on.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn enqueue_on(engine: &PgPool, tenant: &Tenant, onto: &EnqueueOnto) -> JobItemId {
    let EnqueueOnto {
        mapping,
        job_seed,
        item_seed,
        inventory,
        operation,
    } = onto.clone();
    let mut new_item = item(item_seed);
    new_item.mapping = mapping;
    new_item.operation = operation;
    JobRepo::new(engine.clone())
        .enqueue(
            tenant.org,
            &NewJob {
                job: JobId(Uuid([job_seed; 16])),
                inventory,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            std::slice::from_ref(&new_item),
        )
        .await
        .expect("the fixture job enqueues");
    new_item.item
}

/// Leases the next item, opens its write attempt, and parks it on the gate a
/// session that died mid-submit writes. This is the driver's own order --
/// `RecordIntent` opens the attempt, the submit fails `SessionExpired`, and
/// `SyncMachine::park` advances with that attempt still `in_flight` -- which
/// is the whole reason the park needs releasing rather than merely reviving.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn park_mid_submit(
    app: &PgPool,
    engine: &PgPool,
    org: OrgId,
    worker: &str,
    mapping: MappingId,
) -> JobItemId {
    let leases = LeaseRepo::new(engine.clone());
    let lease = claim(app, org, worker, 60).await.expect("the item leases");
    assert_eq!(
        lease.mapping, mapping,
        "the fixture depends on FIFO order, so the expected item must be the one leased"
    );
    WriteAttemptRepo::new(engine.clone())
        .open(
            &lease.lease_ref(),
            // Fresh per call: `open` is idempotent on the id now, so a fixture
            // reusing one would silently skip the second mapping's attempt.
            tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the attempt opens");
    leases
        .park(&lease.lease_ref(), REAUTH_REQUIRED, 1)
        .await
        .expect("the park is fenced on a live lease");
    lease.item
}

/// The shape `intent_as_json` writes for a create: the rendered field set as
/// pairs, which is where the claim reads the recorded title from.
const RECORDED_TITLE: &str = "Fractions pack";

/// The pairs as the driver encodes them, built from real `FieldKey` values
/// rather than spelt out here.
///
/// The claim matches on the literal `'Title'`, which is `FieldKey`'s serde
/// spelling and nothing else; a fixture that wrote that string by hand would
/// keep passing if the enum were renamed or given a `rename_all`, and the
/// claim would silently stop finding any title. Going through `serde_json`
/// over the real values ties the two together.
///
/// Description first on purpose: with the title in front, a claim that
/// dropped its `entry->>0 = 'Title'` filter and simply took the first pair
/// would still pass.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn recorded_entries() -> serde_json::Value {
    serde_json::to_value(vec![
        (FieldKey::Description, "A worksheet.".to_owned()),
        (FieldKey::Title, RECORDED_TITLE.to_owned()),
    ])
    .expect("a field set encodes")
}

fn intent() -> AttemptIntent {
    AttemptIntent {
        body: serde_json::json!({
            "operation": "create",
            "entries": recorded_entries(),
            "files": [],
        }),
        hash: vec![0x01; 32],
    }
}

async fn attempt_states(engine: &PgPool, org: OrgId, mapping: MappingId) -> Vec<String> {
    attempt_rows(engine, org, mapping)
        .await
        .into_iter()
        .map(|row| row.0)
        .collect()
}

/// The attempt's state beside the listing it names. Both together, because
/// the state alone cannot tell a faithful release from one that dropped the
/// subject: `WriteAttemptRepo::settle` writes these three columns from
/// `addressed_by`, and the re-link arm has to reproduce that or the ledger's
/// "every settled row names what the write was about" stops holding on this
/// path.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn attempt_rows(
    engine: &PgPool,
    org: OrgId,
    mapping: MappingId,
) -> Vec<(String, Option<String>, Option<String>, Option<i64>)> {
    sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<i64>)>(
        "SELECT state, remote_id_kind, remote_url, remote_numeric_id \
         FROM write_attempt WHERE org_id = $1 AND mapping_id = $2 ORDER BY opened_at",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(mapping.0))
    .fetch_all(engine)
    .await
    .expect("the attempt rows read")
}

/// The park a re-link is the only thing that clears.
///
/// `ReauthRequired` is outside `REVIVABLE_GATES` because that list is
/// time-gated, so before this arm existed the item sat `parked_live` forever
/// and the job it belonged to read active forever: the expiry pass neither
/// revived it nor reached the give-up arm, and re-linking the connection did
/// nothing to it.
///
/// The attempt assertions are what make this severe rather than merely green.
/// Reviving the item alone would leave its `in_flight` attempt standing, and
/// `write_attempt_one_in_flight` would then refuse `AttemptRepo::open` on
/// every subsequent pass until the budget settled the item `failed` with
/// nothing in the ledger -- a state in which "the item leases again" is still
/// true and the fix is still absent.
#[sqlx::test(migrations = "./migrations")]
async fn a_relink_revives_the_park_the_clock_can_never_clear(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC8, true).await;
    let tpt_mapping = seed_mapping_on(&app, &tenant, 0xD1, InventoryId::Tpt).await;
    link_connection(&app, tenant.org, "tpt", 0xD2).await;
    let revision = ItemOperation::Revise {
        subject: subject(0x71),
        transition: LifecycleTransition {
            from: ListingState::Draft,
            to: ListingState::Live,
        },
    };
    let tes_item = enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tenant.mapping,
            job_seed: 0x71,
            item_seed: 0x72,
            inventory: InventoryId::TesGb,
            operation: revision.clone(),
        },
    )
    .await;
    let tpt_item = enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tpt_mapping,
            job_seed: 0x73,
            item_seed: 0x74,
            inventory: InventoryId::Tpt,
            operation: revision,
        },
    )
    .await;

    let leases = LeaseRepo::new(engine.clone());
    assert_eq!(
        park_mid_submit(&app, &engine, tenant.org, "w1", tenant.mapping).await,
        tes_item
    );
    assert_eq!(
        park_mid_submit(&app, &engine, tenant.org, "w2", tpt_mapping).await,
        tpt_item
    );
    // What `Effect::RequeueBehindGate` does the moment the machine parks.
    leases
        .gate_connection(tenant.org, InventoryId::TesGb, T0)
        .await
        .expect("the tes connection gates");
    leases
        .gate_connection(tenant.org, InventoryId::Tpt, T0)
        .await
        .expect("the tpt connection gates");

    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 2_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        0,
        "the connection is still unusable, so the park stands however long the clock runs"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tenant.mapping).await,
        vec!["in_flight".to_owned()],
        "nothing is released while the item stays parked"
    );

    relink(&engine, tenant.org, "tes").await;
    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 3_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        1,
        "the re-linked marketplace's item requeues, and only that one"
    );
    let RemoteListingId::Tes { url } = subject(0x71) else {
        panic!("the fixture subject is a tes listing")
    };
    assert_eq!(
        attempt_rows(&engine, tenant.org, tenant.mapping).await,
        vec![(
            "abandoned".to_owned(),
            Some("tes".to_owned()),
            Some(url),
            None
        )],
        "the stranded attempt is settled in the same transaction -- which is what frees the \
         mapping for the next run -- and it still names the listing the revise addressed, \
         exactly as `addressed_by` would have written it"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tpt_mapping).await,
        vec!["in_flight".to_owned()],
        "a marketplace that was not re-linked keeps both its park and its fence"
    );

    let resumed = claim(&app, tenant.org, "w3", 60)
        .await
        .expect("the revived item leases again");
    assert_eq!(
        resumed.item, tes_item,
        "the item that leases is the one the re-link revived"
    );
    WriteAttemptRepo::new(engine.clone())
        .open(
            &resumed.lease_ref(),
            tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect(
            "a fresh attempt opens: without the release this is where the revived run would \
             have abandoned on write_attempt_one_in_flight, every pass, until the budget ran out",
        );

    let parked: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT state, blocked_on FROM job_item WHERE org_id = $1 AND id = $2")
            .bind(db_uuid(tenant.org.0))
            .bind(db_uuid(tpt_item.0))
            .fetch_all(&engine)
            .await
            .expect("the ledger is readable");
    assert_eq!(
        parked,
        vec![("parked_live".to_owned(), Some(REAUTH_REQUIRED.to_owned()))],
        "the re-link arm is scoped to the marketplace that was re-linked, so a sibling parked \
         on another one is untouched"
    );
}

/// A create is the one operation the re-link cannot resume, and it stays
/// parked deliberately.
///
/// `write_attempt_one_in_flight` is the only fence between a requeued create
/// and a second listing on the seller's store, and neither adapter offers an
/// idempotent create. A submit that died on `SessionExpired` may or may not
/// have landed, so releasing the fence here would risk the one failure this
/// ledger cannot undo. The resolution a create needs is a read-back that
/// settles on what is actually on the marketplace, which this scan cannot do.
#[sqlx::test(migrations = "./migrations")]
async fn a_relinked_create_stays_parked_behind_its_own_duplicate_fence(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC9, true).await;
    let created = enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tenant.mapping,
            job_seed: 0x81,
            item_seed: 0x82,
            inventory: InventoryId::TesGb,
            operation: ItemOperation::Create,
        },
    )
    .await;

    let leases = LeaseRepo::new(engine.clone());
    assert_eq!(
        park_mid_submit(&app, &engine, tenant.org, "w1", tenant.mapping).await,
        created
    );
    leases
        .gate_connection(tenant.org, InventoryId::TesGb, T0)
        .await
        .expect("the connection gates");
    relink(&engine, tenant.org, "tes").await;
    // The park age is stated rather than left to elapse. The park-age arm
    // moves an aged-out create to `awaiting_seller_signin`, and this test is
    // about the re-link arm leaving a create alone, so a park that expired
    // while the test ran would assert the wrong thing intermittently.
    sqlx::query("UPDATE job_item SET park_expires_at = now() + interval '1 day'")
        .execute(&engine)
        .await
        .expect("the park is held open for the duration of this test");

    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 3_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        0,
        "a create is excluded from the re-link arm, so the re-link revives nothing"
    );
    assert_eq!(
        states(&engine, tenant.org).await,
        vec![("parked_live".to_owned(), Some(REAUTH_REQUIRED.to_owned()))],
        "the create stays where the driver put it"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tenant.mapping).await,
        vec!["in_flight".to_owned()],
        "and its attempt keeps standing, which is the fence doing its job"
    );
}

/// The tenant's connection id, for assertions about the lifecycle audit.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn connection_of(pool: &sqlx::PgPool, org: OrgId) -> tam_types::ConnectionId {
    let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM connection WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .fetch_one(pool)
        .await
        .expect("the fixture linked exactly one connection");
    tam_types::ConnectionId(tam_types::Uuid(*id.as_bytes()))
}

/// A publish addressed no listing, and the release must not invent one.
///
/// `ItemOperation::subject()` answers `None` for a publish -- the id it will
/// name is resolved from the binding at lease time rather than stated at
/// enqueue -- so `landing_effect` maps an uncommitted publish to
/// `LandingEffect::None` and the driver's own settle leaves the three remote
/// columns NULL. The re-link arm copies `job_item`'s subject columns, which
/// `job_item_operation_total` keeps NULL for a publish, so the two agree.
/// Asserted rather than assumed: a copy that reached for the binding instead
/// would put a listing on a settled row that never addressed one, and a
/// publish is the only non-create shape where the distinction shows.
#[sqlx::test(migrations = "./migrations")]
async fn a_released_publish_names_no_listing(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xCA, true).await;
    let published = enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tenant.mapping,
            job_seed: 0x91,
            item_seed: 0x92,
            inventory: InventoryId::TesGb,
            operation: ItemOperation::Publish {
                to: ListingState::Live,
            },
        },
    )
    .await;

    let leases = LeaseRepo::new(engine.clone());
    assert_eq!(
        park_mid_submit(&app, &engine, tenant.org, "w1", tenant.mapping).await,
        published
    );
    leases
        .gate_connection(tenant.org, InventoryId::TesGb, T0)
        .await
        .expect("the connection gates");
    relink(&engine, tenant.org, "tes").await;

    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 3_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs")
            .requeued,
        1,
        "a publish is not a create, so the re-link resumes it"
    );
    assert_eq!(
        attempt_rows(&engine, tenant.org, tenant.mapping).await,
        vec![("abandoned".to_owned(), None, None, None)],
        "a publish addressed no listing, so its released attempt names none either"
    );
}

/// D1's build-failing rule, bound to a database fact.
///
/// `Marketplace::transport_class()` is the source of truth for which branch of
/// the automation rule a marketplace falls in, and the claim statement filters
/// on `marketplace_inventory.transport_class`. Nothing keeps a Rust match and a
/// SQL column agreeing except this test, so a no-API marketplace given a server
/// transport in either place fails the build here rather than shipping.
#[sqlx::test(migrations = "./migrations")]
async fn every_marketplace_transport_class_matches_the_rust_source_of_truth(app: PgPool) {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT DISTINCT marketplace, transport_class FROM marketplace_inventory")
            .fetch_all(&app)
            .await
            .expect("the inventory table reads");
    assert!(
        !rows.is_empty(),
        "an empty table would make every assertion below vacuous"
    );
    for (marketplace, stored) in rows {
        let declared = match marketplace.as_str() {
            "tes" => Marketplace::Tes,
            "tpt" => Marketplace::Tpt,
            "etsy" => Marketplace::Etsy,
            other => panic!("the table names a marketplace Rust does not: {other}"),
        };
        let expected = match declared.transport_class() {
            TransportClass::SellerDevice => "seller_device",
            TransportClass::OfficialApi => "official_api",
        };
        assert_eq!(
            stored, expected,
            "{marketplace}: the column and Marketplace::transport_class() disagree, which is \
             the two-branch rule having become decoration"
        );
    }
}

/// Every marketplace the type system knows is represented in the table, so the
/// test above cannot pass by simply not covering one.
#[sqlx::test(migrations = "./migrations")]
async fn every_declared_marketplace_has_a_transport_class_row(app: PgPool) {
    let present: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT marketplace FROM marketplace_inventory")
            .fetch_all(&app)
            .await
            .expect("the inventory table reads");
    for marketplace in [Marketplace::Tes, Marketplace::Tpt, Marketplace::Etsy] {
        let wire = match marketplace {
            Marketplace::Tes => "tes",
            Marketplace::Tpt => "tpt",
            Marketplace::Etsy => "etsy",
        };
        assert!(
            present.iter().any(|row| row == wire),
            "{wire} carries a transport class in Rust and no row in the table, so the \
             equality test would never see it"
        );
    }
}

/// The mutex re-scope, contending half.
///
/// Two inventories of one marketplace share one seller login, so they share one
/// live-lease slot. This is the invariant the index protects, stated as the
/// behaviour a second claimant sees.
#[sqlx::test(migrations = "./migrations")]
async fn two_inventories_of_one_marketplace_cannot_both_hold_a_live_lease(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xE1, true).await;
    let us_mapping = seed_mapping_on(&app, &tenant, 0xE4, InventoryId::TesUs).await;
    enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tenant.mapping,
            job_seed: 0xE5,
            item_seed: 0xE6,
            inventory: InventoryId::TesGb,
            operation: ItemOperation::Create,
        },
    )
    .await;
    enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: us_mapping,
            job_seed: 0xE7,
            item_seed: 0xE8,
            inventory: InventoryId::TesUs,
            operation: ItemOperation::Create,
        },
    )
    .await;

    let first = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the first item leases");
    assert_eq!(
        first.inventory,
        InventoryId::TesGb,
        "the fixture depends on FIFO order"
    );
    let second = claim(&app, tenant.org, "w2", 60).await;
    assert!(
        second.is_none(),
        "tes_gb and tes_us are one Tes login, so the second claimant must find the slot \
         taken rather than open a second live session on one marketplace account: {second:?}"
    );
}

/// The mutex re-scope, non-contending half.
///
/// Two marketplaces are two logins, so they are two slots. Under the
/// per-organisation index this was one, which is what capped a seller at one
/// working device and serialised the two branches against each other.
///
/// Deliberately Tpt rather than Etsy: both are the seller-device branch, so
/// what separates these two leases is the mutex and nothing else. Pairing
/// against Etsy would let the transport-class split do the separating and the
/// assertion would still pass with the mutex broken.
#[sqlx::test(migrations = "./migrations")]
async fn two_marketplaces_can_each_hold_a_live_lease(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xF1, true).await;
    let tpt_mapping = seed_mapping_on(&app, &tenant, 0xF4, InventoryId::Tpt).await;
    link_connection(&app, tenant.org, "tpt", 0xFB).await;
    enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tenant.mapping,
            job_seed: 0xF6,
            item_seed: 0xF7,
            inventory: InventoryId::TesGb,
            operation: ItemOperation::Create,
        },
    )
    .await;
    enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tpt_mapping,
            job_seed: 0xF8,
            item_seed: 0xF9,
            inventory: InventoryId::Tpt,
            operation: ItemOperation::Create,
        },
    )
    .await;

    let first = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the first item leases");
    let second = claim(&app, tenant.org, "w2", 60)
        .await
        .expect("a second marketplace is a second session, so it leases too");
    assert_ne!(
        first.inventory, second.inventory,
        "the two live leases must be on different marketplaces, or the mutex is not what \
         let them both through"
    );
}

/// Two devices racing one mapping: the fence admits exactly one.
///
/// `open` is idempotent on the caller's id, so a lost response is recovered by
/// re-offering the same id. A *different* id arriving while one is in flight is
/// a second create rather than a retry, and it is refused — which is the only
/// thing standing between a requeued item and a duplicate listing.
#[sqlx::test(migrations = "./migrations")]
async fn exactly_one_open_per_mapping_survives_two_devices(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xB1, true).await;
    enqueue_one(&engine, &tenant, 0xB2, 0xB3).await;
    let lease = claim(&app, tenant.org, "device-a", 60)
        .await
        .expect("the item leases");
    let attempts = WriteAttemptRepo::new(engine.clone());
    let first = Uuid([0xB4; 16]);
    let second = Uuid([0xB5; 16]);
    attempts
        .open(&lease.lease_ref(), first, &new_attempt(tenant.mapping))
        .await
        .expect("the first device opens the fence");
    let racing = attempts
        .open(&lease.lease_ref(), second, &new_attempt(tenant.mapping))
        .await;
    assert!(
        matches!(racing, Err(StorageError::AttemptInFlight)),
        "a second device minting its own id is a second create, and the fence refuses it: \
         {racing:?}"
    );
    let replayed = attempts
        .open(&lease.lease_ref(), first, &new_attempt(tenant.mapping))
        .await;
    assert!(
        replayed.is_ok(),
        "re-offering the id already standing is the lost-response recovery, not a second \
         create, so it answers Ok against the row already there: {replayed:?}"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tenant.mapping).await,
        vec!["in_flight".to_owned()],
        "and exactly one attempt row exists however many times it was offered"
    );
}

fn new_attempt(mapping: MappingId) -> NewAttempt<'static> {
    static INTENT: std::sync::OnceLock<AttemptIntent> = std::sync::OnceLock::new();
    NewAttempt {
        mapping,
        intent: INTENT.get_or_init(|| AttemptIntent {
            body: serde_json::json!({}),
            hash: vec![0x01],
        }),
        stamp: Stamp {
            at: T0,
            actor: Actor::System(SystemComponent::Engine),
        },
    }
}

/// A revoked device claims nothing.
///
/// The seller's own sign-out on the "Your devices" page sets `revoked_at`, and
/// the claim reads it. Nothing else has to happen for the device to stop
/// working: it learns of the revocation when its next claim comes back empty.
#[sqlx::test(migrations = "./migrations")]
async fn a_revoked_device_claims_nothing(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xD1, true).await;
    enqueue_one(&engine, &tenant, 0xD2, 0xD3).await;
    assert!(
        claim(&app, tenant.org, "revoked-device", 60)
            .await
            .is_some(),
        "the fixture must be claimable before revocation, or the assertion below proves \
         nothing"
    );
    settle_back_to_queued(&engine).await;
    revoke_device(&app, tenant.org, "revoked-device").await;

    assert!(
        claim(&app, tenant.org, "revoked-device", 60)
            .await
            .is_none(),
        "a revoked device is refused at the claim, which is the only enforcement a device \
         that has already stopped checking in would ever see"
    );
    assert!(
        claim(&app, tenant.org, "another-device", 60)
            .await
            .is_some(),
        "and the revocation is of that device rather than of the tenant: an unrevoked \
         device still claims the same work"
    );
}

/// A free-tier organisation still claims.
///
/// Entitlement is not "has paid". An organisation that never subscribed has no
/// billing row at all and is entitled within the Free quotas `tam-limits`
/// already grants it, so the predicate must not touch it.
#[sqlx::test(migrations = "./migrations")]
async fn a_free_tier_organisation_still_claims(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xD5, true).await;
    enqueue_one(&engine, &tenant, 0xD6, 0xD7).await;
    let billed: i64 = sqlx::query_scalar("SELECT count(*) FROM billing_subscription")
        .fetch_one(&app)
        .await
        .expect("the billing table reads");
    assert_eq!(
        billed, 0,
        "the fixture must be a tenant that never subscribed"
    );

    assert!(
        claim(&app, tenant.org, "free-device", 60).await.is_some(),
        "a Free organisation syncs within its quotas, so the entitlement predicate must \
         not be reading this as unpaid-and-therefore-blocked"
    );
}

/// A plan that lapsed and stayed lapsed past the grace claims nothing.
///
/// This is the only thing the plan side of the predicate blocks. A status that
/// stopped entitling within the grace still claims, so a card that failed on
/// Monday does not stop a seller's Monday.
#[sqlx::test(migrations = "./migrations")]
async fn a_plan_lapsed_past_the_grace_claims_nothing(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xD9, true).await;
    enqueue_one(&engine, &tenant, 0xDA, 0xDB).await;
    subscribe(&app, tenant.org, "canceled", "now() - interval '1 hour'").await;
    assert!(
        claim(&app, tenant.org, "lapsed-device", 60).await.is_some(),
        "inside the grace the plan has lapsed but the seller has not lost the day"
    );
    settle_back_to_queued(&engine).await;

    subscribe(&app, tenant.org, "canceled", "now() - interval '48 hours'").await;
    assert!(
        claim(&app, tenant.org, "lapsed-device", 60).await.is_none(),
        "past the grace the plan blocks, which is the only subscription enforcement left \
         for work that runs on the seller's own machine"
    );
}

/// A sibling device holding the slot is a different answer from an empty queue.
///
/// Told `Empty`, an idle device polls again soon; told the slot is taken, it
/// backs off until the holder settles. Collapsing the two would have it
/// hot-poll a queue it cannot win.
#[sqlx::test(migrations = "./migrations")]
async fn a_second_device_is_told_the_slot_is_held_rather_than_that_nothing_is_queued(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xE9, true).await;
    enqueue_one(&engine, &tenant, 0xEA, 0xEB).await;
    register_device(&app, tenant.org, "device-a").await;
    register_device(&app, tenant.org, "device-b").await;
    let leases = LeaseRepo::new(app.clone());
    assert!(matches!(
        leases
            .claim_for_device(
                &DeviceRef {
                    org: tenant.org,
                    device: "device-a"
                },
                &ClaimPolicy {
                    ttl_seconds: 60,
                    grace_hours: 24,
                    marketplace: None,
                    reconcile: true,
                },
                T0
            )
            .await
            .expect("the first claim runs"),
        DeviceClaim::Leased(_)
    ));

    let second = leases
        .claim_for_device(
            &DeviceRef {
                org: tenant.org,
                device: "device-b",
            },
            &ClaimPolicy {
                ttl_seconds: 60,
                grace_hours: 24,
                marketplace: None,
                reconcile: true,
            },
            T0,
        )
        .await
        .expect("the second claim runs");
    assert_eq!(
        second,
        DeviceClaim::HeldByAnotherDevice,
        "the sibling holds the only slot for this marketplace account, and saying so is \
         what lets the idle device back off instead of polling"
    );
}

/// Returns every item to the queue, so one fixture can be claimed twice.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn settle_back_to_queued(engine: &PgPool) {
    sqlx::query(
        "UPDATE job_item SET state = 'queued', lease_owner = NULL, lease_expires_at = NULL",
    )
    .execute(engine)
    .await
    .expect("the item returns to the queue");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn revoke_device(app: &PgPool, org: OrgId, device: &str) {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query("UPDATE device SET revoked_at = now() WHERE org_id = $1 AND id = $2")
        .bind(db_uuid(org.0))
        .bind(device)
        .execute(&mut *tx)
        .await
        .expect("the device revokes");
    tx.commit().await.expect("the revocation commits");
}

/// A billing row in a stated status whose period ended a stated interval ago.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn subscribe(app: &PgPool, org: OrgId, status: &str, period_end: &str) {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(&format!(
        "INSERT INTO billing_subscription \
           (org_id, paddle_subscription_id, paddle_customer_id, status, \
            current_period_end, occurred_at, updated_at) \
         VALUES ($1, 'sub_fixture', 'ctm_fixture', $2, {period_end}, now(), now()) \
         ON CONFLICT (org_id) DO UPDATE SET status = $2, current_period_end = {period_end}"
    ))
    .bind(db_uuid(org.0))
    .bind(status)
    .execute(&mut *tx)
    .await
    .expect("the subscription row writes");
    tx.commit().await.expect("the subscription commits");
}

/// A filtered claim never returns another marketplace's item.
///
/// The device gates its own readiness per marketplace before it pulls, so it
/// asks for the one it is ready for. Serving it a different marketplace's item
/// would hand it work it can only refuse, and the refusal costs a lease expiry
/// before anyone else can take that item.
#[sqlx::test(migrations = "./migrations")]
async fn a_filtered_claim_returns_only_the_marketplace_it_asked_for(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC2, true).await;
    let tpt_mapping = seed_mapping_on(&app, &tenant, 0xC6, InventoryId::Tpt).await;
    link_connection(&app, tenant.org, "tpt", 0xC7).await;
    // Tes first in FIFO order, so an unfiltered claim would take it and a
    // filtered one asking for Tpt must not.
    enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tenant.mapping,
            job_seed: 0xCA,
            item_seed: 0xCB,
            inventory: InventoryId::TesGb,
            operation: ItemOperation::Create,
        },
    )
    .await;
    enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: tpt_mapping,
            job_seed: 0xCC,
            item_seed: 0xCD,
            inventory: InventoryId::Tpt,
            operation: ItemOperation::Create,
        },
    )
    .await;
    register_device(&app, tenant.org, "filtering-device").await;
    let leases = LeaseRepo::new(app.clone());

    let tpt = leases
        .claim_for_device(
            &DeviceRef {
                org: tenant.org,
                device: "filtering-device",
            },
            &ClaimPolicy {
                ttl_seconds: 60,
                grace_hours: 24,
                marketplace: Some(Marketplace::Tpt),
                reconcile: true,
            },
            T0,
        )
        .await
        .expect("the filtered claim runs");
    let DeviceClaim::Leased(item) = tpt else {
        panic!("the Tpt item is claimable and the filter asked for it: {tpt:?}");
    };
    assert_eq!(
        item.inventory,
        InventoryId::Tpt,
        "the filter asked for Tpt, and the Tes item is first in FIFO order, so serving \
         Tes here would be the filter having done nothing"
    );

    // And a device asking for the other marketplace still claims: the slot
    // taken above is Tpt's, and Tes was never contended.
    register_device(&app, tenant.org, "tes-device").await;
    let anything = leases
        .claim_for_device(
            &DeviceRef {
                org: tenant.org,
                device: "tes-device",
            },
            &ClaimPolicy {
                ttl_seconds: 60,
                grace_hours: 24,
                marketplace: Some(Marketplace::Tes),
                reconcile: true,
            },
            T0,
        )
        .await
        .expect("the second filtered claim runs");
    let DeviceClaim::Leased(other) = anything else {
        panic!(
            "Tes was never contended, so a device asking for it must claim rather than be \
             told the queue is held: {anything:?}"
        );
    };
    assert_eq!(
        other.inventory,
        InventoryId::TesGb,
        "and what it claims is the marketplace it asked for"
    );
}

/// A renewed lease outlives the reaper; an unrenewed one does not.
///
/// This is the whole point of the heartbeat: before it, the reaper could only
/// tell that a fixed TTL had elapsed, so a device still working lost its item
/// to one that could not distinguish slow from gone. After it, a lease that
/// stopped being extended is one whose holder stopped.
#[sqlx::test(migrations = "./migrations")]
async fn the_reaper_reclaims_a_lease_that_stopped_heartbeating_and_not_one_that_did_not(
    app: PgPool,
) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA4, true).await;
    enqueue_one(&engine, &tenant, 0xA5, 0xA6).await;
    let leases = LeaseRepo::new(engine.clone());
    let held = claim(&app, tenant.org, "beating-device", 60)
        .await
        .expect("the item leases");

    // The lease ages, as it would while the device was working.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the lease ages");
    // The heartbeat, which is what the interpreter sends before every
    // network-bearing call.
    leases
        .renew(&held.lease_ref())
        .await
        .expect("a live lease renews");
    assert_eq!(
        leases
            .expire_and_steal(T0, ATTEMPTS_MAX)
            .await
            .expect("the reaper runs"),
        0,
        "the device is still beating, so the reaper leaves its item alone"
    );

    // And now it stops.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the lease ages again");
    assert_eq!(
        leases
            .expire_and_steal(T0, ATTEMPTS_MAX)
            .await
            .expect("the reaper runs"),
        1,
        "a lease that stopped being extended is one whose holder stopped, and that is \
         the item the reaper is for"
    );
}

/// A renew from a run that no longer holds the item is refused.
///
/// The fence is what makes the heartbeat safe to put before a marketplace
/// request: a run whose lease was stolen learns it there, and stops, rather
/// than issuing a write under a lease it does not hold.
#[sqlx::test(migrations = "./migrations")]
async fn a_renew_against_a_bumped_epoch_is_refused(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA8, true).await;
    enqueue_one(&engine, &tenant, 0xA9, 0xAA).await;
    let leases = LeaseRepo::new(engine.clone());
    let held = claim(&app, tenant.org, "losing-device", 60)
        .await
        .expect("the item leases");
    let stale = held.lease_ref();

    sqlx::query(
        "UPDATE job_item SET lease_epoch = lease_epoch + 1, lease_owner = 'another-device'",
    )
    .execute(&engine)
    .await
    .expect("another device takes the item");

    assert!(
        matches!(leases.renew(&stale).await, Err(StorageError::StaleLease)),
        "the epoch moved, so the heartbeat tells the old holder it no longer holds the \
         item rather than quietly buying it time it has no right to"
    );
}

/// The renewed expiry is the server's own TTL, not a number a caller named.
///
/// The claim here takes a deliberately short lease and the renew answers a
/// full one: the caller states no duration at all, so there is no value it
/// could saturate, and a device asking for a lease measured in decades gets
/// exactly what every other renew gets. Both instants come out of the one
/// statement, so the difference is exact rather than a comparison of two
/// clocks.
#[sqlx::test(migrations = "./migrations")]
async fn a_renew_mints_the_expiry_from_the_servers_own_ttl(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xB4, true).await;
    enqueue_one(&engine, &tenant, 0xB5, 0xB6).await;
    let held = claim(&app, tenant.org, "asking-device", 5)
        .await
        .expect("the item leases");

    let renewed = LeaseRepo::new(engine)
        .renew(&held.lease_ref())
        .await
        .expect("a live lease renews");

    assert_eq!(
        renewed.expires_at.0 - renewed.server_now.0,
        i64::from(LEASE_TTL_SECS) * 1_000,
        "the lease the server hands back is the one it minted from its own constant, and \
         the five seconds the claim asked for had no part in it"
    );
}

/// A release from a run that no longer holds the item is refused.
///
/// The same fence every other epoch-keyed write here has. Before it, a release
/// answered `Ok` whether or not it had released anything, so a claimant that
/// had already lost the item was told it had handed back something it did not
/// have.
#[sqlx::test(migrations = "./migrations")]
async fn a_release_from_a_run_that_no_longer_holds_the_item_is_refused(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xB7, true).await;
    enqueue_one(&engine, &tenant, 0xB8, 0xB9).await;
    let leases = LeaseRepo::new(engine.clone());
    let held = claim(&app, tenant.org, "declining-device", 60)
        .await
        .expect("the item leases");
    let stale = held.lease_ref();

    leases.release(&stale).await.expect("the holder releases");
    let state: String = sqlx::query_scalar("SELECT state FROM job_item")
        .fetch_one(&engine)
        .await
        .expect("the item reads");
    assert_eq!(
        state, "queued",
        "a claimant that declines the work it claimed puts it back on the queue rather \
         than holding it to expiry"
    );

    assert!(
        matches!(leases.release(&stale).await, Err(StorageError::StaleLease)),
        "and a second release has nothing to release, which is the fence answering rather \
         than a write silently doing nothing"
    );
}

/// The reaper's SQL rule and the per-item charge agree at the boundary.
///
/// They cannot share one spelling: `expire_and_steal` is a cross-tenant
/// set-based scan and cannot call into Rust per row, so the threshold exists
/// twice — once as `attempt_count + 1 >= $2` in its statement and once as
/// `LeaseRepo::attempt_budget_spent`. This is what keeps the two honest: the
/// same item, one attempt short of its budget, is requeued by both, and on its
/// last attempt is settled failed by both.
#[sqlx::test(migrations = "./migrations")]
async fn the_reaper_and_the_charge_agree_at_the_boundary(app: PgPool) {
    const BUDGET: i32 = 3;
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC1, true).await;
    enqueue_one(&engine, &tenant, 0xC2, 0xC3).await;
    let leases = LeaseRepo::new(engine.clone());

    // One attempt short of the budget: both dispositions hand it back.
    sqlx::query("UPDATE job_item SET attempt_count = $1")
        .bind(BUDGET - 2)
        .execute(&engine)
        .await
        .expect("the item is set one short of its budget");
    let held = claim(&app, tenant.org, "charging-device", 60)
        .await
        .expect("the item leases");
    assert_eq!(
        leases
            .charge_and_requeue(
                &held.lease_ref(),
                BUDGET,
                Some(FailureDetail("a preparation failure".to_owned())),
                T0,
            )
            .await
            .expect("the charge lands"),
        Charged::Requeued,
        "one attempt short of the budget, the charge hands the item back"
    );
    let (state, count): (String, i32) = sqlx::query_as("SELECT state, attempt_count FROM job_item")
        .fetch_one(&engine)
        .await
        .expect("the item reads");
    assert_eq!(
        (state.as_str(), count),
        ("queued", BUDGET - 1),
        "charged exactly one attempt and queued"
    );

    // And now it is on its last: the reaper would settle it here, and so does
    // the charge.
    let held = claim(&app, tenant.org, "charging-device", 60)
        .await
        .expect("the item leases again");
    assert!(
        attempt_budget_spent(BUDGET - 1, BUDGET),
        "the one Rust spelling of the rule says this attempt is the last, which is what \
         the reaper's SQL predicate says of the same numbers"
    );
    assert_eq!(
        leases
            .charge_and_requeue(
                &held.lease_ref(),
                BUDGET,
                Some(FailureDetail("a preparation failure".to_owned())),
                T0,
            )
            .await
            .expect("the charge lands"),
        Charged::Settled,
        "on the last attempt it settles rather than queueing a run nothing would finish"
    );
    let (state, outcome): (String, Option<String>) =
        sqlx::query_as("SELECT state, outcome FROM job_item")
            .fetch_one(&engine)
            .await
            .expect("the item reads");
    assert_eq!(
        (state.as_str(), outcome.as_deref()),
        ("settled", Some("failed")),
        "which is the disposition the reaper's exhaust arm reaches for the same numbers"
    );
}

/// A charge from a run that no longer holds the item is refused.
#[sqlx::test(migrations = "./migrations")]
async fn a_charge_from_a_run_that_no_longer_holds_the_item_is_refused(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xC4, true).await;
    enqueue_one(&engine, &tenant, 0xC5, 0xC6).await;
    let leases = LeaseRepo::new(engine.clone());
    let held = claim(&app, tenant.org, "losing-device", 60)
        .await
        .expect("the item leases");
    let stale = held.lease_ref();

    sqlx::query("UPDATE job_item SET lease_epoch = lease_epoch + 1")
        .execute(&engine)
        .await
        .expect("another holder takes the item");

    assert!(
        matches!(
            leases
                .charge_and_requeue(
                    &stale,
                    5,
                    Some(FailureDetail("a preparation failure".to_owned())),
                    T0,
                )
                .await,
            Err(StorageError::StaleLease)
        ),
        "a run that lost the item cannot charge an attempt against it, or a steal would \
         cost the item two"
    );
}

/// A create whose mapping was bound while it ran is refused at `open`.
///
/// `prepare_item` already refuses a create against a bound mapping, so the
/// only way to reach this is for the bind to land between that check and this
/// write: a concurrent run, or a lease stolen and re-offered. The refusal is
/// distinct from `AttemptInFlight` because it is permanent — the listing
/// exists — and the interpreter settles rather than abandons on it.
#[sqlx::test(migrations = "./migrations")]
async fn a_create_against_a_mapping_bound_since_admission_is_refused_at_open(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xD1, true).await;
    enqueue_one(&engine, &tenant, 0xD2, 0xD3).await;
    let held = claim(&app, tenant.org, "creating-device", 60)
        .await
        .expect("the item leases");

    // The bind lands after this run was admitted, which is the whole window
    // the re-check exists to close.
    sqlx::query(
        "UPDATE mapping SET binding_state = 'bound', remote_id_kind = 'tes', \
             remote_url = 'https://www.tes.com/teaching-resource/x-1', \
             first_seen_at = now(), verify_stale_since = now()",
    )
    .execute(&engine)
    .await
    .expect("another run binds the mapping");

    let opened = WriteAttemptRepo::new(engine.clone())
        .open(
            &held.lease_ref(),
            tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: held.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await;
    assert!(
        matches!(opened, Err(StorageError::MappingAlreadyBound)),
        "the create is refused with the reason that names it, not with the in-flight \
         fence and not by succeeding: {opened:?}"
    );
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM write_attempt")
        .fetch_one(&engine)
        .await
        .expect("the attempts read");
    assert_eq!(
        attempts, 0,
        "and nothing was written, so the refusal is the whole of what happened"
    );
}

/// A create parked on reauth leaves the park for a gate the seller can act
/// on, and nothing else moves.
///
/// The fence is the point. The attempt stays in flight because releasing it
/// is the only thing standing between this seller and a second live listing;
/// the mapping stays bound to nothing; the item stays parked and is charged
/// no attempt. What changes is the one thing that can change safely: what the
/// seller is told.
#[sqlx::test(migrations = "./migrations")]
async fn a_create_parked_on_reauth_leaves_the_park_for_a_gate_the_seller_can_act_on(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xE1, true).await;
    enqueue_one(&engine, &tenant, 0xE2, 0xE3).await;
    let held = claim(&app, tenant.org, "parking-device", 60)
        .await
        .expect("the item leases");
    let leases = LeaseRepo::new(engine.clone());
    leases
        .park(&held.lease_ref(), REAUTH_REQUIRED, 1)
        .await
        .expect("the create parks on reauth");
    // An attempt left standing, as the submit path leaves one.
    WriteAttemptRepo::new(engine.clone())
        .open(
            &held.lease_ref(),
            tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: held.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the attempt opens");
    sqlx::query("UPDATE job_item SET park_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the park ages out");

    leases
        .revive_expired(T0, ATTEMPTS_MAX)
        .await
        .expect("the revive pass runs");

    let (state, gate, attempts, expires): (String, Option<String>, i32, bool) = sqlx::query_as(
        "SELECT state, blocked_on, attempt_count, park_expires_at IS NOT NULL FROM job_item",
    )
    .fetch_one(&engine)
    .await
    .expect("the item reads");
    assert_eq!(
        (state.as_str(), gate.as_deref()),
        ("parked_live", Some(AWAITING_SELLER_SIGNIN)),
        "the item stays parked and now names the one thing that clears it"
    );
    assert!(
        !expires,
        "and it carries no expiry, because no clock opens this gate"
    );
    assert_eq!(
        attempts, held.attempt_count,
        "no attempt is charged: the seller not having signed in is not the item failing"
    );
    let in_flight: i64 =
        sqlx::query_scalar("SELECT count(*) FROM write_attempt WHERE state = 'in_flight'")
            .fetch_one(&engine)
            .await
            .expect("the attempts read");
    assert_eq!(
        in_flight, 1,
        "the attempt is still standing, which is the fence against a second live listing \
         and the whole reason this is not a revive"
    );
}

/// The park-age arm is for creates, and leaves everything else alone.
///
/// A revise or a removal parked on `ReauthRequired` has a resolution a create
/// does not: it can simply be re-run once the seller signs in, because
/// re-applying the same fields or re-deleting something already gone is safe.
/// The re-link arm exists for exactly that, so moving one to
/// `awaiting_seller_signin` would take it out of the arm that can actually
/// clear it and strand work that was never stuck.
#[sqlx::test(migrations = "./migrations")]
async fn the_park_age_arm_leaves_a_revise_for_the_re_link_arm(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xE7, true).await;
    enqueue_operation(
        &engine,
        &tenant,
        0xE8,
        0xE9,
        ItemOperation::Revise {
            subject: RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/x-7".to_owned(),
            },
            transition: LifecycleTransition {
                from: ListingState::Draft,
                to: ListingState::Live,
            },
        },
    )
    .await;
    let held = claim(&app, tenant.org, "revising-device", 60)
        .await
        .expect("the item leases");
    let leases = LeaseRepo::new(engine.clone());
    leases
        .park(&held.lease_ref(), REAUTH_REQUIRED, 1)
        .await
        .expect("the revise parks on reauth");
    sqlx::query("UPDATE job_item SET park_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the park ages out");
    relink(&engine, tenant.org, "tes").await;

    leases
        .revive_expired(T0, ATTEMPTS_MAX)
        .await
        .expect("the revive pass runs");

    let (state, gate): (String, Option<String>) =
        sqlx::query_as("SELECT state, blocked_on FROM job_item")
            .fetch_one(&engine)
            .await
            .expect("the item reads");
    assert_eq!(
        (state.as_str(), gate),
        ("queued", None),
        "the revise is revived by the re-link arm rather than re-labelled by the \
         park-age arm, which is what the operation predicate on that arm is for"
    );
}

/// The lock order holds under a forced overlap.
///
/// How the overlap is forced, since a `join!` alone leaves it to the
/// scheduler and would only sometimes overlap. A test-held transaction plays
/// the settle's lock sequence: it takes the in-flight `write_attempt` row
/// first, exactly as `settle` does, and then pauses. The real `open` runs
/// against that. Only when `open` is observably waiting — read out of
/// `pg_stat_activity` rather than slept for — does the held transaction reach
/// for the mapping, which is `settle`'s second lock.
///
/// That is what makes it discriminate. With the order this crate documents,
/// `open` inserts its row first and blocks there on the held attempt row,
/// holding no mapping lock; the held transaction takes the mapping
/// unopposed, commits, and `open` proceeds. With the order reversed, `open`
/// would hold the mapping and wait for the attempt row while the held
/// transaction waits for the mapping — a cycle, and Postgres kills one of
/// them. There is no retry on this path, so the victim would be a create that
/// may already have reached the marketplace and can no longer record that it
/// did.
///
/// The reversed variant is not run: it is the code this prevents rather than
/// a branch of it. Reverse the two statements in `open_asserted` and this
/// test fails with a deadlock, every time.
#[sqlx::test(migrations = "./migrations")]
async fn the_lock_order_holds_when_a_settle_and_an_open_overlap(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xF1, true).await;
    enqueue_one(&engine, &tenant, 0xF2, 0xF3).await;
    let held = claim(&app, tenant.org, "racing-device", 60)
        .await
        .expect("the item leases");
    let first = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
    WriteAttemptRepo::new(engine.clone())
        .open(
            &held.lease_ref(),
            first,
            &NewAttempt {
                mapping: held.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the first attempt opens");

    let settling = engine_pool(&app).await;
    let mut settle_like = settling
        .begin()
        .await
        .expect("the settle-like transaction opens");
    sqlx::query("SELECT id FROM write_attempt WHERE org_id = $1 AND id = $2 FOR UPDATE")
        .bind(uuid::Uuid::from_bytes(tenant.org.0 .0))
        .bind(uuid::Uuid::from_bytes(first.0))
        .fetch_one(&mut *settle_like)
        .await
        .expect("it takes the attempt row, which is what settle takes first");

    let opening = WriteAttemptRepo::new(engine.clone());
    let lease = held.lease_ref();
    let mapping = held.mapping;
    let intent = intent();
    let stamp = Stamp {
        at: T0,
        actor: Actor::System(SystemComponent::Engine),
    };
    let waiting = engine_pool(&app).await;
    let second = NewAttempt {
        mapping,
        intent: &intent,
        stamp,
    };
    let second_id = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
    let (opened, ()) = tokio::join!(opening.open(&lease, second_id, &second), async {
        // Wait for the open to be observably blocked rather than sleeping
        // for a guessed interval, so the overlap is a fact rather than a
        // hope. Then take the mapping, which is settle's second lock.
        for _ in 0..600 {
            let blocked: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity \
                     WHERE wait_event_type = 'Lock' AND query ILIKE '%write_attempt%'",
            )
            .fetch_one(&waiting)
            .await
            .unwrap_or(0);
            if blocked > 0 {
                break;
            }
            std::thread::yield_now();
        }
        sqlx::query("SELECT id FROM mapping WHERE org_id = $1 AND id = $2 FOR UPDATE")
            .bind(uuid::Uuid::from_bytes(tenant.org.0 .0))
            .bind(uuid::Uuid::from_bytes(mapping.0 .0))
            .fetch_one(&mut *settle_like)
            .await
            .expect(
                "the settle-like transaction takes the mapping second; a deadlock here \
                     is the lock order inverted",
            );
        settle_like
            .commit()
            .await
            .expect("and commits, releasing both");
    },);

    if let Err(error) = opened {
        assert!(
            matches!(
                error,
                StorageError::AttemptInFlight | StorageError::MappingAlreadyBound
            ),
            "the open is refused by a fence if at all, never by the deadlock detector: \
             {error:?}"
        );
    }
}

/// A settle and an open on the same mapping run at once without deadlocking.
///
/// Both take the `write_attempt` row and then the `mapping` row, and this is
/// what holds them to it. The reversed variant cannot be run — it is the code
/// this test exists to prevent, not a branch of it — so what is asserted is
/// the consequence: two transactions racing on one mapping both return, one
/// possibly refused by a fence, neither killed by the deadlock detector.
///
/// Why that mattered enough to add a dev-dependency for: with `open` taking
/// the mapping first, a settle holding the attempt row and wanting the mapping
/// met an open holding the mapping and wanting the attempt row. Postgres
/// resolves that by killing one, there is no retry on this path, and the
/// victim is a create that may already have reached the marketplace and can no
/// longer record that it did — the one failure this ledger cannot undo.
///
/// Two pool connections and `join!` rather than one: a single connection
/// serialises the two into something that cannot reproduce it at all.
#[sqlx::test(migrations = "./migrations")]
async fn a_settle_and_an_open_on_one_mapping_do_not_deadlock(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xF1, true).await;
    enqueue_one(&engine, &tenant, 0xF2, 0xF3).await;
    let held = claim(&app, tenant.org, "racing-device", 60)
        .await
        .expect("the item leases");
    let first = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
    WriteAttemptRepo::new(engine.clone())
        .open(
            &held.lease_ref(),
            first,
            &NewAttempt {
                mapping: held.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the first attempt opens");

    let settling = WriteAttemptRepo::new(engine.clone());
    let opening = WriteAttemptRepo::new(engine_pool(&app).await);
    let lease = held.lease_ref();
    let mapping = held.mapping;
    let verdict = AttemptVerdict {
        state: "committed".to_owned(),
        failure_code: None,
        landing: LandingEffect::Landed {
            id: RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/x-9".to_owned(),
            },
            lifecycle: RemoteLifecycle::Draft,
        },
    };
    let racing = AttemptRef {
        attempt: first,
        mapping,
    };
    let intent = intent();
    let stamp = Stamp {
        at: T0,
        actor: Actor::System(SystemComponent::Engine),
    };
    let second = NewAttempt {
        mapping,
        intent: &intent,
        stamp,
    };
    let opened_id = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
    let (settled, opened) = tokio::join!(
        settling.settle(&lease, racing, &verdict, T0),
        opening.open(&lease, opened_id, &second),
    );

    drop(settled.expect("the settle completes rather than being killed as a deadlock victim"));
    if let Err(error) = opened {
        assert!(
            matches!(
                error,
                StorageError::AttemptInFlight | StorageError::MappingAlreadyBound
            ),
            "the open is refused by a fence if at all, never by the deadlock detector: \
             {error:?}"
        );
    }
}

/// An attempt naming a mapping its item does not is refused.
///
/// The item's mapping is the server's, read under the lease; the caller's is
/// data. Without this the two are never compared, so a device could open a
/// fencing row against a mapping it was not leased for — and the fence that
/// stops a duplicate listing is per mapping, so a row filed under the wrong
/// one fences nothing.
#[sqlx::test(migrations = "./migrations")]
async fn an_attempt_naming_a_mapping_its_item_does_not_is_refused(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xD7, true).await;
    enqueue_one(&engine, &tenant, 0xD8, 0xD9).await;
    let held = claim(&app, tenant.org, "confused-device", 60)
        .await
        .expect("the item leases");
    // A real mapping of this tenant's, on another inventory, so the foreign
    // key is satisfied and the only thing wrong is that it is not the
    // mapping this item names. A mapping that did not exist at all would be
    // refused by the key before the check ran, which would prove nothing.
    let other = seed_mapping_on(&app, &tenant, 0xDA, InventoryId::TesUs).await;

    let opened = WriteAttemptRepo::new(engine.clone())
        .open(
            &held.lease_ref(),
            tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: other,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await;
    assert!(
        matches!(opened, Err(StorageError::Inconsistent { .. })),
        "the caller's mapping is compared against the item's rather than trusted: {opened:?}"
    );
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM write_attempt")
        .fetch_one(&engine)
        .await
        .expect("the attempts read");
    assert_eq!(
        attempts, 0,
        "and the insert is rolled back, so a refused open leaves no row filed under \
         either mapping"
    );
}

/// The park exit records what happened, and what happened is the gate moving.
#[sqlx::test(migrations = "./migrations")]
async fn the_park_exit_records_the_gate_it_moved_to(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xEA, true).await;
    enqueue_one(&engine, &tenant, 0xEB, 0xEC).await;
    let held = claim(&app, tenant.org, "parked-device", 60)
        .await
        .expect("the item leases");
    let leases = LeaseRepo::new(engine.clone());
    leases
        .park(&held.lease_ref(), REAUTH_REQUIRED, 1)
        .await
        .expect("the create parks on reauth");
    sqlx::query("UPDATE job_item SET park_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the park ages out");

    leases
        .revive_expired(T0, ATTEMPTS_MAX)
        .await
        .expect("the revive pass runs");

    let (kind, payload): (String, serde_json::Value) =
        sqlx::query_as("SELECT kind, payload FROM job_event ORDER BY org_seq DESC LIMIT 1")
            .fetch_one(&engine)
            .await
            .expect("the newest event reads");
    assert_eq!(
        kind, "ItemGateChanged",
        "the timeline says the gate moved, not that a second park began"
    );
    assert_eq!(
        payload["gate"], AWAITING_SELLER_SIGNIN,
        "and it names the gate the item moved to, which is the whole of what changed: \
         {payload}"
    );
}

/// A replay after the original bound the mapping still answers the attempt it
/// already has.
///
/// The admission re-check runs when the insert writes, and only then. A caller
/// re-offering an id it already opened is recovering a lost response, and the
/// row it is asking about may since have committed and bound the mapping
/// itself — so refusing the replay would tell a device its own committed
/// create belonged to somebody else, and the recovery `open`'s idempotence
/// exists for would be exactly the case that could not use it.
#[sqlx::test(migrations = "./migrations")]
async fn a_replay_after_its_own_create_bound_the_mapping_is_still_idempotent(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xE3, true).await;
    enqueue_one(&engine, &tenant, 0xE4, 0xE5).await;
    let held = claim(&app, tenant.org, "recovering-device", 60)
        .await
        .expect("the item leases");
    let attempts = WriteAttemptRepo::new(engine.clone());
    let minted = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
    let new = NewAttempt {
        mapping: held.mapping,
        intent: &intent(),
        stamp: Stamp {
            at: T0,
            actor: Actor::System(SystemComponent::Engine),
        },
    };
    attempts
        .open(&held.lease_ref(), minted, &new)
        .await
        .expect("the attempt opens");
    // This run's own create landed and bound the mapping. The response was
    // lost, so the device re-offers the id it minted.
    sqlx::query(
        "UPDATE mapping SET binding_state = 'bound', remote_id_kind = 'tes', \
             remote_url = 'https://www.tes.com/teaching-resource/x-3', \
             first_seen_at = now(), verify_stale_since = now()",
    )
    .execute(&engine)
    .await
    .expect("this run's create binds the mapping");

    attempts.open(&held.lease_ref(), minted, &new).await.expect(
        "the replay answers the attempt it already has; the bind it is being refused \
             for is its own",
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM write_attempt")
        .fetch_one(&engine)
        .await
        .expect("the attempts read");
    assert_eq!(rows, 1, "and no second row is written");
}

/// Strands a create the way a run that ended without settling it does: the
/// item leases, its fencing attempt opens, and the item is parked on the gate
/// with the attempt still `in_flight`.
///
/// The attempt is what makes it stranded rather than merely parked. Nothing
/// can create in its place while that row stands, so every pass that leaves
/// it there leaves a mapping's fence shut.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn strand_a_create(
    app: &PgPool,
    engine: &PgPool,
    org: OrgId,
    mapping: MappingId,
) -> (JobItemId, Uuid) {
    let lease = claim(app, org, DEVICE, 60).await.expect("the item leases");
    assert_eq!(
        lease.mapping, mapping,
        "the fixture depends on claim order, so the expected item must be the one leased"
    );
    let attempt = Uuid(*uuid::Uuid::new_v4().as_bytes());
    WriteAttemptRepo::new(engine.clone())
        .open(
            &lease.lease_ref(),
            attempt,
            &NewAttempt {
                mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the fencing attempt opens");
    LeaseRepo::new(engine.clone())
        .park(&lease.lease_ref(), AWAITING_MARKETPLACE_ANSWER, 3_600)
        .await
        .expect("the park is fenced on a live lease");
    (lease.item, attempt)
}

/// One item's attempt count, park clock and outcome, which together say
/// whether a pass charged it, timed it, or settled it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn item_disposition(
    pool: &PgPool,
    org: OrgId,
    item: JobItemId,
) -> (String, Option<String>, i32, bool, Option<String>) {
    sqlx::query_as::<_, (String, Option<String>, i32, bool, Option<String>)>(
        "SELECT state, blocked_on, attempt_count, park_expires_at IS NOT NULL, outcome \
         FROM job_item WHERE org_id = $1 AND id = $2",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(item.0))
    .fetch_one(pool)
    .await
    .expect("the item row is readable")
}

/// The claim admits a stranded create out of its park and serves it ahead of
/// queued work, carrying the attempt the device must reconcile.
///
/// The queued sibling is aged past the stranded one deliberately: under the
/// plain FIFO this claim used to have it would be served first, so the
/// assertion fails unless the reconcile-first ordering is what decided. That
/// ordering is not a preference — the stranded item holds its mapping's fence,
/// and every pass that serves something else leaves it shut.
#[sqlx::test(migrations = "./migrations")]
async fn a_stranded_create_is_claimed_ahead_of_older_queued_work(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x61, true).await;
    enqueue_one(&engine, &tenant, 0x62, 0x63).await;
    let (stranded, attempt) = strand_a_create(&app, &engine, tenant.org, tenant.mapping).await;

    // On the other device-branch inventory, because `mapping_one_per_inventory`
    // allows this product only one mapping per inventory and the stranded item
    // holds the Tes one.
    let second = seed_mapping_on(&app, &tenant, 0x64, InventoryId::Tpt).await;
    link_connection(&app, tenant.org, "tpt", 0x6F).await;
    let queued = enqueue_on(
        &engine,
        &tenant,
        &EnqueueOnto {
            mapping: second,
            job_seed: 0x65,
            item_seed: 0x66,
            inventory: InventoryId::Tpt,
            operation: ItemOperation::Create,
        },
    )
    .await;
    sqlx::query("UPDATE job_item SET created_at = created_at - interval '1 hour' WHERE id = $1")
        .bind(db_uuid(queued.0))
        .execute(&engine)
        .await
        .expect("the queued sibling ages past the stranded one");

    let leased = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the stranded create is claimable");
    assert_eq!(
        leased.item, stranded,
        "the older queued item would win a plain FIFO, so serving the stranded one is the \
         reconcile-first ordering and nothing else"
    );
    assert_eq!(
        (leased.stranded_attempt, leased.stranded_title.as_deref()),
        (Some(attempt), Some(RECORDED_TITLE)),
        "and the claim names the attempt to reconcile and the title that attempt recorded it \
         sent, which is the only way the device can tell a reconcile from an ordinary create \
         and the only thing that identifies the listing: the operation still reads 'create', \
         and the product's title now may not be the one the create used"
    );
    let (state, blocked_on, _, timed, _) = item_disposition(&engine, tenant.org, stranded).await;
    assert_eq!(
        (state.as_str(), blocked_on.as_deref(), timed),
        ("leased", None, false),
        "the claim took it out of the park rather than leaving a gate a later pass would \
         act on"
    );
}

/// The claim serves a stranded create parked under either word.
///
/// Two arms write a stranded create's park and they disagree on the word. The
/// reaper writes `awaiting_marketplace_answer`, which is what a create whose
/// fate is unknown is actually waiting on; step 12's re-gate arm still writes
/// `awaiting_seller_signin`, for a create whose reauth park aged out, where
/// signing in is genuinely the thing that helps. Both are stranded creates
/// holding a mapping's fence, so both have to be reachable -- a predicate
/// admitting only the newer word would leave every item the older arm had
/// already parked unreconcilable, and nothing else in this file would notice.
#[sqlx::test(migrations = "./migrations")]
async fn a_stranded_create_is_claimed_under_either_park_word(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x71, true).await;
    enqueue_one(&engine, &tenant, 0x72, 0x73).await;
    let (stranded, attempt) = strand_a_create(&app, &engine, tenant.org, tenant.mapping).await;

    // What the older arm leaves behind, written directly: reaching it through
    // `revive_expired` would need a reauth park aged past its clock, which is
    // a different fixture proving a different thing.
    sqlx::query("UPDATE job_item SET blocked_on = $1 WHERE org_id = $2 AND id = $3")
        .bind(AWAITING_SELLER_SIGNIN)
        .bind(db_uuid(tenant.org.0))
        .bind(db_uuid(stranded.0))
        .execute(&engine)
        .await
        .expect("the older arm's word is written");

    let leased = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("a create parked under the older word is still claimable");
    assert_eq!(
        (leased.item, leased.stranded_attempt),
        (stranded, Some(attempt)),
        "and it is served as a reconcile carrying its own attempt, exactly as one parked \
         under the newer word is"
    );
}

/// A device claims only work whose marketplace it holds a connected session
/// for, and a session the seller signed out of is still a row rather than an
/// absent one.
///
/// Both halves are asserted against the same fixture: the signed-out claim
/// finds nothing and the reconnected one finds the item, so the predicate is
/// pinned to the session rather than to anything else the fixture happens to
/// have done.
#[sqlx::test(migrations = "./migrations")]
async fn a_stranded_create_needs_a_connected_session_for_its_marketplace(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x67, true).await;
    enqueue_one(&engine, &tenant, 0x68, 0x69).await;
    let (stranded, attempt) = strand_a_create(&app, &engine, tenant.org, tenant.mapping).await;

    set_session_status(&app, tenant.org, DEVICE, "tes", "signed_out").await;
    assert!(
        claim(&app, tenant.org, DEVICE, 60).await.is_none(),
        "a device that cannot reach the marketplace under the seller's own session must not \
         be handed its reconciliation: the enumeration is a request like any other"
    );
    let (state, blocked_on, _, _, _) = item_disposition(&engine, tenant.org, stranded).await;
    assert_eq!(
        (state.as_str(), blocked_on.as_deref()),
        ("parked_live", Some(AWAITING_MARKETPLACE_ANSWER)),
        "and the refused claim left it exactly where it was, still fencing its mapping"
    );

    set_session_status(&app, tenant.org, DEVICE, "tes", "connected").await;
    let leased = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the reconnected device claims it");
    assert_eq!(
        (leased.item, leased.stranded_attempt),
        (stranded, Some(attempt)),
        "the session was the only thing standing between the device and the same item"
    );
}

/// The reaper parks a create whose device went away rather than settling or
/// stealing it, charges it nothing, and stops its day clock.
///
/// The item is set one attempt short of its budget on purpose. That makes it
/// match the settle arm as well as the steal arm, so an ordering in which the
/// park ran anywhere but first would settle it `failed` — an outcome nobody
/// observed, recorded against a write that may well have landed. The
/// unchanged `attempt_count` is the other half: a device going offline
/// mid-run is not the item failing, so nothing is charged for it.
#[sqlx::test(migrations = "./migrations")]
async fn the_reaper_parks_a_stranded_create_rather_than_settling_or_stealing_it(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x6A, true).await;
    let item = enqueue_one(&engine, &tenant, 0x6B, 0x6C).await;
    let lease = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the item leases");
    WriteAttemptRepo::new(engine.clone())
        .open(
            &lease.lease_ref(),
            Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the fencing attempt opens");
    // The device stops here: the request went out and no read-back followed.
    sqlx::query(
        "UPDATE job_item SET lease_expires_at = now() - interval '1 hour', \
             attempt_count = $1 WHERE id = $2",
    )
    .bind(ATTEMPTS_MAX - 1)
    .bind(db_uuid(item.0))
    .execute(&engine)
    .await
    .expect("the lease ages and the budget is brought to its last attempt");

    let touched = LeaseRepo::new(engine.clone())
        .expire_and_steal(Timestamp(T0.0 + 61_000), ATTEMPTS_MAX)
        .await
        .expect("the reaper runs");
    assert_eq!(touched, 1, "the expired lease was acted on exactly once");

    let (state, blocked_on, attempts, timed, outcome) =
        item_disposition(&engine, tenant.org, item).await;
    assert_eq!(
        (state.as_str(), blocked_on.as_deref(), outcome.as_deref()),
        ("parked_live", Some(AWAITING_MARKETPLACE_ANSWER), None),
        "it goes to the park the reconcile path reads, not to the queue and not to a \
         settled row recording an outcome nobody observed"
    );
    assert_eq!(
        attempts,
        ATTEMPTS_MAX - 1,
        "and it is charged nothing: the steal arm would have spent its last attempt and the \
         settle arm would have ended it"
    );
    assert!(
        !timed,
        "the day clock stops, because no amount of waiting reconciles a create: only the \
         seller's device reading their own catalogue does"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tenant.mapping).await,
        vec!["in_flight".to_owned()],
        "the fence it was holding is still standing, which is what makes it reconcilable \
         rather than merely retryable"
    );
}

/// A requeued item is not a reconcile, even while its create's attempt is
/// still standing.
///
/// `charge_and_requeue` bumps the item's epoch and deliberately leaves the
/// attempt alone, so the pairing it produces is an attempt
/// `WriteAttemptRepo::settle` is fenced against. Handing that out as a
/// reconcile is unrecoverable rather than merely wrong: the run would
/// enumerate the seller's catalogue, find the listing, fail to settle it and
/// abandon, charged nothing and re-served first on the next poll without end.
#[sqlx::test(migrations = "./migrations")]
async fn a_requeued_item_is_not_handed_the_attempt_its_epoch_has_moved_past(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x6D, true).await;
    enqueue_one(&engine, &tenant, 0x6E, 0x70).await;
    let (item, attempt) = strand_a_create(&app, &engine, tenant.org, tenant.mapping).await;

    let reconciling = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the stranded create is claimable");
    assert_eq!(
        reconciling.stranded_attempt,
        Some(attempt),
        "the premise: at a matching epoch this is a reconcile"
    );
    let charged = LeaseRepo::new(engine.clone())
        .charge_and_requeue(
            &reconciling.lease_ref(),
            ATTEMPTS_MAX,
            Some(FailureDetail("the fixture could not prepare it".to_owned())),
            T0,
        )
        .await
        .expect("the charge runs");
    assert_eq!(
        charged,
        Charged::Requeued,
        "the fixture depends on the requeue arm rather than the give-up arm"
    );

    let again = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the requeued item is claimable again");
    assert_eq!(again.item, item, "the same item comes back");
    assert_eq!(
        again.stranded_attempt, None,
        "and it comes back as ordinary work: the attempt standing against it was opened \
         under an epoch this item has moved past, so nothing this run does could settle it"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tenant.mapping).await,
        vec!["in_flight".to_owned()],
        "the attempt is still standing, which is what makes the run abandon on the fence \
         rather than mint a second listing -- bounded by the attempt budget"
    );
}

/// The reaper parks only a create it could actually reconcile.
///
/// An attempt opened under an epoch the item has moved past cannot be settled
/// by any later run, so parking on it would hold the item in the reconcile
/// park for ever, uncharged. It falls through to the steal instead, which is
/// the bounded pre-existing failure.
#[sqlx::test(migrations = "./migrations")]
async fn the_reaper_leaves_a_stale_epoch_attempt_to_the_steal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x74, true).await;
    let item = enqueue_one(&engine, &tenant, 0x75, 0x76).await;
    let lease = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the item leases");
    WriteAttemptRepo::new(engine.clone())
        .open(
            &lease.lease_ref(),
            Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &intent(),
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the fencing attempt opens");
    // The item moves on without its attempt, which is exactly what
    // `charge_and_requeue` does; done here directly so the row is left leased
    // and expired for the reaper to find.
    sqlx::query(
        "UPDATE job_item SET lease_epoch = lease_epoch + 1, \
             lease_expires_at = now() - interval '1 hour' WHERE id = $1",
    )
    .bind(db_uuid(item.0))
    .execute(&engine)
    .await
    .expect("the item's epoch moves past its attempt's and the lease ages");

    let touched = LeaseRepo::new(engine.clone())
        .expire_and_steal(Timestamp(T0.0 + 61_000), ATTEMPTS_MAX)
        .await
        .expect("the reaper runs");
    assert_eq!(touched, 1, "the expired lease was acted on exactly once");

    let (state, blocked_on, attempts, _, _) = item_disposition(&engine, tenant.org, item).await;
    assert_eq!(
        (state.as_str(), blocked_on.as_deref()),
        ("queued", None),
        "it was stolen rather than parked: a park here would hold it in the reconcile queue \
         waiting for a settle no epoch can perform"
    );
    assert_eq!(
        attempts, 1,
        "and the steal charged it, which is what bounds the failure"
    );
}

/// A stranded create is not served where nothing could reconcile it.
///
/// The reconcile searches the seller's catalogue for a correlation marker, and
/// no inventory is configured with a strategy that writes one, so serving the
/// item would settle it ambiguous to learn what the caller already knew. It
/// stays parked instead, still fencing its mapping, for the build that can
/// reconcile it. Both halves are asserted against the same fixture so the flag
/// is provably what decided.
#[sqlx::test(migrations = "./migrations")]
async fn a_stranded_create_is_left_parked_where_no_reconcile_can_run(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x77, true).await;
    enqueue_one(&engine, &tenant, 0x78, 0x79).await;
    let (stranded, attempt) = strand_a_create(&app, &engine, tenant.org, tenant.mapping).await;

    assert!(
        claim_with_reconcile(&app, tenant.org, DEVICE, 60, false)
            .await
            .is_none(),
        "the queue looks empty rather than handing out an item whose reconcile cannot run"
    );
    let (state, blocked_on, attempts, _, _) = item_disposition(&engine, tenant.org, stranded).await;
    assert_eq!(
        (state.as_str(), blocked_on.as_deref(), attempts),
        ("parked_live", Some(AWAITING_MARKETPLACE_ANSWER), 0),
        "and it is left exactly where the reaper put it, charged nothing"
    );
    assert_eq!(
        attempt_states(&engine, tenant.org, tenant.mapping).await,
        vec!["in_flight".to_owned()],
        "with its fence still standing, so no later pass can create a second listing"
    );

    let leased = claim_with_reconcile(&app, tenant.org, DEVICE, 60, true)
        .await
        .expect("the same item is claimable once a reconcile can run");
    assert_eq!(
        (leased.item, leased.stranded_attempt),
        (stranded, Some(attempt)),
        "the flag was the only thing standing between the device and the same item"
    );
}

/// An attempt whose intent names no title is not a reconcile.
///
/// The title is the whole of the identification under a marker-free strategy,
/// so an attempt that cannot supply one leaves the item an ordinary create
/// rather than sending a device to search its catalogue for nothing. The
/// attempt still stands, so that run abandons on the fence — bounded, and the
/// item stays reconcilable by a build that can read a title from it.
#[sqlx::test(migrations = "./migrations")]
async fn an_attempt_whose_intent_names_no_title_is_not_a_reconcile(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x7A, true).await;
    enqueue_one(&engine, &tenant, 0x7B, 0x7C).await;
    let (item, _) = strand_a_create(&app, &engine, tenant.org, tenant.mapping).await;
    // Non-empty and title-free, so a claim whose `EXISTS` had lost its
    // `entry->>0 = 'Title'` filter would still find a pair here and hand out a
    // reconcile it cannot identify. An empty array could not tell the two
    // apart.
    sqlx::query("UPDATE write_attempt SET intent = $1")
        .bind(serde_json::json!({
            "operation": "create",
            "entries": serde_json::to_value(vec![(
                FieldKey::Description,
                "A worksheet.".to_owned(),
            )])
            .expect("a field set encodes"),
            "files": [],
        }))
        .execute(&engine)
        .await
        .expect("the intent loses its entries");

    let leased = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the item is still claimable");
    assert_eq!(leased.item, item, "the same item comes back");
    assert_eq!(
        (leased.stranded_attempt, leased.stranded_title),
        (None, None),
        "both or neither: an attempt that names no title identifies no listing, so this is \
         not a reconcile and the device is not sent to search for one"
    );
}

/// A device that claims and abandons without attempting anything cannot loop
/// for ever: every cycle is charged, and the budget ends it.
///
/// This is the kill gate the entitlement work opens. A run stopped before its
/// first request now hands the lease back instead of settling the item, which
/// is right — and a requeue with no terminator would be a seller's item
/// claimed and dropped by an unentitled device on every poll, for ever. The
/// terminator is not the loop's own: it is the reaper charging the steal, and
/// the attempt budget settling the item once the charges reach the cap. The
/// claim's subscription filter is the other half and is asserted separately by
/// `a_plan_lapsed_past_the_grace_claims_nothing`; this half is the one that
/// holds even for a device that is entitled and merely keeps stopping.
#[sqlx::test(migrations = "./migrations")]
async fn abandoning_without_attempting_is_bounded_by_the_budget(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x7D, true).await;
    let item = enqueue_one(&engine, &tenant, 0x7E, 0x7F).await;
    let leases = LeaseRepo::new(engine.clone());

    let mut charged = Vec::new();
    for cycle in 0..ATTEMPTS_MAX {
        let leased = claim(&app, tenant.org, DEVICE, 60).await;
        let Some(leased) = leased else {
            panic!("cycle {cycle}: the item stopped being claimable before the budget ended it");
        };
        assert_eq!(leased.item, item, "the same item comes back each cycle");
        // The device stops before its first request: no attempt is opened and
        // nothing is settled, which is exactly what the driver now does when a
        // run is stopped before the form scrape.
        sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
            .execute(&engine)
            .await
            .expect("the lease ages");
        leases
            .expire_and_steal(Timestamp(T0.0 + 61_000), ATTEMPTS_MAX)
            .await
            .expect("the reaper runs");
        let (_, _, attempts, _, _) = item_disposition(&engine, tenant.org, item).await;
        charged.push(attempts);
    }

    let (last, climbing) = charged.split_last().expect("the loop ran");
    assert_eq!(
        climbing,
        (1..ATTEMPTS_MAX).collect::<Vec<_>>(),
        "every cycle but the last charged exactly one attempt, which is what bounds the loop: \
         a cycle that charged nothing would repeat for ever, and that is how a stranded \
         create's park arm differs from this one"
    );
    assert_eq!(
        *last,
        ATTEMPTS_MAX - 1,
        "and the last charges nothing because it settles instead: the reaper's give-up arm \
         takes an item whose next attempt would exceed the budget, so the terminator is the \
         settle rather than one more charge"
    );
    let (state, _, _, _, outcome) = item_disposition(&engine, tenant.org, item).await;
    assert_eq!(
        (state.as_str(), outcome.as_deref()),
        ("settled", Some("failed")),
        "and the budget ended it rather than the loop noticing anything itself"
    );
    assert!(
        claim(&app, tenant.org, DEVICE, 60).await.is_none(),
        "a settled item is not claimable, so the cycle cannot start again"
    );
}

/// An attested intent is recorded whole, and its hash is stored as given
/// rather than derived from what was recorded.
///
/// The ledger is the durable record of what a write went out under, and it has
/// to be, because the crate that supplies the attestation today is scheduled
/// for deletion. This is the storage half of that: the body arrives with the
/// declaration in it and the hash is carried, not recomputed. That the hash
/// excludes the attestation is the driver's property and is asserted there, by
/// `an_attestation_reaches_the_recorded_body_and_never_the_hashed_intent`,
/// which can hash because that crate has the hasher; the two together are the
/// guarantee, and the dangerous half is the hash, since it feeds the
/// idempotency key and a re-attested create must not become a second listing.
#[sqlx::test(migrations = "./migrations")]
async fn an_attested_intent_is_recorded_whole_with_its_hash_carried_not_recomputed(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0x80, true).await;
    enqueue_one(&engine, &tenant, 0x81, 0x82).await;
    let lease = claim(&app, tenant.org, DEVICE, 60)
        .await
        .expect("the item leases");

    let mut body = serde_json::json!({
        "operation": "create",
        "entries": recorded_entries(),
        "files": [],
    });
    let object = body.as_object_mut().expect("the intent body is an object");
    object.insert("attested_by".to_owned(), "the seller".into());
    object.insert("attested_at_ms".to_owned(), 1_756_000_000_000_i64.into());
    // Deliberately unrelated to the body's bytes, so a store that recomputed
    // the hash from what it was given could not accidentally match it.
    let hash = vec![0xA7; 32];

    WriteAttemptRepo::new(engine.clone())
        .open(
            &lease.lease_ref(),
            Uuid(*uuid::Uuid::new_v4().as_bytes()),
            &NewAttempt {
                mapping: tenant.mapping,
                intent: &AttemptIntent {
                    body: body.clone(),
                    hash: hash.clone(),
                },
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
        )
        .await
        .expect("the attested attempt opens");

    let (stored_body, stored_hash): (serde_json::Value, Vec<u8>) =
        sqlx::query_as("SELECT intent, intent_hash FROM write_attempt WHERE org_id = $1")
            .bind(db_uuid(tenant.org.0))
            .fetch_one(&engine)
            .await
            .expect("the attempt row reads");
    assert_eq!(
        stored_body, body,
        "the row records the intent whole, attestation included: this row is written before \
         the click and is what survives the deletion of the crate that supplies it"
    );
    assert_eq!(
        stored_hash, hash,
        "and the hash is carried rather than recomputed from the body, which is what keeps \
         the idempotency key an identity of what was written rather than of who attested"
    );
}
