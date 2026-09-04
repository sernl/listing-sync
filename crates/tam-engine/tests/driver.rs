//! The driver end to end against the live ledger with a scripted adapter:
//! the happy path settles Succeeded with its attempt committed, and an
//! ambiguous submit under HaltOnAmbiguity settles Ambiguous with the
//! tenant's inventory halted and the seller notified — the halting is the
//! test's point, because that is the account-safety gate.

#![cfg(feature = "pg-tests")]

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{ItemOutcome, StepBudget};
use tam_engine::ledger::{to_wire_item, PgLedger, RandomIds, TokenCancellation};
use tam_engine::seed::verify_policy;
use tam_engine_driver::driver::{
    run_item, DriverContext, MachineSeed, NowSource, RunVerdict, PREFLIGHT_FAILURES_MAX,
};
use tam_engine_driver::memory::ScriptedReconcile;
use tam_marketplace::FetchReason;
use tam_marketplace::{
    AdapterError, AmbiguityCause, ChallengeKind, CreateStrategy, FieldSet, FormId,
    FormSchemaFingerprint, IdempotencyKey, InstantPause, LifecycleTransition, ListingLocator,
    ListingState, MarketplaceAdapter, ObservedListing, ProjectedListing, RemoteLifecycle,
    RemoteListingId, RemovalPlan, RevisePlan, SubmitEvidence,
};
use tam_storage::{
    ClaimPolicy, DeviceRef, JobRepo, LeaseRepo, MappingRepo, NewJob, NewJobItem, ProductRepo,
};
use tam_types::{
    Actor, ContentHash, CopyFormat, FieldKey, InventoryId, JobId, MappingId, OrgId, Stamp,
    SystemComponent, Timestamp, Uuid,
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
    /// What a lifecycle write answers, where a fixture drives one. `None`
    /// keeps the refusal below, so a test that grew a revise by accident says
    /// so rather than replaying a create's scripted answer.
    revise_answer: Option<Result<SubmitEvidence, AdapterError>>,
    /// The token the run is driven under, cancelled once the named call has
    /// answered. That is how a fixture places a suspended device precisely
    /// between a committed transition and the loop top that reads the token.
    cancel: Arc<CancellationToken>,
    cancel_after_submit: bool,
    cancel_after_read_back: bool,
}

impl ScriptedAdapter {
    fn answering(submit: Result<SubmitEvidence, AdapterError>) -> Self {
        Self {
            submit_answers: vec![submit],
            submit_cursor: AtomicUsize::new(0),
            read_back_condition: None,
            preflight_answers: vec![Ok(FormSchemaFingerprint(ContentHash([0x0F; 32])))],
            preflight_cursor: AtomicUsize::new(0),
            revise_answer: None,
            cancel: Arc::new(CancellationToken::new()),
            cancel_after_submit: false,
            cancel_after_read_back: false,
        }
    }

