//! What a seller is told about a connection, and the one function that
//! decides it.
//!
//! The stored `connection.state` answers whether a link was ever made. It
//! does not answer whether the credential behind that link still works, and
//! those are different questions: a connection reads `linked` from the moment
//! the vault seals a secret until something fails against the marketplace,
//! which can be days after the session behind it died. Reporting `linked` as
//! "connected" therefore tells a seller their queue is fine while it is
//! stalled. The four values below are the seller-facing answer, derived from
//! the link state and the verification columns together.

use serde::{Deserialize, Serialize};

use crate::Timestamp;

/// How long a positive verification stands before it stops being evidence.
///
/// This is a backstop, not the primary deadline: the refresher writes
/// `session_refresh_after`, and a row past that instant is already reported
/// [`ConnectionStatus::Checking`] by the clause above this one. The window
/// only decides rows whose next verification is unset or far out, and it is
/// set so that any verification cadence shorter than a day leaves a healthy
/// connection `Connected`, while a connection nothing has verified for over a
/// day is reported `Checking` rather than asserted as working. If the
/// refresher's period ever exceeds this, this constant moves with it.
///
/// An ordinary `const` beside its caller rather than a `tam-limits` entry:
/// that crate admits a number only when exceeding it is a shared-resource
/// incident across tenants, and this one bounds nothing but the sentence
/// shown on one seller's connections page.
pub const VERIFICATION_FRESHNESS_MS: i64 = 24 * 60 * 60 * 1000;

/// What the connections page says about one marketplace link.
///
/// Four values rather than the five stored states, because the seller's
/// question is not which row state we are in but whether the connection is
/// carrying work: [`Self::Connected`] is a positive assertion backed by a
/// recent verification, and everything short of that evidence is reported as
/// one of the other three rather than rounded up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    /// Linked, and verified against the marketplace inside the freshness
    /// window. The only value that claims the connection works.
    Connected,
    /// Linked, with no failing verification, but no current proof either:
    /// never verified, a verification due or in flight, or a last success
    /// older than [`VERIFICATION_FRESHNESS_MS`].
    Checking,
    /// Linked, and verification is failing without yet having been classified
    /// as an authentication problem. Nothing is asked of the seller here; the
    /// distinction from `Disconnected` is exactly that re-linking is not
    /// known to be the remedy.
    Unstable,
    /// There is nothing to verify, or the seller must re-link before there
    /// is: revoked, never linked, or gated to `needs_reauth`.
    Disconnected,
}

impl ConnectionStatus {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 4] = [
        Self::Connected,
        Self::Checking,
        Self::Unstable,
        Self::Disconnected,
    ];

    /// The wire spelling, identical to the serde rename.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Checking => "checking",
            Self::Unstable => "unstable",
            Self::Disconnected => "disconnected",
        }
    }
}

/// What happened to a connection, as `connection_audit.event` records it.
///
/// Shared vocabulary rather than string literals at each writer, and it lives
/// in this crate rather than in `tam-storage` because the credential broker
/// writes some of these rows and deliberately does not depend on the storage
/// crate: that dependency would put the vault's own queries on the far side
/// of a privilege boundary the design keeps closed. The database's CHECK
/// constraint holds the same closed set from the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionEvent {
    /// A credential was sealed against this connection for the first time.
    Linked,
    /// The platform account behind the link was identified and the
    /// exclusivity lock taken.
    Claimed,
    /// A verification proved the session still live.
    Refreshed,
    /// A verification attempt failed without being classified as an
    /// authentication problem.
    RefreshFailed,
    /// The connection was gated: something classified the failure as
    /// authentication, and the seller must re-link.
    NeedsReauth,
    /// A credential was sealed against a connection that already had one.
    Relinked,
    /// The stored credential was tombstoned.
    Revoked,
    /// The link was given up and the account released.
    Unlinked,
}

impl ConnectionEvent {
    /// The closed set, in a stable order.
    pub const ALL: [Self; 8] = [
        Self::Linked,
        Self::Claimed,
        Self::Refreshed,
        Self::RefreshFailed,
        Self::NeedsReauth,
        Self::Relinked,
        Self::Revoked,
        Self::Unlinked,
    ];

    /// The stored spelling, identical to the serde rename and to the CHECK
    /// constraint migration 0032 spells.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Claimed => "claimed",
            Self::Refreshed => "refreshed",
            Self::RefreshFailed => "refresh_failed",
            Self::NeedsReauth => "needs_reauth",
            Self::Relinked => "relinked",
            Self::Revoked => "revoked",
            Self::Unlinked => "unlinked",
        }
    }
}

