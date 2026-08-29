//! The sync worker: the canonicalisation drain on a poll until ctrl-c.
//!
//! It is the first process in the fleet to combine two shapes the tree
//! already has separately -- `tam-worker`'s poll loop holding the broker
//! socket, and `tam-import`'s `tam_app`-role run of `import_one` against a
//! gateway. That combination, and the credential below, are what this
//! process costs; the role is what it buys.
//!
//! The role is `tam_app`, deliberately. It has forced row-level security, so
//! the drain pins one organisation per request and cannot read two tenants'
//! rows in one statement. Granting the engine INSERT on the catalogue was the
//! alternative, and it would hand the cross-tenant BYPASSRLS role authorship
//! of the catalogue, which is what the grant enumeration exists to prevent.
//!
//! Tes source reads need no credential: they ride the broker's gateway, and
//! the lease names its purpose so this process's session and the item pump's
//! coexist on the one connection a tenant has rather than cancelling each
//! other. TPT source reads do need one, because TPT rides a direct transport
//! and skips the broker entirely, so this process is the second holder of the
//! fleet's highest-value credential -- and it applies the same one-tenant pin
//! `tam-worker` does.
//!
//! Usage: tam-sync-worker <app-database-url> <broker-socket> <kek-path> \
//!            <store-root> [poll-ms]
//!
//! Environment: `TAM_TPT_COOKIE_JAR`, `TAM_TPT_AUTHORSHIP` and `TAM_TPT_ORG`,
//! required together for TPT-as-source. Without them a TPT source request is
//! refused before any read rather than driven against the wrong account.

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_engine::broker_client::{request_lease, LeasePurpose};
use tam_import::{ImportRun, NoImportFiles};
use tam_marketplace_tes::{GatewayTransport, TesAdapter};
use tam_secrets::Kek;
use tam_storage::{ConnectionRepo, SyncRequestRepo};
use tam_sync_worker::{drain_request, pending_work, reason_for};
use tam_types::{ConnectionId, InventoryId, Marketplace, OrgId, Timestamp, Uuid};

/// How many requests one tenant contributes to a pass. A batch bound rather
/// than a fairness mechanism: without it one tenant's backlog would hold the
/// pass while every other tenant's request waited behind it.
const REQUESTS_PER_TENANT_PER_PASS: i64 = 8;

const DEFAULT_POLL_MS: u64 = 5_000;

/// The organisation a configured TPT jar speaks for, or nothing.
///
/// The jar is process-global and the request scan is every tenant's, so a TPT
/// source read for any other organisation would be driven against the wrong
/// seller's account -- reading one seller's catalogue into another's product.
/// Refused before the read rather than after it.
struct TptPin {
    org: OrgId,
}

impl TptPin {
    fn speaks_for(&self, org: OrgId) -> bool {
        self.org == org
    }
}

fn load_tpt_pin() -> Option<TptPin> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is the configuration boundary: the organisation the Tpt jar belongs to enters the process here and nowhere else"
    )]
    let raw_org = std::env::var("TAM_TPT_ORG").ok()?;
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is the configuration boundary: the Tpt cookie jar path enters the process here and nowhere else"
    )]
    let jar_path = std::env::var("TAM_TPT_COOKIE_JAR").ok()?;
    let mut jar = String::new();
    if let Err(error) =
        std::fs::File::open(&jar_path).and_then(|mut file| file.read_to_string(&mut jar).map(drop))
    {
        eprintln!(
            "tam-sync-worker: TAM_TPT_COOKIE_JAR is unreadable, Tpt sources refused: {error}"
        );
        return None;
    }
    parse_org(&raw_org).map(|org| TptPin { org })
}

fn parse_org(raw: &str) -> Option<OrgId> {
    let hex: String = raw.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(hex.get(start..start + 2)?, 16).ok()?;
    }
    Some(OrgId(Uuid(bytes)))
}

