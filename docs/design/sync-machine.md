# The sync state machine

This document is the transition specification for `SyncMachine`, which the design specification summarises and does not enumerate.
Every type named here is defined in [`sketches/domain.rs`](sketches/domain.rs), which is the artefact of record and compiles clean under `rustc 1.97.1` with `-D warnings`.
The machine is pure, deterministic and total: it performs no I/O, allocates no runtime, and its crate bans `tokio`, `tokio-util`, `reqwest` and `sqlx` from its dependency tree.
Everything it wants done is returned as data in `EffectList`, and the worker outside it decides how to do it.

## The vocabulary, in one place

An agent implementing this machine needs every identifier resolved before it starts, so they are gathered here rather than scattered through prose.

| Identifier | Kind | What it is |
|---|---|---|
| `SyncState` | enum | The eight states below |
| `Input` | enum | The nine things that can happen to the machine |
| `Effect` | enum | The ten things the machine can ask for |
| `EffectList` | newtype | An ordered `Vec<Effect>`, executed in order |
| `Transition` | struct | The next machine plus its effects |
| `MachineError` | enum | `InputNotApplicable`, `AttemptMismatch`, `EffectBudgetExceeded`, `UnloweredPublish`, `ResumeNotACreate` |
| `LogicalInstant` | newtype | A clock reading passed in, never read by the machine |
| `Outcome` | enum | The six terminal answers a submit can reach |
| `FailureCode` | enum | The closed, versioned failure vocabulary shared with the client |
| `FailureDetail` | newtype | Adapter free text, never parsed, never decisive |
| `FieldDiffReport` | struct | Normaliser version plus the field mismatches read-back found |
| `AmbiguityCause` | enum | The five ways a write becomes unknown |
| `ChallengeKind` | enum | The four ways a marketplace interrupts a session |
| `SchemaDrift` | struct | Expected against observed form fingerprint, plus added and removed names |
| `ConnectFailure` | enum | The four provably-never-sent transport failures |
| `WriteAttemptId` | newtype | The fencing token, minted before the click |
| `IdempotencyKey` | newtype | Ours, derived below, stable across a park and requeue |
| `FormSchemaFingerprint` | newtype | A content hash over the form's input-name set |
| `FieldSet` | struct | The projected, length-capped values a submit writes |
| `SubmitEvidence` | struct | What the driver observed, which is evidence and never a verdict |
| `ListingLocator` | enum | `Durable(RemoteListingId)`, `Marker { .. }` or `Recorded { .. }` |
| `ObservedListing` | struct | What read-back saw |
| `StepBudget` | struct | Remaining interpreter actions for this job |
| `HaltScope` | enum | `OrgInventory`, `Org`, `FleetInventory` |
| `CaptureCause` | enum | Why diagnostics were captured |
| `SellerEvent` | enum | What the seller is told |
| `CreateStrategy` | enum | Configured per inventory, never branched on in code |
| `DraftSupport` | enum | `Supported`, `Unsupported`, `Unprobed`, set by the M-1 probe |
| `MarkerField`, `MarkerLifetime` | enum, struct | Where a correlation marker sits and how long it lives |

The word step carries exactly one meaning in this codebase: `SyncMachine::step`, the transition function.
An interpreter action is an `ActionId` and is bounded by `MAX_ACTIONS_PER_JOB`, and the client surface is a per-item action timeline.
Nothing else is called a step.

## The idempotency key

The key is ours because the marketplace offers none, and it is keyed on the inventory rather than on the marketplace.
That is load-bearing rather than pedantic: both ends of a Tes GB-to-US duplication share tenant, marketplace, product, intent version and content hash, so a marketplace-keyed tuple would collide on `UNIQUE (org_id, idempotency_key)` and the second half of the first chargeable product would silently never run.

The derivation is UUIDv5 over a canonical byte encoding, which is a judgement this specification makes rather than a research finding, taken because version 5 is deterministic, collision-resistant in practice, and fits the `uuid NOT NULL` column the job ledger already declares.

```text
idempotency_key = uuidv5(NAMESPACE_TAM_INTENT, canonical)
canonical       = org_id_bytes            (16 bytes)
               || inventory_discriminant  (1 byte, the InventoryId ordinal)
               || product_id_bytes        (16 bytes)
               || intent_version          (4 bytes, big-endian u32)
               || intent_hash             (32 bytes, blake3 over the FieldSet)
```

`NAMESPACE_TAM_INTENT` is a fixed UUID constant in `tam-types`, generated once and never rotated, because rotating it would re-key every item in flight.
The encoding is fixed-width in every field, so no separator is needed and no length-extension ambiguity exists.
A requeued item recomputes the same key, which is what lets a connection that transitions to `NeedsReauth` requeue every item behind a gate without any of them losing identity.

