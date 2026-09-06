//! The import end to end over a cassette-shaped draft read: files through
//! the real pipeline into real blobs, taxonomy inbound over a seeded
//! crosswalk, grades verbatim with a derived interval, the target mapping
//! created, and the drain report raising exactly the gaps.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::equivalence::{NewProjectionOverride, OverrideKind, ProjectionOverride};
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, NoCounterpart, ProjectionEdge, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_import::{
    import_one, measure_one, record_drain_report, AppliedResource, DrainTotals, HeldFile,
    ImportRun, ImportedFile, MeasureTotals,
};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{HttpRequest, HttpResponse};
use tam_marketplace::{FetchReason, ListingState, RemoteListingId};
use tam_marketplace_tes::{endpoints as tes, DraftId, TesAdapter};
use tam_pipeline::archive::ExtractBudget;
use tam_pipeline::pipeline::{ingest, ArchiveMode, IngestContext};
use tam_pipeline::scan::EicarScanner;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{BlobRepo, JobReadRepo, OverrideRepo, ProductRepo, TaxonomyRepo, TenantBlobSink};
use tam_taxonomy::licences::derive_licence_crosswalk;
use tam_types::{
    CanonicalTermId, ConnectionId, ContentHash, FileBytes, FileKind, InventoryId, Marketplace,
    Observation, OrgId, PriceIntent, ScanOutcome, Timestamp, UserId, Uuid,
};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
/// A device identifier in the shape the registry stores one.
const DEVICE: &str = "11112222333344445555666677778888";
/// A second tenant on the same global relation, so a test about one seller's
/// decision can show it is one seller's.
const OTHER_ORG: OrgId = OrgId(Uuid([0xAB; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const TOPIC: CanonicalTermId = CanonicalTermId(Uuid([0x78; 16]));
/// The resource type the fixture's `mainType: 1` names.
const RESOURCE_TYPE: CanonicalTermId = CanonicalTermId(Uuid([0x79; 16]));
const NOW: Timestamp = Timestamp(1_000);

fn draft_body(resource: i64) -> String {
    draft_body_licensed(resource, "CC-BY", serde_json::Value::Null)
}

fn draft_body_licensed(resource: i64, licence: &str, price: serde_json::Value) -> String {
    let mut body = serde_json::json!({
        "id": resource,
        "title": "Fractions practice",
        "descriptionRaw": "A worksheet.",
        "descriptionRawType": "md",
        "licence": licence,
        "categories": [{ "id": 1_000_454 }, { "id": 1_000_732 }],
        "ageRanges": [2],
        "yearGroups": ["year-2"],
        "curriculum": "English",
        "mainAge": 6,
        "mainType": 1,
        "ages": [5, 6, 7]
    });
    if !price.is_null() {
        body["price"] = price;
    }
    body.to_string()
}

fn adapter_for(resource: i64) -> TesAdapter<CassetteTransport, NoImportFiles> {
    adapter_licensed(resource, "CC-BY", serde_json::Value::Null)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn adapter_licensed(
    resource: i64,
    licence: &str,
    price: serde_json::Value,
) -> TesAdapter<CassetteTransport, NoImportFiles> {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: HttpRequest::get(format!(
                "https://www.tes.com/api/v2/resources/{resource}/draft"
            )),
            response: HttpResponse::plain(
                200,
                draft_body_licensed(resource, licence, price).into_bytes(),
            ),
        }],
    };
    TesAdapter::new(
        InventoryId::TesGb,
        CassetteTransport::new(cassette),
        NoImportFiles,
    )
    .expect("TesGb is a Tes inventory")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool, with_nz_edges: bool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let taxonomy = TaxonomyRepo::new(pool.clone());
    // The licence relation as the seeder writes it: one canonical term per
    // Tes token with an identity edge into every Tes inventory. A Tes-to-Tes
    // sync therefore translates the seller's own grant through the relation
    // and asks nothing, which is the production shape.
    let licences = derive_licence_crosswalk(
        include_str!("../../../docs/design/data/tes-vocabulary.json"),
        NOW,
    )
    .expect("the polled licence vocabulary parses");
    taxonomy
        .seed(&licences.terms, &licences.edges)
        .await
        .expect("the licence crosswalk seeds");
    let terms = [
        CanonicalTerm {
            id: SUBJECT,
            kind: TermKind::Subject,
            parent: None,
            label: "Maths for early years".to_owned(),
        },
        CanonicalTerm {
            id: TOPIC,
            kind: TermKind::Topic,
            parent: Some(SUBJECT),
            label: "Time".to_owned(),
        },
        CanonicalTerm {
            id: RESOURCE_TYPE,
            kind: TermKind::ResourceType,
            parent: None,
            label: "Worksheet".to_owned(),
        },
    ];
    // The resource type's edges are seeded on both sides whatever
    // `with_nz_edges` says, so the tests that count subject gaps count the
    // same gaps they counted before the type crossed with the read.
    let mut edges = vec![
        edge(
            SUBJECT,
            InventoryId::TesGb,
            TermKind::Subject,
            &["Maths for early years"],
            "1000454",
        ),
        edge(
            TOPIC,
            InventoryId::TesGb,
            TermKind::Topic,
            &["Maths for early years", "Time"],
            "1000732",
        ),
        edge(
            RESOURCE_TYPE,
            InventoryId::TesGb,
            TermKind::ResourceType,
            &["Worksheet"],
            "1",
        ),
        edge(
            RESOURCE_TYPE,
            InventoryId::TesNz,
            TermKind::ResourceType,
            &["Worksheet"],
            "1",
        ),
    ];
    if with_nz_edges {
        edges.push(edge(
            SUBJECT,
            InventoryId::TesNz,
            TermKind::Subject,
            &["Maths for early years"],
            "7000454",
        ));
        edges.push(edge(
            TOPIC,
            InventoryId::TesNz,
            TermKind::Topic,
            &["Maths for early years", "Time"],
            "7000732",
        ));
    }
    taxonomy
        .seed(&terms, &edges)
        .await
        .expect("the crosswalk seeds");
}

fn edge(
    from: CanonicalTermId,
    inventory: InventoryId,
    kind: TermKind,
    segments: &[&str],
    native: &str,
) -> ProjectionEdge {
    ProjectionEdge {
        from,
        to: VocabularyPath {
            vocabulary: VocabularyId(inventory, kind),
            segments: segments.iter().map(|s| (*s).to_owned()).collect(),
            native_id: Some(native.to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: NOW,
    }
}

fn pdf() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(b"an import fixture body");
    bytes
}

/// The adapter this suite builds reads and never writes, so the file source it
/// takes is a refusal. Local to the tests because the library no longer takes
/// an adapter at all.
pub struct NoImportFiles;

impl tam_marketplace::FileSource for NoImportFiles {
    fn fetch(
        &self,
        file: tam_types::FileId,
    ) -> impl core::future::Future<
        Output = Result<tam_marketplace::FileContent, tam_marketplace::FileSourceError>,
    > + Send {
        core::future::ready(Err(tam_marketplace::FileSourceError::Unreadable {
            file,
            detail: "the import reads listings and never uploads files".to_owned(),
        }))
    }
}

/// The run, for a tenant.
fn run_for(pool: PgPool) -> ImportRun {
    run_for_org(pool, ORG)
}

/// The same run for a stated tenant.
fn run_for_org(pool: PgPool, org: OrgId) -> ImportRun {
    ImportRun {
        pool,
        org,
        source: InventoryId::TesGb,
        target: InventoryId::TesNz,
        now: NOW,
    }
}

/// One count, read under a tenant pin.
///
/// Not a convenience. `blob` and `product_file` carry forced row-level
/// security, so an unpinned count returns zero whatever the table holds — and
/// a test asserting "no bytes were stored" against an unpinned read passes
/// because it can see nothing, not because nothing is there. That is the trap
/// `engine-driver-split.md` records from step 11's fixture, and this helper
/// exists so no assertion in this file falls into it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn pinned_count(pool: &PgPool, org: OrgId, sql: &str) -> i64 {
    let mut tx = pool.begin().await.expect("the count opens a transaction");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let count: i64 = sqlx::query_scalar(sql)
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("the count reads");
    count
}

/// The listing a cassette answers, for the paths that need it without files.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn listing_of(
    adapter: &TesAdapter<CassetteTransport, NoImportFiles>,
    resource: i64,
) -> tam_marketplace::ImportedListing {
    adapter
        .fetch_for_import(
            &FetchReason::FirstPartyExport {
                inventory: InventoryId::TesGb,
            },
            DraftId(resource),
        )
        .await
        .expect("the fixture listing reads")
}

