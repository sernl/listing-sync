//! The internet-facing HTTP process: an address, a listener, a pool, and the
//! router `tam-api` builds. Every route, extractor and error mapping lives in
//! the library, so this binary holds none of them.
//!
//! It does hold one thing a test wants to reach, and holds it deliberately:
//! [`assemble`], which orders the four tiers and decides which policy each
//! answer carries. That ordering is invisible to a test of any single tier, so
//! the `composition` module below drives the assembled router. Reaching it from
//! `tests/` would need this crate to grow a library target; the composition is
//! twenty lines and the binary is where it belongs, so the test comes here
//! instead.
//!
//! Given an engine-role url it also hosts the two service loops the design
//! puts in this process: the outbox drainer and the job-event pruner.
//!
//! Usage: tam-server <db-url> [bind-addr] [--engine-db-url <url>] [--backoffice-db-url <url>] [--paddle-webhook-secret <secret>] [--paddle-price-map <path>] [--ui-dir <path>] [--landing-dir <path>] [--downloads-dir <path>] [--auth-issuer <url> --auth-jwks-url <url>] [--blob-kek-path <path> --blob-store-root <path>] [--entitlement-key-path <path>] [--entitlement-public-key <hex>] [--require-entitlement-key] [--resend-api-key-file <path> --email-from <address> --console-url <url> --auth-internal-url <url> --auth-internal-secret-file <path>] [--disclose-internals]

#![forbid(unsafe_code)]

mod downloads;
mod notify;
mod serving;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tam_api::devices::EntitlementKey;
use tam_api::{
    AppState, AuthBridge, BlobStore, Config, Disclosure, JwkSet, JwksFuture, JwksSource,
    JwksUnavailable, PriceMap, WebhookSecret,
};
use tam_engine::outbox::{drain, Deliverer, LoggingDeliverer};
use tam_storage::{ImportBatchRepo, NotificationRepo, OutboxRepo, PruneRepo};
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

/// The file mapping Paddle price identifiers to what they sell: a JSON
/// object of `"<price_id>": { "plan": "subscriber" }` or
/// `{ "plan": "migration_only", "rung": 50 }`.
///
/// A file rather than a flag value, because the map is per environment and
/// grows a line per rung, and a path is the shape the deployment already
/// uses for the key material beside it. Absent, the map is empty: a
/// completed one-off transaction then grants nothing and says so on the log,
/// which is the fail-closed direction for a purchase we cannot interpret.
const PADDLE_PRICE_MAP_FLAG: &str = "--paddle-price-map";

/// The built client directory, served as the router's fallback so the API
/// and the UI share one origin; unknown paths fall through to index.html,
/// which is what a single-page app's client router needs.
const UI_FLAG: &str = "--ui-dir";

/// The built landing page, served ahead of the console at the paths it holds a
/// file for and nowhere else.
///
/// The founder's decision of 2026-09-05 gives the public page the origin's root
/// and moves the console's home to `/app`; the console's deep routes do not
/// move. Absent, behaviour is exactly what it was before this flag existed: the
/// console answers `/` like every other unclaimed path.
///
/// A separate directory rather than a route inside the console, because the two
/// are separate builds for the reason `docs/notes/design/landing-page.md` gives
/// — the console's root layout turns off both server rendering and
/// prerendering — and separate content-security policies for the reason
/// [`serving::Landing`] gives.
pub(crate) const LANDING_FLAG: &str = "--landing-dir";

/// The directory the desktop installers and their manifest are read from,
/// served under `/downloads/`.
///
/// This process never writes it and never fetches anything into it: its unit
/// denies IP egress, and `teachouse-downloads-refresh` — a separate oneshot on
/// a timer, with its own account and its own egress allowance — is what puts
/// files there. Absent, `/downloads/…` is an unknown path like any other and
/// the console's shell answers it.
pub(crate) const DOWNLOADS_FLAG: &str = "--downloads-dir";

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

/// The five values the completion mail needs, given together or not at all.
///
/// The relay's key and the identity service's shared secret are read from
/// files the way the blob and entitlement keys are, rather than given inline
/// the way the Paddle secret is: a secret on a command line is in every
/// process listing on the host. The other three are not secrets and are given
/// directly.
///
/// All five or none. A key with no sender address composes a mail nothing
/// accepts; a console origin missing puts a button in front of a seller that
/// goes nowhere; and an address route with no secret is a route that answers
/// 404. With none of them the drainer selects `LoggingDeliverer` and behaves
/// exactly as it did before mail existed, which is what development and CI run.
const RESEND_API_KEY_FLAG: &str = "--resend-api-key-file";
/// See [`RESEND_API_KEY_FLAG`].
const EMAIL_FROM_FLAG: &str = "--email-from";
/// The console's own public origin, which the mail's button is resolved
/// against. Stated rather than derived from the identity issuer: they are the
/// same origin in today's deployment shape and nothing holds them to that, and
/// a mail whose button goes nowhere is worse than a mail not sent.
const CONSOLE_URL_FLAG: &str = "--console-url";
/// Where the identity service's internal address route is reached, which is
/// not necessarily its public base: the fence is the shared secret either way.
const AUTH_INTERNAL_URL_FLAG: &str = "--auth-internal-url";
/// See [`RESEND_API_KEY_FLAG`].
const AUTH_INTERNAL_SECRET_FLAG: &str = "--auth-internal-secret-file";

/// The most of a secret file that is read. A relay key and a shared secret are
/// both under a hundred bytes; this refuses to allocate a mis-pointed gigabyte
/// before rejecting it, exactly as the entitlement key's own cap does.
const SECRET_BYTES_MAX: u64 = 8 * 1024;

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

/// The drainer the design hosts in this process, through whichever deliverer
/// the configuration selected: the real mail relay where one is configured,
/// and the logging deliverer otherwise, so drained messages are visible rather
/// than silently accumulating.
#[expect(
    clippy::disallowed_methods,
    reason = "the drain loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
