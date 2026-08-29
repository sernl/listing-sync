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
use tam_marketplace::transport::{HttpRequest, HttpResponse};
use tam_marketplace::FetchReason;
use tam_marketplace_tes::{endpoints as tes, DraftId, TesAdapter};
use tam_secrets::Kek;
use tam_storage::{JobReadRepo, ProductRepo, TaxonomyRepo};
use tam_types::{CanonicalTermId, FileKind, InventoryId, OrgId, PriceIntent, Timestamp, Uuid};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const TOPIC: CanonicalTermId = CanonicalTermId(Uuid([0x78; 16]));
const NOW: Timestamp = Timestamp(1_000);

fn draft_body(resource: i64) -> String {
    draft_body_licensed(resource, "CC-BY", serde_json::Value::Null)
}

fn draft_body_licensed(resource: i64, licence: &str, price: serde_json::Value) -> String {
    let mut body = serde_json::json!({
        "id": resource,
        "title": "Fractions practice",
        "descriptionRaw": "A worksheet.",
        "descriptionRawType": "markdown",
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
) -> ImportRun<'_, TesAdapter<CassetteTransport, NoImportFiles>> {
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
        (
            13_000_003,
            "TES-PAID-SCHOOL",
            serde_json::json!(4.5),
            PriceIntent::Paid(
                tam_types::Money::new(450, tam_types::Currency::Gbp)
                    .expect("450 pence is a positive amount"),
            ),
        ),
    ] {
        let adapter = adapter_licensed(resource, licence, price);
        let run = run_for(
            pool.clone(),
            &adapter,
            store_root(&format!("lic-{licence}")),
        );
        let entry = ImportEntry {
            resource,
            files: vec![NamedBytes {
                name: "worksheet.pdf".to_owned(),
                bytes: pdf(),
            }],
        };
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
    let run = run_for(pool.clone(), &adapter, store_root("discover"));
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
    let entry = ImportEntry {
        resource: published[0].id,
        files: vec![NamedBytes {
            name: format!("{}-bundle.zip", published[0].id),
            bytes: bundle,
        }],
    };
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
        .payload
        .iter()
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
    let run = run_for(pool.clone(), &adapter, store_root("discover-nested"));
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
    let entry = ImportEntry {
        resource: catalogue[0].id,
        files: vec![NamedBytes {
            name: "13549794-bundle.zip".to_owned(),
            bytes: downloaded,
        }],
    };
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
        .payload
        .iter()
        .map(|file| file.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![FileKind::Pdf, FileKind::Zip],
        "extraction is one level deep: an inner archive is stored as a Zip payload, not recursed into"
    );
}
