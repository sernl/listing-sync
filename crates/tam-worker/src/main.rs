//! The automation lane's maintenance pump: steal expired leases, drain the
//! outbox through the logging deliverer, and run the fleet breaker, on a
//! jittered poll until ctrl-c.
//!
//! The item pump itself — leasing and driving items through the machine — is
//! deliberately NOT wired here yet: the driver is built and proven in
//! tam-engine's tests, but a worker cannot lease what it cannot project, and
//! submitting placeholder fields to a live marketplace is the account-safety
//! failure this milestone exists to prevent. M1j supplies the projection and
//! flips the pump on.
//!
//! Usage: tam-worker <engine-database-url> <worker-name> [poll-ms]

#![forbid(unsafe_code)]

use tam_engine::breaker::run_breaker;
use tam_engine::outbox::{drain, LoggingDeliverer};
use tam_storage::{HaltRepo, JobRepo, LeaseRepo, OutboxRepo};
use tam_types::Timestamp;
use tokio_util::sync::CancellationToken;

const DEFAULT_POLL_MS: u64 = 5_000;
const DRAIN_BATCH: i64 = 32;

/// Wall-clock enters here, at the process boundary, as the design's
/// time-as-data rule requires.
#[expect(
    clippy::disallowed_methods,
    reason = "the worker is a clock-reading process boundary; time enters the engine as data from here"
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
    let database_url = arguments
        .first()
        .ok_or("usage: tam-worker <engine-database-url> <worker-name> [poll-ms]")?;
    let worker_name = arguments
        .get(1)
        .ok_or("usage: tam-worker <engine-database-url> <worker-name> [poll-ms]")?;
    let poll_ms: u64 = arguments
        .get(2)
        .map_or(Ok(DEFAULT_POLL_MS), |raw| raw.parse())?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await?;
    let leases = LeaseRepo::new(pool.clone());
    let jobs = JobRepo::new(pool.clone());
    let halts = HaltRepo::new(pool.clone());
    let outbox = OutboxRepo::new(pool);

    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    let ctrl_c = async move {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("tam-worker: cannot wait on ctrl-c, stopping now: {error}");
        }
        stopper.cancel();
    };
    eprintln!("tam-worker {worker_name}: maintenance pump every {poll_ms}ms");

    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            () = &mut ctrl_c => break,
            () = tokio::time::sleep(core::time::Duration::from_millis(poll_ms)) => {}
        }
        if cancel.is_cancelled() {
            break;
        }
        let now = wall_now()?;
        let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
        match leases.expire_and_steal(now, attempts_max).await {
            Ok(0) => {}
            Ok(stolen) => eprintln!("tam-worker {worker_name}: stole {stolen} expired leases"),
            Err(error) => eprintln!("tam-worker {worker_name}: steal failed: {error}"),
        }
        match drain(&outbox, &LoggingDeliverer, now, DRAIN_BATCH).await {
            Ok(report) if report.delivered + report.retried + report.dead > 0 => {
                eprintln!("tam-worker {worker_name}: outbox {report:?}");
            }
            Ok(_) => {}
            Err(error) => eprintln!("tam-worker {worker_name}: drain failed: {error}"),
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
    }
    eprintln!("tam-worker {worker_name}: stopped cleanly");
    Ok(())
}
