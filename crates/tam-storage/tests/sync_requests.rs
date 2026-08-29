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

/// The role decision, pinned at the database rather than in prose. The drain
/// runs as `tam_app` precisely so the cross-tenant role never needs a grant
/// here; a grant added later to make something pass would fail this.
#[sqlx::test(migrations = "./migrations")]
async fn the_cross_tenant_role_cannot_read_or_write_a_sync_request(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    SyncRequestRepo::new(pool.clone())
        .create(ORG_A, &request_for(REQUEST, &["101"]))
        .await
        .expect("the request writes");
    let engine = engine_pool(&pool).await;

    let read = sqlx::query("SELECT id FROM sync_request")
        .fetch_all(&engine)
        .await;
    assert!(
        read.is_err(),
        "the engine holds no grant on the read leg's own table: {read:?}"
    );
    let written = sqlx::query(
        "INSERT INTO sync_request \
         (org_id, id, source, target, disposition, intent, state, requested_at) \
         VALUES ($1, $2, 'tes_gb', 'tes_nz', 'sync', 'draft', 'pending', now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x72; 16]))
    .execute(&engine)
    .await;
    assert!(
        written.is_err(),
        "and it cannot enqueue one either: {written:?}"
    );
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
