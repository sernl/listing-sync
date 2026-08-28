//! The job-ledger structural negatives: the partial unique index that makes a
//! duplicate-upload storm impossible while an attempt is in flight, and the
//! idempotency-key uniqueness on job items.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{MappingRepo, ProductRepo};
use tam_types::{InventoryId, MappingId, PriceIntent, PriceRule, Timestamp, Uuid};

mod common;
use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));
const JOB_1: Uuid = Uuid([0x41; 16]);
const ITEM_1: Uuid = Uuid([0x42; 16]);

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

fn unbound_mapping() -> Mapping {
    Mapping {
        id: MAPPING_1,
        org: ORG_A,
        product: PRODUCT_1,
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
    }
}

async fn pin_a(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_A.0).to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn insert_attempt(pool: &PgPool, attempt: Uuid, state: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    pin_a(&mut tx).await?;
    sqlx::query(
        "INSERT INTO write_attempt \
         (org_id, id, job_item_id, mapping_id, lease_epoch, intent, intent_hash, \
          state, opened_at) \
         VALUES ($1, $2, $3, $4, 1, '{}'::jsonb, decode('00', 'hex'), $5, now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(attempt))
    .bind(db_uuid(ITEM_1))
    .bind(db_uuid(MAPPING_1.0))
    .bind(state)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn one_write_attempt_in_flight_per_mapping(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), Timestamp(1))
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(pool.clone())
        .insert(ORG_A, &unbound_mapping(), 0, Timestamp(1))
        .await
        .expect("the fixture mapping inserts");
    let mut tx = pool.begin().await.expect("transaction begins");
    pin_a(&mut tx).await.expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO job (org_id, id, inventory, marketplace, created_at) \
         VALUES ($1, $2, 'tes_gb', 'tes', now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(JOB_1))
    .execute(&mut *tx)
    .await
    .expect("the fixture job inserts");
    sqlx::query(
        "INSERT INTO job_item \
         (org_id, id, job_id, mapping_id, idempotency_key, state, created_at, operation) \
         VALUES ($1, $2, $3, $4, $5, 'queued', now(), 'create')",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(ITEM_1))
    .bind(db_uuid(JOB_1))
    .bind(db_uuid(MAPPING_1.0))
    .bind(db_uuid(Uuid([0x61; 16])))
    .execute(&mut *tx)
    .await
    .expect("the fixture item inserts");
    tx.commit().await.expect("fixture commits");

    insert_attempt(&pool, Uuid([0x71; 16]), "in_flight")
        .await
        .expect("the first in-flight attempt opens");
    let second = insert_attempt(&pool, Uuid([0x72; 16]), "in_flight").await;
    assert!(
        second.is_err(),
        "a second in-flight attempt for the same mapping must fail at the database"
    );
    insert_attempt(&pool, Uuid([0x73; 16]), "settled")
        .await
        .expect("a settled attempt is not fenced; the index is partial by design");
    let third = insert_attempt(&pool, Uuid([0x74; 16]), "in_flight").await;
    assert!(
        third.is_err(),
        "the fence must still hold while the first attempt stays in flight"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_reused_idempotency_key_is_refused(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), Timestamp(1))
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(pool.clone())
        .insert(ORG_A, &unbound_mapping(), 0, Timestamp(1))
        .await
        .expect("the fixture mapping inserts");
    seed_ledger_job_only(&pool)
        .await
        .expect("the fixture job inserts");

    let mut tx = pool.begin().await.expect("transaction begins");
    pin_a(&mut tx).await.expect("tenant pin applies");
    for (item, expect_ok) in [(ITEM_1, true), (Uuid([0x43; 16]), false)] {
        let inserted = sqlx::query(
            "INSERT INTO job_item \
             (org_id, id, job_id, mapping_id, idempotency_key, state, created_at, operation) \
             VALUES ($1, $2, $3, $4, $5, 'queued', now(), 'create')",
        )
        .bind(db_uuid(ORG_A.0))
        .bind(db_uuid(item))
        .bind(db_uuid(JOB_1))
        .bind(db_uuid(MAPPING_1.0))
        .bind(db_uuid(Uuid([0x61; 16])))
        .execute(&mut *tx)
        .await;
        assert_eq!(
            inserted.is_ok(),
            expect_ok,
            "job_item_idempotent must admit the first key and refuse its reuse"
        );
    }
}

async fn seed_ledger_job_only(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    pin_a(&mut tx).await?;
    sqlx::query(
        "INSERT INTO job (org_id, id, inventory, marketplace, created_at) \
         VALUES ($1, $2, 'tes_gb', 'tes', now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(JOB_1))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
