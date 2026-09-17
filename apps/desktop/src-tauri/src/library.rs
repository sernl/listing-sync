//! The seller's own files, kept on the seller's own machine.
//!
//! An import reads a resource's bundle off the marketplace under the seller's
//! session, describes it, and drops the bytes: the wire carries a digest, a
//! name and a cover, never the file. That is D1 and it stays that way. What
//! this module adds is a place on this device for those bytes to remain, so
//! the seller can preview or open a file they imported without fetching it
//! from the marketplace again, and so a second machine of theirs can take a
//! copy directly. Nothing here reaches the server.
//!
//! Every file is sealed under a per-device key before it touches the disk,
//! with the file's own digest as the additional authenticated data, so a
//! sealed blob moved under another name fails to open rather than opening as
//! the wrong file. The key comes from the same custody the marketplace
//! session does — the operating system's credential store on a computer, the
//! Keystore-wrapped secret on a phone — under an entry of its own, so
//! forgetting one does not make the other unreadable.
//!
//! The index is one JSON file rewritten atomically on every change. A missing
//! or corrupt index is rebuilt empty and the blobs it no longer names are
//! deleted at open, because a blob nothing describes is a file nothing can
//! show the seller and nothing will ever remove otherwise.

use core::future::Future;
use core::pin::Pin;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tam_secrets::{hex_encode, open_bytes, seal_bytes, Kek, Sealed};
use tam_types::{ContentHash, Marketplace, Timestamp};
use tokio::sync::Mutex;

/// The directory under the application data directory.
pub const LIBRARY_DIR: &str = "library";
/// The index file inside it.
pub const INDEX_FILE: &str = "index.json";
/// The sealed blobs' directory inside it.
pub const BLOBS_DIR: &str = "blobs";
/// The settings file beside the library, under the application data
/// directory rather than inside the library so a library wiped for a corrupt
/// index keeps the seller's choice.
pub const SETTINGS_FILE: &str = "library_settings.json";
/// The entry name the library key is filed under, in whichever store holds
/// the session key on this platform.
pub const KEY_ENTRY: &str = "teachouse-library-key";
/// The file the transfer endpoint's node key is sealed in, inside the
/// library directory, so the machine keeps one node identity for the life
/// of the library and the server's record of it stays true.
pub const NODE_KEY_FILE: &str = "node-key.sealed";

/// One kept file, as the index records it and the console lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub hash: ContentHash,
    pub file_name: String,
    pub content_type: String,
    pub byte_len: u64,
    pub marketplace: Marketplace,
    /// The marketplace's own identifier for the resource the file came from.
    pub resource: String,
    pub kept_at: Timestamp,
    /// Kept whatever the seller's `keep_originals` setting says.
    pub pinned: bool,
}

/// The seller's choices about the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibrarySettings {
    /// Whether an import keeps the original file here. On by default: the
    /// bytes are the seller's, on the seller's machine, and keeping them is
    /// what makes a preview possible without a second marketplace read.
    pub keep_originals: bool,
    /// Files kept whatever `keep_originals` says, by digest hex.
    #[serde(default)]
    pub pinned: Vec<String>,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            keep_originals: true,
            pinned: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryError {
    /// The disk refused, with the operating system's own sentence.
    Io(String),
    /// The sealed blob did not open under this device's key, or its bytes do
    /// not hash to the name they were filed under.
    Tampered(ContentHash),
    /// The index or a settings file did not parse.
    Codec(String),
}

impl core::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(why) => write!(f, "the library could not be read or written: {why}"),
            Self::Tampered(hash) => write!(
                f,
                "the kept file {} did not open as the file it was filed as",
                hex_encode(&hash.0)
            ),
            Self::Codec(why) => write!(f, "the library index could not be read: {why}"),
        }
    }
}

impl core::error::Error for LibraryError {}

impl From<std::io::Error> for LibraryError {
    fn from(why: std::io::Error) -> Self {
        Self::Io(why.to_string())
    }
}

/// The index file's shape.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    entries: Vec<LibraryEntry>,
}