    fn revising(mut self, answer: Result<SubmitEvidence, AdapterError>) -> Self {
        self.revise_answer = Some(answer);
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
            appropriate_for_country: None,
        })
    }

    async fn assert_form_schema(
        &self,
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
        _key: IdempotencyKey,
        _fields: FieldSet,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let position = self.submit_cursor.fetch_add(1, Ordering::SeqCst);
        let answer =
            self.submit_answers
                .get(position)
                .cloned()
                .unwrap_or(Err(AdapterError::Ambiguous(
                    AmbiguityCause::ResponseEventLost,
                )));
        if self.cancel_after_submit {
            self.cancel.cancel();
        }
        answer
    }

    /// A lifecycle write answers only where a fixture scripted one. The
    /// default refuses rather than replaying a create's scripted answer, so a
    /// test that grew a revise or a removal by accident says so.
    async fn revise(
        &self,
        _plan: RevisePlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        self.revise_answer
            .clone()
            .unwrap_or(Err(AdapterError::Uncaptured {
                capability: "scripted.revise",
            }))
    }

    async fn remove(
        &self,
        _plan: RemovalPlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        Err(AdapterError::Uncaptured {
            capability: "scripted.remove",
        })
    }

    async fn read_back(
        &self,
        locator: ListingLocator,
        _reason: FetchReason,
        _observed_at: Timestamp,
    ) -> Result<ObservedListing, AdapterError> {
        if let Some(condition) = self.read_back_condition.clone() {
            return Err(condition);
        }
        let id = match locator {
            ListingLocator::Durable(id) => id,
            ListingLocator::Marker { .. } | ListingLocator::Recorded { .. } => {
                RemoteListingId::Tes {
                    url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
                }
            }
        };
        if self.cancel_after_read_back {
            self.cancel.cancel();
        }
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
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
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
        resume: None,
        attestation: None,
        form: FormId(Uuid([0x09; 16])),
        fields: FieldSet {
            entries: vec![(FieldKey::Title, "Fixture".to_owned())],
            files: vec![],
            body_format: None,
            appropriate_for_country: None,
        },
        intent_hash: ContentHash([0x0A; 32]),
        strategy,
        budget: StepBudget {
            actions_remaining: 20,
        },
        verify: verify_policy(InventoryId::TesGb),
    }
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
    app: &PgPool,
    engine: &PgPool,
    adapter: &ScriptedAdapter,
    strategy: CreateStrategy,
    at: Timestamp,
) -> RunVerdict {
    let lease = claim(app, DEVICE, LEASE_SECONDS)
        .await
        .expect("the item leases");
    let clock = SteppingClock(AtomicI64::new(at.0 + 1_000));
    let ledger = PgLedger::new(engine.clone(), lease.job);
    // The fixture's own token, so an adapter call can cancel the run it is
    // being driven under; never cancelled unless a hook was asked for.
    let cancel = TokenCancellation(&adapter.cancel);
    let ctx = DriverContext {
        adapter,
        // No body here drives a reconcile; one that did would say what the
        // seller's catalogue holds.
        reconcile: &ScriptedReconcile::could_not_read("this fixture drives no reconcile"),
        ledger: &ledger,
        clock: &clock,
        ids: &RandomIds,
        cancel: &cancel,
        pause: &InstantPause,
    };
    run_item(&ctx, &to_wire_item(&lease), seed_machine(strategy))
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
async fn requeue(engine: &PgPool, at: Timestamp) -> Timestamp {
    let leases = LeaseRepo::new(engine.clone());
    // The lease expiry is the database's own fact now, so a fixture cannot
    // reach it by advancing its clock: it ages the row instead, which is the
    // same condition a worker that went away leaves behind.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(engine)
        .await
        .expect("the lease ages");
    leases
        .expire_and_steal(
            Timestamp(at.0 + (LEASE_SECONDS + 1) * 1_000),
            i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX),
        )
        .await
        .expect("the maintenance pass runs");
    Timestamp(at.0 + (LEASE_SECONDS + 1) * 1_000)
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

    let mut at = T0;
    let mut verdicts = Vec::new();
    for lease in 0..PREFLIGHT_FAILURES_MAX {
        if lease > 0 {
            at = requeue(&engine, at).await;
        }
        verdicts.push(drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, at).await);
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
        claim(&app, DEVICE, LEASE_SECONDS).await.is_none(),
        "the gated connection holds the tenant's queue back rather than burning it"
    );
    sqlx::query("UPDATE connection SET state = 'linked'")
        .execute(&engine)
        .await
        .expect("the seller re-links");
    assert!(
        claim(&app, DEVICE, LEASE_SECONDS).await.is_none(),
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

    let first = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, T0).await;
    let at = requeue(&engine, T0).await;
    let second = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, at).await;
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

    let at = requeue(&engine, at).await;
    let third = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, at).await;
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

    let first = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, T0).await;
    assert!(
        matches!(first, RunVerdict::Abandoned { .. }),
        "a fourth attempt is still affordable, so the stall bias holds: {first:?}"
    );
    let at = requeue(&engine, T0).await;
    let second = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, at).await;
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

