//! A product file whose bytes are a marketplace resource rather than a blob.
//!
//! Two things are asserted here and they fail differently. That
//! `describe_files` answers for a file with no blob is the fork D27 needs: the
//! join used to be an inner one, so a sourced file was not merely described
//! without a length, it was dropped from the answer entirely and reached the
//! manifest builder as nothing at all. And that the source group is invisible
//! across tenants is the ordinary tenancy guarantee applied to a table that
//! now carries a seller's marketplace locator, which is a new kind of thing
//! for `product_file` to hold.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration};
use tam_marketplace::{FileSource, FileSourceError};
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    describe_files, BlobRepo, PipelineFileSource, ProductFileSourceRepo, ProductRepo,
};
use tam_types::{
    ConnectionId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, ListingCopy,
    Marketplace, Observation, OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x10; 16]));
const FILE: FileId = FileId(Uuid([0x21; 16]));
const CONNECTION: ConnectionId = ConnectionId(Uuid([0xC1; 16]));
const OBSERVED: ContentHash = ContentHash([0x5A; 32]);
const DIFFERENT: ContentHash = ContentHash([0x5B; 32]);

fn db(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool, org: OrgId, device: &str) {
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(db(org.0))
        .bind(format!("org-{device}"))
        .execute(&mut *tx)
        .await
        .expect("the org inserts");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(db(org.0))
    .bind(db(CONNECTION.0))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    sqlx::query(
        "INSERT INTO device \
         (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'laptop', 'linux', 'x86_64', '0.2.0', now(), now())",
    )
    .bind(db(org.0))
    .bind(device)
    .execute(&mut *tx)
    .await
    .expect("the device inserts");
    tx.commit().await.expect("the seed commits");
}

/// A product whose payload is the seller's marketplace resource.
///
/// Built and written through `ProductRepo::insert` rather than through a
/// file-only path, because that is the only way it can be written at all: a
/// product and its first payload must land in one transaction, which
/// `product_payload_nonempty` enforces as a deferred constraint trigger, and
/// no separate write path can compose with that.
fn sourced_product(device: &str, hash: ContentHash) -> CanonicalProduct {
    CanonicalProduct {
        id: PRODUCT,
        org: ORG_A,
        title: Title("A migrated worksheet".to_owned()),
        body: ListingCopy {
            body: "Imported from the seller's Tes catalogue.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FILE,
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Sourced {
                    marketplace: Marketplace::Tes,
                    connection: CONNECTION,
                    resource: "13549794".to_owned(),
                    entry: Some("worksheet.pdf".to_owned()),
                    payload_file_name: "worksheet.pdf".to_owned(),
                    payload_content_type: "application/pdf".to_owned(),
                    observed: observation(device, hash),
                },
            },
            vec![],
        )),
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

