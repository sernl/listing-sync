//! The pure correctness core: taxonomy projection, the canonical product, the
//! mapping aggregate, the job-ledger vocabulary and the sans-IO `SyncMachine`.
//!
//! Every definition is promoted verbatim from `docs/design/sketches/domain.rs`,
//! the artefact of record for the domain types. Nothing here performs I/O;
//! every action the machine wants done is an `Effect` in a returned
//! `EffectList`, and the transition table is specified in
//! `docs/design/sync-machine.md`.

#![forbid(unsafe_code)]

pub mod equivalence;
pub mod registry;

use tam_marketplace::{
    settle, verify_after, AdapterError, AmbiguityCause, ChallengeKind, CorrelationMarker,
    CreateStrategy, EvidenceRef, FetchReason, FieldDiffReport, FieldSet, FormId,
    FormSchemaFingerprint, IdempotencyKey, LifecycleTransition, ListingLocator, ListingState,
    ObservedListing, Outcome, RemoteLifecycle, RemoteListingId, SchemaDrift, SubmitEvidence,
    WriteAttemptId,
};
use tam_types::{
    AttemptId, CanonicalTermId, ConnectionId, ContentHash, FailureCode, FailureDetail,
    FieldMismatch, FileId, ImportedTerm, InventoryId, ListingCopy, LogicalInstant, MappingId,
    OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, Timestamp, Title, UserId,
    Uuid,
};

/// The axis vocabulary lives in `tam-types` because the adapter seam names it
/// too: `ImportedTerm` crosses in `tam-marketplace`, which this crate depends
/// on rather than the other way round. Re-exported here because the taxonomy
/// hub is where it is reasoned about.
pub use tam_types::TermKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobItemId(pub Uuid);

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

/// The durable record that a term genuinely has no counterpart in a target
/// vocabulary, which turns `Absent` from a publish blocker into an omission.
///
/// The tenant path answers one open queue item at a time; this is the shape a
/// derivation emits in bulk, for an omission it computed rather than one a
/// seller declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoCounterpart {
    pub term: CanonicalTermId,
    pub target: VocabularyId,
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

/// What the source stated about the rights it grants, kept as the source's
/// own value rather than as a canonical one.
///
/// A licence is a legal instrument and a translation of one is a different
/// instrument, so nothing derives one vocabulary's grant from another's. The
/// projection either finds an edge the seller authored or asks them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RightsDeclaration {
    /// No grant was captured. The honest state of every product imported
    /// before this existed — the import read the licence and discarded it —
    /// and the reason the backfill is this rather than a plausible default.
    Unstated,
    Declared {
        source: VocabularyPath,
    },
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
    pub rights: RightsDeclaration,
    /// Source values in axes this model does not yet type, kept verbatim.
    ///
    /// Two jobs: a round trip back to the source platform loses nothing, and
    /// a projection into a platform that has no such field can name what it
    /// dropped rather than reducing the founder's tag-flattening requirement
    /// to a log line. Adding an axis later is a pure upgrade — values move
    /// out of residue into a typed axis with no data migration.
    pub native_residue: Vec<ImportedTerm>,
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

/// What one item does to the listing its mapping names. Closed, because the
/// driver's effect interpretation and the read-back's polarity are both total
/// functions of it. A create holds no subject because nothing exists yet; a
/// revise and a remove must hold one, because only a bound mapping can be
/// revised or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemOperation {
    Create,
    Revise {
        subject: RemoteListingId,
        transition: LifecycleTransition,
    },
    Remove {
        subject: RemoteListingId,
        state: ListingState,
    },
}

impl ItemOperation {
    /// The listing this operation addresses, which a create does not have.
    #[must_use]
    pub const fn subject(&self) -> Option<&RemoteListingId> {
        match self {
            Self::Create => None,
            Self::Revise { subject, .. } | Self::Remove { subject, .. } => Some(subject),
        }
    }
}

