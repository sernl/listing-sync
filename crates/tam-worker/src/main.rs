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
//! Both live marketplaces reach their credential the same way, and that
//! symmetry is the point. Tes and Tpt each lease a gateway endpoint from the
//! broker per item, with the seller cookie injected server-side, so this
//! process holds no marketplace credential at any point and a compromised
//! worker can use the connections it leased without being able to exfiltrate
//! one.
//!
//! Tpt's bucket hops are the one exception and are not an exception to that
//! rule: the three-step upload leaves this process directly because those
//! hops carry an S3 signature rather than the session, and the transport's
//! host assertion refuses a session-authenticated request to the bucket and a
//! signed one to the marketplace origin. Proxying them would send the
//! seller's Tpt cookie to Amazon and buy nothing. This is the production
//! answer `docs/design/decisions.md` recommended over both keeping a
//! configured file jar and giving the broker a hand-me-the-cookie operation.
//!
//! The authorship attestation travels with the connection rather than with
//! the process, for the same reason the credential does: a process-global
//! attestation speaks for exactly one seller while `LeaseRepo::acquire` reads
//! across organisations under BYPASSRLS, so the worker would otherwise attest
//! one seller's authorship on another's listing. The broker's link step seals
//! it onto the connection row and `AuthorshipRepo` reads it back per item.
//!
//! Usage: tam-worker <engine-database-url> <worker-name> <broker-socket> \
//!            <kek-path> <store-root> [poll-ms]

#![forbid(unsafe_code)]

use std::io::Read as _;
use std::sync::atomic::{AtomicBool, Ordering};

use tam_domain::{ItemOperation, ItemOutcome};
use tam_engine::breaker::run_breaker;
use tam_engine::broker_client::{request_lease, LeasePurpose};
use tam_engine::driver::{run_item, seed_refused, DriverContext, NowSource, RunVerdict};
use tam_engine::seed::{prepare_item, seed_for_removal, seed_from_projection, ItemPreparation};
use tam_marketplace::{MarketplaceAdapter, Pause, ProjectedListing};
use tam_marketplace_tes::{GatewayTransport as TesGatewayTransport, TesAdapter};
use tam_marketplace_tpt::{
    AuthorshipDeclaration, GatewayTransport as TptGatewayTransport, TptAdapter,
};
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    AuthorshipRepo, BlobRepo, ElectionRepo, HaltRepo, ItemVerdict, JobRepo, LeaseRepo, LeasedItem,
    PipelineFileSource, RateBudgetRepo, WriteAttemptRepo,
};
use tam_types::{FailureCode, FailureDetail, Marketplace, Timestamp};
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

