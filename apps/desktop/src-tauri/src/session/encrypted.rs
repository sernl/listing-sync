//! The [`SessionStore`] that seals the jar under a key the device holds,
//! which is how Android keeps a marketplace session.
//!
//! `keyring` has no Android backend, so the custody [`super::keychain`] gives
//! Windows, macOS and Linux has to come from somewhere else there. Founder
//! decision, 2026-09-03: a secret generated once and held by the Android
//! Keystore, never typed by the seller and never shown, sealing an encrypted
//! file under the application's own private data directory.
//!
//! Nothing here is Android-specific, and that is deliberate. The cipher is
//! portable, the file format is portable, and the only platform-bound part —
//! where the secret comes from — sits behind [`DeviceKeySource`], whose
//! Android implementation talks to the Keystore and whose test implementation
//! holds a fixed key. So this module compiles on every target the workspace
//! builds and its behaviour is tested on the host, rather than being a module
//! compiled solely for a target no lane builds and therefore never tested.

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
        Self { path, keys }
    }

    /// The file beside the device identity, which on Android is the
    /// application's own private files directory.
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

    async fn store(&self, filed: &Filed) -> Result<(), StoreError> {
        let encoded =
            serde_json::to_vec(filed).map_err(|why| StoreError::Codec(why.to_string()))?;
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|why| StoreError::Backend(why.to_string()))?;
        }
        tokio::fs::write(&self.path, encoded)
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

    async fn read(&self, marketplace: Marketplace) -> Result<Option<SessionRecord>, StoreError> {
        let filed = self.load().await?;
        let Some(envelope) = filed.get(entry_key(marketplace)).cloned() else {
            return Ok(None);
        };
        let kek = self.keys.obtain()?;
        let plaintext = open_bytes(&kek, Self::aad(marketplace), &envelope.into())
            .map_err(|why| StoreError::Backend(why.to_string()))?;
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|why| StoreError::Codec(why.to_string()))
    }

    /// Destroys the device key and the file together.
    ///
    /// Separate from [`SessionStore::forget`] and never called by it. This is
    /// the whole-device wipe: without the key nothing that survived the file's
    /// deletion can be read, which is what makes the wipe a fact rather than a
    /// deletion the filesystem might not have honoured.
    pub async fn wipe(&self) -> Result<(), StoreError> {
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
        Box::pin(async move { self.read(marketplace).await })
    }

    /// Forgetting a marketplace that was never stored succeeds, because the
    /// revocation wipe runs this over every marketplace and must not report a
    /// failure for one the seller never connected.
    fn forget(&self, marketplace: Marketplace) -> StoreFuture<'_, ()> {
        Box::pin(async move { self.write(marketplace, None).await })
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
    use crate::session::tests::a_record;
    use crate::session::SessionStore;
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

    #[tokio::test]
    async fn a_file_written_under_one_secret_does_not_open_under_another() {
        let dir = a_dir();
        a_store(&dir, 1)
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("the write");

        let intruder = a_store(&dir, 2);
        assert!(
            intruder.get(Marketplace::Tpt).await.is_err(),
            "the device key is the whole of the secrecy; another key must not open the file"
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

        assert!(
            store.get(Marketplace::Tes).await.is_err(),
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
