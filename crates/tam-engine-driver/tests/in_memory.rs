//! The conformance suite's in-memory instantiation.
//!
//! This is the half that runs inside `just check`: no database, no runtime, no
//! `#[sqlx::test]`. `tam-engine`'s own suite runs the same bodies against
//! Postgres, and a divergence between the two is the split having leaked a
//! storage assumption into the interpreter.

use tam_domain::{ItemOperation, JobItemId};
use tam_engine_driver::conformance;
use tam_engine_driver::memory::{InMemoryLedger, LaggingReconcile, ScriptedReconcile, Seeded};
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
    fixture_with_ceiling(64)
}

/// The same fixture with the connection's rate window stated, for the one body
/// that is about exhausting it.
fn fixture_with_ceiling(rate_ceiling: i32) -> (InMemoryLedger, LeasedItem) {
    let seeded = Seeded {
        org: ORG,
        item: ITEM,
        mapping: MAPPING,
        inventory: InventoryId::Tes,
        connection: CONNECTION,
        lease_epoch: LEASE_EPOCH,
        rate_ceiling,
    };
    let lease = LeasedItem {
        org: ORG,
        item: ITEM,
        job: JOB,
        mapping: MAPPING,
        inventory: InventoryId::Tes,
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

/// The one body that needs the ledger told something Postgres would learn
/// from a concurrent transaction, so it is instantiated here rather than in
/// the conformance macro above.
#[test]
fn a_create_bound_elsewhere_settles_skipped() {
    let (ledger, lease) = fixture();
    ledger.bind_elsewhere(lease.mapping);
    futures::executor::block_on(conformance::a_create_bound_elsewhere_settles_skipped(
        &ledger, &lease,
    ));
}

/// The two answers a reconcile can end on, driven end to end.
///
/// Instantiated here rather than through the conformance macro because each
/// needs the reconcile source told what the seller's catalogue holds, which
/// is the one thing a Postgres ledger cannot be told.
#[test]
fn a_reconcile_that_finds_the_listing_settles_it() {
    let (ledger, lease) = fixture();
    ledger.strand(lease.mapping, conformance::STRANDED);
    futures::executor::block_on(conformance::a_reconcile_that_finds_the_listing_settles_it(
        &ledger,
        &lease,
        // The listing the scripted adapter will observe on the verifying
        // read-back, so the run is proved end to end rather than stopping at
        // a mismatch that is only the fixture disagreeing with itself.
        // Deliberately not `LANDED_URL`: that is what the scripted read-back
        // invents for a marker locator, so a body asserting against it could
        // not tell a verifying read that addressed the found listing from one
        // that re-ran the search.
        tam_marketplace::RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/7777".to_owned(),
        },
    ));
}

#[test]
fn a_complete_enumeration_without_the_listing_leaves_it_stranded() {
    let (ledger, lease) = fixture();
    ledger.strand(lease.mapping, conformance::STRANDED);
    futures::executor::block_on(
        conformance::a_reconcile_that_cannot_identify_leaves_it_stranded(
            &ledger,
            &lease,
            ScriptedReconcile::complete_and_absent(),
        ),
    );
}

/// The other half of the same outcome, and the reason both are tested: a walk
/// that could not be completed says nothing about the listing either, and the
/// two must not be allowed to diverge into different item outcomes.
#[test]
fn a_read_that_could_not_be_performed_leaves_it_stranded_too() {
    let (ledger, lease) = fixture();
    ledger.strand(lease.mapping, conformance::STRANDED);
    futures::executor::block_on(
        conformance::a_reconcile_that_cannot_identify_leaves_it_stranded(
            &ledger,
            &lease,
            ScriptedReconcile::could_not_read("the session lapsed mid-walk"),
        ),
    );
}

/// The lag between a create being accepted and being published, which is the
/// reason the reconcile is a poll rather than one walk. Without it the seller
/// loses their whole inventory to a halt over an indexing delay.
#[test]
fn a_catalogue_that_has_not_published_yet_is_walked_again_rather_than_believed() {
    let (ledger, lease) = fixture();
    ledger.strand(lease.mapping, conformance::STRANDED);
    let found = tam_marketplace::RemoteListingId::Tes {
        url: "https://www.tes.com/api/v2/resources/7778".to_owned(),
    };
    let catalogue = LaggingReconcile::publishing_after(1, found.clone());
    futures::executor::block_on(
        conformance::a_reconcile_that_sees_absence_before_the_listing_still_settles_it(
            &ledger, &lease, &catalogue, found,
        ),
    );
    assert_eq!(
        catalogue.walked(),
        2,
        "one walk that answered absence and one that answered the listing: the poll stops \
         as soon as it has an answer rather than spending the seller's allowance"
    );
}

/// The other half of the hand-back: a run that stopped with a write in flight
/// gives the lease back and the server parks the item behind its own fence.
#[test]
fn an_abandoned_write_parks_the_item_rather_than_holding_its_lease() {
    let (ledger, lease) = fixture();
    futures::executor::block_on(
        conformance::an_abandoned_write_parks_the_item_rather_than_holding_its_lease(
            &ledger, &lease,
        ),
    );
}

/// The two entitlement arms: a run stopped between the scrape and the write,
/// and a window exhausted before the scrape.
#[test]
fn a_run_stopped_before_the_write_abandons_rather_than_settling_the_item() {
    let (ledger, lease) = fixture();
    futures::executor::block_on(
        conformance::a_run_stopped_before_the_write_abandons_rather_than_settling_the_item(
            &ledger, &lease,
        ),
    );
}

#[test]
fn an_exhausted_window_stops_the_form_scrape_before_any_attempt() {
    // Nothing at all, so the very first grant the run asks for is refused,
    // and the first one it asks for is the scrape's.
    let (ledger, lease) = fixture_with_ceiling(0);
    futures::executor::block_on(
        conformance::an_exhausted_window_stops_the_form_scrape_before_any_attempt(&ledger, &lease),
    );
}
