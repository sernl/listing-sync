//! One set of test bodies, run against two ledgers.
//!
//! The bodies here are the whole point of the split's verification. Each takes
//! a ledger that implements both [`ItemLedger`] and [`LedgerInspector`] and
//! asserts only through those two traits, so the same body runs against the
//! in-memory ledger inside `just check` and against Postgres inside
//! `just db-test`. A storage assumption that leaked into the interpreter shows
//! up as a divergence between the two runs rather than as a compile error
//! somebody quietly fixes by hand.
//!
//! An assertion that cannot be written through the inspector is the kill gate
//! for the phase, not a licence to widen the inspector: the inspector reads
//! state the interpreter already caused and exposes no operation absent from
//! `ItemLedger`.

// These are test bodies that happen to live in `src/` so a second crate can
// run them; `allow-expect-in-tests` reaches `#[test]` functions and not a
// module compiled into the library, so the exemption is stated here instead. A
// fixture whose invariant does not hold should stop the run at the assertion
// that noticed, not carry a default into the next one.
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "conformance bodies are tests compiled into the library so two crates can share them"
)]

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use tam_domain::{ItemOutcome, StepBudget};
use tam_marketplace::{
    AdapterError, AmbiguityCause, ChallengeKind, CreateStrategy, FetchReason, FieldSet, FormId,
    FormSchemaFingerprint, IdempotencyKey, InstantPause, ListingLocator, MarketplaceAdapter,
    ObservedListing, ProjectedListing, RecordedTitle, RemoteLifecycle, RemoteLifecycleKind,
    RemoteListingId, RemovalPlan, RevisePlan, SubmitEvidence, WriteAttemptId,
};
use tam_types::{ContentHash, FieldKey, InventoryId, Timestamp, Uuid};

use crate::driver::{run_item, DriverContext, MachineSeed, NowSource, RunVerdict, VerifyPolicy};
use crate::memory::ScriptedReconcile;
use crate::ports::{Cancellation, IdSource, ItemLedger, LedgerInspector};
use crate::vocabulary::LeasedItem;

/// The instant every body starts from.
pub const T0: Timestamp = Timestamp(1_756_000_000_000);

/// The listing the scripted read-back observes, and the URL a settled attempt
/// must therefore record.
pub const LANDED_URL: &str = "https://www.tes.com/api/v2/resources/9001";

/// A clock that advances a second per reading, which is what keeps a body's
/// wall-clock budget from expiring inside the run.
pub struct SteppingClock(AtomicI64);

impl SteppingClock {
    #[must_use]
    pub const fn from(at: Timestamp) -> Self {
        Self(AtomicI64::new(at.0))
    }
}

impl NowSource for SteppingClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.fetch_add(1_000, Ordering::SeqCst))
    }
}

/// Attempt ids drawn in order, so a body's assertions do not depend on
/// randomness the interpreter deliberately does not hold.
pub struct SequentialIds(AtomicUsize);

impl Default for SequentialIds {
    fn default() -> Self {
        Self(AtomicUsize::new(1))
    }
}

impl IdSource for SequentialIds {
    fn new_id(&self) -> Uuid {
        let next = self.0.fetch_add(1, Ordering::SeqCst);
        let mut bytes = [0u8; 16];
        bytes[0] = u8::try_from(next & 0xFF).unwrap_or(0xFF);
        Uuid(bytes)
    }
}