fn spawn_outbox_drain(
    outbox: OutboxRepo,
    deliverer: impl Deliverer + 'static,
    cancel: CancellationToken,
) {
    let period = core::time::Duration::from_secs(tam_limits::ledger::OUTBOX_DRAIN_INTERVAL_SECS);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(period) => {}
            }
            match drain(&outbox, &deliverer, wall_now(), DRAIN_BATCH).await {
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

/// The spreadsheet-import expiry sweep, settling a batch nobody finished and
/// releasing the files attached to it.
///
/// Beside the pruner because it is the same shape of thing: a cross-tenant pass
/// on the engine's own pool, woken on a timer, whose failure is reported and
/// retried rather than fatal. The deadline itself is stored on each batch when
/// it is written, so this pass carries no window of its own -- it asks the
/// current instant and settles whatever is already past its own date.
#[expect(
    clippy::disallowed_methods,
    reason = "the sweep loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
fn spawn_import_batch_sweep(batches: ImportBatchRepo, cancel: CancellationToken) {
    let period = core::time::Duration::from_secs(tam_api::import_batch::sweep::SWEEP_INTERVAL_SECS);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(period) => {}
            }
            match tam_api::import_batch::sweep::pass(&batches, wall_now()).await {
                Ok(report) if report.abandoned > 0 => eprintln!(
                    "tam-server: swept {} expired imports, releasing {} attached files",
                    report.abandoned, report.released
                ),
                Ok(report) if report.settled > 0 || report.reopened > 0 => eprintln!(
                    "tam-server: settled {} finished imports a dead pass left importing and \
                     returned {} unfinished ones to their sellers",
                    report.settled, report.reopened
                ),
                Ok(_) => {}
                Err(error) => eprintln!("tam-server: import sweep failed: {error}"),
            }
        }
    });
}

/// How often the scheduler's pass runs.
///
/// A minute, because a schedule's finest granularity is a minute: the seller
/// picks a time of day and nothing finer, so a pass any more often would find
/// the same nothing to do and a pass any less often would fire "nine o'clock"
/// at nine past. The pass itself is idempotent on the scheduled instant
/// rather than on its own, so a late pass lands on the right tick.
const SCHEDULER_INTERVAL_SECS: u64 = 60;

/// The scheduler's pass, on the application pool.
///
/// Beside the import sweep and deliberately not in `tam-worker`: that process
/// connects as `tam_engine`, which holds no grant on the tables a tick writes
/// -- migration 0025's rule, restated by 0071 -- and carries no `AppState`,
/// so it could neither see a tenant's schedules nor reuse the commit the pull
/// finishes with. This loop takes the whole router state for that reason: a
/// scheduled commit is the same code a seller's commit runs.
#[expect(
    clippy::disallowed_methods,
    reason = "the scheduler loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
