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
pub mod transport;

use tam_types::{
    AttemptId, ConnectionId, ContentHash, FailureCode, FailureDetail, FieldKey, FieldMismatch,
    FileId, InventoryId, LogicalInstant, Marketplace, MismatchClass, OrgId, Timestamp, Uuid,
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
    SchemaDrift(SchemaDrift),
    RateLimited {
        retry_after: Option<DurationSecs>,
    },
    /// The request provably never left. The only class that is safe to retry.
    NotSent(ConnectFailure),
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
    use super::{settle, FieldDiffReport, Outcome, RemoteListingId};
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
}
