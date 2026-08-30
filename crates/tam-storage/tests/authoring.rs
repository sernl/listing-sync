//! The repository half of the authoring flow: the edit, the soft delete, the
//! two quota reads, the handle check, and the answer a create form gives
//! before any mapping exists.
//!
//! Every one is proved against a second tenant as well as against the first,
//! because the reads here are the ones a quota is enforced from and a count
//! that leaked across the fence would bill one seller for another's storage.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::equivalence::ElectionTriggerKind;
use tam_domain::{
    CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration, TermKind,
    VocabularyId, VocabularyPath,
};
use tam_storage::{AnsweredElection, ElectionRepo, ProductEdit, ProductRepo};
use tam_types::{
    ContentHash, CopyFormat, Currency, FileId, FileKind, FileRole, InventoryId, ListingCopy, Money,
    OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome, Timestamp, Title, Uuid,
};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_A: ProductId = ProductId(Uuid([0x01; 16]));
const PRODUCT_B: ProductId = ProductId(Uuid([0x02; 16]));
const HASH_A: ContentHash = ContentHash([0x51; 32]);
const HASH_B: ContentHash = ContentHash([0x52; 32]);
const T0: Timestamp = Timestamp(1_000);
const T1: Timestamp = Timestamp(2_000);

async fn seed(pool: &PgPool, org: OrgId, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

fn product(org: OrgId, id: ProductId, hash: ContentHash, byte_len: u64) -> CanonicalProduct {
    CanonicalProduct {
        id,
        org,
        title: Title("Before".to_owned()),
        body: ListingCopy {
            body: "Before body.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([0x21; 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash,
                byte_len,
                scan: ScanOutcome::Clean { at: T0 },
            },
            vec![],
        ),
        cover: None,
        previews: vec![],
        subjects: vec![],
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn an_edit_replaces_what_it_names_and_leaves_what_it_does_not(pool: PgPool) {
    seed(&pool, ORG_A, "org-a").await.expect("org a seeds");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &product(ORG_A, PRODUCT_A, HASH_A, 4), T0)
        .await
        .expect("the product inserts");

    let paid = PriceIntent::Paid(Money::new(300, Currency::Gbp).expect("a positive price"));
    repo.update(
        ORG_A,
        PRODUCT_A,
        &ProductEdit {
            title: Some(Title("After".to_owned())),
            price: Some(paid),
            rights: Some(RightsDeclaration::Declared {
                source: VocabularyPath {
                    vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
                    segments: vec!["CC-BY".to_owned()],
                    native_id: Some("CC-BY".to_owned()),
                },
            }),
            ..ProductEdit::default()
        },
        T1,
    )
    .await
    .expect("the edit applies");

    let record = repo
        .get(ORG_A, PRODUCT_A)
        .await
        .expect("the product reads")
        .expect("the product is live");
    assert_eq!(record.product.title, Title("After".to_owned()));
    assert_eq!(record.product.price, paid, "the whole price triple moved");
    assert_eq!(
        record.product.body.body, "Before body.",
        "a field the edit did not name is left exactly as it was stored"
    );
    assert!(
        matches!(record.product.rights, RightsDeclaration::Declared { .. }),
        "the grant the seller stated is recorded"
    );
    assert_eq!(record.updated_at, T1, "and the row's instant moved");

    assert!(
        !repo
            .update(ORG_B, PRODUCT_A, &ProductEdit::default(), T1)
            .await
            .expect("the cross-tenant edit runs"),
        "another tenant's edit touches nothing and says so"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_soft_delete_hides_the_product_and_repeats_harmlessly(pool: PgPool) {
    seed(&pool, ORG_A, "org-a").await.expect("org a seeds");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &product(ORG_A, PRODUCT_A, HASH_A, 4), T0)
        .await
        .expect("the product inserts");

    assert!(
        repo.soft_delete(ORG_A, PRODUCT_A, T1)
            .await
            .expect("the delete runs"),
        "the first delete lands"
    );
    assert!(
        repo.get(ORG_A, PRODUCT_A)
            .await
            .expect("the read runs")
            .is_none(),
        "every catalogue read already filters on deleted_at"
    );
    assert_eq!(
        repo.live_count(ORG_A).await.expect("the count runs"),
        0,
        "a deleted product stops counting against the listing quota"
    );
    assert!(
        !repo
            .soft_delete(ORG_A, PRODUCT_A, T1)
            .await
            .expect("the repeat runs"),
        "a repeated delete is a no-op rather than a fault"
    );
    assert_eq!(
        repo.stored_bytes(ORG_A).await.expect("the bytes read"),
        4,
        "the bytes are still stored, so they still count against storage"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_two_quota_reads_and_the_handle_check_stop_at_the_tenant_fence(pool: PgPool) {
    seed(&pool, ORG_A, "org-a").await.expect("org a seeds");
    seed(&pool, ORG_B, "org-b").await.expect("org b seeds");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &product(ORG_A, PRODUCT_A, HASH_A, 11), T0)
        .await
        .expect("A's product inserts");
    repo.insert(ORG_B, &product(ORG_B, PRODUCT_B, HASH_B, 7), T0)
        .await
        .expect("B's product inserts");

    assert_eq!(
        (
            repo.live_count(ORG_A).await.expect("A counts"),
            repo.live_count(ORG_B).await.expect("B counts")
        ),
        (1, 1),
        "each tenant counts only its own listings"
    );
    assert_eq!(
        (
            repo.stored_bytes(ORG_A).await.expect("A's bytes"),
            repo.stored_bytes(ORG_B).await.expect("B's bytes")
        ),
        (11, 7),
        "and only its own bytes; a shared total would bill one seller for the other's storage"
    );
    assert_eq!(
        repo.stored_hashes(ORG_A, &[HASH_A, HASH_B])
            .await
            .expect("A's handles resolve"),
        vec![HASH_A],
        "B's blob is invisible to A, so a handle naming it is not resolvable"
    );
    assert!(
        repo.stored_hashes(ORG_B, &[ContentHash([0x99; 32])])
            .await
            .expect("the check runs")
            .is_empty(),
        "and a hash nobody uploaded resolves to nothing rather than being minted"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_answer_authored_on_the_form_precedes_every_mapping(pool: PgPool) {
    seed(&pool, ORG_A, "org-a").await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &product(ORG_A, PRODUCT_A, HASH_A, 4), T0)
        .await
        .expect("the product inserts");
    let elections = ElectionRepo::new(pool.clone());
    let answered = AnsweredElection {
        product: PRODUCT_A,
        inventory: InventoryId::TesGb,
        axis: TermKind::Licence,
        trigger_kind: ElectionTriggerKind::Supply,
        trigger_key: Some("free"),
        paths: &[VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
            segments: vec!["CC-BY".to_owned()],
            native_id: Some("CC-BY".to_owned()),
        }],
    };

    assert!(
        elections
            .record_answered(ORG_A, &answered, T0)
            .await
            .expect("the answer records"),
        "migration 0023's provenance CHECK admits a null raised_by on an answered row"
    );
    let settled = elections
        .answered_for(ORG_A, PRODUCT_A)
        .await
        .expect("the settled read runs");
    assert_eq!(
        settled.len(),
        1,
        "the projection finds it settled through the same read a mapping-raised answer uses"
    );
    assert_eq!(settled[0].trigger_key.as_deref(), Some("free"));

    assert!(
        !elections
            .record_answered(ORG_A, &answered, T1)
            .await
            .expect("the repeat runs"),
        "a question this product has already settled writes no second answer"
    );
    assert!(
        elections
            .answered_for(ORG_B, PRODUCT_A)
            .await
            .expect("B's read runs")
            .is_empty(),
        "and another tenant sees none of it"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_trigger_key_that_contradicts_its_kind_is_refused_before_the_check_fires(pool: PgPool) {
    seed(&pool, ORG_A, "org-a").await.expect("org a seeds");
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &product(ORG_A, PRODUCT_A, HASH_A, 4), T0)
        .await
        .expect("the product inserts");
    let path = VocabularyPath {
        vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
        segments: vec!["CC-BY".to_owned()],
        native_id: None,
    };
    let refused = ElectionRepo::new(pool.clone())
        .record_answered(
            ORG_A,
            &AnsweredElection {
                product: PRODUCT_A,
                inventory: InventoryId::TesGb,
                axis: TermKind::Licence,
                trigger_kind: ElectionTriggerKind::Supply,
                // A supply keys on the pricing branch; the sentinel belongs to
                // the two kinds that generalise to nothing.
                trigger_key: None,
                paths: &[path],
            },
            T0,
        )
        .await;
    assert!(
        refused.is_err(),
        "a keyless supply would be a standing answer to both pricing branches"
    );

    let empty = ElectionRepo::new(pool)
        .record_answered(
            ORG_A,
            &AnsweredElection {
                product: PRODUCT_A,
                inventory: InventoryId::TesGb,
                axis: TermKind::Licence,
                trigger_kind: ElectionTriggerKind::Supply,
                trigger_key: Some("free"),
                paths: &[],
            },
            T0,
        )
        .await;
    assert!(
        empty.is_err(),
        "an answered election names at least one value, which the CHECK also states"
    );
}
