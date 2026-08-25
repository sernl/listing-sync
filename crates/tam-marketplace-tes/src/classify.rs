//! Positive-assertion classification, promoted from the M0 spike: a write is
//! `Committed`-class only when the response is JSON carrying the expected id;
//! everything the transport cannot prove is `Ambiguous` and never retried.
//! The one retry-safe class is `NotSent`, established below the HTTP layer.

use serde_json::Value;
use tam_marketplace::transport::{HttpResponse, TransportError};
use tam_marketplace::{AdapterError, AmbiguityCause};
use tam_types::{FailureCode, FailureDetail};

use crate::endpoints::DraftId;

fn detail(status: u16, body: &str) -> FailureDetail {
    let snippet: String = body.chars().take(200).collect();
    FailureDetail(format!("{status}: {snippet}"))
}

fn looks_like_signin(body: &str) -> bool {
    body.contains("<html") && (body.contains("sign-in") || body.contains("login"))
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

/// Classifies a MUTATING call. 401, 403 and 429 are ambiguous here — the
/// write may have landed before the refusal — where a read classifies them
/// as session or rate conditions; that asymmetry is the whole point of
/// having two classifiers. 2xx interstitials and unexpected shapes are
/// ambiguous because an edge cache answering 200 with a sign-in page was
/// measured in M-1.
pub fn classify_write(response: &HttpResponse, expected_id: i64) -> Result<Value, AdapterError> {
    match response.status {
        200 | 201 => {
            if let Ok(value) = serde_json::from_str::<Value>(&response.body) {
                if value.get("id").and_then(Value::as_i64) == Some(expected_id) {
                    return Ok(value);
                }
            }
            Err(AdapterError::Ambiguous(
                AmbiguityCause::ReadBackIndeterminate,
            ))
        }
        408 => Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut)),
        400 | 404 | 422 => Err(AdapterError::Rejected {
            code: FailureCode::UploadRejected,
            detail: detail(response.status, &response.body),
        }),
        // 401, 403 and 429 land here deliberately: the write may have
        // landed before the refusal, so they are ambiguous like any other
        // unproven status, per the doc comment above.
        _ => Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
    }
}

/// Classifies a mutating call whose positive assertion is JSON of any
/// shape; the id assertion lives with the caller where one exists.
pub fn classify_write_json(response: &HttpResponse) -> Result<Value, AdapterError> {
    classify_write_status(response)?;
    serde_json::from_str::<Value>(&response.body)
        .map_err(|_| AdapterError::Ambiguous(AmbiguityCause::ReadBackIndeterminate))
}

/// Classifies a mutating call by status alone, for steps whose real
/// verification is a separate read (publish) or a later confirm (the S3
/// POST, which answers with XML or nothing).
pub fn classify_write_status(response: &HttpResponse) -> Result<(), AdapterError> {
    match response.status {
        200..=299 => Ok(()),
        408 => Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut)),
        400 | 404 | 422 => Err(AdapterError::Rejected {
            code: FailureCode::UploadRejected,
            detail: detail(response.status, &response.body),
        }),
        // 401, 403 and 429 land here deliberately: the write may have landed
        // before the refusal.
        _ => Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
    }
}

/// Classifies a create: the positive assertion is a JSON body carrying the
/// new identifier, and a 2xx without one is precisely the
/// no-durable-identifier ambiguity.
pub fn classify_create(response: &HttpResponse) -> Result<DraftId, AdapterError> {
    let value = classify_write_json(response)?;
    value
        .get("id")
        .and_then(Value::as_i64)
        .map(DraftId)
        .ok_or(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier))
}

/// Classifies a NON-mutating read, where no write state is at stake and the
/// conditions can be named plainly.
pub fn classify_read(response: &HttpResponse) -> Result<Value, AdapterError> {
    match response.status {
        200 => {
            if looks_like_signin(&response.body) {
                return Err(AdapterError::SessionExpired);
            }
            serde_json::from_str::<Value>(&response.body).map_err(|_| AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: detail(response.status, &response.body),
            })
        }
        401 | 403 => Err(AdapterError::SessionExpired),
        429 => Err(AdapterError::RateLimited { retry_after: None }),
        404 => Err(AdapterError::Rejected {
            code: FailureCode::PreconditionElementAbsent,
            detail: detail(response.status, &response.body),
        }),
        _ => Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_read, classify_write};
    use tam_marketplace::transport::HttpResponse;
    use tam_marketplace::{AdapterError, AmbiguityCause};
    use tam_types::FailureCode;

    const ID: i64 = 42;

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.to_owned(),
        }
    }

    #[test]
    fn committed_needs_the_expected_id() {
        let value =
            classify_write(&response(200, r#"{"id":42}"#), ID).expect("the expected id commits");
        assert_eq!(value["id"], 42, "the classified value carries the body");
    }

    #[test]
    fn a_wrong_id_is_ambiguous_not_committed() {
        assert!(
            matches!(
                classify_write(&response(200, r#"{"id":99}"#), ID),
                Err(AdapterError::Ambiguous(_))
            ),
            "a 2xx naming a different id proves nothing about our write"
        );
    }

    #[test]
    fn a_truncated_body_is_ambiguous() {
        assert!(
            matches!(
                classify_write(&response(200, r#"{"id"#), ID),
                Err(AdapterError::Ambiguous(_))
            ),
            "a 2xx with an unparseable body is not a verdict"
        );
    }

    #[test]
    fn a_signin_interstitial_is_ambiguous_for_a_write() {
        assert!(
            matches!(
                classify_write(&response(200, "<html>please sign-in</html>"), ID),
                Err(AdapterError::Ambiguous(_))
            ),
            "an edge cache answering 200 with a login page was measured in M-1"
        );
    }

    #[test]
    fn an_auth_challenge_is_ambiguous_for_a_write() {
        assert!(
            matches!(
                classify_write(&response(403, ""), ID),
                Err(AdapterError::Ambiguous(_))
            ),
            "the write may have landed before the refusal"
        );
    }

    #[test]
    fn a_rate_limit_is_ambiguous_for_a_write() {
        assert!(
            matches!(
                classify_write(&response(429, ""), ID),
                Err(AdapterError::Ambiguous(_))
            ),
            "a throttled write's state is unknown"
        );
    }

    #[test]
    fn a_hard_reject_is_failed() {
        assert!(
            matches!(
                classify_write(&response(422, "bad licence"), ID),
                Err(AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    ..
                })
            ),
            "a definitive 4xx is the one honest failure class"
        );
    }

    #[test]
    fn a_timeout_status_is_the_timed_out_ambiguity() {
        assert!(
            matches!(
                classify_write(&response(408, ""), ID),
                Err(AdapterError::Ambiguous(AmbiguityCause::SubmitTimedOut))
            ),
            "408 names the cause precisely"
        );
    }

    #[test]
    fn a_read_names_session_and_rate_conditions_plainly() {
        assert!(
            matches!(
                classify_read(&response(401, "")),
                Err(AdapterError::SessionExpired)
            ),
            "no write state is at stake on a read"
        );
        assert!(
            matches!(
                classify_read(&response(429, "")),
                Err(AdapterError::RateLimited { .. })
            ),
            "a throttled read is just throttled"
        );
    }

    #[test]
    fn a_read_of_a_signin_interstitial_is_a_dead_session() {
        assert!(
            matches!(
                classify_read(&response(200, "<html>login</html>")),
                Err(AdapterError::SessionExpired)
            ),
            "positive assertion: a 200 that is not the resource is not a success"
        );
    }
}
