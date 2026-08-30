//! The identity bridge: the login assertion the dashboard fetches from the
//! identity service, verified here against that service's published key set,
//! so the exchange mints our own session without ever calling Node.
//!
//! The assertion is asked for exactly one fact — which human is signing in —
//! and is consulted for nothing else. No claim names an organisation, so a
//! forged or stale token cannot widen a tenant boundary: Postgres draws that
//! boundary from `app_user`, after this module has finished.
//!
//! Verification is once per login rather than once per request, because the
//! session cookie carries every request after it. The key set is therefore
//! cached rather than fetched per call, and the cooldown below is what keeps
//! an unknown-`kid` flood from turning this endpoint into an amplifier
//! pointed at the identity service.

use core::future::Future;
use core::pin::Pin;

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use tam_types::{Timestamp, Uuid};

/// Re-exported so a binary can implement [`JwksSource`] without taking its own
/// dependency on the JWT crate; the key set's wire shape is this API's
/// contract with whatever fetches it.
pub use jsonwebtoken::jwk::JwkSet;

/// The audience every accepted assertion must name. Fixed by the contract
/// with the identity service rather than configured: a deployment able to
/// widen this would accept a token minted for a different service.
pub const AUDIENCE: &str = "tam-api";

/// The signature algorithm the contract fixes, checked before a key is even
/// looked up. Naming exactly one algorithm is what closes the substitution
/// attack in which a token declares a family the verifier will honour with a
/// key intended for another.
const ALGORITHM: Algorithm = Algorithm::EdDSA;

/// How long after one key-set fetch the next may run.
///
/// An unknown `kid` is the only thing that triggers a refetch, and anyone can
/// mint unlimited unknown ones for free, so this bound is the difference
/// between rotation support and a reflected denial-of-service against the
/// identity service. It is deliberately a constant next to its caller rather
/// than a `tam-limits` entry: it bounds this process's own outbound rate, not
/// a tenant's share of anything.
pub const JWKS_REFETCH_COOLDOWN_MS: i64 = 60_000;

/// The key set could not be read from the identity service. Carries its
/// diagnostic for the fetching implementation to log; the exchange itself
/// answers the same 401 it answers every other failed assertion, so nothing
/// here reaches a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwksUnavailable(pub String);

impl core::fmt::Display for JwksUnavailable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "the identity service's key set is unavailable: {}",
            self.0
        )
    }
}

/// The future a fetch returns. Spelled out rather than reached for through a
/// macro crate: one boxed future is the whole cost of keeping the trait
/// object-safe.
pub type JwksFuture<'a> =
    Pin<Box<dyn Future<Output = Result<JwkSet, JwksUnavailable>> + Send + 'a>>;

/// Where the key set comes from, as a value the binary supplies. This library
/// holds no HTTP client for the same reason it holds no clock: both are
/// process-boundary concerns, and injecting them is what lets a test drive
/// the whole exchange with no listener bound.
pub trait JwksSource: Send + Sync {
    fn fetch(&self) -> JwksFuture<'_>;
}

/// What a verified assertion says, and the whole of it.
///
/// `email_verified` is absent by construction: an unverified address is
/// refused during verification, so no caller can forget to check it. The
/// organisation is absent for the reason given in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedSubject {
    pub subject: Uuid,
}

/// The claim set the contract fixes. `iss` and `aud` are verified by the
/// decoder and so are not named here; `exp` is, because it is re-checked
/// against the injected clock rather than the machine's.
#[derive(serde::Deserialize)]
struct Assertion {
    sub: String,
    exp: i64,
    email_verified: bool,
}

#[derive(Default)]
struct KeyCache {
    keys: Option<JwkSet>,
    last_fetch_at: Option<Timestamp>,
}

impl KeyCache {
    fn may_refetch(&self, now: Timestamp) -> bool {
        self.last_fetch_at
            .is_none_or(|last| now.0.saturating_sub(last.0) >= JWKS_REFETCH_COOLDOWN_MS)
    }
}

/// The identity service as this API sees it: the issuer its assertions must
/// name, and the key set they are signed under.
pub struct AuthBridge {
    issuer: String,
    source: Box<dyn JwksSource>,
    cache: tokio::sync::Mutex<KeyCache>,
}

