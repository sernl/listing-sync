//! The ledger's value vocabulary, owned here rather than borrowed from
//! `tam-storage`.
//!
//! These are the Phase 2 wire shapes: the device will send and receive them,
//! and the server converts to and from its own rows at the boundary. That is
//! why they are not `tam-storage`'s types — a schema change must be forced
//! through a conversion the compiler checks rather than silently re-cutting
//! the client contract.
//!
//! Every type here derives `Serialize` and `Deserialize`, and so do the types
//! they embed, derived where those live rather than restated here. One
//! definition is the whole point: the desktop client imports these, and
//! `tam-api` serialises them directly, so the envelope on the wire and the
//! envelope in the interpreter cannot drift apart.

use serde::{Deserialize, Serialize};

use crate::driver::VerifyPolicy;
use tam_domain::{ItemOperation, ItemOutcome, JobItemId, StepBudget};
use tam_marketplace::{
    CreateStrategy, FormId, IdempotencyKey, ProjectedListing, RemoteLifecycle, RemoteListingId,
};
use tam_types::{
    ContentHash, FailureCode, FailureDetail, FileId, InventoryId, JobId, MappingId, OrgId, Uuid,
};

/// Which organisation, item and epoch a fenced write speaks for. Every ledger
/// method is keyed on one, so the server derives org, connection, inventory
/// and epoch from the lease it issued rather than trusting a client argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRef {
    pub org: OrgId,
    pub item: JobItemId,
    pub lease_epoch: i64,
}

/// One claimed item, as the server handed it over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeasedItem {
    pub org: OrgId,
    pub item: JobItemId,
    pub job: JobId,
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub idempotency_key: IdempotencyKey,
    pub operation: ItemOperation,
    pub lease_epoch: i64,
    pub attempt_count: i32,
    pub requires_bound_on: Option<InventoryId>,
}

impl LeasedItem {
    #[must_use]
    pub const fn lease_ref(&self) -> LeaseRef {
        LeaseRef {
            org: self.org,
            item: self.item,
            lease_epoch: self.lease_epoch,
        }
    }
}

/// What a write attempt intends: the projected body and its hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptIntent {
    pub body: serde_json::Value,
    pub hash: Vec<u8>,
}

/// What opening an attempt asserts. A struct rather than four more
/// parameters, which is also what keeps the identity and the intent
/// travelling together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAttempt {
    /// Minted by the caller, so a lost response is recoverable by re-offering
    /// the same id rather than by spending another of the item's attempts.
    pub attempt: Uuid,
    pub mapping: MappingId,
    pub intent: AttemptIntent,
}

/// The two rows a settlement writes: the fenced attempt, and the mapping a
/// landed write binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRef {
    pub attempt: Uuid,
    pub mapping: MappingId,
}

/// What a settling attempt did to the mapping it names.
///
/// A create and a revise land; a removal severs; an attempt that addressed a
/// listing and changed nothing still says which listing that was. Splitting
/// these is what stops a committed removal binding the mapping to a listing
/// that no longer exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandingEffect {
    /// The verdict named no listing; there is nothing to record.
    None,
    /// The attempt addressed this listing and changed no binding.
    Addressed { id: RemoteListingId },
    /// The write landed here: bind the mapping to it, recording the lifecycle
    /// the verification read observed rather than the one the create
    /// convention would imply.
    Landed {
        id: RemoteListingId,
        lifecycle: RemoteLifecycle,
    },
    /// The write removed it: sever the binding.
    Severed { id: RemoteListingId },
}

/// How an attempt settled in the ledger's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptVerdict {
    pub state: String,
    pub failure_code: Option<FailureCode>,
    pub landing: LandingEffect,
}

/// What the settle did to the binding, as the server decided it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[must_use]
pub enum BindDisposition {
    NotLanded,
    Addressed,
    Bound,
    AlreadyBound,
    DivergentLanding { existing: RemoteListingId },
    ClaimedElsewhere { existing_mapping: MappingId },
    Refused { state: String },
    Severed,
    SeverRefused { state: String },
    SeverDiverged { existing: RemoteListingId },
}

/// How an item settled in the ledger's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemVerdict {
    pub outcome: ItemOutcome,
    pub failure_code: Option<FailureCode>,
    pub failure_detail: Option<FailureDetail>,
}

