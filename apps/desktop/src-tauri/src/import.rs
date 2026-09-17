//! The catalogue import, on the machine that holds the seller's session.
//!
//! Decision D27 puts file ingest here, and this is the pass that makes it
//! true for a whole shop rather than for one upload. The device enumerates the
//! seller's own catalogue, and for each resource reads the listing, fetches
//! the bundle, decides what the payload actually is, measures it, and reports
//! what it saw. The server receives descriptions and never bytes.
//!
//! What is kept and what is not is the whole point, so it is stated here
//! rather than left to the code. The payload bytes exist in this process for
//! the length of one resource and are dropped before the next is fetched:
//! nothing accumulates, nothing is written to disk, and a catalogue of five
//! hundred resources costs the memory of the largest one rather than of all of
//! them. The one thing that survives is the cover, because Q-c decided the
//! console may hold a derived thumbnail so a seller can see their own
//! catalogue, and a 512 by 384 image is not the thing they sell.
//!
//! The cover travels inside the page rather than through a separate upload.
//! That is a departure from the design note, and it is the better shape for
//! two reasons: the page becomes atomic, so a resource is described and its
//! cover stored together or not at all rather than leaving an orphaned blob
//! behind a failed page; and the upload route would run the ingest pipeline
//! over an image that this pass has already produced, generating a cover of a
//! cover.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::sync::Arc;

use tam_marketplace::ImportedListing;
use tam_types::{ContentHash, FileKind, Marketplace, ScanOutcome, Timestamp};
use tokio::sync::Mutex;

// The page vocabulary is `tam-engine-driver`'s, because `tam-api` consumes
// exactly what this pass produces and a server struct that happens to match a
// client one is a coincidence rather than a contract. Re-exported rather than
// merely imported so this module's surface is unchanged by where the
// definitions now live.
pub use tam_engine_driver::import::{
    base64, ContentType, Cover, FileName, Fingerprint, ImportLease, ImportPage,
    ImportProgressReport, ImportReasonCode, ImportStage, ListedResource, Locator, NotReportable,
    ObservedFile, ObservedResource, Reason, SkippedResource, CONTENT_TYPE_MAX, COVER_BYTES_MAX,
    LOCATOR_MAX, NAME_MAX, PNG_MAGIC, REASON_MAX,
};

use crate::entitlement::EntitlementGate;
use crate::heartbeat::ControlPlaneError;
use crate::ledger::LedgerTransport;
use crate::state::DesktopState;

/// The control-plane path one page of the catalogue is posted to.
///
/// A free function so the tests name the same expression the pass uses rather
/// than a copy of it, as every other device path in this crate is.
#[must_use]
pub fn import_path(device: &crate::device::DeviceId) -> String {
    format!("/v1/devices/{device}/import")
}

/// The control-plane path the device reads its selection from.
///
/// A free function beside [`import_path`] for the same reason: the wire test
/// names this expression rather than a second spelling of it.
#[must_use]
pub fn selection_path(device: &crate::device::DeviceId, run: tam_types::Uuid) -> String {
    format!(
        "/v1/devices/{device}/import/{}/selection",
        uuid::Uuid::from_bytes(run.0).as_hyphenated()
    )
}

/// The control-plane path the device asks for the organisation's open import
/// run on, at every check-in.
///
/// A free function beside the two above for the same reason, and one the
/// device reads rather than one the server pushes: D1 keeps "do it now" off
/// the wire, so a scheduled pull the seller configured on the console becomes
/// a run sitting in `reading` state until the device next asks.
#[must_use]
pub fn open_import_path(device: &crate::device::DeviceId) -> String {
    format!("/v1/devices/{device}/import/open")
}

/// The path this device claims a run on, taking the fence with it.
///
/// A free function beside the others for the same reason: the wire test names
/// this expression rather than a second spelling of it.
#[must_use]
pub fn claim_path(device: &crate::device::DeviceId, run: tam_types::Uuid) -> String {
    format!(
        "/v1/devices/{device}/import/{}/claim",
        uuid::Uuid::from_bytes(run.0).as_hyphenated()
    )
}

/// The path the fence is renewed on, which the device posts to on its own
/// timer rather than as part of any marketplace work.
#[must_use]
pub fn renew_path(device: &crate::device::DeviceId, run: tam_types::Uuid) -> String {
    format!(
        "/v1/devices/{device}/import/{}/renew",
        uuid::Uuid::from_bytes(run.0).as_hyphenated()
    )
}

/// The path this device submits an abandonment on.
///
/// The device's own route, in the same family as the claim, the renewal and
/// the progress report rather than a spelling of its own: they are all "this
/// device's work on this run", and one convention is one thing to remember.
///
/// The right route whenever this device knows the attempt it held: it carries
/// the fence, it is idempotent — a run already abandoned answers that it is
/// abandoned rather than refusing — and it deliberately admits a lapsed
/// lease, because the device that owned the attempt is still the device
/// entitled to stop it. A later attempt's, or another device's, is refused.
#[must_use]
pub fn device_stop_path(device: &crate::device::DeviceId, run: tam_types::Uuid) -> String {
    format!(
        "/v1/devices/{device}/import/{}/stop",
        uuid::Uuid::from_bytes(run.0).as_hyphenated()
    )
}

/// The path a stage, its counts and any reason are reported on.
///
/// Separate from the page path because a report is not a page: it carries no
/// resource, it is accepted for a device that holds no marketplace session —
/// which is the whole point, since "I have no session for your shop" is a
/// thing only this device knows and must be able to say — and it is what the
/// console reads as the run's stage.
#[must_use]
pub fn progress_path(device: &crate::device::DeviceId, run: tam_types::Uuid) -> String {
    format!(
        "/v1/devices/{device}/import/{}/progress",
        uuid::Uuid::from_bytes(run.0).as_hyphenated()
    )
}

/// One import run this device may owe work on, as the device reads it.
///
/// `listed` and `selected` are what the run already holds rather than what
/// the device remembers doing, so a run listed by another device — or by this
/// one before it restarted — is not walked again. The device decides only
/// which half it owes from these two facts.
///
/// The route answers a list rather than one of these: an organisation may
/// have a Tes run and a TPT run open at once, and the singleton this replaced
/// is what made one source block the other.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OpenImportRun {
    pub run: tam_types::Uuid,
    /// Which shop, read from the run rather than chosen here, exactly as
    /// [`crate::heartbeat::ControlPlane::import_run_facts`] is for a run the
    /// console opened.
    pub source: tam_types::InventoryId,
    /// Whether the shop has already been enumerated into this run.
    pub listed: bool,
    /// Whether a selection has been recorded for it, by the seller on the
    /// console or by the server on a scheduled run.
    pub selected: bool,
    /// Whether the seller's cadence minted this run rather than a press.
    ///
    /// `serde(default)` for the reason every added field on this wire is: a
    /// device one version behind must keep reading the route it already reads.
    /// It is the difference between "nobody has come for this yet, which is
    /// ordinary" and "somebody pressed a button and is waiting".
    #[serde(default)]
    pub scheduled: bool,
    /// Which device currently holds the fence, where one does.
    ///
    /// A run another live device owns is one this one leaves alone: reading a
    /// shop twice at once is two rounds of marketplace requests for one run,
    /// and the fence is what decides between them rather than a race.
    #[serde(default)]
    pub owner_device: Option<String>,
}

/// Which half of an import a device is doing.
///
/// The run's own `listed` and `selected` flags decide it, not a memory of
/// what this process did: a run listed by another device is not listed again,
/// and a run whose selection nobody has recorded is nobody's work yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// Read the shop and post what is in it, so the seller can choose.
    Discover,
    /// Read what the selection names.
    Describe,
}

impl RunPhase {
    /// The stage the server displays while this phase is running.
    #[must_use]
    pub const fn stage(self) -> ImportStage {
        match self {
            Self::Discover => ImportStage::Discovering,
            Self::Describe => ImportStage::Reading,
        }
    }

    /// What this device owes on a run it found, or nothing.
    ///
    /// A run that is listed but whose selection nobody has recorded yet is the
    /// seller still choosing, and there is nothing owed until they have.
    #[must_use]
    pub const fn owed(open: &OpenImportRun) -> Option<Self> {
        if !open.listed {
            Some(Self::Discover)
        } else if open.selected {
            Some(Self::Describe)
        } else {
            None
        }
    }
}

/// The name the console invokes and the application registers.
///
/// One constant on this side too, so the registration test names the same
/// string the console's own constant does rather than a third spelling.
pub const START_IMPORT_COMMAND: &str = "start_import";

/// The second half of the same flow, invoked once the seller has ticked.
pub const CONTINUE_IMPORT_COMMAND: &str = "continue_import";

/// The command that stops one run's work on this device.
///
/// A third name rather than a flag on the other two, because it is a
/// different act with a different answer: starting asks this device to take a
/// run, and stopping asks it to put one down, which it can do while offline
/// and while holding nothing the server has acknowledged yet.
pub const STOP_IMPORT_COMMAND: &str = "stop_import";

/// How often the fence is renewed.
///
/// The server's approved fifteen seconds against a sixty-second lease, so
/// three consecutive failures are survivable. Renewal runs on its own task
/// and never behind a marketplace request, which is the property that made
/// the old arrangement untruthful: a device blocked on a slow shop stopped
/// renewing, and nothing on the server could tell that from a device that had
/// gone away.
pub const RENEW_EVERY: Duration = Duration::from_secs(15);

/// How long a page may hold described resources before it is posted anyway.
///
/// The page size is still the bound on how much work one page risks; this is
/// the bound on how long the seller waits to see any of it. A shop of six
/// must not sit behind a twenty-five-item threshold.
pub const FLUSH_AFTER: Duration = Duration::from_secs(15);

/// The least time between two progress reports, so a five-hundred-resource
/// shop is a bounded number of small posts rather than one per resource.
///
/// A stage change and the last resource of a phase are reported regardless,
/// because those are the two the console's next action depends on.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(2);

/// How many consecutive page submissions this device offers a server that
/// keeps failing them before it stops taking the run by itself.
///
/// Three, and the same shape [`RENEW_EVERY`] is chosen against: the offer
/// itself and two more in case what failed was the connection rather than
/// the server. Past that, offering the page again is evidence of nothing —
/// and each offer is a fresh claim, which supersedes the run's fence and
/// starts the selection again. The run of 2026-09-16 climbed from attempt
/// twelve to attempt fifty-three in twenty-four minutes that way, with one
/// of two resources described.
///
/// A bound on this device's own reclaims rather than on the run: the page
/// stays queued, the reason stays on the run, and an explicit press resumes
/// it.
pub const OFFERS_BEFORE_PAUSE: u32 = 3;

/// A listed row's marketplace resource id.
///
/// Both marketplaces address a resource by a number, and the page carries it
/// as the string the locator is; a row whose locator is not one is dropped
/// rather than guessed at, because a resource this device cannot address is
/// one it cannot read either.
#[must_use]
pub fn resource_id(listed: &ListedResource) -> Option<i64> {
    listed.locator.as_str().parse().ok()
}

/// How long the catalogue walk may take before it is refused.
///
/// The command waits for this, so the bound is what stops a slow marketplace
/// turning a button press into a window that never answers. Five minutes is
/// generous against a several-hundred-resource shop walked a page at a time and
/// short against a seller's patience; a bound this large exists to catch a
/// stall rather than to pace a healthy read.
pub const ENUMERATION_BUDGET: core::time::Duration = core::time::Duration::from_mins(5);

/// How many resources one page carries.
///
/// A page is the unit of resumability rather than of efficiency: the server
/// records a breadcrumb per resource, so a page that fails is re-walked and
/// one that succeeded is skipped. Small enough that a failure costs little
/// work, large enough that a five-hundred-resource shop is not five hundred
/// round trips.
pub const PAGE_SIZE: usize = 25;

/// Why the seller's catalogue, or one resource in it, could not be read.
///
/// Its own type rather than [`crate::heartbeat::ControlPlaneError`], whose
/// sentences name the control plane: every failure here is on the marketplace
/// side of this device, and the seller reads these words on the request page
/// as the reason a listing did not cross.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// This device could compose no request for the marketplace: it holds no
    /// session for it, or the client over the one it holds could not be built.
    NoClient(String),
    /// The marketplace answered, and this is what it said.
    Marketplace {
        marketplace: Marketplace,
        why: String,
    },
}

impl core::fmt::Display for SourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoClient(why) => f.write_str(why),
            Self::Marketplace { marketplace, why } => write!(f, "{marketplace:?} answered: {why}"),
        }
    }
}

impl core::error::Error for SourceError {}

/// One read of the seller's catalogue, boxed for the same reason
/// [`crate::heartbeat::PlaneFuture`] is.
pub type SourceFuture<'a, T> =
    core::pin::Pin<Box<dyn core::future::Future<Output = Result<T, SourceError>> + Send + 'a>>;

/// How a catalogue walk says how far it has got.
///
/// A bounded callback rather than a channel or a second task: it is called on
/// the walk's own task, it is handed one number, and it returns nothing. The
/// reporting is the worker's — the walk records, one loop reports — so there
/// is no second pipeline and no way for a chatty marketplace to turn a
/// discovery into a flood of control-plane calls.
pub type CatalogueProgress = dyn Fn(u32) + Send + Sync;

/// The seller's own catalogue, as this device can read it.
///
/// A seam over the marketplace adapter rather than the adapter itself, for the
/// reason every seam in this crate exists: the pass is then provable without a
/// marketplace, and the adapter's associated types — how one marketplace names
/// a catalogue row and addresses a resource — stay at the one edge that knows
/// them.
pub trait CatalogueSource: Send + Sync {
    /// Every resource the seller has, whole, as the enumeration saw it.
    ///
    /// A walk that cannot reach its end refuses rather than returning a
    /// truncation, which is the contract `list_own_resources` already keeps:
    /// a short catalogue read as complete would silently migrate part of a
    /// shop.
    ///
    /// Rows rather than bare ids, because the seller picks from this: the
    /// selection step shows a title and a price, and both are already on the
    /// catalogue row. Asking for them a second time would be a request per
    /// resource before the seller has chosen anything.
    ///
    /// `found` is called with the running total after each page the walk
    /// reads, and is the only thing this seam says about how the walk is
    /// going. It exists because the walk is several marketplace requests
    /// behind one future: before it, a seller watching an import was shown
    /// nothing at all until every request had answered, which on a large shop
    /// is minutes of a run that looks stalled. It carries a count and not a
    /// cursor, because neither marketplace offers a stable one and an offset
    /// this device resumed from would be a snapshot it does not have.
    fn list<'a>(&'a self, found: &'a CatalogueProgress) -> SourceFuture<'a, Vec<ListedResource>>;

    /// One listing, verbatim, for canonicalisation.
    fn read(&self, resource: i64) -> SourceFuture<'_, ImportedListing>;

    /// The bytes of one resource's bundle, where the marketplace has any to
    /// hand over.
    ///
    /// `Ok(None)` is an absence the marketplace itself stated: it answered,
    /// and its answer was that this resource has no file to download. `Err`
    /// is a fetch that was attempted and failed — a lapsed session, an
    /// unreachable host, a rate limit, an answer nothing could parse. The
    /// difference is what the seller reads and what their catalogue ends up
    /// holding: a resource whose file could not be fetched is skipped and
    /// named, and one that genuinely has no file still crosses, carrying its
    /// listing and saying so. A binding that reported a failure as an
    /// absence would import a catalogue entry with no file for a resource
    /// whose file was there, silently, which is the one outcome this
    /// distinction exists to prevent.
    ///
    /// Both Tes and TPT fetch their bundles. The Tes binding answers `None`
    /// only for the adapter's own no-published-bundle verdict, which that
    /// adapter reaches by confirming the absence against the route a
    /// published resource answers on; the TPT binding answers `None` nowhere,
    /// because no capture of that marketplace has ever shown a product
    /// without a downloadable file and inventing the answer would be this
    /// device stating a fact TPT did not.
    fn bundle(&self, resource: i64) -> SourceFuture<'_, Option<Vec<u8>>>;
}

/// So a command that picked its source at run time can hold one.
///
/// Two marketplaces are two bindings with two concrete types, and the pass is
/// generic over the seam rather than over the marketplace; boxing is what lets
/// one call site choose between them without the pass learning which
/// marketplaces exist.
impl CatalogueSource for Box<dyn CatalogueSource> {
    fn list<'a>(&'a self, found: &'a CatalogueProgress) -> SourceFuture<'a, Vec<ListedResource>> {
        (**self).list(found)
    }

    fn read(&self, resource: i64) -> SourceFuture<'_, ImportedListing> {
        (**self).read(resource)
    }

    fn bundle(&self, resource: i64) -> SourceFuture<'_, Option<Vec<u8>>> {
        (**self).bundle(resource)
    }
}

/// What one pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PassReport {
    pub described: usize,
    pub pages_posted: usize,
    pub skipped: Vec<SkippedResource>,
}

/// Progress, as the screen reads it while the pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct ImportProgress {
    /// How many resources the catalogue holds. Known after the enumeration and
    /// zero before it, which the screen shows as "reading your catalogue"
    /// rather than as a total of nothing.
    pub total: usize,
    pub described: usize,
    pub skipped: usize,
}

/// Anything that stopped the pass rather than one resource.
///
/// Every arm maps to one [`ImportReasonCode`], because the run's own page is
/// where the seller reads this and a sentence the console had to parse would
/// be a second vocabulary to keep in step. The mapping is
/// [`PassError::reason_code`] and is total by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassError {
    /// The catalogue could not be read.
    ///
    /// Never rendered as "no listings". An empty answer and a failed read are
    /// indistinguishable from here, and the live run of 2026-08-28 saw both
    /// dashboard routes answer an empty array with HTTP 200 on a session that
    /// was authenticated, so telling a seller their shop is empty on this
    /// evidence would be stating something we do not know.
    Catalogue(String),
    /// A page could not be posted, so the work it described is not recorded.
    Page(String),
    /// The server read one of this run's pages and will not keep it.
    ///
    /// Its own arm rather than a [`Self::Page`] with a different sentence,
    /// because the two end the run differently: an outage lifts and the page
    /// this device still holds is delivered by the next connection, while a
    /// body the server has read and refused is the same body however many
    /// times it is offered. Reported to the seller with the same code — the
    /// remedy is ours either way — and settled rather than left open, which
    /// is what stops the run being offered back to this device so it can
    /// walk the seller's shop and rebuild the identical page.
    RejectedPage(String),
    /// The seller signed this device out while the import was running.
    Revoked,
    /// The entitlement for the marketplace being read does not stand.
    NotEntitled(Marketplace),
    /// Nobody has signed in to that marketplace on this device.
    ///
    /// The refusal the incident of 2026-09-12 could produce and could not
    /// report: it is decided before a single marketplace request is composed,
    /// it is the one thing only this device knows, and the seller's remedy is
    /// to connect this device or to use the one that is connected.
    NoSession(Marketplace),
    /// This device cannot read that kind of shop at all.
    ///
    /// A marketplace with an official API is read on our own servers under a
    /// sanctioned token, so no device will ever claim such a run: left open it
    /// would wait for a device that is never coming.
    UnsupportedSource(String),
    /// Everything the run named failed to be described.
    ///
    /// Distinct from a run that finished with some skips, which completes and
    /// names them. A run where nothing at all could be read is a failure, and
    /// reporting it as a completion with a full page of skips is how a seller
    /// comes to believe an import worked.
    Descriptions(String),
    /// The seller stopped this run, and this device put it down.
    Stopped,
    /// Another attempt owns this run now, or the lease lapsed while this
    /// device was busy. Whatever this attempt still held is refused.
    FenceLost,
    /// The run's own source could not be read from the control plane.
    ///
    /// Its own arm because it happens after the claim and before anything
    /// else: the device owns the run, and the one thing it needs to know
    /// about it did not arrive. Before this was reported the seller's only
    /// account of it was a rejected promise in a window they had left.
    Source(String),
    /// This device could not record what it was about to do.
    ///
    /// Reported rather than swallowed, because the whole point of the
    /// journal is that a page or a failure survives the process: carrying on
    /// after a write that did not happen would be claiming a durability this
    /// device does not have.
    Journal(String),
}

impl PassError {
    /// The class the console renders and the seller acts on.
    #[must_use]
    pub const fn reason_code(&self) -> ImportReasonCode {
        match self {
            Self::Catalogue(_) => ImportReasonCode::EnumerationFailed,
            // A page and both control-plane/local-durability failures all mean
            // that this attempt could not submit what it owed. The seller's
            // remedy is the same, and inventing narrower protocol codes would
            // make distinctions the server cannot act on.
            Self::Page(_) | Self::RejectedPage(_) | Self::Source(_) | Self::Journal(_) => {
                ImportReasonCode::SubmissionFailed
            }
            // A revocation and a lapsed grant are one class to the server —
            // this device may not work — and two sentences to the seller,
            // which is what `Display` below is for.
            Self::Revoked | Self::NotEntitled(_) => ImportReasonCode::NotPermitted,
            Self::NoSession(_) => ImportReasonCode::MissingSession,
            Self::UnsupportedSource(_) => ImportReasonCode::UnsupportedSource,
            Self::Descriptions(_) => ImportReasonCode::DescriptionFailed,
            Self::Stopped => ImportReasonCode::Stopped,
            Self::FenceLost => ImportReasonCode::LeaseExpired,
        }
    }

    /// The stage a run reaches by this ending.
    ///
    /// The line is whether the run can still be finished. A stop is an
    /// interruption because the seller asked for it and the checkpoint
    /// stands; a lost fence because another attempt has the run; and an
    /// exchange with our own control plane that did not complete — a page
    /// that could not be posted, a source that could not be read, a journal
    /// that could not be written — because none of those says anything about
    /// the seller's shop. Calling an outage a failure is how a replayable run
    /// became unreplayable: the server settles it, the page this device still
    /// holds can never be delivered, and the seller is told their import
    /// failed when nothing about it did.
    ///
    /// What answers `Failed` is what will not come right by itself: no
    /// session on this device, a source no device can read, a lapsed
    /// entitlement, a signed-out device, a catalogue the marketplace refused,
    /// a selection where nothing could be read, and a page the server has
    /// read and refused. That last one is the distinction this pair of arms
    /// is for: an outage is a page the server has not seen, and a rejection
    /// is an answer about the bytes, so leaving it open would offer the run
    /// back to a device whose only move is to rebuild the identical page.
    #[must_use]
    pub const fn stage(&self) -> ImportStage {
        match self {
            Self::Stopped
            | Self::FenceLost
            | Self::Page(_)
            | Self::Source(_)
            | Self::Journal(_) => ImportStage::Interrupted,
            Self::Revoked
            | Self::NotEntitled(_)
            | Self::NoSession(_)
            | Self::UnsupportedSource(_)
            | Self::Catalogue(_)
            | Self::RejectedPage(_)
            | Self::Descriptions(_) => ImportStage::Failed,
        }
    }
}

impl core::fmt::Display for PassError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Catalogue(why) => {
                write!(f, "your catalogue could not be read: {why}")
            }
            Self::Page(why) => write!(f, "a page of the import could not be recorded: {why}"),
            Self::RejectedPage(why) => write!(
                f,
                "your import could not be recorded: the server would not keep a page of it: \
                 {why}"
            ),
            Self::Revoked => f.write_str("this device was signed out while the import was running"),
            Self::NotEntitled(marketplace) => write!(
                f,
                "this device's entitlement for {marketplace:?} does not stand, so it stopped \
                 reading part way through"
            ),
            Self::NoSession(marketplace) => write!(
                f,
                "this device is not signed in to {marketplace:?}, so it cannot read your shop. \
                 Connect it on this device and start the import again"
            ),
            Self::UnsupportedSource(why) => f.write_str(why),
            Self::Descriptions(why) => write!(
                f,
                "nothing in your selection could be read: {why}. Nothing was added to your \
                 catalogue"
            ),
            Self::Source(why) => write!(
                f,
                "this device could not read which shop this import names: {why}"
            ),
            Self::Journal(why) => write!(
                f,
                "this device could not record what it was about to do, so it stopped rather \
                 than risk losing it: {why}"
            ),
            Self::Stopped => f.write_str("you stopped this import on this device"),
            Self::FenceLost => f.write_str(
                "another device took this import over, or its lease lapsed while this device \
                 was reading, so this device stopped",
            ),
        }
    }
}

impl core::error::Error for PassError {}

/// Why this device may not go on working a run, if it may not.
///
/// Three facts rather than one flag, because the three are answered
/// differently and the seller reads them differently: a revocation is the
/// seller signing this machine out of the console, a cancellation is the
/// seller stopping this one import, and a lost fence is another attempt
/// holding the run. The revocation half is shared with the whole process —
/// it is [`crate::state::DesktopState::stopper`] — while the other two are
/// this run's own, which is what makes stopping one run leave the other
/// source alone.
#[derive(Debug, Clone)]
pub struct StopSignal {
    revoked: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    fenced_out: Arc<AtomicBool>,
    /// The work is over, however it ended.
    ///
    /// Not a stop cause: nothing is refused because of it and no run is
    /// reported as interrupted by it. It exists so the renewal task ends when
    /// the work does — a run that completed left its keeper renewing a lease
    /// for work that had finished, which holds a fence nobody needs and
    /// keeps a task alive for the life of the process.
    done: Arc<AtomicBool>,
}

/// Why a run stopped, where it stopped for one of these reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    Revoked,
    Cancelled,
    FenceLost,
}

impl StopSignal {
    /// One run's signal, sharing the process-wide revocation flag.
    #[must_use]
    pub fn over(revoked: Arc<AtomicBool>) -> Self {
        Self {
            revoked,
            cancelled: Arc::new(AtomicBool::new(false)),
            fenced_out: Arc::new(AtomicBool::new(false)),
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A signal for a pass with no supervisor above it, which is what the
    /// tests of the pass itself drive.
    #[must_use]
    pub fn never() -> Self {
        Self::over(Arc::new(AtomicBool::new(false)))
    }

    /// Why this run must stop, or `None` while it may go on.
    ///
    /// Revocation is read first because it is the widest: a signed-out device
    /// stops everything, and reporting it as a cancellation would tell the
    /// seller they stopped something they did not.
    #[must_use]
    pub fn why(&self) -> Option<StopCause> {
        if self.revoked.load(Ordering::SeqCst) {
            Some(StopCause::Revoked)
        } else if self.cancelled.load(Ordering::SeqCst) {
            Some(StopCause::Cancelled)
        } else if self.fenced_out.load(Ordering::SeqCst) {
            Some(StopCause::FenceLost)
        } else {
            None
        }
    }

    /// The seller stopped this run.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// The server refused this attempt's fence, so nothing it still holds may
    /// be written.
    pub fn lose_fence(&self) {
        self.fenced_out.store(true, Ordering::SeqCst);
    }

    /// Whether this run was stopped on this device, as opposed to stopped for
    /// any other reason.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// The work has ended, so anything running beside it may end too.
    pub fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
    }

    /// Whether the work has ended.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
}

impl StopCause {
    /// The ending this cause is reported as.
    #[must_use]
    pub const fn ending(self) -> PassError {
        match self {
            Self::Revoked => PassError::Revoked,
            Self::Cancelled => PassError::Stopped,
            Self::FenceLost => PassError::FenceLost,
        }
    }
}

/// The content type of the bytes handed onward.
///
/// From the bytes rather than from the file's name, because a name is what the
/// marketplace called it and the magic is what the bytes are.
///
/// The image arm takes the bytes as well as the kind, and that is why this is
/// not a `const fn` over the kind alone. `FileKind::Image` covers PNG, JPEG
/// and GIF, so answering `image/png` for all three describes a JPEG as a PNG —
/// and this value is not merely recorded, it is signed into the target
/// marketplace's string-to-sign, so the description would be wrong inside a
/// signature rather than only in a column.
#[must_use]
pub fn content_type_for(kind: FileKind, bytes: &[u8]) -> &'static str {
    match kind {
        FileKind::Pdf => "application/pdf",
        FileKind::Pptx => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        FileKind::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        FileKind::Zip => "application/zip",
        FileKind::Image => image_content_type(bytes),
    }
}

/// Which image, from its own leading bytes.
///
/// The same three the pipeline's probe recognises, in the same order. Anything
/// else cannot reach here, because the probe would not have called it an
/// image, so the last arm is a fall-through rather than a default and it
/// answers the one type that says "bytes" rather than guessing a picture
/// format.
#[must_use]
fn image_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(PNG_MAGIC) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

/// The bundle reduced to what will actually be uploaded.
///
/// The rule is the one the whole migration turns on: the target's product slot
/// takes one file, so a bundle of several travels whole — which is also what
/// the source's own buyers receive — and a bundle of one becomes that one
/// file, because a buyer expects the worksheet rather than a zip wrapping the
/// worksheet.
fn payload_of(bundle: Vec<u8>, fallback_name: &str) -> (Vec<u8>, String, Option<String>) {
    match tam_pipeline::archive::sole_entry(
        &bundle,
        tam_pipeline::archive::ExtractBudget::default(),
    ) {
        Some(only) => {
            let name = only
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&only.path)
                .to_owned();
            (only.bytes, name, Some(only.path))
        }
        None => (bundle, fallback_name.to_owned(), None),
    }
}

/// The receipt one page is offered under.
///
/// Minted once, when the page is first queued, and kept with the page for
/// every replay afterwards. Deliberately random rather than derived: a
/// receipt computed from the page's position in a walk restarts at zero on
/// whichever device claims the run next, so two devices' first pages collide
/// on one receipt carrying different content — which the server reads as a
/// conflict on work that was never a replay. A minted receipt travels with
/// the payload that earned it and belongs to nothing else.
///
/// Re-enumerated rows are not this function's problem, and that is the
/// division: the server applies a page per locator, so a resource read twice
/// under two receipts is applied once.
#[must_use]
pub fn minted_receipt() -> tam_types::Uuid {
    tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes())
}

/// One run's progress as this device will remember it across a restart.
///
/// What is here is what a resumed attempt needs and nothing else: which half
/// it was doing, what the counts were, how many pages of that half have been
/// acknowledged, and which resources the server already has. The acknowledged
/// list is what stops a resumed pass re-fetching a shop it has already
/// described — bounded by the selection, and locators rather than resources,
/// so it is a list of short strings and not a copy of the catalogue.
///
/// `pages_posted` is remembered rather than derived. Deriving it from the
/// acknowledged count assumes every page held exactly [`PAGE_SIZE`]
/// resources, which the time-based partial flush makes false: a shop of six
/// posted as two pages of three would resume at ordinal zero and re-offer a
/// receipt the server has already answered for different content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunCheckpoint {
    pub run: tam_types::Uuid,
    pub source: tam_types::InventoryId,
    pub attempt: u64,
    pub phase: RunPhase,
    #[serde(default)]
    pub discovered: u32,
    #[serde(default)]
    pub processed: u32,
    #[serde(default)]
    pub described: u32,
    #[serde(default)]
    pub enumeration_complete: bool,
    #[serde(default)]
    pub pages_posted: u32,
    #[serde(default)]
    pub acknowledged: Vec<String>,
}

impl RunCheckpoint {
    /// What a resumed attempt continues from.
    #[must_use]
    pub fn progress(&self) -> RunProgress {
        RunProgress {
            discovered: self.discovered,
            processed: self.processed,
            described: self.described,
            enumeration_complete: self.enumeration_complete,
            pages_posted: self.pages_posted,
            acknowledged: self.acknowledged.clone(),
        }
    }
}

/// One post this device owes the server and has not had an answer to.
///
/// The body verbatim rather than the facts to rebuild it from, and that is
/// the difference between a replay the server accepts and one it refuses: a
/// re-described resource would carry a freshly rendered cover, so the same
/// receipt would arrive with different content, which the protocol calls a
/// conflict rather than a replay.
///
/// `id` is this device's own name for the entry and is not on the wire. The
/// body cannot serve as one: an ordinary progress report names no run, so two
/// sources reporting equal counts would produce identical bodies and
/// acknowledging one would erase the other.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingPost {
    pub id: tam_types::Uuid,
    pub run: tam_types::Uuid,
    pub path: String,
    pub body: String,
    /// The checkpoint this post's delivery earns, where it earns one.
    ///
    /// Recorded with the post and applied in the same change that retires it,
    /// which is the only way the two can agree. Written separately, a page
    /// that was queued and then delivered by a later attempt left the
    /// checkpoint where the first attempt had it: the page count had not
    /// moved, so the next description reused the receipt the server had just
    /// answered for different content, and the counts lost the page as well.
    ///
    /// Absent on a progress report, which earns no checkpoint: it says where
    /// the run has got to and changes nothing about what is still owed.
    #[serde(default)]
    pub earns: Option<RunCheckpoint>,
    /// What this post is, which decides what answer counts as an answer.
    ///
    /// Carried rather than inferred from the path: a reader of the path has
    /// to know every route to know what it is looking at, and the thing that
    /// matters here is what proves delivery — a page's acknowledgement, an
    /// abandonment's, or nothing at all for a progress line.
    #[serde(default)]
    pub kind: PostKind,
}

impl PendingPost {
    /// One owed post, under a name of this device's own minting.
    #[must_use]
    pub fn new(run: tam_types::Uuid, path: &str, body: &str) -> Self {
        Self {
            id: tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes()),
            run,
            path: path.to_owned(),
            body: body.to_owned(),
            earns: None,
            kind: PostKind::Report,
        }
    }

    /// The same post, as one of the kinds whose delivery has to be proved.
    #[must_use]
    pub fn of_kind(self, kind: PostKind) -> Self {
        Self { kind, ..self }
    }

    /// The same post, carrying the checkpoint its delivery earns.
    #[must_use]
    pub fn earning(self, checkpoint: RunCheckpoint) -> Self {
        Self {
            earns: Some(checkpoint),
            ..self
        }
    }

    /// Whether this post's delivery settles a local stop.
    #[must_use]
    pub const fn settles_stop(&self) -> bool {
        matches!(self.kind, PostKind::Stop)
    }

    /// Whether this post is the instruction that closes the run's record.
    ///
    /// A stop and a terminal ending both are, and both are therefore offered
    /// while the server still lists the run as open: waiting for the row to
    /// close would wait forever, because this post is what closes it. A page
    /// and a progress line are not — they are work and commentary on a run
    /// that is still going.
    #[must_use]
    pub const fn settles_run(&self) -> bool {
        self.kind.settles_run()
    }

    /// The same post under a later fence.
    ///
    /// The attempt is the one field a re-offer may change, and the protocol
    /// says so: it is excluded from a receipt's content identity precisely so
    /// that a page queued under a lapsed lease can be delivered under the new
    /// one. Everything else — the receipt, the resources, the covers — is
    /// left exactly as it was, because changing any of it would turn a replay
    /// into a conflict.
    ///
    /// A body that carries no attempt is left alone: anything this device
    /// cannot parse is offered as it stands rather than rewritten on a guess.
    #[must_use]
    pub fn under(&self, attempt: u64) -> Self {
        let Ok(mut body) = serde_json::from_str::<serde_json::Value>(&self.body) else {
            return self.clone();
        };
        let Some(fields) = body.as_object_mut() else {
            return self.clone();
        };
        if !fields.contains_key("attempt") {
            return self.clone();
        }
        fields.insert("attempt".to_owned(), serde_json::json!(attempt));
        serde_json::to_string(&body).map_or_else(
            |_| self.clone(),
            |body| Self {
                body,
                ..self.clone()
            },
        )
    }
}

