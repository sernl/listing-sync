//! field_audit is append-only for the application role: insert and select
//! are granted, update and delete are revoked, and the revocation binds the
//! owner too because privileges are checked against the ACL, not ownership.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_types::{OrgId, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));

async fn seed_org_a(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_audit_trail_cannot_be_rewritten(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO field_audit \
         (org_id, mapping_id, field, intended, observed_before, observed_after, \
          class, normaliser_version, created_at) \
         VALUES ($1, $2, 'title', 'a', NULL, 'a', 'normalised', 1, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x31; 16]))
    .execute(&mut *tx)
    .await
    .expect("the application role may append");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM field_audit")
        .fetch_one(&mut *tx)
        .await
        .expect("the application role may read");
    assert_eq!(count, 1, "the appended row is visible under the tenant pin");

    let rewritten = sqlx::query("UPDATE field_audit SET intended = 'b'")
        .execute(&mut *tx)
        .await;
    assert!(
        rewritten.is_err(),
        "update on field_audit must be denied to the application role"
    );
    tx.commit().await.expect("the read-write half commits");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let erased = sqlx::query("DELETE FROM field_audit")
        .execute(&mut *tx)
        .await;
    assert!(
        erased.is_err(),
        "delete on field_audit must be denied to the application role"
    );
}
