//! The device's pull: ask the control plane what is due, run it here, settle it.
//!
//! This is where the data plane actually is. The scheduler decides whether to
//! ask; this module asks, and everything that follows an answer of `work`
//! happens on this machine: the adapter renders the projection, composes every
//! request, and issues them under the seller's own session, and the
//! interpreter decides which request comes next.
//!
//! The server's part of the run is a ledger and a lease. It never says now, it
//! never composes, and nothing it sends is a URL, a header, a form field name
//! or an encoding.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::AtomicBool;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tam_engine_driver::driver::{run_item, DriverContext, EngineError, RunVerdict};
use tam_engine_driver::ports::{ItemLedger, ReconcileSource};
use tam_engine_driver::seed::{seed_for_removal, seed_from_projection};
use tam_engine_driver::vocabulary::{ClaimView, WorkOrder};
use tam_marketplace::transport::Transport;
use tam_marketplace::{
    AdapterError, FetchReason, ListingLocator, ListingState, MarketplaceAdapter, Pause,
    RecordedTitle, RemoteListingId, WriteAttemptId,
};
use tam_marketplace_tes::TesAdapter;
use tam_marketplace_tpt::write_model::AuthorshipDeclaration;
use tam_marketplace_tpt::{listing_state_from_status, TptAdapter};
use tam_types::{FailureCode, FailureDetail, Marketplace};

use crate::device::DeviceId;
use crate::ledger::{HttpLedger, LedgerTransport};
use crate::marketplace::{SessionTransport, TesLive, TptLive};
use crate::payload::{DevicePayloads, PayloadTransport};
use crate::run::{DeviceClock, DeviceIds, RunGate, SleepingPause};
use crate::scheduler::{PullFuture, WorkError, WorkSource};
use crate::session::SessionStore;
use crate::state::WorkEvent;

/// Where the device asks what is due.
#[must_use]
pub fn work_path(device: &DeviceId) -> String {
    format!("/v1/devices/{device}/work")
}

/// The claim's body: which marketplace this ask is for.
///
/// Owed. The route takes no body today, so the server ignores this and answers
/// whatever is due for the device. The device therefore checks the answer
/// against what it asked for, because the readiness gate is per marketplace
/// and an item for a marketplace that did not pass it must not run. See
/// `docs/notes/design/desktop-data-plane.md`.
#[must_use]
pub fn claim_body(marketplace: Marketplace) -> String {
    serde_json::json!({ "marketplace": marketplace }).to_string()
}

/// Everything this device asks of its own control plane during one run: the
/// claim and the ledger over one seam, the payload bytes over the other.
pub trait DevicePlane: PayloadTransport + LedgerTransport {}

impl<T: PayloadTransport + LedgerTransport + ?Sized> DevicePlane for T {}

impl<T: PayloadTransport + ?Sized> PayloadTransport for &T {
    fn fetch<'a>(&'a self, path: &'a str) -> crate::heartbeat::PlaneFuture<'a, Vec<u8>> {
        (**self).fetch(path)
    }
}

impl<T: LedgerTransport + ?Sized> LedgerTransport for &T {
    fn post<'a>(
        &'a self,
        path: &'a str,
        body: String,
    ) -> crate::heartbeat::PlaneFuture<'a, String> {
        (**self).post(path, body)
    }
}

impl<T: PayloadTransport> tam_marketplace::FileSource for &DevicePayloads<T> {
    fn fetch(
        &self,
        file: tam_types::FileId,
    ) -> impl core::future::Future<
        Output = Result<tam_marketplace::FileContent, tam_marketplace::FileSourceError>,
    > + Send {
        (**self).fetch(file)
    }
}

/// One run of the interpreter, boxed so the seam stays object-safe.
pub type RunFuture<'a> = Pin<Box<dyn Future<Output = Result<RunVerdict, EngineError>> + Send + 'a>>;

/// How an order becomes a run against a marketplace.
///
/// A seam, and the only one in this module: with it, the pull, the settle and
/// the order the seller sees events in are all provable without a marketplace.
/// [`LiveMarketplaces`] binds the real adapters and the seller's own session;
/// a test binds a scripted adapter to the same interpreter.
pub trait Marketplaces<P: DevicePlane>: Send + Sync {
    fn drive<'a>(
        &'a self,
        order: &'a WorkOrder,
        ledger: &'a HttpLedger<&'a P>,
        gate: &'a RunGate,
        payloads: &'a DevicePayloads<&'a P>,
    ) -> RunFuture<'a>;
}

/// The shipping binding: the marketplace's own adapter, over the seller's own
/// session, over this device's own file cache.
pub struct LiveMarketplaces {
    sessions: Arc<dyn SessionStore>,
}

impl core::fmt::Debug for LiveMarketplaces {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LiveMarketplaces").finish_non_exhaustive()
    }
}

impl LiveMarketplaces {
    #[must_use]
    pub const fn new(sessions: Arc<dyn SessionStore>) -> Self {
        Self { sessions }
    }
}

impl<P: DevicePlane> Marketplaces<P> for LiveMarketplaces {
    /// One arm per marketplace rather than a boxed adapter, because the two
    /// adapters differ in their type parameters and a trait object over
    /// `MarketplaceAdapter` would need the associated types erased.
    fn drive<'a>(
        &'a self,
        order: &'a WorkOrder,
        ledger: &'a HttpLedger<&'a P>,
        gate: &'a RunGate,
        payloads: &'a DevicePayloads<&'a P>,
    ) -> RunFuture<'a> {
        Box::pin(async move {
            // The marketplace is the inventory's, and `execute` has already
            // refused an order whose inventory is not the one it gated on.
            match order.lease.inventory.marketplace() {
                Marketplace::Tpt => {
                    let transport = SessionTransport::new(TptLive, Arc::clone(&self.sessions))
                        .map_err(|why| refusal(&why))?;
                    let adapter = tpt_adapter(transport, payloads, order);
                    interpret(&adapter, &TptCatalogue(&adapter), ledger, gate, order).await
                }
                Marketplace::Tes => {
                    let transport = SessionTransport::new(TesLive, Arc::clone(&self.sessions))
                        .map_err(|why| refusal(&why))?;
                    let adapter = TesAdapter::new(order.lease.inventory, transport, payloads)
                        .map_err(|why| refusal(&why))?;
                    interpret(&adapter, &TesCatalogue(&adapter), ledger, gate, order).await
                }
                // Unreachable through the scheduler, which walks only the
                // seller-device marketplaces, and stated rather than assumed:
                // a sanctioned marketplace's automation runs server-side under
                // its own token and never composes a request here.
                Marketplace::Etsy => Err(refusal(
                    &crate::marketplace::NoLocalTransport::NotSellerDevice(Marketplace::Etsy),
                )),
            }
        })
    }
}

/// The work source that actually runs items.
pub struct DeviceWork<P: DevicePlane, M: Marketplaces<P>> {
    device: DeviceId,
    plane: Arc<P>,
    marketplaces: M,
    /// Where the interim payload cache lives, one directory per item beneath
    /// it.
    data_dir: PathBuf,
    /// Raised when the device is revoked. Shared with every run in flight,
    /// which is how a revocation reaches the interpreter before its next
    /// marketplace request rather than after it.
    stopper: Arc<AtomicBool>,
}