/// Cancellation a body drives itself, so the suspended-device paths are
/// reachable without a runtime.
#[derive(Default)]
pub struct Switch(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Switch {
    pub fn trip(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// The same flag, for a fixture that has to trip it from somewhere the
    /// adapter cannot reach — the ledger, whose calls are where a run sits
    /// between recording an intent and sending the write it authorises.
    #[must_use]
    pub fn shared(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.0)
    }
}

impl Cancellation for &Switch {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Scripted per call, exactly as the Postgres suite's fixture was: submit and
/// preflight answers are consumed in order, and the read-back either observes
/// the draft or answers the one condition the fixture was built with.
pub struct ScriptedAdapter<'a> {
    submit_answers: Vec<Result<SubmitEvidence, AdapterError>>,
    submit_cursor: AtomicUsize,
    read_back_condition: Option<AdapterError>,
    preflight_answers: Vec<Result<FormSchemaFingerprint, AdapterError>>,
    preflight_cursor: AtomicUsize,
    cancel: Option<&'a Switch>,
    cancel_after_preflight: bool,
    cancel_after_submit: bool,
    cancel_after_read_back: bool,
}

impl<'a> ScriptedAdapter<'a> {
    #[must_use]
    pub fn answering(submit: Result<SubmitEvidence, AdapterError>) -> Self {
        Self {
            submit_answers: vec![submit],
            submit_cursor: AtomicUsize::new(0),
            read_back_condition: None,
            preflight_answers: vec![Ok(FormSchemaFingerprint(ContentHash([0x0F; 32])))],
            preflight_cursor: AtomicUsize::new(0),
            cancel: None,
            cancel_after_preflight: false,
            cancel_after_submit: false,
            cancel_after_read_back: false,
        }
    }

    #[must_use]
    pub fn with_read_back_condition(mut self, condition: AdapterError) -> Self {
        self.read_back_condition = Some(condition);
        self
    }

    #[must_use]
    pub fn cancelling_after_submit(mut self, switch: &'a Switch) -> Self {
        self.cancel = Some(switch);
        self.cancel_after_submit = true;
        self
    }

    /// Stops the run after the form scrape and before the write, which is the
    /// window an entitlement lapsing mid-tick actually lands in.
    #[must_use]
    pub fn cancelling_after_preflight(mut self, switch: &'a Switch) -> Self {
        self.cancel = Some(switch);
        self.cancel_after_preflight = true;
        self
    }

    #[must_use]
    pub fn cancelling_after_read_back(mut self, switch: &'a Switch) -> Self {
        self.cancel = Some(switch);
        self.cancel_after_read_back = true;
        self
    }
}

impl MarketplaceAdapter for ScriptedAdapter<'_> {
    fn inventory(&self) -> InventoryId {
        InventoryId::TesGb
    }

    fn project_fields(&self, listing: &ProjectedListing) -> Result<FieldSet, AdapterError> {
        Ok(FieldSet {
            entries: vec![(FieldKey::Title, listing.title.clone())],
            files: listing.files.clone(),
            body_format: Some(listing.body_format),
            appropriate_for_country: None,
        })
    }

    async fn assert_form_schema(
        &self,
        _form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let position = self.preflight_cursor.fetch_add(1, Ordering::SeqCst);
        if self.cancel_after_preflight {
            if let Some(switch) = self.cancel {
                switch.trip();
            }
        }
        self.preflight_answers
            .get(position)
            .or_else(|| self.preflight_answers.last())
            .cloned()
            .unwrap_or(Err(AdapterError::Uncaptured {
                capability: "scripted.preflight",
            }))
    }

    async fn submit(
        &self,
        _key: IdempotencyKey,
        _fields: FieldSet,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        let position = self.submit_cursor.fetch_add(1, Ordering::SeqCst);
        let answer =
            self.submit_answers
                .get(position)
                .cloned()
                .unwrap_or(Err(AdapterError::Ambiguous(
                    AmbiguityCause::ResponseEventLost,
                )));
        if self.cancel_after_submit {
            if let Some(switch) = self.cancel {
                switch.trip();
            }
        }
        answer
    }

    async fn revise(
        &self,
        _plan: RevisePlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        Err(AdapterError::Uncaptured {
            capability: "scripted.revise",
        })
    }

    async fn remove(
        &self,
        _plan: RemovalPlan,
        _now: Timestamp,
    ) -> Result<SubmitEvidence, AdapterError> {
        Err(AdapterError::Uncaptured {
            capability: "scripted.remove",
        })
    }

    async fn read_back(
        &self,
        locator: ListingLocator,
        _reason: FetchReason,
        _observed_at: Timestamp,
    ) -> Result<ObservedListing, AdapterError> {
        if let Some(condition) = self.read_back_condition.clone() {
            return Err(condition);
        }
        let id = match locator {
            ListingLocator::Durable(id) => id,
            ListingLocator::Marker { .. } | ListingLocator::Recorded { .. } => {
                RemoteListingId::Tes {
                    url: LANDED_URL.to_owned(),
                }
            }
        };
        if self.cancel_after_read_back {
            if let Some(switch) = self.cancel {
                switch.trip();
            }
        }
        Ok(ObservedListing {
            id,
            fields: vec![(FieldKey::Title, "Fixture".to_owned())],
            lifecycle: RemoteLifecycle::Draft,
        })
    }
}