/// What the verification read has to show for the operation to be proved,
/// mirroring what the live runners learned. A create and a revise-to-draft
/// are proved by finding the listing, a publish by finding it live, and a
/// removal by not finding it.
///
/// One function with two callers deliberately: the driver polls until this
/// answers true, and the machine settles on whatever the poll hands back
/// when it stops. Two copies of it drifted apart once already — the poll
/// required `Live` for a publish while the machine asked only whether the
/// listing was absent, so a publish whose budget ran out while the listing
/// still read `Draft` settled Succeeded and bound the mapping `'draft'`.
#[must_use]
pub fn verification_settles(operation: &ItemOperation, observed: &ObservedListing) -> bool {
    let absent = matches!(observed.lifecycle, RemoteLifecycle::Absent);
    match operation {
        ItemOperation::Create => !absent,
        ItemOperation::Revise { transition, .. } => match transition.to {
            ListingState::Draft => !absent,
            ListingState::Live => matches!(observed.lifecycle, RemoteLifecycle::Live { .. }),
        },
        ItemOperation::Remove { .. } => absent,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    AwaitingPreflight,
    /// `None` where the operation asserts no form: a revise and a removal
    /// both skip the preflight, so there is no fingerprint for a later drift
    /// to be measured against and saying so is more honest than inventing one.
    PreflightAsserted {
        schema: Option<FormSchemaFingerprint>,
    },
    /// Carries the asserted fingerprint alongside the attempt, because a
    /// provably-never-sent submit returns to `PreflightAsserted`, and that
    /// state is defined by the fingerprint a later drift is measured against.
    IntentRecorded {
        attempt: WriteAttemptId,
        schema: Option<FormSchemaFingerprint>,
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
    /// Names no attempt, because the machine holds no randomness and cannot
    /// mint one. The driver opens the `write_attempt` row and reports the
    /// identifier back as `Input::IntentRecorded`, which is the direction the
    /// transition table already runs in and is what lets a never-sent submit
    /// ask for a second intent without inventing its identity.
    RecordIntent {
        intent_hash: ContentHash,
    },
    Submit {
        attempt: WriteAttemptId,
        key: IdempotencyKey,
        fields: FieldSet,
    },
    /// Neither lifecycle write carries an `IdempotencyKey`, because neither
    /// platform offers an idempotent edit or delete: the fenced attempt row
    /// is what stands in, which is the same argument `Submit` already records
    /// for Tes.
    Revise {
        attempt: WriteAttemptId,
        subject: RemoteListingId,
        fields: FieldSet,
        transition: LifecycleTransition,
    },
    Remove {
        attempt: WriteAttemptId,
        subject: RemoteListingId,
        state: ListingState,
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
    /// The attempt is optional because a preflight that fails on schema drift
    /// captures its diagnostics before any attempt exists.
    CaptureDiagnostics {
        attempt: Option<WriteAttemptId>,
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
    /// A transition would emit more effects than the budget permits. The
    /// machine is consumed and unchanged: the caller held a budget it had
    /// already spent and should have sent `Input::BudgetExhausted` instead.
    EffectBudgetExceeded,
}

/// Whether a halting-ambiguous row captures diagnostics before it halts. The
/// table varies this by row rather than by cause: the ambiguous-submit row
/// captures, the two reconcile rows do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capture {
    Diagnostics,
    Skip,
}

/// A park waits on the seller rather than on us, and a day is the window
/// `ItemState::ParkedLive` is dimensioned for.
const PARK_TTL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMachine {
    pub org: OrgId,
    pub inventory: InventoryId,
    /// Held because `Effect::RequeueBehindGate` names it, and the machine can
    /// only emit what it holds.
    pub connection: ConnectionId,
    /// Held because `Effect::AssertFormSchema` names it, on the entry row and
    /// again when a cleared challenge re-enters the flow.
    pub form: FormId,
    pub item: JobItemId,
    pub key: IdempotencyKey,
    /// The hash over `fields`, held because `Effect::RecordIntent` names it.
    pub intent_hash: ContentHash,
    /// The projected values a submit writes, held because `Effect::Submit`
    /// names them and a resubmit after a never-sent request writes the same set.
    pub fields: FieldSet,
    pub strategy: CreateStrategy,
    /// What this item does to the listing its mapping names. The entry row,
    /// the write effect, the read-back's polarity and the reconcile are all
    /// total functions of it.
    pub operation: ItemOperation,
    pub state: SyncState,
    pub budget: StepBudget,
}

impl SyncMachine {
    /// The entry row: a machine in `AwaitingPreflight` that has already asked
    /// for the form schema to be asserted. The entry effect is charged against
    /// the budget like any other, so a machine started with no allowance is
    /// refused here rather than one transition later.
    #[expect(
        clippy::too_many_arguments,
        reason = "these are the machine's construction data; a parameter struct \
                  would be a second name for the machine itself"
    )]
    pub fn initial(
        org: OrgId,
        inventory: InventoryId,
        connection: ConnectionId,
        form: FormId,
        item: JobItemId,
        key: IdempotencyKey,
        intent_hash: ContentHash,
        fields: FieldSet,
        strategy: CreateStrategy,
        operation: ItemOperation,
        budget: StepBudget,
    ) -> Result<Transition, MachineError> {
        let machine = Self {
            org,
            inventory,
            connection,
            form,
            item,
            key,
            intent_hash,
            fields,
            strategy,
            operation,
            state: SyncState::AwaitingPreflight,
            budget,
        };
        let (state, effects) = machine.entry_row(form);
        machine.advance(state, effects)
    }

    /// Only a create asserts a form schema. Tes's assertion is write-bearing —
    /// it creates a draft, writes it, reads it and deletes it on the seller's
    /// real store — so asserting on every revise would multiply probe drafts
    /// by the size of a bulk revise, and a removal describes no form at all.
    /// The edit path's drift detection is the canary's, on its own cadence.
    fn entry_row(&self, form: FormId) -> (SyncState, Vec<Effect>) {
        match self.operation {
            ItemOperation::Create => (
                SyncState::AwaitingPreflight,
                vec![Effect::AssertFormSchema { form }],
            ),
            ItemOperation::Revise { .. } | ItemOperation::Remove { .. } => (
                SyncState::PreflightAsserted { schema: None },
                vec![Effect::RecordIntent {
                    intent_hash: self.intent_hash,
                }],
            ),
        }
    }

    /// Consumes the machine so a stale state cannot be stepped twice, and takes
    /// `now` as a parameter rather than reading a clock.
    ///
    /// Implements the table in `docs/design/sync-machine.md`. Every business
    /// outcome including ambiguity is a value in `Outcome` on the success
    /// channel; `Err` means the transition was impossible, never unsuccessful.
    pub fn step(self, input: Input, now: LogicalInstant) -> Result<Transition, MachineError> {
        if let Some(conflict) = self.attempt_conflict(&input) {
            return Err(conflict);
        }
        if matches!(input, Input::BudgetExhausted) {
            return self.exhaust_budget();
        }
        match self.state.clone() {
            SyncState::AwaitingPreflight => self.awaiting_preflight_rows(input),
            SyncState::PreflightAsserted { schema } => self.preflight_asserted_rows(&input, schema),
            SyncState::IntentRecorded { attempt, schema } => {
                self.intent_recorded_rows(input, attempt, schema, now)
            }
            SyncState::Submitted { .. } | SyncState::Terminal(_) => {
                Err(MachineError::InputNotApplicable)
            }
            SyncState::AwaitingReadBack { attempt, .. } => {
                self.awaiting_read_back_rows(input, attempt, now)
            }
            SyncState::Parked { challenge, .. } => self.parked_rows(&input, challenge),
        }
    }

    /// Only `Input::IntentRecorded` names an attempt. A state that owns none
    /// accepts and stores whatever it is given; a state that owns a different
    /// one refuses; a state re-offered the identifier it already holds is being
    /// replayed a consumed input, which is inapplicable rather than a mismatch.
    fn attempt_conflict(&self, input: &Input) -> Option<MachineError> {
        let Input::IntentRecorded(offered) = input else {
            return None;
        };
        let owned = match &self.state {
            SyncState::AwaitingPreflight
            | SyncState::PreflightAsserted { .. }
            | SyncState::Terminal(_) => None,
            SyncState::IntentRecorded { attempt, .. }
            | SyncState::Submitted { attempt, .. }
            | SyncState::AwaitingReadBack { attempt, .. } => Some(*attempt),
            SyncState::Parked { attempt, .. } => *attempt,
        };
        match owned {
            None => None,
            Some(held) if held == *offered => Some(MachineError::InputNotApplicable),
            Some(_) => Some(MachineError::AttemptMismatch),
        }
    }

    fn awaiting_preflight_rows(self, input: Input) -> Result<Transition, MachineError> {
        match input {
            Input::PreflightResult(Ok(schema)) => {
                let effects = vec![Effect::RecordIntent {
                    intent_hash: self.intent_hash,
                }];
                self.advance(
                    SyncState::PreflightAsserted {
                        schema: Some(schema),
                    },
                    effects,
                )
            }
            Input::PreflightResult(Err(drift)) => {
                let effects = vec![
                    Effect::CaptureDiagnostics {
                        attempt: None,
                        cause: CaptureCause::SchemaDrift,
                    },
                    Effect::Halt {
                        scope: self.halt_scope(),
                    },
                    Effect::Notify {
                        org: self.org,
                        event: SellerEvent::InventoryHalted,
                    },
                ];
                let outcome = Outcome::Rejected {
                    code: FailureCode::FormSchemaDrift,
                    detail: drift_detail(&drift),
                };
                self.advance(SyncState::Terminal(outcome), effects)
            }
            Input::IntentRecorded(_)
            | Input::SubmitResult(_)
            | Input::ReadBackResult(_)
            | Input::ReconcileResult(_)
            | Input::ChallengeCleared
            | Input::ParkExpired
            | Input::BudgetExhausted => Err(MachineError::InputNotApplicable),
        }
    }

    fn preflight_asserted_rows(
        self,
        input: &Input,
        schema: Option<FormSchemaFingerprint>,
    ) -> Result<Transition, MachineError> {
        match *input {
            Input::IntentRecorded(attempt) => {
                let effects = vec![self.write_effect(attempt)];
                self.advance(SyncState::IntentRecorded { attempt, schema }, effects)
            }
            Input::PreflightResult(_)
            | Input::SubmitResult(_)
            | Input::ReadBackResult(_)
            | Input::ReconcileResult(_)
            | Input::ChallengeCleared
            | Input::ParkExpired
            | Input::BudgetExhausted => Err(MachineError::InputNotApplicable),
        }
    }

    /// The one write this operation means, named by the attempt that
    /// authorises it. Every arm carries the same fencing token, which is what
    /// makes the write-safety properties quantify over all three.
    fn write_effect(&self, attempt: WriteAttemptId) -> Effect {
        match &self.operation {
            ItemOperation::Create => Effect::Submit {
                attempt,
                key: self.key,
                fields: self.fields.clone(),
            },
            ItemOperation::Revise {
                subject,
                transition,
            } => Effect::Revise {
                attempt,
                subject: subject.clone(),
                fields: self.fields.clone(),
                transition: *transition,
            },
            ItemOperation::Remove { subject, state } => Effect::Remove {
                attempt,
                subject: subject.clone(),
                state: *state,
            },
        }
    }

    fn intent_recorded_rows(
        self,
        input: Input,
        attempt: WriteAttemptId,
        schema: Option<FormSchemaFingerprint>,
        now: LogicalInstant,
    ) -> Result<Transition, MachineError> {
        match input {
            Input::SubmitResult(Ok(evidence)) => self.await_read_back(attempt, evidence.landed),
            Input::SubmitResult(Err(AdapterError::Ambiguous(_))) => self.reconcile(attempt),
            Input::SubmitResult(Err(AdapterError::Rejected { code, detail })) => self.advance(
                SyncState::Terminal(Outcome::Rejected { code, detail }),
                vec![],
            ),
            Input::SubmitResult(Err(AdapterError::Challenge(challenge))) => {
                self.park(attempt, challenge, now)
            }
            Input::SubmitResult(Err(AdapterError::SessionExpired)) => {
                self.park(attempt, ChallengeKind::ReauthRequired, now)
            }
            Input::SubmitResult(Err(AdapterError::SchemaDrift(drift))) => {
                let effects = vec![
                    Effect::CaptureDiagnostics {
                        attempt: Some(attempt),
                        cause: CaptureCause::SchemaDrift,
                    },
                    Effect::Halt {
                        scope: self.halt_scope(),
                    },
                ];
                let outcome = Outcome::Rejected {
                    code: FailureCode::FormSchemaDrift,
                    detail: drift_detail(&drift),
                };
                self.advance(SyncState::Terminal(outcome), effects)
            }
            Input::SubmitResult(Err(AdapterError::RateLimited { .. })) => self.advance(
                SyncState::Terminal(Outcome::Skipped {
                    code: FailureCode::RateLimited,
                }),
                vec![],
            ),
            // A capability no capture settles: the adapter had nothing to
            // send, so the item is skipped rather than recorded as a
            // marketplace refusal of a submit that never happened.
            Input::SubmitResult(Err(AdapterError::Uncaptured { .. })) => self.advance(
                SyncState::Terminal(Outcome::Skipped {
                    code: FailureCode::Other,
                }),
                vec![],
            ),
            Input::SubmitResult(Err(AdapterError::NotSent(_))) => {
                let effects = vec![Effect::RecordIntent {
                    intent_hash: self.intent_hash,
                }];
                self.advance(SyncState::PreflightAsserted { schema }, effects)
            }
            Input::PreflightResult(_)
            | Input::IntentRecorded(_)
            | Input::ReadBackResult(_)
            | Input::ReconcileResult(_)
            | Input::ChallengeCleared
            | Input::ParkExpired
            | Input::BudgetExhausted => Err(MachineError::InputNotApplicable),
        }
    }

    fn awaiting_read_back_rows(
        self,
        input: Input,
        attempt: WriteAttemptId,
        now: LogicalInstant,
    ) -> Result<Transition, MachineError> {
        match input {
            Input::ReadBackResult(Ok(observed)) => self.observed_rows(observed, attempt, now),
            Input::ReadBackResult(Err(AdapterError::Ambiguous(cause))) => {
                let effects = vec![
                    Effect::CaptureDiagnostics {
                        attempt: Some(attempt),
                        cause: CaptureCause::Ambiguity,
                    },
                    Effect::Halt {
                        scope: self.halt_scope(),
                    },
                    Effect::Notify {
                        org: self.org,
                        event: SellerEvent::InventoryHalted,
                    },
                ];
                self.advance(SyncState::Terminal(ambiguous(attempt, cause)), effects)
            }
            Input::ReconcileResult(Ok(Some(id))) => {
                let outcome = settle(
                    as_attempt_id(attempt),
                    id.clone(),
                    Timestamp(now.0),
                    unnormalised_report(),
                );
                // The receipt the settle just minted is the only thing that can
                // justify the post-settle verification read, and `settle`
                // returns nothing but the two committed-class outcomes; the
                // remaining arms exist so the match is total.
                let effects = match &outcome {
                    Outcome::Committed { receipt, .. } | Outcome::Degraded { receipt, .. } => {
                        vec![Effect::ReadBack {
                            locator: ListingLocator::Durable(id),
                            reason: verify_after(receipt.clone()),
                        }]
                    }
                    Outcome::Rejected { .. }
                    | Outcome::Ambiguous { .. }
                    | Outcome::Blocked { .. }
                    | Outcome::Skipped { .. } => vec![],
                };
                self.advance(SyncState::Terminal(outcome), effects)
            }
            Input::ReconcileResult(Ok(None)) => {
                self.halt_ambiguous(attempt, AmbiguityCause::NoDurableIdentifier, Capture::Skip)
            }
            Input::ReconcileResult(Err(_)) => self.halt_ambiguous(
                attempt,
                AmbiguityCause::ReadBackIndeterminate,
                Capture::Skip,
            ),
            Input::PreflightResult(_)
            | Input::IntentRecorded(_)
            | Input::SubmitResult(_)
            | Input::ReadBackResult(Err(
                AdapterError::Rejected { .. }
                | AdapterError::Challenge(_)
                | AdapterError::SessionExpired
                | AdapterError::SchemaDrift(_)
                | AdapterError::RateLimited { .. }
                | AdapterError::NotSent(_)
                | AdapterError::Uncaptured { .. },
            ))
            | Input::ChallengeCleared
            | Input::ParkExpired
            | Input::BudgetExhausted => Err(MachineError::InputNotApplicable),
        }
    }

    /// What the verification read proves, deferred whole to
    /// [`verification_settles`] so the machine settles on exactly the
    /// predicate the driver polled for.
    ///
    /// A create, a revise or a publish the read never proved after the whole
    /// poll budget is the stall bias doing its job — the write may have
    /// landed and we cannot see it, which is `Ambiguous`, never a silent
    /// commit. That covers a publish still reading `Draft` as much as a
    /// create reading `Absent`: both are a write we could not confirm. A
    /// removal that still finds the listing is the one unproved read that is
    /// not an ambiguity, because it did not take — a refusal the ledger can
    /// act on rather than one that halts the tenant.
    fn observed_rows(
        self,
        observed: ObservedListing,
        attempt: WriteAttemptId,
        now: LogicalInstant,
    ) -> Result<Transition, MachineError> {
        if verification_settles(&self.operation, &observed) {
            let outcome = settle(
                as_attempt_id(attempt),
                observed.id,
                Timestamp(now.0),
                unnormalised_report(),
            );
            return self.advance(SyncState::Terminal(outcome), vec![]);
        }
        if matches!(self.operation, ItemOperation::Remove { .. }) {
            let outcome = Outcome::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail(
                    "the removal's verification read still found the listing".to_owned(),
                ),
            };
            return self.advance(SyncState::Terminal(outcome), vec![]);
        }
        self.halt_ambiguous(
            attempt,
            AmbiguityCause::ReadBackIndeterminate,
            Capture::Diagnostics,
        )
    }

    fn parked_rows(
        self,
        input: &Input,
        challenge: ChallengeKind,
    ) -> Result<Transition, MachineError> {
        match *input {
            // Branched exactly as the entry row is: a revise or a removal
            // parked on a captcha and then cleared must not acquire a form
            // assertion on its way back in, which on Tes is the write-bearing
            // probe draft the whole branch exists to avoid.
            Input::ChallengeCleared => {
                let (state, effects) = self.entry_row(self.form);
                self.advance(state, effects)
            }
            Input::ParkExpired => {
                let effects = vec![Effect::Notify {
                    org: self.org,
                    event: SellerEvent::ItemParked,
                }];
                self.advance(SyncState::Terminal(Outcome::Blocked { challenge }), effects)
            }
            Input::PreflightResult(_)
            | Input::IntentRecorded(_)
            | Input::SubmitResult(_)
            | Input::ReadBackResult(_)
            | Input::ReconcileResult(_)
            | Input::BudgetExhausted => Err(MachineError::InputNotApplicable),
        }
    }

    /// The budget report is driver-injected and exempt from the charge check:
    /// a report that the allowance ran out must never be blocked by the
    /// allowance running out.
    ///
    /// It settles as `Ambiguous` from the states where a submit was possible
    /// and as `Skipped` from the states where it was not. `FailureCode` carries
    /// no budget variant, so the skipped rows use `Other` until the closed
    /// vocabulary grows one.
    fn exhaust_budget(self) -> Result<Transition, MachineError> {
        let (attempt, outcome) = match &self.state {
            SyncState::AwaitingPreflight | SyncState::PreflightAsserted { .. } => (
                None,
                Outcome::Skipped {
                    code: FailureCode::Other,
                },
            ),
            SyncState::Parked { attempt, .. } => (
                *attempt,
                Outcome::Skipped {
                    code: FailureCode::Other,
                },
            ),
            SyncState::IntentRecorded { attempt, .. }
            | SyncState::Submitted { attempt, .. }
            | SyncState::AwaitingReadBack { attempt, .. } => (
                Some(*attempt),
                ambiguous(*attempt, AmbiguityCause::ProcessKilledByBackstop),
            ),
            SyncState::Terminal(_) => return Err(MachineError::InputNotApplicable),
        };
        let effects = vec![Effect::CaptureDiagnostics {
            attempt,
            cause: CaptureCause::Ambiguity,
        }];
        Ok(self.emit_unbilled(SyncState::Terminal(outcome), effects))
    }

    /// A submit that landed with no statable identifier and no marker to
    /// search: nothing can verify it, so it halts as the no-durable-identifier
    /// ambiguity — the governing axiom, a stalled queue over a duplicate storm.
    fn ambiguous_no_identifier(self, attempt: WriteAttemptId) -> Result<Transition, MachineError> {
        self.halt_ambiguous(
            attempt,
            AmbiguityCause::NoDurableIdentifier,
            Capture::Diagnostics,
        )
    }

    /// The pre-settle verification read. When the submit stated the durable
    /// identifier it landed on, the read addresses it directly under
    /// `VerifyAttempt`, justified by the open `write_attempt` row. When it did
    /// not, only a marker strategy can search for the landing; any other
    /// strategy has nothing to verify against, so the write that cannot be
    /// verified halts as ambiguous rather than guessing — the stall bias.
    fn await_read_back(
        self,
        attempt: WriteAttemptId,
        landed: Option<RemoteListingId>,
    ) -> Result<Transition, MachineError> {
        let locator = match (landed, &self.strategy) {
            (Some(id), _) => ListingLocator::Durable(id),
            (None, CreateStrategy::CorrelationMarker { .. }) => ListingLocator::Marker {
                marker: marker_for(attempt),
                inventory: self.inventory,
            },
            (None, _) => return self.ambiguous_no_identifier(attempt),
        };
        let effects = vec![Effect::ReadBack {
            locator: locator.clone(),
            reason: FetchReason::VerifyAttempt { attempt },
        }];
        self.advance(SyncState::AwaitingReadBack { attempt, locator }, effects)
    }

    /// Only a marker strategy can reconcile an ambiguous create in this
    /// milestone: reconciliation searches for something the driver embedded,
    /// and neither `DraftThenPublish` nor `HaltOnAmbiguity` embeds anything.
    /// Where there is nothing to search for, the strategy's meaning is to stop,
    /// which is what `HaltOnAmbiguity` says in its name.
    fn reconcile(self, attempt: WriteAttemptId) -> Result<Transition, MachineError> {
        // A revise and a removal were *given* a durable identifier, which is
        // strictly better than a marker: there is nothing to search for, so
        // the read-back's polarity decides and the strategy has no bearing.
        // Halting the tenant's inventory here would be the halt-on-lag
        // failure the verification poll exists to remove, arriving through
        // another door.
        if let Some(subject) = self.operation.subject() {
            let locator = ListingLocator::Durable(subject.clone());
            let effects = vec![
                Effect::ReadBack {
                    locator: locator.clone(),
                    reason: FetchReason::VerifyAttempt { attempt },
                },
                Effect::CaptureDiagnostics {
                    attempt: Some(attempt),
                    cause: CaptureCause::Ambiguity,
                },
            ];
            return self.advance(SyncState::AwaitingReadBack { attempt, locator }, effects);
        }
        let marker = match self.strategy {
            CreateStrategy::CorrelationMarker { .. } => marker_for(attempt),
            CreateStrategy::DraftThenPublish { .. } | CreateStrategy::HaltOnAmbiguity => {
                return self.halt_ambiguous(
                    attempt,
                    AmbiguityCause::NoDurableIdentifier,
                    Capture::Diagnostics,
                );
            }
        };
        let locator = ListingLocator::Marker {
            marker,
            inventory: self.inventory,
        };
        let effects = vec![
            Effect::Reconcile {
                attempt,
                locator: locator.clone(),
            },
            Effect::CaptureDiagnostics {
                attempt: Some(attempt),
                cause: CaptureCause::Ambiguity,
            },
        ];
        self.advance(SyncState::AwaitingReadBack { attempt, locator }, effects)
    }

    fn halt_ambiguous(
        self,
        attempt: WriteAttemptId,
        cause: AmbiguityCause,
        capture: Capture,
    ) -> Result<Transition, MachineError> {
        let mut effects = Vec::with_capacity(3);
        if matches!(capture, Capture::Diagnostics) {
            effects.push(Effect::CaptureDiagnostics {
                attempt: Some(attempt),
                cause: CaptureCause::Ambiguity,
            });
        }
        effects.push(Effect::Halt {
            scope: self.halt_scope(),
        });
        effects.push(Effect::Notify {
            org: self.org,
            event: SellerEvent::InventoryHalted,
        });
        self.advance(SyncState::Terminal(ambiguous(attempt, cause)), effects)
    }

    fn park(
        self,
        attempt: WriteAttemptId,
        challenge: ChallengeKind,
        now: LogicalInstant,
    ) -> Result<Transition, MachineError> {
        let reauth = matches!(challenge, ChallengeKind::ReauthRequired);
        let effects = vec![
            Effect::ParkItem {
                item: self.item,
                challenge,
                expires: LogicalInstant(now.0.saturating_add(PARK_TTL_MS)),
            },
            Effect::RequeueBehindGate {
                connection: self.connection,
                cause: if reauth {
                    BlockCause::Reauth
                } else {
                    BlockCause::Challenge
                },
            },
            Effect::Notify {
                org: self.org,
                event: if reauth {
                    SellerEvent::ReauthRequired
                } else {
                    SellerEvent::ItemParked
                },
            },
        ];
        self.advance(
            SyncState::Parked {
                attempt: Some(attempt),
                challenge,
            },
            effects,
        )
    }

    fn halt_scope(&self) -> HaltScope {
        HaltScope::OrgInventory {
            org: self.org,
            inventory: self.inventory,
        }
    }

    /// Every emitted effect is one interpreter action, so a transition that
    /// would emit more than the allowance permits is refused outright rather
    /// than emitting a prefix.
    fn advance(
        mut self,
        state: SyncState,
        effects: Vec<Effect>,
    ) -> Result<Transition, MachineError> {
        let cost = u32::try_from(effects.len()).unwrap_or(u32::MAX);
        if cost > self.budget.actions_remaining {
            return Err(MachineError::EffectBudgetExceeded);
        }
        self.budget = StepBudget {
            actions_remaining: self.budget.actions_remaining.saturating_sub(cost),
        };
        self.state = state;
        Ok(Transition {
            next: self,
            effects: EffectList(effects),
        })
    }

    fn emit_unbilled(mut self, state: SyncState, effects: Vec<Effect>) -> Transition {
        let cost = u32::try_from(effects.len()).unwrap_or(u32::MAX);
        self.budget = StepBudget {
            actions_remaining: self.budget.actions_remaining.saturating_sub(cost),
        };
        self.state = state;
        Transition {
            next: self,
            effects: EffectList(effects),
        }
    }
}

