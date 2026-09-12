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
use std::sync::Arc;

use tam_marketplace::{ImportedListing, ListingState};
use tam_types::{ContentHash, FileKind, Marketplace, ScanOutcome, Timestamp};
use tokio::sync::Mutex;

// The page vocabulary is `tam-engine-driver`'s, because `tam-api` consumes
// exactly what this pass produces and a server struct that happens to match a
// client one is a coincidence rather than a contract. Re-exported rather than
// merely imported so this module's surface is unchanged by where the
// definitions now live.
pub use tam_engine_driver::import::{
    base64, ContentType, Cover, FileName, Fingerprint, ImportPage, ListedResource, Locator,
    NotReportable, ObservedFile, ObservedResource, Reason, SkippedResource, CONTENT_TYPE_MAX,
    COVER_BYTES_MAX, LOCATOR_MAX, NAME_MAX, PNG_MAGIC, REASON_MAX,
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

/// The organisation's one open import run, as the device reads it.
///
/// `listed` and `selected` are what the run already holds rather than what
/// the device remembers doing, so a run listed by another device — or by this
/// one before it restarted — is not walked again. The device decides only
/// which half it owes from these two facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct OpenImportRun {
    pub run: tam_types::Uuid,
    /// Which shop, read from the run rather than chosen here, exactly as
    /// [`crate::heartbeat::ControlPlane::import_run_source`] is for a run the
    /// console opened.
    pub source: tam_types::InventoryId,
    /// Whether the shop has already been enumerated into this run.
    pub listed: bool,
    /// Whether a selection has been recorded for it, by the seller on the
    /// console or by the server on a scheduled run.
    pub selected: bool,
}

/// Which half of an import an unattended cycle is doing.
///
/// The per-process memory is keyed on the pair rather than on the run, because
/// the two halves happen at different cycles: a run is listed at one and
/// described at a later one, and a single "served" mark would refuse the
/// second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScheduledStep {
    List,
    Describe,
}

/// The name the console invokes and the application registers.
///
/// One constant on this side too, so the registration test names the same
/// string the console's own constant does rather than a third spelling.
pub const START_IMPORT_COMMAND: &str = "start_import";

/// The second half of the same flow, invoked once the seller has ticked.
pub const CONTINUE_IMPORT_COMMAND: &str = "continue_import";

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
    fn list(&self) -> SourceFuture<'_, Vec<ListedResource>>;

    /// One listing, verbatim, for canonicalisation.
    fn read(&self, resource: i64) -> SourceFuture<'_, ImportedListing>;

    /// The bytes of one resource's bundle, where this source hands them over
    /// at all.
    ///
    /// `Ok(None)` is a source whose own-file download this device has no
    /// capture for, which is TPT's measured state — distinct from `Err`,
    /// which is a fetch that was attempted and failed. The difference is what
    /// the seller reads: a resource whose file could not be fetched is
    /// skipped and named, and a resource from a source that has no file
    /// download at all still crosses, carrying its listing and no file.
    fn bundle(&self, resource: i64) -> SourceFuture<'_, Option<Vec<u8>>>;
}

/// So a command that picked its source at run time can hold one.
///
/// Two marketplaces are two bindings with two concrete types, and the pass is
/// generic over the seam rather than over the marketplace; boxing is what lets
/// one call site choose between them without the pass learning which
/// marketplaces exist.
impl CatalogueSource for Box<dyn CatalogueSource> {
    fn list(&self) -> SourceFuture<'_, Vec<ListedResource>> {
        (**self).list()
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
    /// The seller signed this device out while the import was running.
    Revoked,
    /// The entitlement for the marketplace being read does not stand.
    NotEntitled(Marketplace),
}

impl core::fmt::Display for PassError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Catalogue(why) => {
                write!(f, "your catalogue could not be read: {why}")
            }
            Self::Page(why) => write!(f, "a page of the import could not be recorded: {why}"),
            Self::Revoked => f.write_str("this device was signed out while the import was running"),
            Self::NotEntitled(marketplace) => write!(
                f,
                "this device's entitlement for {marketplace:?} does not stand, so it stopped \
                 reading part way through"
            ),
        }
    }
}

