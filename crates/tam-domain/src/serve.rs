//! The serve capability: what the server signs when it asks one of the
//! seller's devices for part of one file, defined once for both ends.
//!
//! An imported file stays on the seller's devices. When the seller opens one
//! in a browser, the server asks a device that holds it for the bytes and
//! passes them through without keeping them. Each ask carries one of these,
//! signed with the key entitlement tokens are signed with, so the device can
//! check that the server — and not whoever else can reach the channel —
//! asked for exactly this file, this range, this once, and soon.
//!
//! No seller credential travels the other way: the device answers under its
//! own session, and the capability is the server's word, not the seller's.
//!
//! Nothing here signs or verifies. `tam-api` signs with `jsonwebtoken` over
//! Ed25519 and `tam-desktop` verifies with the keys its build embeds, as for
//! [`crate::entitlement`].

use serde::{Deserialize, Serialize};

/// The audience a serve capability names. Distinct from the entitlement
/// token's so neither can be replayed as the other.
pub const AUDIENCE: &str = "tam-desktop/serve";

/// The issuer every accepted capability must name.
pub const ISSUER: &str = crate::entitlement::ISSUER;

/// How long a capability stands. Long enough for a device's long poll to
/// hand it over and the device to open its library; short enough that one
/// lifted from a log is worthless.
pub const VALIDITY_SECS: i64 = 120;

/// The header the device answers a stream under, carrying the capability it
/// was handed back to the server.
pub const CAPABILITY_HEADER: &str = "x-teachouse-capability";

/// The header a device refuses a stream under, with an empty body, naming a
/// [`Refusal`] by its wire word.
pub const REFUSED_HEADER: &str = "x-teachouse-serve-refused";

/// What the server signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The organisation, hyphenated. Attributable; read by nothing.
    pub sub: String,
    pub aud: String,
    pub iss: String,
    /// The device asked, as `DeviceId` spells it.
    pub device: String,
    /// The stream this answers, hyphenated. Used once: a device refuses a
    /// stream id it has already answered.
    pub stream: String,
    /// The file, as sixty-four lowercase hex characters.
    pub hash: String,
    /// The first byte asked for.
    pub first: u64,
    /// The last byte asked for, inclusive.
    pub last: u64,
    /// Seconds since the epoch.
    pub exp: i64,
}

impl Claims {
    /// How many bytes the answer carries.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.last.saturating_sub(self.first).saturating_add(1)
    }

    /// Never empty: a range is inclusive at both ends.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }
}

/// Why a device would not serve a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The device no longer holds the file.
    Missing,
    /// The capability did not verify, named another device or stream, had
    /// expired, or was used before.
    Capability,
    /// The range lies outside the file.
    Range,
}

impl Refusal {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Capability => "capability",
            Self::Range => "range",
        }
    }

    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "missing" => Some(Self::Missing),
            "capability" => Some(Self::Capability),
            "range" => Some(Self::Range),
            _ => None,
        }
    }
}

/// One stream a device's long poll hands over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamRequest {
    pub stream: String,
    pub hash: String,
    pub first: u64,
    pub last: u64,
    pub capability: String,
}

/// What a device's long poll answers: the streams waiting for it, possibly
/// none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StreamRequests {
    pub requests: Vec<StreamRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_round_trip_their_wire_word() {
        for refusal in [Refusal::Missing, Refusal::Capability, Refusal::Range] {
            assert_eq!(Refusal::from_word(refusal.word()), Some(refusal));
        }
        assert_eq!(Refusal::from_word("other"), None);
    }
}