/// One resource, read from the cassette and ingested from the fixture bytes,
/// exactly as the operator import does it.
///
/// The suite keeps driving `Held` files through the same pipeline the operator
/// path uses, which is what makes every existing assertion below a golden test
/// of the split: the apply half stopped fetching and stopped ingesting, and
/// each of these still reports what it reported before.
async fn applied_held(
    pool: &PgPool,
    store_root: std::path::PathBuf,
    adapter: &TesAdapter<CassetteTransport, NoImportFiles>,
    resource: i64,
) -> AppliedResource {
    applied_fixture(Fixture {
        pool,
        store_root,
        adapter,
        resource,
        bytes: pdf(),
        org: ORG,
    })
    .await
}

/// The same, for a stated tenant, because a blob belongs to one.
async fn applied_held_for(
    pool: &PgPool,
    store_root: std::path::PathBuf,
    adapter: &TesAdapter<CassetteTransport, NoImportFiles>,
    resource: i64,
    org: OrgId,
) -> AppliedResource {
    applied_fixture(Fixture {
        pool,
        store_root,
        adapter,
        resource,
        bytes: pdf(),
        org,
    })
    .await
}

/// The same, from stated bytes rather than the standard fixture, for the paths
/// that download a real bundle.
async fn applied_bytes(
    pool: &PgPool,
    store_root: std::path::PathBuf,
    adapter: &TesAdapter<CassetteTransport, NoImportFiles>,
    resource: i64,
    bytes: Vec<u8>,
) -> AppliedResource {
    applied_fixture(Fixture {
        pool,
        store_root,
        adapter,
        resource,
        bytes,
        org: ORG,
    })
    .await
}