/// The submit landed and named its listing.
#[must_use]
pub fn landed_evidence() -> SubmitEvidence {
    SubmitEvidence {
        http_status: Some(200),
        response_body_digest: None,
        landed_on_route: Some(LANDED_URL.to_owned()),
        landed: Some(RemoteListingId::Tes {
            url: LANDED_URL.to_owned(),
        }),
        observed_lag: false,
    }
}

#[must_use]
fn seed_machine() -> MachineSeed {
    MachineSeed {
        resume: None,
        attestation: None,
        form: FormId(Uuid([0x09; 16])),
        fields: FieldSet {
            entries: vec![(FieldKey::Title, "Fixture".to_owned())],
            files: vec![],
            body_format: None,
            appropriate_for_country: None,
        },
        intent_hash: ContentHash([0x0A; 32]),
        strategy: CreateStrategy::HaltOnAmbiguity,
        budget: StepBudget {
            actions_remaining: 20,
        },
        verify: VerifyPolicy {
            tries: 3,
            interval_ms: 1,
        },
    }
}

async fn drive<L: ItemLedger>(
    ledger: &L,
    lease: &LeasedItem,
    adapter: &ScriptedAdapter<'_>,
    switch: &Switch,
) -> RunVerdict {
    let clock = SteppingClock::from(Timestamp(T0.0 + 1_000));
    let ids = SequentialIds::default();
    let ctx = DriverContext {
        reconcile: &ScriptedReconcile::could_not_read(
            "this body drives no reconcile; a body that does says so",
        ),
        adapter,
        ledger,
        clock: &clock,
        ids: &ids,
        cancel: &switch,
        pause: &InstantPause,
    };
    match run_item(&ctx, lease, seed_machine()).await {
        Ok(verdict) => verdict,
        Err(error) => panic!("the driver runs: {error:?}"),
    }
}

/// The title the stranded create recorded that it sent, which is what
/// identifies it to the catalogue walk.
pub const STRANDED_TITLE: &str = "Fixture";

/// The attempt a reconcile body finds already standing.
pub const STRANDED: Uuid = Uuid([0x5A; 16]);

/// The same drive, seeded to reconcile a standing attempt rather than to
/// create.
///
/// The seed's `resume` is what makes it one: the run steps straight to the
/// search and the entry row's effects are discarded, so no form is asserted
/// and no second attempt is opened.
async fn drive_reconcile<L: ItemLedger>(
    ledger: &L,
    lease: &LeasedItem,
    adapter: &ScriptedAdapter<'_>,
    switch: &Switch,
    reconcile: &ScriptedReconcile,
) -> RunVerdict {
    let clock = SteppingClock::from(Timestamp(T0.0 + 1_000));
    let ids = SequentialIds::default();
    let ctx = DriverContext {
        reconcile,
        adapter,
        ledger,
        clock: &clock,
        ids: &ids,
        cancel: &switch,
        pause: &InstantPause,
    };
    let mut seed = seed_machine();
    seed.resume = Some((
        WriteAttemptId(STRANDED),
        RecordedTitle(STRANDED_TITLE.to_owned()),
    ));
    // Draft-then-publish, which is what production configures, so these bodies
    // drive the path a real stranded create takes: identified by the title it
    // recorded rather than by a marker. The shared seed's `HaltOnAmbiguity`
    // leaves nothing to identify and would end each body before the reconcile
    // source was ever asked.
    seed.strategy = CreateStrategy::DraftThenPublish {
        draft_state: RemoteLifecycleKind::Draft,
    };
    match run_item(&ctx, lease, seed).await {
        Ok(verdict) => verdict,
        Err(error) => panic!("the driver runs: {error:?}"),
    }
}

