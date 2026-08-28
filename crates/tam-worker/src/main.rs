//! The automation worker: the item pump — lease, seed, drive, settle — plus
//! the maintenance passes (steal expired leases, run the fleet breaker) on a
//! jittered poll until ctrl-c. The outbox drain lives in tam-server per the
//! design's single-home line; this process no longer duplicates it.
//!
//! Per item: the projection gates the item (a blocked projection parks it
//! behind the queue items it just raised), the broker leases a gateway
//! endpoint so this process never holds a credential, the adapter renders
//! the field set it will submit, and the M1d driver runs the machine against
//! that adapter with the fenced attempt and the read-back verification it
//! was built with.
//!
//! Usage: tam-worker <engine-database-url> <worker-name> <broker-socket> \
//!            <kek-path> <store-root> [poll-ms]

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_engine::breaker::run_breaker;
use tam_engine::broker_client::request_lease;
use tam_engine::driver::{run_item, DriverContext, NowSource, RunVerdict};
use tam_engine::seed::{prepare_item, seed_for_removal, seed_from_projection, ItemPreparation};
use tam_engine::Pause;
use tam_marketplace_tes::{GatewayTransport, TesAdapter};
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    BlobRepo, HaltRepo, JobRepo, LeaseRepo, LeasedItem, PipelineFileSource, RateBudgetRepo,
    WriteAttemptRepo,
};
use tam_types::{Marketplace, Timestamp};
use tokio_util::sync::CancellationToken;

const DEFAULT_POLL_MS: u64 = 5_000;
const LEASE_TTL_SECS: i64 = 300;
/// A projection-blocked item parks for a day; a drained queue un-parks it
/// into a clean retry on the next steal pass after expiry.
const BLOCKED_PARK_MS: i64 = 24 * 60 * 60 * 1000;

/// The real wait the driver's verification poll takes between reads. The
/// engine holds no timer by design, so the sleep enters here, at the process
/// boundary, exactly as the wall clock does.
struct SleepingPause;

impl Pause for SleepingPause {
    fn pause(&self, ms: u32) -> impl core::future::Future<Output = ()> + Send {
        tokio::time::sleep(core::time::Duration::from_millis(u64::from(ms)))
    }
}

/// Wall-clock enters here, at the process boundary, as the design's
/// time-as-data rule requires. A clock before the epoch saturates to zero,
/// which reads as "everything expired" — fail closed, not fail weird.
struct WallClock;

impl NowSource for WallClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is a clock-reading process boundary; time enters the engine as data from here"
    )]
    fn now(&self) -> Timestamp {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        Timestamp(i64::try_from(millis).unwrap_or(0))
    }
}

fn load_kek(path: &str) -> Result<Kek, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(Kek::from_bytes(&bytes)?)
}

struct Pump {
    pool: sqlx::PgPool,
    leases: LeaseRepo,
    halts: HaltRepo,
    attempts: WriteAttemptRepo,
    budgets: RateBudgetRepo,
    broker_socket: std::path::PathBuf,
    kek: Kek,
    store_root: std::path::PathBuf,
    cancel: CancellationToken,
}

