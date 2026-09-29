//! The [`SessionStore`] every platform keeps a marketplace session in: the
//! record sealed under a key the device holds, in a file under the
//! application's own data directory.
//!
//! One design on every operating system. Only where the thirty-two-byte key
//! comes from differs, and that sits behind [`DeviceKeySource`]: the Android
//! Keystore on a phone (founder decision, 2026-09-03), the operating system's
//! credential store on Windows, macOS and Linux
//! ([`super::keychain::KeychainKey`]). The jar itself never goes into a
//! credential store any more. It used to, on the desktop, and Windows
//! Credential Manager refuses a blob longer than 2560 UTF-16 characters — a
//! TPT or Tes cookie jar is routinely longer, so connecting failed with "the
//! session store refused". A key is 64 characters on every platform.
//!
//! A session a previous version filed in the keychain is moved here the first
//! time it is read: see [`EncryptedSessionStore::migrating_from`].
//!
//! The cipher is portable, the file format is portable, and the test
//! implementation of the key source holds a fixed key, so this module's
//! behaviour is tested on the host rather than on a target no lane builds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tam_secrets::{open_bytes, seal_bytes, Kek, Sealed};
use tam_types::Marketplace;

use super::{entry_key, SessionRecord, SessionStore, StoreError, StoreFuture};

/// The sealed file's name inside the application data directory.
pub const STORE_FILE: &str = "sessions.sealed";

/// Where the sealing key comes from.
///
/// Two methods and no more. `obtain` is called on every operation rather than
/// cached, so a key the platform has destroyed stops working immediately
/// rather than at the next launch. `forget` destroys the key itself and is
/// therefore a device-wide wipe: [`SessionStore::forget`] never calls it,
/// because forgetting one marketplace must not make the others unreadable.
pub trait DeviceKeySource: Send + Sync {
    /// The key-encryption key. Returned as a [`Kek`] rather than as bytes
    /// because `Kek` is `ZeroizeOnDrop`, so the secret does not outlive the
    /// call that used it.
    fn obtain(&self) -> Result<Kek, StoreError>;

    /// Destroys the device's key. Everything sealed under it becomes
    /// permanently unreadable, which is the point: it is how a wipe stops
    /// being a deletion the filesystem might not have honoured.
    fn forget(&self) -> Result<(), StoreError>;
}

/// One record's sealed envelope, as it is written to disk.
///
/// A mirror of [`Sealed`] rather than a serde derive on the type itself:
/// `tam-secrets` carries no serde at all, because its own consumer persists
/// these four values as separate database columns. Nothing secret is in the
/// field names, and the four values are ciphertext, so this struct's `Debug`
/// needs no special handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// The whole file: one envelope per marketplace, filed under the same key the
/// keychain store uses. A `BTreeMap` rather than a `HashMap` so the file's
/// bytes are stable across writes that changed nothing.
type Filed = BTreeMap<String, Envelope>;

pub struct EncryptedSessionStore {
    path: PathBuf,
    keys: Arc<dyn DeviceKeySource>,
    /// Where an earlier version kept sessions, read once per marketplace and
    /// emptied as each is moved into the file.
    legacy: Option<Arc<dyn SessionStore>>,
    /// Held across each read-modify-write of the file. Every marketplace
    /// shares the one file, and two check-ins writing back rotated cookies
    /// at once would otherwise each write a map missing the other's entry.
    writes: tokio::sync::Mutex<()>,
}

/// Prints the path and nothing about the key. The same rule
/// [`super::CookieJar`] follows: this type sits beside a credential, and a
/// derive would carry the key source into any panic message.
impl core::fmt::Debug for EncryptedSessionStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EncryptedSessionStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl EncryptedSessionStore {
    #[must_use]
    pub fn new(path: PathBuf, keys: Arc<dyn DeviceKeySource>) -> Self {
        Self {
            path,
            keys,
            legacy: None,
            writes: tokio::sync::Mutex::new(()),
        }
    }

    /// This store, taking over whatever `legacy` still holds.
    ///
    /// A marketplace the file has no session for is looked up in `legacy`;
    /// one found there is sealed into the file and then removed from
    /// `legacy`, so the move happens once and the file is the only custody
    /// afterwards. Forgetting a marketplace forgets it in both, so a
    /// disconnect cannot leave behind a copy the next read would resurrect.
    #[must_use]
    pub fn migrating_from(mut self, legacy: Arc<dyn SessionStore>) -> Self {
        self.legacy = Some(legacy);
        self
    }