/// One fixture resource: where its blobs land, whose they are, which cassette
/// answers its listing, and what its bytes say. Bundled because the arity
/// would otherwise exceed the workspace argument limit, which is a shared gate
/// rather than something to widen for a test helper.
struct Fixture<'a> {
    pool: &'a PgPool,
    store_root: std::path::PathBuf,
    adapter: &'a TesAdapter<CassetteTransport, NoImportFiles>,
    resource: i64,
    bytes: Vec<u8>,
    org: OrgId,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn applied_fixture(fixture: Fixture<'_>) -> AppliedResource {
    let Fixture {
        pool,
        store_root,
        adapter,
        resource,
        bytes,
        org,
    } = fixture;
    let listing = adapter
        .fetch_for_import(
            &FetchReason::FirstPartyExport {
                inventory: InventoryId::TesGb,
            },
            DraftId(resource),
        )
        .await
        .expect("the fixture listing reads");
    let repo = BlobRepo::new(
        pool.clone(),
        LocalObjectStore::new(store_root),
        Kek::from_bytes(&[0x11; 32]).expect("a well-formed kek"),
    );
    let sink = TenantBlobSink {
        repo: &repo,
        org,
        at: NOW,
    };
    let ingested = ingest(
        &bytes,
        &EicarScanner,
        &sink,
        IngestContext {
            budget: ExtractBudget::default(),
            now: NOW,
            archives: ArchiveMode::Explode,
        },
    )
    .await
    .expect("the fixture ingests");
    AppliedResource {
        resource,
        listing,
        payload: ingested
            .payload
            .iter()
            .map(|stored| ImportedFile {
                kind: stored.kind,
                bytes: FileBytes::Held {
                    hash: stored.hash,
                    byte_len: stored.byte_len,
                    scan: ScanOutcome::Clean { at: NOW },
                },
            })
            .collect(),
        cover: HeldFile {
            kind: FileKind::Image,
            hash: ingested.cover.hash,
            byte_len: ingested.cover.byte_len,
            scan: ScanOutcome::Clean { at: NOW },
        },
    }
}

fn store_root(tag: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("import-{tag}-{}", std::process::id()))
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_mapped_catalogue_row_imports_and_projects(pool: PgPool) {
    seed(&pool, true).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());

    let entry = applied_held(&pool, store_root("mapped"), &adapter, 13_549_794).await;
    let report = import_one(&run, &entry).await.expect("the import runs");

    assert_eq!(
        (report.terms_seen, report.terms_mapped, report.projectable),
        (2, 2, true),
        "both categories mapped inbound and the NZ projection is green"
    );
    assert_eq!(
        (report.raised.new, report.raised.already_open),
        (0, 0),
        "a covered catalogue raises nothing — the drain's happy half"
    );

    let record = ProductRepo::new(pool)
        .get(ORG, report.product)
        .await
        .expect("the product reads back")
        .expect("the product exists");
    let product = record.product;
    assert_eq!(product.title.0, "Fractions practice");
    assert_eq!(product.price, PriceIntent::Free, "CC-BY is free");
    assert_eq!(
        product.subjects,
        vec![SUBJECT, TOPIC, RESOURCE_TYPE],
        "the canonical terms landed on the product, the declared resource type among them"
    );
    assert_eq!(
        product.grades.raw[0].native_id.as_deref(),
        Some("2"),
        "the declared age range is retained verbatim"
    );
    let derived = product.grades.derived.expect("range 2 is bounded");
    assert_eq!(
        (derived.low_years(), derived.high_years()),
        (5, 7),
        "the interval derives from the measured table"
    );
    assert!(product.cover.is_some(), "the pipeline generated the cover");
    assert_eq!(
        product.rights,
        tam_domain::RightsDeclaration::Declared {
            source: tam_domain::VocabularyPath {
                vocabulary: tam_domain::VocabularyId(
                    InventoryId::TesGb,
                    tam_domain::TermKind::Licence
                ),
                segments: vec!["CC-BY".to_owned()],
                native_id: Some("CC-BY".to_owned()),
            },
        },
        "the import read the licence and discarded it before this; the grant is the seller's \
         and now travels with the product"
    );
    assert_eq!(
        product
            .native_residue
            .iter()
            .filter_map(|term| term.native_id.clone())
            .collect::<Vec<_>>(),
        vec!["year-2".to_owned(), "English".to_owned()],
        "a GB resource answers the phase axis from ageRanges, so the yearGroups value and \
         the curriculum orientation are values in axes this model does not type, kept \
         verbatim rather than filed under a kind they do not mean"
    );
    assert_eq!(
        report.curriculum,
        vec!["English".to_owned()],
        "the operator-facing column is derived from the residue by membership in the \
         twelve-value orientation vocabulary, not carried as its own field"
    );
}