## The states

`AwaitingPreflight` is the entry state, and nothing else may precede it.
`PreflightAsserted` holds the fingerprint the form actually presented, which is the value a later drift is measured against.
`IntentRecorded` means the `write_attempt` row is committed, so a crash from here on leaves something reconciliation can find.
`Submitted` holds the driver's evidence and is the only state from which ambiguity can arise.
`AwaitingReadBack` holds the locator that will settle the write.
`Parked` holds a live challenge and is deliberately not terminal.
`Stranded` is the run ending without a verdict on the item: a create whose write went out under a strategy this build can identify and whose fate this run cannot determine, holding the attempt that fences its mapping and the locator a later run should search for.
Two arrivals reach it, a submit whose answer was lost and a challenge that arrived mid-write, and they are one situation rather than two.
No input carries the machine forward from it, which is the point — entering `AwaitingReadBack` without having asked for a read would let a fabricated read result commit a listing nobody looked at, and `every_committed_terminal_follows_a_read` is the property that refuses it.
`BudgetExhausted` still terminates it, as `Ambiguous`, because a write did go out; nothing else applies.
`Terminal` holds an `Outcome` and can be stepped no further, which `step` taking `self` by value enforces at compile time.

## The transitions

Every row is total: an `Input` arriving in a state not listed here returns `Err(MachineError::InputNotApplicable)`, and an input naming a `WriteAttemptId` the machine does not own returns `Err(MachineError::AttemptMismatch)`.
`ResumeStranded` additionally returns `Err(MachineError::ResumeNotACreate)` against any other operation, because only a create leaves a fencing row nothing can settle: a revise re-applies the same fields and a removal re-deletes something already gone, so both are re-run rather than reconciled.
It is an input rather than a second constructor deliberately — every state but the entry one is reached by a transition, which is what makes this table the whole specification, and a constructor starting mid-graph could establish a state no transition ever checked.
It adopts the standing attempt rather than opening another, and the entry row's effects are discarded rather than executed, which matters because the first of them is the form assertion and on Tes that is a write.
How the create is identified is the strategy's to say, and the two that can be searched are searched differently: `CorrelationMarker` embeds a unique marker and is matched by substring, while `DraftThenPublish` embeds nothing and is identified by the title the attempt recorded that it sent, matched exactly and narrowed to the state a create leaves a listing in, because a title is not unique and only one surviving candidate is an identification.
The recorded title travels on the input rather than being read from the machine's own fields, which hold a fresh projection of a product the seller may have renamed since the strand; searching for the current title is how a reconcile binds a listing its create never made.
Where the strategy leaves nothing to identify — `HaltOnAmbiguity`, which says so in its name — the resume settles ambiguous with a notification and no halt, leaving the attempt standing: an ambiguity elsewhere means this tenant's automation has stopped being safe to continue, while here it means only that this build configures no identification, which is equally true of every item in the queue and is not a reason to stop it.
A reconcile that finds the listing never settles ambiguous on the grounds that the write's echo is missing, because the find substitutes for the lost write response and nothing more: it binds nothing by itself and leaves the attempt standing, and the verifying read-back behind it decides against the recorded intent — committed where every field the intent asked for is what the marketplace holds, degraded where the listing exists but a field differs, and ambiguous with the attempt still standing only where the read itself could not be performed.
`Result` is used only for transitions that are genuinely impossible rather than merely unsuccessful, so every business outcome including ambiguity is a value in `Outcome` travelling the success channel.