/// One blob's sealed envelope on disk. A mirror of [`Sealed`], as
/// `session::encrypted` keeps one, because `tam-secrets` carries no serde.
#[derive(Serialize, Deserialize)]
struct Envelope {
    key_version: i32,
    wrapped_dek: Vec<u8>,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

impl From<Sealed> for Envelope {
    fn from(sealed: Sealed) -> Self {
        Self {
            key_version: sealed.key_version,
            wrapped_dek: sealed.wrapped_dek,
            nonce: sealed.nonce,
            ciphertext: sealed.ciphertext,
        }
    }
}

impl From<Envelope> for Sealed {
    fn from(envelope: Envelope) -> Self {
        Self {
            key_version: envelope.key_version,
            wrapped_dek: envelope.wrapped_dek,
            nonce: envelope.nonce,
            ciphertext: envelope.ciphertext,
        }
    }
}

pub struct Library {
    root: PathBuf,
    /// Where the settings file lives: the data directory, beside the
    /// library rather than inside it, so a library rebuilt for a corrupt
    /// index keeps the seller's choice.
    settings_path: PathBuf,
    key: Kek,
    /// The index, in memory, under one lock: a keep and a remove racing
    /// would otherwise each rewrite the file from what they read.
    index: Mutex<HashMap<ContentHash, LibraryEntry>>,
    settings: Mutex<LibrarySettings>,
}

/// Prints the root and nothing about the key.
impl core::fmt::Debug for Library {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Library")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// The hex a digest is filed under, which is also the AAD its blob is
/// sealed with.
fn hex_of(hash: ContentHash) -> String {
    hex_encode(&hash.0)
}

/// A digest from its hex spelling, or `None` for anything that is not one.
pub fn hash_from_hex(text: &str) -> Option<ContentHash> {
    if text.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let pair = core::str::from_utf8(pair).ok()?;
        bytes[index] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(ContentHash(bytes))
}

/// Writes a file through a sibling temporary name and a rename, so a reader
/// never sees a partial file and a crash mid-write leaves the old one.
async fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// The same write, blocking, for the one caller that is itself blocking:
/// `open`, which [`LibrarySlot`] calls on the task that asked for the
/// library.
fn write_atomically_blocking(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn encode_index(entries: Vec<LibraryEntry>) -> Result<Vec<u8>, LibraryError> {
    serde_json::to_vec_pretty(&Index { entries })
        .map_err(|why| LibraryError::Codec(why.to_string()))
}

impl Library {
    /// Opens the library under `data_dir`, creating it where none is.
    ///
    /// Blocking, and called from [`LibrarySlot::get`] on whichever task
    /// needed the library: one index this process wrote and one bounded
    /// directory listing. A missing or unreadable index is rebuilt empty and
    /// logged, and every blob the index does not name is deleted, so the
    /// directory holds nothing the seller cannot see.
    #[expect(
        clippy::disallowed_methods,
        reason = "the ban targets upload payloads, which are bounded but not small; the index \
                  is a JSON list this process wrote itself"
    )]
    pub fn open(data_dir: &Path, key: Kek) -> Result<Self, LibraryError> {
        let root = data_dir.join(LIBRARY_DIR);
        std::fs::create_dir_all(root.join(BLOBS_DIR))?;
        let index = match std::fs::read_to_string(root.join(INDEX_FILE)) {
            Ok(text) => match serde_json::from_str::<Index>(&text) {
                Ok(index) => index.entries,
                Err(why) => {
                    eprintln!("the library index did not parse and is rebuilt empty: {why}");
                    Vec::new()
                }
            },
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(why) => return Err(why.into()),
        };
        let held: HashMap<ContentHash, LibraryEntry> =
            index.into_iter().map(|entry| (entry.hash, entry)).collect();
        // Every blob the index does not name goes.
        for entry in std::fs::read_dir(root.join(BLOBS_DIR))? {
            let entry = entry?;
            let name = entry.file_name();
            let known = name
                .to_string_lossy()
                .strip_suffix(".sealed")
                .and_then(hash_from_hex)
                .is_some_and(|hash| held.contains_key(&hash));
            if !known {
                std::fs::remove_file(entry.path())?;
            }
        }
        write_atomically_blocking(
            &root.join(INDEX_FILE),
            &encode_index(held.values().cloned().collect())?,
        )?;
        let settings_path = data_dir.join(SETTINGS_FILE);
        let settings = load_settings(&settings_path).unwrap_or_else(|why| {
            eprintln!("the library settings did not parse and are reset: {why}");
            LibrarySettings::default()
        });
        Ok(Self {
            root,
            settings_path,
            key,
            index: Mutex::new(held),
            settings: Mutex::new(settings),
        })
    }

