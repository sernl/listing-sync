//! The blob store: content-addressed, per-tenant-encrypted resource-file
//! bytes. The database owns the object keys; the object store holds only
//! per-tenant ciphertext, because the files are the asset and cannot be
//! rotated after disclosure. Dedup is per tenant on the blake3 hash — an
//! identical file uploaded twice by one tenant is one blob and one object,
//! but the same bytes under two tenants are two, because a shared blob table
//! keyed on hash alone is an existence oracle.

use sqlx::PgPool;
use tam_pipeline::store::{ObjectStore, StoreError};
use tam_secrets::{open_bytes, seal_bytes, BlobAad, Kek, Sealed};
use tam_types::{ContentHash, OrgId, Timestamp};

use crate::codec::{hash_hex, hash_to_db, timestamp_to_db, uuid_to_db};
use crate::StorageError;

const BLOB_KEY_VERSION: i32 = 1;

pub struct BlobRepo<S> {
    pool: PgPool,
    store: S,
    kek: Kek,
}

#[derive(Debug)]
pub enum BlobError {
    Storage(StorageError),
    Store(String),
    Crypto(String),
}

impl core::fmt::Display for BlobError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "blob storage: {error}"),
            Self::Store(detail) => write!(f, "object store: {detail}"),
            Self::Crypto(detail) => write!(f, "blob crypto: {detail}"),
        }
    }
}

impl core::error::Error for BlobError {}

impl From<StorageError> for BlobError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::Error> for BlobError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(StorageError::Db(error))
    }
}

impl<S: ObjectStore> BlobRepo<S> {
    #[must_use]
    pub fn new(pool: PgPool, store: S, kek: Kek) -> Self {
        Self { pool, store, kek }
    }

    /// Seals the bytes, stores the object, and upserts the row. Per-tenant
    /// dedup: a second put of identical bytes under one tenant is a no-op on
    /// the row and rewrites the same object key. Returns the content hash.
    pub async fn put(
        &self,
        org: OrgId,
        bytes: &[u8],
        at: Timestamp,
    ) -> Result<ContentHash, BlobError> {
        let hash = tam_pipeline::hash::content_hash(bytes);
        let aad = BlobAad {
            org,
            hash: hash.0,
            key_version: BLOB_KEY_VERSION,
        };
        let sealed = seal_bytes(&self.kek, &aad.encode(), bytes)
            .map_err(|error| BlobError::Crypto(format!("{error:?}")))?;
        let object_key = object_key(org, hash);
        self.store
            .put(&object_key, encode_object(&sealed))
            .await
            .map_err(|error| BlobError::Store(error.to_string()))?;

        let byte_len = i64::try_from(bytes.len()).map_err(|_| {
            BlobError::Storage(StorageError::Inconsistent {
                reason: format!("blob of {} bytes exceeds the column range", bytes.len()),
            })
        })?;
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO blob \
             (org_id, hash, byte_len, object_key, dek_key_version, wrapped_dek, nonce, aad, \
              first_seen_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (org_id, hash) DO NOTHING",
            uuid_to_db(org.0),
            hash_to_db(hash),
            byte_len,
            object_key,
            BLOB_KEY_VERSION,
            sealed.wrapped_dek,
            sealed.nonce,
            aad.encode(),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(hash)
    }

    /// Fetches and opens a blob's bytes. The AAD is rebuilt from the row, so a
    /// row whose object was swapped for another tenant's fails to open.
    pub async fn get(&self, org: OrgId, hash: ContentHash) -> Result<Vec<u8>, BlobError> {
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT object_key, dek_key_version, wrapped_dek AS \"wrapped_dek!\", \
             nonce AS \"nonce!\" FROM blob WHERE org_id = $1 AND hash = $2",
            uuid_to_db(org.0),
            hash_to_db(hash),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let row = row.ok_or(BlobError::Store("no blob row for this hash".to_owned()))?;

        let object = self
            .store
            .get(&row.object_key)
            .await
            .map_err(|error: StoreError| BlobError::Store(error.to_string()))?;
        let sealed = decode_object(&row.wrapped_dek, &row.nonce, &object);
        let aad = BlobAad {
            org,
            hash: hash.0,
            key_version: row.dek_key_version,
        };
        open_bytes(&self.kek, &aad.encode(), &sealed)
            .map_err(|error| BlobError::Crypto(format!("{error:?}")))
    }
}

/// A `BlobSink` bound to one tenant, so the pipeline can store blobs without
/// knowing the tenant plumbing. The pipeline seals nothing itself; the repo
/// does, per tenant.
pub struct TenantBlobSink<'a, S> {
    pub repo: &'a BlobRepo<S>,
    pub org: OrgId,
    pub at: Timestamp,
}

