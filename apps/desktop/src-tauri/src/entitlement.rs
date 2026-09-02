//! The entitlement token: the one claim this architecture accepts, and the
//! narrowest form of it.
//!
//! The charter says authorisation is decided in Rust from Postgres and is
//! never asserted by a token claim. Decision D10 records the single bounded
//! exception, and this module is it: Postgres remains the decision-maker, the
//! token only transports a decision Postgres already made so that a seller's
//! device can run scheduled no-API work between check-ins, and the server
//! re-checks Postgres on every control-plane call regardless of what the token
//! says. The gate below is secondary, fail-closed and advisory. It can never
//! grant what the server did not, because it can only ever refuse.
//!
//! The token is a compact JWS with `alg: EdDSA` over Ed25519, verified against
//! a key compiled into the binary. The format matches the assertion
//! `crates/tam-api/src/auth.rs` already verifies, and reuses the same crate,
//! because a second signature stack is a second thing to get wrong.
//!
//! Validity and grace are D11: one hour of validity and a twenty-four hour
//! grace, failing closed. Those numbers are the server's to set — they arrive
//! in the token — and the kill-switch latency the founder commits to publicly
//! is their sum.

use std::collections::HashSet;

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tam_types::{Marketplace, Timestamp};

use crate::device::DeviceId;

/// The audience every accepted token must name, fixed rather than configured:
/// a build able to widen this would accept a token minted for something else.
pub const AUDIENCE: &str = "tam-desktop";

/// The issuer every accepted token must name.
pub const ISSUER: &str = "tam-server";

/// The Ed25519 public key the shipping binary verifies against, as its raw
/// thirty-two bytes.
///
/// All zeroes is a placeholder and is not a valid Ed25519 point, so a build
/// carrying it verifies nothing and every gate answers no. That is the correct
/// failure for a placeholder: the founder supplies the real key, and until
/// then the client cannot be told it may work. See
/// `docs/notes/design/desktop-client.md`.
pub const EMBEDDED_PUBLIC_KEY: [u8; 32] = [0u8; 32];

/// What the server signed. Nothing here is consulted for anything but the gate
/// below; no claim names an organisation's permissions, because the server
/// decides those from Postgres on every call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The account the device is entitled under.
    pub sub: String,
    pub aud: String,
    pub iss: String,
    /// The device this token was minted for, as [`crate::device::DeviceId`]
    /// spells it. A token is useless on any other machine.
    pub device: String,
    /// The marketplaces this device may work. A marketplace absent here is
    /// revoked, which is the per-marketplace kill switch.
    pub marketplaces: Vec<Marketplace>,
    /// When revalidation is due, in seconds since the epoch. Seconds because
    /// that is what RFC 7519 fixes `exp` to be; `tam_types::Timestamp` is
    /// milliseconds, and the two are converted at every comparison below
    /// rather than assumed to agree.
    pub exp: i64,
    /// The last second at which work is still permitted without a successful
    /// revalidation. Never earlier than `exp`.
    pub grace: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntitlementError {
    /// The signature did not verify against the compiled-in key, or the token
    /// is not a well-formed EdDSA JWS, or a required claim is missing.
    Rejected(String),
    /// The token was minted for a different device.
    WrongDevice,
    /// `grace` precedes `exp`, which no honest issuer produces and which would
    /// otherwise make the grace period silently negative.
    GraceBeforeExpiry,
}

impl core::fmt::Display for EntitlementError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rejected(why) => write!(f, "the entitlement token was rejected: {why}"),
            Self::WrongDevice => f.write_str("the entitlement token names a different device"),
            Self::GraceBeforeExpiry => {
                f.write_str("the entitlement token's grace deadline precedes its expiry")
            }
        }
    }
}

impl core::error::Error for EntitlementError {}

/// A token that verified. Holding one is not permission; ask [`EntitlementGate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    claims: Claims,
}

