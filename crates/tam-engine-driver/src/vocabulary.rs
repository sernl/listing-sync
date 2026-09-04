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
use tam_domain::{ItemOperation, ItemOutcome, JobItemId, SellerEvent, StepBudget};
use tam_marketplace::{
    CreateStrategy, FormId, IdempotencyKey, ProjectedListing, RemoteLifecycle, RemoteListingId,
};
use tam_types::{
    ConnectionId, ContentHash, FailureCode, FailureDetail, FileId, InventoryId, JobEventPayload,
    JobId, MappingId, Marketplace, OrgId, Uuid,
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

/// The seller's own statement that the work they are listing is theirs, as the
/// wire carries it.
///
/// Optional because it is a fact of one marketplace rather than of every
/// write: TPT's product form makes the seller declare it and Tes's does not,
/// so an order for a Tes item carries none and that absence is not a missing
/// value. The instant is when the seller declared, and it reaches this order
/// off the connection row rather than being minted on the device: it is
/// stamped where the declaration was made, by the broker's link or by the
/// declaration route, so a machine that only carries the write never dates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    pub attested_by: String,
    pub attested_at_ms: i64,
}

/// One bound listing a capture reads metrics for: the mapping it belongs to
/// and the remote listing that names it on the marketplace.
///
/// Here rather than in `tam-storage` because the capture that consumes it now
/// runs on the seller's own device, and the interpreter's vocabulary is the
/// one crate both ends already share. The storage repository that produces
/// these converts at its own boundary, the way every other wire type on this
/// surface is converted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundListing {
    pub mapping: MappingId,
    pub remote: RemoteListingId,
}

/// One metric reading, as the device reports it back.
///
/// `metric` is an open string rather than a closed set, because the three
/// captured today are a shortlist off the twelve the adapter names and
/// widening it must not need a migration or a redeploy of the read side --
/// which is the same reason the column carries no CHECK.
///
/// `total_value` is the untyped double the adapter read: a count for sales, an
/// amount for earnings, a ratio for the Easel rates. Typing it here would
/// decide per metric what the read deliberately does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricSnapshot {
    pub mapping: MappingId,
    pub metric: String,
    pub observed_at: i64,
    pub total_value: f64,
}

/// The stranded create a reconcile is settling, and what identifies it.
///
/// The title is the one that create recorded that it sent, read from its own
/// `write_attempt` intent, rather than the title the product carries now.
/// The two differ whenever the seller edited the listing between the strand
/// and the resume, and searching a catalogue for the current title is not
/// merely how a search misses: it is how it matches some other listing that
/// has since acquired that title and binds the mapping to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileSubject {
    pub attempt: AttemptRef,
    pub title: String,
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
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
pub enum BudgetGrant {
    Granted { used: i32 },
    Exhausted,
}

/// Which allowance a grant is drawn against. The driver states what it is
/// about to do; the server decides what that costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    /// A lifecycle write: a create, a revise or a removal.
    Write,
    /// One try of a verification read-back.
    VerifyRead,
    /// One scrape of a marketplace's own create or edit form, which the
    /// preflight makes before any write.
    ///
    /// Its own kind rather than either of the others because it is neither:
    /// charging a scrape against the write allowance would make that
    /// allowance wrong about how many listings a seller may touch, and
    /// charging it as a verification read would report a read of a listing
    /// that does not exist yet. Both existing kinds draw on the same window
    /// today, so this changes what is reported before it changes what is
    /// permitted.
    FormRead,
}

/// The conditions the interpreter branches on, and nothing else.
///
/// `StorageError` cannot cross this boundary: its database variant is
/// `#[from] sqlx::Error`, which is the dependency this crate exists to refuse.
/// The two named variants are the two the driver actually tests for; every
/// other server-side condition arrives opaque, because the interpreter's only
/// answer to one is the stall bias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerError {
    /// The mapping already has an open attempt, which is the duplicate-create
    /// fence holding.
    AttemptInFlight,
    /// A create was refused because the mapping is already bound: the listing
    /// it would have made exists, made by another run.
    ///
    /// Distinct from [`Self::AttemptInFlight`] because the interpreter must
    /// act differently. An attempt in flight may still clear, so the run
    /// abandons and the item is offered again; a bound mapping is permanent
    /// for this item, so the run settles it rather than handing back work no
    /// later lease could do either.
    MappingAlreadyBound,
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
            Self::MappingAlreadyBound => {
                f.write_str("that mapping is already bound, so this create has nothing to make")
            }
            Self::StaleLease => f.write_str("the lease epoch is stale"),
            Self::Refused { detail } => write!(f, "the ledger refused the write: {detail}"),
        }
    }
}