    fn blob_path(&self, hash: ContentHash) -> PathBuf {
        self.root
            .join(BLOBS_DIR)
            .join(format!("{}.sealed", hex_of(hash)))
    }

    /// Rewrites the index from what is held.
    async fn persist(&self) -> Result<(), LibraryError> {
        let entries: Vec<LibraryEntry> = self.index.lock().await.values().cloned().collect();
        write_atomically(&self.root.join(INDEX_FILE), &encode_index(entries)?).await
    }

    /// Files one entry's bytes, sealed, and records it.
    ///
    /// The bytes are hashed here rather than trusted: an entry naming one
    /// digest over bytes that hash to another would file a blob no read
    /// could ever open. A second keep of a hash already held updates the
    /// resource and the instant and writes no bytes.
    pub async fn keep(&self, entry: LibraryEntry, bytes: &[u8]) -> Result<(), LibraryError> {
        let actual = ContentHash(*blake3::hash(bytes).as_bytes());
        if actual != entry.hash {
            return Err(LibraryError::Tampered(entry.hash));
        }
        let mut index = self.index.lock().await;
        if let Some(held) = index.get_mut(&entry.hash) {
            held.resource = entry.resource;
            held.kept_at = entry.kept_at;
            held.pinned = held.pinned || entry.pinned;
        } else {
            let sealed = seal_bytes(&self.key, hex_of(entry.hash).as_bytes(), bytes)
                .map_err(|_| LibraryError::Io("the cipher refused to seal".to_owned()))?;
            let encoded = serde_json::to_vec(&Envelope::from(sealed))
                .map_err(|why| LibraryError::Codec(why.to_string()))?;
            write_atomically(&self.blob_path(entry.hash), &encoded).await?;
            index.insert(entry.hash, entry);
        }
        drop(index);
        self.persist().await
    }

    /// The plaintext of one kept file, verified against its digest, or
    /// `None` where nothing is kept under that hash.
    pub async fn read(&self, hash: ContentHash) -> Result<Option<Vec<u8>>, LibraryError> {
        if !self.index.lock().await.contains_key(&hash) {
            return Ok(None);
        }
        let encoded = match tokio::fs::read(self.blob_path(hash)).await {
            Ok(bytes) => bytes,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(why) => return Err(why.into()),
        };
        let envelope: Envelope =
            serde_json::from_slice(&encoded).map_err(|why| LibraryError::Codec(why.to_string()))?;
        let plaintext = open_bytes(&self.key, hex_of(hash).as_bytes(), &envelope.into())
            .map_err(|_| LibraryError::Tampered(hash))?;
        if ContentHash(*blake3::hash(&plaintext).as_bytes()) != hash {
            return Err(LibraryError::Tampered(hash));
        }
        Ok(Some(plaintext))
    }

    /// Removes one kept file and its index row. Removing what is not there
    /// is success.
    pub async fn remove(&self, hash: ContentHash) -> Result<(), LibraryError> {
        self.index.lock().await.remove(&hash);
        match tokio::fs::remove_file(self.blob_path(hash)).await {
            Ok(()) => {}
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => return Err(why.into()),
        }
        self.persist().await
    }

    /// Marks or unmarks one kept file as pinned.
    pub async fn pin(&self, hash: ContentHash, pinned: bool) -> Result<(), LibraryError> {
        if let Some(entry) = self.index.lock().await.get_mut(&hash) {
            entry.pinned = pinned;
        }
        self.persist().await
    }

