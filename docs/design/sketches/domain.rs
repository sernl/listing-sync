//! Compilable sketch of the canonical-product domain model.
//!
//! This file exists because an adversarial review of the first engineering
//! charter found that its flagship artefact had never been compiled. Every type
//! quoted in `../2026-08-25-listing-sync-design.md` and its live siblings is
//! taken from here verbatim. Verify with:
//!
//! ```text
//! rustc --edition 2021 --crate-type lib -D warnings docs/design/sketches/domain.rs -o /dev/null
//! ```
//!
//! Stand-in types stand where a real crate type will go, so that the sketch
//! compiles with no dependency graph at all.

#![forbid(unsafe_code)]

// ---------------------------------------------------------------- stand-ins

/// Stands in for the `uuid` crate's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid(pub [u8; 16]);

/// Stands in for a real instant type. Milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub i64);

/// Stands in for a blake3 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

// ------------------------------------------------------------------ identity

macro_rules! id_newtype {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub struct $name(pub Uuid);
        )*
    };
}

id_newtype!(
    OrgId,
    ProductId,
    FileId,
    MappingId,
    AttemptId,
    JobId,
    ConnectionId,
    CanonicalTermId,
    UserId,
);

// -------------------------------------------------------- marketplace target

/// A marketplace as an account and a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Marketplace {
    Tes,
    Etsy,
    Tpt,
}

/// The inventory a listing is actually created in, which is the unit the model
/// keys on. Tes runs disjoint GB and US inventories under one marketplace, so
/// keying projections on `Marketplace` would make the entire first chargeable
/// product unrepresentable. Whether one author login reaches both inventories
/// is an assumption rather than a research finding, and it is settled by the
/// M-1 probe on the founder's own account; `Connection` scope depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InventoryId {
    TesGb,
    TesUs,
    Etsy,
    Tpt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Currency {
    Gbp,
    Usd,
}

/// How an inventory decides the currency a price is denominated in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrencyRule {
    /// Fixed by the inventory itself. Measured on Tes: fetching two US-inventory
    /// resources from a New Zealand client with `geoCurrency=AUD` cookies still
    /// returned USD offers, so currency follows the inventory rather than the
    /// viewer. The GB-cookie case specifically has not been tested.
    Fixed(Currency),
    /// Set by the seller at shop level. Unverified for Etsy and TPT; must be
    /// established before either connector is built.
    SellerScoped,
}

impl InventoryId {
    #[must_use]
    pub const fn marketplace(self) -> Marketplace {
        match self {
            Self::TesGb | Self::TesUs => Marketplace::Tes,
            Self::Etsy => Marketplace::Etsy,
            Self::Tpt => Marketplace::Tpt,
        }
    }

    #[must_use]
    pub const fn currency_rule(self) -> CurrencyRule {
        match self {
            Self::TesGb => CurrencyRule::Fixed(Currency::Gbp),
            Self::TesUs => CurrencyRule::Fixed(Currency::Usd),
            Self::Etsy | Self::Tpt => CurrencyRule::SellerScoped,
        }
    }
}

// --------------------------------------------------------------------- money

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Money {
    minor_units: i64,
    currency: Currency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    NotPositive,
}

impl Money {
    /// A positive amount. Zero is not a price; it is the `Free` variant of
    /// `PriceIntent`, which the marketplace parity rules treat differently.
    pub fn new(minor_units: i64, currency: Currency) -> Result<Self, MoneyError> {
        if minor_units <= 0 {
            return Err(MoneyError::NotPositive);
        }
        Ok(Self {
            minor_units,
            currency,
        })
    }

    #[must_use]
    pub const fn minor_units(self) -> i64 {
        self.minor_units
    }

