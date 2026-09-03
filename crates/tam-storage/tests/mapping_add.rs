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
use tam_domain::{
    Binding, CanonicalProduct, FieldPolicies, FieldPolicy, Mapping, PublishMode, Verification,
};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{MappingAdd, MappingRepo, PastedBind, ProductRepo};
use tam_types::{
    ContentHash, FileId, FileKind, FileRole, InventoryId, MappingId, OrgId, PayloadSet,
    PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome, Timestamp, Uuid,
};

mod common;
use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_2: ProductId = ProductId(Uuid([0x02; 16]));
const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));
const MAPPING_2: MappingId = MappingId(Uuid([0x32; 16]));
const NOW: Timestamp = Timestamp(5_000);

/// A second product in the same organisation, carrying its own payload file.
///
/// `product_file` is keyed by `(org_id, id)`, so two products in one tenant
/// cannot share a file identifier; reusing the fixture's whole product for a
/// same-tenant second row collides on that key rather than on anything the
/// test is about.
fn second_product() -> CanonicalProduct {
    let mut product = minimal_product();
    product.id = PRODUCT_2;
    product.payload = PayloadSet::new(
        ProductFile {
            id: FileId(Uuid([0x22; 16])),
            role: FileRole::Payload,
            kind: FileKind::Pdf,
            hash: ContentHash([0x52; 32]),
            byte_len: 4,
            scan: ScanOutcome::Pending,
        },
        vec![],
    );
    product
}

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

/// The head carries the bound listing so the API can derive the page a seller
/// opens; an unbound mapping carries none, which is what makes `listing_url`
/// null rather than a link to a listing that does not exist.
#[sqlx::test(migrations = "./migrations")]
async fn a_head_carries_the_listing_only_while_the_mapping_is_bound(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");

    let repo = MappingRepo::new(pool.clone());
    repo.add(
        ORG_A,
        &unbound(ORG_A, PRODUCT_1, InventoryId::TesGb, MAPPING_1),
        0,
        NOW,
    )
    .await
    .expect("the unbound add lands");

    let mut bound = unbound(ORG_A, PRODUCT_1, InventoryId::Tpt, MAPPING_2);
    bound.binding = Binding::Bound {
        id: RemoteListingId::Tpt {
            product_id: 17_511_712,
        },
        first_seen: NOW,
        verified: Verification::Stale { since: NOW },
    };
    repo.insert(ORG_A, &bound, 0, NOW)
        .await
        .expect("the bound mapping inserts");

    let heads = repo.list_heads(ORG_A).await.expect("the heads read");
    let unbound_head = heads
        .iter()
        .find(|head| head.id == MAPPING_1)
        .expect("the unbound mapping lists");
    assert_eq!(
        unbound_head.remote, None,
        "an unbound mapping binds no listing, so the head names none"
    );
    let bound_head = heads
        .iter()
        .find(|head| head.id == MAPPING_2)
        .expect("the bound mapping lists");
    assert_eq!(
        bound_head.remote,
        Some(RemoteListingId::Tpt {
            product_id: 17_511_712
        }),
        "the head carries the identifier the binding holds"
    );

    let single = repo
        .head(ORG_A, MAPPING_2)
        .await
        .expect("the head reads")
        .expect("the bound mapping is readable");
    assert_eq!(
        single.remote, bound_head.remote,
        "the one-mapping read and the listing agree on the binding"
    );
}

