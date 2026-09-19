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
use tam_types::{ContentHash, FileBytes, FileId, Marketplace, Observation, OrgId, Timestamp};

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
    /// This organisation has sealed no blob under that hash.
    ///
    /// Its own variant rather than a [`Self::Store`] carrying a sentence,
    /// because a caller has to tell it from a store that failed: one is a
    /// request for something that is not there and the other is a fault of
    /// ours, and answering the second as the first tells a seller their file
    /// is gone when it is only unreachable.
    Missing,
    Store(String),
    Crypto(String),
}

impl core::fmt::Display for BlobError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "blob storage: {error}"),
            Self::Missing => write!(f, "no blob under that hash for this organisation"),
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
    /// dedup: the row insert is the arbiter, so a second put of identical
    /// bytes under one tenant writes neither the row nor the object. Rewriting
    /// the object would leave ciphertext sealed under a fresh DEK beside a row
    /// that kept the first put's wrapped DEK and nonce, and every later `get`
    /// would fail to authenticate. Returns the content hash.
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

        let byte_len = i64::try_from(bytes.len()).map_err(|_| {
            BlobError::Storage(StorageError::Inconsistent {
                reason: format!("blob of {} bytes exceeds the column range", bytes.len()),
            })
        })?;
        let mut tx = self.pool.begin().await?;
        crate::pin_org(&mut tx, org).await?;
        let inserted = sqlx::query!(
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
        .await?
        .rows_affected();
        // Written before the commit, so a failed object write rolls the row
        // back: a committed row whose object never landed would be permanent,
        // every later put conflicting away without writing it.
        if inserted == 1 {
            self.store
                .put(&object_key, encode_object(&sealed))
                .await
                .map_err(|error| BlobError::Store(error.to_string()))?;
        }
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
        let row = row.ok_or(BlobError::Missing)?;

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
        // A marketplace-sourced file has no blob to resolve, and saying
        // `Missing` would be false: the file is here, its bytes are not, and
        // they are the seller's device's to fetch under the seller's own
        // session. Only a server-side upload path reaches this at all, so this
        // is the refusal that names why rather than a case to support.
        let Some(stored) = row.hash else {
            return Err(tam_marketplace::FileSourceError::Unreadable {
                file,
                detail: "this file names a marketplace resource rather than a stored blob. \
                         Its bytes are the seller's, held by the marketplace, and never \
                         ours: a server-side path has reached for bytes D27 says the \
                         server must not hold"
                    .to_owned(),
            });
        };
        let hash = crate::codec::hash_from_db(&stored).map_err(|error| {
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
        "image" => "png",
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

/// One stored file, as the payload manifest needs to describe it.
///
/// The name and the content type are derived from the file's kind exactly as
/// [`PipelineFileSource::fetch`] derives them for a blob-backed file, so the
/// manifest describes the same bytes the upload would send under the same
/// name; a sourced file carries the name and type the producer recorded for
/// the bytes handed onward after the unwrap decision — the entry's where there
/// is one, the bundle's otherwise — rather than a name derived from a digest
/// we do not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    pub id: FileId,
    pub file_name: String,
    pub content_type: String,
    pub bytes: FileBytes,
}

/// Describes the stated files, in the order asked for, skipping any the tenant
/// does not hold.
///
/// The join onto `blob` is a LEFT one because a marketplace-sourced file has
/// no blob row by construction. An inner join would not merely omit its length
/// — it would drop the file from the answer entirely, which is how a sourced
/// file reached the manifest builder as nothing at all before this fork.
/// Which arm a row is in is decided by `hash`, and the schema's
/// `product_file_blob_or_source` CHECK is what makes that decision total.
pub async fn describe_files(
    pool: &PgPool,
    org: OrgId,
    files: &[FileId],
) -> Result<Vec<StoredFile>, StorageError> {
    let mut described = Vec::with_capacity(files.len());
    let mut tx = pool.begin().await?;
    crate::pin_org(&mut tx, org).await?;
    for file in files {
        let row = sqlx::query!(
            "SELECT pf.kind, pf.hash, b.byte_len AS \"byte_len?\", \
                    pf.scan_state, pf.scan_signature, pf.scanned_at, pf.scan_failure_code, \
                    pf.source_marketplace, pf.source_connection, pf.source_resource, \
                    pf.source_entry, pf.observed_hash, pf.observed_byte_len, \
                    pf.observed_by_device, pf.observed_at, \
                    pf.asserted_scan_state, pf.asserted_scan_signature, \
                    pf.asserted_scanned_at, pf.asserted_scan_failure_code, \
                    pf.payload_file_name, pf.payload_content_type \
             FROM product_file pf \
             LEFT JOIN blob b ON b.org_id = pf.org_id AND b.hash = pf.hash \
             WHERE pf.org_id = $1 AND pf.id = $2 AND pf.deleted_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(file.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else { continue };
        let described_file = match (row.hash, row.byte_len) {
            (Some(hash), Some(byte_len)) => {
                let hash = crate::codec::hash_from_db(&hash)?;
                StoredFile {
                    id: *file,
                    // Synthesised, because a blob-backed file has no name of
                    // its own: it was uploaded as bytes and is named by what
                    // it is. A sourced file does have one and uses it below.
                    file_name: format!("{}.{}", crate::codec::hash_hex(hash), extension(&row.kind)),
                    content_type: content_type(&row.kind),
                    bytes: FileBytes::Held {
                        hash,
                        byte_len: byte_len.try_into().map_err(|_| StorageError::CorruptRow {
                            reason: format!("blob byte_len {byte_len} is negative"),
                        })?,
                        scan: crate::codec::scan_from_db(
                            required(row.scan_state, "scan_state")?.as_str(),
                            row.scan_signature,
                            row.scanned_at,
                            row.scan_failure_code.as_deref(),
                        )?,
                    },
                }
            }
            // A blob-backed row whose blob has gone is not a sourced file and
            // must not be described as one; it is the same missing file the
            // inner join used to skip, and it keeps being skipped.
            (Some(_), None) => continue,
            (None, _) => {
                let (marketplace, resource) = sourced(
                    row.source_marketplace.as_deref(),
                    row.source_resource.as_deref(),
                )?;
                let observed_byte_len = required(row.observed_byte_len, "observed_byte_len")?;
                let payload_file_name = required(row.payload_file_name, "payload_file_name")?;
                let payload_content_type =
                    required(row.payload_content_type, "payload_content_type")?;
                StoredFile {
                    id: *file,
                    // The producer's own name and type, never synthesised: it
                    // is the one thing here we could not derive, and deriving
                    // it is how a worksheet reaches a storefront named as a
                    // zip.
                    file_name: payload_file_name.clone(),
                    content_type: payload_content_type.clone(),
                    bytes: FileBytes::Sourced {
                        marketplace,
                        connection: tam_types::ConnectionId(crate::codec::uuid_from_db(required(
                            row.source_connection,
                            "source_connection",
                        )?)),
                        resource,
                        entry: row.source_entry,
                        payload_file_name,
                        payload_content_type,
                        observed: Observation {
                            device: required(row.observed_by_device, "observed_by_device")?,
                            hash: crate::codec::hash_from_db(&required(
                                row.observed_hash,
                                "observed_hash",
                            )?)?,
                            byte_len: observed_byte_len.try_into().map_err(|_| {
                                StorageError::CorruptRow {
                                    reason: format!(
                                        "observed_byte_len {observed_byte_len} is negative"
                                    ),
                                }
                            })?,
                            scan: crate::codec::scan_from_db(
                                required(row.asserted_scan_state, "asserted_scan_state")?.as_str(),
                                row.asserted_scan_signature,
                                row.asserted_scanned_at,
                                row.asserted_scan_failure_code.as_deref(),
                            )?,
                            observed_at: crate::codec::timestamp_from_db(required(
                                row.observed_at,
                                "observed_at",
                            )?),
                        },
                    },
                }
            }
        };
        described.push(described_file);
    }
    tx.commit().await?;
    Ok(described)
}

/// Reads the source branch's two identifying columns.
///
/// Every column here is guaranteed present by `product_file_blob_or_source`,
/// so a missing one is a corrupt row rather than a case to handle, and saying
/// so names the constraint that was supposed to prevent it.
fn sourced(
    marketplace: Option<&str>,
    resource: Option<&str>,
) -> Result<(Marketplace, String), StorageError> {
    let raw = marketplace.ok_or_else(|| StorageError::CorruptRow {
        reason: "a file with no hash names no source marketplace, which \
                 product_file_blob_or_source forbids"
            .to_owned(),
    })?;
    let marketplace = crate::connections::marketplace_from_db(raw)?;
    let resource = resource.ok_or_else(|| StorageError::CorruptRow {
        reason: "a file with no hash names no source resource, which \
                 product_file_blob_or_source forbids"
            .to_owned(),
    })?;
    Ok((marketplace, resource.to_owned()))
}

/// One column of the source branch, which the CHECK guarantees is present.
fn required<T>(value: Option<T>, column: &str) -> Result<T, StorageError> {
    value.ok_or_else(|| StorageError::CorruptRow {
        reason: format!(
            "a file with no hash has no {column}, which product_file_blob_or_source forbids"
        ),
    })
}
