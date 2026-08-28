//! The effect interpreter: pumps one leased item through the machine,
//! executing each effect in order and feeding the result back as the next
//! input. Every ledger write carries the lease epoch; time enters as data
//! through the clock seam; and every unhandleable condition abandons the
//! item so the lease expires and the stealer requeues it — the stall bias,
//! never an invented outcome.

use serde_json::json;
use tam_domain::{
    BlockCause, Effect, HaltScope, Input, ItemOperation, ItemOutcome, MachineError, SellerEvent,
    StepBudget, SyncMachine, SyncState, Transition,
};
use tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX;
use tam_marketplace::{
    AdapterError, CreateStrategy, FetchReason, FieldSet, FormId, ListingLocator, ListingState,
    MarketplaceAdapter, ObservedListing, Outcome, Pause, RemoteLifecycle, RemoteListingId,
    RemovalPlan, RevisePlan, WriteAttemptId,
};
use tam_storage::{
    append_event, AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant,
    EventScope, HaltCause, HaltRepo, ItemVerdict, LandingEffect, LeaseRef, LeaseRepo, LeasedItem,
    NewOutboxMessage, OutboxRepo, RateBudgetRepo, StorageError, WriteAttemptRepo,
};
use tam_types::{
    BindAnomaly, ConnectionId, ContentHash, FailureCode, JobEventPayload, LogicalInstant, OrgId,
    Timestamp, Uuid,
};
use tokio_util::sync::CancellationToken;

/// The clock seam: the binaries read the wall clock at this boundary
/// (expect-attributed there); tests hand instants in.
pub trait NowSource: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// How long the driver polls a marketplace's own read for a write it just
/// made. Held here rather than in the machine, which is pure and holds no
/// clock, and produced per inventory by the seed.
///
/// The budget has to fit inside the lease, or the item is stolen mid-run and
/// the epoch-fenced attempt settle fails *after* a listing has landed:
///
/// ```text
/// submit_worst_case + tries * interval_ms < LEASE_TTL_SECS
/// ```
///
/// On today's numbers that holds for the measured Tpt create — roughly 180s
/// against a 300s lease, with 22s of poll on top — and fails at that
/// platform's theoretical worst case, where two queue-job polls alone can
/// spend 360s. That is a pre-existing hazard the poll narrows the margin on
/// rather than one it creates, and Phase 3 asserts the inequality rather than
/// raising a limit to hide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub struct DriverContext<'a, A, N, P> {
    pub adapter: &'a A,
    pub leases: &'a LeaseRepo,
    pub halts: &'a HaltRepo,
    pub attempts: &'a WriteAttemptRepo,
    pub budgets: &'a RateBudgetRepo,
    pub pool: &'a sqlx::PgPool,
    pub clock: &'a N,
    pub cancel: &'a CancellationToken,
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
    Storage(StorageError),
    Machine(MachineError),
    /// The adapter refused to render the projection into its own wire
    /// shape: the item is unrepresentable on this marketplace, so its lease
    /// is left to expire rather than settled.
    Projection(AdapterError),
}

impl From<StorageError> for EngineError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
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
            Self::Storage(error) => write!(f, "storage: {error}"),
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
pub(crate) fn intent_as_json(operation: &ItemOperation, fields: &FieldSet) -> serde_json::Value {
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
        ItemOperation::Remove { subject, state } => json!({
            "operation": "remove",
            "subject": subject_as_json(subject),
            "state": state_name(*state),
        }),
    }
}

fn outcome_to_item(outcome: &Outcome) -> ItemVerdict {
    let (item_outcome, failure_code, failure_detail) = match outcome {
        Outcome::Committed { .. } => (ItemOutcome::Succeeded, None, None),
        Outcome::Degraded { .. } => (ItemOutcome::Degraded, None, None),
        Outcome::Rejected { code, detail } => {
            (ItemOutcome::Failed, Some(*code), Some(detail.clone()))
        }
        Outcome::Ambiguous { .. } => (ItemOutcome::Ambiguous, None, None),
        Outcome::Blocked { .. } => (ItemOutcome::Blocked, None, None),
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
        (ItemOperation::Create | ItemOperation::Revise { .. }, Some(id)) => {
            observed.map_or(LandingEffect::None, |lifecycle| LandingEffect::Landed {
                id: id.clone(),
                lifecycle: lifecycle.clone(),
            })
        }
        (ItemOperation::Create, None) => LandingEffect::None,
        (ItemOperation::Revise { subject, .. } | ItemOperation::Remove { subject, .. }, None) => {
            LandingEffect::Addressed {
                id: subject.clone(),
            }
        }
    }
}

