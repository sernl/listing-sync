//! Organisation-first repositories over PostgreSQL with row-level security.
//!
//! Every repository method takes the tenant as its first positional parameter
//! and pins it into the connection as a transaction-local `app.current_org`
//! setting, so the row-level-security policies in `migrations/` are a boundary
//! a forgotten `WHERE` clause cannot cross. The queries still filter by
//! `org_id` explicitly; the policies are the backstop the tenancy tests prove,
//! not the only fence.

#![forbid(unsafe_code)]

pub mod analytics;
pub mod authorship;
pub mod backoffice;
pub mod blobs;
mod codec;
pub mod connections;
pub mod job_reads;
pub mod jobs;
pub mod lowering;
mod mapping;
pub mod operators;
pub mod org;
mod product;
pub mod pruning;
pub mod sessions;
pub mod taxonomy;

pub use analytics::{AnalyticsRepo, LatestMetric, MetricSnapshot};
pub use authorship::{AuthorshipRecord, ConnectionFactsRepo};
pub use backoffice::{
    BackofficeRepo, DailyCount, FailedWrite, HaltRecord, OrgDetail, OrgSummary, SignupsRepo,
    SyncHealth,
};
pub use blobs::{BlobError, BlobRepo, PipelineFileSource, TenantBlobSink};
pub use connections::{
    record_connection_event, ConnectionAudit, ConnectionAuditRow, ConnectionEventRecord,
    ConnectionRepo, ConnectionRow,
};
pub use job_reads::{
    intent_digest, payload_digest, EventRow, ItemCounts, ItemRow, ItemStateKind, ItemsPageParams,
    JobListRow, JobReadRepo, JobSnapshot, LedgerCursor, MappingSeed,
};
pub use jobs::{
    append_event, revive_by_gap, revive_counterparts, revive_on, settle_if_complete, AttemptIntent,
    AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, CreatedJob, EventScope, HaltCause,
    HaltRepo, InventoryFailureWindow, InventoryHaltRow, ItemVerdict, JobRepo, LandingEffect,
    LeaseRef, LeaseRepo, LeasedItem, MessageRef, NewAttempt, NewJob, NewJobItem, NewOutboxMessage,
    OutboxMessage, OutboxRepo, RateBudgetRepo, WriteAttemptRepo, AWAITING_COUNTERPART, ELECTION,
    REAUTH_REQUIRED, REVIVABLE_GATES,
};
pub use lowering::{
    lower, requires_bound_on, uncaptured_source, uncaptured_transition, LoweringRefusal,
};
pub use mapping::{BoundListing, LossScope, MappingHead, MappingRecord, MappingRepo, RecordedLoss};
pub use operators::{OperatorRecord, OperatorRepo};
pub use org::{OrgRecord, OrgRepo};
pub use product::{ProductRecord, ProductRepo, ProductSummary};
pub use pruning::{PruneRepo, PruneReport};
pub use sessions::{NewTenant, SessionIdentity, SessionRepo, SessionToken};
pub mod elections;
pub mod sync_requests;
pub use elections::{AnswerReport, ElectionRepo, NewAnswer, OpenElection};
pub use sync_requests::{
    job_request_key, Canonicalised, Disposition, Enqueued, NewSyncRequest, SyncIntent,
    SyncRequestRecord, SyncRequestRepo, SyncResourceRecord, CREATE_LEG, REMOVE_LEG,
};
pub use taxonomy::{
    DrainStats, NoCounterpartReport, OpenItem, RaiseReport, RaiseScope, SeedReport, TaxonomyRepo,
};

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
    /// A second mapping claims a listing another mapping already binds.
    ///
    /// Reachable through the ordinary seller path rather than only through a
    /// corrupt write: a migrate mints a fresh product per read and never
    /// dedupes by remote id, so submitting the same source listing twice
    /// under two idempotency keys lands here. Named rather than left as a
    /// bare unique violation, which surfaces as a 500 for what is a
    /// validation answer.
    #[error("that listing is already bound to another mapping")]
    ListingAlreadyBound,
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
