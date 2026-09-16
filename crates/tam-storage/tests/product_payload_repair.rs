//! Giving a resource the bytes a later read captured, and never taking a
//! seller's own away.
//!
//! The write under test is the payload leg of a re-import, the mirror of
//! `product_cover_repair`. Two directions matter and both are asserted here
//! against real rows. A resource whose bytes were never captured, or whose
//! captured commitment has gone stale under it, has to end the repair holding
//! the bytes the read actually saw — otherwise the manifest a device receives
//! names a download nothing can verify. And a repeated read of an unchanged
//! resource has to write nothing at all, because a payload offer that appended
//! every time would turn an idempotent import into a growing pile of
//! identical files.
//!
//! The deferred half is the reason this needs a live database: a mapping onto
//! the product means `mapping_payload_nonempty` re-runs at commit, so a
//! refresh that retired the old row and wrote the new one in two transactions
//! — or in the wrong order within one — loses the whole page rather than the
//! one row.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    Mapping, PublishMode, RightsDeclaration,
};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{MappingRepo, PayloadOffer, PayloadOfferPolicy, ProductRepo};
use tam_types::{
    ConnectionId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, MappingId, Marketplace, Observation, OrgId, PayloadSet, PriceIntent, PriceRule,
    ProductFile, ProductId, ScanOutcome, Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const T1: Timestamp = Timestamp(1_756_000_100_000);
const ORG: OrgId = OrgId(Uuid([0xAC; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x12; 16]));
const ABSENT: ProductId = ProductId(Uuid([0x1A; 16]));
/// The payload row an import left behind, and the row a later read captures.
/// Two identifiers because a capture is a new observation of the resource
/// rather than an edit of the old row, and `product_file` is append-plus-retire.
const OLD_PAYLOAD: FileId = FileId(Uuid([0x32; 16]));
const NEW_PAYLOAD: FileId = FileId(Uuid([0x33; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x34; 16]));
/// The marketplace login and the machine a sourced payload names, both real
/// rows because `product_file_source_connection_fk` and
/// `product_file_observed_by_device_fk` say a claim about somebody's bytes
/// names the login it was read under and the machine that read it.
const CONNECTION: ConnectionId = ConnectionId(Uuid([0x42; 16]));
const DEVICE: &str = "11112222333344445555666677778888";
/// The one resource these reads are of. Identity is this locator plus the
/// entry within it, which is what makes a second read of the same listing a
/// refresh rather than a different file.
const RESOURCE: &str = "https://www.tes.com/teaching-resource/-1";
const OTHER_RESOURCE: &str = "https://www.tes.com/teaching-resource/-2";

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
    sqlx::query(
        "INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-payload', now())",
    )
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

/// One read's claim about a marketplace resource's bytes: which resource, and
/// what the device saw when it fetched it.
fn capture(id: FileId, resource: &str, marker: u8, byte_len: u64, at: Timestamp) -> ProductFile {
    ProductFile {
        id,
        role: FileRole::Payload,
        kind: FileKind::Pdf,
        bytes: FileBytes::Sourced {
            marketplace: Marketplace::Tes,
            connection: CONNECTION,
            resource: resource.to_owned(),
            entry: None,
            payload_file_name: "worksheet.pdf".to_owned(),
            payload_content_type: "application/pdf".to_owned(),
            observed: Observation {
                device: DEVICE.to_owned(),
                hash: ContentHash([marker; 32]),
                byte_len,
                scan: ScanOutcome::Clean { at },
                observed_at: at,
            },
        },
    }
}

/// The seller's own file: bytes this deployment holds, which no automatic
/// write may exchange for a marketplace claim.
fn uploaded(id: FileId, marker: u8) -> ProductFile {
    ProductFile {
        id,
        role: FileRole::Payload,
        kind: FileKind::Pdf,
        bytes: FileBytes::Held {
            hash: ContentHash([marker; 32]),
            byte_len: 8_192,
            scan: ScanOutcome::Clean { at: T0 },
        },
    }
}

fn resource_with(payload: Option<PayloadSet>) -> CanonicalProduct {
    CanonicalProduct {
        id: PRODUCT,
        org: ORG,
        title: Title("Fractions pack".to_owned()),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload,
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

/// The marketplace that carries the resource, so the deferred payload trigger
/// has something to judge: 0061 makes the requirement a property of the
/// mapping, and a refresh that left the product momentarily fileless at
/// commit would lose the transaction here rather than in a later run.
fn carrying_mapping() -> Mapping {
    Mapping {
        id: MAPPING,
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
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn offer(
    pool: &PgPool,
    product: ProductId,
    file: &ProductFile,
    policy: PayloadOfferPolicy,
) -> PayloadOffer {
    let mut tx = pool.begin().await.expect("tx begins");
    tam_storage::pin_tenant(&mut tx, ORG)
        .await
        .expect("the tenant pins");
    let answer = tam_storage::offer_payload(&mut tx, ORG, product, file, None, policy, T1)
        .await
        .expect("the offer answers");
    // The commit is where the deferred payload trigger runs, so an offer that
    // left the resource fileless fails here rather than silently.
    tx.commit().await.expect("the offer commits");
    answer
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn live_payloads(repo: &ProductRepo) -> Vec<ProductFile> {
    repo.get(ORG, PRODUCT)
        .await
        .expect("the resource reads back")
        .expect("the resource still exists")
        .product
        .payload_files()
        .cloned()
        .collect()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_resource_with_no_bytes_takes_the_ones_a_later_read_captured(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG, &resource_with(None), T0)
        .await
        .unwrap_or_else(|error| panic!("the fileless resource seeds: {error}"));

    assert_eq!(
        offer(
            &pool,
            PRODUCT,
            &capture(NEW_PAYLOAD, RESOURCE, 0x5B, 120_000, T1),
            PayloadOfferPolicy::FillMissing,
        )
        .await,
        PayloadOffer::Written,
        "a resource that held no payload takes the captured one, which is what makes its \
         listing claim legal at all"
    );

    let files = live_payloads(&repo).await;
    assert_eq!(files.len(), 1, "one capture, one payload row");
    assert_eq!(files[0].id, NEW_PAYLOAD);
}

/// The idempotence the import depends on: a read that saw exactly what the
/// catalogue already records writes nothing, and says so as itself rather than
/// as a refusal, because the caller records a fingerprint only for a payload
/// that really matches the capture.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_repeated_read_of_unchanged_bytes_accumulates_no_files(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(
        ORG,
        &resource_with(Some(PayloadSet::new(
            capture(OLD_PAYLOAD, RESOURCE, 0x5A, 120_000, T0),
            vec![],
        ))),
        T0,
    )
    .await
    .unwrap_or_else(|error| panic!("the imported resource seeds: {error}"));

    for policy in [
        PayloadOfferPolicy::FillMissing,
        PayloadOfferPolicy::RestoreSource,
    ] {
        assert_eq!(
            offer(
                &pool,
                PRODUCT,
                &capture(NEW_PAYLOAD, RESOURCE, 0x5A, 120_000, T1),
                policy,
            )
            .await,
            PayloadOffer::Unchanged,
            "the same resource, the same bytes, the same length: nothing to write under \
             {policy:?}"
        );
    }

    let files = live_payloads(&repo).await;
    assert_eq!(
        files.len(),
        1,
        "and no second copy of the same file, which is the difference between a repeatable \
         import and a growing pile"
    );
    assert_eq!(
        files[0].id, OLD_PAYLOAD,
        "the row already there is the row that stays"
    );
}

/// The restore this repair exists for: the marketplace bundle changed while
/// the resource sat deleted, so the retained commitment names bytes no device
/// can still fetch. The capture that just proved fetchable has to become the
/// payload, under the same mapping and without a moment where the resource
/// has none.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_stale_sourced_commitment_gives_way_to_the_fresh_capture(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(
        ORG,
        &resource_with(Some(PayloadSet::new(
            capture(OLD_PAYLOAD, RESOURCE, 0x5A, 120_000, T0),
            vec![],
        ))),
        T0,
    )
    .await
    .unwrap_or_else(|error| panic!("the imported resource seeds: {error}"));
    MappingRepo::new(pool.clone())
        .add(ORG, &carrying_mapping(), 0, T0)
        .await
        .unwrap_or_else(|error| panic!("the marketplace carries the resource: {error}"));

    let fresh = capture(NEW_PAYLOAD, RESOURCE, 0x6B, 130_000, T1);
    assert_eq!(
        offer(&pool, PRODUCT, &fresh, PayloadOfferPolicy::RestoreSource).await,
        PayloadOffer::Written,
        "the same resource read again, with different bytes: the restore takes them"
    );

    let files = live_payloads(&repo).await;
    assert_eq!(
        files.len(),
        1,
        "the refreshed resource holds one payload, not the stale row beside the fresh one"
    );
    assert_eq!(
        files[0].id, NEW_PAYLOAD,
        "and it is the capture's own row: the old commitment is retired rather than edited"
    );
    let FileBytes::Sourced { observed, .. } = &files[0].bytes else {
        panic!("a restored marketplace payload is still a claim about the seller's own bytes");
    };
    assert_eq!(
        (observed.hash, observed.byte_len),
        (ContentHash([0x6B; 32]), 130_000),
        "the commitment a manifest sends is the one the device just verified, not the one it \
         could no longer fetch"
    );
}

/// The same staleness, offered by the path that may not touch anything: a
/// plain fill is for a resource with no payload, and it stays that way, so a
/// re-import of an unchanged live resource can never rewrite its file.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_fill_leaves_even_a_stale_commitment_alone(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(
        ORG,
        &resource_with(Some(PayloadSet::new(
            capture(OLD_PAYLOAD, RESOURCE, 0x5A, 120_000, T0),
            vec![],
        ))),
        T0,
    )
    .await
    .unwrap_or_else(|error| panic!("the imported resource seeds: {error}"));

    assert_eq!(
        offer(
            &pool,
            PRODUCT,
            &capture(NEW_PAYLOAD, RESOURCE, 0x6B, 130_000, T1),
            PayloadOfferPolicy::FillMissing,
        )
        .await,
        PayloadOffer::AlreadyHeld,
        "a fill is not a replacement, whatever the read saw"
    );

    let files = live_payloads(&repo).await;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, OLD_PAYLOAD, "the existing row is untouched");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_restore_never_takes_the_sellers_own_file(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(
        ORG,
        &resource_with(Some(PayloadSet::new(uploaded(OLD_PAYLOAD, 0x7C), vec![]))),
        T0,
    )
    .await
    .unwrap_or_else(|error| panic!("the resource with the seller's own file seeds: {error}"));

    assert_eq!(
        offer(
            &pool,
            PRODUCT,
            &capture(NEW_PAYLOAD, RESOURCE, 0x6B, 130_000, T1),
            PayloadOfferPolicy::RestoreSource,
        )
        .await,
        PayloadOffer::AlreadyHeld,
        "bytes this deployment holds are the seller's, and a marketplace read is not \
         permission to exchange them for a claim"
    );

    let files = live_payloads(&repo).await;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, OLD_PAYLOAD);
    assert!(
        matches!(files[0].bytes, FileBytes::Held { .. }),
        "and it is still the file the seller uploaded"
    );
}

