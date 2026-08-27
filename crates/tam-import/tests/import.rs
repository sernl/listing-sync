//! The import end to end over a cassette-shaped draft read: files through
//! the real pipeline into real blobs, taxonomy inbound over a seeded
//! crosswalk, grades verbatim with a derived interval, the target mapping
//! created, and the drain report raising exactly the gaps.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_import::{
    import_one, measure_one, record_drain_report, DrainTotals, ImportEntry, ImportRun,
    MeasureTotals, NamedBytes, NoImportFiles,
};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{HttpRequest, HttpResponse, Method, RequestBody};
use tam_marketplace_tes::TesAdapter;
use tam_secrets::Kek;
use tam_storage::{JobReadRepo, ProductRepo, TaxonomyRepo};
use tam_types::{CanonicalTermId, InventoryId, OrgId, PriceIntent, Timestamp, Uuid};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const TOPIC: CanonicalTermId = CanonicalTermId(Uuid([0x78; 16]));
const NOW: Timestamp = Timestamp(1_000);

fn draft_body(resource: i64) -> String {
    serde_json::json!({
        "id": resource,
        "title": "Fractions practice",
        "descriptionRaw": "A worksheet.",
        "descriptionRawType": "markdown",
        "licence": "CC-BY",
        "categories": [{ "id": 1_000_454 }, { "id": 1_000_732 }],
        "ageRanges": [2],
        "yearGroups": ["year-2"],
        "curriculum": "English",
        "mainAge": 6,
        "mainType": 1,
        "ages": [5, 6, 7]
    })
    .to_string()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn adapter_for(resource: i64) -> TesAdapter<CassetteTransport, NoImportFiles> {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: HttpRequest {
                method: Method::Get,
                url: format!("https://www.tes.com/api/v2/resources/{resource}/draft"),
                body: RequestBody::Empty,
            },
            response: HttpResponse {
                status: 200,
                body: draft_body(resource).into_bytes(),
            },
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
    ];
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn run_for(
    pool: PgPool,
    adapter: &TesAdapter<CassetteTransport, NoImportFiles>,
    store_root: std::path::PathBuf,
) -> ImportRun<'_, CassetteTransport> {
    ImportRun {
        pool,
        kek: Kek::from_bytes(&[0x11; 32]).expect("a well-formed kek"),
        store_root,
        adapter,
        org: ORG,
        source: InventoryId::TesGb,
        target: InventoryId::TesNz,
        now: NOW,
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
    let run = run_for(pool.clone(), &adapter, store_root("mapped"));
    let entry = ImportEntry {
        resource: 13_549_794,
        files: vec![NamedBytes {
            name: "worksheet.pdf".to_owned(),
            bytes: pdf(),
        }],
    };
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
        vec![SUBJECT, TOPIC],
        "the canonical terms landed on the product"
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
    assert_eq!(report.curriculum, vec!["English".to_owned()]);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn measure_reports_a_covered_catalogue_as_zero_uncovered_without_files(pool: PgPool) {
    seed(&pool, true).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone(), &adapter, store_root("measure-covered"));
    let report = measure_one(&run, 13_549_794)
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
    let run = run_for(pool.clone(), &adapter, store_root("measure-gap"));
    let report = measure_one(&run, 13_549_794)
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
    let run = run_for(pool.clone(), &adapter, store_root("gap"));
    let entry = ImportEntry {
        resource: 13_549_794,
        files: vec![NamedBytes {
            name: "worksheet.pdf".to_owned(),
            bytes: pdf(),
        }],
    };
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

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_drain_report_lands_as_a_job_event_the_client_can_read(pool: PgPool) {
    seed(&pool, false).await;
    let adapter = adapter_for(13_549_794);
    let run = run_for(pool.clone(), &adapter, store_root("drain"));
    let entry = ImportEntry {
        resource: 13_549_794,
        files: vec![NamedBytes {
            name: "worksheet.pdf".to_owned(),
            bytes: pdf(),
        }],
    };
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