/// The tenant's linked connection for one marketplace, read through the repo
/// that pins.
///
/// `LeaseRepo::connection_for` runs on the bare pool with no pin because it
/// was written for the BYPASSRLS engine; under forced RLS it returns a clean
/// empty read, so a drain using it would conclude "no linked connection" and
/// silently no-op every sync request forever.
async fn linked_connection(
    connections: &ConnectionRepo,
    org: OrgId,
    marketplace: Marketplace,
) -> Option<ConnectionId> {
    connections
        .list(org)
        .await
        .ok()?
        .into_iter()
        .find(|row| row.marketplace == marketplace && row.state == "linked")
        .map(|row| row.id)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let database_url = args.first().ok_or("missing app database url")?;
    let broker_socket = std::path::PathBuf::from(args.get(1).ok_or("missing broker socket")?);
    let kek_path = args.get(2).ok_or("missing kek path")?;
    let store_root = std::path::PathBuf::from(args.get(3).ok_or("missing store root")?);
    let poll_ms: u64 = match args.get(4) {
        Some(raw) => raw.parse()?,
        None => DEFAULT_POLL_MS,
    };

    let mut kek_bytes = Vec::new();
    std::fs::File::open(kek_path)?.read_to_end(&mut kek_bytes)?;
    let kek = Kek::from_bytes(&kek_bytes)?;
    let pool = sqlx::PgPool::connect(database_url).await?;
    let requests = SyncRequestRepo::new(pool.clone());
    let connections = ConnectionRepo::new(pool.clone());
    let tpt = load_tpt_pin();

    loop {
        let work = pending_work(&requests, REQUESTS_PER_TENANT_PER_PASS).await?;
        for (org, pending) in work {
            for request in pending {
                let Some(record) = requests.get(org, request).await? else {
                    continue;
                };
                if record.source.marketplace() == Marketplace::Tpt
                    && !tpt.as_ref().is_some_and(|pin| pin.speaks_for(org))
                {
                    // Refused before any read, for the same reason the item
                    // pump refuses a foreign Tpt item: a process-global jar
                    // speaks for one seller and this request is another's.
                    requests
                        .record_failure(
                            org,
                            request,
                            "this process holds no Tpt credential for that organisation",
                            now(),
                        )
                        .await?;
                    continue;
                }
                if let Err(error) = drain_one(
                    &requests,
                    &connections,
                    &pool,
                    &kek,
                    &store_root,
                    &broker_socket,
                    org,
                    record.source,
                    record.target,
                    request,
                )
                .await
                {
                    eprintln!("tam-sync-worker: request {request:?} failed: {error}");
                }
            }
        }
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_millis(poll_ms)) => {}
            result = tokio::signal::ctrl_c() => {
                result?;
                return Ok(());
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the drain's per-request context is genuinely this wide and bundling it would name a struct nothing else constructs"
)]
async fn drain_one(
    requests: &SyncRequestRepo,
    connections: &ConnectionRepo,
    pool: &sqlx::PgPool,
    kek: &Kek,
    store_root: &std::path::Path,
    broker_socket: &std::path::Path,
    org: OrgId,
    source: InventoryId,
    target: InventoryId,
    request: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    let marketplace = source.marketplace();
    let Some(connection) = linked_connection(connections, org, marketplace).await else {
        requests
            .record_failure(org, request, "no linked connection for the source", now())
            .await?;
        return Ok(());
    };
    // The drain's own broker identity. Without the purpose the lease would
    // abort the item pump's gateway for this tenant mid-write.
    let lease = request_lease(
        broker_socket,
        org,
        connection,
        marketplace,
        LeasePurpose::Drain,
    )
    .await?;
    let adapter = TesAdapter::new(
        source,
        GatewayTransport::new(lease.endpoint.clone())?,
        NoImportFiles,
    )?;
    let run = ImportRun {
        pool: pool.clone(),
        kek: kek.clone(),
        store_root: store_root.to_path_buf(),
        adapter: &adapter,
        org,
        source,
        target,
        now: now(),
    };
    let report = drain_request(requests, &run, request, &reason_for(source)).await?;
    println!(
        "tam-sync-worker: request {request:?}: {} canonicalised, {} already done, {} failed",
        report.canonicalised, report.skipped, report.failed
    );
    Ok(())
}

fn now() -> Timestamp {
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is the wall clock's boundary: the drain's instant is read here and threaded onward"
    )]
    Timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX),
    )
}