/// When the verification read has answered, per operation — mirroring what
/// the live runners learned. A create and a revise-to-draft are proved by
/// finding the listing, a publish by finding it live, and a removal by not
/// finding it.
fn verification_settles(operation: &ItemOperation, observed: &ObservedListing) -> bool {
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

/// Three answers, because evidence about the listing and evidence about us
/// settle differently.
enum VerifyOutcome {
    /// The predicate accepted, the read returned a condition the machine has
    /// an arm for, or the try budget ran out. Either way this is evidence
    /// about the listing, and the machine settles on it.
    Observed(Result<ObservedListing, AdapterError>),
    /// The per-connection rate window closed before the predicate could be
    /// answered. The write may have landed; we simply stopped looking.
    RateWindowClosed,
    /// The wall clock or the cancellation token ended the run.
    Cut,
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
/// `Input::BudgetExhausted`: in `AwaitingReadBack` that input is
/// `InputNotApplicable`, so it would crash the run instead of settling it.
async fn verify_with_backoff<A: MarketplaceAdapter, N: NowSource, P: Pause>(
    ctx: &DriverContext<'_, A, N, P>,
    lease: &LeasedItem,
    request: Verification<'_>,
) -> Result<VerifyOutcome, EngineError> {
    let ceiling = i32::try_from(OUTBOUND_REQUESTS_PER_MINUTE_MAX.get()).unwrap_or(i32::MAX);
    let mut last: Option<Result<ObservedListing, AdapterError>> = None;
    for attempted in 0..request.policy.tries {
        let now = ctx.clock.now();
        if ctx.cancel.is_cancelled() || now.0 >= request.deadline {
            return Ok(VerifyOutcome::Cut);
        }
        // Recomputed on every read: `consume` is keyed on the window start,
        // so one window carried across a poll that straddles a minute
        // boundary keeps incrementing a bucket it has already left.
        let window = Timestamp(now.0 - now.0.rem_euclid(60_000));
        let grant = ctx
            .budgets
            .consume(lease.org, request.connection, window, ceiling)
            .await?;
        if grant == BudgetGrant::Exhausted {
            return Ok(VerifyOutcome::RateWindowClosed);
        }
        let observed = ctx
            .adapter
            .read_back(
                lease.org,
                request.locator.clone(),
                request.reason.clone(),
                now,
            )
            .await;
        match &observed {
            // Every `AdapterError` is a condition rather than lag —
            // absence became an observation precisely so that lag is not one
            // — and the machine has an arm for each, so polling through one
            // would only spend the budget.
            Err(_) => return Ok(VerifyOutcome::Observed(observed)),
            Ok(listing) if verification_settles(request.operation, listing) => {
                return Ok(VerifyOutcome::Observed(observed))
            }
            Ok(_) => {}
        }
        last = Some(observed);
        if attempted + 1 < request.policy.tries {
            ctx.pause.pause(request.policy.interval_ms).await;
        }
    }
    // A policy of zero tries never looked, which is the one case with no
    // answer to hand over; the stall bias reports it as a cut run.
    Ok(last.map_or(VerifyOutcome::Cut, VerifyOutcome::Observed))
}

const fn outcome_to_attempt_state(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Committed { .. } | Outcome::Degraded { .. } => "committed",
        Outcome::Rejected { .. } => "failed",
        Outcome::Ambiguous { .. } => "ambiguous",
        Outcome::Blocked { .. } | Outcome::Skipped { .. } => "abandoned",
    }
}

/// The seven design topics are fixed; the parked-job mail is also the reauth
/// prompt, and a halt notice rides the settled-job topic until the design
/// grows a dedicated one.
const fn seller_event_topic(event: SellerEvent) -> &'static str {
    match event {
        SellerEvent::ReauthRequired | SellerEvent::ItemParked => "email.parked_job",
        SellerEvent::JobSettled | SellerEvent::InventoryHalted => "email.job_settled",
    }
}

