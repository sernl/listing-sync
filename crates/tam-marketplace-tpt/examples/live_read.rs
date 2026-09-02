//! The operator's supervised live read of their own TPT catalogue.
//!
//! Usage, from the repo root with the operator's cookie jar:
//!   cargo run -p tam-marketplace-tpt --example live_read -- [jar-path]
//!   cargo run -p tam-marketplace-tpt --example live_read -- [jar-path] \
//!       --id <product-id>
//!   cargo run -p tam-marketplace-tpt --example live_read -- [jar-path] \
//!       --filter <substring>
//!
//! Read only: it walks `MyProductListings`, or reads one product through the
//! by-id route, and prints. It builds no submission, posts no form and touches
//! no write path.
//!
//! Without a flag it prints the catalogue count and the first [`SHOWN`] rows.
//! `--id` reads one product and prints its lifecycle and the fields it carries,
//! which is what an assertion that a named listing is still there needs and
//! what a catalogue count cannot give. `--filter` prints every row whose name
//! carries the substring alongside the catalogue total, which is what a sweep
//! for disposable titles reads; a ten-row window of a 154-row catalogue cannot
//! answer either question.

use std::io::Read as _;

use tam_marketplace::{
    FetchReason, FileContent, FileSource, FileSourceError, ListingLocator, MarketplaceAdapter as _,
};
use tam_marketplace_tpt::read_model::TptPrice;
use tam_marketplace_tpt::{InstantPause, ProductId, ReqwestTransport, TptAdapter, TptSession};
use tam_types::{FileId, InventoryId, Timestamp};

const DEFAULT_JAR: &str = "probes/local/tpt-cookies.jar";
const SHOWN: usize = 10;

type Failure = Box<dyn std::error::Error>;
type Adapter = TptAdapter<ReqwestTransport, NoFiles, InstantPause>;

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

#[expect(
    clippy::disallowed_methods,
    reason = "the live runner is a clock-reading process boundary; time enters the adapter as data from here"
)]
fn wall_now() -> Result<Timestamp, Failure> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

fn money(price: &TptPrice) -> String {
    let major = price.minor_units.checked_div(100).unwrap_or_default();
    let minor = price.minor_units.checked_rem(100).unwrap_or_default();
    format!("{}{major}.{:02}", price.symbol, minor.unsigned_abs())
}

/// What the operator asked for.
///
/// `--id` and `--filter` are alternatives rather than companions: one names a
/// product and reads it through the by-id route, the other names a substring
/// and walks the catalogue, and a run given both would leave which read the
/// exit status is about unstated.
enum Request {
    Catalogue { filter: Option<String> },
    Product(ProductId),
}

/// A flag's value, distinguishing "not asked for" from "asked for and left
/// empty": a `--id` whose value is missing must not read as a catalogue walk.
fn valued<'a>(arguments: &'a [String], name: &str) -> Result<Option<&'a String>, Failure> {
    let Some(at) = arguments.iter().position(|argument| argument == name) else {
        return Ok(None);
    };
    arguments
        .get(at.saturating_add(1))
        .filter(|argument| !argument.starts_with("--"))
        .map(Some)
        .ok_or_else(|| format!("{name} needs a value").into())
}

fn requested(arguments: &[String]) -> Result<Request, Failure> {
    match (valued(arguments, "--id")?, valued(arguments, "--filter")?) {
        (Some(_), Some(_)) => Err("pass one of --id or --filter, never both".into()),
        (Some(raw), None) => Ok(Request::Product(ProductId(
            raw.parse::<u64>()
                .map_err(|error| format!("{raw:?} is not a TPT product id: {error}"))?,
        ))),
        (None, filter) => Ok(Request::Catalogue {
            filter: filter.cloned(),
        }),
    }
}

/// The jar stays the leading positional argument it has always been, so the
/// flagless invocation and every recorded one keep working; a run that names
/// no jar takes [`DEFAULT_JAR`].
fn jar_path(arguments: &[String]) -> &str {
    arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .map_or(DEFAULT_JAR, String::as_str)
}

async fn read_catalogue(adapter: &Adapter, filter: Option<&str>) -> Result<(), Failure> {
    let entries = adapter
        .list_own_resources(&FetchReason::FirstPartyExport {
            inventory: InventoryId::Tpt,
        })
        .await
        .map_err(|error| format!("catalogue read failed: {error:?}"))?;
    println!("catalogue: {} products", entries.len());
    let shown: Vec<_> = match filter {
        Some(needle) => entries
            .iter()
            .filter(|entry| entry.name.contains(needle))
            .collect(),
        None => entries.iter().take(SHOWN).collect(),
    };
    for entry in &shown {
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
    if let Some(needle) = filter {
        println!(
            "filter {needle:?}: {} of {} products carry it",
            shown.len(),
            entries.len()
        );
    }
    Ok(())
}

/// One product, read through the by-id route the engine's own read-back takes
/// rather than by scanning the catalogue for it.
///
/// `Absent` is an observation and not a failed read: `MyProductListings` is
/// the only witness TPT offers and the page that answered spoke for the whole
/// catalogue, so a product it does not carry is a product TPT does not hold.
async fn read_product(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<(), Failure> {
    let observed = adapter
        .read_back(
            ListingLocator::Durable(product.remote()),
            FetchReason::FirstPartyExport {
                inventory: InventoryId::Tpt,
            },
            now,
        )
        .await
        .map_err(|error| format!("the read of {product} failed: {error:?}"))?;
    println!("product {product}");
    println!("  lifecycle {:?}", observed.lifecycle);
    for (key, value) in &observed.fields {
        println!("  {key:?} = {value}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let request = requested(&arguments)?;
    let mut jar = String::new();
    std::fs::File::open(jar_path(&arguments))?.read_to_string(&mut jar)?;
    let session = TptSession::from_netscape_jar(&jar)?;
    let transport = ReqwestTransport::new(&session)?;
    let adapter = TptAdapter::new(transport, NoFiles, InstantPause);

    match request {
        Request::Catalogue { filter } => read_catalogue(&adapter, filter.as_deref()).await,
        Request::Product(product) => read_product(&adapter, product, wall_now()?).await,
    }
}
