//! The marketplace adapter seam, the write path and its three-valued outcome.
//!
//! Every definition is promoted verbatim from `docs/design/sketches/domain.rs`,
//! the artefact of record for the domain types. This crate is the only place a
//! `WriteReceipt` is minted, which is why `WriteReceipt` and `FetchReason` live
//! beside the write path rather than in `tam-types`: Rust's finest visibility
//! tool is crate-scoped, so the read capability is only airtight because the
//! constructor and its sole caller, [`settle`], share this crate.

#![forbid(unsafe_code)]

pub mod cassette;
pub mod idempotency;
pub mod transport;

use tam_types::{
    AttemptId, ConnectionId, ContentHash, CopyFormat, FailureCode, FailureDetail, FieldKey,
    FieldMismatch, FileId, ImportedPrice, ImportedTerm, InventoryId, LogicalInstant, Marketplace,
    MismatchClass, OrgId, PriceIntent, TermKind, Timestamp, Uuid,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WriteAttemptId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrantId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(pub Uuid);

/// A marketplace's durable identifier for a listing, one variant per
/// marketplace, so a TPT identifier cannot be stored where a Tes one belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteListingId {
    /// The resource URL, which Tes states stays tied to the original resource
    /// title even after the author retitles it.
    Tes {
        url: String,
    },
    /// The numeric product id. The slug is decorative: a product path carrying
    /// the wrong slug still serves the correct product.
    Tpt {
        product_id: u64,
    },
    Etsy {
        listing_id: u64,
    },
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
    /// The pre-settle verification read, justified by the open `write_attempt`
    /// fencing row rather than by a receipt — the receipt cannot exist yet
    /// because minting it needs this read's result. The attempt is the
    /// authorisation trail: the row was written before the click.
    VerifyAttempt { attempt: WriteAttemptId },
    /// Tier two, stretched: a listing whose moderation outcome is still pending
    /// is polled by the same durable identifier on a bounded schedule.
    PollLifecycle { receipt: WriteReceipt },
    /// The hourly read-only structural probe, which resolves the current
    /// selectors against a form and creates, submits and deletes nothing.
    StructuralProbe { grant: CanaryGrant },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelationMarker(pub String);

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
    Committed {
        receipt: WriteReceipt,
        report: FieldDiffReport,
    },
    Degraded {
        receipt: WriteReceipt,
        report: FieldDiffReport,
    },
    Rejected {
        code: FailureCode,
        detail: FailureDetail,
    },
    Ambiguous {
        attempt: AttemptId,
        cause: AmbiguityCause,
        evidence: EvidenceRef,
    },
    /// Non-terminal while the challenge is live. It becomes the terminal
    /// item outcome `Blocked` only when the park expires unanswered.
    Blocked {
        challenge: ChallengeKind,
    },
    Skipped {
        code: FailureCode,
    },
}

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
        .any(|m| matches!(m.class, MismatchClass::Truncated { .. }));
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
    CorrelationMarker {
        field: MarkerField,
        ttl: MarkerLifetime,
    },
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

/// Tes states a resource may not appear for up to three working days after
/// submission, so published is not the terminal state and a boolean is wrong.
/// `Draft` is unconditional here; whether a given inventory can be driven into
/// it is the per-inventory `DraftSupport` capability, which the M-1 probe sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteLifecycle {
    Absent,
    Draft,
    Submitted {
        at: Timestamp,
    },
    InReview {
        since: Timestamp,
    },
    Live {
        since: Timestamp,
    },
    Rejected {
        at: Timestamp,
        reason: Option<String>,
    },
    Withdrawn {
        at: Timestamp,
    },
}

/// Which side of the draft line a listing sits on: the two states both
/// platforms distinguish on the *write* path. Narrower than
/// [`RemoteLifecycleKind`], which is the *observation* vocabulary and carries
/// moderation states no write can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingState {
    Draft,
    Live,
}