| State | Input | Next state | Effects |
|---|---|---|---|
| `AwaitingPreflight` | (entry) | `AwaitingPreflight` | `AssertFormSchema` |
| `AwaitingPreflight` | `ResumeStranded` (marker or recorded-title strategy) | `AwaitingReadBack` | `Reconcile` |
| `AwaitingPreflight` | `ResumeStranded` (nothing to identify) | `Terminal(Ambiguous NoDurableIdentifier)` | `Notify` |
| `AwaitingPreflight` | `PreflightResult(Ok)` | `PreflightAsserted` | `RecordIntent` |
| `AwaitingPreflight` | `PreflightResult(Err)` | `Terminal(Rejected FormSchemaDrift)` | `CaptureDiagnostics`, `Halt OrgInventory`, `Notify` |
| `PreflightAsserted` | `IntentRecorded` | `IntentRecorded` | `Submit` |
| `IntentRecorded` | `SubmitResult(Ok)` | `AwaitingReadBack` | `ReadBack` |
| `IntentRecorded` | `SubmitResult(Err Ambiguous)` (marker strategy) | `AwaitingReadBack` | `Reconcile`, `CaptureDiagnostics` |
| `IntentRecorded` | `SubmitResult(Err Ambiguous)` (recorded-title strategy) | `Stranded` | `CaptureDiagnostics` |
| `IntentRecorded` | `SubmitResult(Err Ambiguous)` (nothing to identify) | `Terminal(Ambiguous NoDurableIdentifier)` | `CaptureDiagnostics`, `Halt OrgInventory`, `Notify` |
| `IntentRecorded` | `SubmitResult(Err Rejected)` | `Terminal(Rejected)` | none |
| `IntentRecorded` | `SubmitResult(Err Challenge)` (reauth or one-time password) | `Parked` | `ParkItem`, `RequeueBehindGate`, `Notify` |
| `IntentRecorded` | `SubmitResult(Err Challenge)` (captcha or interstitial, not a create) | `Terminal(Blocked)` | none |
| `IntentRecorded` | `SubmitResult(Err Challenge)` (captcha or interstitial, create, marker strategy) | `AwaitingReadBack` | `Reconcile`, `CaptureDiagnostics` |
| `IntentRecorded` | `SubmitResult(Err Challenge)` (captcha or interstitial, create, recorded-title strategy) | `Stranded` | `CaptureDiagnostics` |
| `IntentRecorded` | `SubmitResult(Err Challenge)` (captcha or interstitial, create, nothing to identify) | `Terminal(Ambiguous NoDurableIdentifier)` | `CaptureDiagnostics`, `Halt OrgInventory`, `Notify` |
| `IntentRecorded` | `SubmitResult(Err SessionExpired)` | `Parked` | `ParkItem`, `RequeueBehindGate`, `Notify` |
| `IntentRecorded` | `SubmitResult(Err SchemaDrift)` | `Terminal(Rejected FormSchemaDrift)` | `CaptureDiagnostics`, `Halt OrgInventory` |
| `IntentRecorded` | `SubmitResult(Err RateLimited)` | `Terminal(Skipped RateLimited)` | none |
| `IntentRecorded` | `SubmitResult(Err NotSent)` | `PreflightAsserted` | `RecordIntent` |
| `AwaitingReadBack` | `ReadBackResult(Ok)` | `Terminal(Committed or Degraded)` | none |
| `AwaitingReadBack` | `ReadBackResult(Err Ambiguous)` | `Terminal(Ambiguous)` | `CaptureDiagnostics`, `Halt OrgInventory`, `Notify` |
| `AwaitingReadBack` | `ReconcileResult(Ok Some)` | `AwaitingReadBack` | `ReadBack` |
| `AwaitingReadBack` | `ReconcileResult(Ok None)` | `Terminal(Ambiguous NoDurableIdentifier)` | `Halt OrgInventory`, `Notify` |
| `AwaitingReadBack` | `ReconcileResult(Err)` | `Terminal(Ambiguous ReadBackIndeterminate)` | `Halt OrgInventory`, `Notify` |
| `Parked` | `ChallengeCleared` | `AwaitingPreflight` | `AssertFormSchema` |
| `Parked` | `ParkExpired` | `Terminal(Blocked)` | `Notify` |
| any non-terminal | `BudgetExhausted` | `Terminal(Ambiguous or Skipped)` | `CaptureDiagnostics` |

`NotSent` is the only class that returns to a pre-submit state, because it is the only class where the request provably never left.

An ambiguous submit on a create is the first of the rows whose effects are chosen so that the run stops rather than continues.
Under a marker strategy the search runs inside the same run, because a marker is embedded at submit time and is there to be found.
Under the recorded-title strategy it does not: the listing sits in the marketplace's own processing queue for minutes after the submit, so a search run now answers a completed-and-absent `Ok(None)`, which this table settles ambiguous and halts the tenant's inventory on.
So the machine records the identification in the locator, emits only `CaptureDiagnostics`, and steps to `Stranded`, a state no input carries forward; the interpreter maps it straight to an abandoned run, leaving the attempt in flight and the mapping fenced.
It is `Stranded` rather than `AwaitingReadBack` because no read was asked for, and a state that accepted a read result it never requested would let one be invented.
The reaper parks the item on `awaiting_marketplace_answer` and a later claim reconciles it against the seller's own catalogue, by which time the marketplace has had time to answer.
The recorded title is read from the machine's own `fields` here and only here: this is the run that rendered them, so they are the intent the submit actually sent, where `ResumeStranded` must carry the title on the input because its `fields` are a fresh projection of a product the seller may have renamed since.

