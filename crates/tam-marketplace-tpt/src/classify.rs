//! Positive-assertion classification for TPT's GraphQL reads. A GraphQL
//! service answers a failed operation with HTTP 200 and an `errors` array, so
//! the status is never the verdict here: a response is a read only when it
//! carries a `data` object and no errors.

use serde_json::Value;
use tam_marketplace::transport::{HttpResponse, ResponseHeader, TransportError};
use tam_marketplace::{AdapterError, AmbiguityCause, ChallengeKind};
use tam_types::{FailureCode, FailureDetail};

use crate::form::{scrape_form_page, TptFormPage};
use crate::read_model::ProductId;
use crate::upload::{ProcessedHandle, QueueJob, QueuePayload, QueueState, ThumbnailCollection};

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

/// Cloudflare fronts every TPT origin request. Nothing in either capture was
/// ever challenged, so this recognises the interstitial by the markers
/// Cloudflare's own managed challenge emits rather than by a status: a
/// challenge answered 200 is the shape that would otherwise be mistaken for a
/// form.
fn looks_like_challenge(body: &str) -> bool {
    body.contains("/cdn-cgi/challenge-platform")
        || body.contains("cf-browser-verification")
        || body.contains("Just a moment...")
}

/// A rendered form page, which is the sole source of the write path's tokens.
///
/// A scrape failure is reported as schema drift rather than as a transport or
/// session condition: the page arrived, it was readable, and it did not carry
/// the input the write needs, which is a statement about the form's shape.
pub fn classify_form_page(response: &HttpResponse) -> Result<TptFormPage, AdapterError> {
    let text = response.text();
    match response.status {
        200 => {
            if looks_like_challenge(&text) {
                return Err(AdapterError::Challenge(
                    ChallengeKind::JavaScriptInterstitial,
                ));
            }
            if looks_like_signin(&text) {
                return Err(AdapterError::SessionExpired);
            }
            scrape_form_page(&text).map_err(|error| AdapterError::Rejected {
                code: FailureCode::FormSchemaDrift,
                detail: FailureDetail(error.to_string()),
            })
        }
        401 | 403 if looks_like_challenge(&text) => Err(AdapterError::Challenge(
            ChallengeKind::JavaScriptInterstitial,
        )),
        401 | 403 => Err(AdapterError::SessionExpired),
        429 => Err(AdapterError::RateLimited { retry_after: None }),
        503 => Err(AdapterError::Challenge(
            ChallengeKind::JavaScriptInterstitial,
        )),
        _ => Err(AdapterError::Rejected {
            code: FailureCode::PreconditionElementAbsent,
            detail: detail(response.status, &text),
        }),
    }
}

/// One of the four XHR hops. These are ordinary JSON writes against the
/// seller's own staging area: nothing about a product exists yet, so a
/// refusal here is a refusal and not an ambiguity.
pub fn classify_xhr_json(response: &HttpResponse) -> Result<Value, AdapterError> {
    let text = response.text();
    match response.status {
        200 | 201 => {
            if looks_like_signin(&text) {
                return Err(AdapterError::SessionExpired);
            }
            serde_json::from_slice(&response.body).map_err(|error| AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                detail: FailureDetail(format!("upload hop answered unparseable JSON: {error}")),
            })
        }
        401 | 403 => Err(AdapterError::SessionExpired),
        429 => Err(AdapterError::RateLimited { retry_after: None }),
        _ => Err(AdapterError::Rejected {
            code: FailureCode::UploadRejected,
            detail: detail(response.status, &text),
        }),
    }
}

/// The job id, which arrives only in the `x-queue-tracking-id` response
/// header. A hop that answered `{"success":true}` and carried no header is
/// unpollable, and saying so beats polling an empty job forever.
pub fn queue_job(response: &HttpResponse) -> Result<QueueJob, AdapterError> {
    response
        .header(ResponseHeader::QueueTrackingId)
        .filter(|id| !id.is_empty())
        .map(|id| QueueJob::new(id.to_owned()))
        .ok_or_else(|| AdapterError::Rejected {
            code: FailureCode::UploadRejected,
            detail: FailureDetail(
                "the enqueue answered without an x-queue-tracking-id header, so the job cannot \
                 be polled"
                    .to_owned(),
            ),
        })
}

