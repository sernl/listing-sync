//! The driver end to end against the live ledger with a scripted adapter:
//! the happy path settles Succeeded with its attempt committed, and an
//! ambiguous submit under HaltOnAmbiguity settles Ambiguous with the
//! tenant's inventory halted and the seller notified — the halting is the
//! test's point, because that is the account-safety gate.

#![cfg(feature = "pg-tests")]

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{ItemOutcome, StepBudget};
use tam_engine::driver::{run_item, DriverContext, MachineSeed, NowSource, RunVerdict};
use tam_marketplace::FetchReason;
use tam_marketplace::{
    AdapterError, AmbiguityCause, CreateStrategy, FieldSet, FormId, FormSchemaFingerprint,
    IdempotencyKey, ListingLocator, MarketplaceAdapter, ObservedListing, ProjectedListing,
    RemoteLifecycle, RemoteListingId, RemovalPlan, RevisePlan, SubmitEvidence,
};
use tam_storage::{
    HaltRepo, JobRepo, LeaseRepo, MappingRepo, NewJob, NewJobItem, ProductRepo, RateBudgetRepo,
    WriteAttemptRepo,
};
use tam_types::{ContentHash, FieldKey, InventoryId, JobId, MappingId, OrgId, Timestamp, Uuid};
use tokio_util::sync::CancellationToken;

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));

struct SteppingClock(AtomicI64);

impl NowSource for SteppingClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.fetch_add(1_000, Ordering::SeqCst))
    }
}

/// Scripted per call: submit answers are consumed in order; preflight and
/// read-back are fixed.
struct ScriptedAdapter {
    submit_answers: Vec<Result<SubmitEvidence, AdapterError>>,
    submit_cursor: AtomicUsize,
}

impl MarketplaceAdapter for ScriptedAdapter {
    fn inventory(&self) -> InventoryId {
        InventoryId::TesGb
    }

    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
        Ok(FieldSet {
            entries: vec![(FieldKey::Title, listing.title.clone())],
            files: listing.files.clone(),
        })
    }

    async fn assert_form_schema(
        &self,
        _org: OrgId,
        _form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        Ok(FormSchemaFingerprint(ContentHash([0x0F; 32])))
    }

    async fn submit(
        &self,
        _org: OrgId,
        _key: IdempotencyKey,
        _fields: FieldSet,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let position = self.submit_cursor.fetch_add(1, Ordering::SeqCst);
        self.submit_answers
            .get(position)
            .cloned()
            .unwrap_or(Err(AdapterError::Ambiguous(
                AmbiguityCause::ResponseEventLost,
            )))
    }

    /// The lifecycle writes are unreachable from this fixture: every item it
    /// drives is a create. They refuse rather than answer, so a test that
    /// grew a revise or a removal would say so instead of replaying a
    /// scripted submit answer that describes a different write.
    async fn revise(
        &self,
        _org: OrgId,
        _plan: RevisePlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        Err(AdapterError::Uncaptured {
            capability: "scripted.revise",
        })
    }

    async fn remove(
        &self,
        _org: OrgId,
        _plan: RemovalPlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        Err(AdapterError::Uncaptured {
            capability: "scripted.remove",
        })
    }

    async fn read_back(
        &self,
        _org: OrgId,
        locator: ListingLocator,
        _reason: FetchReason,
        _observed_at: Timestamp,
    ) -> Result<ObservedListing, AdapterError> {
        let id = match locator {
            ListingLocator::Durable(id) => id,
            ListingLocator::Marker { .. } => RemoteListingId::Tes {
                url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
            },
        };
        Ok(ObservedListing {
            id,
            fields: vec![(FieldKey::Title, "Fixture".to_owned())],
            lifecycle: RemoteLifecycle::Draft,
        })
    }
}

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
        .expect("the engine role connects")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed(app: &PgPool, engine: &PgPool) -> MappingId {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(app)
        .await
        .expect("the org inserts");
    let product = tam_types::ProductId(Uuid([0x01; 16]));
    let mapping = MappingId(Uuid([0x02; 16]));
    ProductRepo::new(app.clone())
        .insert(
            ORG,
            &tam_domain::CanonicalProduct {
                id: product,
                org: ORG,
                title: tam_types::Title("Fixture".to_owned()),
                body: tam_types::ListingCopy {
                    body: "Fixture".to_owned(),
                },
                payload: tam_types::PayloadSet::new(
                    tam_types::ProductFile {
                        id: tam_types::FileId(Uuid([0x03; 16])),
                        role: tam_types::FileRole::Payload,
                        kind: tam_types::FileKind::Pdf,
                        hash: ContentHash([0x04; 32]),
                        byte_len: 4,
                        scan: tam_types::ScanOutcome::Pending,
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
                price: tam_types::PriceIntent::Free,
            },
            T0,
        )
        .await
        .expect("the product inserts");
    MappingRepo::new(app.clone())
        .insert(
            ORG,
            &tam_domain::Mapping {
                id: mapping,
                org: ORG,
                product,
                inventory: InventoryId::TesGb,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: tam_types::PriceRule::Explicit(tam_types::PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: RemoteLifecycle::Absent,
            },
            0,
            T0,
        )
        .await
        .expect("the mapping inserts");
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes([0x05; 16]))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    tx.commit().await.expect("the fixture commits");

    JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: JobId(Uuid([0x06; 16])),
                inventory: InventoryId::TesGb,
                at: T0,
            },
            &[NewJobItem {
                item: tam_domain::JobItemId(Uuid([0x07; 16])),
                mapping,
                idempotency_key: IdempotencyKey(Uuid([0x08; 16])),
                operation: tam_domain::ItemOperation::Create,
            }],
        )
        .await
        .expect("the job enqueues");
    mapping
}