/// Drives one leased item to a terminal state, a park, or abandonment.
#[expect(
    clippy::too_many_lines,
    reason = "the effect loop is one cohesive interpreter; the lint is advisory here by charter"
)]
pub async fn run_item<A: MarketplaceAdapter, N: NowSource, P: Pause>(
    ctx: &DriverContext<'_, A, N, P>,
    lease: &LeasedItem,
    seed: MachineSeed,
) -> Result<RunVerdict, EngineError> {
    let org = lease.org;
    let lease_ref = lease.lease_ref();
    let Some(connection) = ctx.leases.connection_for(org, lease.inventory).await? else {
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
        if ctx.cancel.is_cancelled() || now.0 >= wall_deadline {
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
                    let asserted = ctx.adapter.assert_form_schema(org, form).await;
                    pending = Some(match asserted {
                        Ok(fingerprint) => Input::PreflightResult(Ok(fingerprint)),
                        Err(AdapterError::SchemaDrift(drift)) => {
                            Input::PreflightResult(Err(*drift))
                        }
                        Err(error) => {
                            return Ok(RunVerdict::Abandoned {
                                reason: format!("preflight failed transiently: {error:?}"),
                            })
                        }
                    });
                }
                Effect::RecordIntent { intent_hash } => {
                    let attempt = ctx
                        .attempts
                        .open(
                            &lease_ref,
                            lease.mapping,
                            &AttemptIntent {
                                body: intent_as_json(&operation, &next.fields),
                                hash: intent_hash.0.to_vec(),
                            },
                            now,
                        )
                        .await;
                    match attempt {
                        Ok(attempt) => {
                            let attempt = WriteAttemptId(attempt);
                            current_attempt = Some(attempt);
                            pending = Some(Input::IntentRecorded(attempt));
                        }
                        Err(StorageError::AttemptInFlight) => {
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
                    let grant = consume_write_grant(ctx, org, connection, now).await?;
                    if grant == BudgetGrant::Exhausted {
                        return rate_refused_before_the_write(
                            ctx,
                            &lease_ref,
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            now,
                        )
                        .await;
                    }
                    record_action(ctx, lease, sequence, "submit", now).await?;
                    let submitted = ctx.adapter.submit(org, key, fields, now).await;
                    pending = Some(Input::SubmitResult(submitted));
                }
                Effect::Revise {
                    attempt,
                    subject,
                    fields,
                    transition: lifecycle,
                } => {
                    let grant = consume_write_grant(ctx, org, connection, now).await?;
                    if grant == BudgetGrant::Exhausted {
                        return rate_refused_before_the_write(
                            ctx,
                            &lease_ref,
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            now,
                        )
                        .await;
                    }
                    record_action(ctx, lease, sequence, "revise", now).await?;
                    let revised = ctx
                        .adapter
                        .revise(
                            org,
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
                    let grant = consume_write_grant(ctx, org, connection, now).await?;
                    if grant == BudgetGrant::Exhausted {
                        return rate_refused_before_the_write(
                            ctx,
                            &lease_ref,
                            AttemptRef {
                                attempt: attempt.0,
                                mapping: lease.mapping,
                            },
                            now,
                        )
                        .await;
                    }
                    record_action(ctx, lease, sequence, "remove", now).await?;
                    let removed = ctx
                        .adapter
                        .remove(
                            org,
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
                        VerifyOutcome::RateWindowClosed | VerifyOutcome::Cut => {
                            let stopped_by = match verified {
                                VerifyOutcome::RateWindowClosed => {
                                    "the per-connection rate window closed"
                                }
                                VerifyOutcome::Observed(_) | VerifyOutcome::Cut => {
                                    "the run was cut"
                                }
                            };
                            if let Some(attempt) = current_attempt {
                                settle_open_attempt(
                                    ctx,
                                    &lease_ref,
                                    AttemptRef {
                                        attempt: attempt.0,
                                        mapping: lease.mapping,
                                    },
                                    "ambiguous",
                                    now,
                                )
                                .await?;
                            }
                            return Ok(RunVerdict::Abandoned {
                                reason: format!("{stopped_by} before the write could be verified"),
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
                    ctx.leases
                        .park(&lease_ref, &format!("{challenge:?}"), Timestamp(expires.0))
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
                    ctx.leases.gate_connection(org, lease.inventory).await?;
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
                    let cause = HaltCause {
                        raised_by: "machine".to_owned(),
                        reason: "the transition table demanded a halt".to_owned(),
                        at: now,
                    };
                    match scope {
                        HaltScope::OrgInventory { org, inventory } => {
                            ctx.halts
                                .raise_org_inventory(org, inventory, &cause)
                                .await?;
                        }
                        HaltScope::Org { org } => ctx.halts.raise_org(org, &cause).await?,
                        HaltScope::FleetInventory { inventory } => {
                            ctx.halts.raise_fleet_inventory(inventory, &cause).await?;
                        }
                    }
                }
                Effect::Notify { org, event } => {
                    notify(ctx, org, lease, event, now).await?;
                }
            }
        }

        match (&next.state, pending) {
            (SyncState::Terminal(outcome), _) => {
                let now = ctx.clock.now();
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
                        .attempts
                        .settle(
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
                let item_settled = ctx.leases.settle(&lease_ref, &verdict, now).await;
                if severed && matches!(item_settled, Err(StorageError::StaleLease)) {
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

/// One grant against the connection's declared per-minute ceiling, with the
/// window recomputed from the clock at the moment of the call.
///
/// What the ceiling bounds is *effects* rather than requests, and always
/// has: one submit issues a create, a metadata write, a three-step upload per
/// file and a state read, and consumes one grant for all of it. So this is a
/// lower bound on outbound traffic rather than a count of it. Making it exact
/// means consuming inside the transport seam, which is recorded as founder-
/// gated rather than assumed here.
async fn consume_write_grant(
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource, impl Pause>,
    org: OrgId,
    connection: ConnectionId,
    now: Timestamp,
) -> Result<BudgetGrant, EngineError> {
    let window = Timestamp(now.0 - now.0.rem_euclid(60_000));
    let ceiling = i32::try_from(OUTBOUND_REQUESTS_PER_MINUTE_MAX.get()).unwrap_or(i32::MAX);
    Ok(ctx
        .budgets
        .consume(org, connection, window, ceiling)
        .await?)
}

/// The rate window closed before a lifecycle write was called. Nothing was
/// sent, so the attempt settles `'abandoned'` exactly as the submit path's
/// refusal does, and the run abandons into the stealer.
async fn rate_refused_before_the_write(
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource, impl Pause>,
    lease_ref: &LeaseRef,
    settling: AttemptRef,
    at: Timestamp,
) -> Result<RunVerdict, EngineError> {
    settle_open_attempt(ctx, lease_ref, settling, "abandoned", at).await?;
    Ok(RunVerdict::Abandoned {
        reason: "the per-connection rate window is exhausted".to_owned(),
    })
}

/// Settles an attempt the run is about to walk away from, so abandoning does
/// not strand an `in_flight` row on the mapping.
///
/// `expire_and_steal` requeues the item and bumps `job_item.lease_epoch` but
/// settles no orphan attempt, and `write_attempt_one_in_flight` admits one
/// open attempt per mapping — so a re-leased item whose predecessor left one
/// standing abandons on `AttemptInFlight` at `RecordIntent` on every pass,
/// burning its whole retry allowance without another request leaving the
/// process.
///
/// `state` is the whole difference between the two callers: `'abandoned'`
/// where the refusal came before the write was called and nothing was sent,
/// `'ambiguous'` where the write went out and the verification could not
/// finish. The mapping is left alone either way, which `LandingEffect::None`
/// records honestly.
async fn settle_open_attempt(
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource, impl Pause>,
    lease: &LeaseRef,
    settling: AttemptRef,
    state: &str,
    at: Timestamp,
) -> Result<(), EngineError> {
    let verdict = AttemptVerdict {
        state: state.to_owned(),
        failure_code: None,
        landing: LandingEffect::None,
    };
    match ctx.attempts.settle(lease, settling, &verdict, at).await {
        // A fenced settle means the lease was stolen, and the steal owns the
        // story from here — the same reading the terminal path already takes.
        Ok(_) | Err(StorageError::StaleLease) => Ok(()),
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
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource, impl Pause>,
    lease: &LeasedItem,
    payload: &JobEventPayload,
    at: Timestamp,
) -> Result<(), StorageError> {
    let mut tx = ctx.pool.begin().await?;
    append_event(
        &mut tx,
        &EventScope {
            org: lease.org,
            job: lease.job,
            item: Some(lease.item),
        },
        payload,
        at,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn record_action(
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource, impl Pause>,
    lease: &LeasedItem,
    sequence: u32,
    label: &str,
    at: Timestamp,
) -> Result<(), StorageError> {
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

async fn notify(
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource, impl Pause>,
    org: OrgId,
    lease: &LeasedItem,
    event: SellerEvent,
    at: Timestamp,
) -> Result<(), StorageError> {
    let mut tx = ctx.pool.begin().await?;
    tam_storage::OutboxRepo::append(
        &mut tx,
        &NewOutboxMessage {
            org,
            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
            topic: seller_event_topic(event).to_owned(),
            dedupe_key: format!("{event:?}:{:02x?}", lease.item.0 .0),
            payload: json!({ "event": format!("{event:?}") }),
            at,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Silences the unused-import warning path for OutboxRepo's inherent fn use.
const _: fn(sqlx::PgPool) -> OutboxRepo = OutboxRepo::new;

#[cfg(test)]
mod tests {
    use super::{bind_anomaly, outcome_to_item};
    use crate::seed::verify_policy;
    use tam_domain::ItemOutcome;
    use tam_marketplace::{Outcome, RemoteListingId};
    use tam_storage::BindDisposition;
    use tam_types::{BindAnomaly, FailureCode, FailureDetail, InventoryId, MappingId, Uuid};

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

    /// The lease TTL the worker leases with. Mirrored rather than imported:
    /// it is a binary's constant and `tam-engine` is a library, so the
    /// inequality is asserted where `VerifyPolicy` lives and this value is
    /// kept in step with `crates/tam-worker/src/main.rs` by hand.
    const LEASE_TTL_SECS: i64 = 300;

    /// The slowest Tpt create actually measured, recorded as "nearly three
    /// minutes" at `crates/tam-marketplace-tpt/src/flows.rs`. The theoretical
    /// worst case is larger — two queue-job polls at `QUEUE_POLL_MAX` can
    /// spend 360s inside `submit` alone — and exceeds the lease before any
    /// poll is added. That is a pre-existing hazard recorded for the founder,
    /// not one this poll creates and not one Phase 3 hides by raising a
    /// limit.
    const MEASURED_SUBMIT_WORST_CASE_MS: u64 = 180_000;

    #[test]
    fn the_verification_poll_fits_inside_the_lease() {
        let lease_ms = u64::try_from(LEASE_TTL_SECS * 1_000).unwrap_or(u64::MAX);
        for inventory in [
            InventoryId::TesGb,
            InventoryId::TesUs,
            InventoryId::TesNz,
            InventoryId::Etsy,
            InventoryId::Tpt,
        ] {
            let spent = MEASURED_SUBMIT_WORST_CASE_MS + verify_policy(inventory).window_ms();
            assert!(
                spent < lease_ms,
                "{inventory:?}: submit_worst_case + tries * interval is {spent}ms against a \
                 {lease_ms}ms lease; when the lease expires mid-run the epoch-fenced attempt \
                 settle fails after a listing has landed, and the mapping never binds"
            );
        }
    }
}