/// Which kind of post this device owes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostKind {
    /// A progress line. Any success is an answer: the route returns no body
    /// this device reads, and a lost progress line costs nothing.
    #[default]
    Report,
    /// One page of the catalogue. Proved by the route's own acknowledgement.
    Page,
    /// The abandonment this device owes the server. Proved by the server
    /// saying the run is abandoned, and by nothing weaker.
    Stop,
    /// The reason and the stage this run ended on.
    ///
    /// A progress line by route and by proof, and its own kind for one
    /// reason: it is the post that closes the run's record, so the drain
    /// offers it while the run is still listed open rather than waiting for
    /// a row that this post is what closes.
    Ending,
}

impl PostKind {
    /// Whether the body the server answered proves this post was applied.
    ///
    /// A status code cannot: a gateway, a captive portal and a changed route
    /// all answer two-hundred. So each kind names the answer it needs, and a
    /// post whose answer does not arrive stays owed.
    #[must_use]
    pub fn proved_by(self, body: &str) -> bool {
        match self {
            Self::Report | Self::Ending => true,
            Self::Page => serde_json::from_str::<PageAck>(body).is_ok(),
            // The route's own acknowledgement and nothing weaker. An empty
            // body, an empty object, a refusal object and a gateway's page
            // are all things a two-hundred can carry, and settling a
            // seller's stop on any of them would have this device believe
            // the server accepted something it never saw.
            Self::Stop => abandoned(body),
        }
    }

    /// Whether a post of this kind settles a local stop when it is answered.
    #[must_use]
    pub const fn settles_stop(self) -> bool {
        matches!(self, Self::Stop)
    }

    /// Whether a post of this kind is the one that closes the run's record.
    #[must_use]
    pub const fn settles_run(self) -> bool {
        matches!(self, Self::Stop | Self::Ending)
    }
}

/// One run's consecutive unaccepted page offers.
///
/// A count rather than a flag, so the bound is explicit and one bad
/// connection does not pause a run the next offer would have finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunFailures {
    pub run: tam_types::Uuid,
    #[serde(default)]
    pub offers: u32,
}

/// Everything this device remembers about imports across a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportJournalState {
    #[serde(default)]
    pub runs: Vec<RunCheckpoint>,
    #[serde(default, deserialize_with = "restore_outbox")]
    pub outbox: Vec<PendingPost>,
    /// Runs the seller stopped on this device whose abandonment the server
    /// has not been seen to accept.
    ///
    /// Durable, because a volatile flag is what let a stop be undone by a
    /// restart: the handle went with the process, the run was still open on
    /// the server, and the next discovery cycle claimed it again and carried
    /// on reading the shop the seller had stopped.
    #[serde(default)]
    pub stopped: Vec<tam_types::Uuid>,
    /// Runs whose pages the server keeps failing, and how many consecutive
    /// offers of one have gone unaccepted.
    ///
    /// Durable for the same reason the stop marks are: the loop this bounds
    /// spans attempts and processes, so a count that lived in a task would be
    /// reset by the very reclaim it exists to stop. Kept apart from
    /// [`Self::stopped`], because the two mean different things and are
    /// cleared by different events — a seller's stop is settled by the
    /// server accepting the abandonment, and a pause by the seller pressing
    /// again, by a page being accepted, or by the run leaving the open list.
    #[serde(default)]
    pub failing: Vec<RunFailures>,
}

// Earlier clients stored terminal failures as ordinary reports. Normalize
// their kind when restoring the journal so delivery cannot restart the run.
fn restore_outbox<'de, D>(deserializer: D) -> Result<Vec<PendingPost>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    struct ReportStage {
        stage: ImportStage,
    }

    let mut posts = <Vec<PendingPost> as serde::Deserialize>::deserialize(deserializer)?;
    for post in &mut posts {
        if post.kind == PostKind::Report
            && serde_json::from_str::<ReportStage>(&post.body)
                .is_ok_and(|report| report.stage == ImportStage::Failed)
        {
            post.kind = PostKind::Ending;
        }
    }
    Ok(posts)
}

impl ImportJournalState {
    /// This run's checkpoint, where one was kept.
    #[must_use]
    pub fn checkpoint(&self, run: tam_types::Uuid) -> Option<&RunCheckpoint> {
        self.runs.iter().find(|kept| kept.run == run)
    }

    /// Whether the seller stopped this run on this device and the server has
    /// not been seen to agree yet.
    #[must_use]
    pub fn was_stopped(&self, run: tam_types::Uuid) -> bool {
        self.stopped.contains(&run)
    }

    /// Whether this device has stopped offering that run's page by itself.
    ///
    /// Read where a discovery cycle would otherwise claim the run again. It
    /// says nothing about the run being over: the page is still queued, the
    /// reason is on the run, and a press resumes it.
    #[must_use]
    pub fn paused(&self, run: tam_types::Uuid) -> bool {
        self.failures(run) >= OFFERS_BEFORE_PAUSE
    }

    /// How many consecutive offers of this run's page the server has failed.
    #[must_use]
    pub fn failures(&self, run: tam_types::Uuid) -> u32 {
        self.failing
            .iter()
            .find(|failing| failing.run == run)
            .map_or(0, |failing| failing.offers)
    }

    /// What this device still owes the server, oldest first.
    #[must_use]
    pub fn owed(&self, run: tam_types::Uuid) -> Vec<PendingPost> {
        self.outbox
            .iter()
            .filter(|owed| owed.run == run)
            .cloned()
            .collect()
    }
}

/// One change to the journal.
///
/// A closed set rather than a closure, for two reasons that pull the same
/// way: the trait stays object-safe, and every read-modify-write happens
/// inside the journal under one lock. Two runs and a renewal task write this
/// file, and before this each of them read the whole state, changed its own
/// part and wrote the whole state back — so the last writer erased whatever
/// the others had recorded in between, which is exactly the page and the
/// failure a restart needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalChange {
    /// Records where one run has got to.
    Keep(RunCheckpoint),
    /// Adds one post this device owes the server.
    Enqueue(PendingPost),
    /// The server answered this post, so it is no longer owed and the
    /// checkpoint it earned stands.
    Delivered(tam_types::Uuid),
    /// This post will never be applied — a refused sign-in, a superseded
    /// fence — so it is dropped without crediting anything it described.
    Abandoned(tam_types::Uuid),
    /// One more offer of this run's page went unaccepted, and the page is
    /// still owed.
    ///
    /// Counted rather than acted on here: [`ImportJournalState::paused`] is
    /// what a discovery cycle reads, so the bound lives in one place and the
    /// count survives the attempt that recorded it.
    Failed(tam_types::Uuid),
    /// The server read this run's page and refused it, so this device stops
    /// offering pages for the run by itself.
    ///
    /// The same mark [`Self::Failed`] reaches after [`OFFERS_BEFORE_PAUSE`]
    /// offers, reached in one: a refusal of the bytes is not a connection
    /// that might come back, so a second and a third offer would buy two
    /// more walks of the seller's shop for an answer this device has already
    /// had. Counting it instead would never bound anything, because a run
    /// whose earlier page the server kept has its streak cleared by that
    /// acceptance and starts again from one every cycle.
    ///
    /// Cleared by the same three events a streak is — an accepted page, an
    /// explicit press, or the run leaving the open list — so recovery, a
    /// resume and a takeover are unchanged.
    Refused(tam_types::Uuid),
    /// Re-fences everything this run still owes to a later attempt.
    Refence { run: tam_types::Uuid, attempt: u64 },
    /// The seller stopped this run on this device.
    Stopped(tam_types::Uuid),
    /// The seller has explicitly asked for this run again, or the server has
    /// settled it: clear the stop and the outage pause, retire any queued stop
    /// for the superseded attempt, and preserve every other post the run still
    /// owes.
    ///
    /// The pause goes with the press because that is the seller answering the
    /// question the pause asks. It does not go with an accepted stop: a
    /// tombstone settled by the server retires the whole run through
    /// [`Self::Forget`], and nothing about a stop being acknowledged says the
    /// server has started keeping pages again.
    Resumed(tam_types::Uuid),
    /// Everything about this run is the server's record from here.
    Forget(tam_types::Uuid),
}

/// Moves a run's checkpoint forward, never back.
///
/// The greatest of each count and the union of the acknowledged locators,
/// because the entry being applied may be an older attempt's: a page queued
/// under a lapsed lease and delivered under the new one earns exactly what it
/// said it would, and must not undo anything the new attempt has since done.
/// The attempt itself is the live one's rather than the entry's, for the same
/// reason.
fn advance(state: &mut ImportJournalState, earned: RunCheckpoint) {
    let merged = match state.checkpoint(earned.run) {
        Some(kept) => RunCheckpoint {
            attempt: kept.attempt.max(earned.attempt),
            discovered: kept.discovered.max(earned.discovered),
            processed: kept.processed.max(earned.processed),
            described: kept.described.max(earned.described),
            enumeration_complete: kept.enumeration_complete || earned.enumeration_complete,
            pages_posted: kept.pages_posted.max(earned.pages_posted),
            acknowledged: union_of(&kept.acknowledged, &earned.acknowledged),
            ..earned
        },
        None => earned,
    };
    state.runs.retain(|kept| kept.run != merged.run);
    state.runs.push(merged);
}

/// Both lists, in order, without repeats.
fn union_of(kept: &[String], earned: &[String]) -> Vec<String> {
    let mut all = kept.to_vec();
    for locator in earned {
        if !all.contains(locator) {
            all.push(locator.clone());
        }
    }
    all
}

/// Applies one change. Shared by both journals so the two cannot disagree
/// about what a change means.
fn apply(state: &mut ImportJournalState, change: &JournalChange) {
    match change {
        JournalChange::Keep(checkpoint) => {
            state.runs.retain(|kept| kept.run != checkpoint.run);
            state.runs.push(checkpoint.clone());
        }
        JournalChange::Enqueue(post) => state.outbox.push(post.clone()),
        JournalChange::Delivered(id) => {
            let answered = state.outbox.iter().find(|owed| owed.id == *id).cloned();
            let Some(answered) = answered else {
                return;
            };
            if answered.settles_stop() {
                // The server has the abandonment, so there is nothing left to
                // hold for this run: the stop mark has done its job and
                // whatever else was queued for it is moot.
                apply(state, &JournalChange::Forget(answered.run));
                return;
            }
            // The checkpoint the post earned and the post's retirement, in
            // one change. Anything less and a page delivered by a later
            // attempt would be gone from the queue with the run's page count
            // still behind it, so the next description would reuse a receipt
            // the server had already answered.
            let run = answered.run;
            let was_page = matches!(answered.kind, PostKind::Page);
            if let Some(earned) = answered.earns {
                advance(state, earned);
            }
            // A page the server kept is the progress the pause was waiting
            // for, so the streak starts again from nothing. Only a page: a
            // progress line is answered by every gateway and captive portal
            // there is, and reading one as recovery is what would leave a run
            // reclaiming itself through an outage that never lifted.
            if was_page {
                state.failing.retain(|failing| failing.run != run);
            }
            state.outbox.retain(|owed| owed.id != *id);
        }
        // Dropped and credited with nothing, a stop included. A post the
        // server never applied says nothing about the run, so an
        // abandonment refused for a reason this device does not recognise
        // leaves the local stop standing: the seller stopped the run, and
        // until the server is known to agree, no discovery cycle here picks
        // it up again.
        JournalChange::Abandoned(id) => state.outbox.retain(|owed| owed.id != *id),
        // Retained and pushed rather than edited in place, which is the same
        // shape `advance` uses for a checkpoint: one entry per run, and the
        // count read before the entry is replaced.
        JournalChange::Failed(run) => {
            let offers = state.failures(*run).saturating_add(1);
            state.failing.retain(|failing| failing.run != *run);
            state.failing.push(RunFailures { run: *run, offers });
        }
        // The bound reached in one step. Written as the same mark rather than
        // as a second flag, so `paused` stays the one question a discovery
        // cycle asks and a press clears both by clearing one.
        JournalChange::Refused(run) => {
            state.failing.retain(|failing| failing.run != *run);
            state.failing.push(RunFailures {
                run: *run,
                offers: OFFERS_BEFORE_PAUSE,
            });
        }
        JournalChange::Refence { run, attempt } => {
            for owed in &mut state.outbox {
                if owed.run == *run {
                    *owed = owed.under(*attempt);
                }
            }
        }
        JournalChange::Stopped(run) => {
            if !state.stopped.contains(run) {
                state.stopped.push(*run);
            }
        }
        JournalChange::Resumed(run) => {
            state.stopped.retain(|kept| kept != run);
            state.failing.retain(|failing| failing.run != *run);
            state
                .outbox
                .retain(|owed| owed.run != *run || !owed.settles_stop());
        }
        JournalChange::Forget(run) => {
            state.runs.retain(|kept| kept.run != *run);
            state.outbox.retain(|owed| owed.run != *run);
            state.stopped.retain(|kept| kept != run);
            state.failing.retain(|failing| failing.run != *run);
        }
    }
}

/// One read or change of this device's import journal.
pub type JournalFuture<'a, T> =
    core::pin::Pin<Box<dyn core::future::Future<Output = Result<T, String>> + Send + 'a>>;

/// Where a checkpoint and an unacknowledged page survive a restart.
///
/// A seam over the one file rather than the file itself, for the reason every
/// seam in this module exists: a supervisor driven against a directory could
/// not be driven deterministically, and the property the tests need is what
/// the journal held rather than how it was spelled on disk.
///
/// Both operations may fail and both say so. A journal that could not be
/// written is not a detail to log: the page or the failure it was recording
/// is the thing a restart would have replayed, so the caller refuses rather
/// than carrying on believing it is safe.
pub trait ImportJournal: Send + Sync {
    fn read(&self) -> JournalFuture<'_, ImportJournalState>;

    fn mutate<'a>(&'a self, change: &'a JournalChange) -> JournalFuture<'a, ()>;
}

/// The file the journal lives in, inside the application data directory.
///
/// Beside `device.json` and following the same pattern, which is the design's
/// instruction: the existing device persistence rather than a database this
/// slice would have to introduce.
pub const JOURNAL_FILE: &str = "imports.json";

/// The journal on this device's own disk.
///
/// The lock is what serialises the two runs and the renewal task against each
/// other. It is held across the read and the write of one change and across
/// nothing else: no marketplace request and no server call happens under it.
pub struct FileJournal {
    path: std::path::PathBuf,
    writing: Mutex<()>,
}

impl FileJournal {
    #[must_use]
    pub fn in_data_dir(data_dir: &std::path::Path) -> Self {
        Self {
            path: data_dir.join(JOURNAL_FILE),
            writing: Mutex::new(()),
        }
    }

    /// The journal as it stands, or why it could not be read.
    ///
    /// A file that is not there is an empty journal, because a device that
    /// has never imported has nothing recorded. Everything else is reported:
    /// an unreadable or unparseable journal means this device cannot tell
    /// whether it owes the server a page, and answering "nothing" to that
    /// question is how an unacknowledged page and an undelivered failure get
    /// lost for good.
    async fn load(&self) -> Result<ImportJournalState, String> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|why| format!("the import journal did not parse: {why}")),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {
                Ok(ImportJournalState::default())
            }
            Err(why) => Err(format!("the import journal could not be read: {why}")),
        }
    }

    /// Replaces the file rather than overwriting it in place.
    ///
    /// A partial write is the one failure this file cannot survive: it is
    /// read once at start-up and a truncated one is indistinguishable from a
    /// corrupt one. Writing a sibling and renaming it is atomic on every
    /// platform this client ships to, so the file is always either the state
    /// before the change or the state after it.
    async fn replace(&self, state: &ImportJournalState) -> Result<(), String> {
        let encoded = serde_json::to_vec(state)
            .map_err(|why| format!("the import journal could not be encoded: {why}"))?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|why| format!("the import journal's directory is unusable: {why}"))?;
        }
        let sibling = self.path.with_extension("json.writing");
        // Written, flushed, and only then renamed. A write that is merely
        // buffered survives this process and not the machine: a phone whose
        // battery goes while a page is unacknowledged would come back to a
        // journal that never reached the disk, which is the one failure this
        // file exists to survive.
        let mut writing = tokio::fs::File::create(&sibling)
            .await
            .map_err(|why| format!("the import journal could not be opened: {why}"))?;
        tokio::io::AsyncWriteExt::write_all(&mut writing, &encoded)
            .await
            .map_err(|why| format!("the import journal could not be written: {why}"))?;
        writing
            .sync_all()
            .await
            .map_err(|why| format!("the import journal could not be flushed: {why}"))?;
        drop(writing);
        tokio::fs::rename(&sibling, &self.path)
            .await
            .map_err(|why| format!("the import journal could not be replaced: {why}"))?;
        self.flush_directory().await
    }

    /// Flushes the directory entry the rename created.
    ///
    /// Reported rather than shrugged off: a change this device cannot vouch
    /// for is exactly what its caller refuses on, so a failed flush fails the
    /// change. The guarantee is stated per platform rather than claimed
    /// everywhere and held nowhere — on Unix the entry is synced or the write
    /// fails, and on Windows there is no directory handle to flush, so the
    /// promise is the platform's own atomic replace and this says so instead
    /// of pretending to more.
    #[cfg(unix)]
    async fn flush_directory(&self) -> Result<(), String> {
        let Some(parent) = self.path.parent() else {
            return Ok(());
        };
        tokio::fs::File::open(parent)
            .await
            .map_err(|why| format!("the import journal's directory could not be opened: {why}"))?
            .sync_all()
            .await
            .map_err(|why| format!("the import journal's directory could not be flushed: {why}"))
    }

    /// Windows has no directory handle to flush; `rename` is its own atomic
    /// replace. Named rather than absent so the weaker guarantee is a thing a
    /// reader finds here.
    #[cfg(not(unix))]
    #[expect(
        clippy::unused_async,
        reason = "one signature on every platform; the Unix half awaits two syscalls"
    )]
    async fn flush_directory(&self) -> Result<(), String> {
        Ok(())
    }
}

impl ImportJournal for FileJournal {
    fn read(&self) -> JournalFuture<'_, ImportJournalState> {
        Box::pin(async move { self.load().await })
    }

    fn mutate<'a>(&'a self, change: &'a JournalChange) -> JournalFuture<'a, ()> {
        Box::pin(async move {
            let writing = self.writing.lock().await;
            let mut state = self.load().await?;
            apply(&mut state, change);
            let written = self.replace(&state).await;
            drop(writing);
            written
        })
    }
}

/// A journal that forgets when the process does.
///
/// The default for a state built without one, and what the tests drive. Not a
/// silent fallback in the application: [`crate::run`]'s start-up hands the
/// file-backed one in, and a build that did not would lose its checkpoints at
/// a restart rather than lose them silently mid-run.
#[derive(Debug, Default)]
pub struct MemoryJournal {
    state: Mutex<ImportJournalState>,
}

impl MemoryJournal {
    /// A journal holding a state a test put there, which is what a restart
    /// looks like from the code's point of view.
    #[must_use]
    pub fn holding(state: ImportJournalState) -> Self {
        Self {
            state: Mutex::new(state),
        }
    }
}

impl ImportJournal for MemoryJournal {
    fn read(&self) -> JournalFuture<'_, ImportJournalState> {
        Box::pin(async move { Ok(self.state.lock().await.clone()) })
    }

    fn mutate<'a>(&'a self, change: &'a JournalChange) -> JournalFuture<'a, ()> {
        Box::pin(async move {
            apply(&mut *self.state.lock().await, change);
            Ok(())
        })
    }
}

/// One run's half of the conversation with the control plane.
///
/// Everything a pass sends goes through this rather than through the
/// transport directly, and that is what makes four properties true at once:
/// nothing is posted after the run has been stopped or fenced out, every page
/// is written where a restart can find it before it is offered and cleared
/// only once the server's answer has been read, what this device still owes
/// is delivered in order and before anything newer, and a page whose
/// acknowledgement was lost is re-offered under the current fence without its
/// receipt or its content changing.
pub struct RunLedger {
    device: crate::device::DeviceId,
    run: tam_types::Uuid,
    attempt: u64,
    phase: RunPhase,
    source: tam_types::InventoryId,
    plane: Arc<dyn LedgerTransport>,
    journal: Arc<dyn ImportJournal>,
    stop: StopSignal,
    /// What a previous attempt already got through, which this one continues
    /// from rather than counting again.
    baseline: RunProgress,
    last_report: Mutex<Option<tokio::time::Instant>>,
}

impl RunLedger {
    #[must_use]
    pub fn new(
        device: crate::device::DeviceId,
        claimed: &ClaimedRun,
        plane: Arc<dyn LedgerTransport>,
        journal: Arc<dyn ImportJournal>,
    ) -> Self {
        Self {
            device,
            run: claimed.run,
            attempt: claimed.attempt,
            phase: claimed.phase,
            source: claimed.source,
            plane,
            journal,
            stop: claimed.stop.clone(),
            baseline: claimed.progress.clone(),
            last_report: Mutex::new(None),
        }
    }

    #[must_use]
    pub const fn run(&self) -> tam_types::Uuid {
        self.run
    }

    #[must_use]
    pub const fn attempt(&self) -> u64 {
        self.attempt
    }

    #[must_use]
    pub fn stop(&self) -> StopSignal {
        self.stop.clone()
    }

    /// What a previous attempt already got through.
    #[must_use]
    pub fn baseline(&self) -> RunProgress {
        self.baseline.clone()
    }

    /// Why this run may not post another thing, if it may not.
    #[must_use]
    pub fn halted(&self) -> Option<PassError> {
        self.stop.why().map(StopCause::ending)
    }

    /// Posts one page, having first written it where a restart can find it and
    /// delivered anything older that is still owed.
    ///
    /// The order is the guarantee: drain, journal, offer, read the answer,
    /// clear. A process that dies before the answer is read re-offers the
    /// identical page under the identical receipt, which the server answers
    /// with the acknowledgement it already gave rather than with a second
    /// page's worth of counts.
    pub async fn post_page(
        &self,
        mut page: ImportPage,
        earns: &RunProgress,
    ) -> Result<(), PassError> {
        if let Some(halted) = self.halted() {
            return Err(halted);
        }
        page.attempt = Some(self.attempt);
        // Minted here, once, and then kept: the journalled body carries it,
        // so every replay and every re-fence offers the same receipt for the
        // same payload without this device having to reproduce it from
        // anything.
        page.receipt = Some(minted_receipt());
        let body = serde_json::to_string(&page)
            .map_err(|why| PassError::Page(format!("the page could not be encoded: {why}")))?;
        // The checkpoint this page's delivery earns travels with it, so
        // whichever attempt finally delivers it advances the run in the same
        // change that retires it.
        let owed = PendingPost::new(self.run, &import_path(&self.device), &body)
            .of_kind(PostKind::Page)
            .earning(self.checkpoint_of(earns));
        // Written down before anything is offered, and before whatever this
        // device already owed is retried: an outage that stopped the queue
        // must not stop the page being recorded, or the one copy a restart
        // could replay would never exist. Refused rather than sent if that
        // write fails, because offering it then would be claiming a
        // durability this build does not have.
        self.journal
            .mutate(&JournalChange::Enqueue(owed.clone()))
            .await
            .map_err(PassError::Journal)?;
        // One queue, delivered in order, this page last. An answer for this
        // page is therefore an answer for everything before it, and a failure
        // anywhere in the queue leaves this page owed — which is the honest
        // outcome, because the server has not seen it.
        self.flush_outbox().await
    }

    /// Retires one owed post.
    ///
    /// A failure here is logged rather than returned, and that asymmetry is
    /// deliberate: the post has already been answered, and a journal that
    /// still lists it only causes the identical bytes to be offered again
    /// under the identical receipt, which the protocol defines as a replay.
    /// Failing the page instead would report a failure for work that landed.
    async fn delivered(&self, owed: &PendingPost) {
        self.retire(&JournalChange::Delivered(owed.id)).await;
    }

    /// Drops one owed post without crediting it.
    ///
    /// A refused sign-in and a lost fence both mean the server did not apply
    /// the page, so retiring it as delivered would record resources as
    /// acknowledged that were never written — and a resumed run would then
    /// skip exactly the resources nobody has.
    async fn abandoned(&self, owed: &PendingPost) {
        self.retire(&JournalChange::Abandoned(owed.id)).await;
    }

    /// Counts one offer of this run's page that the server did not accept.
    ///
    /// Pages only. A progress line is answered by every gateway and captive
    /// portal there is, and one that failed costs nothing, so counting it
    /// would pause runs for the wrong reason.
    ///
    /// Logged rather than surfaced, like the retirements above: the caller is
    /// already returning the submission failure, and a count that could not
    /// be written costs one more reclaim rather than any work.
    async fn unaccepted(&self, owed: &PendingPost) {
        if !matches!(owed.kind, PostKind::Page) {
            return;
        }
        if let Err(why) = self.journal.mutate(&JournalChange::Failed(self.run)).await {
            eprintln!("a failed import page could not be counted on this device: {why}");
        }
    }

    /// Records that the server has read this run's page and refused it, so
    /// this device offers no further page for the run by itself.
    ///
    /// Logged rather than surfaced for the same reason the count above is:
    /// the caller is already returning the failure, and a mark that could
    /// not be written costs one more reclaim rather than any work.
    async fn refused(&self) {
        if let Err(why) = self.journal.mutate(&JournalChange::Refused(self.run)).await {
            eprintln!("a refused import page could not be recorded on this device: {why}");
        }
    }

    async fn retire(&self, change: &JournalChange) {
        if let Err(why) = self.journal.mutate(change).await {
            eprintln!("an answered import post could not be retired locally: {why}");
        }
    }

    /// This run's checkpoint as it will stand at the given progress.
    fn checkpoint_of(&self, progress: &RunProgress) -> RunCheckpoint {
        RunCheckpoint {
            run: self.run,
            source: self.source,
            attempt: self.attempt,
            phase: self.phase,
            discovered: progress.discovered,
            processed: progress.processed,
            described: progress.described,
            enumeration_complete: progress.enumeration_complete,
            pages_posted: progress.pages_posted,
            acknowledged: progress.acknowledged.clone(),
        }
    }

    /// Reports where this run has got to, at most once every
    /// [`PROGRESS_EVERY`] unless the report is one the console's next action
    /// depends on.
    pub async fn report(
        &self,
        report: ImportProgressReport,
        always: bool,
    ) -> Result<(), PassError> {
        if !always && !self.due().await {
            return Ok(());
        }
        let body =
            serde_json::to_string(&report).map_err(|why| PassError::Page(why.to_string()))?;
        // A terminal report is kept apart from a progress line by kind, not
        // by path: both go to the one route, and the difference that matters
        // to a queue is that this one is the post which closes the run's
        // record. A drain that waited for the row to close before offering
        // it would wait on itself.
        let kind = if matches!(report.stage, ImportStage::Failed) {
            PostKind::Ending
        } else {
            PostKind::Report
        };
        let owed =
            PendingPost::new(self.run, &progress_path(&self.device, self.run), &body).of_kind(kind);
        self.journal
            .mutate(&JournalChange::Enqueue(owed.clone()))
            .await
            .map_err(PassError::Journal)?;
        // Settled with the same classes a page is, minus the acknowledgement
        // check: the progress route answers no body this device reads, so a
        // two-hundred is the whole of its answer.
        match self.plane.post(&owed.path, owed.body.clone()).await {
            Ok(_) => {
                self.delivered(&owed).await;
                Ok(())
            }
            // A refused sign-in, a machine the seller signed out, and a body
            // the server read and will not take. One arm because the answer
            // is the same for all three: none was applied, none will be by
            // repeating it, so the report is dropped rather than left at the
            // head of the outbox where every post behind it would wait on a
            // refusal that never lifts.
            Err(why @ (ControlPlaneError::Denied(_) | ControlPlaneError::Rejected(_))) => {
                self.abandoned(&owed).await;
                Err(PassError::Page(why.to_string()))
            }
            Err(ControlPlaneError::Revoked) => {
                self.stop.lose_fence();
                self.abandoned(&owed).await;
                Err(PassError::Page(ControlPlaneError::Revoked.to_string()))
            }
            Err(ControlPlaneError::Fenced(_)) => {
                self.stop.lose_fence();
                self.abandoned(&owed).await;
                Err(PassError::FenceLost)
            }
            // Kept, not lost: the next connection delivers it, which is the
            // whole reason a report is journalled before it is offered.
            Err(why) => Err(PassError::Page(why.to_string())),
        }
    }

    /// The stage, counts and reason this run ended on.
    ///
    /// Always delivered rather than throttled, and always attempted even when
    /// the run is already stopped: this is the one report whose absence is the
    /// defect the whole repair exists to fix. A delivery that could not happen
    /// leaves it in the outbox, which is what a later connection drains.
    pub async fn report_ending(&self, why: &PassError, progress: &RunProgress) {
        let ending = ImportProgressReport {
            attempt: self.attempt,
            stage: why.stage(),
            discovered: progress.discovered,
            processed: progress.processed,
            reason_code: Some(why.reason_code()),
            reason: Some(Reason::truncating(&why.to_string())),
        };
        if let Err(unreported) = self.report(ending, true).await {
            // Not surfaced further: the caller is already reporting a
            // failure, and replacing its reason with a second one about
            // reporting it would tell the seller less. It is kept in the
            // journal, which is what makes this survivable rather than lost.
            eprintln!("an import ending is still owed to the server: {unreported}");
        }
    }

    async fn due(&self) -> bool {
        let mut last = self.last_report.lock().await;
        let now = tokio::time::Instant::now();
        if last.is_some_and(|then| now.duration_since(then) < PROGRESS_EVERY) {
            return false;
        }
        *last = Some(now);
        true
    }

    /// What this device still owes the server for this run.
    pub async fn owed(&self) -> Result<Vec<PendingPost>, PassError> {
        self.journal
            .read()
            .await
            .map(|state| state.owed(self.run))
            .map_err(PassError::Journal)
    }

    /// Offers everything this device still owes the server for this run,
    /// under this attempt's fence.
    ///
    /// The re-fence is what makes a queued page deliverable at all: it was
    /// built under a lease that has since lapsed, and the server refuses a
    /// stale attempt. The receipt and the content are untouched, which is
    /// exactly the identity the protocol dedupes on.
    pub async fn flush_outbox(&self) -> Result<(), PassError> {
        for post in self.owed().await? {
            let post = post.under(self.attempt);
            let sent = self.plane.post(&post.path, post.body.clone()).await;
            match sent {
                // A two-hundred carrying something else — a gateway's HTML, a
                // truncated body, an empty object — proves nothing. Retiring
                // a page on it would clear the one copy this device could
                // replay and advance the checkpoint past work the server may
                // never have recorded; retiring an abandonment on it would
                // leave this device believing the server had accepted a stop
                // it never saw.
                Ok(body) if !post.kind.proved_by(&body) => {
                    self.unaccepted(&post).await;
                    return Err(PassError::Page(
                        "the server's answer was not one this device could read as an \
                         acknowledgement, so what it sent is still owed"
                            .to_owned(),
                    ));
                }
                // The post that closes the run's record has landed, so this
                // attempt holds a run the server has settled: it may do no
                // further work on it, and a walk from here would post pages
                // against a closed run.
                Ok(_) if matches!(post.kind, PostKind::Ending) => {
                    self.delivered(&post).await;
                    self.stop.lose_fence();
                    return Err(PassError::FenceLost);
                }
                Ok(_) => self.delivered(&post).await,
                // Refused sign-in, or this machine signed out: not owed
                // again, and not credited either. The server applied nothing,
                // so recording its resources as acknowledged would make a
                // resumed run skip exactly what nobody has, and leaving the
                // post owed would stall every post behind it on a refusal
                // that repeating cannot lift. The shop is re-read by the run
                // that comes after the seller signs the machine back in, which
                // is the only copy that was ever authoritative.
                Err(why @ ControlPlaneError::Denied(_)) => {
                    self.abandoned(&post).await;
                    return Err(PassError::Page(why.to_string()));
                }
                Err(ControlPlaneError::Revoked) => {
                    // And the walk ends here rather than at the next page: a
                    // signed-out device may make no further marketplace
                    // request for this run.
                    self.stop.lose_fence();
                    self.abandoned(&post).await;
                    return Err(PassError::Page(ControlPlaneError::Revoked.to_string()));
                }
                Err(ControlPlaneError::Fenced(why)) => {
                    // The fence rather than the transport: this attempt has
                    // been superseded, so nothing it holds is owed again and
                    // the walk must end rather than retry.
                    self.stop.lose_fence();
                    self.settle_fenced(&post, &why).await;
                    return Err(PassError::FenceLost);
                }
                // The server read the body and will not take it. Kept out of
                // the outage arm below deliberately: an outage lifts and the
                // post is owed until it does, while this answer is a fact
                // about the bytes — so the one copy is discarded here rather
                // than offered under every future fence, and the failure is
                // reported where the seller reads it.
                //
                // A page is also the end of the run rather than an
                // interruption of it, and the disposition is recorded before
                // the failure is returned. Counting the offer instead is what
                // bounded nothing: a run whose earlier page the server kept
                // has its streak cleared by that acceptance, so the next
                // cycle reclaimed the run, re-walked the seller's shop and
                // rebuilt the identical page for the identical answer. The
                // mark says this device offers no further page for the run by
                // itself, and it stands until the server accepts one, the
                // seller presses, or the run leaves the open list.
                //
                // Anything else this device owes — a progress line, an
                // abandonment — is dropped and reported as before: neither is
                // the run's work, and neither says the run is over.
                Err(why @ ControlPlaneError::Rejected(_)) => {
                    self.abandoned(&post).await;
                    if !matches!(post.kind, PostKind::Page) {
                        return Err(PassError::Page(why.to_string()));
                    }
                    self.refused().await;
                    return Err(PassError::RejectedPage(why.to_string()));
                }
                Err(why) => {
                    self.unaccepted(&post).await;
                    return Err(PassError::Page(why.to_string()));
                }
            }
        }
        Ok(())
    }

