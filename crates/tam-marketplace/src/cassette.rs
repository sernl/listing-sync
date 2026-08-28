//! Recorded interactions replayed as the transport, so adapter flows are
//! tested end to end offline, in the gated lane and the nix sandbox alike.
//!
//! Replay is strict and in order: the flow under test must issue exactly the
//! recorded requests in the recorded sequence, and a test asserts
//! [`CassetteTransport::remaining`] is zero so an interaction the flow never
//! reached fails the test rather than passing silently.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::transport::{HttpRequest, HttpResponse, Transport, TransportError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    pub request: HttpRequest,
    pub response: HttpResponse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cassette {
    pub interactions: Vec<Interaction>,
}

/// The cursor is an atomic rather than a mutex because this crate takes no
/// async runtime and `std::sync::Mutex` is lint-banned workspace-wide;
/// append-only interactions plus a fetch-add cursor need neither.
pub struct CassetteTransport {
    interactions: Vec<Interaction>,
    cursor: AtomicUsize,
}

impl CassetteTransport {
    #[must_use]
    pub fn new(cassette: Cassette) -> Self {
        Self {
            interactions: cassette.interactions,
            cursor: AtomicUsize::new(0),
        }
    }

    /// Interactions not yet replayed. A finished test asserts this is zero.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.interactions
            .len()
            .saturating_sub(self.cursor.load(Ordering::SeqCst))
    }
}

