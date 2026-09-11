//! The analytics store: the bound-listing read a capture addresses the
//! marketplace through, the snapshot write, the newest-per-metric read, and
//! the retention pass.
//!
//! Retention runs on the engine role, because it crosses tenants and forced
//! row-level security would show the application role an empty table — the
//! same posture as the job-event pruner beside it.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_marketplace::RemoteListingId;
use tam_storage::{AnalyticsRepo, MappingRepo, MetricSnapshot, ProductRepo, PruneRepo};
use tam_types::{InventoryId, MappingId, Timestamp, Uuid};

mod common;
use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

const TPT_MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
const TES_MAPPING: MappingId = MappingId(Uuid([0x32; 16]));
const UNBOUND_MAPPING: MappingId = MappingId(Uuid([0x33; 16]));

const DAY: i64 = 24 * 60 * 60 * 1000;
const NOW: Timestamp = Timestamp(500 * DAY);

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

/// One organisation, one product, and three mappings on it: a bound TPT
/// listing, a bound Tes listing and an unbound TPT one, so the read under test
/// has both axes to discriminate on.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    seed_org_a(pool).await.expect("the fixture org inserts");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), Timestamp(1))
        .await
        .expect("the fixture product inserts");

    let mut tx = pool.begin().await.expect("the transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the fixture pins the tenant");
    for (mapping, inventory, marketplace, kind, url, numeric, state) in [
        (
            TPT_MAPPING,
            "tpt",
            "tpt",
            Some("tpt"),
            None,
            Some(9_001_i64),
            "bound",
        ),
        (
            TES_MAPPING,
            "tes",
            "tes",
            Some("tes"),
            Some("https://www.tes.com/teaching-resource/x-1"),
            None,
            "bound",
        ),
        // Etsy: the one inventory left that is neither of the two bound rows
        // above, since one product carries one mapping per inventory.
        (UNBOUND_MAPPING, "etsy", "etsy", None, None, None, "unbound"),
    ] {
        sqlx::query(
            "INSERT INTO mapping \
             (org_id, id, product_id, inventory, marketplace, binding_state, \
              remote_id_kind, remote_url, remote_numeric_id, first_seen_at, \
              verify_state, verify_stale_since, normaliser_version, \
              policy_title, policy_description, policy_price, policy_taxonomy, \
              policy_grades, policy_files, price_rule_kind, price_explicit_kind, \
              publish_mode, lifecycle_state, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, \
                     CASE WHEN $6 = 'bound' THEN now() END, 'stale', \
                     CASE WHEN $6 = 'bound' THEN now() END, 0, \
                     'managed', 'managed', 'managed', 'managed', 'managed', 'managed', \
                     'explicit', 'free', 'publish', 'absent', now(), now())",
        )
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .bind(uuid::Uuid::from_bytes(mapping.0 .0))
        .bind(uuid::Uuid::from_bytes(PRODUCT_1.0 .0))
        .bind(inventory)
        .bind(marketplace)
        .bind(state)
        .bind(kind)
        .bind(url)
        .bind(numeric)
        .execute(&mut *tx)
        .await
        .expect("the fixture mapping inserts");
    }
    tx.commit().await.expect("the fixture commits");
}

