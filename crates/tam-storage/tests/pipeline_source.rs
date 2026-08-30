//! The pipeline closes the Tes adapter's FileSource seam end to end: a PDF is
//! ingested into encrypted blobs, its product_file rows are written, and the
//! FileSource reads the bytes back for the adapter — the seam M1c called into
//! a stub is now live.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_marketplace::{FileSource, FileSourceError};
use tam_pipeline::archive::ExtractBudget;
use tam_pipeline::pipeline::{ingest, IngestContext};
use tam_pipeline::scan::AllowAllScanner;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{BlobRepo, PipelineFileSource, TenantBlobSink};
use tam_types::{FileId, OrgId, Timestamp, Uuid};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper; a broken fixture should panic"
)]
async fn seed_org(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("org inserts");
}

fn pdf() -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(b"a worksheet body worth some bytes");
    bytes
}

#[sqlx::test(migrations = "./migrations")]
async fn ingest_then_fetch_back_closes_the_adapter_seam(pool: PgPool) {
    seed_org(&pool).await;
    let dir = std::env::temp_dir().join(format!("tam-pipe-{}", std::process::id()));
    let repo = BlobRepo::new(
        pool.clone(),
        LocalObjectStore::new(dir.clone()),
        Kek::from_bytes(&[0x33; 32]).expect("kek"),
    );
    let sink = TenantBlobSink {
        repo: &repo,
        org: ORG,
        at: T0,
    };

    let ingested = ingest(
        &pdf(),
        &AllowAllScanner,
        &sink,
        IngestContext {
            budget: ExtractBudget::default(),
            now: T0,
            archives: tam_pipeline::pipeline::ArchiveMode::Explode,
        },
    )
    .await
    .expect("the pdf ingests into encrypted blobs");
    assert_eq!(ingested.payload.len(), 1, "the pdf is one payload blob");

    // Write the product_file row the FileSource joins on (the catalogue
    // insert does this in the full flow; here we write the one row directly).
    let file_id = FileId(Uuid([0x21; 16]));
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("pin");
    sqlx::query(
        "INSERT INTO product \
         (org_id, id, title, body, body_format, price_kind, rights_state, \
          created_at, updated_at) \
         VALUES ($1, $2, 't', 'b', 'markdown', 'free', 'unstated', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes([0x10; 16]))
    .execute(&mut *tx)
    .await
    .expect("product inserts");
    sqlx::query(
        "INSERT INTO product_file \
         (org_id, id, product_id, position, role, kind, hash, scan_state, created_at) \
         VALUES ($1, $2, $3, 0, 'payload', 'pdf', $4, 'pending', now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(file_id.0 .0))
    .bind(uuid::Uuid::from_bytes([0x10; 16]))
    .bind(ingested.payload[0].hash.0.to_vec())
    .execute(&mut *tx)
    .await
    .expect("product_file inserts");
    // The deferred payload trigger is satisfied: one payload row exists.
    tx.commit().await.expect("commit");

    let source = PipelineFileSource::new(repo, ORG, pool);
    let content = source
        .fetch(file_id)
        .await
        .expect("the FileSource reads the blob back for the adapter");
    assert_eq!(
        content.bytes,
        pdf(),
        "the bytes the adapter would upload are the bytes ingested, through the encrypted store"
    );
    assert_eq!(
        content.content_type, "application/pdf",
        "the content type is set for the upload"
    );

    let missing = source.fetch(FileId(Uuid([0x99; 16]))).await;
    assert!(
        matches!(missing, Err(FileSourceError::Missing(_))),
        "an unknown file is Missing, not a panic"
    );
}