/// A clean submit and read-back settles the item succeeded, commits the
/// fencing attempt, and binds the mapping to the listing the write landed on.
pub async fn the_happy_path_settles_succeeded<L: ItemLedger + LedgerInspector>(
    ledger: &L,
    lease: &LeasedItem,
) {
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "a clean submit and read-back settles succeeded"
    );
    let item = ledger.item(lease.item).await;
    assert_eq!(
        (item.state.as_str(), item.outcome.as_deref()),
        ("settled", Some("succeeded")),
        "the item row carries the outcome"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the run opened a fencing attempt");
    assert_eq!(
        (attempt.state.as_str(), attempt.settled),
        ("committed", true),
        "the fencing row settled with the item"
    );
    assert_eq!(
        (
            attempt.remote_id_kind.as_deref(),
            attempt.remote_url.as_deref()
        ),
        (Some("tes"), Some(LANDED_URL)),
        "the committed attempt records the listing the write landed on, or nothing can \
         reconcile what it created"
    );
    let binding = ledger
        .binding(lease.mapping)
        .await
        .expect("the mapping exists");
    assert_eq!(
        (
            binding.binding_state.as_str(),
            binding.remote_id_kind.as_deref(),
            binding.remote_url.as_deref(),
            binding.verify_state.as_str(),
            binding.never_verified,
            binding.stale_since_first_seen,
        ),
        ("bound", Some("tes"), Some(LANDED_URL), "stale", true, true),
        "the settle binds the mapping to the listing, stale because the report it settled \
         on was never normalised"
    );
    assert!(
        ledger.event_count(lease.item).await >= 3,
        "the run recorded its actions"
    );
}

/// An unreconcilable ambiguous submit settles ambiguous, halts the tenant's
/// inventory, notifies the seller and binds nothing.
pub async fn an_ambiguous_submit_halts_the_inventory<L: ItemLedger + LedgerInspector>(
    ledger: &L,
    lease: &LeasedItem,
) {
    let switch = Switch::default();
    let adapter =
        ScriptedAdapter::answering(Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut)));
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Ambiguous),
        "an unreconcilable ambiguous submit settles ambiguous, never retries"
    );
    assert_eq!(
        ledger.halt_count(lease.org).await,
        1,
        "the tenant's inventory halted — the account-safety gate"
    );
    assert!(
        ledger.outbox_count(lease.org, None).await >= 1,
        "the seller notification queued"
    );
    let binding = ledger
        .binding(lease.mapping)
        .await
        .expect("the mapping exists");
    assert_eq!(
        binding.binding_state, "unbound",
        "an ambiguous submit landed nothing, so there is nothing to bind"
    );
}

/// A read-back that answers a condition rather than an observation abandons
/// the run and leaves the attempt standing, which is the create fence.
pub async fn a_read_back_condition_abandons_rather_than_crashing<L>(ledger: &L, lease: &LeasedItem)
where
    L: ItemLedger + LedgerInspector,
{
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()))
        .with_read_back_condition(AdapterError::SessionExpired);
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "a read condition abandons into the stealer rather than inventing an outcome: \
         {verdict:?}"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the run opened a fencing attempt");
    assert!(
        !attempt.settled,
        "the create's attempt stays in flight so a requeue cannot mint a second listing"
    );
    let binding = ledger
        .binding(lease.mapping)
        .await
        .expect("the mapping exists");
    assert_eq!(
        binding.binding_state, "unbound",
        "nothing was verified, so nothing binds"
    );
}

