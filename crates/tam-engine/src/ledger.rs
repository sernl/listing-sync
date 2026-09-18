//! The server's side of the driver's ledger port.
//!
//! Two things live here and nowhere else. The Postgres implementation of
//! [`ItemLedger`], which is the only code entitled to turn a lease reference
//! into a row; and the conversions between the driver's wire vocabulary and
//! `tam-storage`'s row vocabulary.
//!
//! Every conversion is a total match with no wildcard arm. That is the point
//! of having two vocabularies: a variant added on either side fails to
//! compile here rather than being silently mapped onto a neighbour, so the
//! client contract cannot drift with the schema.

use sqlx::PgPool;
use tam_domain::PARK_TTL_MS;
use tam_engine_driver::ports::ItemLedger;
use tam_engine_driver::vocabulary as wire;
use tam_storage::{
    append_event_asserted, EventScope, HaltCause, HaltRepo, LeaseRepo, NewOutboxMessage,
    OutboxRepo, RateBudgetRepo, StorageError, WriteAttemptRepo,
};
use tam_types::{
    ConnectionId, InventoryId, JobEventPayload, Stamp, SystemComponent, Timestamp, Uuid,
};

/// The seven design topics are fixed; the parked-job mail is also the reauth
/// prompt, and a halt notice rides the settled-job topic until the design
/// grows a dedicated one.
///
/// It lives on the server because the port carries the closed `SellerEvent`
/// and never a topic: a topic string crossing the boundary would name any
/// relay the drainer knows.
const fn seller_event_topic(event: tam_domain::SellerEvent) -> &'static str {
    use tam_domain::SellerEvent;
    match event {
        SellerEvent::ReauthRequired | SellerEvent::ItemParked => "email.parked_job",
        SellerEvent::JobSettled | SellerEvent::InventoryHalted => "email.job_settled",
    }
}

/// One lease reference, crossing from the wire vocabulary to storage's.
///
/// Public because the API's own handlers hold a wire lease and sometimes need
/// a storage one — releasing a claim they cannot use, for instance — and a
/// second conversion written at the call site is the drift this crate exists
/// to prevent.
#[must_use]
pub fn to_storage_lease(lease: &wire::LeaseRef) -> tam_storage::LeaseRef {
    tam_storage::LeaseRef {
        org: lease.org,
        item: lease.item,
        lease_epoch: lease.lease_epoch,
    }
}

/// One claimed item as the driver sees it. The scan is the server's, so this
/// runs once per lease on the way out.
#[must_use]
pub fn to_wire_item(item: &tam_storage::LeasedItem) -> wire::LeasedItem {
    wire::LeasedItem {
        org: item.org,
        item: item.item,
        job: item.job,
        mapping: item.mapping,
        inventory: item.inventory,
        idempotency_key: item.idempotency_key,
        operation: item.operation.clone(),
        lease_epoch: item.lease_epoch,
        attempt_count: item.attempt_count,
        requires_bound_on: item.requires_bound_on,
    }
}

fn to_storage_landing(landing: &wire::LandingEffect) -> tam_storage::LandingEffect {
    match landing {
        wire::LandingEffect::None => tam_storage::LandingEffect::None,
        wire::LandingEffect::Addressed { id } => {
            tam_storage::LandingEffect::Addressed { id: id.clone() }
        }
        wire::LandingEffect::Landed { id, lifecycle } => tam_storage::LandingEffect::Landed {
            id: id.clone(),
            lifecycle: lifecycle.clone(),
        },
        wire::LandingEffect::Severed { id } => {
            tam_storage::LandingEffect::Severed { id: id.clone() }
        }
    }
}

fn to_wire_disposition(disposition: tam_storage::BindDisposition) -> wire::BindDisposition {
    match disposition {
        tam_storage::BindDisposition::NotLanded => wire::BindDisposition::NotLanded,
        tam_storage::BindDisposition::Addressed => wire::BindDisposition::Addressed,
        tam_storage::BindDisposition::Bound => wire::BindDisposition::Bound,
        tam_storage::BindDisposition::AlreadyBound => wire::BindDisposition::AlreadyBound,
        tam_storage::BindDisposition::DivergentLanding { existing } => {
            wire::BindDisposition::DivergentLanding { existing }
        }
        tam_storage::BindDisposition::ClaimedElsewhere { existing_mapping } => {
            wire::BindDisposition::ClaimedElsewhere { existing_mapping }
        }
        tam_storage::BindDisposition::Refused { state } => wire::BindDisposition::Refused { state },
        tam_storage::BindDisposition::Severed => wire::BindDisposition::Severed,
        tam_storage::BindDisposition::SeverRefused { state } => {
            wire::BindDisposition::SeverRefused { state }
        }
        tam_storage::BindDisposition::SeverDiverged { existing } => {
            wire::BindDisposition::SeverDiverged { existing }
        }
    }
}

