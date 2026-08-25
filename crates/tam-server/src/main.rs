//! The internet-facing HTTP process: an address, a listener, a pool, and the
//! router `tam-api` builds. Every route, extractor and error mapping lives in
//! the library, so this binary holds nothing a test would want to reach.
//!
//! Usage: tam-server <db-url> [bind-addr] [--disclose-internals]

#![forbid(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tam_api::{AppState, Config, Disclosure};
use tam_types::Timestamp;

/// Loopback rather than `0.0.0.0`, so a development run is not reachable off
/// the machine by forgetting an argument.
const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

/// The development opt-in that discloses fault internals in responses. A flag
/// rather than a build property, because this workspace ships
/// `debug-assertions = true` in release, so no `cfg` can tell production
/// apart; redaction is the default in every build.
const DISCLOSE_FLAG: &str = "--disclose-internals";

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = parse_invocation()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&invocation.db_url)
        .await?;
    let state = AppState {
        pool,
        config: invocation.config,
        wall: wall_now,
    };

    let listener = tokio::net::TcpListener::bind(invocation.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!("tam-server listening on http://{bound}");
    if invocation.config.disclosure == Disclosure::Full {
        eprintln!("tam-server disclosing fault internals ({DISCLOSE_FLAG}); development only");
    }

    axum::serve(listener, tam_api::router(state))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

struct Invocation {
    db_url: String,
    bind: SocketAddr,
    config: Config,
}

/// The database url first, then an optional bind address and the disclosure
/// flag in either order. Configuration is read from the command line rather
/// than the environment, which the lint table bans outside the one crate
/// that will own it.
fn parse_invocation() -> Result<Invocation, Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let mut config = Config::default();
    for argument in std::env::args().skip(1) {
        if argument == DISCLOSE_FLAG {
            config.disclosure = Disclosure::Full;
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
    })
}

async fn shutdown() {
    let _interrupted = tokio::signal::ctrl_c().await;
}
