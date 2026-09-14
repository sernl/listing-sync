//! The bytes an upload needs, on the machine that performs the upload.
//!
//! Decision D27 puts file ingest on the seller's device: if the upload is
//! itself a marketplace request that must originate here, the bytes must be
//! here at upload time. Two things can be true of a file, and
//! [`PayloadSource`] is which. Either the bytes are ours — ingested through
//! `POST /v1/uploads` and living in our object store, so the device fetches
//! them back from the control plane for the duration of one run — or they are
//! the seller's, held by the marketplace they sell on, and the device fetches
//! them from there under the seller's own session. The first is the older
//! arrangement and is what a hand-uploaded file still uses; the second is what
//! D27 is for.
//!
//! So this module does reach a marketplace, through a
//! [`MarketplaceFiles`] implementation handed to it by the caller that holds
//! the seller's sessions. It composes no marketplace request itself and knows
//! no marketplace's wire: it names a resource and receives bytes. That
//! division is the two-branch rule holding at this seam rather than an
//! accident of layering, and the request it causes is issued on the seller's
//! own device under the seller's own login, which is what D1 requires.
//!
//! What a transfer is checked against depends on who committed to it, and the
//! manifest says. A response that restates its own digest proves nothing; a
//! digest committed to before the transfer proves the transfer. Our own arm
//! always carries one. A marketplace-held file carries one only after some
//! run has observed it, and on a first observation there is nothing to check
//! against and none is invented — see [`checked`]. The manifest is the driver
//! crate's [`PayloadManifest`], so server and device read one definition of
//! what was committed to.
//!
//! The bytes live in one directory per item under the application data
//! directory, and [`DevicePayloads::discard`] removes that directory once the
//! item settles. [`DevicePayloads`] also removes it on drop, so a run that
//! ended by an error rather than by a settle leaves nothing, and
//! [`sweep`] removes what a killed process could not. That promise is
//! unchanged by where the bytes came from, and it matters more for the
//! seller's own files than for ours: a copy we already hold is not made more
//! private by being deleted here, and a copy of the seller's is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tam_engine_driver::vocabulary::{Committed, PayloadManifest, PayloadSource};
use tam_marketplace::{FileContent, FileSource, FileSourceError};
use tam_types::{ContentHash, FileId, Marketplace};

use crate::device::DeviceId;
use crate::heartbeat::{ControlPlaneError, PlaneFuture};

/// The directory, under the application data directory, that every fetched
/// payload lives beneath. One level so [`sweep`] has something to sweep.
pub const CACHE_DIR: &str = "payloads";

/// The control-plane path one payload is read from.
///
/// A free function so the tests name the same expression the client uses
/// rather than a copy of it, exactly as [`crate::control_plane::heartbeat_path`]
/// is. The device is in the path because the server authorises the read
/// against the lease that device holds, and the file is in the path because
/// the read is a read: nothing about it is a body.
#[must_use]
pub fn payload_path(device: &DeviceId, file: FileId) -> String {
    format!("/v1/devices/{device}/payload/{}", file.0.to_hyphenated())
}

/// One read of one of our own control-plane paths, as bytes.
///
/// A trait rather than a direct call so the payload fetch is testable without
/// a socket, and so the one implementation that opens one — `HttpControlPlane`
/// — keeps the resolve-the-session-then-send order in the single place that
/// already states why that order matters.
pub trait PayloadTransport: Send + Sync {
    fn fetch<'a>(&'a self, path: &'a str) -> PlaneFuture<'a, Vec<u8>>;
}

