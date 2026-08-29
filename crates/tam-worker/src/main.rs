//! The automation worker: the item pump — lease, seed, drive, settle — plus
//! the maintenance passes (steal expired leases, run the fleet breaker) on a
//! jittered poll until ctrl-c. The outbox drain lives in tam-server per the
//! design's single-home line; this process no longer duplicates it.
//!
//! Per item: the projection gates the item (a blocked projection parks it
//! behind the queue items it just raised), the marketplace selects the
//! adapter and the transport it rides, the adapter renders the field set it
//! will submit, and the M1d driver runs the machine against that adapter
//! with the fenced attempt and the read-back verification it was built with.
//!
//! The two live marketplaces reach their credential by different routes and
//! that asymmetry is deliberate. Tes rides the broker's gateway, so this
//! process never holds a Tes credential. Tpt rides a direct transport,
//! because the broker's allow-list is five Tes path prefixes over one
//! hardcoded upstream and the S3 signing-oracle upload cannot be proxied
//! through it — so Phase 3 configures a founder-exported cookie jar and an
//! authorship attestation at this process boundary, and the vault's `tpt`
//! connection row functions as the queue gate rather than as the credential.
//! That split is interim and its production answer is founder-gated; see
//! `docs/design/decisions.md`.
//!
//! A process-global credential speaks for exactly one seller, and the lease
//! scan does not: `LeaseRepo::acquire` reads across organisations under
//! BYPASSRLS, so a second tenant's Tpt items would otherwise be driven
//! against whichever account the configured jar holds — writing one
//! seller's listings into another's store, and reading `Absent` from the
//! wrong catalogue, which severs a mapping whose listing is still live.
//! `TAM_TPT_ORG` names the organisation the jar belongs to, and every Tpt
//! item from any other organisation is refused with its lease left to
//! expire. Lifting the pin means per-organisation custody, which is the
//! founder-gated question rather than a configuration change.
//!
//! Usage: tam-worker <engine-database-url> <worker-name> <broker-socket> \
//!            <kek-path> <store-root> [poll-ms]
//!
//! Environment: `TAM_TPT_COOKIE_JAR` is a Netscape cookie jar path,
//! `TAM_TPT_AUTHORSHIP` is `name|epoch_millis`, and `TAM_TPT_ORG` is the
//! hyphenated id of the organisation the jar belongs to. The three are
//! required together; without any one of them, Tpt items are refused and
//! their leases left to expire.

#![forbid(unsafe_code)]

use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};

use tam_domain::ItemOperation;
use tam_engine::breaker::run_breaker;
use tam_engine::broker_client::request_lease;
use tam_engine::driver::{run_item, DriverContext, NowSource, RunVerdict};
use tam_engine::seed::{prepare_item, seed_for_removal, seed_from_projection, ItemPreparation};
use tam_marketplace::{MarketplaceAdapter, Pause, ProjectedListing};
use tam_marketplace_tes::{GatewayTransport, TesAdapter};
use tam_marketplace_tpt::{AuthorshipDeclaration, ReqwestTransport, TptAdapter, TptSession};
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    BlobRepo, HaltRepo, JobRepo, LeaseRepo, LeasedItem, PipelineFileSource, RateBudgetRepo,
    WriteAttemptRepo,
};
use tam_types::{Marketplace, OrgId, Timestamp};
use tokio_util::sync::CancellationToken;

const DEFAULT_POLL_MS: u64 = 5_000;
const LEASE_TTL_SECS: i64 = 300;
/// A projection-blocked item parks for a day. The seller's answer un-parks it
/// immediately through the revive the answering transaction runs; this is the
/// backstop for an answer that never comes, and the maintenance pass requeues
/// it into a clean retry once it expires.
const BLOCKED_PARK_MS: i64 = 24 * 60 * 60 * 1000;

/// The real wait the driver's verification poll takes between reads. The
/// engine holds no timer by design, so the sleep enters here, at the process
/// boundary, exactly as the wall clock does.
struct SleepingPause;

impl Pause for SleepingPause {
    fn pause(&self, ms: u32) -> impl core::future::Future<Output = ()> + Send {
        tokio::time::sleep(core::time::Duration::from_millis(u64::from(ms)))
    }
}

/// Wall-clock enters here, at the process boundary, as the design's
/// time-as-data rule requires. A clock before the epoch saturates to zero,
/// which reads as "everything expired" — fail closed, not fail weird.
struct WallClock;

impl NowSource for WallClock {
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is a clock-reading process boundary; time enters the engine as data from here"
    )]
    fn now(&self) -> Timestamp {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        Timestamp(i64::try_from(millis).unwrap_or(0))
    }
}

