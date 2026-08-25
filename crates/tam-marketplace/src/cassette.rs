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
    use crate::transport::{
        HttpRequest, HttpResponse, Method, RequestBody, Transport, TransportError,
    };

    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            url: url.to_owned(),
            body: RequestBody::Empty,
        }
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_owned(),
        }
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
        assert_eq!(first.body, "a", "the recorded response comes back");
        assert_eq!(transport.remaining(), 1, "one interaction consumed");
        let second = futures::executor::block_on(transport.send(get("https://example.test/b")))
            .expect("the second recorded interaction replays");
        assert_eq!(second.body, "b", "order is preserved");
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
                request: HttpRequest {
                    method: Method::Post,
                    url: "https://example.test/c".to_owned(),
                    body: RequestBody::Json(serde_json::json!({"k": 1})),
                },
                response: ok("{}"),
            }],
        };
        let encoded = serde_json::to_string(&cassette).expect("a cassette serialises");
        let back: Cassette = serde_json::from_str(&encoded).expect("a cassette deserialises");
        assert_eq!(back, cassette, "the fixture format survives a round trip");
    }
}
