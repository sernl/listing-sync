//! The import end to end over a cassette-shaped draft read: files through
//! the real pipeline into real blobs, taxonomy inbound over a seeded
//! crosswalk, grades verbatim with a derived interval, the target mapping
//! created, and the drain report raising exactly the gaps.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_import::{import_one, ImportEntry, ImportRun, NamedBytes, NoImportFiles};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{HttpRequest, HttpResponse, Method, RequestBody};
use tam_marketplace_tes::TesAdapter;
use tam_secrets::Kek;
use tam_storage::{ProductRepo, TaxonomyRepo};
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
                body: draft_body(resource),
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
