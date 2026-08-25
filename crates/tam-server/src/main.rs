//! The internet-facing HTTP process: an address, a listener, a pool, and the
//! router `tam-api` builds. Every route, extractor and error mapping lives in
//! the library, so this binary holds nothing a test would want to reach.
//!
//! Usage: tam-server <db-url> [bind-addr] [--broker-socket <path>] [--ui-dir <path>] [--disclose-internals]

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

/// The credential broker's unix socket; without it the revoke endpoint
/// answers 503 rather than pretending.
const BROKER_FLAG: &str = "--broker-socket";

/// The built client directory, served as the router's fallback so the API
/// and the UI share one origin; unknown paths fall through to index.html,
/// which is what a single-page app's client router needs.
const UI_FLAG: &str = "--ui-dir";

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
        config: invocation.config.clone(),
        wall: wall_now,
    };

    let listener = tokio::net::TcpListener::bind(invocation.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!("tam-server listening on http://{bound}");
    if invocation.config.disclosure == Disclosure::Full {
        eprintln!("tam-server disclosing fault internals ({DISCLOSE_FLAG}); development only");
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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

struct Invocation {
    db_url: String,
    bind: SocketAddr,
    config: Config,
    ui_dir: Option<std::path::PathBuf>,
}

/// The database url first, then an optional bind address and the disclosure
/// flag in either order. Configuration is read from the command line rather
/// than the environment, which the lint table bans outside the one crate
/// that will own it.
fn parse_invocation() -> Result<Invocation, Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let mut config = Config::default();
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
