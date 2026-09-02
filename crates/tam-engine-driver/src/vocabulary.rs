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
use tam_domain::{ItemOperation, ItemOutcome, JobItemId};
use tam_marketplace::{IdempotencyKey, RemoteLifecycle, RemoteListingId};
use tam_types::{FailureCode, FailureDetail, InventoryId, JobId, MappingId, OrgId, Uuid};

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
