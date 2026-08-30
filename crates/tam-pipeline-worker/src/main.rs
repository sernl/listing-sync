//! The document-ingestion lane. It carries the heaviest native closures —
//! archive inspection, image encode, the scan — and must never enter the
//! API's closure, which is why it is its own binary and its own systemd unit.
//!
//! The queue-driven pump lands with M1j (the projection and the job wiring);
//! this milestone ships the ingest logic and a one-shot operator mode that
//! ingests a file from disk under one tenant, so the pipeline exists as the
//! design's unit and is exercisable end to end without unbuilt queue
//! consumption.
//!
//! Usage: tam-pipeline-worker ingest <db-url> <object-store-root> <kek-path> \
//!            <org-uuid-hex> <file-path>

#![forbid(unsafe_code)]

use std::io::Read as _;
use std::path::PathBuf;

use tam_pipeline::archive::ExtractBudget;
use tam_pipeline::pipeline::{ingest, IngestContext};
use tam_pipeline::scan::EicarScanner;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{BlobRepo, TenantBlobSink};
use tam_types::{OrgId, Timestamp, Uuid};

fn load_kek(path: &str) -> Result<Kek, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(Kek::from_bytes(&bytes)?)
}

fn org_from_hex(hex: &str) -> Result<OrgId, Box<dyn std::error::Error>> {
    if hex.len() != 32 {
        return Err("org hex must be 32 characters".into());
    }
    let mut array = [0u8; 16];
    for (index, slot) in array.iter_mut().enumerate() {
        let start = index * 2;
        let pair = hex.get(start..start + 2).ok_or("org hex is malformed")?;
        *slot = u8::from_str_radix(pair, 16)?;
    }
    Ok(OrgId(Uuid(array)))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the worker is a clock-reading process boundary; time enters the pipeline as data from here"
)]
fn wall_now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) != Some("ingest") {
        return Err(
            "usage: tam-pipeline-worker ingest <db-url> <root> <kek-path> <org-hex> <file>".into(),
        );
    }
    let db_url = arguments.get(1).ok_or("missing db url")?;
    let root = arguments.get(2).ok_or("missing object-store root")?;
    let kek_path = arguments.get(3).ok_or("missing kek path")?;
    let org = org_from_hex(arguments.get(4).ok_or("missing org hex")?)?;
    let file_path = arguments.get(5).ok_or("missing file path")?;

    let mut upload = Vec::new();
    std::fs::File::open(file_path)?.read_to_end(&mut upload)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await?;
    let repo = BlobRepo::new(
        pool,
        LocalObjectStore::new(PathBuf::from(root)),
        load_kek(kek_path)?,
    );
    let now = wall_now()?;
    let sink = TenantBlobSink {
        repo: &repo,
        org,
        at: now,
    };

    let ingested = ingest(
        &upload,
        &EicarScanner,
        &sink,
        IngestContext {
            budget: ExtractBudget::default(),
            now,
            archives: tam_pipeline::pipeline::ArchiveMode::Explode,
        },
    )
    .await?;
    eprintln!(
        "ingested: {} payload files, cover kind {:?}",
        ingested.payload.len(),
        ingested.cover.kind
    );
    Ok(())
}
