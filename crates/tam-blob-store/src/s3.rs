//! The S3-compatible arm of the object store: path-style URLs, SigV4 over
//! every request, one bucket. Written against the wire protocol rather than an
//! SDK for the reason [`crate::sigv4`] gives.

use std::sync::Arc;

use sha2::{Digest as _, Sha256};
use tam_pipeline::store::{ensure_flat_key, ObjectStore, StoreError};

use crate::sigv4::{authorization, hex, CanonicalRequest, SigningTime};
use crate::Credentials;

/// The service name every S3-compatible endpoint signs under, Garage and
/// MinIO included: it is part of the signature's scope, not a vendor name.
const SERVICE: &str = "s3";

/// Named because the lint table refuses a client built without them. One
/// object is bounded by the upload cap and the endpoint is a neighbour, so
/// these are generous on the body and short on the connection.
const REQUEST_TIMEOUT_SECS: u64 = 60;
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// Everything about one bucket that does not change between requests, held
/// behind an `Arc` so a clone per handler copies two pointers rather than the
/// credentials.
struct Bucket {
    /// The endpoint with no trailing slash, so `{base}{path}` is one url.
    base: String,
    /// What the `host` header carries, which is what the signature covers.
    host: String,
    name: String,
    region: String,
    credentials: Credentials,
}

/// One bucket over HTTP. Cloning shares the connection pool and the
/// credentials, which is what lets a handler take a store per request without
/// opening a second pool.
#[derive(Clone)]
pub struct S3ObjectStore {
    client: reqwest::Client,
    bucket: Arc<Bucket>,
}

impl S3ObjectStore {
    /// `endpoint` carries its scheme, as an operator writes it:
    /// `http://garage.internal:3900`.
    pub fn new(
        endpoint: &str,
        bucket: String,
        region: String,
        credentials: Credentials,
    ) -> Result<Self, String> {
        let base = endpoint.trim_end_matches('/');
        let authority = base
            .strip_prefix("https://")
            .or_else(|| base.strip_prefix("http://"))
            .ok_or_else(|| format!("{endpoint} needs an http:// or https:// scheme"))?;
        let host = authority.split('/').next().unwrap_or(authority);
        if host.is_empty() {
            return Err(format!("{endpoint} names no host"));
        }
        if bucket.is_empty() {
            return Err("the bucket name is empty".to_owned());
        }
        let client = reqwest::Client::builder()
            .timeout(core::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .connect_timeout(core::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .build()
            .map_err(|why| format!("the object-store client could not be built: {why}"))?;
        Ok(Self {
            client,
            bucket: Arc::new(Bucket {
                base: base.to_owned(),
                host: host.to_owned(),
                name: bucket,
                region,
                credentials,
            }),
        })
    }

    /// What a start-up line prints: the bucket and where it is, and neither
    /// key.
    pub(crate) fn describe(&self) -> String {
        format!(
            "bucket {} at {} in region {}",
            self.bucket.name, self.bucket.base, self.bucket.region
        )
    }

    /// The signed headers for one request, in the order the canonical form
    /// defines: `host`, then the two `x-amz-` headers, sorted by name.
    fn signed(
        &self,
        method: &str,
        path: &str,
        payload_sha256: &str,
    ) -> Result<[(&'static str, String); 4], StoreError> {
        let time = SigningTime::from_unix_seconds(now_unix_seconds()?)
            .map_err(|why| StoreError::Backend(why.to_string()))?;
        let header = authorization(
            &CanonicalRequest {
                method,
                path,
                query: "",
                headers: &[
                    ("host", &self.bucket.host),
                    ("x-amz-content-sha256", payload_sha256),
                    ("x-amz-date", &time.stamp),
                ],
                payload_sha256,
            },
            &self.bucket.credentials,
            &self.bucket.region,
            SERVICE,
            &time,
        )
        .map_err(|why| StoreError::Backend(why.to_string()))?;
        Ok([
            ("host", self.bucket.host.clone()),
            ("x-amz-content-sha256", payload_sha256.to_owned()),
            ("x-amz-date", time.stamp),
            ("authorization", header),
        ])
    }

    fn url(&self, key: &str) -> String {
        format!("{}/{}/{key}", self.bucket.base, self.bucket.name)
    }

    fn path(&self, key: &str) -> String {
        format!("/{}/{key}", self.bucket.name)
    }
}

impl ObjectStore for S3ObjectStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
        ensure_flat_key(key)?;
        let payload = hex(&Sha256::digest(&bytes));
        let mut request = self
            .client
            .put(self.url(key))
            .header("content-type", "application/octet-stream");
        for (name, value) in self.signed("PUT", &self.path(key), &payload)? {
            request = request.header(name, value);
        }
        let response = request
            .body(bytes)
            .send()
            .await
            .map_err(|why| StoreError::Backend(format!("put {key}: {why}")))?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(StoreError::Backend(format!(
                "put {key}: the object store answered {status}"
            )))
        }
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, StoreError> {
        ensure_flat_key(key)?;
        let payload = hex(&Sha256::digest([]));
        let mut request = self.client.get(self.url(key));
        for (name, value) in self.signed("GET", &self.path(key), &payload)? {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|why| StoreError::Backend(format!("get {key}: {why}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(StoreError::NotFound);
        }
        if !status.is_success() {
            return Err(StoreError::Backend(format!(
                "get {key}: the object store answered {status}"
            )));
        }
        // Bounded by the upload cap and held in memory across the whole
        // pipeline, exactly as the local store's read is.
        let bytes = response
            .bytes()
            .await
            .map_err(|why| StoreError::Backend(format!("get {key}: {why}")))?;
        Ok(bytes.to_vec())
    }
}

/// The one clock read in this crate: a signature is only valid within minutes
/// of the instant it names, so it cannot be supplied as data the way the
/// pipeline's timestamps are.
#[expect(
    clippy::disallowed_methods,
    reason = "a SigV4 stamp must be the signing instant, which no caller can hand in"
)]
fn now_unix_seconds() -> Result<i64, StoreError> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|why| StoreError::Backend(format!("the host clock predates 1970: {why}")))?;
    i64::try_from(since_epoch.as_secs())
        .map_err(|why| StoreError::Backend(format!("the host clock is not signable: {why}")))
}