    #[must_use]
    pub const fn currency(self) -> Currency {
        self.currency
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceIntent {
    Free,
    Paid(Money),
}

/// How one inventory's price is derived. Separate from the parity invariant,
/// which is a cross-inventory check rather than a derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceRule {
    /// Convert the canonical price at a rate recorded on the mapping.
    Converted { rate_micros: i64, rounding: Rounding },
    /// The seller set this inventory's price by hand.
    Explicit(PriceIntent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    Nearest,
    UpToCharm,
}

// --------------------------------------------------------------------- files

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    Payload,
    Preview,
    Cover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Pdf,
    Pptx,
    Docx,
    Zip,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Pending,
    Clean { at: Timestamp },
    Infected { signature: String },
    Failed { code: FailureCode },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductFile {
    pub id: FileId,
    pub role: FileRole,
    pub kind: FileKind,
    pub hash: ContentHash,
    pub byte_len: u64,
    pub scan: ScanOutcome,
}

/// A product with no payload cannot be listed anywhere, so the empty case is
/// removed rather than validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadSet {
    head: ProductFile,
    tail: Vec<ProductFile>,
}

impl PayloadSet {
    #[must_use]
    pub fn new(head: ProductFile, tail: Vec<ProductFile>) -> Self {
        Self { head, tail }
    }

    pub fn iter(&self) -> impl Iterator<Item = &ProductFile> {
        std::iter::once(&self.head).chain(self.tail.iter())
    }
}

// ------------------------------------------------------------------ taxonomy

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
    Resolved { edge: ProjectionEdge },
    /// The term genuinely has no counterpart and the mapping must omit it.
    NoCounterpart { decided_by: Decider, at: Timestamp },
}

// -------------------------------------------------------------------- grades

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

// ---------------------------------------------------------- listing copy

/// A title as the seller authored it. Per-inventory caps and character rules
/// are applied at projection time, never at authoring time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(pub String);

/// How a marketplace counts a title against its cap. Unverified on TPT, whose
/// 80-character cap was established from a sample containing no astral-plane
/// characters and therefore cannot distinguish these cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Bytes,
    Utf16CodeUnits,
    Codepoints,
    GraphemeClusters,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingCopy {
    pub body: String,
}

// -------------------------------------------------------------- remote identity

/// A marketplace's durable identifier for a listing, one variant per
/// marketplace, so a TPT identifier cannot be stored where a Tes one belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteListingId {
    /// The resource URL, which Tes states stays tied to the original resource
    /// title even after the author retitles it.
    Tes { url: String },
    /// The numeric product id. The slug is decorative: a product path carrying
    /// the wrong slug still serves the correct product.
    Tpt { product_id: u64 },
    Etsy { listing_id: u64 },
}

impl RemoteListingId {
    #[must_use]
    pub const fn marketplace(&self) -> Marketplace {
        match self {
            Self::Tes { .. } => Marketplace::Tes,
            Self::Tpt { .. } => Marketplace::Tpt,
            Self::Etsy { .. } => Marketplace::Etsy,
        }
    }
}

/// Proof that a write landed. Its fields are private and its only constructor
/// belongs to the adapter layer, so a read cannot be justified by a receipt
/// that no write produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReceipt {
    attempt: AttemptId,
    id: RemoteListingId,
    at: Timestamp,
}

impl WriteReceipt {
    #[must_use]
    pub(crate) fn issue(attempt: AttemptId, id: RemoteListingId, at: Timestamp) -> Self {
        Self { attempt, id, at }
    }

    #[must_use]
    pub const fn attempt(&self) -> AttemptId {
        self.attempt
    }

    #[must_use]
    pub const fn listing(&self) -> &RemoteListingId {
        &self.id
    }

    #[must_use]
    pub const fn at(&self) -> Timestamp {
        self.at
    }
}

/// A grant issued per marketplace from a recorded permission decision. It keeps
/// the structural probe available on Tes and unreachable on TPT until written
/// permission exists, which is the point on which the research is clearest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryGrant {
    pub inventory: InventoryId,
    pub decided_at: Timestamp,
}

