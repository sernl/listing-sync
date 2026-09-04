//! The ledger's maintenance pass: steal expired leases, revive expired parks,
//! and run the fleet breaker, on a fixed-interval poll until ctrl-c.
//!
//! Three arms and nothing else. `expire_and_steal` takes back leases whose
//! holder went away, parking a stranded create rather than stealing it;
//! `revive_expired` releases parks whose clock has run out and re-gates the
//! ones no clock can open; `run_breaker` reads recent outcomes across tenants
//! and trips the fleet halt. All three are cross-tenant ledger work and none
//! of them touches a marketplace, which is why they stay on a server while the
//! writing does not.
//!
//! This process reaches no marketplace at all, and that is D1 rather than a
//! coincidence. The item pump that used to live here leased a gateway from the
//! credential broker per item and drove the machine against a Tes or Tpt
//! adapter; every one of those requests now originates on the seller's own
//! device under the seller's own session, so the pump, the account-claim mode
//! that shared its transports, and the adapter and secret dependencies they
//! needed have all gone. What is left carries no adapter, no transport, no
//! key-encryption key and no broker socket, and `cargo tree -e normal` shows
//! no adapter crate among this binary's own edges.
//!
//! The official-API branch will live here when it exists. Where a marketplace
//! publishes an API and issues a token for the purpose, its automation runs
//! server-side under that token, and this is the process that will run it —
//! Etsy is that branch and has no adapter yet, so today this binary hosts the
//! maintenance pass alone. It is deliberately not renamed for that future.
//!
//! The outbox drain lives in tam-server per the design's single-home line, and
//! items are enqueued by the api and by the import command; neither has ever
//! been this process's work, and calling what remains an "enqueue half" would
//! overstate it.
//!
//! `examples/live_provision.rs` is the exception to all of the above and is
//! not part of this binary: it renders listings to write the ledger rows a
//! supervised end-to-end proof needs, so it keeps the adapter crates as
//! dev-dependencies. Whether founder-run tooling that reaches a marketplace
//! server-side is itself within D1 is recorded as its own question, alongside
//! `tam-canary`, and is not settled by this file.

#![forbid(unsafe_code)]

use tam_engine::breaker::run_breaker;
use tam_storage::{HaltRepo, JobRepo, LeaseRepo};
use tam_types::Timestamp;
use tokio_util::sync::CancellationToken;

const DEFAULT_POLL_MS: u64 = 5_000;

/// Wall-clock enters here, at the process boundary, as the design's
/// time-as-data rule requires. A clock before the epoch saturates to zero,
/// which reads as "everything expired" -- fail closed, not fail weird.
#[expect(
    clippy::disallowed_methods,
    reason = "the maintenance pass is the process boundary where time enters; the ledger it drives holds no clock of its own"
)]
fn wall_clock() -> Timestamp {
    Timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(0)),
    )
}

/// One invocation, and there is only one mode now.
///
/// The claim verb went with the pump it served. A binary with one mode still
/// refuses another rather than treating it as the first: an operator who types
/// a verb that no longer exists is told so, instead of having their argument
/// list silently read as a database url.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(verb) = unknown_verb(&arguments) {
        return Err(format!("tam-worker has no {verb} mode; it runs the maintenance pass").into());
    }
    run_maintenance(&arguments).await
}

/// A leading word that cannot be a database url, which is the only thing the
/// first argument may be. Anything with a scheme is an address; anything else
/// in that position is a verb somebody expected to work.
fn unknown_verb(arguments: &[String]) -> Option<&str> {
    let first = arguments.first()?;
    (!first.contains("://")).then_some(first.as_str())
}

/// The ledger's maintenance pass.
async fn run_maintenance(arguments: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    const USAGE: &str = "usage: tam-worker <engine-database-url> <worker-name> [poll-ms]";
    let database_url = arguments.first().ok_or(USAGE)?;
    let worker_name = arguments.get(1).ok_or(USAGE)?;
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

    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    let ctrl_c = async move {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("tam-worker: cannot wait on ctrl-c, stopping now: {error}");
        }
        stopper.cancel();
    };
    eprintln!("tam-worker {worker_name}: maintenance pass every {poll_ms}ms");

    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            () = &mut ctrl_c => break,
            () = tokio::time::sleep(core::time::Duration::from_millis(poll_ms)) => {}
        }
        if cancel.is_cancelled() {
            break;
        }
        let now = wall_clock();
        let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
        match leases.expire_and_steal(now, attempts_max).await {
            Ok(0) => {}
            // Reaped rather than stolen: the pass settles, steals and parks, and a
            // stranded create takes the park arm, so naming this a steal reports one
            // that did not happen.
            Ok(reaped) => eprintln!("tam-worker {worker_name}: reaped {reaped} expired leases"),
            Err(error) => eprintln!("tam-worker {worker_name}: steal failed: {error}"),
        }
        match leases.revive_expired(now, attempts_max).await {
            // Reported apart, because they are different things: one put work
            // back on the queue, the other left work parked and changed what
            // it waits on.
            Ok(revived) => {
                if revived.requeued > 0 {
                    eprintln!(
                        "tam-worker {worker_name}: revived {} expired parks",
                        revived.requeued
                    );
                }
                if revived.re_gated > 0 {
                    eprintln!(
                        "tam-worker {worker_name}: {} parked create(s) now await the seller \
                         signing in",
                        revived.re_gated
                    );
                }
            }
            Err(error) => eprintln!("tam-worker {worker_name}: revive failed: {error}"),
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

#[cfg(test)]
mod tests {
    use super::unknown_verb;

    /// The claim verb went with the claim, and a binary with one mode still
    /// refuses another.
    ///
    /// The pump's arguments were positional and the verb was the only thing
    /// that selected the other mode, so with the verb gone an operator typing
    /// it would have had `claim` read as a database url and met a connection
    /// error naming a host called "claim". Refusing on the shape of the first
    /// argument tells them what actually happened.
    #[test]
    fn a_verb_that_no_longer_exists_is_refused_rather_than_read_as_an_address() {
        for verb in ["claim", "pump", "provision"] {
            assert_eq!(
                unknown_verb(&[verb.to_owned()]),
                Some(verb),
                "{verb} is not an address, so it is somebody expecting a mode"
            );
        }
    }

    /// And the one mode it does have is still reached, argument for argument.
    #[test]
    fn a_database_url_is_the_maintenance_pass_and_not_a_verb() {
        let arguments = [
            "postgres://tam_engine@127.0.0.1:5433/tam".to_owned(),
            "w1".to_owned(),
        ];
        assert_eq!(
            unknown_verb(&arguments),
            None,
            "an address in the first position is the pass's own first argument"
        );
        assert_eq!(
            unknown_verb(&[]),
            None,
            "and no arguments at all is the usage error, not an unknown verb"
        );
    }
}
