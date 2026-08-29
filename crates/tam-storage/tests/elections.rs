//! The election queue: the dedup key that keeps a stale question from
//! answering a changed product, the two-layer legal backstop, the engine's
//! grant asymmetry, and tenant isolation.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_domain::equivalence::{
    Election, ElectionAnswer, ElectionRule, ElectionRuleError, ElectionTrigger,
    ElectionTriggerKind, Loss, LossKind, NewElectionRule, PricingBranch,
};
use tam_domain::{
    Binding, Decider, FieldPolicies, FieldPolicy, Mapping, PublishMode, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_domain::{ItemOperation, JobItemId};
use tam_marketplace::{IdempotencyKey, RemoteLifecycle};
use tam_storage::{
    ElectionRepo, JobRepo, LossScope, MappingRepo, NewAnswer, NewJob, NewJobItem, ProductRepo,
    StorageError,
};
use tam_types::{
    AttemptId, CanonicalTermId, InventoryId, JobId, MappingId, OrgId, PriceIntent, PriceRule,
    Timestamp, Uuid,
};

use common::{minimal_product, seed_org_a, ORG_A};

const T0: Timestamp = Timestamp(1_000);
const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn fixture(app: &PgPool) -> MappingId {
    seed_org_a(app).await.expect("org-a seeds");
    ProductRepo::new(app.clone())
        .insert(ORG_A, &minimal_product(), T0)
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(app.clone())
        .insert(
            ORG_A,
            &Mapping {
                id: MAPPING_1,
                org: ORG_A,
                product: minimal_product().id,
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
    MAPPING_1
}

fn supply(pricing: PricingBranch) -> Election {
    Election {
        product: minimal_product().id,
        inventory: InventoryId::TesGb,
        axis: TermKind::Licence,
        trigger: ElectionTrigger::Supply { pricing },
    }
}

fn licence(token: &str) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
        segments: vec![token.to_owned()],
        native_id: Some(token.to_owned()),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_price_change_asks_the_new_question_rather_than_keeping_the_stale_one(app: PgPool) {
    let mapping = fixture(&app).await;
    let repo = ElectionRepo::new(app);
    let first = repo
        .raise(ORG_A, mapping, &[supply(PricingBranch::Free)], T0)
        .await
        .expect("the free question raises");
    assert_eq!((first.new, first.already_open), (1, 0));

    let again = repo
        .raise(ORG_A, mapping, &[supply(PricingBranch::Free)], T0)
        .await
        .expect("the second raise dedups");
    assert_eq!((again.new, again.already_open), (0, 1));

    let paid = repo
        .raise(ORG_A, mapping, &[supply(PricingBranch::Paid)], T0)
        .await
        .expect("the paid question raises");
    assert_eq!(
        (paid.new, paid.already_open),
        (1, 0),
        "without trigger_key in the dedup index the seller would answer the free question \
         — a Creative Commons value — for a paid listing, which is the one combination \
         Tes refuses"
    );
    assert_eq!(
        repo.open_items(ORG_A).await.expect("the queue reads").len(),
        2
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn answering_settles_the_item_and_a_second_answer_is_refused(app: PgPool) {
    let mapping = fixture(&app).await;
    let repo = ElectionRepo::new(app);
    repo.raise(ORG_A, mapping, &[supply(PricingBranch::Free)], T0)
        .await
        .expect("the question raises");
    let item = repo.open_items(ORG_A).await.expect("the queue reads")[0].id;

    let report = repo
        .answer(
            ORG_A,
            NewAnswer {
                item,
                paths: &[licence("CC-BY-SA")],
                at: T0,
                promote: None,
            },
        )
        .await
        .expect("the answer records");
    assert_eq!(
        report.revived, 0,
        "nothing was parked behind it in this fixture, and the count is reported rather \
         than assumed because a revive that releases nothing is what an unpinned update \
         looks like"
    );
    assert!(repo
        .open_items(ORG_A)
        .await
        .expect("the queue reads")
        .is_empty());
    assert!(
        matches!(
            repo.answer(
                ORG_A,
                NewAnswer {
                    item,
                    paths: &[licence("CC-BY")],
                    at: T0,
                    promote: None,
                },
            )
            .await,
            Err(StorageError::Inconsistent { .. })
        ),
        "two concurrent answers cannot both settle one question"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_standing_rule_round_trips_and_a_legal_axis_refuses_delegation_at_both_layers(
    app: PgPool,
) {
    seed_org_a(&app).await.expect("org-a seeds");
    let repo = ElectionRepo::new(app.clone());
    let rule = ElectionRule::new(NewElectionRule {
        org: ORG_A,
        inventory: InventoryId::TesGb,
        axis: TermKind::Licence,
        trigger_kind: ElectionTriggerKind::Supply,
        trigger_key: Some("free".to_owned()),
        answer: ElectionAnswer::Value {
            path: licence("CC-BY-SA"),
        },
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: T0,
    })
    .expect("a value answer on a legal axis is admissible");
    repo.upsert_rule(&rule).await.expect("the rule records");
    assert_eq!(
        repo.rules(ORG_A).await.expect("the rules read"),
        vec![rule],
        "the stored answer round-trips whole, so satisfied_by matches on the same value \
         it was given"
    );

    assert!(
        matches!(
            ElectionRule::new(NewElectionRule {
                org: ORG_A,
                inventory: InventoryId::TesGb,
                axis: TermKind::Licence,
                trigger_kind: ElectionTriggerKind::Supply,
                trigger_key: Some("free".to_owned()),
                answer: ElectionAnswer::Delegate,
                decided_by: Decider::Imported {
                    source: "test fixture".to_owned(),
                },
                decided_at: T0,
            }),
            Err(ElectionRuleError::NotDelegable(_))
        ),
        "the domain half of the backstop"
    );
    let direct = sqlx::query(
        "INSERT INTO election_rule \
         (org_id, inventory, axis, trigger_kind, trigger_key, answer_kind, answer, \
          decided_by, decided_source, decided_at) \
         VALUES ($1, 'tes_gb', 'licence', 'supply', 'paid', 'delegate', '[]', \
                 'imported', 'direct', now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .execute(&app)
    .await;
    assert!(
        direct.is_err(),
        "and the database half, because the domain check passes for anything writing SQL"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_misspelled_axis_is_refused_by_the_column_check(app: PgPool) {
    seed_org_a(&app).await.expect("org-a seeds");
    let refused = sqlx::query(
        "INSERT INTO election_rule \
         (org_id, inventory, axis, trigger_kind, trigger_key, answer_kind, answer, \
          decided_by, decided_source, decided_at) \
         VALUES ($1, 'tes_gb', 'licences', 'supply', 'free', 'value', \
                 '[{\"segments\": [\"CC-BY\"], \"native_id\": \"CC-BY\"}]', \
                 'imported', 'direct', now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .execute(&app)
    .await;
    assert!(
        refused.is_err(),
        "the delegate CHECK states a rule over a set of axis values, and a set is only \
         trustworthy if the column's spelling is pinned"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn tenant_b_sees_none_of_tenant_a_s_decisions(app: PgPool) {
    let mapping = fixture(&app).await;
    ElectionRepo::new(app.clone())
        .raise(ORG_A, mapping, &[supply(PricingBranch::Free)], T0)
        .await
        .expect("the question raises");
    let other = OrgId(Uuid([0xB2; 16]));
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(other.0 .0))
        .execute(&app)
        .await
        .expect("org-b seeds");
    assert!(
        ElectionRepo::new(app)
            .open_items(other)
            .await
            .expect("the queue reads")
            .is_empty(),
        "a seller's legal and presentational choices are theirs alone"
    );
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects to the test database")
}

/// The engine raises and reads; the seller resolves. Grants are easy to
/// over-issue while making a test pass, so the asymmetry is pinned rather than
/// reviewed: the engine may enqueue the question its own sync run discovered,
/// and may never author the answer.
#[sqlx::test(migrations = "./migrations")]
async fn the_engine_may_raise_a_question_and_may_never_answer_one(app: PgPool) {
    let mapping = fixture(&app).await;
    let engine = engine_pool(&app).await;

    let raised = sqlx::query(
        "INSERT INTO election_item \
         (org_id, id, product_id, inventory, axis, trigger_kind, trigger_key, raised_by, \
          raised_at, state) \
         VALUES ($1, $2, $3, 'tes_gb', 'licence', 'supply', 'free', $4, now(), 'open')",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x5A; 16]))
    .bind(uuid::Uuid::from_bytes(minimal_product().id.0 .0))
    .bind(uuid::Uuid::from_bytes(mapping.0 .0))
    .execute(&engine)
    .await;
    assert!(
        raised.is_ok(),
        "the sync run that discovers the question is the engine's, and making the seller \
         poll for it would leave the decision surface lagging the discovery: {raised:?}"
    );

    let answered = sqlx::query("UPDATE election_item SET state = 'withdrawn'")
        .execute(&engine)
        .await;
    assert!(
        answered.is_err(),
        "the engine never settles a seller's question"
    );

    let authored = sqlx::query(
        "INSERT INTO election_rule \
         (org_id, inventory, axis, trigger_kind, trigger_key, answer_kind, answer, \
          decided_by, decided_source, decided_at) \
         VALUES ($1, 'tes_gb', 'licence', 'supply', 'free', 'value', \
                 '[{\"segments\": [\"CC-BY\"], \"native_id\": \"CC-BY\"}]', \
                 'imported', 'engine', now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .execute(&engine)
    .await;
    assert!(
        authored.is_err(),
        "and never states a standing policy on the seller's behalf"
    );
}

/// A loss is append-only as a fence rather than as a convention, and survives
/// the mapping it was recorded against.
#[sqlx::test(migrations = "./migrations")]
async fn a_loss_is_recorded_per_attempt_and_the_api_role_cannot_erase_it(app: PgPool) {
    let mapping = fixture(&app).await;
    let repo = MappingRepo::new(app.clone());
    let scope = LossScope {
        org: ORG_A,
        mapping,
        attempt: AttemptId(Uuid([0x7A; 16])),
        at: T0,
    };
    let losses = [
        Loss::NoTargetField {
            axis: TermKind::Licence,
            value: licence("CC-BY-ND"),
        },
        Loss::Broadened {
            to: licence("CC-BY"),
            dropped: vec![CanonicalTermId(Uuid([0x11; 16]))],
        },
    ];
    assert_eq!(
        repo.record_losses(scope, &losses)
            .await
            .expect("the losses record"),
        2
    );

    let again = LossScope {
        attempt: AttemptId(Uuid([0x7B; 16])),
        ..scope
    };
    repo.record_losses(again, &losses[..1])
        .await
        .expect("a second attempt records its own");
    let read = repo.losses(ORG_A, mapping).await.expect("the losses read");
    assert_eq!(
        read.len(),
        3,
        "a re-projection writes a new attempt's rows rather than replacing the old ones, \
         which is what buys the engine's no-delete rule"
    );
    assert_eq!(read[0].kind, LossKind::NoTargetField);
    assert_eq!(read[0].axis, Some(TermKind::Licence));
    assert_eq!(
        read[0].detail["value"]["native_id"],
        serde_json::json!("CC-BY-ND"),
        "the seller sees which grant the target had nowhere to put"
    );

    for statement in [
        "UPDATE mapping_loss SET kind = 'collapsed'",
        "DELETE FROM mapping_loss",
    ] {
        assert!(
            sqlx::query(statement).execute(&app).await.is_err(),
            "tam_app owns every table, so append-only is a revoke and not a comment: \
             {statement}"
        );
    }
}

const JOB: JobId = JobId(Uuid([0x41; 16]));
const ITEM: JobItemId = JobItemId(Uuid([0x42; 16]));

/// One item on the fixture mapping, in whatever state the caller needs it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn item_in(app: &PgPool, mapping: MappingId, state: &str, gate: Option<&str>) {
    JobRepo::new(app.clone())
        .enqueue(
            ORG_A,
            &NewJob {
                job: JOB,
                inventory: InventoryId::TesGb,
                at: T0,
            },
            &[NewJobItem {
                item: ITEM,
                mapping,
                idempotency_key: IdempotencyKey(Uuid([0x43; 16])),
                operation: ItemOperation::Create,
                requires_bound_on: None,
            }],
        )
        .await
        .expect("the fixture job enqueues");
    set_item(app, state, gate).await;
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn set_item(app: &PgPool, state: &str, gate: Option<&str>) {
    let mut tx = app.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "UPDATE job_item SET state = $2, blocked_on = $3, \
                park_expires_at = CASE WHEN $3 IS NULL THEN NULL ELSE now() + interval '1 day' END, \
                lease_owner = CASE WHEN $2 = 'leased' THEN 'fixture' END, \
                lease_expires_at = CASE WHEN $2 = 'leased' THEN now() + interval '10 minutes' END \
         WHERE org_id = $1 AND id = $4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(state)
    .bind(gate)
    .bind(uuid::Uuid::from_bytes(ITEM.0 .0))
    .execute(&mut *tx)
    .await
    .expect("the item moves");
    tx.commit().await.expect("the move commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn gate_of(app: &PgPool) -> (String, Option<String>) {
    let mut tx = app.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let row =
        sqlx::query_as("SELECT state, blocked_on FROM job_item WHERE org_id = $1 AND id = $2")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
            .bind(uuid::Uuid::from_bytes(ITEM.0 .0))
            .fetch_one(&mut *tx)
            .await
            .expect("the item reads");
    tx.commit().await.expect("the read commits");
    row
}

/// An answer landing between the raise and the park does not strand the item.
///
/// `prepare_item` commits the raise in its own transaction and returns
/// `Blocked`; the worker parks the item afterwards, and in that window the
/// item is still `leased`. `revive_on` filters on `parked_live`, so the
/// answer's own revive matched nothing and reported `revived: 0`, the park
/// that followed had nothing left to clear it, and the seller waited out the
/// full twenty-four hours after deciding -- which is exactly the latency the
/// answer-driven revive exists to remove.
#[sqlx::test(migrations = "./migrations")]
async fn an_answer_that_lands_before_the_park_still_releases_the_item(app: PgPool) {
    let mapping = fixture(&app).await;
    item_in(&app, mapping, "leased", None).await;
    let repo = ElectionRepo::new(app.clone());
    repo.raise(ORG_A, mapping, &[supply(PricingBranch::Free)], T0)
        .await
        .expect("the question raises");
    let item = repo.open_items(ORG_A).await.expect("the queue reads")[0].id;

    let report = repo
        .answer(
            ORG_A,
            NewAnswer {
                item,
                paths: &[licence("CC-BY-SA")],
                at: T0,
                promote: None,
            },
        )
        .await
        .expect("the answer records");
    assert_eq!(
        report.revived, 0,
        "the item is still leased, so the answer's own revive is the miss this closes"
    );

    // The worker parks a beat later, on an item whose question is already
    // settled.
    set_item(&app, "parked_live", Some("election")).await;
    assert_eq!(
        repo.revive_if_answered(ORG_A, mapping, T0)
            .await
            .expect("the post-park re-check runs"),
        1,
        "nothing is open, so the park has nothing to wait for"
    );
    assert_eq!(
        gate_of(&app).await,
        ("queued".to_owned(), None),
        "the item runs again rather than waiting out the day"
    );
}

/// The re-check leaves a park that is genuinely still waiting.
#[sqlx::test(migrations = "./migrations")]
async fn a_park_with_a_question_still_open_is_left_alone(app: PgPool) {
    let mapping = fixture(&app).await;
    item_in(&app, mapping, "leased", None).await;
    let repo = ElectionRepo::new(app.clone());
    repo.raise(
        ORG_A,
        mapping,
        &[supply(PricingBranch::Free), supply(PricingBranch::Paid)],
        T0,
    )
    .await
    .expect("both questions raise");
    let item = repo.open_items(ORG_A).await.expect("the queue reads")[0].id;
    repo.answer(
        ORG_A,
        NewAnswer {
            item,
            paths: &[licence("CC-BY-SA")],
            at: T0,
            promote: None,
        },
    )
    .await
    .expect("one of the two is answered");

    set_item(&app, "parked_live", Some("election")).await;
    assert_eq!(
        repo.revive_if_answered(ORG_A, mapping, T0)
            .await
            .expect("the re-check runs"),
        0,
        "one question is still open, so the item is still waiting on the seller"
    );
    assert_eq!(
        gate_of(&app).await,
        ("parked_live".to_owned(), Some("election".to_owned())),
    );
}