/// The stored `connection.state`, as a closed set rather than a string.
///
/// The column's own CHECK constraint holds this set from the other side; the
/// enum is what makes the status derivation exhaustive, so a state added
/// later is a compile error at the one place that has to decide what to tell
/// the seller about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Unlinked,
    Linking,
    Linked,
    NeedsReauth,
    Revoked,
}

impl ConnectionState {
    /// The closed set, in a stable order.
    pub const ALL: [Self; 5] = [
        Self::Unlinked,
        Self::Linking,
        Self::Linked,
        Self::NeedsReauth,
        Self::Revoked,
    ];

    /// The stored spelling, identical to the serde rename.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unlinked => "unlinked",
            Self::Linking => "linking",
            Self::Linked => "linked",
            Self::NeedsReauth => "needs_reauth",
            Self::Revoked => "revoked",
        }
    }

    /// Recovers the state from its stored spelling. `None` is a row the
    /// CHECK constraint should have refused, which the caller reports as a
    /// corrupt row rather than rounding to a status.
    #[must_use]
    pub fn from_db(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == raw)
    }
}

/// The verification columns of one `connection` row, as the derivation reads
/// them. A struct rather than four positional arguments because three of the
/// four are nullable and two are instants, which is exactly the shape an
/// argument-order slip goes unnoticed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionHealth {
    pub state: ConnectionState,
    /// When a verification last succeeded.
    pub session_verified_at: Option<Timestamp>,
    /// When the next verification comes due.
    pub session_refresh_after: Option<Timestamp>,
    /// Consecutive failed verification attempts since the last success.
    pub refresh_failures: i32,
}

