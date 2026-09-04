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

use tam_marketplace::ImportedListing;
use tam_types::{ContentHash, FileKind, Marketplace, ScanOutcome, Timestamp};
use tokio::sync::Mutex;

// The page vocabulary is `tam-engine-driver`'s, because `tam-api` consumes
// exactly what this pass produces and a server struct that happens to match a
// client one is a coincidence rather than a contract. Re-exported rather than
// merely imported so this module's surface is unchanged by where the
// definitions now live.
pub use tam_engine_driver::import::{
    base64, ContentType, Cover, FileName, ImportPage, Locator, NotReportable, ObservedFile,
    ObservedResource, Reason, SkippedResource, CONTENT_TYPE_MAX, COVER_BYTES_MAX, LOCATOR_MAX,
    NAME_MAX, PNG_MAGIC, REASON_MAX,
};

use crate::entitlement::EntitlementGate;
use crate::heartbeat::{ControlPlaneError, PlaneFuture};
use crate::ledger::LedgerTransport;

/// The control-plane path one page of the catalogue is posted to.
///
/// A free function so the tests name the same expression the pass uses rather
/// than a copy of it, as every other device path in this crate is.
#[must_use]
pub fn import_path(device: &crate::device::DeviceId) -> String {
    format!("/v1/devices/{device}/import")
}

/// How many resources one page carries.
///
/// A page is the unit of resumability rather than of efficiency: the server
/// records a breadcrumb per resource, so a page that fails is re-walked and
/// one that succeeded is skipped. Small enough that a failure costs little
/// work, large enough that a five-hundred-resource shop is not five hundred
/// round trips.
pub const PAGE_SIZE: usize = 25;

/// The seller's own catalogue, as this device can read it.
///
/// A seam over the marketplace adapter rather than the adapter itself, for the
/// reason every seam in this crate exists: the pass is then provable without a
/// marketplace, and the adapter's associated types — how one marketplace names
/// a catalogue row and addresses a resource — stay at the one edge that knows
/// them.
pub trait CatalogueSource: Send + Sync {
    /// Every resource the seller has, whole. A walk that cannot reach its end
    /// refuses rather than returning a truncation, which is the contract
    /// `list_own_resources` already keeps: a short catalogue read as complete
    /// would silently migrate part of a shop.
    fn list(&self) -> PlaneFuture<'_, Vec<i64>>;

    /// One listing, verbatim, for canonicalisation.
    fn read(&self, resource: i64) -> PlaneFuture<'_, ImportedListing>;