    /// Retires a post the server fenced, and decides what that does to a
    /// local stop.
    ///
    /// A conflict is proof about the run only when it names one. A stop
    /// refused because the attempt it spoke for is superseded, or because
    /// another owner holds the run, is a stop that is spent, and the
    /// tombstone has nothing left to protect. Any other conflict — an
    /// unfamiliar code, an unparseable body — proves nothing: the post is
    /// dropped, because it will never be accepted, and the tombstone stays,
    /// so this device still refuses to pick the run up by itself.
    async fn settle_fenced(&self, post: &PendingPost, why: &str) {
        if post.settles_stop() && stop_is_spent(why) {
            self.delivered(post).await;
            return;
        }
        self.abandoned(post).await;
    }

    /// Writes this run's checkpoint, so a restart resumes rather than
    /// re-reads.
    ///
    /// Surfaced rather than logged: a checkpoint this device could not write
    /// is one a restart would not see, so the caller stops instead of walking
    /// a shop it will walk again.
    pub async fn checkpoint(&self, progress: &RunProgress) -> Result<(), PassError> {
        self.journal
            .mutate(&JournalChange::Keep(self.checkpoint_of(progress)))
            .await
            .map_err(PassError::Journal)
    }

    /// Drops everything remembered about this run, which is what a settled
    /// ending means: the server's record is the one that stands from here.
    ///
    /// Only ever called once nothing is owed. A run whose last page or whose
    /// ending is still queued keeps its journal entry, because clearing it is
    /// how the page and the reason a restart needed came to be lost.
    pub async fn settled(&self) {
        if let Err(why) = self.journal.mutate(&JournalChange::Forget(self.run)).await {
            eprintln!("a settled import could not be cleared locally: {why}");
        }
    }
}

/// The part of the page route's acknowledgement this device reads.
///
/// Four of `ImportAck`'s fields, all required, and nothing else: unknown
/// fields are ignored so a server that adds one does not strand a shipped
/// device, while a body that carries none of these is not an acknowledgement
/// at all. That distinction is the whole point — "valid JSON" is not
/// acceptance, and `{}` or a refusal object is exactly what a gateway, a
/// captive portal or a changed route answers with a two-hundred.
///
/// `applied` is also the field that tells a first delivery from a replay, so
/// requiring it is requiring the one number this device's own counts depend
/// on rather than an arbitrary token of well-formedness.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[expect(
    dead_code,
    reason = "each field is required on the way in, which is the whole check: a body missing \
              any of them is not this route's acknowledgement. The values themselves are the \
              server's business, and this device reads only whether it was answered"
)]
struct PageAck {
    applied: u32,
    skipped: u32,
    described_total: u32,
    complete: bool,
}

/// The one conflict code that settles a stop.
///
/// A settled run is the answer to a second abandonment: the run is no longer
/// open, so there is nothing left for this device to stop and nothing left
/// for the tombstone to protect.
///
/// Deliberately not the fence code. A fenced abandonment means another
/// attempt or another owner holds the run — the run is still open — and
/// forgetting the seller's stop then would let the next discovery cycle here
/// pick it up as though they had never stopped it.
pub const STOP_SETTLED_CODE: &str = "import_run_settled";

/// The codes the server refused with, read from the field rather than from
/// the sentence.
///
/// The message wording is not stable and is not ours; the code is its own
/// snake_case field on each error entry. A body this device cannot parse
/// yields nothing, which is the safe answer everywhere this is used: an
/// unrecognised refusal settles nothing.
#[must_use]
pub fn refusal_codes(body: &str) -> Vec<String> {
    let Ok(answer) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    answer
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("code"))
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the server's conflict says the run is already settled.
fn stop_is_spent(body: &str) -> bool {
    refusal_codes(body)
        .iter()
        .any(|code| code == STOP_SETTLED_CODE)
}

/// The device stop route's acknowledgement.
///
/// `abandoned` is required and must be true. A replay meets exactly this
/// answer rather than a conflict, which is what makes the queued stop safe to
/// offer as often as it takes.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct StopAck {
    abandoned: bool,
}

/// Whether the device stop route said the run is abandoned.
fn abandoned(body: &str) -> bool {
    serde_json::from_str::<StopAck>(body).is_ok_and(|answered| answered.abandoned)
}

/// Whether a page's answer is the route's own acknowledgement.
///
/// Decoding is the assertion. Every field of [`PageAck`] is required, so
/// `{}`, a refusal object, a bare string and a gateway's HTML all fail here,
/// where a test for well-formed JSON passed three of the four. Kept as a name
/// of its own because the page is the kind whose proof the tests pin
/// directly; [`PostKind::proved_by`] is what the delivery path asks.
#[cfg(test)]
fn acknowledged(body: &str) -> bool {
    PostKind::Page.proved_by(body)
}

/// What one half of one run has got through.
///
/// Counts the console renders and the checkpoint keeps, in one value so the
/// two cannot disagree. `discovered` and `processed` are the protocol's own
/// names rather than the screen's, because the server is what decides what
/// the screen shows.
///
/// A resumed attempt starts from the checkpoint's rather than from zero. The
/// server keeps the greatest count it has seen, so a resumed walk that
/// counted its own five resources would report five against a recorded
/// twenty-five and the run would appear to have gone backwards — or, worse,
/// to have stalled at twenty-five while work was plainly happening.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunProgress {
    pub discovered: u32,
    pub processed: u32,
    /// What this run has described over its whole life, this attempt and
    /// every earlier one. The question "has anything at all come across" is
    /// asked of this and never of `processed`, which counts failures too.
    pub described: u32,
    pub enumeration_complete: bool,
    pub pages_posted: u32,
    pub acknowledged: Vec<String>,
}

impl RunProgress {
    /// The report the server reads this as, at the given stage.
    #[must_use]
    pub const fn at(&self, attempt: u64, stage: ImportStage) -> ImportProgressReport {
        ImportProgressReport {
            attempt,
            stage,
            discovered: self.discovered,
            processed: self.processed,
            reason_code: None,
            reason: None,
        }
    }
}

/// One import of one seller's catalogue.
///
/// Generic over the catalogue and over nothing else. Everything to do with
/// the run — which attempt holds it, where its pages go, what survives a
/// restart — is [`RunLedger`]'s, so this type is the marketplace half and
/// only that: it reads a shop, measures what it finds, and hands pages over.
pub struct ImportPass<S: CatalogueSource> {
    source: S,
    ledger: Arc<RunLedger>,
    permission: SourcePermission,
    /// Where an import's originals are kept on this machine, where the
    /// build has one.
    library: Option<Arc<crate::library::LibrarySlot>>,
}

/// Whether this pass may go on reading the marketplace it is reading.
///
/// The facts together rather than as parameters, which is also what keeps
/// them travelling as one: enumerating a catalogue and fetching a bundle are
/// marketplace requests like any other, so Q-g's rule applies to them — the
/// grant for that marketplace must stand, and the kill switch must be able to
/// stop the pass between resources rather than only between imports. Held
/// rather than read once at the start, because an import of five hundred
/// resources runs long enough for a revocation to arrive during it.
///
/// The shop is named as an inventory rather than as a marketplace, because
/// that is the run's own fact and the marketplace is derived from it; two
/// fields would be two places for them to disagree.
pub struct SourcePermission {
    pub source: tam_types::InventoryId,
    pub gate: Arc<Mutex<EntitlementGate>>,
    pub stop: StopSignal,
}

impl SourcePermission {
    #[must_use]
    pub fn marketplace(&self) -> Marketplace {
        self.source.marketplace()
    }
}

impl<S: CatalogueSource> ImportPass<S> {
    #[must_use]
    pub const fn new(source: S, ledger: Arc<RunLedger>, permission: SourcePermission) -> Self {
        Self {
            source,
            ledger,
            permission,
            library: None,
        }
    }

    /// Attaches the machine's library, so each described original is kept
    /// where the seller's setting says to. A builder rather than a fourth
    /// argument, so the tests that build a pass without one read as before.
    ///
    /// The slot rather than an opened library: a pass claimed while the
    /// library could not be opened keeps the originals it describes after it
    /// can be, which on a phone is as soon as the seller unlocks it.
    #[must_use]
    pub fn keeping(mut self, library: Option<Arc<crate::library::LibrarySlot>>) -> Self {
        self.library = library;
        self
    }

    #[must_use]
    pub fn ledger(&self) -> Arc<RunLedger> {
        Arc::clone(&self.ledger)
    }

    /// Why this pass may not make another marketplace request, if it may not.
    ///
    /// Consulted before each one rather than once at the start. The answers
    /// are different things the seller acts on differently: a revocation is
    /// the seller signing this machine out, a stop is the seller stopping this
    /// one import, a lost fence is another attempt holding it, and a lapsed
    /// grant is the entitlement the subscription carries.
    async fn refusal(&self, now: Timestamp) -> Option<PassError> {
        if let Some(cause) = self.permission.stop.why() {
            return Some(cause.ending());
        }
        let marketplace = self.permission.marketplace();
        if !self.permission.gate.lock().await.may_work(marketplace, now) {
            return Some(PassError::NotEntitled(marketplace));
        }
        None
    }

    /// Reads the whole catalogue, posts the listing, then describes every
    /// resource in it.
    ///
    /// The two-step flow the seller actually drives is `enumerate` +
    /// `post_listing`, then `describe_all` over what they ticked; this is
    /// that flow with "all of it" as the selection, and it is what the tests
    /// drive because the selection is the console's decision rather than the
    /// pass's.
    ///
    /// `now` is passed in rather than read, because this crate holds no clock:
    /// the scan instant it records is the caller's reading, exactly as every
    /// other instant on this device is.
    pub async fn run(
        &self,
        now: Timestamp,
        progress: impl FnMut(ImportProgress) + Send,
    ) -> Result<PassReport, PassError> {
        let catalogue = self.enumerate(now).await?;
        self.post_listing(catalogue.clone()).await?;
        let selection = catalogue.iter().filter_map(resource_id).collect();
        self.describe_all(selection, || now, progress).await
    }

    /// Reads the seller's catalogue and stops.
    ///
    /// Separate from the rest so a caller can do it while the seller is still
    /// looking. Everything that can fail before a single page is posted fails
    /// here — the entitlement, the revocation, and the catalogue read itself —
    /// and each of those is a refusal the seller can act on. A caller that ran
    /// the whole pass in the background would answer "started" and then have
    /// nowhere to put the failure, because the run's own view holds nothing
    /// until a page lands.
    pub async fn enumerate(&self, now: Timestamp) -> Result<Vec<ListedResource>, PassError> {
        // Before the enumeration, which is itself a marketplace request.
        if let Some(refusal) = self.refusal(now).await {
            return Err(refusal);
        }
        // Bounded, because a walk that cannot reach its end must be refused
        // by name rather than left hanging: the page walk is several requests
        // over a large shop and each has its own transport timeout, but
        // nothing bounded the whole of it, so a marketplace answering slowly
        // rather than not at all could hold a run open indefinitely.
        //
        // Raced against the run's own handle as well as against the clock,
        // and that is the second bound: the adapter reads several pages
        // behind one future, so a stop, a revocation or a takeover arriving
        // during the walk had no effect until the whole of it finished or the
        // five minutes ran out. The seller's stop is answered between
        // marketplace requests at worst, rather than after all of them.
        let walk = tokio::time::timeout(ENUMERATION_BUDGET, self.listing());
        match walk.await {
            Ok(listed) => listed,
            Err(_) => Err(PassError::Catalogue(
                "reading your catalogue took longer than five minutes, so it was stopped; \
                 nothing was imported and starting again is safe"
                    .to_owned(),
            )),
        }
    }

    /// The adapter's walk, against the run's handle and a heartbeat.
    ///
    /// The heartbeat is activity rather than a count: the adapter answers the
    /// whole shop at once, so until it does there is nothing found to report
    /// and reporting a number would be inventing one. What the run needs
    /// meanwhile is to be seen to be discovering rather than to look stalled,
    /// which is what a `discovering` report with the counts it actually has
    /// says.
    ///
    /// A shop reported row by row needs the adapter to offer its pages as it
    /// reads them; `CatalogueSource` hands over a whole vector, so that half
    /// belongs to the marketplace adapters and is called out as owed rather
    /// than faked here.
    async fn listing(&self) -> Result<Vec<ListedResource>, PassError> {
        // The walk records and this loop reports. One number, stored by the
        // walk's own task and read by this one, is the whole of the coupling:
        // a chatty marketplace cannot turn a discovery into a flood of
        // control-plane calls, and the reporting stays where the fence and
        // the outbox already are.
        let found = Arc::new(core::sync::atomic::AtomicU32::new(0));
        let counting = Arc::clone(&found);
        let sink = move |rows: u32| {
            counting.store(rows, Ordering::SeqCst);
        };
        let mut reading = core::pin::pin!(self.source.list(&sink));
        loop {
            if let Ok(listed) = tokio::time::timeout(PROGRESS_EVERY, &mut reading).await {
                return listed.map_err(|why| PassError::Catalogue(why.to_string()));
            }
            if let Some(refusal) = self.refusal(crate::run::wall_now()).await {
                return Err(refusal);
            }
            let mut progress = self.ledger.baseline();
            // What the walk has actually seen, never a percentage:
            // neither source states a trustworthy closed total before
            // the end, and `enumeration_complete` is what closes
            // discovery rather than a count reaching one.
            progress.discovered = progress.discovered.max(found.load(Ordering::SeqCst));
            self.ledger
                .report(
                    progress.at(self.ledger.attempt(), ImportStage::Discovering),
                    false,
                )
                .await
                .ok();
        }
    }

    /// Posts the shop as the enumeration saw it, and nothing else.
    ///
    /// The page the selection step renders. It is not complete and carries no
    /// resource: the run holds items in `listed`, the seller ticks, and the
    /// second command describes only what was ticked. Posting this before the
    /// seller chooses is what makes the choice possible at all — the console
    /// reads the run rather than holding a list the device sent it directly,
    /// so closing the window between the two steps loses nothing.
    pub async fn post_listing(&self, listed: Vec<ListedResource>) -> Result<(), PassError> {
        let baseline = self.ledger.baseline();
        let mut progress = baseline.clone();
        // Counted for this walk and then taken as the greater of the two,
        // rather than added to what the run already recorded. A re-enumerated
        // shop would otherwise be counted twice: a resumed discovery reads
        // the same rows again, and adding them to the previous total reports
        // twice the shop the seller has.
        let mut walked: u32 = 0;
        // Posted in pages rather than as one body, because a shop of several
        // hundred rows is a large request from a phone and because the
        // protocol now says what closes discovery: a listing page is not the
        // last one until `enumeration_complete` says so, so the seller sees
        // rows appearing instead of nothing for the length of the walk.
        let pages = listed.chunks(PAGE_SIZE).len().max(1);
        for (index, chunk) in listed
            .chunks(PAGE_SIZE)
            .chain(empty_shop(&listed))
            .enumerate()
        {
            let last = index + 1 == pages;
            walked = walked.saturating_add(u32::try_from(chunk.len()).unwrap_or(u32::MAX));
            progress.discovered = baseline.discovered.max(walked);
            progress.enumeration_complete = last;
            progress.pages_posted = progress.pages_posted.saturating_add(1);
            self.ledger
                .post_page(
                    ImportPage {
                        run: self.ledger.run(),
                        attempt: None,
                        receipt: None,
                        request: None,
                        listed: Some(chunk.to_vec()),
                        resources: Vec::new(),
                        skipped: Vec::new(),
                        enumeration_complete: last,
                        complete: false,
                        failed: None,
                    },
                    &progress,
                )
                .await?;
            self.ledger
                .report(
                    progress.at(self.ledger.attempt(), ImportStage::Discovering),
                    last,
                )
                .await
                .ok();
        }
        Ok(())
    }

    /// Describes an already-enumerated catalogue, posting pages as it goes.
    ///
    /// `clock` rather than an instant, because the between-resource refusal is
    /// the whole reason that check exists and a shop of several hundred
    /// resources takes long enough for the answer to change. An entitlement
    /// whose grace period expires mid-pass has to be read against the current
    /// instant; against the instant the pass started, the grace never runs out
    /// and the run continues on a subscription that has lapsed. The crate still
    /// holds no clock — this is the caller's reading, taken repeatedly rather
    /// than once.
    ///
    /// `progress` is the local callback the screen of this device reads;
    /// every step is also reported to the run, which is what the console
    /// reads. Both, because they answer different questions: one is what this
    /// window is doing and the other is what the run has got through.
    pub async fn describe_all(
        &self,
        catalogue: Vec<i64>,
        clock: impl Fn() -> Timestamp + Send,
        mut progress: impl FnMut(ImportProgress) + Send,
    ) -> Result<PassReport, PassError> {
        let mut walk = Walk::resuming(catalogue.len(), self.ledger.baseline());
        progress(walk.local());
        for (index, locator) in catalogue.iter().copied().enumerate() {
            // Between resources rather than once at the start: an import of a
            // large shop runs long enough for a revocation or a stop to
            // arrive during it, and the next fetch after one must not happen.
            // What has already been described is posted first, so the work is
            // not lost.
            if let Some(refusal) = self.refusal(clock()).await {
                self.flush(&mut walk, false).await?;
                return Err(refusal);
            }
            walk.took(self.describe(locator, clock()).await, locator);
            let last = index + 1 == catalogue.len();
            // The last page completes the run only if something was read. A
            // selection where every resource failed must not arrive as a
            // completion carrying nothing but skips: completion is what mints
            // the write jobs, and the seller would read it as success.
            let completes = last && !walk.nothing_described();
            if last || walk.page_is_due() {
                self.flush(&mut walk, completes).await?;
            }
            progress(walk.local());
            self.ledger
                .report(
                    walk.progress
                        .at(self.ledger.attempt(), ImportStage::Reading),
                    last,
                )
                .await
                .ok();
        }

        // A catalogue that turned out to hold nothing still completes, or the
        // server would never mint the jobs and the seller would watch an
        // import that never ends.
        if catalogue.is_empty() {
            self.flush(&mut walk, true).await?;
        }
        // A selection where every single resource failed is a failed run
        // rather than a completed one with a full page of skips. The
        // difference is what the seller reads: "nothing came across" is a
        // thing to act on, and a completion with nothing in it is a thing to
        // misread as success.
        if walk.nothing_described() {
            return Err(PassError::Descriptions(
                walk.report.skipped.len().to_string() + " of them could not be read",
            ));
        }
        Ok(walk.report)
    }

    /// Posts what the walk is holding, and records that the server has it.
    async fn flush(&self, walk: &mut Walk, complete: bool) -> Result<(), PassError> {
        if walk.nothing_pending() && !complete {
            return Ok(());
        }
        let described = walk.take_described();
        let acknowledged: Vec<String> = described
            .iter()
            .map(|resource| resource.locator.as_str().to_owned())
            .collect();
        // The checkpoint this page will have earned, worked out before it is
        // offered and recorded with it: whichever attempt finally delivers
        // the page advances the run in the same change that retires it, so a
        // page delivered by a later attempt cannot leave the run's counts
        // behind the work the server holds.
        let earns = walk.once_posted(&acknowledged);
        self.ledger
            .post_page(
                ImportPage {
                    run: self.ledger.run(),
                    attempt: None,
                    receipt: None,
                    request: None,
                    listed: None,
                    resources: described,
                    skipped: walk.take_skipped(),
                    enumeration_complete: false,
                    complete,
                    failed: None,
                },
                &earns,
            )
            .await?;
        walk.posted(acknowledged);
        Ok(())
    }

    /// One resource, read and measured, with its payload bytes dropped before
    /// this returns.
    ///
    /// Everything that touches the seller's bytes happens inside this
    /// function, which is what makes "nothing accumulates" checkable by
    /// reading rather than by trusting: the bundle and the payload are locals,
    /// and the value handed back describes them without carrying them.
    async fn describe(&self, locator: i64, now: Timestamp) -> Result<ObservedResource, String> {
        let listing = self
            .source
            .read(locator)
            .await
            .map_err(|why| format!("its listing could not be read: {why}"))?;
        // Every resource the seller ticked has its file asked for, whatever
        // the listing's state says. The state is the source's snapshot, and
        // on Tes that snapshot is the `/{id}/draft` overlay — a published
        // resource with an unpublished edit answers it too, so refusing on
        // it drops resources whose bundle was there to be had. The founder's
        // 2026-09-16 import selected four and imported none on exactly that
        // reading.
        //
        // What separates a resource with no file from one whose file could
        // not be fetched is therefore the source's own answer and not a
        // state: `Ok(None)` is an absence the marketplace confirmed, and an
        // `Err` is a fetch that failed and stays a named skip.
        let Some(bundle) = self
            .source
            .bundle(locator)
            .await
            .map_err(|why| format!("its file could not be fetched: {why}"))?
        else {
            // Metadata-only rather than skipped: a listing with no file is
            // still a catalogue entry the seller can use, and its title is
            // still evidence the matcher reads. Backed by the wire already —
            // every file-shaped field of `ObservedResource` is optional.
            return Ok(ObservedResource {
                locator: Locator::from_resource_id(locator),
                fingerprint: Some(Fingerprint::of_title(&listing.title)),
                listing,
                file: None,
                cover_png: None,
            });
        };

        let (payload, name, entry) = payload_of(bundle, &format!("{locator}-bundle.zip"));
        let kind = tam_pipeline::probe::probe_kind(&payload)
            .ok_or_else(|| "its file is of a kind this device does not recognise".to_owned())?;

        // The scan is the device's own and is recorded as the device's. Q-b
        // decided it is acceptable and advisory precisely because we never see
        // these bytes, so nothing here restates it as a verdict of ours.
        let scan =
            tam_pipeline::scan::Scanner::scan(&tam_pipeline::scan::EicarScanner, &payload, now)
                .await;
        if let ScanOutcome::Infected { signature } = &scan {
            return Err(format!(
                "its file did not pass a scan on this device: {signature}"
            ));
        }

        // The cover, from the resource's own bytes: the seller's preview
        // picture inside the bundle, or the document's stored thumbnail,
        // where either exists, and the kind's generated card where neither
        // does. Whichever it is, it is derived here, on this device, from
        // bytes this pass already holds -- no second marketplace request is
        // made for a picture, and the provenance travels no further than this
        // function because the wire carries the cover and not a claim about
        // it.
        let drawn = tam_pipeline::render::cover(kind, &payload)
            .map_err(|why| format!("no cover could be made from its file: {why}"))?;
        let cover = drawn.image;

        // Measured here, where the bytes are, and nowhere else. The sketch is
        // fixed-width and cannot be read back into the document, which is what
        // lets it cross a wire the payload may not: see `tam-fingerprint`.
        let fingerprint =
            tam_fingerprint::fingerprint(kind, &payload, &listing.title, Some(&cover.png));

        let hash = blake3::hash(&payload);
        let observed = ObservedResource {
            locator: Locator::from_resource_id(locator),
            listing,
            file: Some(ObservedFile {
                payload_file_name: FileName::new(&name)
                    .map_err(|why| format!("its file name is not one we will report: {why}"))?,
                payload_content_type: ContentType::new(content_type_for(kind, &payload))
                    .map_err(|why| format!("its media type is not one we will report: {why}"))?,
                kind,
                hash: ContentHash(*hash.as_bytes()),
                byte_len: payload.len() as u64,
                scan,
                entry: entry
                    .map(|path| FileName::new(&path))
                    .transpose()
                    .map_err(|why| format!("its entry name is not one we will report: {why}"))?,
            }),
            cover_png: Some(
                Cover::encode(&cover.png)
                    .map_err(|why| format!("the cover made from its file is not one: {why}"))?,
            ),
            fingerprint: Some(fingerprint),
        };
        // The seller's bytes end here as far as the wire is concerned. Every
        // field of the value above is bounded or fixed-width — a digest, a
        // validated name, a number, a closed set, a checked cover — so there
        // is nowhere in it for a payload to be. That claim is checked rather
        // than asserted: see `the_page_is_the_same_size_whatever_the_payload_weighs`.
        //
        // What does keep the bytes is this machine's own library, sealed
        // under this machine's own key, and only where the seller's setting
        // says so. A failure to keep is logged and does not fail the import:
        // the resource is described either way, and the library is a
        // convenience for the seller rather than a step the read depends on.
        //
        // The library is asked for here, resource by resource, rather than
        // held open from the start of the run. A phone whose keyguard was
        // showing when the application launched could not open it then and
        // can open it now, and an import the seller started after unlocking
        // their phone has to keep what it reads.
        if let Some(slot) = &self.library {
            let refused = match slot.get().await {
                Ok(library) => library
                    .keep_original(
                        crate::library::LibraryEntry {
                            hash: ContentHash(*hash.as_bytes()),
                            file_name: name.clone(),
                            content_type: content_type_for(kind, &payload).to_owned(),
                            byte_len: payload.len() as u64,
                            marketplace: self.permission.marketplace(),
                            resource: locator.to_string(),
                            kept_at: now,
                            pinned: false,
                        },
                        &payload,
                    )
                    .await
                    .err(),
                // A library that will not open is reported the same way a
                // keep that failed is: the seller's import goes on either way.
                Err(why) => Some(why),
            };
            if let Some(why) = refused {
                eprintln!("the original of resource {locator} was not kept on this machine: {why}");
            }
        }
        Ok(observed)
    }
}

/// One walk through a selection: what has been described, what was skipped,
/// and what the run's counts are.
///
/// Its own value rather than four locals, because the page and the counts and
/// the checkpoint all have to agree and a partial flush is where they would
/// otherwise drift: what is posted, what is acknowledged and what is reported
/// are three views of this one state.
struct Walk {
    total: usize,
    report: PassReport,
    progress: RunProgress,
    page: Vec<ObservedResource>,
    skipped: Vec<SkippedResource>,
    opened: tokio::time::Instant,
}

impl Walk {
    /// A walk continuing from what a previous attempt got through.
    ///
    /// The baseline is not cosmetic. The server keeps the greatest count it
    /// has seen, so an attempt that resumed the last five resources of a
    /// thirty-resource selection and counted from zero would report five
    /// against a recorded twenty-five: the run would sit at twenty-five,
    /// looking stalled, for the whole of the work that finished it.
    fn resuming(total: usize, baseline: RunProgress) -> Self {
        Self {
            total,
            report: PassReport::default(),
            progress: baseline,
            page: Vec::new(),
            skipped: Vec::new(),
            opened: tokio::time::Instant::now(),
        }
    }

    /// What this device's own screen reads while the pass runs.
    fn local(&self) -> ImportProgress {
        ImportProgress {
            total: self.total,
            described: self.report.described,
            skipped: self.report.skipped.len(),
        }
    }

    /// Records one resource, described or skipped.
    ///
    /// `truncating` rather than a refusal for a skip: losing the tail of an
    /// adapter's sentence is better than losing the skip it explains, and the
    /// skip is what stops a partial catalogue publishing as a whole one.
    fn took(&mut self, outcome: Result<ObservedResource, String>, locator: i64) {
        self.progress.processed = self.progress.processed.saturating_add(1);
        match outcome {
            Ok(observed) => {
                self.page.push(observed);
                self.report.described += 1;
                self.progress.described = self.progress.described.saturating_add(1);
            }
            Err(why) => {
                let entry = SkippedResource {
                    locator: Locator::from_resource_id(locator),
                    why: Reason::truncating(&why),
                };
                self.skipped.push(entry.clone());
                self.report.skipped.push(entry);
            }
        }
    }

    /// Whether the page should go now.
    ///
    /// The size bound is what limits the work one page risks; the age bound is
    /// what limits how long the seller waits to see any of it, because a shop
    /// of six must not sit behind a twenty-five-item threshold.
    fn page_is_due(&self) -> bool {
        if self.page.is_empty() && self.skipped.is_empty() {
            return false;
        }
        self.page.len() >= PAGE_SIZE
            || tokio::time::Instant::now().duration_since(self.opened) >= FLUSH_AFTER
    }

    /// Whether this run has read nothing at all, over its whole life, while
    /// failing on something — which is a failed run rather than a completed
    /// one.
    ///
    /// The baseline is part of the question, not decoration. A run that
    /// described twenty-five resources, was interrupted, and resumed to find
    /// its last five unreadable has read twenty-five: calling that a failure
    /// would tell the seller nothing came across when almost all of it did,
    /// and would settle the run failed over five skips.
    fn nothing_described(&self) -> bool {
        self.progress.described == 0 && !self.report.skipped.is_empty()
    }

    fn nothing_pending(&self) -> bool {
        self.page.is_empty() && self.skipped.is_empty()
    }

    fn take_described(&mut self) -> Vec<ObservedResource> {
        core::mem::take(&mut self.page)
    }

    fn take_skipped(&mut self) -> Vec<SkippedResource> {
        core::mem::take(&mut self.skipped)
    }

    /// This walk's progress as it will stand once the page carrying those
    /// locators has been acknowledged.
    fn once_posted(&self, acknowledged: &[String]) -> RunProgress {
        let mut earned = self.progress.clone();
        earned.acknowledged.extend(acknowledged.iter().cloned());
        earned.pages_posted = earned.pages_posted.saturating_add(1);
        earned
    }

    /// A page the server has acknowledged: its resources are what a resumed
    /// attempt must not read again, and the clock for the next flush starts
    /// here.
    fn posted(&mut self, acknowledged: Vec<String>) {
        self.progress.acknowledged.extend(acknowledged);
        self.progress.pages_posted = self.progress.pages_posted.saturating_add(1);
        self.report.pages_posted += 1;
        self.opened = tokio::time::Instant::now();
    }
}

/// The one listing page a shop that holds nothing still posts.
///
/// `chunks` yields nothing for an empty slice, and an empty shop that posted
/// no listing page at all would leave the run with no discovery to close: the
/// seller would watch a run that never leaves `waiting`. One empty page,
/// carrying `enumeration_complete`, is what says "there is nothing here"
/// rather than saying nothing.
fn empty_shop(listed: &[ListedResource]) -> impl Iterator<Item = &[ListedResource]> {
    core::iter::once(listed).filter(|shop| shop.is_empty())
}

/// How this device reaches one seller's shop.
///
/// A parameter rather than a direct call into [`crate::commands`], for the
/// reason every other seam in this module is one: the production factory
/// builds a marketplace client over the seller's stored session, so a test
/// that had to go through it could not drive this without a marketplace.
///
/// Shared and owned rather than borrowed, because a run outlives the call
/// that started it: the factory travels into the task with everything else
/// the task needs.
pub type CatalogueFactory = Arc<
    dyn Fn(&ImportContext, tam_types::InventoryId) -> Result<Box<dyn CatalogueSource>, String>
        + Send
        + Sync,
>;

/// Everything one run's work needs, owned.
///
/// The supervisor's whole reason for existing is that a run must outlive the
/// window that asked for it, so nothing here may borrow from the command that
/// started it. Every field is a handle the application already holds, taken
/// from [`crate::state::DesktopState`] in one place so a task cannot be
/// handed half of them.
#[derive(Clone)]
pub struct ImportContext {
    pub device: crate::device::DeviceId,
    pub plane: Arc<dyn crate::heartbeat::ControlPlane>,
    pub ledger: Arc<dyn LedgerTransport>,
    pub sessions: Arc<dyn crate::session::SessionStore>,
    pub gate: Arc<Mutex<EntitlementGate>>,
    /// The process-wide revocation flag, shared so a check-in that learns of
    /// a sign-out reaches every run in flight before its next request.
    pub revoked: Arc<AtomicBool>,
    pub journal: Arc<dyn ImportJournal>,
    pub supervisor: Arc<ImportSupervisor>,
    pub catalogue: CatalogueFactory,
    /// Where an import keeps the originals it reads, where this build has
    /// a library.
    pub library: Option<Arc<crate::library::LibrarySlot>>,
}

impl ImportContext {
    /// The handles the running application holds.
    #[must_use]
    pub fn of(state: &DesktopState) -> Option<Self> {
        Some(Self {
            device: state.device().id.clone(),
            plane: state.plane_handle(),
            // A build with no transport cannot import, and says so rather than
            // starting a run whose pages would have nowhere to go.
            ledger: state.ledger()?,
            sessions: state.store_handle(),
            gate: state.gate_handle(),
            revoked: state.stopper(),
            journal: state.journal(),
            supervisor: state.supervisor(),
            catalogue: state.catalogue(),
            library: state.library(),
        })
    }
}

/// Who asked for this run to be worked, which decides what happens when the
/// device cannot work it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartIntent {
    /// The seller pressed Start, Continue or Resume on this device.
    ///
    /// The claim comes first, before this device has read the run's source or
    /// checked whether it can reach the shop at all, and that order is the
    /// repair: every refusal after it is a durable fact about a run this
    /// device owns rather than a sentence in a window that is about to
    /// navigate away.
    Pressed { takeover: bool },
    /// This device found the run at a check-in, a resume or a start-up.
    ///
    /// The claim comes last for a run that merely happens to be open, after
    /// the local session and the entitlement, because an idle device that
    /// cannot read a shop must leave the run for one that can rather than
    /// fail it.
    ///
    /// `nominated` reverses that. A run whose owner is this device is one
    /// somebody chose this device for, and then the silence is the defect
    /// rather than the courtesy: the run is waiting for a device that cannot
    /// read the shop, and nobody else will ever claim it, so the refusal is
    /// claimed and reported exactly as a press's would be. That is the
    /// missing-session case the console cannot see and only this device
    /// knows.
    Found { nominated: bool },
}

/// One instruction to the supervisor.
///
/// `source` is absent where the run is the only thing the caller knows, which
/// is every press: the console asks by run id alone, so the device reads the
/// shop from the run itself — after the claim, so that read failing is
/// reported rather than lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOrder {
    pub run: tam_types::Uuid,
    pub source: Option<tam_types::InventoryId>,
    pub phase: RunPhase,
    pub intent: StartIntent,
}

