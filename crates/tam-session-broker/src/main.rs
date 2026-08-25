//! The privilege boundary, as a process rather than a module.
//!
//! The broker is the sole holder of the key-encryption key and the only
//! process whose database role can read the credential vault. It exposes one
//! narrow unix-socket surface — link, lease, revoke, health — and hands out
//! authenticating gateway endpoints, never secret material, so a compromised
//! automation worker can use the connections it leased and cannot exfiltrate
//! the vault. It exists from the first commit because that boundary cannot be
//! introduced later without redesigning every call site that holds plaintext.
//!
//! Usage:
//!   tam-session-broker serve <socket-path> <broker-db-url> <kek-path> [upstream-base]
//!   tam-session-broker revoke-all <broker-db-url> <kek-path>   (the operator drill)

#![forbid(unsafe_code)]

mod gateway;
mod protocol;
mod service;
mod vault;

use std::io::Read as _;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

use crate::protocol::{Request, Response};
use crate::service::Broker;
use crate::vault::Vault;
use tam_secrets::Kek;

const DEFAULT_UPSTREAM: &str = "https://www.tes.com";

fn load_kek(path: &str) -> Result<Kek, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    // A KEK file may be exactly 32 raw bytes or 64 hex chars; accept both so
    // the operator can escrow whichever is convenient.
    let key = if bytes.len() == 64 && bytes.iter().all(u8::is_ascii_hexdigit) {
        decode_hex(&bytes)?
    } else {
        bytes
    };
    Ok(Kek::from_bytes(&key)?)
}

fn decode_hex(hex: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let text = std::str::from_utf8(hex)?;
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(text.get(i..i + 2).ok_or("odd-length hex")?, 16).map_err(Into::into)
        })
        .collect()
}

async fn pool(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("serve") => serve(&arguments[1..]).await,
        Some("revoke-all") => revoke_all(&arguments[1..]).await,
        _ => Err("usage: tam-session-broker serve|revoke-all ...".into()),
    }
}

async fn serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = args.first().ok_or("serve needs a socket path")?;
    let db_url = args.get(1).ok_or("serve needs a broker database url")?;
    let kek_path = args.get(2).ok_or("serve needs a kek path")?;
    let upstream = args.get(3).map_or(DEFAULT_UPSTREAM, String::as_str);

    let vault = Vault::new(pool(db_url).await?, load_kek(kek_path)?);
    let root = CancellationToken::new();
    let broker = Broker::new(vault, upstream.to_owned(), root.clone());

    clear_stale_socket(Path::new(socket_path))?;
    let listener = UnixListener::bind(socket_path)?;
    eprintln!("tam-session-broker serving on {socket_path}");

    let shutdown = root.clone();
    tokio::pin! {
        let ctrl_c = async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("tam-session-broker: cannot wait on ctrl-c: {error}");
            }
            shutdown.cancel();
        };
    }
    loop {
        tokio::select! {
            () = &mut ctrl_c => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _peer)) => {
                    if let Err(error) = handle_connection(&broker, stream).await {
                        eprintln!("tam-session-broker: connection error: {error}");
                    }
                }
                Err(error) => eprintln!("tam-session-broker: accept failed: {error}"),
            },
        }
    }
    root.cancel();
    eprintln!("tam-session-broker: stopped");
    Ok(())
}

async fn handle_connection(
    broker: &Broker,
    stream: tokio::net::UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => broker.handle(request, wall_ms()).await,
            Err(error) => Response::Error {
                detail: format!("malformed request: {error}"),
            },
        };
        let mut encoded = serde_json::to_string(&response)?;
        encoded.push('\n');
        write.write_all(encoded.as_bytes()).await?;
    }
    Ok(())
}

async fn revoke_all(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let db_url = args
        .first()
        .ok_or("revoke-all needs a broker database url")?;
    let kek_path = args.get(1).ok_or("revoke-all needs a kek path")?;
    let vault = Vault::new(pool(db_url).await?, load_kek(kek_path)?);
    let broker = Broker::new(vault, DEFAULT_UPSTREAM.to_owned(), CancellationToken::new());
    match broker.revoke_all().await {
        Response::Revoked {
            connections,
            elapsed_ms,
        } => {
            eprintln!("revoked {connections} connections in {elapsed_ms}ms");
            Ok(())
        }
        Response::Error { detail } => Err(format!("revoke-all failed: {detail}").into()),
        Response::Linked | Response::Leased { .. } | Response::Healthy => {
            Err("revoke-all returned an unexpected response".into())
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the broker socket loop reads the wall clock to stamp lease expiries; time enters as data from this process boundary"
)]
fn wall_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
    )
    .unwrap_or(i64::MAX)
}

fn clear_stale_socket(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(path),
        Ok(_) => Err(std::io::Error::other(format!(
            "{} exists and is not a socket",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