/// An ambiguous submit on a create this build can identify abandons the run
/// and leaves its fence standing, rather than halting the tenant.
///
/// The end-to-end half of the transition row. The machine steps to
/// `SyncState::Stranded`, a named state carrying the attempt and the locator,
/// and the driver's own arm for it returns `RunVerdict::Abandoned` without
/// settling anything. What that leaves behind is what this test is for and is
/// exactly what the machine tests cannot see, because they stop at the
/// transition: the attempt still in flight, so nothing can create a second
/// listing on that mapping, the mapping unbound, because nothing was observed,
/// and no halt row, because the fence already does the job a halt would.
///
/// The contrast with `a_challenge_on_a_create_holds_the_fence_and_mints_nothing_further`
/// above is deliberate. Both are a create whose fate is unknown; that one halts
/// under `HaltOnAmbiguity`, which embeds nothing a walk could find, and this
/// one records what it sent and waits, because a draft-then-publish create is
/// identifiable by the title its own attempt recorded.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_ambiguous_submit_under_draft_then_publish_abandons_and_holds_the_fence(app: PgPool) {
    let adapter =
        ScriptedAdapter::answering(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut)));
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;

    let verdict = drive(
        &app,
        &engine,
        &adapter,
        CreateStrategy::DraftThenPublish {
            draft_state: tam_marketplace::RemoteLifecycleKind::Draft,
        },
        T0,
    )
    .await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "the run stops without settling, so the item is still there to be reconciled when \
         the marketplace has answered: {verdict:?}"
    );

    let attempts: Vec<String> = sqlx::query_scalar("SELECT state FROM write_attempt")
        .fetch_all(&engine)
        .await
        .expect("the attempt rows read");
    assert_eq!(
        attempts,
        vec!["in_flight".to_owned()],
        "the fence is the point: while this row stands nothing can create a second \
         listing for that mapping, which is the failure this ledger cannot undo"
    );

    let bound: Option<String> = sqlx::query_scalar("SELECT binding_state FROM mapping LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the mapping row reads");
    assert_ne!(
        bound.as_deref(),
        Some("bound"),
        "nothing was bound, because nothing was observed"
    );

    let halted: i64 = sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt")
        .fetch_one(&engine)
        .await
        .expect("the halt table reads");
    assert_eq!(
        halted, 0,
        "and the tenant's inventory is not stopped: the fence already prevents the second \
         create, so a halt would only stop work that is still safe to do"
    );
}

/// A challenge the seller cannot clear, arriving mid-create, reaches the same
/// place a lost submit answer does.
///
/// The two are one situation: a challenge does not prove the write did not
/// land, because Tes posts the create and reads the resource afterwards, so an
/// edge block on that read arrives with a draft already minted. Under the
/// strategy this build configures, that create is identifiable by the title
/// its own attempt recorded, so the run stops and records rather than halting
/// the tenant.
///
/// What this asserts that the machine test cannot: the attempt is still in
/// flight when the run ends, which is the fence, and no halt row was written.
/// The contrast is with
/// `a_challenge_on_a_create_holds_the_fence_and_mints_nothing_further` below,
/// which drives the same adapter answer under `HaltOnAmbiguity` — a strategy
/// that leaves nothing a walk could find, and therefore still halts.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_challenge_on_a_create_under_draft_then_publish_strands_and_holds_the_fence(app: PgPool) {
    let adapter = ScriptedAdapter::answering(Err(AdapterError::Challenge(
        ChallengeKind::JavaScriptInterstitial,
    )));
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;

    let verdict = drive(
        &app,
        &engine,
        &adapter,
        CreateStrategy::DraftThenPublish {
            draft_state: tam_marketplace::RemoteLifecycleKind::Draft,
        },
        T0,
    )
    .await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "the run stops without settling, so the item is still there to be reconciled once \
         the marketplace can be read: {verdict:?}"
    );

    let attempts: Vec<String> = sqlx::query_scalar("SELECT state FROM write_attempt")
        .fetch_all(&engine)
        .await
        .expect("the attempt rows read");
    assert_eq!(
        attempts,
        vec!["in_flight".to_owned()],
        "the fence stands, which is what makes the halt unnecessary rather than merely \
         unpleasant: nothing can create a second listing for this mapping while it does"
    );

    let bound: Option<String> = sqlx::query_scalar("SELECT binding_state FROM mapping LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the mapping row reads");
    assert_ne!(
        bound.as_deref(),
        Some("bound"),
        "nothing was bound, because the edge answered before anything was observed"
    );

    let halted: i64 = sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt")
        .fetch_one(&engine)
        .await
        .expect("the halt table reads");
    assert_eq!(
        halted, 0,
        "and the tenant's queue keeps running: this item's fate is unknown, which is not a \
         reason to stop every sibling that is still safe to send"
    );

    // F9's own content, preserved on this arrival as on the terminal one: the
    // edge answered our address rather than our credential, so the seller is
    // never sent to re-link over it.
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "linked",
        "the credential was never in question and the status page must keep saying so"
    );
}