/// Where the caller states the listing is now and where this write leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleTransition {
    pub from: ListingState,
    pub to: ListingState,
}

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
    Rejected {
        code: FailureCode,
        detail: FailureDetail,
    },
    Challenge(ChallengeKind),
    SessionExpired,
    /// Boxed so the error stays register-sized: the drift report carries the
    /// full added/removed field lists and would otherwise dominate every
    /// `Result` on the adapter path.
    SchemaDrift(Box<SchemaDrift>),
    RateLimited {
        retry_after: Option<DurationSecs>,
    },
    /// The request provably never left. The only class that is safe to retry.
    NotSent(ConnectFailure),
    /// A capability the adapter does not have because no capture settles it.
    /// Distinct from `Rejected`: the marketplace refused nothing, there was
    /// nothing to send. The TPT file download and TPT import canonicalisation
    /// are today's members, both on the M7 plan's deferred list.
    Uncaptured {
        capability: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormSchemaFingerprint(pub ContentHash);

/// The named field set a submit writes, already projected and already length-
/// capped for this inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSet {
    pub entries: Vec<(FieldKey, String)>,
    pub files: Vec<FileId>,
}

/// One projected vocabulary term as the seam carries it: the marketplace's
/// own identifier where the crosswalk holds one, and the path it named. The
/// canonical term stays on the domain side; an adapter sees only what the
/// marketplace itself would recognise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTerm {
    pub native_id: Option<String>,
    pub segments: Vec<String>,
}

/// A product's derived age span in years, where its grade declaration
/// resolved to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgeSpan {
    pub low_years: u8,
    pub high_years: u8,
}

/// The platform-neutral rendering of one product, and the input to
/// [`MarketplaceAdapter::project_fields`]. It carries what every marketplace
/// needs and nothing any one of them encodes: no licence token, no category
/// numbering, no age-range JSON, because those are wire shapes and a wire
/// shape belongs to the adapter that speaks it.
///
/// This is the seam-side image of the domain's listing projection. The domain
/// crate depends on this one, so the projection type itself cannot appear
/// here; the engine lowers it at the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedListing {
    pub title: String,
    pub body: String,
    /// How `body` is written, so an adapter whose platform takes the other
    /// format refuses rather than posting escaped markup nobody asked for.
    pub body_format: CopyFormat,
    pub price: PriceIntent,
    pub taxonomy: Vec<NativeTerm>,
    pub grades: Vec<NativeTerm>,
    pub ages: Option<AgeSpan>,
    pub files: Vec<FileId>,
    /// Values the projection resolved in axes this struct names no field for,
    /// each labelled by the axis it answers. Empty for every listing whose
    /// target binds no such axis, which is why the fields above do not move
    /// and why the TPT adapter reads nothing new.
    pub natives: Vec<NativeAxis>,
}

/// One axis's resolved value as the seam carries it: which equivalence axis it
/// answers, and the target vocabulary's own term for it.
///
/// The value travels as `NativeTerm`, whose `native_id` is "the marketplace's
/// own identifier where the crosswalk holds one" -- so an elected Tes licence
/// arrives here as the wire token itself, exactly as `taxonomy` already
/// carries category ids. What stays in the adapter is which `FieldKey` the
/// value lands in and how it is framed on the wire, not the identifier: an
/// adapter that read a display label out of `segments` and re-derived a token
/// from it would reintroduce the guess the election exists to remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAxis {
    pub axis: TermKind,
    pub value: NativeTerm,
}

/// What the driver observed about the submit itself, which is evidence and
/// never a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitEvidence {
    pub http_status: Option<u16>,
    pub response_body_digest: Option<ContentHash>,
    pub landed_on_route: Option<String>,
    /// The durable identifier the submit landed on, when the adapter can
    /// state it — Tes can, because its create returns the resource id. This
    /// is what lets the pre-settle verification read address the listing by a
    /// durable id rather than by a marker search no adapter can serve yet.
    ///
    /// It is what the verification read *addresses*, not a claim that this
    /// write created it: a removal states the listing whose disappearance is
    /// its proof, and the driver turns that into a sever rather than a bind.
    pub landed: Option<RemoteListingId>,
    pub observed_lag: bool,
}