/// The only way to construct a marketplace read. Every variant names a reason
/// the read is permitted, and there is no constructor for any other reason, so
/// link-following, listing pages, search and pagination are unrepresentable
/// rather than merely forbidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchReason {
    /// Tier one: the marketplace's own first-party export of the seller's data.
    FirstPartyExport { inventory: InventoryId },
    /// Tier two: one fetch of one listing by a durable identifier already held,
    /// caused by and immediately following an authorised write.
    VerifyWrite { receipt: WriteReceipt },
    /// Tier two, stretched: a listing whose moderation outcome is still pending
    /// is polled by the same durable identifier on a bounded schedule.
    PollLifecycle { receipt: WriteReceipt },
    /// The hourly read-only structural probe, which resolves the current
    /// selectors against a form and creates, submits and deletes nothing.
    StructuralProbe { grant: CanaryGrant },
}

// ------------------------------------------------------------------- binding

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelationMarker(pub String);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKey {
    Title,
    Description,
    Price,
    Taxonomy,
    Grades,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMismatch {
    pub field: FieldKey,
    pub class: MismatchClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MismatchClass {
    /// Entity re-encoding, Unicode normalisation, curly quotes, whitespace
    /// collapse, tag reordering. Expected, and not a defect.
    Normalised,
    Truncated { limit_observed: usize },
    Missing,
    WrongField { observed_in: FieldKey },
    Unexpected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchResponse {
    Accept,
    Degrade,
    HaltInventory,
    HaltAndPage,
}

impl MismatchClass {
    /// The response is a total function of the class rather than a runbook
    /// paragraph, so an agent editing the classifier cannot leave a class
    /// without an action.
    #[must_use]
    pub const fn response(&self) -> MismatchResponse {
        match self {
            Self::Normalised => MismatchResponse::Accept,
            Self::Truncated { .. } => MismatchResponse::Degrade,
            Self::Missing => MismatchResponse::HaltInventory,
            Self::WrongField { .. } | Self::Unexpected => MismatchResponse::HaltAndPage,
        }
    }
}

// ------------------------------------------------------------------- mapping

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

/// Tes states a resource may not appear for up to three working days after
/// submission, so published is not the terminal state and a boolean is wrong.
/// `Draft` is unconditional here; whether a given inventory can be driven into
/// it is the per-inventory `DraftSupport` capability, which the M-1 probe sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteLifecycle {
    Absent,
    Draft,
    Submitted { at: Timestamp },
    InReview { since: Timestamp },
    Live { since: Timestamp },
    Rejected { at: Timestamp, reason: Option<String> },
    Withdrawn { at: Timestamp },
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

// ------------------------------------------------------- canonical product

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

// -------------------------------------------------------------- write outcome

/// A submit has three answers, not two, and the third is not a kind of failure:
/// it landed, it did not land, or we do not know. An ambiguous attempt is never
/// retried; it is reconciled, and if reconciliation cannot decide it is
/// escalated with this inventory halted for this tenant.
///
/// The six variants below are those three answers plus the three ways an item
/// reaches a terminal state without a submit being attempted or completed.
/// `Committed` and `Degraded` both carry a `WriteReceipt` rather than a bare
/// identifier, because the receipt is the only thing that can construct the
/// read-back in `FetchReason::VerifyWrite`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Committed { receipt: WriteReceipt, report: FieldDiffReport },
    Degraded { receipt: WriteReceipt, report: FieldDiffReport },
    Rejected { code: FailureCode, detail: FailureDetail },
    Ambiguous { attempt: AttemptId, cause: AmbiguityCause, evidence: EvidenceRef },
    /// Non-terminal while the challenge is live. It becomes the terminal
    /// `ItemOutcome::Blocked` only when the park expires unanswered.
    Blocked { challenge: ChallengeKind },
    Skipped { code: FailureCode },
}

/// The per-field comparison of declared intent against what read-back observed.
/// Empty means every managed field matched after normalisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDiffReport {
    pub normaliser_version: u32,
    pub mismatches: Vec<FieldMismatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbiguityCause {
    SubmitTimedOut,
    ResponseEventLost,
    ProcessKilledByBackstop,
    ReadBackIndeterminate,
    NoDurableIdentifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeKind {
    EmailedOneTimePassword,
    Captcha,
    JavaScriptInterstitial,
    ReauthRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRef(pub String);

/// Closed, versioned and low-cardinality, with an explicit other-case, so a
/// failure list of two hundred items is filterable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureCode {
    SelectorNotFound,
    SelectorAmbiguous,
    SelectorResolvedViaFallback,
    PreconditionElementAbsent,
    NavigationCancelled,
    UnexpectedOrigin,
    SubmitNoConfirmation,
    ChallengePresented,
    SessionExpired,
    UploadRejected,
    RateLimited,
    VerificationMismatch,
    FormSchemaDrift,
    /// The loaded selector pack's declared adapter version is not one this
    /// build accepts. Replaces the client-fleet-era `PackExpired`/`PackRejected`.
    AdapterVersionRejected,
    Other,
}

/// Adapter-supplied free text accompanying a `FailureCode`. Never parsed, never
/// crosswalked to copy, and never permitted to decide an outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureDetail(pub String);

// --------------------------------------------------------------- job ledger

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

// ------------------------------------------------------------ the write path

/// The only place a `WriteReceipt` is minted. In the real workspace this module
/// is the `tam-marketplace` crate, which is why `WriteReceipt` and `FetchReason`
/// live beside the write path rather than in `tam-types`: Rust's finest
/// visibility tool is crate-scoped, so the capability is only airtight when the
/// constructor and its sole caller share a crate.
pub mod adapter {
    use super::{
        AttemptId, FetchReason, FieldDiffReport, Outcome, RemoteListingId, Timestamp,
        WriteReceipt,
    };

    /// Called once, after read-back has settled a write. The report decides
    /// which of the two committed-class outcomes this is, so a degraded landing
    /// cannot be recorded as a clean one.
    #[must_use]
    pub fn settle(
        attempt: AttemptId,
        id: RemoteListingId,
        at: Timestamp,
        report: FieldDiffReport,
    ) -> Outcome {
        let receipt = WriteReceipt::issue(attempt, id, at);
        let degraded = report
            .mismatches
            .iter()
            .any(|m| matches!(m.class, super::MismatchClass::Truncated { .. }));
        if degraded {
            Outcome::Degraded { receipt, report }
        } else {
            Outcome::Committed { receipt, report }
        }
    }

    /// A read is justified by a receipt or it does not happen.
    #[must_use]
    pub fn verify_after(receipt: WriteReceipt) -> FetchReason {
        FetchReason::VerifyWrite { receipt }
    }
}

// ------------------------------------------------------------ create strategy

/// Whether an inventory can be driven into `RemoteLifecycle::Draft`. The M-1
/// probe sets it; until then it is `Unprobed` and `DraftThenPublish` may not be
/// configured, so an unresolved probe blocks a configuration rather than
/// leaving a type variant conditionally present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftSupport {
    Supported,
    Unsupported,
    Unprobed,
}

/// The seller-visible field a correlation marker may occupy. The Tes Author
/// Code prohibits external URLs in descriptions, titles and previews, so a
/// marker is never URL-shaped, and the title is excluded because listing copy
/// must read as the seller's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerField {
    DescriptionTail,
    InternalReference,
}

/// How long a marker stays in the field before a scheduled pass removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerLifetime {
    pub seconds: u32,
}