/// Why a payload could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadError {
    /// The envelope named no manifest for this file, so there is nothing to
    /// verify a transfer against and none is attempted.
    Unmanifested(FileId),
    /// The control plane refused or could not be reached.
    Plane(ControlPlaneError),
    /// The bytes that arrived are not the bytes the envelope committed to.
    /// Never retried in place: a mismatch is either corruption or the wrong
    /// file, and both are conditions the run stalls on rather than works
    /// around.
    DigestMismatch {
        file: FileId,
        expected: ContentHash,
        received: ContentHash,
    },
    /// The transfer is the wrong length for the manifest, caught before the
    /// digest so a truncated multi-hundred-megabyte transfer is named as one.
    LengthMismatch {
        file: FileId,
        expected: i64,
        received: i64,
    },
    /// The manifest names an origin this source cannot fetch from.
    ///
    /// Not a transport failure, and never retried: no number of attempts
    /// gives this source a fetcher it does not have. The seller's own bytes
    /// on a no-API marketplace are fetched by a marketplace-backed source
    /// under the seller's own session, and a sanctioned marketplace's are the
    /// server's to hold, so an order naming one here would be asking this
    /// device to stand in for the API branch.
    UnsupportedSource { file: FileId, origin: Marketplace },
    /// The marketplace holding the bytes did not hand them over.
    ///
    /// Distinct from [`Self::Plane`] because the seller acts on it
    /// differently: our control plane being unreachable is ours to fix, and a
    /// marketplace refusing the seller's own session is a login for them to
    /// renew. It carries the marketplace's own sentence rather than a code,
    /// because the run ends the same way whatever the cause and what the
    /// seller needs is what happened.
    Source {
        file: FileId,
        origin: Marketplace,
        detail: String,
    },
    /// The cache directory could not be written or read.
    Cache(String),
}

impl core::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unmanifested(file) => write!(
                f,
                "the work envelope named no manifest for file {}, so no transfer can be verified",
                file.0.to_hyphenated()
            ),
            Self::Plane(why) => write!(f, "{why}"),
            Self::DigestMismatch { file, .. } => write!(
                f,
                "the bytes received for file {} are not the bytes the envelope committed to",
                file.0.to_hyphenated()
            ),
            Self::LengthMismatch {
                file,
                expected,
                received,
            } => write!(
                f,
                "file {} is {expected} bytes in the envelope and {received} arrived",
                file.0.to_hyphenated()
            ),
            Self::UnsupportedSource { file, origin } => write!(
                f,
                "file {} is held by {origin:?} and this source fetches only what the control \
                 plane holds",
                file.0.to_hyphenated()
            ),
            Self::Source {
                file,
                origin,
                detail,
            } => write!(
                f,
                "{origin:?} did not hand over file {}: {detail}",
                file.0.to_hyphenated()
            ),
            Self::Cache(why) => write!(f, "the payload cache is unusable: {why}"),
        }
    }
}

impl core::error::Error for PayloadError {}

/// One read of the seller's own bytes from a marketplace, under the seller's
/// own session.
///
/// A trait object rather than a second type parameter on [`DevicePayloads`],
/// so the seven call sites of [`DevicePayloads::for_item`] keep one signature
/// and the marketplace half is attached by [`DevicePayloads::sourcing`] where
/// a run has one. The implementation lives beside the adapters in `work.rs`,
/// because it is the only place holding the seller's marketplace sessions.
///
/// The error is a string because the caller cannot act on its structure: an
/// unreachable marketplace, a refused session and a bundle that will not parse
/// all end this run the same way, and what the seller needs is the sentence.
pub trait MarketplaceFiles: Send + Sync {
    fn fetch<'a>(
        &'a self,
        marketplace: Marketplace,
        resource: &'a str,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = Result<Vec<u8>, String>> + Send + 'a>>;
}

/// The [`FileSource`] the adapters upload through on this device.
///
/// One instance per run, holding the manifests that run's envelope carried and
/// owning the directory the fetched bytes live in.
pub struct DevicePayloads<T: PayloadTransport> {
    device: DeviceId,
    transport: T,
    /// The seller's own marketplace sessions, where this run has any. `None`
    /// is a run whose every file is ours, which needs no marketplace at all.
    marketplace: Option<std::sync::Arc<dyn MarketplaceFiles>>,
    /// This machine's library of imported originals, where the build has
    /// one: a file it holds under the manifest's digest is read from it
    /// rather than fetched from anywhere.
    library: Option<std::sync::Arc<crate::library::Library>>,
    manifests: HashMap<FileId, PayloadManifest>,
    directory: PathBuf,
}

