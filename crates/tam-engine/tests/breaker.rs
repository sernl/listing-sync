//! The fleet breaker's predicate against the live ledger. What it has to
//! separate is a tenant-local failure from a marketplace-wide one, so the
//! question these ask is which settled outcomes count as evidence that the
//! marketplace itself has stopped working — and `blocked` is the answer that
//! is easy to omit, because a condition stopping every tenant *before* the
//! write settles there rather than in `failed` or `ambiguous`.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{Binding, FieldPolicies, FieldPolicy, ItemOperation, JobItemId, PublishMode};
use tam_engine::breaker::{run_breaker, BREAKER_MIN_SAMPLE};
use tam_marketplace::{IdempotencyKey, RemoteLifecycle};
use tam_storage::{HaltRepo, JobRepo, MappingRepo, NewJob, NewJobItem, ProductRepo};
use tam_types::{
    Actor, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, JobId,
    ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId,
    ScanOutcome, Stamp, SystemComponent, Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));

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

/// One tenant's window: an item per named outcome, all settled at `T0` on
/// TesGb. The breaker groups by inventory and never by tenant, so one tenant
/// standing in for the fleet is the same arithmetic the cross-tenant case
/// performs — what these tests vary is the outcome mix, not who owns it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed_window(app: &PgPool, engine: &PgPool, outcomes: &[&str], failure_code: Option<&str>) {
    let product = ProductId(Uuid([0x01; 16]));
    let mapping = MappingId(Uuid([0x02; 16]));
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(app)
        .await
        .expect("the org inserts");
    ProductRepo::new(app.clone())
        .insert(
            ORG,
            &tam_domain::CanonicalProduct {
                id: product,
                org: ORG,
                title: Title("Fixture".to_owned()),
                body: ListingCopy {
                    body: "Fixture".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: PayloadSet::new(
                    ProductFile {
                        id: FileId(Uuid([0x03; 16])),
                        role: FileRole::Payload,
                        kind: FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: ContentHash([0x04; 32]),
                            byte_len: 4,
                            scan: ScanOutcome::Pending,
                        },
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
        .expect("the mapping inserts");

    let plan: Vec<(u8, &str)> = (0x40_u8..).zip(outcomes.iter().copied()).collect();
    let items: Vec<NewJobItem> = plan
        .iter()
        .map(|(byte, _)| NewJobItem {
            item: JobItemId(Uuid([*byte; 16])),
            mapping,
            idempotency_key: IdempotencyKey(Uuid([byte.wrapping_add(0x40); 16])),
            operation: ItemOperation::Create,
            requires_bound_on: None,
        })
        .collect();
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
            &items,
        )
        .await
        .expect("the job enqueues");

    for (byte, outcome) in plan {
        sqlx::query(
            "UPDATE job_item SET state = 'settled', outcome = $1, failure_code = $4, \
                 settled_at = to_timestamp($2::bigint / 1000.0) \
             WHERE id = $3",
        )
        .bind(outcome)
        .bind(T0.0)
        .bind(uuid::Uuid::from_bytes([byte; 16]))
        .bind(failure_code)
        .execute(engine)
        .await
        .expect("the item settles");
    }
}

/// A fleet-wide condition that stops every tenant before the write settles
/// `blocked`, never `failed` or `ambiguous`. Counting those rows in the
/// denominator alone inverts the breaker: the more tenants the outage stops,
/// the lower the ratio it computes, and the marketplace-wide event it exists
/// to catch is the one event it cannot see.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_window_of_blocked_settlements_trips_the_breaker(app: PgPool) {
    let engine = engine_pool(&app).await;
    seed_window(&app, &engine, &["blocked"; 6], None).await;
    let report = run_breaker(
        &JobRepo::new(engine.clone()),
        &HaltRepo::new(engine.clone()),
        T0,
    )
    .await
    .expect("the breaker runs");
    assert_eq!(
        report.tripped,
        vec!["TesGb".to_owned()],
        "six of six settlements blocked is a marketplace that has stopped working"
    );
    let halt: (String, String) =
        sqlx::query_as("SELECT raised_by, reason FROM inventory_halt LIMIT 1")
            .fetch_one(&engine)
            .await
            .expect("the halt row reads");
    assert_eq!(
        halt.0, "breaker",
        "the fleet halt is durable, not a log line"
    );
    assert!(
        halt.1.contains("6 of 6"),
        "the halt states the window it tripped on: {}",
        halt.1
    );
}

/// The other direction, so the fix is a predicate rather than a switch: one
/// blocked item among healthy ones is a tenant's bad minute and must not halt
/// the fleet. Six settlements also clears `BREAKER_MIN_SAMPLE`, so what holds
/// the breaker quiet here is the ratio and not the sample floor.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_blocked_settlement_among_healthy_ones_does_not(app: PgPool) {
    let engine = engine_pool(&app).await;
    let outcomes = [
        "succeeded",
        "succeeded",
        "succeeded",
        "succeeded",
        "succeeded",
        "blocked",
    ];
    assert!(
        i64::try_from(outcomes.len()).unwrap_or(i64::MAX) > BREAKER_MIN_SAMPLE,
        "the window has to clear the sample floor or this proves nothing"
    );
    seed_window(&app, &engine, &outcomes, None).await;
    let report = run_breaker(
        &JobRepo::new(engine.clone()),
        &HaltRepo::new(engine.clone()),
        T0,
    )
    .await
    .expect("the breaker runs");
    assert_eq!(
        report.tripped,
        Vec::<String>::new(),
        "one in six is a tenant's bad minute, not a fleet event"
    );
    let halted: i64 = sqlx::query_scalar("SELECT count(*) FROM inventory_halt")
        .fetch_one(&engine)
        .await
        .expect("the halt table reads");
    assert_eq!(halted, 0, "nothing halted");
}

/// The fleet remedy an egress block actually gets. A Cloudflare rule keyed on
/// our address answers every tenant identically, and none of them can clear
/// it — so each settles one item `blocked` on `ChallengePresented` and stops.
/// Gating their connections would ask every seller to re-link a credential
/// that works; halting the inventory is the answer that matches the cause, and
/// this is the arithmetic that reaches it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_window_of_challenge_blocked_settlements_trips_the_breaker(app: PgPool) {
    let engine = engine_pool(&app).await;
    seed_window(&app, &engine, &["blocked"; 6], Some("ChallengePresented")).await;
    let report = run_breaker(
        &JobRepo::new(engine.clone()),
        &HaltRepo::new(engine.clone()),
        T0,
    )
    .await
    .expect("the breaker runs");
    assert_eq!(
        report.tripped,
        vec!["TesGb".to_owned()],
        "the breaker reads the outcome, so a challenge-coded block reaches it exactly as \
         any other adverse settlement does"
    );
    let coded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_item WHERE outcome = 'blocked' \
           AND failure_code = 'ChallengePresented'",
    )
    .fetch_one(&engine)
    .await
    .expect("the item rows read");
    assert_eq!(
        coded, 6,
        "and the rows carry the challenge code, so narrowing the breaker's filter by \
         failure_code later would have to break this test to do it"
    );
}