fn snapshot(metric: &str, at: Timestamp, value: f64) -> MetricSnapshot {
    MetricSnapshot {
        mapping: TPT_MAPPING,
        metric: metric.to_owned(),
        observed_at: at,
        total_value: value,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_bound_read_answers_the_identifier_the_state_label_hides(pool: PgPool) {
    provision(&pool).await;
    let bound = MappingRepo::new(pool.clone())
        .bound_listings(ORG_A, InventoryId::Tpt)
        .await
        .expect("the bound read runs");
    assert_eq!(
        bound.len(),
        1,
        "one bound TPT mapping; the Tes one and the unbound one are not it"
    );
    assert_eq!(
        (bound[0].mapping, bound[0].remote.clone()),
        (TPT_MAPPING, RemoteListingId::Tpt { product_id: 9_001 }),
        "the read carries the remote listing id, which is what a marketplace read needs"
    );
    assert_eq!(
        MappingRepo::new(pool)
            .bound_listings(ORG_A, InventoryId::Etsy)
            .await
            .expect("the bound read runs"),
        Vec::new(),
        "an inventory whose only mapping is unbound answers nothing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_read_answers_the_newest_snapshot_of_each_metric(pool: PgPool) {
    provision(&pool).await;
    let analytics = AnalyticsRepo::new(pool.clone());
    let written = analytics
        .record(
            ORG_A,
            &[
                snapshot("sales_count", Timestamp(1_000), 5.0),
                snapshot("sales_count", Timestamp(2_000), 9.0),
                snapshot("resource_views", Timestamp(1_000), 400.0),
            ],
        )
        .await
        .expect("the pass records");
    assert_eq!(written, 3, "three distinct keys, three rows");

    let latest = analytics.latest(ORG_A).await.expect("the summary reads");
    assert_eq!(
        latest.len(),
        2,
        "two metrics, so two rows, whatever the number of captures behind them"
    );
    let sales = latest
        .iter()
        .find(|row| row.metric == "sales_count")
        .expect("the read answers the metric it was given");
    assert_eq!(
        (sales.total_value, sales.observed_at),
        (9.0, Timestamp(2_000)),
        "the newer capture wins and states its own instant"
    );
    assert_eq!(
        sales.inventory,
        InventoryId::Tpt,
        "the inventory comes from the mapping the snapshot names"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn writing_one_pass_twice_changes_nothing(pool: PgPool) {
    provision(&pool).await;
    let analytics = AnalyticsRepo::new(pool);
    let pass = [snapshot("sales_count", Timestamp(1_000), 5.0)];
    assert_eq!(
        analytics
            .record(ORG_A, &pass)
            .await
            .expect("the first write"),
        1,
        "the first write lands"
    );
    assert_eq!(
        analytics
            .record(ORG_A, &pass)
            .await
            .expect("the re-run runs"),
        0,
        "a re-run of the same pass writes nothing rather than raising"
    );
    assert_eq!(
        analytics
            .latest(ORG_A)
            .await
            .expect("the summary reads")
            .len(),
        1,
        "and leaves one row standing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_snapshot_of_no_metrics_touches_the_database_not_at_all(pool: PgPool) {
    provision(&pool).await;
    let analytics = AnalyticsRepo::new(pool);
    assert_eq!(
        analytics
            .record(ORG_A, &[])
            .await
            .expect("an empty pass runs"),
        0,
        "nothing to write"
    );
    assert!(
        analytics
            .latest(ORG_A)
            .await
            .expect("the summary reads")
            .is_empty(),
        "a tenant whose capture has not run answers an empty list"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn retention_erases_what_aged_out_and_leaves_the_rest(app: PgPool) {
    provision(&app).await;
    let analytics = AnalyticsRepo::new(app.clone());
    let stale = Timestamp(NOW.0 - 401 * DAY);
    let fresh = Timestamp(NOW.0 - 399 * DAY);
    analytics
        .record(
            ORG_A,
            &[
                snapshot("sales_count", stale, 1.0),
                snapshot("sales_count", fresh, 2.0),
            ],
        )
        .await
        .expect("both snapshots record");

    let engine = engine_pool(&app).await;
    let pruner = PruneRepo::new(engine);
    let cutoff = Timestamp(NOW.0 - 400 * DAY);
    assert_eq!(
        pruner
            .prune_snapshots(cutoff, 10)
            .await
            .expect("the retention pass runs"),
        1,
        "only the snapshot older than the window goes"
    );
    let survivors = analytics.latest(ORG_A).await.expect("the summary reads");
    assert_eq!(
        survivors.len(),
        1,
        "the figure inside the window is still readable"
    );
    assert_eq!(
        survivors[0].observed_at, fresh,
        "and it is the one that was not erased"
    );
    assert_eq!(
        pruner
            .prune_snapshots(cutoff, 10)
            .await
            .expect("the second pass runs"),
        0,
        "a second pass at the same cutoff is a no-op"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_application_role_cannot_erase_a_snapshot_by_running_the_pass(app: PgPool) {
    provision(&app).await;
    AnalyticsRepo::new(app.clone())
        .record(ORG_A, &[snapshot("sales_count", Timestamp(1_000), 1.0)])
        .await
        .expect("the snapshot records");
    // The same pass on the pinning role, which is the role a capture holds:
    // forced row-level security hides every tenant's rows from an unpinned
    // statement, so the sweep sees nothing rather than erasing one tenant's
    // history from the wrong process.
    assert_eq!(
        PruneRepo::new(app.clone())
            .prune_snapshots(Timestamp(2_000), 10)
            .await
            .expect("the pass runs on the application role too"),
        0,
        "retention is the engine role's pass and no other's"
    );
    assert_eq!(
        AnalyticsRepo::new(app)
            .latest(ORG_A)
            .await
            .expect("the summary reads")
            .len(),
        1,
        "the row is still there"
    );
}
