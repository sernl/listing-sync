//! The engine-side guarantees, proven against the live database over the
//! tam_engine role: the per-tenant mutex is structural, a stolen lease fences
//! its previous holder, halts and the connection gate fail closed, the
//! attempt budget settles rather than loops, the outbox deduplicates and
//! dead-letters, and org_seq is consecutive per organisation.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{
    Binding, FieldPolicies, FieldPolicy, ItemOperation, ItemOutcome, JobItemId, Mapping,
    PublishMode,
};
use tam_marketplace::{
    IdempotencyKey, LifecycleTransition, ListingState, RemoteLifecycle, RemoteListingId,
};
use tam_storage::{
    revive_by_gap, revive_on, settle_if_complete, AttemptIntent, BudgetGrant, ConnectionAudit,
    DeviceClaim, DeviceRef, HaltCause, HaltRepo, ItemVerdict, JobReadRepo, JobRepo, LeaseRepo,
    LeasedItem, MappingRepo, NewAttempt, NewJob, NewJobItem, NewOutboxMessage, OutboxRepo,
    ProductRepo, RateBudgetRepo, StorageError, WriteAttemptRepo, REAUTH_REQUIRED,
};
use tam_types::{
    Actor, CanonicalTermId, ConnectionId, ContentHash, CopyFormat, FailureCode, FailureDetail,
    FileId, FileKind, FileRole, InventoryId, JobId, ListingCopy, MappingId, Marketplace, OrgId,
    PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome, Stamp,
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
}

/// The seller-device claim, as the tests take it. `acquire` is the other
/// branch's scan and no longer sees a Tes or Tpt item at all, so a fixture
/// that means to lease one claims as a device.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn claim(app: &PgPool, org: OrgId, device: &str, ttl: i64) -> Option<LeasedItem> {
    // Each worker name is a device now, so the fixture registers whichever one
    // is claiming. Registration is what these tests assume rather than what
    // they are about; the tests that are about it revoke explicitly.
    register_device(app, org, device).await;
    match LeaseRepo::new(app.clone())
        .claim_for_device(&DeviceRef { org, device }, ttl, 24, T0)
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
            .expect("the unparker runs"),
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
            .expect("the unparker runs"),
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
            .expect("the unparker runs"),
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
            .expect("the unparker runs"),
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

#[sqlx::test(migrations = "./migrations")]
async fn a_stale_worker_is_fenced_after_a_steal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    enqueue_one(&engine, &tenant, 0x11, 0x21).await;

    let leases = LeaseRepo::new(engine.clone());
    let lease = claim(&app, tenant.org, "w1", 60)
        .await
        .expect("the item leases");
    // The lease expiry is the database's own fact now, so a fixture that means
    // to expire one ages the row rather than naming a later instant.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(&engine)
        .await
        .expect("the lease ages");
    let after_expiry = Timestamp(T0.0 + 61_000);
    let touched = leases
        .expire_and_steal(after_expiry, 5)
        .await
        .expect("the stealer runs");
    assert_eq!(touched, 1, "the expired lease is stolen");

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

fn intent() -> AttemptIntent {
    AttemptIntent {
        body: serde_json::json!({ "fixture": true }),
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
            .expect("the unparker runs"),
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
            .expect("the unparker runs"),
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

    assert_eq!(
        leases
            .revive_expired(Timestamp(T0.0 + 3_000), ATTEMPTS_MAX)
            .await
            .expect("the unparker runs"),
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
            .expect("the unparker runs"),
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
                60,
                24,
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
            60,
            24,
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
