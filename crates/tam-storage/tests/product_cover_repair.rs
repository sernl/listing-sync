//! Giving a resource the thumbnail it never had, and never taking one away.
//!
//! The write under test is the repair leg of a re-import: a resource imported
//! before a cover could be derived has no `product_file` row in the cover
//! slot, nothing after the commit redraws one, and the console's file policy
//! refuses to let a seller supply one for a marketplace-sourced payload. So
//! the only way it can ever gain a thumbnail is a later read offering one —
//! and the only way a seller's own thumbnail can be lost is that offer
//! overwriting it. Both directions are asserted here against real rows.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration};
use tam_storage::{CoverOffer, ProductRepo};
use tam_types::{
    ConnectionId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, ListingCopy,
    OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome, Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const T1: Timestamp = Timestamp(1_756_000_100_000);
const ORG: OrgId = OrgId(Uuid([0xAB; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x11; 16]));
const ABSENT: ProductId = ProductId(Uuid([0x19; 16]));
const PAYLOAD: FileId = FileId(Uuid([0x31; 16]));
/// The marketplace login and the machine the sourced payload names.
///
/// Both are real rows, because `product_file_source_connection_fk` and
/// `product_file_observed_by_device_fk` say a claim about somebody's bytes
/// names the login it was read under and the machine that read it. A fixture
/// that invented either identifier would be asserting a file nobody could
/// have observed.
const CONNECTION: ConnectionId = ConnectionId(Uuid([0x41; 16]));
const DEVICE: &str = "11112222333344445555666677778888";

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
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-cover', now())")
        .bind(db(ORG.0))
        .execute(&mut *tx)
        .await
        .expect("the org inserts");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(db(ORG.0))
    .bind(db(CONNECTION.0))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    sqlx::query(
        "INSERT INTO device \
         (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'a test machine', 'linux', 'x86_64', '0.9.0', now(), now())",
    )
    .bind(db(ORG.0))
    .bind(DEVICE)
    .execute(&mut *tx)
    .await
    .expect("the device inserts");
    tx.commit().await.expect("the seed commits");
}

/// A payload the seller's own marketplace holds the bytes of, which is what
/// an imported resource carries: the server has a claim about these bytes and
/// not the bytes, so no server path can redraw a thumbnail from them.
fn sourced_payload() -> ProductFile {
    ProductFile {
        id: PAYLOAD,
        role: FileRole::Payload,
        kind: FileKind::Pdf,
        bytes: FileBytes::Sourced {
            marketplace: tam_types::Marketplace::Tes,
            connection: CONNECTION,
            resource: "https://www.tes.com/teaching-resource/-1".to_owned(),
            entry: None,
            payload_file_name: "worksheet.pdf".to_owned(),
            payload_content_type: "application/pdf".to_owned(),
            observed: tam_types::Observation {
                device: DEVICE.to_owned(),
                hash: ContentHash([0x5A; 32]),
                byte_len: 120_000,
                scan: ScanOutcome::Clean { at: T0 },
                observed_at: T0,
            },
        },
    }
}

fn cover(id: FileId, marker: u8) -> ProductFile {
    ProductFile {
        id,
        role: FileRole::Cover,
        kind: FileKind::Image,
        bytes: FileBytes::Held {
            hash: ContentHash([marker; 32]),
            byte_len: 4_096,
            scan: ScanOutcome::Clean { at: T0 },
        },
    }
}

