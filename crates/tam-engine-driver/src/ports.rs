//! What the interpreter is allowed to ask of the ledger, and nothing more.
//!
//! Every method is keyed on a [`LeaseRef`], so the server derives the
//! organisation, the connection, the inventory and the epoch from the lease it
//! issued rather than trusting an argument. Four shapes are deliberate
//! refusals rather than renames, and each removes a capability a seller's
//! device must not hold:
//!
//! - [`ItemLedger::halt_this_tenant`] takes no scope. The wider scopes stop
//!   one tenant entirely and every tenant on a marketplace respectively, and
//!   the breaker and the canary remain their only writers.
//! - [`ItemLedger::request_grant`] takes no window and no ceiling. Supplying
//!   either is the governed party setting its own rate limit.
//! - [`ItemLedger::notify`] takes the closed [`SellerEvent`] and never a topic
//!   string, which would otherwise name any relay the drainer knows.
//! - [`ItemLedger::open_attempt`] takes a caller-minted attempt id, so a lost
//!   response is recoverable rather than burning the item's whole budget.

use tam_domain::{JobItemId, SellerEvent};
use tam_types::{ConnectionId, InventoryId, JobEventPayload, MappingId, OrgId, Timestamp};

use tam_marketplace::{AdapterError, ListingLocator, RemoteListingId, WriteAttemptId};

use crate::vocabulary::{
    AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, GrantKind, ItemVerdict, LeaseRef,
    LedgerError, NewAttempt, PreflightStreak, Renewed,
};

/// Whether the run has been asked to stop. A capability rather than a
/// cancellation token, because the token's crate is a runtime dependency this
/// crate refuses and the observation itself is a single atomic read.
pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

/// Where a fresh attempt id comes from. A capability for the same reason the
/// clock is one: this crate takes no ambient I/O, so randomness enters at the
/// host boundary and a test hands in a scripted sequence instead.
pub trait IdSource: Send + Sync {
    fn new_id(&self) -> tam_types::Uuid;
}

/// The client-reachable ledger.
///
/// Written with explicit `impl Future` returns rather than `async fn`, because
/// `async_fn_in_trait` is a hard error under a deny-warnings build and the
/// desugaring is what a multi-threaded host requires anyway.
/// The seller's own catalogue, searched for one stranded create's listing.
///
/// A port rather than a method on the ledger, and served by the device rather
/// than the server, because the read runs under the seller's own session:
/// D1 puts every request to a no-API marketplace on the seller's machine, and
/// an enumeration is a request like any other.
///
/// The contract the answer has to keep, and the whole reconciliation rests on
/// it. `Ok(Some)` is the listing, found. `Ok(None)` is a complete enumeration
/// that did not contain it — a walk that reached its end under the seller's
/// own session and saw everything there was to see. Anything else is `Err`: a
/// partial page, a session that lapsed mid-walk, an endpoint that refused, a
/// search this adapter cannot perform. The two look identical from here and
/// only the adapter can tell them apart, which is why this says so rather
/// than assuming it.
///
/// What turns on the distinction: `Ok(None)` is the only answer that could
/// ever justify releasing the duplicate-create fence, and releasing it
/// wrongly is the one failure this ledger cannot undo. Nothing releases it
/// today — the machine halts ambiguous on both — but the day that changes,
/// this contract is what it will change against.
pub trait ReconcileSource: Send + Sync {
    fn find_listing<'a>(
        &'a self,
        locator: &'a ListingLocator,
        attempt: WriteAttemptId,
    ) -> impl core::future::Future<Output = Result<Option<RemoteListingId>, AdapterError>> + Send + 'a;
}

pub trait ItemLedger: Send + Sync {
    /// The connection this item's inventory resolves to, read once per run.
    fn connection_for(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
    ) -> impl core::future::Future<Output = Result<Option<ConnectionId>, LedgerError>> + Send;