    /// Every kept file, newest first.
    pub async fn entries(&self) -> Vec<LibraryEntry> {
        let mut entries: Vec<LibraryEntry> = self.index.lock().await.values().cloned().collect();
        entries.sort_by_key(|entry| core::cmp::Reverse(entry.kept_at));
        entries
    }

    /// Whether a file is kept under this hash.
    pub async fn holds(&self, hash: ContentHash) -> bool {
        self.index.lock().await.contains_key(&hash)
    }

    /// The bytes kept, as the seller would count them: the plaintext sizes,
    /// not the sealed files'.
    pub async fn usage(&self) -> u64 {
        self.index
            .lock()
            .await
            .values()
            .map(|entry| entry.byte_len)
            .sum()
    }
}

/// The seller's settings, from the file beside the library or the default.
#[expect(
    clippy::disallowed_methods,
    reason = "the ban targets upload payloads, which are bounded but not small; the settings \
              file is two fields this process wrote itself"
)]
fn load_settings(path: &Path) -> Result<LibrarySettings, LibraryError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|why| LibraryError::Codec(why.to_string())),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(LibrarySettings::default()),
        Err(why) => Err(why.into()),
    }
}

impl Library {
    /// The seller's choices, as they stand.
    pub async fn settings(&self) -> LibrarySettings {
        self.settings.lock().await.clone()
    }

    /// Records whether imports keep their originals here.
    pub async fn set_keep_originals(&self, keep: bool) -> Result<LibrarySettings, LibraryError> {
        let mut settings = self.settings.lock().await;
        settings.keep_originals = keep;
        let encoded = serde_json::to_vec_pretty(&*settings)
            .map_err(|why| LibraryError::Codec(why.to_string()))?;
        write_atomically(&self.settings_path, &encoded).await?;
        Ok(settings.clone())
    }

    /// Keeps an import's original where the seller's setting says to, and
    /// answers whether it did. A file already pinned is kept whatever the
    /// setting says.
    pub async fn keep_original(
        &self,
        entry: LibraryEntry,
        bytes: &[u8],
    ) -> Result<bool, LibraryError> {
        let wanted = self.settings.lock().await.keep_originals || self.holds(entry.hash).await;
        if !wanted {
            return Ok(false);
        }
        self.keep(entry, bytes).await.map(|()| true)
    }

    /// The transfer endpoint's node key: thirty-two random bytes sealed
    /// under the library key, created on first use. Kept beside the blobs
    /// so the node identity the server records outlives restarts.
    pub async fn node_secret(&self) -> Result<[u8; 32], LibraryError> {
        let path = self.root.join(NODE_KEY_FILE);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let envelope: Envelope = serde_json::from_slice(&bytes)
                    .map_err(|why| LibraryError::Codec(why.to_string()))?;
                let secret = open_bytes(&self.key, NODE_KEY_FILE.as_bytes(), &envelope.into())
                    .map_err(|why| LibraryError::Codec(why.to_string()))?;
                <[u8; 32]>::try_from(secret.as_slice())
                    .map_err(|_| LibraryError::Codec("the node key is not 32 bytes".to_owned()))
            }
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {
                let ContentHash(secret) =
                    hash_from_hex(&tam_secrets::random_token()).ok_or_else(|| {
                        LibraryError::Codec("the fresh node key is not one".to_owned())
                    })?;
                let sealed = seal_bytes(&self.key, NODE_KEY_FILE.as_bytes(), &secret)
                    .map_err(|_| LibraryError::Io("the cipher refused to seal".to_owned()))?;
                let encoded = serde_json::to_vec(&Envelope::from(sealed))
                    .map_err(|why| LibraryError::Codec(why.to_string()))?;
                write_atomically(&path, &encoded).await?;
                Ok(secret)
            }
            Err(why) => Err(why.into()),
        }
    }
}