    /// The file beside the device identity: the application's own data
    /// directory, which on Android is its private files directory.
    #[must_use]
    pub fn in_data_dir(data_dir: &Path, keys: Arc<dyn DeviceKeySource>) -> Self {
        Self::new(data_dir.join(STORE_FILE), keys)
    }

    /// What is on disk, or an empty map when nothing is.
    ///
    /// The distinction between absent and unreadable is kept rather than
    /// collapsed: a file that exists but does not parse is an error, never an
    /// empty map, because answering a damaged store with "no sessions" would
    /// make the seller's sessions look like sessions they never had, and the
    /// next write would overwrite what might still be recoverable.
    async fn load(&self) -> Result<Filed, StoreError> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|why| StoreError::Codec(why.to_string()))
            }
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(Filed::new()),
            Err(why) => Err(StoreError::Backend(why.to_string())),
        }
    }

    /// Writes through a sibling temporary name and a rename, so a crash
    /// mid-write leaves the previous file rather than a torn one that
    /// [`Self::load`] would then refuse, taking every marketplace with it.
    async fn store(&self, filed: &Filed) -> Result<(), StoreError> {
        let encoded =
            serde_json::to_vec(filed).map_err(|why| StoreError::Codec(why.to_string()))?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|why| StoreError::Backend(why.to_string()))?;
        }
        let staged = self.path.with_extension("tmp");
        tokio::fs::write(&staged, encoded)
            .await
            .map_err(|why| StoreError::Backend(why.to_string()))?;
        tokio::fs::rename(&staged, &self.path)
            .await
            .map_err(|why| StoreError::Backend(why.to_string()))
    }

    /// The additional authenticated data one marketplace's envelope is bound
    /// to. Binding it means an envelope moved to another marketplace's entry
    /// fails to authenticate rather than opening as the wrong session.
    fn aad(marketplace: Marketplace) -> &'static [u8] {
        entry_key(marketplace).as_bytes()
    }

    async fn write(
        &self,
        marketplace: Marketplace,
        record: Option<&SessionRecord>,
    ) -> Result<(), StoreError> {
        let _held = self.writes.lock().await;
        let mut filed = self.load().await?;
        match record {
            Some(record) => {
                let plaintext =
                    serde_json::to_vec(record).map_err(|why| StoreError::Codec(why.to_string()))?;
                let kek = self.keys.obtain()?;
                // `SealError` has one variant and no `Display`; naming the
                // failure here is more use than its `Debug` spelling.
                let sealed = seal_bytes(&kek, Self::aad(marketplace), &plaintext)
                    .map_err(|_| StoreError::Backend("the cipher refused to seal".to_owned()))?;
                filed.insert(entry_key(marketplace).to_owned(), sealed.into());
            }
            None => {
                filed.remove(entry_key(marketplace));
            }
        }
        self.store(&filed).await
    }

    /// One marketplace's session, or `None` where there is none to have.
    ///
    /// An envelope the device key does not open is `None` too, and logged. The
    /// device key is the only key there is, so an envelope it cannot
    /// authenticate is a session nothing will ever open again: one sealed
    /// before a key reset — a reinstall that dropped the Keystore alias — or a
    /// tampered file, and the two are indistinguishable by design. Answering it
    /// as an error would leave the marketplace showing a fault the seller
    /// cannot act on; answering "no session" lets them sign in again, and the
    /// next write replaces what nothing could have read. That is unlike a
    /// file that fails to parse, which [`Self::load`] keeps as an error
    /// because the bytes may still be recoverable.
    async fn read(&self, marketplace: Marketplace) -> Result<Option<SessionRecord>, StoreError> {
        let filed = self.load().await?;
        let Some(envelope) = filed.get(entry_key(marketplace)).cloned() else {
            return Ok(None);
        };
        let kek = self.keys.obtain()?;
        let Ok(plaintext) = open_bytes(&kek, Self::aad(marketplace), &envelope.into()) else {
            eprintln!(
                "the {} session on this device was sealed under a key this device no longer holds; it is treated as signed out",
                entry_key(marketplace)
            );
            return Ok(None);
        };
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|why| StoreError::Codec(why.to_string()))
    }

    /// The file's session, or the legacy store's moved into the file.
    ///
    /// The legacy copy is removed only after the file holds it. A removal
    /// that fails is logged rather than returned: the session is safe in the
    /// file, which is read first from now on, and the next forget retries it.
    async fn read_or_migrate(
        &self,
        marketplace: Marketplace,
    ) -> Result<Option<SessionRecord>, StoreError> {
        if let Some(record) = self.read(marketplace).await? {
            return Ok(Some(record));
        }
        let Some(legacy) = &self.legacy else {
            return Ok(None);
        };
        let Some(record) = legacy.get(marketplace).await? else {
            return Ok(None);
        };
        self.write(marketplace, Some(&record)).await?;
        if let Err(why) = legacy.forget(marketplace).await {
            eprintln!(
                "the {} session moved into the sealed file, but its old copy could not be removed: {why}",
                entry_key(marketplace)
            );
        }
        Ok(Some(record))
    }

    async fn remove(&self, marketplace: Marketplace) -> Result<(), StoreError> {
        self.write(marketplace, None).await?;
        match &self.legacy {
            Some(legacy) => legacy.forget(marketplace).await,
            None => Ok(()),
        }
    }

    /// Destroys the device key and the file together.
    ///
    /// Separate from [`SessionStore::forget`] and never called by it. This is
    /// the whole-device wipe: without the key nothing that survived the file's
    /// deletion can be read, which is what makes the wipe a fact rather than a
    /// deletion the filesystem might not have honoured.
    pub async fn wipe(&self) -> Result<(), StoreError> {
        let _held = self.writes.lock().await;
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(why) => Err(StoreError::Backend(why.to_string())),
        }?;
        self.keys.forget()
    }
}

