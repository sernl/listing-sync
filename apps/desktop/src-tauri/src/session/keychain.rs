//! The operating system's own credential store — DPAPI-backed Credential
//! Manager on Windows, Keychain on macOS, Secret Service on Linux — as the
//! place the desktop keeps the key its sessions are sealed under.
//!
//! Only the key. The sealed records live in a file
//! ([`super::encrypted::EncryptedSessionStore`]); a credential store is for
//! short secrets, and Windows refuses anything longer than 2560 UTF-16
//! characters, which a marketplace cookie jar routinely is.
//!
//! [`KeychainSessionStore`] is what earlier versions kept the whole record in,
//! one entry per marketplace. It is kept only so those entries can be moved
//! into the file and removed.

use tam_secrets::Kek;
use tam_types::{ContentHash, Marketplace};

use super::encrypted::DeviceKeySource;
use super::{entry_key, SessionRecord, SessionStore, StoreError, StoreFuture};
use crate::library::hash_from_hex;

/// The service name every entry is filed under. The bundle identifier, so a
/// human reading their own keychain can tell which application asked.
pub const SERVICE: &str = "io.teachouse.desktop";

/// The entry the session key is filed under, beside the library's key.
pub const KEY_ENTRY: &str = "session.key";

/// The sealing key for this device's sessions: thirty-two random bytes,
/// generated on first use and held in the credential store as 64 hex
/// characters.
pub struct KeychainKey {
    service: String,
}

/// Prints nothing but the service: the entry it reads is a secret.
impl core::fmt::Debug for KeychainKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KeychainKey")
            .field("service", &self.service)
            .finish()
    }
}

impl KeychainKey {
    #[must_use]
    pub fn under(service: &str) -> Self {
        Self {
            service: service.to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(&self.service, KEY_ENTRY)
            .map_err(|why| StoreError::Backend(why.to_string()))
    }
}

/// The key a stored hex spelling names, or a refusal that says the entry is
/// not one — never a fresh key, which would orphan every sealed session.
fn key_from_hex(hex: &str) -> Result<Kek, StoreError> {
    hash_from_hex(hex.trim())
        .and_then(|ContentHash(bytes)| Kek::from_bytes(&bytes).ok())
        .ok_or_else(|| StoreError::Codec("the session key entry is not a key".to_owned()))
}

impl DeviceKeySource for KeychainKey {
    fn obtain(&self) -> Result<Kek, StoreError> {
        let entry = self.entry()?;
        match entry.get_password() {
            Ok(hex) => key_from_hex(&hex),
            Err(keyring::Error::NoEntry) => {
                let fresh = tam_secrets::random_token();
                entry
                    .set_password(&fresh)
                    .map_err(|why| StoreError::Backend(why.to_string()))?;
                key_from_hex(&fresh)
            }
            Err(why) => Err(StoreError::Backend(why.to_string())),
        }
    }

    fn forget(&self) -> Result<(), StoreError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(why) => Err(StoreError::Backend(why.to_string())),
        }
    }
}

/// Where versions before 0.16.0 kept the record itself, read only to move it.
#[derive(Debug, Clone)]
pub struct KeychainSessionStore {
    service: String,
}

impl Default for KeychainSessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeychainSessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            service: SERVICE.to_owned(),
        }
    }

    /// A store under a different service name, so a test can write to a
    /// keychain entry that is not the shipping one.
    #[must_use]
    pub fn under_service(service: String) -> Self {
        Self { service }
    }

    fn entry(&self, marketplace: Marketplace) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(&self.service, entry_key(marketplace))
            .map_err(|why| StoreError::Backend(why.to_string()))
    }
}

impl KeychainSessionStore {
    fn store(&self, record: &SessionRecord) -> Result<(), StoreError> {
        let encoded =
            serde_json::to_string(record).map_err(|why| StoreError::Codec(why.to_string()))?;
        self.entry(record.marketplace)?
            .set_password(&encoded)
            .map_err(|why| StoreError::Backend(why.to_string()))
    }

    fn read(&self, marketplace: Marketplace) -> Result<Option<SessionRecord>, StoreError> {
        match self.entry(marketplace)?.get_password() {
            Ok(encoded) => serde_json::from_str(&encoded)
                .map(Some)
                .map_err(|why| StoreError::Codec(why.to_string())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(why) => Err(StoreError::Backend(why.to_string())),
        }
    }

    fn remove(&self, marketplace: Marketplace) -> Result<(), StoreError> {
        match self.entry(marketplace)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(why) => Err(StoreError::Backend(why.to_string())),
        }
    }
}

impl SessionStore for KeychainSessionStore {
    fn put<'a>(&'a self, record: &'a SessionRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move { self.store(record) })
    }

    fn get(&self, marketplace: Marketplace) -> StoreFuture<'_, Option<SessionRecord>> {
        Box::pin(async move { self.read(marketplace) })
    }

    fn forget(&self, marketplace: Marketplace) -> StoreFuture<'_, ()> {
        Box::pin(async move { self.remove(marketplace) })
    }
}

#[cfg(test)]
mod tests {
    use super::{key_from_hex, KeychainSessionStore, KEY_ENTRY, SERVICE};
    use crate::session::entry_key;
    use tam_types::Marketplace;

    #[test]
    fn a_fresh_key_fits_every_credential_store_and_parses_back() {
        let fresh = tam_secrets::random_token();
        assert_eq!(
            fresh.len(),
            64,
            "Windows caps an entry at 2560 UTF-16 characters; the key is all that goes there"
        );
        assert!(key_from_hex(&fresh).is_ok());
        assert!(
            key_from_hex("not a key").is_err(),
            "an entry that is not a key is refused rather than replaced, which would orphan \
             every sealed session"
        );
        assert!(
            Marketplace::ALL
                .iter()
                .all(|&marketplace| entry_key(marketplace) != KEY_ENTRY),
            "the key's entry must not collide with a legacy session entry the migration removes"
        );
    }

    #[test]
    fn every_marketplace_has_its_own_entry_under_one_service() {
        let mut keys: Vec<&str> = Marketplace::ALL.iter().copied().map(entry_key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            keys.len(),
            Marketplace::ALL.len(),
            "two marketplaces sharing one keychain entry would have one overwrite the other"
        );
        assert_eq!(
            KeychainSessionStore::new().service,
            SERVICE,
            "the shipping store files under the bundle identifier, so a human reading their own \
             keychain can tell which application asked"
        );
    }
}
