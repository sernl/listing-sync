//! Who the session belongs to, as Tes itself asserts it.
//!
//! This is the identity read the exclusivity claim consumes: the broker seals
//! a credential and the connection is usable immediately, the caller reads the
//! account back through a lease on that very connection, and the claim writes
//! the digest that takes the global exclusivity lock.
//!
//! Only a server-asserted identity is admissible, which is why this parses a
//! response rather than accepting a seller-typed value. An account identifier
//! a seller could type would be a denial-of-service primitive: anybody could
//! claim a storefront they do not own, and its real owner would then be unable
//! to link at all.
//!
//! Which response is read is the whole of the safety argument, and here it is
//! structural rather than reasoned. `GET /api/tier/gmv/me` names its principal
//! in the path and takes no selector, no query and no body, so
//! [`crate::endpoints::seller_tier_request`] has no parameter through which a
//! caller could name somebody else's account. Tpt's identity read had to
//! choose between two operations carrying the same author field, only one of
//! which the server scopes to the session; this route offers no such choice.

use serde_json::Value;
use tam_marketplace::transport::{HttpResponse, Transport, TransportError};

/// A Tes seller, by the id the platform puts on that seller's tier record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellerId(pub String);

/// The one field read off the tier record.
const USER_ID_FIELD: &str = "userId";

/// The seller id in a `GET /api/tier/gmv/me` response.
///
/// Read at the top level of the object rather than searched for, which is the
/// opposite of the Tpt parser and for a reason the two shapes decide: this
/// body is one record about one principal, so a `userId` appearing anywhere
/// deeper in it is not the session's own and a recursive search would be a way
/// to find one and lock the wrong account on it.
///
/// The capture carries the field as a JSON integer and only a non-negative
/// integer is accepted. A string, a float or a negative is not the observed
/// shape, and rendering one into a decimal would let one account digest two
/// ways and hold the exclusivity lock twice over.
///
/// `None` on every other shape, a challenge page included, which leaves the
/// connection unclaimed rather than locked to a guess.
#[must_use]
pub fn seller_user_id(body: &[u8]) -> Option<SellerId> {
    let parsed: Value = serde_json::from_slice(body).ok()?;
    let id = parsed.as_object()?.get(USER_ID_FIELD)?.as_u64()?;
    Some(SellerId(id.to_string()))
}

/// Reads the seller's own account id back through a transport.
///
/// The one identity read the claim path may use. The status is checked rather
/// than left to the parser: Tes answers this route with JSON either way, so
/// unlike Tpt's author search the shape alone does not separate a refusal from
/// an answer, and the lock may only be taken on a read that was live.
///
/// `Ok(None)` is an answer that named nobody — a refusal, an interstitial, or
/// a shape that changed — which costs the lock and not the connection: the
/// claim is opportunistic and a later read may still name the account.
pub async fn read_seller_user_id<T: Transport>(
    transport: &T,
) -> Result<Option<SellerId>, TransportError> {
    let response: HttpResponse = transport
        .send(crate::endpoints::seller_tier_request())
        .await?;
    if !(200..300).contains(&response.status) {
        return Ok(None);
    }
    Ok(seller_user_id(&response.body))
}

/// Renews the seller's session and says whether Tes accepted it.
///
/// Beside the identity read rather than in a module of its own, because the
/// two are one question asked twice: this one asks Tes to rotate the cookies
/// it is about to check, and [`read_seller_user_id`] asks whether what came
/// back still authenticates. Neither names an account, so neither can be
/// pointed at another seller's session.
///
/// A non-2xx is `false` rather than an error: the renewal is the marketplace's
/// answer about the session it holds, and the caller's remedy for a refused
/// one is a fresh sign-in rather than a retry. A transport failure stays an
/// error, because it says nothing about the session at all.
pub async fn refresh_session<T: Transport>(transport: &T) -> Result<bool, TransportError> {
    let response: HttpResponse = transport
        .send(crate::endpoints::refresh_cookies_request())
        .await?;
    Ok((200..300).contains(&response.status))
}

#[cfg(test)]
mod tests {
    use super::{seller_user_id, SellerId};
    use serde_json::Value;

    /// The account id in the committed cassette. Sanitised, as the Tpt
    /// cassettes are: the captured HAR is probe-local and gitignored, and the
    /// real seller's id does not enter version control to make a fixture.
    const CASSETTE_ACCOUNT: &str = "28000001";