impl core::error::Error for LedgerError {}

/// What a transfer of one file is checked against.
///
/// The pair travels together because the check is one check: the length is
/// compared first so a truncated transfer of a large file is named as one
/// rather than as an unexplained digest mismatch, and the hash is what
/// actually decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Committed {
    /// The blake3 content hash of the bytes.
    pub hash: ContentHash,
    /// How many bytes, checked before the hash so a truncated transfer of a
    /// large file is named as a truncation rather than as an unexplained
    /// mismatch.
    pub byte_len: i64,
}

/// Where the bytes an upload needs come from, and what the device may check
/// them against on arrival.
///
/// The two arms differ in who committed to the bytes, which is the whole of
/// what integrity means here. Under `ControlPlane` the server states the hash
/// and the length before the transfer begins, so what arrives is proved
/// against a commitment made in advance; that is why the payload route
/// deliberately carries no digest of its own, a response restating its own
/// digest proving nothing.
///
/// Under `Marketplace` the bytes are the seller's, held by the marketplace,
/// and the device fetches them under the seller's own session because D27
/// puts file ingest there: if the upload is itself a marketplace request that
/// must originate on the seller's machine, the bytes must be on that machine
/// at upload time. The server holds no copy and so can commit to nothing on a
/// first observation, which `expected: None` states rather than hides. Where a
/// previous observation exists it travels as `expected`, and what it proves is
/// narrower than the `ControlPlane` case and worth naming: it catches a
/// changed or truncated re-fetch and makes a retry idempotent, but it cannot
/// prove the bytes are the seller's original, because a wrong first fetch is
/// what every later one would agree with.
/// Externally tagged and accepting unknown fields, which is the convention
/// every nested value enum in this module follows — `LandingEffect`,
/// `BindDisposition`, `BudgetGrant`, `GrantKind`, `LedgerError`. The
/// internally tagged, `deny_unknown_fields` shape belongs to the enums that
/// dispatch a device-to-server call, where refusing a field nobody recognises
/// is the server declining to guess what a client meant. This travels the
/// other way, server to device, and a new field added here must be ignorable
/// by an older client rather than fatal to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadSource {
    ControlPlane {
        committed: Committed,
    },
    Marketplace {
        marketplace: Marketplace,
        /// How that marketplace addresses the resource holding the bytes, in
        /// the spelling its own adapter takes.
        resource: String,
        /// Which file inside the resource, where the marketplace hands over a
        /// bundle rather than a file. `None` is the bundle whole, which is
        /// what a migration sends, because the target's product slot takes one
        /// file and the bundle is what the source's buyers already receive.
        entry: Option<String>,
        expected: Option<Committed>,
    },
}

/// One file the operation uploads, as the server describes it before the bytes
/// move.
///
/// The name and the content type are what the upload form carries, derived
/// from the file's kind by the same rule the server's own file source uses.
/// What the bytes are checked against, and whether they can be checked at all,
/// is [`PayloadSource`]'s to say.
///
/// Deserialise is derived and Serialize is not, and the asymmetry is a
/// deprecation shim rather than a style. Desktop 0.1.3 is published and
/// auto-updating, and its own copy of this struct declares `hash` and
/// `byte_len` as required top-level fields with no serde attributes. A server
/// emitting only `source` would make every claimed item carrying a file fail
/// to decode there — and fail after the claim, so the item sits leased until
/// its lease expires while the device reports a failure every poll. So the
/// serialisation below emits the old pair beside `source`, derived from the
/// source rather than stored, which is why they cannot drift out of step with
/// it. Reading tolerates both shapes because the derived `Deserialize`
/// ignores fields it does not know.
///
/// The window ends when every registered device reports an `app_version` at or
/// past this change; `device` rows carry it at registration and at every
/// check-in. What must not happen before then is a marketplace-sourced
/// manifest reaching an old client, and see [`PayloadManifest::serialize`] for
/// what that case deliberately does.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PayloadManifest {
    pub file: FileId,
    pub file_name: String,
    pub content_type: String,
    pub source: PayloadSource,
}