fn to_wire_grant(grant: tam_storage::BudgetGrant) -> wire::BudgetGrant {
    match grant {
        tam_storage::BudgetGrant::Granted { used } => wire::BudgetGrant::Granted { used },
        tam_storage::BudgetGrant::Exhausted => wire::BudgetGrant::Exhausted,
    }
}

/// The two conditions the interpreter branches on survive as themselves;
/// every other row becomes opaque text, because the interpreter's only answer
/// to one is the stall bias and `sqlx::Error` must not cross the boundary.
///
/// Public because `seed.rs` reaches storage directly and answers in the same
/// `EngineError`, so it needs the one conversion rather than a second one.
#[must_use]
pub fn to_wire_error(error: &StorageError) -> wire::LedgerError {
    match *error {
        StorageError::AttemptInFlight => wire::LedgerError::AttemptInFlight,
        StorageError::MappingAlreadyBound => wire::LedgerError::MappingAlreadyBound,
        StorageError::StaleLease => wire::LedgerError::StaleLease,
        StorageError::Db(_)
        | StorageError::TimestampOutOfRange { .. }
        | StorageError::CorruptRow { .. }
        | StorageError::OrgMismatch
        | StorageError::Inconsistent { .. }
        | StorageError::SellerRuleBlocked { .. }
        | StorageError::DuplicateIdempotencyKey { .. }
        | StorageError::ListingAlreadyBound
        | StorageError::InventoryMappingAlreadyExists => wire::LedgerError::Refused {
            detail: error.to_string(),
        },
    }
}

/// Every repository the interpreter is allowed to reach, behind one seam.
pub struct PgLedger {
    pool: PgPool,
    /// Set when this ledger serves a seller's device rather than our own
    /// worker. It decides two things: rows are attributed to `Device` rather
    /// than `Engine`, and the instant the caller supplies is recorded as the
    /// device's assertion beside our own receipt instead of being taken as
    /// our record.
    device: Option<String>,
    leases: LeaseRepo,
    halts: HaltRepo,
    attempts: WriteAttemptRepo,
    budgets: RateBudgetRepo,
    /// The job the lease belongs to, so `record_event` can scope an event the
    /// driver keys only by lease.
    job: tam_types::JobId,
}

impl PgLedger {
    #[must_use]
    pub fn new(pool: PgPool, job: tam_types::JobId) -> Self {
        Self {
            leases: LeaseRepo::new(pool.clone()),
            halts: HaltRepo::new(pool.clone()),
            attempts: WriteAttemptRepo::new(pool.clone()),
            budgets: RateBudgetRepo::new(pool.clone()),
            pool,
            device: None,
            job,
        }
    }

    /// The same ledger serving a named device, which changes how its writes
    /// are attributed and dated.
    #[must_use]
    pub fn for_device(pool: PgPool, job: tam_types::JobId, device: String) -> Self {
        Self {
            device: Some(device),
            ..Self::new(pool, job)
        }
    }

    /// The server's own clock, read at the boundary where a receipt is minted.
    #[expect(
        clippy::disallowed_methods,
        reason = "the receipt is by definition our clock rather than the caller's; reading it here is the whole point of recording two instants"
    )]
    fn receipt() -> Timestamp {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        Timestamp(i64::try_from(since).unwrap_or(i64::MAX))
    }

    /// Who a row is attributed to, and what its two instants are.
    ///
    /// A server write is stamped `Engine` at the instant it supplied, with no
    /// assertion to record. A device write is stamped `Device` at the server's
    /// own receipt, with the device's instant recorded beside it: the seller's
    /// clock is preserved as their claim rather than mistaken for our record.
    fn stamped(&self, at: Timestamp, receipt: Timestamp) -> (Stamp, Option<Timestamp>) {
        if self.device.is_some() {
            (Stamp::system(SystemComponent::Device, receipt), Some(at))
        } else {
            (Stamp::system(SystemComponent::Engine, at), None)
        }
    }

    async fn append_event(
        &self,
        scope: &EventScope,
        payload: &JobEventPayload,
        stamp: Stamp,
        asserted: Option<Timestamp>,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        tam_storage::pin_tenant(&mut tx, scope.org).await?;
        append_event_asserted(&mut tx, scope, payload, stamp, asserted).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn append_outbox(&self, message: &NewOutboxMessage) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        tam_storage::pin_tenant(&mut tx, message.org).await?;
        OutboxRepo::append(&mut tx, message).await?;
        tx.commit().await?;
        Ok(())
    }
}