impl SessionStore for EncryptedSessionStore {
    fn put<'a>(&'a self, record: &'a SessionRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move { self.write(record.marketplace, Some(record)).await })
    }

    fn get(&self, marketplace: Marketplace) -> StoreFuture<'_, Option<SessionRecord>> {
        Box::pin(async move { self.read_or_migrate(marketplace).await })
    }

    /// Forgetting a marketplace that was never stored succeeds, because the
    /// revocation wipe runs this over every marketplace and must not report a
    /// failure for one the seller never connected.
    fn forget(&self, marketplace: Marketplace) -> StoreFuture<'_, ()> {
        Box::pin(async move { self.remove(marketplace).await })
    }
}

/// A [`DeviceKeySource`] holding one key in memory. For tests, and for no
/// other purpose: a key that does not survive the process cannot keep a
/// session, so this is never the store the application selects.
#[derive(Debug, Clone)]
pub struct FixedKey([u8; 32]);

impl FixedKey {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl DeviceKeySource for FixedKey {
    fn obtain(&self) -> Result<Kek, StoreError> {
        Kek::from_bytes(&self.0).map_err(|why| StoreError::Backend(why.to_string()))
    }

    fn forget(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceKeySource, EncryptedSessionStore, FixedKey, StoreError, STORE_FILE};
    use crate::device::DeviceId;
    use crate::session::memory::MemorySessionStore;
    use crate::session::tests::a_record;
    use crate::session::{Cookie, CookieJar, SessionStore};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tam_secrets::Kek;
    use tam_types::Marketplace;

    /// The workspace convention for a scratch directory: `temp_dir` plus a
    /// fresh identifier, rather than a `tempfile` dependency.
    fn a_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tam-desktop-sealed-{}", DeviceId::generate()));
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        dir
    }

    fn a_store(dir: &Path, byte: u8) -> EncryptedSessionStore {
        EncryptedSessionStore::in_data_dir(dir, Arc::new(FixedKey::new([byte; 32])))
    }