struct Pump {
    pool: sqlx::PgPool,
    leases: LeaseRepo,
    halts: HaltRepo,
    attempts: WriteAttemptRepo,
    budgets: RateBudgetRepo,
    broker_socket: std::path::PathBuf,
    kek: Kek,
    store_root: std::path::PathBuf,
    authorship: AuthorshipRepo,
    cancel: CancellationToken,
    unadapted_said: OncePerPass,
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
    /// Closes the window between raising an election and parking on it.
    ///
    /// `prepare_item` commits the raise in its own transaction and returns
    /// `Blocked`; the park is a separate statement here. An answer arriving
    /// in between finds the item still `leased`, so the answer's own revive
    /// matches nothing and reports `revived: 0`, and the park that follows
    /// has nothing left to clear it -- the seller waits the full day after
    /// deciding, which is the latency the answer-driven revive exists to
    /// remove. Verifying after the park rather than before it makes the
    /// check total: either this read sees the answer, or the answer sees the
    /// parked row.
    ///
    /// Only when the raise minted something. A projection blocking on a
    /// question the seller has already answered raises nothing, and
    /// requeueing on that would re-project, re-block, re-park and re-check
    /// without bound; parking it for the day is the honest outcome until the
    /// answer itself is made to apply.
    async fn revive_if_answered_meanwhile(&self, worker: &str, item: &LeasedItem, now: Timestamp) {
        match ElectionRepo::new(self.pool.clone())
            .revive_if_answered(item.org, item.mapping, now)
            .await
        {
            Ok(0) => {}
            Ok(_) => eprintln!(
                "tam-worker {worker}: item {:?} was answered while it was being parked, and \
                 is queued rather than waiting out the park",
                item.item
            ),
            Err(error) => eprintln!(
                "tam-worker {worker}: the post-park election re-check failed, so the item \
                 waits out its park: {error}"
            ),
        }
    }

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
                    Ok(()) => {
                        eprintln!(
                            "tam-worker {worker}: item {:?} parked on {gate} \
                             ({} item(s) raised, {} already open)",
                            item.item, raised.new, raised.already_open
                        );
                        // Only an election park, and only when the raise
                        // minted something: a projection blocking on a
                        // question already answered raises nothing, and
                        // requeueing on that would spin.
                        if gate == tam_storage::ELECTION && raised.new > 0 {
                            self.revive_if_answered_meanwhile(worker, item, now).await;
                        }
                    }
                    Err(error) => {
                        eprintln!("tam-worker {worker}: park failed: {error}");
                    }
                }
                return;
            }
            // Waiting is over rather than merely unsatisfied. The listing is
            // safely on both platforms, which is what the gate is for; what
            // the seller needs now is to be told the migration ended and why.
            Ok(ItemPreparation::CounterpartLost { counterpart }) => {
                let verdict = ItemVerdict {
                    outcome: ItemOutcome::Skipped,
                    failure_code: Some(FailureCode::Other),
                    failure_detail: Some(FailureDetail(format!(
                        "the listing on {counterpart:?} never bound, so this item's counterpart \
                         will not arrive; the source listing was left in place"
                    ))),
                };
                if let Err(error) = self.leases.settle(&item.lease_ref(), &verdict, now).await {
                    eprintln!("tam-worker {worker}: settling a lost counterpart failed: {error}");
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
                let Some(adapter) = self.tpt_adapter(work.worker, item).await else {
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
    ) -> Option<TesAdapter<TesGatewayTransport, PipelineFileSource<LocalObjectStore>>> {
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
            LeasePurpose::Pump,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                eprintln!("tam-worker {worker}: broker lease refused: {error}");
                return None;
            }
        };
        let transport = match TesGatewayTransport::new(gateway.endpoint.clone(), &gateway.token) {
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
    /// Tpt leases the broker's gateway exactly as Tes does: the endpoint is
    /// minted per item with the seller cookie and the mirrored CSRF header
    /// injected server-side, so this process holds no Tpt credential either.
    ///
    /// The bucket hops still leave directly, which is what makes the lease
    /// possible at all — they carry an S3 signature rather than the session,
    /// and `GatewayTransport`'s host assertion refuses the crossing in both
    /// directions.
    ///
    /// The attestation is read per item rather than configured, because the
    /// lease scan is cross-tenant: an attestation held by the process would
    /// name one seller on every tenant's listings. An item whose connection
    /// carries none is refused here rather than at the write — `submit` and
    /// `update` both open with `self.attestation()?` and answer a missing one
    /// with `UploadRejected`, which the machine settles `Failed`, where
    /// refusing up front leaves the lease to expire instead, which is the
    /// stall bias.
    async fn tpt_adapter(
        &self,
        worker: &str,
        item: &LeasedItem,
    ) -> Option<TptAdapter<TptGatewayTransport, PipelineFileSource<LocalObjectStore>, SleepingPause>>
    {
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
        let attestation = match self
            .authorship
            .for_connection(item.org, Marketplace::Tpt)
            .await
        {
            Ok(Some(record)) => AuthorshipDeclaration::attested(record.name, record.attested_at),
            Ok(None) => {
                eprintln!(
                    "tam-worker {worker}: the Tpt connection carries no authorship \
                     attestation, so the lease is left to expire rather than writing \
                     a listing nobody attested to"
                );
                return None;
            }
            Err(error) => {
                eprintln!("tam-worker {worker}: authorship lookup failed: {error}");
                return None;
            }
        };
        let gateway = match request_lease(
            &self.broker_socket,
            item.org,
            connection,
            Marketplace::Tpt,
            LeasePurpose::Pump,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                eprintln!("tam-worker {worker}: broker lease refused: {error}");
                return None;
            }
        };
        let transport = match TptGatewayTransport::new(gateway.endpoint.clone(), &gateway.token) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("tam-worker {worker}: Tpt transport build failed: {error}");
                return None;
            }
        };
        Some(TptAdapter::new(transport, self.files(item), SleepingPause).attesting(attestation))
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
        // A removal renders nothing, so it never reaches the adapter's
        // projection: the listing is being taken down rather than described.
        let seeded = projected.map_or_else(
            || Ok(seed_for_removal(item, operation)),
            |projected| seed_from_projection(adapter, item, projected),
        );
        // Both arms answer the same way, so a refusal the adapter raised
        // before the write settles the item through the path a refusal
        // raised during one already takes.
        let outcome = match seeded {
            Ok(seed) => run_item(&ctx, item, seed).await,
            Err(error) => seed_refused(&ctx, item, &error, ctx.clock.now()).await,
        };
        match outcome {
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

    let pump = Pump {
        pool: pool.clone(),
        leases: LeaseRepo::new(pool.clone()),
        halts: HaltRepo::new(pool.clone()),
        attempts: WriteAttemptRepo::new(pool.clone()),
        budgets: RateBudgetRepo::new(pool.clone()),
        broker_socket,
        kek,
        store_root,
        authorship: AuthorshipRepo::new(pool.clone()),
        cancel: cancel.clone(),
        unadapted_said: OncePerPass::new(),
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
        let now = WallClock.now();
        let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
        match leases.expire_and_steal(now, attempts_max).await {
            Ok(0) => {}
            Ok(stolen) => eprintln!("tam-worker {worker_name}: stole {stolen} expired leases"),
            Err(error) => eprintln!("tam-worker {worker_name}: steal failed: {error}"),
        }
        match leases.revive_expired(now, attempts_max).await {
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
    use super::OncePerPass;

    #[test]
    fn a_once_per_pass_refusal_speaks_once_and_again_after_the_reset() {
        let said = OncePerPass::new();
        assert!(said.first(), "the first refusal of the pass is reported");
        assert!(!said.first(), "the rest of the pass is silent");
        said.reset();
        assert!(said.first(), "the next pass reports again");
    }
}
