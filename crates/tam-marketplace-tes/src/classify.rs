//! Positive-assertion classification, promoted from the M0 spike: a write is
//! `Committed`-class only when the response is JSON carrying the expected id;
//! everything the transport cannot prove is `Ambiguous` and never retried.
//! The one retry-safe class is `NotSent`, established below the HTTP layer.

use serde_json::Value;
use tam_marketplace::transport::{HttpResponse, TransportError};
use tam_marketplace::{AdapterError, AmbiguityCause, ChallengeKind};
use tam_types::{FailureCode, FailureDetail};

use crate::endpoints::DraftId;

fn detail(status: u16, body: &str) -> FailureDetail {
    let snippet: String = body.chars().take(200).collect();
    FailureDetail(format!("{status}: {snippet}"))
}

fn looks_like_signin(body: &str) -> bool {
    body.contains("<html") && (body.contains("sign-in") || body.contains("login"))
}

/// Cloudflare's interstitial, which the edge serves in place of the origin's
/// answer while it decides about the caller. Recognised by its markers rather
/// than by a status, because it arrives under 403 and under 200 alike.
fn looks_like_challenge(body: &str) -> bool {
    body.contains("/cdn-cgi/challenge-platform/h/")
        || body.contains("cf-browser-verification")
        || body.contains("Just a moment...")
}

/// Cloudflare's edge refusal, a different condition from its interstitial:
/// a blocked request is answered by the `/cdn-cgi/error` page naming a ray id
/// and a firewall rule, and it clears when the caller's address is allowed
/// rather than when the session is refreshed.
fn looks_like_edge_block(body: &str) -> bool {
    body.contains("/cdn-cgi/error")
        || body.contains("Sorry, you have been blocked")
        || body.contains("Attention Required! | Cloudflare")
        || body.contains("Error 1020")
}

/// Which condition a 401 or a 403 from the Cloudflare-fronted origin is.
///
/// The two call for different remedies and the status alone does not separate
/// them. A 401, or a 403 carrying none of Cloudflare's markup, is Tes saying
/// the cookie jar has lapsed, which a re-link fixes. A 403 carrying that
/// markup is the edge refusing the caller, which no credential refresh
/// clears. Reporting both as `SessionExpired`, as this crate did until the
/// connection-truth work, tells a seller to re-link over an egress block.
///
/// The marker sets are the Tpt crate's verbatim: they describe Cloudflare's
/// own pages rather than either marketplace, and these two adapters already
/// keep their own `detail` and `looks_like_signin` rather than sharing one.
fn denial(status: u16, body: &str) -> AdapterError {
    if looks_like_challenge(body) || (status == 403 && looks_like_edge_block(body)) {
        AdapterError::Challenge(ChallengeKind::JavaScriptInterstitial)
    } else {
        AdapterError::SessionExpired
    }
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
        // Known and declined rather than unknown: the sentence travels and the
        // item fails on it, instead of becoming an ambiguity that halts a
        // tenant and waits for an operator who has nothing to decide.
        TransportError::Refused { detail } => AdapterError::Rejected {
            code: FailureCode::Other,
            detail: FailureDetail(detail),
        },
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
            if let Ok(value) = serde_json::from_slice::<Value>(&response.body) {
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
            detail: detail(response.status, &response.text()),
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
    serde_json::from_slice::<Value>(&response.body)
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
            detail: detail(response.status, &response.text()),
        }),
        // 401, 403 and 429 land here deliberately: the write may have landed
        // before the refusal.
        _ => Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate,
        )),
    }
}

/// Classifies a delete, where the status is evidence rather than a verdict.
///
/// A 404 from either delete route is the ordinary answer for a resource that
/// is not there *on that route*, which is what an already-removed listing
/// answers and what `endpoints` records a draft answering on the published
/// route. `classify_write_status` reads it as `UploadRejected`, which
/// settles the item `Failed` and skips the driver's absence poll entirely —
/// so the one read that can tell those apart is never made, and a mapping
/// stays bound to a listing that is already gone with no path to sever it.
/// Handing the 404 to the poll instead lets the removal's own predicate,
/// which settles on absence, be the verdict.
///
/// Every other status keeps `classify_write_status`'s reading, the
/// misleading 204 included: a delete that answered success is still unproven
/// until the read agrees.
pub fn classify_delete_status(response: &HttpResponse) -> Result<(), AdapterError> {
    if response.status == 404 {
        return Ok(());
    }
    classify_write_status(response)
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
            let text = response.text();
            if looks_like_signin(&text) {
                return Err(AdapterError::SessionExpired);
            }
            serde_json::from_slice::<Value>(&response.body).map_err(|_| AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                detail: detail(response.status, &text),
            })
        }
        status @ (401 | 403) => Err(denial(status, &response.text())),
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