impl<T: PayloadTransport> core::fmt::Debug for DevicePayloads<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DevicePayloads")
            .field("device", &self.device)
            .field("files", &self.manifests.len())
            .field("directory", &self.directory)
            .field("marketplace_source", &self.marketplace.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: PayloadTransport> DevicePayloads<T> {
    /// Binds the manifests one envelope carried to a directory of their own.
    ///
    /// `data_dir` is the application data directory; the run's own directory
    /// is created beneath [`CACHE_DIR`] and named for the item, so two items
    /// running in sequence never see each other's bytes and a sweep can tell
    /// them apart.
    #[must_use]
    pub fn for_item(
        device: DeviceId,
        transport: T,
        data_dir: &Path,
        item: &str,
        manifests: Vec<PayloadManifest>,
    ) -> Self {
        let directory = data_dir.join(CACHE_DIR).join(segment(item));
        Self {
            device,
            transport,
            marketplace: None,
            library: None,
            manifests: manifests
                .into_iter()
                .map(|manifest| (manifest.file, manifest))
                .collect(),
            directory,
        }
    }

    /// Attaches this machine's library, so a file it already holds is read
    /// from it rather than transferred again. The same builder shape as
    /// [`Self::sourcing`].
    #[must_use]
    pub fn reading(mut self, library: std::sync::Arc<crate::library::Library>) -> Self {
        self.library = Some(library);
        self
    }

    /// Attaches the seller's own marketplace sessions to this run.
    ///
    /// A builder rather than a sixth argument to [`Self::for_item`], so a run
    /// whose files are all ours reads exactly as it did before this existed;
    /// it is the same shape `TptAdapter::attesting` uses for the declaration a
    /// write may or may not need.
    #[must_use]
    pub fn sourcing(mut self, files: std::sync::Arc<dyn MarketplaceFiles>) -> Self {
        self.marketplace = Some(files);
        self
    }

    /// The directory this run's bytes live in.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Removes every byte this run fetched.
    ///
    /// Called once the item has settled. Removing a directory that was never
    /// created is success, because an item whose upload never ran has nothing
    /// to remove and that is not a failure to report.
    pub async fn discard(&self) -> Result<(), PayloadError> {
        match tokio::fs::remove_dir_all(&self.directory).await {
            Ok(()) => Ok(()),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(why) => Err(PayloadError::Cache(why.to_string())),
        }
    }

    fn path_for(&self, file: FileId) -> PathBuf {
        // The file name is a hyphenated UUID this process formatted, so it is
        // a path segment by construction and needs no sanitising of its own.
        self.directory.join(file.0.to_hyphenated())
    }

    async fn load(&self, manifest: &PayloadManifest) -> Result<Vec<u8>, PayloadError> {
        // The cache is consulted before the source is, because a second read
        // of one file in one run must not be a second transfer whichever end
        // holds the bytes. What a cache hit is checked against is the same
        // question the source decides below, so the check travels with it.
        let cached = self.path_for(manifest.file);
        match tokio::fs::read(&cached).await {
            Ok(bytes) => return checked(manifest, bytes),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => return Err(PayloadError::Cache(why.to_string())),
        }
        // Then this machine's own library, which holds the originals its
        // imports read. A hit is the seller's file already on the seller's
        // machine, so no marketplace is asked and nothing crosses a wire;
        // it is verified against the commitment exactly as a transfer is,
        // and a library that cannot answer falls through to the source.
        if let (Some(library), Some(committed)) = (&self.library, manifest.committed()) {
            if let Ok(Some(bytes)) = library.read(committed.hash).await {
                return checked(manifest, bytes);
            }
        }

        let bytes = match &manifest.source {
            PayloadSource::ControlPlane { .. } => self
                .transport
                .fetch(&payload_path(&self.device, manifest.file))
                .await
                .map_err(PayloadError::Plane)?,
            PayloadSource::Marketplace {
                marketplace,
                resource,
                ..
            } => {
                let files = self
                    .marketplace
                    .as_ref()
                    .ok_or(PayloadError::UnsupportedSource {
                        file: manifest.file,
                        origin: *marketplace,
                    })?;
                let bundle = files
                    .fetch(*marketplace, resource)
                    .await
                    .map_err(|detail| PayloadError::Source {
                        file: manifest.file,
                        origin: *marketplace,
                        detail,
                    })?;
                // The marketplace hands over a bundle; what the target's one
                // product slot takes is a file. The rule is decided here, on
                // the bytes, rather than recorded at import, because it never
                // changes the file count and so cannot move the projection.
                unwrapped(bundle)
            }
        };
        let bytes = checked(manifest, bytes)?;

        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|why| PayloadError::Cache(why.to_string()))?;
        tokio::fs::write(&cached, &bytes)
            .await
            .map_err(|why| PayloadError::Cache(why.to_string()))?;
        Ok(bytes)
    }
}

