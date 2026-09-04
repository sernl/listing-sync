//! The internet-facing HTTP process: an address, a listener, a pool, and the
//! router `tam-api` builds. Every route, extractor and error mapping lives in
//! the library, so this binary holds nothing a test would want to reach.
//!
//! Given an engine-role url it also hosts the two service loops the design
//! puts in this process: the outbox drainer and the job-event pruner.
//!
//! Usage: tam-server <db-url> [bind-addr] [--engine-db-url <url>] [--backoffice-db-url <url>] [--paddle-webhook-secret <secret>] [--ui-dir <path>] [--auth-issuer <url> --auth-jwks-url <url>] [--blob-kek-path <path> --blob-store-root <path>] [--entitlement-key-path <path>] [--entitlement-public-key <hex>] [--require-entitlement-key] [--disclose-internals]

#![forbid(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tam_api::devices::EntitlementKey;
use tam_api::{
    AppState, AuthBridge, BlobStore, Config, Disclosure, JwkSet, JwksFuture, JwksSource,
    JwksUnavailable, WebhookSecret,
};
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

/// The engine-role url the service loops run on. They cross tenants — the
/// drainer claims every organisation's due messages and one prune pass covers
/// the whole ledger — so they cannot run on the application pool, whose forced
/// row-level security would show them an empty database. Absent, this process
/// only serves.
const ENGINE_DB_FLAG: &str = "--engine-db-url";

/// The backoffice-role url the operator surface reads on. That role is
/// SELECT-only and its reach is enumerated table by table in migration 0037,
/// so it can aggregate across tenants without holding any privilege that
/// would let a wrong predicate write one. Absent, every `/admin` route
/// refuses: a deployment serves the operator surface or it does not.
const BACKOFFICE_DB_FLAG: &str = "--backoffice-db-url";

/// Paddle's notification-webhook secret, which is the whole authentication of
/// the billing webhook. Given on the command line like every other
/// configuration value here, because the lint table bans environment reads
/// outside the one crate that will own them. Absent, `/{version}/billing/webhook`
/// answers 503: there is no unauthenticated mode of that route to fall back to.
const PADDLE_WEBHOOK_SECRET_FLAG: &str = "--paddle-webhook-secret";

/// The built client directory, served as the router's fallback so the API
/// and the UI share one origin; unknown paths fall through to index.html,
/// which is what a single-page app's client router needs.
const UI_FLAG: &str = "--ui-dir";

/// The identity service's issuer, which every login assertion's `iss` must
/// equal. Absent, no assertion is accepted and the break-glass token minted by
/// `tam-mint-session` is the only way in.
const AUTH_ISSUER_FLAG: &str = "--auth-issuer";

/// Where that service publishes the keys its assertions are signed under.
const AUTH_JWKS_FLAG: &str = "--auth-jwks-url";

/// The key every tenant's blob data-encryption key is wrapped under, read
/// from a file the way `tam-worker` and `tam-import` read theirs. Given on
/// the command line like every other configuration value here, because the
/// lint table bans environment reads outside the one crate that will own
/// them. Absent, `POST /{version}/uploads` answers 503: an upload has to seal
/// its bytes somewhere, and there is no unsealed mode of it to fall back to.
const BLOB_KEK_FLAG: &str = "--blob-kek-path";

/// The Ed25519 key entitlement tokens are signed under, read from a file the
/// way the blob key-encryption key is and given on the command line for the
/// same reason: the lint table bans environment reads outside the one crate
/// that will own them. The file holds PKCS#8 DER, which is the only form
/// `jsonwebtoken` accepts in this workspace -- it is built without `use_pem`,
/// so there is no `from_ed_pem` to take the other one.
///
/// Absent, a heartbeat still answers. It carries no token, every device's gate
/// stays closed, and nothing is served that would have been served anyway:
/// refusing the route instead would stop a signed-out device learning it was
/// signed out, which is the one thing a heartbeat must never fail to say.
const ENTITLEMENT_KEY_FLAG: &str = "--entitlement-key-path";

/// Refuse to start without that key.
///
/// A positive assertion rather than an inferred mode, and for exactly the
/// reason `--disclose-internals` is a flag: this workspace ships
/// `debug-assertions = true` in release, so no `cfg` can tell production
/// apart. Production passes this; `just dev` does not, and a development run
/// with no key grants nothing and says so.
const REQUIRE_ENTITLEMENT_KEY_FLAG: &str = "--require-entitlement-key";