/// The per-device library key from the operating system's credential store,
/// created on first use. The same service the session entries live under,
/// so a human reading their own keychain sees one application.
#[cfg(not(target_os = "android"))]
pub fn keychain_library_key(service: &str) -> Result<Kek, LibraryError> {
    let entry =
        keyring::Entry::new(service, KEY_ENTRY).map_err(|why| LibraryError::Io(why.to_string()))?;
    match entry.get_password() {
        Ok(hex) => hash_from_hex(hex.trim())
            .and_then(|ContentHash(bytes)| Kek::from_bytes(&bytes).ok())
            .ok_or_else(|| LibraryError::Codec("the library key entry is not a key".to_owned())),
        Err(keyring::Error::NoEntry) => {
            let fresh = tam_secrets::random_token();
            entry
                .set_password(&fresh)
                .map_err(|why| LibraryError::Io(why.to_string()))?;
            hash_from_hex(&fresh)
                .and_then(|ContentHash(bytes)| Kek::from_bytes(&bytes).ok())
                .ok_or_else(|| LibraryError::Codec("the fresh library key is not a key".to_owned()))
        }
        Err(why) => Err(LibraryError::Io(why.to_string())),
    }
}

/// The per-device library key on a phone: thirty-two random bytes sealed
/// under the Keystore-held session key into a file of their own, created on
/// first use. The same store the session file lives in, under its own name.
///
/// Nothing is written until the sealing has succeeded, and the write itself
/// goes through a temporary name and a rename. That order matters while the
/// phone is locked: the Keystore refuses the wrap, this returns the refusal,
/// and the file is either the one it already was or absent — never a
/// half-written envelope and never one sealed under something else. The next
/// attempt, after the seller unlocks the phone, creates the key as a first
/// use still.
#[cfg(target_os = "android")]
pub async fn sealed_library_key(
    data_dir: &Path,
    keys: &dyn crate::session::encrypted::DeviceKeySource,
) -> Result<Kek, LibraryError> {
    let path = data_dir.join(format!("{KEY_ENTRY}.sealed"));
    let device_key = keys
        .obtain()
        .map_err(|why| LibraryError::Io(why.to_string()))?;
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let envelope: Envelope = serde_json::from_slice(&bytes)
                .map_err(|why| LibraryError::Codec(why.to_string()))?;
            let secret = open_bytes(&device_key, KEY_ENTRY.as_bytes(), &envelope.into())
                .map_err(|why| LibraryError::Codec(why.to_string()))?;
            Kek::from_bytes(&secret).map_err(|why| LibraryError::Codec(why.to_string()))
        }
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => {
            let fresh = tam_secrets::random_token();
            let ContentHash(secret) = hash_from_hex(&fresh).ok_or_else(|| {
                LibraryError::Codec("the fresh library key is not a key".to_owned())
            })?;
            let sealed = seal_bytes(&device_key, KEY_ENTRY.as_bytes(), &secret)
                .map_err(|_| LibraryError::Io("the cipher refused to seal".to_owned()))?;
            let encoded = serde_json::to_vec(&Envelope::from(sealed))
                .map_err(|why| LibraryError::Codec(why.to_string()))?;
            tokio::fs::create_dir_all(data_dir).await?;
            write_atomically(&path, &encoded).await?;
            Kek::from_bytes(&secret).map_err(|why| LibraryError::Codec(why.to_string()))
        }
        Err(why) => Err(why.into()),
    }
}

/// The library key, as a future, so both platforms are asked for it the same
/// way: the phone's sealed file is read asynchronously and the desktop's
/// credential store is read synchronously.
pub type KeyFuture<'a> = Pin<Box<dyn Future<Output = Result<Kek, LibraryError>> + Send + 'a>>;

/// Where this machine's library key comes from.
///
/// A seam rather than a direct call at each use, because the key is not
/// always obtainable and the answer changes over the life of the process. A
/// phone's Keystore refuses every operation while the keyguard is showing —
/// the wrapping key is deliberately generated with
/// `setUnlockedDeviceRequired(true)` — so an application launched from the
/// lock screen cannot have the key yet and can have it a minute later. The
/// slot below is what turns that into a retry; this is what it retries
/// through.
pub trait LibraryKeySource: Send + Sync {
    /// This machine's library key, created on first use.
    fn library_key(&self) -> KeyFuture<'_>;
}