impl core::error::Error for PassError {}

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

/// One import of one seller's catalogue.
/// `P` is `?Sized` so the application can hand this a trait object.
///
/// The pass is generic for the tests, which drive it against a fake plane, and
/// the application has one concrete plane behind an `Arc<dyn LedgerTransport>`
/// it shares with the heartbeat. Without the relaxation the command would have
/// to name that concrete type, which would put a transport's identity into the
/// state every other caller reads.
pub struct ImportPass<S: CatalogueSource, P: LedgerTransport + ?Sized> {
    device: crate::device::DeviceId,
    source: S,
    plane: Arc<P>,
    run: tam_types::Uuid,
    permission: SourcePermission,
}

/// Whether this pass may go on reading the marketplace it is reading.
///
/// The three facts together rather than three parameters, which is also what
/// keeps them travelling as one: enumerating a catalogue and fetching a bundle
/// are marketplace requests like any other, so Q-g's rule applies to them —
/// the grant for that marketplace must stand, and the kill switch must be able
/// to stop the pass between resources rather than only between imports. Held
/// rather than read once at the start, because an import of five hundred
/// resources runs long enough for a revocation to arrive during it.
pub struct SourcePermission {
    pub marketplace: Marketplace,
    pub gate: Arc<Mutex<EntitlementGate>>,
    pub stopper: Arc<AtomicBool>,
}

impl<S: CatalogueSource, P: LedgerTransport + ?Sized> ImportPass<S, P> {
    #[must_use]
    pub const fn new(
        device: crate::device::DeviceId,
        source: S,
        plane: Arc<P>,
        run: tam_types::Uuid,
        permission: SourcePermission,
    ) -> Self {
        Self {
            device,
            source,
            plane,
            run,
            permission,
        }
    }