/// The whole row report for one covered listing, unchanged by the split.
///
/// The apply half stopped reading the marketplace and stopped ingesting, and
/// `ImportRowReport` gained `terms_uncovered`; nothing else about what a
/// resource becomes was meant to move. Most values below are ones this file
/// already asserted before the split — the counts, the projection verdict and
/// the curriculum column from `a_mapped_catalogue_row_imports_and_projects`,
/// the zero unmapped ids from the drain event's `terms_unmapped` — gathered
/// onto one listing, so a field that quietly changed fails here rather than
/// nowhere. Three are pinned here for the first time and are not golden in
/// that sense: `terms_uncovered`, which the split added, and `source` and
/// `source_state`, which the row report already carried but which nothing in
/// this suite asserted. The two minted identifiers are the only fields not
/// pinned at all, because they are fresh per run by design.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_row_report_for_a_covered_listing_is_what_it_was_before_the_split(pool: PgPool) {
    seed(&pool, true).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());

    let entry = applied_held(&pool, store_root("golden"), &adapter, 13_549_794).await;
    let report = import_one(&run, &entry).await.expect("the import runs");

    assert_eq!(report.resource, 13_549_794);
    assert_eq!(report.title, "Fractions practice");
    assert_eq!(
        (
            report.terms_seen,
            report.terms_mapped,
            report.terms_uncovered
        ),
        (2, 2, 0),
        "both categories map inbound over the Tes relation and both reach an NZ counterpart"
    );
    assert!(
        report.unmapped_native_ids.is_empty(),
        "every native id the read carried on the subject axis reached a canonical term"
    );
    assert_eq!(report.curriculum, vec!["English".to_owned()]);
    assert_eq!(
        (report.raised.new, report.raised.already_open),
        (0, 0),
        "a covered catalogue raises nothing"
    );
    assert_eq!(
        (report.projectable, report.blocked_by.as_deref()),
        (true, None)
    );
    assert_eq!(
        report.source,
        RemoteListingId::Tes {
            url: "https://www.tes.com/teaching-resource/-13549794".to_owned(),
        },
        "a migrate's removal names the source from the read that produced this row, so the \
         row carries it rather than a second read recovering it"
    );
    assert_eq!(
        report.source_state,
        Some(ListingState::Live),
        "the fixture carries no `draft` key, which the Tes read takes as live rather than \
         leaving the lifecycle unobserved"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_three_licences_the_import_used_to_refuse_now_import(pool: PgPool) {
    seed(&pool, true).await;
    // TES-PAID-SCHOOL, TES-V1 and TES-V2 are real rows the store holds and
    // the adapter modelled four of seven, so each of these was a silent
    // per-listing drop that the drain totals concealed: import_and_report
    // prints the failure and continues.
    for (resource, licence, price, expected) in [
        (
            13_000_001_i64,
            "TES-V1",
            serde_json::Value::Null,
            PriceIntent::Free,
        ),
        (
            13_000_002,
            "TES-V2",
            serde_json::Value::Null,
            PriceIntent::Free,
        ),
        // The wire's integer is minor units: the captured publish body reads
        // `"price": 500` for GBP 5.00, and the dashboard rows carry the same
        // number as `price_pence`. The `4.5` this fixture carried until
        // 2026-09-07 was an invented major-unit amount, and the read that
        // agreed with it imported the founder's £5.00 listings as £500.00.
        (
            13_000_003,
            "TES-PAID-SCHOOL",
            serde_json::json!(450),
            PriceIntent::Paid(
                tam_types::Money::new(450, tam_types::Currency::Gbp)
                    .expect("450 pence is a positive amount"),
            ),
        ),
    ] {
        let adapter = adapter_licensed(resource, licence, price);
        let run = run_for(pool.clone());

        let entry = applied_held(
            &pool,
            store_root(&format!("lic-{licence}")),
            &adapter,
            resource,
        )
        .await;
        let report = import_one(&run, &entry)
            .await
            .unwrap_or_else(|error| panic!("{licence} imports rather than being dropped: {error}"));
        let product = ProductRepo::new(pool.clone())
            .get(ORG, report.product)
            .await
            .expect("the product reads back")
            .expect("the product exists");
        assert_eq!(
            product.product.price, expected,
            "{licence} classifies from the polled paid flag rather than from a four-value \
             transcription"
        );
        assert!(
            matches!(
                product.product.rights,
                tam_domain::RightsDeclaration::Declared { .. }
            ),
            "{licence} is the seller's grant and travels with the product"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn measure_reports_a_covered_catalogue_as_zero_uncovered_without_files(pool: PgPool) {
    seed(&pool, true).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());
    let listing = listing_of(&adapter, 13_549_794).await;
    let report = measure_one(&run, 13_549_794, &listing)
        .await
        .expect("the measure runs with no files");
    assert_eq!(
        (
            report.terms_seen,
            report.terms_mapped,
            report.terms_uncovered
        ),
        (2, 2, 0),
        "both categories map and both have an NZ counterpart, so the drain is zero"
    );
    let mut totals = MeasureTotals::default();
    totals.absorb(&report);
    assert_eq!(
        totals.share(),
        Some(0.0),
        "a fully covered sample is 0% drain"
    );

    let product_rows: (i64,) = sqlx::query_as("SELECT count(*) FROM product")
        .fetch_one(&pool)
        .await
        .expect("the product count reads");
    assert_eq!(product_rows.0, 0, "a measurement persists no product");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn measure_counts_the_uncovered_terms_a_full_import_would_raise(pool: PgPool) {
    seed(&pool, false).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());
    let listing = listing_of(&adapter, 13_549_794).await;
    let report = measure_one(&run, 13_549_794, &listing)
        .await
        .expect("the measure runs");
    assert_eq!(
        (report.terms_mapped, report.terms_uncovered),
        (2, 2),
        "no NZ edges: both mapped terms are uncovered, matching the full import's raise count"
    );
    let mut totals = MeasureTotals::default();
    totals.absorb(&report);
    assert_eq!(
        totals.share(),
        Some(1.0),
        "an empty crosswalk is 100% drain"
    );

    let open = TaxonomyRepo::new(pool)
        .open_items(ORG)
        .await
        .expect("the queue reads");
    assert!(
        open.is_empty(),
        "a measurement raises no reconciliation item"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_gap_blocks_the_projection_and_raises_exactly_once(pool: PgPool) {
    seed(&pool, false).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());

    let entry = applied_held(&pool, store_root("gap"), &adapter, 13_549_794).await;
    let report = import_one(&run, &entry).await.expect("the import runs");
    assert_eq!(
        (report.projectable, report.blocked_by.as_deref()),
        (false, Some("taxonomy")),
        "no NZ edges: the projection blocks on the taxonomy gate"
    );
    assert_eq!(report.raised.new, 2, "each of the two gaps raised its item");

    let open = TaxonomyRepo::new(pool)
        .open_items(ORG)
        .await
        .expect("the queue reads");
    assert_eq!(open.len(), 2, "the founder sees exactly the two gaps");
}

/// A seller's own override answers a gap the global relation cannot, for that
/// seller and for nobody else.
///
/// The fixture is the one `a_gap_blocks_the_projection_and_raises_exactly_once`
/// uses: no NZ edges, so both terms are uncovered and two items raise. One
/// override on the subject leaves exactly one gap for the org that set it and
/// both for an org that did not, which is the tenancy of the layer and its
/// effect in the same assertion. Before this wiring the importer projected
/// through `project_listing`, which delegates with an empty override set, so
/// the org that had answered still had its answer raised back at it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sellers_override_answers_a_gap_for_that_seller_only(pool: PgPool) {
    seed(&pool, false).await;
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(OTHER_ORG.0 .0))
        .execute(&pool)
        .await
        .expect("the second org seeds");
    let decided = ProjectionOverride::new(NewProjectionOverride {
        org: ORG,
        inventory: InventoryId::TesNz,
        axis: TermKind::Subject,
        from: SUBJECT,
        to: VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesNz, TermKind::Subject),
            segments: vec!["Mathematics".to_owned()],
            native_id: Some("nz-mathematics".to_owned()),
        },
        kind: OverrideKind::Exact,
        decided_by: Decider::Human {
            user: UserId(Uuid([0xC1; 16])),
            org: ORG,
        },
        decided_at: NOW,
    })
    .expect("the override is well formed");
    OverrideRepo::new(pool.clone())
        .upsert(&decided)
        .await
        .expect("the override writes");

    let adapter = adapter_for(13_549_794);
    let mine = import_one(
        &run_for_org(pool.clone(), ORG),
        &applied_held(&pool, store_root("override-mine"), &adapter, 13_549_794).await,
    )
    .await
    .expect("the import runs for the org that decided");
    assert_eq!(
        mine.raised.new, 1,
        "the subject is answered by the seller's own decision, so only the topic is still a \
         gap: the importer consulted the override rather than raising a question back at the \
         seller who had already answered it"
    );

    let other_adapter = adapter_for(13_549_794);
    let theirs = import_one(
        &run_for_org(pool.clone(), OTHER_ORG),
        &applied_held_for(
            &pool,
            store_root("override-theirs"),
            &other_adapter,
            13_549_794,
            OTHER_ORG,
        )
        .await,
    )
    .await
    .expect("the import runs for the org that decided nothing");
    assert_eq!(
        theirs.raised.new, 2,
        "and the other tenant sees both gaps, because an override is one seller's decision \
         about their own listings and not a change to the relation everyone shares"
    );
}