/// One poll's answer. `data` is an empty array while queued and an object
/// once running, so the three states are told apart by `status` and each
/// reads its own payload shape.
pub fn classify_queue_poll(body: &Value) -> Result<QueueState, AdapterError> {
    let status = body
        .get("status")
        .and_then(Value::as_i64)
        .ok_or_else(|| queue_shape("a poll answered without a status"))?;
    let data = body.get("data");
    match status {
        0 => Ok(QueueState::Queued),
        1 => Ok(QueueState::Running {
            fraction: data
                .and_then(|data| data.get("value"))
                .and_then(Value::as_f64)
                .unwrap_or_default(),
        }),
        2 => {
            let data = data.ok_or_else(|| queue_shape("a terminal poll carried no data"))?;
            if let Some(error) = data.get("error").and_then(Value::as_str) {
                return Err(AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    detail: FailureDetail(format!("the processing job failed: {error}")),
                });
            }
            if let Some(key) = data.get("key").and_then(Value::as_str) {
                return Ok(QueueState::Complete(QueuePayload::Processed(
                    ProcessedHandle::new(key.to_owned()),
                )));
            }
            let collection = data
                .get("collection_key")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    queue_shape("a terminal poll carried neither a key nor a collection_key")
                })?;
            let count = data
                .get("thumbnails")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Ok(QueueState::Complete(QueuePayload::Thumbnails(
                ThumbnailCollection::new(collection.to_owned(), count),
            )))
        }
        // No failure terminal was ever captured, so an unrecognised status is
        // named as unrecognised rather than assumed to be a failure or a
        // retryable state.
        other => Err(queue_shape(&format!(
            "the queue answered status {other}, which no capture contains"
        ))),
    }
}

fn queue_shape(what: &str) -> AdapterError {
    AdapterError::Rejected {
        code: FailureCode::UploadRejected,
        detail: FailureDetail(format!("queue poll: {what}")),
    }
}

/// An S3 answer's XML. S3 answers an in-flight failure with 200 and an
/// `<Error>` document, so the status is never the verdict here either.
pub fn classify_s3(response: &HttpResponse) -> Result<String, AdapterError> {
    let text = response.text().into_owned();
    if !(200..300).contains(&response.status) || text.contains("<Error>") {
        return Err(AdapterError::Rejected {
            code: FailureCode::UploadRejected,
            detail: detail(response.status, &text),
        });
    }
    Ok(text)
}

/// The ETag S3 returned for one uploaded part, which the completion body
/// must list back verbatim.
pub fn part_etag(response: &HttpResponse) -> Result<String, AdapterError> {
    response
        .header(ResponseHeader::ETag)
        .filter(|etag| !etag.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AdapterError::Rejected {
            code: FailureCode::UploadRejected,
            detail: FailureDetail(
                "the part upload answered without an ETag, so the completion cannot name it"
                    .to_owned(),
            ),
        })
}

/// The product id a submit redirected to. `/Product/test-17511712` and
/// `/Product/<slug>-13042099` both end in the id; the slug is decorative.
#[must_use]
pub fn product_id_from_location(location: &str) -> Option<ProductId> {
    location
        .rsplit(['-', '/'])
        .next()
        .and_then(|tail| tail.parse::<u64>().ok())
        .map(ProductId)
}

/// The final POST, and the one classification in this crate that refuses to
/// guess.
///
/// A 302 whose `Location` yields a product id is the only shape any capture
/// contains, and it is the only shape read as a landing. Everything else —
/// a 200 carrying the form again, a `SecurityComponent` blackhole, a
/// challenge, a redirect somewhere else — is answered with
/// `NoDurableIdentifier`, because no capture contains a refusal and the shape
/// of one is therefore unknown. Reading a 200-with-form-HTML as a clean
/// rejection would be inventing evidence that the product was not created,
/// and an ambiguous attempt is reconciled or halted rather than retried,
/// which is the safe side of that unknown.
pub fn classify_submit(response: &HttpResponse) -> Result<SubmitLanding, AdapterError> {
    let landed = (response.status == 302)
        .then(|| response.header(ResponseHeader::Location))
        .flatten()
        .and_then(|location| {
            product_id_from_location(location).map(|product| SubmitLanding {
                product,
                location: location.to_owned(),
            })
        });
    landed.ok_or(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier))
}

/// Where a submit landed: the durable identifier and the route that carried
/// it, both of which the write evidence records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitLanding {
    pub product: ProductId,
    pub location: String,
}

/// True where a non-redirect answer looks like CakePHP's `SecurityComponent`
/// blackhole — the form re-rendered, or a bare bad request. Diagnostic only:
/// [`classify_submit`] answers `NoDurableIdentifier` either way, because a
/// blackhole that fired after the record was written looks the same from here
/// as one that fired before.
#[must_use]
pub fn looks_like_blackhole(response: &HttpResponse) -> bool {
    let text = response.text();
    response.status == 400 || (response.status == 200 && text.contains("id=\"ItemAddForm\""))
}

#[cfg(test)]
mod tests {
    use super::classify_graphql_read;
    use tam_marketplace::transport::HttpResponse;
    use tam_marketplace::AdapterError;
    use tam_types::FailureCode;

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse::plain(status, body.as_bytes().to_vec())
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
    fn a_three_oh_two_whose_location_names_a_product_is_the_only_landing() {
        let landed = super::classify_submit(&HttpResponse {
            status: 302,
            body: Vec::new(),
            headers: vec![(
                tam_marketplace::transport::ResponseHeader::Location,
                "/Product/test-17511712".to_owned(),
            )],
        });
        assert_eq!(
            landed.map(|landing| landing.product),
            Ok(crate::read_model::ProductId(17_511_712)),
            "the slug is decorative; the trailing number is the durable identifier"
        );
    }

