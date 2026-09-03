//! Adding a marketplace to a product that already exists: the mapping it
//! mints, the tenant fence around it, and what a second add of the same
//! marketplace answers.
//!
//! The duplicate case is the one worth a live database: it is decided by
//! `mapping_one_per_inventory` rather than by a read before the write, so a
//! test that stubbed the constraint would prove nothing about the race it
//! exists to settle.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{MappingAdd, MappingRepo, ProductRepo};
use tam_types::{
    InventoryId, MappingId, OrgId, PriceIntent, PriceRule, ProductId, Timestamp, Uuid,
};

mod common;
use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_2: ProductId = ProductId(Uuid([0x02; 16]));
const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));
const MAPPING_2: MappingId = MappingId(Uuid([0x32; 16]));
const NOW: Timestamp = Timestamp(5_000);

fn unbound(org: OrgId, product: ProductId, inventory: InventoryId, id: MappingId) -> Mapping {
    Mapping {
        id,
        org,
        product,
        inventory,
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

#[sqlx::test(migrations = "./migrations")]
async fn an_add_mints_the_mapping_a_create_would_have(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");

    let repo = MappingRepo::new(pool.clone());
    let added = repo
        .add(
            ORG_A,
            &unbound(ORG_A, PRODUCT_1, InventoryId::Tpt, MAPPING_1),
            0,
            NOW,
        )
        .await
        .expect("the add lands");
    assert_eq!(added, MappingAdd::Added);

    let head = repo
        .head(ORG_A, MAPPING_1)
        .await
        .expect("the head reads")
        .expect("the mapping just added is readable");
    assert_eq!(head.product, PRODUCT_1);
    assert_eq!(head.inventory, InventoryId::Tpt);
    assert_eq!(
        (head.binding_state.as_str(), head.lifecycle_state.as_str()),
        ("unbound", "absent"),
        "an added marketplace starts unbound and absent, exactly as a create's own mapping does"
    );

    let record = repo
        .get(ORG_A, MAPPING_1)
        .await
        .expect("the aggregate reads")
        .expect("the mapping is there");
    assert_eq!(record.mapping.binding, Binding::Unbound);
    assert_eq!(record.mapping.lifecycle, RemoteLifecycle::Absent);
    assert_eq!(
        record.mapping.publish,
        PublishMode::DryRun,
        "an add writes the catalogue and enqueues nothing, so it never elects a publish"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_add_of_the_same_marketplace_mints_nothing(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");

    let repo = MappingRepo::new(pool.clone());
    repo.add(
        ORG_A,
        &unbound(ORG_A, PRODUCT_1, InventoryId::Tpt, MAPPING_1),
        0,
        NOW,
    )
    .await
    .expect("the first add lands");

    let again = repo
        .add(
            ORG_A,
            &unbound(ORG_A, PRODUCT_1, InventoryId::Tpt, MAPPING_2),
            0,
            NOW,
        )
        .await
        .expect("the second add answers rather than faulting");
    assert_eq!(again, MappingAdd::AlreadyMapped);
    assert!(
        repo.head(ORG_A, MAPPING_2)
            .await
            .expect("the head reads")
            .is_none(),
        "the refused add writes no row at all"
    );
    assert_eq!(
        repo.list_for_product(ORG_A, PRODUCT_1)
            .await
            .expect("the product's mappings read")
            .len(),
        1,
        "one marketplace, one mapping"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn another_marketplace_on_the_same_product_is_a_second_mapping(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");

    let repo = MappingRepo::new(pool.clone());
    for (inventory, id) in [
        (InventoryId::Tpt, MAPPING_1),
        (InventoryId::TesGb, MAPPING_2),
    ] {
        assert_eq!(
            repo.add(ORG_A, &unbound(ORG_A, PRODUCT_1, inventory, id), 0, NOW)
                .await
                .expect("the add lands"),
            MappingAdd::Added
        );
    }
    assert_eq!(
        repo.list_for_product(ORG_A, PRODUCT_1)
            .await
            .expect("the product's mappings read")
            .len(),
        2
    );
}

/// The fence, probed the way the tenancy suite probes every other one: the
/// reads carry an org filter of their own, so the assertion is that a second
/// tenant sees nothing rather than that the filter was written.
#[sqlx::test(migrations = "./migrations")]
async fn one_tenants_added_marketplace_is_invisible_to_another(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(&pool)
        .await
        .expect("org b seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("tenant a's product inserts");
    let mut theirs = minimal_product();
    theirs.id = PRODUCT_2;
    theirs.org = ORG_B;
    products
        .insert(ORG_B, &theirs, NOW)
        .await
        .expect("tenant b's product inserts");

    let repo = MappingRepo::new(pool.clone());
    repo.add(
        ORG_A,
        &unbound(ORG_A, PRODUCT_1, InventoryId::Tpt, MAPPING_1),
        0,
        NOW,
    )
    .await
    .expect("tenant a's add lands");

    assert!(
        repo.head(ORG_B, MAPPING_1)
            .await
            .expect("the head reads")
            .is_none(),
        "tenant b cannot read tenant a's mapping by its identifier"
    );
    assert!(
        repo.list_heads(ORG_B)
            .await
            .expect("the listing reads")
            .is_empty(),
        "tenant b's mapping listing carries none of tenant a's"
    );
    assert_eq!(
        repo.add(
            ORG_B,
            &unbound(ORG_B, PRODUCT_2, InventoryId::Tpt, MAPPING_2),
            0,
            NOW,
        )
        .await
        .expect("tenant b's own add lands"),
        MappingAdd::Added,
        "the one-per-marketplace constraint is per tenant, not global"
    );
}