/// Configured per inventory, never branched on in code, so the M-1 probe result
/// changes a value rather than a control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStrategy {
    /// Create as draft, then publish; an ambiguous publish is safely repeatable.
    /// Configurable only where `DraftSupport::Supported`.
    DraftThenPublish { draft_state: RemoteLifecycleKind },
    /// Embed a correlation marker in a named seller-visible field, then
    /// reconcile on it.
    CorrelationMarker { field: MarkerField, ttl: MarkerLifetime },
    /// No reconciliation is possible; record `Ambiguous` and halt this
    /// inventory for this tenant.
    HaltOnAmbiguity,
}

/// The discriminant of `RemoteLifecycle`, usable in `Copy` configuration where
/// the timestamped variant payloads are not wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteLifecycleKind {
    Absent,
    Draft,
    Submitted,
    InReview,
    Live,
    Rejected,
    Withdrawn,
}

// -------------------------------------------------------------- adapter seam

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDrift {
    pub form: FormId,
    pub expected: FormSchemaFingerprint,
    pub observed: FormSchemaFingerprint,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// Classified at the `reqwest` layer, below `thirtyfour`, because
/// `impl From<reqwest::Error> for WebDriverError` flattens the error to a
/// string and destroys `is_timeout()` versus `is_connect()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFailure {
    DnsFailure,
    TcpRefused,
    TlsHandshakeFailed,
    NoRouteToHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurationSecs(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// The write may have landed. Never retried; reconciled or escalated.
    Ambiguous(AmbiguityCause),
    Rejected { code: FailureCode, detail: FailureDetail },
    Challenge(ChallengeKind),
    SessionExpired,
    SchemaDrift(SchemaDrift),
    RateLimited { retry_after: Option<DurationSecs> },
    /// The request provably never left. The only class that is safe to retry.
    NotSent(ConnectFailure),
}

// ------------------------------------------------------------ the sync machine

/// A clock reading passed into the machine rather than read by it, which is
/// what makes replay exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalInstant(pub i64);

id_newtype!(WriteAttemptId, JobItemId, FormId, ActionId, GrantId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormSchemaFingerprint(pub ContentHash);

/// The named field set a submit writes, already projected and already length-
/// capped for this inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSet {
    pub entries: Vec<(FieldKey, String)>,
    pub files: Vec<FileId>,
}

/// What the driver observed about the submit itself, which is evidence and
/// never a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitEvidence {
    pub http_status: Option<u16>,
    pub response_body_digest: Option<ContentHash>,
    pub landed_on_route: Option<String>,
    pub observed_lag: bool,
}

