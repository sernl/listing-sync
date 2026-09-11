//! Grade provenance: the seller's declaration is the fact, stored verbatim
//! as an ordered list of vocabulary paths, and the derived interval never
//! replaces it. Re-emitting to the vocabulary the declaration came from uses
//! the declaration, which is what makes the round-trip law hold by
//! construction; this test pins the imported, multi-path, native-id case the
//! aggregate round-trip's seller fixture does not reach.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_domain::{AgeInterval, DeclarationSource, GradeDeclaration, TermKind, VocabularyId};
use tam_storage::ProductRepo;
use tam_types::{InventoryId, Timestamp};

use common::{minimal_product, seed_org_a, ORG_A, PRODUCT_1};

fn phase_path(low: &str, native: &str) -> tam_domain::VocabularyPath {
    tam_domain::VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tes, TermKind::Phase),
        segments: vec![low.to_owned()],
        native_id: Some(native.to_owned()),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn an_imported_declaration_round_trips_verbatim_and_ordered(pool: PgPool) {
    seed_org_a(&pool).await.expect("org-a seeds");
    let mut product = minimal_product();
    product.grades = GradeDeclaration {
        source: DeclarationSource::Imported {
            vocabulary: VocabularyId(InventoryId::Tes, TermKind::Phase),
        },
        raw: vec![phase_path("5-7", "2"), phase_path("7-11", "3")],
        derived: Some(AgeInterval::new(5, 11).expect("a well-formed interval")),
    };
    let repo = ProductRepo::new(pool);
    repo.insert(ORG_A, &product, Timestamp(1_000))
        .await
        .expect("the product inserts");
    let read = repo
        .get(ORG_A, PRODUCT_1)
        .await
        .expect("the product reads back")
        .expect("the product exists");
    assert_eq!(
        read.product.grades, product.grades,
        "the declaration is the fact: source, ordered paths, native ids and \
         the derived interval all read back verbatim"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unbounded_declaration_keeps_no_invented_interval(pool: PgPool) {
    seed_org_a(&pool).await.expect("org-a seeds");
    let mut product = minimal_product();
    product.grades = GradeDeclaration {
        source: DeclarationSource::Seller,
        raw: vec![phase_path("16+", "6")],
        derived: None,
    };
    let repo = ProductRepo::new(pool);
    repo.insert(ORG_A, &product, Timestamp(1_000))
        .await
        .expect("the product inserts");
    let read = repo
        .get(ORG_A, PRODUCT_1)
        .await
        .expect("the product reads back")
        .expect("the product exists");
    assert_eq!(
        read.product.grades.derived, None,
        "an open-ended range derives nothing rather than an invented bound"
    );
    assert_eq!(
        read.product.grades.raw, product.grades.raw,
        "the verbatim declaration survives regardless"
    );
}