/// The bytes, checked against whatever this manifest committed to.
///
/// A manifest with no commitment is the first observation of a
/// marketplace-held file, and there is nothing to check it against: the server
/// holds no copy and so committed to nothing. The bytes are returned rather
/// than verified, and that is the honest state until the import pass records
/// an observation for the next run to check. Nothing substitutes for the
/// missing commitment, because a check against a value invented here would
/// assert a guarantee nobody made.
fn checked(manifest: &PayloadManifest, bytes: Vec<u8>) -> Result<Vec<u8>, PayloadError> {
    match manifest.committed() {
        Some(committed) => verified(manifest.file, committed, bytes),
        None => Ok(bytes),
    }
}

/// A bundle reduced to the one file it holds, or left whole.
///
/// The target's product slot takes exactly one file, so a bundle of several is
/// sent as the bundle — which is also what the source marketplace's own buyers
/// receive. A bundle of one is sent as that one file instead, because a buyer
/// expects the worksheet rather than a zip wrapping the worksheet.
///
/// Anything that is not a readable archive is left exactly as it arrived. This
/// function's job is to unwrap a bundle, and a thing it cannot read is not a
/// bundle it should be guessing about.
fn unwrapped(bundle: Vec<u8>) -> Vec<u8> {
    match tam_pipeline::archive::sole_entry(
        &bundle,
        tam_pipeline::archive::ExtractBudget::default(),
    ) {
        Some(only) => only.bytes,
        None => bundle,
    }
}

/// One path segment, from a name the work envelope supplied.
///
/// This is a correctness rail rather than tidiness: the directory it names is
/// removed recursively, by [`DevicePayloads::discard`] and again on drop, so a
/// name carrying `..` or a separator would delete something else. The item id
/// is a hyphenated UUID in every envelope the server sends, and the filter
/// keeps exactly that alphabet; anything outside it collapses to a fixed name
/// rather than escaping.
fn segment(raw: &str) -> String {
    let kept: String = raw
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(64)
        .collect();
    if kept.is_empty() {
        "unnamed-item".to_owned()
    } else {
        kept
    }
}

/// The bytes, if they are the bytes that were committed to.
///
/// Length first so a truncated transfer is named as one rather than as an
/// unexplained digest mismatch; the digest is what actually decides.
///
/// Takes the commitment rather than the manifest, because the check is the
/// same check whoever made the commitment: what differs between a
/// control-plane source and a marketplace one is what the commitment proves,
/// which [`PayloadSource`] states and this function does not need to know.
fn verified(file: FileId, committed: &Committed, bytes: Vec<u8>) -> Result<Vec<u8>, PayloadError> {
    let received = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if received != committed.byte_len {
        return Err(PayloadError::LengthMismatch {
            file,
            expected: committed.byte_len,
            received,
        });
    }
    let digest = ContentHash(blake3::hash(&bytes).into());
    if digest != committed.hash {
        return Err(PayloadError::DigestMismatch {
            file,
            expected: committed.hash,
            received: digest,
        });
    }
    Ok(bytes)
}

impl<T: PayloadTransport> FileSource for DevicePayloads<T> {
    async fn fetch(&self, file: FileId) -> Result<FileContent, FileSourceError> {
        let manifest = self
            .manifests
            .get(&file)
            .ok_or(FileSourceError::Missing(file))?;
        let bytes = self
            .load(manifest)
            .await
            .map_err(|why| FileSourceError::Unreadable {
                file,
                detail: why.to_string(),
            })?;
        Ok(FileContent {
            file_name: manifest.file_name.clone(),
            content_type: manifest.content_type.clone(),
            bytes,
        })
    }
}