impl Entitlement {
    /// Verifies a token against `public_key` and binds it to `device`.
    ///
    /// Expiry is decided by [`EntitlementGate::may_work`] rather than by the
    /// JWT library, which is why `validate_exp` is off: a token past `exp` but
    /// inside `grace` is still workable under D11, and the library would
    /// refuse it outright and collapse the grace period to nothing.
    pub fn verify(
        token: &str,
        public_key: &[u8],
        device: &DeviceId,
    ) -> Result<Self, EntitlementError> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[AUDIENCE]);
        validation.set_issuer(&[ISSUER]);
        validation.required_spec_claims =
            HashSet::from(["exp".to_owned(), "aud".to_owned(), "iss".to_owned()]);
        validation.validate_exp = false;

        let decoded = decode::<Claims>(token, &DecodingKey::from_ed_der(public_key), &validation)
            .map_err(|why| EntitlementError::Rejected(why.to_string()))?;
        let claims = decoded.claims;

        if claims.device != device.as_str() {
            return Err(EntitlementError::WrongDevice);
        }
        if claims.grace < claims.exp {
            return Err(EntitlementError::GraceBeforeExpiry);
        }
        Ok(Self { claims })
    }

    /// An entitlement whose claims bypassed verification. Test-only, and the
    /// reason [`Entitlement`]'s field is private: outside tests, the verifier
    /// is the only way to hold one.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn from_verified_claims(claims: Claims) -> Self {
        Self { claims }
    }

    #[must_use]
    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// Whether the device should check in with the server before working
    /// again. True from `exp` onward; work continues through the grace window
    /// while this is true, which is the point of having two deadlines.
    ///
    /// An overflow converting the claim to milliseconds answers yes, which is
    /// the fail-closed direction: check in rather than assume.
    #[must_use]
    pub fn needs_revalidation(&self, now: Timestamp) -> bool {
        millis(self.claims.exp).is_none_or(|exp| now.0 > exp)
    }
}

/// The advisory gate on the seller's own machine. Fail-closed by construction:
/// the only state it can be in without a token is the one that refuses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntitlementGate(Option<Entitlement>);

impl EntitlementGate {
    /// No token: every marketplace is refused.
    #[must_use]
    pub const fn closed() -> Self {
        Self(None)
    }

    #[must_use]
    pub const fn holding(entitlement: Entitlement) -> Self {
        Self(Some(entitlement))
    }

    #[must_use]
    pub const fn entitlement(&self) -> Option<&Entitlement> {
        self.0.as_ref()
    }

    /// May this device work `marketplace` right now?
    ///
    /// No token is no. A marketplace the token does not name is no, which is
    /// how one marketplace is revoked across the installed fleet within one
    /// revalidation window. Past the grace deadline is no.
    #[must_use]
    pub fn may_work(&self, marketplace: Marketplace, now: Timestamp) -> bool {
        let Some(entitlement) = self.0.as_ref() else {
            return false;
        };
        let Some(grace) = millis(entitlement.claims.grace) else {
            return false;
        };
        entitlement.claims.marketplaces.contains(&marketplace) && now.0 <= grace
    }
}

/// A JWT deadline in seconds, as the milliseconds `tam_types::Timestamp`
/// counts. `None` on overflow, and every caller treats that as "refuse".
const fn millis(seconds: i64) -> Option<i64> {
    seconds.checked_mul(1_000)
}

#[cfg(test)]
mod tests {
    use super::{Claims, Entitlement, EntitlementError, EntitlementGate, AUDIENCE, ISSUER};
    use crate::device::DeviceId;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use tam_types::{Marketplace, Timestamp};

    /// Seconds, because these are JWT claim values.
    const NOW: i64 = 1_756_000_000;
    /// D11's numbers: one hour of validity, twenty-four hours of grace.
    const VALIDITY: i64 = 3_600;
    const GRACE: i64 = 86_400;

    /// The same instant as a `Timestamp`, which counts milliseconds. The whole
    /// reason this helper exists rather than being inlined: the two units are
    /// one careless comparison apart, and `tam-api` already had to convert.
    const fn at(seconds: i64) -> Timestamp {
        Timestamp(seconds * 1_000)
    }

    struct TestKey {
        encoding: EncodingKey,
        public: Vec<u8>,
    }

