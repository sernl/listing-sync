//! An in-memory [`SessionStore`], for tests and for a machine with no usable
//! keychain. It is deliberately not a fallback the application selects on its
//! own: a session that silently stopped being persisted would look identical
//! to one that was.

use std::collections::HashMap;

use tam_types::Marketplace;
use tokio::sync::Mutex;

use super::{SessionRecord, SessionStore, StoreFuture};

#[derive(Debug, Default)]
pub struct MemorySessionStore {
    records: Mutex<HashMap<Marketplace, SessionRecord>>,
}

impl MemorySessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SessionStore for MemorySessionStore {
    fn put<'a>(&'a self, record: &'a SessionRecord) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            self.records
                .lock()
                .await
                .insert(record.marketplace, record.clone());
            Ok(())
        })
    }

    fn get(&self, marketplace: Marketplace) -> StoreFuture<'_, Option<SessionRecord>> {
        Box::pin(async move { Ok(self.records.lock().await.get(&marketplace).cloned()) })
    }

    fn forget(&self, marketplace: Marketplace) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            self.records.lock().await.remove(&marketplace);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::MemorySessionStore;
    use crate::session::tests::a_record;
    use crate::session::SessionStore;
    use tam_types::Marketplace;

    #[tokio::test]
    async fn a_stored_session_reads_back_and_a_forgotten_one_does_not() {
        let store = MemorySessionStore::new();
        assert_eq!(store.get(Marketplace::Tpt).await, Ok(None));

        let record = a_record(Marketplace::Tpt);
        store
            .put(&record)
            .await
            .expect("the store accepts a record");
        assert_eq!(
            store.get(Marketplace::Tpt).await,
            Ok(Some(record)),
            "what went in comes back unchanged"
        );

        store
            .forget(Marketplace::Tpt)
            .await
            .expect("forgetting succeeds");
        assert_eq!(
            store.get(Marketplace::Tpt).await,
            Ok(None),
            "forget must actually remove, because it is the seller's disconnect"
        );
    }

    #[tokio::test]
    async fn one_marketplace_supersedes_only_itself() {
        let store = MemorySessionStore::new();
        store
            .put(&a_record(Marketplace::Tpt))
            .await
            .expect("tpt stores");
        store
            .put(&a_record(Marketplace::Tes))
            .await
            .expect("tes stores");
        store
            .forget(Marketplace::Tpt)
            .await
            .expect("tpt is forgotten");
        assert_eq!(store.get(Marketplace::Tpt).await, Ok(None));
        assert!(
            store
                .get(Marketplace::Tes)
                .await
                .expect("tes still reads")
                .is_some(),
            "disconnecting one marketplace must not disconnect another"
        );
    }
}
