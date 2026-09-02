//! The conformance suite's in-memory instantiation.
//!
//! This is the half that runs inside `just check`: no database, no runtime, no
//! `#[sqlx::test]`. `tam-engine`'s own suite runs the same bodies against
//! Postgres, and a divergence between the two is the split having leaked a
//! storage assumption into the interpreter.

use tam_domain::{ItemOperation, JobItemId};
use tam_engine_driver::conformance;
use tam_engine_driver::memory::{InMemoryLedger, Seeded};
use tam_engine_driver::vocabulary::LeasedItem;
use tam_marketplace::IdempotencyKey;
use tam_types::{ConnectionId, InventoryId, JobId, MappingId, OrgId, Uuid};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const ITEM: JobItemId = JobItemId(Uuid([0x11; 16]));
const JOB: JobId = JobId(Uuid([0x22; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x33; 16]));
const CONNECTION: ConnectionId = ConnectionId(Uuid([0x44; 16]));
const LEASE_EPOCH: i64 = 1;

fn fixture() -> (InMemoryLedger, LeasedItem) {
    let seeded = Seeded {
        org: ORG,
        item: ITEM,
        mapping: MAPPING,
        inventory: InventoryId::TesGb,
        connection: CONNECTION,
        lease_epoch: LEASE_EPOCH,
        // Generous on purpose: a body that means to exhaust the window is the
        // one that should say so, not every other body by accident.
        rate_ceiling: 64,
    };
    let lease = LeasedItem {
        org: ORG,
        item: ITEM,
        job: JOB,
        mapping: MAPPING,
        inventory: InventoryId::TesGb,
        idempotency_key: IdempotencyKey(Uuid([0x55; 16])),
        operation: ItemOperation::Create,
        lease_epoch: LEASE_EPOCH,
        attempt_count: 0,
        requires_bound_on: None,
    };
    (InMemoryLedger::seeded(&seeded), lease)
}

/// Every body, run against the in-memory ledger. `block_on` rather than a
/// `#[tokio::test]`, because `just purity` bans the runtime from this crate
/// and the interpreter needs none: it awaits only its own ports.
macro_rules! conformance_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            let (ledger, lease) = fixture();
            futures::executor::block_on(conformance::$name(&ledger, &lease));
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
