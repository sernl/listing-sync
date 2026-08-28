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
    BudgetGrant, HaltCause, HaltRepo, ItemVerdict, JobReadRepo, JobRepo, LeaseRepo, MappingRepo,
    NewJob, NewJobItem, NewOutboxMessage, OutboxRepo, ProductRepo, RateBudgetRepo, StorageError,
};
use tam_types::{
    CanonicalTermId, ConnectionId, ContentHash, FailureCode, FailureDetail, FileId, FileKind,
    FileRole, InventoryId, JobId, ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent,
    PriceRule, ProductFile, ProductId, ScanOutcome, Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);

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
    Tenant { org, mapping }
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
                at: T0,
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

    let leases = LeaseRepo::new(engine.clone());
    let first = leases
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
        .expect("something is leasable");
    let second = leases
        .acquire("w2", T0, 60)
        .await
        .expect("the scan runs")
        .expect("the other tenant is leasable");
    assert_ne!(
        first.org, second.org,
        "one live lease per tenant: the second lease must come from the other org"
    );
    let third = leases.acquire("w3", T0, 60).await.expect("the scan runs");
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

#[sqlx::test(migrations = "./migrations")]
async fn a_stale_worker_is_fenced_after_a_steal(app: PgPool) {
    let engine = engine_pool(&app).await;
    let tenant = seed_tenant(&app, 0xA0, true).await;
    enqueue_one(&engine, &tenant, 0x11, 0x21).await;

    let leases = LeaseRepo::new(engine.clone());
    let lease = leases
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
        .expect("the item leases");
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

    let release = leases
        .acquire("w2", after_expiry, 60)
        .await
        .expect("the scan runs")
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
    let nothing = leases.acquire("w1", T0, 60).await.expect("the scan runs");
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
    let leased = leases
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
        .expect("the linked tenant now leases");
    assert_eq!(leased.org, unlinked.org, "only the linked tenant leases");

    leases
        .gate_connection(leased.org, InventoryId::TesGb)
        .await
        .expect("the gate flips");
    leases
        .settle(&leased.lease_ref(), &verdict(ItemOutcome::Blocked), T0)
        .await
        .expect("the item settles");
    enqueue_one(&engine, &unlinked, 0x13, 0x23).await;
    let gated = leases.acquire("w1", T0, 60).await.expect("the scan runs");
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
    leases
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
        .expect("the item leases");
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
                at: T0,
            },
            &[first_item, second_item],
        )
        .await
        .expect("the fixture job enqueues");

    let leases = LeaseRepo::new(engine.clone());
    let first = leases
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
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

    let second = leases
        .acquire("w2", T0, 60)
        .await
        .expect("the scan runs")
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
        .items_page(tenant.org, job, None, 10)
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
                at: T0,
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

    let leases = LeaseRepo::new(engine.clone());
    let first = leases
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
        .expect("something is leasable");
    let second = leases
        .acquire("w2", T0, 60)
        .await
        .expect("the scan runs")
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
                at: T0,
            },
            std::slice::from_ref(&new_item),
        )
        .await
        .expect("the request-keyed job enqueues");
    assert!(
        !created.replay,
        "the first carrier of the key creates a job"
    );

    let leased = LeaseRepo::new(engine.clone())
        .acquire("w1", T0, 60)
        .await
        .expect("the scan runs")
        .expect("the enqueued removal is leasable");
    assert_eq!(
        leased.operation, removal,
        "an item enqueued through the request-key path leases as what it was enqueued as"
    );
}
