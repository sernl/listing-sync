//! The internet-facing HTTP process: an address, a listener, a pool, and the
//! router `tam-api` builds. Every route, extractor and error mapping lives in
//! the library, so this binary holds nothing a test would want to reach.
//!
//! Given an engine-role url it also hosts the two service loops the design
//! puts in this process: the outbox drainer and the job-event pruner.
//!
//! Usage: tam-server <db-url> [bind-addr] [--broker-socket <path>] [--engine-db-url <url>] [--ui-dir <path>] [--disclose-internals]

#![forbid(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tam_api::{AppState, Config, Disclosure};
use tam_engine::outbox::{drain, LoggingDeliverer};
use tam_storage::{OutboxRepo, PruneRepo};
use tam_types::Timestamp;
use tokio_util::sync::CancellationToken;

/// Loopback rather than `0.0.0.0`, so a development run is not reachable off
/// the machine by forgetting an argument.
const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

/// The development opt-in that discloses fault internals in responses. A flag
/// rather than a build property, because this workspace ships
/// `debug-assertions = true` in release, so no `cfg` can tell production
/// apart; redaction is the default in every build.
const DISCLOSE_FLAG: &str = "--disclose-internals";

/// The credential broker's unix socket; without it the revoke endpoint
/// answers 503 rather than pretending.
const BROKER_FLAG: &str = "--broker-socket";

/// The engine-role url the service loops run on. They cross tenants — the
/// drainer claims every organisation's due messages and one prune pass covers
/// the whole ledger — so they cannot run on the application pool, whose forced
/// row-level security would show them an empty database. Absent, this process
/// only serves.
const ENGINE_DB_FLAG: &str = "--engine-db-url";

/// The built client directory, served as the router's fallback so the API
/// and the UI share one origin; unknown paths fall through to index.html,
/// which is what a single-page app's client router needs.
const UI_FLAG: &str = "--ui-dir";

/// One drain pass claims at most this many messages, so a backlog is worked
/// off over several passes rather than held in one long transaction.
const DRAIN_BATCH: i64 = 32;

const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1_000;

/// The instant, read at the one process boundary the lint table permits and
/// handed to the library as data. A clock before the epoch saturates to zero,
/// which reads as "everything is expired" — fail closed, not fail weird.
#[expect(
    clippy::disallowed_methods,
    reason = "the serving binary is the clock-reading process boundary; time enters every handler as data from here"
)]
fn wall_now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(i64::try_from(millis).unwrap_or(0))
}

/// Events older than this are prunable. Either arithmetic failure clamps to
/// the epoch, which prunes nothing: an overflow must not be able to erase a
/// ledger.
fn retention_cutoff(now: Timestamp) -> Timestamp {
    let millis = tam_limits::ledger::JOB_EVENT_RETENTION_DAYS
        .checked_mul(MILLIS_PER_DAY)
        .and_then(|window| now.0.checked_sub(window))
        .unwrap_or(0);
    Timestamp(millis.max(0))
}

/// The drainer the design hosts in this process, through the logging
/// deliverer until a relay exists, so drained messages are visible rather
/// than silently accumulating.
#[expect(
    clippy::disallowed_methods,
    reason = "the drain loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
fn spawn_outbox_drain(outbox: OutboxRepo, cancel: CancellationToken) {
    let period = core::time::Duration::from_secs(tam_limits::ledger::OUTBOX_DRAIN_INTERVAL_SECS);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(period) => {}
            }
            match drain(&outbox, &LoggingDeliverer, wall_now(), DRAIN_BATCH).await {
                Ok(report) if report.delivered + report.retried + report.dead > 0 => eprintln!(
                    "tam-server: outbox delivered {} retried {} dead {}",
                    report.delivered, report.retried, report.dead
                ),
                Ok(_) => {}
                Err(error) => eprintln!("tam-server: outbox drain failed: {error}"),
            }
        }
    });
}

/// The retention pruner, advancing the watermark the progress stream's resync
/// decision reads.
#[expect(
    clippy::disallowed_methods,
    reason = "the prune loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