/// A restore is scoped to the resource it re-read. Another marketplace
/// resource's file on the same product is not stale just because this read
/// happened, and replacing it would lose a file nothing here has seen.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_restore_never_takes_an_unrelated_source_file(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(
        ORG,
        &resource_with(Some(PayloadSet::new(
            capture(OLD_PAYLOAD, OTHER_RESOURCE, 0x5A, 120_000, T0),
            vec![],
        ))),
        T0,
    )
    .await
    .unwrap_or_else(|error| panic!("the imported resource seeds: {error}"));

    assert_eq!(
        offer(
            &pool,
            PRODUCT,
            &capture(NEW_PAYLOAD, RESOURCE, 0x6B, 130_000, T1),
            PayloadOfferPolicy::RestoreSource,
        )
        .await,
        PayloadOffer::AlreadyHeld,
        "a different resource's commitment is somebody else's file as far as this read is \
         concerned"
    );

    let files = live_payloads(&repo).await;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, OLD_PAYLOAD);
}

/// Two live payloads are a resource nothing automatic can reason about: which
/// of them the read refreshed is not decidable from one capture, so both are
/// kept.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_restore_onto_a_multi_file_resource_keeps_every_file(pool: PgPool) {
    seed(&pool).await;
    let repo = ProductRepo::new(pool.clone());
    repo.insert(
        ORG,
        &resource_with(Some(PayloadSet::new(
            capture(OLD_PAYLOAD, RESOURCE, 0x5A, 120_000, T0),
            vec![uploaded(FileId(Uuid([0x35; 16])), 0x7C)],
        ))),
        T0,
    )
    .await
    .unwrap_or_else(|error| panic!("the two-file resource seeds: {error}"));

    assert_eq!(
        offer(
            &pool,
            PRODUCT,
            &capture(NEW_PAYLOAD, RESOURCE, 0x6B, 130_000, T1),
            PayloadOfferPolicy::RestoreSource,
        )
        .await,
        PayloadOffer::AlreadyHeld,
        "a refresh that guessed which of two files it replaced could lose the other"
    );
    assert_eq!(live_payloads(&repo).await.len(), 2);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_offer_to_a_resource_this_tenant_does_not_hold_writes_nothing(pool: PgPool) {
    seed(&pool).await;
    assert_eq!(
        offer(
            &pool,
            ABSENT,
            &capture(NEW_PAYLOAD, RESOURCE, 0x6B, 130_000, T1),
            PayloadOfferPolicy::RestoreSource,
        )
        .await,
        PayloadOffer::NoProduct,
        "no live resource of that identifier, so nothing to offer it"
    );
}
