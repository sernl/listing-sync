//! The scheduled analytics capture, one pass per invocation: a deployment
//! timer owns the schedule and this process's exit status says whether the
//! pass was whole. Nothing here reads a schedule, and nothing here loops; a
//! poll interval is not a cadence a seller can be told, and the pass is
//! tenant-driven where the item pump is item-driven.
//!
//! Per tenant: read the bound TPT mappings, lease a session under the
//! analytics purpose, ask the gateway for each captured metric over batches of
//! the seller's own resource ids, and write one snapshot row per listing per
//! metric. A tenant with no bound TPT listing takes no lease at all.
//!
//! Two roles, deliberately. The capture runs as `tam_app`, which has forced
//! row-level security and pins one organisation per statement, so it cannot
//! read two tenants' listings in one query. Retention is the opposite shape —
//! one cross-tenant sweep — so it runs on the `tam_engine` pool that holds the
//! DELETE grant, exactly like the job-event pruner it sits beside.
//!
//! Usage: tam-analytics <app-database-url> <engine-database-url> <broker-socket>

#![forbid(unsafe_code)]

use tam_analytics::{capture_listings, retention_cutoff, SNAPSHOT_PRUNE_BATCH};
use tam_engine::broker_client::{request_lease, LeasePurpose};
use tam_marketplace::{FileContent, FileSource, FileSourceError, InstantPause};
use tam_marketplace_tpt::{GatewayTransport, TptAdapter};
use tam_storage::{AnalyticsRepo, ConnectionRepo, MappingRepo, OrgRepo, PruneRepo, StorageError};
use tam_types::{ConnectionId, FileId, InventoryId, Marketplace, OrgId, Timestamp};

const USAGE: &str = "usage: tam-analytics <app-database-url> <engine-database-url> <broker-socket>";

/// A capture never uploads; the adapter's file seam is satisfied by a source
/// that refuses everything.
struct NoFiles;

impl FileSource for NoFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Missing(file)))
    }
}

/// Wall-clock enters here, at the process boundary, as the design's
/// time-as-data rule requires. Read once for the whole pass so every row it
/// writes carries the same instant.
#[expect(
    clippy::disallowed_methods,
    reason = "the capture is a clock-reading process boundary; the pass's instant is read here and threaded onward"
)]
fn now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// The tenant's linked TPT connection, read through the repo that pins.
///
/// `LeaseRepo::connection_for` is the engine's read and runs unpinned; under
/// this process's forced row-level security it would answer empty and the
/// capture would conclude every tenant is unlinked.
async fn linked_tpt_connection(
    connections: &ConnectionRepo,
    org: OrgId,
    at: Timestamp,
) -> Result<Option<ConnectionId>, StorageError> {
    Ok(connections
        .list(org, at)
        .await?
        .into_iter()
        .find(|row| row.marketplace == Marketplace::Tpt && row.state == "linked")
        .map(|row| row.id))
}

/// One tenant's capture. `Ok(false)` is a pass that reached the marketplace
/// but did not get everything it asked for.
async fn capture_tenant(
    pool: &sqlx::PgPool,
    broker_socket: &std::path::Path,
    org: OrgId,
    at: Timestamp,
) -> Result<bool, Box<dyn std::error::Error>> {
    let listings = MappingRepo::new(pool.clone())
        .bound_listings(org, InventoryId::Tpt)
        .await?;
    if listings.is_empty() {
        return Ok(true);
    }
    let Some(connection) =
        linked_tpt_connection(&ConnectionRepo::new(pool.clone()), org, at).await?
    else {
        eprintln!(
            "tam-analytics: organisation {} has {} bound Tpt listings and no linked Tpt \
             connection; nothing captured",
            org.0.to_hyphenated(),
            listings.len()
        );
        return Ok(false);
    };
    let lease = request_lease(
        broker_socket,
        org,
        connection,
        Marketplace::Tpt,
        LeasePurpose::Analytics,
    )
    .await?;
    let adapter = TptAdapter::new(
        GatewayTransport::new(lease.endpoint.clone(), &lease.token)?,
        NoFiles,
        InstantPause,
    );
    // The library speaks the shared vocabulary now, so the two plain data
    // types convert here rather than the capture reaching for a storage type
    // it must not depend on. This is the only place either shape crosses.
    let wire: Vec<tam_engine_driver::vocabulary::BoundListing> =
        listings.iter().cloned().map(Into::into).collect();
    let outcome = capture_listings(&adapter, &wire, at).await;
    let snapshots: Vec<tam_storage::MetricSnapshot> =
        outcome.snapshots.into_iter().map(Into::into).collect();
    let written = AnalyticsRepo::new(pool.clone())
        .record(org, &snapshots)
        .await?;
    println!(
        "tam-analytics: organisation {}: {} listings, {} snapshots written",
        org.0.to_hyphenated(),
        listings.len(),
        written
    );
    for failure in &outcome.failures {
        eprintln!(
            "tam-analytics: organisation {}: {failure}",
            org.0.to_hyphenated()
        );
    }
    Ok(outcome.failures.is_empty())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let database_url = arguments.first().ok_or(USAGE)?;
    let engine_url = arguments.get(1).ok_or(USAGE)?;
    let broker_socket = std::path::PathBuf::from(arguments.get(2).ok_or(USAGE)?);

    let at = now()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await?;

    let mut whole = true;
    for org in OrgRepo::new(pool.clone()).tenants().await? {
        match capture_tenant(&pool, &broker_socket, org, at).await {
            Ok(complete) => whole &= complete,
            Err(error) => {
                // One tenant's fault does not end the pass: the remaining
                // tenants' figures are independent of it, and the exit status
                // below still reports that this run was not whole.
                eprintln!(
                    "tam-analytics: organisation {} failed: {error}",
                    org.0.to_hyphenated()
                );
                whole = false;
            }
        }
    }

    let engine = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(engine_url)
        .await?;
    let erased = PruneRepo::new(engine)
        .prune_snapshots(retention_cutoff(at), SNAPSHOT_PRUNE_BATCH)
        .await?;
    if erased > 0 {
        println!("tam-analytics: pruned {erased} snapshots past the retention window");
    }

    if whole {
        Ok(())
    } else {
        Err(
            "the capture pass did not complete for every tenant; the timer unit state is the alert"
                .into(),
        )
    }
}