    #[tokio::test]
    async fn a_session_survives_being_written_and_reopened() {
        let dir = a_dir();
        let record = a_record(Marketplace::Tpt);

        let store = a_store(&dir, 3);
        assert_eq!(store.get(Marketplace::Tpt).await, Ok(None));
        store.put(&record).await.expect("the write");

        // A second store over the same file, which is what a relaunch is:
        // reading back through the first would prove only that it remembered.
        let reopened = a_store(&dir, 3);
        assert_eq!(
            reopened.get(Marketplace::Tpt).await,
            Ok(Some(record)),
            "the file is the custody; a record that does not survive a reopen was never kept"
        );
        assert_eq!(
            reopened.get(Marketplace::Tes).await,
            Ok(None),
            "one marketplace's entry must not answer for another's"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The Windows defect: Credential Manager refuses a blob over 2560
    /// UTF-16 characters, and a real TPT jar is longer. The file has no such
    /// limit, and the jar is ciphertext in it.
    #[tokio::test]
    async fn a_jar_longer_than_any_credential_store_allows_round_trips_sealed() {
        let dir = a_dir();
        let mut record = a_record(Marketplace::Tpt);
        let long_value = "v".repeat(6_000);
        record.jar = CookieJar::new(vec![Cookie {
            name: "PHPSESSID".to_owned(),
            value: long_value.clone(),
        }]);

        a_store(&dir, 7).put(&record).await.expect("the write");

        assert_eq!(
            a_store(&dir, 7).get(Marketplace::Tpt).await,
            Ok(Some(record))
        );
        let raw = tokio::fs::read_to_string(dir.join(STORE_FILE))
            .await
            .expect("the file");
        assert!(
            !raw.contains(&long_value) && !raw.contains("PHPSESSID"),
            "the file holds ciphertext; a cookie in the clear is the keychain's custody lost"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An upgrade from the version that filed sessions in the keychain: the
    /// seller stays signed in, and the keychain copy is gone afterwards.
    #[tokio::test]
    async fn a_session_in_the_legacy_store_moves_into_the_file_on_first_read() {
        let dir = a_dir();
        let legacy = Arc::new(MemorySessionStore::new());
        let record = a_record(Marketplace::Tes);
        legacy.put(&record).await.expect("the old version's write");

        let store = a_store(&dir, 10).migrating_from(legacy.clone());
        assert_eq!(
            store.get(Marketplace::Tes).await,
            Ok(Some(record.clone())),
            "an upgrade must not sign the seller out"
        );
        assert_eq!(
            legacy.get(Marketplace::Tes).await,
            Ok(None),
            "the move happens once; a copy left behind is a second custody"
        );
        assert_eq!(
            a_store(&dir, 10).get(Marketplace::Tes).await,
            Ok(Some(record)),
            "the file now holds it without the legacy store"
        );
        assert_eq!(
            store.get(Marketplace::Tpt).await,
            Ok(None),
            "a marketplace neither store holds is simply not connected"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_file_wins_over_a_stale_legacy_copy() {
        let dir = a_dir();
        let legacy = Arc::new(MemorySessionStore::new());
        let mut stale = a_record(Marketplace::Tpt);
        stale.account_label = Some("stale".to_owned());
        legacy.put(&stale).await.expect("write");
        let store = a_store(&dir, 11).migrating_from(legacy.clone());
        let fresh = a_record(Marketplace::Tpt);
        store.put(&fresh).await.expect("write");

        assert_eq!(store.get(Marketplace::Tpt).await, Ok(Some(fresh)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn forgetting_a_marketplace_forgets_the_legacy_copy_too() {
        let dir = a_dir();
        let legacy = Arc::new(MemorySessionStore::new());
        legacy
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("write");
        let store = a_store(&dir, 12).migrating_from(legacy.clone());

        store.forget(Marketplace::Tpt).await.expect("forget");

        assert_eq!(
            store.get(Marketplace::Tpt).await,
            Ok(None),
            "a disconnect that left the keychain copy would be undone by the next read"
        );
        assert_eq!(legacy.get(Marketplace::Tpt).await, Ok(None));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn concurrent_writes_for_different_marketplaces_both_land() {
        let dir = a_dir();
        let store = a_store(&dir, 13);
        let (tes, tpt) = (a_record(Marketplace::Tes), a_record(Marketplace::Tpt));
        let (first, second) = tokio::join!(store.put(&tes), store.put(&tpt));
        first.expect("write");
        second.expect("write");

        assert_eq!(store.get(Marketplace::Tes).await, Ok(Some(tes)));
        assert_eq!(store.get(Marketplace::Tpt).await, Ok(Some(tpt)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_file_written_under_one_secret_does_not_open_under_another() {
        let dir = a_dir();
        a_store(&dir, 1)
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("the write");

        let intruder = a_store(&dir, 2);
        assert_eq!(
            intruder.get(Marketplace::Tpt).await,
            Ok(None),
            "the device key is the whole of the secrecy; under another key the file holds no session"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_envelope_moved_to_another_marketplace_does_not_authenticate() {
        let dir = a_dir();
        let store = a_store(&dir, 4);
        store
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("the write");

        // Swap the two entries' keys on disk, which is the attack the AAD
        // exists to stop: without it the Tes entry would open as a TPT
        // session.
        let path = dir.join(STORE_FILE);
        let raw = tokio::fs::read_to_string(&path).await.expect("the file");
        let swapped = raw.replace("session.tpt", "session.tes");
        std::fs::write(&path, swapped).expect("the swap");

        assert_eq!(
            store.get(Marketplace::Tes).await,
            Ok(None),
            "an envelope bound to one marketplace must not open under another's key"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_forgotten_session_is_gone_and_the_others_remain() {
        let dir = a_dir();
        let store = a_store(&dir, 5);
        store.put(&a_record(Marketplace::Tes)).await.expect("write");
        store.put(&a_record(Marketplace::Tpt)).await.expect("write");
        store.forget(Marketplace::Tes).await.expect("forget");

        let reopened = a_store(&dir, 5);
        assert_eq!(
            reopened.get(Marketplace::Tes).await,
            Ok(None),
            "forget must reach the file, because it is the seller's disconnect"
        );
        assert!(
            reopened
                .get(Marketplace::Tpt)
                .await
                .expect("read")
                .is_some(),
            "forgetting one marketplace must leave the others readable"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn forgetting_a_marketplace_that_was_never_stored_succeeds() {
        let dir = a_dir();
        assert_eq!(
            a_store(&dir, 9).forget(Marketplace::Tpt).await,
            Ok(()),
            "the revocation wipe runs over every marketplace and must not fail on an unused one"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_damaged_file_is_an_error_rather_than_an_empty_store() {
        let dir = a_dir();
        let store = a_store(&dir, 6);
        store.put(&a_record(Marketplace::Tpt)).await.expect("write");
        std::fs::write(dir.join(STORE_FILE), b"{not json").expect("the damage");

        assert!(
            matches!(store.get(Marketplace::Tpt).await, Err(StoreError::Codec(_))),
            "a store that answered a damaged file with 'no sessions' would overwrite it next write"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The lowercase hex spelling of some bytes.
    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        bytes
            .iter()
            .flat_map(|&byte| [byte >> 4, byte & 0x0F])
            .map(|nibble| char::from(DIGITS[usize::from(nibble)]))
            .collect()
    }

    /// The standard-alphabet base64 spelling of some bytes, padded.
    ///
    /// Hand-rolled because adding a dependency is a founder decision, and this
    /// is one encoding of one array in one test.
    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut spelled = String::new();
        for chunk in bytes.chunks(3) {
            let mut block = [0_u8; 3];
            block
                .iter_mut()
                .zip(chunk)
                .for_each(|(slot, byte)| *slot = *byte);
            let packed = (usize::from(block[0]) << 16)
                | (usize::from(block[1]) << 8)
                | usize::from(block[2]);
            for slot in 0..4 {
                if slot <= chunk.len() {
                    spelled.push(char::from(ALPHABET[(packed >> (18 - 6 * slot)) & 0x3F]));
                } else {
                    spelled.push('=');
                }
            }
        }
        spelled
    }

    /// What is searched for has to be something the scratch path cannot
    /// contain. `a_dir` names the directory with a version-4 UUID in hex, so a
    /// short hex-alphabet needle occurs there by chance: searching for `ab` and
    /// `171`, as this test once did, failed on about one run in eight. Every
    /// needle below is a full-length encoding of the whole key, and the base64
    /// one is not even in the hex alphabet.
    #[test]
    fn debug_carries_nothing_about_the_key() {
        const KEY_BYTE: u8 = 0xAB;

        let key = [KEY_BYTE; 32];
        let dir = a_dir();
        let printed = format!("{:?}", a_store(&dir, KEY_BYTE));

        assert!(
            printed.ends_with(", .. }"),
            "the marker standing for the withheld fields is gone, so nothing says a field \
             was withheld: {printed}"
        );
        let as_hex = hex(&key);
        for spelling in [
            format!("{key:?}"),
            as_hex.to_uppercase(),
            base64(&key),
            as_hex,
        ] {
            assert!(
                !printed.contains(&spelling),
                "the key reached a formatter as `{spelling}`: {printed}"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A source whose key the platform has destroyed, which is what the
    /// Android Keystore does on an uninstall or a wipe.
    struct NoKey;

    impl DeviceKeySource for NoKey {
        fn obtain(&self) -> Result<Kek, StoreError> {
            Err(StoreError::Backend("the device key is gone".to_owned()))
        }
        fn forget(&self) -> Result<(), StoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_destroyed_device_key_fails_the_read_rather_than_emptying_it() {
        let dir = a_dir();
        a_store(&dir, 8)
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("write");

        let orphaned = EncryptedSessionStore::in_data_dir(&dir, Arc::new(NoKey));
        assert!(
            orphaned.get(Marketplace::Tpt).await.is_err(),
            "a lost key must read as a failure the seller can act on, not as no session"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
