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
use tam_engine::driver::{
    run_item, DriverContext, MachineSeed, NowSource, RunVerdict, PREFLIGHT_FAILURES_MAX,
};
use tam_engine::seed::verify_policy;
use tam_marketplace::FetchReason;
use tam_marketplace::{
    AdapterError, AmbiguityCause, CreateStrategy, FieldSet, FormId, FormSchemaFingerprint,
    IdempotencyKey, InstantPause, ListingLocator, MarketplaceAdapter, ObservedListing,
    ProjectedListing, RemoteLifecycle, RemoteListingId, RemovalPlan, RevisePlan, SubmitEvidence,
};
use tam_storage::{
    HaltRepo, JobRepo, LeaseRepo, MappingRepo, NewJob, NewJobItem, ProductRepo, RateBudgetRepo,
    WriteAttemptRepo,
};
use tam_types::{
    ContentHash, CopyFormat, FieldKey, InventoryId, JobId, MappingId, OrgId, Timestamp, Uuid,
};
use tokio_util::sync::CancellationToken;

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));

struct SteppingClock(AtomicI64);

impl NowSource for SteppingClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.fetch_add(1_000, Ordering::SeqCst))
    }
}

/// Scripted per call: submit and preflight answers are consumed in order, and
/// the read-back either observes the draft or answers the one condition the
/// fixture was built with.
struct ScriptedAdapter {
    submit_answers: Vec<Result<SubmitEvidence, AdapterError>>,
    submit_cursor: AtomicUsize,
    read_back_condition: Option<AdapterError>,
    /// Consumed in order, then held at the last entry: a preflight scripted
    /// to fail is standing in for a condition that holds across leases, not
    /// for one unlucky call, and the adapter is rebuilt per run in neither
    /// case — one fixture drives every lease the test takes.
    preflight_answers: Vec<Result<FormSchemaFingerprint, AdapterError>>,
    preflight_cursor: AtomicUsize,
}

impl ScriptedAdapter {
    fn answering(submit: Result<SubmitEvidence, AdapterError>) -> Self {
        Self {
            submit_answers: vec![submit],
            submit_cursor: AtomicUsize::new(0),
            read_back_condition: None,
            preflight_answers: vec![Ok(FormSchemaFingerprint(ContentHash([0x0F; 32])))],
            preflight_cursor: AtomicUsize::new(0),
        }
    }

    fn with_read_back_condition(mut self, condition: AdapterError) -> Self {
        self.read_back_condition = Some(condition);
        self
    }

    fn with_preflight_script(
        mut self,
        answers: Vec<Result<FormSchemaFingerprint, AdapterError>>,
    ) -> Self {
        self.preflight_answers = answers;
        self
    }
}

impl MarketplaceAdapter for ScriptedAdapter {
    fn inventory(&self) -> InventoryId {
        InventoryId::TesGb
    }

    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
        Ok(FieldSet {
            entries: vec![(FieldKey::Title, listing.title.clone())],
            files: listing.files.clone(),
            body_format: Some(listing.body_format),
        })
    }

    async fn assert_form_schema(
        &self,
        _org: OrgId,
        _form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let position = self.preflight_cursor.fetch_add(1, Ordering::SeqCst);
        self.preflight_answers
            .get(position)
            .or_else(|| self.preflight_answers.last())
            .cloned()
            .unwrap_or(Err(AdapterError::Uncaptured {
                capability: "scripted.preflight",
            }))
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
        if let Some(condition) = self.read_back_condition.clone() {
            return Err(condition);
        }
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
                    format: CopyFormat::Markdown,
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
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
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
                requires_bound_on: None,
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
            body_format: None,
        },
        intent_hash: ContentHash([0x0A; 32]),
        strategy,
        budget: StepBudget {
            actions_remaining: 20,
        },
        verify: verify_policy(InventoryId::TesGb),
    }
}

async fn run(
    app: &PgPool,
    adapter: &ScriptedAdapter,
    strategy: CreateStrategy,
) -> (RunVerdict, PgPool) {
    let engine = engine_pool(app).await;
    seed(app, &engine).await;
    let leases = LeaseRepo::new(engine.clone());
    let verdict = drive(&engine, &leases, adapter, strategy, T0).await;
    (verdict, engine)
}