/// The operating system's credential store, as a key source.
#[cfg(not(target_os = "android"))]
pub struct KeychainKey {
    service: String,
}

#[cfg(not(target_os = "android"))]
impl KeychainKey {
    /// The key filed under [`KEY_ENTRY`] in `service`.
    #[must_use]
    pub fn under(service: &str) -> Self {
        Self {
            service: service.to_owned(),
        }
    }
}

#[cfg(not(target_os = "android"))]
impl LibraryKeySource for KeychainKey {
    fn library_key(&self) -> KeyFuture<'_> {
        // Synchronous underneath, and wrapped rather than moved to a blocking
        // pool: it is one credential-store lookup, which is what every
        // session read on this platform already does inline.
        Box::pin(async move { keychain_library_key(&self.service) })
    }
}

/// The Keystore-wrapped file beside the session's, as a key source.
#[cfg(target_os = "android")]
pub struct SealedKey {
    data_dir: PathBuf,
    keys: Arc<dyn crate::session::encrypted::DeviceKeySource>,
}

#[cfg(target_os = "android")]
impl SealedKey {
    /// The key sealed under the device key in `data_dir`.
    #[must_use]
    pub fn in_data_dir(
        data_dir: &Path,
        keys: Arc<dyn crate::session::encrypted::DeviceKeySource>,
    ) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            keys,
        }
    }
}

#[cfg(target_os = "android")]
impl LibraryKeySource for SealedKey {
    fn library_key(&self) -> KeyFuture<'_> {
        Box::pin(sealed_library_key(&self.data_dir, self.keys.as_ref()))
    }
}

/// This machine's library, opened on first use and retried until it opens.
///
/// Held by everything that needs the library — the state the console's
/// commands read, the work source, an import pass, the payload cache, the
/// sync schedule — and asked at the moment of use rather than at start-up.
/// That is the whole reason it exists. The library used to be opened once in
/// the application's set-up closure, and whatever came of that one attempt
/// was handed out for the life of the process: a phone launched while locked
/// could not obtain the key, so every import in that process kept no
/// original, the console said this machine keeps no files, and nothing
/// changed when the seller unlocked the phone until they restarted the
/// application. Asking the slot makes "can this machine keep files" a
/// question about now.
pub struct LibrarySlot {
    data_dir: PathBuf,
    keys: Arc<dyn LibraryKeySource>,
    /// The opened library, once it has opened. A lock around an option
    /// rather than a `OnceCell`, because a failed attempt has to be
    /// repeatable; and the lock is held across the attempt, so two consumers
    /// asking at once open one library rather than two.
    open: Mutex<Option<Arc<Library>>>,
}

