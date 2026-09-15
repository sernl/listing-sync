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
//! Usage: tam-pipeline-worker ingest <db-url> <kek-path> <org-uuid-hex> \
//!            <file-path> \
//!            (--blob-store-root <dir> | --blob-store-s3 <endpoint> \
//!             --blob-store-bucket <name> --blob-store-credentials <path> \
//!             [--blob-store-region <region>])
//!
//! The store flags are the ones `tam-server` takes, read through the same
//! crate: the two processes hold the same objects, so a spelling that differed
//! between them would be a worker reading a bucket the server never wrote to.

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_blob_store::BackendFlags;
use tam_pipeline::archive::ExtractBudget;
use tam_pipeline::pipeline::{ingest, IngestContext};
use tam_pipeline::scan::EicarScanner;
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
    const USAGE: &str = "usage: tam-pipeline-worker ingest <db-url> <kek-path> <org-hex> <file> \
                         (--blob-store-root <dir> | --blob-store-s3 <endpoint> \
                         --blob-store-bucket <name> --blob-store-credentials <path> \
                         [--blob-store-region <region>])";
    // Configuration is read from the command line rather than the
    // environment, which the lint table bans outside the one crate that will
    // own it.
    // The store flags may appear anywhere among the positional arguments,
    // which is what lets a unit file keep them in one block.
    let mut arguments = std::env::args().skip(1);
    let mut positional = Vec::new();
    let mut blob_store = BackendFlags::default();
    while let Some(argument) = arguments.next() {
        if !blob_store.accept(&argument, &mut arguments)? {
            positional.push(argument);
        }
    }
    if positional.first().map(String::as_str) != Some("ingest") {
        return Err(USAGE.into());
    }
    let db_url = positional.get(1).ok_or(USAGE)?;
    let kek_path = positional.get(2).ok_or(USAGE)?;
    let org = org_from_hex(positional.get(3).ok_or(USAGE)?)?;
    let file_path = positional.get(4).ok_or(USAGE)?;
    let backend = blob_store
        .resolve()?
        .ok_or("the worker needs the store the server writes into")?;

    let mut upload = Vec::new();
    std::fs::File::open(file_path)?.read_to_end(&mut upload)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await?;
    let repo = BlobRepo::new(pool, backend.object_store(), load_kek(kek_path)?);
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
