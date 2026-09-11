//! Two writers racing on one product's files.
//!
//! The invariant under test is the one `product_payload_nonempty` cannot hold
//! on its own, and the reason the file-mutating methods take `FOR UPDATE` on
//! the product row. That trigger is `DEFERRABLE INITIALLY DEFERRED`, so it
//! fires inside each transaction at commit — and at that moment neither
//! transaction can see the other's uncommitted delete. Two concurrent removals
//! of two different payload files therefore each pass their own trigger, and a
//! product that had two payload files ends with none.
//!
//! Asserted here rather than argued from reading the lock ordering, because
//! the failure it prevents is invisible in the source: both statements are
//! correct, both triggers pass, and the corruption exists only in the
//! interleaving.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    Mapping, PublishMode, RightsDeclaration,
};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{FileRefusal, FileTarget, MappingRepo, ProductRepo};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, ListingCopy,
    MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x10; 16]));
const FIRST: FileId = FileId(Uuid([0x21; 16]));
const SECOND: FileId = FileId(Uuid([0x22; 16]));

fn db(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool) {
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db(ORG.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-race', now())")
        .bind(db(ORG.0))
        .execute(&mut *tx)
        .await
        .expect("the org inserts");
    tx.commit().await.expect("the seed commits");
}

/// Puts the product on a marketplace, which is what makes the payload
/// requirement apply to it at all: migration 0061 moved that requirement from
/// the product to the mapping, so an unmapped product may lose every file and
/// the race below would have nothing to race for.
///
/// Written through `MappingRepo` rather than as raw SQL, because a mapping row
/// answers to four CHECK constraints whose combinations only the repository
/// knows how to satisfy.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn list_it(pool: &PgPool) {
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &Mapping {
                id: MappingId(Uuid([0x31; 16])),
                org: ORG,
                product: PRODUCT,
                inventory: InventoryId::Tes,
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
            },
            0,
            T0,
        )
        .await
        .expect("the mapping inserts");
}

fn held(id: FileId, marker: u8) -> ProductFile {
    ProductFile {
        id,
        role: FileRole::Payload,
        kind: FileKind::Pdf,
        bytes: FileBytes::Held {
            hash: ContentHash([marker; 32]),
            byte_len: 2_048,
            scan: ScanOutcome::Clean { at: T0 },
        },
    }
}

/// A product with exactly two payload files, which is the smallest shape in
/// which two removals can both look safe and together be fatal.
fn two_payloads() -> CanonicalProduct {
    CanonicalProduct {
        id: PRODUCT,
        org: ORG,
        title: Title("Two files, one product".to_owned()),
        body: ListingCopy {
            body: "A worksheet and its answer key.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(held(FIRST, 0x5A), vec![held(SECOND, 0x5B)])),
        cover: None,
        previews: vec![],
        subjects: vec![],
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            derived: None,
            raw: vec![],
        },
        price: PriceIntent::Free,
        rights: RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn two_removals_racing_cannot_leave_a_product_without_a_payload(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG, &two_payloads(), T0)
        .await
        .unwrap_or_else(|error| panic!("the product seeds: {error}"));
    list_it(&pool).await;

    // Both see two live payload files if they are allowed to look at once.
    let (left, right) = tokio::join!(
        repo.remove_file(
            ORG,
            FileTarget {
                product: PRODUCT,
                file: FIRST,
            },
            // No thumbnail to redraw: this product carries none, which is what
            // isolates the race from the redraw the route would otherwise add.
            // The mapping seeded above is what makes the payload requirement
            // apply, so the loser of the race is refused rather than allowed.
            None,
            T0,
        ),
        repo.remove_file(
            ORG,
            FileTarget {
                product: PRODUCT,
                file: SECOND,
            },
            None,
            T0,
        ),
    );
    let left = left.unwrap_or_else(|error| panic!("the first removal answers: {error}"));
    let right = right.unwrap_or_else(|error| panic!("the second removal answers: {error}"));

    let outcomes = [left, right];
    assert_eq!(
        outcomes.iter().filter(|one| one.is_ok()).count(),
        1,
        "exactly one removal lands; the other waits for the row and then sees one payload left"
    );
    assert!(
        outcomes.contains(&Err(FileRefusal::LastPayload)),
        "and the one that waited is refused by name rather than by the deferred trigger, \
         which would have surfaced as a fault instead of a sentence"
    );

    let record = repo
        .get(ORG, PRODUCT)
        .await
        .unwrap_or_else(|error| panic!("the product reads back: {error}"))
        .unwrap_or_else(|| panic!("the product still exists"));
    assert_eq!(
        record.product.payload_files().count(),
        1,
        "one payload file survives, which is the invariant the deferred trigger cannot hold alone"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn two_adds_racing_take_different_positions(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG, &two_payloads(), T0)
        .await
        .unwrap_or_else(|error| panic!("the product seeds: {error}"));

    // `product_file_position` is unique per product and is not partial, so two
    // adds that read `MAX(position)` at the same moment would collide on it.
    // The lock is what makes the read and the write one step.
    let third = held(FileId(Uuid([0x23; 16])), 0x5C);
    let fourth = held(FileId(Uuid([0x24; 16])), 0x5D);
    let (left, right) = tokio::join!(
        repo.add_file(ORG, PRODUCT, (&third, None), T0),
        repo.add_file(ORG, PRODUCT, (&fourth, None), T0),
    );
    left.unwrap_or_else(|error| panic!("the first add answers: {error}"))
        .unwrap_or_else(|refusal| panic!("the first add lands: {refusal:?}"));
    right
        .unwrap_or_else(|error| panic!("the second add answers: {error}"))
        .unwrap_or_else(|refusal| panic!("the second add lands: {refusal:?}"));

    let record = repo
        .get(ORG, PRODUCT)
        .await
        .unwrap_or_else(|error| panic!("the product reads back: {error}"))
        .unwrap_or_else(|| panic!("the product still exists"));
    assert_eq!(
        record.product.payload_files().count(),
        4,
        "both adds land, neither losing the position race with the other"
    );
}