/// The public half this deployment is expected to be signing under, as the
/// sixty-four lowercase hex characters the fleet's `TAM_ENTITLEMENT_PUBLIC_KEY`
/// repository variable carries. Given, a mismatch refuses the start.
///
/// It exists because the failure it catches is otherwise silent. A restored
/// backup or a half-finished rotation puts the wrong pair at the key path;
/// every token still signs, every client then fails to verify it, and every
/// seller's gate closes on their next check-in with nothing in any log
/// separating that from a healthy deployment. Optional rather than required
/// because a deployment that does not know its own fleet's key is still better
/// off minting than not, and the public half is printed either way.
const ENTITLEMENT_PUBLIC_KEY_FLAG: &str = "--entitlement-public-key";

/// The most of an entitlement key file that is read.
///
/// An Ed25519 PKCS#8 key pair is under a hundred bytes; this is three orders of
/// magnitude of headroom and still refuses to allocate a mis-pointed gigabyte
/// before rejecting it. Named because the doc comment on the reader calls it
/// bounded, and without this that described the path rather than the length.
const ENTITLEMENT_KEY_BYTES_MAX: u64 = 8 * 1024;

/// The directory the sealed objects are written beneath, which is the same
/// root the worker reads them back from. Paired with the key above for the
/// reason the identity pair is paired: a key with nowhere to write would seal
/// bytes into nothing, and a root with no key would have nothing to write.
const BLOB_STORE_ROOT_FLAG: &str = "--blob-store-root";

/// The key set is small and the identity service is a neighbour, so these are
/// short. They are named at all because the lint table refuses a client built
/// without them: an untimed fetch here would stall a login indefinitely.
const JWKS_TIMEOUT_SECS: u64 = 5;
const JWKS_CONNECT_TIMEOUT_SECS: u64 = 2;

/// The key set over HTTP. Lives in the binary rather than the library for the
/// same reason the clock does: `tam-api` holds no client and opens no socket,
/// which is what lets its tests drive the whole exchange with no listener.
struct HttpJwks {
    client: reqwest::Client,
    url: String,
}