fn spawn_scheduler_pass(state: AppState, cancel: CancellationToken) {
    let period = core::time::Duration::from_secs(SCHEDULER_INTERVAL_SECS);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(period) => {}
            }
            match tam_api::scheduler::pass(&state, wall_now()).await {
                Ok(report) if report.eventful() => {
                    eprintln!(
                        "tam-server scheduler fired {} schedule ticks, minted {} jobs, opened {} \
                         marketplace reads and committed {} resources across {} tenants",
                        report.ticks, report.jobs, report.pulls, report.committed, report.tenants
                    );
                    // Per tenant, because that is the scope the pass isolates
                    // to: one seller's revoked connection or unparseable zone
                    // must be visible without hiding the rest of the fleet's
                    // work behind one failed pass.
                    for failure in &report.failures {
                        eprintln!("tam-server scheduler skipped a tenant: {failure}");
                    }
                }
                Ok(_) => {}
                Err(error) => eprintln!("tam-server: scheduler pass failed: {error}"),
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
    if invocation.config.paddle_price_map.is_empty() {
        eprintln!(
            "tam-server has no Paddle price map ({PADDLE_PRICE_MAP_FLAG}); a completed one-off \
             transaction will grant nothing"
        );
    } else {
        eprintln!(
            "tam-server mapping {} Paddle prices to plans",
            invocation.config.paddle_price_map.len()
        );
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
        eprintln!(
            "tam-server hosting the outbox drainer, the job-event pruner and the import sweep"
        );
        match notify::select(invocation.mail.as_ref()) {
            notify::Selected::Logging => {
                eprintln!(
                    "tam-server sending no completion mail ({RESEND_API_KEY_FLAG} unset); \
                     the console's notification list still fills"
                );
                spawn_outbox_drain(
                    OutboxRepo::new(engine.clone()),
                    LoggingDeliverer,
                    loops.clone(),
                );
            }
            notify::Selected::Email => {
                let mail = invocation
                    .mail
                    .as_ref()
                    .ok_or("the mail deliverer was selected with no configuration")?;
                eprintln!(
                    "tam-server sending completion mail from {}",
                    mail.email_from
                );
                spawn_outbox_drain(
                    OutboxRepo::new(engine.clone()),
                    notify::EmailDeliverer::new(
                        NotificationRepo::new(engine.clone()),
                        notify::AuthAddresses::new(
                            &mail.auth_internal_url,
                            &mail.auth_internal_secret,
                        )?,
                        notify::ResendRelay::new(&mail.resend_api_key, &mail.email_from)?,
                        &mail.console_url,
                    ),
                    loops.clone(),
                );
            }
        }
        spawn_event_pruner(PruneRepo::new(engine.clone()), loops.clone());
        spawn_import_batch_sweep(ImportBatchRepo::new(engine), loops.clone());
    }
    // Unconditional, unlike the three above: those cross tenants and so need
    // the engine role, and this one pins one tenant at a time and runs on the
    // application pool every deployment already has. A server started without
    // an engine url still owes its sellers their Friday drop.
    eprintln!("tam-server hosting the scheduler pass every {SCHEDULER_INTERVAL_SECS}s");
    spawn_scheduler_pass(state.clone(), loops.clone());

    let console = match &invocation.ui_dir {
        Some(dir) => {
            eprintln!("tam-server serving the client from {}", dir.display());
            Some(console_router(dir)?)
        }
        None => None,
    };
    let landing = match &invocation.landing_dir {
        Some(dir) => {
            eprintln!("tam-server serving the landing page from {}", dir.display());
            Some(std::sync::Arc::new(serving::Landing::load(dir)?))
        }
        None => None,
    };
    let downloads = match &invocation.downloads_dir {
        Some(dir) => {
            eprintln!(
                "tam-server serving the desktop downloads from {}",
                dir.display()
            );
            Some(std::sync::Arc::new(downloads::Downloads::open(dir)?))
        }
        None => None,
    };
    let app = assemble(state, console, landing, downloads);
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
    /// The built landing page, if this deployment serves one at its root.
    /// Independent of `ui_dir`: either, both or neither is a coherent
    /// deployment.
    landing_dir: Option<std::path::PathBuf>,
    /// The directory the desktop installers are read from, if this deployment
    /// serves them. Independent of both directories above.
    downloads_dir: Option<std::path::PathBuf>,
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
    /// The completion mail's five values, if this deployment sends any.
    mail: Option<notify::MailConfig>,
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
    let mut landing_dir = None;
    let mut downloads_dir = None;
    let mut auth_issuer = None;
    let mut auth_jwks_url = None;
    let mut blob_kek_path = None;
    let mut blob_store_root = None;
    let mut entitlement_key_path = None;
    let mut entitlement_public_key = None;
    let mut require_entitlement_key = false;
    let mut resend_api_key_file = None;
    let mut email_from = None;
    let mut console_url = None;
    let mut auth_internal_url = None;
    let mut auth_internal_secret_file = None;
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
        } else if argument == PADDLE_PRICE_MAP_FLAG {
            let path = arguments
                .next()
                .ok_or("--paddle-price-map needs a path argument")?;
            config.paddle_price_map = load_price_map(&path)?;
        } else if argument == UI_FLAG {
            ui_dir = Some(std::path::PathBuf::from(
                arguments.next().ok_or("--ui-dir needs a path argument")?,
            ));
        } else if argument == LANDING_FLAG {
            landing_dir = Some(std::path::PathBuf::from(
                arguments
                    .next()
                    .ok_or("--landing-dir needs a path argument")?,
            ));
        } else if argument == DOWNLOADS_FLAG {
            downloads_dir = Some(std::path::PathBuf::from(
                arguments
                    .next()
                    .ok_or("--downloads-dir needs a path argument")?,
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
        } else if argument == RESEND_API_KEY_FLAG {
            resend_api_key_file = Some(
                arguments
                    .next()
                    .ok_or("--resend-api-key-file needs a path argument")?,
            );
        } else if argument == EMAIL_FROM_FLAG {
            email_from = Some(
                arguments
                    .next()
                    .ok_or("--email-from needs an address argument")?,
            );
        } else if argument == CONSOLE_URL_FLAG {
            console_url = Some(
                arguments
                    .next()
                    .ok_or("--console-url needs a url argument")?,
            );
        } else if argument == AUTH_INTERNAL_URL_FLAG {
            auth_internal_url = Some(
                arguments
                    .next()
                    .ok_or("--auth-internal-url needs a url argument")?,
            );
        } else if argument == AUTH_INTERNAL_SECRET_FLAG {
            auth_internal_secret_file = Some(
                arguments
                    .next()
                    .ok_or("--auth-internal-secret-file needs a path argument")?,
            );
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
    // Refused rather than half-configured, exactly as the identity and blob
    // pairs are: four fifths of a mail path is a deployment that composes mail
    // it cannot address, cannot send, or points at nothing.
    let mail = match (
        resend_api_key_file,
        email_from,
        console_url,
        auth_internal_url,
        auth_internal_secret_file,
    ) {
        (None, None, None, None, None) => None,
        (Some(key_path), Some(from), Some(console), Some(auth_url), Some(secret_path)) => {
            Some(notify::MailConfig {
                resend_api_key: read_secret(&key_path)?,
                email_from: from,
                console_url: console,
                auth_internal_url: auth_url,
                auth_internal_secret: read_secret(&secret_path)?,
            })
        }
        _ => {
            return Err(format!(
                "{RESEND_API_KEY_FLAG}, {EMAIL_FROM_FLAG}, {CONSOLE_URL_FLAG}, \
                 {AUTH_INTERNAL_URL_FLAG} and {AUTH_INTERNAL_SECRET_FLAG} are given together \
                 or not at all"
            )
            .into())
        }
    };
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
        landing_dir,
        downloads_dir,
        identity,
        blobs,
        entitlement_key_path,
        entitlement_public_key,
        mail,
    })
}

