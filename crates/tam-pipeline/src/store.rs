//! Object storage for blob bytes, behind a trait so the local filesystem
//! stands where S3 will. The database owns the keys; this only moves bytes.
//! Bytes are sealed per tenant by the caller before they arrive here, so the
//! store itself never holds plaintext.

use std::path::PathBuf;

pub trait ObjectStore: Send + Sync {
    fn put(
        &self,
        key: &str,
        bytes: Vec<u8>,
    ) -> impl core::future::Future<Output = Result<(), StoreError>> + Send;

    fn get(
        &self,
        key: &str,
    ) -> impl core::future::Future<Output = Result<Vec<u8>, StoreError>> + Send;
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    NotFound,
    /// A key that would escape the store root, refused rather than joined.
    UnsafeKey,
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "object store io: {error}"),
            Self::NotFound => f.write_str("object not found"),
            Self::UnsafeKey => f.write_str("object key escapes the store root"),
        }
    }
}

impl core::error::Error for StoreError {}

/// A filesystem-backed store under one root directory. Keys are single path
/// segments (a hex hash), validated so nothing escapes the root.
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn resolve(&self, key: &str) -> Result<PathBuf, StoreError> {
        // A key is a flat identifier: hex digits and a couple of safe
        // punctuation marks, no separators, so it cannot traverse.
        let safe = !key.is_empty()
            && key.len() <= 128
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');
        if !safe || key.contains("..") {
            return Err(StoreError::UnsafeKey);
        }
        Ok(self.root.join(key))
    }
}

impl ObjectStore for LocalObjectStore {
    fn put(
        &self,
        key: &str,
        bytes: Vec<u8>,
    ) -> impl core::future::Future<Output = Result<(), StoreError>> + Send {
        let path = self.resolve(key);
        let root = self.root.clone();
        async move {
            let path = path?;
            std::fs::create_dir_all(&root).map_err(StoreError::Io)?;
            std::fs::write(&path, &bytes).map_err(StoreError::Io)
        }
    }

    fn get(
        &self,
        key: &str,
    ) -> impl core::future::Future<Output = Result<Vec<u8>, StoreError>> + Send {
        let path = self.resolve(key);
        async move {
            // The bytes are already bounded by UPLOAD_BODY_BYTES_MAX and held
            // in memory across the whole pipeline; the streaming ban targets
            // unbounded reads, which this is not.
            #[expect(
                clippy::disallowed_methods,
                reason = "blob bytes are bounded by the upload cap and in memory throughout the pipeline"
            )]
            match std::fs::read(path?) {
                Ok(bytes) => Ok(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Err(StoreError::NotFound)
                }
                Err(error) => Err(StoreError::Io(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalObjectStore, ObjectStore, StoreError};

    #[test]
    fn a_key_with_separators_is_refused() {
        let store = LocalObjectStore::new(std::env::temp_dir());
        let refused = futures::executor::block_on(store.get("a/../escape"));
        assert!(
            matches!(refused, Err(StoreError::UnsafeKey)),
            "a key that could traverse is refused, not joined"
        );
    }
}