impl<P: DevicePlane, M: Marketplaces<P>> core::fmt::Debug for DeviceWork<P, M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceWork")
            .field("device", &self.device)
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl<P: DevicePlane, M: Marketplaces<P>> DeviceWork<P, M> {
    #[must_use]
    pub fn new(
        device: DeviceId,
        plane: Arc<P>,
        marketplaces: M,
        data_dir: &Path,
        stopper: Arc<AtomicBool>,
    ) -> Self {
        Self {
            device,
            plane,
            marketplaces,
            data_dir: data_dir.to_path_buf(),
            stopper,
        }
    }

    async fn ask(&self, marketplace: Marketplace) -> Result<ClaimView, WorkError> {
        let reply = self
            .plane
            .post(&work_path(&self.device), claim_body(marketplace))
            .await
            .map_err(|why| WorkError(why.to_string()))?;
        serde_json::from_str(&reply).map_err(|why| WorkError(why.to_string()))
    }

    async fn pull_one(&self, marketplace: Marketplace) -> Result<Vec<WorkEvent>, WorkError> {
        match self.ask(marketplace).await? {
            ClaimView::Idle { .. } => Ok(vec![WorkEvent::Idle]),
            ClaimView::Held { next_poll_ms } => Ok(vec![WorkEvent::Held { next_poll_ms }]),
            ClaimView::Work(order) => Ok(self.execute(marketplace, *order).await),
        }
    }

    /// Runs one order to a verdict and reports what the seller sees.
    ///
    /// Never returns an error: once an item is claimed, everything that can go
    /// wrong is a verdict the interpreter already has a shape for, and the
    /// stall bias covers the rest. A failure here that surfaced as a work-source
    /// error would leave the seller with no record of an item their device did
    /// claim.
    async fn execute(&self, marketplace: Marketplace, order: WorkOrder) -> Vec<WorkEvent> {
        let item = order.lease.item.0.to_hyphenated();
        if order.lease.inventory.marketplace() != marketplace {
            return vec![WorkEvent::Failed {
                detail: format!(
                    "the control plane answered a {:?} item for a {marketplace:?} ask; the \
                     readiness gate is per marketplace, so this item is left for the tick that \
                     gated on its own",
                    order.lease.inventory.marketplace()
                ),
            }];
        }

        let mut events = vec![WorkEvent::Started { item: item.clone() }];
        let gate = RunGate::from_envelope(order.server_now_ms, order.server_deadline_ms)
            .stopped_by(Arc::clone(&self.stopper));
        // The ledger moves this run's deadline when the server extends the
        // lease. Built after the gate for that reason: the heartbeat's whole
        // purpose is that a run doing slow work keeps the item, and the gate
        // is what would otherwise stop it at the original deadline.
        let ledger = HttpLedger::new(self.device.clone(), &*self.plane).moving(gate.deadline());
        let payloads = DevicePayloads::for_item(
            self.device.clone(),
            &*self.plane,
            &self.data_dir,
            &item,
            order.payload.clone(),
        );

        let verdict = self
            .marketplaces
            .drive(&order, &ledger, &gate, &payloads)
            .await;

        // Before the verdict is reported, because the promise is that nothing
        // is kept after the item settles rather than shortly afterwards.
        if let Err(why) = payloads.discard().await {
            events.push(WorkEvent::Failed {
                detail: why.to_string(),
            });
        }

        events.push(match verdict {
            Ok(RunVerdict::Settled(outcome)) => WorkEvent::Settled { item, outcome },
            Ok(RunVerdict::Parked) => WorkEvent::Parked {
                item,
                blocked_on: ledger
                    .parked_on()
                    .await
                    .unwrap_or_else(|| "an unnamed gate".to_owned()),
            },
            Ok(RunVerdict::Abandoned { reason }) => WorkEvent::Abandoned { item, reason },
            Err(why) => WorkEvent::Abandoned {
                item,
                reason: format!("{why}"),
            },
        });
        events
    }
}

/// A refusal before the interpreter starts, in the one shape this function can
/// return. `Refused` rather than a new variant: the interpreter's answer to a
/// condition it cannot act on is the stall bias, and the lease expiring is
/// exactly that.
fn refusal(why: &dyn core::fmt::Display) -> EngineError {
    EngineError::Ledger(tam_engine_driver::vocabulary::LedgerError::Refused {
        detail: why.to_string(),
    })
}

/// The TPT adapter this order is to be run through, carrying the seller's own
/// copyright declaration where the order supplies one.
///
/// A function rather than three lines at the call site because its absence was
/// invisible: the adapter refuses every write without an attestation, which is
/// right — the declaration is the seller's statement and not a constant this
/// connector may make for them — and the device branch could not write to TPT
/// at all while nothing here supplied it. Named, it can be asserted about.
fn tpt_adapter<T: Transport, F: tam_marketplace::FileSource>(
    transport: T,
    files: F,
    order: &WorkOrder,
) -> tam_marketplace_tpt::TptAdapter<T, F, SleepingPause> {
    let adapter = TptAdapter::new(transport, files, SleepingPause);
    match order.attestation.as_ref() {
        Some(attested) => adapter.attesting(AuthorshipDeclaration::attested(
            attested.attested_by.clone(),
            tam_types::Timestamp(attested.attested_at_ms),
        )),
        None => adapter,
    }
}

/// What a work order says this run is resuming, where it says anything.
///
/// The title comes off the order rather than out of the seed's own fields:
/// those are a fresh projection of the product as it is now, and a create is
/// identified by what it recorded that it sent. A seller who renamed the
/// product between the strand and the resume would otherwise have this device
/// search their catalogue for a title the listing never carried.
fn resume_from(order: &WorkOrder) -> Option<(WriteAttemptId, RecordedTitle)> {
    order.reconcile.as_ref().map(|stranded| {
        (
            WriteAttemptId(stranded.attempt.attempt),
            RecordedTitle(stranded.title.clone()),
        )
    })
}

/// Seeds the machine from the server's preparation and pumps it.
///
/// The seed is taken here rather than sent, which is the custody line: the
/// adapter renders the field set and the intent hash is taken over what it
/// rendered, so the recorded intent is the bytes the submit will carry.
async fn interpret<A: MarketplaceAdapter, L: ItemLedger, R: ReconcileSource>(
    adapter: &A,
    // Per marketplace rather than generic over the adapter: the catalogue read
    // is an inherent method on each adapter rather than part of the seam, and
    // widening the seam for one caller would put an enumeration on every
    // adapter that has no business with one.
    reconcile: &R,
    ledger: &L,
    gate: &RunGate,
    order: &WorkOrder,
) -> Result<RunVerdict, EngineError> {
    // A reconcile is seeded from the attempt rather than from a projection:
    // its fields are never rendered and never submitted, because the whole
    // run is one read. Branching here rather than inside the seed keeps
    // `seed_from_projection` about projecting.
    let mut seed = match order.preparation.projected.as_ref() {
        Some(listing) => seed_from_projection(adapter, &order.preparation, listing)?,
        None => seed_for_removal(&order.preparation),
    };
    seed.resume = resume_from(order);
    seed.attestation = order.attestation.clone();
    let clock = DeviceClock;
    let ids = DeviceIds;
    let pause = SleepingPause;
    let context = DriverContext {
        adapter,
        reconcile,
        ledger,
        clock: &clock,
        ids: &ids,
        cancel: gate,
        pause: &pause,
    };
    run_item(&context, &order.lease, seed).await
}