/// Derives what the seller is told, from the row and the current instant.
///
/// Precedence, first match winning:
///
/// 1. `Revoked`, `Unlinked` and `NeedsReauth` are
///    [`ConnectionStatus::Disconnected`]. All three mean there is no
///    credential we may use; `NeedsReauth` is the engine's gate, and the
///    other two are an absent secret.
/// 2. `Linking` is [`ConnectionStatus::Checking`]. A link in flight is not a
///    seller action item, and calling it disconnected mid-flow would ask for
///    one that is already under way. It shares its answer with clause 6 but
///    not its reason.
/// 3. `Linked` with `refresh_failures > 0` is [`ConnectionStatus::Unstable`].
///    This ranks above the freshness clauses deliberately: a row can be
///    inside the window on its last success and already failing since, and
///    the failure is the newer fact.
/// 4. `Linked` and never verified is [`ConnectionStatus::Checking`].
/// 5. `Linked` and at or past `session_refresh_after` is
///    [`ConnectionStatus::Checking`] — the verification is due or running.
/// 6. `Linked` with a success inside [`VERIFICATION_FRESHNESS_MS`] is
///    [`ConnectionStatus::Connected`]; anything left is
///    [`ConnectionStatus::Checking`], being linked, not failing, and with no
///    recent proof. `Connected` is asserted only on evidence, so the residue
///    falls to `Checking` rather than there.
#[must_use]
pub fn connection_status(health: &ConnectionHealth, now: Timestamp) -> ConnectionStatus {
    match health.state {
        ConnectionState::Revoked | ConnectionState::Unlinked | ConnectionState::NeedsReauth => {
            ConnectionStatus::Disconnected
        }
        ConnectionState::Linking => ConnectionStatus::Checking,
        ConnectionState::Linked => {
            if health.refresh_failures > 0 {
                return ConnectionStatus::Unstable;
            }
            let Some(verified) = health.session_verified_at else {
                return ConnectionStatus::Checking;
            };
            if health
                .session_refresh_after
                .is_some_and(|due| now.0 >= due.0)
            {
                return ConnectionStatus::Checking;
            }
            if now.0.saturating_sub(verified.0) <= VERIFICATION_FRESHNESS_MS {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::Checking
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        connection_status, ConnectionHealth, ConnectionState, ConnectionStatus,
        VERIFICATION_FRESHNESS_MS,
    };
    use crate::Timestamp;

    const NOW: Timestamp = Timestamp(1_700_000_000_000);

    fn linked() -> ConnectionHealth {
        ConnectionHealth {
            state: ConnectionState::Linked,
            session_verified_at: Some(Timestamp(NOW.0 - 60_000)),
            session_refresh_after: Some(Timestamp(NOW.0 + 60_000)),
            refresh_failures: 0,
        }
    }

    #[test]
    fn a_recent_verification_is_the_only_thing_that_reads_connected() {
        assert_eq!(
            connection_status(&linked(), NOW),
            ConnectionStatus::Connected
        );
    }

    #[test]
    fn the_three_credential_absent_states_all_read_disconnected() {
        for state in [
            ConnectionState::Revoked,
            ConnectionState::Unlinked,
            ConnectionState::NeedsReauth,
        ] {
            let health = ConnectionHealth { state, ..linked() };
            assert_eq!(
                connection_status(&health, NOW),
                ConnectionStatus::Disconnected,
                "{} leaves nothing we may use",
                state.as_str()
            );
        }
    }

    #[test]
    fn a_link_in_flight_reads_checking_rather_than_disconnected() {
        let health = ConnectionHealth {
            state: ConnectionState::Linking,
            session_verified_at: None,
            session_refresh_after: None,
            ..linked()
        };
        assert_eq!(connection_status(&health, NOW), ConnectionStatus::Checking);
    }

    #[test]
    fn a_linked_row_never_verified_reads_checking() {
        let health = ConnectionHealth {
            session_verified_at: None,
            session_refresh_after: None,
            ..linked()
        };
        assert_eq!(connection_status(&health, NOW), ConnectionStatus::Checking);
    }

    #[test]
    fn a_verification_come_due_reads_checking() {
        let health = ConnectionHealth {
            session_refresh_after: Some(NOW),
            ..linked()
        };
        assert_eq!(
            connection_status(&health, NOW),
            ConnectionStatus::Checking,
            "at the instant it comes due, not only past it"
        );
    }

    #[test]
    fn a_failing_refresh_reads_unstable() {
        let health = ConnectionHealth {
            refresh_failures: 1,
            ..linked()
        };
        assert_eq!(connection_status(&health, NOW), ConnectionStatus::Unstable);
    }

    /// The one ordering the clauses genuinely contend over: a row whose last
    /// success is still inside the window and which has failed since. The
    /// failure is the newer fact, so `Unstable` outranks `Connected`.
    #[test]
    fn a_failure_since_a_fresh_success_outranks_that_success() {
        let health = ConnectionHealth {
            session_verified_at: Some(Timestamp(NOW.0 - 1_000)),
            refresh_failures: 2,
            ..linked()
        };
        assert_eq!(connection_status(&health, NOW), ConnectionStatus::Unstable);
    }

    /// The residue: linked, not failing, nothing due, and the last success
    /// older than the window. `Connected` is asserted on evidence only.
    #[test]
    fn a_stale_success_with_nothing_due_reads_checking_not_connected() {
        let health = ConnectionHealth {
            session_verified_at: Some(Timestamp(NOW.0 - VERIFICATION_FRESHNESS_MS - 1)),
            session_refresh_after: Some(Timestamp(NOW.0 + 60_000)),
            ..linked()
        };
        assert_eq!(connection_status(&health, NOW), ConnectionStatus::Checking);
    }

    #[test]
    fn the_window_boundary_is_inclusive() {
        let health = ConnectionHealth {
            session_verified_at: Some(Timestamp(NOW.0 - VERIFICATION_FRESHNESS_MS)),
            ..linked()
        };
        assert_eq!(connection_status(&health, NOW), ConnectionStatus::Connected);
    }

    /// The set is closed at the type, so a state the CHECK constraint should
    /// have refused never reaches the derivation at all.
    #[test]
    fn an_unrecognised_state_is_refused_at_the_parse() {
        assert_eq!(
            ConnectionState::from_db("something_the_check_constraint_forbids"),
            None
        );
        for state in ConnectionState::ALL {
            assert_eq!(ConnectionState::from_db(state.as_str()), Some(state));
        }
    }

    #[test]
    fn every_audit_event_spelling_round_trips_through_serde() {
        use super::ConnectionEvent;
        for event in ConnectionEvent::ALL {
            let json = serde_json::to_string(&event).expect("an event serialises");
            assert_eq!(json, format!("\"{}\"", event.as_str()));
        }
    }

    #[test]
    fn the_wire_spellings_are_the_founders_four_words() {
        let rendered: Vec<&str> = ConnectionStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            rendered,
            ["connected", "checking", "unstable", "disconnected"]
        );
        for status in ConnectionStatus::ALL {
            let json = serde_json::to_string(&status).expect("a status serialises");
            assert_eq!(
                json,
                format!("\"{}\"", status.as_str()),
                "as_str and serde must not drift"
            );
        }
    }
}