impl<T: PayloadTransport> Drop for DevicePayloads<T> {
    /// The backstop for a run that ended by an error rather than by a settle.
    ///
    /// Synchronous and result-ignoring because a drop can be neither
    /// asynchronous nor fallible, and because the alternative to a
    /// best-effort removal here is keeping a seller's file on disk until the
    /// next sweep. `discard` remains the path that reports a failure.
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

/// Removes every payload directory left under `data_dir`.
///
/// Run at start-up. A process killed mid-run runs neither `discard` nor the
/// drop above, and the promise that nothing is kept after a settle is worth
/// only as much as the case where there was no settle.
pub async fn sweep(data_dir: &Path) -> Result<(), PayloadError> {
    match tokio::fs::remove_dir_all(data_dir.join(CACHE_DIR)).await {
        Ok(()) => Ok(()),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(why) => Err(PayloadError::Cache(why.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        payload_path, segment, sweep, unwrapped, DevicePayloads, MarketplaceFiles, PayloadManifest,
        PayloadTransport, CACHE_DIR,
    };
    use crate::device::DeviceId;
    use crate::heartbeat::{ControlPlaneError, PlaneFuture};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tam_engine_driver::vocabulary::{Committed, PayloadSource};
    use tam_marketplace::{FileSource, FileSourceError};
    use tam_types::{ContentHash, FileId, Marketplace, Uuid};
    use tokio::sync::Mutex;

    const DEVICE: &str = "11112222333344445555666677778888";
    const ITEM: &str = "3f1c8a2e-0000-4000-8000-000000000001";
    const BYTES: &[u8] = b"%PDF-1.7 the seller's own worksheet\n\xff\xfe binary tail";

    fn file(last: u8) -> FileId {
        let mut raw = [0u8; 16];
        raw[15] = last;
        FileId(Uuid(raw))
    }

    fn manifest_for(id: FileId, bytes: &[u8]) -> PayloadManifest {
        PayloadManifest {
            file: id,
            file_name: "worksheet.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            source: PayloadSource::ControlPlane {
                committed: Committed {
                    hash: ContentHash(blake3::hash(bytes).into()),
                    byte_len: i64::try_from(bytes.len()).expect("a fixture fits in an i64"),
                },
            },
        }
    }

    /// The same file, held by the marketplace the seller sells it on rather
    /// than by us.
    fn marketplace_manifest(id: FileId) -> PayloadManifest {
        PayloadManifest {
            file: id,
            file_name: "worksheet.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            source: PayloadSource::Marketplace {
                marketplace: Marketplace::Tes,
                resource: "13549126".to_owned(),
                entry: None,
                expected: None,
            },
        }
    }

    /// A control plane that answers canned bytes and records what it was asked
    /// for, so the path and the number of transfers are both assertable
    /// without a socket.
    #[derive(Default)]
    struct FakePlane {
        answer: Option<Vec<u8>>,
        asked: Mutex<Vec<String>>,
    }

    impl FakePlane {
        fn answering(bytes: &[u8]) -> Self {
            Self {
                answer: Some(bytes.to_vec()),
                asked: Mutex::new(Vec::new()),
            }
        }

        async fn asked(&self) -> Vec<String> {
            self.asked.lock().await.clone()
        }
    }

    impl PayloadTransport for FakePlane {
        fn fetch<'a>(&'a self, path: &'a str) -> PlaneFuture<'a, Vec<u8>> {
            Box::pin(async move {
                self.asked.lock().await.push(path.to_owned());
                self.answer
                    .clone()
                    .ok_or_else(|| ControlPlaneError::Refused("nothing to serve".to_owned()))
            })
        }
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tam-desktop-payload-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        dir
    }

    fn payloads(
        data_dir: &Path,
        plane: FakePlane,
        manifests: Vec<PayloadManifest>,
    ) -> DevicePayloads<FakePlane> {
        DevicePayloads::for_item(DeviceId::from_raw(DEVICE), plane, data_dir, ITEM, manifests)
    }

    #[tokio::test]
    async fn the_payload_is_fetched_once_verified_and_cached_under_the_data_directory() {
        let data_dir = scratch();
        let id = file(1);
        let source = payloads(
            &data_dir,
            FakePlane::answering(BYTES),
            vec![manifest_for(id, BYTES)],
        );

        let first = source.fetch(id).await.expect("the payload is produced");
        assert_eq!(first.bytes, BYTES, "the seller's own bytes, unchanged");
        assert_eq!(first.file_name, "worksheet.pdf");
        assert_eq!(first.content_type, "application/pdf");

        let second = source
            .fetch(id)
            .await
            .expect("the payload is produced again");
        assert_eq!(second.bytes, BYTES);

        assert_eq!(
            source.transport.asked().await,
            vec![payload_path(&DeviceId::from_raw(DEVICE), id)],
            "a second read of one file in one run comes from the cache, not from a second \
             transfer of the same hundreds of megabytes"
        );
        assert!(
            source.directory().starts_with(data_dir.join(CACHE_DIR)),
            "the bytes live under the application data directory and nowhere else"
        );
        assert!(source.directory().join(id.0.to_hyphenated()).exists());

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    /// A marketplace that hands back whatever it was given, recording what it
    /// was asked for.
    struct FakeMarketplace {
        answer: Result<Vec<u8>, String>,
        asked: Mutex<Vec<(Marketplace, String)>>,
    }

    impl FakeMarketplace {
        fn answering(bytes: Vec<u8>) -> Arc<Self> {
            Arc::new(Self {
                answer: Ok(bytes),
                asked: Mutex::new(Vec::new()),
            })
        }

        fn refusing(why: &str) -> Arc<Self> {
            Arc::new(Self {
                answer: Err(why.to_owned()),
                asked: Mutex::new(Vec::new()),
            })
        }
    }

    impl MarketplaceFiles for FakeMarketplace {
        fn fetch<'a>(
            &'a self,
            marketplace: Marketplace,
            resource: &'a str,
        ) -> core::pin::Pin<
            Box<dyn core::future::Future<Output = Result<Vec<u8>, String>> + Send + 'a>,
        > {
            Box::pin(async move {
                self.asked
                    .lock()
                    .await
                    .push((marketplace, resource.to_owned()));
                self.answer.clone()
            })
        }
    }

    /// The bytes come from the marketplace holding them, under the seller's
    /// own session, and never from us.
    ///
    /// The second assertion is the decision's own property: our control plane
    /// is asked for nothing, because these are bytes we do not have and must
    /// not have.
    #[tokio::test]
    async fn a_marketplace_sourced_file_is_fetched_from_the_marketplace_and_not_from_us() {
        let data_dir = scratch();
        let id = file(3);
        let market = FakeMarketplace::answering(BYTES.to_vec());
        // Method-call syntax rather than `Arc::clone`, which would resolve its
        // own type parameter against the concrete type and refuse the unsizing
        // coercion; the same note sits on the two bindings in `lib.rs`.
        let files: Arc<dyn MarketplaceFiles> = market.clone();
        let source = payloads(
            &data_dir,
            FakePlane::answering(b"our copy, which must never be reached"),
            vec![marketplace_manifest(id)],
        )
        .sourcing(files);

        let got = source.fetch(id).await.expect("the seller's own bytes");
        assert_eq!(got.bytes, BYTES, "the marketplace's bytes, unchanged");
        assert_eq!(
            market.asked.lock().await.as_slice(),
            [(Marketplace::Tes, "13549126".to_owned())],
            "asked the marketplace the manifest named, for the resource it named"
        );
        assert!(
            source.transport.asked().await.is_empty(),
            "our control plane is asked for nothing: these are the seller's bytes and D27 is \
             that we never hold them"
        );

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    /// A first observation has nothing to check against, and says so by
    /// accepting rather than by inventing a value to check.
    #[tokio::test]
    async fn a_first_observation_is_accepted_because_nobody_committed_to_it() {
        let data_dir = scratch();
        let id = file(4);
        // Bytes that would fail any check, which is the point: with no
        // commitment there is no check to fail.
        let market = FakeMarketplace::answering(b"whatever the marketplace had".to_vec());
        let source = payloads(
            &data_dir,
            FakePlane::default(),
            vec![marketplace_manifest(id)],
        )
        .sourcing(market);

        let got = source
            .fetch(id)
            .await
            .expect("an uncommitted observation is not a failure");
        assert_eq!(got.bytes, b"whatever the marketplace had");

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    /// A marketplace that will not hand the file over is named as itself.
    ///
    /// It must not read as our control plane failing: the seller renews a
    /// login for one and waits for us on the other.
    #[tokio::test]
    async fn a_marketplace_that_refuses_is_named_rather_than_read_as_our_own_failure() {
        let data_dir = scratch();
        let id = file(5);
        let market = FakeMarketplace::refusing("the session has expired");
        let source = payloads(
            &data_dir,
            FakePlane::default(),
            vec![marketplace_manifest(id)],
        )
        .sourcing(market);

        let why = source.fetch(id).await.expect_err("a refusal is not bytes");
        let FileSourceError::Unreadable { detail, .. } = why else {
            panic!("a marketplace refusal is unreadable content, not a missing file");
        };
        assert!(
            detail.contains("Tes") && detail.contains("the session has expired"),
            "the refusal names the marketplace and carries its own sentence: {detail}"
        );

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    /// The unwrap's policy half. Its mechanics — what "exactly one entry"
    /// means — are `tam_pipeline::archive::sole_entry`'s and are tested there
    /// against real archives; what belongs here is that anything the archive
    /// reader cannot reduce to one file is uploaded exactly as it arrived,
    /// because the alternative is this device inventing a payload.
    #[test]
    fn bytes_that_are_not_a_single_file_archive_are_left_exactly_as_they_arrived() {
        assert_eq!(
            unwrapped(BYTES.to_vec()),
            BYTES,
            "a file that is not an archive is the payload, untouched"
        );
        assert_eq!(
            unwrapped(b"PK\x03\x04 truncated".to_vec()),
            b"PK\x03\x04 truncated",
            "something claiming to be an archive but unreadable is passed through rather than \
             guessed at"
        );
    }

    /// The source arm decides before anything is read.
    ///
    /// The assertion that matters is the second one: a manifest naming bytes
    /// this source does not hold must cost no request to our control plane,
    /// because the alternative is a 403 from the payload route standing in for
    /// a decision the manifest already stated.
    #[tokio::test]
    async fn a_marketplace_sourced_file_is_refused_before_any_transfer_is_attempted() {
        let data_dir = scratch();
        let id = file(7);
        let source = payloads(
            &data_dir,
            FakePlane::answering(BYTES),
            vec![marketplace_manifest(id)],
        );

        let why = source
            .fetch(id)
            .await
            .expect_err("bytes this source does not hold are not produced");
        let FileSourceError::Unreadable { file, detail } = why else {
            panic!("a file held elsewhere is unreadable here, not absent from the envelope");
        };
        assert_eq!(file, id);
        assert!(
            detail.contains("Tes"),
            "the refusal names the marketplace holding the bytes, so what is missing is a \
             login rather than a file: {detail}"
        );
        assert!(
            source.transport.asked().await.is_empty(),
            "a manifest this source cannot serve costs no request to our control plane"
        );

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn bytes_that_are_not_the_ones_the_envelope_committed_to_are_refused() {
        let data_dir = scratch();
        let id = file(2);
        let source = payloads(
            &data_dir,
            FakePlane::answering(b"a different file of the same length!!"),
            vec![manifest_for(id, b"the file the envelope committed to!!!")],
        );

        let why = source.fetch(id).await.expect_err("the digest is checked");
        let FileSourceError::Unreadable { file, detail } = why else {
            panic!("a digest mismatch is unreadable content, not a missing file");
        };
        assert_eq!(file, id);
        assert!(
            detail.contains("not the bytes the envelope committed to"),
            "the refusal names what failed: {detail}"
        );
        assert!(
            !source.directory().join(id.0.to_hyphenated()).exists(),
            "unverified bytes are never written to the cache, or the next run would read them \
             back and skip the check"
        );

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_truncated_transfer_is_named_as_one() {
        let data_dir = scratch();
        let id = file(3);
        let source = payloads(
            &data_dir,
            FakePlane::answering(&BYTES[..10]),
            vec![manifest_for(id, BYTES)],
        );

        let why = source.fetch(id).await.expect_err("the length is checked");
        let FileSourceError::Unreadable { detail, .. } = why else {
            panic!("a truncated transfer is unreadable content");
        };
        assert!(
            detail.contains(&format!("{} bytes in the envelope", BYTES.len())),
            "a truncated transfer of a large file must not read as an unexplained digest \
             mismatch: {detail}"
        );

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_file_the_envelope_did_not_manifest_is_missing_rather_than_fetched() {
        let data_dir = scratch();
        let source = payloads(&data_dir, FakePlane::answering(BYTES), Vec::new());

        assert_eq!(
            source.fetch(file(4)).await,
            Err(FileSourceError::Missing(file(4))),
            "with no manifest there is nothing to verify a transfer against, so none is made"
        );
        assert!(
            source.transport.asked().await.is_empty(),
            "and the control plane is never asked"
        );

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_control_plane_that_refuses_produces_no_cached_file() {
        let data_dir = scratch();
        let id = file(5);
        let source = payloads(
            &data_dir,
            FakePlane::default(),
            vec![manifest_for(id, BYTES)],
        );

        assert!(source.fetch(id).await.is_err());
        assert!(!source.directory().join(id.0.to_hyphenated()).exists());

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn discarding_removes_every_byte_the_run_fetched() {
        let data_dir = scratch();
        let id = file(6);
        let source = payloads(
            &data_dir,
            FakePlane::answering(BYTES),
            vec![manifest_for(id, BYTES)],
        );
        source.fetch(id).await.expect("the payload is produced");
        let cached = source.directory().to_path_buf();
        assert!(cached.exists());

        source.discard().await.expect("the cache is removable");
        assert!(
            !cached.exists(),
            "the seller's file must not outlive the item that needed it"
        );
        source
            .discard()
            .await
            .expect("discarding twice is not a failure");

        drop(source);
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn dropping_without_a_settle_leaves_nothing_behind() {
        let data_dir = scratch();
        let id = file(7);
        let cached = {
            let source = payloads(
                &data_dir,
                FakePlane::answering(BYTES),
                vec![manifest_for(id, BYTES)],
            );
            source.fetch(id).await.expect("the payload is produced");
            let cached = source.directory().to_path_buf();
            assert!(cached.exists());
            cached
        };
        assert!(
            !cached.exists(),
            "a run that ended by an error rather than by a settle must leave no file either"
        );
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_sweep_removes_what_a_killed_process_could_not() {
        let data_dir = scratch();
        let stale = data_dir.join(CACHE_DIR).join("an-item-that-never-settled");
        std::fs::create_dir_all(&stale).expect("the stale directory is creatable");
        std::fs::write(stale.join("left-behind"), BYTES).expect("the stale file is writable");

        sweep(&data_dir).await.expect("the sweep succeeds");
        assert!(
            !data_dir.join(CACHE_DIR).exists(),
            "a process killed mid-run runs neither discard nor drop, and its bytes are still \
             the seller's"
        );
        sweep(&data_dir)
            .await
            .expect("sweeping a machine with nothing to sweep is not a failure");

        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[tokio::test]
    async fn a_traversing_item_name_cannot_reach_outside_the_cache() {
        let data_dir = scratch();
        let sibling = data_dir.join("not-the-cache");
        std::fs::create_dir_all(&sibling).expect("the sibling directory is creatable");
        std::fs::write(sibling.join("keep-me"), b"not a payload").expect("the sibling file writes");

        {
            let source = payloads(&data_dir, FakePlane::default(), Vec::new());
            let escaping = DevicePayloads::for_item(
                DeviceId::from_raw(DEVICE),
                FakePlane::default(),
                &data_dir,
                "../../not-the-cache",
                Vec::new(),
            );
            assert!(
                escaping.directory().starts_with(data_dir.join(CACHE_DIR)),
                "the cache directory is removed recursively on drop, so a name from the envelope                  must not be able to point it anywhere else"
            );
            drop(escaping);
            drop(source);
        }

        assert!(
            sibling.join("keep-me").exists(),
            "dropping a run whose item name traversed must not have deleted a sibling directory"
        );
        std::fs::remove_dir_all(&data_dir).ok();
    }

    #[test]
    fn a_directory_name_is_one_segment_of_the_alphabet_an_item_id_uses() {
        assert_eq!(segment("3f1c8a2e-0000-4000-8000-000000000001"), ITEM);
        assert_eq!(segment("../../etc"), "etc");
        assert_eq!(segment("a/b"), "ab");
        assert_eq!(segment(".."), "unnamed-item");
        assert_eq!(segment(""), "unnamed-item");
    }

    #[test]
    fn the_path_names_the_device_and_the_file() {
        assert_eq!(
            payload_path(&DeviceId::from_raw(DEVICE), file(8)),
            "/v1/devices/11112222333344445555666677778888/payload/\
             00000000-0000-0000-0000-000000000008",
            "the device is in the path because the server authorises the read against the lease \
             that device holds; this is the contract owed in the design note"
        );
    }
}