fn spawn_event_pruner(pruner: PruneRepo, cancel: CancellationToken) {
    let period = core::time::Duration::from_secs(tam_limits::ledger::PRUNE_INTERVAL_SECS);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(period) => {}
            }
            let cutoff = retention_cutoff(wall_now());
            let batch = tam_limits::ledger::PRUNE_BATCH;
            match pruner.prune_pass(cutoff, batch).await {
                Ok(report) if report.deleted + report.watermark_advances > 0 => eprintln!(
                    "tam-server: pruned {} job events, advanced {} watermarks",
                    report.deleted, report.watermark_advances
                ),
                Ok(_) => {}
                Err(error) => eprintln!("tam-server: prune pass failed: {error}"),
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = parse_invocation()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&invocation.db_url)
        .await?;
    let state = AppState {
        pool,
        config: invocation.config.clone(),
        wall: wall_now,
    };

    let listener = tokio::net::TcpListener::bind(invocation.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!("tam-server listening on http://{bound}");
    if invocation.config.disclosure == Disclosure::Full {
        eprintln!("tam-server disclosing fault internals ({DISCLOSE_FLAG}); development only");
    }

    let loops = CancellationToken::new();
    if let Some(url) = &invocation.engine_db_url {
        let engine = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(url)
            .await?;
        eprintln!("tam-server hosting the outbox drainer and the job-event pruner");
        spawn_outbox_drain(OutboxRepo::new(engine.clone()), loops.clone());
        spawn_event_pruner(PruneRepo::new(engine), loops.clone());
    }

    let app = match &invocation.ui_dir {
        Some(dir) => {
            eprintln!("tam-server serving the client from {}", dir.display());
            // The single-page shell is read once and served explicitly with
            // 200 for any path the API and the asset tree do not claim;
            // tower-http's not_found_service coerces the status to 404 by
            // design; its `fallback` passes the shell through as the 200 the
            // client router needs.
            let shell = read_shell(&dir.join("index.html"))?;
            let spa = axum::routing::any(move || {
                let shell = shell.clone();
                async move {
                    (
                        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        shell,
                    )
                }
            });
            tam_api::router(state)
                .fallback_service(tower_http::services::ServeDir::new(dir).fallback(spa))
        }
        None => tam_api::router(state),
    };
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await;
    loops.cancel();
    served?;
    Ok(())
}

struct Invocation {
    db_url: String,
    bind: SocketAddr,
    config: Config,
    engine_db_url: Option<String>,
    ui_dir: Option<std::path::PathBuf>,
}

/// The database url first, then an optional bind address and the disclosure
/// flag in either order. Configuration is read from the command line rather
/// than the environment, which the lint table bans outside the one crate
/// that will own it.
fn parse_invocation() -> Result<Invocation, Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let mut config = Config::default();
    let mut engine_db_url = None;
    let mut ui_dir = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == DISCLOSE_FLAG {
            config.disclosure = Disclosure::Full;
        } else if argument == BROKER_FLAG {
            let path = arguments
                .next()
                .ok_or("--broker-socket needs a path argument")?;
            config.broker_socket = Some(std::path::PathBuf::from(path));
        } else if argument == ENGINE_DB_FLAG {
            engine_db_url = Some(
                arguments
                    .next()
                    .ok_or("--engine-db-url needs a url argument")?,
            );
        } else if argument == UI_FLAG {
            ui_dir = Some(std::path::PathBuf::from(
                arguments.next().ok_or("--ui-dir needs a path argument")?,
            ));
        } else {
            positional.push(argument);
        }
    }
    let db_url = positional
        .first()
        .cloned()
        .ok_or("usage: tam-server <db-url> [bind-addr] [--disclose-internals]")?;
    let bind = match positional.get(1) {
        Some(raw) => raw.parse()?,
        None => DEFAULT_BIND,
    };
    Ok(Invocation {
        db_url,
        bind,
        config,
        engine_db_url,
        ui_dir,
    })
}

fn read_shell(path: &std::path::Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

async fn shutdown() {
    let _interrupted = tokio::signal::ctrl_c().await;
}
