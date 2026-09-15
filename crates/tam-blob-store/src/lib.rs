//! Where a deployment's sealed blob objects live, and the one place that
//! decides it: the local directory every deployment has had, or an
//! S3-compatible bucket for the ones that keep their objects in Garage or
//! MinIO.
//!
//! Its own crate rather than a module of `tam-pipeline`, which owns the
//! `ObjectStore` trait, and the reason is a dependency edge rather than
//! tidiness: the pipeline is on the portable leg of `just check-portable`,
//! which compiles it for wasm32 and four client triples that carry no HTTP
//! transport at all.
//!
//! The backend is chosen once, at start-up, from the flags named here, and
//! both binaries that hold blobs — `tam-server` and `tam-pipeline-worker` —
//! read the same five. A store is then built per request from
//! [`BlobBackend::object_store`], which for the S3 arm clones a connection
//! pool rather than opening one.

#![forbid(unsafe_code)]

mod s3;
mod sigv4;

pub use s3::S3ObjectStore;

use std::io::Read as _;
use std::path::PathBuf;

use tam_pipeline::store::{LocalObjectStore, ObjectStore, StoreError};

/// See [`BlobBackend`]; the local root and the S3 set are mutually exclusive.
pub const STORE_ROOT_FLAG: &str = "--blob-store-root";
/// The endpoint url, carrying its scheme: `http://garage.internal:3900`.
pub const STORE_S3_FLAG: &str = "--blob-store-s3";
/// The one bucket every object of this deployment is written into.
pub const STORE_BUCKET_FLAG: &str = "--blob-store-bucket";
/// A file holding two lines, the access key then the secret key. A file
/// rather than an inline value for the reason the other secrets here are
/// files: a secret on a command line is in every process listing on the host.
pub const STORE_CREDENTIALS_FLAG: &str = "--blob-store-credentials";
/// The signing region. Part of a signature's scope on every S3-compatible
/// endpoint, including the ones with no regions to speak of.
pub const STORE_REGION_FLAG: &str = "--blob-store-region";

/// What Garage signs under unless it is configured otherwise, which is the
/// store this option exists for.
pub const DEFAULT_REGION: &str = "garage";

/// The most of a credentials file that is read, so a mis-pointed flag is
/// refused rather than allocated.
const CREDENTIALS_BYTES_MAX: u64 = 8 * 1024;

/// One S3 key pair, read from disk at start-up and held for the process's
/// life: nothing re-reads the file, so rotating it is a restart.
#[derive(Clone)]
pub struct Credentials {
    access_key: String,
    secret_key: String,
}

impl Credentials {
    /// Two lines, the access key then the secret key, each trimmed. Read
    /// through a bounded `File` rather than `std::fs::read`, which the lint
    /// table bans.
    pub fn read(path: &str) -> Result<Self, String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|why| format!("{path} could not be opened: {why}"))?
            .take(CREDENTIALS_BYTES_MAX + 1)
            .read_to_end(&mut bytes)
            .map_err(|why| format!("{path} could not be read: {why}"))?;
        if bytes.len() as u64 > CREDENTIALS_BYTES_MAX {
            return Err(format!(
                "{path} is larger than {CREDENTIALS_BYTES_MAX} bytes, so it is not a key pair"
            ));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_why| format!("{path} is not utf-8, so it is not a key pair"))?;
        let mut lines = text.lines();
        let access_key = lines.next().unwrap_or_default().trim().to_owned();
        let secret_key = lines.next().unwrap_or_default().trim().to_owned();
        if access_key.is_empty() || secret_key.is_empty() {
            return Err(format!(
                "{path} holds two lines, the access key then the secret key"
            ));
        }
        Ok(Self {
            access_key,
            secret_key,
        })
    }
}

/// Which store a deployment writes its sealed objects into. Cloned per
/// request by [`Self::object_store`], so both arms are cheap to copy.
#[derive(Clone)]
pub enum BlobBackend {
    Local(PathBuf),
    S3(S3ObjectStore),
}

impl BlobBackend {
    #[must_use]
    pub fn object_store(&self) -> AnyObjectStore {
        match self {
            Self::Local(root) => AnyObjectStore::Local(LocalObjectStore::new(root.clone())),
            Self::S3(store) => AnyObjectStore::S3(store.clone()),
        }
    }

    /// What a start-up line prints, so an operator can see which store the
    /// process took without reading its unit file back.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Local(root) => root.display().to_string(),
            Self::S3(store) => store.describe(),
        }
    }
}

/// The store a handler holds: one enum rather than a trait object, because
/// `ObjectStore`'s methods return futures and a `dyn` edge would need every
/// one of them boxed.
pub enum AnyObjectStore {
    Local(LocalObjectStore),
    S3(S3ObjectStore),
}