/// How a listing is addressed for read-back. A locator is derivable from a
/// receipt or from a marker search, and from nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingLocator {
    Durable(RemoteListingId),
    Marker { marker: CorrelationMarker, inventory: InventoryId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedListing {
    pub id: RemoteListingId,
    pub fields: Vec<(FieldKey, String)>,
    pub lifecycle: RemoteLifecycle,
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
    PreflightAsserted { schema: FormSchemaFingerprint },
    IntentRecorded { attempt: WriteAttemptId },
    Submitted { attempt: WriteAttemptId, evidence: SubmitEvidence },
    AwaitingReadBack { attempt: WriteAttemptId, locator: ListingLocator },
    Parked { attempt: Option<WriteAttemptId>, challenge: ChallengeKind },
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
    AssertFormSchema { form: FormId },
    RecordIntent { attempt: WriteAttemptId, intent_hash: ContentHash },
    Submit { attempt: WriteAttemptId, key: IdempotencyKey, fields: FieldSet },
    ReadBack { locator: ListingLocator, reason: FetchReason },
    /// Search for a marker or an in-flight attempt's landing, which is the only
    /// permitted response to an ambiguous submit.
    Reconcile { attempt: WriteAttemptId, locator: ListingLocator },
    /// Tear the browser down and hold the durable item.
    ParkItem { item: JobItemId, challenge: ChallengeKind, expires: LogicalInstant },
    /// Put every item for this connection back in the queue behind a gate,
    /// idempotency keys intact.
    RequeueBehindGate { connection: ConnectionId, cause: BlockCause },
    CaptureDiagnostics { attempt: WriteAttemptId, cause: CaptureCause },
    Halt { scope: HaltScope },
    Notify { org: OrgId, event: SellerEvent },
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
    /// The body here is a stub: this file is the artefact of record for the
    /// types, and the transition table is specified in `../sync-machine.md`.
    pub fn step(self, input: Input, now: LogicalInstant) -> Result<Transition, MachineError> {
        let _ = (self, input, now);
        Err(MachineError::InputNotApplicable)
    }
}

// ------------------------------------------------------------- custody seam

/// How a seller's marketplace access is held. `StoredCredential` is today's
/// model and the founder recorded it as interim, so the two better models are
/// variants of this enum rather than a redesign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyModel {
    /// Today. Seller supplies credentials; we hold ciphertext under a
    /// per-tenant data-encryption key.
    StoredCredential,
    /// Seller authenticates themselves; we never see the password.
    SellerDrivenSession,
    /// A marketplace-sanctioned delegated grant, if one is ever obtained.
    PartnerGrant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeasePurpose {
    Write,
    VerifyWrite,
    PollLifecycle,
    StructuralProbe,
}

/// A driver endpoint for a browser the broker launched and primed. It exposes
/// no secret material, which is the type-level statement of the rule that a
/// worker can use a connection and cannot read one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLease {
    pub endpoint: String,
    pub grant: GrantId,
    pub expires: LogicalInstant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyError {
    NotLinked,
    NeedsReauth,
    Revoked,
    BrokerUnavailable,
    GrantRefused,
}

pub trait ConnectionProvider: Send + Sync {
    fn model(&self) -> CustodyModel;

    /// There is deliberately no `get_session`, and no accessor returns secret
    /// material. Written with an explicit `impl Future` return rather than
    /// `async fn`, because `async_fn_in_trait` is a hard error under a
    /// deny-warnings build and the desugaring is what a multi-threaded runtime
    /// requires anyway.
    fn lease(
        &self,
        org: OrgId,
        connection: ConnectionId,
        purpose: LeasePurpose,
        grant: GrantId,
    ) -> impl std::future::Future<Output = Result<SessionLease, CustodyError>> + Send;
}

// --------------------------------------------------------------- job ledger

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

// ------------------------------------------------- adapter and fault seams

/// The adapter seam. Keyed on `InventoryId` rather than `Marketplace`, so the
/// Tes crate registers two adapters that share markup, a login and an upload
/// flow and differ only in vocabulary and currency, which are data.
///
/// Every method is written as `fn f(..) -> impl Future<..> + Send` rather than
/// `async fn`, because `async_fn_in_trait` is a hard error under a
/// deny-warnings build and the desugaring is what a multi-threaded runtime
/// requires anyway.
pub trait MarketplaceAdapter: Send + Sync {
    fn inventory(&self) -> InventoryId;

    fn assert_form_schema(
        &self,
        org: OrgId,
        form: FormId,
    ) -> impl std::future::Future<Output = Result<FormSchemaFingerprint, AdapterError>> + Send;

    /// `IdempotencyKey` is required, so a submit without one does not typecheck.
    fn submit(
        &self,
        org: OrgId,
        key: IdempotencyKey,
        fields: FieldSet,
    ) -> impl std::future::Future<Output = Result<SubmitEvidence, AdapterError>> + Send;

    fn read_back(
        &self,
        org: OrgId,
        locator: ListingLocator,
        reason: FetchReason,
    ) -> impl std::future::Future<Output = Result<ObservedListing, AdapterError>> + Send;
}

/// The six transport faults the research names, plus the two ambiguity faults
/// this design adds because no external service will produce them on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectedFault {
    RateLimited { retry_after: Option<DurationSecs> },
    Forbidden,
    HtmlInterstitial,
    RedirectToSignIn,
    TruncatedBody,
    ConnectionResetMidBody,
    ResponseEventLost,
    ProcessKilledAfterSubmit,
}

/// A trait rather than a `cfg(test)` switch, because the same faults must be
/// walked by a scheduled synthetic in production.
pub trait FaultPlan: Send + Sync {
    fn next_fault(&self, action: ActionId) -> Option<InjectedFault>;
}