    fn preflight_succeeded(
        &self,
        lease: &LeaseRef,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    fn preflight_failed(
        &self,
        lease: &LeaseRef,
        edge_class: bool,
    ) -> impl core::future::Future<Output = Result<PreflightStreak, LedgerError>> + Send;

    /// Neither an instant nor a duration: the reaper reads `park_expires_at`
    /// against the server's clock, so an instant would be two clocks compared
    /// and a duration would be the parked party setting the length of its own
    /// park. The caller names the gate; the server mints the when.
    fn park(
        &self,
        lease: &LeaseRef,
        blocked_on: &str,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    fn settle_item(
        &self,
        lease: &LeaseRef,
        verdict: &ItemVerdict,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    /// The attempt id is the caller's, so a response lost in flight is
    /// recoverable by re-offering the same id rather than by minting a second
    /// attempt against an item whose budget the first one spent.
    fn open_attempt(
        &self,
        lease: &LeaseRef,
        new: &NewAttempt,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    fn settle_attempt(
        &self,
        lease: &LeaseRef,
        attempt: AttemptRef,
        verdict: &AttemptVerdict,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<BindDisposition, LedgerError>> + Send;

    /// The reauth gate, which outlives the lease that raised it.
    fn gate_connection(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    /// The only halt reachable from here, fixed to this tenant's inventory.
    fn halt_this_tenant(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
        reason: &str,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    /// The window and the ceiling are the server's, derived from its own copy
    /// of the limits rather than from anything said here.
    fn request_grant(
        &self,
        lease: &LeaseRef,
        connection: ConnectionId,
        kind: GrantKind,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<BudgetGrant, LedgerError>> + Send;

    /// Extends this lease, fenced on its epoch.
    ///
    /// The reaper reclaims a lease whose expiry has passed, and before this
    /// existed that expiry was a fixed TTL: a device still working lost its
    /// item to a reaper that could not tell slow from gone. With a renew the
    /// two are distinguishable — a lease that stopped being extended is one
    /// whose holder stopped, which on a laptop that closed its lid is the
    /// common case rather than the exception.
    ///
    /// The caller states no duration. The reaper reads the new expiry against
    /// the server's clock, so the server mints it from its own constant; a
    /// duration the caller supplied would be a device setting the length of
    /// the lease it holds.
    ///
    /// The caller states no instant either: a renew writes no `job_event` and
    /// no `write_attempt` row, so there is nothing for an asserted instant to
    /// be recorded beside.
    ///
    /// Answers both of the server's own instants, so a device moves its run
    /// deadline by their difference and never by arithmetic of its own.
    fn renew(
        &self,
        lease: &LeaseRef,
    ) -> impl core::future::Future<Output = Result<Renewed, LedgerError>> + Send;

    fn record_event(
        &self,
        lease: &LeaseRef,
        payload: &JobEventPayload,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;

    /// The event is closed and the topic is a total function of it, so no
    /// relay name crosses this boundary.
    fn notify(
        &self,
        lease: &LeaseRef,
        event: SellerEvent,
        at: Timestamp,
    ) -> impl core::future::Future<Output = Result<(), LedgerError>> + Send;
}

/// What a test may observe of a ledger, and nothing a client may.
///
/// It exists so one test body can assert against two implementations. Every
/// method is a read of state the interpreter already caused; none of them
/// writes, and none exposes an operation absent from [`ItemLedger`], which is
/// the property that keeps the conformance suite from proving the boundary
/// and widening it in the same stroke.
///
/// It has no wire form and is never served: the Postgres implementation lives
/// beside the repositories it reads, and the in-memory one is the fixture
/// itself.
pub trait LedgerInspector {
    /// The item row: state, outcome, what it is blocked on, and the failure
    /// pair a settle recorded.
    fn item(&self, item: JobItemId) -> impl core::future::Future<Output = ItemObservation> + Send;

    /// The fencing attempt for a mapping, if one was ever opened.
    fn attempt(
        &self,
        mapping: MappingId,
    ) -> impl core::future::Future<Output = Option<AttemptObservation>> + Send;

    /// What the mapping is bound to, and how stale the binding's verification
    /// is. `None` where no mapping row exists.
    fn binding(
        &self,
        mapping: MappingId,
    ) -> impl core::future::Future<Output = Option<BindingObservation>> + Send;

    /// The connection's lifecycle state for this organisation's inventory.
    fn connection_state(
        &self,
        org: OrgId,
        inventory: InventoryId,
    ) -> impl core::future::Future<Output = Option<String>> + Send;

    /// How many tenant-inventory halts stand for this organisation.
    fn halt_count(&self, org: OrgId) -> impl core::future::Future<Output = usize> + Send;

    /// Queued seller notifications, filtered by topic where one is named.
    fn outbox_count(
        &self,
        org: OrgId,
        topic: Option<&str>,
    ) -> impl core::future::Future<Output = usize> + Send;

    /// Events recorded against this item.
    fn event_count(&self, item: JobItemId) -> impl core::future::Future<Output = usize> + Send;
}

/// The item row as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemObservation {
    pub state: String,
    pub outcome: Option<String>,
    pub blocked_on: Option<String>,
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub preflight_failures: i32,
}

/// The fencing attempt as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptObservation {
    pub state: String,
    pub settled: bool,
    pub remote_id_kind: Option<String>,
    pub remote_url: Option<String>,
    pub remote_numeric_id: Option<i64>,
}

/// The mapping's binding as a test reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingObservation {
    pub binding_state: String,
    pub remote_id_kind: Option<String>,
    pub remote_url: Option<String>,
    pub verify_state: String,
    pub never_verified: bool,
    pub stale_since_first_seen: bool,
}