impl ObjectStore for AnyObjectStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
        match self {
            Self::Local(store) => store.put(key, bytes).await,
            Self::S3(store) => store.put(key, bytes).await,
        }
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StoreError> {
        match self {
            Self::Local(store) => store.get(key).await,
            Self::S3(store) => store.get(key).await,
        }
    }
}

/// The five flags as they arrive on a command line, before they are read as a
/// backend. Collected by the binary's own parse loop through
/// [`Self::accept`], so there is one spelling of each flag in the workspace.
#[derive(Default)]
pub struct BackendFlags {
    root: Option<PathBuf>,
    endpoint: Option<String>,
    bucket: Option<String>,
    credentials: Option<String>,
    region: Option<String>,
}

impl BackendFlags {
    /// Takes `argument` if it names one of these flags, pulling its value off
    /// the caller's own iterator, and answers whether it did.
    pub fn accept(
        &mut self,
        argument: &str,
        arguments: &mut impl Iterator<Item = String>,
    ) -> Result<bool, String> {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} needs a value"))
        };
        if argument == STORE_ROOT_FLAG {
            self.root = Some(PathBuf::from(value()?));
        } else if argument == STORE_S3_FLAG {
            self.endpoint = Some(value()?);
        } else if argument == STORE_BUCKET_FLAG {
            self.bucket = Some(value()?);
        } else if argument == STORE_CREDENTIALS_FLAG {
            self.credentials = Some(value()?);
        } else if argument == STORE_REGION_FLAG {
            self.region = Some(value()?);
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// The backend these flags name, or `None` when none of them was given.
    ///
    /// Refused rather than half-configured, exactly as the binaries' other
    /// paired flags are: a bucket with no credentials cannot be written to,
    /// and a deployment that named both a local root and a bucket has not
    /// said where its objects go.
    pub fn resolve(self) -> Result<Option<BlobBackend>, String> {
        let s3_named = self.endpoint.is_some()
            || self.bucket.is_some()
            || self.credentials.is_some()
            || self.region.is_some();
        match (self.root, s3_named) {
            (Some(_), true) => Err(format!(
                "{STORE_ROOT_FLAG} and {STORE_S3_FLAG} name two different stores; give one"
            )),
            (Some(root), false) => Ok(Some(BlobBackend::Local(root))),
            (None, false) => Ok(None),
            (None, true) => {
                let (Some(endpoint), Some(bucket), Some(credentials)) =
                    (self.endpoint, self.bucket, self.credentials)
                else {
                    return Err(format!(
                        "{STORE_S3_FLAG}, {STORE_BUCKET_FLAG} and {STORE_CREDENTIALS_FLAG} are \
                         given together or not at all"
                    ));
                };
                let region = self.region.unwrap_or_else(|| DEFAULT_REGION.to_owned());
                Ok(Some(BlobBackend::S3(S3ObjectStore::new(
                    &endpoint,
                    bucket,
                    region,
                    Credentials::read(&credentials)?,
                )?)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendFlags, STORE_BUCKET_FLAG, STORE_ROOT_FLAG, STORE_S3_FLAG};

    /// The backend these arguments resolve to, rendered by `describe` so the
    /// assertion reads the store that was chosen rather than a pointer.
    fn flags(arguments: &[&str]) -> Result<Option<String>, String> {
        let mut collected = BackendFlags::default();
        let mut arguments = arguments.iter().map(|argument| (*argument).to_owned());
        while let Some(argument) = arguments.next() {
            assert!(
                collected.accept(&argument, &mut arguments)?,
                "every argument here is a backend flag: {argument}"
            );
        }
        Ok(collected.resolve()?.map(|backend| backend.describe()))
    }

    #[test]
    fn a_local_root_alone_is_the_local_backend() {
        assert_eq!(
            flags(&[STORE_ROOT_FLAG, "/var/lib/teachouse/blobs"]),
            Ok(Some("/var/lib/teachouse/blobs".to_owned())),
            "the default deployment shape stays the local directory"
        );
    }

    #[test]
    fn the_two_stores_are_refused_together() {
        let resolved = flags(&[
            STORE_ROOT_FLAG,
            "/var/lib/teachouse/blobs",
            STORE_S3_FLAG,
            "http://garage:3900",
            STORE_BUCKET_FLAG,
            "teachouse",
        ]);
        assert!(
            resolved.is_err(),
            "a deployment naming both stores has not said where its objects go: {resolved:?}",
        );
    }

    #[test]
    fn a_bucket_without_its_credentials_is_refused() {
        let resolved = flags(&[STORE_S3_FLAG, "http://garage:3900", STORE_BUCKET_FLAG, "b"]);
        assert!(
            resolved.is_err(),
            "a bucket with no credentials cannot be written to: {resolved:?}",
        );
    }
}