impl core::fmt::Debug for AuthBridge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthBridge")
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

impl AuthBridge {
    #[must_use]
    pub fn new(issuer: String, source: Box<dyn JwksSource>) -> Self {
        Self {
            issuer,
            source,
            cache: tokio::sync::Mutex::new(KeyCache::default()),
        }
    }

    /// The assertion to the subject it proves, or `None`.
    ///
    /// Every failure is the same `None`: a wrong issuer, a wrong audience, a
    /// bad signature, an expired token and an unverified address are not
    /// distinguishable to the caller, because a caller that could tell them
    /// apart would be an oracle for probing the identity service.
    pub async fn verify(&self, token: &str, now: Timestamp) -> Option<VerifiedSubject> {
        let header = decode_header(token).ok()?;
        if header.alg != ALGORITHM {
            return None;
        }
        let key = self.key_for(&header.kid?, now).await?;

        let mut validation = Validation::new(ALGORITHM);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        // Presence of `exp` is still required above; only the comparison moves.
        // It is made against the clock the binary injected, so one process has
        // one notion of now and a test can fix it, and with no leeway, which is
        // stricter than the decoder's own minute of tolerance.
        validation.validate_exp = false;
        validation.validate_nbf = false;

        let assertion = decode::<Assertion>(token, &key, &validation).ok()?.claims;
        if !assertion.email_verified {
            return None;
        }
        if assertion.exp.checked_mul(1_000)? <= now.0 {
            return None;
        }
        Some(VerifiedSubject {
            subject: Uuid(*uuid::Uuid::parse_str(&assertion.sub).ok()?.as_bytes()),
        })
    }

    /// The signing key for a `kid`, refetching the key set at most once per
    /// cooldown when the cache does not hold it.
    ///
    /// The lock is deliberately held across the fetch: it collapses a burst of
    /// requests naming the same freshly-rotated key into one outbound call
    /// rather than one per request.
    async fn key_for(&self, kid: &str, now: Timestamp) -> Option<DecodingKey> {
        let mut cache = self.cache.lock().await;
        let cached = cache
            .keys
            .as_ref()
            .and_then(|set| set.find(kid))
            .and_then(|jwk| DecodingKey::from_jwk(jwk).ok());
        if cached.is_some() {
            return cached;
        }
        if !cache.may_refetch(now) {
            return None;
        }
        // Stamped before the await and regardless of outcome, so a failing
        // identity service is not retried per request either.
        cache.last_fetch_at = Some(now);
        let fetched = self.source.fetch().await.ok()?;
        let key = fetched
            .find(kid)
            .and_then(|jwk| DecodingKey::from_jwk(jwk).ok());
        cache.keys = Some(fetched);
        key
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyCache, JWKS_REFETCH_COOLDOWN_MS};
    use tam_types::Timestamp;

    #[test]
    fn a_cache_that_has_never_fetched_may_fetch() {
        assert!(
            KeyCache::default().may_refetch(Timestamp(0)),
            "the first unknown kid must be able to load the key set"
        );
    }

    #[test]
    fn the_cooldown_bounds_the_refetch_rate() {
        let cache = KeyCache {
            keys: None,
            last_fetch_at: Some(Timestamp(1_000)),
        };
        assert!(
            !cache.may_refetch(Timestamp(1_000 + JWKS_REFETCH_COOLDOWN_MS - 1)),
            "a second unknown kid inside the cooldown does not reach the identity service"
        );
        assert!(
            cache.may_refetch(Timestamp(1_000 + JWKS_REFETCH_COOLDOWN_MS)),
            "the cooldown expires rather than latching, so key rotation still lands"
        );
    }

    #[test]
    fn a_backwards_clock_does_not_open_the_gate() {
        let cache = KeyCache {
            keys: None,
            last_fetch_at: Some(Timestamp(10_000)),
        };
        assert!(
            !cache.may_refetch(Timestamp(0)),
            "a clock that steps backwards must not read as a cooldown that has elapsed"
        );
    }
}