impl Transport for CassetteTransport {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl core::future::Future<Output = Result<HttpResponse, TransportError>> + Send {
        let position = self.cursor.fetch_add(1, Ordering::SeqCst);
        let outcome = match self.interactions.get(position) {
            None => Err(TransportError::Harness {
                detail: format!(
                    "interaction {position} requested but the cassette holds {}",
                    self.interactions.len()
                ),
            }),
            Some(interaction) if interaction.request == request => Ok(interaction.response.clone()),
            Some(interaction) => Err(TransportError::Harness {
                detail: format!(
                    "interaction {position} diverged: recorded {} {:?}, flow sent {} {:?}",
                    interaction.request.url,
                    interaction.request.method,
                    request.url,
                    request.method
                ),
            }),
        };
        core::future::ready(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::{Cassette, CassetteTransport, Interaction};
    use crate::transport::{HttpRequest, HttpResponse, ResponseHeader, Transport, TransportError};

    fn get(url: &str) -> HttpRequest {
        HttpRequest::get(url.to_owned())
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse::plain(200, body.as_bytes().to_vec())
    }

    fn two_step() -> CassetteTransport {
        CassetteTransport::new(Cassette {
            interactions: vec![
                Interaction {
                    request: get("https://example.test/a"),
                    response: ok("a"),
                },
                Interaction {
                    request: get("https://example.test/b"),
                    response: ok("b"),
                },
            ],
        })
    }

    #[test]
    fn replays_in_order_and_counts_down() {
        let transport = two_step();
        assert_eq!(transport.remaining(), 2, "nothing replayed yet");
        let first = futures::executor::block_on(transport.send(get("https://example.test/a")))
            .expect("the first recorded interaction replays");
        assert_eq!(first.text(), "a", "the recorded response comes back");
        assert_eq!(transport.remaining(), 1, "one interaction consumed");
        let second = futures::executor::block_on(transport.send(get("https://example.test/b")))
            .expect("the second recorded interaction replays");
        assert_eq!(second.text(), "b", "order is preserved");
        assert_eq!(transport.remaining(), 0, "the cassette is exhausted");
    }

    #[test]
    fn a_diverging_request_fails_loudly() {
        let transport = two_step();
        let diverged =
            futures::executor::block_on(transport.send(get("https://example.test/wrong")));
        assert!(
            matches!(diverged, Err(TransportError::Harness { .. })),
            "a request the recording does not contain must fail, not fall through"
        );
    }

    #[test]
    fn an_exhausted_cassette_refuses_further_requests() {
        let transport = CassetteTransport::new(Cassette {
            interactions: vec![],
        });
        let refused = futures::executor::block_on(transport.send(get("https://example.test/a")));
        assert!(
            matches!(refused, Err(TransportError::Harness { .. })),
            "an unrecorded interaction must fail, not hang or improvise"
        );
    }

    #[test]
    fn the_cassette_format_round_trips_as_json() {
        let cassette = Cassette {
            interactions: vec![Interaction {
                request: HttpRequest::post_json(
                    "https://example.test/c".to_owned(),
                    serde_json::json!({"k": 1}),
                ),
                response: ok("{}"),
            }],
        };
        let encoded = serde_json::to_string(&cassette).expect("a cassette serialises");
        let back: Cassette = serde_json::from_str(&encoded).expect("a cassette deserialises");
        assert_eq!(back, cassette, "the fixture format survives a round trip");
    }

    #[test]
    fn a_session_request_records_without_an_auth_key() {
        let encoded =
            serde_json::to_string(&get("https://example.test/a")).expect("a request serialises");
        assert_eq!(
            encoded, r#"{"method":"Get","url":"https://example.test/a","body":"Empty"}"#,
            "the default auth is skipped, so every fixture written before it existed still round trips"
        );
    }

    #[test]
    fn a_header_less_response_records_without_a_headers_key() {
        let encoded = serde_json::to_string(&ok("a")).expect("a response serialises");
        assert_eq!(
            encoded, r#"{"status":200,"body":"a"}"#,
            "an empty header allow-list is skipped, which is what keeps the fixtures byte-identical"
        );
    }

    #[test]
    fn the_committed_fixture_shape_still_deserialises() {
        let fixture = r#"{"interactions":[{"request":{"method":"Delete","url":"https://www.tes.com/api/v2/resources/9001/draft","body":"Empty"},"response":{"status":204,"body":""}}]}"#;
        let cassette: Cassette =
            serde_json::from_str(fixture).expect("a pre-seam fixture still reads");
        let interaction = cassette
            .interactions
            .first()
            .expect("the fixture holds one interaction");
        assert!(
            interaction.request.auth.is_session(),
            "an absent auth key means the session, which is what every recorded request was"
        );
        assert!(
            interaction.response.headers.is_empty(),
            "an absent headers key means no allow-listed header was read"
        );
    }

    #[test]
    fn a_recorded_header_round_trips_through_the_allow_list() {
        let response = HttpResponse {
            headers: vec![(
                ResponseHeader::Location,
                "https://example.test/landed".to_owned(),
            )],
            ..ok("")
        };
        let encoded = serde_json::to_string(&response).expect("a response serialises");
        let back: HttpResponse = serde_json::from_str(&encoded).expect("a response deserialises");
        assert_eq!(
            back.header(ResponseHeader::Location),
            Some("https://example.test/landed"),
            "the allow-listed header survives the fixture round trip"
        );
        assert_eq!(
            back.header(ResponseHeader::ETag),
            None,
            "a header the recording does not carry reads as absent, never as empty"
        );
    }

    #[test]
    fn a_utf8_body_still_records_as_a_json_string() {
        let response = ok(r#"{"id":42}"#);
        let encoded = serde_json::to_string(&response).expect("a response serialises");
        assert_eq!(
            encoded, r#"{"status":200,"body":"{\"id\":42}"}"#,
            "a text body keeps the representation every committed fixture was written in"
        );
    }

    #[test]
    fn a_binary_body_survives_the_fixture_round_trip_byte_for_byte() {
        let zip: Vec<u8> = b"PK\x03\x04\x14\x00\x08\x00\x08\x00"
            .iter()
            .copied()
            .chain((0u8..=255).cycle().take(512))
            .collect();
        let response = HttpResponse::plain(200, zip.clone());
        let encoded = serde_json::to_string(&response).expect("a binary response serialises");
        let back: HttpResponse =
            serde_json::from_str(&encoded).expect("a binary response deserialises");
        assert_eq!(
            back.body, zip,
            "a download bundle must round trip byte for byte; a lossy body would corrupt it"
        );
        assert_eq!(
            &back.body[..4],
            b"PK\x03\x04",
            "the zip magic survives, which is what the import depends on"
        );
    }
}
