//! The effect interpreter: pumps one leased item through the machine,
//! executing each effect in order and feeding the result back as the next
//! input. Every ledger write carries the lease epoch; time enters as data
//! through the clock seam; and every unhandleable condition abandons the
//! item — the stall bias, never an invented outcome.
//!
//! An abandoned run hands the lease back rather than leaving it to expire.
//! The expiry is the backstop for a device that stopped existing, and it was
//! being used as the ordinary path: the live-lease mutex is per marketplace,
//! so an item nobody was working still held its seller's whole queue for the
//! rest of its TTL, and the work resumed one poll after the reaper requeued
//! it. What the run knows at the moment it stops is exactly what the server
//! needs to decide where the item goes, so it says so instead.

use serde_json::json;
use tam_domain::{
    attempt_budget_spent, seller_clears, verification_settles, BlockCause, CaptureCause, Effect,
    Input, ItemOperation, ItemOutcome, MachineError, SellerEvent, StepBudget, SyncMachine,
    SyncState, Transition,
};
use tam_marketplace::{
    AdapterError, AmbiguityCause, ChallengeKind, CreateStrategy, FetchReason, FieldSet, FormId,
    ListingLocator, ListingState, MarketplaceAdapter, ObservedListing, Outcome, Pause,
    RecordedTitle, RemoteLifecycle, RemoteListingId, RemovalPlan, RevisePlan, WriteAttemptId,
};
use tam_types::{
    BindAnomaly, ConnectionId, ContentHash, FailureCode, FailureDetail, JobEventPayload,
    LogicalInstant, Timestamp,
};

use crate::ports::{Cancellation, IdSource, ItemLedger, ReconcileSource};
use crate::vocabulary::{
    AttemptIntent, AttemptRef, AttemptVerdict, Attestation, BindDisposition, BudgetGrant,
    GrantKind, ItemVerdict, LandingEffect, LeaseRef, LeasedItem, LedgerError, NewAttempt,
};

/// The clock seam: the binaries read the wall clock at this boundary
/// (expect-attributed there); tests hand instants in.
pub trait NowSource: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// How long the driver polls a marketplace's own read for a write it just
/// made. Held here rather than in the machine, which is pure and holds no
/// clock, and produced per inventory by the seed.
///
/// The heartbeat renews the lease before every network-bearing effect and
/// before every verification try, so the requirement is not that the whole run
/// fits inside one lease but that no single uninterrupted stretch between two
/// renews outlives it:
///
/// ```text
/// slowest_single_effect + interval_ms < LEASE_TTL_SECS
/// ```
///
/// That holds for the measured Tpt create — roughly 180s against a 300s
/// lease, with 22s of poll on top — and does not hold at Tpt's theoretical
/// worst case, where two queue-job polls inside one submit can spend 360s with
/// no renew between them. The heartbeat narrows that hazard without closing
/// it; `lease_budget_tests` in `tam-engine` asserts both facts rather than the
/// comfortable one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerifyPolicy {
    pub tries: u32,
    pub interval_ms: u32,
}

impl VerifyPolicy {
    /// The poll's whole wall time, in milliseconds.
    #[must_use]
    pub fn window_ms(self) -> u64 {
        u64::from(self.tries) * u64::from(self.interval_ms)
    }
}

/// The machine's construction data the ledger does not hold: the projected
/// field set and its hash, the form, the per-inventory strategy and the
/// verification budget. The projection (M1g) and the scheduler wiring (M1j)
/// will own producing this; until then the worker builds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSeed {
    /// The stranded attempt this run is reconciling and the title it
    /// recorded, where it is one.
    ///
    /// Set, the run does not begin at the beginning: it steps straight to the
    /// search, and the entry row's effects are discarded rather than
    /// executed. That matters because the first of them is the form
    /// assertion, which on Tes is a write.
    pub resume: Option<(WriteAttemptId, RecordedTitle)>,
    /// The attestation this run's writes go out under, where the marketplace
    /// requires one.
    ///
    /// Recorded in the attempt's intent body and deliberately not in the
    /// intent hash: the hash feeds the idempotency key, so a re-attested
    /// create whose fields are unchanged must stay the same create rather
    /// than becoming a second listing.
    pub attestation: Option<Attestation>,
    pub form: FormId,
    pub fields: FieldSet,
    pub intent_hash: ContentHash,
    pub strategy: CreateStrategy,
    pub budget: StepBudget,
    pub verify: VerifyPolicy,
}

pub struct DriverContext<'a, A, N, P, L, C, I, R> {
    pub adapter: &'a A,
    /// The seller's own catalogue, searched when a create's fate is unknown.
    /// A port rather than a method on the adapter, because the enumeration
    /// runs under the seller's session on the seller's machine.
    pub reconcile: &'a R,
    /// The one ledger seam. Four repositories and two raw-pool writers stood
    /// here; collapsing them is what lets the whole surface be keyed on the
    /// lease the server issued.
    pub ledger: &'a L,
    pub clock: &'a N,
    pub ids: &'a I,
    pub cancel: &'a C,
    /// The wait between verification reads. A capability rather than a
    /// runtime call, because this crate takes no timer by design: the worker
    /// binds a real sleep and a test binds an instant return, so the poll is
    /// exercised at zero wall time in the gated lane.
    pub pause: &'a P,
}

/// What one pump of one item came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunVerdict {
    Settled(ItemOutcome),
    Parked,
    /// The run stopped without deciding the item. The lease is handed back —
    /// see [`ItemLedger::hand_back`] — so the server disposes of the item at
    /// once rather than a TTL later. The stall bias as a value: the run
    /// invents no outcome.
    Abandoned {
        reason: String,
    },
}

#[derive(Debug)]
pub enum EngineError {
    Ledger(LedgerError),
    Machine(MachineError),
    /// The adapter refused to render the projection into its own wire
    /// shape: the item is unrepresentable on this marketplace, so its lease
    /// is left to expire rather than settled.
    Projection(AdapterError),
}

impl From<LedgerError> for EngineError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger(error)
    }
}

impl From<MachineError> for EngineError {
    fn from(error: MachineError) -> Self {
        Self::Machine(error)
    }
}

impl From<AdapterError> for EngineError {
    fn from(error: AdapterError) -> Self {
        Self::Projection(error)
    }
}

impl core::fmt::Display for EngineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ledger(error) => write!(f, "ledger: {error}"),
            Self::Machine(error) => write!(f, "machine: {error:?}"),
            Self::Projection(error) => write!(f, "projection: {error:?}"),
        }
    }
}

impl core::error::Error for EngineError {}

const fn state_name(state: ListingState) -> &'static str {
    match state {
        ListingState::Draft => "draft",
        ListingState::Live => "live",
    }
}

fn subject_as_json(subject: &RemoteListingId) -> serde_json::Value {
    match subject {
        RemoteListingId::Tes { url } => json!({ "kind": "tes", "url": url }),
        RemoteListingId::Tpt { product_id } => json!({ "kind": "tpt", "id": product_id }),
        RemoteListingId::Etsy { listing_id } => json!({ "kind": "etsy", "id": listing_id }),
    }
}

/// What the attempt row records it intended. A create records the field set
/// it will post, as it always has, now tagged with the operation; a revise
/// adds the listing it addresses and both ends of the transition; a removal
/// carries no field set at all, because a removal describes nothing.
/// The recorded intent, with the attestation the write goes out under.
///
/// The attestation joins the body and deliberately not the hash.
/// `intent_hash` is computed from `intent_as_json` alone in the seed and feeds
/// the idempotency key, so folding an attestation into it would make a
/// re-attested create a different create — a second listing for the same
/// product, which is the one failure that key exists to prevent. The body is
/// the durable record of what a write went out under; the hash is the
/// identity of what was written, and only one of them is about who attested.
fn attested_intent(
    operation: &ItemOperation,
    fields: &FieldSet,
    attestation: Option<&Attestation>,
) -> serde_json::Value {
    let mut body = intent_as_json(operation, fields);
    let (Some(attestation), Some(object)) = (attestation, body.as_object_mut()) else {
        return body;
    };
    object.insert(
        "attested_by".to_owned(),
        serde_json::Value::String(attestation.attested_by.clone()),
    );
    object.insert(
        "attested_at_ms".to_owned(),
        serde_json::Value::from(attestation.attested_at_ms),
    );
    body
}

pub fn intent_as_json(operation: &ItemOperation, fields: &FieldSet) -> serde_json::Value {
    let entries = serde_json::to_value(&fields.entries).unwrap_or(serde_json::Value::Null);
    let files = serde_json::to_value(&fields.files).unwrap_or(serde_json::Value::Null);
    match operation {
        ItemOperation::Create => json!({
            "operation": "create",
            "entries": entries,
            "files": files,
        }),
        ItemOperation::Revise {
            subject,
            transition,
        } => json!({
            "operation": "revise",
            "subject": subject_as_json(subject),
            "from": state_name(transition.from),
            "to": state_name(transition.to),
            "entries": entries,
            "files": files,
        }),
        // The subject is deliberately absent: a publish that reaches the
        // driver has been lowered to a revise, so this arm is the record of
        // an unlowered one rather than an intent anything will act on.
        ItemOperation::Publish { to } => json!({
            "operation": "publish",
            "to": state_name(*to),
            "entries": entries,
            "files": files,
        }),
        ItemOperation::Remove { subject, state } => json!({
            "operation": "remove",
            "subject": subject_as_json(subject),
            "state": state_name(*state),
        }),
    }
}

/// How a blocking condition reads to the seller, split by whose it is.
///
/// The split is [`seller_clears`]', the same one the machine parks on, so the
/// row's copy and the item's fate cannot say different things. A lapsed
/// session or a mailed one-time password is theirs and a re-link supplies it.
/// A captcha or an interstitial is the edge answering our address rather than
/// our credential, which no re-link touches — so the copy says what is true,
/// that the retry is ours. Reporting the second as `SessionExpired` is how a
/// connections page ends up telling a seller to re-link over an egress block.
fn challenge_verdict(challenge: ChallengeKind) -> (FailureCode, tam_types::FailureDetail) {
    if seller_clears(challenge) {
        return (
            FailureCode::SessionExpired,
            tam_types::FailureDetail(
                "this marketplace connection needs re-linking: it is asking us to sign in \
                 again"
                    .to_owned(),
            ),
        );
    }
    (
        FailureCode::ChallengePresented,
        tam_types::FailureDetail(
            "the marketplace is challenging our requests; retrying is ours to do, not yours"
                .to_owned(),
        ),
    )
}

/// What a halted ambiguity tells the seller and the operator.
///
/// `FailureCode::Other` because the item's own `outcome` column already says
/// `ambiguous` and no code in the closed set names one of the five causes;
/// the cause travels in the detail, where it is the only thing on the item
/// row that says which ambiguity this was. Before this, an ambiguous item
/// settled with both columns NULL, so the halted tenant's own record said
/// nothing at all about why.
fn ambiguity_verdict(cause: AmbiguityCause) -> (FailureCode, FailureDetail) {
    (
        FailureCode::Other,
        FailureDetail(format!(
            "this write's fate is unknown ({}); an operator settles it rather than a retry",
            cause.name()
        )),
    )
}