/// A lapsed session is the seller's to fix, so the item parks on the reauth
/// gate, the connection flips, and the seller is told.
pub async fn an_expired_session_parks_and_gates<L: ItemLedger + LedgerInspector>(
    ledger: &L,
    lease: &LeasedItem,
) {
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Err(AdapterError::SessionExpired));
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert_eq!(
        verdict,
        RunVerdict::Parked,
        "a re-link is a thing the seller can actually do, so the item waits for it"
    );
    let item = ledger.item(lease.item).await;
    assert_eq!(
        item.state, "parked_live",
        "parked on the gate the re-link arm of revive_expired reads back"
    );
    assert_eq!(
        ledger.connection_state(lease.org, lease.inventory).await,
        Some("needs_reauth".to_owned()),
        "the gate goes up: every sibling would spend a lease learning the same thing"
    );
    assert_eq!(
        ledger
            .outbox_count(lease.org, Some("email.parked_job"))
            .await,
        1,
        "and the seller was told, which is the only way a park ever clears"
    );
}

/// The loop-top guard, from the terminal side: a cancellation arriving after
/// the read-back has satisfied the predicate must settle what the write
/// committed rather than fail the run with the listing still open.
pub async fn a_cancellation_after_the_read_back_still_settles<L>(ledger: &L, lease: &LeasedItem)
where
    L: ItemLedger + LedgerInspector,
{
    let switch = Switch::default();
    let adapter =
        ScriptedAdapter::answering(Ok(landed_evidence())).cancelling_after_read_back(&switch);
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "the read-back had already satisfied the predicate, so the cancellation settles the \
         run rather than failing it with the listing still open"
    );
    let item = ledger.item(lease.item).await;
    assert_eq!(
        (item.state.as_str(), item.outcome.as_deref()),
        ("settled", Some("succeeded")),
        "an unsettled item is what a closed lid used to cost"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the run opened a fencing attempt");
    assert_eq!(
        (attempt.state.as_str(), attempt.settled),
        ("committed", true),
        "the fence settles with it rather than standing open against every later lease"
    );
}

/// The same guard from the parked side: the park, the gate and the
/// notification must all land rather than being exchanged for an abandoned
/// attempt while the mapping is still unbound.
pub async fn a_cancellation_after_a_lapsed_session_still_parks<L>(ledger: &L, lease: &LeasedItem)
where
    L: ItemLedger + LedgerInspector,
{
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Err(AdapterError::SessionExpired))
        .cancelling_after_submit(&switch);
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert_eq!(
        verdict,
        RunVerdict::Parked,
        "the park was already decided when the token was cancelled, so the run reports it"
    );
    let item = ledger.item(lease.item).await;
    assert_eq!(
        item.state, "parked_live",
        "the park landed rather than being exchanged for an abandoned attempt"
    );
    assert_eq!(
        ledger.connection_state(lease.org, lease.inventory).await,
        Some("needs_reauth".to_owned()),
        "and the gate went up, which is what holds every sibling item back"
    );
    assert_eq!(
        ledger
            .outbox_count(lease.org, Some("email.parked_job"))
            .await,
        1,
        "and the seller was told"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the run opened a fencing attempt");
    assert!(
        !attempt.settled,
        "the create fence stays shut: settling it here is what would let the park resume \
         into a second listing"
    );
}

