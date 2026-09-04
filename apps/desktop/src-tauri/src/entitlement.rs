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
//! The claim set itself is not defined here. It lives in
//! `tam_domain::entitlement`, which the server mints from and this module
//! verifies into, so the wire contract has one definition and a field added on
//! one side is a compile error on the other. What stays here is everything
//! that is the device's own: the key it verifies against, and the gate.
//!
//! Validity and grace are D11: one hour of validity and a twenty-four hour
//! grace, failing closed. Those numbers are the server's to set — they arrive
//! in the token — and the kill-switch latency the founder commits to publicly
//! is their sum.

use std::collections::HashSet;

use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use tam_types::{Marketplace, Timestamp};

use crate::device::DeviceId;

/// The audience and issuer every accepted token must name, and the claim set
/// itself, re-exported from the crate that defines them so this module's own
/// surface is unchanged by where they live.
pub use tam_domain::entitlement::{Claims, AUDIENCE, ISSUER};

/// Ed25519 public keys are thirty-two bytes.
pub const PUBLIC_KEY_BYTES: usize = 32;

/// The Ed25519 public keys this binary verifies against, as their raw bytes.
///
/// Written by `build.rs` from the `TAM_ENTITLEMENT_PUBLIC_KEY` environment
/// variable: one key as sixty-four lowercase hex characters, or two separated by
/// a comma. The release workflow supplies it from a repository variable and
/// `just desktop-dev` from the development key pair.
///
/// Empty is what a build with no such variable gets, and empty is a state rather
/// than a value: [`Entitlement::verify`] refuses everything against an empty set,
/// so such a build verifies no token and every gate answers no. That distinction
/// is load-bearing. This constant was once thirty-two zero bytes, described in
/// four places as "not a valid Ed25519 point, so it verifies nothing"; those
/// bytes are in fact a valid point of order four, and a signature can be forged
/// against them with no private key at all — `S = 0` with `R` the identity
/// encoding satisfies the cofactorless equation whenever the challenge is
/// divisible by four, which is about one payload in four. The test below builds
/// exactly that forgery.
///
/// Two keys exist for rotation. A release carrying the outgoing and the incoming
/// key verifies tokens signed by either, which is what makes the ordering in
/// `docs/notes/design/desktop-client.md` safe rather than merely conventional.
///
/// A release that forgot the variable is refused before it is built, by the
/// desktop-release workflow's own guard.
pub const EMBEDDED_PUBLIC_KEYS: &[[u8; PUBLIC_KEY_BYTES]] =
    include!(concat!(env!("OUT_DIR"), "/entitlement_key.rs"));

