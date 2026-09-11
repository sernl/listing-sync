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
pub mod billing;
pub mod blobs;
mod codec;
pub mod connections;
pub mod device;
pub mod entitlement;
pub mod file_source;
pub mod import_batches;
pub mod job_reads;
pub mod jobs;
pub mod labels;
pub mod lowering;
mod mapping;
pub mod marketplace_requests;
pub mod notifications;
pub mod operators;
pub mod org;
pub mod overrides;
mod product;
pub mod profile;
pub mod pruning;
pub mod resource_templates;
pub mod sessions;
pub mod taxonomy;
pub mod tpt_base;

pub use analytics::{AnalyticsRepo, LatestMetric, MetricSnapshot};
pub use authorship::{AuthorshipRecord, ConnectionFactsRepo};
pub use backoffice::{
    BackofficeRepo, DailyCount, DeadLetterTopic, FailedWrite, HaltRecord, IdentityAuditRepo,
    ImpersonationEvent, ImportDrainPage, ImportDrainRun, OrgDetail, OrgSummary, SignupsRepo,
    SubscriptionRecord, SyncHealth,
};
pub use billing::{BillingRepo, SubscriptionState};
pub use blobs::{
    describe_files, BlobError, BlobRepo, PipelineFileSource, StoredFile, TenantBlobSink,
};
pub use connections::{
    record_connection_event, ConnectionAudit, ConnectionAuditRow, ConnectionEventRecord,
    ConnectionRepo, ConnectionRow,
};
pub use device::{
    DeviceHeartbeat, DeviceRecord, DeviceRegistration, DeviceRepo, DeviceSessionRecord,
    DeviceSessionReport, DeviceSessionStatus,
};
pub use entitlement::{EntitlementRepo, Grant, GrantRecord, GrantedBy, NewGrant, Usage};
pub use file_source::ProductFileSourceRepo;
pub use import_batches::{
    AttachCounts, BatchState, BatchWrite, BindOutcome, BoundRow, ClaimedRow, CommitCounts,
    CommitOpening, ImportBatchDraftRecord, ImportBatchRecord, ImportBatchRepo,
    ImportBatchRowRecord, NewImportBatch, NewImportBatchRow, RowAddress, RowFile, RowFiles,
    RowIntent, RowRef, RowState, StaleReport, SweepReport, UnbindOutcome, BATCHES_LISTED_MAX,
};
pub use job_reads::{
    intent_digest, payload_digest, EventRow, ItemCounts, ItemRow, ItemStateKind, ItemsPageParams,
    JobListRow, JobReadRepo, JobSnapshot, LedgerCursor, MappingSeed,
};
pub use jobs::{
    append_event, append_event_asserted, revive_by_gap, revive_counterparts, revive_on,
    settle_if_complete, AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant,
    Charged, ClaimPolicy, CreatedJob, DeviceClaim, DeviceRef, EventScope, HaltCause, HaltRepo,
    InventoryFailureWindow, InventoryHaltRow, ItemVerdict, JobOrigin, JobRepo, LandingEffect,
    LeaseRef, LeaseRepo, LeasedItem, MessageRef, NewAttempt, NewJob, NewJobItem, NewOutboxMessage,
    OutboxMessage, OutboxRepo, RateBudgetRepo, RenewedLease, Revived, WriteAttemptRepo, ALL_GATES,
    AWAITING_COUNTERPART, AWAITING_MARKETPLACE_ANSWER, AWAITING_SELLER_SIGNIN, ELECTION,
    REAUTH_REQUIRED, REVIVABLE_GATES,
};
pub use labels::{Colour, LabelRecord, LabelRename, LabelRepo};
pub use lowering::{
    lower, requires_bound_on, uncaptured_source, uncaptured_transition, LoweringRefusal,
};
pub use mapping::{
    BoundListing, LossScope, MappingAdd, MappingHead, MappingRecord, MappingRepo, PastedBind,
    RecordedLoss,
};
pub use marketplace_requests::{
    MarketplaceRequestBackofficeRepo, MarketplaceRequestRecord, MarketplaceRequestRepo,
    MarketplaceRequestWrite, NewMarketplaceRequest, PAGE_LIMIT_MAX as REQUEST_PAGE_LIMIT_MAX,
    REQUESTS_PER_ORG_MAX,
};
pub use notifications::{
    NotificationCursor, NotificationRecord, NotificationRepo, Recipient, JOB_SETTLED_TOPIC,
};
pub use operators::{OperatorRecord, OperatorRepo};
pub use org::{OrgRecord, OrgRepo, OrgWrite};
pub use overrides::OverrideRepo;
pub use product::{
    ExportedListing, ExportedResource, FileRefusal, FileReplacement, FileSwap, FileTarget,
    ProductEdit, ProductFiles, ProductRecord, ProductRepo, ProductSummary, ReplacedFiles,
    StoredCover, ThumbnailChange,
};
pub use profile::{AvatarWrite, ProfileRepo};
pub use pruning::{PruneRepo, PruneReport};
pub use resource_templates::{
    NewResourceTemplate, ResourceTemplateRecord, ResourceTemplateRepo, ResourceTemplateSummary,
    TemplateChange, TemplateEdit, TemplateWrite, TEMPLATES_PER_ORG_MAX,
};
pub use sessions::{NewTenant, SessionIdentity, SessionRepo, SessionToken};
pub use tpt_base::{TptBaseRecord, TptBaseRepo};
pub mod elections;
pub mod sync_requests;
pub use elections::{AnswerReport, AnsweredElection, ElectionRepo, NewAnswer, OpenElection};
pub use sync_requests::{
    job_request_key, Canonicalised, Completion, Disposition, Enqueued, Mint, NewSyncRequest,
    Observed, ResourceCoverage, SyncIntent, SyncRequestRecord, SyncRequestRepo, SyncRequestSummary,
    SyncResourceRecord, CREATE_LEG, IMPORT_LEG, REMOVE_LEG,
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
    /// A create was refused at `open` because the mapping is already bound.
    ///
    /// Distinct from [`Self::AttemptInFlight`] because the caller must act
    /// differently: an attempt in flight is a fence that may still clear,
    /// while a bound mapping means the listing this create would have made
    /// already exists, and no later run will change that.
    #[error("that mapping is already bound, so this create has nothing to make")]
    MappingAlreadyBound,
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

/// The tenant pin, for a caller assembling its own transaction across this
/// crate's boundary.
///
/// Public because `tam-engine`'s ledger opens transactions of its own and
/// serves them under `tam_app` for a device, where forced row-level security
/// is the tenancy: an unpinned transaction there writes nothing and reads
/// nothing, which is a fence that refuses everyone including the holder.
pub async fn pin_tenant(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
) -> Result<(), StorageError> {
    pin_org(tx, org).await
}

/// Declares the tenant for the rest of this transaction.
///
/// `set_config` with `is_local = true` resets at transaction end, so a pooled
/// connection never leaks one tenant's pin into the next request.
///
/// Every write and read this crate serves under `tam_app` passes through here,
/// because `job_item` and its neighbours carry forced row-level security: an
/// unpinned statement matches no rows, so it writes nothing and answers as
/// though a fence had refused it. Under the engine role, which bypasses
/// row-level security, the same statement is fine — which is what makes the
/// omission survive every test that drives it through the worker.
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