const LEASE_SECONDS: i64 = 600;

/// One lease and one pump against a ledger the caller has already seeded, so
/// a test that needs the same item driven across several leases can take them
/// one at a time.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn drive(
    engine: &PgPool,
    leases: &LeaseRepo,
    adapter: &ScriptedAdapter,
    strategy: CreateStrategy,
    at: Timestamp,
) -> RunVerdict {
    let lease = leases
        .acquire("driver-test", at, LEASE_SECONDS)
        .await
        .expect("the scan runs")
        .expect("the item leases");
    let clock = SteppingClock(AtomicI64::new(at.0 + 1_000));
    let cancel = CancellationToken::new();
    let ctx = DriverContext {
        adapter,
        leases,
        halts: &HaltRepo::new(engine.clone()),
        attempts: &WriteAttemptRepo::new(engine.clone()),
        budgets: &RateBudgetRepo::new(engine.clone()),
        pool: engine,
        clock: &clock,
        cancel: &cancel,
        pause: &InstantPause,
    };
    run_item(&ctx, &lease, seed_machine(strategy))
        .await
        .expect("the driver runs")
}

/// The worker's maintenance pass, run far enough past the lease that the
/// abandoned item is stolen back onto the queue with its epoch bumped. What
/// it answers is the instant the next lease starts from.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn requeue(leases: &LeaseRepo, at: Timestamp) -> Timestamp {
    let after = Timestamp(at.0 + (LEASE_SECONDS + 1) * 1_000);
    leases
        .expire_and_steal(
            after,
            i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX),
        )
        .await
        .expect("the maintenance pass runs");
    after
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
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
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
    let adapter =
        ScriptedAdapter::answering(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut)));
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

/// The submit landed and named its listing; the verification read then
/// answered a condition rather than an observation.
fn landed_evidence() -> SubmitEvidence {
    SubmitEvidence {
        http_status: Some(200),
        response_body_digest: None,
        landed_on_route: Some("https://www.tes.com/api/v2/resources/9001".to_owned()),
        landed: Some(RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        }),
        observed_lag: false,
    }
}

/// `AwaitingReadBack` has arms for an observation and for `Ambiguous`, and
/// calls the other seven `AdapterError`s inapplicable. Handing one to the
/// machine returns `Err` out of `run_item` with the attempt still in flight,
/// which no caller settles and the next lease cannot get past — so the poll
/// treats them the way it treats the rate window closing, as evidence about
/// us rather than about the listing.
///
/// An expired session mid-poll is the ordinary trigger: `classify_read` maps
/// a 401, a 403 or a sign-in interstitial behind a 200 straight to it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_read_back_condition_abandons_rather_than_crashing_the_run(app: PgPool) {
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()))
        .with_read_back_condition(AdapterError::SessionExpired);
    let (verdict, engine) = run(&app, &adapter, CreateStrategy::HaltOnAmbiguity).await;
    let RunVerdict::Abandoned { reason } = verdict else {
        panic!("a read-back condition abandons into the stealer: {verdict:?}");
    };
    assert!(
        reason.contains("SessionExpired"),
        "the abandon names the condition that stopped the poll: {reason}"
    );
    let attempt: (String, bool) =
        sqlx::query_as("SELECT state, settled_at IS NULL FROM write_attempt LIMIT 1")
            .fetch_one(&engine)
            .await
            .expect("the attempt row reads");
    assert_eq!(
        (attempt.0.as_str(), attempt.1),
        ("in_flight", true),
        "the item is a create, so the fence is held rather than settled; what this test \
         pins is that the run reports a verdict at all instead of returning Err with the \
         same row left behind and no reason recorded"
    );
    let binding: String = sqlx::query_scalar("SELECT binding_state FROM mapping LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the mapping row reads");
    assert_eq!(
        binding, "unbound",
        "the read never observed the listing, so there is nothing to bind"
    );
}