impl Pump {
    /// Drives one leased item to wherever it goes; every refusal path leaves
    /// the lease to expire into the stealer, which is the stall bias.
    async fn pump_item(&self, worker: &str, item: &LeasedItem) {
        let now = WallClock.now();
        let (operation, projected) = match prepare_item(&self.pool, item, now).await {
            Ok(ItemPreparation::Ready {
                operation,
                projected,
            }) => (operation, projected),
            Ok(ItemPreparation::Blocked { gate, raised }) => {
                let until = Timestamp(now.0.saturating_add(BLOCKED_PARK_MS));
                match self.leases.park(&item.lease_ref(), gate, until).await {
                    Ok(()) => eprintln!(
                        "tam-worker {worker}: item {:?} parked on {gate} \
                         ({} item(s) raised, {} already open)",
                        item.item, raised.new, raised.already_open
                    ),
                    Err(error) => {
                        eprintln!("tam-worker {worker}: park failed: {error}");
                    }
                }
                return;
            }
            Err(error) => {
                eprintln!("tam-worker {worker}: preparation failed, lease left to expire: {error}");
                return;
            }
        };

        if item.inventory.marketplace() != Marketplace::Tes {
            eprintln!(
                "tam-worker {worker}: no adapter for {:?} yet, lease left to expire",
                item.inventory
            );
            return;
        }
        let Some(connection) = (match self.leases.connection_for(item.org, item.inventory).await {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("tam-worker {worker}: connection lookup failed: {error}");
                return;
            }
        }) else {
            eprintln!(
                "tam-worker {worker}: no linked connection for {:?}, lease left to expire",
                item.inventory
            );
            return;
        };
        let gateway = match request_lease(
            &self.broker_socket,
            item.org,
            connection,
            Marketplace::Tes,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                eprintln!("tam-worker {worker}: broker lease refused: {error}");
                return;
            }
        };
        let transport = match GatewayTransport::new(gateway.endpoint.clone()) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("tam-worker {worker}: transport build failed: {error}");
                return;
            }
        };
        let files = PipelineFileSource::new(
            BlobRepo::new(
                self.pool.clone(),
                LocalObjectStore::new(self.store_root.clone()),
                self.kek.clone(),
            ),
            item.org,
            self.pool.clone(),
        );
        let adapter = match TesAdapter::new(item.inventory, transport, files) {
            Ok(adapter) => adapter,
            Err(error) => {
                eprintln!("tam-worker {worker}: {error}");
                return;
            }
        };
        // A removal renders nothing, so it never reaches the adapter's
        // projection: the listing is being taken down rather than described.
        let seed = match projected.as_ref().map_or_else(
            || Ok(seed_for_removal(item, &operation)),
            |projected| seed_from_projection(&adapter, item, projected),
        ) {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("tam-worker {worker}: seed failed, lease left to expire: {error}");
                return;
            }
        };
        let ctx = DriverContext {
            adapter: &adapter,
            leases: &self.leases,
            halts: &self.halts,
            attempts: &self.attempts,
            budgets: &self.budgets,
            pool: &self.pool,
            clock: &WallClock,
            cancel: &self.cancel,
            pause: &SleepingPause,
        };
        match run_item(&ctx, item, seed).await {
            Ok(RunVerdict::Settled(outcome)) => {
                eprintln!(
                    "tam-worker {worker}: item {:?} settled {outcome:?}",
                    item.item
                );
            }
            Ok(RunVerdict::Parked) => {
                eprintln!("tam-worker {worker}: item {:?} parked", item.item);
            }
            Ok(RunVerdict::Abandoned { reason }) => {
                eprintln!(
                    "tam-worker {worker}: item {:?} abandoned: {reason}",
                    item.item
                );
            }
            Err(error) => {
                eprintln!("tam-worker {worker}: run failed, lease left to expire: {error}");
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    const USAGE: &str = "usage: tam-worker <engine-database-url> <worker-name> \
                         <broker-socket> <kek-path> <store-root> [poll-ms]";
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let database_url = arguments.first().ok_or(USAGE)?;
    let worker_name = arguments.get(1).ok_or(USAGE)?;
    let broker_socket = std::path::PathBuf::from(arguments.get(2).ok_or(USAGE)?);
    let kek = load_kek(arguments.get(3).ok_or(USAGE)?)?;
    let store_root = std::path::PathBuf::from(arguments.get(4).ok_or(USAGE)?);
    let poll_ms: u64 = arguments
        .get(5)
        .map_or(Ok(DEFAULT_POLL_MS), |raw| raw.parse())?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await?;
    let leases = LeaseRepo::new(pool.clone());
    let jobs = JobRepo::new(pool.clone());
    let halts = HaltRepo::new(pool.clone());

    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    let ctrl_c = async move {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("tam-worker: cannot wait on ctrl-c, stopping now: {error}");
        }
        stopper.cancel();
    };
    eprintln!("tam-worker {worker_name}: item pump live, maintenance every {poll_ms}ms");

    let pump = Pump {
        pool: pool.clone(),
        leases: LeaseRepo::new(pool.clone()),
        halts: HaltRepo::new(pool.clone()),
        attempts: WriteAttemptRepo::new(pool.clone()),
        budgets: RateBudgetRepo::new(pool.clone()),
        broker_socket,
        kek,
        store_root,
        cancel: cancel.clone(),
    };

    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            () = &mut ctrl_c => break,
            () = tokio::time::sleep(core::time::Duration::from_millis(poll_ms)) => {}
        }
        if cancel.is_cancelled() {
            break;
        }
        let now = WallClock.now();
        let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
        match leases.expire_and_steal(now, attempts_max).await {
            Ok(0) => {}
            Ok(stolen) => eprintln!("tam-worker {worker_name}: stole {stolen} expired leases"),
            Err(error) => eprintln!("tam-worker {worker_name}: steal failed: {error}"),
        }
        match run_breaker(&jobs, &halts, now).await {
            Ok(report) if !report.tripped.is_empty() => {
                eprintln!(
                    "tam-worker {worker_name}: BREAKER TRIPPED {:?}",
                    report.tripped
                );
            }
            Ok(_) => {}
            Err(error) => eprintln!("tam-worker {worker_name}: breaker failed: {error}"),
        }
        // The item pump: one at a time, until the queue is dry this pass.
        // Per-tenant concurrency is one to two by design, and the per-tenant
        // mutex serialises deeper anyway.
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match leases
                .acquire(worker_name, WallClock.now(), LEASE_TTL_SECS)
                .await
            {
                Ok(Some(item)) => pump.pump_item(worker_name, &item).await,
                Ok(None) => break,
                Err(error) => {
                    eprintln!("tam-worker {worker_name}: acquire failed: {error}");
                    break;
                }
            }
        }
    }
    eprintln!("tam-worker {worker_name}: stopped cleanly");
    Ok(())
}
