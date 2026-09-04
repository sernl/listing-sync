//! The operator import: the founder's catalogue in, canonical products and
//! their target mappings out, with the drain report printed per row and in
//! total. The file bytes come from disk and no marketplace read happens here,
//! so this process holds no marketplace session for a source it reads.
//!
//! Usage: tam-import <db-url> <org-hex> <kek-path> <store-root> <manifest.json>
//!
//! The manifest states its own route: { "source": .., "target": ..,
//! "rows": [ { "resource": <numeric id>, "files": ["/path/to/original", ...] } ] },
//! taking the file bytes from disk. That is what lets this path carry a TPT
//! source — the bytes are already there, so no seller-download capture is
//! needed to run one — and a TPT source needs `TAM_TPT_COOKIE_JAR`.
//!
//! TPT is the only source this path serves, because it carries one adapter and
//! reads no marketplace. A source whose catalogue has to be read is read on the
//! seller's own device under D1, which is where
//! `docs/notes/design/migration-file-routing.md` routes it.

#![forbid(unsafe_code)]

use std::io::Read as _;

use serde::Deserialize;
use tam_import::{
    import_one, record_drain_report, DrainTotals, ImportEntry, ImportRun, NamedBytes, NoImportFiles,
};
use tam_marketplace::{FirstPartyExport, InstantPause};
use tam_marketplace_tpt::{ReqwestTransport, TptAdapter, TptSession};
use tam_secrets::Kek;
use tam_types::{InventoryId, Marketplace, OrgId, Timestamp, Uuid};

#[derive(Deserialize)]
struct ManifestRow {
    resource: i64,
    files: Vec<String>,
}

/// The manifest: the route it states and the rows it carries.
///
/// One shape, because the other could only ever be refused. A bare array was
/// the original form and meant Tes GB into Tes NZ, which this binary no longer
/// serves, so it is gone rather than kept as a spelling whose only outcome is
/// the refusal below.
#[derive(Deserialize)]
struct Manifest {
    source: InventoryId,
    target: InventoryId,
    rows: Vec<ManifestRow>,
}

/// The seller's own TPT credential, read here because this is the
/// configuration boundary.
fn tpt_session() -> Result<TptSession, Box<dyn std::error::Error>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the operator import is the configuration boundary: the Tpt cookie jar path enters the process here and nowhere else"
    )]
    let jar_path =
        std::env::var("TAM_TPT_COOKIE_JAR").map_err(|_| "a TPT source needs TAM_TPT_COOKIE_JAR")?;
    let mut jar = String::new();
    std::fs::File::open(&jar_path)?.read_to_string(&mut jar)?;
    Ok(TptSession::from_netscape_jar(&jar)?)
}

/// The manifest drain, over the adapter the route named. Generic because the
/// loop asks nothing of the adapter beyond what `import_one` already does.
async fn drain_manifest<A: FirstPartyExport>(
    run: &ImportRun<'_, A>,
    rows: &[ManifestRow],
) -> Result<(), Box<dyn std::error::Error>>
where
    A::Resource: TryFrom<i64>,
    <A::Resource as TryFrom<i64>>::Error: core::fmt::Display,
{
    let mut totals = DrainTotals::default();
    for row in rows {
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
        import_and_report(run, &entry, &mut totals).await;
    }
    record_and_report(run, totals).await
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
/// on the way past.
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
    let kek_path = arguments.get(2).ok_or("missing kek path")?;
    let store_root = arguments.get(3).ok_or("missing object-store root")?;
    let manifest_path = arguments.get(4).ok_or("missing manifest path")?;

    let manifest: Manifest = serde_json::from_slice(&read_bytes(manifest_path)?)?;
    let (source, target) = (manifest.source, manifest.target);
    if source.marketplace() != Marketplace::Tpt {
        return Err(format!(
            "this path takes a TPT source and {source:?} is not one. It carries one \
             adapter and reads no marketplace: the bytes come from the manifest's own \
             files. A source whose catalogue has to be read is read on the seller's \
             device instead, which is where docs/notes/design/migration-file-routing.md \
             routes it."
        )
        .into());
    }
    let kek = Kek::from_bytes(&read_bytes(kek_path)?)?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(db_url)
        .await?;
    let session = tpt_session()?;
    let adapter = TptAdapter::new(
        ReqwestTransport::new(&session)?,
        NoImportFiles,
        InstantPause,
    );
    let run = ImportRun {
        pool,
        kek,
        store_root: std::path::PathBuf::from(store_root),
        adapter: &adapter,
        org,
        source,
        target,
        now: wall_now()?,
    };
    drain_manifest(&run, &manifest.rows).await
}