/// The wedge this bound exists for, as the Phase 5 battery met it: the broker
/// held a stale Tes secret, so the write-bearing preflight answered
/// `Ambiguous(ReadBackIndeterminate)` on every lease. Each run abandoned, the
/// lease sat unexpired for its whole TTL, and `job_item_one_live_lease_per_org`
/// held the tenant's every other item behind it for that time — cycle after
/// cycle, until a human re-linked the connection.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_preflight_that_stays_indeterminate_stops_wedging_the_queue(app: PgPool) {
    let adapter =
        ScriptedAdapter::answering(Ok(landed_evidence())).with_preflight_script(vec![Err(
            AdapterError::Ambiguous(AmbiguityCause::ReadBackIndeterminate),
        )]);
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;
    let leases = LeaseRepo::new(engine.clone());

    let mut at = T0;
    let mut verdicts = Vec::new();
    for lease in 0..PREFLIGHT_FAILURES_MAX {
        if lease > 0 {
            at = requeue(&leases, at).await;
        }
        verdicts.push(
            drive(
                &engine,
                &leases,
                &adapter,
                CreateStrategy::HaltOnAmbiguity,
                at,
            )
            .await,
        );
    }

    let (last, earlier) = verdicts.split_last().expect("the loop ran at least once");
    for verdict in earlier {
        let RunVerdict::Abandoned { reason } = verdict else {
            panic!("under the bound the preflight is still a transient: {verdict:?}");
        };
        assert!(
            reason.contains("ReadBackIndeterminate"),
            "the abandon names the condition that stopped it: {reason}"
        );
    }
    assert_eq!(
        last,
        &RunVerdict::Settled(ItemOutcome::Blocked),
        "the same answer on the {PREFLIGHT_FAILURES_MAX}th lease is evidence about the \
         connection, so the item stops waiting for a session that is not coming"
    );

    let settled: (String, Option<String>, Option<String>, Option<String>, i32) = sqlx::query_as(
        "SELECT state, outcome, failure_code, failure_detail, preflight_failures \
         FROM job_item LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the item row reads");
    assert_eq!(
        (
            settled.0.as_str(),
            settled.1.as_deref(),
            settled.2.as_deref(),
            settled.4
        ),
        (
            "settled",
            Some("blocked"),
            Some("SessionExpired"),
            i32::try_from(PREFLIGHT_FAILURES_MAX).unwrap_or(i32::MAX)
        ),
        "the streak is what settled the item, and it settles blocked on the session \
         rather than failed on nothing"
    );
    let detail = settled.3.expect("a blocked item states why");
    assert!(
        detail.contains("re-linking"),
        "the report needs a sentence the seller can act on, not an enum: {detail}"
    );

    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "needs_reauth",
        "the condition is the connection's, so it surfaces there — otherwise the next \
         item repeats the whole streak against the same dead credential"
    );
    let notified: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox_message WHERE topic = 'email.parked_job'")
            .fetch_one(&engine)
            .await
            .expect("the outbox reads");
    assert_eq!(notified, 1, "the seller is asked to re-link exactly once");

    assert!(
        leases
            .acquire("driver-test", Timestamp(at.0 + 1_000_000), LEASE_SECONDS)
            .await
            .expect("the scan runs")
            .is_none(),
        "the gated connection holds the tenant's queue back rather than burning it"
    );
    sqlx::query("UPDATE connection SET state = 'linked'")
        .execute(&engine)
        .await
        .expect("the seller re-links");
    assert!(
        leases
            .acquire("driver-test", Timestamp(at.0 + 2_000_000), LEASE_SECONDS)
            .await
            .expect("the scan runs")
            .is_none(),
        "and the settled item is never re-selected, so the queue drains past it once the \
         connection is healthy again"
    );
}

