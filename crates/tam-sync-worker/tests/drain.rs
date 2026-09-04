//! The drain's resumability, which is the whole reason `sync_request_resource`
//! carries a breadcrumb at all.
//!
//! `import_one` commits four times internally and mints a fresh product id on
//! every pass, so a request re-entered in `draining` skips the resources an
//! earlier pass finished. What the removal leg is built from is therefore the
//! question: the current pass's rows, or the request's own record of every
//! resource it has canonicalised.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    Mapping, PublishMode, RightsDeclaration,
};
use tam_marketplace::{
    AdapterError, FetchReason, FirstPartyExport, ImportedListing, ListingState, RemoteLifecycle,
    RemoteListingId,
};
use tam_secrets::Kek;
use tam_storage::{
    Canonicalised, Disposition, ItemsPageParams, JobReadRepo, MappingRepo, NewSyncRequest,
    ProductRepo, SyncIntent, SyncRequestRepo,
};
use tam_sync_worker::drain_request;
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, JobId,
    ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId,
    ScanOutcome, Timestamp, Title, Uuid,
};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const REQUEST: Uuid = Uuid([0x71; 16]);
const NOW: Timestamp = Timestamp(1_756_000_000_000);
const SOURCE: InventoryId = InventoryId::TesGb;
const TARGET: InventoryId = InventoryId::TesNz;

/// An adapter this test never reaches. Every resource is already
/// canonicalised, so the drain takes the skip branch for all of them and the
/// removal leg is built entirely from breadcrumbs — which is the point: built
/// from the current pass's work it would be empty.
struct NeverRead;

impl FirstPartyExport for NeverRead {
    type CatalogueEntry = ();
    type Resource = i64;

    fn list_own_resources(
        &self,
        _reason: &FetchReason,
    ) -> impl core::future::Future<Output = Result<Vec<()>, AdapterError>> + Send {
        core::future::ready(Err(AdapterError::SessionExpired))
    }

    fn download_resource_bundle(
        &self,
        _reason: &FetchReason,
        _id: i64,
    ) -> impl core::future::Future<Output = Result<Vec<u8>, AdapterError>> + Send {
        core::future::ready(Err(AdapterError::SessionExpired))
    }

