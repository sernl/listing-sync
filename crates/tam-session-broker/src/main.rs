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
//!   tam-session-broker serve <socket-path> <broker-db-url> <kek-path> \
//!       [tes-upstream] [tpt-upstream]
//!   tam-session-broker revoke-all <broker-db-url> <kek-path>   (the operator drill)
//!   tam-session-broker link-from-jar <broker-db-url> <kek-path> <marketplace> \
//!       <org-uuid> <connection-uuid> <netscape-jar-path> [authorship-name|epoch-ms]

#![forbid(unsafe_code)]

mod gateway;
mod jar;
mod protocol;
mod service;
mod vault;

use std::io::Read as _;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

use crate::protocol::{Authorship, Request, Response};
use crate::service::{Broker, Upstreams};
use crate::vault::Vault;
use tam_secrets::Kek;

const DEFAULT_TES_UPSTREAM: &str = "https://www.tes.com";
const DEFAULT_TPT_UPSTREAM: &str = "https://www.teacherspayteachers.com";

fn default_upstreams() -> Upstreams {
    Upstreams {
        tes: DEFAULT_TES_UPSTREAM.to_owned(),
        tpt: DEFAULT_TPT_UPSTREAM.to_owned(),
    }
}

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
        Some("link-from-jar") => link_from_jar(&arguments[1..]).await,
        _ => Err("usage: tam-session-broker serve|revoke-all|link-from-jar ...".into()),
    }
}

async fn serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = args.first().ok_or("serve needs a socket path")?;
    let db_url = args.get(1).ok_or("serve needs a broker database url")?;
    let kek_path = args.get(2).ok_or("serve needs a kek path")?;
    let upstreams = Upstreams {
        tes: args
            .get(3)
            .map_or(DEFAULT_TES_UPSTREAM, String::as_str)
            .to_owned(),
        tpt: args
            .get(4)
            .map_or(DEFAULT_TPT_UPSTREAM, String::as_str)
            .to_owned(),
    };

    let vault = Vault::new(pool(db_url).await?, load_kek(kek_path)?);
    let root = CancellationToken::new();
    let broker = Broker::new(vault, upstreams, root.clone());

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
                code: None,
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
    let broker = Broker::new(vault, default_upstreams(), CancellationToken::new());
    match broker.revoke_all().await {
        Response::Revoked {
            connections,
            elapsed_ms,
        } => {
            eprintln!("revoked {connections} connections in {elapsed_ms}ms");
            Ok(())
        }
        Response::Error { detail, .. } => Err(format!("revoke-all failed: {detail}").into()),
        Response::Linked
        | Response::Claimed
        | Response::Refreshed { .. }
        | Response::Leased { .. }
        | Response::Healthy => Err("revoke-all returned an unexpected response".into()),
    }
}

/// Seals a cookie jar the operator already holds on disk into the vault,
/// without contacting the marketplace.
///
/// This exists because the founder's Tpt session was exported from a browser
/// long before there was a link flow to receive it, and re-obtaining it would
/// mean another manual capture. The jar is read here, sealed through the same
/// `Link` the socket surface exposes, and never echoed. No health probe runs:
/// verifying a session means reaching the marketplace, which is a live-gated
/// action taken deliberately rather than as a side effect of sealing a file.
async fn link_from_jar(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let db_url = args
        .first()
        .ok_or("link-from-jar needs a broker database url")?;
    let kek_path = args.get(1).ok_or("link-from-jar needs a kek path")?;
    let marketplace = parse_marketplace(args.get(2).ok_or("link-from-jar needs a marketplace")?)?;
    let org = parse_uuid(
        args.get(3)
            .ok_or("link-from-jar needs an organisation uuid")?,
    )?;
    let connection = parse_uuid(args.get(4).ok_or("link-from-jar needs a connection uuid")?)?;
    let jar_path = args.get(5).ok_or("link-from-jar needs a jar path")?;
    let authorship = match args.get(6) {
        Some(raw) => Some(parse_authorship(raw)?),
        None => None,
    };

    let cookie_header = cookie_header_from_netscape(&read_file(jar_path)?)
        .ok_or("the jar carries no cookies, so there is nothing to seal")?;

    let vault = Vault::new(pool(db_url).await?, load_kek(kek_path)?);
    let broker = Broker::new(vault, default_upstreams(), CancellationToken::new());
    let response = broker
        .handle(
            Request::Link {
                org: tam_types::OrgId(org),
                connection: tam_types::ConnectionId(connection),
                marketplace,
                cookie_header,
                authorship,
            },
            wall_ms(),
        )
        .await;
    match response {
        Response::Linked => {
            eprintln!("sealed the jar for {marketplace:?}; no marketplace was contacted");
            Ok(())
        }
        Response::Error { detail, .. } => Err(format!("link-from-jar failed: {detail}").into()),
        Response::Claimed
        | Response::Refreshed { .. }
        | Response::Revoked { .. }
        | Response::Leased { .. }
        | Response::Healthy => Err("link-from-jar returned an unexpected response".into()),
    }
}