/// An imported resource as it stands before the repair: a marketplace-sourced
/// payload and no thumbnail at all.
fn imported_without_a_cover() -> CanonicalProduct {
    CanonicalProduct {
        id: PRODUCT,
        org: ORG,
        title: Title("Fractions pack".to_owned()),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(sourced_payload(), vec![])),
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

/// One stand-in for "a picture nothing meant anything by", which is the class
/// of cover an automatic repair may exchange for a real one. The predicate is
/// the caller's everywhere, so the test states its own rather than importing
/// the renderer's: what is under test is the write's behaviour given an
/// answer, not which digests the renderer draws.
const REPLACEABLE: ContentHash = ContentHash([0xD0; 32]);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn offer(pool: &PgPool, product: ProductId, file: &ProductFile) -> CoverOffer {
    let mut tx = pool.begin().await.expect("tx begins");
    tam_storage::pin_tenant(&mut tx, ORG)
        .await
        .expect("the tenant pins");
    let answer =
        tam_storage::offer_cover(&mut tx, ORG, product, file, |held| held == REPLACEABLE, T1)
            .await
            .expect("the offer answers");
    tx.commit().await.expect("the offer commits");
    answer
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_resource_with_no_thumbnail_gains_the_one_a_later_read_offers(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG, &imported_without_a_cover(), T0)
        .await
        .unwrap_or_else(|error| panic!("the imported resource seeds: {error}"));
    assert!(
        repo.covers(ORG, &[PRODUCT])
            .await
            .unwrap_or_else(|error| panic!("the cover read answers: {error}"))
            .is_empty(),
        "the resource starts with no thumbnail, which is the state being repaired"
    );

    let offered = cover(FileId(Uuid([0x51; 16])), 0xC1);
    assert_eq!(
        offer(&pool, PRODUCT, &offered).await,
        CoverOffer::Written,
        "the offer is taken by a resource that had none"
    );

    let stored = repo
        .covers(ORG, &[PRODUCT])
        .await
        .unwrap_or_else(|error| panic!("the cover read answers: {error}"));
    assert_eq!(
        stored.len(),
        1,
        "the catalogue now serves exactly one thumbnail for this resource"
    );
    assert_eq!(
        stored[0].hash,
        ContentHash([0xC1; 32]),
        "and it names the bytes the read offered rather than any other picture"
    );

    // The repair is a thumbnail and nothing else. What the resource is, whose
    // bytes it is and where they live are untouched.
    let record = repo
        .get(ORG, PRODUCT)
        .await
        .unwrap_or_else(|error| panic!("the resource reads back: {error}"))
        .unwrap_or_else(|| panic!("the resource still exists"));
    assert_eq!(
        record.product.title.0, "Fractions pack",
        "the title the seller reads is not rewritten by a thumbnail repair"
    );
    let payloads: Vec<&ProductFile> = record.product.payload_files().collect();
    assert_eq!(payloads.len(), 1, "the payload is still the one payload");
    assert_eq!(
        payloads[0].id, PAYLOAD,
        "and it is the same file row, not a replacement"
    );
    assert!(
        matches!(payloads[0].bytes, FileBytes::Sourced { .. }),
        "the payload is still a claim about the seller's own marketplace bytes: a thumbnail \
         repair never turns one into bytes this server holds"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_thumbnail_already_held_is_never_overwritten(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    let mut seeded = imported_without_a_cover();
    // The seller's own thumbnail, as the console's upload path writes it.
    seeded.cover = Some(cover(FileId(Uuid([0x52; 16])), 0xB0));
    repo.insert(ORG, &seeded, T0)
        .await
        .unwrap_or_else(|error| panic!("the resource seeds: {error}"));

    let later = cover(FileId(Uuid([0x53; 16])), 0xC2);
    assert_eq!(
        offer(&pool, PRODUCT, &later).await,
        CoverOffer::AlreadyHeld,
        "a re-import offers its thumbnail and is told one is already held"
    );

    let stored = repo
        .covers(ORG, &[PRODUCT])
        .await
        .unwrap_or_else(|error| panic!("the cover read answers: {error}"));
    assert_eq!(stored.len(), 1, "still exactly one thumbnail");
    assert_eq!(
        stored[0].hash,
        ContentHash([0xB0; 32]),
        "and it is the seller's own, which every re-import leaves exactly where it was"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_thumbnail_the_caller_calls_replaceable_gives_way_to_the_picture(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    let mut seeded = imported_without_a_cover();
    // The state the live catalogue is in for every Tes bundle imported so
    // far: a thumbnail that is a picture of nothing, which the caller
    // recognises by digest.
    let mut card = cover(FileId(Uuid([0x55; 16])), 0x00);
    card.bytes = FileBytes::Held {
        hash: REPLACEABLE,
        byte_len: 4_768,
        scan: ScanOutcome::Clean { at: T0 },
    };
    seeded.cover = Some(card);
    repo.insert(ORG, &seeded, T0)
        .await
        .unwrap_or_else(|error| panic!("the resource seeds: {error}"));

    let picture = cover(FileId(Uuid([0x56; 16])), 0xC9);
    assert_eq!(
        offer(&pool, PRODUCT, &picture).await,
        CoverOffer::Replaced,
        "a stand-in thumbnail gives way to a picture of the resource"
    );
    let stored = repo
        .covers(ORG, &[PRODUCT])
        .await
        .unwrap_or_else(|error| panic!("the cover read answers: {error}"));
    assert_eq!(
        stored.len(),
        1,
        "exactly one live thumbnail: the old row is retired rather than left beside the new \
         one, which `product_file_one_cover` would refuse outright"
    );
    assert_eq!(
        stored[0].hash,
        ContentHash([0xC9; 32]),
        "and it is the picture"
    );

    // Offering the same picture again changes nothing, so a replayed repair
    // is not a second write.
    assert_eq!(
        offer(&pool, PRODUCT, &cover(FileId(Uuid([0x57; 16])), 0xC9)).await,
        CoverOffer::AlreadyHeld,
        "the same bytes twice is not a replacement"
    );
    assert_eq!(
        repo.covers(ORG, &[PRODUCT])
            .await
            .unwrap_or_else(|error| panic!("the cover read answers: {error}"))
            .len(),
        1,
        "still one live thumbnail"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_offer_to_a_resource_this_tenant_does_not_hold_writes_nothing(pool: PgPool) {
    seed(&pool).await;
    assert_eq!(
        offer(&pool, ABSENT, &cover(FileId(Uuid([0x54; 16])), 0xC3)).await,
        CoverOffer::NoProduct,
        "a thumbnail is never written against an identifier this tenant holds no resource for"
    );
    assert!(
        ProductRepo::new(pool.clone())
            .covers(ORG, &[ABSENT])
            .await
            .unwrap_or_else(|error| panic!("the cover read answers: {error}"))
            .is_empty(),
        "and no orphan file row is left behind"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_thumbnail_must_be_bytes_this_deployment_holds(pool: PgPool) {
    seed(&pool).await;
    ProductRepo::new(pool.clone())
        .insert(ORG, &imported_without_a_cover(), T0)
        .await
        .unwrap_or_else(|error| panic!("the resource seeds: {error}"));

    let mut sourced = sourced_payload();
    sourced.role = FileRole::Cover;
    let mut tx = pool
        .begin()
        .await
        .unwrap_or_else(|error| panic!("tx begins: {error}"));
    tam_storage::pin_tenant(&mut tx, ORG)
        .await
        .unwrap_or_else(|error| panic!("the tenant pins: {error}"));
    let refused = tam_storage::offer_cover(
        &mut tx,
        ORG,
        PRODUCT,
        &sourced,
        |held| held == REPLACEABLE,
        T1,
    )
    .await;
    assert!(
        refused.is_err(),
        "a thumbnail the server does not hold the bytes of is refused rather than recorded: \
         nothing could serve it and the publish gate could not see it"
    );
}