    #[test]
    fn every_other_answer_to_the_final_post_is_an_ambiguity() {
        let ambiguous =
            AdapterError::Ambiguous(tam_marketplace::AmbiguityCause::NoDurableIdentifier);
        let form_again = HttpResponse::plain(
            200,
            b"<html><form id=\"ItemAddForm\"></form></html>".to_vec(),
        );
        assert_eq!(
            super::classify_submit(&form_again).err(),
            Some(ambiguous.clone()),
            "no capture contains a refusal, so a re-rendered form proves nothing"
        );
        assert!(
            super::looks_like_blackhole(&form_again),
            "it is recognisably a blackhole, and recognising it still does not settle the write"
        );
        let bad_request = HttpResponse::plain(400, Vec::new());
        assert_eq!(
            super::classify_submit(&bad_request).err(),
            Some(ambiguous.clone()),
            "a SecurityComponent blackhole may have fired before or after the record was written"
        );
        assert!(super::looks_like_blackhole(&bad_request));
        let elsewhere = HttpResponse {
            status: 302,
            body: Vec::new(),
            headers: vec![(
                tam_marketplace::transport::ResponseHeader::Location,
                "/My-Products".to_owned(),
            )],
        };
        assert_eq!(
            super::classify_submit(&elsewhere).err(),
            Some(ambiguous),
            "a redirect carrying no product id yields no durable identifier"
        );
        assert!(
            !super::looks_like_blackhole(&elsewhere),
            "and a redirect is not a blackhole"
        );
    }

    #[test]
    fn the_queue_states_are_told_apart_by_status_and_each_reads_its_own_shape() {
        use crate::upload::{QueuePayload, QueueState};
        let poll = |body: &str| {
            super::classify_queue_poll(
                &serde_json::from_str(body).expect("the fixture body parses"),
            )
        };
        assert_eq!(poll(r#"{"status":0,"data":[]}"#), Ok(QueueState::Queued));
        assert_eq!(
            poll(r#"{"status":1,"data":{"value":0.87,"message":""}}"#),
            Ok(QueueState::Running { fraction: 0.87 }),
            "data is an array while queued and an object while running"
        );
        let terminal = poll(r#"{"status":2,"data":{"error":null,"key":"KEY"}}"#);
        assert!(
            matches!(
                terminal,
                Ok(QueueState::Complete(QueuePayload::Processed(_)))
            ),
            "the processing job's terminal payload is the key the form posts, got {terminal:?}"
        );
        let thumbs =
            poll(r#"{"status":2,"data":{"thumbnails":[{"original":"u"}],"collection_key":"CK"}}"#);
        let Ok(QueueState::Complete(QueuePayload::Thumbnails(collection))) = thumbs else {
            panic!("the thumbnail job's terminal payload is a collection, got {thumbs:?}");
        };
        assert_eq!(collection.thumbnail_count(), 1);
        assert!(
            poll(r#"{"status":7,"data":{}}"#).is_err(),
            "no failure terminal was captured, so an unknown status is named as unknown"
        );
    }

    #[test]
    fn a_job_id_is_read_from_the_header_because_no_body_carries_one() {
        use tam_marketplace::transport::ResponseHeader;
        let with_header = HttpResponse {
            status: 200,
            body: br#"{"success":true}"#.to_vec(),
            headers: vec![(ResponseHeader::QueueTrackingId, "abc".to_owned())],
        };
        assert_eq!(
            super::queue_job(&with_header).map(|job| job.as_str().to_owned()),
            Ok("abc".to_owned())
        );
        assert!(
            super::queue_job(&HttpResponse::plain(200, br#"{"success":true}"#.to_vec())).is_err(),
            "a flow that reads only bodies would poll an empty job forever"
        );
    }

    #[test]
    fn an_s3_error_document_answered_two_hundred_is_not_a_stored_object() {
        assert!(
            super::classify_s3(&HttpResponse::plain(
                200,
                b"<Error><Code>SignatureDoesNotMatch</Code></Error>".to_vec()
            ))
            .is_err(),
            "S3 answers an in-flight failure with 200 and an error document"
        );
        assert!(
            super::classify_s3(&HttpResponse::plain(200, b"<Ok/>".to_vec())).is_ok(),
            "and anything else is handed to the caller's own parser"
        );
    }

    #[test]
    fn a_challenge_is_a_challenge_whatever_status_it_arrives_under() {
        use tam_marketplace::ChallengeKind;
        let interstitial = "<html><head><title>Just a moment...</title></head>\
                            <body><script src=\"/cdn-cgi/challenge-platform/h\"></script>\
                            </body></html>";
        for status in [200_u16, 403] {
            assert_eq!(
                super::classify_form_page(&HttpResponse::plain(
                    status,
                    interstitial.as_bytes().to_vec()
                )),
                Err(AdapterError::Challenge(
                    ChallengeKind::JavaScriptInterstitial
                )),
                "a challenge answered {status} is parked, never retried"
            );
        }
        assert_eq!(
            super::classify_form_page(&HttpResponse::plain(
                200,
                b"<html>Please sign-in</html>".to_vec()
            )),
            Err(AdapterError::SessionExpired),
            "and a sign-in page is a session condition rather than drift"
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