fn read_file(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut text = String::new();
    std::fs::File::open(path)?.read_to_string(&mut text)?;
    Ok(text)
}

fn parse_marketplace(raw: &str) -> Result<tam_types::Marketplace, Box<dyn std::error::Error>> {
    match raw {
        "tes" => Ok(tam_types::Marketplace::Tes),
        "tpt" => Ok(tam_types::Marketplace::Tpt),
        "etsy" => Ok(tam_types::Marketplace::Etsy),
        other => Err(format!("unknown marketplace {other:?}").into()),
    }
}

fn parse_uuid(raw: &str) -> Result<tam_types::Uuid, Box<dyn std::error::Error>> {
    tam_types::Uuid::parse_hyphenated(raw.trim())
        .ok_or_else(|| format!("{raw:?} is not a hyphenated uuid").into())
}

fn parse_authorship(raw: &str) -> Result<Authorship, Box<dyn std::error::Error>> {
    let (name, attested) = raw
        .split_once('|')
        .ok_or("authorship is \"name|epoch_millis\"")?;
    let name = name.trim();
    if name.is_empty() {
        return Err("the authorship name is empty".into());
    }
    Ok(Authorship {
        name: name.to_owned(),
        attested_at_ms: attested.trim().parse()?,
    })
}

/// Parses a Netscape cookie jar (fields: domain, flag, path, secure, expiry,
/// name, value), tolerating curl's `#HttpOnly_` prefix, into the `Cookie`
/// header the vault seals.
fn cookie_header_from_netscape(text: &str) -> Option<String> {
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if let (Some(name), Some(value)) = (fields.get(5), fields.get(6)) {
            pairs.push(format!("{name}={value}"));
        }
    }
    (!pairs.is_empty()).then(|| pairs.join("; "))
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

#[cfg(test)]
mod tests {
    use super::{cookie_header_from_netscape, parse_authorship, parse_marketplace, parse_uuid};
    use tam_types::Marketplace;

    #[test]
    fn a_netscape_jar_becomes_the_cookie_header_the_vault_seals() {
        let jar = "# Netscape HTTP Cookie File\n\
                   .teacherspayteachers.com\tTRUE\t/\tTRUE\t0\tsessionKey\tabc123\n\
                   #HttpOnly_.teacherspayteachers.com\tTRUE\t/\tTRUE\t0\tcsrfToken\tdeadbeef\n";
        assert_eq!(
            cookie_header_from_netscape(jar).as_deref(),
            Some("sessionKey=abc123; csrfToken=deadbeef"),
            "both plain and HttpOnly cookies join the header, or the sealed session is \
             missing the double-submit token every Tpt write needs"
        );
    }

    #[test]
    fn a_jar_with_nothing_in_it_seals_nothing() {
        for empty in ["", "# only comments\n", "\n\n"] {
            assert_eq!(
                cookie_header_from_netscape(empty),
                None,
                "sealing an empty jar would put a connection into a linked state holding no \
                 credential, which reads as healthy and fails on the first item"
            );
        }
    }

    #[test]
    fn the_marketplace_argument_is_a_known_name_or_a_refusal() {
        assert_eq!(
            parse_marketplace("tpt").expect("tpt is a marketplace"),
            Marketplace::Tpt
        );
        assert_eq!(
            parse_marketplace("tes").expect("tes is a marketplace"),
            Marketplace::Tes
        );
        assert!(
            parse_marketplace("teacherspayteachers").is_err(),
            "an unrecognised name must refuse rather than fall back to a default, or the \
             credential seals under the wrong tenant context and never opens again"
        );
    }

    #[test]
    fn an_organisation_argument_is_a_hyphenated_uuid_or_a_refusal() {
        assert!(
            parse_uuid("  aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa  ").is_ok(),
            "the hyphenated form parses, surrounding whitespace included"
        );
        for raw in ["", "not-an-id", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {
            assert!(
                parse_uuid(raw).is_err(),
                "{raw:?} must refuse; sealing against a mistyped tenant produces ciphertext \
                 nothing can ever open"
            );
        }
    }

    #[test]
    fn an_attestation_argument_carries_both_halves_or_neither() {
        let parsed = parse_authorship("A. Seller|1724889600000").expect("the paired form parses");
        assert_eq!(parsed.name, "A. Seller");
        assert_eq!(
            parsed.attested_at_ms, 1_724_889_600_000,
            "the instant is the seller's and travels verbatim; minting it here would be this \
             process attesting on their behalf"
        );
        for raw in [
            "A. Seller",
            "A. Seller|",
            "|1724889600000",
            "   |1724889600000",
        ] {
            assert!(
                parse_authorship(raw).is_err(),
                "{raw:?} must refuse; defaulting either half would put a declaration on the \
                 wire that nobody made"
            );
        }
    }
}