    /// Why this pass may not make another marketplace request, if it may not.
    ///
    /// Consulted before each one rather than once at the start. The two
    /// answers are different things the seller acts on differently: a
    /// revocation is the seller signing this machine out, and a lapsed grant
    /// is the entitlement the subscription carries.
    async fn refusal(&self, now: Timestamp) -> Option<PassError> {
        if self.permission.stopper.load(Ordering::SeqCst) {
            return Some(PassError::Revoked);
        }
        let marketplace = self.permission.marketplace;
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
        // Bounded, because the caller waits for this: the command answers only
        // once the shop is enumerated, so an unbounded walk is a button that
        // never comes back and a seller with nothing to read. The page walk is
        // several requests over a large shop and each has its own transport
        // timeout, but nothing bounded the whole of it, so a marketplace
        // answering slowly rather than not at all could hold the click open
        // indefinitely. Refused by name at the bound rather than left hanging.
        match tokio::time::timeout(ENUMERATION_BUDGET, self.source.list()).await {
            Ok(listed) => listed.map_err(|why| PassError::Catalogue(why.to_string())),
            Err(_) => Err(PassError::Catalogue(
                "reading your catalogue took longer than five minutes, so it was stopped; \
                 nothing was imported and starting again is safe"
                    .to_owned(),
            )),
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
        self.send(ImportPage {
            run: self.run,
            request: None,
            listed: Some(listed),
            resources: Vec::new(),
            skipped: Vec::new(),
            complete: false,
            failed: None,
        })
        .await
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
    pub async fn describe_all(
        &self,
        catalogue: Vec<i64>,
        clock: impl Fn() -> Timestamp + Send,
        mut progress: impl FnMut(ImportProgress) + Send,
    ) -> Result<PassReport, PassError> {
        let mut report = PassReport::default();
        let mut page: Vec<ObservedResource> = Vec::new();
        let mut skipped: Vec<SkippedResource> = Vec::new();
        progress(ImportProgress {
            total: catalogue.len(),
            described: 0,
            skipped: 0,
        });

        for (index, locator) in catalogue.iter().copied().enumerate() {
            // Between resources rather than once at the start: an import of a
            // large shop runs long enough for a revocation to arrive during
            // it, and the next fetch after one must not happen. What has
            // already been described is posted first, so the work is not lost.
            if let Some(refusal) = self.refusal(clock()).await {
                if !page.is_empty() {
                    self.post(
                        core::mem::take(&mut page),
                        core::mem::take(&mut skipped),
                        false,
                    )
                    .await?;
                    report.pages_posted += 1;
                }
                return Err(refusal);
            }
            match self.describe(locator, clock()).await {
                Ok(observed) => {
                    page.push(observed);
                    report.described += 1;
                }
                Err(why) => {
                    // `truncating` rather than a refusal: losing the tail of an
                    // adapter's sentence is better than losing the skip it
                    // explains, and the skip is what stops a partial catalogue
                    // publishing as a whole one.
                    let entry = SkippedResource {
                        locator: Locator::from_resource_id(locator),
                        why: Reason::truncating(&why),
                    };
                    skipped.push(entry.clone());
                    report.skipped.push(entry);
                }
            }
            let last = index + 1 == catalogue.len();
            if page.len() >= PAGE_SIZE || last {
                self.post(
                    core::mem::take(&mut page),
                    core::mem::take(&mut skipped),
                    last,
                )
                .await?;
                report.pages_posted += 1;
            }
            progress(ImportProgress {
                total: catalogue.len(),
                described: report.described,
                skipped: report.skipped.len(),
            });
        }

        // A catalogue that turned out to hold nothing still completes, or the
        // server would never mint the jobs and the seller would watch an
        // import that never ends.
        if catalogue.is_empty() {
            self.post(Vec::new(), core::mem::take(&mut skipped), true)
                .await?;
            report.pages_posted += 1;
        }
        Ok(report)
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
        // Skipped by its state rather than by its download failing: a draft
        // has no published bundle, and the page its manifest route answers
        // with reads as a dead session to a classifier that never sees the
        // state. The seller is told the one thing that changes it.
        if listing.state == Some(ListingState::Draft) {
            return Err(format!(
                "it is a draft on {:?}, and a draft has no published file to bring across; \
                 publish it there and import again",
                self.permission.marketplace
            ));
        }
        let Some(bundle) = self
            .source
            .bundle(locator)
            .await
            .map_err(|why| format!("its file could not be fetched: {why}"))?
        else {
            // A source this device holds no file capture for. The listing
            // still crosses — the seller's title, body, price and taxonomy
            // are the bulk of what an import is for — and the four absences
            // are what confine the matcher to L4 and L5 for it, which is
            // exactly the state §2 of the design describes for TPT.
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

        let cover = tam_pipeline::render::cover(kind, &payload)
            .map_err(|why| format!("no cover could be made from its file: {why}"))?;

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
        // The seller's bytes end here. Every field of the value above is
        // bounded or fixed-width — a digest, a validated name, a number, a
        // closed set, a checked cover — so there is nowhere in it for a
        // payload to be, and `payload` and `bundle` are dropped at this
        // return. That claim is checked rather than asserted: see
        // `the_page_is_the_same_size_whatever_the_payload_weighs`, which is
        // the test an earlier version of this comment needed and did not
        // have.
        Ok(observed)
    }

    async fn post(
        &self,
        resources: Vec<ObservedResource>,
        skipped: Vec<SkippedResource>,
        complete: bool,
    ) -> Result<(), PassError> {
        self.send(ImportPage {
            run: self.run,
            request: None,
            listed: None,
            resources,
            skipped,
            complete,
            failed: None,
        })
        .await
    }

    /// Tells the server the import stopped, and why.
    ///
    /// The seller reads the run's own page, so a failure that posted
    /// nothing has to arrive there or it arrives nowhere: the console would
    /// otherwise watch a run that never changes state. Best effort by
    /// construction — the thing that failed may be the very transport this
    /// needs — so a failure to report a failure is swallowed rather than
    /// replacing the original reason with a second one.
    pub async fn report_failure(&self, why: &PassError) {
        let page = ImportPage {
            run: self.run,
            request: None,
            listed: None,
            resources: Vec::new(),
            skipped: Vec::new(),
            complete: true,
            failed: Some(Reason::truncating(&why.to_string())),
        };
        if let Err(unreported) = self.send(page).await {
            // Deliberately not surfaced. The transport this needs may be the
            // very thing that failed, and replacing the original reason with a
            // second one about reporting it would tell the seller less.
            drop(unreported);
        }
    }

    async fn send(&self, page: ImportPage) -> Result<(), PassError> {
        let body = serde_json::to_string(&page)
            .map_err(|why| PassError::Page(format!("the page could not be encoded: {why}")))?;
        self.plane
            .post(&import_path(&self.device), body)
            .await
            .map_err(|why: ControlPlaneError| PassError::Page(why.to_string()))?;
        Ok(())
    }
}

/// How an unattended pass reaches the seller's shop.
///
/// A parameter rather than a direct call into [`crate::commands`], for the
/// reason every other seam in this module is one: the production factory
/// builds a marketplace client over the seller's stored session, so a test
/// that had to go through it could not drive this without a marketplace.
pub(crate) type CatalogueFactory<'a> = &'a (dyn Fn(&DesktopState, tam_types::InventoryId) -> Result<Box<dyn CatalogueSource>, String>
         + Send
         + Sync);

/// Does whatever the organisation's open import run is owed by this device,
/// once per check-in.
///
/// This is the half of the scheduled pull that cannot be server-side. The
/// server mints the run when the seller's cadence comes due and then waits;
/// nothing else starts it, because a server that told a device to read a shop
/// now would be the causation D1 keeps on this side of the wire. So the device
/// asks at every check-in, and a run it finds is exactly the console flow with
/// the console's press removed: enumerate and post the listing while the run
/// holds none, then describe what the selection names.
///
/// Every refusal here is silent, and that is deliberate rather than lax. No
/// open run is the ordinary answer to this question and would otherwise be an
/// hourly log line saying nothing; a shop this device holds no session for, a
/// lapsed entitlement, and a console import already running are all states the
/// seller resolves on the console, where they are already shown. What is not
/// silent is a pass that started and failed: that one is reported to the run
/// itself, because the run's own page is where the seller reads it.
pub(crate) async fn serve_open_run(
    state: &DesktopState,
    plane: &dyn crate::heartbeat::ControlPlane,
    now: Timestamp,
    catalogue: CatalogueFactory<'_>,
) {
    let Ok(Some(open)) = plane.open_import_run(&state.device().id).await else {
        return;
    };
    // A run that is listed but whose selection nobody has recorded yet is the
    // seller still choosing. There is nothing owed until they have.
    let step = if open.listed {
        if !open.selected {
            return;
        }
        ScheduledStep::Describe
    } else {
        ScheduledStep::List
    };
    if state.scheduled_step_done(open.run, step).await {
        return;
    }
    // The console's own single-flight guard rather than a second one beside
    // it: a seller who pressed Start while this cycle was polling must not
    // have their shop walked twice at once, and which of the two claimed it
    // first does not matter.
    let Ok(ready) = crate::commands::ready_to_import(state, open.run, open.source, catalogue).await
    else {
        return;
    };
    let outcome = match step {
        ScheduledStep::List => Some(list_open_run(&ready).await),
        ScheduledStep::Describe => describe_open_run(state, plane, &ready, open.run).await,
    };
    state.release_import(open.run).await;
    match outcome {
        // Marked on success rather than on the attempt, so a cycle that could
        // not reach the marketplace is retried at the next one instead of
        // leaving the run waiting until the application restarts.
        Some(Ok(())) => state.mark_scheduled_step(open.run, step).await,
        Some(Err(why)) => {
            ready.pass.report_failure(&why).await;
            state
                .record(
                    ready.marketplace,
                    now,
                    crate::state::WorkEvent::Abandoned {
                        item: uuid::Uuid::from_bytes(open.run.0)
                            .as_hyphenated()
                            .to_string(),
                        reason: why.to_string(),
                    },
                )
                .await;
        }
        // The selection could not be read, so nothing was attempted and there
        // is nothing to report to the run. The next cycle asks again.
        None => {}
    }
}

/// The first half: read the shop and post it as the run's listing.
async fn list_open_run(
    ready: &crate::commands::ImportReady<dyn LedgerTransport>,
) -> Result<(), PassError> {
    let listed = ready.pass.enumerate(ready.now).await?;
    ready.pass.post_listing(listed).await
}

/// The second half: describe what the run's own selection names.
///
/// `None` where the selection could not be read at all, which is a
/// control-plane failure rather than a pass that went wrong, and is the one
/// ending the run is told nothing about.
async fn describe_open_run(
    state: &DesktopState,
    plane: &dyn crate::heartbeat::ControlPlane,
    ready: &crate::commands::ImportReady<dyn LedgerTransport>,
    run: tam_types::Uuid,
) -> Option<Result<(), PassError>> {
    let selection = plane.import_selection(&state.device().id, run).await.ok()?;
    let chosen: Vec<i64> = selection
        .iter()
        .filter_map(|locator| locator.parse().ok())
        .collect();
    Some(
        ready
            .pass
            .describe_all(chosen, crate::run::wall_now, |_progress| {})
            .await
            .map(|_report| ()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        base64, content_type_for, import_path, payload_of, CatalogueSource, ImportPage, ImportPass,
        ListedResource, Locator, PassError, Reason, SourcePermission, PAGE_SIZE, PNG_MAGIC,
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
        /// Resources the marketplace holds as drafts, whose bundle the pass
        /// must never ask for.
        drafts: Vec<i64>,
        /// A source that hands this device no file at all, which is TPT.
        fileless: bool,
    }

    impl Scripted {
        fn of(count: i64, bundle: Vec<u8>) -> Self {
            Self {
                catalogue: Ok((1..=count).collect()),
                bundle,
                unfetchable: Vec::new(),
                drafts: Vec::new(),
                fileless: false,
            }
        }
    }

    impl CatalogueSource for Scripted {
        fn list(&self) -> SourceFuture<'_, Vec<ListedResource>> {
            Box::pin(async move {
                let ids = self.catalogue.clone().map_err(|why| tes_answered(&why))?;
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
                if self.drafts.contains(&resource) {
                    return Err(tes_answered("a draft's bundle was asked for"));
                }
                if self.unfetchable.contains(&resource) {
                    return Err(tes_answered("the session expired"));
                }
                Ok(Some(self.bundle.clone()))
            })
        }
    }

    /// A control plane that records the pages it was posted.
    #[derive(Default)]
    struct FakePlane {
        posted: Mutex<Vec<ImportPage>>,
        refuse: bool,
    }

    impl LedgerTransport for FakePlane {
        fn post<'a>(&'a self, _path: &'a str, body: String) -> PlaneFuture<'a, String> {
            Box::pin(async move {
                if self.refuse {
                    return Err(ControlPlaneError::Refused("no".to_owned()));
                }
                self.posted
                    .lock()
                    .await
                    .push(serde_json::from_str(&body).expect("the page is well-formed json"));
                Ok(String::new())
            })
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

    fn pass(source: Scripted, plane: &Arc<FakePlane>) -> ImportPass<Scripted, FakePlane> {
        passing(
            source,
            plane,
            gate_for(vec![Marketplace::Tes]),
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn passing(
        source: Scripted,
        plane: &Arc<FakePlane>,
        gate: Arc<Mutex<EntitlementGate>>,
        stopper: Arc<AtomicBool>,
    ) -> ImportPass<Scripted, FakePlane> {
        ImportPass::new(
            DeviceId::from_raw(DEVICE),
            source,
            Arc::clone(plane),
            RUN,
            SourcePermission {
                marketplace: Marketplace::Tes,
                gate,
                stopper,
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
        let stopper = Arc::new(AtomicBool::new(false));
        // Raised before the run, which is the same code path a check-in takes
        // when it learns of a revocation mid-import.
        stopper.store(true, core::sync::atomic::Ordering::SeqCst);
        let why = passing(
            Scripted::of(3, PDF.to_vec()),
            &plane,
            gate_for(vec![Marketplace::Tes]),
            Arc::clone(&stopper),
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
            Arc::new(AtomicBool::new(false)),
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
    ///
    /// TPT's own-file download is uncaptured. Before the `Option` arm a
    /// resource from it could not be described at all — every one would have
    /// become a skip, so a TPT import would have reported a shop of nothing
    /// but refusals, and the listing, price and taxonomy the seller actually
    /// wanted brought across would have stayed behind.
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

    /// A draft is skipped by its state, with the sentence that names what
    /// changes it, and its bundle is never asked for.
    ///
    /// The founder's 2026-09-07 import skipped both drafts in the shop as
    /// "the control plane refused: SessionExpired": the marketplace's answer
    /// was rendered as the control plane's, in the classifier's own
    /// vocabulary, for a resource whose only fact was that it was a draft.
    #[tokio::test]
    async fn a_draft_is_skipped_by_its_state_and_its_bundle_is_never_asked_for() {
        let plane = Arc::new(FakePlane::default());
        let mut source = Scripted::of(3, PDF.to_vec());
        source.drafts = vec![2];
        let report = pass(source, &plane)
            .run(NOW, |_| {})
            .await
            .expect("a draft costs that draft and not the migration");

        assert_eq!(report.described, 2, "the two published ones crossed");
        let [skipped] = report.skipped.as_slice() else {
            panic!("one skip, and got {:?}", report.skipped);
        };
        assert_eq!(skipped.locator, Locator::from_resource_id(2));
        assert!(
            skipped.why.as_str().contains("draft on Tes")
                && skipped.why.as_str().contains("publish it there"),
            "the seller reads what it is and what changes it: {}",
            skipped.why
        );
        assert!(
            !skipped.why.as_str().contains("could not be fetched"),
            "and no download was attempted for it: {}",
            skipped.why
        );
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

    /// A pass that stopped posts the reason, and the request renders it.
    ///
    /// r-c5b2's O1. A terminal failure that posted no resources used to leave
    /// the request holding exactly nothing, so the console watched a state
    /// that never changed and the seller was told nothing at all. The page
    /// below is the only thing that changes it, so its shape is asserted here
    /// rather than at the one call site, which is a Tauri command no test
    /// constructs.
    #[tokio::test]
    async fn a_stopped_pass_posts_the_reason_the_request_will_render() {
        let plane = Arc::new(FakePlane::default());
        pass(Scripted::of(1, PDF.to_vec()), &plane)
            .report_failure(&PassError::Revoked)
            .await;

        let posted = plane.posted.lock().await.clone();
        let [page] = posted.as_slice() else {
            panic!("one page, and it is the failure report: {posted:?}");
        };
        let expected = PassError::Revoked.to_string();
        assert_eq!(
            page.failed.as_ref().map(Reason::as_str),
            Some(expected.as_str()),
            "the sentence the request settles with is the pass's own"
        );
        assert!(
            page.complete,
            "a stopped import is over, so the server is not left holding the request open"
        );
        assert!(
            page.resources.is_empty() && page.skipped.is_empty(),
            "it reports the stop and nothing else: {page:?}"
        );
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
            fileless: false,
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
        let report = pass(Scripted::of(count, PDF.to_vec()), &plane)
            .run(NOW, |_| {})
            .await
            .expect("the pass completes");

        assert_eq!(report.pages_posted, 2);
        let all = plane.posted.lock().await.clone();
        assert_eq!(
            all.first()
                .and_then(|page| page.listed.as_ref())
                .map(Vec::len),
            Some(usize::try_from(count).expect("the count fits")),
            "the first page is the shop as the enumeration saw it, and nothing read"
        );
        let posted: Vec<&ImportPage> = all.iter().filter(|page| page.listed.is_none()).collect();
        assert_eq!(posted.len(), 2);
        assert!(!posted[0].complete, "only the last page completes");
        assert!(posted[1].complete);
        assert_eq!(posted[0].resources.len(), PAGE_SIZE);
        assert_eq!(posted[1].resources.len(), 3);
        assert!(
            all.iter()
                .all(|page| page.run == RUN && page.request.is_none()),
            "every page names the same run, or a resumed pass becomes a second import"
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

    /// The wire shape, as the server answers it.
    ///
    /// The only thing binding this device to that route is the deserialiser,
    /// and every other test here builds the value in Rust and never crosses
    /// it. A renamed field or a wrapped `null` would otherwise be found by a
    /// seller whose scheduled pull quietly stopped happening.
    #[test]
    fn the_open_run_answer_is_read_from_the_shape_the_server_sends() {
        let absent: Option<super::OpenImportRun> =
            serde_json::from_str("null").expect("no open run is a value, not a failure");
        assert_eq!(absent, None);

        let open: Option<super::OpenImportRun> = serde_json::from_str(
            r#"{"run":"71717171-7171-7171-7171-717171717171","source":"Tes","listed":true,"selected":false}"#,
        )
        .expect("the open run reads");
        assert_eq!(open, Some(open_run(true, false)));
    }

    /// A control plane holding one answer to the open-run question, counting
    /// how often it was asked.
    ///
    /// The count is an assertion of its own: the poll is cheap and the guard
    /// is over the work, so a cycle must go on asking even after it has served
    /// a run — or a run the seller ticks an hour later is never described.
    struct OpenRuns {
        answer: Option<super::OpenImportRun>,
        selection: Vec<String>,
        asked: AtomicUsize,
    }

    impl OpenRuns {
        fn answering(answer: Option<super::OpenImportRun>) -> Self {
            Self {
                answer,
                selection: Vec::new(),
                asked: AtomicUsize::new(0),
            }
        }

        fn chosen(answer: super::OpenImportRun, selection: &[&str]) -> Self {
            Self {
                selection: selection.iter().map(|one| (*one).to_owned()).collect(),
                ..Self::answering(Some(answer))
            }
        }
    }

    impl crate::heartbeat::ControlPlane for OpenRuns {
        fn reachable(&self) -> PlaneFuture<'_, ()> {
            Box::pin(core::future::ready(Ok(())))
        }

        fn sync_request_source(
            &self,
            _request: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_run_source(
            &self,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'_, tam_types::InventoryId> {
            Box::pin(core::future::ready(Ok(tam_types::InventoryId::Tes)))
        }

        fn import_selection<'a>(
            &'a self,
            _device: &'a DeviceId,
            _run: tam_types::Uuid,
        ) -> PlaneFuture<'a, Vec<String>> {
            let selection = self.selection.clone();
            Box::pin(core::future::ready(Ok(selection)))
        }

        fn open_import_run<'a>(
            &'a self,
            _device: &'a DeviceId,
        ) -> PlaneFuture<'a, Option<super::OpenImportRun>> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Box::pin(core::future::ready(Ok(self.answer)))
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
        }
    }

    /// A device signed in to Tes, entitled, and able to post pages.
    ///
    /// The entitlement is minted against the wall clock rather than [`NOW`],
    /// because the gate this path reads is the one `ready_to_import` consults
    /// at the instant the pass starts, and a claim that expired last year
    /// would refuse every cycle here.
    async fn device_serving(ledger: &Arc<FakePlane>) -> DesktopState {
        let store = Arc::new(crate::session::memory::MemorySessionStore::new());
        crate::session::SessionStore::put(
            store.as_ref(),
            &crate::session::SessionRecord {
                marketplace: Marketplace::Tes,
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
        // The claims are JWT deadlines, which are seconds, and the instant is
        // this device's own reading in milliseconds.
        let seconds = crate::run::wall_now().0.saturating_div(1_000);
        let held: Arc<FakePlane> = Arc::clone(ledger);
        let transport: Arc<dyn LedgerTransport> = held;
        let state = DesktopState::new(
            crate::device::DeviceIdentity {
                id: DeviceId::from_raw(DEVICE),
                label: "founder-pc".to_owned(),
            },
            store,
        )
        .with_ledger(transport);
        state
            .set_gate(EntitlementGate::holding(Entitlement::from_verified_claims(
                Claims {
                    sub: "org-1".to_owned(),
                    aud: crate::entitlement::AUDIENCE.to_owned(),
                    iss: crate::entitlement::ISSUER.to_owned(),
                    device: DEVICE.to_owned(),
                    marketplaces: vec![Marketplace::Tes],
                    plan: crate::entitlement::Plan::Subscriber,
                    exp: seconds + 3_600,
                    grace: seconds + 3_600 + 86_400,
                },
            )))
            .await;
        state
    }

    /// A shop of two, for a cycle nobody pressed a button to start.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the shape is the factory seam's, whose production side refuses a \
                  marketplace this device cannot enumerate"
    )]
    fn two_resources(
        _state: &DesktopState,
        _source: tam_types::InventoryId,
    ) -> Result<Box<dyn CatalogueSource>, String> {
        Ok(Box::new(Scripted::of(2, PDF.to_vec())))
    }

    /// The scheduled pull's first half, and the guard that makes it safe to
    /// ask at every check-in.
    ///
    /// Two cycles rather than one, because the server's own `listed` flag is
    /// what would otherwise stop the second — and this device must not depend
    /// on having observed it before the next hour comes round.
    #[tokio::test]
    async fn an_open_run_nobody_listed_is_walked_once_however_many_cycles_pass() {
        let ledger = Arc::new(FakePlane::default());
        let state = device_serving(&ledger).await;
        let plane = OpenRuns::answering(Some(open_run(false, false)));

        for _ in 0..2 {
            super::serve_open_run(&state, &plane, NOW, &two_resources).await;
        }

        let posted = ledger.posted.lock().await.clone();
        assert_eq!(
            posted.len(),
            1,
            "a shop enumerated twice is two rounds of marketplace requests for one run"
        );
        assert!(
            posted[0].listed.is_some() && !posted[0].complete,
            "the first half posts the listing the seller chooses from and completes nothing"
        );
        assert_eq!(
            plane.asked.load(Ordering::SeqCst),
            2,
            "the guard is over the work, not over the question: a run the seller ticks an \
             hour later has to be found by a later cycle"
        );
    }

    /// The second half, driven by the run's own selection rather than by a
    /// list the console carried over.
    #[tokio::test]
    async fn a_listed_run_the_seller_has_chosen_from_is_described_once() {
        let ledger = Arc::new(FakePlane::default());
        let state = device_serving(&ledger).await;
        let plane = OpenRuns::chosen(open_run(true, true), &["1", "2"]);

        for _ in 0..2 {
            super::serve_open_run(&state, &plane, NOW, &two_resources).await;
        }

        let posted = ledger.posted.lock().await.clone();
        assert_eq!(posted.len(), 1, "the selection is described once");
        assert_eq!(
            posted[0].resources.len(),
            2,
            "both resources the run names are described"
        );
        assert!(
            posted[0].complete,
            "the last page completes the run, or the server never mints the jobs"
        );
    }

    /// The ordinary cycle, and the one before the seller has ticked anything.
    #[tokio::test]
    async fn a_cycle_with_nothing_owed_makes_no_marketplace_request() {
        for answer in [None, Some(open_run(true, false))] {
            let ledger = Arc::new(FakePlane::default());
            let state = device_serving(&ledger).await;
            let plane = OpenRuns::answering(answer);

            super::serve_open_run(&state, &plane, NOW, &two_resources).await;

            assert!(
                ledger.posted.lock().await.is_empty(),
                "no open run, and a run whose seller is still choosing, both owe this \
                 device nothing: {answer:?}"
            );
        }
    }
}
