//! The read leg's own record: that a redrained request knows which resources
//! it already canonicalised, that a migrate's two jobs get two keys, that the
//! cross-tenant role cannot see a request at all, and that one tenant's
//! requests are invisible to another.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_marketplace::{ListingState, RemoteListingId};
use tam_storage::{
    job_request_key, Canonicalised, DeletionStatus, Disposition, Enqueued, NewSyncRequest,
    ResourceAdmission, SyncIntent, SyncRequestPage, SyncRequestRepo, CREATE_LEG, REMOVE_LEG,
};
use tam_types::{
    Actor, InventoryId, MappingId, OrgId, ProductId, Stamp, SystemComponent, Timestamp, Uuid,
};

use common::{seed_org_a, ORG_A};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const REQUEST: Uuid = Uuid([0x71; 16]);
const T0: Timestamp = Timestamp(1_000);

fn request_for(id: Uuid, locators: &[&str]) -> NewSyncRequest {
    NewSyncRequest {
        id,
        source: InventoryId::Tes,
        target: InventoryId::Tpt,
        disposition: Disposition::Sync,
        intent: SyncIntent::Draft,
        requested_at: T0,
        locators: locators.iter().map(|l| (*l).to_owned()).collect(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_deleted_request_cannot_admit_a_prelisted_resource(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    repo.create(ORG_A, &request_for(REQUEST, &["101", "202"]))
        .await
        .expect("the request writes");
    let stamp = Stamp {
        at: T0,
        actor: Actor::System(SystemComponent::Device),
    };
    assert_eq!(
        repo.delete(ORG_A, REQUEST, stamp)
            .await
            .expect("the request stops"),
        Some(DeletionStatus::Deleted)
    );
    assert_eq!(
        repo.admit_resource(ORG_A, REQUEST, "101", T0)
            .await
            .expect("the admission is decided"),
        ResourceAdmission::Stopped
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_stopped_request_may_finish_only_the_resource_already_admitted(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    repo.create(ORG_A, &request_for(REQUEST, &["101", "202"]))
        .await
        .expect("the request writes");
    assert_eq!(
        repo.admit_resource(ORG_A, REQUEST, "101", T0)
            .await
            .expect("the first resource starts"),
        ResourceAdmission::Admitted(0)
    );
    let stamp = Stamp {
        at: T0,
        actor: Actor::System(SystemComponent::Device),
    };
    assert_eq!(
        repo.delete(ORG_A, REQUEST, stamp)
            .await
            .expect("the request stops"),
        Some(DeletionStatus::Stopping)
    );
    assert_eq!(
        repo.admit_resource(ORG_A, REQUEST, "101", T0)
            .await
            .expect("the earlier resource may resume"),
        ResourceAdmission::Admitted(0)
    );
    assert_eq!(
        repo.admit_resource(ORG_A, REQUEST, "202", T0)
            .await
            .expect("the next admission is decided"),
        ResourceAdmission::Stopped
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
         VALUES ($1, $2, 'tes', 'tpt', 'sync', 'draft', 'pending', now())",
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

/// The migration list's page boundary, which is where this list can lose a
/// row.
///
/// Every request a seller raises in one submit carries the same
/// `requested_at`, so a page of the list ends inside a tie. The keyset is
/// `(requested_at, id)` — the pair the `ORDER BY` sorts by — and this walk is
/// the record of it: a cursor on the instant alone would hand over one of the
/// three and leave the other two unreachable.
#[sqlx::test(migrations = "./migrations")]
async fn a_walk_of_the_list_loses_no_request_that_shares_an_instant(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    for byte in [0x90u8, 0x91, 0x92] {
        repo.create(ORG_A, &request_for(Uuid([byte; 16]), &["101"]))
            .await
            .expect("the request writes");
    }

    let mut seen: Vec<Uuid> = Vec::new();
    let mut after: Option<(Timestamp, Uuid)> = None;
    for _turn in 0..5 {
        let page = repo
            .list(
                ORG_A,
                &SyncRequestPage {
                    after,
                    limit: 1,
                    disposition: None,
                    state: None,
                },
            )
            .await
            .expect("the page reads");
        let Some(row) = page.first() else {
            after = None;
            break;
        };
        seen.push(row.id);
        after = Some((row.requested_at, row.id));
    }

    seen.sort_unstable_by_key(|id| id.0);
    seen.dedup_by_key(|id| id.0);
    assert_eq!(
        seen.len(),
        3,
        "all three requests of the shared instant are reachable, each once"
    );
    assert!(
        after.is_none(),
        "the walk reaches the end and stops rather than looping"
    );
}

/// The narrowing is applied before the limit.
///
/// A page filtered after the limit answers "the migrations among the newest
/// one request", which is empty for a seller whose newest request was a sync
/// — and the migration screen would show them no history at all.
#[sqlx::test(migrations = "./migrations")]
async fn the_disposition_narrows_before_the_limit(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = SyncRequestRepo::new(pool);
    let migrated = Uuid([0x95; 16]);
    let mut migration = request_for(migrated, &["101"]);
    migration.disposition = Disposition::Migrate;
    repo.create(ORG_A, &migration)
        .await
        .expect("the migration writes");
    // Newer than the migration, so an unfiltered page of one holds only this.
    let mut newer = request_for(Uuid([0x96; 16]), &["202"]);
    newer.requested_at = Timestamp(2_000);
    repo.create(ORG_A, &newer).await.expect("the sync writes");

    let page = repo
        .list(
            ORG_A,
            &SyncRequestPage {
                after: None,
                limit: 1,
                disposition: Some(Disposition::Migrate),
                state: None,
            },
        )
        .await
        .expect("the page reads");
    assert_eq!(
        page.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![migrated],
        "the one migration is on the first page even though a newer sync exists"
    );
}

/// The itemless anchor a legacy migration left, minted with the request's
/// import key and naming no workflow of its own.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn legacy_anchor(pool: &PgPool, org: OrgId, key_of: Uuid) -> tam_types::JobId {
    let minted = tam_storage::JobRepo::new(pool.clone())
        .create_with_request_key(
            org,
            tam_storage::JobOrigin {
                request_key: job_request_key(key_of, tam_storage::IMPORT_LEG),
                run: None,
                import_run: None,
            },
            &tam_storage::NewJob {
                job: tam_types::JobId(Uuid(*uuid::Uuid::new_v4().as_bytes())),
                inventory: InventoryId::Tes,
                stamp: Stamp::system(SystemComponent::Import, T0),
            },
            &[],
        )
        .await
        .expect("the anchor mints");
    match minted {
        tam_storage::Minted::Job(created) => Some(created.job),
        tam_storage::Minted::WorkflowDeleted(_) => None,
    }
    .expect("an anchor names no workflow that could refuse it")
}

/// Normalization links a legacy migration's anchor to its request without
/// touching an import's own anchor, another tenant's rows, or anything it has
/// already linked.
///
/// The three boundaries worth asserting, because none of them is decidable
/// from the anchor alone. An import run's anchor is minted with
/// `job_request_key(run, IMPORT_LEG)` too (`tam-api/src/import_runs.rs`), so
/// a run whose id matches a request's is reached by exactly the derivation
/// this pass runs in reverse — and reassigning it would move the fence off
/// the import that owns the work. The import key is per tenant, so two
/// tenants can hold one request id and one tenant's pass must leave the
/// other's anchor alone. And the pass is run from a launcher on every boot,
/// so a second run over normalized rows has to move nothing.
#[sqlx::test(migrations = "./migrations")]
async fn normalizing_legacy_anchors_spares_import_anchors_and_other_tenants(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await
        .expect("org b seeds");

    let repo = SyncRequestRepo::new(pool.clone());
    let jobs = tam_storage::JobRepo::new(pool.clone());

    // A tombstoned migration, which is the row whose anchor matters most: its
    // next page is the one that must be refused.
    repo.create(ORG_A, &request_for(REQUEST, &["101"]))
        .await
        .expect("the migration writes");
    let orphan = legacy_anchor(&pool, ORG_A, REQUEST).await;
    repo.delete(ORG_A, REQUEST, Stamp::system(SystemComponent::Device, T0))
        .await
        .expect("the deletion answers")
        .expect("the request is this tenant's");

    // The same request id in another tenant, with its own unlinked anchor.
    repo.create(ORG_B, &request_for(REQUEST, &["101"]))
        .await
        .expect("org b's migration writes");
    let elsewhere = legacy_anchor(&pool, ORG_B, REQUEST).await;

    // A request and an import run sharing one id, so the import's own anchor
    // holds the very key this pass derives.
    let shared = Uuid([0x77; 16]);
    repo.create(ORG_A, &request_for(shared, &["202"]))
        .await
        .expect("the colliding request writes");
    let native = legacy_anchor(&pool, ORG_A, shared).await;
    tam_storage::ImportRunRepo::new(pool.clone())
        .create(
            ORG_A,
            &tam_storage::NewImportRun {
                id: shared,
                kind: tam_storage::RunKind::Marketplace,
                source: Some(InventoryId::Tes),
                batch_id: None,
                target: None,
                anchor_job: native,
                created_at: T0,
                scheduled: false,
                start_key: None,
                retry_of: None,
            },
        )
        .await
        .expect("the import opens");

    let reads = tam_storage::JobReadRepo::new(pool.clone());
    let before = reads
        .events_after(ORG_A, 0, 100)
        .await
        .expect("the tenant's events read");

    assert_eq!(
        repo.normalize_migration_anchors(ORG_A)
            .await
            .expect("the pass runs"),
        1,
        "the one anchor naming nothing is linked; the import's own anchor is not"
    );
    assert_eq!(
        jobs.owner(ORG_A, orphan).await.expect("the owner reads"),
        Some(tam_storage::JobOwner::SyncRequest(REQUEST)),
        "the tombstoned migration now owns its anchor, so its deletion fences it and its next \
         page is refused"
    );
    assert_eq!(
        jobs.owner(ORG_A, native).await.expect("the owner reads"),
        Some(tam_storage::JobOwner::ImportRun(shared)),
        "the import's own anchor keeps the import as its owner even though it holds the key a \
         request of the same id derives"
    );
    assert_eq!(
        jobs.owner(ORG_B, elsewhere).await.expect("the owner reads"),
        Some(tam_storage::JobOwner::Standalone),
        "one tenant's pass links nothing in another's, even where both hold the same request id"
    );
    assert_eq!(
        reads
            .events_after(ORG_A, 0, 100)
            .await
            .expect("the tenant's events read"),
        before,
        "the pass writes one column and appends no event, so every anchor's history is exactly \
         what it was"
    );

    assert_eq!(
        repo.normalize_migration_anchors(ORG_A)
            .await
            .expect("the second pass runs"),
        0,
        "a launcher that runs this on every boot moves nothing the second time"
    );
}