impl ItemLedger for PgLedger {
    async fn connection_for(
        &self,
        lease: &wire::LeaseRef,
        inventory: InventoryId,
    ) -> Result<Option<ConnectionId>, wire::LedgerError> {
        self.leases
            .connection_for(lease.org, inventory)
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn preflight_succeeded(&self, lease: &wire::LeaseRef) -> Result<(), wire::LedgerError> {
        self.leases
            .preflight_succeeded(&to_storage_lease(lease))
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn preflight_failed(
        &self,
        lease: &wire::LeaseRef,
        edge_class: bool,
    ) -> Result<wire::PreflightStreak, wire::LedgerError> {
        self.leases
            .preflight_failed(&to_storage_lease(lease), edge_class)
            .await
            .map(|streak| wire::PreflightStreak {
                failures: streak.failures,
                edge_only: streak.edge_only,
            })
            .map_err(|error| to_wire_error(&error))
    }

    /// The span is [`PARK_TTL_MS`], the machine's own park window, supplied
    /// here rather than by the caller: on the device branch the caller is the
    /// party being parked.
    /// The gate is checked against the vocabulary before it is written. A
    /// device names it, `blocked_on` carries no database constraint, and an
    /// unchecked one would put a gate in the ledger that no revive arm matches
    /// and no console can label: a park nothing ever clears, written by the
    /// party the park is holding.
    async fn park(
        &self,
        lease: &wire::LeaseRef,
        blocked_on: &str,
    ) -> Result<(), wire::LedgerError> {
        if !tam_storage::ALL_GATES.contains(&blocked_on) {
            return Err(wire::LedgerError::Refused {
                detail: format!("{blocked_on} is not a gate this ledger knows"),
            });
        }
        self.leases
            .park(
                &to_storage_lease(lease),
                blocked_on,
                PARK_TTL_MS.div_euclid(1_000),
            )
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn settle_item(
        &self,
        lease: &wire::LeaseRef,
        verdict: &wire::ItemVerdict,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        let verdict = tam_storage::ItemVerdict {
            outcome: verdict.outcome,
            failure_code: verdict.failure_code,
            failure_detail: verdict.failure_detail.clone(),
        };
        let (stamp, asserted) = self.stamped(at, Self::receipt());
        self.leases
            .settle_recorded(&to_storage_lease(lease), &verdict, at, (stamp, asserted))
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn open_attempt(
        &self,
        lease: &wire::LeaseRef,
        new: &wire::NewAttempt,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        let intent = tam_storage::AttemptIntent {
            body: new.intent.body.clone(),
            hash: new.intent.hash.clone(),
        };
        let (stamp, asserted) = self.stamped(at, Self::receipt());
        self.attempts
            .open_asserted(
                &to_storage_lease(lease),
                new.attempt,
                &tam_storage::NewAttempt {
                    mapping: new.mapping,
                    intent: &intent,
                    stamp,
                },
                asserted,
            )
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn settle_attempt(
        &self,
        lease: &wire::LeaseRef,
        attempt: wire::AttemptRef,
        verdict: &wire::AttemptVerdict,
        at: Timestamp,
    ) -> Result<wire::BindDisposition, wire::LedgerError> {
        let verdict = tam_storage::AttemptVerdict {
            state: verdict.state.clone(),
            failure_code: verdict.failure_code,
            landing: to_storage_landing(&verdict.landing),
        };
        self.attempts
            .settle(
                &to_storage_lease(lease),
                tam_storage::AttemptRef {
                    attempt: attempt.attempt,
                    mapping: attempt.mapping,
                },
                &verdict,
                at,
            )
            .await
            .map(to_wire_disposition)
            .map_err(|error| to_wire_error(&error))
    }

    async fn gate_connection(
        &self,
        lease: &wire::LeaseRef,
        inventory: InventoryId,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        self.leases
            .gate_connection(lease.org, inventory, at)
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn halt_this_tenant(
        &self,
        lease: &wire::LeaseRef,
        inventory: InventoryId,
        reason: &str,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        self.halts
            .raise_org_inventory(
                lease.org,
                inventory,
                &HaltCause {
                    raised_by: "machine".to_owned(),
                    reason: reason.to_owned(),
                    at,
                },
            )
            .await
            .map_err(|error| to_wire_error(&error))
    }

    async fn request_grant(
        &self,
        lease: &wire::LeaseRef,
        connection: ConnectionId,
        kind: wire::GrantKind,
        at: Timestamp,
    ) -> Result<wire::BudgetGrant, wire::LedgerError> {
        // Every kind draws on the same per-connection window today. The
        // distinction is carried so the ceiling can differ without the
        // interpreter learning what either ceiling is.
        let ceiling = match kind {
            wire::GrantKind::Write | wire::GrantKind::VerifyRead | wire::GrantKind::FormRead => {
                i32::try_from(tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX.get())
                    .unwrap_or(i32::MAX)
            }
        };
        // Recomputed per call: `consume` is keyed on the window start, so one
        // window carried across a poll that straddles a minute boundary keeps
        // incrementing a bucket it has already left.
        let window = Timestamp(at.0 - at.0.rem_euclid(60_000));
        self.budgets
            .consume(lease.org, connection, window, ceiling)
            .await
            .map(to_wire_grant)
            .map_err(|error| to_wire_error(&error))
    }

    async fn renew(&self, lease: &wire::LeaseRef) -> Result<wire::Renewed, wire::LedgerError> {
        let renewed = self
            .leases
            .renew(&to_storage_lease(lease))
            .await
            .map_err(|error| to_wire_error(&error))?;
        // Both instants come out of the one statement, so the device's
        // remaining lease is a subtraction between two readings of the same
        // clock rather than across this process's and the database's.
        Ok(wire::Renewed {
            server_now_ms: renewed.server_now.0,
            server_deadline_ms: renewed.expires_at.0,
        })
    }

    async fn record_event(
        &self,
        lease: &wire::LeaseRef,
        payload: &JobEventPayload,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        // `settle_item` records this terminal event in the same storage
        // transaction that changes the item and releases its lease. The
        // driver's following call exists for ledgers whose settle operation
        // cannot do that atomically; repeating it here would expose two
        // terminal transitions for one item.
        if matches!(payload, JobEventPayload::ItemSettled { .. }) {
            return Ok(());
        }
        let (stamp, asserted) = self.stamped(at, Self::receipt());
        self.append_event(
            &EventScope {
                org: lease.org,
                job: self.job,
                item: Some(lease.item),
            },
            payload,
            stamp,
            asserted,
        )
        .await
        .map_err(|error| to_wire_error(&error))
    }

    async fn notify(
        &self,
        lease: &wire::LeaseRef,
        event: tam_domain::SellerEvent,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        self.append_outbox(&NewOutboxMessage {
            org: lease.org,
            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
            topic: seller_event_topic(event).to_owned(),
            dedupe_key: format!("{event:?}:{:02x?}", lease.item.0 .0),
            payload: serde_json::json!({ "event": format!("{event:?}") }),
            at,
        })
        .await
        .map_err(|error| to_wire_error(&error))
    }

    async fn hand_back(
        &self,
        lease: &wire::LeaseRef,
        at: Timestamp,
    ) -> Result<(), wire::LedgerError> {
        // The disposition is the repository's, taken from the item's own
        // state: this method carries nothing the caller chose. The attempt
        // ceiling is read from the limits here for the same reason the
        // rate ceiling is — it is ours, and a device naming it would be
        // setting its own.
        let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
        self.leases
            .hand_back(&to_storage_lease(lease), attempts_max, at)
            .await
            .map(drop)
            .map_err(|error| to_wire_error(&error))
    }
}

/// The wall clock as the driver's id source. `uuid::Uuid::new_v4` is the same
/// mint the outbox uses; it is a capability here only because the interpreter
/// crate takes no ambient randomness.
pub struct RandomIds;

impl tam_engine_driver::ports::IdSource for RandomIds {
    fn new_id(&self) -> Uuid {
        Uuid(*uuid::Uuid::new_v4().as_bytes())
    }
}

/// The host's cancellation, behind the interpreter's own capability so that
/// crate never names `tokio-util`.
pub struct TokenCancellation<'a>(pub &'a tokio_util::sync::CancellationToken);

impl tam_engine_driver::ports::Cancellation for TokenCancellation<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

/// The Postgres side of the conformance suite's observations.
///
/// Gated on `pg-tests` because it exists only so one test body can run against
/// two ledgers: it reads with runtime SQL and panics on a broken read, which is
/// right for a fixture and wrong for anything shipped. Nothing here writes, and
/// nothing here exposes an operation absent from [`ItemLedger`].
#[cfg(feature = "pg-tests")]
#[expect(
    clippy::expect_used,
    reason = "a conformance inspector whose read fails has a broken fixture, and should say so loudly rather than assert against a default"
)]
impl tam_engine_driver::ports::LedgerInspector for PgLedger {
    async fn item(&self, item: tam_domain::JobItemId) -> tam_engine_driver::ports::ItemObservation {
        let row: (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            i32,
        ) = sqlx::query_as(
            "SELECT state, outcome, blocked_on, failure_code, failure_detail, \
                        preflight_failures FROM job_item WHERE id = $1",
        )
        .bind(uuid::Uuid::from_bytes(item.0 .0))
        .fetch_one(&self.pool)
        .await
        .expect("the item row reads");
        tam_engine_driver::ports::ItemObservation {
            state: row.0,
            outcome: row.1,
            blocked_on: row.2,
            failure_code: row.3,
            failure_detail: row.4,
            preflight_failures: row.5,
        }
    }

    async fn attempt(
        &self,
        mapping: tam_types::MappingId,
    ) -> Option<tam_engine_driver::ports::AttemptObservation> {
        let row: Option<(String, bool, Option<String>, Option<String>, Option<i64>)> =
            sqlx::query_as(
                "SELECT state, settled_at IS NOT NULL, remote_id_kind, remote_url, \
                        remote_numeric_id FROM write_attempt WHERE mapping_id = $1 \
                 ORDER BY opened_at DESC LIMIT 1",
            )
            .bind(uuid::Uuid::from_bytes(mapping.0 .0))
            .fetch_optional(&self.pool)
            .await
            .expect("the attempt row reads");
        row.map(
            |(state, settled, remote_id_kind, remote_url, remote_numeric_id)| {
                tam_engine_driver::ports::AttemptObservation {
                    state,
                    settled,
                    remote_id_kind,
                    remote_url,
                    remote_numeric_id,
                }
            },
        )
    }

    async fn binding(
        &self,
        mapping: tam_types::MappingId,
    ) -> Option<tam_engine_driver::ports::BindingObservation> {
        let row: Option<(
            String,
            Option<String>,
            Option<String>,
            String,
            Option<bool>,
            Option<bool>,
        )> = sqlx::query_as(
            "SELECT binding_state, remote_id_kind, remote_url, verify_state, \
                        verified_at IS NULL, verify_stale_since = first_seen_at \
                 FROM mapping WHERE id = $1",
        )
        .bind(uuid::Uuid::from_bytes(mapping.0 .0))
        .fetch_optional(&self.pool)
        .await
        .expect("the mapping row reads");
        row.map(|r| tam_engine_driver::ports::BindingObservation {
            binding_state: r.0,
            remote_id_kind: r.1,
            remote_url: r.2,
            verify_state: r.3,
            never_verified: r.4.unwrap_or(true),
            stale_since_first_seen: r.5.unwrap_or(false),
        })
    }

    async fn connection_state(
        &self,
        org: tam_types::OrgId,
        inventory: InventoryId,
    ) -> Option<String> {
        sqlx::query_scalar("SELECT state FROM connection WHERE org_id = $1 AND marketplace = $2")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(marketplace_wire(inventory))
            .fetch_optional(&self.pool)
            .await
            .expect("the connection row reads")
    }

    async fn halt_count(&self, org: tam_types::OrgId) -> usize {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt WHERE org_id = $1")
                .bind(uuid::Uuid::from_bytes(org.0 .0))
                .fetch_one(&self.pool)
                .await
                .expect("the halt count reads");
        usize::try_from(count).unwrap_or(0)
    }

    async fn outbox_count(&self, org: tam_types::OrgId, topic: Option<&str>) -> usize {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox_message WHERE org_id = $1 \
               AND ($2::text IS NULL OR topic = $2)",
        )
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(topic)
        .fetch_one(&self.pool)
        .await
        .expect("the outbox count reads");
        usize::try_from(count).unwrap_or(0)
    }

    async fn event_count(&self, item: tam_domain::JobItemId) -> usize {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM job_event WHERE job_item_id = $1")
                .bind(uuid::Uuid::from_bytes(item.0 .0))
                .fetch_one(&self.pool)
                .await
                .expect("the event count reads");
        usize::try_from(count).unwrap_or(0)
    }
}

/// The wire spelling the `connection` row keys on. A connection is per
/// marketplace rather than per inventory.
#[cfg(feature = "pg-tests")]
fn marketplace_wire(inventory: InventoryId) -> &'static str {
    match inventory.marketplace() {
        tam_types::Marketplace::Tes => "tes",
        tam_types::Marketplace::Tpt => "tpt",
        tam_types::Marketplace::Etsy => "etsy",
    }
}