fn outcome_to_item(outcome: &Outcome) -> ItemVerdict {
    let (item_outcome, failure_code, failure_detail) = match outcome {
        Outcome::Committed { .. } => (ItemOutcome::Succeeded, None, None),
        Outcome::Degraded { .. } => (ItemOutcome::Degraded, None, None),
        Outcome::Rejected { code, detail } => {
            (ItemOutcome::Failed, Some(*code), Some(detail.clone()))
        }
        Outcome::Ambiguous { cause, .. } => {
            let (code, detail) = ambiguity_verdict(*cause);
            (ItemOutcome::Ambiguous, Some(code), Some(detail))
        }
        Outcome::Blocked { challenge } => {
            let (code, detail) = challenge_verdict(*challenge);
            (ItemOutcome::Blocked, Some(code), Some(detail))
        }
        Outcome::Skipped { code } => (ItemOutcome::Skipped, Some(*code), None),
    };
    ItemVerdict {
        outcome: item_outcome,
        failure_code,
        failure_detail,
    }
}

/// What a settled outcome did to the mapping, as a total function of the
/// operation. A committed removal severs rather than binding, because binding
/// would record the mapping against a listing that no longer exists; a revise
/// or removal that did not commit still names what it addressed, so a failed
/// removal says what it failed to remove; and a create that did not commit
/// knows no identifier at all.
///
/// The bound lifecycle is the one the verification read observed, not one
/// derived from the adapter's create convention: `Outcome::Committed` is
/// reachable only through a read-back, so an observation is always in hand,
/// and where it somehow is not the honest answer is to record nothing rather
/// than to invent a state the mapping was never seen in.
fn outcome_to_landing(
    operation: &ItemOperation,
    outcome: &Outcome,
    observed: Option<&RemoteLifecycle>,
) -> LandingEffect {
    let committed = match outcome {
        Outcome::Committed { receipt, .. } | Outcome::Degraded { receipt, .. } => {
            Some(receipt.listing())
        }
        Outcome::Rejected { .. }
        | Outcome::Ambiguous { .. }
        | Outcome::Blocked { .. }
        | Outcome::Skipped { .. } => None,
    };
    match (operation, committed) {
        (ItemOperation::Remove { .. }, Some(id)) => LandingEffect::Severed { id: id.clone() },
        (
            ItemOperation::Create | ItemOperation::Publish { .. } | ItemOperation::Revise { .. },
            Some(id),
        ) => observed.map_or(LandingEffect::None, |lifecycle| LandingEffect::Landed {
            id: id.clone(),
            lifecycle: lifecycle.clone(),
        }),
        (ItemOperation::Create | ItemOperation::Publish { .. }, None) => LandingEffect::None,
        (ItemOperation::Revise { subject, .. } | ItemOperation::Remove { subject, .. }, None) => {
            LandingEffect::Addressed {
                id: subject.clone(),
            }
        }
    }
}

/// What one verification poll is asked to establish. A struct rather than six
/// arguments, and it is also what keeps the locator, the reason and the
/// operation's predicate travelling together.
struct Verification<'a> {
    policy: VerifyPolicy,
    locator: ListingLocator,
    reason: FetchReason,
    operation: &'a ItemOperation,
    connection: ConnectionId,
    deadline: i64,
}

/// Two answers, because evidence about the listing and evidence about us
/// settle differently.
enum VerifyOutcome {
    /// The predicate accepted, the try budget ran out, or the read returned
    /// the one condition `AwaitingReadBack` has an arm for. Either way this
    /// is evidence about the listing, and the machine settles on it.
    Observed(Result<ObservedListing, AdapterError>),
    /// The poll stopped without answering the predicate. The write may have
    /// landed; we simply stopped looking.
    Stopped(VerifyStop),
}

/// Why a poll stopped with nothing to say about the listing.
///
/// `ReadCondition` is the reason this is an enum rather than a flag.
/// `awaiting_read_back_rows` answers `Ok(..)` and `Err(Ambiguous(..))` and
/// calls every other `AdapterError` `InputNotApplicable`, so stepping one
/// would return `Err` out of `run_item` with the write attempt still
/// `in_flight` — the same stranded row the rate-window path exists to avoid,
/// reached from the other side. An expired session, a 429 or a dropped
/// connection mid-poll is enough to produce it.
enum VerifyStop {
    /// The per-connection rate window closed before the predicate could be
    /// answered.
    RateWindowClosed,
    /// The wall clock or the cancellation token ended the run.
    Cut,
    /// The read answered a condition rather than an observation. Absence is
    /// an observation precisely so that lag is not one, which leaves these
    /// as facts about us rather than about the listing.
    ReadCondition(AdapterError),
}

impl VerifyStop {
    fn reason(&self) -> String {
        match self {
            Self::RateWindowClosed => "the per-connection rate window closed".to_owned(),
            Self::Cut => "the run was cut".to_owned(),
            Self::ReadCondition(error) => format!("the verification read answered {error:?}"),
        }
    }
}

/// Polls the marketplace's own read until the operation's predicate accepts,
/// the budget runs out, or something other than lag stops us.
///
/// Exhaustion is *not* reported as a failure of the read: the last answer
/// goes to the machine, so a create the poll never saw settles the halting
/// ambiguity rather than a silent commit. Exhaustion of the *rate window*
/// is different in kind and never reads as absence, because at one submit
/// plus eleven reads against a thirty-per-minute ceiling the third item on
/// one connection inside one minute would otherwise halt the tenant's
/// inventory on pure throughput.
///
/// The deadline and the cancellation token are checked between tries, which
/// leaves up to one `interval_ms` of uninterruptible pause after a ctrl-c —
/// two seconds, stated rather than implied. Neither is answered by stepping
/// `Input::BudgetExhausted`, which `SyncMachine::step` intercepts ahead of
/// the state dispatch: from `AwaitingReadBack` that would settle the item
/// `Ambiguous`, but this same effect is raised again from
/// `SyncState::Terminal` after a reconciled settle, and `exhaust_budget`
/// answers a terminal machine `InputNotApplicable`. Walking away covers both
/// callers with one rule.
async fn verify_with_backoff<
    A: MarketplaceAdapter,
    N: NowSource,
    P: Pause,
    L: ItemLedger,
    C: Cancellation,
    I: IdSource,
    R: ReconcileSource,
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I, R>,
    lease: &LeasedItem,
    request: Verification<'_>,
) -> Result<VerifyOutcome, EngineError> {
    let mut last: Option<ObservedListing> = None;
    for attempted in 0..request.policy.tries {
        let now = ctx.clock.now();
        if ctx.cancel.is_cancelled() || now.0 >= request.deadline {
            return Ok(VerifyOutcome::Stopped(VerifyStop::Cut));
        }
        // One read-back can expand into eleven marketplace reads, which is the
        // longest stretch under one lease, so each try extends it.
        ctx.ledger.renew(&lease.lease_ref()).await?;
        let grant = ctx
            .ledger
            .request_grant(
                &lease.lease_ref(),
                request.connection,
                GrantKind::VerifyRead,
                now,
            )
            .await?;
        if grant == BudgetGrant::Exhausted {
            return Ok(VerifyOutcome::Stopped(VerifyStop::RateWindowClosed));
        }
        let observed = ctx
            .adapter
            .read_back(request.locator.clone(), request.reason.clone(), now)
            .await;
        match observed {
            // Every `AdapterError` is a condition rather than lag — absence
            // became an observation precisely so that lag is not one — so
            // polling through one would only spend the budget. Which way out
            // it takes is decided by the machine's arm set rather than by
            // the condition's severity: `Ambiguous` is the one
            // `AwaitingReadBack` accepts, and every other one stops the poll
            // instead of being stepped into `InputNotApplicable`.
            Err(AdapterError::Ambiguous(cause)) => {
                return Ok(VerifyOutcome::Observed(Err(AdapterError::Ambiguous(cause))))
            }
            Err(error) => return Ok(VerifyOutcome::Stopped(VerifyStop::ReadCondition(error))),
            Ok(listing) if verification_settles(request.operation, &listing) => {
                return Ok(VerifyOutcome::Observed(Ok(listing)))
            }
            Ok(listing) => last = Some(listing),
        }
        if attempted + 1 < request.policy.tries {
            ctx.pause.pause(request.policy.interval_ms).await;
        }
    }
    // A policy of zero tries never looked, which is the one case with no
    // answer to hand over; the stall bias reports it as a cut run.
    Ok(
        last.map_or(VerifyOutcome::Stopped(VerifyStop::Cut), |listing| {
            VerifyOutcome::Observed(Ok(listing))
        }),
    )
}

/// How long a stranded create waits, inside its own lease, for the
/// marketplace to publish what it did with the write, before the seller's
/// catalogue is walked looking for it.
///
/// A wait rather than a re-claim, and the difference is the whole of what the
/// heartbeat is for. The wait used to be spent by ending the run: the item
/// kept its lease until the TTL expired, the reaper parked it, and the next
/// claim reconciled it. So one stranded write cost the seller a full lease
/// TTL — and cost every sibling item on that marketplace the same, because
/// the live-lease mutex is per marketplace and an item nobody was working
/// still held it. Waited for here, the cost is the marketplace's own lag and
/// nothing else.
pub const STRAND_SETTLE_WAIT_MS: u32 = 30_000;

/// How many catalogue walks one reconcile is worth, and why absence needs
/// more than one of them.
///
/// `Ok(None)` is a complete enumeration that did not contain the listing, and
/// the machine settles an ambiguity and halts the tenant's inventory on it.
/// That is the right answer to absence and the wrong one to lag — the same
/// distinction [`verify_with_backoff`] exists for. A resource the marketplace
/// has accepted but not yet indexed reads exactly like one it never made, so
/// an absence seen before the last walk is treated as lag and the walk is
/// repeated; only the last answer is handed to the machine.
pub const RECONCILE_WALKS_MAX: u32 = 3;

/// The wait between two walks of one reconcile.
pub const RECONCILE_WALK_INTERVAL_MS: u32 = 30_000;

/// The longest stretch of either wait taken without looking at the
/// cancellation.
///
/// The interpreter's interruption points are between effects, so a wait taken
/// whole is a wait a closing application or a revoked device sits through. Two
/// seconds is the granularity the verification poll already has — its interval
/// is the same order — so the waits added here are no coarser than the ones
/// that were already here.
const WAIT_SLICE_MS: u32 = 2_000;

