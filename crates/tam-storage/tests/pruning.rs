//! The retention pruner: the oldest events go, the watermark follows them
//! exactly, and running the same cutoff twice changes nothing. The pass runs
//! on the engine role, because it crosses tenants and forced row-level
//! security would show the application role an empty ledger.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_storage::{append_event, EventScope, JobRepo, NewJob, PruneRepo};
use tam_types::{Actor, InventoryId, JobEventPayload, JobId, SystemComponent, Timestamp, Uuid};

mod common;
use common::{seed_org_a, ORG_A};

const JOB: JobId = JobId(Uuid([0x41; 16]));

/// The four event times the tests prune between. The enqueue writes the first
/// one; the loop below writes the rest.
const EVENT_TIMES: [i64; 4] = [1_000, 2_000, 3_000, 4_000];

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name reads");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects")
}

/// One organisation, one job, and an event at each of `EVENT_TIMES`, so
/// `org_seq` and the event time rise together and a cutoff names a prefix.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    seed_org_a(pool).await.expect("the fixture org inserts");
    JobRepo::new(pool.clone())
        .enqueue(
            ORG_A,
            &NewJob {
                job: JOB,
                inventory: InventoryId::TesNz,
                at: Timestamp(EVENT_TIMES[0]),
                actor: Actor::System(SystemComponent::Engine),
            },
            &[],
        )
        .await
        .expect("the job enqueues with its JobQueued event");
    for at in EVENT_TIMES.into_iter().skip(1) {
        let mut tx = pool.begin().await.expect("the transaction begins");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
            .execute(&mut *tx)
            .await
            .expect("the tenant pin applies");
        append_event(
            &mut tx,
            &EventScope {
                org: ORG_A,
                job: JOB,
                item: None,
            },
            &JobEventPayload::JobQueued { items: 0 },
            Timestamp(at),
            Actor::System(SystemComponent::Engine),
        )
        .await
        .expect("the event appends");
        tx.commit().await.expect("the event commits");
    }
}

/// Read on the engine pool so the assertions see the ledger whole rather than
/// through the tenant pin the writes needed.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn surviving_seqs(engine: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT org_seq FROM job_event ORDER BY org_seq")
        .fetch_all(engine)
        .await
        .expect("the surviving events read")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn watermark(engine: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT prune_watermark FROM org_event_counter WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .fetch_one(engine)
        .await
        .expect("the watermark reads")
}

#[sqlx::test(migrations = "./migrations")]
async fn a_pass_erases_the_prefix_and_advances_the_watermark_onto_it(app: PgPool) {
    provision(&app).await;
    let engine = engine_pool(&app).await;
    let pruner = PruneRepo::new(engine.clone());

    let report = pruner
        .prune_pass(Timestamp(EVENT_TIMES[2]), 10)
        .await
        .expect("the first pass runs");
    assert_eq!(report.deleted, 2, "the two events before the cutoff go");
    assert_eq!(
        report.watermark_advances, 1,
        "one organisation lost rows, so one watermark moves"
    );
    assert_eq!(
        surviving_seqs(&engine).await,
        vec![3, 4],
        "the events at and after the cutoff survive"
    );
    assert_eq!(
        watermark(&engine).await,
        2,
        "the watermark lands on the greatest pruned sequence, not past it"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_pass_at_the_same_cutoff_erases_nothing(app: PgPool) {
    provision(&app).await;
    let engine = engine_pool(&app).await;
    let pruner = PruneRepo::new(engine.clone());
    pruner
        .prune_pass(Timestamp(EVENT_TIMES[2]), 10)
        .await
        .expect("the first pass runs");

    let again = pruner
        .prune_pass(Timestamp(EVENT_TIMES[2]), 10)
        .await
        .expect("the second pass runs");
    assert_eq!(again.deleted, 0, "there is nothing left before the cutoff");
    assert_eq!(
        again.watermark_advances, 0,
        "a watermark that cannot move must not be reported as moved"
    );
    assert_eq!(
        surviving_seqs(&engine).await,
        vec![3, 4],
        "the second pass leaves the survivors alone"
    );
    assert_eq!(
        watermark(&engine).await,
        2,
        "the watermark is where the first pass left it"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_batch_bounds_one_pass(app: PgPool) {
    provision(&app).await;
    let engine = engine_pool(&app).await;

    let report = PruneRepo::new(engine.clone())
        .prune_pass(Timestamp(EVENT_TIMES[3] + 1), 1)
        .await
        .expect("the bounded pass runs");
    assert_eq!(
        report.deleted, 1,
        "a batch of one erases one event even though every event is prunable"
    );
    assert_eq!(
        surviving_seqs(&engine).await,
        vec![2, 3, 4],
        "the oldest event is the one that goes"
    );
    assert_eq!(
        watermark(&engine).await,
        1,
        "the watermark advances only as far as the pass actually erased"
    );
}