/// Where an item's run of failed preflights stands: how many in a row, and
/// whether every one was the marketplace's edge rather than something the
/// seller could act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightStreak {
    pub failures: u32,
    pub edge_only: bool,
}

/// A windowed allowance the server issued. The window and the ceiling are
/// absent by construction: a governed party that supplied either would be
/// setting its own rate limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetGrant {
    Granted { used: i32 },
    Exhausted,
}

/// Which allowance a grant is drawn against. The driver states what it is
/// about to do; the server decides what that costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrantKind {
    /// A lifecycle write: a create, a revise or a removal.
    Write,
    /// One try of a verification read-back.
    VerifyRead,
}

/// The conditions the interpreter branches on, and nothing else.
///
/// `StorageError` cannot cross this boundary: its database variant is
/// `#[from] sqlx::Error`, which is the dependency this crate exists to refuse.
/// The two named variants are the two the driver actually tests for; every
/// other server-side condition arrives opaque, because the interpreter's only
/// answer to one is the stall bias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerError {
    /// The mapping already has an open attempt, which is the duplicate-create
    /// fence holding.
    AttemptInFlight,
    /// Another holder owns this item now; the epoch fence refused the write.
    StaleLease,
    /// Anything else the server refused, carried as text because the
    /// interpreter cannot act on the distinction.
    Refused { detail: String },
}

impl core::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AttemptInFlight => f.write_str("an attempt is already in flight"),
            Self::StaleLease => f.write_str("the lease epoch is stale"),
            Self::Refused { detail } => write!(f, "the ledger refused the write: {detail}"),
        }
    }
}

impl core::error::Error for LedgerError {}

/// One file the operation uploads, as the server commits to it before the
/// bytes move.
///
/// The hash and the length are the commitment: the device fetches the bytes
/// separately and checks what arrived against these before it uploads
/// anything, so a truncated or substituted transfer is caught on the device
/// rather than discovered on the marketplace. The name and the content type
/// are what the upload form carries, derived from the file's kind by the same
/// rule the server's own file source uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadManifest {
    pub file: FileId,
    pub file_name: String,
    pub content_type: String,
    /// The blake3 content hash of the stored bytes.
    pub hash: ContentHash,
    pub byte_len: i64,
}

/// What the server has decided about an item, which is everything the device
/// needs to run it and nothing about how to ask a marketplace for it.
///
/// This is the preparation rather than the seed, and the distinction is the
/// custody line. The device renders the field set through its own adapter and
/// derives the intent hash over what it rendered, so the recorded intent is
/// the bytes the submit will carry. A server that shipped the rendered field
/// set would be composing, which is the architecture at S3 rather than S1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemPreparation {
    pub operation: ItemOperation,
    /// The projected listing, absent for a removal, which describes nothing.
    pub projected: Option<ProjectedListing>,
    pub form: FormId,
    pub strategy: CreateStrategy,
    pub budget: StepBudget,
    pub verify: VerifyPolicy,
}

/// One item's whole work order.
///
/// `server_now_ms` travels beside `server_deadline_ms` so the device drives
/// its budgets off the difference rather than off its own clock: the two
/// together are a duration the server vouches for, where an absolute instant
/// alone would be two clocks being compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkOrder {
    pub lease: LeasedItem,
    pub preparation: ItemPreparation,
    pub payload: Vec<PayloadManifest>,
    pub server_now_ms: i64,
    pub server_deadline_ms: i64,
    pub next_poll_ms: u64,
}

/// What the device is told when it asks for work.
///
/// The three answers are distinct on purpose. `Work` carries an item; `Idle`
/// says the queue is empty; `Held` says a sibling device holds the only slot
/// for this marketplace account, which is what lets the asking device back off
/// rather than poll a queue it cannot win.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClaimView {
    Work(Box<WorkOrder>),
    Idle { next_poll_ms: u64 },
    Held { next_poll_ms: u64 },
}

/// What the device reports when the run is over, fenced on the lease it ran
/// under: a settle naming a run the caller is not in is refused rather than
/// written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettleEnvelope {
    pub lease: LeaseRef,
    pub verdict: ItemVerdict,
}