/// Waits, in slices, and answers whether the run may still continue.
///
/// `false` is a cancellation or the wall-clock deadline arriving inside the
/// wait. The caller stops rather than carrying on: the point of waiting was to
/// give the marketplace time, and a run that has been told to stop is not
/// going to use it.
async fn waited_through<
    A: MarketplaceAdapter,
    N: NowSource,
    P: Pause,
    L: ItemLedger,
    C: Cancellation,
    I: IdSource,
    R: ReconcileSource,
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I, R>,
    total_ms: u32,
    deadline: i64,
) -> bool {
    let mut waited = 0;
    while waited < total_ms {
        if ctx.cancel.is_cancelled() || ctx.clock.now().0 >= deadline {
            return false;
        }
        let slice = WAIT_SLICE_MS.min(total_ms - waited);
        ctx.pause.pause(slice).await;
        waited += slice;
    }
    !ctx.cancel.is_cancelled() && ctx.clock.now().0 < deadline
}

/// What one reconcile poll is asked to establish.
struct Reconciliation {
    locator: ListingLocator,
    attempt: WriteAttemptId,
    connection: ConnectionId,
    deadline: i64,
}

/// Two answers, for the same reason the verification poll has two: what the
/// catalogue says about the listing is evidence the machine settles on, and
/// what stopped us looking says nothing about the listing at all.
enum ReconcileOutcome {
    Answered(Result<Option<RemoteListingId>, AdapterError>),
    /// The walk could not be performed to an answer. The write may have
    /// landed and we simply stopped looking, so the create's fence stays
    /// standing and the run abandons.
    Stopped(&'static str),
}

/// Walks the seller's own catalogue for a create whose fate is unknown,
/// repeating the walk while it answers absence.
///
/// The lease is extended before every walk, as before every other
/// network-bearing effect: an enumeration is one, and on a large catalogue a
/// slow one. Each walk is charged as a read, because that is what it is, and
/// a window that closes mid-poll stops the poll rather than being reported as
/// absence — absence halts the tenant's inventory, and a rate ceiling is a
/// fact about us rather than about the listing.
async fn reconcile_with_backoff<
    A: MarketplaceAdapter,
    N: NowSource,
    P: Pause,
    L: ItemLedger,
    C: Cancellation,
    I: IdSource,
    R: ReconcileSource,
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I, R>,
    lease: &LeasedItem,
    request: Reconciliation,
) -> Result<ReconcileOutcome, EngineError> {
    let lease_ref = lease.lease_ref();
    for walked in 0..RECONCILE_WALKS_MAX {
        let now = ctx.clock.now();
        if ctx.cancel.is_cancelled() || now.0 >= request.deadline {
            return Ok(ReconcileOutcome::Stopped("the run was cut"));
        }
        ctx.ledger.renew(&lease_ref).await?;
        let grant = ctx
            .ledger
            .request_grant(&lease_ref, request.connection, GrantKind::VerifyRead, now)
            .await?;
        if grant == BudgetGrant::Exhausted {
            return Ok(ReconcileOutcome::Stopped(
                "the per-connection rate window closed",
            ));
        }
        let found = ctx
            .reconcile
            .find_listing(&request.locator, request.attempt)
            .await;
        let last_walk = walked + 1 >= RECONCILE_WALKS_MAX;
        match found {
            // Found, or a condition rather than lag: both are answers, and
            // repeating the walk would only spend the seller's allowance on
            // a question already decided.
            Ok(Some(_)) | Err(_) => return Ok(ReconcileOutcome::Answered(found)),
            Ok(None) if last_walk => return Ok(ReconcileOutcome::Answered(found)),
            // Absence this early is lag, so the wait between walks is where
            // the marketplace catches up. A cancellation arriving inside it
            // stops the poll rather than being slept through: the answer
            // this run would have had is not one it may invent.
            Ok(None) => {
                if !waited_through(ctx, RECONCILE_WALK_INTERVAL_MS, request.deadline).await {
                    return Ok(ReconcileOutcome::Stopped("the run was cut"));
                }
            }
        }
    }
    // Unreachable while the bound is non-zero, and stated rather than
    // assumed: a poll that never looked has nothing to hand over, and
    // absence is the one answer it must not invent.
    Ok(ReconcileOutcome::Stopped("the reconcile never walked"))
}

/// What a strand recorded that identifies its create to a catalogue walk.
///
/// Only the recorded title: a marker strategy never strands — its ambiguous
/// submit emits the search directly, because the marker is embedded in the
/// write and needs no wait — and a durable identifier belongs to an
/// operation that was given one. So `None` is a locator this run has no
/// search for, which abandons rather than guessing.
fn recorded_by(locator: &ListingLocator) -> Option<RecordedTitle> {
    match locator {
        ListingLocator::Recorded { title, .. } => Some(title.clone()),
        ListingLocator::Marker { .. } | ListingLocator::Durable(_) => None,
    }
}

const fn outcome_to_attempt_state(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Committed { .. } | Outcome::Degraded { .. } => "committed",
        Outcome::Rejected { .. } => "failed",
        Outcome::Ambiguous { .. } => "ambiguous",
        Outcome::Blocked { .. } | Outcome::Skipped { .. } => "abandoned",
    }
}

/// The ambiguity a settled outcome names, where it names one.
const fn outcome_ambiguity(outcome: &Outcome) -> Option<AmbiguityCause> {
    match outcome {
        Outcome::Ambiguous { cause, .. } => Some(*cause),
        Outcome::Committed { .. }
        | Outcome::Degraded { .. }
        | Outcome::Rejected { .. }
        | Outcome::Blocked { .. }
        | Outcome::Skipped { .. } => None,
    }
}

/// The ambiguity an adapter answer carries, read off the input on its way
/// into the machine.
///
/// The machine drops it on the submit row — `ambiguous_submit` takes the
/// attempt and not the cause, because a strand is reconciled by a search
/// rather than by the cause of the strand. That is right for the transition
/// table and wrong for the record: the capture action recorded 73 seconds
/// into a create is the only trace of why that create became unknown, and
/// without this it said `capture:Ambiguity` and nothing more. So the driver
/// keeps what it fed in.
const fn input_ambiguity(input: &Input) -> Option<AmbiguityCause> {
    let answered = match input {
        Input::SubmitResult(Err(error))
        | Input::ReadBackResult(Err(error))
        | Input::ReconcileResult(Err(error)) => error,
        Input::PreflightResult(_)
        | Input::IntentRecorded(_)
        | Input::SubmitResult(Ok(_))
        | Input::ReadBackResult(Ok(_))
        | Input::ReconcileResult(Ok(_))
        | Input::ResumeStranded { .. }
        | Input::ChallengeCleared
        | Input::ParkExpired
        | Input::BudgetExhausted => return None,
    };
    match answered {
        AdapterError::Ambiguous(cause) => Some(*cause),
        AdapterError::Rejected { .. }
        | AdapterError::Challenge(_)
        | AdapterError::SessionExpired
        | AdapterError::SchemaDrift(_)
        | AdapterError::RateLimited { .. }
        | AdapterError::NotSent(_)
        | AdapterError::Uncaptured { .. } => None,
    }
}

/// The capture action's label, which is the operator's first look at a
/// halted create.
///
/// Three fields rather than two: the capture cause says which class of
/// diagnostic this is, the ambiguity names which of the five ways the write
/// became unknown, and the attempt names the row. `unknown` is written where
/// nothing observed a cause — a schema drift or a verification mismatch
/// captures diagnostics with no ambiguity at all — rather than leaving the
/// field out, so the label's shape does not change with its content.
fn capture_label(
    cause: CaptureCause,
    ambiguity: Option<AmbiguityCause>,
    attempt: Option<WriteAttemptId>,
) -> String {
    format!(
        "capture:{cause:?}:{}:{attempt:?}",
        ambiguity.map_or("unknown", AmbiguityCause::name)
    )
}

/// The ambiguity a machine state names, which is only ever a terminal one.
const fn state_ambiguity(state: &SyncState) -> Option<AmbiguityCause> {
    match state {
        SyncState::Terminal(outcome) => outcome_ambiguity(outcome),
        SyncState::AwaitingPreflight
        | SyncState::PreflightAsserted { .. }
        | SyncState::IntentRecorded { .. }
        | SyncState::Submitted { .. }
        | SyncState::AwaitingReadBack { .. }
        | SyncState::Stranded { .. }
        | SyncState::Parked { .. } => None,
    }
}

/// Everything this run knows, at this instant, about which ambiguity it met.
///
/// Three sources because the capture effect can be raised before, with or
/// after the answer that explains it: the machine emits `CaptureDiagnostics`
/// alongside the read-back it has not yet taken (`reconcile`), alongside the
/// terminal outcome that names the cause (`halt_ambiguous`), and one
/// transition after the submit that produced it (`ambiguous_submit`). Newest
/// first, so a read-back that answered a different ambiguity than the submit
/// is the one recorded.
fn ambiguity_now(
    observed: Option<AmbiguityCause>,
    pending: Option<&Input>,
    state: &SyncState,
) -> Option<AmbiguityCause> {
    pending
        .and_then(input_ambiguity)
        .or_else(|| state_ambiguity(state))
        .or(observed)
}

/// How many indeterminate preflights in a row stop being read as a transient.
///
/// A preflight failure other than schema drift abandons the run, which is the
/// stall bias and is right for a moment. It is wrong for a condition that
/// holds: the Tes preflight is write-bearing, so a broker holding a stale
/// secret fails it identically every cycle, and each abandoned lease keeps
/// `job_item_one_live_lease_per_org` shut for its whole TTL — the tenant's
/// entire queue waiting behind an item that will never move.
///
/// Three rather than five: the streak advances once per lease, so this is
/// three separate sessions against the marketplace before the connection
/// rather than the moment is blamed.
///
/// It is not the only thing that ends the streak. `attempt_count` is the
/// item's whole history — never reset, advanced by every expired lease, every
/// park revive and every preparation the host could not complete — so an item
/// that arrives here already near
/// `ATTEMPTS_MAX` has fewer leases left than this bound needs.
/// [`preflight_failed`] ends the streak early in that case rather than
/// letting the attempt reaper settle it `failed`/`Other`.
pub const PREFLIGHT_FAILURES_MAX: u32 = 3;

/// What this states is only that a *fresh* item can reach the streak bound at
/// all: an item admitted at `attempt_count` zero has `ATTEMPTS_MAX` leases and
/// spends one per failure. It states nothing about an item that has already
/// spent some of them — that race is decided in [`preflight_failed`], which
/// reads the item's actual `attempt_count`, rather than here.
const _: () = assert!(
    PREFLIGHT_FAILURES_MAX < tam_limits::job::ATTEMPTS_MAX,
    "an item admitted with no attempts behind it must be able to reach the streak bound"
);