/// A marketplace-sourced resource imports, and no blob of it is stored.
///
/// D27's property on the server side, and the mirror of the device's own
/// assertion that no payload bytes reach the wire: mine says none are sent,
/// this says none are kept. The cover is `Held` because Q-c allows a derived
/// thumbnail; the payload is `Sourced` and must leave the object store empty
/// of anything but that cover.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sourced_payload_imports_and_stores_no_bytes_of_it(pool: PgPool) {
    seed(&pool, true).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());

    // The cover alone goes through the pipeline, exactly as the device's
    // import does it: the seller's own file is named, never stored.
    let held = applied_held(&pool, store_root("sourced"), &adapter, 13_549_794).await;
    let blobs_after_cover =
        pinned_count(&pool, ORG, "SELECT count(*) FROM blob WHERE org_id = $1").await;
    assert!(
        blobs_after_cover > 0,
        "the cover must actually be stored, or the comparison below is between two zeroes \
         and proves nothing"
    );

    // A real connection, because a sourced file names the one that can fetch
    // it and the schema holds it to that.
    // Written under a tenant pin, because `connection` carries forced
    // row-level security: an unpinned write is refused rather than silently
    // misfiled, which is the same rail the device write paths run under.
    let connection = ConnectionId(Uuid([0x44; 16]));
    let mut tx = pool.begin().await.expect("the fixture opens a transaction");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', $3, $3)",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(connection.0 .0))
    .bind(sqlx::types::chrono::DateTime::from_timestamp_millis(NOW.0).expect("a valid instant"))
    .execute(&mut *tx)
    .await
    .expect("the connection seeds");
    // And the device that observed it, for the same reason: a sourced file
    // records which machine saw it, and the schema holds it to a real one.
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, first_seen_at, \
         last_seen_at) VALUES ($1, $2, 'a test machine', 'linux', 'x86_64', '0.2.0', $3, $3)",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(DEVICE)
    .bind(sqlx::types::chrono::DateTime::from_timestamp_millis(NOW.0).expect("a valid instant"))
    .execute(&mut *tx)
    .await
    .expect("the device seeds");
    tx.commit().await.expect("the fixture commits");

    let applied = AppliedResource {
        resource: 13_549_794,
        listing: held.listing.clone(),
        payload: vec![ImportedFile {
            kind: FileKind::Pdf,
            bytes: FileBytes::Sourced {
                marketplace: Marketplace::Tes,
                connection,
                resource: "13549794".to_owned(),
                entry: None,
                payload_file_name: "13549794-bundle.zip".to_owned(),
                payload_content_type: "application/zip".to_owned(),
                observed: Observation {
                    device: DEVICE.to_owned(),
                    hash: ContentHash([0x5A; 32]),
                    byte_len: 4_096,
                    scan: ScanOutcome::Clean { at: NOW },
                    observed_at: NOW,
                },
            },
        }],
        cover: held.cover,
    };

    let report = import_one(&run, &applied)
        .await
        .expect("a sourced payload imports");
    assert_eq!(report.resource, 13_549_794);

    let blobs_after = pinned_count(&pool, ORG, "SELECT count(*) FROM blob WHERE org_id = $1").await;
    assert_eq!(
        blobs_after, blobs_after_cover,
        "importing a marketplace-sourced payload stored bytes. The cover is ours to keep and \
         is already counted; anything beyond it is the seller's file on our servers, which is \
         the one thing D27 forbids"
    );

    let sourced = pinned_count(
        &pool,
        ORG,
        "SELECT count(*) FROM product_file WHERE org_id = $1 AND hash IS NULL",
    )
    .await;
    assert_eq!(
        sourced, 1,
        "and the payload row exists, named rather than held, so this is an import that \
         happened rather than one that quietly did nothing"
    );
    let held_rows = pinned_count(
        &pool,
        ORG,
        "SELECT count(*) FROM product_file WHERE org_id = $1 AND hash IS NOT NULL",
    )
    .await;
    assert_eq!(
        held_rows, 1,
        "the cover is blob-backed and the same pinned read finds it, so the count above is \
         one row seen and not two rows missed"
    );

    // The assertion the whole test is for, and its twin. A sourced file's
    // bytes are the seller's, so nothing in `blob` may carry the digest the
    // device reported for them. The twin runs the identical join against the
    // cover's own hash, which must find its blob: without it a pin that
    // returned nothing, a join written against the wrong columns, or a table
    // the reader cannot see would all pass the first assertion by seeing
    // nothing at all.
    let sourced_blobs = pinned_count(
        &pool,
        ORG,
        "SELECT count(*) FROM blob b \
         JOIN product_file f ON f.org_id = b.org_id AND f.observed_hash = b.hash \
         WHERE f.org_id = $1 AND f.hash IS NULL",
    )
    .await;
    assert_eq!(
        sourced_blobs, 0,
        "a blob carries the digest the device reported for the seller's own file, which is \
         those bytes on our servers under the one name D27 forbids them to have"
    );
    let cover_blobs = pinned_count(
        &pool,
        ORG,
        "SELECT count(*) FROM blob b \
         JOIN product_file f ON f.org_id = b.org_id AND f.hash = b.hash \
         WHERE f.org_id = $1 AND f.hash IS NOT NULL",
    )
    .await;
    assert_eq!(
        cover_blobs, 1,
        "the same join finds the cover's blob, so the zero above is an absence this read \
         could have seen rather than one it was blind to"
    );
}

