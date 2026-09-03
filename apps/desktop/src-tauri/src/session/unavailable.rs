//! The [`SessionStore`] for a platform whose credential store this build does
//! not yet have: Android, where `keyring` has no backend at all.
//!
//! It refuses rather than forgets. [`super::memory::MemorySessionStore`]
//! records why that distinction matters — a session that silently stopped
//! being persisted looks identical to one that was — and the same argument
//! rules it out as the interim here. A capture against this store fails with a
//! reason the interface can show, so an Android build cannot appear to hold a
//! session it will lose on the next launch.
//!
//! Which store replaces it is a founder decision, taken between a Kotlin
//! plugin over `androidx.security.crypto`, `tauri-plugin-stronghold`, and the
//! application's own private files directory; the three are compared in
//! `docs/notes/design/android-client.md`.

use tam_types::Marketplace;

use super::{SessionRecord, SessionStore, StoreError, StoreFuture};

#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableSessionStore;

impl UnavailableSessionStore {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn refusal() -> StoreError {
        StoreError::Backend(
            "this build has no session store for Android yet, so a marketplace sign-in cannot be \
             kept; see docs/notes/design/android-client.md"
                .to_owned(),
        )
    }
}

impl SessionStore for UnavailableSessionStore {
    fn put<'a>(&'a self, _record: &'a SessionRecord) -> StoreFuture<'a, ()> {
        Box::pin(async { Err(Self::refusal()) })
    }

    /// `None` rather than an error: nothing is held, which is a fact rather
    /// than a fault, and the callers that read this treat an absent session as
    /// "not connected" and say so.
    fn get(&self, _marketplace: Marketplace) -> StoreFuture<'_, Option<SessionRecord>> {
        Box::pin(async { Ok(None) })
    }

    /// Forgetting nothing succeeds. The revocation wipe runs this on every
    /// marketplace and must not report a failure for a store that never held
    /// anything.
    fn forget(&self, _marketplace: Marketplace) -> StoreFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::UnavailableSessionStore;
    use crate::session::tests::a_record;
    use crate::session::SessionStore;
    use tam_types::Marketplace;

    #[tokio::test]
    async fn a_capture_is_refused_rather_than_silently_dropped() {
        let store = UnavailableSessionStore::new();
        let record = a_record(Marketplace::Tpt);
        assert!(
            store.put(&record).await.is_err(),
            "a store that accepted a write it cannot keep would be indistinguishable from one \
             that kept it, which is the failure this type exists to prevent"
        );
    }

    #[tokio::test]
    async fn nothing_is_held_and_forgetting_nothing_succeeds() {
        let store = UnavailableSessionStore::new();
        assert_eq!(store.get(Marketplace::Tes).await, Ok(None));
        assert_eq!(store.forget(Marketplace::Tes).await, Ok(()));
    }
}