impl Serialize for PayloadManifest {
    /// Emits the current shape, plus the pre-`source` pair where there is one
    /// to emit.
    ///
    /// A control-plane source has a commitment, so the old fields are exactly
    /// that commitment and an old client behaves as it always did.
    ///
    /// A marketplace source has no commitment to restate, and emits neither
    /// field, so an old client fails to decode the envelope. That is chosen
    /// rather than fallen into. No value would let an old client succeed: it
    /// has no marketplace fetcher, and our object store holds no bytes for
    /// that file, so a synthesised hash would only send it to a payload route
    /// that answers 404 — failing later, after a round trip, in a shape that
    /// reads as our server being broken rather than as a client being too old.
    /// Failing at the envelope is louder and truer. The cost is real and is
    /// the reason the window has an end condition rather than a hope: the
    /// item stays leased until its lease expires. Nothing emits a marketplace
    /// source yet, and nothing may until every device has updated.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let legacy = match &self.source {
            PayloadSource::ControlPlane { committed } => Some(committed),
            PayloadSource::Marketplace { .. } => None,
        };
        let fields = if legacy.is_some() { 6 } else { 4 };
        let mut out = serializer.serialize_struct("PayloadManifest", fields)?;
        out.serialize_field("file", &self.file)?;
        out.serialize_field("file_name", &self.file_name)?;
        out.serialize_field("content_type", &self.content_type)?;
        out.serialize_field("source", &self.source)?;
        if let Some(committed) = legacy {
            out.serialize_field("hash", &committed.hash)?;
            out.serialize_field("byte_len", &committed.byte_len)?;
        }
        out.end()
    }
}

impl PayloadManifest {
    /// What a transfer of this file may be checked against, where anything
    /// may.
    ///
    /// `None` is the one case with no commitment at all — a marketplace source
    /// nobody has observed yet — and it is returned rather than substituted
    /// for, because a caller that verified against a value it invented would
    /// be asserting a guarantee this manifest does not carry.
    #[must_use]
    pub const fn committed(&self) -> Option<&Committed> {
        match &self.source {
            PayloadSource::ControlPlane { committed } => Some(committed),
            PayloadSource::Marketplace { expected, .. } => expected.as_ref(),
        }
    }
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
    /// Present only on a reconcile, and its presence is what makes the order
    /// one: the device seeds the interpreter to search for this attempt's
    /// listing rather than to create another.
    ///
    /// The device cannot derive it — a stranded create is still
    /// `operation = 'create'`, which is exactly why its fence is what it is —
    /// and the server knows it at claim time, so it travels here.
    /// `default` so a client built before this field decodes an order without
    /// one as the ordinary order it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconcile: Option<ReconcileSubject>,
    /// The attestation this write goes out under, where the marketplace
    /// requires one.
    ///
    /// It travels rather than being read on the device because it is the
    /// seller's declaration held against their connection, and the device
    /// holds no connection row. Absent for a marketplace that asks for none;
    /// absent for one that does is a connection the seller has not attested
    /// on, and the adapter refuses rather than supplying a constant on their
    /// behalf. `default` so an order encoded before this field decodes as one
    /// carrying no attestation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<Attestation>,
    pub server_now_ms: i64,
    pub server_deadline_ms: i64,
    pub next_poll_ms: u64,
}

/// What the device is asking for when it pulls.
///
/// The device gates readiness per marketplace before it pulls — a session
/// present, entitlement standing, no halt — so it asks for work it has already
/// decided it can do. Absent means whatever is due, which is what the
/// in-process worker wants; present means a mismatched order never reaches a
/// device that would only refuse it and spend a lease expiry doing so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkFilter {
    #[serde(default)]
    pub marketplace: Option<Marketplace>,
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
    /// The device's asserted instant, recorded as the seller's assertion
    /// beside the server's own receipt. `org_seq` keeps ordering
    /// server-authoritative, so this is evidence rather than authority.
    pub at_ms: i64,
}

