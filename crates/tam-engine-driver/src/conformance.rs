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
    ObservedListing, ProjectedListing, RemoteLifecycle, RemoteListingId, RemovalPlan, RevisePlan,
    SubmitEvidence,
};
use tam_types::{ContentHash, FieldKey, InventoryId, Timestamp, Uuid};

use crate::driver::{run_item, DriverContext, MachineSeed, NowSource, RunVerdict, VerifyPolicy};
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
pub struct Switch(std::sync::atomic::AtomicBool);

impl Switch {
    pub fn trip(&self) {
        self.0.store(true, Ordering::SeqCst);
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
        })
    }

    async fn assert_form_schema(
        &self,
        _form: FormId,
    ) -> Result<FormSchemaFingerprint, AdapterError> {
        let position = self.preflight_cursor.fetch_add(1, Ordering::SeqCst);
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
            ListingLocator::Marker { .. } => RemoteListingId::Tes {
                url: LANDED_URL.to_owned(),
            },
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
        form: FormId(Uuid([0x09; 16])),
        fields: FieldSet {
            entries: vec![(FieldKey::Title, "Fixture".to_owned())],
            files: vec![],
            body_format: None,
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