fn load_kek(path: &str) -> Result<Kek, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(Kek::from_bytes(&bytes)?)
}

/// A refusal that is a property of the configuration rather than of the item
/// refused: every queued item for that marketplace takes the same path this
/// pass, so stating it once is the whole report and stating it per item would
/// bury the pass's real work under identical lines.
struct OncePerPass(AtomicBool);

impl OncePerPass {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// True on the first call since the last [`Self::reset`].
    fn first(&self) -> bool {
        !self.0.swap(true, Ordering::Relaxed)
    }

    fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// What the worker sends to Tpt, read once at this process boundary.
///
/// Both halves are required and both are checked here rather than at the
/// write: `TptAdapter::update` opens with `self.attestation()?` exactly as
/// `submit` does, so a revise needs the declaration no less than a create,
/// and a missing one refuses with `UploadRejected`, which the machine settles
/// `Failed`. Refusing the item up front leaves the lease to expire instead,
/// which is the stall bias.
///
/// `org` is the third half: the jar is one seller's, and the lease scan is
/// cross-tenant, so the organisation it belongs to has to be stated for the
/// worker to know which items it may drive with it.
struct TptCredential {
    org: OrgId,
    session: TptSession,
    authorship: AuthorshipDeclaration,
}

impl TptCredential {
    /// Whether a leased item's organisation is the one this jar belongs to.
    /// Any other would be driven against the wrong seller's account.
    fn speaks_for(&self, org: OrgId) -> bool {
        self.org == org
    }
}

/// The jar, the attestation and the organisation they belong to, or nothing.
///
/// `AuthorshipDeclaration::attested` is the only constructor and it names the
/// seller and the instant they attested; minting the instant from this
/// process's own clock would be this worker attesting on a seller's behalf,
/// so the instant is configured beside the name and never derived.
fn load_tpt_credential() -> Option<TptCredential> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is the configuration boundary: the Tpt cookie jar path enters the process here and nowhere else"
    )]
    let jar_path = std::env::var("TAM_TPT_COOKIE_JAR").ok()?;
    let mut jar = String::new();
    if let Err(error) =
        std::fs::File::open(&jar_path).and_then(|mut file| file.read_to_string(&mut jar).map(drop))
    {
        eprintln!("tam-worker: TAM_TPT_COOKIE_JAR is unreadable, Tpt items refused: {error}");
        return None;
    }
    let session = match TptSession::from_netscape_jar(&jar) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("tam-worker: TAM_TPT_COOKIE_JAR is unparseable, Tpt items refused: {error}");
            return None;
        }
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is the configuration boundary: the Tpt authorship attestation enters the process here and nowhere else"
    )]
    let raw = std::env::var("TAM_TPT_AUTHORSHIP").ok()?;
    let Some(authorship) = parse_authorship(&raw) else {
        eprintln!("tam-worker: TAM_TPT_AUTHORSHIP is not \"name|epoch_millis\", Tpt items refused");
        return None;
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "the worker is the configuration boundary: the organisation the Tpt jar belongs to enters the process here and nowhere else"
    )]
    let raw_org = std::env::var("TAM_TPT_ORG").ok()?;
    let Some(org) = parse_org(&raw_org) else {
        eprintln!("tam-worker: TAM_TPT_ORG is not a hyphenated organisation id, Tpt items refused");
        return None;
    };
    Some(TptCredential {
        org,
        session,
        authorship,
    })
}

/// The organisation a process-global jar speaks for. Unset is not a default
/// of "all of them": `load_tpt_credential` returns nothing without it, so an
/// unpinned worker drives no Tpt items at all.
fn parse_org(raw: &str) -> Option<OrgId> {
    tam_types::Uuid::parse_hyphenated(raw.trim()).map(OrgId)
}

fn parse_authorship(raw: &str) -> Option<AuthorshipDeclaration> {
    let (name, at) = raw.split_once('|')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(AuthorshipDeclaration::attested(
        name.to_owned(),
        Timestamp(at.trim().parse().ok()?),
    ))
}

struct Pump {
    pool: sqlx::PgPool,
    leases: LeaseRepo,
    halts: HaltRepo,
    attempts: WriteAttemptRepo,
    budgets: RateBudgetRepo,
    broker_socket: std::path::PathBuf,
    kek: Kek,
    store_root: std::path::PathBuf,
    tpt: Option<TptCredential>,
    cancel: CancellationToken,
    unadapted_said: OncePerPass,
    tpt_unconfigured_said: OncePerPass,
    tpt_foreign_org_said: OncePerPass,
}