/// The import's coverage number is the measurement's, exactly.
///
/// This pins the definition rather than a fixture, and it is the pin that
/// matters: the founder compares this number across a series of migrations, so
/// the two callers must compute one quantity rather than two that agree today.
/// The near neighbour it would drift into is the listing projection's own
/// gaps, which count over five routed axes rather than two, count per
/// term-and-target rather than per distinct term, and report nothing at all
/// when a listing blocks for a reason that is not a term — a currency nobody
/// measured, a missing cover, an unanswered election. Any of those three
/// divergences breaks this equality.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_imports_coverage_number_is_the_measurements(pool: PgPool) {
    // Seeded without the NZ edges, so there is genuinely something uncovered
    // and the equality is not satisfied by both sides being zero.
    seed(&pool, false).await;
    let run = run_for(pool.clone());

    // One cassette per read, because a cassette answers each recorded
    // interaction once and both halves of this equality read the same draft.
    let measured_adapter = adapter_for(13_549_794);
    let listing = listing_of(&measured_adapter, 13_549_794).await;
    let measured = measure_one(&run, 13_549_794, &listing)
        .await
        .expect("the measurement runs");

    let adapter = adapter_for(13_549_794);
    let entry = applied_held(&pool, store_root("uncovered-agree"), &adapter, 13_549_794).await;
    let imported = import_one(&run, &entry).await.expect("the import runs");

    assert!(
        measured.terms_uncovered > 0,
        "the fixture must have something uncovered, or this equality proves nothing"
    );
    assert_eq!(
        imported.terms_uncovered, measured.terms_uncovered,
        "the import and the measurement must report one number. They differ, which means the \
         import is counting something else — most likely the projection's first blocker, \
         which is a different quantity over different axes"
    );
}