    fn test_key() -> TestKey {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random)
            .expect("the test key generates");
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
            .expect("the generated key parses");
        TestKey {
            encoding: EncodingKey::from_ed_der(pkcs8.as_ref()),
            public: ring::signature::KeyPair::public_key(&pair)
                .as_ref()
                .to_vec(),
        }
    }

    fn device() -> DeviceId {
        DeviceId::from_raw("11112222333344445555666677778888")
    }

    fn claims(issued_at: i64, marketplaces: Vec<Marketplace>) -> Claims {
        Claims {
            sub: "org-1".to_owned(),
            aud: AUDIENCE.to_owned(),
            iss: ISSUER.to_owned(),
            device: device().as_str().to_owned(),
            marketplaces,
            exp: issued_at + VALIDITY,
            grace: issued_at + VALIDITY + GRACE,
        }
    }

    fn mint(key: &TestKey, claims: &Claims) -> String {
        encode(
            &Header::new(jsonwebtoken::Algorithm::EdDSA),
            claims,
            &key.encoding,
        )
        .expect("the test token signs")
    }

    fn gate(key: &TestKey, claims: &Claims) -> EntitlementGate {
        let token = mint(key, claims);
        EntitlementGate::holding(
            Entitlement::verify(&token, &key.public, &device()).expect("the token verifies"),
        )
    }

    #[test]
    fn a_current_token_permits_the_marketplaces_it_names() {
        let key = test_key();
        let gate = gate(&key, &claims(NOW, vec![Marketplace::Tpt, Marketplace::Tes]));
        assert!(gate.may_work(Marketplace::Tpt, at(NOW)));
        assert!(gate.may_work(Marketplace::Tes, at(NOW)));
        assert!(
            !gate
                .entitlement()
                .expect("the gate holds a token")
                .needs_revalidation(at(NOW)),
            "a token inside its validity window is not due for a check-in"
        );
    }

    #[test]
    fn a_token_expired_inside_its_grace_still_permits_work() {
        let key = test_key();
        let gate = gate(&key, &claims(NOW, vec![Marketplace::Tpt]));
        let inside_grace = at(NOW + VALIDITY + GRACE - 1);
        assert!(
            gate.may_work(Marketplace::Tpt, inside_grace),
            "the grace window is what keeps a seller working while our server is unreachable"
        );
        assert!(
            gate.entitlement()
                .expect("the gate holds a token")
                .needs_revalidation(inside_grace),
            "and it is exactly the window in which the device should be trying to check in"
        );
    }

    #[test]
    fn a_token_past_its_grace_permits_nothing() {
        let key = test_key();
        let gate = gate(&key, &claims(NOW, vec![Marketplace::Tpt]));
        assert!(
            !gate.may_work(Marketplace::Tpt, at(NOW + VALIDITY + GRACE + 1)),
            "past the grace deadline the gate fails closed, which is the kill-switch latency \
             the founder commits to publicly"
        );
    }

    #[test]
    fn a_token_signed_by_the_wrong_key_does_not_verify() {
        let (signer, verifier) = (test_key(), test_key());
        let token = mint(&signer, &claims(NOW, vec![Marketplace::Tpt]));
        let outcome = Entitlement::verify(&token, &verifier.public, &device());
        assert!(
            matches!(outcome, Err(EntitlementError::Rejected(_))),
            "a token this build did not have the key for must be rejected, not read: {outcome:?}"
        );
    }

    #[test]
    fn a_marketplace_absent_from_the_token_is_revoked() {
        let key = test_key();
        let gate = gate(&key, &claims(NOW, vec![Marketplace::Tes]));
        assert!(gate.may_work(Marketplace::Tes, at(NOW)));
        assert!(
            !gate.may_work(Marketplace::Tpt, at(NOW)),
            "dropping one marketplace from the grant set is how a cease-and-desist is answered \
             across the installed fleet without shipping an update"
        );
    }

    #[test]
    fn no_token_at_all_permits_nothing() {
        let gate = EntitlementGate::closed();
        for marketplace in Marketplace::ALL {
            assert!(
                !gate.may_work(marketplace, at(NOW)),
                "the gate must fail closed with no token, which is also the state a build \
                 carrying the placeholder key is permanently in"
            );
        }
    }

    #[test]
    fn a_token_minted_for_another_device_is_refused() {
        let key = test_key();
        let token = mint(&key, &claims(NOW, vec![Marketplace::Tpt]));
        assert_eq!(
            Entitlement::verify(&token, &key.public, &DeviceId::from_raw("ffff")),
            Err(EntitlementError::WrongDevice),
            "a token copied to a second machine must not work there"
        );
    }

    #[test]
    fn a_grace_deadline_before_the_expiry_is_refused() {
        let key = test_key();
        let mut malformed = claims(NOW, vec![Marketplace::Tpt]);
        malformed.grace = malformed.exp - 1;
        let token = mint(&key, &malformed);
        assert_eq!(
            Entitlement::verify(&token, &key.public, &device()),
            Err(EntitlementError::GraceBeforeExpiry),
            "a negative grace period is a malformed token, not a very short one"
        );
    }

    #[test]
    fn a_token_for_another_audience_does_not_verify() {
        let key = test_key();
        let mut foreign = claims(NOW, vec![Marketplace::Tpt]);
        foreign.aud = "some-other-service".to_owned();
        let token = mint(&key, &foreign);
        assert!(
            matches!(
                Entitlement::verify(&token, &key.public, &device()),
                Err(EntitlementError::Rejected(_))
            ),
            "an audience check is what stops a token minted elsewhere being replayed here"
        );
    }

    #[test]
    fn the_placeholder_key_verifies_nothing() {
        let key = test_key();
        let token = mint(&key, &claims(NOW, vec![Marketplace::Tpt]));
        assert!(
            Entitlement::verify(&token, &super::EMBEDDED_PUBLIC_KEY, &device()).is_err(),
            "a build shipped before the founder supplies a key must refuse every token rather \
             than accept any"
        );
    }
}
