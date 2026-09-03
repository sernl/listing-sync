//! An in-memory ledger, so the interpreter's own suite can run without a
//! database.
//!
//! It exists to falsify rather than to serve: the same test bodies run against
//! this and against Postgres, and a storage assumption that leaked into the
//! interpreter shows up as a divergence between the two runs instead of as a
//! compile error somebody quietly fixes by hand. It models exactly the state
//! the ledger port can change, which is why it is small; anything it cannot
//! answer is an assertion reaching past the boundary.

use std::collections::HashMap;
// `clippy.toml` disallows `std::sync::Mutex` and prescribes `tokio::sync::Mutex`,
// which `just purity` bans from this crate: the interpreter runs on the
// seller's device and takes no runtime. The stated hazard is poisoning
// cascading through a server, and neither half of it reaches here — this is a
// test fixture with no request to unwind, and `lock` recovers the guard from a
// poisoned mutex rather than unwrapping it. Narrow by construction: the type
// appears twice, both inside this fixture.
#[expect(
    clippy::disallowed_types,
    reason = "the prescribed tokio::sync::Mutex is banned from this crate by just purity; poisoning is recovered from below rather than unwrapped"
)]
use std::sync::Mutex;

use tam_domain::{JobItemId, SellerEvent};
use tam_types::{ConnectionId, InventoryId, JobEventPayload, MappingId, OrgId, Timestamp, Uuid};

use crate::ports::{
    AttemptObservation, BindingObservation, ItemLedger, ItemObservation, LedgerInspector,
};
use crate::vocabulary::{
    AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, GrantKind, ItemVerdict,
    LandingEffect, LeaseRef, LedgerError, NewAttempt, PreflightStreak, Renewed,
};

#[derive(Debug, Clone)]
struct Item {
    state: String,
    outcome: Option<String>,
    blocked_on: Option<String>,
    failure_code: Option<String>,
    failure_detail: Option<String>,
    preflight_failures: i32,
    preflight_edge_only: bool,
    lease_epoch: i64,
}

#[derive(Debug, Clone)]
struct Attempt {
    id: Uuid,
    state: String,
    settled: bool,
    remote_id_kind: Option<String>,
    remote_url: Option<String>,
    remote_numeric_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct Binding {
    state: String,
    remote_id_kind: Option<String>,
    remote_url: Option<String>,
    verify_state: String,
    never_verified: bool,
    stale_since_first_seen: bool,
}

#[derive(Debug, Default)]
struct State {
    items: HashMap<JobItemId, Item>,
    attempts: HashMap<MappingId, Attempt>,
    bindings: HashMap<MappingId, Binding>,
    connections: HashMap<(OrgId, InventoryId), (ConnectionId, String)>,
    halts: Vec<(OrgId, InventoryId)>,
    outbox: Vec<(OrgId, String)>,
    events: Vec<(JobItemId, String)>,
    /// Grants already issued in the current window, keyed by connection.
    grants: HashMap<ConnectionId, i32>,
    ceiling: i32,
}

/// What this fixture answers a renew with, in milliseconds of lease.
///
/// Deliberately not the server's own TTL: the fixture keeps no clock, and a
/// number equal to the real one would let a test pass while the device was
/// computing the deadline itself rather than reading the server's answer.
const FIXTURE_RENEW_MS: i64 = 120_000;

/// A ledger held entirely in memory, seeded by the test that drives it.
#[expect(
    clippy::disallowed_types,
    reason = "see the import: the prescribed replacement is a runtime dependency this crate refuses"
)]
pub struct InMemoryLedger {
    state: Mutex<State>,
}

/// How a fixture describes the item it is about to drive.
#[derive(Debug, Clone)]
pub struct Seeded {
    pub org: OrgId,
    pub item: JobItemId,
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub connection: ConnectionId,
    pub lease_epoch: i64,
    /// How many grants the connection's window still allows. The default is
    /// generous; a test that means to exhaust it says so.
    pub rate_ceiling: i32,
}