/// A preflight that answers a challenge the seller cannot clear settles the
/// item blocked without gating the connection.
pub async fn a_preflight_challenge_abandons_and_advances_the_streak<L>(
    ledger: &L,
    lease: &LeasedItem,
) where
    L: ItemLedger + LedgerInspector,
{
    let switch = Switch::default();
    let mut adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    adapter.preflight_answers = vec![Err(AdapterError::Challenge(
        ChallengeKind::JavaScriptInterstitial,
    ))];
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "one interstitial is a transient: the item abandons into the stealer rather than \
         spending the seller's attention: {verdict:?}"
    );
    assert_eq!(
        ledger.item(lease.item).await.preflight_failures,
        1,
        "the streak advanced, which is what eventually ends the run"
    );
    assert_eq!(
        ledger.connection_state(lease.org, lease.inventory).await,
        Some("linked".to_owned()),
        "an interstitial is not something a re-link fixes, so the connection stays linked"
    );
}

/// A create whose mapping was bound while it ran settles skipped.
///
/// The refusal is permanent for this item — the listing exists and no later
/// lease could make it again — so the interpreter settles rather than
/// abandoning, which is what stops the item re-abandoning on every lease
/// until its attempt budget settled it `failed` with nothing in the ledger to
/// say why. Not a conformance body over both ledgers: it needs the ledger to
/// answer a refusal Postgres reaches through a concurrent bind, which only
/// the in-memory one can be told to do.
pub async fn a_create_bound_elsewhere_settles_skipped<L: ItemLedger + LedgerInspector>(
    ledger: &L,
    lease: &LeasedItem,
) {
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Skipped),
        "the create settles skipped rather than abandoning, because nothing it could do \
         on a later lease would change the answer"
    );
    let item = ledger.item(lease.item).await;
    assert_eq!(
        (item.state.as_str(), item.outcome.as_deref()),
        ("settled", Some("skipped")),
        "and the item row says so, so the job that owns it can complete"
    );
    assert!(
        item.failure_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("created by another run")),
        "the detail names what happened, because an operator reading the row cannot \
         otherwise tell this from an ordinary skip: {:?}",
        item.failure_detail
    );
    assert!(
        ledger.attempt(lease.mapping).await.is_none(),
        "and no attempt is left standing: the refusal happened before one was opened, so \
         there is no fence to release and none to leak"
    );
}

/// A run stopped between the form scrape and the write abandons the attempt,
/// and does not settle the item.
///
/// The loop-top guard steps `BudgetExhausted`, which from a state holding an
/// open attempt settles the item `Ambiguous` — correct for a run killed after
/// a write, and wrong for one stopped before it, where nothing was sent. An
/// ambiguity is a claim that a write may have landed, so settling on it ends
/// the item terminally and never runs it again: the seller's work lost to one
/// device going away. Nothing halts on that path, and the halt assertion below
/// is a guard rather than the point.
pub async fn a_run_stopped_before_the_write_abandons_rather_than_settling_the_item(
    ledger: &crate::memory::InMemoryLedger,
    lease: &LeasedItem,
) {
    let switch = Switch::default();
    // Stopped the moment the attempt opens, which is the only window an
    // adapter hook cannot reach and the one this arm is about.
    ledger.stopping_when_an_attempt_opens(switch.shared());
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "nothing was sent, so the run hands the lease back rather than deciding the item: \
         {verdict:?}"
    );
    assert_eq!(
        ledger.halt_count(lease.org).await,
        0,
        "and the tenant's inventory is untouched: one device losing its entitlement is not \
         evidence that this seller's automation has stopped being safe"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the run opened a fencing attempt before it was stopped");
    assert_eq!(
        (attempt.state.as_str(), attempt.settled),
        ("abandoned", true),
        "the attempt it opened is settled abandoned rather than left in flight, so the next \
         lease can open its own rather than colliding with this one"
    );
    let item = ledger.item(lease.item).await;
    assert_ne!(
        item.state, "settled",
        "and the item itself is left for a device that can still do it"
    );
}