/// How a listing is addressed for read-back. A locator is derivable from a
/// receipt or from a marker search, and from nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingLocator {
    Durable(RemoteListingId),
    Marker {
        marker: CorrelationMarker,
        inventory: InventoryId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedListing {
    pub id: RemoteListingId,
    pub fields: Vec<(FieldKey, String)>,
    pub lifecycle: RemoteLifecycle,
}

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

    /// Renders a projected listing into the field set this marketplace's
    /// submit accepts. Every wire shape lives behind this method — licence
    /// tokens, category numbering, age-range JSON — so the engine that seeds
    /// an item carries no marketplace's encoding. Pure, and therefore not a
    /// future: it reads no session and performs no I/O.
    ///
    /// Refuses rather than approximates. A projection this marketplace cannot
    /// express — a term whose native identifier is missing or of the wrong
    /// shape — is a rejection here, before an attempt is opened.
    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError>;

    fn assert_form_schema(
        &self,
        org: OrgId,
        form: FormId,
    ) -> impl std::future::Future<Output = Result<FormSchemaFingerprint, AdapterError>> + Send;

    /// `IdempotencyKey` is required, so a submit without one does not typecheck.
    ///
    /// `now` is the driver's clock reading passed in as data, mirroring
    /// `read_back`'s `observed_at`: an adapter holds no clock, and a submit
    /// whose wire shape needs the current instant reads it from here.
    fn submit(
        &self,
        org: OrgId,
        key: IdempotencyKey,
        fields: FieldSet,
        now: Timestamp,
    ) -> impl std::future::Future<Output = Result<SubmitEvidence, AdapterError>> + Send;

    /// `observed_at` is the driver's clock reading passed in as data — an
    /// adapter holds no clock — and it is what an observed lifecycle instant
    /// is attested against.
    fn read_back(
        &self,
        org: OrgId,
        locator: ListingLocator,
        reason: FetchReason,
        observed_at: Timestamp,
    ) -> impl std::future::Future<Output = Result<ObservedListing, AdapterError>> + Send;

    /// Rewrites an existing listing and states which side of the draft line
    /// it should land on. Publishing is this call with `to: Live`; an edit
    /// that means to keep the current state names it on both halves.
    ///
    /// The transition travels whole because both routes are asymmetric and
    /// discovering the current state is unsound: the route that would answer
    /// lags the write that produced it (Tes measured 2026-08-29, TPT
    /// 2026-08-28), so a probe run soon after a write reads a live listing as
    /// a draft. A caller that just wrote always knows what it wrote.
    ///
    /// Every implementation posts and classifies; none of them verifies.
    /// Verification is the driver's, because only the driver holds a budget
    /// to poll with.
    fn revise(
        &self,
        org: OrgId,
        plan: RevisePlan,
        now: Timestamp,
    ) -> impl std::future::Future<Output = Result<SubmitEvidence, AdapterError>> + Send;

    /// Removes an existing listing from the state the caller states it is in.
    ///
    /// `plan.attempt` is the authorisation: a removal is permitted only while
    /// the caller holds the open, epoch-fenced `write_attempt` row for the
    /// mapping that names this listing. The row is written before the click,
    /// so a removal that reached the marketplace is one the ledger asked for.
    /// The structural enforcement is the closed `Effect` set, not this
    /// argument: `Effect::Remove` is constructible only inside `tam-domain`
    /// and reachable only from a state a successful `RecordIntent` produced.
    fn remove(
        &self,
        org: OrgId,
        plan: RemovalPlan,
        now: Timestamp,
    ) -> impl std::future::Future<Output = Result<SubmitEvidence, AdapterError>> + Send;
}