/// A secret off disk: a bounded read, trimmed of the newline a file written by
/// an editor or a deployment tool carries, rather than `std::fs::read`, which
/// the lint table bans.
fn read_secret(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(SECRET_BYTES_MAX + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > SECRET_BYTES_MAX {
        return Err(format!(
            "{path} is larger than {SECRET_BYTES_MAX} bytes, so it is not a secret"
        )
        .into());
    }
    let secret = String::from_utf8(bytes)
        .map_err(|_| format!("{path} is not utf-8, so it is not a secret this process can send"))?
        .trim()
        .to_owned();
    if secret.is_empty() {
        return Err(format!("{path} is empty").into());
    }
    Ok(secret)
}

/// How large the Paddle price map may be. A line per price and a handful of
/// prices; anything at this cap is a mis-pointed path rather than a map.
const PRICE_MAP_BYTES_MAX: u64 = 64 * 1024;

/// The Paddle price map off disk: a bounded read, for the reason the two
/// key loaders beside it are bounded, and a parse that refuses rather than
/// half-applies. A malformed map must stop the server at startup, because
/// the alternative is a seller paying for a rung the running process cannot
/// interpret.
fn load_price_map(path: &str) -> Result<PriceMap, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| format!("{PADDLE_PRICE_MAP_FLAG} {path}: {error}"))?
        .take(PRICE_MAP_BYTES_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{PADDLE_PRICE_MAP_FLAG} {path}: {error}"))?;
    if bytes.len() as u64 > PRICE_MAP_BYTES_MAX {
        return Err(format!(
            "{path} is larger than {PRICE_MAP_BYTES_MAX} bytes, so it is not a price map"
        )
        .into());
    }
    let raw = String::from_utf8(bytes)
        .map_err(|_| format!("{PADDLE_PRICE_MAP_FLAG} {path} is not utf-8"))?;
    PriceMap::parse(&raw).map_err(|error| format!("{PADDLE_PRICE_MAP_FLAG} {path}: {error}").into())
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

/// The console's own service: its asset tree, its single-page shell for every
/// path that tree does not claim, and the policy both are served under.
fn console_router(dir: &std::path::Path) -> Result<axum::Router, Box<dyn std::error::Error>> {
    // The single-page shell is read once and served explicitly with 200 for any
    // path the API and the asset tree do not claim; tower-http's
    // not_found_service coerces the status to 404 by design; its `fallback`
    // passes the shell through as the 200 the client router needs.
    let shell = read_shell(&dir.join("index.html"))?;
    // Read once at start-up, so the hashes in the policy are the ones for the
    // shell this process is actually serving.
    let shell_text = String::from_utf8(shell.clone())
        .map_err(|_| "the console shell is not utf-8, so its policy cannot be computed")?;
    let spa = axum::routing::any(move || {
        let shell = shell.clone();
        async move {
            (
                [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                shell,
            )
        }
    });
    // On the console's own service rather than on the whole router, so it
    // covers the shell, the fallback page and every asset beside them and none
    // of the `/v1` answers. A policy on a JSON response governs nothing — no
    // browser applies one to a fetch — so putting it there was noise on every
    // API call and a claim this comment would have had to make and could not.
    let policy: std::sync::Arc<str> = std::sync::Arc::from(console_policy(&shell_text));
    Ok(axum::Router::new()
        .fallback_service(tower_http::services::ServeDir::new(dir).fallback(spa))
        .layer(axum::middleware::from_fn_with_state(
            policy,
            console_security_headers,
        )))
}

/// The whole router, from the four tiers this deployment was configured with.
///
/// A function rather than a block inside `main`, and that is what lets it be
/// driven by a test: the ordering it encodes is invisible to a test of any
/// single tier, and three mutations of it — swapping the two `.layer()` calls,
/// dropping the policy from a landing response, serving the API's namespace
/// from a static tier — produce a router that answers wrongly while every unit
/// test still passes.
///
/// The static tiers go ahead of the console rather than beside it: the decision
/// is one function, and layering it here is what puts it in front of the
/// console's policy layer, which inserts its own header over anything already
/// there.
fn assemble(
    state: AppState,
    console: Option<axum::Router>,
    landing: Option<std::sync::Arc<serving::Landing>>,
    downloads: Option<std::sync::Arc<downloads::Downloads>>,
) -> axum::Router {
    let fallback = if landing.is_some() || downloads.is_some() {
        let tiers = std::sync::Arc::new(Tiers { landing, downloads });
        Some(
            console
                .unwrap_or_else(|| axum::Router::new().fallback(nothing_here))
                .layer(axum::middleware::from_fn_with_state(tiers, static_tiers)),
        )
    } else {
        console
    };
    match fallback {
        Some(fallback) => tam_api::router(state).fallback_service(fallback),
        None => tam_api::router(state),
    }
}

/// The two static tiers this deployment was configured with, either of which
/// may be absent.
struct Tiers {
    landing: Option<std::sync::Arc<serving::Landing>>,
    downloads: Option<std::sync::Arc<downloads::Downloads>>,
}

/// The landing page and the downloads directory ahead of the console, at the
/// paths each holds a file for.
///
/// Outermost so their answers short-circuit before the console's policy layer
/// runs: that layer inserts the console's policy over whatever is already
/// there, and the policies are deliberately different — the landing carries its
/// own, and an installer carries none, because a policy governs a document and
/// nothing renders one of these.
async fn static_tiers(
    axum::extract::State(tiers): axum::extract::State<std::sync::Arc<Tiers>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let answer = serving::route(
        request.uri().path(),
        |file| tiers.landing.as_ref().is_some_and(|it| it.has(file)),
        tiers.downloads.is_some(),
    );
    let serves_static = matches!(
        answer,
        serving::Answer::Landing(_) | serving::Answer::Downloads(_)
    );
    if serves_static && !serving::method_serves_static(request.method()) {
        return serving::method_not_allowed();
    }
    let if_none_match = request
        .headers()
        .get(axum::http::header::IF_NONE_MATCH)
        .cloned();
    match answer {
        serving::Answer::Landing(file) => match &tiers.landing {
            Some(landing) => landing.respond(&file, if_none_match.as_ref()),
            None => next.run(request).await,
        },
        serving::Answer::Downloads(file) => match &tiers.downloads {
            Some(downloads) => downloads.respond(&file, if_none_match.as_ref()).await,
            None => next.run(request).await,
        },
        // The API's own routes are matched before this ever runs, so both of
        // these mean the same thing here and are named separately anyway: one
        // is a path the API owns and no static tier may answer, the other is
        // the console's.
        serving::Answer::Api | serving::Answer::Console => next.run(request).await,
    }
}

/// What a deployment carrying a landing page and no console answers where the
/// landing page has no file: the same 404 it answered before either existed.
async fn nothing_here() -> axum::http::StatusCode {
    axum::http::StatusCode::NOT_FOUND
}

/// The console's content-security policy, now that the desktop window loads the
/// console from here rather than from its own bundle.
///
/// The policy used to live in `tauri.conf.json` and applied to a window loading
/// bundled content. That window now navigates to this origin, so the bundle's
/// policy governs nothing the seller sees and this header is the only thing
/// that does.
///
/// It does NOT simply copy the bundle's text, and the reason is the defect the
/// first version of this shipped with. Tauri augments the configured policy at
/// run time with the inline-script hashes it collected at build time
/// (tauri 2.11.5, manager/mod.rs:53-105), so `script-src 'self'` was safe there
/// and is not here: SvelteKit's shell boots from one inline `<script>` block,
/// and a bare `'self'` refuses it. The window would have rendered an empty div,
/// for every desktop seller and every browser user, and the two tests that
/// existed asserted the string rather than loading a page. The hashes below are
/// what Tauri did for the bundle, done here.
///
/// Every host admitted is admitted by directive with its reason, in this one
/// place, so an addition is a decision rather than an accretion:
/// `challenges.cloudflare.com` is Turnstile, which needs both a script and a
/// frame; `cdn.paddle.com` is the checkout script. `wasm-unsafe-eval` is the
/// console's WebAssembly, which Chromium engines refuse without it.
///
/// It named Google's two font hosts until the console's faces moved into
/// `web/static/fonts`, declared by `@font-face` in `web/src/app.css`. The
/// desktop and Android builds show this console under a policy that admits no
/// third-party origin at all, so those hosts were never reachable there and
/// the app rendered fallback type; self-hosting is what repaired that, and it
/// leaves these two grants naming hosts nothing asks for. A dead grant on the
/// public origin is a grant nobody is checking, which is the reason
/// `landing_policy` gives for dropping the same pair.
///
/// `frame-src` names `'self'` beside Turnstile because a directive that is
/// present does not fall back to `default-src`: naming only the one host would
/// refuse the console's own frames too, silently and with nothing to explain
/// it.
///
/// `connect-src` stays `'self'` and names no host. The console is served from
/// the same origin it calls, which is the whole reason one host carries the
/// console and `/v1`, so naming it would be a second thing to edit at cutover
/// buying no guarantee — and a stale host there would break every request
/// rather than failing a build.
///
/// `worker-src` is named rather than left to fall back through `child-src` to
/// `script-src`, and it is the one directive that admits `blob:`. The preview
/// maker renders the seller's PDF with pdf.js, whose worker is a same-origin
/// asset our build emits; when a browser cannot load that asset directly,
/// pdf.js falls back to a worker built from a blob URL, and the fallback
/// fails silently under a policy that admits only `'self'`. Nothing else on
/// the console starts a worker.
fn console_policy(shell: &str) -> String {
    let mut script = String::from("script-src 'self' 'wasm-unsafe-eval'");
    for hash in serving::inline_script_hashes(shell) {
        script.push_str(" '");
        script.push_str(&hash);
        script.push('\'');
    }
    script.push_str(" https://challenges.cloudflare.com https://cdn.paddle.com");
    format!(
        "default-src 'self'; connect-src 'self'; {script}; \
         img-src 'self' data: blob:; \
         style-src 'self' 'unsafe-inline'; \
         font-src 'self' data:; \
         frame-src 'self' https://challenges.cloudflare.com; \
         worker-src 'self' blob:"
    )
}

async fn console_security_headers(
    axum::extract::State(policy): axum::extract::State<std::sync::Arc<str>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let immutable = request.uri().path().starts_with("/_app/immutable/");
    let mut response = next.run(request).await;
    // A page carries a fresh nonce beside the hashes and an asset carries the
    // policy as built; see `serving::fresh_nonce` for the script it admits.
    let is_page = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    // The shell must be revalidated on every load and the bundle under it may
    // be kept for a year: the shell names its chunks by content hash, so a
    // shell a browser kept past a deploy asks for chunks the new build no
    // longer serves, and the console fails to start until the shell is fetched
    // again — which is what the Android app showed after a deploy, and what
    // closing and reopening it cleared. `no-cache` is a revalidation, not a
    // refusal to cache, so a reload costs one conditional request.
    let freshness = if is_page {
        Some("no-cache")
    } else if immutable {
        Some("public, max-age=31536000, immutable")
    } else {
        None
    };
    if let Some(value) = freshness.and_then(|value| axum::http::HeaderValue::from_str(value).ok()) {
        response
            .headers_mut()
            .insert(axum::http::header::CACHE_CONTROL, value);
    }
    let header = if is_page {
        axum::http::HeaderValue::from_str(&serving::with_nonce(&policy, &serving::fresh_nonce()))
    } else {
        axum::http::HeaderValue::from_str(&policy)
    };
    if let Ok(value) = header {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_SECURITY_POLICY, value);
    }
    response
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

#[cfg(test)]
mod tests {
    /// Every hash a shell needs is in the policy that shell is served with.
    ///
    /// This is the assertion the first version of the header needed and did not
    /// have. The two tests it shipped with read the string and never loaded a
    /// shell, so `script-src 'self'` looked correct and would have rendered an
    /// empty window for every seller.
    ///
    /// Over a fixture rather than over `web/build/index.html`, which is
    /// gitignored and outside the sandbox's source filter: the version that
    /// read it returned early in every CI run and reported a pass. The claim it
    /// was making about the real artefact — that SvelteKit's shell does boot
    /// from an inline block — is now `checks.served-artefacts` in `flake.nix`,
    /// where the artefact exists.
    #[test]
    fn every_hash_a_shell_needs_is_in_its_policy() {
        let shell = "<html><head><script>\n\t{__sveltekit_1a2b3c = {};\n}\n</script>\
                     <script src=\"/_app/immutable/entry/start.js\"></script></head></html>";
        let hashes = crate::serving::inline_script_hashes(shell);
        assert_eq!(
            hashes.len(),
            1,
            "the sourced block contributes nothing; the inline one is the boot"
        );
        let policy = super::console_policy(shell);
        for hash in &hashes {
            assert!(
                policy.contains(hash.as_str()),
                "every hash the shell needs is in the policy it is served with: {policy}"
            );
        }
        assert!(
            policy.contains("'wasm-unsafe-eval'"),
            "the console's WebAssembly is refused without it: {policy}"
        );
    }

    /// The policy admits each third-party host on the directive it needs, and
    /// no other.
    #[test]
    fn every_third_party_host_is_admitted_by_directive() {
        let policy = super::console_policy("<html></html>");
        for (directive, host) in [
            ("script-src", "https://challenges.cloudflare.com"),
            ("frame-src", "https://challenges.cloudflare.com"),
            ("script-src", "https://cdn.paddle.com"),
        ] {
            let section = policy
                .split(';')
                .find(|part| part.trim_start().starts_with(directive))
                .unwrap_or_else(|| panic!("{directive} is in the policy"));
            assert!(
                section.contains(host),
                "{host} belongs on {directive}: {section}"
            );
        }
    }

    /// Type is served from this origin, so no directive names a font host.
    ///
    /// The pair this replaces was reachable in a browser and never in the app,
    /// whose own policy admits no third-party origin, so the console rendered
    /// two different sets of faces depending on where it was opened. The faces
    /// now live in `web/static/fonts`; asserting their absence here is what
    /// stops a future edit from restoring the grant and, with it, the split.
    #[test]
    fn no_directive_names_a_font_host() {
        let policy = super::console_policy("<html></html>");
        for host in ["fonts.googleapis.com", "fonts.gstatic.com"] {
            assert!(
                !policy.contains(host),
                "{host} is not needed and not admitted: {policy}"
            );
        }
    }

    /// The preview maker's worker is admitted, and `blob:` reaches no
    /// directive that would let a script be built from one.
    #[test]
    fn the_worker_directive_admits_the_pdf_renderer_and_nothing_scriptable() {
        let policy = super::console_policy("<html></html>");
        let worker = policy
            .split(';')
            .find(|part| part.trim_start().starts_with("worker-src"))
            .expect("worker-src is in the policy");
        assert!(
            worker.contains("'self'") && worker.contains("blob:"),
            "pdf.js loads its worker from our own asset and falls back to a blob URL: {worker}"
        );
        let script = policy
            .split(';')
            .find(|part| part.trim_start().starts_with("script-src"))
            .expect("script-src is in the policy");
        assert!(
            !script.contains("blob:"),
            "a blob a page can build is not a script it can run: {script}"
        );
    }

    /// `connect-src` names no host.
    ///
    /// Narrowed from the whole policy, which now names two hosts on purpose.
    /// This directive is the one that must stay relative: the console is served
    /// from the origin it calls, so naming it here would be a second thing to
    /// edit at cutover and a stale value would break every request rather than
    /// failing a build.
    #[test]
    fn the_connect_directive_names_no_host() {
        let policy = super::console_policy("<html></html>");
        let connect = policy
            .split(';')
            .find(|part| part.trim_start().starts_with("connect-src"))
            .expect("connect-src is in the policy");
        assert!(
            !connect.contains("http"),
            "a host here is a second thing to keep in step with the origin: {connect}"
        );
    }

    /// The policy is a header this build can send.
    ///
    /// It is built at start-up and inserted per response, so an invalid byte
    /// would drop the header silently rather than failing anything.
    #[test]
    fn the_policy_is_a_sendable_header() {
        let policy = super::console_policy("<html><script>boot()</script></html>");
        assert!(
            axum::http::HeaderValue::from_str(&policy).is_ok(),
            "the computed policy has to survive being put in a header: {policy}"
        );
    }
}

/// The assembled router, driven without a listener.
///
/// Every other test in this crate reads one tier's decision or one policy's
/// string. None of them can see the thing those pieces are assembled into, and
/// three mutations of that assembly answer wrongly while the whole suite stays
/// green: swapping the two `.layer()` calls, which puts the console's policy —
/// `wasm-unsafe-eval`, Turnstile, Paddle, no `frame-ancestors` — on every
/// marketing page; dropping the policy header from `Landing::respond`, which
/// serves the public root under no policy at all; and answering the API's
/// namespace from a static tier. Each is asserted below.
///
/// The pool is built lazily and never connects. Nothing here reaches a handler
/// that would use it: `/healthz` predates version negotiation and takes no
/// extractor, and the four static tiers touch no database at all.
#[cfg(test)]
mod composition {
    use axum::body::Body;
    use axum::http::{header, Method, Request, StatusCode};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use crate::serving::Landing;

    /// A landing build that claims the console's home, the console's client
    /// bundle and a path in the API's namespace.
    ///
    /// Hostile on purpose: a fixture that merely omitted those names would let
    /// every reservation below pass without being enforced, which is how the
    /// rule went unenforced while a design note said it held.
    const GREEDY_LANDING: [(&str, &str); 7] = [
        (
            "index.html",
            "<!doctype html><title>the public page</title>",
        ),
        (
            "pricing/index.html",
            "<!doctype html><title>pricing</title>",
        ),
        ("_astro/Base.abc123.css", "body{color:red}"),
        (
            "app/index.html",
            "<!doctype html><title>NOT the console</title>",
        ),
        ("_app/immutable/start.js", "// NOT the console bundle"),
        // The likeliest one to be written for real rather than to be hostile:
        // a teaching-resources marketing site has an obvious use for the word,
        // and the catalogue board answers under it.
        (
            "resources/index.html",
            "<!doctype html><title>NOT the console</title>",
        ),
        ("v1/nothing", "NOT the api"),
    ];

    const DOWNLOADS: [(&str, &str); 2] = [
        (
            "downloads.json",
            r#"{"version":"0.2.0","apple":null,"refreshed_at":"2026-09-05T00:00:00Z"}"#,
        ),
        (
            "Teachouse_0.2.0_x64-setup.exe",
            "MZ not really an installer",
        ),
    ];

    const CONSOLE_SHELL: &str =
        "<!doctype html><head><script>\n\t{__sveltekit = {};\n}\n</script></head>";

    /// The three directories, written once for the whole test binary.
    ///
    /// Once rather than per test, because these tests run concurrently and
    /// `File::create` truncates: a second test rewriting the fixture while the
    /// first was serving it handed back a zero-length page, which is how this
    /// harness failed on its first run while every test passed alone.
    struct Fixtures {
        console: std::path::PathBuf,
        landing: std::path::PathBuf,
        downloads: std::path::PathBuf,
    }

    fn fixtures() -> &'static Fixtures {
        static WRITTEN: std::sync::OnceLock<Fixtures> = std::sync::OnceLock::new();
        WRITTEN.get_or_init(|| Fixtures {
            console: tree(
                "console",
                &[
                    ("index.html", CONSOLE_SHELL),
                    (
                        "_app/immutable/entry/start.abc123.js",
                        "// the console bundle",
                    ),
                ],
            ),
            landing: tree("landing", &GREEDY_LANDING),
            downloads: tree("downloads", &DOWNLOADS),
        })
    }

    /// A directory tree under one per-process root, named for what it stands in
    /// for so a failure leaves something greppable.
    ///
    /// The root is removed before it is written, so a run leaves one tree rather
    /// than accumulating one per run. The last run's tree does survive: libtest
    /// has no after-all hook, and a `Drop` on the `OnceLock` would run while
    /// another test could still be reading it.
    fn tree(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        use std::io::Write as _;
        let root = std::env::temp_dir()
            .join(format!("tam-server-composition-{}", std::process::id()))
            .join(name);
        drop(std::fs::remove_dir_all(&root));
        for (relative, body) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a temporary directory");
            }
            let mut file = std::fs::File::create(&path).expect("a temporary file");
            file.write_all(body.as_bytes())
                .expect("writing the fixture");
        }
        root
    }

    /// The router `main` builds, over the three directories above.
    fn assembled() -> axum::Router {
        let state = tam_api::AppState {
            // Lazy: this never opens a socket, and no route reached below would
            // use it if it did.
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://tam_app@127.0.0.1/tam")
                .expect("a lazy pool needs no server"),
            config: tam_api::Config::default(),
            wall: crate::wall_now,
            auth: None,
            backoffice: None,
            blobs: None,
        };
        let built = fixtures();
        crate::assemble(
            state,
            Some(
                crate::console_router(&built.console)
                    .expect("the console fixture is a built console"),
            ),
            Some(std::sync::Arc::new(
                Landing::load(&built.landing).expect("the landing fixture is a built site"),
            )),
            Some(std::sync::Arc::new(
                crate::downloads::Downloads::open(&built.downloads).expect("a directory"),
            )),
        )
    }

    struct Answer {
        status: StatusCode,
        headers: axum::http::HeaderMap,
        body: String,
    }

    impl Answer {
        fn header(&self, name: header::HeaderName) -> &str {
            self.headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
        }
    }

    async fn ask(method: Method, path: &str) -> Answer {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("a well-formed request");
        let response = assembled()
            .oneshot(request)
            .await
            .expect("the router is infallible");
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("a complete body")
            .to_bytes();
        Answer {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        }
    }

    async fn get(path: &str) -> Answer {
        ask(Method::GET, path).await
    }

    /// Each of the four tiers answers, and answers as itself.
    #[tokio::test]
    async fn all_four_tiers_answer_through_one_router() {
        let api = get("/healthz").await;
        assert_eq!(api.status, StatusCode::OK, "the API keeps its own routes");

        let landing = get("/").await;
        assert_eq!(landing.status, StatusCode::OK, "the public page takes /");
        assert!(
            landing.body.contains("the public page"),
            "the root is the landing page, not the console shell: {}",
            landing.body
        );

        let download = get("/downloads/downloads.json").await;
        assert_eq!(download.status, StatusCode::OK, "the manifest is served");
        assert_eq!(
            download.header(header::CONTENT_TYPE),
            "application/json",
            "the console parses this"
        );

        // Two paths rather than one, because they reach the shell by different
        // routes: `/resources` is a reserved console namespace and never
        // consults the landing build at all, while `/labels` is claimed by
        // nothing and arrives here as the fallback. One probe covering only the
        // reserved name would leave the fallback itself unasserted.
        for path in ["/resources", "/labels"] {
            let console = get(path).await;
            assert_eq!(
                console.status,
                StatusCode::OK,
                "{path} is answered by the SPA"
            );
            assert!(
                console.body.contains("__sveltekit"),
                "the console's shell answers its own deep routes: {}",
                console.body
            );
        }
    }

    /// The landing page carries the landing policy and not the console's.
    ///
    /// This is the assertion that fails if the two `.layer()` calls are
    /// swapped, and the one that fails if the policy is dropped from
    /// `Landing::respond`. Both mutations leave every other test green.
    #[tokio::test]
    async fn a_landing_answer_carries_only_the_landing_policy() {
        let landing = get("/").await;
        let policy = landing.header(header::CONTENT_SECURITY_POLICY);
        assert!(
            policy.contains("frame-ancestors 'none'"),
            "the landing policy, not the console's and not none at all: {policy}"
        );
        assert!(
            !policy.contains("'wasm-unsafe-eval'") && !policy.contains("https://"),
            "the console's policy has been written over the landing one: {policy}"
        );
        assert_eq!(
            landing.header(header::CACHE_CONTROL),
            "no-cache",
            "a cutover has to be visible on the next request"
        );
        assert!(
            landing.header(header::ETAG).starts_with('"'),
            "a landing response carries a validator"
        );

        let asset = get("/_astro/Base.abc123.css").await;
        assert_eq!(
            asset.header(header::CACHE_CONTROL),
            "public, max-age=31536000, immutable",
            "a content-addressed asset is held for a year"
        );

        let console = get("/resources").await;
        assert!(
            console
                .header(header::CONTENT_SECURITY_POLICY)
                .contains("'wasm-unsafe-eval'"),
            "the console keeps its own policy: {}",
            console.header(header::CONTENT_SECURITY_POLICY)
        );
        assert_eq!(
            console.header(header::CACHE_CONTROL),
            "no-cache",
            "the shell names its chunks by hash, so a shell kept past a deploy asks for a bundle \
             that is gone: it is revalidated on every load"
        );
        assert_eq!(
            get("/_app/immutable/entry/start.abc123.js")
                .await
                .header(header::CACHE_CONTROL),
            "public, max-age=31536000, immutable",
            "the console's own content-addressed bundle is held for a year"
        );
    }

    /// A download carries its own type and freshness rule, and no policy.
    #[tokio::test]
    async fn a_download_carries_its_type_and_no_policy() {
        let installer = get("/downloads/Teachouse_0.2.0_x64-setup.exe").await;
        assert_eq!(installer.status, StatusCode::OK, "the installer is served");
        assert_eq!(
            installer.header(header::CONTENT_TYPE),
            "application/vnd.microsoft.portable-executable",
            "a browser's download prompt says what the file is"
        );
        assert_eq!(
            installer.header(header::CACHE_CONTROL),
            "public, max-age=3600",
            "the name carries the version"
        );
        assert_eq!(
            installer.header(header::CONTENT_SECURITY_POLICY),
            "",
            "a policy governs a document, and nothing renders an installer"
        );
        assert_eq!(
            get("/downloads/downloads.json")
                .await
                .header(header::CACHE_CONTROL),
            "no-cache",
            "a held manifest shows the previous release"
        );
        assert_eq!(
            get("/downloads").await.status,
            StatusCode::OK,
            "the directory itself is the console's unknown path, never a listing"
        );
        assert!(
            get("/downloads").await.body.contains("__sveltekit"),
            "and what answers it is the shell"
        );
    }

    /// The reserved namespaces hold against a landing build that claims them.
    ///
    /// The fixture really does contain `app/index.html`, `_app/immutable/…`,
    /// `resources/index.html` and `v1/nothing`, so each of these passes only
    /// because `route` refuses them before the landing probe.
    #[tokio::test]
    async fn the_reserved_namespaces_hold_against_a_hostile_landing_build() {
        for path in [
            "/app",
            "/app/settings",
            "/_app/immutable/start.js",
            "/resources",
            "/resources/9f2c8a11-0000-4000-8000-000000000000",
        ] {
            let answer = get(path).await;
            assert!(
                answer.body.contains("__sveltekit"),
                "{path} is the console's, and the landing build claims it: {} {}",
                answer.status,
                answer.body
            );
        }
        // Under a version this build serves but at no route it has. The API
        // router declines, and the landing build must not answer for it.
        let api = get("/v1/nothing").await;
        assert!(
            api.body.contains("__sveltekit"),
            "the API's namespace is never a static tier's: {}",
            api.body
        );
    }

    /// Only GET and HEAD reach a static tier, through the assembled router.
    #[tokio::test]
    async fn a_static_tier_answers_no_other_method() {
        for path in ["/", "/downloads/downloads.json"] {
            let answer = ask(Method::POST, path).await;
            assert_eq!(
                answer.status,
                StatusCode::METHOD_NOT_ALLOWED,
                "POST {path} was a 405 before this tier existed"
            );
            assert_eq!(
                answer.header(header::ALLOW),
                "GET, HEAD",
                "and the refusal says what would have been allowed"
            );
        }
        let head = ask(Method::HEAD, "/").await;
        assert_eq!(head.status, StatusCode::OK, "HEAD reads a static file");

        // The gate covers the static tiers and stops there. A console path is
        // refused by the console's own `ServeDir`, which has answered 405 to
        // every non-GET/HEAD since before any of these tiers existed and does
        // not consult its fallback for one.
        //
        // So widening the gate to `Answer::Console` changes which component
        // writes the 405 and not what a client sees — except for one byte.
        // tower-http writes `GET,HEAD` and this crate writes `GET, HEAD`, and
        // tower-http asserts its own spelling in its test suite
        // (`serve_dir/tests.rs:765`), so the separator is a contract rather
        // than an accident. That byte is the whole observable difference, and
        // it is what this asserts; a reader who finds that too fine a hook may
        // delete this knowing what it was for.
        let posted = ask(Method::POST, "/login").await;
        assert_eq!(
            posted.status,
            StatusCode::METHOD_NOT_ALLOWED,
            "a console path has always refused a POST"
        );
        assert_eq!(
            posted.header(header::ALLOW),
            "GET,HEAD",
            "and the console's own service is what refused it, not this crate's gate"
        );
    }

    /// A validator the client already holds earns a 304 with no body.
    ///
    /// Through the router rather than through either `respond`, because the
    /// header has to survive being read off the request and handed down: the
    /// whole conditional path can be deleted by passing `None` where
    /// `if-none-match` is extracted, and every other test here still passes.
    #[tokio::test]
    async fn a_held_validator_earns_a_304_from_both_static_tiers() {
        for path in ["/", "/_astro/Base.abc123.css", "/downloads/downloads.json"] {
            let first = get(path).await;
            assert_eq!(first.status, StatusCode::OK, "{path} answers");
            let etag = first.header(header::ETAG).to_owned();
            assert!(!etag.is_empty(), "{path} carries a validator");

            let request = Request::builder()
                .method(Method::GET)
                .uri(path)
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .expect("a well-formed request");
            let response = assembled()
                .oneshot(request)
                .await
                .expect("the router is infallible");
            assert_eq!(
                response.status(),
                StatusCode::NOT_MODIFIED,
                "{path} was already held under {etag}"
            );
            let body = response
                .into_body()
                .collect()
                .await
                .expect("a complete body")
                .to_bytes();
            assert!(
                body.is_empty(),
                "a 304 carries no body, and {path} sent one"
            );
        }
    }

    /// A download the directory does not hold is a 404, through the router.
    #[tokio::test]
    async fn an_unpublished_download_is_a_404() {
        let answer = get("/downloads/Teachouse_9.9.9_arm64.apk").await;
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "nothing invents a download the refresh did not publish"
        );
    }

    /// A traversal answers the console shell rather than a file.
    #[tokio::test]
    async fn a_traversal_answers_no_file() {
        let answer = get("/downloads/%2e%2e%2f%2e%2e%2fetc%2fpasswd").await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "it is not an error, it is a page"
        );
        assert!(
            answer.body.contains("__sveltekit"),
            "an encoded traversal names no file of any tier: {}",
            answer.body
        );
    }
}
