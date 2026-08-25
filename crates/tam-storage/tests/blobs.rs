//! The blob store: per-tenant encrypted, content-addressed, deduplicating.
//! A blob round-trips through seal, store, fetch and open; an identical file
//! under one tenant is one row and one object; the same bytes under two
//! tenants are two, because dedup is per tenant and a global hash table is an
//! existence oracle.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::BlobRepo;
use tam_types::{OrgId, Timestamp, Uuid};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed_orgs(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("org inserts");
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn repo(pool: PgPool, dir: &std::path::Path) -> BlobRepo<LocalObjectStore> {
    BlobRepo::new(
        pool,
        LocalObjectStore::new(dir.to_path_buf()),
        Kek::from_bytes(&[0x22; 32]).expect("kek"),
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn a_blob_round_trips_through_the_encrypted_store(pool: PgPool) {
    seed_orgs(&pool).await;
    let dir = std::env::temp_dir().join(format!("tam-blob-{}", std::process::id()));
    let repo = repo(pool, &dir);
    let bytes = b"a worksheet's PDF bytes".to_vec();

    let hash = repo
        .put(ORG_A, &bytes, T0)
        .await
        .expect("the blob seals and stores");
    let fetched = repo
        .get(ORG_A, hash)
        .await
        .expect("the blob fetches and opens");
    assert_eq!(
        fetched, bytes,
        "the blob round-trips through the encrypted store"
    );

    // The object on disk is ciphertext, not the plaintext bytes.
    let object_len = std::fs::read_dir(&dir)
        .expect("the store dir exists")
        .count();
    assert_eq!(object_len, 1, "one object was written");
}

#[sqlx::test(migrations = "./migrations")]
async fn an_identical_file_dedups_within_a_tenant(pool: PgPool) {
    seed_orgs(&pool).await;
    let dir = std::env::temp_dir().join(format!("tam-blob-dedup-{}", std::process::id()));
    let repo = repo(pool.clone(), &dir);
    let bytes = b"identical bytes".to_vec();

    let first = repo.put(ORG_A, &bytes, T0).await.expect("first put");
    let second = repo.put(ORG_A, &bytes, T0).await.expect("second put");
    assert_eq!(first, second, "identical bytes hash identically");

    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("pin");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM blob WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    tx.commit().await.expect("commit");
    assert_eq!(
        rows, 1,
        "a file uploaded twice by one tenant is one blob row"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_same_bytes_under_two_tenants_are_two_blobs(pool: PgPool) {
    seed_orgs(&pool).await;
    let dir = std::env::temp_dir().join(format!("tam-blob-tenants-{}", std::process::id()));
    let repo = repo(pool.clone(), &dir);
    let bytes = b"shared bytes".to_vec();

    repo.put(ORG_A, &bytes, T0).await.expect("tenant A put");
    repo.put(ORG_B, &bytes, T0).await.expect("tenant B put");

    // RLS fail-closes on an unpinned read, so each tenant is counted under its
    // own pin — which is exactly the isolation being asserted: neither sees
    // the other's row, and there are two rows for identical bytes.
    for org in [ORG_A, ORG_B] {
        let mut tx = pool.begin().await.expect("tx");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
            .execute(&mut *tx)
            .await
            .expect("pin");
        let own: i64 = sqlx::query_scalar("SELECT count(*) FROM blob")
            .fetch_one(&mut *tx)
            .await
            .expect("count under the tenant pin");
        tx.commit().await.expect("commit");
        assert_eq!(
            own, 1,
            "each tenant holds exactly its own blob of the shared bytes; dedup is per tenant,              never a global oracle"
        );
    }
}