impl InMemoryLedger {
    /// A ledger holding one leased item, one linked connection and an unbound
    /// mapping — the state `seed()` leaves behind on the Postgres side.
    #[must_use]
    pub fn seeded(seed: &Seeded) -> Self {
        let mut state = State {
            ceiling: seed.rate_ceiling,
            ..State::default()
        };
        state.items.insert(
            seed.item,
            Item {
                state: "leased".to_owned(),
                outcome: None,
                blocked_on: None,
                failure_code: None,
                failure_detail: None,
                preflight_failures: 0,
                preflight_edge_only: true,
                lease_epoch: seed.lease_epoch,
            },
        );
        state.bindings.insert(
            seed.mapping,
            Binding {
                state: "unbound".to_owned(),
                remote_id_kind: None,
                remote_url: None,
                verify_state: "unverified".to_owned(),
                never_verified: true,
                stale_since_first_seen: false,
            },
        );
        state.connections.insert(
            (seed.org, seed.inventory),
            (seed.connection, "linked".to_owned()),
        );
        Self {
            #[expect(
                clippy::disallowed_types,
                reason = "see the import: the prescribed replacement is a runtime dependency this crate refuses"
            )]
            state: Mutex::new(state),
        }
    }

    /// The attempt-count half of the reaper's give-up predicate, which the
    /// Postgres fixture sets with an UPDATE.
    pub fn set_preflight_failures(&self, item: JobItemId, failures: i32) {
        self.with(|state| {
            if let Some(row) = state.items.get_mut(&item) {
                row.preflight_failures = failures;
            }
        });
    }

    /// The only place the guard is taken, so its scope is one closure
    /// everywhere and no caller can hold it across an await.
    ///
    /// Poisoning is recovered from rather than propagated: a fixture whose
    /// assertion panicked mid-write still holds the state the next assertion
    /// needs to read, and there is no request here for a cascade to reach.
    fn with<T>(&self, act: impl FnOnce(&mut State) -> T) -> T {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        act(&mut guard)
    }

    /// The epoch fence, applied the way the server applies it: a write naming
    /// a stale epoch matches no row.
    fn fenced(state: &State, lease: &LeaseRef) -> Result<(), LedgerError> {
        match state.items.get(&lease.item) {
            Some(item) if item.lease_epoch == lease.lease_epoch => Ok(()),
            Some(_) => Err(LedgerError::StaleLease),
            None => Err(LedgerError::Refused {
                detail: "no such item".to_owned(),
            }),
        }
    }
}

impl ItemLedger for InMemoryLedger {
    async fn connection_for(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
    ) -> Result<Option<ConnectionId>, LedgerError> {
        Ok(self.with(|state| {
            state
                .connections
                .get(&(lease.org, inventory))
                .map(|(id, _)| *id)
        }))
    }

    async fn preflight_succeeded(&self, lease: &LeaseRef) -> Result<(), LedgerError> {
        self.with(|state| {
            Self::fenced(state, lease)?;
            if let Some(item) = state.items.get_mut(&lease.item) {
                item.preflight_failures = 0;
                item.preflight_edge_only = true;
            }
            Ok(())
        })
    }

    async fn preflight_failed(
        &self,
        lease: &LeaseRef,
        edge_class: bool,
    ) -> Result<PreflightStreak, LedgerError> {
        self.with(|state| {
            Self::fenced(state, lease)?;
            let item = state.items.get_mut(&lease.item).ok_or_else(missing_item)?;
            item.preflight_failures = item.preflight_failures.saturating_add(1);
            item.preflight_edge_only = item.preflight_edge_only && edge_class;
            Ok(PreflightStreak {
                failures: u32::try_from(item.preflight_failures).unwrap_or(u32::MAX),
                edge_only: item.preflight_edge_only,
            })
        })
    }

    async fn park(&self, lease: &LeaseRef, blocked_on: &str) -> Result<(), LedgerError> {
        self.with(|state| {
            Self::fenced(state, lease)?;
            if let Some(item) = state.items.get_mut(&lease.item) {
                "parked_live".clone_into(&mut item.state);
                item.blocked_on = Some(blocked_on.to_owned());
            }
            Ok(())
        })
    }

    async fn settle_item(
        &self,
        lease: &LeaseRef,
        verdict: &ItemVerdict,
        _at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.with(|state| {
            Self::fenced(state, lease)?;
            if let Some(item) = state.items.get_mut(&lease.item) {
                "settled".clone_into(&mut item.state);
                item.outcome = Some(format!("{:?}", verdict.outcome).to_lowercase());
                item.failure_code = verdict.failure_code.map(|code| format!("{code:?}"));
                item.failure_detail = verdict.failure_detail.as_ref().map(|d| d.0.clone());
            }
            Ok(())
        })
    }