/// What a revise addresses and what it means to do to it. A struct rather
/// than four more parameters, which is also what keeps the subject and the
/// transition travelling together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisePlan {
    pub subject: RemoteListingId,
    pub fields: FieldSet,
    pub transition: LifecycleTransition,
}

/// What a removal addresses, and the open fenced attempt that authorises it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalPlan {
    pub attempt: WriteAttemptId,
    pub subject: RemoteListingId,
    pub state: ListingState,
}

/// A seller's own listing as the first-party import read yields it:
/// verbatim marketplace vocabulary for canonicalisation, no interpretation.
/// Read only under [`FetchReason::FirstPartyExport`]; the adapter refuses
/// any other reason.
///
/// One shape serves both sources. The per-axis field lists it used to carry
/// were Tes's own wire names, so a second platform had nowhere to put its
/// values; `native` is every source value in the source's own vocabulary,
/// tagged with an axis where the adapter knows one and left untagged where it
/// does not, which is the honest state of eight of TPT's twelve facet
/// categories.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedListing {
    pub remote: RemoteListingId,
    pub title: String,
    pub body: String,
    pub body_format: CopyFormat,
    /// Every value the read carried, verbatim. What used to be `licence`,
    /// `category_native_ids`, `age_range_native_ids`, `year_groups` and
    /// `curriculum`, plus everything a second platform carries that no Tes
    /// field name covers.
    pub native: Vec<ImportedTerm>,
    /// The grant the source stated, as the source stated it. Read and
    /// discarded before this existed, which is the defect it repairs.
    pub rights: Option<ImportedTerm>,
    pub price: ImportedPrice,
    /// Which side of the draft line the source sits on, read rather than
    /// assumed. `None` means the read did not carry it — the honest state of
    /// every TPT capture on file. A migrate's removal names this value, so
    /// `None` refuses at the API rather than defaulting: a removal must never
    /// carry a lifecycle nobody has observed.
    pub state: Option<ListingState>,
}

impl ImportedListing {
    /// The source's own values on one axis, in read order. An untagged term
    /// answers no axis and is therefore in no axis's list.
    #[must_use]
    pub fn axis(&self, kind: TermKind) -> Vec<&ImportedTerm> {
        self.native
            .iter()
            .filter(|term| term.kind == Some(kind))
            .collect()
    }

    /// The wire tokens on one axis, which is what the taxonomy relation joins
    /// on. A term with no native id contributes nothing, because the id is
    /// what the join needs.
    #[must_use]
    pub fn native_ids(&self, kind: TermKind) -> Vec<String> {
        self.axis(kind)
            .into_iter()
            .filter_map(|term| term.native_id.clone())
            .collect()
    }
}

/// The tier-one capability as a seam: the marketplace's own reads of the
/// seller's own data, which is the only tier that justifies an enumeration-
/// shaped read. Separate from [`MarketplaceAdapter`] because exporting and
/// writing are different privileges, and an importer that binds this trait
/// addresses the capability rather than one marketplace's client type.
///
/// Every implementation refuses a reason other than
/// [`FetchReason::FirstPartyExport`]; the trait states the shape and the
/// adapter states the refusal, because the reason is checked against the
/// marketplace the adapter serves.
///
/// The two associated types are the marketplace's own handles — how it names
/// a catalogue row and how it addresses one of the seller's resources — which
/// no shared type can stand in for. Written with explicit `impl Future`
/// returns rather than `async fn`, because `async_fn_in_trait` is a hard
/// error under a deny-warnings build.
pub trait FirstPartyExport: Send + Sync {
    /// One row of the seller's own catalogue, in the marketplace's vocabulary.
    type CatalogueEntry;
    /// How the marketplace addresses one of the seller's own resources.
    type Resource;

