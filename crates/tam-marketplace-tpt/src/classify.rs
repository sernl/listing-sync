//! Positive-assertion classification for TPT's GraphQL reads. A GraphQL
//! service answers a failed operation with HTTP 200 and an `errors` array, so
//! the status is never the verdict here: a response is a read only when it
//! carries a `data` object and no errors.

use serde_json::Value;
use tam_marketplace::transport::{HttpResponse, TransportError};
use tam_marketplace::{AdapterError, AmbiguityCause};
use tam_types::{FailureCode, FailureDetail};

fn detail(status: u16, body: &str) -> FailureDetail {
    let snippet: String = body.chars().take(200).collect();
    FailureDetail(format!("{status}: {snippet}"))
}

fn looks_like_signin(body: &str) -> bool {
    body.contains("<html") && (body.contains("sign-in") || body.contains("Sign In"))
}

/// Maps a transport failure onto the seam vocabulary. `AfterSend` is the
/// response-event-lost ambiguity by definition; `Harness` occurs only under
/// the cassette transport and surfaces as a rejection so a diverging test
/// fails loudly rather than reading as a marketplace condition.
#[must_use]
pub fn classify_transport(error: TransportError) -> AdapterError {
    match error {
        TransportError::NotSent(cause) => AdapterError::NotSent(cause),
        TransportError::AfterSend { .. } => {
            AdapterError::Ambiguous(AmbiguityCause::ResponseEventLost)
        }
        TransportError::Harness { detail } => AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(format!("cassette divergence: {detail}")),
        },
    }
}

/// Renders a GraphQL `errors` array into one detail line, so a rejection
/// names what the service objected to rather than the status it used.
fn error_detail(errors: &[Value]) -> FailureDetail {
    let messages: Vec<String> = errors
        .iter()
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("<no message>")
                .to_owned()
        })
        .collect();
    FailureDetail(format!("graphql errors: {}", messages.join("; ")))
}

/// Classifies a NON-mutating GraphQL read. No write state is at stake, so the
/// conditions are named plainly rather than treated as ambiguities.
pub fn classify_graphql_read(response: &HttpResponse) -> Result<Value, AdapterError> {
    match response.status {
        200 => {
            let text = response.text();
            if looks_like_signin(&text) {
                return Err(AdapterError::SessionExpired);
            }
            let body: Value =
                serde_json::from_slice(&response.body).map_err(|_| AdapterError::Rejected {
                    code: FailureCode::VerificationMismatch,
                    detail: detail(response.status, &text),
                })?;
            if let Some(errors) = body.get("errors").and_then(Value::as_array) {
                if !errors.is_empty() {
                    return Err(AdapterError::Rejected {
                        code: FailureCode::Other,
                        detail: error_detail(errors),
                    });
                }
            }
            if !body.get("data").is_some_and(Value::is_object) {
                return Err(AdapterError::Rejected {
                    code: FailureCode::VerificationMismatch,
                    detail: detail(response.status, &text),
                });
            }
            Ok(body)
        }
        401 | 403 => Err(AdapterError::SessionExpired),
        429 => Err(AdapterError::RateLimited { retry_after: None }),
        404 => Err(AdapterError::Rejected {
            code: FailureCode::PreconditionElementAbsent,
            detail: detail(response.status, &response.text()),
        }),
        _ => Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::classify_graphql_read;
    use tam_marketplace::transport::HttpResponse;
    use tam_marketplace::AdapterError;
    use tam_types::FailureCode;

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_two_hundred_carrying_graphql_errors_is_not_a_read() {
        let body = r#"{"data":null,"errors":[{"message":"Not authorised"}]}"#;
        let outcome = classify_graphql_read(&response(200, body));
        let Err(AdapterError::Rejected { code, detail }) = outcome else {
            panic!("a 200 with an errors array must be a rejection, got {outcome:?}");
        };
        assert_eq!(
            code,
            FailureCode::Other,
            "the service objected, not the transport"
        );
        assert!(
            detail.0.contains("Not authorised"),
            "the rejection names what the service said, got {detail:?}"
        );
    }

    #[test]
    fn a_two_hundred_without_a_data_object_is_refused() {
        let outcome = classify_graphql_read(&response(200, r#"{"errors":[]}"#));
        assert!(
            matches!(
                outcome,
                Err(AdapterError::Rejected {
                    code: FailureCode::VerificationMismatch,
                    ..
                })
            ),
            "a body with no data object is not a read, got {outcome:?}"
        );
    }

    #[test]
    fn a_sign_in_interstitial_answered_two_hundred_is_an_expired_session() {
        let body = "<html><body>Please sign-in to continue</body></html>";
        assert_eq!(
            classify_graphql_read(&response(200, body)),
            Err(AdapterError::SessionExpired),
            "an edge cache answering 200 with a sign-in page is not a read"
        );
    }

    #[test]
    fn the_refusal_statuses_map_to_named_conditions() {
        assert_eq!(
            classify_graphql_read(&response(403, "")),
            Err(AdapterError::SessionExpired),
            "a forbidden read is a session condition"
        );
        assert_eq!(
            classify_graphql_read(&response(429, "")),
            Err(AdapterError::RateLimited { retry_after: None }),
            "no capture exposes a Retry-After, so none is invented"
        );
    }

    #[test]
    fn a_clean_body_comes_back_whole() {
        let body = r#"{"data":{"seller":{"resources":{"results":[]}}}}"#;
        let parsed = classify_graphql_read(&response(200, body)).expect("a data object is a read");
        assert!(
            parsed.pointer("/data/seller/resources/results").is_some(),
            "the caller parses the response it was handed"
        );
    }
}
