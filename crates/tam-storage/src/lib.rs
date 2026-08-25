//! Organisation-first repositories over PostgreSQL with row-level security.
//!
//! Every repository method takes the tenant as its first positional parameter
//! and pins it into the connection as a transaction-local `app.current_org`
//! setting, so the row-level-security policies in `migrations/` are a boundary
//! a forgotten `WHERE` clause cannot cross. The queries still filter by
//! `org_id` explicitly; the policies are the backstop the tenancy tests prove,
//! not the only fence.

#![forbid(unsafe_code)]

pub mod blobs;
mod codec;
pub mod jobs;
mod mapping;
mod product;
pub mod sessions;
pub mod taxonomy;

pub use blobs::{BlobError, BlobRepo, PipelineFileSource, TenantBlobSink};
pub use jobs::{
    append_event, AttemptIntent, AttemptVerdict, BudgetGrant, EventScope, HaltCause, HaltRepo,
    InventoryFailureWindow, JobRepo, LeaseRef, LeaseRepo, LeasedItem, MessageRef, NewJob,
    NewJobItem, NewOutboxMessage, OutboxMessage, OutboxRepo, RateBudgetRepo, WriteAttemptRepo,
};
pub use mapping::{MappingRecord, MappingRepo};
pub use product::{ProductRecord, ProductRepo, ProductSummary};
pub use sessions::{SessionIdentity, SessionRepo, SessionToken};
pub use taxonomy::{DrainStats, OpenItem, RaiseReport, RaiseScope, SeedReport, TaxonomyRepo};

use sqlx::{Postgres, Transaction};
use tam_types::OrgId;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("timestamp {millis}ms is outside the representable range")]
    TimestampOutOfRange { millis: i64 },
    #[error("stored row violates a domain invariant: {reason}")]
    CorruptRow { reason: String },
    #[error("the aggregate names a different organisation than the call pinned")]
    OrgMismatch,
    #[error("the aggregate is inconsistent: {reason}")]
    Inconsistent { reason: String },
    #[error("the lease epoch is stale; another worker holds this item")]
    StaleLease,
    #[error("idempotency key {key} already has an item")]
    DuplicateIdempotencyKey { key: uuid::Uuid },
    #[error("another write attempt is in flight for this mapping")]
    AttemptInFlight,
}

/// Declares the tenant for the rest of this transaction. `set_config` with
/// `is_local = true` resets at transaction end, so a pooled connection never
/// leaks one tenant's pin into the next request.
pub(crate) async fn pin_org(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
) -> Result<(), StorageError> {
    let org_text = codec::uuid_to_db(org.0).to_string();
    sqlx::query!("SELECT set_config('app.current_org', $1, true)", org_text)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}