/// The duplicate-create path this arm opened, closed. Tes mints the draft in
/// `create_listing` and only then reads the resource back, so a Cloudflare
/// block on that read arrives *after* a listing exists. Settling terminal
/// there records no landing, leaves the mapping unbound, and the next lowering
/// of the same mapping asks for a second create — a duplicate live product,
/// the one failure this ledger cannot undo. So a create-challenge takes the
/// ambiguity route, and what stops the second create is the org-inventory
/// halt, which the lease scan reads.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_challenge_on_a_create_holds_the_fence_and_mints_nothing_further(app: PgPool) {
    let adapter = ScriptedAdapter::answering(Err(AdapterError::Challenge(
        ChallengeKind::JavaScriptInterstitial,
    )));
    let engine = engine_pool(&app).await;
    let mapping = seed(&app, &engine).await;
    enqueue_sibling(&engine, mapping, tam_domain::ItemOperation::Create).await;

    let verdict = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, T0).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Ambiguous),
        "a challenge on a create is a write that may have landed, and that is where \
         may-have-landed already goes"
    );

    let halted: i64 = sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt")
        .fetch_one(&engine)
        .await
        .expect("the halt table reads");
    assert_eq!(
        halted, 1,
        "the tenant's inventory halts, which is what freezes the queue instead of \
         minting a second listing"
    );

    // The assertion that is the whole point. The sibling is a second Create on
    // the same mapping; if the scan hands it out, the next run posts a second
    // draft over a listing that already exists.
    assert!(
        claim(&app, DEVICE, LEASE_SECONDS).await.is_none(),
        "no second create is leasable while the ambiguity stands"
    );
    let binding: String = sqlx::query_scalar("SELECT binding_state FROM mapping LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the mapping row reads");
    assert_eq!(
        binding, "unbound",
        "and nothing was bound on the strength of a read the edge answered"
    );

    // F9's own content, preserved: the halt is an inventory halt, never a
    // connection gate, so the seller is not sent to re-link over an egress
    // block.
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "linked",
        "the credential was never in question and the status page must keep saying so"
    );
}

/// The non-create side, where F9's settle-without-gating survives. A revise
/// re-applies the same fields, so re-running it mints nothing and there is no
/// duplicate hazard to fence against.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_challenge_on_a_revise_settles_blocked_and_leaves_the_connection_linked(app: PgPool) {
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence())).revising(Err(
        AdapterError::Challenge(ChallengeKind::JavaScriptInterstitial),
    ));
    let engine = engine_pool(&app).await;
    let mapping = seed(&app, &engine).await;
    enqueue_sibling(&engine, mapping, revision()).await;

    // The create leases first (FIFO on created_at); settle it out of the way
    // so the revise is what the next scan hands over.
    let first = drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, T0).await;
    assert_eq!(
        first,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "the create is fixture, not subject: {first:?}"
    );
    let verdict = drive(
        &app,
        &engine,
        &adapter,
        CreateStrategy::HaltOnAmbiguity,
        Timestamp(T0.0 + 1_000),
    )
    .await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Blocked),
        "a revise cannot mint a second anything, so the challenge is terminal"
    );

    let settled: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT failure_code, failure_detail FROM job_item WHERE id = $1")
            .bind(uuid::Uuid::from_bytes([0x09; 16]))
            .fetch_one(&engine)
            .await
            .expect("the item row reads");
    assert_eq!(
        settled.0.as_deref(),
        Some("ChallengePresented"),
        "the code names the condition rather than borrowing the session's"
    );
    let detail = settled.1.expect("a blocked item states why");
    assert!(
        !detail.contains("re-link") && detail.contains("ours to do"),
        "the seller is told whose problem this is, and it is not theirs: {detail}"
    );
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "linked",
        "no gate: the credential was never in question"
    );
    let blocked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_event WHERE kind = 'ItemBlocked' \
           AND payload->>'cause' = 'challenge'",
    )
    .fetch_one(&engine)
    .await
    .expect("the events read");
    assert_eq!(
        blocked, 1,
        "and the cause reaches the progress stream, which no gate would have carried"
    );
}

