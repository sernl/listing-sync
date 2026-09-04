//! The entitlement token's claim set, defined once for both ends of the wire.
//!
//! Decision D10 accepts exactly one client-side claim, and this is its
//! vocabulary. The server mints these claims and signs them; the seller's
//! device verifies the signature and asks its own gate. Neither end owns the
//! shape, because two copies of a wire contract are two things to keep in
//! step: `tam-api` serialises this type and `tam-desktop` deserialises it, and
//! a field added on one side is a compile error on the other rather than a
//! token nobody reads.
//!
//! Nothing here signs or verifies anything. The signature stack is
//! `jsonwebtoken` over Ed25519 at both ends, and it stays out of this crate so
//! the pure core keeps its dependency fence.
//!
//! What the claims are *for* is narrower than what they say. The token
//! transports a decision Postgres already made so a device can run scheduled
//! no-API work between check-ins; the server re-checks Postgres on every
//! control-plane call regardless, and the device's gate can only ever refuse.

use serde::{Deserialize, Serialize};
use tam_types::Marketplace;

use crate::{ENTITLEMENT_GRACE_HOURS, ENTITLEMENT_TOKEN_VALIDITY_SECS};

/// The audience every accepted token must name, fixed rather than configured:
/// a build able to widen this would accept a token minted for something else.
pub const AUDIENCE: &str = "tam-desktop";

/// The issuer every accepted token must name.
pub const ISSUER: &str = "tam-server";

const SECS_PER_HOUR: i64 = 3_600;

/// What the server signed. Nothing here is consulted for anything but the
/// device's own gate; no claim names an organisation's permissions, because
/// the server decides those from Postgres on every call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The account the device is entitled under, as the organisation's
    /// hyphenated identifier. Carried so a token is attributable, and read by
    /// nothing: authorisation is decided in Postgres and never asserted here.
    pub sub: String,
    pub aud: String,
    pub iss: String,
    /// The device this token was minted for, as `DeviceId` spells it.
    ///
    /// A token is useless on any machine that does not hold this id. Not quite
    /// "any other machine": `device`'s primary key is `(org_id, id)` and the
    /// register route takes a client-chosen id, so two tenants may hold the same
    /// one, and `sub` is verified by nothing — such tokens are interchangeable.
    /// No exploit follows, because obtaining another tenant's token needs that
    /// tenant's session cookie and the gate is advisory either way, but the
    /// binding is to an id rather than to a machine and the difference is worth
    /// stating where a reader will rely on it.
    pub device: String,
    /// The marketplaces this device may work. A marketplace absent here is
    /// revoked, which is the per-marketplace kill switch.
    pub marketplaces: Vec<Marketplace>,
    /// When revalidation is due, in seconds since the epoch. Seconds because
    /// that is what RFC 7519 fixes `exp` to be; `tam_types::Timestamp` is
    /// milliseconds, and the two are converted at every comparison rather than
    /// assumed to agree.
    pub exp: i64,
    /// The last second at which work is still permitted without a successful
    /// revalidation. Never earlier than `exp`.
    pub grace: i64,
}

impl Claims {
    /// The claim set for one device, with D11's two deadlines computed from
    /// the constants rather than from arguments.
    ///
    /// Neither window is a parameter, for the reason `LedgerCall::Renew` gives
    /// for the lease: a caller able to name its own validity could buy itself
    /// a token of any length. `issued_at_secs` is the server's own clock,
    /// which is the only input a mint needs.
    ///
    /// Saturating rather than checked: at the saturation point `grace` and
    /// `exp` are both `i64::MAX`, which satisfies the verifier's requirement
    /// that grace never precede expiry, and a clock that far out is a fault
    /// the device answers by refusing on its own comparison.
    #[must_use]
    pub fn mint(
        subject: String,
        device: String,
        marketplaces: Vec<Marketplace>,
        issued_at_secs: i64,
    ) -> Self {
        let exp = issued_at_secs.saturating_add(ENTITLEMENT_TOKEN_VALIDITY_SECS);
        Self {
            sub: subject,
            aud: AUDIENCE.to_owned(),
            iss: ISSUER.to_owned(),
            device,
            marketplaces,
            exp,
            grace: exp.saturating_add(i64::from(ENTITLEMENT_GRACE_HOURS) * SECS_PER_HOUR),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Claims, AUDIENCE, ISSUER};
    use tam_types::Marketplace;

    const NOW: i64 = 1_756_000_000;

    fn minted() -> Claims {
        Claims::mint(
            "11111111-2222-3333-4444-555555555555".to_owned(),
            "11112222333344445555666677778888".to_owned(),
            vec![Marketplace::Tpt, Marketplace::Tes],
            NOW,
        )
    }

    #[test]
    fn the_two_deadlines_are_d11s_numbers_and_grace_never_precedes_expiry() {
        let claims = minted();
        assert_eq!(claims.exp, NOW + 3_600, "one hour of validity");
        assert_eq!(
            claims.grace,
            claims.exp + 86_400,
            "and twenty-four hours of grace after it"
        );
        assert!(
            claims.grace >= claims.exp,
            "a grace before the expiry is refused by the verifier as malformed"
        );
    }

    #[test]
    fn a_clock_at_the_end_of_time_still_mints_a_verifiable_ordering() {
        let claims = Claims::mint(
            "org".to_owned(),
            "device".to_owned(),
            vec![Marketplace::Tpt],
            i64::MAX,
        );
        assert!(
            claims.grace >= claims.exp,
            "saturation must not invert the two deadlines, which would make every \
             token from such a clock malformed rather than merely expired"
        );
    }

    #[test]
    fn the_audience_and_issuer_are_the_fixed_pair_the_verifier_demands() {
        let claims = minted();
        assert_eq!(claims.aud, AUDIENCE);
        assert_eq!(claims.iss, ISSUER);
    }

    #[test]
    fn the_grant_set_is_carried_verbatim_and_in_order() {
        let claims = minted();
        assert_eq!(
            claims.marketplaces,
            vec![Marketplace::Tpt, Marketplace::Tes],
            "the mint decides the set; nothing here may reorder or widen it"
        );
    }
}