/// The fence this device now holds, and where the work resumes from.
pub struct ClaimedRun {
    pub run: tam_types::Uuid,
    pub source: tam_types::InventoryId,
    pub attempt: u64,
    pub lease_expires_at: i64,
    pub phase: RunPhase,
    pub stop: StopSignal,
    /// What a previous attempt got through, read from the checkpoint.
    pub progress: RunProgress,
}

/// What a start, a continue or a resume answers once the run is durably this
/// device's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    pub attempt: u64,
    pub lease_expires_at: i64,
    pub phase: RunPhase,
}

/// Why a run was not taken.
///
/// Two answers rather than one, because only one of them is about the run: a
/// refusal is a fact the run has been told, and a busy answer is this device
/// already taking it and is told to nobody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotAccepted {
    /// This device is already taking that run, in a call that has not
    /// finished claiming it yet.
    Busy,
    /// The run was not taken, and the reason is a durable fact about it
    /// wherever this device owned it.
    Refused(PassError),
}

impl core::fmt::Display for NotAccepted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Busy => f.write_str(
                "this device is already starting that import; the run's own page shows what it \
                 is doing",
            ),
            Self::Refused(why) => why.fmt(f),
        }
    }
}

/// What stopping one run on this device did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped {
    /// Whether this device was working the run at all. False is not a
    /// failure: the seller may be stopping a run another device holds, and
    /// the server is what settles that one.
    pub was_running: bool,
    /// Whether the stop was written down.
    ///
    /// Surfaced rather than hidden: a stop this device could not record is
    /// one a restart would not know about, so the console can say that the
    /// work here has ended but the intention was not kept — which is a
    /// different thing from waiting on the server.
    pub recorded: bool,
    /// Whether the server has yet to be told. True while this device is
    /// offline, which the console renders as "stopped here, confirmation
    /// pending" rather than as a failure.
    pub server_pending: bool,
}

/// The runs this device is working, and the handle that stops each.
///
/// One cancellation handle per run rather than a set of ids, which is the
/// difference the incident turned on: a set could say that a run was claimed
/// and could not stop it, so the console's Stop settled the run on the server
/// while the phone went on making marketplace requests for it.
///
/// Deliberately small and import-specific. It holds no runtime, no
/// transaction and no session; the only thing that happens under its lock is
/// taking or releasing a slot.
#[derive(Default)]
pub struct ImportSupervisor {
    active: Mutex<HashMap<tam_types::Uuid, Slot>>,
    /// Names each occupancy of a slot, so an attempt that unwinds late
    /// releases its own rather than the one that replaced it.
    generations: core::sync::atomic::AtomicU64,
}

/// One run this device is working, or is in the middle of taking.
struct Slot {
    generation: u64,
    /// `None` while the claim is in flight. A slot exists from before the
    /// first await so two concurrent calls cannot both find the run absent
    /// and both claim it, which produced two fences and two tasks for one
    /// run.
    held: Option<Accepted>,
    stop: StopSignal,
}

impl ImportSupervisor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this device is working that run right now.
    pub async fn holds(&self, run: tam_types::Uuid) -> bool {
        self.active.lock().await.contains_key(&run)
    }

    /// Every run this device is working.
    pub async fn running(&self) -> Vec<tam_types::Uuid> {
        self.active.lock().await.keys().copied().collect()
    }

    /// Takes one run, durably, and starts working it in the background.
    ///
    /// Returns once the server has recorded this device as the owner, the
    /// checkpoint is on disk and anything this device still owed for the run
    /// has been offered under the new fence — which is what "accepted" means
    /// — rather than once the shop has been read. Everything after that point
    /// is reported to the run, because by then the window may be gone.
    pub async fn accept(
        &self,
        ctx: &ImportContext,
        order: RunOrder,
    ) -> Result<Accepted, NotAccepted> {
        // The slot is taken before the first await, so a press racing a
        // check-in reattaches to the run in hand instead of claiming a second
        // fence for it.
        let taken = match self.reserve(ctx, order.run).await {
            Reservation::Held(accepted) => return Ok(accepted),
            Reservation::Taking => return Err(NotAccepted::Busy),
            Reservation::Mine(mine) => mine,
        };
        match self.take(ctx, order, &taken).await {
            Ok(accepted) => Ok(accepted),
            Err(why) => {
                // The slot goes back on every failing path, or a refused run
                // could never be tried again without a restart.
                self.release(order.run, taken.generation).await;
                Err(NotAccepted::Refused(why))
            }
        }
    }

    /// Takes the slot, or says who has it.
    ///
    /// The handle the slot is created with is handed back rather than looked
    /// up again later, and that is the invariant: an occupancy uses its own
    /// signal from beginning to end. Reading the map again would let a call
    /// that was cancelled and replaced pick up the replacement's handle and
    /// spawn work the replacement cannot stop.
    async fn reserve(&self, ctx: &ImportContext, run: tam_types::Uuid) -> Reservation {
        let mut active = self.active.lock().await;
        if let Some(slot) = active.get(&run) {
            return slot.held.map_or(Reservation::Taking, Reservation::Held);
        }
        let generation = self.generations.fetch_add(1, Ordering::SeqCst);
        let stop = StopSignal::over(Arc::clone(&ctx.revoked));
        active.insert(
            run,
            Slot {
                generation,
                held: None,
                stop: stop.clone(),
            },
        );
        drop(active);
        Reservation::Mine(Occupancy { generation, stop })
    }

    /// Whether this occupancy still holds the slot.
    ///
    /// Asked immediately before the work is started. A stop that arrived
    /// while the claim was in flight removed the slot, and spawning then
    /// would leave work running under a handle nothing holds any more.
    async fn still_mine(&self, run: tam_types::Uuid, generation: u64) -> bool {
        self.active
            .lock()
            .await
            .get(&run)
            .is_some_and(|slot| slot.generation == generation)
    }

    /// Records what the claim answered, so a later caller reattaches.
    async fn hold(&self, run: tam_types::Uuid, generation: u64, accepted: Accepted) {
        if let Some(slot) = self.active.lock().await.get_mut(&run) {
            if slot.generation == generation {
                slot.held = Some(accepted);
            }
        }
    }

    /// Releases the slot, but only if it is still this occupancy's.
    ///
    /// The generation is what stops a task unwinding late from releasing the
    /// attempt that replaced it, which would leave a run being worked with no
    /// handle able to stop it.
    async fn release(&self, run: tam_types::Uuid, generation: u64) {
        let mut active = self.active.lock().await;
        if active
            .get(&run)
            .is_some_and(|slot| slot.generation == generation)
        {
            active.remove(&run);
        }
    }

    /// Everything between taking the slot and the work being under way.
    async fn take(
        &self,
        ctx: &ImportContext,
        order: RunOrder,
        mine: &Occupancy,
    ) -> Result<Accepted, PassError> {
        match order.intent {
            // A run this device merely found, owes nothing for and was not
            // nominated for: local readiness decides before the claim, so an
            // idle device leaves it for one that can read the shop.
            StartIntent::Found { nominated: false } if !owes_anything(ctx, order.run).await => {
                self.found(ctx, order, mine).await
            }
            // Nominated, or carrying something this device still owes the
            // server for the run. Either way the claim comes first: a run
            // waiting for this device is not somebody else's to leave alone,
            // and a refusal already recorded must be delivered before local
            // readiness is allowed to end the attempt again.
            StartIntent::Found { .. } => self.pressed(ctx, order, mine, false).await,
            StartIntent::Pressed { takeover } => {
                // An explicit press is the seller asking for this run again,
                // so a stop this device recorded while offline no longer
                // holds it back. Recorded before the claim, because the claim
                // is the point of no return.
                ctx.journal
                    .mutate(&JournalChange::Resumed(order.run))
                    .await
                    .map_err(PassError::Journal)?;
                self.pressed(ctx, order, mine, takeover).await
            }
        }
    }

    /// A run this device found: everything local is checked before the claim,
    /// so an idle device that cannot read the shop leaves the run for one that
    /// can.
    async fn found(
        &self,
        ctx: &ImportContext,
        order: RunOrder,
        mine: &Occupancy,
    ) -> Result<Accepted, PassError> {
        let source = order
            .source
            .ok_or_else(|| PassError::Source("a found run names its own shop".to_owned()))?;
        let catalogue = (ctx.catalogue)(ctx, source).map_err(PassError::UnsupportedSource)?;
        preflight(ctx, source).await?;
        let claimed = claim(ctx, order, source, mine.stop.clone(), false).await?;
        self.begin(ctx, claimed, catalogue, mine).await
    }

    /// A run the seller pressed for, was nominated for, or still owes the
    /// server something about: the claim comes first so every refusal after
    /// it is durable, and each is reported to the run before it is handed
    /// back to the window.
    async fn pressed(
        &self,
        ctx: &ImportContext,
        order: RunOrder,
        mine: &Occupancy,
        takeover: bool,
    ) -> Result<Accepted, PassError> {
        // The source is read after the claim rather than before it, and that
        // is the correction: this read can fail on its own — an outage, a
        // protocol change, a run another tenant owns — and before the claim
        // there was no run this device owned to record that against.
        let unsourced = claim_unsourced(ctx, order, mine.stop.clone(), takeover).await?;
        let ledger = Arc::new(RunLedger::new(
            ctx.device.clone(),
            &unsourced,
            Arc::clone(&ctx.ledger),
            Arc::clone(&ctx.journal),
        ));
        // Before local readiness is consulted at all. A refusal this device
        // recorded while offline — "no session for your shop" is the whole
        // reason the journal holds one — must reach the run on the first
        // connection that can carry it, and the old order let the same
        // missing session that produced it stop it being delivered, forever.
        deliver_owed(ctx, &ledger, &unsourced).await?;
        let source = match resolve_source(ctx, order).await {
            Ok(source) => source,
            Err(why) => return Err(refused(&ledger, why).await),
        };
        let catalogue = match (ctx.catalogue)(ctx, source).map_err(PassError::UnsupportedSource) {
            Ok(catalogue) => catalogue,
            Err(why) => return Err(refused(&ledger, why).await),
        };
        if let Err(why) = preflight(ctx, source).await {
            return Err(refused(&ledger, why).await);
        }
        let claimed = ClaimedRun {
            source,
            progress: resume_point(ctx, order.run, order.phase).await?,
            ..unsourced
        };
        self.begin(ctx, claimed, catalogue, mine).await
    }

    /// Registers the fence, delivers what the run still owed under it, and
    /// starts the work.
    async fn begin(
        &self,
        ctx: &ImportContext,
        claimed: ClaimedRun,
        catalogue: Box<dyn CatalogueSource>,
        mine: &Occupancy,
    ) -> Result<Accepted, PassError> {
        let accepted = Accepted {
            attempt: claimed.attempt,
            lease_expires_at: claimed.lease_expires_at,
            phase: claimed.phase,
        };
        let ledger = Arc::new(RunLedger::new(
            ctx.device.clone(),
            &claimed,
            Arc::clone(&ctx.ledger),
            Arc::clone(&ctx.journal),
        ));
        deliver_owed(ctx, &ledger, &claimed).await?;
        // Read back rather than assumed. Delivering a queued page advances
        // the run's checkpoint in the same change that retires it, so what
        // this attempt must not read again is whatever the journal says now —
        // not what it said before the queue was drained.
        let resumed = resume_point(ctx, claimed.run, claimed.phase).await?;
        let claimed = ClaimedRun {
            progress: resumed,
            ..claimed
        };
        let ledger = Arc::new(RunLedger::new(
            ctx.device.clone(),
            &claimed,
            Arc::clone(&ctx.ledger),
            Arc::clone(&ctx.journal),
        ));
        // The checkpoint before the work rather than after the first page:
        // this is what "durable acceptance" means, and it is what lets a
        // process killed one instant later resume rather than re-read. It
        // carries the resumed counts rather than zeros, because the server
        // keeps the greatest it has seen and a run must not appear to go
        // backwards.
        ledger.checkpoint(&claimed.progress).await?;
        let pass = ImportPass::new(
            catalogue,
            Arc::clone(&ledger),
            SourcePermission {
                source: claimed.source,
                gate: Arc::clone(&ctx.gate),
                stop: claimed.stop.clone(),
            },
        )
        .keeping(ctx.library.clone());
        // Nothing is started for an occupancy that no longer holds the slot:
        // a stop that arrived while the claim was in flight took the slot
        // away, and spawning under a handle nobody holds is work nothing can
        // stop.
        if !self.still_mine(claimed.run, mine.generation).await {
            return Err(PassError::Stopped);
        }
        self.hold(claimed.run, mine.generation, accepted).await;
        spawn_keeper(ctx, claimed.run, &claimed);
        spawn_worker(ctx.clone(), claimed, pass, ledger, mine.generation);
        Ok(accepted)
    }

    /// Stops this device's work on one run.
    ///
    /// The handle is raised before anything else happens, so the next check
    /// between resources ends the pass and a page built before the stop is
    /// refused on its way out. Nothing here waits for the marketplace request
    /// already in flight: that one completes and its result is dropped, which
    /// is bounded by the transport's own timeout rather than by the shop.
    ///
    /// The stop is written down as well as raised. A handle goes with the
    /// process, and a stop that existed only in memory was undone by a
    /// restart: the run was still open on the server, the next discovery
    /// cycle claimed it again, and the phone carried on reading the shop the
    /// seller had stopped.
    pub async fn cancel(
        &self,
        journal: &Arc<dyn ImportJournal>,
        device: &crate::device::DeviceId,
        run: tam_types::Uuid,
    ) -> Stopped {
        let held = self.active.lock().await.remove(&run);
        if let Some(slot) = held.as_ref() {
            slot.stop.cancel();
        }
        // Recorded whether or not this device was working the run. A stop
        // this device knows about and did not write down is one the next
        // discovery cycle would undo by claiming the run again, and that is
        // true of a run this device had not yet started as much as of one it
        // had.
        let recorded = journal.mutate(&JournalChange::Stopped(run)).await;
        if let Err(why) = recorded.as_ref() {
            eprintln!("a stopped import could not be recorded on this device: {why}");
        }
        // The abandonment this device owes the server. The console asks the
        // server directly too, and that is the authoritative path; this is
        // the one that survives a phone with no signal, where the console's
        // own call cannot happen at all. Both are the same instruction about
        // the same run, and a server that has already accepted one answers
        // the other as settled or as already abandoned.
        //
        // The device route is used only where this device knows the attempt it
        // held, because that route carries the fence and admits the lapsed
        // lease this case produces. A run this device never held has no safe
        // delayed request to queue.
        let attempt = match held.as_ref().and_then(|slot| slot.held) {
            Some(accepted) => Some(accepted.attempt),
            None => journal
                .read()
                .await
                .ok()
                .and_then(|state| state.checkpoint(run).map(|kept| kept.attempt)),
        };
        // Only under a fence, and only this device's own. An unfenced
        // abandonment queued here and delivered later could stop a newer
        // attempt that a takeover had since started — the seller's stop was
        // about this device's work, not about whatever is running now — so a
        // run this device holds no attempt for is left to the console's own
        // call, and the tombstone above is what keeps this device from
        // picking it up again meanwhile.
        if let Some(attempt) = attempt {
            let owed = PendingPost::new(
                run,
                &device_stop_path(device, run),
                &serde_json::json!({ "attempt": attempt }).to_string(),
            )
            .of_kind(PostKind::Stop);
            if let Err(why) = journal.mutate(&JournalChange::Enqueue(owed)).await {
                eprintln!("a stop this device owes the server could not be queued: {why}");
            }
        }
        Stopped {
            was_running: held.is_some(),
            recorded: recorded.is_ok(),
            // Always pending, because this device never learns here that the
            // server agreed: the console's own abandon call is what settles
            // the run, and the mark is only cleared once a later cycle sees
            // the server stop offering it. Answering "confirmed" on a local
            // act would be this device vouching for a server it has not
            // spoken to — which it cannot do at all when it is offline, and
            // must not do when it is not.
            server_pending: true,
        }
    }
}

/// Who holds a run's slot.
enum Reservation {
    /// This device already holds the run under a known fence.
    Held(Accepted),
    /// Another call is claiming it and has not answered yet.
    Taking,
    /// This call holds the slot, under this occupancy.
    Mine(Occupancy),
}

/// One call's hold on a run's slot: which occupancy it is, and the handle
/// that occupancy created.
///
/// Both travel together for the whole of `accept`, so nothing downstream has
/// to look the slot up again and risk finding a later one's.
struct Occupancy {
    generation: u64,
    stop: StopSignal,
}

/// Whether this device still owes the server anything about that run.
///
/// Asked before local readiness is consulted, because a device that cannot
/// read a shop may still be the only one holding the account of why: the
/// refusal it recorded while offline is delivered under a fresh claim, not
/// abandoned because the same missing session is still missing.
async fn owes_anything(ctx: &ImportContext, run: tam_types::Uuid) -> bool {
    ctx.journal
        .read()
        .await
        .is_ok_and(|state| !state.owed(run).is_empty())
}

/// Re-fences everything this run still owes to the attempt in hand, and
/// offers it.
///
/// Before any new work and before any local decision. A page or a failure
/// queued under a lapsed lease is refused as stale, so it has to be re-fenced
/// first; and it has to be offered before readiness is consulted, or the
/// condition that produced the refusal is the condition that buries it.
async fn deliver_owed(
    ctx: &ImportContext,
    ledger: &RunLedger,
    claimed: &ClaimedRun,
) -> Result<(), PassError> {
    ctx.journal
        .mutate(&JournalChange::Refence {
            run: claimed.run,
            attempt: claimed.attempt,
        })
        .await
        .map_err(PassError::Journal)?;
    match ledger.flush_outbox().await {
        Ok(()) => Ok(()),
        // A terminal ending has to be reported here, because this call
        // returns before any worker exists: the caller hands the failure back
        // to whoever asked, and nothing else on this path tells the run.
        Err(why) if matches!(why.stage(), ImportStage::Failed) => Err(refused(ledger, why).await),
        Err(why) => Err(why),
    }
}

/// Reports a refusal to the run, then hands it back for the window as well.
///
/// Both, and in that order: the run is where the seller will look, and the
/// window is where they are looking now. Before this the second happened and
/// the first did not, which is the whole of the reported incident.
///
/// Nothing is cleared here. The report may not have reached the server — that
/// is the ordinary case on the device this repair is about — so the journal
/// keeps it for the next connection, and a later cycle delivers it.
async fn refused(ledger: &RunLedger, why: PassError) -> PassError {
    ledger.report_ending(&why, &RunProgress::default()).await;
    why
}

/// Which shop the run names, read from the run itself.
async fn resolve_source(
    ctx: &ImportContext,
    order: RunOrder,
) -> Result<tam_types::InventoryId, PassError> {
    if let Some(source) = order.source {
        return Ok(source);
    }
    ctx.plane
        .import_run_facts(order.run)
        .await
        .map(|facts| facts.source)
        .map_err(|why| PassError::Source(why.to_string()))
}

/// Whether this device can read that shop at all.
///
/// The local session first, because it is the fact only this device holds and
/// the one the console cannot see: an organisation "connected" on another
/// phone is not a session here. Then the entitlement, which is the
/// subscription's.
async fn preflight(ctx: &ImportContext, source: tam_types::InventoryId) -> Result<(), PassError> {
    let marketplace = source.marketplace();
    if ctx.revoked.load(Ordering::SeqCst) {
        return Err(PassError::Revoked);
    }
    if ctx.sessions.get(marketplace).await.ok().flatten().is_none() {
        return Err(PassError::NoSession(marketplace));
    }
    if !ctx
        .gate
        .lock()
        .await
        .may_work(marketplace, crate::run::wall_now())
    {
        return Err(PassError::NotEntitled(marketplace));
    }
    Ok(())
}

/// Takes the fence for a run whose shop this device has not read yet.
///
/// The source is a placeholder here and is replaced the moment the run's own
/// is known. It exists because a failure reporter has to work before an
/// adapter does: the ledger this builds is what carries "I could not even
/// find out which shop this is" to the run.
async fn claim_unsourced(
    ctx: &ImportContext,
    order: RunOrder,
    stop: StopSignal,
    takeover: bool,
) -> Result<ClaimedRun, PassError> {
    let assumed = order.source.unwrap_or(tam_types::InventoryId::Tes);
    claim(ctx, order, assumed, stop, takeover).await
}

/// Takes the fence for this attempt.
///
/// A refused claim is either somebody else's run or a run that is over, and
/// both are [`PassError::FenceLost`]: this device does not own it and must not
/// write to it. Anything else is an outage, which is a different answer
/// because it resolves itself.
async fn claim(
    ctx: &ImportContext,
    order: RunOrder,
    source: tam_types::InventoryId,
    stop: StopSignal,
    takeover: bool,
) -> Result<ClaimedRun, PassError> {
    let body = serde_json::json!({ "takeover": takeover }).to_string();
    let answer = ctx
        .ledger
        .post(&claim_path(&ctx.device, order.run), body)
        .await;
    let lease = match answer {
        Ok(body) => serde_json::from_str::<ImportLease>(&body)
            .map_err(|why| PassError::Page(why.to_string()))?,
        Err(ControlPlaneError::Fenced(_)) => return Err(PassError::FenceLost),
        Err(why) => return Err(PassError::Page(why.to_string())),
    };
    Ok(ClaimedRun {
        run: order.run,
        source,
        attempt: lease.attempt,
        lease_expires_at: lease.lease_expires_at,
        phase: order.phase,
        stop,
        progress: resume_point(ctx, order.run, order.phase).await?,
    })
}

/// What the run itself says about how far it has got.
///
/// Read from the server rather than assumed from this device's journal, and
/// that is the takeover case: the phone taking a run over has an empty
/// journal, so a baseline taken from it alone would report a run that had
/// described two hundred resources as having described none. The counts are
/// the server's, which is the only place they are authoritative.
///
/// The acknowledged locators are deliberately not here. The run's view
/// carries counts, not a list, so a device that takes over re-reads what it
/// cannot know was done — which is safe because the server applies a page
/// per locator, so a resource described twice is applied once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFacts {
    pub source: tam_types::InventoryId,
    pub discovered: u32,
    /// Everything the run has been through, failures included.
    pub processed: u32,
    /// What it actually described. Not the same number, and the difference
    /// matters: a device taking over a run of thirty that read twenty-five
    /// and failed five has described twenty-five, and judging "has this run
    /// read anything at all" by `processed` would call a run successful that
    /// had failed on every resource it touched.
    pub described: u32,
    pub enumeration_complete: bool,
}

/// Where a resumed attempt picks this half up from.
///
/// The checkpoint's, where one matches this half, and the beginning
/// otherwise: a run resumed into the other half starts its pages at zero
/// because the receipts of the two halves are separate sequences. Read rather
/// than derived, and a journal that could not be read stops the attempt
/// instead of resuming from a state this device invented.
async fn resume_point(
    ctx: &ImportContext,
    run: tam_types::Uuid,
    phase: RunPhase,
) -> Result<RunProgress, PassError> {
    let state = ctx.journal.read().await.map_err(PassError::Journal)?;
    let mut resumed = state
        .checkpoint(run)
        .filter(|kept| kept.phase == phase)
        .map_or_else(RunProgress::default, RunCheckpoint::progress);
    // The run's own counts, taken as the greater. This device's journal is
    // empty on a takeover and stale after an outage, and the server keeps the
    // greatest count it has seen: resuming from the local view alone would
    // report a run that had described two hundred resources as having
    // described none, and the console would show it going backwards.
    // Surfaced rather than shrugged off: a run whose facts this device could
    // not read is one it cannot resume truthfully, and resuming from zero is
    // how a takeover comes to report work already done as undone.
    let facts = ctx
        .plane
        .import_run_facts(run)
        .await
        .map_err(|why| PassError::Source(why.to_string()))?;
    resumed.discovered = resumed.discovered.max(facts.discovered);
    resumed.processed = resumed.processed.max(facts.processed);
    resumed.described = resumed.described.max(facts.described);
    resumed.enumeration_complete |= facts.enumeration_complete;
    Ok(resumed)
}

/// Renews the fence on its own timer.
///
/// Its own task rather than a step in the walk, which is the property the old
/// arrangement could not have: a device blocked on a slow marketplace request
/// went on holding a lease it was not renewing, so the server could not tell
/// a working device from a vanished one.
///
/// Renewal and nothing else. What this device owes the server is the worker's
/// to deliver, in order, before its next page; a second task offering the
/// same queue would be a second authority over one run's lifecycle.
fn spawn_keeper(ctx: &ImportContext, run: tam_types::Uuid, claimed: &ClaimedRun) {
    let ctx = ctx.clone();
    let stop = claimed.stop.clone();
    let attempt = claimed.attempt;
    tauri::async_runtime::spawn(async move {
        while stop.why().is_none() && !stop.finished() {
            tokio::time::sleep(RENEW_EVERY).await;
            // Both conditions again after the sleep: the work may have
            // finished or been stopped while this task was waiting, and
            // renewing a lease for work that is over holds a fence nobody
            // needs.
            if stop.why().is_some() || stop.finished() {
                break;
            }
            if let Err(PassError::FenceLost) = renew(&ctx, run, attempt).await {
                // Another attempt holds this run, or it is over. Raising the
                // handle is what stops the work between resources and refuses
                // the page it was building.
                stop.lose_fence();
                break;
            }
        }
    });
}

/// One renewal. An outage is not a lost fence: the lease lapsing server-side
/// is what makes the run truthfully interrupted, and a device that stopped
/// reading on one failed renewal would abandon work it could still finish.
///
/// A sign-out is not an outage, and this is the fastest path that learns of
/// one: the keeper renews every fifteen seconds, so reading a revocation here
/// as a lost fence is what stops the shop being read within one renewal of
/// the seller signing this machine out. Treating it as transient would leave
/// the keeper renewing a lease the server refuses while the walk went on
/// making marketplace requests for a device that may make none.
async fn renew(ctx: &ImportContext, run: tam_types::Uuid, attempt: u64) -> Result<(), PassError> {
    let body = serde_json::json!({ "attempt": attempt }).to_string();
    match ctx.ledger.post(&renew_path(&ctx.device, run), body).await {
        Ok(_) => Ok(()),
        Err(
            ControlPlaneError::Fenced(_)
            | ControlPlaneError::Denied(_)
            | ControlPlaneError::Revoked,
        ) => Err(PassError::FenceLost),
        Err(why) => Err(PassError::Page(why.to_string())),
    }
}

/// Works the run to its ending, whatever that ending is, and reports it.
fn spawn_worker(
    ctx: ImportContext,
    claimed: ClaimedRun,
    pass: ImportPass<Box<dyn CatalogueSource>>,
    ledger: Arc<RunLedger>,
    generation: u64,
) {
    tauri::async_runtime::spawn(async move {
        let outcome = match claimed.phase {
            RunPhase::Discover => discover(&pass).await,
            RunPhase::Describe => describe(&ctx, &claimed, &pass).await,
        };
        finish(&ctx, &claimed, &ledger, outcome).await;
        // The renewal task ends with the work, whatever the ending was.
        claimed.stop.finish();
        ctx.supervisor.release(claimed.run, generation).await;
    });
}

/// The first half: read the shop and post what is in it.
async fn discover(pass: &ImportPass<Box<dyn CatalogueSource>>) -> Result<(), PassError> {
    let listed = pass.enumerate(crate::run::wall_now()).await?;
    pass.post_listing(listed).await
}

/// The second half: describe what the run's own selection names, minus what a
/// previous attempt already got acknowledged.
async fn describe(
    ctx: &ImportContext,
    claimed: &ClaimedRun,
    pass: &ImportPass<Box<dyn CatalogueSource>>,
) -> Result<(), PassError> {
    let selection = ctx
        .plane
        .import_selection(&ctx.device, claimed.run)
        .await
        .map_err(|why| PassError::Page(why.to_string()))?;
    let done = &claimed.progress.acknowledged;
    let chosen: Vec<i64> = selection
        .iter()
        .filter(|locator| !done.contains(locator))
        .filter_map(|locator| locator.parse().ok())
        .collect();
    // The local callback is dropped here on purpose, and it is not the
    // progress that used to be discarded: every step of this walk is reported
    // to the run by the ledger, which is the channel the console reads. The
    // callback is this device's own screen, and a background task has no
    // screen.
    pass.describe_all(chosen, crate::run::wall_now, |_progress| {})
        .await
        .map(|_report| ())
}

/// Records how the run ended, and lets the next attempt take it.
///
/// The journal entry goes only when there is nothing left to say: a run whose
/// last page or whose ending is still queued keeps its entry, whatever the
/// ending was, because clearing it is precisely how the page a restart would
/// have replayed and the reason the seller would have read came to be lost.
/// An interruption keeps it too, and for a second reason: the checkpoint is
/// what a resume reads.
async fn finish(
    ctx: &ImportContext,
    claimed: &ClaimedRun,
    ledger: &RunLedger,
    outcome: Result<(), PassError>,
) {
    let reached = ctx
        .journal
        .read()
        .await
        .ok()
        .and_then(|state| state.checkpoint(claimed.run).map(RunCheckpoint::progress))
        .unwrap_or_else(|| claimed.progress.clone());
    let terminal = match outcome {
        Ok(()) => true,
        Err(why) => {
            ledger.report_ending(&why, &reached).await;
            matches!(why.stage(), ImportStage::Failed)
        }
    };
    let owed = ledger.owed().await.unwrap_or_else(|_| Vec::new());
    if terminal && owed.is_empty() {
        ledger.settled().await;
    }
}

/// Takes whatever work the runs this device can see are owed, once per
/// check-in, resume or start-up.
///
/// This is the half of the scheduled pull that cannot be server-side. The
/// server mints the run when the seller's cadence comes due and then waits;
/// nothing else starts it, because a server that told a device to read a shop
/// now would be the causation D1 keeps on this side of the wire. So the
/// device asks, and a run it finds enters the same supervisor a press enters.
///
/// Every refusal here is silent to the seller, and that is deliberate rather
/// than lax: no open run is the ordinary answer, and a shop this device holds
/// no session for is somebody else's run to work rather than this device's to
/// fail. What it is not silent about is the coordinator's timing question —
/// whether this pass reached the control plane at all — because a pass that
/// could not read the open runs must back off rather than report the same
/// quiet it would report with an empty list.
pub(crate) async fn serve_open_runs(
    state: &DesktopState,
    plane: &dyn crate::heartbeat::ControlPlane,
) -> crate::scheduler::Discovery {
    let Some(ctx) = ImportContext::of(state) else {
        // A build with no ledger transport, which is a fact about the build
        // and not an outage: there is nothing to retry sooner.
        return crate::scheduler::Discovery::Quiet;
    };
    let Ok(open) = plane.open_import_runs(&state.device().id).await else {
        return crate::scheduler::Discovery::Failed;
    };
    // Outbox entries which need no new claim are delivered first. The returned
    // stop snapshot also guards this cycle's stale open-run answer: a stop
    // accepted below settled the run after that answer was read, so the same
    // row must not be reclaimed in this pass.
    let Some(stopped) = drain_unclaimed_outbox(&ctx, &open).await else {
        // The drain stopped at a post it could not deliver, so the connection
        // is the thing at fault and the next pass waits on the ladder.
        return crate::scheduler::Discovery::Failed;
    };
    reconcile_stops(&ctx, &open).await;
    // A run whose pages the server keeps failing is the one case where this
    // pass has something to say about its own cadence: ten seconds is the
    // right gap for finding new work and the wrong one for a server that
    // cannot keep a page, so the existing ladder paces the retries rather
    // than a timer of this module's own.
    let mut failing = false;
    for run in &open {
        if !stopped.contains(&run.run) {
            failing |= consider(&ctx, run).await;
        }
    }
    if failing {
        return crate::scheduler::Discovery::Failed;
    }
    crate::scheduler::Discovery::Quiet
}

/// Offers outbox entries which do not need a new claim.
///
/// Every entry for a run no longer listed as open keeps the fence it already
/// carries. A queued stop does too, and so does a queued terminal ending: both
/// are offered while their run remains open, because each is the instruction
/// that closes the row and waiting for the row to close would wait forever.
/// The ending matters as much as the stop does — a run whose page the server
/// refused is over, and until that reason reaches the server the row stays
/// open and the device is offered work whose only outcome is the same refusal.
///
/// Returns the stop snapshot read with the outbox. Those runs are skipped for
/// the rest of this discovery pass even when their stop is accepted, because
/// `open` was read before the acceptance and is stale at that point.
async fn drain_unclaimed_outbox(
    ctx: &ImportContext,
    open: &[OpenImportRun],
) -> Option<Vec<tam_types::Uuid>> {
    let state = ctx.journal.read().await.ok()?;
    let stopped = state.stopped;
    for owed in state.outbox {
        let row = open.iter().find(|open| open.run == owed.run);
        if row.is_some() && !owed.settles_run() {
            continue;
        }
        let mut offer = owed.clone();
        let mut answered = ctx.ledger.post(&offer.path, offer.body.clone()).await;
        // A queued ending outlives the fence it was built under, and the
        // server answers a stale fence with a conflict. Re-fenced and offered
        // again rather than dropped: it is the only account of why the run
        // ended, and the row stays open until the server has it.
        if let (Err(ControlPlaneError::Fenced(why)), Some(row)) = (&answered, row) {
            if matches!(owed.kind, PostKind::Ending) && !stop_is_spent(why) {
                let Some(refenced) = refence_for_ending(ctx, &owed, row).await else {
                    // Another owner holds the run, or this device is working
                    // it: the ending waits for a later pass rather than being
                    // dropped.
                    continue;
                };
                offer = refenced;
                answered = ctx.ledger.post(&offer.path, offer.body.clone()).await;
            }
        }
        let retire = match &answered {
            // Proved, not merely answered: the same rule the live path uses,
            // because a gateway answers a settled run's queue exactly as
            // happily as it answers an open one's.
            Ok(body) if !offer.kind.proved_by(body) => return Some(stopped),
            Ok(_) => JournalChange::Delivered(owed.id),
            // An ending is retired only once the server has it or says the
            // run is settled. Any other conflict is ambiguous, and dropping
            // the reason on it is how a run is left open with nothing that
            // will ever explain it.
            Err(ControlPlaneError::Fenced(why)) if matches!(owed.kind, PostKind::Ending) => {
                if stop_is_spent(why) {
                    JournalChange::Delivered(owed.id)
                } else {
                    continue;
                }
            }
            // A conflict saying the run is already settled does spend a local
            // stop, because there is then nothing left to stop.
            Err(ControlPlaneError::Fenced(why))
                if owed.kind.settles_stop() && stop_is_spent(why) =>
            {
                JournalChange::Delivered(owed.id)
            }
            // Neither a fence, nor a refused sign-in, nor a machine the seller
            // signed out was applied, so none of them credits the run: a post
            // recorded as acknowledged would tell a later resume that the
            // server holds resources nobody wrote. The post is dropped,
            // because it will never be accepted, and any tombstone stands, so
            // this device still will not pick the run up by itself.
            //
            // Revocation belongs here rather than with the outage below, and
            // the queue is why: the drain stops at the first transient failure
            // to keep order, so one post refused for a sign-out would hold
            // every post behind it — for other runs included — until the
            // seller signed this machine back in. Dropping it costs nothing
            // the server ever had.
            // A page the server read and refused belongs here too, and for
            // the same reason it does on the live path: the bytes are the
            // thing it will not take, so offering them again is a queue this
            // device can never empty.
            Err(
                ControlPlaneError::Fenced(_)
                | ControlPlaneError::Denied(_)
                | ControlPlaneError::Revoked
                | ControlPlaneError::Rejected(_),
            ) => JournalChange::Abandoned(owed.id),
            // Still offline. Everything after this would fail the same way,
            // and order matters, so it waits for the next cycle.
            Err(_) => return Some(stopped),
        };
        if let Err(why) = ctx.journal.mutate(&retire).await {
            eprintln!("an answered import post could not be retired locally: {why}");
        }
    }
    Some(stopped)
}