    /// Pulls one interaction's response body out of a committed cassette, so
    /// the extraction is proved against the shape Tes actually sent rather
    /// than against a hand-written sample of it.
    fn cassette_body(cassette: &str, index: usize) -> Vec<u8> {
        let parsed: Value = serde_json::from_str(cassette).expect("the cassette parses");
        parsed["interactions"][index]["response"]["body"]
            .as_str()
            .expect("the recorded body is a string")
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn the_account_id_is_read_from_the_tier_cassette_the_claim_path_uses() {
        let body = cassette_body(include_str!("../tests/cassettes/seller_tier.json"), 0);
        assert_eq!(
            seller_user_id(&body),
            Some(SellerId(CASSETTE_ACCOUNT.to_owned())),
            "the tier record names its own principal at the top level, and that is the value \
             the exclusivity lock is taken on"
        );
    }

    #[test]
    fn a_numeric_identifier_becomes_its_decimal_spelling() {
        assert_eq!(
            seller_user_id(br#"{"userId":28768710}"#),
            Some(SellerId("28768710".to_owned())),
            "the field is JSON's number type, and the lock is equality on a hash of a string, \
             so one account must render to exactly one spelling"
        );
    }

    #[test]
    fn a_nested_identifier_is_not_this_session_and_names_nobody() {
        assert_eq!(
            seller_user_id(br#"{"tier":{"userId":99},"isOverridden":false}"#),
            None,
            "only the record's own top-level principal is the session's; claiming on an id \
             found somewhere deeper would take the global lock on an account this session may \
             not hold"
        );
    }

    #[test]
    fn an_identifier_that_is_not_a_whole_number_names_nobody() {
        for body in [
            br#"{"userId":"28768710"}"#.as_slice(),
            br#"{"userId":2.5}"#.as_slice(),
            br#"{"userId":-1}"#.as_slice(),
            br#"{"userId":null}"#.as_slice(),
        ] {
            assert_eq!(
                seller_user_id(body),
                None,
                "none of these is the captured shape, and coercing one would let a single \
                 account digest under two spellings: {}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn a_body_with_no_identifier_names_nobody() {
        assert_eq!(
            seller_user_id(br#"{"tier":{"name":"Bronze"}}"#),
            None,
            "a tier record without a principal names no account, which leaves the connection \
             unclaimed rather than locked to a guess"
        );
    }

    #[test]
    fn a_body_that_is_not_json_names_nobody() {
        assert_eq!(
            seller_user_id(b"<html>challenge</html>"),
            None,
            "an interstitial must not be mistaken for an identity"
        );
    }

    /// A transport that answers one canned response and records what it was
    /// asked for, so the identity read's own request shape is under test
    /// rather than assumed. `OnceLock` because exactly one request is expected
    /// and it is the only shared-mutable state the trait's `Sync` bound admits
    /// without a lock.
    struct RecordingTransport {
        seen: std::sync::OnceLock<tam_marketplace::transport::HttpRequest>,
        status: u16,
        body: Vec<u8>,
    }

    impl tam_marketplace::transport::Transport for RecordingTransport {
        async fn send(
            &self,
            request: tam_marketplace::transport::HttpRequest,
        ) -> Result<
            tam_marketplace::transport::HttpResponse,
            tam_marketplace::transport::TransportError,
        > {
            drop(self.seen.set(request));
            Ok(tam_marketplace::transport::HttpResponse::plain(
                self.status,
                self.body.clone(),
            ))
        }
    }

    #[test]
    fn the_identity_read_asks_the_server_who_the_session_is() {
        let transport = RecordingTransport {
            seen: std::sync::OnceLock::new(),
            status: 200,
            body: cassette_body(include_str!("../tests/cassettes/seller_tier.json"), 0),
        };
        let found = futures::executor::block_on(super::read_seller_user_id(&transport))
            .expect("the read succeeds");
        assert_eq!(
            found,
            Some(SellerId(CASSETTE_ACCOUNT.to_owned())),
            "the identity comes back from the response"
        );

        let request = transport.seen.get().expect("exactly one request was made");
        assert_eq!(
            *request,
            crate::endpoints::seller_tier_request(),
            "the claim path may issue the session's own tier read and nothing else"
        );
        assert_eq!(
            request.url, "https://www.tes.com/api/tier/gmv/me",
            "the route carries no selector and no query, so the account it names is the \
             server's choice rather than the caller's: {}",
            request.url
        );
    }

    /// The renewal's whole job is to be sent, so what is asserted is the
    /// route it sends and the fact that a refusal is a verdict rather than an
    /// error: a caller that treated a bad day on this route as a failure
    /// would stop probing the session it was about to check.
    #[test]
    fn the_renewal_asks_the_cookie_route_and_reports_a_refusal_as_a_verdict() {
        let transport = RecordingTransport {
            seen: std::sync::OnceLock::new(),
            status: 200,
            body: Vec::new(),
        };
        assert_eq!(
            futures::executor::block_on(super::refresh_session(&transport)),
            Ok(true)
        );
        assert_eq!(
            *transport.seen.get().expect("exactly one request was made"),
            crate::endpoints::refresh_cookies_request(),
            "the renewal names no account and takes nothing, so it cannot renew somebody \
             else's session"
        );

        let refused = RecordingTransport {
            seen: std::sync::OnceLock::new(),
            status: 401,
            body: Vec::new(),
        };
        assert_eq!(
            futures::executor::block_on(super::refresh_session(&refused)),
            Ok(false),
            "a refused renewal is an answer about the session, not a transport fault"
        );
    }

    #[test]
    fn a_refusal_carrying_an_identifier_still_names_nobody() {
        let transport = RecordingTransport {
            seen: std::sync::OnceLock::new(),
            status: 403,
            body: br#"{"userId":28768710}"#.to_vec(),
        };
        let found = futures::executor::block_on(super::read_seller_user_id(&transport))
            .expect("the transport answered");
        assert_eq!(
            found, None,
            "the lock is taken on a live read; a route that answers JSON either way must be \
             separated by its status, because the body shape does not separate it"
        );
    }
}