/// Drives one leased item to a terminal state, a park, or abandonment, and
/// hands the lease back where the run decided nothing.
///
/// The hand-back is here rather than at each of the eight places that
/// abandon, so a path added to the interpreter cannot forget it. It is
/// best-effort by construction: the refusal is reported in the verdict's own
/// reason and never replaces it, because a lease we could not hand back is
/// exactly the case the expiry backstop still covers.
pub async fn run_item<
    A: MarketplaceAdapter,
    N: NowSource,
    P: Pause,
    L: ItemLedger,
    C: Cancellation,
    I: IdSource,
    R: ReconcileSource,
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I, R>,
    lease: &LeasedItem,
    seed: MachineSeed,
) -> Result<RunVerdict, EngineError> {
    let run = drive_item(ctx, lease, seed).await;
    // A settle and a park both end the item and release its lease in the
    // server's own transaction; only an undecided run leaves one standing.
    let undecided = match &run {
        Ok(RunVerdict::Settled(_) | RunVerdict::Parked) => false,
        Ok(RunVerdict::Abandoned { .. }) | Err(_) => true,
    };
    if !undecided {
        return run;
    }
    let handed = ctx
        .ledger
        .hand_back(&lease.lease_ref(), ctx.clock.now())
        .await;
    // Only an abandoned run has a sentence to append the refusal to, and a
    // stolen lease is not worth appending: the steal already owns the item,
    // which is why the hand-back was fenced out. Every other pairing keeps
    // the run's own answer.
    match (run, handed) {
        (Ok(RunVerdict::Abandoned { reason }), Err(why))
            if !matches!(why, LedgerError::StaleLease) =>
        {
            Ok(RunVerdict::Abandoned {
                reason: format!(
                    "{reason}; and the lease could not be handed back, so it runs to its \
                     expiry: {why}"
                ),
            })
        }
        (run, _) => run,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the effect loop is one cohesive interpreter; the lint is advisory here by charter"
)]
async fn drive_item<
    A: MarketplaceAdapter,
    N: NowSource,
    P: Pause,
    L: ItemLedger,
    C: Cancellation,
    I: IdSource,
    R: ReconcileSource,
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I, R>,
    lease: &LeasedItem,
    seed: MachineSeed,
) -> Result<RunVerdict, EngineError> {
    let org = lease.org;
    let lease_ref = lease.lease_ref();
    let Some(connection) = ctx
        .ledger
        .connection_for(&lease_ref, lease.inventory)
        .await?
    else {
        return Ok(RunVerdict::Abandoned {
            reason: "no linked connection at run time".to_owned(),
        });
    };
    let started = ctx.clock.now();
    let wall_deadline =
        started.0 + i64::try_from(tam_limits::job::WALL_CLOCK_MAX.as_millis()).unwrap_or(i64::MAX);

    // The ledger's own statement of what this item does, validated against
    // the mapping's binding by `prepare_item` before the seed was built: the
    // machine, the verification predicate and the landing effect are all
    // total functions of it, so all three read the same value.
    let operation = lease.operation.clone();
    let verify = seed.verify;
    let seed_attestation = seed.attestation.clone();
    let mut transition = SyncMachine::initial(
        org,
        lease.inventory,
        connection,
        seed.form,
        lease.item,
        lease.idempotency_key,
        seed.intent_hash,
        seed.fields,
        seed.strategy,
        operation.clone(),
        seed.budget,
    )?;
    // A reconcile does not start at the beginning. Stepping the resume here
    // replaces the entry row's effects rather than running them, and the
    // first of those is the form assertion — write-bearing on Tes, and the
    // last thing an item whose create may already have landed should do.
    if let Some((stranded, recorded)) = seed.resume {
        let at = ctx.clock.now();
        transition = transition.next.step(
            Input::ResumeStranded {
                attempt: stranded,
                recorded,
            },
            LogicalInstant(at.0),
        )?;
    }
    let mut current_attempt: Option<WriteAttemptId> = None;
    // The last ambiguity an adapter answered this run with, kept because the
    // machine's transition table does not: the capture label and the
    // `write_attempt.ambiguity_cause` column are both written from it, and
    // both said nothing before.
    let mut observed_ambiguity: Option<AmbiguityCause> = None;
    // What the verification read last saw, so a bind records the lifecycle
    // the listing was observed in rather than the one its create convention
    // would imply.
    let mut observed_lifecycle: Option<RemoteLifecycle> = None;
    let mut sequence: u32 = 0;
    // Whether this run has already resumed its own stranded create. One
    // resume per claim: the wait and the walks are bounded, and a second
    // strand means the search itself is not answering.
    let mut resumed = false;
    // Whether the write this run sent was answered by a challenge rather
    // than by the marketplace. See the submit arm: it decides whether a
    // strand is searchable from inside this run.
    let mut blocked_mid_write = false;

    loop {
        let now = ctx.clock.now();
        // Stepped only where the machine can still take the input, which is
        // the same rule `verify_with_backoff` states for itself. A terminal
        // machine answers `InputNotApplicable`, returning `Err` with a
        // listing the submit already committed left unsettled; a parked one
        // exchanges its own park, gate and notification for an abandoned
        // attempt, releasing the only fence against a second create while the
        // mapping is still unbound. Both arms return from the match below
        // instead, which is what the cancellation wanted in the first place.
        let stopping = ctx.cancel.is_cancelled() || now.0 >= wall_deadline;
        // Nothing has been attempted yet, so there is nothing to settle on.
        // Returning here rather than stepping the machine is the whole of the
        // rule: an item stopped before its first request is the seller's work
        // left undone, not work this run learned anything about, and the
        // reaper hands it to a device that can still do it. Settling it would
        // drop a seller's queued item on the strength of this device's
        // entitlement rather than on anything about the item.
        if stopping {
            // Split by whether a request can have gone out, which is what the
            // machine's own state says and what `BudgetExhausted` does not
            // distinguish. Stepping it from a state that has sent nothing
            // settles the item terminally — `Skipped` before the attempt,
            // `Ambiguous` after it — on the strength of this device stopping
            // rather than of anything about the item. Neither halts anything:
            // `exhaust_budget` emits `CaptureDiagnostics` alone, and the halt
            // belongs to the ambiguous-submit row, which is a different
            // situation reached a different way.
            match transition.next.state {
                // Nothing attempted and nothing to settle: hand the lease back
                // and let the reaper give the work to a device that can do it.
                SyncState::AwaitingPreflight | SyncState::PreflightAsserted { .. } => {
                    return Ok(RunVerdict::Abandoned {
                        reason: "stopped before the first request, with nothing attempted"
                            .to_owned(),
                    })
                }
                // The attempt is open and the write has not gone out. It has to
                // be settled rather than left standing, or the fence it holds
                // blocks every later lease on this mapping; abandoned rather
                // than ambiguous, because an ambiguity is a claim that a write
                // may have landed, and nothing was sent. The item would settle
                // terminally on that claim and never run again, which is the
                // seller's work lost to one device going away.
                SyncState::IntentRecorded { attempt, .. } => {
                    return rate_refused_before_the_write(
                        ctx,
                        &lease_ref,
                        Abandonment {
                            settling: AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            operation: &operation,
                            at: now,
                            reason: "stopped after the intent was recorded and before the \
                                     write, having sent nothing",
                        },
                    )
                    .await
                }
                // Something may already be on the wire, so stopping here is
                // the backstop's ambiguity rather than an abandon, and the
                // machine decides it.
                SyncState::Submitted { .. }
                | SyncState::AwaitingReadBack { .. }
                | SyncState::Stranded { .. }
                | SyncState::Parked { .. }
                | SyncState::Terminal(_) => {}
            }
        }
        if stopping
            && !matches!(
                transition.next.state,
                SyncState::Terminal(_) | SyncState::Parked { .. } | SyncState::Stranded { .. }
            )
        {
            transition = transition
                .next
                .step(Input::BudgetExhausted, LogicalInstant(now.0))?;
        }
        let Transition { next, effects } = transition;
        // Read before the effects run, because the arm below needs the
        // strand's own identification and the machine is consumed by the
        // step that follows. `None` is either a run that has already
        // searched once — one wait and one poll per claim, so a marketplace
        // that answers nothing cannot hold the lease indefinitely — or a
        // locator this run cannot search, which is the case the abandon
        // still covers.
        let strand_resume = match (&next.state, resumed) {
            (SyncState::Stranded { attempt, locator }, false) if !blocked_mid_write => {
                recorded_by(locator).map(|title| (*attempt, title))
            }
            _ => None,
        };
        let mut pending: Option<Input> = None;
        for effect in effects.0 {
            sequence += 1;
            let now = ctx.clock.now();
            match effect {
                Effect::AssertFormSchema { form } => {
                    // The form scrape is the first marketplace request a
                    // create makes, so it is the first place a lease stolen
                    // while the device was idle can be found — before the
                    // request rather than after it.
                    ctx.ledger.renew(&lease_ref).await?;
                    // Charged like every other marketplace request, and it was
                    // not before: the scrape went out against nobody's
                    // allowance, so a seller's rate window under-counted by one
                    // request per create. Nothing has been attempted at this
                    // point, so an exhausted window abandons with nothing to
                    // settle, which is the same shape stopping before the first
                    // request takes.
                    let grant = ctx
                        .ledger
                        .request_grant(&lease_ref, connection, GrantKind::FormRead, now)
                        .await?;
                    if grant == BudgetGrant::Exhausted {
                        return Ok(RunVerdict::Abandoned {
                            reason: "the rate window closed before the form scrape, with \
                                     nothing attempted"
                                .to_owned(),
                        });
                    }
                    let asserted = ctx.adapter.assert_form_schema(form).await;
                    pending = Some(match asserted {
                        Ok(fingerprint) => {
                            ctx.ledger.preflight_succeeded(&lease_ref).await?;
                            Input::PreflightResult(Ok(fingerprint))
                        }
                        Err(AdapterError::SchemaDrift(drift)) => {
                            Input::PreflightResult(Err(*drift))
                        }
                        Err(error) => return preflight_failed(ctx, lease, &error, now).await,
                    });
                }
                Effect::RecordIntent { intent_hash } => {
                    // Minted here rather than by the ledger, so a response
                    // lost in flight is recoverable by re-offering the same id
                    // instead of burning the item's whole attempt budget.
                    let minted = ctx.ids.new_id();
                    let attempt = ctx
                        .ledger
                        .open_attempt(
                            &lease_ref,
                            &NewAttempt {
                                attempt: minted,
                                mapping: lease.mapping,
                                intent: AttemptIntent {
                                    body: attested_intent(
                                        &operation,
                                        &next.fields,
                                        seed_attestation.as_ref(),
                                    ),
                                    hash: intent_hash.0.to_vec(),
                                },
                            },
                            now,
                        )
                        .await;
                    match attempt {
                        Ok(()) => {
                            let attempt = WriteAttemptId(minted);
                            current_attempt = Some(attempt);
                            pending = Some(Input::IntentRecorded(attempt));
                        }
                        Err(LedgerError::AttemptInFlight) => {
                            return Ok(RunVerdict::Abandoned {
                                reason: "another attempt is in flight for this mapping".to_owned(),
                            })
                        }
                        // Settled rather than abandoned, and the difference
                        // matters. The mapping was bound between this run's
                        // admission and this write, so the listing exists and
                        // no later lease could make it again; abandoning would
                        // re-abandon on every lease until the attempt budget
                        // settled the item `failed` with nothing in the ledger
                        // to say why.
                        Err(LedgerError::MappingAlreadyBound) => {
                            let verdict = ItemVerdict {
                                outcome: ItemOutcome::Skipped,
                                failure_code: Some(FailureCode::Other),
                                failure_detail: Some(FailureDetail(
                                    "the listing this item would have created was created by \
                                     another run, so this item had nothing left to do"
                                        .to_owned(),
                                )),
                            };
                            ctx.ledger.settle_item(&lease_ref, &verdict, now).await?;
                            record_event(
                                ctx,
                                lease,
                                &JobEventPayload::ItemSettled {
                                    outcome: format!("{:?}", ItemOutcome::Skipped),
                                },
                                now,
                            )
                            .await?;
                            return Ok(RunVerdict::Settled(ItemOutcome::Skipped));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Effect::Submit {
                    attempt,
                    key,
                    fields,
                } => {
                    let grant = consume_write_grant(ctx, &lease_ref, connection, now).await?;
                    if grant == BudgetGrant::Exhausted {
                        return rate_refused_before_the_write(
                            ctx,
                            &lease_ref,
                            Abandonment {
                                settling: AttemptRef {
                                    attempt: attempt.0,
                                    mapping: lease.mapping,
                                },
                                operation: &operation,
                                at: now,
                                reason: "the per-connection rate window is exhausted",
                            },
                        )
                        .await;
                    }
                    ctx.ledger.renew(&lease_ref).await?;
                    record_action(ctx, lease, sequence, "submit", now).await?;
                    let submitted = ctx.adapter.submit(key, fields, now).await;
                    // A create can strand two ways and only one of them is
                    // searchable from here. A lost answer leaves the
                    // marketplace reachable and merely behind, which is what
                    // the walk below waits out. A challenge or a lapsed
                    // session is the edge answering our address rather than
                    // our write, and it answers an enumeration the same way:
                    // the walk would come back empty, and an empty walk is
                    // the answer that settles an ambiguity and halts this
                    // tenant's inventory. So the run records the strand and
                    // stops, exactly as it did before, and the item is
                    // reconciled once the seller has cleared the edge.
                    blocked_mid_write = matches!(
                        submitted,
                        Err(AdapterError::Challenge(_) | AdapterError::SessionExpired)
                    );
                    pending = Some(Input::SubmitResult(submitted));
                }
                Effect::Revise {
                    attempt,
                    subject,
                    fields,
                    transition: lifecycle,
                } => {
                    let grant = consume_write_grant(ctx, &lease_ref, connection, now).await?;
                    if grant == BudgetGrant::Exhausted {
                        return rate_refused_before_the_write(
                            ctx,
                            &lease_ref,
                            Abandonment {
                                settling: AttemptRef {
                                    attempt: attempt.0,
                                    mapping: lease.mapping,
                                },
                                operation: &operation,
                                at: now,
                                reason: "the per-connection rate window is exhausted",
                            },
                        )
                        .await;
                    }
                    ctx.ledger.renew(&lease_ref).await?;
                    record_action(ctx, lease, sequence, "revise", now).await?;
                    let revised = ctx
                        .adapter
                        .revise(
                            RevisePlan {
                                subject,
                                fields,
                                transition: lifecycle,
                            },
                            now,
                        )
                        .await;
                    pending = Some(Input::SubmitResult(revised));
                }
                Effect::Remove {
                    attempt,
                    subject,
                    state,
                } => {
                    let grant = consume_write_grant(ctx, &lease_ref, connection, now).await?;
                    if grant == BudgetGrant::Exhausted {
                        return rate_refused_before_the_write(
                            ctx,
                            &lease_ref,
                            Abandonment {
                                settling: AttemptRef {
                                    attempt: attempt.0,
                                    mapping: lease.mapping,
                                },
                                operation: &operation,
                                at: now,
                                reason: "the per-connection rate window is exhausted",
                            },
                        )
                        .await;
                    }
                    ctx.ledger.renew(&lease_ref).await?;
                    record_action(ctx, lease, sequence, "remove", now).await?;
                    let removed = ctx
                        .adapter
                        .remove(
                            RemovalPlan {
                                attempt,
                                subject,
                                state,
                            },
                            now,
                        )
                        .await;
                    pending = Some(Input::SubmitResult(removed));
                }
                Effect::ReadBack { locator, reason } => {
                    ctx.ledger.renew(&lease_ref).await?;
                    // One action for the whole poll: a poll is one logical
                    // read of the marketplace, and recording each try would
                    // multiply the item's event stream by the try budget.
                    record_action(ctx, lease, sequence, "read-back", now).await?;
                    let verified = verify_with_backoff(
                        ctx,
                        lease,
                        Verification {
                            policy: verify,
                            locator,
                            reason,
                            operation: &operation,
                            connection,
                            deadline: wall_deadline,
                        },
                    )
                    .await?;
                    match verified {
                        VerifyOutcome::Observed(observed) => {
                            if let Ok(listing) = &observed {
                                observed_lifecycle = Some(listing.lifecycle.clone());
                            }
                            pending = Some(Input::ReadBackResult(observed));
                        }
                        // The write went out and we stopped looking, so the
                        // attempt is ambiguous rather than abandoned. The
                        // settle is what breaks the deadlock: without it the
                        // next lease abandons on `AttemptInFlight` and the
                        // item burns its whole retry allowance with nothing
                        // leaving the process.
                        //
                        // Except for a create, which is the one operation a
                        // requeue can repeat into a second listing:
                        // `may_settle_unverified` holds that fence shut and
                        // accepts the deadlock as the cheaper failure.
                        VerifyOutcome::Stopped(stop) => {
                            let stopped_by = stop.reason();
                            let settling =
                                current_attempt.filter(|_| may_settle_unverified(&operation));
                            if let Some(attempt) = settling {
                                let verdict = AttemptVerdict {
                                    state: "ambiguous".to_owned(),
                                    failure_code: None,
                                    // Whatever this run observed, which for a
                                    // run cut while polling is usually
                                    // nothing: the column stays NULL rather
                                    // than naming the stop as a cause, which
                                    // it is not — the stop is why we gave up
                                    // looking, not why the write is unknown.
                                    ambiguity: observed_ambiguity,
                                    landing: addressed_by(&operation),
                                };
                                settle_open_attempt(
                                    ctx,
                                    &lease_ref,
                                    AttemptRef {
                                        attempt: attempt.0,
                                        mapping: lease.mapping,
                                    },
                                    &verdict,
                                    now,
                                )
                                .await?;
                            }
                            let fenced = if settling.is_none() && current_attempt.is_some() {
                                "; the attempt stays in flight so the create cannot repeat"
                            } else {
                                ""
                            };
                            return Ok(RunVerdict::Abandoned {
                                reason: format!(
                                    "{stopped_by} before the write could be verified{fenced}"
                                ),
                            });
                        }
                    }
                }
                Effect::Reconcile { attempt, locator } => {
                    // One action for the whole poll, exactly as the read-back
                    // records one: a reconcile is one logical read of the
                    // seller's catalogue, and recording each walk would
                    // multiply the item's event stream by the walk budget.
                    record_action(ctx, lease, sequence, "reconcile", now).await?;
                    let walked = reconcile_with_backoff(
                        ctx,
                        lease,
                        Reconciliation {
                            locator,
                            attempt,
                            connection,
                            deadline: wall_deadline,
                        },
                    )
                    .await?;
                    let found = match walked {
                        ReconcileOutcome::Answered(found) => found,
                        // Nothing was established about the listing, so
                        // nothing is settled: the attempt stays in flight and
                        // the create cannot repeat. The same shape the
                        // read-back's stop takes, for the same reason.
                        ReconcileOutcome::Stopped(stopped_by) => {
                            return Ok(RunVerdict::Abandoned {
                                reason: format!(
                                    "{stopped_by} before the seller's catalogue could answer \
                                     for this create; the attempt stays in flight so the \
                                     create cannot repeat"
                                ),
                            })
                        }
                    };
                    // A resume opens no attempt, so the run reaches here owning
                    // none. The find substitutes for the lost write response,
                    // and adopting the standing row on it is what lets the
                    // terminal settle that row rather than leave the
                    // duplicate-create fence standing for ever. Only on a find:
                    // an answer that identified nothing settles nothing, which
                    // is the whole of why the other two outcomes stay stranded.
                    if matches!(found, Ok(Some(_))) {
                        current_attempt = Some(attempt);
                    }
                    pending = Some(Input::ReconcileResult(found));
                }
                Effect::ParkItem {
                    item: _,
                    challenge,
                    expires,
                } => {
                    // The machine's `expires` is recorded in the event below
                    // as what it asked for; the park's own expiry is the
                    // server's, minted from its own constant, because the
                    // reaper is the only clock that reads it back.
                    ctx.ledger
                        .park(&lease_ref, &format!("{challenge:?}"))
                        .await?;
                    record_event(
                        ctx,
                        lease,
                        &JobEventPayload::ItemParked {
                            expires_ms: expires.0,
                        },
                        now,
                    )
                    .await?;
                }
                Effect::RequeueBehindGate {
                    connection: _,
                    cause,
                } => {
                    // The gate is `Reauth`'s alone. It flips the connection to
                    // `needs_reauth`, which the lease scan reads as "hold this
                    // tenant's whole marketplace" and the status page reads as
                    // "disconnected — re-link": the right answer for a lapsed
                    // session and the wrong one for every other cause here,
                    // none of which a re-link clears. The machine no longer
                    // raises this effect for a challenge; gating on the cause
                    // rather than on the effect's presence is what keeps that
                    // true if it ever does again.
                    if matches!(cause, BlockCause::Reauth) {
                        ctx.ledger
                            .gate_connection(&lease_ref, lease.inventory, now)
                            .await?;
                    }
                    record_event(
                        ctx,
                        lease,
                        &JobEventPayload::ItemBlocked {
                            cause: block_cause_name(cause).to_owned(),
                        },
                        now,
                    )
                    .await?;
                }
                Effect::CaptureDiagnostics { attempt, cause } => {
                    // Read here rather than remembered, because this effect
                    // is raised before, with and after the answer that
                    // explains it depending on which row emitted it.
                    let ambiguity =
                        ambiguity_now(observed_ambiguity, pending.as_ref(), &next.state);
                    observed_ambiguity = ambiguity;
                    record_action(
                        ctx,
                        lease,
                        sequence,
                        &capture_label(cause, ambiguity, attempt),
                        now,
                    )
                    .await?;
                }
                Effect::Halt { scope } => {
                    // One scope exists, so there is no wider one to refuse.
                    // `HaltScope` is a struct rather than a sum precisely so
                    // this match cannot name the fleet kill switch.
                    ctx.ledger
                        .halt_this_tenant(
                            &lease_ref,
                            scope.inventory,
                            "the transition table demanded a halt",
                            now,
                        )
                        .await?;
                }
                Effect::Notify { org: _, event } => {
                    notify(ctx, lease, event, now).await?;
                }
            }
        }

        match (&next.state, pending) {
            (SyncState::Terminal(outcome), _) => {
                let now = ctx.clock.now();
                // A blocked terminal raises no gate, so nothing else on this
                // path would tell the progress stream what stopped the item.
                // The event is the cause; the settle below is only the fact.
                if let Outcome::Blocked { challenge } = outcome {
                    let cause = if tam_domain::seller_clears(*challenge) {
                        BlockCause::Reauth
                    } else {
                        BlockCause::Challenge
                    };
                    record_event(
                        ctx,
                        lease,
                        &JobEventPayload::ItemBlocked {
                            cause: block_cause_name(cause).to_owned(),
                        },
                        now,
                    )
                    .await?;
                }
                let verdict = outcome_to_item(outcome);
                let item_outcome = verdict.outcome;
                let mut severed = false;
                if let Some(attempt) = current_attempt {
                    // Best-effort: a fenced-out attempt settle means the item
                    // was stolen, and the steal owns the story from here.
                    let attempt_verdict = AttemptVerdict {
                        state: outcome_to_attempt_state(outcome).to_owned(),
                        failure_code: verdict.failure_code,
                        // The terminal outcome's own cause where it has one,
                        // and it does on every ambiguous settle: the machine
                        // computes it in `halt_ambiguous` and it was dropped
                        // here, which is why every ambiguous row in the
                        // ledger carries a NULL cause today.
                        ambiguity: outcome_ambiguity(outcome).or(observed_ambiguity),
                        landing: outcome_to_landing(
                            &operation,
                            outcome,
                            observed_lifecycle.as_ref(),
                        ),
                    };
                    let settled = ctx
                        .ledger
                        .settle_attempt(
                            &lease_ref,
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            &attempt_verdict,
                            now,
                        )
                        .await;
                    match settled {
                        Ok(disposition) => {
                            severed = matches!(disposition, BindDisposition::Severed);
                            if let Some(anomaly) = bind_anomaly(disposition) {
                                record_event(
                                    ctx,
                                    lease,
                                    &JobEventPayload::ItemBindAnomaly { anomaly },
                                    now,
                                )
                                .await?;
                            }
                        }
                        Err(error) => {
                            return Ok(RunVerdict::Abandoned {
                                reason: format!("the attempt settle was fenced: {error}"),
                            })
                        }
                    }
                }
                // Nothing fences the attempt settle against the item's epoch,
                // so a run whose lease was stolen — and whose item the steal
                // already settled — still severs the mapping here, and this
                // fenced item settle is where that becomes knowable. Record
                // it rather than fence it: real fencing changes the create
                // path too and is founder-gated.
                let item_settled = ctx.ledger.settle_item(&lease_ref, &verdict, now).await;
                if severed && matches!(item_settled, Err(LedgerError::StaleLease)) {
                    record_event(
                        ctx,
                        lease,
                        &JobEventPayload::ItemBindAnomaly {
                            anomaly: BindAnomaly::SeveredAfterSteal {
                                lease_epoch: lease_ref.lease_epoch,
                            },
                        },
                        now,
                    )
                    .await?;
                }
                item_settled?;
                record_event(
                    ctx,
                    lease,
                    &JobEventPayload::ItemSettled {
                        outcome: format!("{item_outcome:?}"),
                    },
                    now,
                )
                .await?;
                return Ok(RunVerdict::Settled(item_outcome));
            }
            (SyncState::Parked { .. }, _) => return Ok(RunVerdict::Parked),
            // The write went out and its fate is unknown. Nothing is settled
            // here — the attempt stays in flight so no second create can
            // start — and where the marketplace is still readable the search
            // for it happens in this run rather than in a later claim. A
            // later claim could not begin until this lease expired, so
            // deferring cost the seller a whole TTL of held queue for every
            // stranded write; this run already holds the lease, the session
            // and the catalogue access it needs.
            (SyncState::Stranded { .. }, _) => {
                let Some((attempt, recorded)) = strand_resume else {
                    let why = if blocked_mid_write {
                        "the marketplace is challenging this device, so the catalogue \
                         cannot be walked from here either; the item is reconciled once \
                         the challenge is cleared"
                    } else if resumed {
                        "this run has already searched the seller's catalogue for it once"
                    } else {
                        "this strand records no identification a catalogue walk could use"
                    };
                    return Ok(RunVerdict::Abandoned {
                        reason: format!(
                            "the write went out and its fate is unknown; the attempt stays \
                             in flight and {why}"
                        ),
                    });
                };
                resumed = true;
                // The lease is extended before the wait rather than after
                // it. The heartbeat is what makes a run allowed to be slow,
                // and a wait taken without one would hand the item to the
                // reaper in the middle of it — which is the failure this
                // whole path replaces.
                ctx.ledger.renew(&lease_ref).await?;
                if !waited_through(ctx, STRAND_SETTLE_WAIT_MS, wall_deadline).await {
                    return Ok(RunVerdict::Abandoned {
                        reason: "the write went out and its fate is unknown; the run was \
                                 cut while waiting for the marketplace to publish it, and \
                                 the attempt stays in flight so the create cannot repeat"
                            .to_owned(),
                    });
                }
                let now = ctx.clock.now();
                transition = next.step(
                    Input::ResumeStranded { attempt, recorded },
                    LogicalInstant(now.0),
                )?;
            }
            (_, Some(input)) => {
                // Before the step, because the machine's submit row drops the
                // cause and the capture action one transition later is where
                // an operator looks for it.
                observed_ambiguity = input_ambiguity(&input).or(observed_ambiguity);
                let now = ctx.clock.now();
                transition = next.step(input, LogicalInstant(now.0))?;
            }
            (_, None) => {
                return Ok(RunVerdict::Abandoned {
                    reason: "the effects produced no next input in a non-terminal state".to_owned(),
                })
            }
        }
    }
}

/// One grant for one lifecycle write.
///
/// What the ceiling bounds is *effects* rather than requests, and always
/// has: one submit issues a create, a metadata write, a three-step upload per
/// file and a state read, and consumes one grant for all of it. So this is a
/// lower bound on outbound traffic rather than a count of it. Making it exact
/// means consuming inside the transport seam, which is recorded as founder-
/// gated rather than assumed here.
///
/// Neither the window nor the ceiling appears: both are the server's, derived
/// from its own copy of the limits. A governed party that supplied either
/// would be setting its own rate limit.
async fn consume_write_grant(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeaseRef,
    connection: ConnectionId,
    now: Timestamp,
) -> Result<BudgetGrant, EngineError> {
    Ok(ctx
        .ledger
        .request_grant(lease, connection, GrantKind::Write, now)
        .await?)
}

/// The rate window closed before a lifecycle write was called. Nothing was
/// sent, so the attempt settles `'abandoned'` exactly as the submit path's
/// refusal does, and the run abandons into the stealer.
async fn rate_refused_before_the_write(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease_ref: &LeaseRef,
    abandonment: Abandonment<'_>,
) -> Result<RunVerdict, EngineError> {
    let Abandonment {
        settling,
        operation,
        at,
        reason,
    } = abandonment;
    let verdict = AttemptVerdict {
        state: "abandoned".to_owned(),
        failure_code: None,
        // Nothing was sent, so there is no ambiguity to name.
        ambiguity: None,
        landing: addressed_by(operation),
    };
    settle_open_attempt(ctx, lease_ref, settling, &verdict, at).await?;
    Ok(RunVerdict::Abandoned {
        reason: reason.to_owned(),
    })
}

/// An open attempt settled without anything having been sent, and why.
///
/// Bundled rather than passed loose because the four travel together and the
/// reason is the only one that differs between callers: a closed rate window
/// and a stopped run reach the identical ledger state and would be
/// indistinguishable in the item's history without it.
struct Abandonment<'a> {
    settling: AttemptRef,
    operation: &'a ItemOperation,
    at: Timestamp,
    reason: &'a str,
}

/// The preflight answered something other than drift. While the item can
/// still afford to try again this is the stall bias as it always was —
/// abandon, let the lease expire, let the stealer requeue.
///
/// Two things end that. The streak reaching [`PREFLIGHT_FAILURES_MAX`] is the
/// evidential one: the same answer across that many separate leases is about
/// the connection rather than about the moment. The item running out of
/// attempt budget is the arithmetic one, and it is why this reads
/// `attempt_count` rather than trusting the constants to order themselves.
/// `attempt_count` is prior history this run did not choose — never reset,
/// advanced by every expired lease, every park revive and every preparation
/// the host could not complete — so an item can
/// arrive here with one lease left and no way to reach the streak bound.
/// Abandoning on that last lease hands it to `expire_and_steal`, which
/// settles it `failed`/`Other` with a null detail, no gate and no re-link
/// prompt, and the next item then repeats the whole wedge. The cause is known
/// either way, so settling early with the outcome that names it is the honest
/// answer rather than a guess.
///
/// The item settles `Blocked` rather than `Failed` either way, because
/// nothing was refused — no write was ever attempted — and `Blocked` is what
/// the fleet breaker counts, so a marketplace-wide preflight outage reaches
/// it. What the error class decides is the other half: whose condition this
/// is. [`preflight_challenge`] says how far the error can be trusted to
/// answer that, and it is not all the way.
///
/// A lapsed session is the seller's, so the connection is gated first and
/// settled second — the gate is what stops this tenant's remaining items each
/// repeating the whole streak against the same dead credential, and settling
/// alone would clear one item and hand the wedge to the next. A challenge is
/// the marketplace's edge, so nothing is gated at all: printing "re-link" over
/// an egress block sends the seller to fix a credential that works, and the
/// remedy that fits is the breaker noticing every tenant settling the same way.
///
/// Blaming the connection can still be wrong where the error cannot say —
/// a marketplace having a bad hour fails the preflight indeterminately, and
/// indeterminate reads as reauth here. The arithmetic arm widens that a
/// little, because an item at its last attempt is gated on a shorter streak.
/// It stays the cheap error of the two: re-linking costs one seller one minute
/// and the gate lifts, where the alternative spends every item in the queue on
/// the same dead leases and settles them all `failed`/`Other`.
async fn preflight_failed(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeasedItem,
    error: &AdapterError,
    at: Timestamp,
) -> Result<RunVerdict, EngineError> {
    let lease_ref = lease.lease_ref();
    let seen = preflight_challenge(error);
    let streak = ctx
        .ledger
        .preflight_failed(&lease_ref, !seller_clears(seen))
        .await?;
    // The reaper's own predicate, read against the value `acquire` returned:
    // nothing moves `attempt_count` during a run, so this is exactly what
    // `expire_and_steal` will test when this lease expires.
    let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
    let last_attempt = attempt_budget_spent(lease.attempt_count, attempts_max);
    if streak.failures < PREFLIGHT_FAILURES_MAX && !last_attempt {
        return Ok(RunVerdict::Abandoned {
            reason: format!("preflight failed transiently: {error:?}"),
        });
    }
    // The whole streak decides, not its last member. A streak that mixed a
    // Cloudflare block with a lapsed session takes the seller-actionable
    // floor: a real auth failure anywhere in it is something they can fix,
    // where an edge block is not, and judging by whichever error arrived last
    // would make the seller's instructions depend on arrival order.
    let challenge = if streak.edge_only {
        seen
    } else {
        ChallengeKind::ReauthRequired
    };
    let cause = if seller_clears(challenge) {
        BlockCause::Reauth
    } else {
        BlockCause::Challenge
    };
    // The one line that makes a refused claim diagnosable on the device.
    // `ItemBlocked{cause: "reauth"}` reaches the server one second after the
    // lease and says nothing about what the marketplace answered, so a
    // session the check-in probe proved good seconds earlier reads as a
    // lapsed credential with no evidence either way. This names the
    // adapter's own verdict, the streak that widened it and whether the
    // whole streak was the edge — enough to tell "Tes really said 401" from
    // "the preflight failed indeterminately and indeterminate reads as
    // reauth here". `eprintln!` because on Android this is the only channel
    // that reaches logcat, and the adapter's own step line printed just
    // above it names the request and status this verdict was read from.
    eprintln!(
        "driver: preflight blocked {} after {} failure(s), edge_only {}: seen {seen:?} from \
         {error:?}",
        block_cause_name(cause),
        streak.failures,
        streak.edge_only,
    );
    if matches!(cause, BlockCause::Reauth) {
        ctx.ledger
            .gate_connection(&lease_ref, lease.inventory, at)
            .await?;
    }
    record_event(
        ctx,
        lease,
        &JobEventPayload::ItemBlocked {
            cause: block_cause_name(cause).to_owned(),
        },
        at,
    )
    .await?;
    let (code, detail) = challenge_verdict(challenge);
    let verdict = ItemVerdict {
        outcome: ItemOutcome::Blocked,
        failure_code: Some(code),
        // The streak is deliberately not quoted in the detail. It reads 1 or 2
        // when the attempt budget is what ended the run, so a sentence built
        // on it would tell the seller a number that is about our own
        // bookkeeping. `job_item.preflight_failures` holds the count.
        failure_detail: Some(detail),
    };
    ctx.ledger.settle_item(&lease_ref, &verdict, at).await?;
    record_event(
        ctx,
        lease,
        &JobEventPayload::ItemSettled {
            outcome: format!("{:?}", ItemOutcome::Blocked),
        },
        at,
    )
    .await?;
    if matches!(cause, BlockCause::Reauth) {
        notify(ctx, lease, SellerEvent::ReauthRequired, at).await?;
    }
    Ok(RunVerdict::Settled(ItemOutcome::Blocked))
}

/// Which class of condition ended the preflight, as far as the error can say.
///
/// `Challenge` and `SessionExpired` are the adapter having decided, and both
/// reach here: the Tes preflight's read steps run `classify_read`, which
/// splits a 401 or a 403 on Cloudflare's own markers into one or the other.
///
/// Everything else answers `ReauthRequired`, and that is a floor rather than a
/// judgement. The preflight's first two steps are writes, and `classify_write`
/// maps 401, 403 and 429 to `Ambiguous(ReadBackIndeterminate)` on purpose —
/// the write may have landed before the refusal, and that asymmetry is the
/// whole reason there are two classifiers. So an edge block on the create step
/// arrives byte-identical to a lapsed session on the create step. The data
/// does not carry the distinction and this must not invent one; the arm keeps
/// the behaviour the streak bound shipped with, which is right for the
/// condition it was written against and wrong for an egress block that only
/// ever fails on a write. Splitting it means giving the write classifier a
/// marker check of its own, which is a change to what an ambiguous write means
/// and is not this function's to make.
const fn preflight_challenge(error: &AdapterError) -> ChallengeKind {
    match error {
        AdapterError::Challenge(kind) => *kind,
        AdapterError::SessionExpired
        | AdapterError::Ambiguous(_)
        | AdapterError::Rejected { .. }
        | AdapterError::SchemaDrift(_)
        | AdapterError::RateLimited { .. }
        | AdapterError::NotSent(_)
        | AdapterError::Uncaptured { .. } => ChallengeKind::ReauthRequired,
    }
}

/// What an unfinished attempt names. A revise or a removal addressed a
/// listing that already exists, so the settled row says which one and the
/// ledger's "every settled row names what the write was about" holds on the
/// walk-away paths too; a create knows no identifier until its read-back
/// answers, and inventing one would be worse than recording nothing.
fn addressed_by(operation: &ItemOperation) -> LandingEffect {
    operation
        .subject()
        .map_or(LandingEffect::None, |id| LandingEffect::Addressed {
            id: id.clone(),
        })
}

/// Whether a run walking away from a write it could not verify may settle
/// that write's attempt.
///
/// `write_attempt_one_in_flight` is the only fence this system has against a
/// second create: neither adapter has an idempotent create to fall back on,
/// and both say so where they take the idempotency key. Settling releases
/// the fence, so a create's attempt is left standing instead — the requeued
/// item abandons on `AttemptInFlight` until `ATTEMPTS_MAX` settles it
/// `failed`, with nothing further leaving the process. That stall is the
/// worse-looking outcome and the better one: a duplicate listing on a
/// seller's store is the one failure this ledger cannot undo.
///
/// A revise or a removal addresses a listing that already exists, so
/// re-running it mints nothing and the settle is purely the deadlock repair
/// it was written as.
const fn may_settle_unverified(operation: &ItemOperation) -> bool {
    !matches!(operation, ItemOperation::Create)
}

/// Settles an attempt the run is about to walk away from, so abandoning does
/// not strand an `in_flight` row on the mapping.
///
/// `expire_and_steal` requeues the item and bumps `job_item.lease_epoch` but
/// settles no orphan attempt, and `write_attempt_one_in_flight` admits one
/// open attempt per mapping — so a re-leased item whose predecessor left one
/// standing abandons on `AttemptInFlight` at `RecordIntent` on every pass,
/// burning its whole retry allowance without another request leaving the
/// process. Where that fence is the only thing standing between a requeue
/// and a duplicate listing, [`may_settle_unverified`] keeps the row standing
/// deliberately and this helper is not called at all.
///
/// The verdict's state is the whole difference between the two callers:
/// `'abandoned'` where the refusal came before the write was called and
/// nothing was sent, `'ambiguous'` where the write went out and the
/// verification could not finish. Neither writes the mapping — `Addressed`
/// names the listing in the attempt row and leaves the binding alone.
async fn settle_open_attempt(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeaseRef,
    settling: AttemptRef,
    verdict: &AttemptVerdict,
    at: Timestamp,
) -> Result<(), EngineError> {
    match ctx
        .ledger
        .settle_attempt(lease, settling, verdict, at)
        .await
    {
        // A fenced settle means the lease was stolen, and the steal owns the
        // story from here — the same reading the terminal path already takes.
        Ok(_) | Err(LedgerError::StaleLease) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// The ledger's share of a bind disposition. The three that leave a landed
/// listing unrecorded travel; the three that recorded it, or had nothing to
/// record, do not, so a row in the ledger is always something to act on.
fn bind_anomaly(disposition: BindDisposition) -> Option<BindAnomaly> {
    match disposition {
        BindDisposition::Bound
        | BindDisposition::AlreadyBound
        | BindDisposition::NotLanded
        | BindDisposition::Addressed
        | BindDisposition::Severed => None,
        BindDisposition::DivergentLanding { existing } => Some(BindAnomaly::DivergentLanding {
            existing_remote: format!("{existing:?}"),
        }),
        BindDisposition::ClaimedElsewhere { existing_mapping } => {
            Some(BindAnomaly::ClaimedElsewhere {
                claiming_mapping: existing_mapping,
            })
        }
        // A refused sever genuinely means the row was not bound, because a
        // sever that fails on the remote identity while the row is still
        // bound reports `SeverDiverged` instead.
        BindDisposition::Refused { state } | BindDisposition::SeverRefused { state } => {
            Some(BindAnomaly::Refused {
                binding_state: state,
            })
        }
        BindDisposition::SeverDiverged { existing } => Some(BindAnomaly::SeverDiverged {
            existing_remote: format!("{existing:?}"),
        }),
    }
}

const fn block_cause_name(cause: BlockCause) -> &'static str {
    match cause {
        BlockCause::Reauth => "reauth",
        BlockCause::Challenge => "challenge",
        BlockCause::Reconciliation => "reconciliation",
        BlockCause::RateGovernor => "rate-governor",
    }
}

async fn record_event(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeasedItem,
    payload: &JobEventPayload,
    at: Timestamp,
) -> Result<(), LedgerError> {
    ctx.ledger
        .record_event(&lease.lease_ref(), payload, at)
        .await
}

async fn record_action(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeasedItem,
    sequence: u32,
    label: &str,
    at: Timestamp,
) -> Result<(), LedgerError> {
    record_event(
        ctx,
        lease,
        &JobEventPayload::ItemActionStarted {
            sequence,
            label: label.to_owned(),
        },
        at,
    )
    .await
}

/// The adapter refused to render the projection, before any lease was spent
/// on a write.
///
/// The refusal is deterministic. `seed_from_projection` has one fallible
/// step, `project_fields`, which is a pure function of the projected listing;
/// that listing is rebuilt from the same durable `projection_edge` rows and
/// the same stored election answers on every lease, so an identical refusal
/// is what the next attempt gets. Abandoning it holds the organisation's one
/// live lease for the whole lease TTL, repeats for the attempt budget, and
/// then settles the item `Failed`/`Other` with no detail at all -- discarding
/// the one sentence the adapter composed that tells the seller which term
/// cannot be posted and why.
///
/// So it settles here exactly as the same `AdapterError::Rejected` settles
/// when it arrives one step later from a submit: the closed code and the
/// adapter's own text cross into the item verdict, `LeaseRepo::settle`
/// releases the lease and runs `settle_if_complete` for the job, and the
/// `ItemSettled` event lands. There is no attempt row to settle beside it,
/// because nothing was ever sent.
///
/// Every other seed error abandons, which is the stall bias unchanged. The
/// remaining `AdapterError` variants report a marketplace's answer to a
/// request, and a pure projection makes no request: none of them is
/// reachable from this step today, and were one to become so, waiting is the
/// conservative reading of an error whose determinism is not established.
pub async fn seed_refused(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeasedItem,
    error: &EngineError,
    at: Timestamp,
) -> Result<RunVerdict, EngineError> {
    let EngineError::Projection(AdapterError::Rejected { code, detail }) = error else {
        return Ok(RunVerdict::Abandoned {
            reason: format!("the seed failed and may yet succeed: {error:?}"),
        });
    };
    let verdict = ItemVerdict {
        outcome: ItemOutcome::Failed,
        failure_code: Some(*code),
        failure_detail: Some(detail.clone()),
    };
    ctx.ledger
        .settle_item(&lease.lease_ref(), &verdict, at)
        .await?;
    record_event(
        ctx,
        lease,
        &JobEventPayload::ItemSettled {
            outcome: format!("{:?}", ItemOutcome::Failed),
        },
        at,
    )
    .await?;
    Ok(RunVerdict::Settled(ItemOutcome::Failed))
}

async fn notify(
    ctx: &DriverContext<
        '_,
        impl MarketplaceAdapter,
        impl NowSource,
        impl Pause,
        impl ItemLedger,
        impl Cancellation,
        impl IdSource,
        impl ReconcileSource,
    >,
    lease: &LeasedItem,
    event: SellerEvent,
    at: Timestamp,
) -> Result<(), LedgerError> {
    ctx.ledger.notify(&lease.lease_ref(), event, at).await
}

#[cfg(test)]
mod tests {
    use super::{
        ambiguity_now, attested_intent, bind_anomaly, capture_label, intent_as_json,
        outcome_to_item,
    };
    use crate::vocabulary::{Attestation, BindDisposition};
    use tam_domain::{CaptureCause, Input, ItemOperation, ItemOutcome, SyncState};
    use tam_marketplace::{
        AdapterError, AmbiguityCause, FieldSet, Outcome, RemoteListingId, WriteAttemptId,
    };
    use tam_types::{
        BindAnomaly, CopyFormat, FailureCode, FailureDetail, FieldKey, MappingId, Uuid,
    };

    /// The attestation reaches the recorded body and never the hashed intent.
    ///
    /// The hash feeds the idempotency key, so an attestation folded into it
    /// would make a re-attested create a different create — a second listing
    /// for the same product, which is the one failure that key exists to
    /// prevent. This is the before-and-after hash in one body rather than a
    /// pinned literal, so it fails on a change to either side.
    #[test]
    fn an_attestation_reaches_the_recorded_body_and_never_the_hashed_intent() {
        let operation = ItemOperation::Create;
        let fields = FieldSet {
            entries: vec![(FieldKey::Title, "Fractions".to_owned())],
            files: vec![],
            cover: None,
            body_format: Some(CopyFormat::Html),
            appropriate_for_country: None,
        };
        let attestation = Attestation {
            attested_by: "the seller".to_owned(),
            attested_at_ms: 1_756_000_000_000,
        };

        let hashed = intent_as_json(&operation, &fields);
        let recorded = attested_intent(&operation, &fields, Some(&attestation));
        assert_eq!(
            recorded
                .get("attested_by")
                .and_then(serde_json::Value::as_str),
            Some("the seller"),
            "the body records who attested, which is the whole point of carrying it"
        );
        assert!(
            hashed.get("attested_by").is_none(),
            "and the hashed intent does not, because the hash is the identity of what was \
             written rather than of who said it was theirs"
        );

        let mut stripped = recorded;
        let object = stripped
            .as_object_mut()
            .expect("the intent body is an object");
        object.remove("attested_by");
        object.remove("attested_at_ms");
        assert_eq!(
            blake3::hash(stripped.to_string().as_bytes()),
            blake3::hash(hashed.to_string().as_bytes()),
            "the body is the hashed intent plus the attestation and nothing else: this is the \
             before-and-after hash, and it fails the moment either side moves"
        );
        assert_eq!(
            attested_intent(&operation, &fields, None),
            hashed,
            "and a marketplace that asks for no attestation records exactly what it hashes"
        );
    }

    #[test]
    fn a_rejection_carries_its_detail_into_the_verdict() {
        let detail = FailureDetail("the upload was refused".to_owned());
        let verdict = outcome_to_item(&Outcome::Rejected {
            code: FailureCode::UploadRejected,
            detail: detail.clone(),
        });
        assert_eq!(
            verdict.outcome,
            ItemOutcome::Failed,
            "a rejection settles the item failed"
        );
        assert_eq!(
            verdict.failure_code,
            Some(FailureCode::UploadRejected),
            "the closed-enum code crosses with it"
        );
        assert_eq!(
            verdict.failure_detail,
            Some(detail),
            "the adapter's free text must survive the crossing into the ledger"
        );
    }

    #[test]
    fn an_outcome_with_no_adapter_text_leaves_the_detail_empty() {
        let verdict = outcome_to_item(&Outcome::Skipped {
            code: FailureCode::RateLimited,
        });
        assert_eq!(
            verdict.outcome,
            ItemOutcome::Skipped,
            "a skip settles the item skipped"
        );
        assert_eq!(
            verdict.failure_code,
            Some(FailureCode::RateLimited),
            "a skip still names its code"
        );
        assert_eq!(
            verdict.failure_detail, None,
            "no detail is written where the adapter supplied none"
        );
    }

    #[test]
    fn the_dispositions_that_recorded_the_landing_leave_the_ledger_silent() {
        for disposition in [
            BindDisposition::Bound,
            BindDisposition::AlreadyBound,
            BindDisposition::NotLanded,
            BindDisposition::Addressed,
            BindDisposition::Severed,
        ] {
            assert_eq!(
                bind_anomaly(disposition.clone()),
                None,
                "{disposition:?} left nothing unrecorded, so it writes no event"
            );
        }
    }

    #[test]
    fn every_landing_the_mapping_does_not_record_reaches_the_ledger() {
        let url = "https://www.tes.com/teaching-resource/fractions-9001";
        assert_eq!(
            bind_anomaly(BindDisposition::DivergentLanding {
                existing: RemoteListingId::Tes {
                    url: url.to_owned(),
                },
            }),
            Some(BindAnomaly::DivergentLanding {
                existing_remote: format!(
                    "{:?}",
                    RemoteListingId::Tes {
                        url: url.to_owned()
                    }
                ),
            }),
            "a divergent landing carries the identifier the mapping already holds"
        );
        let claimant = MappingId(Uuid([0x31; 16]));
        assert_eq!(
            bind_anomaly(BindDisposition::ClaimedElsewhere {
                existing_mapping: claimant,
            }),
            Some(BindAnomaly::ClaimedElsewhere {
                claiming_mapping: claimant,
            }),
            "a cross-mapping claim names the mapping holding the listing"
        );
        assert_eq!(
            bind_anomaly(BindDisposition::Refused {
                state: "severed".to_owned(),
            }),
            Some(BindAnomaly::Refused {
                binding_state: "severed".to_owned(),
            }),
            "a refused bind carries the state the fence was refused against"
        );
        assert_eq!(
            bind_anomaly(BindDisposition::SeverRefused {
                state: "unbound".to_owned(),
            }),
            Some(BindAnomaly::Refused {
                binding_state: "unbound".to_owned(),
            }),
            "a refused sever genuinely means the row was not bound, so Refused is correct here"
        );
        assert_eq!(
            bind_anomaly(BindDisposition::SeverDiverged {
                existing: RemoteListingId::Tes {
                    url: url.to_owned(),
                },
            }),
            Some(BindAnomaly::SeverDiverged {
                existing_remote: format!(
                    "{:?}",
                    RemoteListingId::Tes {
                        url: url.to_owned()
                    }
                ),
            }),
            "a removal that took down a listing the mapping no longer holds names the one it \
             does, which a refusal against the binding state could not say without \
             contradicting itself"
        );
    }

    /// The capture label names the ambiguity, which is the line an operator
    /// reads first.
    ///
    /// The label was `capture:Ambiguity:Some(WriteAttemptId(..))` — a class
    /// and a row id, with nothing saying which of the five ambiguities it
    /// was. Pinned as a literal deliberately: this string is read by a human
    /// out of `job_event`, so its shape is the contract, and the `unknown`
    /// row is here because a schema-drift capture carries no ambiguity at
    /// all and must still produce a four-field label.
    #[test]
    fn the_capture_label_names_which_ambiguity_it_was() {
        let attempt = WriteAttemptId(Uuid([7; 16]));
        assert_eq!(
            capture_label(
                CaptureCause::Ambiguity,
                Some(AmbiguityCause::ResponseEventLost),
                Some(attempt),
            ),
            format!("capture:Ambiguity:response_event_lost:{:?}", Some(attempt)),
        );
        assert_eq!(
            capture_label(CaptureCause::SchemaDrift, None, None),
            "capture:SchemaDrift:unknown:None",
            "a capture with no ambiguity says so rather than dropping the field"
        );
    }

    /// An ambiguous item's own row states the cause too.
    ///
    /// The attempt row is the operator's record and the item row is the
    /// seller-facing one; both were blank on this path, so the failure
    /// surface showed an ambiguous item with no code at all.
    #[test]
    fn an_ambiguous_item_carries_its_cause_in_the_detail() {
        let verdict = outcome_to_item(&Outcome::Ambiguous {
            attempt: tam_types::AttemptId(Uuid([9; 16])),
            cause: AmbiguityCause::SubmitTimedOut,
            evidence: tam_marketplace::EvidenceRef("none".to_owned()),
        });
        assert_eq!(verdict.outcome, ItemOutcome::Ambiguous);
        assert_eq!(verdict.failure_code, Some(FailureCode::Other));
        let detail = verdict.failure_detail.expect("an ambiguity states itself");
        assert!(
            detail.0.contains("submit_timed_out"),
            "the item's detail spells the cause the way the ledger column spells it: {detail:?}"
        );
    }

    /// The newest answer wins, and a terminal outcome outranks what the run
    /// remembered.
    ///
    /// The capture effect is raised before, with and after the answer that
    /// explains it depending on which transition emitted it, so the
    /// precedence is the whole of whether the label says anything true.
    #[test]
    fn the_recorded_ambiguity_prefers_the_newest_answer() {
        let pending = Input::ReadBackResult(Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )));
        assert_eq!(
            ambiguity_now(
                Some(AmbiguityCause::SubmitTimedOut),
                Some(&pending),
                &SyncState::AwaitingPreflight,
            ),
            Some(AmbiguityCause::ReadBackIndeterminate),
            "the answer about to be stepped is newer than the one remembered"
        );
        assert_eq!(
            ambiguity_now(
                Some(AmbiguityCause::SubmitTimedOut),
                None,
                &SyncState::Terminal(Outcome::Ambiguous {
                    attempt: tam_types::AttemptId(Uuid([3; 16])),
                    cause: AmbiguityCause::NoDurableIdentifier,
                    evidence: tam_marketplace::EvidenceRef("none".to_owned()),
                }),
            ),
            Some(AmbiguityCause::NoDurableIdentifier),
            "a settled outcome's own cause is the record, not the run's memory of the submit"
        );
        assert_eq!(
            ambiguity_now(
                Some(AmbiguityCause::SubmitTimedOut),
                Some(&Input::ChallengeCleared),
                &SyncState::AwaitingPreflight,
            ),
            Some(AmbiguityCause::SubmitTimedOut),
            "an input that answers no ambiguity does not erase the one already observed"
        );
    }
}