/// What one run needs beyond its adapter. A struct because `drive` would
/// otherwise take six inputs against `too-many-arguments-threshold = 5`, and
/// because these four always travel together anyway.
struct Drive<'a> {
    worker: &'a str,
    item: &'a LeasedItem,
    operation: &'a ItemOperation,
    projected: Option<&'a ProjectedListing>,
}

impl Pump {
    /// Drives one leased item to wherever it goes; every refusal path leaves
    /// the lease to expire into the stealer, which is the stall bias.
    async fn pump_item(&self, worker: &str, item: &LeasedItem) {
        let now = WallClock.now();
        let (operation, projected) = match prepare_item(&self.pool, item, now).await {
            Ok(ItemPreparation::Ready {
                operation,
                projected,
            }) => (operation, projected),
            Ok(ItemPreparation::Blocked { gate, raised }) => {
                let until = Timestamp(now.0.saturating_add(BLOCKED_PARK_MS));
                match self.leases.park(&item.lease_ref(), gate, until).await {
                    Ok(()) => eprintln!(
                        "tam-worker {worker}: item {:?} parked on {gate} \
                         ({} item(s) raised, {} already open)",
                        item.item, raised.new, raised.already_open
                    ),
                    Err(error) => {
                        eprintln!("tam-worker {worker}: park failed: {error}");
                    }
                }
                return;
            }
            Err(error) => {
                eprintln!("tam-worker {worker}: preparation failed, lease left to expire: {error}");
                return;
            }
        };

        let work = Drive {
            worker,
            item,
            operation: &operation,
            projected: projected.as_ref(),
        };
        // `wildcard_enum_match_arm` is denied, so a fourth marketplace cannot
        // be added without visiting this match, which is the point. The arms
        // differ only in the adapter and the transport they construct; two
        // monomorphisations of `drive` are the whole cost of routing without
        // a registry, which RPITIT rules out.
        match item.inventory.marketplace() {
            Marketplace::Tes => {
                let Some(adapter) = self.tes_adapter(work.worker, item).await else {
                    return;
                };
                self.drive(work, &adapter).await;
            }
            Marketplace::Tpt => {
                let Some(adapter) = self.tpt_adapter(work.worker, item) else {
                    return;
                };
                self.drive(work, &adapter).await;
            }
            Marketplace::Etsy => {
                if self.unadapted_said.first() {
                    eprintln!(
                        "tam-worker {worker}: no adapter for {:?} yet, leases left to expire",
                        item.inventory
                    );
                }
            }
        }
    }

