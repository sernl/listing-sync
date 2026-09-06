//! The read leg's own record: that a redrained request knows which resources
//! it already canonicalised, that a migrate's two jobs get two keys, that the
//! cross-tenant role cannot see a request at all, and that one tenant's
//! requests are invisible to another.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_marketplace::{ListingState, RemoteListingId};
use tam_storage::{
    job_request_key, Canonicalised, Disposition, Enqueued, NewSyncRequest, SyncIntent,
    SyncRequestRepo, CREATE_LEG, REMOVE_LEG,
};
use tam_types::{InventoryId, MappingId, OrgId, ProductId, Timestamp, Uuid};

use common::{seed_org_a, ORG_A};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const REQUEST: Uuid = Uuid([0x71; 16]);
const T0: Timestamp = Timestamp(1_000);

fn request_for(id: Uuid, locators: &[&str]) -> NewSyncRequest {
    NewSyncRequest {
        id,
        source: InventoryId::TesGb,
        target: InventoryId::TesNz,
        disposition: Disposition::Sync,
        intent: SyncIntent::Draft,
        requested_at: T0,
        locators: locators.iter().map(|l| (*l).to_owned()).collect(),
    }
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

/// The resumability property, and the reason the breadcrumb exists.
///
/// `import_one` mints a fresh product id on every pass and commits four
/// times internally, and `mapping_one_per_inventory` is keyed on the product,
/// so a second pass over a resource the first pass already canonicalised
/// inserts a duplicate product and a duplicate mapping that no unique index
/// refuses. The drain skips what the row already names.
#[sqlx::test(migrations = "./migrations")]
async fn a_canonicalised_resource_names_what_it_produced_and_is_skipped_on_a_redrain(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    repo.create(ORG_A, &request_for(REQUEST, &["101", "202"]))
        .await
        .expect("the request writes");

    let before = repo
        .get(ORG_A, REQUEST)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert_eq!(before.resources.len(), 2);
    assert!(
        before.resources.iter().all(|row| !row.is_canonicalised()),
        "nothing is done before the drain runs"
    );

    repo.record_canonicalised(
        ORG_A,
        &Canonicalised {
            request: REQUEST,
            ordinal: 0,
            product: ProductId(Uuid([0x01; 16])),
            mapping: MappingId(Uuid([0x31; 16])),
            source: RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/fixture-101".to_owned(),
            },
            source_state: Some(ListingState::Live),
        },
    )
    .await
    .expect("the breadcrumb writes");

    let after = repo
        .get(ORG_A, REQUEST)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert!(
        after.resources[0].is_canonicalised(),
        "the first resource carries both ids, so a redrain skips it"
    );
    assert_eq!(
        after.resources[0].product,
        Some(ProductId(Uuid([0x01; 16]))),
        "and names the product it produced rather than only that it finished"
    );
    assert_eq!(
        (
            after.resources[0].source.clone(),
            after.resources[0].source_state
        ),
        (
            Some(RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/fixture-101".to_owned(),
            }),
            Some(ListingState::Live)
        ),
        "and what the read observed about the source, which is what a redrain needs to \
         rebuild this resource's removal item rather than drop it"
    );
    assert!(
        !after.resources[1].is_canonicalised(),
        "the second is still the drain's to do"
    );
}

/// A migrate is two jobs and `job_request_idempotent` admits one job per key
/// per organisation. Passing the request's own id twice would make the second
/// creation a replay of the first: the first job comes back with
/// `replay: true`, the removal's items are dropped with no error anywhere,
/// and the migrate silently degrades into a plain sync.
#[test]
fn a_migrate_derives_one_request_key_per_leg() {
    let create = job_request_key(REQUEST, CREATE_LEG);
    let remove = job_request_key(REQUEST, REMOVE_LEG);
    assert_ne!(
        create, remove,
        "the two legs must not collide on job_request_idempotent"
    );
    assert_ne!(create, REQUEST, "and neither is the request's own id");
    assert_eq!(
        create,
        job_request_key(REQUEST, CREATE_LEG),
        "derivation is a function of the request, so a redrain replays rather than duplicates"
    );
}

/// The read half of the role decision, and the reason the grant behind it
/// exists.
///
/// The engine may read a request in order to settle a run. `settle_if_complete`
/// resolves which run a settled job belongs to, so that a migration's two legs
/// produce one seller notification rather than two, and it is reached as
/// `tam_engine` from the worker and from `expire_and_steal` — which settles an
/// attempt-exhausted item from the maintenance loop with no job context at all.
/// The role is BYPASSRLS, which bypasses the policy and not the table
/// privilege, so without migration 0063's `GRANT SELECT ON sync_request` that
/// settle fails `permission denied` on every reaper-settled item.
///
/// Asserted positively rather than left implicit, because the grant is now
/// load-bearing in both directions: revoking it would break the settle path,
/// and nothing else here would say so.
#[sqlx::test(migrations = "./migrations")]
async fn the_cross_tenant_role_may_read_a_sync_request_to_settle_a_run(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    SyncRequestRepo::new(pool.clone())
        .create(ORG_A, &request_for(REQUEST, &["101"]))
        .await
        .expect("the request writes");
    let engine = engine_pool(&pool).await;

    let read = sqlx::query(
        "SELECT org_id, id, disposition, target, create_job_id, remove_job_id \
           FROM sync_request",
    )
    .fetch_all(&engine)
    .await;
    assert!(
        read.is_ok(),
        "the settle resolves a job's run through these six columns: {read:?}"
    );
}