impl JwksSource for HttpJwks {
    fn fetch(&self) -> JwksFuture<'_> {
        Box::pin(async move {
            let failed = |what: &str, error: &dyn core::fmt::Display| {
                let reason = format!("{what}: {error}");
                eprintln!("tam-server: {} {reason}", self.url);
                JwksUnavailable(reason)
            };
            let answer = self
                .client
                .get(&self.url)
                .send()
                .await
                .map_err(|error| failed("the key set could not be fetched", &error))?
                .error_for_status()
                .map_err(|error| failed("the key set answered an error status", &error))?;
            answer
                .json::<JwkSet>()
                .await
                .map_err(|error| failed("the key set did not parse", &error))
        })
    }
}

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
    let mut invocation = parse_invocation()?;
    // Before anything else: the standards corpus is compiled into this binary,
    // and a build that cannot parse its own corpus should fail here rather
    // than at the first seller's search. Parsing it once also means no request
    // pays for it.
    tam_api::product::standards::prime()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&invocation.db_url)
        .await?;
    let auth = match &invocation.identity {
        Some((issuer, url)) => {
            eprintln!("tam-server accepting login assertions from {issuer}");
            let client = reqwest::Client::builder()
                .timeout(core::time::Duration::from_secs(JWKS_TIMEOUT_SECS))
                .connect_timeout(core::time::Duration::from_secs(JWKS_CONNECT_TIMEOUT_SECS))
                .build()?;
            Some(std::sync::Arc::new(AuthBridge::new(
                issuer.clone(),
                Box::new(HttpJwks {
                    client,
                    url: url.clone(),
                }),
            )))
        }
        None => None,
    };
    let backoffice = match &invocation.backoffice_db_url {
        Some(url) => {
            eprintln!("tam-server serving the operator backoffice");
            Some(
                sqlx::postgres::PgPoolOptions::new()
                    .max_connections(2)
                    .connect(url)
                    .await?,
            )
        }
        None => None,
    };
    let blobs = match &invocation.blobs {
        Some((kek_path, root)) => {
            eprintln!(
                "tam-server accepting resource uploads into {}",
                root.display()
            );
            Some(BlobStore {
                kek: load_kek(kek_path)?,
                root: root.clone(),
            })
        }
        None => None,
    };
    invocation.config.entitlement_key = if let Some(path) = &invocation.entitlement_key_path {
        let key = load_entitlement_key(path)?;
        // The checked parse, here rather than at the first check-in: a key that
        // is not a key pair is a start-up fault, not a seller's fault.
        let public = key
            .public_key_hex()
            .map_err(|why| format!("{path} is not an Ed25519 signing key: {why}"))?;
        if let Some(expected) = &invocation.entitlement_public_key {
            if expected != &public {
                return Err(format!(
                    "{path} signs under public key {public}, but {ENTITLEMENT_PUBLIC_KEY_FLAG} \
                     expects {expected}. A server signing under a key the fleet does not carry \
                     closes every seller's gate on their next check-in, so this refuses to \
                     start rather than looking healthy while doing it."
                )
                .into());
            }
        }
        // Printed whether or not it was checked, so an operator can compare it
        // with the fleet's repository variable in one glance.
        eprintln!("tam-server minting entitlement tokens under public key {public}");
        Some(key)
    } else {
        eprintln!(
            "tam-server minting no entitlement tokens ({ENTITLEMENT_KEY_FLAG} unset); \
             every desktop client's gate stays closed"
        );
        None
    };
    let state = AppState {
        pool,
        config: invocation.config.clone(),
        wall: wall_now,
        auth,
        backoffice,
        blobs,
    };

    let listener = tokio::net::TcpListener::bind(invocation.bind).await?;
    let bound = listener.local_addr()?;
    eprintln!("tam-server listening on http://{bound}");
    if invocation.config.paddle_webhook_secret.is_some() {
        eprintln!("tam-server accepting Paddle billing notifications");
    }
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
    backoffice_db_url: Option<String>,
    ui_dir: Option<std::path::PathBuf>,
    /// The issuer and the key-set url, which are meaningless apart and so are
    /// parsed as one value.
    identity: Option<(String, String)>,
    /// The key-encryption key's path and the object-store root, which are
    /// meaningless apart for the same reason.
    blobs: Option<(String, std::path::PathBuf)>,
    /// Where the entitlement signing key is, if this deployment mints tokens.
    entitlement_key_path: Option<String>,
    /// The public half that key is expected to have, if the operator stated one.
    entitlement_public_key: Option<String>,
}