impl<P: DevicePlane, M: Marketplaces<P>> WorkSource for DeviceWork<P, M> {
    fn pull(&self, marketplace: Marketplace) -> PullFuture<'_> {
        Box::pin(async move { self.pull_one(marketplace).await })
    }
}

/// Exactly one candidate identifies a stranded create; zero and several are
/// different answers and neither of them is an identification.
///
/// Zero is `Ok(None)`: the walk completed and the listing was not in it, which
/// is the only answer the port's contract lets anything ever build on. Several
/// is `Err`, because two listings answering to one recorded title is not
/// absence, it is not knowing which — and a title, unlike a marker, is not
/// unique, so this is the case the whole marker-free identification turns on.
fn only_candidate<T>(mut candidates: Vec<T>, title: &str) -> Result<Option<T>, AdapterError> {
    if candidates.len() > 1 {
        return Err(AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(format!(
                "{} of the seller's own listings answer to the recorded title {title:?}, so \
                 which one this create made cannot be told from the catalogue",
                candidates.len()
            )),
        });
    }
    Ok(candidates.pop())
}

/// What a locator asks this source to look for.
enum Search {
    Marker(String),
    Recorded(String),
}

/// A locator this source can act on, or the refusal saying it cannot.
///
/// Never `Ok(None)` for a locator it cannot search. That answer means a
/// completed enumeration that did not contain the listing, and it is the one
/// answer the port's contract says could ever justify releasing the
/// duplicate-create fence; a search this source cannot perform is
/// indeterminate and has to say so.
fn search_for(locator: &ListingLocator) -> Result<Search, AdapterError> {
    match locator {
        ListingLocator::Marker { marker, .. } => Ok(Search::Marker(marker.0.clone())),
        ListingLocator::Recorded { title, .. } => Ok(Search::Recorded(title.0.clone())),
        ListingLocator::Durable(_) => Err(AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(
                "this reconcile source searches the seller's catalogue by correlation marker \
                 or by a stranded create's recorded title, and was handed a locator that \
                 names neither"
                    .to_owned(),
            ),
        }),
    }
}

/// The reconcile port on Tes: the seller's catalogue, searched for a marker.
///
/// The contract the port states is the one `list_own_resources` already
/// keeps: the walk reaches its end or it refuses, never a truncation. That is
/// what makes `Ok(None)` mean the listing is genuinely absent from the
/// seller's catalogue rather than absent from the part of it we managed to
/// read, which is the distinction the whole reconciliation turns on.
///
/// `VerifyAttempt` rather than `FirstPartyExport`, because that is what this
/// read is: the pre-settle verification of an attempt whose fencing row is
/// still standing, justified by the row rather than by an export capability
/// nobody is exercising.
struct TesCatalogue<'a, T, F>(&'a tam_marketplace_tes::TesAdapter<T, F>);

impl<T: Transport + Sync, F: tam_marketplace::FileSource + Sync> ReconcileSource
    for TesCatalogue<'_, T, F>
{
    async fn find_listing<'a>(
        &'a self,
        locator: &'a ListingLocator,
        attempt: WriteAttemptId,
    ) -> Result<Option<RemoteListingId>, AdapterError> {
        let search = search_for(locator)?;
        let entries = self
            .0
            .list_own_resources(&FetchReason::VerifyAttempt { attempt })
            .await?;
        // `CatalogueEntry::remote`, never a URL spelled here: the write path
        // records `canonical_url`, the ledger compares bind identities as
        // strings, and the import's `teaching-resource/-{id}` form addresses
        // the same resource under a different one. A reconcile that minted the
        // import spelling would bind an identity no later revise or removal
        // could match.
        match search {
            Search::Marker(marker) => Ok(entries
                .into_iter()
                .find(|entry| entry.title.contains(&marker))
                .map(|entry| entry.remote())),
            // Exact, and narrowed to unpublished. A draft-then-publish create
            // leaves its resource a draft, so every listing the seller has
            // already published is excluded — which is the collision that
            // would otherwise matter, since a title is not unique and one of
            // the seller's live listings could easily carry this one.
            Search::Recorded(title) => only_candidate(
                entries
                    .into_iter()
                    .filter(|entry| !entry.published && entry.title == title)
                    .collect(),
                &title,
            )
            .map(|found| found.map(|entry| entry.remote())),
        }
    }
}

/// The same port on Tpt, differing only in how a listing is named.
struct TptCatalogue<'a, T, F, P>(&'a tam_marketplace_tpt::TptAdapter<T, F, P>);

impl<T: Transport + Sync, F: tam_marketplace::FileSource + Sync, P: Pause + Sync> ReconcileSource
    for TptCatalogue<'_, T, F, P>
{
    async fn find_listing<'a>(
        &'a self,
        locator: &'a ListingLocator,
        attempt: WriteAttemptId,
    ) -> Result<Option<RemoteListingId>, AdapterError> {
        let search = search_for(locator)?;
        let entries = self
            .0
            .list_own_resources(&FetchReason::VerifyAttempt { attempt })
            .await?;
        match search {
            Search::Marker(marker) => Ok(entries
                .into_iter()
                .find(|entry| entry.name.contains(&marker))
                .map(|entry| entry.remote())),
            // Narrowed by the status TPT states rather than by a guess: an
            // unknown status classifies as neither draft nor live and is
            // therefore not a candidate, which is the conservative direction.
            Search::Recorded(title) => only_candidate(
                entries
                    .into_iter()
                    .filter(|entry| {
                        entry.name == title
                            && entry.status.as_deref().and_then(listing_state_from_status)
                                == Some(ListingState::Draft)
                    })
                    .collect(),
                &title,
            )
            .map(|found| found.map(|entry| entry.remote())),
        }
    }
}

/// The two reconcile ports, driven against recorded catalogues.
///
/// These are the only non-refusal `ReconcileSource` implementations in the
/// tree and the only place a found listing acquires its durable identity, so
/// the identity each one mints is asserted against the spelling the write path
/// records rather than against itself.
#[cfg(test)]
mod reconcile_tests {
    use super::{TesCatalogue, TptCatalogue};
    use tam_engine_driver::ports::ReconcileSource;
    use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
    use tam_marketplace::transport::HttpResponse;
    use tam_marketplace::{
        AdapterError, CorrelationMarker, FileContent, FileSource, FileSourceError, ListingLocator,
        RecordedTitle, RemoteListingId, WriteAttemptId,
    };
    use tam_marketplace_tes::endpoints::{self as tes_endpoints, DraftId};
    use tam_marketplace_tes::TesAdapter;
    use tam_marketplace_tpt::endpoints as tpt_endpoints;
    use tam_marketplace_tpt::{InstantPause, TptAdapter};
    use tam_types::{FileId, InventoryId, Uuid};

    const MARKER: &str = "tam-0f0f";
    const RESOURCE: i64 = 9001;
    const PRODUCT: u64 = 12_854_712;

    /// A file source no reconcile reaches: an enumeration fetches no file.
    struct NoFiles;

    impl FileSource for NoFiles {
        fn fetch(
            &self,
            file: FileId,
        ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send
        {
            core::future::ready(Err(FileSourceError::Missing(file)))
        }
    }

    fn attempt() -> WriteAttemptId {
        WriteAttemptId(Uuid([0x0F; 16]))
    }

    fn marker_locator() -> ListingLocator {
        ListingLocator::Marker {
            marker: CorrelationMarker(MARKER.to_owned()),
            inventory: InventoryId::TesGb,
        }
    }

