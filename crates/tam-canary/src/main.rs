//! The scheduled structural probe, one run per invocation: the systemd timer
//! owns the schedule and this process's exit status is the alert. It probes
//! the draft form on the founder's own account for each Tes inventory; drift
//! raises the fleet halt for that inventory before any customer meets it.
//!
//! Usage: tam-canary <engine-database-url> <cookie-jar-path>

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_engine::canary::{probe, ProbeVerdict};
use tam_marketplace::{FileContent, FileSource, FileSourceError, FormId};
use tam_marketplace_tes::{ReqwestTransport, TesAdapter, TesSession};
use tam_storage::HaltRepo;
use tam_types::{FileId, InventoryId, Timestamp, Uuid};

/// The canary never uploads; the adapter's file seam is satisfied by a
/// source that refuses everything.
struct NoFiles;

impl FileSource for NoFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Missing(file)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let database_url = arguments
        .first()
        .ok_or("usage: tam-canary <engine-database-url> <cookie-jar-path>")?;
    let jar_path = arguments
        .get(1)
        .ok_or("usage: tam-canary <engine-database-url> <cookie-jar-path>")?;

    let mut jar = String::new();
    std::fs::File::open(jar_path)?.read_to_string(&mut jar)?;
    let session = TesSession::from_netscape_jar(&jar)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await?;
    let halts = HaltRepo::new(pool);
    // Wall-clock enters here, at the process boundary, as the design's
    // time-as-data rule requires; expect-attributed because this binary is
    // the driver that legitimately reads the clock.
    #[expect(
        clippy::disallowed_methods,
        reason = "the canary is a clock-reading process boundary; time enters the engine as data from here"
    )]
    let now = Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?);

    let mut failed = false;
    for (inventory, form_seed) in [(InventoryId::Tes, 0x01u8)] {
        let transport = ReqwestTransport::new(&session)?;
        let adapter = TesAdapter::new(inventory, transport, NoFiles)?;
        let verdict = probe(&adapter, &halts, FormId(Uuid([form_seed; 16])), now).await?;
        match verdict {
            ProbeVerdict::Clean => eprintln!("canary {inventory:?}: clean"),
            ProbeVerdict::Drifted { detail } => {
                eprintln!("canary {inventory:?}: DRIFTED and halted — {detail}");
                failed = true;
            }
            ProbeVerdict::Failed { detail } => {
                eprintln!("canary {inventory:?}: probe failed — {detail}");
                failed = true;
            }
        }
    }
    if failed {
        return Err("a canary probe drifted or failed; the timer unit state is the alert".into());
    }
    Ok(())
}