/// The preflight half. `AdapterError::Challenge` reaches the driver from the
/// preflight's read steps, where `classify_read` splits a 401 or a 403 on
/// Cloudflare's own markers, so the distinction here is data rather than
/// invention. The write steps cannot carry it — `classify_write` maps 401,
/// 403 and 429 to `Ambiguous` on purpose — and
/// `a_preflight_that_stays_indeterminate_stops_wedging_the_queue` pins that
/// arm still gating, which is the behaviour the streak bound shipped with.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_preflight_challenge_settles_blocked_without_gating(app: PgPool) {
    let adapter =
        ScriptedAdapter::answering(Ok(landed_evidence())).with_preflight_script(vec![Err(
            AdapterError::Challenge(ChallengeKind::JavaScriptInterstitial),
        )]);
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;

    let mut at = T0;
    let mut verdicts = Vec::new();
    for lease in 0..PREFLIGHT_FAILURES_MAX {
        if lease > 0 {
            at = requeue(&engine, at).await;
        }
        verdicts.push(drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, at).await);
    }
    let last = verdicts.last().expect("the loop ran at least once");
    assert_eq!(
        last,
        &RunVerdict::Settled(ItemOutcome::Blocked),
        "the streak bound still ends the run; what the error class decides is what the \
         ending says"
    );

    let settled: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT failure_code, failure_detail FROM job_item LIMIT 1")
            .fetch_one(&engine)
            .await
            .expect("the item row reads");
    assert_eq!(
        settled.0.as_deref(),
        Some("ChallengePresented"),
        "a challenge the preflight could name is not a session expiry"
    );
    let detail = settled.1.expect("a blocked item states why");
    assert!(
        !detail.contains("re-link"),
        "and the seller is not sent to fix a credential that works: {detail}"
    );
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(connection, "linked", "nothing was gated");
    // Scoped to the topic rather than the table: the item was the job's last,
    // so `settle_if_complete` queues the job's own `email.job_settled`, which
    // is the seller being told their job finished and not a re-link prompt.
    let prompted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox_message WHERE topic = 'email.parked_job'")
            .fetch_one(&engine)
            .await
            .expect("the outbox reads");
    assert_eq!(
        prompted, 0,
        "and nobody is emailed a re-link prompt for work that is ours to retry"
    );
}

/// A second item for the same tenant, so a test can ask whether the queue kept
/// moving. Its own job, because `enqueue` writes a job and its items together.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn enqueue_sibling(
    engine: &PgPool,
    mapping: MappingId,
    operation: tam_domain::ItemOperation,
) {
    JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: JobId(Uuid([0x0B; 16])),
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[NewJobItem {
                item: tam_domain::JobItemId(Uuid([0x09; 16])),
                mapping,
                idempotency_key: IdempotencyKey(Uuid([0x0A; 16])),
                operation,
                requires_bound_on: None,
            }],
        )
        .await
        .expect("the sibling job enqueues");
}

/// A draft-to-draft revise against the listing the fixture's create lands on.
fn revision() -> tam_domain::ItemOperation {
    tam_domain::ItemOperation::Revise {
        subject: RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        },
        transition: LifecycleTransition {
            from: ListingState::Draft,
            to: ListingState::Draft,
        },
    }
}