    async fn open_attempt(
        &self,
        lease: &LeaseRef,
        new: &NewAttempt,
        _at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.with(|state| {
            Self::fenced(state, lease)?;
            // `write_attempt_one_in_flight` as structure: one unsettled
            // attempt per mapping is the only fence against a second create.
            if state
                .attempts
                .get(&new.mapping)
                .is_some_and(|attempt| !attempt.settled)
            {
                return Err(LedgerError::AttemptInFlight);
            }
            state.attempts.insert(
                new.mapping,
                Attempt {
                    id: new.attempt,
                    state: "in_flight".to_owned(),
                    settled: false,
                    remote_id_kind: None,
                    remote_url: None,
                    remote_numeric_id: None,
                },
            );
            Ok(())
        })
    }

    async fn settle_attempt(
        &self,
        lease: &LeaseRef,
        attempt: AttemptRef,
        verdict: &AttemptVerdict,
        _at: Timestamp,
    ) -> Result<BindDisposition, LedgerError> {
        self.with(|state| {
            Self::fenced(state, lease)?;
            let (kind, url, numeric) = addressed_columns(&verdict.landing);
            let row =
                state
                    .attempts
                    .get_mut(&attempt.mapping)
                    .ok_or_else(|| LedgerError::Refused {
                        detail: "no open attempt for that mapping".to_owned(),
                    })?;
            if row.id != attempt.attempt {
                return Err(LedgerError::StaleLease);
            }
            row.state.clone_from(&verdict.state);
            row.settled = true;
            row.remote_id_kind.clone_from(&kind);
            row.remote_url.clone_from(&url);
            row.remote_numeric_id = numeric;
            let binding =
                state
                    .bindings
                    .get_mut(&attempt.mapping)
                    .ok_or_else(|| LedgerError::Refused {
                        detail: "no mapping for that attempt".to_owned(),
                    })?;
            Ok(match &verdict.landing {
                LandingEffect::None => BindDisposition::NotLanded,
                LandingEffect::Addressed { .. } => BindDisposition::Addressed,
                LandingEffect::Landed { .. } => {
                    if binding.state == "bound" {
                        BindDisposition::AlreadyBound
                    } else {
                        "bound".clone_into(&mut binding.state);
                        binding.remote_id_kind.clone_from(&kind);
                        binding.remote_url.clone_from(&url);
                        // The report the settle bound on was never
                        // normalised, so the binding is stale from the
                        // instant it is first seen.
                        "stale".clone_into(&mut binding.verify_state);
                        binding.never_verified = true;
                        binding.stale_since_first_seen = true;
                        BindDisposition::Bound
                    }
                }
                LandingEffect::Severed { .. } => {
                    if binding.state == "bound" {
                        "unbound".clone_into(&mut binding.state);
                        BindDisposition::Severed
                    } else {
                        BindDisposition::SeverRefused {
                            state: binding.state.clone(),
                        }
                    }
                }
            })
        })
    }

    async fn gate_connection(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
        _at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.with(|state| {
            if let Some((_, connection)) = state.connections.get_mut(&(lease.org, inventory)) {
                "needs_reauth".clone_into(connection);
            }
        });
        Ok(())
    }

    async fn halt_this_tenant(
        &self,
        lease: &LeaseRef,
        inventory: InventoryId,
        _reason: &str,
        _at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.with(|state| {
            if !state.halts.contains(&(lease.org, inventory)) {
                state.halts.push((lease.org, inventory));
            }
        });
        Ok(())
    }

    async fn request_grant(
        &self,
        _lease: &LeaseRef,
        connection: ConnectionId,
        _kind: GrantKind,
        _at: Timestamp,
    ) -> Result<BudgetGrant, LedgerError> {
        Ok(self.with(|state| {
            let ceiling = state.ceiling;
            let used = state.grants.entry(connection).or_insert(0);
            if *used >= ceiling {
                return BudgetGrant::Exhausted;
            }
            *used += 1;
            BudgetGrant::Granted { used: *used }
        }))
    }

    async fn renew(&self, lease: &LeaseRef) -> Result<Renewed, LedgerError> {
        self.with(|state| Self::fenced(state, lease))?;
        Ok(Renewed {
            server_now_ms: 0,
            server_deadline_ms: FIXTURE_RENEW_MS,
        })
    }