The other rows chosen that way are the create's challenge rows, and they are chosen that way for the same reason.
A challenge the seller cannot clear — a captcha, an interstitial, a firewall rule — does not prove the write did not land: an adapter may mint the listing and only then meet the edge on the read that follows, which is exactly what Tes does.
So a create meeting one is a create of unknown fate, and it takes the same three rows an ambiguous submit takes, decided by what the strategy leaves behind rather than by how the fate became unknown.
Under the recorded-title strategy it strands, and the halt this row used to raise was stopping a whole tenant's queue to prevent the second create that the standing attempt prevents by itself.
Under a strategy that leaves nothing to identify, or where the recorded intent names no title, it still halts, because there the fence is the only thing standing and nothing will ever come to settle it.
A revise and a removal keep the terminal `Blocked` settle: neither can mint a duplicate, so neither has a fate worth holding open.
`BudgetExhausted` settles as `Ambiguous` from `IntentRecorded`, `AwaitingReadBack` or `Stranded` and as `Skipped` from any earlier state, because the budget can only have run out after a submit was possible in those three.
The shipped interpreter never steps it into `Stranded` — its budget guard excludes that state, so a cancellation cannot convert a stranded create into a terminal ambiguity — but the machine answers it, because every non-terminal state must reach a terminal in one transition and a write had gone out.
A re-entry from `Parked` re-asserts the form schema rather than resuming mid-flow, because the markup may have changed while the item was parked and the assertion is one request.

## The invariants the property suite asserts

`step` is pure and total, so a property test generates arbitrary `Input` sequences and asserts the five properties that carry the whole correctness argument.
No `Submit` effect is ever emitted twice for one `WriteAttemptId`.
Every terminal `Ambiguous` is preceded by a `RecordIntent`.
No path leads from `Ambiguous` back to `Submit`.
Every `Committed` and every `Degraded` is preceded by a `ReadBack`.
And a `BudgetExhausted` input always reaches a terminal state in one transition.

These run without a database, a browser or a network, in milliseconds rather than browser-minutes.
`cargo-mutants` 27.1.0 is the adequacy check on the suite, which the adversarial charter review endorsed over assertion-density metrics.

## The adapter seam

```rust
pub trait MarketplaceAdapter: Send + Sync {
    fn inventory(&self) -> InventoryId;

    fn assert_form_schema(
        &self,
        org: OrgId,
        form: FormId,
    ) -> impl std::future::Future<Output = Result<FormSchemaFingerprint, AdapterError>> + Send;

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
```

Every method is written as `fn f(..) -> impl Future<Output = ..> + Send` rather than `async fn`, because `async_fn_in_trait` is a hard error under a deny-warnings build on the pinned toolchain and the compiler's own suggested desugaring is also what a multi-threaded runtime requires.
`OrgId` is a mandatory positional parameter on every method and is never read from task-local state.
`IdempotencyKey` is required on `submit`, so a submit without one does not typecheck.
`AdapterError::Ambiguous` carries no retry affordance of any kind, so the only way to retry an ambiguous write is to construct a different error.
`NotSent` is constructible only from a `reqwest` connect failure classified below `thirtyfour`, which is the third wrapper obligation expressed as a type.
The adapter is keyed on `InventoryId` rather than `Marketplace`, so the Tes crate registers two adapters that share markup, a login and an upload flow and differ only in vocabulary and currency.

## The fault-injection seam

The 429 handler will never execute in development and will first execute in production during the incident it exists to contain, so the error path is made reachable on purpose through a `FaultPlan` trait at the transport boundary below the adapter and above the driver.
It is a trait rather than a `cfg(test)` switch because the same faults must be walked by a scheduled synthetic in production.

```rust
pub trait FaultPlan: Send + Sync {
    fn next_fault(&self, action: ActionId) -> Option<InjectedFault>;
}
```

`InjectedFault` carries the six transport faults the research names — HTTP 429 with an optional `Retry-After`, HTTP 403, an HTML interstitial, a redirect to sign-in, a truncated body and a connection reset mid-body — plus the two ambiguity faults this design adds, `ResponseEventLost` and `ProcessKilledAfterSubmit`, because those two produce `Ambiguous` rather than `Rejected` and no external service will produce them on demand.
The M0 acceptance criterion is that a deliberately interrupted write classifies as ambiguous rather than as success or failure, so this seam is built at M0 and not later.

Deterministic simulation testing is deliberately deferred on the adversarial review's grounds that none of it gets harder by waiting, with `turmoil` 0.7.2 and `madsim` 0.2.34 as the verified candidates.
The trigger is stated as a judgement rather than a finding: either the day serialisation is relaxed, when a pool replaces the one-to-two lanes or the per-tenant mutex loosens and interleaving becomes a real state space, or the first production `Ambiguous` whose sequence cannot be reproduced from the recorded intent and event logs.
