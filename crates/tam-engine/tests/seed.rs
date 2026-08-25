//! The seed producer against a live catalogue: a projectable mapping yields
//! the machine seed field-for-field, and a blocked one raises its queue
//! items and reports the park gate — the worker cannot lease what it cannot
//! project.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_engine::seed::{seed_for_item, SeedOutcome};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_storage::{LeasedItem, MappingRepo, ProductRepo, TaxonomyRepo};
use tam_types::{
    CanonicalTermId, ContentHash, FieldKey, FileId, FileKind, FileRole, InventoryId, JobId,
    ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId,
    ScanOutcome, Timestamp, Title, Uuid,
};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const NOW: Timestamp = Timestamp(1_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool, with_nz_edge: bool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let taxonomy = TaxonomyRepo::new(pool.clone());
    let terms = [CanonicalTerm {
        id: SUBJECT,
        kind: TermKind::Subject,
        parent: None,
        label: "Maths for early years".to_owned(),
    }];
    let mut edges = vec![edge(InventoryId::TesGb, "1000454")];
    if with_nz_edge {
        edges.push(edge(InventoryId::TesNz, "7000454"));
    }
    taxonomy
        .seed(&terms, &edges)
        .await
        .expect("the crosswalk seeds");

    let product = tam_domain::CanonicalProduct {
        id: PRODUCT,
        org: ORG,
        title: Title("Fractions practice".to_owned()),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([0x21; 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([0x51; 32]),
                byte_len: 4,
                scan: ScanOutcome::Clean { at: NOW },
            },
            vec![],
        ),
        cover: Some(ProductFile {
            id: FileId(Uuid([0x22; 16])),
            role: FileRole::Cover,
            kind: FileKind::Image,
            hash: ContentHash([0x52; 32]),
            byte_len: 4,
            scan: ScanOutcome::Clean { at: NOW },
        }),
        previews: vec![],
        subjects: vec![SUBJECT],
        grades: tam_domain::GradeDeclaration {
            source: tam_domain::DeclarationSource::Imported {
                vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Phase),
            },
            raw: vec![VocabularyPath {
                vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Phase),
                segments: vec!["5-7".to_owned()],
                native_id: Some("2".to_owned()),
            }],
            derived: Some(tam_domain::AgeInterval::new(5, 7).expect("a bounded range")),
        },
        price: PriceIntent::Free,
    };
    ProductRepo::new(pool.clone())
        .insert(ORG, &product, NOW)
        .await
        .expect("the product inserts");
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &tam_domain::Mapping {
                id: MAPPING,
                org: ORG,
                product: PRODUCT,
                inventory: InventoryId::TesNz,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: PriceRule::Explicit(PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
}

fn edge(inventory: InventoryId, native: &str) -> ProjectionEdge {
    ProjectionEdge {
        from: SUBJECT,
        to: VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Subject),
            segments: vec!["Maths for early years".to_owned()],
            native_id: Some(native.to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: NOW,
    }
}

fn lease() -> LeasedItem {
    LeasedItem {
        org: ORG,
        item: tam_domain::JobItemId(Uuid([0x41; 16])),
        job: JobId(Uuid([0x42; 16])),
        mapping: MAPPING,
        inventory: InventoryId::TesNz,
        idempotency_key: derive_idempotency_key(
            ORG,
            InventoryId::TesNz,
            PRODUCT,
            1,
            ContentHash([0x51; 32]),
        ),
        lease_epoch: 0,
        attempt_count: 0,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn entry(seed: &tam_engine::driver::MachineSeed, key: FieldKey) -> String {
    seed.fields
        .entries
        .iter()
        .find(|(field, _)| *field == key)
        .map(|(_, value)| value.clone())
        .expect("the seed carries the field")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_projectable_mapping_seeds_the_machine(pool: PgPool) {
    provision(&pool, true).await;
    let outcome = seed_for_item(&pool, &lease(), NOW)
        .await
        .expect("the seed runs");
    let SeedOutcome::Ready(seed) = outcome else {
        panic!("a covered mapping seeds");
    };
    assert_eq!(entry(&seed, FieldKey::Title), "Fractions practice");
    assert_eq!(entry(&seed, FieldKey::Price), "CC-BY", "free is CC-BY");
    let taxonomy: serde_json::Value =
        serde_json::from_str(&entry(&seed, FieldKey::Taxonomy)).expect("taxonomy is JSON");
    assert_eq!(
        taxonomy["categories"],
        serde_json::json!([7_000_454]),
        "the NZ category id travelled from the crosswalk into the seed"
    );
    let grades: serde_json::Value =
        serde_json::from_str(&entry(&seed, FieldKey::Grades)).expect("grades are JSON");
    assert_eq!(grades["ageRanges"], serde_json::json!([2]));
    assert_eq!(
        grades["ages"],
        serde_json::json!([5, 6, 7]),
        "the derived interval expands into the ages list"
    );
    assert_eq!(seed.fields.files, vec![FileId(Uuid([0x21; 16]))]);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_gap_parks_the_item_behind_the_queue_it_just_raised(pool: PgPool) {
    provision(&pool, false).await;
    let outcome = seed_for_item(&pool, &lease(), NOW)
        .await
        .expect("the seed runs");
    let SeedOutcome::Blocked { gate, raised } = outcome else {
        panic!("no NZ edge means the seed blocks");
    };
    assert_eq!(gate, "reconciliation");
    assert_eq!(raised.new, 1, "the gap raised its item");
    let open = TaxonomyRepo::new(pool)
        .open_items(ORG)
        .await
        .expect("the queue reads");
    assert_eq!(open.len(), 1, "the founder sees the gap the worker hit");
}
