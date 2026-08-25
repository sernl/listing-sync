//! The two-tenant negative test: row-level security, not the queries'
//! `WHERE org_id` clauses, is what keeps tenant A's rows out of tenant B's
//! reads. The raw probes below deliberately omit any org filter so the test
//! fails if the policy is dropped, disabled, or not FORCEd onto the owner.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{NewProduct, ProductRepo};
use tam_types::{ListingCopy, OrgId, PriceIntent, ProductId, Timestamp, Title, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_1: ProductId = ProductId(Uuid([0x01; 16]));

fn sample_product() -> NewProduct {
    NewProduct {
        id: PRODUCT_1,
        title: Title("Fractions revision pack".to_owned()),
        body: ListingCopy {
            body: "A worked example pack.".to_owned(),
        },
        price: PriceIntent::Free,
        created_at: Timestamp(1_756_000_000_000),
        updated_at: Timestamp(1_756_000_000_000),
    }
}

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

async fn seed_organisations(pool: &PgPool) -> Result<(), sqlx::Error> {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(db_uuid(org.0))
            .bind(name)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Counts every visible product row with NO org filter, under the given pin.
async fn visible_rows(pool: &PgPool, pin: Option<OrgId>) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    if let Some(org) = pin {
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(db_uuid(org.0).to_string())
            .execute(&mut *tx)
            .await?;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM product")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(count)
}

#[sqlx::test(migrations = "./migrations")]
async fn tenant_b_sees_nothing_of_tenant_a(pool: PgPool) {
    seed_organisations(&pool)
        .await
        .expect("organisation fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &sample_product())
        .await
        .expect("tenant A inserts a product");

    let found = repo
        .get(ORG_A, PRODUCT_1)
        .await
        .expect("tenant A reads back");
    assert!(
        found.is_some(),
        "positive control: tenant A must see its own row, or every assertion below is vacuous"
    );

    assert_eq!(
        visible_rows(&pool, Some(ORG_A))
            .await
            .expect("the pinned probe runs"),
        1,
        "probe control: the unfiltered probe must see A's row under A's pin"
    );
    assert_eq!(
        visible_rows(&pool, Some(ORG_B))
            .await
            .expect("the pinned probe runs"),
        0,
        "row-level security must hide tenant A's rows from tenant B even without a WHERE clause"
    );
    let listed = repo.list(ORG_B).await.expect("tenant B lists");
    assert!(
        listed.is_empty(),
        "the repository surface must return nothing for tenant B"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unpinned_connection_sees_an_empty_table(pool: PgPool) {
    seed_organisations(&pool)
        .await
        .expect("organisation fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &sample_product())
        .await
        .expect("tenant A inserts a product");

    assert_eq!(
        visible_rows(&pool, None)
            .await
            .expect("the unpinned probe runs"),
        0,
        "a connection that never declared its tenant must fail closed and see nothing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn tenant_b_cannot_write_a_row_into_tenant_a(pool: PgPool) {
    seed_organisations(&pool)
        .await
        .expect("organisation fixture rows insert");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_B.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let smuggled = sqlx::query(
        "INSERT INTO product \
         (org_id, id, title, body, price_kind, price_minor_units, price_currency, \
          created_at, updated_at) \
         VALUES ($1, $2, 't', 'b', 'free', NULL, NULL, now(), now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(PRODUCT_1.0))
    .execute(&mut *tx)
    .await;
    assert!(
        smuggled.is_err(),
        "the WITH CHECK half of the policy must refuse a write into another tenant"
    );
}