/// What the coverage number counts, pinned against the two readings it would
/// otherwise drift into.
///
/// The founder compares this number across a series of migrations, so it has
/// to mean one thing over time, and both near neighbours are one plausible
/// edit away.
///
/// The first is the listing projection's own gaps. Etsy declares no
/// equivalence axis at all, so the projection visits none, raises no term
/// cause, and blocks at the currency gate instead — an outcome that says
/// nothing whatever about coverage. A report reading the projection's causes
/// would call this resource fully covered while both of its terms have
/// nowhere in Etsy to go. The number is taken from the mapped terms before
/// the projection runs, which is exactly why it survives a blocker that is
/// not a term.
///
/// The second is counting a recorded no-counterpart. Somebody decided that
/// axis does not cross, and a decision is not a gap; the second half records
/// two and watches the same fixture fall to zero.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_coverage_number_counts_terms_and_not_the_projections_blocker(pool: PgPool) {
    seed(&pool, true).await;
    // Paid, because the currency gate applies to a price and reaching a
    // blocker that is not a term is the whole point of the fixture.
    let adapter = adapter_licensed(13_549_794, "TES-PAID-SCHOOL", serde_json::json!(450));
    let run = ImportRun {
        pool: pool.clone(),
        org: ORG,
        source: InventoryId::TesGb,
        target: InventoryId::Etsy,
        now: NOW,
    };

    let entry = applied_held(&pool, store_root("etsy-coverage"), &adapter, 13_549_794).await;
    let report = import_one(&run, &entry).await.expect("the import runs");
    assert_eq!(
        report.blocked_by.as_deref(),
        Some("currency_unknown"),
        "Etsy denominates per seller and no seller's is measured, so a paid listing blocks \
         there rather than on any term"
    );
    assert_eq!(
        (report.raised.new, report.raised.already_open),
        (0, 0),
        "and that arm raises nothing at all, which is precisely the zero a report reading \
         the projection's causes would publish as coverage"
    );
    assert_eq!(
        (report.terms_mapped, report.terms_uncovered),
        (2, 2),
        "both terms mapped inbound and neither has anywhere declared to go in Etsy, so the \
         count is two: taken from the terms rather than from what stopped the projection"
    );

    TaxonomyRepo::new(pool.clone())
        .seed_no_counterparts(&[
            NoCounterpart {
                term: SUBJECT,
                target: VocabularyId(InventoryId::Etsy, TermKind::Subject),
                decided_by: Decider::Imported {
                    source: "test fixture".to_owned(),
                },
                decided_at: NOW,
            },
            NoCounterpart {
                term: TOPIC,
                target: VocabularyId(InventoryId::Etsy, TermKind::Topic),
                decided_by: Decider::Imported {
                    source: "test fixture".to_owned(),
                },
                decided_at: NOW,
            },
        ])
        .await
        .expect("the no-counterpart decisions record");

    let again = adapter_licensed(13_549_794, "TES-PAID-SCHOOL", serde_json::json!(450));
    let entry = applied_held(&pool, store_root("etsy-coverage"), &again, 13_549_794).await;
    let decided = import_one(&run, &entry)
        .await
        .expect("the import runs again");
    assert_eq!(
        decided.blocked_by.as_deref(),
        Some("currency_unknown"),
        "nothing about the currency changed, so the same blocker stands"
    );
    assert_eq!(
        (decided.terms_mapped, decided.terms_uncovered),
        (2, 0),
        "the same two terms map and neither is uncovered now: a recorded no-counterpart is \
         omitted from the number rather than counted as a gap"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_drain_report_lands_as_a_job_event_the_client_can_read(pool: PgPool) {
    seed(&pool, false).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone());

    let entry = applied_held(&pool, store_root("drain"), &adapter, 13_549_794).await;
    let report = import_one(&run, &entry).await.expect("the import runs");
    let mut totals = DrainTotals::default();
    totals.absorb(&report);
    let job = record_drain_report(&run, totals)
        .await
        .expect("the drain report records");

    let reads = JobReadRepo::new(pool);
    let snapshot = reads
        .snapshot(ORG, job)
        .await
        .expect("the job reads")
        .expect("the job exists");
    assert_eq!(
        snapshot.inventory,
        InventoryId::TesGb,
        "the job carries the inventory the run reads"
    );
    assert_eq!(
        snapshot.counts.total, 0,
        "an import publishes nothing, so its job carries no item the lease scan could claim"
    );

    let events = reads.events_after(ORG, 0, 16).await.expect("events read");
    let drain = events
        .iter()
        .find(|event| event.kind == "ImportDrainMeasured")
        .expect("the drain report is on the stream");
    assert_eq!(drain.job, job, "the report is scoped to the run's job");
    assert_eq!(
        drain.payload,
        serde_json::json!({
            "source": "TesGb",
            "target": "TesNz",
            "rows": 1,
            "terms_seen": 2,
            "terms_unmapped": 0,
            "terms_covered": 0,
            "items_new": 2,
            "items_already_open": 0,
        }),
        "an empty NZ crosswalk raises every canonical term: the gate's first-run share is 2 of 2"
    );
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A stored (uncompressed) ZIP assembled by hand, so this test needs no
/// archive dependency of its own. The pipeline reads it with the real
/// reader, so these bytes have to be a genuine archive.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let small = "the fixture archive is small";
    let mut out: Vec<u8> = Vec::new();
    let mut directory: Vec<u8> = Vec::new();
    for (name, bytes) in entries {
        let offset = u32::try_from(out.len()).expect(small);
        let len = u32::try_from(bytes.len()).expect(small);
        let name_len = u16::try_from(name.len()).expect(small);
        let crc = crc32(bytes);

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&10u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(bytes);

        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&10u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&len.to_le_bytes());
        directory.extend_from_slice(&len.to_le_bytes());
        directory.extend_from_slice(&name_len.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = u32::try_from(out.len()).expect(small);
    let directory_len = u32::try_from(directory.len()).expect(small);
    let count = u16::try_from(entries.len()).expect(small);
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&directory_len.to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn json_ok(value: &serde_json::Value) -> HttpResponse {
    HttpResponse::plain(200, value.to_string().into_bytes())
}

/// The hops discover makes, in order: the catalogue walk, then the two-step
/// download, then the metadata read `import_one` does for itself.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn discover_adapter(
    resource: i64,
    bundle: Vec<u8>,
) -> TesAdapter<CassetteTransport, NoImportFiles> {
    let limit = tes::CATALOGUE_PAGE_LIMIT;
    let path = format!("/teaching-resource/download/{resource}/bundle");
    let mut zip_urls = serde_json::Map::new();
    zip_urls.insert(
        resource.to_string(),
        serde_json::json!({ "url": path, "title": "Fractions practice" }),
    );
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: tes::list_resources_request(0, limit),
                response: json_ok(&serde_json::json!([{
                    "id": resource,
                    "title": "Fractions practice",
                    "licence": "CC-BY",
                    "price": 0,
                    "draft": false
                }])),
            },
            Interaction {
                request: tes::list_resources_request(1, limit),
                response: json_ok(&serde_json::json!([])),
            },
            Interaction {
                request: tes::list_drafts_request(0, limit),
                response: json_ok(&serde_json::json!([])),
            },
            Interaction {
                request: tes::download_manifest_request(DraftId(resource)),
                response: json_ok(&serde_json::json!({ "zipUrls": zip_urls })),
            },
            Interaction {
                request: tes::download_bundle_request(&path),
                response: HttpResponse::plain(200, bundle),
            },
            Interaction {
                request: HttpRequest::get(format!(
                    "https://www.tes.com/api/v2/resources/{resource}/draft"
                )),
                response: HttpResponse::plain(200, draft_body(resource).into_bytes()),
            },
        ],
    };
    TesAdapter::new(
        InventoryId::TesGb,
        CassetteTransport::new(cassette),
        NoImportFiles,
    )
    .expect("TesGb is a Tes inventory")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn discover_lists_downloads_and_imports_with_no_file_on_disk(pool: PgPool) {
    seed(&pool, true).await;
    let adapter = discover_adapter(13_549_794, zip_of(&[("worksheet.pdf", pdf())]));
    let run = run_for(pool.clone());
    let reason = FetchReason::FirstPartyExport {
        inventory: InventoryId::TesGb,
    };

    let catalogue = adapter
        .list_own_resources(&reason)
        .await
        .expect("the catalogue lists");
    let published: Vec<_> = catalogue
        .into_iter()
        .filter(|entry| entry.published)
        .collect();
    assert_eq!(
        published.len(),
        1,
        "one published resource walked out of the catalogue"
    );

    let bundle = adapter
        .download_resource_bundle(&reason, DraftId(published[0].id))
        .await
        .expect("the published bundle downloads");
    let entry = applied_bytes(
        &pool,
        store_root("discover"),
        &adapter,
        published[0].id,
        bundle,
    )
    .await;
    let report = import_one(&run, &entry)
        .await
        .expect("the downloaded bundle imports");
    let mut totals = DrainTotals::default();
    totals.absorb(&report);
    let job = record_drain_report(&run, totals)
        .await
        .expect("the drain report records");

    let record = ProductRepo::new(pool.clone())
        .get(ORG, report.product)
        .await
        .expect("the product reads back")
        .expect("the product exists");
    let kinds: Vec<FileKind> = record
        .product
        .payload_files()
        .map(|file| file.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![FileKind::Pdf],
        "the pipeline extracted the bundle: the payload is the pdf from inside it, not the zip"
    );
    assert_eq!(
        record.product.title.0, "Fractions practice",
        "the metadata read still supplies the listing copy"
    );
    assert!(
        record.product.cover.is_some(),
        "the cover generated from the extracted file"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "list, download and metadata read: exactly the recorded hops and no more"
    );

    let snapshot = JobReadRepo::new(pool)
        .snapshot(ORG, job)
        .await
        .expect("the job reads")
        .expect("the drain job exists");
    assert_eq!(
        snapshot.inventory,
        InventoryId::TesGb,
        "the drain job carries the inventory the run read"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bundle_wrapping_an_inner_zip_keeps_it_as_one_archive_payload(pool: PgPool) {
    seed(&pool, true).await;
    let inner = zip_of(&[("slides.pdf", pdf())]);
    let bundle = zip_of(&[("worksheet.pdf", pdf()), ("extras.zip", inner)]);
    let adapter = discover_adapter(13_549_794, bundle);
    let run = run_for(pool.clone());
    let reason = FetchReason::FirstPartyExport {
        inventory: InventoryId::TesGb,
    };

    let catalogue = adapter
        .list_own_resources(&reason)
        .await
        .expect("the catalogue lists");
    let downloaded = adapter
        .download_resource_bundle(&reason, DraftId(catalogue[0].id))
        .await
        .expect("the bundle downloads");

    let entry = applied_bytes(
        &pool,
        store_root("discover-nested"),
        &adapter,
        catalogue[0].id,
        downloaded,
    )
    .await;
    let report = import_one(&run, &entry)
        .await
        .expect("the nested bundle imports");

    let record = ProductRepo::new(pool)
        .get(ORG, report.product)
        .await
        .expect("the product reads back")
        .expect("the product exists");
    let kinds: Vec<FileKind> = record
        .product
        .payload_files()
        .map(|file| file.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![FileKind::Pdf, FileKind::Zip],
        "extraction is one level deep: an inner archive is stored as a Zip payload, not recursed into"
    );
}