impl<S: ObjectStore> tam_pipeline::pipeline::BlobSink for TenantBlobSink<'_, S> {
    async fn store(&self, bytes: Vec<u8>) -> Result<ContentHash, String> {
        self.repo
            .put(self.org, &bytes, self.at)
            .await
            .map_err(|error| error.to_string())
    }
}

/// The `FileSource` the Tes adapter calls into: it reads a stored, encrypted
/// blob back as `FileContent` for upload, closing the seam M1c left as a stub.
pub struct PipelineFileSource<S> {
    repo: BlobRepo<S>,
    org: OrgId,
    catalogue: PgPool,
}

impl<S: ObjectStore> PipelineFileSource<S> {
    #[must_use]
    pub fn new(repo: BlobRepo<S>, org: OrgId, catalogue: PgPool) -> Self {
        Self {
            repo,
            org,
            catalogue,
        }
    }
}

impl<S: ObjectStore> tam_marketplace::FileSource for PipelineFileSource<S> {
    async fn fetch(
        &self,
        file: tam_types::FileId,
    ) -> Result<tam_marketplace::FileContent, tam_marketplace::FileSourceError> {
        // The product_file row carries the hash and kind; the blob carries the
        // bytes. One join keyed on the tenant.
        let mut tx = self.catalogue.begin().await.map_err(|error| {
            tam_marketplace::FileSourceError::Unreadable {
                file,
                detail: error.to_string(),
            }
        })?;
        crate::pin_org(&mut tx, self.org).await.map_err(|error| {
            tam_marketplace::FileSourceError::Unreadable {
                file,
                detail: error.to_string(),
            }
        })?;
        let row = sqlx::query!(
            "SELECT hash, kind FROM product_file WHERE org_id = $1 AND id = $2",
            uuid_to_db(self.org.0),
            uuid_to_db(file.0),
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| tam_marketplace::FileSourceError::Unreadable {
            file,
            detail: error.to_string(),
        })?;
        drop(tx);
        let row = row.ok_or(tam_marketplace::FileSourceError::Missing(file))?;
        let hash = crate::codec::hash_from_db(&row.hash).map_err(|error| {
            tam_marketplace::FileSourceError::Unreadable {
                file,
                detail: error.to_string(),
            }
        })?;
        let bytes = self.repo.get(self.org, hash).await.map_err(|error| {
            tam_marketplace::FileSourceError::Unreadable {
                file,
                detail: error.to_string(),
            }
        })?;
        Ok(tam_marketplace::FileContent {
            file_name: format!("{}.{}", crate::codec::hash_hex(hash), extension(&row.kind)),
            content_type: content_type(&row.kind),
            bytes,
        })
    }
}

fn extension(kind: &str) -> &'static str {
    match kind {
        "pdf" => "pdf",
        "pptx" => "pptx",
        "docx" => "docx",
        "zip" => "zip",
        _ => "bin",
    }
}

fn content_type(kind: &str) -> String {
    match kind {
        "pdf" => "application/pdf",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "image" => "image/png",
        _ => "application/octet-stream",
    }
    .to_owned()
}

fn object_key(org: OrgId, hash: ContentHash) -> String {
    // A flat key the LocalObjectStore accepts: the tenant prefix keeps a
    // shared dev directory legible, the hash makes it content-addressed, and
    // both are hex so the key has no path separators.
    let mut org_hex = String::with_capacity(32);
    for byte in org.0 .0 {
        use core::fmt::Write;
        let _unused: core::fmt::Result = write!(org_hex, "{byte:02x}");
    }
    format!("{org_hex}-{}", hash_hex(hash))
}

/// The object payload is the ciphertext; the wrapped DEK and nonce ride in
/// the database row, not the object, so the object alone is inert.
fn encode_object(sealed: &Sealed) -> Vec<u8> {
    sealed.ciphertext.clone()
}

fn decode_object(wrapped_dek: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Sealed {
    Sealed {
        key_version: BLOB_KEY_VERSION,
        wrapped_dek: wrapped_dek.to_vec(),
        nonce: nonce.to_vec(),
        ciphertext: ciphertext.to_vec(),
    }
}
