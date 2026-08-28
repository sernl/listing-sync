//! The effect interpreter: pumps one leased item through the machine,
//! executing each effect in order and feeding the result back as the next
//! input. Every ledger write carries the lease epoch; time enters as data
//! through the clock seam; and every unhandleable condition abandons the
//! item so the lease expires and the stealer requeues it — the stall bias,
//! never an invented outcome.

use serde_json::json;
use tam_domain::{
    BlockCause, Effect, HaltScope, Input, ItemOutcome, MachineError, SellerEvent, StepBudget,
    SyncMachine, SyncState, Transition,
};
use tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX;
use tam_marketplace::{
    AdapterError, CreateStrategy, FieldSet, FormId, MarketplaceAdapter, Outcome, RemoteListingId,
    WriteAttemptId,
};
use tam_storage::{
    append_event, AttemptIntent, AttemptRef, AttemptVerdict, BudgetGrant, EventScope, HaltCause,
    HaltRepo, ItemVerdict, LeaseRepo, LeasedItem, NewOutboxMessage, OutboxRepo, RateBudgetRepo,
    StorageError, WriteAttemptRepo,
};
use tam_types::{
    ContentHash, FailureCode, JobEventPayload, LogicalInstant, OrgId, Timestamp, Uuid,
};
use tokio_util::sync::CancellationToken;

/// The clock seam: the binaries read the wall clock at this boundary
/// (expect-attributed there); tests hand instants in.
pub trait NowSource: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The machine's construction data the ledger does not hold: the projected
/// field set and its hash, the form, and the per-inventory strategy. The
/// projection (M1g) and the scheduler wiring (M1j) will own producing this;
/// until then the worker builds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSeed {
    pub form: FormId,
    pub fields: FieldSet,
    pub intent_hash: ContentHash,
    pub strategy: CreateStrategy,
    pub budget: StepBudget,
}

pub struct DriverContext<'a, A, N> {
    pub adapter: &'a A,
    pub leases: &'a LeaseRepo,
    pub halts: &'a HaltRepo,
    pub attempts: &'a WriteAttemptRepo,
    pub budgets: &'a RateBudgetRepo,
    pub pool: &'a sqlx::PgPool,
    pub clock: &'a N,
    pub cancel: &'a CancellationToken,
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

impl core::fmt::Display for EngineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "storage: {error}"),
            Self::Machine(error) => write!(f, "machine: {error:?}"),
        }
    }
}

impl core::error::Error for EngineError {}

fn fields_as_json(fields: &FieldSet) -> serde_json::Value {
    json!({
        "entries": serde_json::to_value(&fields.entries).unwrap_or(serde_json::Value::Null),
        "files": serde_json::to_value(&fields.files).unwrap_or(serde_json::Value::Null),
    })
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

/// The listing a committed-class outcome landed on. Only the two outcomes
/// that carry a receipt can state one; every other outcome settled without a
/// write that landed, so there is nothing to record.
const fn outcome_to_landed(outcome: &Outcome) -> Option<&RemoteListingId> {
    match outcome {
        Outcome::Committed { receipt, .. } | Outcome::Degraded { receipt, .. } => {
            Some(receipt.listing())
        }
        Outcome::Rejected { .. }
        | Outcome::Ambiguous { .. }
        | Outcome::Blocked { .. }
        | Outcome::Skipped { .. } => None,
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
pub async fn run_item<A: MarketplaceAdapter, N: NowSource>(
    ctx: &DriverContext<'_, A, N>,
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
        seed.budget,
    )?;
    let mut current_attempt: Option<WriteAttemptId> = None;
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
                                body: fields_as_json(&next.fields),
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
                    attempt: _,
                    key,
                    fields,
                } => {
                    let window = Timestamp(now.0 - now.0.rem_euclid(60_000));
                    let ceiling =
                        i32::try_from(OUTBOUND_REQUESTS_PER_MINUTE_MAX.get()).unwrap_or(i32::MAX);
                    let grant = ctx
                        .budgets
                        .consume(org, connection, window, ceiling)
                        .await?;
                    if grant == BudgetGrant::Exhausted {
                        return Ok(RunVerdict::Abandoned {
                            reason: "the per-connection rate window is exhausted".to_owned(),
                        });
                    }
                    record_action(ctx, lease, sequence, "submit", now).await?;
                    let submitted = ctx.adapter.submit(org, key, fields).await;
                    pending = Some(Input::SubmitResult(submitted));
                }
                Effect::ReadBack { locator, reason } => {
                    record_action(ctx, lease, sequence, "read-back", now).await?;
                    let observed = ctx.adapter.read_back(org, locator, reason, now).await;
                    pending = Some(Input::ReadBackResult(observed));
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
                if let Some(attempt) = current_attempt {
                    // Best-effort: a fenced-out attempt settle means the item
                    // was stolen, and the steal owns the story from here.
                    let attempt_verdict = AttemptVerdict {
                        state: outcome_to_attempt_state(outcome).to_owned(),
                        failure_code: verdict.failure_code,
                        landed: outcome_to_landed(outcome).cloned(),
                    };
                    if let Err(error) = ctx
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
                        .await
                    {
                        return Ok(RunVerdict::Abandoned {
                            reason: format!("the attempt settle was fenced: {error}"),
                        });
                    }
                }
                ctx.leases.settle(&lease_ref, &verdict, now).await?;
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

const fn block_cause_name(cause: BlockCause) -> &'static str {
    match cause {
        BlockCause::Reauth => "reauth",
        BlockCause::Challenge => "challenge",
        BlockCause::Reconciliation => "reconciliation",
        BlockCause::RateGovernor => "rate-governor",
    }
}

async fn record_event(
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource>,
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
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource>,
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
    ctx: &DriverContext<'_, impl MarketplaceAdapter, impl NowSource>,
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
    use super::outcome_to_item;
    use tam_domain::ItemOutcome;
    use tam_marketplace::Outcome;
    use tam_types::{FailureCode, FailureDetail};

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
}
