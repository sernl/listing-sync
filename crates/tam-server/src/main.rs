//! The internet-facing HTTP process: an address, a listener, and the router
//! `tam-api` builds. Every route, extractor and error mapping lives in the
//! library, so this binary holds nothing a test would want to reach.

#![forbid(unsafe_code)]

use std::net::SocketAddr;

/// Loopback rather than `0.0.0.0`, so a development run is not reachable off
/// the machine by forgetting an argument.
const DEFAULT_BIND: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = bind_address()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    let bound = listener.local_addr()?;
    eprintln!("tam-server listening on http://{bound}");

    axum::serve(listener, tam_api::router())
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// The first positional argument, or the loopback default. Configuration is
/// read from the command line rather than the environment, which the lint
/// table bans outside the one crate that will own it.
fn bind_address() -> Result<SocketAddr, std::net::AddrParseError> {
    let argument = std::env::args().nth(1);
    argument.as_deref().unwrap_or(DEFAULT_BIND).parse()
}

async fn shutdown() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("tam-server: cannot wait on ctrl-c, shutting down now: {error}");
    }
}