/// A streak can mix classes: a Cloudflare block on one lease and a genuinely
/// lapsed session on the next. Judging by whichever error arrived last makes
/// the seller's instructions depend on arrival order, so the streak carries
/// the conjunction and takes the seller-actionable floor unless every failure
/// in it was the edge's. The session expiry sits in the middle here on
/// purpose: the last error is a challenge, so a last-one-wins rule would gate
/// nothing and tell the seller the retry was ours.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_mixed_preflight_streak_takes_the_seller_actionable_floor(app: PgPool) {
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence())).with_preflight_script(vec![
        Err(AdapterError::Challenge(
            ChallengeKind::JavaScriptInterstitial,
        )),
        Err(AdapterError::SessionExpired),
        Err(AdapterError::Challenge(
            ChallengeKind::JavaScriptInterstitial,
        )),
    ]);
    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;

    let mut at = T0;
    let mut verdicts = Vec::new();
    for lease in 0..PREFLIGHT_FAILURES_MAX {
        if lease > 0 {
            at = requeue(&engine, at).await;
        }
        verdicts.push(drive(&app, &engine, &adapter, CreateStrategy::HaltOnAmbiguity, at).await);
    }
    assert_eq!(
        verdicts.last(),
        Some(&RunVerdict::Settled(ItemOutcome::Blocked)),
        "the bound still ends the run"
    );

    let settled: (Option<String>, Option<String>, bool) = sqlx::query_as(
        "SELECT failure_code, failure_detail, preflight_challenge_only FROM job_item LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the item row reads");
    assert!(
        !settled.2,
        "one non-edge failure clears the conjunction for the whole streak"
    );
    assert_eq!(
        settled.0.as_deref(),
        Some("SessionExpired"),
        "so the verdict is the seller-actionable one, even though the last error was not"
    );
    let detail = settled.1.expect("a blocked item states why");
    assert!(
        detail.contains("re-link"),
        "a real auth failure in the streak is a fact the seller can act on: {detail}"
    );
    let connection: String = sqlx::query_scalar("SELECT state FROM connection LIMIT 1")
        .fetch_one(&engine)
        .await
        .expect("the connection row reads");
    assert_eq!(
        connection, "needs_reauth",
        "and the gate goes up, because a re-link is what clears the half we can name"
    );
}

/// The seller's device, which the seller-device claim admits only if it is
/// registered and unrevoked.
const DEVICE: &str = "engine-test-device";

/// Tes is the seller-device branch, so a fixture that means to lease claims as
/// a device rather than through `acquire`, which no longer sees these items.
/// The claim is org-pinned by forced row-level security, so it runs on the app
/// pool; the engine pool is BYPASSRLS and would not be pinned by it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn claim(app: &PgPool, device: &str, ttl: i64) -> Option<tam_storage::LeasedItem> {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, \
                             first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'fixture', 'linux', 'x86_64', '0.0.0', now(), now()) \
         ON CONFLICT (org_id, id) DO NOTHING",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(device)
    .execute(&mut *tx)
    .await
    .expect("the fixture device registers");
    // The claim serves a device only work whose marketplace it holds a
    // connected session for, which is what the real device establishes on its
    // first check-in.
    sqlx::query(
        "INSERT INTO device_marketplace_session \
             (org_id, device_id, marketplace, linked_at, last_used_at, status) \
         VALUES ($1, $2, 'tes', now(), now(), 'connected'), \
                ($1, $2, 'tpt', now(), now(), 'connected') \
         ON CONFLICT (org_id, device_id, marketplace) DO NOTHING",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(device)
    .execute(&mut *tx)
    .await
    .expect("the fixture sessions register");
    tx.commit().await.expect("the fixture device commits");
    match tam_storage::LeaseRepo::new(app.clone())
        .claim_for_device(
            &DeviceRef { org: ORG, device },
            &ClaimPolicy {
                ttl_seconds: ttl,
                grace_hours: 24,
                marketplace: None,
                reconcile: true,
            },
            T0,
        )
        .await
        .expect("the claim runs")
    {
        tam_storage::DeviceClaim::Leased(item) => Some(*item),
        tam_storage::DeviceClaim::Empty | tam_storage::DeviceClaim::HeldByAnotherDevice => None,
    }
}