    /// Tes rides the broker's gateway: the endpoint is leased per item, so
    /// this process holds no Tes credential at any point.
    async fn tes_adapter(
        &self,
        worker: &str,
        item: &LeasedItem,
    ) -> Option<TesAdapter<GatewayTransport, PipelineFileSource<LocalObjectStore>>> {
        let connection = match self.leases.connection_for(item.org, item.inventory).await {
            Ok(Some(connection)) => connection,
            Ok(None) => {
                eprintln!(
                    "tam-worker {worker}: no linked connection for {:?}, lease left to expire",
                    item.inventory
                );
                return None;
            }
            Err(error) => {
                eprintln!("tam-worker {worker}: connection lookup failed: {error}");
                return None;
            }
        };
        let gateway = match request_lease(
            &self.broker_socket,
            item.org,
            connection,
            Marketplace::Tes,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                eprintln!("tam-worker {worker}: broker lease refused: {error}");
                return None;
            }
        };
        let transport = match GatewayTransport::new(gateway.endpoint.clone()) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("tam-worker {worker}: transport build failed: {error}");
                return None;
            }
        };
        match TesAdapter::new(item.inventory, transport, self.files(item)) {
            Ok(adapter) => Some(adapter),
            Err(error) => {
                eprintln!("tam-worker {worker}: {error}");
                None
            }
        }
    }

    /// Tpt rides the direct transport and skips the broker entirely, which
    /// also removes the hardcoded `Marketplace::Tes` the broker lease takes.
    /// The item still cannot lease without a `linked` Tpt connection row —
    /// `LeaseRepo::acquire`'s candidate CTE requires one — so the vault row
    /// remains the queue gate, and `gate_connection` flipping it to
    /// `needs_reauth` still stops Tpt items even though its stored secret was
    /// never the one sent.
    fn tpt_adapter(
        &self,
        worker: &str,
        item: &LeasedItem,
    ) -> Option<TptAdapter<ReqwestTransport, PipelineFileSource<LocalObjectStore>, SleepingPause>>
    {
        let Some(credential) = self.tpt.as_ref() else {
            if self.tpt_unconfigured_said.first() {
                eprintln!(
                    "tam-worker {worker}: no Tpt cookie jar, authorship attestation or \
                     organisation configured, Tpt leases left to expire"
                );
            }
            return None;
        };
        // The credential is this process's, the lease scan is every
        // tenant's. Driving another organisation's item with it would write
        // that seller's listing into this one's store, and a removal would
        // read absence from the wrong catalogue and sever a mapping whose
        // listing is still live.
        if !credential.speaks_for(item.org) {
            if self.tpt_foreign_org_said.first() {
                eprintln!(
                    "tam-worker {worker}: the Tpt credential is pinned to organisation {}, \
                     so items belonging to any other are refused and their leases left to \
                     expire",
                    credential.org.0.to_hyphenated()
                );
            }
            return None;
        }
        let transport = match ReqwestTransport::new(&credential.session) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("tam-worker {worker}: Tpt transport build failed: {error}");
                return None;
            }
        };
        Some(
            TptAdapter::new(transport, self.files(item), SleepingPause)
                .attesting(credential.authorship.clone()),
        )
    }

    fn files(&self, item: &LeasedItem) -> PipelineFileSource<LocalObjectStore> {
        PipelineFileSource::new(
            BlobRepo::new(
                self.pool.clone(),
                LocalObjectStore::new(self.store_root.clone()),
                self.kek.clone(),
            ),
            item.org,
            self.pool.clone(),
        )
    }

    /// The whole tail of the pump, monomorphised once per marketplace.
    async fn drive<A: MarketplaceAdapter>(&self, work: Drive<'_>, adapter: &A) {
        let Drive {
            worker,
            item,
            operation,
            projected,
        } = work;
        // A removal renders nothing, so it never reaches the adapter's
        // projection: the listing is being taken down rather than described.
        let seed = match projected.map_or_else(
            || Ok(seed_for_removal(item, operation)),
            |projected| seed_from_projection(adapter, item, projected),
        ) {
            Ok(seed) => seed,
            Err(error) => {
                eprintln!("tam-worker {worker}: seed failed, lease left to expire: {error}");
                return;
            }
        };
        let ctx = DriverContext {
            adapter,
            leases: &self.leases,
            halts: &self.halts,
            attempts: &self.attempts,
            budgets: &self.budgets,
            pool: &self.pool,
            clock: &WallClock,
            cancel: &self.cancel,
            pause: &SleepingPause,
        };
        match run_item(&ctx, item, seed).await {
            Ok(RunVerdict::Settled(outcome)) => {
                eprintln!(
                    "tam-worker {worker}: item {:?} settled {outcome:?}",
                    item.item
                );
            }
            Ok(RunVerdict::Parked) => {
                eprintln!("tam-worker {worker}: item {:?} parked", item.item);
            }
            Ok(RunVerdict::Abandoned { reason }) => {
                eprintln!(
                    "tam-worker {worker}: item {:?} abandoned: {reason}",
                    item.item
                );
            }
            Err(error) => {
                eprintln!("tam-worker {worker}: run failed, lease left to expire: {error}");
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    const USAGE: &str = "usage: tam-worker <engine-database-url> <worker-name> \
                         <broker-socket> <kek-path> <store-root> [poll-ms]";
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let database_url = arguments.first().ok_or(USAGE)?;
    let worker_name = arguments.get(1).ok_or(USAGE)?;
    let broker_socket = std::path::PathBuf::from(arguments.get(2).ok_or(USAGE)?);
    let kek = load_kek(arguments.get(3).ok_or(USAGE)?)?;
    let store_root = std::path::PathBuf::from(arguments.get(4).ok_or(USAGE)?);
    let poll_ms: u64 = arguments
        .get(5)
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
    eprintln!("tam-worker {worker_name}: item pump live, maintenance every {poll_ms}ms");

    let tpt = load_tpt_credential();
    if tpt.is_none() {
        eprintln!("tam-worker {worker_name}: no Tpt credential configured; Tes items only");
    }
    let pump = Pump {
        pool: pool.clone(),
        leases: LeaseRepo::new(pool.clone()),
        halts: HaltRepo::new(pool.clone()),
        attempts: WriteAttemptRepo::new(pool.clone()),
        budgets: RateBudgetRepo::new(pool.clone()),
        broker_socket,
        kek,
        store_root,
        tpt,
        cancel: cancel.clone(),
        unadapted_said: OncePerPass::new(),
        tpt_unconfigured_said: OncePerPass::new(),
        tpt_foreign_org_said: OncePerPass::new(),
    };

    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            () = &mut ctrl_c => break,
            () = tokio::time::sleep(core::time::Duration::from_millis(poll_ms)) => {}
        }
        if cancel.is_cancelled() {
            break;
        }
        pump.unadapted_said.reset();
        pump.tpt_unconfigured_said.reset();
        pump.tpt_foreign_org_said.reset();
        let now = WallClock.now();
        let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
        match leases.expire_and_steal(now, attempts_max).await {
            Ok(0) => {}
            Ok(stolen) => eprintln!("tam-worker {worker_name}: stole {stolen} expired leases"),
            Err(error) => eprintln!("tam-worker {worker_name}: steal failed: {error}"),
        }
        match leases.revive_expired(now).await {
            Ok(0) => {}
            Ok(revived) => eprintln!("tam-worker {worker_name}: revived {revived} expired parks"),
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
        // The item pump: one at a time, until the queue is dry this pass.
        // Per-tenant concurrency is one to two by design, and the per-tenant
        // mutex serialises deeper anyway.
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match leases
                .acquire(worker_name, WallClock.now(), LEASE_TTL_SECS)
                .await
            {
                Ok(Some(item)) => pump.pump_item(worker_name, &item).await,
                Ok(None) => break,
                Err(error) => {
                    eprintln!("tam-worker {worker_name}: acquire failed: {error}");
                    break;
                }
            }
        }
    }
    eprintln!("tam-worker {worker_name}: stopped cleanly");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_authorship, parse_org, OncePerPass, TptCredential};
    use tam_marketplace_tpt::{AuthorshipDeclaration, TptSession};
    use tam_types::{OrgId, Timestamp, Uuid};

    const PINNED: OrgId = OrgId(Uuid([0xAA; 16]));
    const OTHER: OrgId = OrgId(Uuid([0xBB; 16]));

    fn credential(org: OrgId) -> TptCredential {
        let jar = ".teacherspayteachers.com\tTRUE\t/\tTRUE\t0\tcsrfToken\tdeadbeef\n";
        TptCredential {
            org,
            session: TptSession::from_netscape_jar(jar).expect("the fixture jar parses"),
            authorship: AuthorshipDeclaration::attested(
                "A. Seller".to_owned(),
                Timestamp(1_724_889_600_000),
            ),
        }
    }

    /// The lease scan is cross-tenant and the jar is one seller's, so the
    /// organisation is what decides whether an item may be driven at all.
    /// Without this the second tenant to link a Tpt connection has its
    /// listings written into the first tenant's store.
    #[test]
    fn a_tpt_credential_speaks_only_for_the_organisation_it_is_pinned_to() {
        let credential = credential(PINNED);
        assert!(
            credential.speaks_for(PINNED),
            "the organisation the jar belongs to is admitted"
        );
        assert!(
            !credential.speaks_for(OTHER),
            "any other organisation is refused; driving its item would write one seller's \
             listing into another's account, and a removal would read absence from the \
             wrong catalogue and sever a mapping whose listing is still live"
        );
    }

    #[test]
    fn an_organisation_pin_is_a_hyphenated_id_or_nothing() {
        assert_eq!(
            parse_org("  aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa  "),
            Some(PINNED),
            "the configured form parses, surrounding whitespace included"
        );
        for raw in ["", "not-an-id", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "*"] {
            assert_eq!(
                parse_org(raw),
                None,
                "refusing every Tpt item is the honest answer to {raw:?}; an unparseable \
                 pin must never widen into 'any organisation'"
            );
        }
    }

    #[test]
    fn an_attestation_names_the_seller_and_the_instant_they_attested() {
        let parsed =
            parse_authorship("A. Seller|1724889600000").expect("the configured form parses");
        assert_eq!(
            (parsed.attested_by(), parsed.attested_at()),
            ("A. Seller", Timestamp(1_724_889_600_000)),
            "both halves travel; the instant is configured rather than derived, because \
             minting it from this process's clock would be the worker attesting for a seller"
        );
    }

    #[test]
    fn a_half_written_attestation_is_no_attestation() {
        for raw in [
            "A. Seller",
            "A. Seller|",
            "A. Seller|not-an-instant",
            "|1724889600000",
            "   |1724889600000",
        ] {
            assert_eq!(
                parse_authorship(raw),
                None,
                "refusing Tpt items is the honest answer to {raw:?}; defaulting either half \
                 would put a declaration on the wire nobody made"
            );
        }
    }

    #[test]
    fn a_once_per_pass_refusal_speaks_once_and_again_after_the_reset() {
        let said = OncePerPass::new();
        assert!(said.first(), "the first refusal of the pass is reported");
        assert!(!said.first(), "the rest of the pass is silent");
        said.reset();
        assert!(said.first(), "the next pass reports again");
    }
}