/// The write half, which is the whole of what the role decision ever refused.
///
/// The drain runs as `tam_app` precisely so the cross-tenant role never enqueues
/// a request, and reading one grants nothing toward that: the engine may never
/// insert, update or delete a row here, and all three verbs are asserted rather
/// than the one an earlier version happened to try. A grant widened later to
/// make something pass fails this.
#[sqlx::test(migrations = "./migrations")]
async fn the_cross_tenant_role_can_never_write_a_sync_request(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    SyncRequestRepo::new(pool.clone())
        .create(ORG_A, &request_for(REQUEST, &["101"]))
        .await
        .expect("the request writes");
    let engine = engine_pool(&pool).await;
    let org = uuid::Uuid::from_bytes(ORG_A.0 .0);

    let inserted = sqlx::query(
        "INSERT INTO sync_request \
         (org_id, id, source, target, disposition, intent, state, requested_at) \
         VALUES ($1, $2, 'tes_gb', 'tes_nz', 'sync', 'draft', 'pending', now())",
    )
    .bind(org)
    .bind(uuid::Uuid::from_bytes([0x72; 16]))
    .execute(&engine)
    .await;
    assert!(
        inserted.is_err(),
        "the engine cannot enqueue a request: {inserted:?}"
    );

    let updated = sqlx::query("UPDATE sync_request SET state = 'failed' WHERE org_id = $1")
        .bind(org)
        .execute(&engine)
        .await;
    assert!(
        updated.is_err(),
        "nor settle one it did not drain: {updated:?}"
    );

    let deleted = sqlx::query("DELETE FROM sync_request WHERE org_id = $1")
        .bind(org)
        .execute(&engine)
        .await;
    assert!(
        deleted.is_err(),
        "nor erase one; the engine stalls and settles, it never erases: {deleted:?}"
    );

    // The read is column scoped, so the refusal has a second half: the engine
    // reads the six columns `run_of` names and no others. `failure_detail` is
    // the one that matters most -- it is free text a marketplace's own error
    // message lands in -- and a table-wide grant would have handed every
    // organisation's to a role that crosses tenants by construction.
    for withheld in [
        "source",
        "intent",
        "state",
        "requested_at",
        "settled_at",
        "failure_detail",
    ] {
        let read = sqlx::query(&format!("SELECT {withheld} FROM sync_request"))
            .fetch_all(&engine)
            .await;
        assert!(
            read.is_err(),
            "{withheld} is outside the six columns the settle reads: {read:?}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn one_tenants_sync_requests_are_invisible_to_another(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await
        .expect("org b seeds");
    let repo = SyncRequestRepo::new(pool);
    repo.create(ORG_A, &request_for(REQUEST, &["101"]))
        .await
        .expect("the request writes");

    assert!(
        repo.get(ORG_B, REQUEST)
            .await
            .expect("the read runs")
            .is_none(),
        "tenant B pinned to itself sees nothing of tenant A's request"
    );
    assert!(
        repo.pending(ORG_B, 10)
            .await
            .expect("the scan runs")
            .is_empty(),
        "and its drain has no work to do"
    );
    assert_eq!(
        repo.pending(ORG_A, 10).await.expect("the scan runs"),
        vec![REQUEST],
        "while tenant A's own drain finds exactly its own"
    );
}

/// A settled request names the jobs it produced, because a seller polling it
/// must never be told the read leg finished without being told where the
/// write leg lives.
#[sqlx::test(migrations = "./migrations")]
async fn an_enqueued_request_names_the_job_it_produced(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    repo.create(ORG_A, &request_for(REQUEST, &["101"]))
        .await
        .expect("the request writes");
    let job = Uuid([0x91; 16]);
    repo.record_enqueued(
        ORG_A,
        &Enqueued {
            request: REQUEST,
            create_job: Some(job),
            remove_job: None,
            at: T0,
        },
    )
    .await
    .expect("the jobs record");

    let record = repo
        .get(ORG_A, REQUEST)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert_eq!(record.state, "enqueued");
    assert_eq!(record.create_job, Some(job));
    assert_eq!(
        record.remove_job, None,
        "a sync leaves the source alone, so it has no removal leg"
    );
    assert!(
        repo.pending(ORG_A, 10)
            .await
            .expect("the scan runs")
            .is_empty(),
        "and a settled request is no longer the drain's work"
    );
}

/// A second submit under one key is the retry the endpoint absorbs, not a
/// fault.
///
/// `create_sync_request` read the row first and inserted only on a miss, so
/// two submits racing under one `Idempotency-Key` both saw no row: one INSERT
/// won and the other violated `sync_request`'s primary key, surfacing as a
/// 500 for exactly the double-click the key exists to make harmless. The
/// insert now decides it, so there is nothing left to race.
#[sqlx::test(migrations = "./migrations")]
async fn a_second_submit_under_one_key_replays_rather_than_faulting(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    assert!(
        repo.create(ORG_A, &request_for(REQUEST, &["101", "202"]))
            .await
            .expect("the request writes"),
        "the first submit is the one that wrote it"
    );
    assert!(
        !repo
            .create(ORG_A, &request_for(REQUEST, &["101", "202"]))
            .await
            .expect("the replay is not a fault"),
        "the second submit reports that it wrote nothing"
    );

    let record = repo
        .get(ORG_A, REQUEST)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert_eq!(
        record.resources.len(),
        2,
        "the replay writes no second copy of the seller's locators"
    );
}