/// A device's ledger write records two instants, and they are distinct.
///
/// The seller's clock is evidence of what their machine believed; ours is the
/// record of when we heard it. Collapsing them would let a device's clock date
/// our ledger, and `org_seq` — not either column — is what orders events, so
/// keeping both costs nothing and losing one loses the audit trail's honesty.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_write_records_both_the_asserted_instant_and_our_receipt(app: PgPool) {
    use tam_engine_driver::ports::ItemLedger;

    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;
    let lease = claim(&app, DEVICE, LEASE_SECONDS)
        .await
        .expect("the item leases");
    // An instant the seller's machine asserts, deliberately far from now.
    let asserted = Timestamp(1_600_000_000_000);
    let ledger = PgLedger::for_device(engine.clone(), lease.job, DEVICE.to_owned());
    ledger
        .record_event(
            &to_wire_item(&lease).lease_ref(),
            &tam_types::JobEventPayload::ItemLeased {
                worker: DEVICE.to_owned(),
                lease_epoch: lease.lease_epoch,
            },
            asserted,
        )
        .await
        .expect("the device records its event");

    // Read as epoch milliseconds so the assertion compares the two instants
    // rather than a timestamp library's rendering of them.
    let (created, recorded): (i64, Option<i64>) = sqlx::query_as(
        "SELECT (extract(epoch FROM created_at) * 1000)::bigint, \
                (extract(epoch FROM asserted_at) * 1000)::bigint \
         FROM job_event WHERE kind = 'ItemLeased' ORDER BY org_seq DESC LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the event row reads");
    let recorded = recorded.expect("a device-written row records what the device asserted");
    assert_eq!(
        recorded, asserted.0,
        "the seller's own instant is preserved exactly, as their assertion"
    );
    assert_ne!(
        created, asserted.0,
        "and our receipt is our own clock rather than theirs, which is the whole point \
         of recording two"
    );

    let actor: (String, String) = sqlx::query_as(
        "SELECT actor_kind, actor_id FROM job_event \
         WHERE kind = 'ItemLeased' ORDER BY org_seq DESC LIMIT 1",
    )
    .fetch_one(&engine)
    .await
    .expect("the actor reads");
    assert_eq!(
        (actor.0.as_str(), actor.1.as_str()),
        ("system", "device"),
        "and the row is attributed to the device rather than to our own engine"
    );
}

/// The rate grant's window and ceiling are the server's, whatever the device
/// says.
///
/// The port takes no window and no ceiling by construction, so the only way to
/// assert the server derives them is behaviourally: grants issued against one
/// connection exhaust at the server's own limit, and the instant the device
/// supplies moves the window rather than the allowance.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_rate_window_and_ceiling_are_the_servers_however_the_device_asks(app: PgPool) {
    use tam_engine_driver::ports::ItemLedger;
    use tam_engine_driver::vocabulary::{BudgetGrant, GrantKind};

    let engine = engine_pool(&app).await;
    seed(&app, &engine).await;
    let lease = claim(&app, DEVICE, LEASE_SECONDS)
        .await
        .expect("the item leases");
    let wire = to_wire_item(&lease);
    let ledger = PgLedger::for_device(engine.clone(), lease.job, DEVICE.to_owned());
    let connection = ledger
        .connection_for(&wire.lease_ref(), lease.inventory)
        .await
        .expect("the connection reads")
        .expect("the fixture links one");

    // Every grant names the same instant, so they all fall in one window. The
    // ceiling is the server's copy of the per-minute limit, and the device
    // named neither it nor the window.
    let ceiling = tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX.get();
    let mut granted = 0_u32;
    for _ in 0..ceiling + 2 {
        match ledger
            .request_grant(&wire.lease_ref(), connection, GrantKind::Write, T0)
            .await
            .expect("the grant runs")
        {
            BudgetGrant::Granted { .. } => granted += 1,
            BudgetGrant::Exhausted => break,
        }
    }
    assert_eq!(
        u64::from(granted),
        u64::from(ceiling),
        "the window closed at the server's own ceiling, which the device never named: \
         a governed party that supplied either would be setting its own rate limit"
    );

    // A later instant is a later window, so the allowance refreshes without
    // the device having asked for more.
    let next_window = Timestamp(T0.0 + 60_000);
    assert!(
        matches!(
            ledger
                .request_grant(&wire.lease_ref(), connection, GrantKind::Write, next_window)
                .await
                .expect("the grant runs"),
            BudgetGrant::Granted { .. }
        ),
        "the window is derived from the instant rather than carried by the caller, so \
         the next minute grants again"
    );
}