fn observation(device: &str, hash: ContentHash) -> Observation {
    Observation {
        device: device.to_owned(),
        hash,
        byte_len: 493_000,
        scan: ScanOutcome::Clean { at: T0 },
        observed_at: T0,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn describe_files_answers_for_a_file_with_no_blob(pool: PgPool) {
    seed(&pool, ORG_A, "device-a").await;
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &sourced_product("device-a", OBSERVED), T0)
        .await
        .expect("a product whose payload is sourced inserts");

    let described = describe_files(&pool, ORG_A, &[FILE])
        .await
        .expect("the description runs");
    let [file] = described.as_slice() else {
        panic!("a sourced file must be described, not skipped: {described:?}");
    };
    assert_eq!(
        (file.file_name.as_str(), file.content_type.as_str()),
        ("worksheet.pdf", "application/pdf"),
        "a sourced file's name and type come from the entry the producer recorded, not from a \
         digest we do not have"
    );
    assert_eq!(
        file.bytes,
        FileBytes::Sourced {
            marketplace: Marketplace::Tes,
            connection: CONNECTION,
            resource: "13549794".to_owned(),
            entry: Some("worksheet.pdf".to_owned()),
            payload_file_name: "worksheet.pdf".to_owned(),
            payload_content_type: "application/pdf".to_owned(),
            observed: observation("device-a", OBSERVED),
        },
        "the arm carries the locator and the device's assertion, and never presents it as a \
         commitment of ours"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn one_tenants_source_locator_is_invisible_to_another(pool: PgPool) {
    seed(&pool, ORG_A, "device-a").await;
    seed(&pool, ORG_B, "device-b").await;
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &sourced_product("device-a", OBSERVED), T0)
        .await
        .expect("the first tenant's sourced product inserts");

    let described = describe_files(&pool, ORG_B, &[FILE])
        .await
        .expect("the description runs for the second tenant");
    assert!(
        described.is_empty(),
        "the second tenant must not see the first tenant's marketplace locator: {described:?}"
    );
    assert!(
        !ProductFileSourceRepo::new(pool.clone())
            .disputed(ORG_B, FILE)
            .await
            .expect("the dispute read runs"),
        "nor its observation history"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_device_disagreeing_is_recorded_rather_than_overwriting(pool: PgPool) {
    seed(&pool, ORG_A, "device-a").await;
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &sourced_product("device-a", OBSERVED), T0)
        .await
        .expect("a product whose payload is sourced inserts");
    let repo = ProductFileSourceRepo::new(pool.clone());
    assert!(
        !repo.disputed(ORG_A, FILE).await.expect("the read runs"),
        "one observation is not a disagreement"
    );

    repo.observe(
        ORG_A,
        FILE,
        &Observation {
            observed_at: Timestamp(T0.0 + 1),
            ..observation("device-a", OBSERVED)
        },
        Timestamp(T0.0 + 1),
    )
    .await
    .expect("a repeat observation records");
    assert!(
        !repo.disputed(ORG_A, FILE).await.expect("the read runs"),
        "the same digest twice is agreement however many rows it took"
    );

    repo.observe(
        ORG_A,
        FILE,
        &Observation {
            byte_len: 493_001,
            observed_at: Timestamp(T0.0 + 2),
            ..observation("device-a", DIFFERENT)
        },
        Timestamp(T0.0 + 2),
    )
    .await
    .expect("a disagreeing observation is recorded rather than refused");
    assert!(
        repo.disputed(ORG_A, FILE).await.expect("the read runs"),
        "two distinct digests are a disagreement the item view has to surface"
    );

    let described = describe_files(&pool, ORG_A, &[FILE])
        .await
        .expect("the description runs");
    let [file] = described.as_slice() else {
        panic!("the file is still described")
    };
    let FileBytes::Sourced { observed, .. } = &file.bytes else {
        panic!("still a sourced file")
    };
    assert_eq!(
        observed.hash, OBSERVED,
        "the first observation stands as what the file says about itself; a later one that \
         disagrees is surfaced, never applied"
    );
}

/// The server-side half of D27's fence, and the mirror of the device-side
/// assertion that no blob row is written for a sourced file.
///
/// `Missing` would be the tempting answer and it is the wrong one: it invites
/// the caller to treat the file as absent and carry on, when what has happened
/// is that a server-side path reached for bytes D27 says the server must never
/// hold. The refusal has to name the boundary, so an operator meeting it in a
/// log reads an architectural fact rather than a data problem.
#[sqlx::test(migrations = "./migrations")]
async fn the_servers_file_source_refuses_a_marketplace_sourced_file(pool: PgPool) {
    seed(&pool, ORG_A, "device-a").await;
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &sourced_product("device-a", OBSERVED), T0)
        .await
        .expect("a product whose payload is sourced inserts");

    let store = LocalObjectStore::new(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("file-source-refusal"),
    );
    let repo = BlobRepo::new(
        pool.clone(),
        store,
        Kek::from_bytes(&[0x11; 32]).expect("a well-formed kek"),
    );
    let source = PipelineFileSource::new(repo, ORG_A, pool);

    match source.fetch(FILE).await {
        Err(FileSourceError::Unreadable { file, detail }) => {
            assert_eq!(file, FILE, "the refusal names the file asked for");
            assert!(
                detail.contains("marketplace resource") && detail.contains("never"),
                "the refusal names the boundary rather than reading as a missing file: {detail}"
            );
        }
        Err(FileSourceError::Missing(_)) => panic!(
            "a sourced file is not missing: its bytes are the seller's, and answering Missing \
             invites a caller to carry on as though the file were gone"
        ),
        other => panic!("the server must not resolve bytes it never held: {other:?}"),
    }
}

/// The exactly-one constraint refuses a row that is neither, and one that is
/// both.
///
/// `product_file_blob_or_source` is the whole of the invariant this design
/// rests on — a file is blob-backed or marketplace-sourced, never neither and
/// never both — and until this test it was the one thing in the migration
/// nothing exercised. Written as raw SQL on purpose: no repository can build
/// either row, so the only way to ask whether the database refuses them is to
/// try.
#[sqlx::test(migrations = "./migrations")]
async fn a_row_that_is_neither_blob_backed_nor_sourced_is_refused(pool: PgPool) {
    seed(&pool, ORG_A, "device-a").await;
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &sourced_product("device-a", OBSERVED), T0)
        .await
        .expect("the product inserts, so the trigger is satisfied for both attempts below");

    // Neither: no hash, and no source group either.
    let neither = insert_raw(
        &pool,
        FileId(Uuid([0x22; 16])),
        "NULL, NULL, NULL, NULL, NULL, NULL",
    )
    .await
    .expect_err("a file with no bytes anywhere must be refused");
    assert_constraint(&neither, "product_file_blob_or_source");

    // Both: a hash and a source group at once, which would let one row claim
    // the server verified bytes it never held.
    let both = insert_raw(
        &pool,
        FileId(Uuid([0x23; 16])),
        &format!(
            "decode(repeat('77', 32), 'hex'), 'tes', '{}'::uuid, '13549794', \
             decode(repeat('5a', 32), 'hex'), 493000",
            db(CONNECTION.0)
        ),
    )
    .await
    .expect_err("a file claiming both a blob and a marketplace source must be refused");
    assert_constraint(&both, "product_file_blob_or_source");
}

/// Inserts a `product_file` row whose bytes-columns are written verbatim.
///
/// The six values are SQL text rather than bind parameters because the two
/// cases differ in how many placeholders they would need and in their types,
/// and a mismatched bind position fails as a type error that looks exactly
/// like the constraint refusing — which is what it did on the first run of
/// this test. Only the three ids are bound, and they are the same three in
/// both cases.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn insert_raw(pool: &PgPool, file: FileId, values: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db(ORG_A.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let statement = format!(
        "INSERT INTO product_file \
         (org_id, id, product_id, position, role, kind, created_at, \
          hash, source_marketplace, source_connection, source_resource, \
          observed_hash, observed_byte_len) \
         VALUES ($1, $2, $3, 9, 'payload', 'pdf', now(), {values})"
    );
    sqlx::query(&statement)
        .bind(db(ORG_A.0))
        .bind(db(file.0))
        .bind(db(PRODUCT.0))
        .execute(&mut *tx)
        .await
        .map(drop)
}

#[expect(
    clippy::panic,
    reason = "allow-panic-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a refusal from the client rather than the database is a broken fixture"
)]
fn assert_constraint(error: &sqlx::Error, expected: &str) {
    let sqlx::Error::Database(refusal) = error else {
        panic!("the refusal must come from Postgres, not the client: {error:?}");
    };
    assert_eq!(
        refusal.constraint(),
        Some(expected),
        "refused, but by the wrong constraint, which would mean the invariant is being held \
         by something other than the one that states it: {refusal:?}"
    );
}