/// Takes a fresh fence for one purpose: carrying a queued ending.
///
/// No worker and no walk. The claim carries no takeover, so it can never take
/// a run another owner holds, and it is only made when the server still names
/// this device as the owner and nothing here is working the run. The re-fence
/// is written down before the post is offered, and the post is offered in the
/// same pass: waiting for the next cycle would spend another lease.
async fn refence_for_ending(
    ctx: &ImportContext,
    owed: &PendingPost,
    open: &OpenImportRun,
) -> Option<PendingPost> {
    if open.owner_device.as_deref() != Some(ctx.device.as_str()) {
        return None;
    }
    if ctx.supervisor.holds(owed.run).await {
        return None;
    }
    let order = RunOrder {
        run: owed.run,
        source: Some(open.source),
        phase: RunPhase::owed(open).unwrap_or(RunPhase::Discover),
        intent: StartIntent::Found { nominated: true },
    };
    let claimed = claim(ctx, order, open.source, StopSignal::never(), false)
        .await
        .ok()?;
    ctx.journal
        .mutate(&JournalChange::Refence {
            run: owed.run,
            attempt: claimed.attempt,
        })
        .await
        .ok()?;
    Some(owed.under(claimed.attempt))
}

/// Drops the stop marks and the outage pauses for runs the server has
/// settled.
///
/// A run the seller stopped stays marked while the server still lists it as
/// open, which is what stops a restart quietly reclaiming it. Once the server
/// no longer offers it, the stop has been accepted and the mark has nothing
/// left to protect.
///
/// A paused run is retired by the same fact: the row is gone, so there is no
/// page left to offer and no reclaim left to bound. Leaving the count would
/// have it decide the next run to carry that id, which is nothing this device
/// has evidence about.
async fn reconcile_stops(ctx: &ImportContext, open: &[OpenImportRun]) {
    let Ok(state) = ctx.journal.read().await else {
        return;
    };
    let marked = state
        .stopped
        .iter()
        .copied()
        .chain(state.failing.iter().map(|failing| failing.run));
    for run in marked {
        if !open.iter().any(|row| row.run == run) {
            if let Err(why) = ctx.journal.mutate(&JournalChange::Resumed(run)).await {
                eprintln!("a settled import's local marks could not be cleared: {why}");
            }
        }
    }
}

/// One found run, taken if this device owes it anything.
///
/// Answers whether this device is failing to submit that run's work, which is
/// the one thing a discovery pass reports about its own cadence.
async fn consider(ctx: &ImportContext, open: &OpenImportRun) -> bool {
    let Some(phase) = RunPhase::owed(open) else {
        return false;
    };
    if ctx.supervisor.holds(open.run).await {
        return false;
    }
    let marks = ctx.journal.read().await.ok();
    // A run the seller stopped on this device is not picked up again by a
    // discovery cycle, however many times the server keeps offering it. Only
    // an explicit press resumes it, because the seller stopping something and
    // the device starting it again by itself is the same defect a volatile
    // handle produced across a restart.
    if marks
        .as_ref()
        .is_some_and(|state| state.was_stopped(open.run))
    {
        return false;
    }
    // And a run whose page this server keeps failing is not picked up again
    // either, for a neighbouring reason: each claim supersedes the last
    // attempt's fence and starts the selection over, so a cycle that reclaims
    // through an outage replaces the reason the seller is reading with a run
    // that looks busy and describes nothing. The page stays queued, the
    // reason stays on the run, and a press delivers it.
    if marks.as_ref().is_some_and(|state| state.paused(open.run)) {
        return true;
    }
    // The claim arbitrates rather than this device guessing from
    // `owner_device`: the server knows whether the lease it names is still
    // live, and a run whose owner has gone away must be resumable by the
    // phone the seller is holding.
    let taken = ctx
        .supervisor
        .accept(
            ctx,
            RunOrder {
                run: open.run,
                source: Some(open.source),
                phase,
                intent: StartIntent::Found {
                    nominated: open.owner_device.as_deref() == Some(ctx.device.as_str()),
                },
            },
        )
        .await;
    // A missing session or an unreadable shop is somebody else's run to work
    // and says nothing about this device's cadence. A submission that failed
    // does: it is this device and our own server, and the ladder is what
    // paces the next try.
    matches!(
        &taken,
        Err(NotAccepted::Refused(why)) if why.reason_code() == ImportReasonCode::SubmissionFailed
    )
}

#[cfg(test)]
mod tests {
    use super::{
        base64, content_type_for, import_path, payload_of, CatalogueSource, ImportJournal,
        ImportLease, ImportPage, ImportPass, ImportProgressReport, ImportReasonCode, ImportStage,
        ListedResource, Locator, MemoryJournal, PassError, Reason, RunLedger, RunPhase,
        SourcePermission, StopSignal, PAGE_SIZE, PNG_MAGIC,
    };
    use crate::device::DeviceId;
    use crate::entitlement::{Claims, Entitlement, EntitlementGate};
    use crate::heartbeat::{ControlPlaneError, PlaneFuture};
    use crate::import::{SourceError, SourceFuture};
    use crate::ledger::LedgerTransport;
    use crate::state::DesktopState;
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tam_marketplace::{ImportedListing, ListingState, RemoteListingId};
    use tam_types::{CopyFormat, FileKind, ImportedPrice, Marketplace, ScanOutcome, Timestamp};
    use tokio::sync::Mutex;

    const DEVICE: &str = "11112222333344445555666677778888";
    /// The import this pass belongs to. One value, so a resumed pass posting
    /// the same id is the assertion rather than a coincidence of spelling.
    const RUN: tam_types::Uuid = tam_types::Uuid([0x71; 16]);
    /// The fence the first attempt holds. One value, so a page carrying it is
    /// the assertion rather than a coincidence of spelling.
    const ATTEMPT: u64 = 1;
    const NOW_SECONDS: i64 = 1_756_000_000;
    const NOW: Timestamp = Timestamp(NOW_SECONDS * 1_000);
    const PDF: &[u8] = b"%PDF-1.7 the seller's own worksheet";

    fn listing(id: i64, state: Option<ListingState>) -> ImportedListing {
        ImportedListing {
            remote: RemoteListingId::Tes {
                url: format!("https://www.tes.com/api/v2/resources/{id}"),
            },
            title: format!("Resource {id}"),
            body: "Ten pages of practice.".to_owned(),
            body_format: CopyFormat::Markdown,
            native: Vec::new(),
            rights: None,
            price: ImportedPrice::Free,
            state,
        }
    }

    fn tes_answered(why: &str) -> SourceError {
        SourceError::Marketplace {
            marketplace: Marketplace::Tes,
            why: why.to_owned(),
        }
    }

    /// A catalogue that answers from a script.
    struct Scripted {
        catalogue: Result<Vec<i64>, String>,
        bundle: Vec<u8>,
        /// Resources whose bundle fetch fails, so one bad resource in a shop
        /// can be driven without failing the others.
        unfetchable: Vec<i64>,
        /// Resources whose snapshot reads as a draft. On Tes that snapshot
        /// is the `/{id}/draft` overlay, which a published resource with an
        /// unpublished edit also answers, so a draft here says nothing about
        /// whether there are files to bring across.
        drafts: Vec<i64>,
        /// Resources whose bundle the source confirms is not there, which is
        /// what the live binding answers for a resource with no published
        /// version. Distinct from `unfetchable`, which is a fetch that
        /// failed.
        bundleless: Vec<i64>,
        /// A source that hands this device no file at all, which is TPT.
        fileless: bool,
        pause: Option<Arc<tokio::sync::Barrier>>,
    }

    impl Scripted {
        fn of(count: i64, bundle: Vec<u8>) -> Self {
            Self {
                catalogue: Ok((1..=count).collect()),
                bundle,
                unfetchable: Vec::new(),
                drafts: Vec::new(),
                bundleless: Vec::new(),
                fileless: false,
                pause: None,
            }
        }
    }