/// The bound is on failures in a row, so one healthy preflight has to wipe
/// what came before it. Without the reset a connection that flickers reaches
/// the bound on its own arithmetic and blames a credential that works.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_healthy_preflight_wipes_the_streak(app: PgPool) {
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence())).with_preflight_script(vec![
        Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
        Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
        Ok(FormSchemaFingerprint(ContentHash([0x0F; 32]))),
    ]);
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;
    let leases = LeaseRepo::new(engine.clone());

    let first = drive(
        &engine,
        &leases,
        &adapter,
        CreateStrategy::HaltOnAmbiguity,
        T0,
    )
    .await;
    let at = requeue(&leases, T0).await;
    let second = drive(
        &engine,
        &leases,
        &adapter,
        CreateStrategy::HaltOnAmbiguity,
        at,
    )
    .await;
    assert!(
        matches!(first, RunVerdict::Abandoned { .. })
            && matches!(second, RunVerdict::Abandoned { .. }),
        "two failures is under the bound: {first:?}, {second:?}"
    );
    let streak: i32 = sqlx::query_scalar("SELECT preflight_failures FROM job_item LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the item row reads");
    assert_eq!(streak, 2, "both failures counted");

    let at = requeue(&leases, at).await;
    let third = drive(
        &engine,
        &leases,
        &adapter,
        CreateStrategy::HaltOnAmbiguity,
        at,
    )
    .await;
    assert_eq!(
        third,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "a healthy preflight lets the run finish"
    );
    let streak: i32 = sqlx::query_scalar("SELECT preflight_failures FROM job_item LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the item row reads");
    assert_eq!(
        streak, 0,
        "one success wipes the streak; leaving it at 2 would spend the bound on failures \
         that were never consecutive"
    );
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "linked",
        "nothing reached the bound, so the connection was never blamed"
    );
}

/// `attempt_count` is prior history this run did not choose: nothing resets
/// it, and every expired lease and every park revive advances it. An item
/// that arrives near `ATTEMPTS_MAX` therefore has fewer leases left than the
/// streak bound needs, and abandoning on the last one hands it to
/// `expire_and_steal`, which settles it `failed`/`Other` with a null detail,
/// no gate and no re-link prompt — leaving the next item to repeat the wedge.
/// Seeded at three so the reaper's predicate fires on the second lease, with
/// the streak still at two: the outcome has to be decided by the item's real
/// budget rather than by the two constants happening to be ordered.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_item_out_of_attempt_budget_still_settles_on_the_connection(app: PgPool) {
    let adapter =
        ScriptedAdapter::answering(Ok(landed_evidence())).with_preflight_script(vec![Err(
            AdapterError::Ambiguous(AmbiguityCause::ReadBackIndeterminate),
        )]);
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;
    sqlx::query("UPDATE job_item SET attempt_count = 3")
        .execute(&engine)
        .await
        .expect("the item takes its history");
    let leases = LeaseRepo::new(engine.clone());

    let first = drive(
        &engine,
        &leases,
        &adapter,
        CreateStrategy::HaltOnAmbiguity,
        T0,
    )
    .await;
    assert!(
        matches!(first, RunVerdict::Abandoned { .. }),
        "a fourth attempt is still affordable, so the stall bias holds: {first:?}"
    );
    let at = requeue(&leases, T0).await;
    let second = drive(
        &engine,
        &leases,
        &adapter,
        CreateStrategy::HaltOnAmbiguity,
        at,
    )
    .await;
    assert_eq!(
        second,
        RunVerdict::Settled(ItemOutcome::Blocked),
        "the fifth is the last one, and spending it on an abandon buys a failed/Other \
         settlement instead of a re-link the seller can act on"
    );

    let settled: (String, Option<String>, Option<String>, Option<String>, i32) = sqlx::query_as(
        "SELECT state, outcome, failure_code, failure_detail, preflight_failures \
         FROM job_item LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the item row reads");
    assert_eq!(
        (
            settled.0.as_str(),
            settled.1.as_deref(),
            settled.2.as_deref()
        ),
        ("settled", Some("blocked"), Some("SessionExpired")),
        "the connection-health outcome wins the race it used to lose"
    );
    assert!(
        settled.4 < i32::try_from(PREFLIGHT_FAILURES_MAX).unwrap_or(i32::MAX),
        "the streak never reached its bound, so what settled this item was the attempt \
         budget: {}",
        settled.4
    );
    let detail = settled.3.expect("a blocked item states why");
    assert!(
        detail.contains("re-linking"),
        "the report needs a sentence the seller can act on: {detail}"
    );
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "needs_reauth",
        "and the gate goes up, or the tenant's next item spends its budget the same way"
    );
}
