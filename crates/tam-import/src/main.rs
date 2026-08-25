//! The operator import: the founder's catalogue in, canonical products and
//! their target mappings out, with the drain report printed per row and in
//! total. Reads travel through the broker's gateway (request a lease over
//! the broker socket first; the endpoint it prints is the base URL here), so
//! this process never holds a marketplace credential.
//!
//! Usage: tam-import <db-url> <org-hex> <gateway-base> <kek-path> \
//!            <store-root> <manifest.json>
//!
//! The manifest is a JSON array of { "resource": <numeric id>,
//! "files": ["/path/to/original", ...] } — customer zero's file bytes come
//! from disk, per the plan's uncaptured-endpoint boundary.

#![forbid(unsafe_code)]

use std::io::Read as _;

use serde::Deserialize;
use tam_import::{import_one, ImportEntry, ImportRun, NamedBytes, NoImportFiles};
use tam_marketplace_tes::{GatewayTransport, TesAdapter};
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let db_url = arguments.first().ok_or("missing db url")?;
    let org = org_from_hex(arguments.get(1).ok_or("missing org hex")?)?;
    let gateway = arguments.get(2).ok_or("missing gateway base url")?;
    let kek_path = arguments.get(3).ok_or("missing kek path")?;
    let store_root = arguments.get(4).ok_or("missing object-store root")?;
    let manifest_path = arguments.get(5).ok_or("missing manifest path")?;

    let manifest: Vec<ManifestRow> = serde_json::from_slice(&read_bytes(manifest_path)?)?;
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

    let mut total_terms = 0usize;
    let mut total_new = 0u64;
    let mut total_rows = 0usize;
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
        match import_one(&run, &entry).await {
            Ok(report) => {
                total_terms += report.terms_seen;
                total_new += report.raised.new;
                total_rows += 1;
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
            Err(error) => eprintln!("{} FAILED: {error}", row.resource),
        }
    }
    eprintln!(
        "imported {total_rows} row(s); drain: {total_new} new item(s) over {total_terms} term uses"
    );
    Ok(())
}
