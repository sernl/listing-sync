//! The effect interpreter: pumps one leased item through the machine,
//! executing each effect in order and feeding the result back as the next
//! input. Every ledger write carries the lease epoch; time enters as data
//! through the clock seam; and every unhandleable condition abandons the
//! item so the lease expires and the stealer requeues it — the stall bias,
//! never an invented outcome.

use serde_json::json;
use tam_domain::{
    attempt_budget_spent, seller_clears, verification_settles, BlockCause, Effect, Input,
    ItemOperation, ItemOutcome, MachineError, SellerEvent, StepBudget, SyncMachine, SyncState,
    Transition,
};
use tam_marketplace::{
    AdapterError, ChallengeKind, CreateStrategy, FetchReason, FieldSet, FormId, ListingLocator,
    ListingState, MarketplaceAdapter, ObservedListing, Outcome, Pause, RemoteLifecycle,
    RemoteListingId, RemovalPlan, RevisePlan, WriteAttemptId,
};
use tam_types::{
    BindAnomaly, ConnectionId, ContentHash, FailureCode, JobEventPayload, LogicalInstant, Timestamp,
};

use crate::ports::{Cancellation, IdSource, ItemLedger};
use crate::vocabulary::{
    AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, GrantKind,
    ItemVerdict, LandingEffect, LeaseRef, LeasedItem, LedgerError, NewAttempt,
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
    pub form: FormId,
    pub fields: FieldSet,
    pub intent_hash: ContentHash,
    pub strategy: CreateStrategy,
    pub budget: StepBudget,
    pub verify: VerifyPolicy,
}

pub struct DriverContext<'a, A, N, P, L, C, I> {
    pub adapter: &'a A,
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
    /// The lease was deliberately left to expire; the stealer requeues with
    /// the epoch bumped. The stall bias as a value.
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

fn outcome_to_item(outcome: &Outcome) -> ItemVerdict {
    let (item_outcome, failure_code, failure_detail) = match outcome {
        Outcome::Committed { .. } => (ItemOutcome::Succeeded, None, None),
        Outcome::Degraded { .. } => (ItemOutcome::Degraded, None, None),
        Outcome::Rejected { code, detail } => {
            (ItemOutcome::Failed, Some(*code), Some(detail.clone()))
        }
        Outcome::Ambiguous { .. } => (ItemOutcome::Ambiguous, None, None),
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
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I>,
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

const fn outcome_to_attempt_state(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Committed { .. } | Outcome::Degraded { .. } => "committed",
        Outcome::Rejected { .. } => "failed",
        Outcome::Ambiguous { .. } => "ambiguous",
        Outcome::Blocked { .. } | Outcome::Skipped { .. } => "abandoned",
    }
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

/// Drives one leased item to a terminal state, a park, or abandonment.
#[expect(
    clippy::too_many_lines,
    reason = "the effect loop is one cohesive interpreter; the lint is advisory here by charter"
)]
pub async fn run_item<
    A: MarketplaceAdapter,
    N: NowSource,
    P: Pause,
    L: ItemLedger,
    C: Cancellation,
    I: IdSource,
>(
    ctx: &DriverContext<'_, A, N, P, L, C, I>,
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
    let mut current_attempt: Option<WriteAttemptId> = None;
    // What the verification read last saw, so a bind records the lifecycle
    // the listing was observed in rather than the one its create convention
    // would imply.
    let mut observed_lifecycle: Option<RemoteLifecycle> = None;
    let mut sequence: u32 = 0;

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
        if (ctx.cancel.is_cancelled() || now.0 >= wall_deadline)
            && !matches!(
                transition.next.state,
                SyncState::Terminal(_) | SyncState::Parked { .. }
            )
        {
            transition = transition
                .next
                .step(Input::BudgetExhausted, LogicalInstant(now.0))?;
        }
        let Transition { next, effects } = transition;
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
                                    body: intent_as_json(&operation, &next.fields),
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
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            &operation,
                            now,
                        )
                        .await;
                    }
                    ctx.ledger.renew(&lease_ref).await?;
                    record_action(ctx, lease, sequence, "submit", now).await?;
                    let submitted = ctx.adapter.submit(key, fields, now).await;
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
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            &operation,
                            now,
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
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            &operation,
                            now,
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
                Effect::Reconcile { .. } => {
                    // No adapter in this milestone can search by marker; the
                    // machine turns this refusal into the halting ambiguity,
                    // which is the stall bias doing its job.
                    pending = Some(Input::ReconcileResult(Err(AdapterError::Rejected {
                        code: FailureCode::Other,
                        detail: tam_types::FailureDetail(
                            "marker reconciliation is not implemented yet".to_owned(),
                        ),
                    })));
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
                    record_action(
                        ctx,
                        lease,
                        sequence,
                        &format!("capture:{cause:?}:{attempt:?}"),
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
            (_, Some(input)) => {
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
    >,
    lease_ref: &LeaseRef,
    settling: AttemptRef,
    operation: &ItemOperation,
    at: Timestamp,
) -> Result<RunVerdict, EngineError> {
    let verdict = AttemptVerdict {
        state: "abandoned".to_owned(),
        failure_code: None,
        landing: addressed_by(operation),
    };
    settle_open_attempt(ctx, lease_ref, settling, &verdict, at).await?;
    Ok(RunVerdict::Abandoned {
        reason: "the per-connection rate window is exhausted".to_owned(),
    })
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
    >,
    lease: &LeasedItem,
    event: SellerEvent,
    at: Timestamp,
) -> Result<(), LedgerError> {
    ctx.ledger.notify(&lease.lease_ref(), event, at).await
}

#[cfg(test)]
mod tests {
    use super::{bind_anomaly, outcome_to_item};
    use crate::vocabulary::BindDisposition;
    use tam_domain::ItemOutcome;
    use tam_marketplace::{Outcome, RemoteListingId};
    use tam_types::{BindAnomaly, FailureCode, FailureDetail, MappingId, Uuid};

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
}