/// Prints the directory and nothing about the key, as [`Library`]'s own does.
impl core::fmt::Debug for LibrarySlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LibrarySlot")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl LibrarySlot {
    #[must_use]
    pub fn new(data_dir: &Path, keys: Arc<dyn LibraryKeySource>) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            keys,
            open: Mutex::new(None),
        }
    }

    /// The library, opening it where it is not open yet.
    ///
    /// Cheap once it has opened: one lock and a clone of the handle. Before
    /// then each call is a fresh attempt at the key and at the directory, and
    /// a refusal is returned to the caller to report as that caller reports
    /// things — a command answers the seller, an import logs and goes on
    /// describing the resource — rather than remembered here as a verdict.
    pub async fn get(&self) -> Result<Arc<Library>, LibraryError> {
        let mut open = self.open.lock().await;
        if let Some(library) = open.as_ref() {
            return Ok(Arc::clone(library));
        }
        let key = self.keys.library_key().await?;
        // `Library::open` blocks, and stays blocking: it reads one JSON index
        // this process wrote and lists a directory bounded by the entries
        // that index names. Moving it to a blocking pool would buy nothing
        // but a join failure with nothing to say to the seller.
        let library = Arc::new(Library::open(&self.data_dir, key)?);
        *open = Some(Arc::clone(&library));
        // The lock goes before the handle is handed out, as `keep` releases
        // the index before it persists: nothing after this line needs the
        // slot, and the attempt the next caller makes should not queue behind
        // this one's return.
        drop(open);
        Ok(library)
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "the ban targets upload payloads; these tests read back the small files they wrote"
)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    fn kek() -> Kek {
        Kek::from_bytes(&[0x5A; 32]).expect("32 bytes is a key")
    }

    fn entry(bytes: &[u8], resource: &str) -> LibraryEntry {
        LibraryEntry {
            hash: ContentHash(*blake3::hash(bytes).as_bytes()),
            file_name: "worksheet.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            byte_len: bytes.len() as u64,
            marketplace: Marketplace::Tpt,
            resource: resource.to_owned(),
            kept_at: Timestamp(1_000),
            pinned: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "teachouse-library-{name}-{}-{}",
            std::process::id(),
            tam_secrets::random_token()
        ));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        dir
    }

    #[tokio::test]
    async fn a_kept_file_reads_back_and_is_counted() {
        let dir = scratch("roundtrip");
        let library = Library::open(&dir, kek()).expect("opens");
        let bytes = b"%PDF-1.4 hello";
        library
            .keep(entry(bytes, "101"), bytes)
            .await
            .expect("keeps");
        let hash = entry(bytes, "101").hash;
        assert_eq!(
            library.read(hash).await.expect("reads"),
            Some(bytes.to_vec())
        );
        assert_eq!(library.usage().await, bytes.len() as u64);
        assert!(library.holds(hash).await);
        // The plaintext is not on disk.
        let sealed = std::fs::read(library.blob_path(hash)).expect("the blob is there");
        assert!(
            !sealed.windows(bytes.len()).any(|window| window == bytes),
            "the kept file is sealed, not written in the clear"
        );
        // A second keep of the same hash updates the resource without a second blob.
        library
            .keep(entry(bytes, "202"), bytes)
            .await
            .expect("keeps again");
        let entries = library.entries().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].resource, "202");
    }

    #[tokio::test]
    async fn a_reopened_library_reads_its_index_and_a_tampered_blob_is_refused() {
        let dir = scratch("reopen");
        let bytes = b"%PDF-1.4 kept";
        let hash = entry(bytes, "1").hash;
        {
            let library = Library::open(&dir, kek()).expect("opens");
            library.keep(entry(bytes, "1"), bytes).await.expect("keeps");
        }
        let library = Library::open(&dir, kek()).expect("reopens");
        assert_eq!(
            library.entries().await.len(),
            1,
            "the index survives a reopen"
        );
        assert_eq!(
            library.read(hash).await.expect("reads"),
            Some(bytes.to_vec())
        );

        // Flip a byte of the ciphertext.
        let path = library.blob_path(hash);
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("blob")).expect("json");
        let first = envelope["ciphertext"][0].as_u64().expect("a byte") ^ 0xFF;
        envelope["ciphertext"][0] = serde_json::Value::from(first);
        std::fs::write(&path, serde_json::to_vec(&envelope).expect("json")).expect("write");
        assert_eq!(
            library.read(hash).await,
            Err(LibraryError::Tampered(hash)),
            "a blob that does not authenticate is refused, not returned"
        );
    }

    #[tokio::test]
    async fn a_removed_file_is_gone_from_the_index_and_the_disk() {
        let dir = scratch("remove");
        let library = Library::open(&dir, kek()).expect("opens");
        let bytes = b"bytes to remove";
        let hash = entry(bytes, "1").hash;
        library.keep(entry(bytes, "1"), bytes).await.expect("keeps");
        library.remove(hash).await.expect("removes");
        assert!(!library.blob_path(hash).exists());
        assert!(library.entries().await.is_empty());
        assert_eq!(library.read(hash).await.expect("reads"), None);
        library.remove(hash).await.expect("removing again is fine");
    }

    #[tokio::test]
    async fn a_corrupt_index_is_rebuilt_empty_and_orphan_blobs_are_deleted() {
        let dir = scratch("corrupt");
        let bytes = b"orphaned";
        let hash = entry(bytes, "1").hash;
        {
            let library = Library::open(&dir, kek()).expect("opens");
            library.keep(entry(bytes, "1"), bytes).await.expect("keeps");
        }
        let index = dir.join(LIBRARY_DIR).join(INDEX_FILE);
        std::fs::write(&index, b"{ not json").expect("corrupts");
        let library = Library::open(&dir, kek()).expect("opens despite the corrupt index");
        assert!(library.entries().await.is_empty());
        assert!(
            !library.blob_path(hash).exists(),
            "the orphan blob is deleted"
        );
        let rebuilt: Index =
            serde_json::from_slice(&std::fs::read(&index).expect("index")).expect("json");
        assert!(rebuilt.entries.is_empty());
    }

    #[tokio::test]
    async fn bytes_that_do_not_hash_to_the_entry_are_refused() {
        let dir = scratch("mismatch");
        let library = Library::open(&dir, kek()).expect("opens");
        let mut wrong = entry(b"one", "1");
        wrong.hash = entry(b"two", "1").hash;
        assert!(matches!(
            library.keep(wrong, b"one").await,
            Err(LibraryError::Tampered(_))
        ));
        assert!(library.entries().await.is_empty());
    }

    #[tokio::test]
    async fn keeping_originals_is_on_by_default_and_the_choice_survives_a_reopen() {
        let dir = scratch("settings");
        let bytes = b"an original";
        {
            let library = Library::open(&dir, kek()).expect("opens");
            assert!(library.settings().await.keep_originals);
            assert!(library
                .keep_original(entry(bytes, "1"), bytes)
                .await
                .expect("keeps"));
            library
                .set_keep_originals(false)
                .await
                .expect("the choice is recorded");
        }
        let library = Library::open(&dir, kek()).expect("reopens");
        assert!(!library.settings().await.keep_originals);
        let other = b"another original";
        assert!(
            !library
                .keep_original(entry(other, "2"), other)
                .await
                .expect("answers"),
            "with the setting off, an import keeps nothing new"
        );
        assert!(
            library
                .keep_original(entry(bytes, "3"), bytes)
                .await
                .expect("answers"),
            "but a file already kept goes on being kept"
        );
        assert_eq!(library.entries().await.len(), 1);
    }

    /// A key source that refuses its first caller and answers every caller
    /// after it, which is what a phone's Keystore does across an unlock: the
    /// wrapping key requires an unlocked device, so the operation fails while
    /// the keyguard shows and succeeds once it is gone.
    #[derive(Default)]
    struct LockedOnce {
        asked: AtomicUsize,
    }

    impl LibraryKeySource for LockedOnce {
        fn library_key(&self) -> KeyFuture<'_> {
            Box::pin(async move {
                if self.asked.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(LibraryError::Io(
                        "the session store refused: Keystore operation failed".to_owned(),
                    ));
                }
                Ok(kek())
            })
        }
    }

    #[tokio::test]
    async fn a_library_whose_first_open_failed_opens_on_the_next_ask() {
        let dir = scratch("slot");
        let keys = Arc::new(LockedOnce::default());
        // Method-call syntax for the unsizing coercion, as `lib.rs` does.
        let source: Arc<dyn LibraryKeySource> = keys.clone();
        let slot = LibrarySlot::new(&dir, source);
        assert!(
            slot.get().await.is_err(),
            "a key the platform refuses is a library that cannot open yet"
        );
        let library = slot
            .get()
            .await
            .expect("the ask after the refusal opens the library");
        let bytes = b"%PDF-1.4 kept after the unlock";
        library
            .keep(entry(bytes, "1"), bytes)
            .await
            .expect("an import after the retry keeps its original");
        let again = slot.get().await.expect("stays open");
        assert!(
            Arc::ptr_eq(&library, &again),
            "one library per process, so the index in memory is one index"
        );
        assert_eq!(
            keys.asked.load(Ordering::SeqCst),
            2,
            "an open library is not opened again, so the platform is asked for the key only \
             while there is nothing to hand out"
        );
    }
}
