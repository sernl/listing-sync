//! The internet-facing HTTP process: an address, a listener, and the router
//! `tam-api` builds. Every route, extractor and error mapping lives in the
//! library, so this binary holds nothing a test would want to reach.

#![forbid(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tam_api::{Config, Disclosure};

/// Loopback rather than `0.0.0.0`, so a development run is not reachable off
/// the machine by forgetting an argument.
const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

/// The development opt-in that discloses fault internals in responses. A flag
/// rather than a build property, because this workspace ships
/// `debug-assertions = true` in release, so no `cfg` can tell production
/// apart; redaction is the default in every build.
const DISCLOSE_FLAG: &str = "--disclose-internals";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = parse_invocation()?;
    let listener = tokio::net::TcpListener::bind(invocation.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!("tam-server listening on http://{bound}");
    if invocation.config.disclosure == Disclosure::Full {
        eprintln!("tam-server disclosing fault internals ({DISCLOSE_FLAG}); development only");
    }

    axum::serve(listener, tam_api::router(invocation.config))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

struct Invocation {
    bind: SocketAddr,
    config: Config,
}

/// An optional bind address and the disclosure flag, in either order.
/// Configuration is read from the command line rather than the environment,
/// which the lint table bans outside the one crate that will own it.
fn parse_invocation() -> Result<Invocation, std::net::AddrParseError> {
    let mut bind = None;
    let mut config = Config::default();
    for argument in std::env::args().skip(1) {
        if argument == DISCLOSE_FLAG {
            config.disclosure = Disclosure::Full;
        } else {
            bind = Some(argument.parse()?);
        }
    }
    Ok(Invocation {
        bind: bind.unwrap_or(DEFAULT_BIND),
        config,
    })
}

async fn shutdown() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("tam-server: cannot wait on ctrl-c, shutting down now: {error}");
    }
}
