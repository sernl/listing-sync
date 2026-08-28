//! The operator's supervised live read of their own TPT catalogue.
//!
//! Usage, from the repo root with the operator's cookie jar:
//!   cargo run -p tam-marketplace-tpt --example live_read -- [jar-path]
//!
//! Read only: it walks `MyProductListings` and prints. It builds no
//! submission, posts no form and touches no write path.

use std::io::Read as _;

use tam_marketplace::{FetchReason, FileContent, FileSource, FileSourceError};
use tam_marketplace_tpt::read_model::TptPrice;
use tam_marketplace_tpt::{InstantPause, ReqwestTransport, TptAdapter, TptSession};
use tam_types::{FileId, InventoryId};

const DEFAULT_JAR: &str = "probes/local/tpt-cookies.jar";
const SHOWN: usize = 10;

/// A file source no read flow reaches; the adapter takes one because its
/// write half needs one.
struct NoFiles;

impl FileSource for NoFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Missing(file)))
    }
}

fn money(price: &TptPrice) -> String {
    let major = price.minor_units.checked_div(100).unwrap_or_default();
    let minor = price.minor_units.checked_rem(100).unwrap_or_default();
    format!("{}{major}.{:02}", price.symbol, minor.unsigned_abs())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let jar_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_JAR.to_owned());
    let mut jar = String::new();
    std::fs::File::open(&jar_path)?.read_to_string(&mut jar)?;
    let session = TptSession::from_netscape_jar(&jar)?;
    let transport = ReqwestTransport::new(&session)?;
    let adapter = TptAdapter::new(transport, NoFiles, InstantPause);

    let reason = FetchReason::FirstPartyExport {
        inventory: InventoryId::Tpt,
    };
    let entries = adapter
        .list_own_resources(&reason)
        .await
        .map_err(|error| format!("catalogue read failed: {error:?}"))?;

    println!("catalogue: {} products", entries.len());
    for entry in entries.iter().take(SHOWN) {
        println!(
            "  {} | {} | {} | status={} | kind={} | type={} | free={}",
            entry.id.0,
            entry.name,
            money(&entry.price),
            entry.status.as_deref().unwrap_or("-"),
            entry.kind.as_deref().unwrap_or("-"),
            entry.item_type.as_deref().unwrap_or("-"),
            entry.is_free,
        );
    }
    Ok(())
}