    impl CatalogueSource for Scripted {
        fn list<'a>(
            &'a self,
            found: &'a super::CatalogueProgress,
        ) -> SourceFuture<'a, Vec<ListedResource>> {
            Box::pin(async move {
                if let Some(pause) = &self.pause {
                    pause.wait().await;
                    pause.wait().await;
                }
                let ids = self.catalogue.clone().map_err(|why| tes_answered(&why))?;
                // Reported a page at a time, as a real walk does, so a test
                // can see what a caller would have been told partway through.
                for page in 1..=ids.len().div_ceil(PAGE_SIZE) {
                    found(u32::try_from((page * PAGE_SIZE).min(ids.len())).unwrap_or(u32::MAX));
                }
                Ok(ids
                    .into_iter()
                    .map(|id| ListedResource {
                        locator: Locator::from_resource_id(id),
                        title: format!("Resource {id}"),
                        price_minor: Some(450),
                        currency: Some("GBP".to_owned()),
                        state: Some(if self.drafts.contains(&id) {
                            ListingState::Draft
                        } else {
                            ListingState::Live
                        }),
                    })
                    .collect())
            })
        }

        fn read(&self, resource: i64) -> SourceFuture<'_, ImportedListing> {
            Box::pin(async move {
                let state = if self.drafts.contains(&resource) {
                    Some(ListingState::Draft)
                } else {
                    None
                };
                Ok(listing(resource, state))
            })
        }

        fn bundle(&self, resource: i64) -> SourceFuture<'_, Option<Vec<u8>>> {
            Box::pin(async move {
                if self.fileless {
                    return Ok(None);
                }
                if self.bundleless.contains(&resource) {
                    return Ok(None);
                }
                if self.unfetchable.contains(&resource) {
                    return Err(tes_answered("the session expired"));
                }
                Ok(Some(self.bundle.clone()))
            })
        }
    }

    /// A control plane that records everything it was posted, by path.
    ///
    /// Pages, claims, renewals and progress reports all arrive through the
    /// one transport, exactly as they do in a real build where the control
    /// plane is one object: keeping them apart by path here is what lets a
    /// test assert that a failure reached the run rather than that something
    /// reached the server.
    #[derive(Default)]
    #[expect(
        clippy::struct_excessive_bools,
        reason = "each flag is one failure the fake can be told to produce; a test sets one"
    )]
    struct FakePlane {
        posted: Mutex<Vec<ImportPage>>,
        reports: Mutex<Vec<ImportProgressReport>>,
        claims: Mutex<Vec<String>>,
        stops: Mutex<Vec<String>>,
        /// Pages are refused with this, where it is set.
        refuse: bool,
        /// Everything is answered as a lost fence, which is what a device
        /// whose attempt has been superseded meets.
        fenced: bool,
        /// Everything but the claim is answered as this machine having been
        /// signed out of the seller's account, which is the registry's own
        /// forbidden answer classified by `control_plane::forbidden`.
        signed_out: bool,
        /// Pages are answered with a two-hundred carrying something this
        /// device cannot read, which is what a proxy or a captive portal
        /// does.
        gibberish: bool,
        /// Only the description page is refused, with the answer the live
        /// incident produced: the claim, the renewal and the progress report
        /// all succeed, so nothing this device reads tells it the failure is
        /// its own and everything tells it to try again.
        ///
        /// Settable rather than fixed, because the recovery half of the
        /// contract is what happens once the server comes back.
        refuse_pages: AtomicBool,
        /// How many times a page was offered, refused or not. The count the
        /// hot loop is visible in: a bounded recovery offers a shop's page a
        /// bounded number of times whatever the server keeps answering.
        page_offers: AtomicUsize,
        /// The fence each claim answers with, so a second claim is a later
        /// attempt rather than the same one.
        attempts: AtomicUsize,
    }

    impl FakePlane {
        fn refusing() -> Self {
            Self {
                refuse: true,
                ..Self::default()
            }
        }

        fn fenced_out() -> Self {
            Self {
                fenced: true,
                ..Self::default()
            }
        }

        fn signing_out() -> Self {
            Self {
                signed_out: true,
                ..Self::default()
            }
        }

        fn answering_gibberish() -> Self {
            Self {
                gibberish: true,
                ..Self::default()
            }
        }

        /// The live incident's own shape: every route answers except the one
        /// carrying the descriptions.
        fn refusing_pages() -> Self {
            Self {
                refuse_pages: AtomicBool::new(true),
                ..Self::default()
            }
        }

        /// The server coming back, which is the other half of the contract:
        /// a paused run has to be recoverable rather than merely quiet.
        fn heals(&self) {
            self.refuse_pages.store(false, Ordering::SeqCst);
        }

        async fn claims(&self) -> Vec<String> {
            self.claims.lock().await.clone()
        }

        fn page_offers(&self) -> usize {
            self.page_offers.load(Ordering::SeqCst)
        }

        async fn pages(&self) -> Vec<ImportPage> {
            self.posted.lock().await.clone()
        }

        async fn reported(&self) -> Vec<ImportProgressReport> {
            self.reports.lock().await.clone()
        }

        async fn stops(&self) -> Vec<String> {
            self.stops.lock().await.clone()
        }

        async fn record(&self, path: &str, body: &str) -> Result<String, ControlPlaneError> {
            if self.fenced {
                return Err(ControlPlaneError::Fenced(
                    r#"{"errors":[{"code":"import_run_fenced","kind":"validation","message":"another attempt holds this import"}]}"#
                        .to_owned(),
                ));
            }
            // The claim is answered even while the rest is refused: a device
            // with no connection at all could take no run, and then there
            // would be nothing to be offline about. Everything the run then
            // owes — its pages, its progress, its ending — meets the outage,
            // which is what a queue is for.
            if path.ends_with("/claim") {
                self.claims.lock().await.push(body.to_owned());
                let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
                return Ok(serde_json::json!({
                    "attempt": attempt,
                    "lease_expires_at": NOW.0 + 60_000,
                })
                .to_string());
            }
            if self.refuse {
                return Err(ControlPlaneError::Refused("no".to_owned()));
            }
            if self.signed_out {
                return Err(ControlPlaneError::Revoked);
            }
            if path.ends_with("/renew") {
                return Ok(serde_json::json!({
                    "attempt": self.attempts.load(Ordering::SeqCst).max(1),
                    "lease_expires_at": NOW.0 + 60_000,
                })
                .to_string());
            }
            if path.ends_with("/progress") {
                self.reports
                    .lock()
                    .await
                    .push(serde_json::from_str(body).expect("the report is well-formed json"));
                return Ok(String::new());
            }
            if path.ends_with("/stop") {
                self.stops.lock().await.push(body.to_owned());
                return Ok(serde_json::json!({ "abandoned": true }).to_string());
            }
            self.page_offers.fetch_add(1, Ordering::SeqCst);
            if self.refuse_pages.load(Ordering::SeqCst) {
                // The bytes reached the server and the server could not keep
                // them. A five-hundred rather than a refusal of this device:
                // the page is still owed, the fence is still this attempt's,
                // and nothing the device can read says how long it will last.
                return Err(ControlPlaneError::Refused(
                    "500: the page could not be stored".to_owned(),
                ));
            }
            let page: ImportPage =
                serde_json::from_str(body).expect("the page is well-formed json");
            let described = u32::try_from(page.resources.len()).unwrap_or(u32::MAX);
            let skipped = u32::try_from(page.skipped.len()).unwrap_or(u32::MAX);
            let complete = page.complete;
            self.posted.lock().await.push(page);
            if self.gibberish {
                return Ok("<html><body>Gateway</body></html>".to_owned());
            }
            // The route's own `ImportAck`, whole. The device requires every
            // field of it, so a fixture answering a partial object would
            // prove the validation rather than the behaviour under test —
            // and there is a case that does exactly that, deliberately, in
            // `only_the_routes_own_acknowledgement_counts_as_one`.
            Ok(serde_json::json!({
                "applied": described,
                "skipped": skipped,
                "described_total": described,
                "create_job": serde_json::Value::Null,
                "complete": complete,
            })
            .to_string())
        }
    }

    impl LedgerTransport for FakePlane {
        fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String> {
            Box::pin(async move { self.record(path, &body).await })
        }
    }

    fn gate_for(marketplaces: Vec<Marketplace>) -> Arc<Mutex<EntitlementGate>> {
        Arc::new(Mutex::new(EntitlementGate::holding(
            Entitlement::from_verified_claims(Claims {
                sub: "org-1".to_owned(),
                aud: crate::entitlement::AUDIENCE.to_owned(),
                iss: crate::entitlement::ISSUER.to_owned(),
                device: DEVICE.to_owned(),
                marketplaces,
                plan: crate::entitlement::Plan::Subscriber,
                exp: NOW_SECONDS + 3_600,
                grace: NOW_SECONDS + 3_600 + 86_400,
            }),
        )))
    }

    fn claimed(attempt: u64, phase: RunPhase, stop: StopSignal) -> super::ClaimedRun {
        super::ClaimedRun {
            run: RUN,
            source: tam_types::InventoryId::Tes,
            attempt,
            lease_expires_at: NOW.0 + 60_000,
            phase,
            stop,
            progress: super::RunProgress::default(),
        }
    }

    /// One run's ledger over a fake plane, holding the first attempt.
    fn ledger_for(plane: &Arc<FakePlane>, phase: RunPhase, stop: StopSignal) -> Arc<RunLedger> {
        ledger_keeping(plane, phase, stop, Arc::new(MemoryJournal::default()))
    }

    fn ledger_keeping(
        plane: &Arc<FakePlane>,
        phase: RunPhase,
        stop: StopSignal,
        journal: Arc<dyn ImportJournal>,
    ) -> Arc<RunLedger> {
        ledger_under(ATTEMPT, plane, phase, stop, journal)
    }

    /// The same run under a stated fence, which is what a resumed attempt
    /// holds.
    fn ledger_under(
        attempt: u64,
        plane: &Arc<FakePlane>,
        phase: RunPhase,
        stop: StopSignal,
        journal: Arc<dyn ImportJournal>,
    ) -> Arc<RunLedger> {
        let transport: Arc<dyn LedgerTransport> = Arc::<FakePlane>::clone(plane);
        Arc::new(RunLedger::new(
            DeviceId::from_raw(DEVICE),
            &claimed(attempt, phase, stop),
            transport,
            journal,
        ))
    }

    /// One page, with nothing in it but the shape.
    fn a_page(complete: bool) -> ImportPage {
        ImportPage {
            run: RUN,
            attempt: None,
            receipt: None,
            request: None,
            listed: None,
            resources: Vec::new(),
            skipped: Vec::new(),
            enumeration_complete: false,
            complete,
            failed: None,
        }
    }

    /// A journal on a disk that refuses.
    ///
    /// The failure this device must not shrug off: a page or a failure it
    /// cannot record is one a restart will not replay, so carrying on would
    /// be claiming a durability it does not have.
    struct BrokenJournal;

    impl ImportJournal for BrokenJournal {
        fn read(&self) -> super::JournalFuture<'_, super::ImportJournalState> {
            Box::pin(core::future::ready(Err(
                "the import journal could not be read".to_owned(),
            )))
        }

        fn mutate<'a>(&'a self, _change: &'a super::JournalChange) -> super::JournalFuture<'a, ()> {
            Box::pin(core::future::ready(Err(
                "the import journal could not be written".to_owned(),
            )))
        }
    }

    /// Waits, briefly, for the background run to have written down what it
    /// owes.
    async fn owed_reaching(journal: &Arc<MemoryJournal>, wanted: usize) -> Vec<super::PendingPost> {
        for _ in 0..200_u32 {
            let owed = journal.read().await.expect("the journal reads").outbox;
            if owed.len() >= wanted {
                return owed;
            }
            tokio::time::sleep(core::time::Duration::from_millis(10)).await;
        }
        journal.read().await.expect("the journal reads").outbox
    }

    fn pass(source: Scripted, plane: &Arc<FakePlane>) -> ImportPass<Scripted> {
        passing(
            source,
            plane,
            gate_for(vec![Marketplace::Tes]),
            StopSignal::never(),
        )
    }

    fn passing(
        source: Scripted,
        plane: &Arc<FakePlane>,
        gate: Arc<Mutex<EntitlementGate>>,
        stop: StopSignal,
    ) -> ImportPass<Scripted> {
        ImportPass::new(
            source,
            ledger_for(plane, RunPhase::Describe, stop.clone()),
            SourcePermission {
                source: tam_types::InventoryId::Tes,
                gate,
                stop,
            },
        )
    }

    /// The whole pass, and the property that D27 turns on.
    ///
    /// The assertion that matters is the last one: what reaches the control
    /// plane describes the seller's file and does not contain it. That is
    /// enforced by `ObservedResource` having no field the bytes could occupy,
    /// so this is the type being checked rather than the code being trusted.
    #[tokio::test]
    async fn a_catalogue_is_described_to_the_server_and_its_bytes_are_not() {
        let plane = Arc::new(FakePlane::default());
        let report = pass(Scripted::of(3, PDF.to_vec()), &plane)
            .run(NOW, |_| {})
            .await
            .expect("the pass completes");

        assert_eq!(report.described, 3);
        assert!(report.skipped.is_empty());

        let posted = plane.posted.lock().await.clone();
        let [listed, page] = posted.as_slice() else {
            panic!(
                "one listing and one page of three resources, and got {}",
                posted.len()
            );
        };
        assert_eq!(
            listed.listed.as_ref().map(Vec::len),
            Some(3),
            "the shop is posted before anything is read"
        );
        assert!(page.complete, "the last page says so, or nothing is minted");
        assert_eq!(page.resources.len(), 3);

        let first = &page.resources[0];
        let Some(file) = first.file.as_ref() else {
            panic!("a Tes resource names its file");
        };
        assert_eq!(file.kind, FileKind::Pdf, "probed, not declared");
        assert_eq!(file.byte_len, PDF.len() as u64);
        assert_eq!(
            file.payload_content_type.as_str(),
            content_type_for(FileKind::Pdf, PDF)
        );
        assert!(
            matches!(file.scan, ScanOutcome::Clean { .. }),
            "the device's own scan, recorded as the device's"
        );
        assert!(
            first
                .cover_png
                .as_ref()
                .is_some_and(|cover| cover.bytes().starts_with(PNG_MAGIC)),
            "a cover is derived, kept, and is the PNG the type promises"
        );
        // A sketch is measured and travels; the text layer is absent here
        // because the fixture is a PDF header and not a PDF, which is the
        // honest outcome and the one `tam-fingerprint`'s own tests pin over
        // real documents.
        assert_eq!(
            first
                .fingerprint
                .as_ref()
                .map(|print| print.title_norm.as_str()),
            Some("resource 1"),
            "the sketch the matcher reads is measured where the bytes are"
        );

        let wire = serde_json::to_string(&*posted).expect("the pages serialise");
        assert!(
            !wire.contains(&base64(PDF)) && !wire.contains("%PDF-1.7"),
            "the seller's own bytes do not appear in the page in the two encodings a reader \
             would think to look for"
        );
    }

    /// The one that actually tests D27, rather than the two encodings someone
    /// thought of.
    ///
    /// A mutation review found the previous assertion worthless: a field
    /// carrying the payload hex-encoded passed it, and so did base64 shifted
    /// by one prefix byte. Checking for known encodings can only ever catch
    /// the encodings you name.
    ///
    /// This checks the property instead. Describe the same resource twice,
    /// once with a hundred kilobytes of payload and once with four megabytes,
    /// and the two pages must serialise to exactly the same number of bytes.
    /// Any encoding of the payload, in any field, under any transformation,
    /// makes the larger page larger. It works because everything else about
    /// the two is identical by construction: same listing, same kind, and
    /// therefore the same deterministic placeholder cover, so payload size is
    /// the only thing that varies.
    #[tokio::test]
    async fn the_page_is_the_same_size_whatever_the_payload_weighs() {
        async fn page_bytes(payload: Vec<u8>) -> usize {
            let plane = Arc::new(FakePlane::default());
            pass(Scripted::of(1, payload), &plane)
                .run(NOW, |_| {})
                .await
                .expect("the pass completes");
            let posted = plane.posted.lock().await.clone();
            serde_json::to_string(&posted)
                .expect("the page serialises")
                .len()
        }

        // Both lengths are seven digits, and that is deliberate. The page
        // does carry one number that varies with the payload — `byte_len`,
        // which is the file's length and is meant to be there — so sizes
        // whose decimal representations differ in width would fail this for a
        // reason that is not a leak. Choosing 1 MB against 4 MB removes that
        // confound while leaving three megabytes of difference for any
        // encoding to show up in.
        let mut small = PDF.to_vec();
        small.resize(1_000_000, b'a');
        let mut large = PDF.to_vec();
        large.resize(4_000_000, b'a');

        let (small, large) = (page_bytes(small).await, page_bytes(large).await);
        let drift = small.abs_diff(large);
        // Not exact equality, and the reason is worth stating rather than
        // hiding behind a loose bound. The digest is thirty-two bytes rendered
        // as decimal numbers, so two different digests differ slightly in
        // width — a few bytes, bounded by the digest's own fixed size and
        // independent of how large the file was. The budget covers that and
        // nothing else: three megabytes of payload cannot hide in it under any
        // encoding, since even the densest would add megabytes.
        assert!(
            drift <= 256,
            "a page describing a four-megabyte file weighs {large} and one describing a \
             one-megabyte file weighs {small}; a difference of {drift} is far more than the \
             digest's own rendering can explain, so something in the page is a function of \
             the payload's content, which is the whole of what D27 forbids"
        );
    }

    /// The image content type comes from the bytes, not from the kind.
    ///
    /// `probe_kind` answers `Image` for PNG, JPEG and GIF alike, so a single
    /// answer for that arm describes two of the three wrongly — and this value
    /// is signed into the target marketplace's string-to-sign, so the wrong
    /// description ends up inside a signature.
    #[test]
    fn every_image_kind_is_described_as_itself() {
        for (magic, expected) in [
            (
                vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
                "image/png",
            ),
            (vec![0xFF, 0xD8, 0xFF, 0xE0], "image/jpeg"),
            (b"GIF89a".to_vec(), "image/gif"),
        ] {
            assert_eq!(
                content_type_for(FileKind::Image, &magic),
                expected,
                "an image is described as the image it is"
            );
        }
        assert_eq!(
            content_type_for(FileKind::Pdf, b"%PDF-1.7"),
            "application/pdf",
            "and a kind that is not an image does not consult the bytes"
        );
    }

    /// The bounded fields refuse what they are bounded against, on the way in
    /// as well as on the way out.
    #[test]
    fn a_name_or_a_cover_that_could_carry_a_file_is_refused() {
        use super::{Cover, FileName, COVER_BYTES_MAX, NAME_MAX};

        assert!(FileName::new("worksheet.pdf").is_ok());
        assert!(
            FileName::new(&"a".repeat(NAME_MAX + 1)).is_err(),
            "a name long enough to hold a file is not a name"
        );
        assert!(
            FileName::new("pack/worksheet.pdf").is_err(),
            "a separator makes it a path rather than a name"
        );

        let png = [
            vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            vec![0u8; 32],
        ]
        .concat();
        assert!(Cover::encode(&png).is_ok());
        assert!(
            Cover::encode(b"%PDF-1.7 not a cover at all").is_err(),
            "a cover is a PNG, so a payload cannot ride in the cover field"
        );
        assert!(
            Cover::encode(&vec![0u8; COVER_BYTES_MAX + 1]).is_err(),
            "and a large one is not a cover either"
        );

        // The way back in matters as much: a page arriving from anywhere but
        // this device's own encoder has proved nothing by being well-formed.
        let smuggled: Result<Cover, _> = super::base64(b"%PDF-1.7 a payload").try_into();
        assert!(
            smuggled.is_err(),
            "deserialising re-checks, or the constructor is a suggestion"
        );
    }

    /// A revocation stops the pass between resources, and what was already
    /// described is not lost.
    ///
    /// Checked per resource rather than once at the start, because an import
    /// of a large shop runs long enough for a seller to sign the machine out
    /// during it, and the next fetch after that must not happen.
    #[tokio::test]
    async fn a_revocation_part_way_through_stops_the_pass_and_keeps_what_crossed() {
        let plane = Arc::new(FakePlane::default());
        let revoked = Arc::new(AtomicBool::new(false));
        // Raised before the run, which is the same write a check-in makes
        // when it learns of a revocation mid-import.
        revoked.store(true, Ordering::SeqCst);
        let why = passing(
            Scripted::of(3, PDF.to_vec()),
            &plane,
            gate_for(vec![Marketplace::Tes]),
            StopSignal::over(Arc::clone(&revoked)),
        )
        .run(NOW, |_| {})
        .await
        .expect_err("a revoked device does not keep reading a catalogue");

        assert_eq!(
            why,
            PassError::Revoked,
            "named as itself, not as a fetch failure"
        );
        assert!(
            plane.posted.lock().await.clone().is_empty(),
            "and nothing is posted, least of all a completing page that would mint jobs"
        );
    }

    /// The entitlement for the marketplace being read has to stand, which is
    /// Q-g: fetching the seller's own file is a request to that marketplace.
    #[tokio::test]
    async fn a_lapsed_grant_for_the_source_marketplace_stops_the_pass() {
        let plane = Arc::new(FakePlane::default());
        let why = passing(
            Scripted::of(3, PDF.to_vec()),
            &plane,
            // Entitled somewhere else, which is the case a check on the wrong
            // marketplace would wave through.
            gate_for(vec![Marketplace::Tpt]),
            StopSignal::never(),
        )
        .run(NOW, |_| {})
        .await
        .expect_err("an unentitled marketplace is not read");

        assert_eq!(why, PassError::NotEntitled(Marketplace::Tes));
        assert!(plane.posted.lock().await.clone().is_empty());
    }

    /// What could not be described travels with the page.
    ///
    /// Completion is what mints the write jobs, so a completing page that said
    /// nothing about its failures would start a publish for a partial
    /// catalogue while reporting success — and the seller would find out by
    /// noticing something missing from their own shop.
    #[tokio::test]
    async fn a_page_carries_what_it_could_not_describe() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(3, PDF.to_vec());
        source.unfetchable = vec![2];
        pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("the pass completes");

        let posted = plane.posted.lock().await.clone();
        let [_listed, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        assert_eq!(page.resources.len(), 2);
        let [skipped] = page.skipped.as_slice() else {
            panic!("the skip reaches the server, and got {:?}", page.skipped);
        };
        assert_eq!(skipped.locator, Locator::from_resource_id(2));
        assert!(
            skipped.why.as_str().contains("could not be fetched"),
            "with its reason, so the console can say what did not cross: {}",
            skipped.why
        );
    }

    /// One unreadable resource costs that resource and not the migration.
    #[tokio::test]
    async fn a_resource_that_cannot_be_fetched_is_skipped_and_named() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(3, PDF.to_vec());
        source.unfetchable = vec![2];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("one bad resource does not end the pass");

        assert_eq!(report.described, 2, "the other two still crossed");
        let [skipped] = report.skipped.as_slice() else {
            panic!("one skip, and got {:?}", report.skipped);
        };
        assert_eq!(skipped.locator, Locator::from_resource_id(2));
        assert!(
            skipped.why.as_str().contains("could not be fetched"),
            "the seller is told what happened to it: {}",
            skipped.why
        );
    }

    /// A source this device holds no file capture for still crosses, and its
    /// four absences are absences rather than failures.
    #[tokio::test]
    async fn a_source_with_no_file_download_still_describes_its_listings() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(2, Vec::new());
        source.fileless = true;
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("a shop with no downloadable files is still a shop");

        assert_eq!(report.described, 2, "both listings crossed");
        assert!(report.skipped.is_empty(), "and neither was a refusal");

        let posted = plane.posted.lock().await.clone();
        let [_listing, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        let first = &page.resources[0];
        assert_eq!(
            first.file, None,
            "no bytes were held, so no file is claimed"
        );
        assert_eq!(first.cover_png, None, "and no cover was rendered from them");
        let Some(print) = first.fingerprint.as_ref() else {
            panic!("a title is still something the matcher can read");
        };
        assert_eq!(print.title_norm, "resource 1");
        assert_eq!(print.text, None, "L2 is unavailable and says so");
        assert_eq!(print.page_count, None);
        assert_eq!(print.cover_phash, None);
    }

    /// Every resource the seller ticked crosses, whatever the source's own
    /// snapshot called its state.
    ///
    /// The founder's 2026-09-16 tablet import selected four resources and
    /// imported none: three of the four answered `draft` on the Tes overlay
    /// route, the pass refused them before it asked for a file, and the run
    /// reported `selected 4, skipped 3, imported 0`. A draft overlay is an
    /// unpublished edit, not proof that there is nothing to bring across, so
    /// the state is not a reason to drop a resource the seller chose.
    #[tokio::test]
    async fn every_selected_resource_crosses_when_the_snapshot_says_draft() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(4, PDF.to_vec());
        source.drafts = vec![2, 3, 4];
        source.bundleless = vec![2, 3, 4];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("the seller's whole selection is describable");

        assert_eq!(report.described, 4, "all four of them crossed");
        assert!(
            report.skipped.is_empty(),
            "and none of them was dropped: {:?}",
            report.skipped
        );

        let posted = plane.pages().await;
        let [_listed, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        let carried: Vec<&str> = page
            .resources
            .iter()
            .map(|resource| resource.locator.as_str())
            .collect();
        assert_eq!(
            carried,
            vec!["1", "2", "3", "4"],
            "the page the server keeps carries the whole selection"
        );
        for (index, resource) in page.resources.iter().enumerate() {
            let number = index + 1;
            assert_eq!(
                resource.listing.title,
                format!("Resource {number}"),
                "each one carries its own listing rather than a placeholder"
            );
            assert_eq!(
                resource.listing.body, "Ten pages of practice.",
                "and its description, which is what makes it a usable entry"
            );
            assert!(
                resource.fingerprint.is_some(),
                "and something the matcher can read"
            );
        }
        assert!(
            page.resources[0].file.is_some(),
            "the one with a bundle still carries its file"
        );
    }

    /// A draft snapshot over a resource that does have a downloadable bundle
    /// keeps its file.
    ///
    /// The failure mode the obvious repair would introduce: reading the
    /// overlay's `draft` as "metadata only" would describe this resource
    /// truthfully and still lose the bytes, so the seller's catalogue entry
    /// would have no file and no cover for a file that was there to be had.
    #[tokio::test]
    async fn a_draft_snapshot_over_a_downloadable_bundle_keeps_its_file() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(1, PDF.to_vec());
        source.drafts = vec![1];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("a describable resource is described");

        assert_eq!(report.described, 1);
        let posted = plane.pages().await;
        let [_listed, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        let Some(file) = page.resources[0].file.as_ref() else {
            panic!("the bundle was there, so the file crosses");
        };
        assert_eq!(file.kind, FileKind::Pdf, "probed from the bytes that came");
        assert_eq!(file.byte_len, PDF.len() as u64);
        assert!(
            page.resources[0]
                .cover_png
                .as_ref()
                .is_some_and(|cover| cover.bytes().starts_with(PNG_MAGIC)),
            "and the cover derived from them"
        );
    }

    /// A resource the source confirms has no bundle crosses as its metadata,
    /// and says so rather than claiming a file.
    #[tokio::test]
    async fn a_resource_with_no_bundle_behind_it_crosses_as_its_metadata() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(2, PDF.to_vec());
        source.bundleless = vec![2];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("a confirmed absence of files is not a failure");

        assert_eq!(report.described, 2);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        let posted = plane.pages().await;
        let [_listed, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        let fileless = &page.resources[1];
        assert_eq!(
            fileless.file, None,
            "no bytes were held, so no file is claimed"
        );
        assert_eq!(fileless.cover_png, None, "and no cover was made from them");
        assert_eq!(
            fileless.listing.title, "Resource 2",
            "what it does carry is true"
        );
        assert_eq!(
            fileless
                .fingerprint
                .as_ref()
                .map(|print| print.title_norm.as_str()),
            Some("resource 2"),
            "and its title is still something the matcher can read"
        );
        assert!(
            page.resources[0].file.is_some(),
            "the resource beside it is unaffected"
        );
    }

    /// A fetch that failed is a skip the seller reads, never a resource
    /// imported as though it had no file.
    ///
    /// The one thing the repair must not buy: once an absent bundle is an
    /// ordinary outcome, an expired session or an unreadable answer would
    /// arrive as a metadata-only import — a catalogue entry with no file,
    /// silently, for a resource whose file is sitting there behind a session
    /// the seller only has to refresh.
    #[tokio::test]
    async fn a_fetch_that_failed_is_a_skip_rather_than_a_fileless_import() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(2, PDF.to_vec());
        source.unfetchable = vec![2];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("one bad resource costs that resource");

        assert_eq!(report.described, 1, "only the one that could be read");
        let [skipped] = report.skipped.as_slice() else {
            panic!("one skip, and got {:?}", report.skipped);
        };
        assert_eq!(skipped.locator, Locator::from_resource_id(2));
        assert!(
            skipped.why.as_str().contains("could not be fetched"),
            "the seller reads that the fetch failed: {}",
            skipped.why
        );

        let posted = plane.pages().await;
        let [_listed, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        assert!(
            page.resources
                .iter()
                .all(|resource| resource.locator.as_str() != "2"),
            "and the failed one is not in the page at all, least of all as an entry with no \
             file"
        );
    }

    /// A selection in which nothing has a bundle still completes.
    ///
    /// Its own case because completion is a different branch from a skip: a
    /// run where every resource failed must not complete, and a run where
    /// every resource is fileless must, or the server never mints the jobs
    /// and the seller watches an import that never ends.
    #[tokio::test]
    async fn a_selection_where_nothing_has_a_bundle_still_completes() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(3, PDF.to_vec());
        source.drafts = vec![1, 2, 3];
        source.bundleless = vec![1, 2, 3];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("three describable resources are a completed pass");

        assert_eq!(report.described, 3);
        let posted = plane.pages().await;
        let [_listed, page] = posted.as_slice() else {
            panic!("one listing and one page, and got {}", posted.len());
        };
        assert!(
            page.complete,
            "the last page says so, or nothing is minted and the run never ends"
        );
        assert_eq!(page.resources.len(), 3);
    }

    /// A bundle that opens like an archive and holds none is skipped, not
    /// described.
    ///
    /// The four bytes a ZIP opens with are the cheapest thing a truncated
    /// download, a half-written cache entry or a ranged answer keeps. This
    /// device reads the bundle to say what it is, and when what it says was
    /// "an archive" on those four bytes alone, the resource crossed as file
    /// evidence — a hash, a length, a media type and a cover — for bytes no
    /// reader can open. The seller is told it was skipped instead.
    #[tokio::test]
    async fn a_bundle_that_only_opens_like_an_archive_is_skipped_not_described() {
        let plane = Arc::new(FakePlane::default());
        let mut headless = b"PK\x03\x04".to_vec();
        headless.extend_from_slice(b"a bundle whose central directory never arrived");
        let error = pass(Scripted::of(1, headless), &plane)
            .run(NOW, |_| {})
            .await
            .expect_err("an unreadable selection must not report a successful import");
        assert!(matches!(error, PassError::Descriptions(_)));

        let posted = plane.posted.lock().await;
        assert!(posted.iter().all(|page| page.resources.is_empty()));
        let mut skipped = posted.iter().flat_map(|page| &page.skipped);
        assert_eq!(
            skipped.next().map(|row| &row.locator),
            Some(&Locator::from_resource_id(1))
        );
        assert!(skipped.next().is_none());
        drop(posted);
    }

    /// The marketplace's refusal is reported as the marketplace's, in a
    /// sentence, rather than as the control plane's.
    #[tokio::test]
    async fn a_marketplace_refusal_is_attributed_to_the_marketplace() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(2, PDF.to_vec());
        source.unfetchable = vec![1];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("the pass completes");

        let [skipped] = report.skipped.as_slice() else {
            panic!("one skip, and got {:?}", report.skipped);
        };
        assert_eq!(
            skipped.why.as_str(),
            "its file could not be fetched: Tes answered: the session expired"
        );
    }

    /// An empty catalogue still completes, and is not a failure.
    ///
    /// Both halves matter. Completing is what mints the jobs, so a pass that
    /// returned early would leave a seller watching an import that never ends.
    /// And an empty read is not an error here — it is the interface's job to
    /// refuse to render it as "no listings", because from this layer an empty
    /// shop and a shop we could not read are the same answer.
    #[tokio::test]
    async fn an_empty_catalogue_completes_rather_than_hanging_or_failing() {
        let plane = Arc::new(FakePlane::default());
        let report = pass(Scripted::of(0, PDF.to_vec()), &plane)
            .run(NOW, |_| {})
            .await
            .expect("an empty catalogue is not a failure");

        assert_eq!(report.described, 0);
        let posted = plane.posted.lock().await.clone();
        let [listing, page] = posted.as_slice() else {
            panic!(
                "one listing and one completing page, and got {}",
                posted.len()
            );
        };
        assert_eq!(
            listing.listed,
            Some(Vec::new()),
            "an empty shop says so, rather than saying nothing"
        );
        assert!(page.resources.is_empty());
        assert!(page.complete, "or the server never mints anything");
    }

    /// An ending is reported to the run as a stage, a class and a sentence.
    ///
    /// r-c5b2's O1, moved to the channel that can carry it. The old form was
    /// a completing page with a `failed` reason on it, which could say only
    /// that something went wrong: the console had to parse the sentence to
    /// know what the seller should do about it. The report carries the class
    /// beside the sentence, and carries the counts, so a run that stopped
    /// after reading two hundred resources says so rather than reading as a
    /// run that did nothing.
    ///
    /// Asserted here rather than at the command, which has its own
    /// regression: this pins the shape, and `import_early_failure_tests` in
    /// `commands.rs` pins that the real caller produces one at all.
    #[tokio::test]
    async fn an_ending_is_reported_to_the_run_with_its_class_and_its_counts() {
        let plane = Arc::new(FakePlane::default());
        let ledger = ledger_for(&plane, RunPhase::Describe, StopSignal::never());
        ledger
            .report_ending(
                &PassError::NoSession(Marketplace::Tes),
                &super::RunProgress {
                    discovered: 12,
                    processed: 5,
                    ..super::RunProgress::default()
                },
            )
            .await;

        let reported = plane.reported().await;
        let [report] = reported.as_slice() else {
            panic!("one report, and it is the ending: {reported:?}");
        };
        assert_eq!(report.attempt, ATTEMPT, "under the fence this device holds");
        assert_eq!(
            report.stage,
            ImportStage::Failed,
            "a shop this device cannot read is a failed run, not a quiet one"
        );
        assert_eq!(
            report.reason_code,
            Some(ImportReasonCode::MissingSession),
            "the class the seller acts on, rather than a sentence to parse"
        );
        assert_eq!(
            report.reason.as_ref().map(Reason::as_str),
            Some(PassError::NoSession(Marketplace::Tes).to_string().as_str()),
            "with the words the seller reads beside it"
        );
        assert_eq!(
            (report.discovered, report.processed),
            (12, 5),
            "and what it got through, so the run does not read as one that did nothing"
        );
        assert!(
            plane.pages().await.is_empty(),
            "an ending is not a page: a completing page would mint the write jobs for a \
             catalogue nobody read"
        );
    }

    /// A stop is an interruption rather than a failure.
    ///
    /// The seller asked for it, the checkpoint stands, and a run they stopped
    /// must not be presented to them as something that went wrong. A lost
    /// fence is the same shape for a different reason, which is why both
    /// answer `interrupted`.
    #[tokio::test]
    async fn a_stop_and_a_lost_fence_are_interruptions_and_a_refusal_is_a_failure() {
        for (ending, stage, code) in [
            (
                PassError::Stopped,
                ImportStage::Interrupted,
                ImportReasonCode::Stopped,
            ),
            // An outage is an interruption, not a failure. Settling the run
            // on it is what made a replay impossible: the page this device
            // still holds could never be delivered afterwards, and the seller
            // was told an import had failed when nothing about it had.
            (
                PassError::Page("the connection went away".to_owned()),
                ImportStage::Interrupted,
                ImportReasonCode::SubmissionFailed,
            ),
            // And a page the server read and refused is the other side of
            // that line, under the same class: the remedy is ours either
            // way, but an outage lifts and this answer does not, so leaving
            // the run open would offer it back to a device whose only move is
            // to rebuild the identical page.
            (
                PassError::RejectedPage("422: the page names no run".to_owned()),
                ImportStage::Failed,
                ImportReasonCode::SubmissionFailed,
            ),
            (
                PassError::Source("502".to_owned()),
                ImportStage::Interrupted,
                ImportReasonCode::SubmissionFailed,
            ),
            (
                PassError::Journal("the disk is full".to_owned()),
                ImportStage::Interrupted,
                ImportReasonCode::SubmissionFailed,
            ),
            (
                PassError::NoSession(Marketplace::Tes),
                ImportStage::Failed,
                ImportReasonCode::MissingSession,
            ),
            (
                PassError::FenceLost,
                ImportStage::Interrupted,
                ImportReasonCode::LeaseExpired,
            ),
            (
                PassError::UnsupportedSource("no device reads Etsy".to_owned()),
                ImportStage::Failed,
                ImportReasonCode::UnsupportedSource,
            ),
        ] {
            assert_eq!(ending.stage(), stage, "{ending:?} reaches the wrong stage");
            assert_eq!(
                ending.reason_code(),
                code,
                "{ending:?} names the wrong class"
            );
        }
    }

    /// A catalogue that could not be read is a failure with its own name.
    #[tokio::test]
    async fn a_catalogue_that_could_not_be_read_is_never_an_empty_one() {
        let plane = Arc::new(FakePlane::default());
        let source = Scripted {
            catalogue: Err("403".to_owned()),
            bundle: PDF.to_vec(),
            unfetchable: Vec::new(),
            drafts: Vec::new(),
            bundleless: Vec::new(),
            fileless: false,
            pause: None,
        };
        let why = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect_err("a refused catalogue is not an empty one");

        assert!(matches!(why, PassError::Catalogue(_)));
        assert!(
            plane.posted.lock().await.clone().is_empty(),
            "and nothing is recorded, least of all a completing page that would mint jobs for \
             a catalogue nobody read"
        );
    }

    /// Paging, and that the last page is the one carrying completion.
    #[tokio::test]
    async fn a_catalogue_larger_than_a_page_is_posted_in_order_and_completed_once() {
        let plane = Arc::new(FakePlane::default());
        let count = i64::try_from(PAGE_SIZE).expect("the page size fits") + 3;
        pass(Scripted::of(count, PDF.to_vec()), &plane)
            .run(NOW, |_| {})
            .await
            .expect("the pass completes");

        let all = plane.posted.lock().await.clone();
        let listed: Vec<_> = all
            .iter()
            .take_while(|page| page.listed.is_some())
            .collect();
        let expected: Vec<_> = (1..=count).map(|id| id.to_string()).collect();
        let found: Vec<_> = listed
            .iter()
            .flat_map(|page| page.listed.iter().flatten())
            .map(|item| item.locator.as_str())
            .collect();
        assert_eq!(found, expected, "all listings arrive once, in source order");
        assert_eq!(
            listed
                .iter()
                .filter(|page| page.enumeration_complete)
                .count(),
            1
        );
        assert!(listed.last().is_some_and(|page| page.enumeration_complete));

        let described = &all[listed.len()..];
        assert!(described.iter().all(|page| page.listed.is_none()));
        let read: Vec<_> = described
            .iter()
            .flat_map(|page| &page.resources)
            .map(|item| item.locator.as_str())
            .collect();
        assert_eq!(
            read, expected,
            "every description follows its listing exactly once"
        );
        assert_eq!(described.iter().filter(|page| page.complete).count(), 1);
        assert!(described.last().is_some_and(|page| page.complete));
        let receipts: std::collections::HashSet<_> =
            all.iter().filter_map(|page| page.receipt).collect();
        assert_eq!(
            receipts.len(),
            all.len(),
            "distinct payloads cannot share a receipt"
        );
    }

    /// Progress reaches the screen, and its total is known only after the
    /// enumeration.
    #[tokio::test]
    async fn progress_is_reported_before_the_first_resource_and_after_each() {
        let plane = Arc::new(FakePlane::default());
        let mut seen = Vec::new();
        pass(Scripted::of(2, PDF.to_vec()), &plane)
            .run(NOW, |progress| seen.push(progress))
            .await
            .expect("the pass completes");

        assert_eq!(seen.len(), 3, "once for the total, then once per resource");
        assert_eq!(seen[0].total, 2);
        assert_eq!(seen[0].described, 0, "the total is known before any work");
        assert_eq!(seen[2].described, 2);
    }

    /// The unwrap rule's fall-through, which is the half this crate can test.
    ///
    /// Whether an archive holds exactly one entry is
    /// `tam_pipeline::archive::sole_entry`'s question and is tested there,
    /// against real archives, with the zip writer that crate already has. What
    /// belongs here is the policy: anything the archive reader cannot reduce to
    /// a single file is uploaded exactly as it arrived, because the alternative
    /// is this device inventing a payload.
    #[test]
    fn a_thing_that_is_not_an_archive_is_the_payload_untouched() {
        let (bytes, name, entry) = payload_of(PDF.to_vec(), "9-bundle.zip");
        assert_eq!(bytes, PDF, "a thing that is not an archive is the payload");
        assert_eq!(name, "9-bundle.zip");
        assert_eq!(entry, None, "and no unwrap happened");
    }

    /// This build is new enough to be handed a marketplace-sourced item.
    ///
    /// The server will not offer one to a device below
    /// `SOURCED_PAYLOAD_MIN_VERSION`, because such a device cannot decode the
    /// manifest and would strand the lease it had just claimed. That gate is
    /// the server's; this is the half that belongs here, because a release
    /// built below the line would be refused work it is otherwise ready for
    /// and would look to a seller like a device that had simply stopped
    /// importing.
    ///
    /// Red until the release bump, and deliberately so. This is the forcing
    /// function for a number that otherwise only exists in a constant: the
    /// failure names the version this crate declares, the version the server
    /// requires, and what a release from here could not do. It is not a broken
    /// test and must not be made to pass by weakening it.
    #[test]
    fn this_build_is_at_or_past_the_version_the_server_requires() {
        let declared = env!("CARGO_PKG_VERSION");
        assert!(
            tam_domain::runs_sourced_payloads(declared),
            "this crate declares {declared}, which is below \
             tam_domain::SOURCED_PAYLOAD_MIN_VERSION at {:?}. The server will not hand a \
             marketplace-sourced item to a device at this version, so a release built from \
             here could not run a migration at all. The fix is the release bump, in both \
             Cargo.toml and tauri.conf.json, not a change to this test",
            tam_domain::SOURCED_PAYLOAD_MIN_VERSION
        );
    }

    #[test]
    fn the_import_path_names_the_device() {
        assert_eq!(
            import_path(&DeviceId::from_raw(DEVICE)),
            format!("/v1/devices/{DEVICE}/import")
        );
    }

    #[test]
    fn the_open_run_path_names_the_device() {
        assert_eq!(
            super::open_import_path(&DeviceId::from_raw(DEVICE)),
            format!("/v1/devices/{DEVICE}/import/open")
        );
    }

    #[test]
    fn the_fenced_paths_name_the_device_and_the_run() {
        let device = DeviceId::from_raw(DEVICE);
        let named = "71717171-7171-7171-7171-717171717171";
        assert_eq!(
            super::claim_path(&device, RUN),
            format!("/v1/devices/{DEVICE}/import/{named}/claim")
        );
        assert_eq!(
            super::renew_path(&device, RUN),
            format!("/v1/devices/{DEVICE}/import/{named}/renew")
        );
        assert_eq!(
            super::progress_path(&device, RUN),
            format!("/v1/devices/{DEVICE}/import/{named}/progress")
        );
        // The same family, so the stop reads like its neighbours rather than
        // introducing a second spelling of "this device's work on this run".
        assert_eq!(
            super::device_stop_path(&device, RUN),
            format!("/v1/devices/{DEVICE}/import/{named}/stop")
        );
    }

    /// The wire shape, as the server answers it.
    ///
    /// The only thing binding this device to that route is the deserialiser,
    /// and every other test here builds the value in Rust and never crosses
    /// it. A renamed field or a wrapped `null` would otherwise be found by a
    /// seller whose scheduled pull quietly stopped happening.
    ///
    /// Three shapes rather than one, because the route now answers a list and
    /// a device one version behind the server must keep reading it: an empty
    /// array and a `null` both mean nothing to do, and a run that names no
    /// owner and no cadence still reads.
    #[test]
    fn the_open_run_answer_is_read_from_the_shape_the_server_sends() {
        let absent: Option<Vec<super::OpenImportRun>> =
            serde_json::from_str("null").expect("nothing open is a value, not a failure");
        assert_eq!(absent, None);
        let empty: Vec<super::OpenImportRun> =
            serde_json::from_str("[]").expect("an empty list reads");
        assert!(empty.is_empty());

        let open: Vec<super::OpenImportRun> = serde_json::from_str(
            r#"[{"run":"71717171-7171-7171-7171-717171717171","source":"Tes","listed":true,"selected":false},
                {"run":"72727272-7272-7272-7272-727272727272","source":"Tpt","listed":false,"selected":false,"scheduled":true,"owner_device":"another-phone"}]"#,
        )
        .expect("the open runs read");
        assert_eq!(open[0], open_run(true, false));
        assert_eq!(
            (open[1].source, open[1].scheduled, open[1].listed),
            (tam_types::InventoryId::Tpt, true, false),
            "both sources are open at once, which the singleton this replaced could not say"
        );
        assert_eq!(open[1].owner_device.as_deref(), Some("another-phone"));
        assert!(
            !open[0].scheduled && open[0].owner_device.is_none(),
            "a run from a server that sends neither field reads as unowned and unscheduled \
             rather than failing to decode"
        );
    }

    /// A control plane holding the runs the device will find, counting how
    /// often it was asked.
    ///
    /// The count is an assertion of its own: the poll is cheap and the guard
    /// is the supervisor's handle, so a cycle must go on asking even after it
    /// has taken a run — or a run the seller ticks an hour later is never
    /// described.
    struct OpenRuns {
        answers: Vec<super::OpenImportRun>,
        selection: Vec<String>,
        asked: AtomicUsize,
        /// What the run itself says it has got through, which is what a
        /// device taking it over has to read rather than assume.
        reached: (u32, u32),
    }

    impl OpenRuns {
        fn answering(answers: Vec<super::OpenImportRun>) -> Arc<Self> {
            Arc::new(Self {
                answers,
                selection: Vec::new(),
                asked: AtomicUsize::new(0),
                reached: (0, 0),
            })
        }

        fn chosen(answer: super::OpenImportRun, selection: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                answers: vec![answer],
                selection: selection.iter().map(|one| (*one).to_owned()).collect(),
                asked: AtomicUsize::new(0),
                reached: (0, 0),
            })
        }

        /// A run the server says is already part way through, which is what a
        /// takeover meets.
        fn advanced(answer: super::OpenImportRun, discovered: u32, processed: u32) -> Arc<Self> {
            Arc::new(Self {
                answers: vec![answer],
                selection: Vec::new(),
                asked: AtomicUsize::new(0),
                reached: (discovered, processed),
            })
        }
    }

    impl crate::heartbeat::ControlPlane for OpenRuns {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn consent_stands(&self, _marketplace: tam_types::Marketplace) -> PlaneFuture<'_, bool> {
            Box::pin(core::future::ready(Ok(true)))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_run_facts(
            &self,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'_, crate::import::RunFacts> {
            Box::pin(core::future::ready(Ok(crate::import::RunFacts {
                source: tam_types::InventoryId::Tes,
                discovered: self.reached.0,
                processed: self.reached.1,
                described: self.reached.1,
                enumeration_complete: self.reached.0 > 0,
            })))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a DeviceId,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'a, Vec<String>> {
            let selection = self.selection.clone();
            Box::pin(core::future::ready(Ok(selection)))
        }

        fn open_import_runs<'a>(
            &'a self,
            _device: &'a DeviceId,
        ) -> PlaneFuture<'a, Vec<super::OpenImportRun>> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let answers = self.answers.clone();
            Box::pin(core::future::ready(Ok(answers)))
        }

        fn register<'a>(
            &'a self,
            _device: &'a crate::device::DeviceIdentity,
            _facts: crate::heartbeat::HostFacts,
        ) -> PlaneFuture<'a, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            _sessions: &'a [crate::heartbeat::SessionReport],
        ) -> PlaneFuture<'a, crate::heartbeat::CheckIn> {
            Box::pin(core::future::ready(Ok(crate::heartbeat::CheckIn {
                revoked: false,
                entitlement: None,
            })))
        }
    }

    fn open_run(listed: bool, selected: bool) -> super::OpenImportRun {
        super::OpenImportRun {
            run: RUN,
            source: tam_types::InventoryId::Tes,
            listed,
            selected,
            scheduled: false,
            owner_device: None,
        }
    }

    /// The other source's run, so the two can be shown to advance apart.
    const OTHER_RUN: tam_types::Uuid = tam_types::Uuid([0x72; 16]);

    fn other_run(listed: bool, selected: bool) -> super::OpenImportRun {
        super::OpenImportRun {
            run: OTHER_RUN,
            source: tam_types::InventoryId::Tpt,
            listed,
            selected,
            scheduled: true,
            owner_device: None,
        }
    }

    /// A catalogue whose walk never finishes.
    ///
    /// The reported TES state, reduced to what matters: a request that is
    /// neither answered nor refused. Before the supervisor this held the
    /// whole check-in, so a stalled TES stopped a healthy TPT from being
    /// read at all.
    struct Stalled;

    impl CatalogueSource for Stalled {
        fn list<'a>(
            &'a self,
            _found: &'a super::CatalogueProgress,
        ) -> SourceFuture<'a, Vec<ListedResource>> {
            Box::pin(core::future::pending())
        }

        fn read(&self, _resource: i64) -> SourceFuture<'_, ImportedListing> {
            Box::pin(core::future::pending())
        }

        fn bundle(&self, _resource: i64) -> SourceFuture<'_, Option<Vec<u8>>> {
            Box::pin(core::future::pending())
        }
    }

    /// A device signed in to both no-API marketplaces, entitled, and able to
    /// post pages.
    ///
    /// The entitlement is minted against the wall clock rather than [`NOW`],
    /// because the gate this path reads is the one the preflight consults at
    /// the instant the run is taken, and a claim that expired last year would
    /// refuse every cycle here.
    ///
    /// Generic over the two seams rather than fixed to the two fixtures,
    /// because one fake can be both: a server whose open-run answer depends
    /// on what this device reported has to be the same object on both sides
    /// of the wire, exactly as the real control plane is.
    async fn device_serving<L, P>(
        ledger: &Arc<L>,
        plane: &Arc<P>,
        catalogue: super::CatalogueFactory,
        journal: &Arc<MemoryJournal>,
    ) -> DesktopState
    where
        L: LedgerTransport + 'static,
        P: crate::heartbeat::ControlPlane + 'static,
    {
        let store = Arc::new(crate::session::memory::MemorySessionStore::new());
        for marketplace in [Marketplace::Tes, Marketplace::Tpt] {
            crate::session::SessionStore::put(
                store.as_ref(),
                &crate::session::SessionRecord {
                    marketplace,
                    account_label: None,
                    captured_at: crate::run::wall_now(),
                    device_id: DeviceId::from_raw(DEVICE),
                    jar: crate::session::CookieJar::new(vec![crate::session::Cookie {
                        name: "TESSession".to_owned(),
                        value: "value".to_owned(),
                    }]),
                },
            )
            .await
            .expect("the fixture store accepts");
        }
        // The claims are JWT deadlines, which are seconds, and the instant is
        // this device's own reading in milliseconds.
        let seconds = crate::run::wall_now().0.saturating_div(1_000);
        let transport: Arc<dyn LedgerTransport> = Arc::<L>::clone(ledger);
        let registry: Arc<dyn crate::heartbeat::ControlPlane> = Arc::<P>::clone(plane);
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(journal);
        let state = DesktopState::with_control_plane(
            crate::device::DeviceIdentity {
                id: DeviceId::from_raw(DEVICE),
                label: "founder-pc".to_owned(),
            },
            store,
            registry,
        )
        .with_ledger(transport)
        .with_journal(kept)
        .with_catalogue(catalogue);
        state
            .set_gate(EntitlementGate::holding(Entitlement::from_verified_claims(
                Claims {
                    sub: "org-1".to_owned(),
                    aud: crate::entitlement::AUDIENCE.to_owned(),
                    iss: crate::entitlement::ISSUER.to_owned(),
                    device: DEVICE.to_owned(),
                    marketplaces: vec![Marketplace::Tes, Marketplace::Tpt],
                    plan: crate::entitlement::Plan::Subscriber,
                    exp: seconds + 3_600,
                    grace: seconds + 3_600 + 86_400,
                },
            )))
            .await;
        state
    }

    /// A shop of two, whatever the source, for a cycle nobody pressed a
    /// button to start.
    fn two_resources() -> super::CatalogueFactory {
        Arc::new(|_ctx, _source| Ok(Box::new(Scripted::of(2, PDF.to_vec()))))
    }

    /// Tes stalls; TPT answers.
    fn one_source_stalls() -> super::CatalogueFactory {
        Arc::new(|_ctx, source| match source {
            tam_types::InventoryId::Tes => Ok(Box::new(Stalled)),
            tam_types::InventoryId::Etsy | tam_types::InventoryId::Tpt => {
                Ok(Box::new(Scripted::of(2, PDF.to_vec())))
            }
        })
    }

    /// Waits, briefly, for the background run to reach the server.
    ///
    /// The work happens in a task by design — that is the whole repair — so a
    /// test cannot read the outcome off the call that started it. Bounded at
    /// two seconds so a failure is a failure rather than a hang.
    async fn pages_reaching(plane: &Arc<FakePlane>, wanted: usize) -> Vec<ImportPage> {
        for _ in 0..200_u32 {
            let pages = plane.pages().await;
            if pages.len() >= wanted {
                return pages;
            }
            tokio::time::sleep(core::time::Duration::from_millis(10)).await;
        }
        plane.pages().await
    }

    async fn reports_reaching(plane: &Arc<FakePlane>, wanted: usize) -> Vec<ImportProgressReport> {
        for _ in 0..200_u32 {
            let reports = plane.reported().await;
            if reports.len() >= wanted {
                return reports;
            }
            tokio::time::sleep(core::time::Duration::from_millis(10)).await;
        }
        plane.reported().await
    }

    /// A second discovery pass must not reclaim work still reading the shop.
    #[tokio::test]
    async fn a_running_import_is_not_claimed_again_by_discovery() {
        let ledger = Arc::new(FakePlane::default());
        let plane = OpenRuns::answering(vec![open_run(false, false)]);
        let journal = Arc::new(MemoryJournal::default());
        let pause = Arc::new(tokio::sync::Barrier::new(2));
        let source_pause = Arc::clone(&pause);
        let sources: super::CatalogueFactory = Arc::new(move |_ctx, _source| {
            let mut source = Scripted::of(2, PDF.to_vec());
            source.pause = Some(Arc::clone(&source_pause));
            Ok(Box::new(source))
        });
        let state = device_serving(&ledger, &plane, sources, &journal).await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), pause.wait())
            .await
            .expect("the source begins reading");
        super::serve_open_runs(&state, plane.as_ref()).await;
        assert_eq!(
            ledger.claims.lock().await.len(),
            1,
            "the running import retains its fence"
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), pause.wait())
            .await
            .expect("the source finishes reading");
        let posted = pages_reaching(&ledger, 1).await;

        assert_eq!(
            posted.len(),
            1,
            "a shop enumerated twice is two rounds of marketplace requests for one run"
        );
        assert!(
            posted[0].listed.is_some() && !posted[0].complete,
            "the first half posts the listing the seller chooses from and completes nothing"
        );
        assert!(
            posted[0].enumeration_complete,
            "and says the discovery is closed, which is what the run needs to leave the \
             discovering stage"
        );
    }

    /// The selection is described, and a resumed attempt does not read what
    /// the server already has.
    ///
    /// The restart case, driven through the discovery path a restart takes:
    /// the journal carries an acknowledged locator from the attempt that
    /// died, and the resource behind it is not fetched again. Before the
    /// checkpoint, a phone that restarted mid-pass re-read the whole
    /// selection.
    #[tokio::test]
    async fn a_resumed_run_describes_only_what_the_server_has_not_acknowledged() {
        let ledger = Arc::new(FakePlane::default());
        let plane = OpenRuns::chosen(open_run(true, true), &["1", "2"]);
        // What a phone that died mid-pass left behind: one resource the
        // server acknowledged, and the page count that went with it.
        let journal = Arc::new(MemoryJournal::holding(super::ImportJournalState {
            runs: vec![super::RunCheckpoint {
                run: RUN,
                source: tam_types::InventoryId::Tes,
                attempt: ATTEMPT,
                phase: RunPhase::Describe,
                discovered: 2,
                processed: 1,
                described: 1,
                enumeration_complete: true,
                pages_posted: 1,
                acknowledged: vec!["1".to_owned()],
            }],
            outbox: Vec::new(),
            stopped: Vec::new(),
            failing: Vec::new(),
        }));
        let state = device_serving(&ledger, &plane, two_resources(), &journal).await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        let posted = pages_reaching(&ledger, 1).await;

        let [page] = posted.as_slice() else {
            panic!("one page for the one resource still owed, and got {posted:?}");
        };
        assert_eq!(
            page.resources
                .iter()
                .map(|resource| resource.locator.as_str().to_owned())
                .collect::<Vec<String>>(),
            vec!["2".to_owned()],
            "the acknowledged resource is not fetched again"
        );
        assert!(
            page.complete,
            "and the run still completes, or the server never mints the jobs"
        );
    }

    /// A stalled source does not stop the other one.
    ///
    /// The severe case from the incident: TES hangs while TPT is healthy.
    /// Before the supervisor both halves ran inside the check-in, so the
    /// stalled read held the cycle and the second source was never reached —
    /// and the seller saw two runs sitting still with one cause.
    #[tokio::test]
    async fn a_stalled_source_does_not_stop_the_other_one() {
        let ledger = Arc::new(FakePlane::default());
        let plane = OpenRuns::answering(vec![open_run(false, false), other_run(false, false)]);
        let journal = Arc::new(MemoryJournal::default());
        let state = device_serving(&ledger, &plane, one_source_stalls(), &journal).await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        let posted = pages_reaching(&ledger, 1).await;

        let named: Vec<tam_types::Uuid> = posted.iter().map(|page| page.run).collect();
        assert_eq!(
            named,
            vec![OTHER_RUN],
            "the healthy source advanced while the stalled one held nothing but its own task"
        );
    }

    /// A stop ends this device's work on one run and leaves the other alone.
    #[tokio::test]
    async fn stopping_one_run_leaves_the_other_source_reading() {
        let ledger = Arc::new(FakePlane::default());
        let plane = OpenRuns::answering(vec![open_run(false, false), other_run(false, false)]);
        let journal = Arc::new(MemoryJournal::default());
        let pause = Arc::new(tokio::sync::Barrier::new(2));
        let source_pause = Arc::clone(&pause);
        let sources: super::CatalogueFactory = Arc::new(move |_ctx, source| {
            if source == tam_types::InventoryId::Tes {
                Ok(Box::new(Stalled))
            } else {
                let mut source = Scripted::of(2, PDF.to_vec());
                source.pause = Some(Arc::clone(&source_pause));
                Ok(Box::new(source))
            }
        });
        let state = device_serving(&ledger, &plane, sources, &journal).await;
        super::serve_open_runs(&state, plane.as_ref()).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), pause.wait())
            .await
            .expect("the other source begins reading");

        let journal: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let stopped = state
            .supervisor()
            .cancel(&journal, &DeviceId::from_raw(DEVICE), RUN)
            .await;

        assert!(
            stopped.was_running && stopped.server_pending,
            "the run this device held is stopped here first, and the server is told after: \
             {stopped:?}"
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), pause.wait())
            .await
            .expect("stopping one source must not cancel the other");
        let posted = pages_reaching(&ledger, 1).await;
        assert_eq!(
            posted.iter().map(|page| page.run).collect::<Vec<_>>(),
            vec![OTHER_RUN],
            "the other source reaches the server after the stop"
        );
        let missing = state
            .supervisor()
            .cancel(
                &journal,
                &DeviceId::from_raw(DEVICE),
                tam_types::Uuid([0x99; 16]),
            )
            .await;
        assert!(
            !missing.was_running && missing.recorded && missing.server_pending,
            "a local stop records intent but cannot claim server confirmation: {missing:?}"
        );
    }

    /// A cancelled run stops between resources and posts nothing after.
    #[tokio::test]
    async fn a_cancelled_run_stops_reading_and_its_late_page_is_refused() {
        let plane = Arc::new(FakePlane::default());
        let stop = StopSignal::never();
        stop.cancel();
        let why = passing(
            Scripted::of(3, PDF.to_vec()),
            &plane,
            gate_for(vec![Marketplace::Tes]),
            stop.clone(),
        )
        .describe_all(vec![1, 2, 3], || NOW, |_| {})
        .await
        .expect_err("a stopped run does not keep reading a catalogue");

        assert_eq!(why, PassError::Stopped, "named as the seller's own act");
        assert!(
            plane.pages().await.is_empty(),
            "and nothing crosses afterwards, least of all a completing page"
        );

        // The same handle, now used to refuse a page built before the stop:
        // this is the late result the server must never see.
        let ledger = ledger_for(&plane, RunPhase::Describe, stop);
        let late = ledger
            .post_page(a_page(true), &super::RunProgress::default())
            .await;
        assert_eq!(late, Err(PassError::Stopped));
        assert!(
            plane.pages().await.is_empty(),
            "a completing page arriving after a stop would mint the write jobs for a run the \
             seller ended"
        );
    }

    /// A refused fence stops the local work and refuses everything after.
    ///
    /// The old-fence callback: this attempt was superseded — another device
    /// took the run, or the lease lapsed and the server gave it away — so
    /// nothing it still holds may be written. The conflict answer is what
    /// tells it, and it must not be read as an outage.
    #[tokio::test]
    async fn a_page_from_a_superseded_attempt_is_refused_and_stops_the_run() {
        let plane = Arc::new(FakePlane::fenced_out());
        let stop = StopSignal::never();
        let ledger = ledger_for(&plane, RunPhase::Describe, stop.clone());

        let refused = ledger
            .post_page(a_page(false), &super::RunProgress::default())
            .await;

        assert!(refused.is_err(), "a fenced page is not accepted work");
        assert_eq!(
            stop.why(),
            Some(super::StopCause::FenceLost),
            "and the refusal raises the handle, so the walk ends between resources rather \
             than reading a shop for a run this device no longer owns"
        );
        assert!(
            ledger.halted().is_some(),
            "every later post is refused locally, without asking"
        );
    }

    /// A page whose acknowledgement was lost is offered again under the new
    /// fence, and is otherwise untouched.
    ///
    /// The bytes rather than a re-description, and that is the whole point:
    /// re-reading the resource would render a fresh cover, so the same
    /// receipt would arrive with different content, which the protocol calls
    /// a conflict rather than a replay. The one field that does move is the
    /// attempt, because the queued page was built under a lease that has
    /// since lapsed and the protocol excludes the attempt from a receipt's
    /// content identity for exactly this reason.
    #[tokio::test]
    async fn a_page_whose_acknowledgement_was_lost_is_offered_again_under_the_new_fence() {
        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let offline = Arc::new(FakePlane::refusing());
        let why = ImportPass::new(
            Scripted::of(1, PDF.to_vec()),
            ledger_keeping(
                &offline,
                RunPhase::Describe,
                StopSignal::never(),
                Arc::clone(&kept),
            ),
            SourcePermission {
                source: tam_types::InventoryId::Tes,
                gate: gate_for(vec![Marketplace::Tes]),
                stop: StopSignal::never(),
            },
        )
        .describe_all(vec![1], || NOW, |_| {})
        .await
        .expect_err("a page that could not be posted is not accepted work");
        assert!(matches!(why, PassError::Page(_)));

        let owed = journal.read().await.expect("the journal reads").outbox;
        let [pending] = owed.as_slice() else {
            panic!("the page this device still owes is kept, and got {owed:?}");
        };
        assert_eq!(pending.run, RUN);
        assert_eq!(pending.path, import_path(&DeviceId::from_raw(DEVICE)));
        let queued: ImportPage =
            serde_json::from_str(&pending.body).expect("the queued page is a page");

        // The connection comes back under a later attempt, which is what a
        // resumed claim answers.
        let online = Arc::new(FakePlane::default());
        let resumed = ledger_under(
            ATTEMPT + 1,
            &online,
            RunPhase::Describe,
            StopSignal::never(),
            kept,
        );
        resumed
            .flush_outbox()
            .await
            .expect("the kept page is delivered");

        let posted = online.pages().await;
        let [replayed] = posted.as_slice() else {
            panic!("the kept page is offered again, and got {posted:?}");
        };
        assert_eq!(
            replayed.receipt, queued.receipt,
            "the same receipt, or the server counts the page a second time"
        );
        assert_eq!(
            replayed.resources, queued.resources,
            "and the same content, or the receipt arrives with something the server calls a \
             conflict rather than a replay"
        );
        assert_eq!(
            replayed.attempt,
            Some(ATTEMPT + 1),
            "under the fence this device now holds: the old one is stale and the server \
             refuses it, which would take the new attempt down with it"
        );
        assert!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .outbox
                .is_empty(),
            "and the acknowledgement clears it, so it is not offered a third time"
        );
    }

    /// A page refused because this machine was signed out is not owed again,
    /// and the walk ends rather than carrying on.
    ///
    /// The counterpart of the test above, and the distinction is the whole
    /// point of the two: an outage keeps the page, because the connection will
    /// come back and the page is the only copy of what was read. A sign-out is
    /// not an outage. Repeating the post cannot make the server accept it, so
    /// keeping it at the head of the outbox would stall every post behind it —
    /// for other runs included, because the drain stops at the first transient
    /// failure to preserve order — until the seller signed the machine back
    /// in. Nothing is lost by dropping it: the server never recorded it, so
    /// the resources stay undescribed and a later run re-reads them.
    ///
    /// The fence is surrendered in the same step, which is what stops the
    /// shop: `StopSignal` is the handle the walk reads between resources, so a
    /// revoked device makes no further marketplace request for this run.
    #[tokio::test]
    async fn a_page_refused_because_this_machine_was_signed_out_is_dropped_and_ends_the_walk() {
        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let revoked = Arc::new(FakePlane::signing_out());
        let stop = StopSignal::never();
        let why = ImportPass::new(
            Scripted::of(1, PDF.to_vec()),
            ledger_keeping(
                &revoked,
                RunPhase::Describe,
                stop.clone(),
                Arc::clone(&kept),
            ),
            SourcePermission {
                source: tam_types::InventoryId::Tes,
                gate: gate_for(vec![Marketplace::Tes]),
                stop: stop.clone(),
            },
        )
        .describe_all(vec![1], || NOW, |_| {})
        .await
        .expect_err("a signed-out machine's page is not accepted work");

        let PassError::Page(said) = &why else {
            panic!("a sign-out is reported to the run as a page failure, and got {why:?}");
        };
        assert_eq!(
            said,
            crate::heartbeat::SIGNED_OUT_HERE,
            "and it is reported in the sentence a seller reads, not as a status or a body"
        );
        assert!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .outbox
                .is_empty(),
            "the page is dropped rather than kept owed: repeating it cannot make the server \
             accept it, and keeping it would hold every later post behind a refusal that only \
             an explicit restore lifts"
        );
        assert!(
            stop.why().is_some(),
            "and the run stops rather than reading on: the walk checks this handle between \
             resources, and a device the seller signed out may make no further marketplace \
             request for the run"
        );
    }

    /// An early refusal is reported to the run even while this device is
    /// offline, and delivered when it comes back.
    ///
    /// The half of the reported incident that the local session explains: a
    /// phone with no signal and no session for the shop has two things to
    /// say, and losing either leaves the run looking like work in progress.
    #[tokio::test]
    async fn a_refusal_this_device_could_not_deliver_is_kept_and_delivered_later() {
        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let offline = Arc::new(FakePlane::refusing());
        let ledger = ledger_keeping(
            &offline,
            RunPhase::Discover,
            StopSignal::never(),
            Arc::clone(&kept),
        );

        ledger
            .report_ending(
                &PassError::NoSession(Marketplace::Tes),
                &super::RunProgress::default(),
            )
            .await;
        assert!(
            offline.reported().await.is_empty(),
            "nothing reached the server, because there was no reaching it"
        );
        assert_eq!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .outbox
                .len(),
            1,
            "so the device keeps what it owes rather than losing the refusal with the process"
        );

        let online = Arc::new(FakePlane::default());
        ledger_keeping(&online, RunPhase::Discover, StopSignal::never(), kept)
            .flush_outbox()
            .await
            .expect_err("the delivered ending closes the run");

        let reported = online.reported().await;
        let [report] = reported.as_slice() else {
            panic!("the refusal is delivered on reconnection, and got {reported:?}");
        };
        assert_eq!(report.reason_code, Some(ImportReasonCode::MissingSession));
        assert_eq!(report.stage, ImportStage::Failed);
    }

    /// Discovery reports the rows the walk has actually found, page by page.
    ///
    /// The severe case: before this, `list` answered once at the end and the
    /// run's discovery stage had nothing to show for the whole walk — a
    /// several-hundred-resource shop looked stalled for minutes. A heartbeat
    /// carrying an unchanged count is not progress either, so what is
    /// asserted is that the number the run is told is the number the walk
    /// reported.
    #[tokio::test]
    async fn discovery_reports_the_rows_the_walk_found_rather_than_a_heartbeat() {
        let plane = Arc::new(FakePlane::default());
        // Larger than a page, so the walk has something to report before it
        // finishes.
        let count = i64::try_from(PAGE_SIZE).expect("the page size fits") + 7;
        let listed = pass(Scripted::of(count, PDF.to_vec()), &plane)
            .enumerate(NOW)
            .await
            .expect("the shop is read");
        assert_eq!(
            listed.len(),
            usize::try_from(count).expect("the count fits")
        );

        // The counts the source reported are what a discovery report carries;
        // nothing here invents a denominator or a percentage, because neither
        // marketplace states a trustworthy closed total before the end.
        let walked = walked_counts(&Scripted::of(count, PDF.to_vec())).await;
        assert_eq!(
            walked.last().copied(),
            Some(u32::try_from(count).expect("the count fits")),
            "the last count is what the walk returned: {walked:?}"
        );
        assert!(
            walked.len() > 1,
            "and it is reported per page rather than once at the end: {walked:?}"
        );
        assert!(
            walked
                .windows(2)
                .all(|pair| matches!(pair, [before, after] if before < after)),
            "each report is larger than the last, so the run is seen to be advancing rather \
             than beating: {walked:?}"
        );
    }

    /// Every count one source reported while walking.
    async fn walked_counts(source: &Scripted) -> Vec<u32> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let counting = Arc::clone(&seen);
        let sink = move |found: u32| {
            // `try_lock` because this is called on the walk's own task and
            // nothing else holds the lock; a blocking lock inside a runtime
            // would be the wrong tool even where it worked.
            if let Ok(mut seen) = counting.try_lock() {
                seen.push(found);
            }
        };
        source.list(&sink).await.expect("the scripted shop reads");
        // Bound rather than returned inline: the guard is a temporary of this
        // block, and returning through it would hold a borrow of `seen` past
        // the end of its own scope.
        let counts = seen.lock().await.clone();
        counts
    }

    /// Progress reaches the run as it happens, rather than with the page.
    ///
    /// A six-item shop must not sit behind a twenty-five-item page threshold,
    /// and the console must not have to infer what a device is doing from
    /// when a page happened to land.
    #[tokio::test]
    async fn progress_reaches_the_run_before_the_page_does() {
        let plane = Arc::new(FakePlane::default());
        pass(Scripted::of(2, PDF.to_vec()), &plane)
            .describe_all(vec![1, 2], || NOW, |_| {})
            .await
            .expect("the pass completes");

        let reported = reports_reaching(&plane, 1).await;
        let last = reported
            .last()
            .expect("the run is told where the walk got to");
        assert_eq!(last.stage, ImportStage::Reading);
        assert_eq!(
            last.processed, 2,
            "counted per resource rather than per page, which is what a frozen denominator \
             is rendered against"
        );
        assert_eq!(last.attempt, ATTEMPT);
        assert!(
            last.reason_code.is_none(),
            "an ordinary step names no reason: a reason is what an ending carries"
        );
    }

    /// Two runs and a renewal task writing the journal at once lose nothing.
    ///
    /// The defect this replaces: every writer read the whole journal, changed
    /// its own part and wrote the whole thing back, so the last writer erased
    /// whatever the others had recorded in between — which is the page a
    /// restart needed and the failure the seller was owed.
    #[tokio::test]
    async fn concurrent_writers_do_not_erase_each_others_journal_entries() {
        let journal: Arc<dyn ImportJournal> = Arc::new(MemoryJournal::default());
        let write = |index: u8| {
            let journal = Arc::clone(&journal);
            async move {
                let run = tam_types::Uuid([index; 16]);
                journal
                    .mutate(&super::JournalChange::Enqueue(super::PendingPost::new(
                        run,
                        "/v1/devices/d/import",
                        &format!("{{\"attempt\":1,\"page\":{index}}}"),
                    )))
                    .await
                    .expect("the journal accepts");
            }
        };
        tokio::join!(
            write(0),
            write(1),
            write(2),
            write(3),
            write(4),
            write(5),
            write(6),
            write(7),
        );

        let owed = journal.read().await.expect("the journal reads").outbox;
        assert_eq!(
            owed.len(),
            8,
            "each writer's entry survives the others: a lost one is a page or a failure the \
             server never hears about"
        );
    }

    /// Two sources reporting the same counts do not retire each other's
    /// report.
    ///
    /// An ordinary progress report names no run, so two equal-count reports
    /// have identical bodies: keyed on the body, acknowledging one erased the
    /// other, and the run whose entry vanished was left owing nothing while
    /// having said nothing.
    #[tokio::test]
    async fn identical_report_bodies_from_two_runs_are_two_owed_posts() {
        let journal: Arc<dyn ImportJournal> = Arc::new(MemoryJournal::default());
        let body = "{\"attempt\":1,\"stage\":\"reading\",\"discovered\":2,\"processed\":2}";
        let tes = super::PendingPost::new(RUN, "/v1/devices/d/import/x/progress", body);
        let tpt = super::PendingPost::new(OTHER_RUN, "/v1/devices/d/import/y/progress", body);
        for owed in [&tes, &tpt] {
            journal
                .mutate(&super::JournalChange::Enqueue(owed.clone()))
                .await
                .expect("the journal accepts");
        }

        journal
            .mutate(&super::JournalChange::Delivered(tes.id))
            .await
            .expect("the journal accepts");

        let left = journal.read().await.expect("the journal reads").outbox;
        assert_eq!(
            left.iter().map(|owed| owed.run).collect::<Vec<_>>(),
            vec![OTHER_RUN],
            "delivering one source's report leaves the other's owed, however alike the two \
             bodies are"
        );
    }

    /// A journal this device cannot write stops the page rather than being
    /// swallowed.
    #[tokio::test]
    async fn a_page_this_device_cannot_record_is_refused_rather_than_offered() {
        let plane = Arc::new(FakePlane::default());
        let broken: Arc<dyn ImportJournal> = Arc::new(BrokenJournal);
        let ledger = ledger_keeping(&plane, RunPhase::Describe, StopSignal::never(), broken);

        let refused = ledger
            .post_page(a_page(false), &super::RunProgress::default())
            .await;

        assert!(
            matches!(refused, Err(PassError::Journal(_))),
            "the durability failure is the answer, not a log line: {refused:?}"
        );
        assert!(
            plane.pages().await.is_empty(),
            "and the page is not offered, because a page this device could not record is one a \
             restart would never replay"
        );
    }

    /// A two-hundred this device cannot read is not an acknowledgement.
    ///
    /// A proxy's HTML page, or a truncated body, arrives with the same status
    /// as success. Retiring the page on it would clear the one copy this
    /// device could replay and advance the checkpoint past work the server
    /// may never have recorded.
    #[tokio::test]
    async fn an_answer_this_device_cannot_read_leaves_the_page_owed() {
        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let plane = Arc::new(FakePlane::answering_gibberish());
        let ledger = ledger_keeping(&plane, RunPhase::Describe, StopSignal::never(), kept);

        let answered = ledger
            .post_page(a_page(true), &super::RunProgress::default())
            .await;

        assert!(
            matches!(answered, Err(PassError::Page(_))),
            "an unreadable answer is not success: {answered:?}"
        );
        assert_eq!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .outbox
                .len(),
            1,
            "and the page stays owed, so the next connection offers it again"
        );
    }

    /// Only the route's own acknowledgement retires a page.
    ///
    /// A status code cannot tell acceptance from a gateway's answer, and
    /// neither can well-formed JSON: an empty object and a refusal object are
    /// both valid JSON, and retiring a page on either clears the one copy
    /// this device could replay while advancing the checkpoint past work the
    /// server may never have recorded.
    #[test]
    fn only_the_routes_own_acknowledgement_counts_as_one() {
        let real = serde_json::json!({
            "applied": 3,
            "skipped": 1,
            "described_total": 4,
            "create_job": serde_json::Value::Null,
            "complete": false,
        })
        .to_string();
        assert!(
            super::acknowledged(&real),
            "the route's own answer is accepted, including the field this device does not read"
        );

        for pretender in [
            "{}",
            r#"{"error":"nope"}"#,
            r#"{"applied":3}"#,
            r#""ok""#,
            "[]",
            "<html><body>Gateway</body></html>",
            "",
        ] {
            assert!(
                !super::acknowledged(pretender),
                "{pretender} is not an acknowledgement, however happily it parses"
            );
        }
    }

    /// A page lost to an outage survives the ending, and the ending survives
    /// with it.
    ///
    /// The defect this replaces cleared both: a run that failed because a
    /// page could not be posted was treated as terminal, its journal entry
    /// dropped, and the page and the reason went with it — so the seller's
    /// run sat unexplained and the work had to be done again.
    #[tokio::test]
    async fn a_page_outage_keeps_both_the_page_and_the_reason_for_the_next_connection() {
        let journal = Arc::new(MemoryJournal::default());
        let plane = Arc::new(FakePlane::refusing());
        let runs = OpenRuns::chosen(open_run(true, true), &["1", "2"]);
        let state = device_serving(&plane, &runs, two_resources(), &journal).await;

        super::serve_open_runs(&state, runs.as_ref()).await;
        let owed = owed_reaching(&journal, 2).await;

        assert!(
            owed.iter().any(|post| post.path.ends_with("/import")),
            "the page this device could not post is still owed: {owed:?}"
        );
        assert!(
            owed.iter().any(|post| post.path.ends_with("/progress")),
            "and so is the reason the run stopped, which is what the seller reads: {owed:?}"
        );
        assert!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .checkpoint(RUN)
                .is_some(),
            "and the checkpoint stands, or a resumed attempt would read the shop from the \
             beginning"
        );
    }

    /// A run stopped while offline is not picked up again by a restart.
    ///
    /// The handle went with the process, the run was still open on the
    /// server, and the next discovery cycle claimed it and carried on reading
    /// the shop the seller had stopped. The stop is written down, so the
    /// cycle leaves it alone until the seller asks again.
    #[tokio::test]
    async fn a_run_stopped_on_this_device_is_not_reclaimed_by_the_next_cycle() {
        let plane = Arc::new(FakePlane::default());
        let runs = OpenRuns::answering(vec![open_run(false, false)]);
        let journal = Arc::new(MemoryJournal::default());
        let state = device_serving(&plane, &runs, two_resources(), &journal).await;
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);

        state
            .supervisor()
            .cancel(&kept, &DeviceId::from_raw(DEVICE), RUN)
            .await;
        // A restart is a fresh supervisor over the same journal.
        let restarted = device_serving(&plane, &runs, two_resources(), &journal).await;
        super::serve_open_runs(&restarted, runs.as_ref()).await;
        tokio::time::sleep(core::time::Duration::from_millis(50)).await;

        assert!(
            plane.claims.lock().await.is_empty(),
            "a stopped run is not claimed again by a cycle: the seller stopped it, and only \
             the seller starts it"
        );
        assert!(
            !restarted.supervisor().holds(RUN).await,
            "and no work is taken for it"
        );
    }

    /// A stop queued during an outage is offered while the run is still open.
    ///
    /// The open-run answer necessarily predates the post in this cycle, so
    /// successfully delivering the stop must not also reclaim that stale row.
    #[tokio::test]
    async fn an_offline_stop_is_replayed_without_reclaiming_its_still_open_run() {
        let ledger = Arc::new(FakePlane::default());
        let runs = OpenRuns::answering(vec![open_run(false, false)]);
        let stop = super::PendingPost::new(
            RUN,
            &super::device_stop_path(&DeviceId::from_raw(DEVICE), RUN),
            r#"{"attempt":1}"#,
        )
        .of_kind(super::PostKind::Stop);
        let journal = Arc::new(MemoryJournal::holding(super::ImportJournalState {
            runs: Vec::new(),
            outbox: vec![stop],
            stopped: vec![RUN],
            failing: Vec::new(),
        }));
        let state = device_serving(&ledger, &runs, two_resources(), &journal).await;

        super::serve_open_runs(&state, runs.as_ref()).await;

        assert_eq!(
            ledger.stops().await,
            vec![r#"{"attempt":1}"#.to_owned()],
            "the queued stop keeps its original fence and reaches the server"
        );
        assert!(
            ledger.claims.lock().await.is_empty(),
            "the same stale open-run snapshot is not reclaimed after its stop lands"
        );
        let kept = journal.read().await.expect("the journal reads");
        assert!(
            kept.outbox.is_empty() && !kept.was_stopped(RUN),
            "the server acknowledgement retires both the queued stop and its tombstone"
        );
    }

    /// An explicit press clears the stop the seller is overriding.
    #[tokio::test]
    async fn an_explicit_press_resumes_a_run_the_seller_had_stopped() {
        let journal: Arc<dyn ImportJournal> = Arc::new(MemoryJournal::default());
        journal
            .mutate(&super::JournalChange::Stopped(RUN))
            .await
            .expect("the journal accepts");
        let stop = super::PendingPost::new(
            RUN,
            &super::device_stop_path(&DeviceId::from_raw(DEVICE), RUN),
            r#"{"attempt":1}"#,
        )
        .of_kind(super::PostKind::Stop);
        let progress =
            super::PendingPost::new(RUN, "/v1/devices/d/import/r/progress", r#"{"attempt":1}"#);
        for owed in [stop, progress.clone()] {
            journal
                .mutate(&super::JournalChange::Enqueue(owed))
                .await
                .expect("the journal accepts");
        }

        journal
            .mutate(&super::JournalChange::Resumed(RUN))
            .await
            .expect("the journal accepts");

        let state = journal.read().await.expect("the journal reads");
        assert!(
            !state.was_stopped(RUN),
            "the seller asking for the run again is what lifts their own stop"
        );
        assert_eq!(
            state.outbox,
            vec![progress],
            "the superseded stop is retired rather than re-fenced onto the resumed attempt, \
             while the run's other owed report remains"
        );
    }

    /// A delivered page advances the checkpoint in the same change that
    /// retires it.
    ///
    /// The blocker this replaces: a queued page was dropped from the outbox
    /// on delivery while the run's checkpoint stayed where the attempt that
    /// queued it had left it. What it carried was never recorded as
    /// acknowledged, so the next attempt described those resources again and
    /// its counts sat behind the work the server held.
    #[tokio::test]
    async fn delivering_a_queued_page_advances_the_checkpoint_with_it() {
        let journal: Arc<dyn ImportJournal> = Arc::new(MemoryJournal::default());
        let earned = super::RunCheckpoint {
            run: RUN,
            source: tam_types::InventoryId::Tes,
            attempt: ATTEMPT,
            phase: RunPhase::Describe,
            discovered: 30,
            processed: 26,
            described: 25,
            enumeration_complete: true,
            pages_posted: 2,
            acknowledged: vec!["26".to_owned()],
        };
        let owed =
            super::PendingPost::new(RUN, "/v1/devices/d/import", "{}").earning(earned.clone());
        journal
            .mutate(&super::JournalChange::Enqueue(owed.clone()))
            .await
            .expect("the journal accepts");

        journal
            .mutate(&super::JournalChange::Delivered(owed.id))
            .await
            .expect("the journal accepts");

        let state = journal.read().await.expect("the journal reads");
        assert!(state.outbox.is_empty(), "the page is no longer owed");
        let kept = state.checkpoint(RUN).expect("and the run advanced with it");
        assert_eq!(
            (kept.processed, kept.described, kept.pages_posted),
            (26, 25, 2),
            "the counts the page earned are the run's now, or a resumed attempt reports fewer \
             than the server already holds — and what it described is kept apart from what it \
             merely went through"
        );
        assert_eq!(
            kept.acknowledged,
            vec!["26".to_owned()],
            "and its resources are recorded as the server's, or they are read a second time"
        );
    }

    /// A post the server never applied credits nothing.
    ///
    /// A refused sign-in and a superseded fence both retire the entry — no
    /// number of retries changes either — but neither may record its
    /// resources as acknowledged, or a resumed run would skip exactly the
    /// resources nobody has.
    #[tokio::test]
    async fn a_post_that_was_never_applied_credits_nothing() {
        let journal: Arc<dyn ImportJournal> = Arc::new(MemoryJournal::default());
        let owed = super::PendingPost::new(RUN, "/v1/devices/d/import", "{}").earning(
            super::RunCheckpoint {
                run: RUN,
                source: tam_types::InventoryId::Tes,
                attempt: ATTEMPT,
                phase: RunPhase::Describe,
                discovered: 9,
                processed: 9,
                described: 9,
                enumeration_complete: true,
                pages_posted: 1,
                acknowledged: vec!["9".to_owned()],
            },
        );
        journal
            .mutate(&super::JournalChange::Enqueue(owed.clone()))
            .await
            .expect("the journal accepts");

        journal
            .mutate(&super::JournalChange::Abandoned(owed.id))
            .await
            .expect("the journal accepts");

        let state = journal.read().await.expect("the journal reads");
        assert!(state.outbox.is_empty(), "it is not owed again");
        assert!(
            state.checkpoint(RUN).is_none(),
            "and nothing it described is recorded as the server's: {:?}",
            state.runs
        );
    }

    /// A receipt is minted per payload and kept for every replay.
    ///
    /// A receipt computed from a page's place in a walk restarts at zero on
    /// whichever device claims the run next, so two devices' first pages
    /// collide on one receipt carrying different content — a conflict on work
    /// that was never a replay.
    #[tokio::test]
    async fn a_receipt_is_minted_per_payload_and_kept_for_its_replays() {
        assert_ne!(
            super::minted_receipt(),
            super::minted_receipt(),
            "two payloads never share a receipt, whatever device or attempt built them"
        );

        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let offline = Arc::new(FakePlane::refusing());
        ledger_keeping(
            &offline,
            RunPhase::Describe,
            StopSignal::never(),
            Arc::clone(&kept),
        )
        .post_page(a_page(false), &super::RunProgress::default())
        .await
        .expect_err("the page cannot be posted while offline");
        let owed = journal.read().await.expect("the journal reads").outbox;
        let [queued] = owed.as_slice() else {
            panic!("one queued page, and got {owed:?}");
        };
        let minted: ImportPage =
            serde_json::from_str(&queued.body).expect("the queued page is a page");

        let online = Arc::new(FakePlane::default());
        ledger_under(
            ATTEMPT + 1,
            &online,
            RunPhase::Describe,
            StopSignal::never(),
            kept,
        )
        .flush_outbox()
        .await
        .expect("the queued page is delivered");

        let posted = online.pages().await;
        let [replayed] = posted.as_slice() else {
            panic!("the queued page is offered once, and got {posted:?}");
        };
        assert!(
            replayed.receipt.is_some(),
            "a page always carries one, or the server cannot recognise a replay at all"
        );
        assert_eq!(
            replayed.receipt, minted.receipt,
            "and it is the receipt minted when the page was queued, under whatever attempt \
             finally delivers it"
        );
    }

    /// Only the server saying the run is abandoned settles a stop.
    ///
    /// A two-hundred proves nothing by itself, and neither does an
    /// unfamiliar conflict: settling a seller's stop on either would have
    /// this device believe the server accepted something it never saw, and
    /// then quietly pick the run up again at the next cycle.
    #[test]
    fn only_an_abandonment_answer_settles_a_stop() {
        assert!(
            super::PostKind::Stop.proved_by(r#"{"abandoned":true}"#),
            "the route's own answer settles it"
        );
        for pretender in [
            "{}",
            r#"{"abandoned":false}"#,
            r#"{"error":"nope"}"#,
            r#""ok""#,
            "<html><body>Gateway</body></html>",
            "",
        ] {
            assert!(
                !super::PostKind::Stop.proved_by(pretender),
                "{pretender} does not settle a stop, however happily it parses"
            );
        }
        assert!(
            super::PostKind::Report.proved_by(""),
            "a progress line needs no body: the route answers none and losing one costs \
             nothing"
        );
    }

    /// A conflict settles a stop only when it says the run is settled.
    #[tokio::test]
    async fn only_a_settled_run_spends_a_stop_and_other_conflicts_leave_it_standing() {
        // The server's own shape: the code is a field on each entry, and the
        // message wording is not ours and not stable.
        let settled = r#"{"errors":[{"code":"import_run_settled","kind":"validation","message":"this import was stopped, so it creates nothing more"}]}"#;
        assert_eq!(
            super::refusal_codes(settled),
            vec!["import_run_settled".to_owned()],
            "read from the field rather than found in the sentence"
        );
        assert!(
            super::stop_is_spent(settled),
            "a settled run spends the stop"
        );

        let fenced = r#"{"errors":[{"code":"import_run_fenced","kind":"validation","message":"this import's hold has lapsed; claim it again before writing to it"}]}"#;
        assert!(
            !super::stop_is_spent(fenced),
            "a fenced abandonment does not: the run is still open, under another attempt or a \
             lapsed hold, and forgetting the seller's stop would let the next cycle here pick \
             it up as though they had never stopped it"
        );
        for unreadable in ["{}", "<html>Gateway</html>", "", r#"{"errors":[]}"#] {
            assert!(
                !super::stop_is_spent(unreadable),
                "{unreadable} settles nothing: an unrecognised refusal is not proof"
            );
        }

        // What dropping the post does to the tombstone: nothing.
        let journal: Arc<dyn ImportJournal> = Arc::new(MemoryJournal::default());
        journal
            .mutate(&super::JournalChange::Stopped(RUN))
            .await
            .expect("the journal accepts");
        let owed = super::PendingPost::new(
            RUN,
            &super::device_stop_path(&DeviceId::from_raw(DEVICE), RUN),
            r#"{"attempt":1}"#,
        )
        .of_kind(super::PostKind::Stop);
        journal
            .mutate(&super::JournalChange::Enqueue(owed.clone()))
            .await
            .expect("the journal accepts");

        journal
            .mutate(&super::JournalChange::Abandoned(owed.id))
            .await
            .expect("the journal accepts");

        let state = journal.read().await.expect("the journal reads");
        assert!(state.outbox.is_empty(), "the post is not offered forever");
        assert!(
            state.was_stopped(RUN),
            "and the stop stands, so no discovery cycle reclaims the run"
        );
    }

    /// Only the route's own acknowledgement proves an abandonment.
    #[test]
    fn an_abandonment_is_proved_by_the_answer_and_not_by_the_status() {
        assert!(
            super::PostKind::Stop.proved_by(r#"{"abandoned":true}"#),
            "the route's own answer proves it, and a replay answers exactly this"
        );
        for pretender in [
            "",
            "{}",
            r#"{"abandoned":false}"#,
            r#"{"errors":[{"code":"import_run_fenced"}]}"#,
            "<html><body>Gateway</body></html>",
        ] {
            assert!(
                !super::PostKind::Stop.proved_by(pretender),
                "{pretender} does not prove an abandonment, however happily it arrives with a \
                 two-hundred"
            );
        }
    }

    /// A stop for a run this device holds no fence for queues nothing.
    ///
    /// The tombstone still stands, so this device will not pick the run up
    /// again; what it must not do is queue an unfenced abandonment that a
    /// later delivery could aim at somebody else's newer attempt.
    #[tokio::test]
    async fn a_stop_without_a_fence_queues_nothing_and_still_stands() {
        let plane = Arc::new(FakePlane::default());
        let runs = OpenRuns::answering(Vec::new());
        let journal = Arc::new(MemoryJournal::default());
        let state = device_serving(&plane, &runs, two_resources(), &journal).await;
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);

        let stopped = state
            .supervisor()
            .cancel(&kept, &DeviceId::from_raw(DEVICE), RUN)
            .await;

        assert!(
            !stopped.was_running && stopped.recorded,
            "nothing was running here, and the intention was still written down: {stopped:?}"
        );
        let state = journal.read().await.expect("the journal reads");
        assert!(
            state.was_stopped(RUN),
            "the tombstone stands, so no cycle here reclaims the run"
        );
        assert!(
            state.outbox.is_empty(),
            "and nothing unfenced is queued, or a later delivery could abandon a newer \
             attempt: {:?}",
            state.outbox
        );
    }

    /// A takeover's baseline comes from the run, not from an empty journal.
    ///
    /// The phone taking a run over has recorded nothing about it, so a
    /// baseline read from its own journal alone would report a run that had
    /// described two hundred resources as having described none — and the
    /// server keeps the greatest count it has seen, so the console would show
    /// the run stalled for the whole of the work that finished it.
    #[tokio::test]
    async fn a_takeover_takes_its_counts_from_the_run_rather_than_its_own_journal() {
        let plane = Arc::new(FakePlane::default());
        // A device that has never seen this run: nothing in its journal.
        let runs = OpenRuns::advanced(open_run(true, true), 200, 120);
        let journal = Arc::new(MemoryJournal::default());
        let state = device_serving(&plane, &runs, two_resources(), &journal).await;
        let ctx = super::ImportContext::of(&state).expect("the build can import");

        let resumed = super::resume_point(&ctx, RUN, RunPhase::Describe)
            .await
            .expect("the run's own facts read");

        assert_eq!(
            (resumed.discovered, resumed.processed),
            (200, 120),
            "the run's counts are the baseline, or this attempt reports fewer than the server \
             already holds"
        );
        assert!(
            resumed.acknowledged.is_empty(),
            "and no locator is assumed done: the view carries counts rather than a list, so \
             what this device cannot know was described it reads again — which the server \
             applies once per locator"
        );
    }

    /// A resumed walk counts on from what the server already recorded.
    #[tokio::test]
    async fn a_resumed_walk_reports_cumulative_counts() {
        let plane = Arc::new(FakePlane::default());
        let journal = Arc::new(MemoryJournal::holding(super::ImportJournalState {
            runs: Vec::new(),
            outbox: Vec::new(),
            stopped: Vec::new(),
            failing: Vec::new(),
        }));
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let transport: Arc<dyn LedgerTransport> = Arc::<FakePlane>::clone(&plane);
        let ledger = Arc::new(RunLedger::new(
            DeviceId::from_raw(DEVICE),
            &super::ClaimedRun {
                progress: super::RunProgress {
                    discovered: 30,
                    processed: 25,
                    described: 25,
                    enumeration_complete: true,
                    pages_posted: 1,
                    acknowledged: Vec::new(),
                },
                ..claimed(ATTEMPT, RunPhase::Describe, StopSignal::never())
            },
            transport,
            kept,
        ));
        ImportPass::new(
            Scripted::of(5, PDF.to_vec()),
            ledger,
            SourcePermission {
                source: tam_types::InventoryId::Tes,
                gate: gate_for(vec![Marketplace::Tes]),
                stop: StopSignal::never(),
            },
        )
        .describe_all(vec![1, 2, 3, 4, 5], || NOW, |_| {})
        .await
        .expect("the resumed walk finishes");

        let last = plane
            .reported()
            .await
            .last()
            .expect("the run is told where the walk got to")
            .processed;
        assert_eq!(
            last, 30,
            "twenty-five already recorded plus the five this attempt read; reporting five \
             against a server that keeps the greatest it has seen would leave the run looking \
             stalled at twenty-five for the whole of the work that finished it"
        );
    }

    /// A stop reaches the adapter's own multi-page walk.
    ///
    /// The enumeration is several marketplace requests behind one future, so
    /// before this a stop, a revocation or a takeover had no effect until the
    /// whole walk finished or the five minutes ran out.
    #[tokio::test]
    async fn a_stop_during_the_catalogue_walk_ends_it_rather_than_waiting_it_out() {
        let plane = Arc::new(FakePlane::default());
        let stop = StopSignal::never();
        let pass = ImportPass::new(
            Stalled,
            ledger_for(&plane, RunPhase::Discover, stop.clone()),
            SourcePermission {
                source: tam_types::InventoryId::Tes,
                gate: gate_for(vec![Marketplace::Tes]),
                stop: stop.clone(),
            },
        );
        let stopping = stop.clone();
        let cancel = async move {
            tokio::time::sleep(core::time::Duration::from_millis(20)).await;
            stopping.cancel();
        };
        let (outcome, ()) = tokio::join!(pass.enumerate(NOW), cancel);
        let why = outcome.expect_err("a stopped walk does not run to the budget");

        assert_eq!(
            why,
            PassError::Stopped,
            "and it ends as the seller's own act rather than as a five-minute timeout"
        );
    }

    /// A lease answer is read from the shape the server sends.
    #[test]
    fn a_claim_answer_is_read_from_the_shape_the_server_sends() {
        let lease: ImportLease =
            serde_json::from_str(r#"{"attempt":7,"lease_expires_at":1756000060000}"#)
                .expect("the lease reads");
        assert_eq!(lease.attempt, 7);
        assert_eq!(
            lease.lease_expires_at, 1_756_000_060_000,
            "milliseconds since the epoch, which is the console's own convention"
        );
    }

    /// How many times one device may offer the same page to a server that
    /// keeps failing it.
    ///
    /// Three: the offer itself, and two more in case what failed was the
    /// connection rather than the server. Past that the evidence is that
    /// offering it again changes nothing, and it is the same shape
    /// `RENEW_EVERY` is chosen against — three consecutive failures are
    /// survivable and a fourth is a fact rather than a coincidence.
    ///
    /// A bound on this device's own re-reads rather than on the run: the
    /// queued page is kept, and `scheduler::DISCOVERY_BACKOFF` is what paces
    /// the retries once the pass reports the failure instead of hiding it in
    /// a background task.
    const BOUNDED_OFFERS: usize = 3;

    /// Waits for the run's own task to put the run down.
    ///
    /// The work is deliberately in a task, so a cycle is only one cycle once
    /// the supervisor has released the slot; without this a loop of cycles
    /// would measure the supervisor's guard rather than the reclaim.
    async fn settles(state: &DesktopState, run: tam_types::Uuid) {
        for _ in 0..400_u32 {
            if !state.supervisor().holds(run).await {
                return;
            }
            tokio::time::sleep(core::time::Duration::from_millis(10)).await;
        }
    }

    /// The reported incident, in process: four minutes of discovery cycles
    /// against a server that answers every route but the one carrying the
    /// descriptions.
    ///
    /// The run of 2026-09-16 climbed from attempt twelve to attempt
    /// fifty-three in twenty-four minutes with `processed` frozen at one of
    /// two, because nothing here is bounded: the page is refused, the run
    /// ends as an interruption, the server goes on listing it as open, and
    /// the next cycle ten seconds later claims it again — taking a fresh
    /// fence, restarting the shop, and replacing the reason the seller was
    /// reading with a run that looks busy.
    ///
    /// Three properties, because the defect is all three: the device stops
    /// reclaiming, it stops re-reading the seller's shop, and what it leaves
    /// behind is a reason rather than silence.
    #[tokio::test]
    async fn a_page_the_server_keeps_failing_pauses_the_run_rather_than_reclaiming_it_every_cycle()
    {
        let plane = Arc::new(FakePlane::refusing_pages());
        let runs = OpenRuns::chosen(open_run(true, true), &["1", "2"]);
        let journal = Arc::new(MemoryJournal::default());
        let state = device_serving(&plane, &runs, two_resources(), &journal).await;

        // Six cycles is a minute of the live cadence, and four more than any
        // bounded recovery should need.
        for _ in 0..6_u32 {
            super::serve_open_runs(&state, runs.as_ref()).await;
            settles(&state, RUN).await;
        }

        let claims = plane.claims().await;
        assert!(
            claims.len() <= BOUNDED_OFFERS,
            "six cycles over a server failing one page took {} claims; every claim supersedes \
             the last attempt's fence and starts the selection again, which is the run that \
             reached attempt fifty-three having described nothing",
            claims.len()
        );
        assert!(
            plane.page_offers() <= BOUNDED_OFFERS,
            "and the seller's shop is read a bounded number of times rather than once every \
             ten seconds for as long as the server is unwell: {} offers",
            plane.page_offers()
        );
        let reported = plane.reported().await;
        assert!(
            reported.iter().any(|report| {
                report.reason_code == Some(ImportReasonCode::SubmissionFailed)
                    && report.reason.is_some()
            }),
            "and the run is left carrying why it stopped, which is what the seller reads: a \
             reclaim that erases the reason leaves a run that looks like it is working: \
             {reported:?}"
        );
    }

    /// The other half of a pause: it has to be recoverable, and recovering
    /// must not deliver the same catalogue several times over.
    ///
    /// A guard rather than an account of the incident: this passes on the
    /// code as it stands, and the reason it does is worth writing down.
    /// `deliver_owed` offers what the run already owes before any new work,
    /// so the second cycle fails on the queued page and never reaches the
    /// walk — one page is queued under one receipt however many times the
    /// run is reclaimed. The re-reading the incident shows is therefore the
    /// claim and the fence, which is test A's subject, not duplicate pages.
    ///
    /// What this holds is the recovery side of whatever bound test A forces:
    /// a device that stops reclaiming must still deliver that page exactly
    /// once when asked, under the receipt it was queued with, and the
    /// selection must arrive. A pause that never recovers, or one that
    /// re-describes the shop into a second receipt on the way out, would
    /// fail here.
    #[tokio::test]
    async fn a_paused_run_delivers_its_queued_page_once_when_the_server_recovers() {
        let plane = Arc::new(FakePlane::refusing_pages());
        let runs = OpenRuns::chosen(open_run(true, true), &["1", "2"]);
        let journal = Arc::new(MemoryJournal::default());
        let state = device_serving(&plane, &runs, two_resources(), &journal).await;

        for _ in 0..6_u32 {
            super::serve_open_runs(&state, runs.as_ref()).await;
            settles(&state, RUN).await;
        }
        plane.heals();

        let ctx = super::ImportContext::of(&state).expect("the build can import");
        state
            .supervisor()
            .accept(
                &ctx,
                super::RunOrder {
                    run: RUN,
                    source: None,
                    phase: RunPhase::Describe,
                    intent: super::StartIntent::Pressed { takeover: false },
                },
            )
            .await
            .expect("an explicit press resumes a run this device paused");
        settles(&state, RUN).await;

        let delivered = plane.pages().await;
        let described: Vec<String> = delivered
            .iter()
            .flat_map(|page| page.resources.iter())
            .map(|resource| resource.locator.as_str().to_owned())
            .collect();
        let mut once = described.clone();
        once.sort();
        once.dedup();
        assert_eq!(
            described.len(),
            once.len(),
            "recovery delivers each resource once; {} descriptions of {} resources is the \
             shop arriving several times over, one page per failed cycle, each under its own \
             receipt so the server reads them as distinct work rather than as replays",
            described.len(),
            once.len()
        );
        assert_eq!(
            once.len(),
            2,
            "and the selection does arrive: a pause that never recovers is the outage made \
             permanent"
        );
    }

    /// A page the server kept starts the count again.
    ///
    /// The transition neither test above reaches, and the one that decides
    /// whether the bound is a streak or a lifetime total: a run interrupted
    /// twice by an outage this morning and twice more this afternoon would
    /// stop being reclaimed at all, though the server had accepted a page
    /// between them and nothing about the run was unwell.
    ///
    /// Read through the journal's own reader, because that is what a
    /// discovery cycle asks before it claims.
    #[tokio::test]
    async fn an_accepted_page_starts_the_outage_count_again() {
        let plane = Arc::new(FakePlane::refusing_pages());
        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let ledger = ledger_keeping(&plane, RunPhase::Describe, StopSignal::never(), kept);

        ledger
            .post_page(a_page(false), &super::RunProgress::default())
            .await
            .expect_err("a page the server would not store is not a delivered page");
        assert_eq!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .failures(RUN),
            1,
            "the offer that failed is counted, or nothing bounds the reclaims"
        );

        plane.heals();
        ledger
            .flush_outbox()
            .await
            .expect("the queued page reaches a server that came back");

        let state = journal.read().await.expect("the journal reads");
        assert_eq!(
            state.failures(RUN),
            0,
            "and the page the server kept clears what came before it: a count that only ever \
             grew would pause a healthy run on the strength of two old outages"
        );
        assert!(!state.paused(RUN), "so the next cycle may take the run");
    }

    /// A shop one row over a page, so the walk posts two listing pages and
    /// the second is the one the server reads and refuses.
    const OVER_ONE_PAGE: i64 = 26;

    /// A catalogue that counts how often the seller's shop was walked.
    ///
    /// The count is the seller's own cost of a page this device rebuilds: a
    /// re-offer is a fresh enumeration of their shop against a marketplace
    /// that rate-limits, for an answer the server has already given.
    struct Counted {
        shop: Scripted,
        walks: Arc<AtomicUsize>,
    }

    impl CatalogueSource for Counted {
        fn list<'a>(
            &'a self,
            found: &'a super::CatalogueProgress,
        ) -> SourceFuture<'a, Vec<ListedResource>> {
            self.walks.fetch_add(1, Ordering::SeqCst);
            self.shop.list(found)
        }

        fn read(&self, resource: i64) -> SourceFuture<'_, ImportedListing> {
            self.shop.read(resource)
        }

        fn bundle(&self, resource: i64) -> SourceFuture<'_, Option<Vec<u8>>> {
            self.shop.bundle(resource)
        }
    }

    fn counted_shop(count: i64, walks: &Arc<AtomicUsize>) -> super::CatalogueFactory {
        let counting = Arc::clone(walks);
        Arc::new(move |_ctx, _source| {
            let shop: Box<dyn CatalogueSource> = Box::new(Counted {
                shop: Scripted::of(count, PDF.to_vec()),
                walks: Arc::clone(&counting),
            });
            Ok(shop)
        })
    }

    /// The server as it answers a page it has read and will not take, and as
    /// it answers everything after that.
    ///
    /// Both seams in one object, which is the point of it: an open-run list
    /// that goes on offering a run whatever the device reports cannot tell a
    /// device that settled the run from one that rebuilt the refused page six
    /// times, because it answers both the same. This one closes the run's
    /// record when the device reports a terminal stage for it, exactly as the
    /// run's own record does, and keeps offering it while the device reports
    /// an interruption.
    #[derive(Default)]
    struct Refusing {
        /// How many times the last listing page — the one carrying
        /// `enumeration_complete` — was offered and refused the way a
        /// four-hundred, a four-one-three or a four-two-two is: the bytes
        /// reached the server, which read them and will not keep them.
        refusals: AtomicUsize,
        /// The pages the server did keep, which is the work the seller must
        /// not lose to the page that followed it.
        kept: Mutex<Vec<ImportPage>>,
        reports: Mutex<Vec<ImportProgressReport>>,
        claims: AtomicUsize,
        attempts: AtomicUsize,
        /// Whether the run's record is closed, which is what a terminal
        /// report from the device does to it.
        settled: AtomicBool,
        /// Whether the post that closes the run's record is lost on its way,
        /// which is the case the ending has to survive: the page is refused,
        /// the run is over, and the one post that says so cannot be
        /// delivered yet.
        ///
        /// The terminal report only, rather than the whole route. A route
        /// that was away for every progress line would hold the page queue
        /// behind it — one queue, delivered in order — so the page would
        /// never be offered and there would be no rejection to end the run
        /// with. This isolates the post whose delivery is in question.
        endings_away: AtomicBool,
        /// The next page offer meets an outage, so the page is queued and
        /// offered again by whatever attempt comes next.
        outage_first: AtomicBool,
        /// Every page is refused rather than only the last, which is what a
        /// queued page meets when it is re-offered.
        refuses_every_page: AtomicBool,
        /// Whether a post carrying a superseded fence is answered as one.
        ///
        /// A real server always does; the other cases here never reach it,
        /// so it is opt-in to keep them reading as the one thing they are
        /// about.
        fences_stale: AtomicBool,
    }

    impl Refusing {
        /// The same server losing the post that closes a run's record, so a
        /// terminal ending is queued rather than delivered.
        fn losing_endings() -> Self {
            Self {
                endings_away: AtomicBool::new(true),
                ..Self::default()
            }
        }

        /// The reason's route carrying endings again.
        fn carries_endings(&self) {
            self.endings_away.store(false, Ordering::SeqCst);
        }

        /// A server that loses one page to an outage and then refuses it.
        ///
        /// The order the reported defect needs: the page is queued while the
        /// server is unwell, and the attempt that offers it again is the one
        /// that learns the server will not keep it at all.
        fn refusing_after_an_outage() -> Self {
            Self {
                outage_first: AtomicBool::new(true),
                refuses_every_page: AtomicBool::new(true),
                ..Self::default()
            }
        }

        /// The ending's route away, and every stale fence answered as one.
        fn losing_endings_past_the_lease() -> Self {
            Self {
                endings_away: AtomicBool::new(true),
                fences_stale: AtomicBool::new(true),
                ..Self::default()
            }
        }

        /// The lease this device holds running out while it owes an ending:
        /// the server has moved the run's fence on, so the queued post names
        /// an attempt that no longer holds it.
        fn lease_lapses(&self) {
            self.attempts.fetch_add(1, Ordering::SeqCst);
        }

        /// Whether a post under this attempt still holds the run.
        fn fenced_out(&self, attempt: u64) -> bool {
            let held = u64::try_from(self.attempts.load(Ordering::SeqCst)).unwrap_or(u64::MAX);
            self.fences_stale.load(Ordering::SeqCst) && attempt < held
        }

        async fn record(&self, path: &str, body: &str) -> Result<String, ControlPlaneError> {
            if path.ends_with("/claim") {
                if self.settled.load(Ordering::SeqCst) {
                    return Err(ControlPlaneError::Fenced("import_run_settled".to_owned()));
                }
                self.claims.fetch_add(1, Ordering::SeqCst);
                let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
                return Ok(serde_json::json!({
                    "attempt": attempt,
                    "lease_expires_at": NOW.0 + 60_000,
                })
                .to_string());
            }
            if path.ends_with("/renew") {
                return Ok(serde_json::json!({
                    "attempt": self.attempts.load(Ordering::SeqCst).max(1),
                    "lease_expires_at": NOW.0 + 60_000,
                })
                .to_string());
            }
            if path.ends_with("/progress") {
                let report: ImportProgressReport =
                    serde_json::from_str(body).expect("the report is well-formed json");
                // A post under a fence the run has moved on from, which is
                // what a queued ending meets once the lease it was built
                // under has run out.
                if self.fenced_out(report.attempt) {
                    return Err(ControlPlaneError::Fenced(
                        r#"{"errors":[{"code":"import_run_fenced","kind":"validation","message":"another attempt holds this import"}]}"#
                            .to_owned(),
                    ));
                }
                if report.stage == ImportStage::Failed {
                    if self.endings_away.load(Ordering::SeqCst) {
                        // An outage rather than a refusal: the post is owed
                        // until the route carries it, which is what the
                        // outbox is for.
                        return Err(ControlPlaneError::Refused(
                            "503: the report could not be stored".to_owned(),
                        ));
                    }
                    // The run's record closes on the device's own terminal
                    // report, which is what makes the open-run answer below
                    // evidence rather than a fixture.
                    self.settled.store(true, Ordering::SeqCst);
                }
                self.reports.lock().await.push(report);
                return Ok(String::new());
            }
            let page: ImportPage =
                serde_json::from_str(body).expect("the page is well-formed json");
            if self.outage_first.swap(false, Ordering::SeqCst) {
                // The bytes never arrived, so the page is still owed and the
                // next attempt offers the identical copy.
                return Err(ControlPlaneError::Refused(
                    "500: the page could not be stored".to_owned(),
                ));
            }
            if page.enumeration_complete || self.refuses_every_page.load(Ordering::SeqCst) {
                self.refusals.fetch_add(1, Ordering::SeqCst);
                return Err(ControlPlaneError::Rejected(
                    "422: the page named a resource this run does not hold".to_owned(),
                ));
            }
            let rows = page.listed.as_ref().map_or(0, Vec::len);
            let listed = u32::try_from(rows).unwrap_or(u32::MAX);
            let described = u32::try_from(page.resources.len()).unwrap_or(u32::MAX);
            let applied = listed.max(described);
            self.kept.lock().await.push(page);
            Ok(serde_json::json!({
                "applied": applied,
                "skipped": 0,
                "described_total": applied,
                "create_job": serde_json::Value::Null,
                "complete": false,
            })
            .to_string())
        }
    }

    impl LedgerTransport for Refusing {
        fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String> {
            Box::pin(async move { self.record(path, &body).await })
        }
    }

    impl crate::heartbeat::ControlPlane for Refusing {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn consent_stands(&self, _marketplace: Marketplace) -> PlaneFuture<'_, bool> {
            Box::pin(core::future::ready(Ok(true)))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_run_facts(
            &self,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'_, crate::import::RunFacts> {
            Box::pin(core::future::ready(Ok(crate::import::RunFacts {
                source: tam_types::InventoryId::Tes,
                discovered: 0,
                processed: 0,
                described: 0,
                enumeration_complete: false,
            })))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a DeviceId,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'a, Vec<String>> {
            Box::pin(core::future::ready(Ok(Vec::new())))
        }

        fn open_import_runs<'a>(
            &'a self,
            _device: &'a DeviceId,
        ) -> PlaneFuture<'a, Vec<super::OpenImportRun>> {
            let open = if self.settled.load(Ordering::SeqCst) {
                Vec::new()
            } else {
                // A run this device has claimed is listed as owned by it,
                // which is what a later pass reads before re-fencing.
                let owner = (self.claims.load(Ordering::SeqCst) > 0).then(|| DEVICE.to_owned());
                vec![super::OpenImportRun {
                    owner_device: owner,
                    ..open_run(false, false)
                }]
            };
            Box::pin(core::future::ready(Ok(open)))
        }

        fn register<'a>(
            &'a self,
            _device: &'a crate::device::DeviceIdentity,
            _facts: crate::heartbeat::HostFacts,
        ) -> PlaneFuture<'a, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn heartbeat<'a>(
            &'a self,
            _device: &'a DeviceId,
            _sessions: &'a [crate::heartbeat::SessionReport],
        ) -> PlaneFuture<'a, crate::heartbeat::CheckIn> {
            Box::pin(core::future::ready(Ok(crate::heartbeat::CheckIn {
                revoked: false,
                entitlement: None,
            })))
        }
    }

    /// A page the server has read and refused ends the run, rather than being
    /// built again every cycle for as long as the run stays open.
    ///
    /// The neighbouring defect to the outage this module's pause bounds, and
    /// the reason that bound does not reach it: the first page of this shop is
    /// accepted, which clears the streak, and the second is refused, which
    /// starts it at one. The count never reaches the pause, the run is
    /// reported as an interruption, the server goes on offering it, and every
    /// cycle re-walks the seller's shop to rebuild the one page the server has
    /// already said it will not keep.
    ///
    /// Four properties, because the defect is all four: the shop is walked
    /// once, the refused page is offered once, the page the server did keep
    /// stands, and what the seller is left reading is a failure with a reason
    /// rather than a run that looks like it is still working.
    #[tokio::test]
    async fn a_permanent_second_page_rejection_settles_the_run() {
        let plane = Arc::new(Refusing::default());
        let journal = Arc::new(MemoryJournal::default());
        let walks = Arc::new(AtomicUsize::new(0));
        let state = device_serving(
            &plane,
            &plane,
            counted_shop(OVER_ONE_PAGE, &walks),
            &journal,
        )
        .await;

        // Six cycles is a minute of the live cadence, and four more than any
        // bounded ending should need.
        for _ in 0..6_u32 {
            super::serve_open_runs(&state, plane.as_ref()).await;
            settles(&state, RUN).await;
        }

        let reported = plane.reports.lock().await.clone();
        assert!(
            reported.iter().any(|report| {
                report.stage == ImportStage::Failed
                    && report.reason_code == Some(ImportReasonCode::SubmissionFailed)
                    && report.reason.is_some()
            }),
            "a page the server will not take leaves the seller a failure they can read and act \
             on; reported as an interruption it is a run that looks busy for as long as nobody \
             looks at it: {reported:?}"
        );
        assert_eq!(
            plane.refusals.load(Ordering::SeqCst),
            1,
            "and the refused page is offered once: the server read those bytes and will not \
             keep them, so every further offer is the same answer bought with another walk of \
             the seller's shop"
        );
        assert_eq!(
            walks.load(Ordering::SeqCst),
            1,
            "so the shop is enumerated once rather than once every cycle for as long as the \
             run stays open"
        );
        let kept = plane.kept.lock().await.clone();
        assert_eq!(
            kept.len(),
            1,
            "the page the server did keep is not offered again either, under a second receipt \
             the server would read as a second shop's worth of rows"
        );
        assert_eq!(
            kept.first()
                .and_then(|page| page.listed.as_ref())
                .map_or(0, Vec::len),
            PAGE_SIZE,
            "and it still carries the rows it carried: a refusal of the page after it must not \
             cost the seller the one the server accepted"
        );
        assert_eq!(
            plane.claims.load(Ordering::SeqCst),
            1,
            "one claim rather than one per cycle, each of which supersedes the last attempt's \
             fence and starts the shop again"
        );
    }

    /// The other half of a terminal rejection: the reason has to reach the
    /// server, and it has to reach it before the shop is walked again.
    ///
    /// The page is refused at the instant the route the reason travels on is
    /// away, so the ending is queued rather than delivered and the run stays
    /// listed open. Two things then have to hold together, and they pull
    /// against each other: the queued reason must not be dropped, and the
    /// still-open row must not be taken as an invitation to rebuild the page
    /// the server has already refused. What resolves it is the disposition
    /// this device recorded when the page was refused, plus the drain
    /// offering the one post that closes the row while the row is still open
    /// — the same rule a queued stop has always been offered under.
    #[tokio::test]
    async fn a_queued_terminal_ending_is_delivered_before_the_shop_is_walked_again() {
        let plane = Arc::new(Refusing::losing_endings());
        let journal = Arc::new(MemoryJournal::default());
        let walks = Arc::new(AtomicUsize::new(0));
        let state = device_serving(
            &plane,
            &plane,
            counted_shop(OVER_ONE_PAGE, &walks),
            &journal,
        )
        .await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        settles(&state, RUN).await;
        assert!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .owed(RUN)
                .iter()
                .any(|post| post.path.ends_with("/progress")),
            "the reason the run ended is owed rather than lost when the route it travels on \
             is away"
        );
        plane.carries_endings();

        for _ in 0..5_u32 {
            super::serve_open_runs(&state, plane.as_ref()).await;
            settles(&state, RUN).await;
        }

        let reported = plane.reports.lock().await.clone();
        assert!(
            reported.iter().any(|report| {
                report.stage == ImportStage::Failed
                    && report.reason_code == Some(ImportReasonCode::SubmissionFailed)
            }),
            "the queued ending reaches the server on the first cycle that can carry it, or the \
             seller is left reading a run that looks like it is still working: {reported:?}"
        );
        assert_eq!(
            walks.load(Ordering::SeqCst),
            1,
            "and it reaches it without the shop being walked a second time: a row the server \
             still lists as open is not an invitation to rebuild the page it refused"
        );
        assert_eq!(
            plane.refusals.load(Ordering::SeqCst),
            1,
            "so the refused page is offered once, whatever the reason's route was doing at the \
             time"
        );
        assert_eq!(
            plane.claims.load(Ordering::SeqCst),
            1,
            "and the run is claimed once: a second claim would supersede the fence and start \
             the selection again for the same answer"
        );
    }

    /// A queued page refused on the way in, before any worker exists to
    /// report it.
    ///
    /// The order is the whole of it: the page met an outage and was queued,
    /// and the attempt that offers it again meets the refusal while it is
    /// still delivering what the run owed — before the source is read, before
    /// the shop is walked, before anything is spawned. That path returns the
    /// failure to whoever asked, and nothing on it tells the run, so the
    /// device stops offering the page and the seller is left with a run that
    /// reads as though it were still going.
    #[tokio::test]
    async fn a_queued_page_rejected_before_the_worker_reports_its_failure() {
        let plane = Arc::new(Refusing::refusing_after_an_outage());
        let journal = Arc::new(MemoryJournal::default());
        let walks = Arc::new(AtomicUsize::new(0));
        let state = device_serving(
            &plane,
            &plane,
            counted_shop(OVER_ONE_PAGE, &walks),
            &journal,
        )
        .await;

        for _ in 0..6_u32 {
            super::serve_open_runs(&state, plane.as_ref()).await;
            settles(&state, RUN).await;
        }

        let reported = plane.reports.lock().await.clone();
        assert!(
            reported.iter().any(|report| {
                report.stage == ImportStage::Failed
                    && report.reason_code == Some(ImportReasonCode::SubmissionFailed)
                    && report.reason.is_some()
            }),
            "a page refused while the run's queue was being drained is still the end of the \
             run, and the seller has to be able to read that: {reported:?}"
        );
        assert_eq!(
            walks.load(Ordering::SeqCst),
            1,
            "and the shop is walked once: the attempt that met the refusal never got as far as \
             reading it"
        );
        assert!(
            plane.kept.lock().await.is_empty(),
            "nothing was kept, which is what makes this a failure rather than a partial run"
        );
        assert_eq!(
            plane.claims.load(Ordering::SeqCst),
            2,
            "two claims — the one that queued the page and the one that learned the answer — \
             rather than one per cycle"
        );
    }

    /// A press that delivers a run's own ending does not then read the shop.
    ///
    /// The ending was queued because its route was away, and the seller
    /// presses once it is back. Delivering that post closes the run's record,
    /// so the attempt holding it owns a run the server has settled: walking
    /// the shop from there posts pages against a closed run and charges the
    /// seller a second enumeration for them.
    #[tokio::test]
    async fn a_press_that_delivers_an_acknowledged_ending_does_not_read_the_shop_again() {
        let plane = Arc::new(Refusing::losing_endings());
        let journal = Arc::new(MemoryJournal::default());
        let walks = Arc::new(AtomicUsize::new(0));
        let state = device_serving(
            &plane,
            &plane,
            counted_shop(OVER_ONE_PAGE, &walks),
            &journal,
        )
        .await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        settles(&state, RUN).await;
        plane.carries_endings();

        let ctx = super::ImportContext::of(&state).expect("the build can import");
        let pressed = state
            .supervisor()
            .accept(
                &ctx,
                super::RunOrder {
                    run: RUN,
                    source: None,
                    phase: RunPhase::Discover,
                    intent: super::StartIntent::Pressed { takeover: false },
                },
            )
            .await;
        settles(&state, RUN).await;

        assert!(
            pressed.is_err(),
            "the press carries the reason to the server and then stops, rather than being told \
             work has started on a run the same post has just closed"
        );
        assert_eq!(
            walks.load(Ordering::SeqCst),
            1,
            "so the shop is read once: the press delivered a settled run's ending, which is not \
             an instruction to read the shop again"
        );
        assert_eq!(
            plane.kept.lock().await.len(),
            1,
            "and no second page is posted against a run the server has closed"
        );
        let reported = plane.reports.lock().await.clone();
        assert!(
            reported
                .iter()
                .any(|report| report.stage == ImportStage::Failed),
            "the queued reason does reach the server, which is what the press was for: \
             {reported:?}"
        );
    }

    /// A queued ending whose lease ran out while it waited.
    ///
    /// The outage outlasted the fence, so the post names an attempt the run
    /// has moved on from and the server answers it as a conflict. Dropping it
    /// there loses the only account of why the run ended, and the row stays
    /// open with nothing left that would ever explain it. A fresh fence taken
    /// for the report alone is what carries it: a claim with no takeover
    /// never displaces another owner, and nothing else about the run starts.
    #[tokio::test]
    async fn a_queued_ending_past_its_lease_is_reported_without_reading_the_shop_again() {
        let plane = Arc::new(Refusing::losing_endings_past_the_lease());
        let journal = Arc::new(MemoryJournal::default());
        let walks = Arc::new(AtomicUsize::new(0));
        let state = device_serving(
            &plane,
            &plane,
            counted_shop(OVER_ONE_PAGE, &walks),
            &journal,
        )
        .await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        settles(&state, RUN).await;
        plane.carries_endings();
        plane.lease_lapses();

        for _ in 0..3_u32 {
            super::serve_open_runs(&state, plane.as_ref()).await;
            settles(&state, RUN).await;
        }

        let reported = plane.reports.lock().await.clone();
        assert!(
            reported.iter().any(|report| {
                report.stage == ImportStage::Failed
                    && report.reason_code == Some(ImportReasonCode::SubmissionFailed)
            }),
            "the reason survives a lease it outlived: dropped on the conflict, the run is left \
             open with nothing that will ever say why it stopped: {reported:?}"
        );
        assert!(
            journal
                .read()
                .await
                .expect("the journal reads")
                .owed(RUN)
                .is_empty(),
            "and it is retired once the server has it, rather than offered for the life of the \
             process"
        );
        assert_eq!(
            walks.load(Ordering::SeqCst),
            1,
            "the fresh fence is for the report and nothing else: no worker, and no second walk \
             of the seller's shop"
        );
        assert_eq!(
            plane.kept.lock().await.len(),
            1,
            "and no further page is built under it"
        );
    }

    #[tokio::test]
    async fn an_upgrade_delivers_an_old_terminal_report_without_reading_the_shop() {
        let saved = serde_json::from_str(
            r#"{"outbox":[{
                "id":"73737373-7373-7373-7373-737373737373",
                "run":"71717171-7171-7171-7171-717171717171",
                "path":"/v1/devices/11112222333344445555666677778888/import/71717171-7171-7171-7171-717171717171/progress",
                "body":"{\"attempt\":1,\"stage\":\"failed\",\"discovered\":0,\"processed\":0,\"reason_code\":\"missing_session\",\"reason\":\"Sign in to Tes\"}",
                "kind":"report"
            }]}"#,
        )
        .expect("the journal written before terminal reports had their own kind");
        let journal = Arc::new(MemoryJournal::holding(saved));
        let plane = Arc::new(Refusing::default());
        plane.claims.store(1, Ordering::SeqCst);
        plane.attempts.store(1, Ordering::SeqCst);
        let walks = Arc::new(AtomicUsize::new(0));
        let state = device_serving(
            &plane,
            &plane,
            counted_shop(OVER_ONE_PAGE, &walks),
            &journal,
        )
        .await;

        super::serve_open_runs(&state, plane.as_ref()).await;
        settles(&state, RUN).await;

        assert!(
            plane.reports.lock().await.iter().any(|report| {
                report.stage == ImportStage::Failed
                    && report.reason_code == Some(ImportReasonCode::MissingSession)
            }),
            "the saved failure reaches the run after upgrading"
        );
        assert_eq!(
            walks.load(Ordering::SeqCst),
            0,
            "delivering an old terminal report is not a new marketplace import"
        );
    }
}