    async fn record_event(
        &self,
        lease: &LeaseRef,
        payload: &JobEventPayload,
        _at: Timestamp,
    ) -> Result<(), LedgerError> {
        self.with(|state| state.events.push((lease.item, format!("{payload:?}"))));
        Ok(())
    }

    async fn notify(
        &self,
        lease: &LeaseRef,
        event: SellerEvent,
        _at: Timestamp,
    ) -> Result<(), LedgerError> {
        // The topic is the server's to derive, exactly as it is in Postgres;
        // mirroring the mapping here is what lets a test assert on it.
        let topic = match event {
            SellerEvent::ReauthRequired | SellerEvent::ItemParked => "email.parked_job",
            SellerEvent::JobSettled | SellerEvent::InventoryHalted => "email.job_settled",
        };
        self.with(|state| state.outbox.push((lease.org, topic.to_owned())));
        Ok(())
    }
}

fn missing_item() -> LedgerError {
    LedgerError::Refused {
        detail: "no such item".to_owned(),
    }
}

/// The three remote-id columns every settled attempt records, whatever the
/// write did to the binding.
fn addressed_columns(landing: &LandingEffect) -> (Option<String>, Option<String>, Option<i64>) {
    use tam_marketplace::RemoteListingId;
    let id = match landing {
        LandingEffect::None => return (None, None, None),
        LandingEffect::Addressed { id }
        | LandingEffect::Landed { id, .. }
        | LandingEffect::Severed { id } => id,
    };
    match id {
        RemoteListingId::Tes { url } => (Some("tes".to_owned()), Some(url.clone()), None),
        RemoteListingId::Tpt { product_id } => (
            Some("tpt".to_owned()),
            None,
            Some(i64::try_from(*product_id).unwrap_or(i64::MAX)),
        ),
        RemoteListingId::Etsy { listing_id } => (
            Some("etsy".to_owned()),
            None,
            Some(i64::try_from(*listing_id).unwrap_or(i64::MAX)),
        ),
    }
}

impl LedgerInspector for InMemoryLedger {
    async fn item(&self, item: JobItemId) -> ItemObservation {
        self.with(|state| {
            state
                .items
                .get(&item)
                .map_or_else(ItemObservation::default, |row| ItemObservation {
                    state: row.state.clone(),
                    outcome: row.outcome.clone(),
                    blocked_on: row.blocked_on.clone(),
                    failure_code: row.failure_code.clone(),
                    failure_detail: row.failure_detail.clone(),
                    preflight_failures: row.preflight_failures,
                })
        })
    }

    async fn attempt(&self, mapping: MappingId) -> Option<AttemptObservation> {
        self.with(|state| {
            state.attempts.get(&mapping).map(|row| AttemptObservation {
                state: row.state.clone(),
                settled: row.settled,
                remote_id_kind: row.remote_id_kind.clone(),
                remote_url: row.remote_url.clone(),
                remote_numeric_id: row.remote_numeric_id,
            })
        })
    }

    async fn binding(&self, mapping: MappingId) -> Option<BindingObservation> {
        self.with(|state| {
            state.bindings.get(&mapping).map(|row| BindingObservation {
                binding_state: row.state.clone(),
                remote_id_kind: row.remote_id_kind.clone(),
                remote_url: row.remote_url.clone(),
                verify_state: row.verify_state.clone(),
                never_verified: row.never_verified,
                stale_since_first_seen: row.stale_since_first_seen,
            })
        })
    }

    async fn connection_state(&self, org: OrgId, inventory: InventoryId) -> Option<String> {
        self.with(|state| {
            state
                .connections
                .get(&(org, inventory))
                .map(|(_, connection)| connection.clone())
        })
    }

    async fn halt_count(&self, org: OrgId) -> usize {
        self.with(|state| state.halts.iter().filter(|(o, _)| *o == org).count())
    }

    async fn outbox_count(&self, org: OrgId, topic: Option<&str>) -> usize {
        self.with(|state| {
            state
                .outbox
                .iter()
                .filter(|(o, t)| *o == org && topic.is_none_or(|wanted| wanted == t))
                .count()
        })
    }

    async fn event_count(&self, item: JobItemId) -> usize {
        self.with(|state| state.events.iter().filter(|(i, _)| *i == item).count())
    }
}
