//! The bytes an upload needs, on the machine that performs the upload.
//!
//! Decision D27 puts file ingest on the seller's device: if the upload is
//! itself a marketplace request that must originate here, the bytes must be
//! here at upload time. That is the target. What holds today is the interim —
//! the catalogue's files were ingested through `POST /v1/uploads` and live in
//! our object store — so the device fetches them back from the control plane
//! for the duration of one run and keeps nothing afterwards.
//!
//! Three rules make the interim safe to hold while it lasts.
//!
//! The digest is checked against the manifest the work envelope carried, not
//! against anything the response said about itself. A response that restates
//! its own digest proves nothing; a digest the server committed to before the
//! transfer proves the transfer. The manifest is the driver crate's
//! [`PayloadManifest`], so server and device read one definition of what was
//! committed to.
//!
//! Which of those two things a manifest carries is [`PayloadSource`]'s to say,
//! and this module fetches only the arm it names as ours. A manifest naming a
//! marketplace is the seller's bytes held by the marketplace, fetched under
//! the seller's own session by a source this module does not build.
//!
//! The bytes live in one directory per item under the application data
//! directory, and [`DevicePayloads::discard`] removes that directory once the
//! item settles. [`DevicePayloads`] also removes it on drop, so a run that
//! ended by an error rather than by a settle leaves nothing, and
//! [`sweep`] removes what a killed process could not.
//!
//! Nothing here reaches a marketplace. The one host this module speaks to is
//! ours, through the same control-plane seam and under the same console
//! session as the check-in.

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
            Self::Cache(why) => write!(f, "the payload cache is unusable: {why}"),
        }
    }
}

impl core::error::Error for PayloadError {}

/// The [`FileSource`] the adapters upload through on this device.
///
/// One instance per run, holding the manifests that run's envelope carried and
/// owning the directory the fetched bytes live in.
pub struct DevicePayloads<T: PayloadTransport> {
    device: DeviceId,
    transport: T,
    manifests: HashMap<FileId, PayloadManifest>,
    directory: PathBuf,
}

impl<T: PayloadTransport> core::fmt::Debug for DevicePayloads<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DevicePayloads")
            .field("device", &self.device)
            .field("files", &self.manifests.len())
            .field("directory", &self.directory)
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
            manifests: manifests
                .into_iter()
                .map(|manifest| (manifest.file, manifest))
                .collect(),
            directory,
        }
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
        // The source decides before anything is read, so a manifest this
        // source cannot serve costs no cache read and no request.
        let committed = match &manifest.source {
            PayloadSource::ControlPlane { committed } => committed,
            PayloadSource::Marketplace { marketplace, .. } => {
                return Err(PayloadError::UnsupportedSource {
                    file: manifest.file,
                    origin: *marketplace,
                })
            }
        };
        let cached = self.path_for(manifest.file);
        match tokio::fs::read(&cached).await {
            Ok(bytes) => return verified(manifest.file, committed, bytes),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => return Err(PayloadError::Cache(why.to_string())),
        }

        let bytes = self
            .transport
            .fetch(&payload_path(&self.device, manifest.file))
            .await
            .map_err(PayloadError::Plane)?;
        let bytes = verified(manifest.file, committed, bytes)?;

        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|why| PayloadError::Cache(why.to_string()))?;
        tokio::fs::write(&cached, &bytes)
            .await
            .map_err(|why| PayloadError::Cache(why.to_string()))?;
        Ok(bytes)
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
        payload_path, segment, sweep, DevicePayloads, PayloadManifest, PayloadTransport, CACHE_DIR,
    };
    use crate::device::DeviceId;
    use crate::heartbeat::{ControlPlaneError, PlaneFuture};
    use std::path::{Path, PathBuf};
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