/// An exhausted rate window stops the form scrape, before any attempt exists.
///
/// The scrape is a marketplace request and was drawing on nobody's allowance,
/// so a seller's window under-counted by one request per create. Setting the
/// ceiling to nothing proves the scrape now asks: the run stops before it,
/// which means before `RecordIntent`, so no attempt is opened at all. While
/// the scrape went out uncharged this run reached the write and opened one.
pub async fn an_exhausted_window_stops_the_form_scrape_before_any_attempt<L>(
    ledger: &L,
    lease: &LeasedItem,
) where
    L: ItemLedger + LedgerInspector,
{
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    let verdict = drive(ledger, lease, &adapter, &switch).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "an exhausted window abandons rather than settling: {verdict:?}"
    );
    assert!(
        ledger.attempt(lease.mapping).await.is_none(),
        "and it stops early enough that no fencing attempt is opened, which is only true if \
         the scrape itself asked for a grant"
    );
    assert_eq!(
        ledger.halt_count(lease.org).await,
        0,
        "a closed rate window is a wait rather than a fault, so nothing halts"
    );
}

/// A reconcile that finds the listing settles succeeded and binds it.
///
/// The whole point of the reconciliation: a create whose fate the ledger
/// could not determine is decided by what is actually on the marketplace,
/// and the fencing row that was standing is settled rather than released.
pub async fn a_reconcile_that_finds_the_listing_settles_it<L: ItemLedger + LedgerInspector>(
    ledger: &L,
    lease: &LeasedItem,
    found: RemoteListingId,
) {
    // The scripted read-back echoes a `Durable` locator back and invents
    // `LANDED_URL` for a `Marker` one, so a fixture whose found id is
    // `LANDED_URL` cannot tell the two apart: an implementation that kept the
    // marker locator on the verifying read would still pass. This body names a
    // listing only the find could have supplied.
    let RemoteListingId::Tes { url: expected } = found.clone() else {
        panic!("this body's fixture names a Tes listing: {found:?}");
    };
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    let verdict = drive_reconcile(
        ledger,
        lease,
        &adapter,
        &switch,
        &ScriptedReconcile::found(found),
    )
    .await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "the listing is there, so the create did land and the item settles on that"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the stranded attempt is still the one settled");
    assert!(
        attempt.settled,
        "and the fence it was holding is released by settling it, not by dropping it"
    );
    // A settle keyed on any other attempt is refused as a stale lease, so
    // reaching here at all is what proves the row settled is the stranded one
    // rather than a second the run opened behind it.
    assert_eq!(
        (attempt.state.as_str(), attempt.remote_url.as_deref()),
        ("committed", Some(expected.as_str())),
        "settled on the committed class and addressed to the listing the search found, \
         which is the whole chain: the find supplied the identifier, the read-back \
         verified it, and the settle recorded it"
    );
    let binding = ledger
        .binding(lease.mapping)
        .await
        .expect("the mapping exists");
    assert_eq!(
        (
            binding.binding_state.as_str(),
            binding.remote_url.as_deref()
        ),
        ("bound", Some(expected.as_str())),
        "and the mapping is bound to it, so no later pass can create a second"
    );
}

/// A reconcile that cannot identify the listing leaves it stranded.
///
/// Both remaining answers reach here — a complete enumeration that did not
/// contain it, and a read that could not be performed — because neither is
/// evidence the create did not land. The item stays where an operator can see
/// it rather than being settled on a guess.
pub async fn a_reconcile_that_cannot_identify_leaves_it_stranded<
    L: ItemLedger + LedgerInspector,
>(
    ledger: &L,
    lease: &LeasedItem,
    answer: ScriptedReconcile,
) {
    let switch = Switch::default();
    let adapter = ScriptedAdapter::answering(Ok(landed_evidence()));
    let verdict = drive_reconcile(ledger, lease, &adapter, &switch, &answer).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Ambiguous),
        "not knowing is its own answer, and it is not a failure: the halt is what brings a \
         human to it"
    );
    let attempt = ledger
        .attempt(lease.mapping)
        .await
        .expect("the attempt is still there to be reconciled again");
    assert!(
        !attempt.settled,
        "and it is still standing, because nothing here proved the create did not land and \
         releasing it on that is the one failure this ledger cannot undo"
    );
}
