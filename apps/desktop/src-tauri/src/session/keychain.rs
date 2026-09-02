//! The [`SessionStore`] that puts the jar in the operating system's own
//! credential store: DPAPI-backed Credential Manager on Windows, Keychain on
//! macOS, Secret Service on Linux.
//!
//! One entry per marketplace under one service name, holding the record as
//! JSON. The record rather than the bare jar, so a session read back carries
//! the device it was captured on and can be refused if that changed.

use tam_types::Marketplace;

use super::{entry_key, SessionRecord, SessionStore, StoreError, StoreFuture};

/// The service name every entry is filed under. The bundle identifier, so a
/// human reading their own keychain can tell which application asked.
pub const SERVICE: &str = "io.teachouse.desktop";

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
    use super::{KeychainSessionStore, SERVICE};
    use crate::session::entry_key;
    use tam_types::Marketplace;

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