    /// The bytes of one resource's bundle.
    fn bundle(&self, resource: i64) -> PlaneFuture<'_, Vec<u8>>;
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
pub struct ImportPass<S: CatalogueSource, P: LedgerTransport> {
    device: crate::device::DeviceId,
    source: S,
    plane: Arc<P>,
    request: tam_types::Uuid,
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

impl<S: CatalogueSource, P: LedgerTransport> ImportPass<S, P> {
    #[must_use]
    pub const fn new(
        device: crate::device::DeviceId,
        source: S,
        plane: Arc<P>,
        request: tam_types::Uuid,
        permission: SourcePermission,
    ) -> Self {
        Self {
            device,
            source,
            plane,
            request,
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

    /// Reads the whole catalogue, describing each resource and posting pages.
    ///
    /// `now` is passed in rather than read, because this crate holds no clock:
    /// the scan instant it records is the caller's reading, exactly as every
    /// other instant on this device is.
    pub async fn run(
        &self,
        now: Timestamp,
        mut progress: impl FnMut(ImportProgress) + Send,
    ) -> Result<PassReport, PassError> {
        // Before the enumeration, which is itself a marketplace request.
        if let Some(refusal) = self.refusal(now).await {
            return Err(refusal);
        }
        let catalogue = self
            .source
            .list()
            .await
            .map_err(|why| PassError::Catalogue(why.to_string()))?;

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
            if let Some(refusal) = self.refusal(now).await {
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
            match self.describe(locator, now).await {
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
        let bundle = self
            .source
            .bundle(locator)
            .await
            .map_err(|why| format!("its file could not be fetched: {why}"))?;

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

        let hash = blake3::hash(&payload);
        let observed = ObservedResource {
            locator: Locator::from_resource_id(locator),
            listing,
            file: ObservedFile {
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
            },
            cover_png: Cover::encode(&cover.png)
                .map_err(|why| format!("the cover made from its file is not one: {why}"))?,
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
        let page = ImportPage {
            request: self.request,
            resources,
            skipped,
            complete,
        };
        let body = serde_json::to_string(&page)
            .map_err(|why| PassError::Page(format!("the page could not be encoded: {why}")))?;
        self.plane
            .post(&import_path(&self.device), body)
            .await
            .map_err(|why: ControlPlaneError| PassError::Page(why.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        base64, content_type_for, import_path, payload_of, CatalogueSource, ImportPage, ImportPass,
        Locator, PassError, SourcePermission, PAGE_SIZE, PNG_MAGIC,
    };
    use crate::device::DeviceId;
    use crate::entitlement::{Claims, Entitlement, EntitlementGate};
    use crate::heartbeat::{ControlPlaneError, PlaneFuture};
    use crate::ledger::LedgerTransport;
    use core::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tam_marketplace::{ImportedListing, RemoteListingId};
    use tam_types::{CopyFormat, FileKind, ImportedPrice, Marketplace, ScanOutcome, Timestamp};
    use tokio::sync::Mutex;

    const DEVICE: &str = "11112222333344445555666677778888";
    /// The import this pass belongs to. One value, so a resumed pass posting
    /// the same id is the assertion rather than a coincidence of spelling.
    const REQUEST: tam_types::Uuid = tam_types::Uuid([0x71; 16]);
    const NOW_SECONDS: i64 = 1_756_000_000;
    const NOW: Timestamp = Timestamp(NOW_SECONDS * 1_000);
    const PDF: &[u8] = b"%PDF-1.7 the seller's own worksheet";

    fn listing(id: i64) -> ImportedListing {
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
            state: None,
        }
    }

    /// A catalogue that answers from a script.
    struct Scripted {
        catalogue: Result<Vec<i64>, String>,
        bundle: Vec<u8>,
        /// Resources whose bundle fetch fails, so one bad resource in a shop
        /// can be driven without failing the others.
        unfetchable: Vec<i64>,
    }

    impl Scripted {
        fn of(count: i64, bundle: Vec<u8>) -> Self {
            Self {
                catalogue: Ok((1..=count).collect()),
                bundle,
                unfetchable: Vec::new(),
            }
        }
    }

    impl CatalogueSource for Scripted {
        fn list(&self) -> PlaneFuture<'_, Vec<i64>> {
            Box::pin(async move { self.catalogue.clone().map_err(ControlPlaneError::Refused) })
        }

        fn read(&self, resource: i64) -> PlaneFuture<'_, ImportedListing> {
            Box::pin(async move { Ok(listing(resource)) })
        }

        fn bundle(&self, resource: i64) -> PlaneFuture<'_, Vec<u8>> {
            Box::pin(async move {
                if self.unfetchable.contains(&resource) {
                    return Err(ControlPlaneError::Refused("the session expired".to_owned()));
                }
                Ok(self.bundle.clone())
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
            REQUEST,
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
        let [page] = posted.as_slice() else {
            panic!("three resources are one page, and got {}", posted.len());
        };
        assert!(page.complete, "the last page says so, or nothing is minted");
        assert_eq!(page.resources.len(), 3);

        let first = &page.resources[0];
        assert_eq!(first.file.kind, FileKind::Pdf, "probed, not declared");
        assert_eq!(first.file.byte_len, PDF.len() as u64);
        assert_eq!(
            first.file.payload_content_type.as_str(),
            content_type_for(FileKind::Pdf, PDF)
        );
        assert!(
            matches!(first.file.scan, ScanOutcome::Clean { .. }),
            "the device's own scan, recorded as the device's"
        );
        assert!(
            first.cover_png.bytes().starts_with(PNG_MAGIC),
            "a cover is derived, kept, and is the PNG the type promises"
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
        let [page] = posted.as_slice() else {
            panic!("one page, and got {}", posted.len());
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
        let [page] = posted.as_slice() else {
            panic!("one completing page, and got {}", posted.len());
        };
        assert!(page.resources.is_empty());
        assert!(page.complete, "or the server never mints anything");
    }

    /// A catalogue that could not be read is a failure with its own name.
    #[tokio::test]
    async fn a_catalogue_that_could_not_be_read_is_never_an_empty_one() {
        let plane = Arc::new(FakePlane::default());
        let source = Scripted {
            catalogue: Err("403".to_owned()),
            bundle: PDF.to_vec(),
            unfetchable: Vec::new(),
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
        let posted = plane.posted.lock().await.clone();
        assert_eq!(posted.len(), 2);
        assert!(!posted[0].complete, "only the last page completes");
        assert!(posted[1].complete);
        assert_eq!(posted[0].resources.len(), PAGE_SIZE);
        assert_eq!(posted[1].resources.len(), 3);
        assert!(
            posted.iter().all(|page| page.request == REQUEST),
            "every page names the same request, or a resumed pass becomes a second import"
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
}