/// Adopting a listing this tree did not create: the binding it writes, and the
/// two states it refuses.
#[sqlx::test(migrations = "./migrations")]
async fn a_paste_binds_an_unbound_mapping_and_leaves_it_awaiting_verification(pool: PgPool) {
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
    .expect("the add lands");

    let listing = RemoteListingId::Tpt {
        product_id: 17_511_712,
    };
    assert_eq!(
        repo.bind_pasted(ORG_A, MAPPING_1, &listing, NOW)
            .await
            .expect("the bind lands"),
        PastedBind::Bound
    );

    let record = repo
        .get(ORG_A, MAPPING_1)
        .await
        .expect("the aggregate reads")
        .expect("the mapping is there");
    match &record.mapping.binding {
        Binding::Bound {
            id,
            first_seen,
            verified,
        } => {
            assert_eq!(
                *id, listing,
                "the binding names the listing that was pasted"
            );
            assert_eq!(*first_seen, NOW);
            assert_eq!(
                *verified,
                Verification::Stale { since: NOW },
                "nothing read the listing back, so the binding is a claim awaiting verification"
            );
        }
        other @ (Binding::Unbound
        | Binding::Creating { .. }
        | Binding::AmbiguousCreate { .. }
        | Binding::Severed { .. }) => {
            panic!("a pasted bind leaves the mapping bound, not {other:?}")
        }
    }

    let head = repo
        .head(ORG_A, MAPPING_1)
        .await
        .expect("the head reads")
        .expect("the mapping is readable");
    assert_eq!(head.remote, Some(listing), "the head carries the binding");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_paste_onto_a_bound_mapping_is_refused_by_state(pool: PgPool) {
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
    .expect("the add lands");
    let first = RemoteListingId::Tpt {
        product_id: 17_511_712,
    };
    repo.bind_pasted(ORG_A, MAPPING_1, &first, NOW)
        .await
        .expect("the first bind lands");

    assert_eq!(
        repo.bind_pasted(
            ORG_A,
            MAPPING_1,
            &RemoteListingId::Tpt {
                product_id: 99_999_999
            },
            NOW,
        )
        .await
        .expect("the second paste answers rather than faulting"),
        PastedBind::NotUnbound {
            state: "bound".to_owned()
        },
    );
    let record = repo
        .get(ORG_A, MAPPING_1)
        .await
        .expect("the aggregate reads")
        .expect("the mapping is there");
    assert!(
        matches!(record.mapping.binding, Binding::Bound { ref id, .. } if *id == first),
        "the refused paste left the first binding exactly as it was"
    );
}

/// Two of a seller's own items must not both claim one listing: the partial
/// unique indexes of migration 0018 are what decide it, so the refusal is
/// proven against the database rather than against a read.
#[sqlx::test(migrations = "./migrations")]
async fn two_mappings_cannot_claim_one_listing(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the first product inserts");
    products
        .insert(ORG_A, &second_product(), NOW)
        .await
        .expect("the second product inserts");

    let repo = MappingRepo::new(pool.clone());
    for (product, id) in [(PRODUCT_1, MAPPING_1), (PRODUCT_2, MAPPING_2)] {
        repo.add(
            ORG_A,
            &unbound(ORG_A, product, InventoryId::Tpt, id),
            0,
            NOW,
        )
        .await
        .expect("the add lands");
    }

    let listing = RemoteListingId::Tpt {
        product_id: 17_511_712,
    };
    assert_eq!(
        repo.bind_pasted(ORG_A, MAPPING_1, &listing, NOW)
            .await
            .expect("the first claim lands"),
        PastedBind::Bound
    );
    assert_eq!(
        repo.bind_pasted(ORG_A, MAPPING_2, &listing, NOW)
            .await
            .expect("the second claim answers rather than faulting"),
        PastedBind::ListingClaimed
    );
    let second_head = repo
        .head(ORG_A, MAPPING_2)
        .await
        .expect("the head reads")
        .expect("the mapping is readable");
    assert_eq!(
        second_head.remote, None,
        "the refused claim left the second mapping unbound"
    );
}

/// A Tes listing binds by its canonical identity, which is what
/// `mapping_one_bound_url` indexes.
#[sqlx::test(migrations = "./migrations")]
async fn a_tes_paste_binds_the_canonical_identity(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &minimal_product(), NOW)
        .await
        .expect("the product inserts");
    let repo = MappingRepo::new(pool.clone());
    repo.add(
        ORG_A,
        &unbound(ORG_A, PRODUCT_1, InventoryId::TesGb, MAPPING_1),
        0,
        NOW,
    )
    .await
    .expect("the add lands");

    let listing = RemoteListingId::Tes {
        url: "https://www.tes.com/api/v2/resources/13264370".to_owned(),
    };
    repo.bind_pasted(ORG_A, MAPPING_1, &listing, NOW)
        .await
        .expect("the bind lands");
    let head = repo
        .head(ORG_A, MAPPING_1)
        .await
        .expect("the head reads")
        .expect("the mapping is readable");
    assert_eq!(head.remote, Some(listing));
}
