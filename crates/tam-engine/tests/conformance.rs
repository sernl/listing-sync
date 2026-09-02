//! The conformance suite's Postgres instantiation.
//!
//! The bodies live in `tam-engine-driver::conformance` and are the same ones
//! its own `in_memory` suite runs. This half proves the interpreter behaves
//! identically against the real ledger; a divergence between the two runs is
//! the split having leaked a storage assumption, which is exactly what the
//! phase set out to be able to detect.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_engine::ledger::{to_wire_item, PgLedger};
use tam_engine_driver::conformance;
use tam_storage::LeaseRepo;

mod fixture;

/// One seeded ledger and the item it leased, ready for a shared body.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn leased(app: &PgPool) -> (PgLedger, tam_engine_driver::vocabulary::LeasedItem) {
    let engine = fixture::engine_pool(app).await;
    fixture::seed(app, &engine).await;
    let leases = LeaseRepo::new(engine.clone());
    let item = leases
        .acquire("conformance", 600)
        .await
        .expect("the scan runs")
        .expect("the item leases");
    let lease = to_wire_item(&item);
    (PgLedger::new(engine, item.job), lease)
}

macro_rules! conformance_test {
    ($name:ident) => {
        #[sqlx::test(migrations = "../tam-storage/migrations")]
        async fn $name(app: PgPool) {
            let (ledger, lease) = leased(&app).await;
            conformance::$name(&ledger, &lease).await;
        }
    };
}

conformance_test!(the_happy_path_settles_succeeded);
conformance_test!(an_ambiguous_submit_halts_the_inventory);
conformance_test!(a_read_back_condition_abandons_rather_than_crashing);
conformance_test!(an_expired_session_parks_and_gates);
conformance_test!(a_cancellation_after_the_read_back_still_settles);
conformance_test!(a_cancellation_after_a_lapsed_session_still_parks);
conformance_test!(a_preflight_challenge_abandons_and_advances_the_streak);