    fn fetch_for_import(
        &self,
        _reason: &FetchReason,
        _id: i64,
    ) -> impl core::future::Future<Output = Result<ImportedListing, AdapterError>> + Send {
        core::future::ready(Err(AdapterError::SessionExpired))
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool, tag: u8) -> (ProductId, MappingId) {
    let product = ProductId(Uuid([tag; 16]));
    let mapping = MappingId(Uuid([tag.wrapping_add(0x30); 16]));
    ProductRepo::new(pool.clone())
        .insert(
            ORG,
            &CanonicalProduct {
                id: product,
                org: ORG,
                title: Title("Fractions practice".to_owned()),
                body: ListingCopy {
                    body: "A worksheet.".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: PayloadSet::new(
                    ProductFile {
                        id: FileId(Uuid([tag.wrapping_add(0x60); 16])),
                        role: FileRole::Payload,
                        kind: FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: ContentHash([tag; 32]),
                            byte_len: 4,
                            scan: ScanOutcome::Clean { at: NOW },
                        },
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
            },
            NOW,
        )
        .await
        .expect("the product inserts");
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &Mapping {
                id: mapping,
                org: ORG,
                product,
                inventory: TARGET,
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
            NOW,
        )
        .await
        .expect("the mapping inserts");
    (product, mapping)
}

fn tes(resource: i64) -> RemoteListingId {
    RemoteListingId::Tes {
        url: format!("https://www.tes.com/teaching-resource/fixture-{resource}"),
    }
}

/// A migrate resumed after its canonicalisation finished still removes every
/// source.
///
/// The removal leg used to be built from the resources this pass canonicalised
/// rather than from the request's own record, so a resumed drain minted a
/// removal job holding whatever it happened to redo — nothing, here — and then
/// marked the request enqueued naming it. `create_with_request_key` accepts an
/// empty item slice, and an itemless job reads back settled because zero
/// settled of zero is complete, so the request reported the migration done
/// while both source listings stayed up.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_resumed_migrate_removes_every_source_it_canonicalised(pool: PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(&pool)
        .await
        .expect("the org seeds");
    let requests = SyncRequestRepo::new(pool.clone());
    requests
        .create(
            ORG,
            &NewSyncRequest {
                id: REQUEST,
                source: SOURCE,
                target: TARGET,
                disposition: Disposition::Migrate,
                intent: SyncIntent::Draft,
                requested_at: NOW,
                locators: vec!["101".to_owned(), "202".to_owned()],
            },
        )
        .await
        .expect("the request writes");

    // The earlier pass: both resources canonicalised, and the process died
    // before it minted either job. This is the state `pending` re-picks.
    for (ordinal, tag, resource) in [(0, 0x01_u8, 101_i64), (1, 0x02, 202)] {
        let (product, mapping) = seed(&pool, tag).await;
        requests
            .record_canonicalised(
                ORG,
                &Canonicalised {
                    request: REQUEST,
                    ordinal,
                    product,
                    mapping,
                    source: tes(resource),
                    source_state: Some(ListingState::Live),
                },
            )
            .await
            .expect("the breadcrumb writes");
    }

    let adapter = NeverRead;
    let run = tam_import::ImportRun {
        pool: pool.clone(),
        kek: Kek::from_bytes(&[0x11; 32]).expect("a well-formed kek"),
        store_root: std::path::PathBuf::from("/nonexistent"),
        adapter: &adapter,
        org: ORG,
        source: SOURCE,
        target: TARGET,
        now: NOW,
    };
    let report = drain_request(&requests, &run, REQUEST)
        .await
        .expect("the resumed drain runs without reading the source again");
    assert_eq!(
        (report.skipped, report.failed),
        (2, 0),
        "the whole request was already canonicalised, which is exactly the resume case"
    );
    let remove_job = report.remove_job.expect("a migrate mints a removal job");

    let items = JobReadRepo::new(pool.clone())
        .items_page(
            ORG,
            JobId(remove_job),
            ItemsPageParams {
                cursor: None,
                limit: 10,
                outcome: None,
            },
        )
        .await
        .expect("the removal job's items read");
    assert_eq!(
        items.len(),
        2,
        "one removal per canonicalised resource, not per resource this pass redid"
    );

    let mappings = MappingRepo::new(pool.clone());
    let mut removed = Vec::new();
    for item in &items {
        let record = mappings
            .get(ORG, item.mapping)
            .await
            .expect("the removal's source mapping reads")
            .expect("it exists");
        assert_eq!(record.mapping.inventory, SOURCE);
        let Binding::Bound { id, .. } = record.mapping.binding else {
            panic!("the source mapping is bound from the read that observed it");
        };
        removed.push(id);
    }
    removed.sort_by_key(|id| format!("{id:?}"));
    assert_eq!(
        removed,
        vec![tes(101), tes(202)],
        "the removal job carries every source the request canonicalised"
    );
}

/// A sync asking for a live listing enqueues the publish the intent means.
///
/// The drain used to mint `ItemOperation::Create` whatever the request said.
/// Both adapters create a draft, so a seller who asked for `live` got one:
/// the request settled `enqueued`, the job settled succeeded, and nothing
/// anywhere said the listing was not live. The lowering is the API's own, so
/// the two enqueue paths agree on what `live` means.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_live_sync_enqueues_the_create_and_the_publish_it_gates(pool: PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(&pool)
        .await
        .expect("the org seeds");
    let requests = SyncRequestRepo::new(pool.clone());
    requests
        .create(
            ORG,
            &NewSyncRequest {
                id: REQUEST,
                source: SOURCE,
                target: TARGET,
                disposition: Disposition::Sync,
                intent: SyncIntent::Live,
                requested_at: NOW,
                locators: vec!["303".to_owned()],
            },
        )
        .await
        .expect("the request writes");
    let (product, mapping) = seed(&pool, 0x03).await;
    requests
        .record_canonicalised(
            ORG,
            &Canonicalised {
                request: REQUEST,
                ordinal: 0,
                product,
                mapping,
                source: tes(303),
                source_state: Some(ListingState::Live),
            },
        )
        .await
        .expect("the breadcrumb writes");

    let adapter = NeverRead;
    let run = tam_import::ImportRun {
        pool: pool.clone(),
        kek: Kek::from_bytes(&[0x11; 32]).expect("a well-formed kek"),
        store_root: std::path::PathBuf::from("/nonexistent"),
        adapter: &adapter,
        org: ORG,
        source: SOURCE,
        target: TARGET,
        now: NOW,
    };
    let report = drain_request(&requests, &run, REQUEST)
        .await
        .expect("the drain runs");
    let create_job = report.create_job.expect("a sync mints a write job");

    // Pinned, because `job_item` carries FORCE ROW LEVEL SECURITY and the
    // test pool is `tam_app`: an unpinned read is a clean empty one.
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT operation, requires_bound_on FROM job_item \
         WHERE org_id = $1 AND job_id = $2 ORDER BY operation",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(create_job.0))
    .fetch_all(&mut *tx)
    .await
    .expect("the job's items read");
    assert_eq!(
        rows,
        vec![
            ("create".to_owned(), None),
            ("publish".to_owned(), Some("tes_nz".to_owned())),
        ],
        "live is a create and a publish, and the publish waits for the create's own binding"
    );
}