/// One call the device makes against the ledger, other than the settle.
///
/// Every variant carries the [`LeaseRef`] it speaks for, because the server
/// derives the organisation, the connection, the inventory and the epoch from
/// the lease it issued rather than from anything the caller says. The device
/// names what it wants done; it never names whose work it is.
///
/// `at_ms` is the device's *asserted* instant, on the variants whose ledger
/// method takes one. It is the seller's clock and is recorded as their
/// assertion, never mistaken for our record: the server stamps its own receipt
/// beside it, and `org_seq` keeps ordering server-authoritative regardless.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub enum LedgerCall {
    ConnectionFor {
        lease: LeaseRef,
        inventory: InventoryId,
    },
    PreflightSucceeded {
        lease: LeaseRef,
    },
    PreflightFailed {
        lease: LeaseRef,
        edge_class: bool,
    },
    /// No duration: the reaper reads `park_expires_at` against the server's
    /// clock, and the machine parks every gate for the same span, so the
    /// server mints the when from its own constant rather than taking a
    /// number from the party the park is holding.
    Park {
        lease: LeaseRef,
        blocked_on: String,
    },
    OpenAttempt {
        lease: LeaseRef,
        new: NewAttempt,
        at_ms: i64,
    },
    SettleAttempt {
        lease: LeaseRef,
        attempt: AttemptRef,
        verdict: AttemptVerdict,
        at_ms: i64,
    },
    GateConnection {
        lease: LeaseRef,
        inventory: InventoryId,
        at_ms: i64,
    },
    /// The only halt reachable from a device, fixed to this tenant's
    /// inventory: there is no wider scope to name.
    HaltThisTenant {
        lease: LeaseRef,
        inventory: InventoryId,
        reason: String,
        at_ms: i64,
    },
    /// Neither the window nor the ceiling appears: both are the server's.
    RequestGrant {
        lease: LeaseRef,
        connection: ConnectionId,
        kind: GrantKind,
        at_ms: i64,
    },
    /// The heartbeat. No duration: the server mints the new expiry from its
    /// own constant, so a device cannot buy itself a lease of any length by
    /// naming one. No asserted instant either: it writes no dated row, and an
    /// instant nothing records is a field that only looks like evidence.
    Renew {
        lease: LeaseRef,
    },
    RecordEvent {
        lease: LeaseRef,
        payload: JobEventPayload,
        at_ms: i64,
    },
    /// The closed event, never a topic string, which would otherwise name any
    /// relay the drainer knows.
    Notify {
        lease: LeaseRef,
        event: SellerEvent,
        at_ms: i64,
    },
}

impl LedgerCall {
    /// The lease this call speaks for, which is what the endpoint fences on
    /// before it dispatches anything.
    #[must_use]
    pub const fn lease(&self) -> &LeaseRef {
        match self {
            Self::ConnectionFor { lease, .. }
            | Self::PreflightSucceeded { lease }
            | Self::PreflightFailed { lease, .. }
            | Self::Park { lease, .. }
            | Self::OpenAttempt { lease, .. }
            | Self::SettleAttempt { lease, .. }
            | Self::GateConnection { lease, .. }
            | Self::HaltThisTenant { lease, .. }
            | Self::RequestGrant { lease, .. }
            | Self::Renew { lease, .. }
            | Self::RecordEvent { lease, .. }
            | Self::Notify { lease, .. } => lease,
        }
    }

    /// The device's asserted instant, where the call carries one.
    #[must_use]
    pub const fn asserted_at_ms(&self) -> Option<i64> {
        match self {
            Self::ConnectionFor { .. }
            | Self::PreflightSucceeded { .. }
            | Self::PreflightFailed { .. }
            | Self::Park { .. }
            | Self::Renew { .. } => None,
            Self::OpenAttempt { at_ms, .. }
            | Self::SettleAttempt { at_ms, .. }
            | Self::GateConnection { at_ms, .. }
            | Self::HaltThisTenant { at_ms, .. }
            | Self::RequestGrant { at_ms, .. }
            | Self::RecordEvent { at_ms, .. }
            | Self::Notify { at_ms, .. } => Some(*at_ms),
        }
    }
}

/// A renewed lease, stated in the server's own numbers.
///
/// Both instants travel because the device needs a duration and has no clock
/// we trust: `server_deadline_ms - server_now_ms` is the remaining lease, and
/// every term in it is ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renewed {
    pub server_now_ms: i64,
    pub server_deadline_ms: i64,
}

/// What the ledger answers.
///
/// `Refused` carries a [`LedgerError`] as an answer rather than as a transport
/// failure, and that distinction is the point. `AttemptInFlight` is the
/// duplicate-create fence holding, `StaleLease` is another holder owning the
/// item now, and `MappingAlreadyBound` is a create whose listing another run
/// already made; the interpreter branches on all three, abandoning on the
/// first two and settling on the third. A device that saw them as HTTP faults
/// would retry the conditions it must not retry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum LedgerAnswer {
    /// The call had nothing to return but did it.
    Done,
    Connection {
        connection: Option<ConnectionId>,
    },
    Streak {
        streak: PreflightStreak,
    },
    Bound {
        disposition: BindDisposition,
    },
    Renewed {
        renewed: Renewed,
    },
    Grant {
        grant: BudgetGrant,
    },
    Refused {
        error: LedgerError,
    },
}
