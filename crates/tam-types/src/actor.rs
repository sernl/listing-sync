//! Who performed a write, on every row that records one.
//!
//! Attribution is mandatory rather than optional, and it is a sum rather than
//! a nullable user reference. A nullable user column cannot distinguish "the
//! engine did this" from "we failed to record who did this", and an incident
//! turns on exactly that difference: the first is the system working, the
//! second is the audit trail having a hole in it.

use serde::{Deserialize, Serialize};

use crate::{Timestamp, UserId};

/// A part of this system acting on its own schedule rather than on a
/// seller's request. Named individually because "not a person" is not an
/// answer: the useful question about an unattended write is which component
/// made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemComponent {
    /// The sync driver: leases, write attempts, and the item ledger.
    Engine,
    /// The credential broker, the only holder of the vault's key material.
    Broker,
    /// The cron-driven worker process that runs the lease scan.
    Worker,
    /// The import command, which runs unattended against a seller's export.
    Import,
    /// The interpreter running on a seller's own registered device.
    ///
    /// Distinct from `Engine` because after the two-branch split the same
    /// interpreter runs in two places, and an incident turns on which: an
    /// `Engine` row was written by infrastructure we operate, and a `Device`
    /// row was written by a process on hardware we do not. Collapsing them
    /// would attribute a seller's machine to our own engine, which is the
    /// audit trail having a hole in it rather than a shorter enum.
    Device,
    /// The scheduler's pass, inside the serving process.
    ///
    /// Its own variant rather than `Worker` for `Device`'s reason: a
    /// scheduled publish is a write nobody was present for, and the useful
    /// question about one is which clock reached it. `Worker` names the lease
    /// scan, which writes no listings, so attributing a Friday drop to it
    /// would leave nothing able to say a job came from a timetable.
    Scheduler,
}

impl SystemComponent {
    /// The closed set, in a stable order.
    pub const ALL: [Self; 6] = [
        Self::Engine,
        Self::Broker,
        Self::Worker,
        Self::Import,
        Self::Device,
        Self::Scheduler,
    ];

    /// The stored spelling, identical to the serde rename.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Broker => "broker",
            Self::Worker => "worker",
            Self::Import => "import",
            Self::Device => "device",
            Self::Scheduler => "scheduler",
        }
    }
}

/// Who a write is attributed to.
///
/// Stored as the pair (`actor_kind`, `actor_id`): the kind names the half of
/// the sum and the id carries its payload. The database's CHECK constraints
/// hold the same invariant this type holds — a person is always identified —
/// so a write bypassing this type cannot record an anonymous person either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Actor {
    /// A seller, acting through an authenticated request.
    Person(UserId),
    /// A component of this system, acting unattended.
    System(SystemComponent),
}

impl Actor {
    /// The `actor_kind` column's value.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Person(_) => "person",
            Self::System(_) => "system",
        }
    }

    /// The `actor_id` column's value. Never `NULL` for either half: a system
    /// component names itself, so an unattributed row is a row this type did
    /// not write.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::Person(user) => user.0.to_hyphenated(),
            Self::System(component) => component.as_str().to_owned(),
        }
    }
}

/// Who did this, and when. The two halves of an audit record's provenance,
/// travelling as one value.
///
/// They are paired rather than adjacent because every row that carries one
/// carries the other, and a signature taking them separately is one where an
/// instant and an author can be supplied from different events without
/// anything noticing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub at: Timestamp,
    pub actor: Actor,
}

impl Stamp {
    /// A stamp for an unattended write by one component.
    #[must_use]
    pub const fn system(component: SystemComponent, at: Timestamp) -> Self {
        Self {
            at,
            actor: Actor::System(component),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Actor, SystemComponent};
    use crate::{UserId, Uuid};

    #[test]
    fn a_person_is_identified_by_the_hyphenated_uuid() {
        let actor = Actor::Person(UserId(Uuid([0x0A; 16])));
        assert_eq!(actor.kind(), "person");
        assert_eq!(actor.id(), "0a0a0a0a-0a0a-0a0a-0a0a-0a0a0a0a0a0a");
    }

    #[test]
    fn every_system_component_names_itself() {
        for component in SystemComponent::ALL {
            let actor = Actor::System(component);
            assert_eq!(actor.kind(), "system");
            assert_eq!(actor.id(), component.as_str());
            assert!(
                !actor.id().is_empty(),
                "an unattended write still names its component"
            );
        }
    }

    #[test]
    fn a_system_stamp_carries_both_halves() {
        use super::Stamp;
        use crate::Timestamp;
        let stamp = Stamp::system(SystemComponent::Engine, Timestamp(7));
        assert_eq!(stamp.at, Timestamp(7));
        assert_eq!(stamp.actor.id(), "engine");
    }

    /// `pre-attribution` is migration 0033's backfill marker for rows written
    /// before attribution existed. No live component may collide with it, or
    /// a genuine gap in the trail becomes indistinguishable from a write.
    #[test]
    fn no_component_collides_with_the_backfill_marker() {
        for component in SystemComponent::ALL {
            assert_ne!(component.as_str(), "pre-attribution");
        }
    }
}