/// The all-zero key, refused wherever it appears.
///
/// Refused by name rather than by a small-order test: this is the value an unset
/// variable, a shell that expanded one into zeroes, or the old placeholder
/// produces, and it is the one an accident actually reaches. The wider class is
/// a recorded residual rather than a covered case — the other seven small-order
/// encodings are equally forgeable and are not checked here — and the reason
/// that residual is acceptable is that no key reaches this constant except one a
/// human deliberately set: `build.rs` refuses all-zero at the build boundary and
/// emits an empty set for the absent case, so this check is the second of two.
const SMALL_ORDER_KEY: [u8; PUBLIC_KEY_BYTES] = [0u8; PUBLIC_KEY_BYTES];

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
    /// Verifies a token against any of `public_keys` and binds it to `device`.
    ///
    /// An empty set refuses everything, which is what a build given no key is:
    /// there is nothing to check a signature against, and a client that cannot
    /// check one must not be told it may work.
    ///
    /// More than one key is a rotation in flight. A key is tried, and a
    /// signature it does not verify moves on to the next; the first key whose
    /// signature verifies decides the answer, so a token that verifies but names
    /// another device is refused as [`EntitlementError::WrongDevice`] rather
    /// than being retried under a key that would only reject it and lose the
    /// reason.
    ///
    /// Expiry is decided by [`EntitlementGate::may_work`] rather than by the JWT
    /// library, which is why `validate_exp` is off: a token past `exp` but inside
    /// `grace` is still workable under D11, and the library would refuse it
    /// outright and collapse the grace period to nothing.
    pub fn verify(
        token: &str,
        public_keys: &[[u8; PUBLIC_KEY_BYTES]],
        device: &DeviceId,
    ) -> Result<Self, EntitlementError> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[AUDIENCE]);
        validation.set_issuer(&[ISSUER]);
        validation.required_spec_claims =
            HashSet::from(["exp".to_owned(), "aud".to_owned(), "iss".to_owned()]);
        validation.validate_exp = false;

        let mut refusal = EntitlementError::Rejected(
            "this build carries no entitlement key, so no token can verify".to_owned(),
        );
        for key in public_keys {
            if key == &SMALL_ORDER_KEY {
                refusal = EntitlementError::Rejected(
                    "an all-zero entitlement key is a small-order point, against which a \
                     signature can be forged without a private key"
                        .to_owned(),
                );
                continue;
            }
            match decode::<Claims>(token, &DecodingKey::from_ed_der(key), &validation) {
                Ok(decoded) => return Self::bind(decoded.claims, device),
                Err(why) => refusal = EntitlementError::Rejected(why.to_string()),
            }
        }
        Err(refusal)
    }

    /// What a verified signature still has to satisfy before it is an
    /// entitlement: the right machine, and two deadlines the right way round.
    fn bind(claims: Claims, device: &DeviceId) -> Result<Self, EntitlementError> {
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

/// Key material and claim fixtures for this crate's tests.
///
/// Shared rather than duplicated per module: `heartbeat` exercises the same
/// gate through a check-in, and two generators would eventually mint subtly
/// different tokens and hide the difference in whichever module was not
/// looked at.
#[cfg(test)]
pub(crate) mod testing {
    use super::Claims;
    use crate::device::DeviceId;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use tam_types::{Marketplace, Timestamp};

    /// Seconds, because these are JWT claim values.
    pub(crate) const NOW: i64 = 1_756_000_000;
    /// D11's numbers: one hour of validity, twenty-four hours of grace. Named
    /// here as literals rather than read from `tam_domain`, so a test that
    /// asserts on them is a check of the constants rather than a restatement.
    pub(crate) const VALIDITY: i64 = 3_600;
    pub(crate) const GRACE: i64 = 86_400;

    /// The same instant as a `Timestamp`, which counts milliseconds. The whole
    /// reason this helper exists rather than being inlined: the two units are
    /// one careless comparison apart, and `tam-api` already had to convert.
    pub(crate) const fn at(seconds: i64) -> Timestamp {
        Timestamp(seconds * 1_000)
    }

    pub(crate) struct TestKey {
        pub(crate) encoding: EncodingKey,
        pub(crate) public: [u8; super::PUBLIC_KEY_BYTES],
    }

    impl TestKey {
        /// This key alone, as the one-element set a verifier takes.
        pub(crate) fn only(&self) -> [[u8; super::PUBLIC_KEY_BYTES]; 1] {
            [self.public]
        }
    }

    pub(crate) fn test_key() -> TestKey {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random)
            .expect("the test key generates");
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
            .expect("the generated key parses");
        TestKey {
            encoding: EncodingKey::from_ed_der(pkcs8.as_ref()),
            public: ring::signature::KeyPair::public_key(&pair)
                .as_ref()
                .try_into()
                .expect("an Ed25519 public key is thirty-two bytes"),
        }
    }

    /// Base64url without padding, which is what a JWS segment is.
    ///
    /// Hand-rolled because this crate has no base64 dependency and the only
    /// thing that needs one is the forged signature the placeholder test
    /// builds; a manifest edge for one test would be the wrong trade.
    pub(crate) fn b64url(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let trio = [
                usize::from(chunk[0]),
                usize::from(chunk.get(1).copied().unwrap_or(0)),
                usize::from(chunk.get(2).copied().unwrap_or(0)),
            ];
            let packed = (trio[0] << 16) | (trio[1] << 8) | trio[2];
            let quantum = [
                (packed >> 18) & 63,
                (packed >> 12) & 63,
                (packed >> 6) & 63,
                packed & 63,
            ];
            for symbol in quantum.iter().take(chunk.len() + 1) {
                out.push(char::from(ALPHABET[*symbol]));
            }
        }
        out
    }

    pub(crate) fn device() -> DeviceId {
        DeviceId::from_raw("11112222333344445555666677778888")
    }

    /// A well-formed claim set, built through the shared mint so a test can
    /// never assert against arithmetic the server does not perform.
    pub(crate) fn claims(issued_at: i64, marketplaces: Vec<Marketplace>) -> Claims {
        Claims::mint(
            "org-1".to_owned(),
            device().as_str().to_owned(),
            marketplaces,
            issued_at,
        )
    }

    pub(crate) fn mint(key: &TestKey, claims: &Claims) -> String {
        encode(
            &Header::new(jsonwebtoken::Algorithm::EdDSA),
            claims,
            &key.encoding,
        )
        .expect("the test token signs")
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{
        at, b64url, claims, device, mint, test_key, TestKey, GRACE, NOW, VALIDITY,
    };
    use super::{Entitlement, EntitlementError, EntitlementGate};
    use crate::device::DeviceId;
    use tam_types::Marketplace;

    fn gate(key: &TestKey, claims: &super::Claims) -> EntitlementGate {
        let token = mint(key, claims);
        EntitlementGate::holding(
            Entitlement::verify(&token, &key.only(), &device()).expect("the token verifies"),
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
        let outcome = Entitlement::verify(&token, &verifier.only(), &device());
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
            Entitlement::verify(&token, &key.only(), &DeviceId::from_raw("ffff")),
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
            Entitlement::verify(&token, &key.only(), &device()),
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
                Entitlement::verify(&token, &key.only(), &device()),
                Err(EntitlementError::Rejected(_))
            ),
            "an audience check is what stops a token minted elsewhere being replayed here"
        );
    }

    #[test]
    fn a_build_given_no_key_refuses_an_honestly_signed_token() {
        let key = test_key();
        let token = mint(&key, &claims(NOW, vec![Marketplace::Tpt]));
        assert!(
            Entitlement::verify(&token, &[], &device()).is_err(),
            "a build shipped before the founder supplies a key has nothing to check a \
             signature against, and must refuse every token rather than accept any"
        );
    }

    /// The forgery the all-zero key admits, built rather than described.
    ///
    /// All-zero decodes to a valid Ed25519 point of order four. Cofactorless
    /// verification checks `[S]B = R + [k]A`; with `S = 0` and `R` the identity
    /// encoding that reduces to `[k]A = identity`, which holds whenever the
    /// challenge `k = H(R‖A‖M)` is divisible by four — about one payload in
    /// four. No private key is involved at any point.
    ///
    /// The test proves the forgery is real before asserting we refuse it: if
    /// `ring` ever stopped accepting it the first assertion would fail, and a
    /// rejection test that could not tell "we refuse this" from "nothing could
    /// have accepted it" is the vacuous kind this replaces.
    #[test]
    fn a_signature_forged_against_the_all_zero_key_is_refused() {
        let scratch = test_key();
        let mut forged = None;
        for attempt in 0..64 {
            let mut payload = claims(NOW, vec![Marketplace::Tpt, Marketplace::Tes]);
            payload.sub = format!("forger-{attempt}");
            let honest = mint(&scratch, &payload);
            let signing_input = honest
                .rsplit_once('.')
                .map(|(head, _signature)| head.to_owned())
                .expect("a JWS has three segments");
            // R is the identity point's encoding, S is zero.
            let mut signature = [0u8; 64];
            signature[0] = 1;
            if ring::signature::UnparsedPublicKey::new(
                &ring::signature::ED25519,
                super::SMALL_ORDER_KEY,
            )
            .verify(signing_input.as_bytes(), &signature)
            .is_ok()
            {
                forged = Some(format!("{signing_input}.{}", b64url(&signature)));
                break;
            }
        }
        let token = forged.expect(
            "one payload in four should satisfy the forgery; sixty-four attempts finding none \
             would mean the construction no longer works and this test needs rewriting",
        );

        assert!(
            Entitlement::verify(&token, &[super::SMALL_ORDER_KEY], &device()).is_err(),
            "a token forged against the all-zero key must be refused; the signature itself \
             verifies, which is exactly why the key has to be refused rather than trusted"
        );
        assert!(
            Entitlement::verify(&token, &[], &device()).is_err(),
            "and a build carrying no key at all refuses it too"
        );
    }

    #[test]
    fn a_rotation_set_verifies_a_token_signed_by_either_key() {
        let (outgoing, incoming) = (test_key(), test_key());
        let set = [outgoing.public, incoming.public];
        for signer in [&outgoing, &incoming] {
            let token = mint(signer, &claims(NOW, vec![Marketplace::Tpt]));
            assert!(
                Entitlement::verify(&token, &set, &device()).is_ok(),
                "a release carrying both keys is what lets the fleet cross a rotation without \
                 a window in which somebody's gate is closed"
            );
        }
        let stranger = test_key();
        let token = mint(&stranger, &claims(NOW, vec![Marketplace::Tpt]));
        assert!(
            Entitlement::verify(&token, &set, &device()).is_err(),
            "and a third key is still refused: two accepted keys is a rotation, not a relaxation"
        );
    }
}