fn seed_machine(strategy: CreateStrategy) -> MachineSeed {
    MachineSeed {
        form: FormId(Uuid([0x09; 16])),
        fields: FieldSet {
            entries: vec![(FieldKey::Title, "Fixture".to_owned())],
            files: vec![],
        },
        intent_hash: ContentHash([0x0A; 32]),
        strategy,
        budget: StepBudget {
            actions_remaining: 20,
        },
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn run(
    app: &PgPool,
    adapter: &ScriptedAdapter,
    strategy: CreateStrategy,
) -> (RunVerdict, PgPool) {
    let engine = engine_pool(app).await;
    seed(app, &engine).await;
    let leases = LeaseRepo::new(engine.clone());
    let lease = leases
        .acquire("driver-test", T0, 600)
        .await
        .expect("the scan runs")
        .expect("the item leases");
    let clock = SteppingClock(AtomicI64::new(T0.0 + 1_000));
    let cancel = CancellationToken::new();
    let ctx = DriverContext {
        adapter,
        leases: &leases,
        halts: &HaltRepo::new(engine.clone()),
        attempts: &WriteAttemptRepo::new(engine.clone()),
        budgets: &RateBudgetRepo::new(engine.clone()),
        pool: &engine,
        clock: &clock,
        cancel: &cancel,
    };
    let verdict = run_item(&ctx, &lease, seed_machine(strategy))
        .await
        .expect("the driver runs");
    (verdict, engine)
}

/// The mapping the seed left unbound, read after the run. The bind is folded
/// into the attempt settle, so a committed attempt that carries a listing
/// must leave the mapping bound to it and stale.
async fn assert_bound_to(engine: &PgPool, url: &str) -> Result<(), sqlx::Error> {
    let bound: (
        String,
        Option<String>,
        Option<String>,
        String,
        Option<bool>,
        Option<bool>,
    ) = sqlx::query_as(
        "SELECT binding_state, remote_id_kind, remote_url, verify_state, \
                    verified_at IS NULL, verify_stale_since = first_seen_at \
             FROM mapping LIMIT 1",
    )
    .fetch_one(engine)
    .await?;
    assert_eq!(
        (
            bound.0.as_str(),
            bound.1.as_deref(),
            bound.2.as_deref(),
            bound.3.as_str(),
            bound.4,
            bound.5
        ),
        (
            "bound",
            Some("tes"),
            Some(url),
            "stale",
            Some(true),
            Some(true)
        ),
        "the settle must bind the mapping to the listing the write landed on, stale \
         because the report it settled on was never normalised"
    );
    Ok(())
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_happy_path_settles_succeeded(app: PgPool) {
    let adapter = ScriptedAdapter {
        submit_answers: vec![Ok(SubmitEvidence {
            http_status: Some(200),
            response_body_digest: None,
            landed_on_route: Some("https://www.tes.com/api/v2/resources/9001".to_owned()),
            landed: Some(RemoteListingId::Tes {
                url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
            }),
            observed_lag: false,
        })],
        submit_cursor: AtomicUsize::new(0),
    };
    let (verdict, engine) = run(&app, &adapter, CreateStrategy::HaltOnAmbiguity).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "a clean submit and read-back settles succeeded"
    );
    let attempt_state: String = sqlx::query_scalar("SELECT state FROM write_attempt LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the attempt row reads");
    assert_eq!(
        attempt_state, "committed",
        "the fencing row settled with the item"
    );
    let remote: (Option<String>, Option<String>, Option<i64>) = sqlx::query_as(
        "SELECT remote_id_kind, remote_url, remote_numeric_id FROM write_attempt LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the attempt row reads");
    assert_eq!(
        (remote.0.as_deref(), remote.1.as_deref(), remote.2),
        (
            Some("tes"),
            Some("https://www.tes.com/api/v2/resources/9001"),
            None
        ),
        "the committed attempt records the listing the write landed on, or nothing can \
         reconcile what it created"
    );
    assert_bound_to(&engine, "https://www.tes.com/api/v2/resources/9001")
        .await
        .expect("the mapping row reads");
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM job_event")
        .fetch_one(&engine)
        .await
        .expect("the events read");
    assert!(events >= 3, "the run recorded its actions: {events}");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_ambiguous_submit_halts_the_inventory(app: PgPool) {
    let adapter = ScriptedAdapter {
        submit_answers: vec![Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))],
        submit_cursor: AtomicUsize::new(0),
    };
    let (verdict, engine) = run(&app, &adapter, CreateStrategy::HaltOnAmbiguity).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Ambiguous),
        "an unreconcilable ambiguous submit settles ambiguous, never retries"
    );
    let halted: i64 = sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt")
        .fetch_one(&engine)
        .await
        .expect("the halt reads");
    assert_eq!(
        halted, 1,
        "the tenant's inventory halted — the account-safety gate"
    );
    let notified: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_message")
        .fetch_one(&engine)
        .await
        .expect("the outbox reads");
    assert!(notified >= 1, "the seller notification queued");
    let binding: String = sqlx::query_scalar("SELECT binding_state FROM mapping LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the mapping row reads");
    assert_eq!(
        binding, "unbound",
        "an ambiguous submit landed nothing, so there is nothing to bind"
    );
}
