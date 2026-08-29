//! The operator import: the founder's catalogue in, canonical products and
//! their target mappings out, with the drain report printed per row and in
//! total. Reads travel through the broker's gateway (request a lease over
//! the broker socket first; the endpoint it prints is the base URL here), so
//! this process never holds a marketplace credential.
//!
//! Usage: tam-import <db-url> <org-hex> <gateway-base> <kek-path> \
//!            <store-root> <manifest.json|discover> [measure]
//!
//! The manifest is a JSON array of { "resource": <numeric id>,
//! "files": ["/path/to/original", ...] }, taking the file bytes from disk.
//!
//! `discover` in the manifest's place needs no manifest and no local files:
//! it lists the seller's own catalogue, downloads each published resource's
//! bundle through the same lease, and imports the bytes without ever writing
//! them to disk. A draft has no bundle and is skipped.
//!
//! A trailing `measure` argument reads each resource and reports the drain
//! coverage without its files, its product, or any persistence — the
//! kill-gate number when the originals are not on the box.

#![forbid(unsafe_code)]

use std::io::Read as _;

use serde::Deserialize;
use tam_import::{
    import_one, measure_one, record_drain_report, DrainTotals, ImportEntry, ImportRun,
    MeasureTotals, NamedBytes, NoImportFiles,
};
use tam_marketplace::{FetchReason, FirstPartyExport};
use tam_marketplace_tes::{DraftId, GatewayTransport, TesAdapter};
use tam_secrets::Kek;
use tam_types::{InventoryId, OrgId, Timestamp, Uuid};

#[derive(Deserialize)]
struct ManifestRow {
    resource: i64,
    files: Vec<String>,
}

fn read_bytes(path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
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
    reason = "the import is a clock-reading process boundary; time enters the rows as data from here"
)]
fn wall_now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// One entry through the import, absorbed into the run's totals and reported
/// on the way past. Shared by the manifest path and by discover, so the two
/// print the same shape.
async fn import_and_report<A: FirstPartyExport>(
    run: &ImportRun<'_, A>,
    entry: &ImportEntry,
    totals: &mut DrainTotals,
) where
    A::Resource: TryFrom<i64>,
    <A::Resource as TryFrom<i64>>::Error: core::fmt::Display,
{
    match import_one(run, entry).await {
        Ok(report) => {
            totals.absorb(&report);
            eprintln!(
                "{} → {} \"{}\": {}/{} terms mapped, {} new item(s), {} dedup, {}",
                report.resource,
                report.product.0.to_hyphenated(),
                report.title,
                report.terms_mapped,
                report.terms_seen,
                report.raised.new,
                report.raised.already_open,
                report.blocked_by.as_deref().map_or_else(
                    || "projectable".to_owned(),
                    |gate| format!("blocked: {gate}")
                ),
            );
            if !report.unmapped_native_ids.is_empty() {
                eprintln!("    unmapped native ids: {:?}", report.unmapped_native_ids);
            }
            if !report.curriculum.is_empty() {
                eprintln!("    curriculum tags: {:?}", report.curriculum);
            }
        }
        Err(error) => eprintln!("{} FAILED: {error}", entry.resource),
    }
}

async fn record_and_report<A: FirstPartyExport>(
    run: &ImportRun<'_, A>,
    totals: DrainTotals,
) -> Result<(), Box<dyn std::error::Error>> {
    let job = record_drain_report(run, totals).await?;
    eprintln!(
        "imported {} row(s); drain: {} new item(s), {} already open, {} covered, \
         over {} term use(s) of which {} unmapped",
        totals.rows,
        totals.items_new,
        totals.items_already_open,
        totals.terms_covered,
        totals.terms_seen,
        totals.terms_unmapped,
    );
    eprintln!("drain report recorded as job {}", job.0.to_hyphenated());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let db_url = arguments.first().ok_or("missing db url")?;
    let org = org_from_hex(arguments.get(1).ok_or("missing org hex")?)?;
    let gateway = arguments.get(2).ok_or("missing gateway base url")?;
    let kek_path = arguments.get(3).ok_or("missing kek path")?;
    let store_root = arguments.get(4).ok_or("missing object-store root")?;
    let mode = arguments
        .get(5)
        .ok_or("missing manifest path, or the discover keyword in its place")?;
    let discovering = mode == "discover";

    let manifest: Vec<ManifestRow> = if discovering {
        Vec::new()
    } else {
        serde_json::from_slice(&read_bytes(mode)?)?
    };
    let kek = Kek::from_bytes(&read_bytes(kek_path)?)?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(db_url)
        .await?;
    let adapter = TesAdapter::new(
        InventoryId::TesGb,
        GatewayTransport::new(gateway.clone())?,
        NoImportFiles,
    )?;
    let run = ImportRun {
        pool,
        kek,
        store_root: std::path::PathBuf::from(store_root),
        adapter: &adapter,
        org,
        source: InventoryId::TesGb,
        target: InventoryId::TesNz,
        now: wall_now()?,
    };

    if discovering {
        let reason = FetchReason::FirstPartyExport {
            inventory: run.source,
        };
        let catalogue = adapter
            .list_own_resources(&reason)
            .await
            .map_err(|error| format!("the catalogue read failed: {error:?}"))?;
        let published: Vec<_> = catalogue
            .into_iter()
            .filter(|entry| entry.published)
            .collect();
        eprintln!(
            "discovered {} published resource(s); drafts have no bundle and are skipped",
            published.len()
        );
        let mut totals = DrainTotals::default();
        for entry in &published {
            let bundle = match adapter
                .download_resource_bundle(&reason, DraftId(entry.id))
                .await
            {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("{} FAILED to download: {error:?}", entry.id);
                    continue;
                }
            };
            let downloaded = ImportEntry {
                resource: entry.id,
                files: vec![NamedBytes {
                    name: format!("{}-bundle.zip", entry.id),
                    bytes: bundle,
                }],
            };
            import_and_report(&run, &downloaded, &mut totals).await;
        }
        return record_and_report(&run, totals).await;
    }

    if arguments.get(6).map(String::as_str) == Some("measure") {
        let mut totals = MeasureTotals::default();
        for row in &manifest {
            match measure_one(&run, row.resource).await {
                Ok(report) => {
                    totals.absorb(&report);
                    eprintln!(
                        "{} \"{}\": {}/{} terms mapped, {} uncovered, {} unmapped native id(s)",
                        report.resource,
                        report.title,
                        report.terms_mapped,
                        report.terms_seen,
                        report.terms_uncovered,
                        report.unmapped_native_ids.len(),
                    );
                }
                Err(error) => eprintln!("{} FAILED: {error}", row.resource),
            }
        }
        eprintln!(
            "measured {} row(s); {} of {} mapped term(s) uncovered — drain share {}",
            totals.rows,
            totals.terms_uncovered,
            totals.terms_mapped,
            totals.share().map_or_else(
                || "n/a (no terms mapped)".to_owned(),
                |share| format!("{:.1}%", share * 100.0)
            ),
        );
        return Ok(());
    }

    let mut totals = DrainTotals::default();
    for row in manifest {
        let mut files = Vec::new();
        for path in &row.files {
            files.push(NamedBytes {
                name: path.clone(),
                bytes: read_bytes(path)?,
            });
        }
        let entry = ImportEntry {
            resource: row.resource,
            files,
        };
        import_and_report(&run, &entry, &mut totals).await;
    }
    record_and_report(&run, totals).await
}
