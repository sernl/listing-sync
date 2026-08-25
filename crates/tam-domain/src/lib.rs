//! The pure correctness core: taxonomy projection, the canonical product, the
//! mapping aggregate, the job-ledger vocabulary and the sans-IO `SyncMachine`.
//!
//! Every definition is promoted verbatim from `docs/design/sketches/domain.rs`,
//! the artefact of record for the domain types. Nothing here performs I/O;
//! every action the machine wants done is an `Effect` in a returned
//! `EffectList`, and the transition table is specified in
//! `docs/design/sync-machine.md`.

#![forbid(unsafe_code)]

use tam_marketplace::{
    AdapterError, ChallengeKind, CorrelationMarker, CreateStrategy, FetchReason, FieldSet, FormId,
    FormSchemaFingerprint, IdempotencyKey, ListingLocator, ObservedListing, Outcome,
    RemoteLifecycle, RemoteListingId, SchemaDrift, SubmitEvidence, WriteAttemptId,
};
use tam_types::{
    AttemptId, CanonicalTermId, ConnectionId, ContentHash, FieldMismatch, FileId, InventoryId,
    ListingCopy, LogicalInstant, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, Timestamp, Title, UserId, Uuid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobItemId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermKind {
    Subject,
    Topic,
    ResourceType,
    Phase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTerm {
    pub id: CanonicalTermId,
    pub kind: TermKind,
    pub parent: Option<CanonicalTermId>,
    pub label: String,
}

/// A vocabulary is per inventory, not per marketplace, because Tes GB and Tes
/// US were measured as structurally different trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VocabularyId(pub InventoryId, pub TermKind);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyPath {
    pub vocabulary: VocabularyId,
    pub segments: Vec<String>,
    /// The marketplace's own identifier for the term where it exposes one.
    pub native_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Exact,
    Broader,
    Narrower,
}

/// Who asserted an edge. A model is deliberately absent from this enum: the
/// decision record confines large language models to listing-copy generation
/// and selector rediscovery, so a model may not author a taxonomy edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decider {
    Imported { source: String },
    Human { user: UserId, org: OrgId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionEdge {
    pub from: CanonicalTermId,
    pub to: VocabularyPath,
    pub kind: EdgeKind,
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}

/// The result of projecting one canonical term into one target vocabulary.
/// `Absent` is a first-class answer and never silently becomes a default term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermProjection {
    Exact {
        to: VocabularyPath,
    },
    Broadened {
        to: VocabularyPath,
        dropped: Vec<CanonicalTermId>,
    },
    Ambiguous {
        candidates: Vec<VocabularyPath>,
    },
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationItem {
    pub org: OrgId,
    pub term: CanonicalTermId,
    pub target: VocabularyId,
    pub raised_by: MappingId,
    pub raised_at: Timestamp,
    pub state: ReconciliationState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationState {
    Open,
    /// Resolved by writing a durable edge, so the queue drains rather than
    /// re-raising the same term on the next product.
    Resolved {
        edge: ProjectionEdge,
    },
    /// The term genuinely has no counterpart and the mapping must omit it.
    NoCounterpart {
        decided_by: Decider,
        at: Timestamp,
    },
}

/// The seller's own grade or phase declaration, kept verbatim. The age interval
/// is derived from it; the declaration is the fact, and re-emitting to the
/// vocabulary it came from uses the declaration rather than a round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeDeclaration {
    pub source: DeclarationSource,
    pub raw: Vec<VocabularyPath>,
    pub derived: Option<AgeInterval>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationSource {
    Imported { vocabulary: VocabularyId },
    Seller,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgeInterval {
    low_years: u8,
    high_years: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeIntervalError {
    Inverted,
}

impl AgeInterval {
    pub fn new(low_years: u8, high_years: u8) -> Result<Self, AgeIntervalError> {
        if low_years > high_years {
            return Err(AgeIntervalError::Inverted);
        }
        Ok(Self {
            low_years,
            high_years,
        })
    }

    #[must_use]
    pub const fn low_years(self) -> u8 {
        self.low_years
    }

    #[must_use]
    pub const fn high_years(self) -> u8 {
        self.high_years
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeverCause {
    RemovedByMarketplace,
    RemovedBySeller,
    NotFoundOnVerify,
}

/// The binding between a canonical product and a remote listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Unbound,
    /// A create is in flight. There is no durable identifier yet, which is
    /// exactly what makes an ambiguous create the hardest state in the system.
    Creating {
        attempt: AttemptId,
        marker: Option<CorrelationMarker>,
    },
    Bound {
        id: RemoteListingId,
        first_seen: Timestamp,
        verified: Verification,
    },
    /// A create may or may not have landed and reconciliation could not decide.
    /// Never retried; escalated, with this inventory halted for this tenant.
    AmbiguousCreate {
        attempt: AttemptId,
        candidates: Vec<RemoteListingId>,
        since: Timestamp,
    },
    Severed {
        was: RemoteListingId,
        noticed: Timestamp,
        cause: SeverCause,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    Stale {
        since: Timestamp,
    },
    Clean {
        at: Timestamp,
    },
    Mismatched {
        at: Timestamp,
        first: FieldMismatch,
        rest: Vec<FieldMismatch>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldPolicy {
    /// We own this field; a difference is a defect and is corrected next run.
    Managed,
    /// The seller edited it on the marketplace; we read it back, never write.
    Frozen,
    /// We compute a value and present it as a proposal for per-field accept.
    Propose,
}

/// One field per `FieldKey`, so a policy can be neither missing nor unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldPolicies {
    pub title: FieldPolicy,
    pub description: FieldPolicy,
    pub price: FieldPolicy,
    pub taxonomy: FieldPolicy,
    pub grades: FieldPolicy,
    pub files: FieldPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishMode {
    DryRun,
    Propose,
    Publish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub id: MappingId,
    pub org: OrgId,
    pub product: ProductId,
    pub inventory: InventoryId,
    pub binding: Binding,
    pub policies: FieldPolicies,
    pub price_rule: PriceRule,
    pub publish: PublishMode,
    pub lifecycle: RemoteLifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingError {
    IdentifierMarketplaceMismatch,
}

impl Mapping {
    /// The one invariant a mapping can violate without help: holding a durable
    /// identifier minted by a different marketplace.
    pub fn check(&self) -> Result<(), MappingError> {
        let held = match &self.binding {
            Binding::Bound { id, .. } => Some(id.marketplace()),
            Binding::Severed { was, .. } => Some(was.marketplace()),
            Binding::Unbound | Binding::Creating { .. } | Binding::AmbiguousCreate { .. } => None,
        };
        match held {
            Some(m) if m != self.inventory.marketplace() => {
                Err(MappingError::IdentifierMarketplaceMismatch)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalProduct {
    pub id: ProductId,
    pub org: OrgId,
    pub title: Title,
    pub body: ListingCopy,
    pub payload: PayloadSet,
    pub cover: Option<ProductFile>,
    pub previews: Vec<ProductFile>,
    pub subjects: Vec<CanonicalTermId>,
    pub grades: GradeDeclaration,
    pub price: PriceIntent,
}

/// The per-inventory rendering of a canonical product. Derived, never authored,
/// and produced by a pure function so it is property-testable without I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingProjection {
    pub inventory: InventoryId,
    pub title: String,
    pub body: String,
    pub price: PriceIntent,
    pub taxonomy: Vec<VocabularyPath>,
    pub grades: Vec<VocabularyPath>,
    pub files: Vec<FileId>,
    pub loss: Vec<TermProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionBlocked {
    Taxonomy { items: Vec<ReconciliationItem> },
    CurrencyUnknown { inventory: InventoryId },
    CoverMissing,
    ScanIncomplete { file: FileId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemOutcome {
    Succeeded,
    /// The listing is live and provably shorter or lossier than intended.
    /// Resending makes it worse, so this is terminal and not a retry class.
    Degraded,
    Failed,
    Ambiguous,
    Skipped,
    Blocked,
}

/// Two lifecycles, deliberately not collapsed into one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Unlinked,
    Linking,
    Linked,
    NeedsReauth,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    Queued,
    Leased { until: Timestamp },
    Running,
    Blocked { on: BlockCause },
    ParkedLive { expires: Timestamp },
    ParkedCold,
    Verifying,
    Settled { outcome: ItemOutcome },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCause {
    Reauth,
    Challenge,
    Reconciliation,
    RateGovernor,
}

/// The remaining interpreter-action allowance for this job. Distinct from the
/// machine's own `step`, which is the transition function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepBudget {
    pub actions_remaining: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltScope {
    /// One tenant, one inventory. What an ambiguous create raises.
    OrgInventory { org: OrgId, inventory: InventoryId },
    /// One tenant, every inventory.
    Org { org: OrgId },
    /// Every tenant, one inventory. The fleet kill switch.
    FleetInventory { inventory: InventoryId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCause {
    Ambiguity,
    SchemaDrift,
    VerificationMismatch,
    UnexpectedOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellerEvent {
    ReauthRequired,
    ItemParked,
    JobSettled,
    InventoryHalted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    AwaitingPreflight,
    PreflightAsserted {
        schema: FormSchemaFingerprint,
    },
    IntentRecorded {
        attempt: WriteAttemptId,
    },
    Submitted {
        attempt: WriteAttemptId,
        evidence: SubmitEvidence,
    },
    AwaitingReadBack {
        attempt: WriteAttemptId,
        locator: ListingLocator,
    },
    Parked {
        attempt: Option<WriteAttemptId>,
        challenge: ChallengeKind,
    },
    Terminal(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    PreflightResult(Result<FormSchemaFingerprint, SchemaDrift>),
    IntentRecorded(WriteAttemptId),
    SubmitResult(Result<SubmitEvidence, AdapterError>),
    ReadBackResult(Result<ObservedListing, AdapterError>),
    ReconcileResult(Result<Option<RemoteListingId>, AdapterError>),
    /// The seller answered the challenge and the item may resume.
    ChallengeCleared,
    /// The park expired unanswered.
    ParkExpired,
    /// The interpreter's action allowance ran out.
    BudgetExhausted,
}

/// The closed effect set. Nothing the machine wants done is outside it, which
/// is what makes the interpreter auditable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    AssertFormSchema {
        form: FormId,
    },
    RecordIntent {
        attempt: WriteAttemptId,
        intent_hash: ContentHash,
    },
    Submit {
        attempt: WriteAttemptId,
        key: IdempotencyKey,
        fields: FieldSet,
    },
    ReadBack {
        locator: ListingLocator,
        reason: FetchReason,
    },
    /// Search for a marker or an in-flight attempt's landing, which is the only
    /// permitted response to an ambiguous submit.
    Reconcile {
        attempt: WriteAttemptId,
        locator: ListingLocator,
    },
    /// Tear the browser down and hold the durable item.
    ParkItem {
        item: JobItemId,
        challenge: ChallengeKind,
        expires: LogicalInstant,
    },
    /// Put every item for this connection back in the queue behind a gate,
    /// idempotency keys intact.
    RequeueBehindGate {
        connection: ConnectionId,
        cause: BlockCause,
    },
    CaptureDiagnostics {
        attempt: WriteAttemptId,
        cause: CaptureCause,
    },
    Halt {
        scope: HaltScope,
    },
    Notify {
        org: OrgId,
        event: SellerEvent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectList(pub Vec<Effect>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub next: SyncMachine,
    pub effects: EffectList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineError {
    /// The input cannot apply to the state the machine is in.
    InputNotApplicable,
    /// An input named a write attempt this machine does not own.
    AttemptMismatch,
    /// A transition would emit more effects than the budget permits.
    EffectBudgetExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMachine {
    pub org: OrgId,
    pub inventory: InventoryId,
    pub item: JobItemId,
    pub key: IdempotencyKey,
    pub strategy: CreateStrategy,
    pub state: SyncState,
    pub budget: StepBudget,
}

impl SyncMachine {
    /// Consumes the machine so a stale state cannot be stepped twice, and takes
    /// `now` as a parameter rather than reading a clock.
    ///
    /// The body is a stub until the transition table specified in
    /// `docs/design/sync-machine.md` is implemented later in M1.
    pub fn step(self, input: Input, now: LogicalInstant) -> Result<Transition, MachineError> {
        drop((self, input, now));
        Err(MachineError::InputNotApplicable)
    }
}

/// The closed event vocabulary the progress stream carries. `job_event.kind` is
/// the serde tag of this enum and holds no other value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobEventKind {
    JobQueued,
    JobStarted,
    ItemQueued,
    ItemLeased,
    ItemActionStarted,
    ItemActionFinished,
    ItemBlocked,
    ItemParked,
    ItemResumed,
    ItemSettled,
    JobSettled,
    JobHalted,
}

/// Counts per terminal outcome. There is deliberately no scalar verdict: a job
/// with a hundred succeeded, five failed and three ambiguous items has no
/// honest single answer, so the roll-up reports the vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutcomeSummary {
    pub succeeded: u32,
    pub degraded: u32,
    pub failed: u32,
    pub ambiguous: u32,
    pub skipped: u32,
    pub blocked: u32,
}

/// A total function of the item states, computed rather than stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    /// Every unsettled item is waiting on the seller.
    BlockedOnSeller,
    /// A halt covers this job's inventory.
    Halted,
    Settled(OutcomeSummary),
}

#[cfg(test)]
mod prop_tests {
    use super::{AgeInterval, AgeIntervalError};
    use proptest::prelude::*;

    proptest! {
        /// The smart constructor accepts exactly the ordered pairs and round-trips them,
        /// and rejects exactly the inverted pairs.
        #[test]
        fn age_interval_constructor_contract(low in 0u8..=255, high in 0u8..=255) {
            match AgeInterval::new(low, high) {
                Ok(iv) => {
                    prop_assert!(low <= high);
                    prop_assert_eq!(iv.low_years(), low);
                    prop_assert_eq!(iv.high_years(), high);
                }
                Err(AgeIntervalError::Inverted) => prop_assert!(low > high),
            }
        }

        /// The illegal state is unrepresentable: a constructed interval never holds an
        /// inverted range, whatever it was built from.
        #[test]
        fn age_interval_never_inverted(low in 0u8..=255, high in 0u8..=255) {
            if let Ok(iv) = AgeInterval::new(low, high) {
                prop_assert!(iv.low_years() <= iv.high_years());
            }
        }
    }
}