    fn ok_body(body: &serde_json::Value) -> HttpResponse {
        HttpResponse::plain(200, body.to_string().into_bytes())
    }

    fn tes(cassette: Cassette) -> TesAdapter<CassetteTransport, NoFiles> {
        TesAdapter::new(
            InventoryId::TesGb,
            CassetteTransport::new(cassette),
            NoFiles,
        )
        .expect("TesGb is a Tes inventory")
    }

    /// The published page, then the two empty pages the walk ends each list on.
    fn tes_catalogue(title: &str) -> Cassette {
        tes_rows(&[(RESOURCE, title, false)])
    }

    /// A catalogue of stated rows, each an id, a title and whether it is still
    /// a draft. Every row goes on the first published page; `draft` is the flag
    /// the parser reads, so the walk classifies them without a second list.
    fn tes_rows(rows: &[(i64, &str, bool)]) -> Cassette {
        let limit = tes_endpoints::CATALOGUE_PAGE_LIMIT;
        let empty = serde_json::json!([]);
        let page: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, title, draft)| {
                serde_json::json!({
                    "id": id, "title": title, "licence": "CC-BY",
                    "price": 0, "draft": draft,
                    "url": format!("/teaching-resource/fixture-{id}")
                })
            })
            .collect();
        Cassette {
            interactions: vec![
                Interaction {
                    request: tes_endpoints::list_resources_request(0, limit),
                    response: ok_body(&serde_json::Value::Array(page)),
                },
                Interaction {
                    request: tes_endpoints::list_resources_request(1, limit),
                    response: ok_body(&empty),
                },
                Interaction {
                    request: tes_endpoints::list_drafts_request(0, limit),
                    response: ok_body(&empty),
                },
            ],
        }
    }

    /// One TPT catalogue page of stated rows: an id, a name and TPT's own
    /// status string.
    fn tpt_rows(rows: &[(u64, &str, &str)]) -> Cassette {
        let results: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, name, status)| {
                serde_json::json!({
                    "id": id.to_string(), "name": name, "price": "$1.00", "status": status
                })
            })
            .collect();
        let page = serde_json::json!({"data": {"seller": {"resources": {
            "results": results,
            "pageInfo": {
                "totalResultsCount": rows.len(),
                "currentPage": 1,
                "totalPageCount": 1,
            },
        }}}});
        Cassette {
            interactions: vec![Interaction {
                request: tpt_endpoints::my_product_listings_request(2, 0),
                response: ok_body(&page),
            }],
        }
    }

    fn tpt(cassette: Cassette) -> TptAdapter<CassetteTransport, NoFiles, InstantPause> {
        TptAdapter::new(CassetteTransport::new(cassette), NoFiles, InstantPause).with_page_limit(2)
    }

    const RECORDED: &str = "Fractions pack";

    fn recorded_locator() -> ListingLocator {
        ListingLocator::Recorded {
            title: RecordedTitle(RECORDED.to_owned()),
            inventory: InventoryId::TesGb,
        }
    }

    /// The identity a found Tes listing binds is the write path's, not the
    /// import path's.
    ///
    /// This is the whole point of the assertion: `DraftId::canonical_url` is
    /// what every evidence-producing write cell records as `landed`, the
    /// ledger compares bind identities as strings, and the import spells the
    /// same resource `teaching-resource/-{id}`. A reconcile that minted the
    /// import form would bind an identity the next revise could not match.
    #[tokio::test]
    async fn a_found_tes_listing_carries_the_identity_the_write_path_records() {
        let adapter = tes(tes_catalogue(&format!("Fractions pack {MARKER}")));
        let found = TesCatalogue(&adapter)
            .find_listing(&marker_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found,
            Some(DraftId(RESOURCE).remote()),
            "the found listing is named exactly as the lost write response would have named it"
        );
        assert_eq!(
            found,
            Some(RemoteListingId::Tes {
                url: "https://www.tes.com/api/v2/resources/9001".to_owned()
            }),
            "spelled out once, so a change to the canonical form has to come past this test"
        );
    }

    /// A walk that reached its end without the marker is the one answer that
    /// could ever justify releasing the duplicate-create fence, so it is
    /// distinguished from every failure to look.
    #[tokio::test]
    async fn a_completed_tes_walk_without_the_marker_answers_absent() {
        let adapter = tes(tes_catalogue("Fractions pack"));
        let found = TesCatalogue(&adapter)
            .find_listing(&marker_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found, None,
            "the seller's whole catalogue was read and the marker was not in it"
        );
    }

    /// A walk that could not be completed is indeterminate, never absent.
    #[tokio::test]
    async fn a_tes_walk_the_transport_cannot_answer_is_indeterminate() {
        let adapter = tes(Cassette {
            interactions: Vec::new(),
        });
        let answer = TesCatalogue(&adapter)
            .find_listing(&marker_locator(), attempt())
            .await;
        assert!(
            answer.is_err(),
            "a walk that could not be performed says so; answering absent here would be \
             manufacturing the one answer that releases the fence: {answer:?}"
        );
    }

    /// The same identity discipline on Tpt, where the numeric product id is
    /// the durable name on both paths.
    #[tokio::test]
    async fn a_found_tpt_listing_carries_the_identity_the_write_path_records() {
        let rows = serde_json::json!([
            { "id": PRODUCT.to_string(), "name": format!("Worksheet {MARKER}"), "price": "$1.00" }
        ]);
        let page = serde_json::json!({"data": {"seller": {"resources": {
            "results": rows,
            "pageInfo": { "totalResultsCount": 1, "currentPage": 1, "totalPageCount": 1 },
        }}}});
        let adapter = TptAdapter::new(
            CassetteTransport::new(Cassette {
                interactions: vec![Interaction {
                    request: tpt_endpoints::my_product_listings_request(2, 0),
                    response: ok_body(&page),
                }],
            }),
            NoFiles,
            InstantPause,
        )
        .with_page_limit(2);
        let found = TptCatalogue(&adapter)
            .find_listing(&marker_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found,
            Some(RemoteListingId::Tpt {
                product_id: PRODUCT
            }),
            "the product id is the durable name the write path records too"
        );
    }

    /// One draft answering to the recorded title is the create, and binds.
    #[tokio::test]
    async fn a_recorded_title_matching_one_tes_draft_identifies_it() {
        let adapter = tes(tes_rows(&[
            (RESOURCE, RECORDED, true),
            (9002, "Something else", true),
        ]));
        let found = TesCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found,
            Some(DraftId(RESOURCE).remote()),
            "exactly one candidate survives, so it is the listing this create made"
        );
    }

    /// A published listing carrying the same title is not a candidate.
    ///
    /// This is the narrowing that makes a non-unique title usable at all: a
    /// draft-then-publish create leaves a draft, so the seller's already-live
    /// listings cannot be mistaken for it, and that is the collision most
    /// likely to happen.
    #[tokio::test]
    async fn a_tes_listing_already_published_is_not_a_candidate() {
        let adapter = tes(tes_rows(&[(RESOURCE, RECORDED, false)]));
        let found = TesCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found, None,
            "the walk completed and nothing a create could have left carried the title"
        );
    }

    /// Two drafts answering to one title is not knowing which, never absence.
    #[tokio::test]
    async fn two_tes_drafts_of_one_title_are_indeterminate() {
        let adapter = tes(tes_rows(&[
            (RESOURCE, RECORDED, true),
            (9002, RECORDED, true),
        ]));
        let answer = TesCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await;
        assert!(
            matches!(answer, Err(AdapterError::Rejected { .. })),
            "answering absent here would manufacture the one answer that could release the \
             duplicate-create fence, on a walk that found two candidates: {answer:?}"
        );
    }

    /// The same three outcomes on Tpt, narrowed by the status TPT states.
    #[tokio::test]
    async fn a_recorded_title_matching_one_tpt_draft_identifies_it() {
        let adapter = tpt(tpt_rows(&[
            (PRODUCT, RECORDED, "NOT_ACTIVE"),
            (77, "Something else", "NOT_ACTIVE"),
        ]));
        let found = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found,
            Some(RemoteListingId::Tpt {
                product_id: PRODUCT
            }),
            "exactly one draft answers to the recorded title"
        );
    }

    #[tokio::test]
    async fn a_live_tpt_product_is_not_a_candidate() {
        let adapter = tpt(tpt_rows(&[(PRODUCT, RECORDED, "ACTIVE")]));
        let found = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found, None,
            "an already-live product is not something a fresh create left behind"
        );
    }

    #[tokio::test]
    async fn two_tpt_drafts_of_one_title_are_indeterminate() {
        let adapter = tpt(tpt_rows(&[
            (PRODUCT, RECORDED, "NOT_ACTIVE"),
            (77, RECORDED, "NOT_ACTIVE"),
        ]));
        let answer = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await;
        assert!(
            matches!(answer, Err(AdapterError::Rejected { .. })),
            "two candidates is not knowing which: {answer:?}"
        );
    }

    /// A status TPT has never stated classifies as neither, so it is not a
    /// candidate — the conservative direction, and the same strictness
    /// `listing_state_from_status` states for itself.
    #[tokio::test]
    async fn a_tpt_product_of_unknown_status_is_not_a_candidate() {
        let adapter = tpt(tpt_rows(&[(PRODUCT, RECORDED, "PENDING_REVIEW")]));
        let found = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found, None,
            "a status nobody has seen is not evidence a create left this product"
        );
    }

    /// A run resumes only what the order says it resumes, and identifies it by
    /// the title the order carries rather than by the product's own.
    #[test]
    fn a_run_resumes_what_the_order_says_and_nothing_else() {
        use super::resume_from;
        use tam_engine_driver::vocabulary::{AttemptRef, ReconcileSubject};

        let mut order = super::tests::order();
        assert_eq!(
            resume_from(&order),
            None,
            "an ordinary order resumes nothing, so the run begins at the beginning"
        );

        order.reconcile = Some(ReconcileSubject {
            attempt: AttemptRef {
                attempt: tam_types::Uuid([0x5A; 16]),
                mapping: order.lease.mapping,
            },
            title: "Fractions pack".to_owned(),
        });
        assert_eq!(
            resume_from(&order),
            Some((
                WriteAttemptId(tam_types::Uuid([0x5A; 16])),
                RecordedTitle("Fractions pack".to_owned())
            )),
            "and a reconcile order resumes its attempt under the title that attempt recorded, \
             never under the one the product carries now"
        );
    }

    /// The narrowing runs before the uniqueness test, not after it.
    ///
    /// Both rows answer to the recorded title and only one is in the state a
    /// create leaves, so the draft binds. Narrowing after counting would see
    /// two candidates and answer indeterminate, which would strand every
    /// create whose seller happens to have published a listing of the same
    /// name — and that is the ordinary case, not an exotic one.
    #[tokio::test]
    async fn a_published_tes_listing_of_the_same_title_does_not_hide_the_draft() {
        let adapter = tes(tes_rows(&[
            (9002, RECORDED, false),
            (RESOURCE, RECORDED, true),
        ]));
        let found = TesCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found,
            Some(DraftId(RESOURCE).remote()),
            "the published one is not a candidate at all, so the draft is the only one"
        );
    }

    #[tokio::test]
    async fn a_live_tpt_product_of_the_same_title_does_not_hide_the_draft() {
        let adapter = tpt(tpt_rows(&[
            (77, RECORDED, "ACTIVE"),
            (PRODUCT, RECORDED, "NOT_ACTIVE"),
        ]));
        let found = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found,
            Some(RemoteListingId::Tpt {
                product_id: PRODUCT
            }),
            "the live one is not a candidate at all, so the draft is the only one"
        );
    }

    /// The title match is exact, not a prefix or a substring.
    ///
    /// A seller who drafts "Fractions pack (revised)" beside nothing else has
    /// no listing this create made, and a search matching on containment would
    /// bind that draft and settle the create on a listing it never wrote.
    #[tokio::test]
    async fn a_tes_draft_merely_containing_the_recorded_title_is_not_a_candidate() {
        let adapter = tes(tes_rows(&[(
            RESOURCE,
            &format!("{RECORDED} (revised)"),
            true,
        )]));
        let found = TesCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found, None,
            "a longer title is a different listing, and the walk completed without finding \
             the recorded one"
        );
    }

    #[tokio::test]
    async fn a_tpt_draft_merely_containing_the_recorded_title_is_not_a_candidate() {
        let adapter = tpt(tpt_rows(&[(
            PRODUCT,
            &format!("{RECORDED} (revised)"),
            "NOT_ACTIVE",
        )]));
        let found = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(found, None, "the same on Tpt: containment is not identity");
    }

    /// A product stating no status at all classifies as neither, exactly as an
    /// unrecognised one does.
    #[tokio::test]
    async fn a_tpt_product_stating_no_status_is_not_a_candidate() {
        let page = serde_json::json!({"data": {"seller": {"resources": {
            "results": [
                { "id": PRODUCT.to_string(), "name": RECORDED, "price": "$1.00" }
            ],
            "pageInfo": {
                "totalResultsCount": 1,
                "currentPage": 1,
                "totalPageCount": 1,
            },
        }}}});
        let adapter = tpt(Cassette {
            interactions: vec![Interaction {
                request: tpt_endpoints::my_product_listings_request(2, 0),
                response: ok_body(&page),
            }],
        });
        let found = TptCatalogue(&adapter)
            .find_listing(&recorded_locator(), attempt())
            .await
            .expect("a completed walk answers");
        assert_eq!(
            found, None,
            "silence about the state is not evidence a create left this product"
        );
    }

    /// An order carrying an attestation builds an adapter that gets past the
    /// attestation check; one without does not.
    ///
    /// This is the assertion whose absence hid a live defect. The desktop
    /// built its TPT adapter with no attestation at all, so every create,
    /// revise, publish and removal from a seller's own device refused before
    /// composing anything — on the branch D1 requires every TPT request to
    /// originate from. Nothing caught it because the desktop's other tests
    /// drive a scripted adapter rather than the real one.
    ///
    /// The two cases are told apart by which refusal comes back, and neither
    /// reaches the transport. Without an attestation the adapter refuses on
    /// the declaration, before it looks at the field set at all. With one it
    /// gets past that check and refuses in `listing_from_field_set` instead,
    /// on this fixture's deliberately bare field set — "the projection omitted
    /// Price". Both are `UploadRejected`, so the assertions key on the
    /// refusal's words; that second refusal, and how far it got, is the
    /// evidence.
    #[tokio::test]
    async fn an_order_carrying_an_attestation_builds_an_adapter_that_can_write() {
        use tam_engine_driver::vocabulary::Attestation;
        use tam_marketplace::{FieldSet as Set, IdempotencyKey, MarketplaceAdapter};

        let submission = |order: &super::WorkOrder| {
            let adapter = super::tpt_adapter(
                CassetteTransport::new(Cassette {
                    interactions: Vec::new(),
                }),
                NoFiles,
                order,
            );
            async move {
                adapter
                    .submit(
                        IdempotencyKey(Uuid([0x11; 16])),
                        Set {
                            entries: vec![],
                            files: vec![],
                            body_format: None,
                            appropriate_for_country: None,
                        },
                        tam_types::Timestamp(0),
                    )
                    .await
            }
        };

        // Keyed on the refusal's own words rather than its code: both cases
        // refuse `UploadRejected`, and which check refused is the whole
        // question.
        let attestation_refusal = |answer: &Result<_, AdapterError>| match answer {
            Err(AdapterError::Rejected { detail, .. }) => {
                detail.0.contains("authorship attestation")
            }
            _ => false,
        };

        let bare = super::tests::order();
        let refused = submission(&bare).await;
        assert!(
            attestation_refusal(&refused),
            "with no attestation the adapter refuses on the seller's declaration, before any \
             request: {refused:?}"
        );

        let mut attested = super::tests::order();
        attested.attestation = Some(Attestation {
            attested_by: "the seller".to_owned(),
            attested_at_ms: 1_756_000_000_000,
        });
        let reached = submission(&attested).await;
        assert!(
            !attestation_refusal(&reached),
            "and with one it is past that check, refusing on the fixture's own bare field set \
             instead — which is the only observable difference between an adapter that can \
             write and one that cannot: {reached:?}"
        );
    }

    /// Neither source may answer absent to a search it cannot perform.
    #[tokio::test]
    async fn a_reconcile_source_refuses_a_locator_it_cannot_search() {
        let durable = ListingLocator::Durable(DraftId(RESOURCE).remote());
        let tes_adapter = tes(Cassette {
            interactions: Vec::new(),
        });
        let refused = TesCatalogue(&tes_adapter)
            .find_listing(&durable, attempt())
            .await;
        assert!(
            matches!(refused, Err(AdapterError::Rejected { .. })),
            "a durable locator names no marker, and this source searches by marker: {refused:?}"
        );
        assert_eq!(
            tes_adapter.transport().remaining(),
            0,
            "and it refused without issuing a request, which an empty cassette shows by \
             never having been diverged from"
        );

        let tpt_adapter = TptAdapter::new(
            CassetteTransport::new(Cassette {
                interactions: Vec::new(),
            }),
            NoFiles,
            InstantPause,
        );
        let refused = TptCatalogue(&tpt_adapter)
            .find_listing(&durable, attempt())
            .await;
        assert!(
            matches!(refused, Err(AdapterError::Rejected { .. })),
            "the same on Tpt: {refused:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        claim_body, interpret, work_path, DevicePlane, DeviceWork, LiveMarketplaces, Marketplaces,
        RunFuture,
    };
    use crate::device::DeviceId;
    use crate::entitlement::{Claims, Entitlement, EntitlementGate};
    use crate::heartbeat::{ControlPlaneError, PlaneFuture};
    use crate::ledger::{HttpLedger, LedgerTransport};
    use crate::payload::{DevicePayloads, PayloadTransport};
    use crate::run::RunGate;
    use crate::scheduler::{Readiness, Scheduler, WorkSource};
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::{BlockReason, WorkEvent};
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::time::Duration;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tam_domain::{ItemOperation, JobItemId, StepBudget};
    use tam_engine_driver::conformance::{landed_evidence, ScriptedAdapter};
    use tam_engine_driver::driver::VerifyPolicy;
    use tam_engine_driver::memory::ScriptedReconcile;
    use tam_engine_driver::vocabulary::{
        BindDisposition, BudgetGrant, ClaimView, ItemPreparation, LeasedItem, LedgerAnswer,
        LedgerCall, PreflightStreak, Renewed, SettleEnvelope, WorkOrder,
    };
    use tam_marketplace::{CreateStrategy, FormId, IdempotencyKey, ProjectedListing};
    use tam_types::{
        ConnectionId, CopyFormat, InventoryId, JobId, MappingId, Marketplace, OrgId, PriceIntent,
        Timestamp, Uuid,
    };
    use tokio::sync::Mutex;

    const DEVICE: &str = "11112222333344445555666677778888";
    const NOW_SECONDS: i64 = 1_756_000_000;
    const NOW: Timestamp = Timestamp(NOW_SECONDS * 1_000);

    fn uuid(last: u8) -> Uuid {
        let mut raw = [0u8; 16];
        raw[15] = last;
        Uuid(raw)
    }

    pub(super) fn order() -> WorkOrder {
        WorkOrder {
            reconcile: None,
            attestation: None,
            lease: LeasedItem {
                org: OrgId(uuid(1)),
                item: JobItemId(uuid(2)),
                job: JobId(uuid(3)),
                mapping: MappingId(uuid(4)),
                inventory: InventoryId::TesGb,
                idempotency_key: IdempotencyKey(uuid(5)),
                operation: ItemOperation::Create,
                lease_epoch: 7,
                attempt_count: 0,
                requires_bound_on: None,
            },
            preparation: ItemPreparation {
                operation: ItemOperation::Create,
                projected: Some(ProjectedListing {
                    title: "Fixture".to_owned(),
                    body: "A worksheet.".to_owned(),
                    body_format: CopyFormat::Markdown,
                    price: PriceIntent::Free,
                    taxonomy: vec![],
                    grades: vec![],
                    ages: None,
                    files: vec![],
                    natives: vec![],
                    appropriate_for_country: None,
                }),
                form: FormId(uuid(9)),
                strategy: CreateStrategy::HaltOnAmbiguity,
                budget: StepBudget {
                    actions_remaining: 20,
                },
                verify: VerifyPolicy {
                    tries: 3,
                    interval_ms: 1,
                },
            },
            payload: vec![],
            server_now_ms: NOW.0,
            server_deadline_ms: NOW.0 + 300_000,
            next_poll_ms: 10_000,
        }
    }

    /// A control plane that serves one envelope and then goes idle, answers
    /// every ledger call the happy path makes, and records every path and body
    /// in order.
    struct FakePlane {
        order: Mutex<Option<WorkOrder>>,
        seen: Mutex<Vec<(String, String)>>,
    }

    impl FakePlane {
        fn serving(order: Option<WorkOrder>) -> Self {
            Self {
                order: Mutex::new(order),
                seen: Mutex::new(Vec::new()),
            }
        }

        async fn paths(&self) -> Vec<String> {
            self.seen
                .lock()
                .await
                .iter()
                .map(|(path, _)| path.rsplit('/').next().unwrap_or_default().to_owned())
                .collect()
        }

        /// The ledger calls made after the settle, in order. The interpreter
        /// notifies the seller once the item is terminal, so the settle is not
        /// literally the last request; what matters is that nothing after it
        /// writes to the item.
        async fn after_the_settle(&self) -> Vec<LedgerCall> {
            let seen = self.seen.lock().await.clone();
            let settled = seen.iter().position(|(path, _)| path.ends_with("/settle"));
            settled.map_or_else(Vec::new, |at| {
                seen.iter()
                    .skip(at + 1)
                    .filter(|(path, _)| path.ends_with("/ledger"))
                    .filter_map(|(_, body)| serde_json::from_str(body).ok())
                    .collect()
            })
        }

        async fn ledger_calls(&self) -> Vec<LedgerCall> {
            self.seen
                .lock()
                .await
                .iter()
                .filter(|(path, _)| path.ends_with("/ledger"))
                .filter_map(|(_, body)| serde_json::from_str(body).ok())
                .collect()
        }

        async fn settles(&self) -> Vec<SettleEnvelope> {
            self.seen
                .lock()
                .await
                .iter()
                .filter(|(path, _)| path.ends_with("/settle"))
                .filter_map(|(_, body)| serde_json::from_str(body).ok())
                .collect()
        }

        async fn requests(&self) -> usize {
            self.seen.lock().await.len()
        }

        /// The answer every ledger call the happy path makes needs, and
        /// nothing more: an answer of the wrong shape is what the ledger
        /// client's own tests cover.
        fn answer(call: &LedgerCall) -> LedgerAnswer {
            match *call {
                LedgerCall::ConnectionFor { .. } => LedgerAnswer::Connection {
                    connection: Some(ConnectionId(uuid(6))),
                },
                LedgerCall::PreflightFailed { .. } => LedgerAnswer::Streak {
                    streak: PreflightStreak {
                        failures: 1,
                        edge_only: true,
                    },
                },
                LedgerCall::RequestGrant { .. } => LedgerAnswer::Grant {
                    grant: BudgetGrant::Granted { used: 1 },
                },
                LedgerCall::SettleAttempt { .. } => LedgerAnswer::Bound {
                    disposition: BindDisposition::Bound,
                },
                // A heartbeat the server honours, in its own numbers. The
                // pair is deliberately not the real TTL: a fake echoing the
                // server's own constant would let a device computing its
                // deadline itself pass this test.
                LedgerCall::Renew { .. } => LedgerAnswer::Renewed {
                    renewed: Renewed {
                        server_now_ms: 0,
                        server_deadline_ms: 120_000,
                    },
                },
                LedgerCall::PreflightSucceeded { .. }
                | LedgerCall::Park { .. }
                | LedgerCall::OpenAttempt { .. }
                | LedgerCall::GateConnection { .. }
                | LedgerCall::HaltThisTenant { .. }
                | LedgerCall::RecordEvent { .. }
                | LedgerCall::Notify { .. } => LedgerAnswer::Done,
            }
        }
    }

    impl LedgerTransport for FakePlane {
        fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String> {
            Box::pin(async move {
                self.seen.lock().await.push((path.to_owned(), body.clone()));
                if path.ends_with("/work") {
                    let view = self.order.lock().await.take().map_or(
                        ClaimView::Idle {
                            next_poll_ms: 10_000,
                        },
                        |order| ClaimView::Work(Box::new(order)),
                    );
                    return serde_json::to_string(&view)
                        .map_err(|why| ControlPlaneError::Refused(why.to_string()));
                }
                if path.ends_with("/settle") {
                    return Ok("{}".to_owned());
                }
                let call: LedgerCall = serde_json::from_str(&body)
                    .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;
                serde_json::to_string(&Self::answer(&call))
                    .map_err(|why| ControlPlaneError::Refused(why.to_string()))
            })
        }
    }

    impl PayloadTransport for FakePlane {
        fn fetch<'a>(&'a self, path: &'a str) -> PlaneFuture<'a, Vec<u8>> {
            Box::pin(async move {
                self.seen
                    .lock()
                    .await
                    .push((path.to_owned(), String::new()));
                Err(ControlPlaneError::Refused(
                    "this fixture's item uploads nothing".to_owned(),
                ))
            })
        }
    }

    /// The interpreter, over a scripted adapter rather than a marketplace.
    struct Scripted;

    impl<P: DevicePlane> Marketplaces<P> for Scripted {
        fn drive<'a>(
            &'a self,
            order: &'a WorkOrder,
            ledger: &'a HttpLedger<&'a P>,
            gate: &'a RunGate,
            _payloads: &'a DevicePayloads<&'a P>,
        ) -> RunFuture<'a> {
            Box::pin(async move {
                let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
                interpret(
                    &adapter,
                    // This fixture's order carries no reconcile, so the source
                    // is never reached; one that did would say what the
                    // seller's catalogue holds.
                    &ScriptedReconcile::could_not_read("this fixture drives no reconcile"),
                    ledger,
                    gate,
                    order,
                )
                .await
            })
        }
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tam-desktop-work-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        dir
    }

    fn work(
        plane: &Arc<FakePlane>,
        data_dir: &Path,
        stopper: &Arc<AtomicBool>,
    ) -> DeviceWork<FakePlane, Scripted> {
        DeviceWork::new(
            DeviceId::from_raw(DEVICE),
            Arc::clone(plane),
            Scripted,
            data_dir,
            Arc::clone(stopper),
        )
    }

    async fn sessions_for(marketplaces: &[Marketplace]) -> Arc<dyn SessionStore> {
        let store = Arc::new(MemorySessionStore::new());
        for marketplace in marketplaces {
            store
                .put(&SessionRecord {
                    marketplace: *marketplace,
                    account_label: None,
                    captured_at: NOW,
                    device_id: DeviceId::from_raw(DEVICE),
                    jar: CookieJar::new(vec![Cookie {
                        name: "TESSession".to_owned(),
                        value: "value".to_owned(),
                    }]),
                })
                .await
                .expect("the fixture store accepts");
        }
        store
    }

    fn gate_over(marketplaces: Vec<Marketplace>) -> EntitlementGate {
        EntitlementGate::holding(Entitlement::from_verified_claims(Claims {
            sub: "org-1".to_owned(),
            aud: crate::entitlement::AUDIENCE.to_owned(),
            iss: crate::entitlement::ISSUER.to_owned(),
            device: DEVICE.to_owned(),
            marketplaces,
            exp: NOW_SECONDS + 3_600,
            grace: NOW_SECONDS + 3_600 + 86_400,
        }))
    }

    #[tokio::test]
    async fn one_envelope_is_pulled_run_and_settled_and_the_seller_sees_it_in_order() {
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(Some(order())));
        let stopper = Arc::new(AtomicBool::new(false));
        let source = work(&plane, &data_dir, &stopper);

        let events = source
            .pull(Marketplace::Tes)
            .await
            .expect("the pull returns what the device did");

        let item = uuid(2).to_hyphenated();
        assert_eq!(
            events.first(),
            Some(&WorkEvent::Started { item: item.clone() }),
            "the seller sees the claim before its outcome, and the entry names this device"
        );
        assert_eq!(events.len(), 2, "one claim, one verdict: {events:?}");
        let terminal = events.last().expect("a verdict is reported");
        assert!(
            matches!(terminal, WorkEvent::Settled { .. }),
            "the scripted submit landed and the read-back observed it, so the item settles \
             rather than stalling: {terminal:?}"
        );

        let paths = plane.paths().await;
        assert_eq!(
            paths.first().map(String::as_str),
            Some("work"),
            "the device asks before it does anything: the server never says now"
        );
        assert_eq!(
            paths.iter().filter(|path| *path == "settle").count(),
            1,
            "the evidence goes back once, on the path fenced on the holder: {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path == "ledger"),
            "the run's ledger writes go to the ledger path in between: {paths:?}"
        );
        let trailing = plane.after_the_settle().await;
        assert!(
            trailing.iter().all(|call| matches!(
                *call,
                LedgerCall::Notify { .. } | LedgerCall::RecordEvent { .. }
            )),
            "the settle is the last write to the item; only the seller's notification and the \
             journal follow it: {trailing:?}"
        );

        let calls = plane.ledger_calls().await;
        assert!(
            calls
                .iter()
                .any(|call| matches!(*call, LedgerCall::ConnectionFor { .. })),
            "the connection is read from the ledger rather than assumed: {calls:?}"
        );
        assert!(
            calls
                .iter()
                .any(|call| matches!(*call, LedgerCall::RequestGrant { .. })),
            "and the rate ceiling is asked for rather than set here: {calls:?}"
        );
        assert!(
            calls
                .iter()
                .all(|call| *call.lease() == order().lease.lease_ref()),
            "every write names the lease the server issued, so the server derives the rest"
        );

        let settles = plane.settles().await;
        assert_eq!(settles.len(), 1, "an item settles once");
        assert_eq!(
            settles[0].lease,
            order().lease.lease_ref(),
            "fenced on the item and the epoch it ran under"
        );

        assert!(
            !data_dir
                .join(crate::payload::CACHE_DIR)
                .join(&item)
                .exists(),
            "nothing the run fetched outlives the settle"
        );
        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn an_empty_queue_is_reported_as_idle_and_runs_nothing() {
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(None));
        let stopper = Arc::new(AtomicBool::new(false));
        let source = work(&plane, &data_dir, &stopper);

        assert_eq!(
            source.pull(Marketplace::Tes).await.expect("the pull lands"),
            vec![WorkEvent::Idle]
        );
        assert_eq!(
            plane.requests().await,
            1,
            "an idle answer ends the marketplace's turn; nothing else is asked"
        );
        assert_eq!(
            plane.seen.lock().await[0].1,
            claim_body(Marketplace::Tes),
            "the ask names the marketplace it is for, which is the filter the endpoint is owed"
        );
        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_revocation_in_flight_stops_the_run_before_it_opens_an_attempt() {
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(Some(order())));
        // Raised before the run, which is the state a check-in mid-tick leaves
        // behind: the stopper is the same flag `DesktopState` sets from a
        // revoked heartbeat.
        let stopper = Arc::new(AtomicBool::new(true));
        let source = work(&plane, &data_dir, &stopper);

        let events = source.pull(Marketplace::Tes).await.expect("the pull lands");

        let calls = plane.ledger_calls().await;
        assert!(
            !calls
                .iter()
                .any(|call| matches!(*call, LedgerCall::OpenAttempt { .. })),
            "this is the property that matters: a create cannot reach the marketplace without \
             opening an attempt first, so no attempt means no marketplace request was composed \
             under the seller's session after the revocation: {calls:?}"
        );

        // The item is left for another device rather than settled here.
        // Nothing was attempted, so there is no evidence to settle on: the
        // run abandons, the lease expires, and the reaper hands the work to
        // a device that is still entitled. Settling it terminally would drop
        // the seller's queued work on the strength of this device's
        // subscription rather than on anything about the item.
        assert!(
            plane.settles().await.is_empty(),
            "a revocation before any request settles nothing: the item is the seller's and \
             this device losing its entitlement is not evidence about it"
        );
        assert!(
            !matches!(events.last(), Some(WorkEvent::Settled { .. })),
            "and the seller is not shown a settle that did not happen: {events:?}"
        );
        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_revoked_device_never_reaches_the_control_plane_at_all() {
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(Some(order())));
        let stopper = Arc::new(AtomicBool::new(false));
        let source = work(&plane, &data_dir, &stopper);
        let sessions = sessions_for(&[Marketplace::Tpt, Marketplace::Tes]).await;
        let gate = gate_over(vec![Marketplace::Tpt, Marketplace::Tes]);

        let report = Scheduler::new(
            Duration::from_mins(1),
            vec![Marketplace::Tpt, Marketplace::Tes],
        )
        .tick(
            &Readiness {
                revoked: true,
                signed_in: true,
                gate: &gate,
                sessions: sessions.as_ref(),
            },
            &source,
            NOW,
        )
        .await;

        assert_eq!(
            report.blocked(),
            vec![
                (Marketplace::Tpt, BlockReason::Revoked),
                (Marketplace::Tes, BlockReason::Revoked),
            ]
        );
        assert_eq!(
            plane.requests().await,
            0,
            "the tick is refused before the pull, so a revoked device does not even ask what is \
             due, let alone compose a marketplace request"
        );
        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_marketplace_with_no_session_is_never_asked_what_is_due() {
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(Some(order())));
        let stopper = Arc::new(AtomicBool::new(false));
        let source = work(&plane, &data_dir, &stopper);
        let sessions = sessions_for(&[]).await;
        let gate = gate_over(vec![Marketplace::Tpt, Marketplace::Tes]);

        let report = Scheduler::new(
            Duration::from_mins(1),
            vec![Marketplace::Tpt, Marketplace::Tes],
        )
        .tick(
            &Readiness {
                revoked: false,
                signed_in: true,
                gate: &gate,
                sessions: sessions.as_ref(),
            },
            &source,
            NOW,
        )
        .await;

        assert_eq!(
            report.blocked(),
            vec![
                (Marketplace::Tpt, BlockReason::NoSession),
                (Marketplace::Tes, BlockReason::NoSession),
            ]
        );
        assert_eq!(
            plane.requests().await,
            0,
            "with no session there is nothing to compose a request under, so the device does not \
             claim an item it would only abandon"
        );
        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn an_order_for_another_marketplace_is_refused_rather_than_run() {
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(Some(order())));
        let stopper = Arc::new(AtomicBool::new(false));
        let source = work(&plane, &data_dir, &stopper);

        // The fixture's inventory is TesGb, and this asks as TPT.
        let events = source.pull(Marketplace::Tpt).await.expect("the pull lands");
        assert!(
            matches!(events.as_slice(), [WorkEvent::Failed { .. }]),
            "the readiness gate is per marketplace, so an item for one that did not pass it must \
             not run: {events:?}"
        );
        assert!(plane.settles().await.is_empty());
        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn the_shipping_binding_refuses_a_sanctioned_marketplace() {
        // Etsy is unreachable through the scheduler, which walks only the
        // seller-device marketplaces. Stated here rather than assumed: the
        // two-branch rule must hold at the binding as well as at the timer.
        let live = LiveMarketplaces::new(sessions_for(&[]).await);
        let data_dir = scratch();
        let plane = Arc::new(FakePlane::serving(None));
        let mut etsy = order();
        etsy.lease.inventory = InventoryId::Etsy;
        let ledger = HttpLedger::new(DeviceId::from_raw(DEVICE), &*plane);
        let payloads = DevicePayloads::for_item(
            DeviceId::from_raw(DEVICE),
            &*plane,
            &data_dir,
            "etsy",
            vec![],
        );
        let gate = RunGate::from_envelope(etsy.server_now_ms, etsy.server_deadline_ms);

        let verdict = Marketplaces::<FakePlane>::drive(&live, &etsy, &ledger, &gate, &payloads)
            .await
            .expect_err("Etsy's automation runs server-side under a sanctioned token");
        assert!(
            format!("{verdict}").contains("official API"),
            "the refusal names why: {verdict}"
        );
        drop(payloads);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[test]
    fn the_claim_path_names_the_device() {
        assert_eq!(
            work_path(&DeviceId::from_raw(DEVICE)),
            "/v1/devices/11112222333344445555666677778888/work"
        );
        assert_eq!(Ordering::SeqCst, Ordering::SeqCst);
    }
}