/// The two newtypes are one identity seen from two layers: the machine holds
/// the fencing token as `WriteAttemptId` and the ledger and the receipt name
/// the same uuid `AttemptId`.
fn as_attempt_id(attempt: WriteAttemptId) -> AttemptId {
    AttemptId(attempt.0)
}

/// The attempt is the key the captured diagnostics are stored under, so the
/// attempt rendered deterministically is the evidence reference.
fn ambiguous(attempt: WriteAttemptId, cause: AmbiguityCause) -> Outcome {
    Outcome::Ambiguous {
        attempt: as_attempt_id(attempt),
        cause,
        evidence: EvidenceRef(format!("write-attempt:{}", hex(attempt.0))),
    }
}

/// The driver embeds this same rendering at submit time, so the machine can
/// name a marker it never watched being written.
fn marker_for(attempt: WriteAttemptId) -> CorrelationMarker {
    CorrelationMarker(format!("tam-{}", hex(attempt.0)))
}

fn hex(uuid: Uuid) -> String {
    const DIGITS: [u8; 16] = *b"0123456789abcdef";
    let mut rendered = String::with_capacity(32);
    for byte in uuid.0 {
        rendered.push(char::from(DIGITS[usize::from(byte >> 4)]));
        rendered.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    rendered
}

/// The M1g seam. There is no normaliser yet, so every settled read-back is
/// reported as clean and `settle` classifies it `Committed`. M1g replaces this
/// with the real per-field comparison, which is the only thing that can produce
/// a `Degraded`, and raises `normaliser_version` off zero.
fn unnormalised_report() -> FieldDiffReport {
    FieldDiffReport {
        normaliser_version: 0,
        mismatches: vec![],
    }
}

fn drift_detail(drift: &SchemaDrift) -> FailureDetail {
    FailureDetail(format!(
        "form schema drift: added [{}], removed [{}]",
        drift.added.join(", "),
        drift.removed.join(", ")
    ))
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

#[cfg(test)]
mod machine_tests {
    use super::*;
    use proptest::prelude::*;
    use tam_marketplace::{ConnectFailure, MarkerField, MarkerLifetime, RemoteLifecycleKind};
    use tam_types::FieldKey;

    fn org() -> OrgId {
        OrgId(Uuid([1; 16]))
    }

    fn connection() -> ConnectionId {
        ConnectionId(Uuid([2; 16]))
    }

    fn form() -> FormId {
        FormId(Uuid([3; 16]))
    }

    fn item() -> JobItemId {
        JobItemId(Uuid([4; 16]))
    }

    fn key() -> IdempotencyKey {
        IdempotencyKey(Uuid([5; 16]))
    }

    fn intent_hash() -> ContentHash {
        ContentHash([6; 32])
    }

    fn attempt() -> WriteAttemptId {
        WriteAttemptId(Uuid([7; 16]))
    }

    fn other_attempt() -> WriteAttemptId {
        WriteAttemptId(Uuid([8; 16]))
    }

    fn fields() -> FieldSet {
        FieldSet {
            entries: vec![(FieldKey::Title, "a resource".to_owned())],
            files: vec![],
        }
    }

    fn schema() -> FormSchemaFingerprint {
        FormSchemaFingerprint(ContentHash([9; 32]))
    }

    fn drift() -> SchemaDrift {
        SchemaDrift {
            form: form(),
            expected: schema(),
            observed: FormSchemaFingerprint(ContentHash([10; 32])),
            added: vec!["consent".to_owned()],
            removed: vec!["subtitle".to_owned()],
        }
    }

    fn evidence() -> SubmitEvidence {
        SubmitEvidence {
            http_status: Some(200),
            response_body_digest: None,
            landed_on_route: None,
            landed: None,
            observed_lag: false,
        }
    }

    fn evidence_landed() -> SubmitEvidence {
        SubmitEvidence {
            http_status: Some(200),
            response_body_digest: None,
            landed_on_route: None,
            landed: Some(listing()),
            observed_lag: false,
        }
    }

    fn listing() -> RemoteListingId {
        RemoteListingId::Tes {
            url: "https://www.tes.com/teaching-resource/x-1".to_owned(),
        }
    }

    fn observed() -> ObservedListing {
        ObservedListing {
            id: listing(),
            fields: vec![],
            lifecycle: RemoteLifecycle::Live {
                since: Timestamp(1),
            },
        }
    }

    fn observed_absent() -> ObservedListing {
        ObservedListing {
            id: listing(),
            fields: vec![],
            lifecycle: RemoteLifecycle::Absent,
        }
    }

    fn now() -> LogicalInstant {
        LogicalInstant(1_000)
    }

    fn marker_strategy() -> CreateStrategy {
        CreateStrategy::CorrelationMarker {
            field: MarkerField::DescriptionTail,
            ttl: MarkerLifetime { seconds: 3_600 },
        }
    }

    fn draft_strategy() -> CreateStrategy {
        CreateStrategy::DraftThenPublish {
            draft_state: RemoteLifecycleKind::Draft,
        }
    }

    fn machine(state: SyncState, strategy: CreateStrategy, actions_remaining: u32) -> SyncMachine {
        machine_for(ItemOperation::Create, state, strategy, actions_remaining)
    }

    fn machine_for(
        operation: ItemOperation,
        state: SyncState,
        strategy: CreateStrategy,
        actions_remaining: u32,
    ) -> SyncMachine {
        SyncMachine {
            org: org(),
            inventory: InventoryId::TesGb,
            connection: connection(),
            form: form(),
            item: item(),
            key: key(),
            intent_hash: intent_hash(),
            fields: fields(),
            strategy,
            operation,
            state,
            budget: StepBudget { actions_remaining },
        }
    }

    fn removal() -> ItemOperation {
        ItemOperation::Remove {
            subject: listing(),
            state: ListingState::Live,
        }
    }

    fn publication() -> ItemOperation {
        ItemOperation::Revise {
            subject: listing(),
            transition: LifecycleTransition {
                from: ListingState::Draft,
                to: ListingState::Live,
            },
        }
    }

    fn marker_locator() -> ListingLocator {
        ListingLocator::Marker {
            marker: marker_for(attempt()),
            inventory: InventoryId::TesGb,
        }
    }

    fn halt() -> Effect {
        Effect::Halt {
            scope: HaltScope::OrgInventory {
                org: org(),
                inventory: InventoryId::TesGb,
            },
        }
    }

    fn notify(event: SellerEvent) -> Effect {
        Effect::Notify { org: org(), event }
    }

    /// The outcome the machine is expected to mint for a settled read-back,
    /// built by calling the same public constructor the machine calls.
    fn settled() -> Outcome {
        settle(
            as_attempt_id(attempt()),
            listing(),
            Timestamp(now().0),
            unnormalised_report(),
        )
    }

    fn receipt_of(outcome: &Outcome) -> tam_marketplace::WriteReceipt {
        match outcome {
            Outcome::Committed { receipt, .. } | Outcome::Degraded { receipt, .. } => {
                receipt.clone()
            }
            Outcome::Rejected { .. }
            | Outcome::Ambiguous { .. }
            | Outcome::Blocked { .. }
            | Outcome::Skipped { .. } => {
                panic!("settle returns only the two committed-class outcomes")
            }
        }
    }

    fn entry() -> Transition {
        SyncMachine::initial(
            org(),
            InventoryId::TesGb,
            connection(),
            form(),
            item(),
            key(),
            intent_hash(),
            fields(),
            marker_strategy(),
            ItemOperation::Create,
            StepBudget {
                actions_remaining: 10,
            },
        )
        .expect("a ten-action budget affords the single entry effect")
    }

    #[test]
    fn row_entry_asserts_the_form_schema() {
        let transition = entry();
        assert_eq!(
            transition.next.state,
            SyncState::AwaitingPreflight,
            "the entry row is the entry state and does not leave it"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::AssertFormSchema { form: form() }]),
            "the entry row asserts the form schema and asks for nothing else"
        );
        assert_eq!(
            transition.next.budget,
            StepBudget {
                actions_remaining: 9
            },
            "the entry effect is charged against the budget like any other"
        );
    }

    #[test]
    fn row_entry_is_refused_when_the_budget_affords_nothing() {
        let refused = SyncMachine::initial(
            org(),
            InventoryId::TesGb,
            connection(),
            form(),
            item(),
            key(),
            intent_hash(),
            fields(),
            marker_strategy(),
            ItemOperation::Create,
            StepBudget {
                actions_remaining: 0,
            },
        );
        assert_eq!(
            refused,
            Err(MachineError::EffectBudgetExceeded),
            "a machine with no allowance cannot even ask for the form schema"
        );
    }

    #[test]
    fn row_awaiting_preflight_ok_records_intent() {
        let transition = machine(SyncState::AwaitingPreflight, marker_strategy(), 10)
            .step(Input::PreflightResult(Ok(schema())), now())
            .expect("a preflight result applies in AwaitingPreflight");
        assert_eq!(
            transition.next.state,
            SyncState::PreflightAsserted {
                schema: Some(schema()),
            },
            "the asserted fingerprint is what a later drift is measured against"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::RecordIntent {
                intent_hash: intent_hash()
            }]),
            "an asserted schema asks the driver to open the write attempt"
        );
    }

    #[test]
    fn row_awaiting_preflight_drift_rejects_halts_and_notifies() {
        let transition = machine(SyncState::AwaitingPreflight, marker_strategy(), 10)
            .step(Input::PreflightResult(Err(drift())), now())
            .expect("a preflight result applies in AwaitingPreflight");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(Outcome::Rejected {
                code: FailureCode::FormSchemaDrift,
                detail: drift_detail(&drift()),
            }),
            "drift before any attempt is a rejection, not an ambiguity"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::CaptureDiagnostics {
                    attempt: None,
                    cause: CaptureCause::SchemaDrift,
                },
                halt(),
                notify(SellerEvent::InventoryHalted),
            ]),
            "the drift row captures, halts and notifies, in that order, with no attempt to name"
        );
    }

    #[test]
    fn row_preflight_asserted_intent_recorded_submits() {
        let transition = machine(
            SyncState::PreflightAsserted {
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::IntentRecorded(attempt()), now())
        .expect("a state owning no attempt accepts the one it is given");
        assert_eq!(
            transition.next.state,
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            "the recorded intent carries the fingerprint a never-sent submit returns to"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::Submit {
                attempt: attempt(),
                key: key(),
                fields: fields(),
            }]),
            "a committed intent is followed by exactly one submit"
        );
    }

    #[test]
    fn row_intent_recorded_submit_ok_awaits_read_back() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::SubmitResult(Ok(evidence())), now())
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            "with no stated identifier under a marker strategy, the marker addresses the read-back"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::ReadBack {
                locator: marker_locator(),
                reason: FetchReason::VerifyAttempt { attempt: attempt() },
            }]),
            "the pre-settle read-back is justified by the open write-attempt row, not a receipt"
        );
    }

    #[test]
    fn row_intent_recorded_submit_ok_with_a_durable_id_reads_it_directly() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::SubmitResult(Ok(evidence_landed())), now())
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: ListingLocator::Durable(listing()),
            },
            "a submit that stated its landing is verified by that durable identifier"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::ReadBack {
                locator: ListingLocator::Durable(listing()),
                reason: FetchReason::VerifyAttempt { attempt: attempt() },
            }]),
            "the durable read-back rides the write-attempt capability"
        );
    }

    #[test]
    fn a_landless_submit_under_a_non_marker_strategy_halts_ambiguous() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            CreateStrategy::HaltOnAmbiguity,
            10,
        )
        .step(Input::SubmitResult(Ok(evidence())), now())
        .expect("a submit result applies in IntentRecorded");
        assert!(
            matches!(
                transition.next.state,
                SyncState::Terminal(Outcome::Ambiguous {
                    cause: AmbiguityCause::NoDurableIdentifier,
                    ..
                })
            ),
            "a submit that cannot be verified halts as ambiguous rather than guessing"
        );
    }

    #[test]
    fn row_intent_recorded_submit_ambiguous_reconciles_under_a_marker() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            "an ambiguous submit is reconciled, never retried"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::Reconcile {
                    attempt: attempt(),
                    locator: marker_locator(),
                },
                Effect::CaptureDiagnostics {
                    attempt: Some(attempt()),
                    cause: CaptureCause::Ambiguity,
                },
            ]),
            "the reconcile precedes the capture, in the order the table lists them"
        );
    }

    #[test]
    fn row_intent_recorded_submit_ambiguous_halts_without_a_marker() {
        for strategy in [CreateStrategy::HaltOnAmbiguity, draft_strategy()] {
            let transition = machine(
                SyncState::IntentRecorded {
                    attempt: attempt(),
                    schema: Some(schema()),
                },
                strategy,
                10,
            )
            .step(
                Input::SubmitResult(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))),
                now(),
            )
            .expect("a submit result applies in IntentRecorded");
            assert_eq!(
                transition.next.state,
                SyncState::Terminal(ambiguous(attempt(), AmbiguityCause::NoDurableIdentifier)),
                "a strategy that embedded nothing has nothing to reconcile against"
            );
            assert_eq!(
                transition.effects,
                EffectList(vec![
                    Effect::CaptureDiagnostics {
                        attempt: Some(attempt()),
                        cause: CaptureCause::Ambiguity,
                    },
                    halt(),
                    notify(SellerEvent::InventoryHalted),
                ]),
                "stopping is what HaltOnAmbiguity means, and a draft strategy has no marker either"
            );
        }
    }

    #[test]
    fn row_intent_recorded_submit_rejected_passes_the_code_through() {
        let detail = FailureDetail("the upload was refused".to_owned());
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                detail: detail.clone(),
            })),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(Outcome::Rejected {
                code: FailureCode::UploadRejected,
                detail,
            }),
            "the adapter's code and detail travel unchanged into the outcome"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![]),
            "a clean rejection asks for nothing"
        );
    }

    #[test]
    fn row_intent_recorded_submit_challenge_parks() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::Challenge(ChallengeKind::Captcha))),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::Parked {
                attempt: Some(attempt()),
                challenge: ChallengeKind::Captcha,
            },
            "a park holds the attempt it was interrupted mid-flight for"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::ParkItem {
                    item: item(),
                    challenge: ChallengeKind::Captcha,
                    expires: LogicalInstant(now().0 + PARK_TTL_MS),
                },
                Effect::RequeueBehindGate {
                    connection: connection(),
                    cause: BlockCause::Challenge,
                },
                notify(SellerEvent::ItemParked),
            ]),
            "the park, the gate and the notification, in the order the table lists them"
        );
    }

    #[test]
    fn row_intent_recorded_session_expired_parks_on_reauth() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::SessionExpired)),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::Parked {
                attempt: Some(attempt()),
                challenge: ChallengeKind::ReauthRequired,
            },
            "an expired session is the reauth challenge under another name"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::ParkItem {
                    item: item(),
                    challenge: ChallengeKind::ReauthRequired,
                    expires: LogicalInstant(now().0 + PARK_TTL_MS),
                },
                Effect::RequeueBehindGate {
                    connection: connection(),
                    cause: BlockCause::Reauth,
                },
                notify(SellerEvent::ReauthRequired),
            ]),
            "a reauth park gates on reauth and tells the seller so"
        );
    }

    #[test]
    fn row_intent_recorded_schema_drift_rejects_and_halts_without_notifying() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::SchemaDrift(Box::new(drift())))),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(Outcome::Rejected {
                code: FailureCode::FormSchemaDrift,
                detail: drift_detail(&drift()),
            }),
            "drift found at submit is the same rejection as drift found at preflight"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::CaptureDiagnostics {
                    attempt: Some(attempt()),
                    cause: CaptureCause::SchemaDrift,
                },
                halt(),
            ]),
            "this row captures and halts, and the table does not notify on it"
        );
    }

    #[test]
    fn row_intent_recorded_rate_limited_skips() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::RateLimited { retry_after: None })),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(Outcome::Skipped {
                code: FailureCode::RateLimited,
            }),
            "a rate-limited submit is skipped rather than failed"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![]),
            "a skip asks for nothing"
        );
    }

    #[test]
    fn row_intent_recorded_not_sent_returns_for_a_fresh_intent() {
        let transition = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::SubmitResult(Err(AdapterError::NotSent(ConnectFailure::TcpRefused))),
            now(),
        )
        .expect("a submit result applies in IntentRecorded");
        assert_eq!(
            transition.next.state,
            SyncState::PreflightAsserted {
                schema: Some(schema()),
            },
            "the only class that provably never left is the only one that returns pre-submit"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::RecordIntent {
                intent_hash: intent_hash()
            }]),
            "the machine asks for another intent and lets the driver name it"
        );
    }

    #[test]
    fn row_awaiting_read_back_ok_settles() {
        let transition = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(Input::ReadBackResult(Ok(observed())), now())
        .expect("a read-back result applies in AwaitingReadBack");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(settled()),
            "the observed identifier is what the receipt is minted over"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![]),
            "a settled read-back asks for nothing"
        );
    }

    #[test]
    fn an_absent_observation_never_settles_committed() {
        let transition = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(Input::ReadBackResult(Ok(observed_absent())), now())
        .expect("a read-back result applies in AwaitingReadBack");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(ambiguous(attempt(), AmbiguityCause::ReadBackIndeterminate)),
            "a listing the verification read cannot find is ambiguous, never committed"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::CaptureDiagnostics {
                    attempt: Some(attempt()),
                    cause: CaptureCause::Ambiguity,
                },
                halt(),
                notify(SellerEvent::InventoryHalted),
            ]),
            "an unverifiable write halts the tenant's inventory rather than proceeding"
        );
    }

    /// The single likeliest error in this change is a polarity copied from
    /// the create path, under which a removal would settle Committed on
    /// finding the listing it failed to remove.
    #[test]
    fn a_removal_settles_on_absence_and_refuses_on_presence() {
        let gone = machine_for(
            removal(),
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: ListingLocator::Durable(listing()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::ReadBackResult(Ok(observed_absent())), now())
        .expect("a read-back result applies in AwaitingReadBack");
        assert_eq!(
            gone.next.state,
            SyncState::Terminal(settled()),
            "a removal is proved by the listing not being there"
        );
        assert_eq!(
            gone.effects,
            EffectList(vec![]),
            "a settled removal asks for nothing"
        );

        let refused = machine_for(
            removal(),
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: ListingLocator::Durable(listing()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::ReadBackResult(Ok(observed())), now())
        .expect("a read-back result applies in AwaitingReadBack");
        assert_eq!(
            refused.next.state,
            SyncState::Terminal(Outcome::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: FailureDetail(
                    "the removal's verification read still found the listing".to_owned()
                ),
            }),
            "a removal that still finds its listing did not take, and says so per item"
        );
        assert_eq!(
            refused.effects,
            EffectList(vec![]),
            "and it does not halt the tenant's inventory over one item's refusal"
        );
    }

    /// On Tes the form assertion creates, writes, reads and deletes a probe
    /// draft on the seller's real store. A removal that ran it would do that
    /// before every deletion, and a bulk removal would multiply it.
    #[test]
    fn a_removal_asserts_no_form_schema() {
        let entry = SyncMachine::initial(
            org(),
            InventoryId::TesGb,
            connection(),
            form(),
            item(),
            key(),
            intent_hash(),
            fields(),
            marker_strategy(),
            removal(),
            StepBudget {
                actions_remaining: 10,
            },
        )
        .expect("a ten-action budget affords the single entry effect");
        assert_eq!(
            entry.next.state,
            SyncState::PreflightAsserted { schema: None },
            "a removal describes no form, so there is no fingerprint to assert"
        );
        assert_eq!(
            entry.effects,
            EffectList(vec![Effect::RecordIntent {
                intent_hash: intent_hash(),
            }]),
            "the entry row records the intent and asserts nothing"
        );

        // The parked path is the one a test that only steps `initial` would
        // pass green over: a removal parked on a captcha for a long-running
        // tenant re-enters through here.
        let cleared = machine_for(
            removal(),
            SyncState::Parked {
                attempt: Some(attempt()),
                challenge: ChallengeKind::Captcha,
            },
            marker_strategy(),
            10,
        )
        .step(Input::ChallengeCleared, now())
        .expect("a cleared challenge applies in Parked");
        assert_eq!(
            cleared.next.state,
            SyncState::PreflightAsserted { schema: None },
            "a cleared removal re-enters where it entered, not at the preflight"
        );
        assert!(
            !cleared
                .effects
                .0
                .iter()
                .any(|effect| matches!(effect, Effect::AssertFormSchema { .. })),
            "and it acquires no form assertion on the way back in"
        );
    }

    /// The transition is carried, never reconstructed: no column records which
    /// side of the draft line a listing sits on, so a machine that rebuilt it
    /// from the mapping would be reading a lie.
    #[test]
    fn a_revise_to_live_emits_the_transition_it_was_seeded_with() {
        let transition = machine_for(
            publication(),
            SyncState::PreflightAsserted { schema: None },
            marker_strategy(),
            10,
        )
        .step(Input::IntentRecorded(attempt()), now())
        .expect("an intent applies in PreflightAsserted");
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::Revise {
                attempt: attempt(),
                subject: listing(),
                fields: fields(),
                transition: LifecycleTransition {
                    from: ListingState::Draft,
                    to: ListingState::Live,
                },
            }]),
            "the effect carries the transition the item was seeded with, whole"
        );
        assert_eq!(
            transition.next.state,
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: None,
            },
            "a revise records its intent with no fingerprint, because it asserted none"
        );
    }

    /// An ambiguous revise or removal holds its own durable identifier, which
    /// is strictly better than a marker. Halting the tenant's inventory here
    /// would be the halt-on-lag failure arriving through another door.
    #[test]
    fn an_ambiguous_lifecycle_write_re_reads_its_subject_rather_than_halting() {
        for (label, operation) in [("a publish", publication()), ("a removal", removal())] {
            let transition = machine_for(
                operation,
                SyncState::IntentRecorded {
                    attempt: attempt(),
                    schema: None,
                },
                draft_strategy(),
                10,
            )
            .step(
                Input::SubmitResult(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))),
                now(),
            )
            .expect("a submit result applies in IntentRecorded");
            assert_eq!(
                transition.next.state,
                SyncState::AwaitingReadBack {
                    attempt: attempt(),
                    locator: ListingLocator::Durable(listing()),
                },
                "{label}: the subject it was given is what the read addresses"
            );
            assert_eq!(
                transition.effects,
                EffectList(vec![
                    Effect::ReadBack {
                        locator: ListingLocator::Durable(listing()),
                        reason: FetchReason::VerifyAttempt { attempt: attempt() },
                    },
                    Effect::CaptureDiagnostics {
                        attempt: Some(attempt()),
                        cause: CaptureCause::Ambiguity,
                    },
                ]),
                "{label}: it re-reads under a strategy that would halt a create"
            );
        }
    }

    #[test]
    fn row_awaiting_read_back_ambiguous_halts() {
        let transition = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::ReadBackResult(Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))),
            now(),
        )
        .expect("a read-back result applies in AwaitingReadBack");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(ambiguous(attempt(), AmbiguityCause::ReadBackIndeterminate)),
            "the adapter's own cause travels into the outcome"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![
                Effect::CaptureDiagnostics {
                    attempt: Some(attempt()),
                    cause: CaptureCause::Ambiguity,
                },
                halt(),
                notify(SellerEvent::InventoryHalted),
            ]),
            "capture, halt, notify, in the order the table lists them"
        );
    }

    #[test]
    fn row_awaiting_read_back_reconciled_settles_and_verifies() {
        let transition = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(Input::ReconcileResult(Ok(Some(listing()))), now())
        .expect("a reconcile result applies in AwaitingReadBack");
        let expected = settled();
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(expected.clone()),
            "a reconciled identifier settles the write"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::ReadBack {
                locator: ListingLocator::Durable(listing()),
                reason: verify_after(receipt_of(&expected)),
            }]),
            "the post-settle verification is justified by the receipt just minted"
        );
    }

    #[test]
    fn row_awaiting_read_back_reconciled_to_nothing_halts() {
        let transition = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(Input::ReconcileResult(Ok(None)), now())
        .expect("a reconcile result applies in AwaitingReadBack");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(ambiguous(attempt(), AmbiguityCause::NoDurableIdentifier)),
            "reconciliation that finds nothing has not proved the write did not land"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![halt(), notify(SellerEvent::InventoryHalted)]),
            "this row halts and notifies, and the table does not capture on it"
        );
    }

    #[test]
    fn row_awaiting_read_back_reconcile_failed_halts() {
        let transition = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::ReconcileResult(Err(AdapterError::RateLimited { retry_after: None })),
            now(),
        )
        .expect("a reconcile result applies in AwaitingReadBack");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(ambiguous(attempt(), AmbiguityCause::ReadBackIndeterminate)),
            "a reconciliation that could not run leaves the write indeterminate"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![halt(), notify(SellerEvent::InventoryHalted)]),
            "this row halts and notifies, and the table does not capture on it"
        );
    }

    #[test]
    fn row_parked_cleared_reasserts_the_form_schema() {
        let transition = machine(
            SyncState::Parked {
                attempt: Some(attempt()),
                challenge: ChallengeKind::Captcha,
            },
            marker_strategy(),
            10,
        )
        .step(Input::ChallengeCleared, now())
        .expect("a cleared challenge applies in Parked");
        assert_eq!(
            transition.next.state,
            SyncState::AwaitingPreflight,
            "the markup may have changed while the item was parked"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![Effect::AssertFormSchema { form: form() }]),
            "re-entry re-asserts the schema rather than resuming mid-flow"
        );
    }

    #[test]
    fn row_parked_expired_blocks() {
        let transition = machine(
            SyncState::Parked {
                attempt: Some(attempt()),
                challenge: ChallengeKind::Captcha,
            },
            marker_strategy(),
            10,
        )
        .step(Input::ParkExpired, now())
        .expect("an expired park applies in Parked");
        assert_eq!(
            transition.next.state,
            SyncState::Terminal(Outcome::Blocked {
                challenge: ChallengeKind::Captcha,
            }),
            "an unanswered park is the only way a park becomes terminal"
        );
        assert_eq!(
            transition.effects,
            EffectList(vec![notify(SellerEvent::ItemParked)]),
            "the closed event vocabulary's nearest true statement about an expired park"
        );
    }

    #[test]
    fn row_budget_exhausted_settles_every_non_terminal_state() {
        let expectations = vec![
            (
                SyncState::AwaitingPreflight,
                None,
                Outcome::Skipped {
                    code: FailureCode::Other,
                },
            ),
            (
                SyncState::PreflightAsserted {
                    schema: Some(schema()),
                },
                None,
                Outcome::Skipped {
                    code: FailureCode::Other,
                },
            ),
            (
                SyncState::Parked {
                    attempt: Some(attempt()),
                    challenge: ChallengeKind::Captcha,
                },
                Some(attempt()),
                Outcome::Skipped {
                    code: FailureCode::Other,
                },
            ),
            (
                SyncState::IntentRecorded {
                    attempt: attempt(),
                    schema: Some(schema()),
                },
                Some(attempt()),
                ambiguous(attempt(), AmbiguityCause::ProcessKilledByBackstop),
            ),
            (
                SyncState::Submitted {
                    attempt: attempt(),
                    evidence: evidence(),
                },
                Some(attempt()),
                ambiguous(attempt(), AmbiguityCause::ProcessKilledByBackstop),
            ),
            (
                SyncState::AwaitingReadBack {
                    attempt: attempt(),
                    locator: marker_locator(),
                },
                Some(attempt()),
                ambiguous(attempt(), AmbiguityCause::ProcessKilledByBackstop),
            ),
        ];
        for (state, captured, outcome) in expectations {
            let transition = machine(state.clone(), marker_strategy(), 0)
                .step(Input::BudgetExhausted, now())
                .expect("the budget report is exempt from the budget it reports on");
            assert_eq!(
                transition.next.state,
                SyncState::Terminal(outcome),
                "budget exhaustion settles {state:?} in one transition"
            );
            assert_eq!(
                transition.effects,
                EffectList(vec![Effect::CaptureDiagnostics {
                    attempt: captured,
                    cause: CaptureCause::Ambiguity,
                }]),
                "budget exhaustion captures diagnostics from {state:?} and asks nothing else"
            );
        }
    }

    #[test]
    fn a_terminal_machine_accepts_nothing_at_all() {
        for input in input_pool() {
            let refused = machine(
                SyncState::Terminal(Outcome::Skipped {
                    code: FailureCode::Other,
                }),
                marker_strategy(),
                10,
            )
            .step(input.clone(), now());
            assert_eq!(
                refused,
                Err(MachineError::InputNotApplicable),
                "a terminal machine cannot be stepped by {input:?}, budget exhaustion included"
            );
        }
    }

    #[test]
    fn an_input_naming_another_machines_attempt_is_a_mismatch() {
        let refused = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::IntentRecorded(other_attempt()), now());
        assert_eq!(
            refused,
            Err(MachineError::AttemptMismatch),
            "a machine owning one attempt refuses an input naming another"
        );
    }

    #[test]
    fn replaying_the_attempt_the_machine_already_owns_is_inapplicable() {
        let refused = machine(
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::IntentRecorded(attempt()), now());
        assert_eq!(
            refused,
            Err(MachineError::InputNotApplicable),
            "a consumed input replayed is inapplicable rather than a mismatch"
        );
    }

    #[test]
    fn a_transition_costing_more_than_the_allowance_is_refused_whole() {
        let refused = machine(SyncState::AwaitingPreflight, marker_strategy(), 2)
            .step(Input::PreflightResult(Err(drift())), now());
        assert_eq!(
            refused,
            Err(MachineError::EffectBudgetExceeded),
            "a three-effect row does not emit a two-effect prefix"
        );
        let afforded = machine(SyncState::AwaitingPreflight, marker_strategy(), 3)
            .step(Input::PreflightResult(Err(drift())), now())
            .expect("three actions afford a three-effect row exactly");
        assert_eq!(
            afforded.next.budget,
            StepBudget {
                actions_remaining: 0
            },
            "the allowance is decremented by the number of effects emitted"
        );
    }

    #[test]
    fn the_attempt_renders_deterministically_as_marker_and_as_evidence() {
        let uniform = WriteAttemptId(Uuid([0xab; 16]));
        assert_eq!(
            marker_for(uniform),
            CorrelationMarker("tam-abababababababababababababababab".to_owned()),
            "the driver embeds this exact marker at submit time"
        );
        assert_eq!(
            ambiguous(uniform, AmbiguityCause::NoDurableIdentifier),
            Outcome::Ambiguous {
                attempt: AttemptId(Uuid([0xab; 16])),
                cause: AmbiguityCause::NoDurableIdentifier,
                evidence: EvidenceRef("write-attempt:abababababababababababababababab".to_owned()),
            },
            "the evidence reference is the attempt the diagnostics are stored under"
        );
    }

    /// Every pair the table does not list is inapplicable, and none of them
    /// panics. The states below are every non-terminal variant.
    #[test]
    fn every_state_and_input_pair_is_total() {
        let states = vec![
            SyncState::AwaitingPreflight,
            SyncState::PreflightAsserted {
                schema: Some(schema()),
            },
            SyncState::IntentRecorded {
                attempt: attempt(),
                schema: Some(schema()),
            },
            SyncState::Submitted {
                attempt: attempt(),
                evidence: evidence(),
            },
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            SyncState::Parked {
                attempt: Some(attempt()),
                challenge: ChallengeKind::Captcha,
            },
            SyncState::Terminal(Outcome::Blocked {
                challenge: ChallengeKind::Captcha,
            }),
        ];
        for state in states {
            for input in input_pool() {
                let outcome =
                    machine(state.clone(), marker_strategy(), 50).step(input.clone(), now());
                assert!(
                    outcome.is_ok()
                        || matches!(
                            outcome,
                            Err(MachineError::InputNotApplicable
                                | MachineError::AttemptMismatch
                                | MachineError::EffectBudgetExceeded)
                        ),
                    "stepping {state:?} with {input:?} must answer rather than panic"
                );
            }
        }
    }

    #[test]
    fn a_read_back_failure_the_table_does_not_list_is_inapplicable() {
        let refused = machine(
            SyncState::AwaitingReadBack {
                attempt: attempt(),
                locator: marker_locator(),
            },
            marker_strategy(),
            10,
        )
        .step(
            Input::ReadBackResult(Err(AdapterError::RateLimited { retry_after: None })),
            now(),
        );
        assert_eq!(
            refused,
            Err(MachineError::InputNotApplicable),
            "the table lists only the ambiguous read-back failure, so the rest are the driver's"
        );
    }

    /// A never-sent submit returns to `PreflightAsserted`, which owns no attempt
    /// and so accepts whatever identifier it is next handed. Handing it the one
    /// already submitted therefore submits that fencing token a second time.
    /// The machine holds no set of spent attempts and cannot detect this; not
    /// reusing a token is the driver's obligation, backed by the partial index
    /// on `write_attempt` that refuses a second in-flight row per item. This
    /// test pins the exposure so it stays visible rather than implicit.
    #[test]
    fn a_reused_fencing_token_is_the_drivers_obligation() {
        let first = machine(
            SyncState::PreflightAsserted {
                schema: Some(schema()),
            },
            marker_strategy(),
            10,
        )
        .step(Input::IntentRecorded(attempt()), now())
        .expect("a state owning no attempt accepts the one it is given");
        let returned = first
            .next
            .step(
                Input::SubmitResult(Err(AdapterError::NotSent(ConnectFailure::TcpRefused))),
                now(),
            )
            .expect("a never-sent submit returns to PreflightAsserted");
        let second = returned
            .next
            .step(Input::IntentRecorded(attempt()), now())
            .expect("the returned state owns no attempt and cannot refuse the reused one");
        assert_eq!(
            second.effects,
            EffectList(vec![Effect::Submit {
                attempt: attempt(),
                key: key(),
                fields: fields(),
            }]),
            "the machine resubmits a reused token, so the driver must never reuse one"
        );
    }

    /// The properties above are only worth anything if the generator reaches the
    /// terminals they quantify over, so each class is walked here explicitly.
    #[test]
    fn the_harness_reaches_every_terminal_class_the_properties_assert_about() {
        let start = || machine(SyncState::AwaitingPreflight, marker_strategy(), 40);
        let preflight = Input::PreflightResult(Ok(schema()));
        let intent = Input::IntentRecorded(attempt());

        let committed = drive(
            start(),
            &[
                preflight.clone(),
                intent.clone(),
                Input::SubmitResult(Ok(evidence())),
                Input::ReadBackResult(Ok(observed())),
            ],
        );
        assert!(
            matches!(committed.terminal, Some(Outcome::Committed { .. })),
            "the happy path settles as committed, got {:?}",
            committed.terminal
        );

        let ambiguous_run = drive(
            machine(
                SyncState::AwaitingPreflight,
                CreateStrategy::HaltOnAmbiguity,
                40,
            ),
            &[
                preflight.clone(),
                intent.clone(),
                Input::SubmitResult(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))),
            ],
        );
        assert!(
            matches!(ambiguous_run.terminal, Some(Outcome::Ambiguous { .. })),
            "an unreconcilable ambiguous submit settles as ambiguous, got {:?}",
            ambiguous_run.terminal
        );

        let blocked = drive(
            start(),
            &[
                preflight.clone(),
                intent.clone(),
                Input::SubmitResult(Err(AdapterError::SessionExpired)),
                Input::ParkExpired,
            ],
        );
        assert!(
            matches!(blocked.terminal, Some(Outcome::Blocked { .. })),
            "an unanswered park settles as blocked, got {:?}",
            blocked.terminal
        );

        let skipped = drive(
            start(),
            &[
                preflight.clone(),
                intent.clone(),
                Input::SubmitResult(Err(AdapterError::RateLimited { retry_after: None })),
            ],
        );
        assert!(
            matches!(skipped.terminal, Some(Outcome::Skipped { .. })),
            "a rate-limited submit settles as skipped, got {:?}",
            skipped.terminal
        );

        let rejected = drive(start(), &[Input::PreflightResult(Err(drift()))]);
        assert!(
            matches!(rejected.terminal, Some(Outcome::Rejected { .. })),
            "drift at preflight settles as rejected, got {:?}",
            rejected.terminal
        );

        let reconciled = drive(
            start(),
            &[
                preflight,
                intent,
                Input::SubmitResult(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))),
                Input::ReconcileResult(Ok(Some(listing()))),
            ],
        );
        assert!(
            matches!(reconciled.terminal, Some(Outcome::Committed { .. })),
            "a reconciled ambiguous create settles as committed, got {:?}",
            reconciled.terminal
        );
    }

    /// Four of the five properties are conditional on the run reaching a
    /// particular terminal, so a generator that rarely reaches one would pass
    /// that property vacuously. This samples the generator under a
    /// deterministic RNG and asserts the deep terminals are reached often
    /// enough for the conditionals to bite at the default case count.
    #[test]
    fn the_generator_reaches_the_terminals_the_properties_are_conditional_on() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

        const SAMPLES: u32 = 2_000;
        const FLOOR: u32 = 50;

        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            TestRng::deterministic_rng(RngAlgorithm::ChaCha),
        );
        let mut committed = 0_u32;
        let mut ambiguous_count = 0_u32;
        for _ in 0..SAMPLES {
            let run = arb_run()
                .new_tree(&mut runner)
                .expect("the run strategy always produces a value")
                .current();
            match &run.terminal {
                Some(Outcome::Committed { .. } | Outcome::Degraded { .. }) => committed += 1,
                Some(Outcome::Ambiguous { .. }) => ambiguous_count += 1,
                None
                | Some(
                    Outcome::Rejected { .. } | Outcome::Blocked { .. } | Outcome::Skipped { .. },
                ) => {}
            }
        }
        assert!(
            committed >= FLOOR,
            "the committed-terminal property would be near-vacuous: {committed} in {SAMPLES}"
        );
        assert!(
            ambiguous_count >= FLOOR,
            "the ambiguous-terminal properties would be near-vacuous: \
             {ambiguous_count} in {SAMPLES}"
        );
    }

    fn input_pool() -> Vec<Input> {
        vec![
            Input::PreflightResult(Ok(schema())),
            Input::PreflightResult(Err(drift())),
            Input::IntentRecorded(attempt()),
            Input::IntentRecorded(other_attempt()),
            Input::SubmitResult(Ok(evidence())),
            Input::SubmitResult(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))),
            Input::SubmitResult(Err(AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                detail: FailureDetail("refused".to_owned()),
            })),
            Input::SubmitResult(Err(AdapterError::Challenge(ChallengeKind::Captcha))),
            Input::SubmitResult(Err(AdapterError::SessionExpired)),
            Input::SubmitResult(Err(AdapterError::SchemaDrift(Box::new(drift())))),
            Input::SubmitResult(Err(AdapterError::RateLimited { retry_after: None })),
            Input::SubmitResult(Err(AdapterError::NotSent(ConnectFailure::TcpRefused))),
            Input::ReadBackResult(Ok(observed())),
            Input::ReadBackResult(Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))),
            Input::ReadBackResult(Err(AdapterError::RateLimited { retry_after: None })),
            Input::ReconcileResult(Ok(Some(listing()))),
            Input::ReconcileResult(Ok(None)),
            Input::ReconcileResult(Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))),
            Input::ChallengeCleared,
            Input::ParkExpired,
            Input::BudgetExhausted,
        ]
    }

    fn fresh_attempt(tick: usize) -> WriteAttemptId {
        let mut bytes = [0_u8; 16];
        bytes[0] = u8::try_from(tick).unwrap_or(u8::MAX);
        WriteAttemptId(Uuid(bytes))
    }

    /// Every write the closed set carries, so the two write-safety properties
    /// quantify over the destructive ones by construction. Naming only
    /// `Submit` here would compile and silently stop them at the create.
    fn write_attempt_of(effect: &Effect) -> Option<WriteAttemptId> {
        match effect {
            Effect::Submit { attempt, .. }
            | Effect::Revise { attempt, .. }
            | Effect::Remove { attempt, .. } => Some(*attempt),
            Effect::AssertFormSchema { .. }
            | Effect::RecordIntent { .. }
            | Effect::ReadBack { .. }
            | Effect::Reconcile { .. }
            | Effect::ParkItem { .. }
            | Effect::RequeueBehindGate { .. }
            | Effect::CaptureDiagnostics { .. }
            | Effect::Halt { .. }
            | Effect::Notify { .. } => None,
        }
    }

    #[derive(Debug)]
    struct Run {
        effects: Vec<Effect>,
        terminal: Option<Outcome>,
        /// Where in `effects` the terminal transition's own effects begin.
        terminal_at: usize,
        machine: SyncMachine,
        start_actions: u32,
    }

    /// Drives a machine through a sequence, cloning before each step because
    /// `step` consumes the machine and a refused input must leave it standing.
    /// Stops at the first terminal, which is what a driver does.
    fn drive(start: SyncMachine, inputs: &[Input]) -> Run {
        let start_actions = start.budget.actions_remaining;
        let mut machine = start;
        let mut effects = Vec::new();
        let mut terminal = None;
        let mut terminal_at = 0;
        for (tick, input) in inputs.iter().enumerate() {
            if terminal.is_some() {
                break;
            }
            let at = LogicalInstant(i64::try_from(tick).unwrap_or(i64::MAX));
            // Every delivery names a fresh attempt, which is the driver's
            // contract: it opens a new `write_attempt` row per `RecordIntent`
            // it executes, and the partial index on that table refuses a second
            // in-flight row for the item. The machine cannot check this itself
            // -- see `a_reused_fencing_token_is_the_drivers_obligation`.
            let delivered = if let Input::IntentRecorded(_) = input {
                Input::IntentRecorded(fresh_attempt(tick))
            } else {
                input.clone()
            };
            if let Ok(transition) = machine.clone().step(delivered, at) {
                if let SyncState::Terminal(outcome) = &transition.next.state {
                    terminal = Some(outcome.clone());
                    terminal_at = effects.len();
                }
                effects.extend(transition.effects.0.iter().cloned());
                machine = transition.next;
            }
        }
        if terminal.is_none() {
            terminal_at = effects.len();
        }
        Run {
            effects,
            terminal,
            terminal_at,
            machine,
            start_actions,
        }
    }

    /// The five properties quantify over terminals four and five transitions
    /// deep, and a uniform draw over the whole input vocabulary reaches
    /// `Committed` in well under one run in a hundred. The advancing inputs are
    /// therefore over-represented in the sampling pool, which biases the
    /// generator towards the deep paths without removing any input from it.
    fn sampling_pool() -> Vec<Input> {
        let advancing = [
            Input::PreflightResult(Ok(schema())),
            Input::IntentRecorded(attempt()),
            Input::SubmitResult(Ok(evidence())),
            Input::ReadBackResult(Ok(observed())),
            Input::ReconcileResult(Ok(Some(listing()))),
        ];
        let mut pool = input_pool();
        for _ in 0..6 {
            pool.extend(advancing.iter().cloned());
        }
        pool
    }

    fn arb_run() -> impl Strategy<Value = Run> {
        (
            proptest::collection::vec(proptest::sample::select(sampling_pool()), 0..30),
            1u32..50,
            proptest::sample::select(vec![
                marker_strategy(),
                draft_strategy(),
                CreateStrategy::HaltOnAmbiguity,
            ]),
            proptest::sample::select(vec![ItemOperation::Create, publication(), removal()]),
        )
            .prop_map(|(inputs, actions_remaining, strategy, operation)| {
                let start = machine_for(
                    operation,
                    SyncState::AwaitingPreflight,
                    strategy,
                    actions_remaining,
                );
                drive(start, &inputs)
            })
    }

    proptest! {
        /// No `Submit` effect is ever emitted twice for one `WriteAttemptId`.
        #[test]
        fn no_attempt_is_ever_submitted_twice(run in arb_run()) {
            let mut seen = std::collections::HashSet::new();
            for effect in &run.effects {
                if let Some(attempt) = write_attempt_of(effect) {
                    prop_assert!(
                        seen.insert(attempt),
                        "one write attempt must never be submitted twice"
                    );
                }
            }
        }

        /// Every terminal `Ambiguous` is preceded by a `RecordIntent`, so a
        /// crash from an ambiguous write always leaves something to reconcile.
        #[test]
        fn every_ambiguous_terminal_follows_a_recorded_intent(run in arb_run()) {
            if matches!(run.terminal, Some(Outcome::Ambiguous { .. })) {
                let recorded = run.effects[..run.terminal_at]
                    .iter()
                    .any(|effect| matches!(effect, Effect::RecordIntent { .. }));
                prop_assert!(
                    recorded,
                    "an ambiguous outcome with no recorded intent is unreconcilable"
                );
            }
        }

        /// No path leads from `Ambiguous` back to `Submit`.
        #[test]
        fn nothing_is_submitted_at_or_after_an_ambiguous_terminal(run in arb_run()) {
            if matches!(run.terminal, Some(Outcome::Ambiguous { .. })) {
                let resubmitted = run.effects[run.terminal_at..]
                    .iter()
                    .any(|effect| write_attempt_of(effect).is_some());
                prop_assert!(
                    !resubmitted,
                    "an ambiguous write is reconciled or escalated, never retried"
                );
                prop_assert!(
                    matches!(run.machine.state, SyncState::Terminal(_)),
                    "the run stopped at the terminal it reached"
                );
            }
        }

        /// Every `Committed` and every `Degraded` is preceded by the read that
        /// settled it: the read-back itself, or the reconcile that found the
        /// identifier the read-back then confirmed.
        #[test]
        fn every_committed_terminal_follows_a_read(run in arb_run()) {
            if matches!(
                run.terminal,
                Some(Outcome::Committed { .. } | Outcome::Degraded { .. })
            ) {
                let read = run.effects[..run.terminal_at].iter().any(|effect| {
                    matches!(effect, Effect::ReadBack { .. } | Effect::Reconcile { .. })
                });
                prop_assert!(
                    read,
                    "a committed outcome that no read preceded was never verified"
                );
            }
        }

        /// A `BudgetExhausted` input always reaches a terminal state in one
        /// transition, whatever non-terminal state the run left the machine in.
        #[test]
        fn budget_exhaustion_is_terminal_in_one_transition(run in arb_run()) {
            if !matches!(run.machine.state, SyncState::Terminal(_)) {
                let transition = run
                    .machine
                    .step(Input::BudgetExhausted, now())
                    .expect("a live machine always accepts the budget report");
                prop_assert!(
                    matches!(transition.next.state, SyncState::Terminal(_)),
                    "the budget report settles the item rather than deferring it"
                );
            }
        }

        /// The allowance is monotone: no transition ever hands back more
        /// actions than it was given.
        #[test]
        fn the_allowance_never_grows(run in arb_run()) {
            prop_assert!(
                run.machine.budget.actions_remaining <= run.start_actions,
                "a run cannot end with more allowance than it started with"
            );
            prop_assert!(
                u32::try_from(run.effects.len()).unwrap_or(u32::MAX)
                    <= run.start_actions.saturating_add(1),
                "a run emits no more effects than it was allowed, plus the exempt budget report"
            );
        }
    }
}