/// The database url first, then an optional bind address and the disclosure
/// flag in either order. Configuration is read from the command line rather
/// than the environment, which the lint table bans outside the one crate
/// that will own it.
fn parse_invocation() -> Result<Invocation, Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let mut config = Config::default();
    let mut engine_db_url = None;
    let mut backoffice_db_url = None;
    let mut ui_dir = None;
    let mut auth_issuer = None;
    let mut auth_jwks_url = None;
    let mut blob_kek_path = None;
    let mut blob_store_root = None;
    let mut entitlement_key_path = None;
    let mut entitlement_public_key = None;
    let mut require_entitlement_key = false;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == DISCLOSE_FLAG {
            config.disclosure = Disclosure::Full;
        } else if argument == ENGINE_DB_FLAG {
            engine_db_url = Some(
                arguments
                    .next()
                    .ok_or("--engine-db-url needs a url argument")?,
            );
        } else if argument == BACKOFFICE_DB_FLAG {
            backoffice_db_url = Some(
                arguments
                    .next()
                    .ok_or("--backoffice-db-url needs a url argument")?,
            );
        } else if argument == PADDLE_WEBHOOK_SECRET_FLAG {
            config.paddle_webhook_secret = Some(WebhookSecret::new(
                arguments
                    .next()
                    .ok_or("--paddle-webhook-secret needs a secret argument")?,
            ));
        } else if argument == UI_FLAG {
            ui_dir = Some(std::path::PathBuf::from(
                arguments.next().ok_or("--ui-dir needs a path argument")?,
            ));
        } else if argument == AUTH_ISSUER_FLAG {
            auth_issuer = Some(
                arguments
                    .next()
                    .ok_or("--auth-issuer needs a url argument")?,
            );
        } else if argument == AUTH_JWKS_FLAG {
            auth_jwks_url = Some(
                arguments
                    .next()
                    .ok_or("--auth-jwks-url needs a url argument")?,
            );
        } else if argument == BLOB_KEK_FLAG {
            blob_kek_path = Some(
                arguments
                    .next()
                    .ok_or("--blob-kek-path needs a path argument")?,
            );
        } else if argument == ENTITLEMENT_KEY_FLAG {
            entitlement_key_path = Some(
                arguments
                    .next()
                    .ok_or("--entitlement-key-path needs a path argument")?,
            );
        } else if argument == ENTITLEMENT_PUBLIC_KEY_FLAG {
            entitlement_public_key = Some(
                arguments
                    .next()
                    .ok_or("--entitlement-public-key needs a 64-character hex argument")?,
            );
        } else if argument == REQUIRE_ENTITLEMENT_KEY_FLAG {
            require_entitlement_key = true;
        } else if argument == BLOB_STORE_ROOT_FLAG {
            blob_store_root = Some(std::path::PathBuf::from(
                arguments
                    .next()
                    .ok_or("--blob-store-root needs a path argument")?,
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
    // Refused rather than half-configured: an issuer with nowhere to fetch
    // keys from would verify nothing, and a key set with no expected issuer
    // would accept an assertion minted for somebody else.
    let identity = match (auth_issuer, auth_jwks_url) {
        (Some(issuer), Some(url)) => Some((issuer, url)),
        (None, None) => None,
        _ => {
            return Err(format!(
                "{AUTH_ISSUER_FLAG} and {AUTH_JWKS_FLAG} are given together or not at all"
            )
            .into())
        }
    };
    // Refused rather than half-configured, exactly as the identity pair is: a
    // key with nowhere to write would seal bytes into nothing, and a root with
    // no key would have nothing to write into it.
    let blobs = match (blob_kek_path, blob_store_root) {
        (Some(kek), Some(root)) => Some((kek, root)),
        (None, None) => None,
        _ => {
            return Err(format!(
                "{BLOB_KEK_FLAG} and {BLOB_STORE_ROOT_FLAG} are given together or not at all"
            )
            .into())
        }
    };
    // The assertion a production unit file makes, refused rather than warned
    // about: a deployment that meant to mint tokens and was started without a
    // key would serve every seller a closed gate and look healthy doing it.
    if require_entitlement_key && entitlement_key_path.is_none() {
        return Err(format!(
            "{REQUIRE_ENTITLEMENT_KEY_FLAG} demands {ENTITLEMENT_KEY_FLAG}, which was not given"
        )
        .into());
    }
    Ok(Invocation {
        db_url,
        bind,
        config,
        engine_db_url,
        backoffice_db_url,
        ui_dir,
        identity,
        blobs,
        entitlement_key_path,
        entitlement_public_key,
    })
}

/// The key-encryption key off disk, read the way `tam-worker` reads its own:
/// a bounded file opened and read to end, rather than `std::fs::read`, which
/// the lint table bans.
fn load_kek(path: &str) -> Result<tam_secrets::Kek, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(tam_secrets::Kek::from_bytes(&bytes)?)
}

/// The entitlement signing key off disk: a file opened and read under a byte
/// cap, rather than `std::fs::read`, which the lint table bans.
///
/// Capped at [`ENTITLEMENT_KEY_BYTES_MAX`] rather than read to end, so a
/// mis-pointed path is refused instead of allocating whatever it happened to
/// name. A key pair is under a hundred bytes, so anything at the cap is already
/// not one.
fn load_entitlement_key(path: &str) -> Result<EntitlementKey, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(ENTITLEMENT_KEY_BYTES_MAX + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Err(
            format!("{path} is empty; it holds the PKCS#8 DER of an Ed25519 key pair").into(),
        );
    }
    if bytes.len() as u64 > ENTITLEMENT_KEY_BYTES_MAX {
        return Err(format!(
            "{path} is larger than {ENTITLEMENT_KEY_BYTES_MAX} bytes, so it is not the PKCS#8 \
             DER of an Ed25519 key pair"
        )
        .into());
    }
    Ok(EntitlementKey::new(bytes))
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