    /// The seller's own catalogue, whole. A walk that cannot reach its end
    /// refuses rather than returning a truncation an importer would mistake
    /// for the entire catalogue.
    fn list_own_resources(
        &self,
        reason: &FetchReason,
    ) -> impl std::future::Future<Output = Result<Vec<Self::CatalogueEntry>, AdapterError>> + Send;

    /// The bytes of the seller's own files for one resource.
    fn download_resource_bundle(
        &self,
        reason: &FetchReason,
        id: Self::Resource,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, AdapterError>> + Send;

    /// One of the seller's own listings, verbatim, for canonicalisation.
    fn fetch_for_import(
        &self,
        reason: &FetchReason,
        id: Self::Resource,
    ) -> impl std::future::Future<Output = Result<ImportedListing, AdapterError>> + Send;
}

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

/// File content an adapter uploads, resolved through [`FileSource`]. The
/// name and content type travel with the bytes because the S3 policy signs
/// form fields derived from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContent {
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSourceError {
    Missing(FileId),
    Unreadable { file: FileId, detail: String },
}

/// Resolves a `FileId` to its content. The file pipeline (M1f) implements
/// this over object storage; tests hand bytes straight back.
pub trait FileSource: Send + Sync {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl std::future::Future<Output = Result<FileContent, FileSourceError>> + Send;
}

/// The capability an adapter uses to wait between hops of a flow whose
/// upstream needs settling time. A capability rather than a runtime call,
/// because this crate takes no runtime by design: the worker binds a real
/// sleep, a test binds an instant return, and the flow reads the same in
/// both. Written as `fn -> impl Future` for the same reason every other
/// seam trait is: `async_fn_in_trait` is a hard error under a deny-warnings
/// build.
pub trait Pause: Send + Sync {
    fn pause(&self, ms: u32) -> impl std::future::Future<Output = ()> + Send;
}

/// A [`Pause`] that returns immediately. Cassette-driven tests replay a
/// recorded poll sequence, and the driver's verification poll is stepped the
/// same way, where a real wait would add nothing but wall-clock time to the
/// gated lane.
#[derive(Debug, Clone, Copy, Default)]
pub struct InstantPause;

impl Pause for InstantPause {
    fn pause(&self, _ms: u32) -> impl std::future::Future<Output = ()> + Send {
        core::future::ready(())
    }
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

#[cfg(test)]
mod tests {
    use super::{settle, FieldDiffReport, InstantPause, Outcome, Pause as _, RemoteListingId};
    use tam_types::{AttemptId, FieldKey, FieldMismatch, MismatchClass, Timestamp, Uuid};

    fn receipt_parts() -> (AttemptId, RemoteListingId, Timestamp) {
        (
            AttemptId(Uuid([7; 16])),
            RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/x-1".to_owned(),
            },
            Timestamp(1),
        )
    }

    #[test]
    fn a_clean_read_back_settles_as_committed() {
        let (attempt, id, at) = receipt_parts();
        let report = FieldDiffReport {
            normaliser_version: 1,
            mismatches: vec![],
        };
        let outcome = settle(attempt, id, at, report);
        assert!(
            matches!(outcome, Outcome::Committed { .. }),
            "an empty diff report is a clean landing"
        );
    }

    #[test]
    fn a_truncated_field_settles_as_degraded() {
        let (attempt, id, at) = receipt_parts();
        let report = FieldDiffReport {
            normaliser_version: 1,
            mismatches: vec![FieldMismatch {
                field: FieldKey::Description,
                class: MismatchClass::Truncated { limit_observed: 80 },
            }],
        };
        let outcome = settle(attempt, id, at, report);
        assert!(
            matches!(outcome, Outcome::Degraded { .. }),
            "a truncated landing must not be recorded as clean"
        );
    }

    #[test]
    fn the_instant_pause_returns_without_waiting() {
        futures::executor::block_on(InstantPause.pause(60_000));
    }
}