/// How much of a body is examined for a sign-in interstitial. A bundle is
/// arbitrarily large and decoding all of it to look for `<html` would copy
/// the whole file; an interstitial declares itself in its first bytes.
const INTERSTITIAL_HEAD_BYTES: usize = 2048;

/// Classifies a NON-mutating read whose payload is bytes rather than JSON —
/// a download bundle. Shares [`classify_read`]'s status vocabulary, and still
/// looks for the interstitial, because an expired session answers a download
/// with a sign-in page and a 200 just as readily.
pub fn classify_read_bytes(response: &HttpResponse) -> Result<&[u8], AdapterError> {
    match response.status {
        200 => {
            let head = response
                .body
                .get(..INTERSTITIAL_HEAD_BYTES)
                .unwrap_or(&response.body);
            if looks_like_signin(&String::from_utf8_lossy(head)) {
                return Err(AdapterError::SessionExpired);
            }
            Ok(&response.body)
        }
        // Only the head is decoded, for the reason the constant states; a
        // Cloudflare page declares itself well inside it.
        status @ (401 | 403) => Err(denial(
            status,
            &String::from_utf8_lossy(
                response
                    .body
                    .get(..INTERSTITIAL_HEAD_BYTES)
                    .unwrap_or(&response.body),
            ),
        )),
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
    use super::{classify_read, classify_write};
    use tam_marketplace::transport::HttpResponse;
    use tam_marketplace::{AdapterError, AmbiguityCause};
    use tam_types::FailureCode;

    const ID: i64 = 42;

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse::plain(status, body.as_bytes().to_vec())
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

    /// Synthetic: no capture in this tree was ever challenged or blocked, so
    /// these carry the markers Cloudflare's pages are documented to carry. A
    /// live capture should replace them once one is taken.
    const CHALLENGE_BODY: &str = "<html><head><title>Just a moment...</title></head><body>\
         <script src=\"/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1\">\
         </script></body></html>";
    const BLOCKED_BODY: &str = "<html><head><title>Attention Required! | Cloudflare</title>\
         </head><body>Sorry, you have been blocked<br>Error 1020\
         <a href=\"/cdn-cgi/error/1020\">ray</a></body></html>";

    #[test]
    fn a_cloudflare_challenge_is_not_a_dead_session() {
        use tam_marketplace::ChallengeKind;
        assert!(
            matches!(
                classify_read(&response(403, CHALLENGE_BODY)),
                Err(AdapterError::Challenge(
                    ChallengeKind::JavaScriptInterstitial
                ))
            ),
            "the edge deciding about the caller is not the jar having lapsed"
        );
        assert!(
            matches!(
                classify_read(&response(403, BLOCKED_BODY)),
                Err(AdapterError::Challenge(
                    ChallengeKind::JavaScriptInterstitial
                ))
            ),
            "an egress block clears when the address is allowed, not on a re-link"
        );
        assert!(
            matches!(
                super::classify_read_bytes(&response(403, CHALLENGE_BODY)),
                Err(AdapterError::Challenge(
                    ChallengeKind::JavaScriptInterstitial
                ))
            ),
            "the bundle read reads the same markers out of its decoded head"
        );
    }

    #[test]
    fn a_genuine_expiry_stays_a_dead_session() {
        assert!(
            matches!(
                classify_read(&response(401, r#"{"error":"unauthorized"}"#)),
                Err(AdapterError::SessionExpired)
            ),
            "a 401 carrying a JSON auth failure is the jar, which a re-link fixes"
        );
        assert!(
            matches!(
                classify_read(&response(403, "")),
                Err(AdapterError::SessionExpired)
            ),
            "a 403 carrying none of Cloudflare's markers stays what it always was"
        );
        assert!(
            matches!(
                super::classify_read_bytes(&response(401, "")),
                Err(AdapterError::SessionExpired)
            ),
            "the bundle read keeps the same reading of a bare 401"
        );
    }

    /// A challenge met on a *write* stays ambiguous: the write may have landed
    /// before the edge answered, and that asymmetry is deliberate.
    #[test]
    fn a_challenge_on_a_write_is_still_ambiguous() {
        assert!(
            matches!(
                classify_write(&response(403, CHALLENGE_BODY), ID),
                Err(AdapterError::Ambiguous(_))
            ),
            "the write classifier's 401/403 reading is untouched by the denial split"
        );
    }
}
