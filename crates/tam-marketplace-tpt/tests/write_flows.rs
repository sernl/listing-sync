//! The TPT write path driven end to end against cassettes.
//!
//! Two committed fixtures carry the only recorded shape a write depends on:
//! `create_form_page.json` and `edit_form_page.json` are renders of the two
//! product forms, reduced to what the scrape reads. Every token in them is a
//! placeholder of the captured shape — no real `_Token` hash, no real CSRF
//! value, no real AWS access key id, no seller id and no asset handle is
//! committed anywhere in this repository. The two canaries at the foot of
//! this file hold that claim by scanning for the shapes rather than for the
//! values, which is what keeps the claim from being made in the same file
//! that would break it.
//!
//! The remaining hops are built here from the endpoint builders rather than
//! hand-authored as JSON, which is what makes the placeholder signatures
//! consistent: the flow derives each `StringToSign` from the ticket and the
//! bytes, the recorded `sign_auth` answer is a placeholder, and the recorded
//! S3 request carries that same placeholder because the flow put it there.
//! Every negative fixture below is hand-authored and says so; no capture
//! contains a refusal of any kind.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestAuth, RequestBody, ResponseHeader, Transport,
    TransportError,
};
use tam_marketplace::{
    AdapterError, AmbiguityCause, ChallengeKind, FetchReason, FieldSet, FileContent, FileSource,
    FileSourceError, FormId, IdempotencyKey, LifecycleTransition, ListingLocator, ListingState,
    MarketplaceAdapter, NativeTerm, ProjectedListing, RemoteLifecycle, RemoteListingId,
    RemovalPlan, RevisePlan, WriteAttemptId,
};
use tam_marketplace_tpt::endpoints::{self, FormTarget, SignedS3Call, UploadReservation};
use tam_marketplace_tpt::s3::{self, S3Operation, S3Signature, UploadSlot, UploadTicket};
use tam_marketplace_tpt::upload::{cache_buster, Hop, ProcessedHandle, QueueJob, UploadHandle};
use tam_marketplace_tpt::write_model::{self, AuthorshipDeclaration};
use tam_marketplace_tpt::{InstantPause, ProductId, TptAdapter};
use tam_types::{
    ContentHash, CopyFormat, FailureCode, FieldKey, FileId, InventoryId, Timestamp, Uuid,
};

// Placeholders throughout. Same shape as the captured values, same length
// class where the length matters, and no relationship to anything live.
const BUCKET: &str = "live.digital.upload";
const OBJECT_PATH: &str =
    "de07-00000000-2026-08-28/product/0123456789abcdef0123456789abcdef_000000001.png";
const AWS_KEY_ID: &str = "AKIAPLACEHOLDER00000";
const SIGNATURE: &str = "cGxhY2Vob2xkZXJzaWduYXR1cmU=";
const AMZ_DATE: &str = "Fri, 28 Aug 2026 05:57:21 GMT";
const UPLOAD_ID: &str = "PLACEHOLDERUPLOADID";
const UPLOAD_HANDLE: &str = "PLACEHOLDER-UPLOAD-HANDLE";
const PROCESSED_HANDLE: &str = "PLACEHOLDER-PROCESSED-HANDLE";
const COLLECTION_KEY: &str = "PLACEHOLDER-COLLECTION-KEY==";
const PROCESS_JOB: &str = "00000000000000000000000000000001";
const THUMBS_JOB: &str = "00000000000000000000000000000002";
const PRODUCT_ID: u64 = 17_511_712;
const NOW: Timestamp = Timestamp(1_787_896_640_872);

fn key() -> IdempotencyKey {
    IdempotencyKey(Uuid([5; 16]))
}

fn file() -> FileContent {
    FileContent {
        file_name: "fractions.png".to_owned(),
        content_type: "image/png".to_owned(),
        // A few hundred bytes rather than the captured 224 KB image: a
        // fixture proves the sequence, not the payload.
        bytes: (0_u8..=255).cycle().take(300).collect(),
    }
}

/// Hands back one file's bytes. The pipeline's own source lands with M1f;
/// a test hands bytes straight back.
struct OneFile(FileContent);

impl FileSource for OneFile {
    fn fetch(
        &self,
        _file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Ok(self.0.clone()))
    }
}

fn attested() -> AuthorshipDeclaration {
    AuthorshipDeclaration::attested("founder@example.invalid".to_owned(), Timestamp(1))
}

fn adapter(cassette: Cassette) -> TptAdapter<CassetteTransport, OneFile, InstantPause> {
    TptAdapter::new(
        CassetteTransport::new(cassette),
        OneFile(file()),
        InstantPause,
    )
    .attesting(attested())
}

/// A transport that loses the response to one nominated hop and replays the
/// cassette for every other. Hand-authored: every recorded hop answered, and
/// which hop loses its answer is precisely what is under test.
struct LosesHop {
    inner: CassetteTransport,
    lose_at: usize,
    seen: AtomicUsize,
}

impl Transport for LosesHop {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        if self.seen.fetch_add(1, Ordering::SeqCst) == self.lose_at {
            return Err(TransportError::AfterSend {
                detail: "the response never arrived".to_owned(),
            });
        }
        self.inner.send(request).await
    }
}

fn losing(cassette: Cassette, lose_at: usize) -> TptAdapter<LosesHop, OneFile, InstantPause> {
    TptAdapter::new(
        LosesHop {
            inner: CassetteTransport::new(cassette),
            lose_at,
            seen: AtomicUsize::new(0),
        },
        OneFile(file()),
        InstantPause,
    )
    .attesting(attested())
}

fn projected() -> ProjectedListing {
    ProjectedListing {
        title: "Fractions pack".to_owned(),
        body: "<p>ten worksheets</p>".to_owned(),
        price: tam_types::PriceIntent::Free,
        taxonomy: vec![
            NativeTerm {
                native_id: Some("math".to_owned()),
                segments: vec!["Math".to_owned()],
            },
            NativeTerm {
                native_id: Some("unit-plans".to_owned()),
                segments: vec!["Complete Topic".to_owned()],
            },
        ],
        grades: vec![NativeTerm {
            native_id: Some("4th-grade".to_owned()),
            segments: vec!["Grade 4".to_owned()],
        }],
        ages: None,
        files: vec![FileId(Uuid([1; 16]))],
        body_format: CopyFormat::Html,
        natives: Vec::new(),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn fields() -> FieldSet {
    write_model::project_fields(&projected()).expect("a free listing projects")
}

fn ticket() -> UploadTicket {
    UploadTicket::new(BUCKET.to_owned(), OBJECT_PATH.to_owned())
}

fn text(status: u16, body: &str) -> HttpResponse {
    HttpResponse::plain(status, body.as_bytes().to_vec())
}

fn with_header(status: u16, body: &str, header: ResponseHeader, value: &str) -> HttpResponse {
    HttpResponse {
        status,
        body: body.as_bytes().to_vec(),
        headers: vec![(header, value.to_owned())],
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn form_page(fixture: &str) -> Interaction {
    let cassette: Cassette =
        serde_json::from_str(fixture).expect("the committed form render parses");
    cassette
        .interactions
        .into_iter()
        .next()
        .expect("the fixture holds the render")
}

fn create_render() -> Interaction {
    form_page(include_str!("cassettes/create_form_page.json"))
}

fn edit_render() -> Interaction {
    form_page(include_str!("cassettes/edit_form_page.json"))
}

/// TPT's clock as each signed call reads it. The reads advance, which is the
/// whole point of reading per call: one instant minted at the initiate is
/// already skewed by the time a long upload reaches its last part, and AWS
/// rejects a SigV2 signature outside its fifteen-minute window.
fn amz_date(step: u32) -> String {
    format!(
        "Fri, 28 Aug 2026 05:57:{:02} GMT",
        21_u32.saturating_add(step)
    )
}

/// One signing round trip: the request the flow will build for this
/// operation, and the placeholder signature the oracle answers with.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn sign(operation: &S3Operation, content_md5: Option<&str>, date: &str) -> Interaction {
    let ticket = ticket();
    let signable = s3::string_to_sign(&ticket, operation, content_md5, "image/png", date);
    Interaction {
        request: endpoints::sign_auth_request(&ticket, &signable, date)
            .expect("this operation is in scope for its own ticket"),
        response: text(200, SIGNATURE),
    }
}

fn s3_call(
    operation: &S3Operation,
    content_md5: Option<&str>,
    body: RequestBody,
    date: &str,
) -> HttpRequest {
    let ticket = ticket();
    let key_id = s3::AwsKeyId::new(AWS_KEY_ID.to_owned());
    let signature = S3Signature::new(SIGNATURE.to_owned());
    endpoints::s3_request(
        &SignedS3Call {
            ticket: &ticket,
            operation,
            key_id: &key_id,
            signature: &signature,
            amz_date: date,
            content_type: "image/png",
            content_md5,
        },
        body,
    )
}

/// The three hops one signed S3 call issues: the clock read, the signature
/// over that instant, and the call carrying the same instant.
fn signed_call(
    step: u32,
    operation: &S3Operation,
    content_md5: Option<&str>,
    body: RequestBody,
    response: HttpResponse,
) -> [Interaction; 3] {
    let date = amz_date(step);
    [
        Interaction {
            request: endpoints::time_request(NOW.0.saturating_add(i64::from(step))),
            response: text(200, &date),
        },
        sign(operation, content_md5, &date),
        Interaction {
            request: s3_call(operation, content_md5, body, &date),
            response,
        },
    ]
}

/// The whole out-of-band upload, from the key reservation to the two
/// handles the product form consumes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn staging_chain(part_size: usize) -> Vec<Interaction> {
    let payload = file().bytes;
    let mut chain = vec![Interaction {
        request: endpoints::upload_file_request(&UploadReservation {
            slot: UploadSlot::Product,
            file_name: "fractions.png",
            size: payload.len(),
            last_modified_ms: NOW.0,
            item_id: None,
        }),
        response: text(
            200,
            &format!(r#"{{"key":"{UPLOAD_HANDLE}","bucket":"{BUCKET}","path":"{OBJECT_PATH}"}}"#),
        ),
    }];
    chain.extend(signed_call(
        0,
        &S3Operation::Initiate,
        None,
        RequestBody::Empty,
        text(
            200,
            &format!(
                "<?xml version=\"1.0\"?><InitiateMultipartUploadResult><Bucket>{BUCKET}\
                 </Bucket><Key>{OBJECT_PATH}</Key><UploadId>{UPLOAD_ID}</UploadId>\
                 </InitiateMultipartUploadResult>"
            ),
        ),
    ));

    let plan = s3::plan_parts(payload.len(), part_size).expect("the payload is plannable");
    let mut completed: Vec<(u32, String)> = Vec::new();
    for part in &plan {
        let bytes = part
            .slice(&payload)
            .expect("a planned part fits its payload");
        let digest = s3::content_md5(bytes);
        let operation = S3Operation::UploadPart {
            part_number: part.number,
            upload_id: UPLOAD_ID.to_owned(),
        };
        let etag = format!("\"placeholderetag{:016}\"", part.number);
        chain.extend(signed_call(
            part.number,
            &operation,
            Some(&digest),
            RequestBody::Bytes(bytes.to_vec()),
            with_header(200, "", ResponseHeader::ETag, &etag),
        ));
        completed.push((part.number, etag));
    }

    let complete = S3Operation::Complete {
        upload_id: UPLOAD_ID.to_owned(),
    };
    let parts = u32::try_from(plan.len()).unwrap_or(u32::MAX);
    chain.extend(signed_call(
        parts.saturating_add(1),
        &complete,
        None,
        RequestBody::Bytes(s3::complete_multipart_body(&completed).into_bytes()),
        text(
            200,
            "<CompleteMultipartUploadResult><Location>s3</Location><ETag>\"e-1\"</ETag>\
             </CompleteMultipartUploadResult>",
        ),
    ));

    chain.extend(queue_chain());
    chain
}

/// The two async jobs that exchange a staged object for the two handles the
/// product form consumes: enqueue, poll to the terminal answer, twice.
fn queue_chain() -> Vec<Interaction> {
    let staged = UploadHandle::new(UPLOAD_HANDLE.to_owned());
    let processed = ProcessedHandle::new(PROCESSED_HANDLE.to_owned());
    vec![
        Interaction {
            request: endpoints::process_file_request(
                &staged,
                None,
                &cache_buster(key(), Hop::ProcessFile.ordinal()),
            ),
            response: with_header(
                200,
                r#"{"success":true,"error":""}"#,
                ResponseHeader::QueueTrackingId,
                PROCESS_JOB,
            ),
        },
        poll(
            PROCESS_JOB,
            1,
            0,
            &format!(r#"{{"status":2,"data":{{"error":null,"key":"{PROCESSED_HANDLE}"}}}}"#),
        ),
        Interaction {
            request: endpoints::generate_thumbs_request(
                &processed,
                None,
                &cache_buster(key(), Hop::GenerateThumbs.ordinal()),
            ),
            response: with_header(
                200,
                r#"{"success":true}"#,
                ResponseHeader::QueueTrackingId,
                THUMBS_JOB,
            ),
        },
        poll(THUMBS_JOB, 2, 0, r#"{"status":0,"data":[]}"#),
        poll(
            THUMBS_JOB,
            2,
            1,
            &format!(
                r#"{{"status":2,"data":{{"thumbnails":[{{"original":"https://example.invalid/0.jpg"}}],"collection_key":"{COLLECTION_KEY}"}}}}"#
            ),
        ),
    ]
}

fn poll(job: &str, ordinal: u32, attempt: u32, body: &str) -> Interaction {
    Interaction {
        request: endpoints::queue_results_request(
            &QueueJob::new(job.to_owned()),
            &cache_buster(
                key(),
                Hop::QueuePoll {
                    job: ordinal,
                    attempt,
                }
                .ordinal(),
            ),
        ),
        response: text(200, body),
    }
}

/// The cassette transport compares the whole request, so the submit's
/// recorded body has to be the one the builder produces. Built here from the
/// same render the chain replays.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn create_submit(location: &str) -> Interaction {
    let page = tam_marketplace_tpt::form::scrape_form_page(&create_render().response.text())
        .expect("the committed render parses");
    let listing =
        write_model::listing_from_field_set(&fields()).expect("the projection parses back");
    let authorship = attested();
    let body = write_model::create_fields(&write_model::CreateSubmission {
        tokens: page.tokens(),
        listing: &listing,
        product: &ProcessedHandle::new(PROCESSED_HANDLE.to_owned()),
        thumbs_collection_key: COLLECTION_KEY,
        authorship: &authorship,
    });
    Interaction {
        request: endpoints::submit_form_request(FormTarget::CreateDigital, body),
        response: with_header(302, "", ResponseHeader::Location, location),
    }
}

fn create_cassette(part_size: usize, location: &str) -> Cassette {
    let mut interactions = vec![create_render()];
    interactions.extend(staging_chain(part_size));
    interactions.push(create_submit(location));
    Cassette { interactions }
}

#[test]
fn the_create_chain_walks_every_hop_and_lands_on_the_redirect_location() {
    let cassette = create_cassette(usize::MAX, "/Product/test-17511712");
    let hops = cassette.interactions.len();
    let adapter = adapter(cassette);
    let evidence = futures::executor::block_on(adapter.submit(key(), fields(), NOW))
        .expect("the recorded create chain replays");
    assert_eq!(
        hops, 17,
        "the render, the reservation, three signed S3 calls each reading the clock and taking \
         its own signature, the two enqueues, three queue polls and the form post"
    );
    assert_eq!(
        evidence.landed,
        Some(RemoteListingId::Tpt {
            product_id: PRODUCT_ID
        }),
        "the product id lives only in the redirect's Location"
    );
    assert_eq!(
        evidence.landed_on_route.as_deref(),
        Some("/Product/test-17511712"),
        "the route the write landed on is recorded as evidence"
    );
    assert_eq!(
        evidence.http_status,
        Some(302),
        "a followed redirect would have reported the product page's status"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the fixture recorded exactly the hops the flow issues"
    );
}

#[test]
fn a_payload_over_the_part_size_signs_and_puts_each_part_under_its_own_digest() {
    // No capture covers a multi-part upload: the one recorded file went as a
    // single 223 791-byte part. The part size is cut down here so a fixture a
    // few hundred bytes long exercises the loop the wire never showed us.
    let cassette = create_cassette(128, "/Product/test-17511712");
    let signed = cassette
        .interactions
        .iter()
        .filter(|hop| hop.request.url.contains("/uploads/sign_auth"))
        .count();
    let puts = cassette
        .interactions
        .iter()
        .filter(|hop| hop.request.method == Method::Put)
        .count();
    assert_eq!(puts, 3, "300 bytes at 128 per part is three parts");
    assert_eq!(
        signed, 5,
        "one signature per S3 call: initiate, three parts and the completion"
    );
    let digests: Vec<&str> = cassette
        .interactions
        .iter()
        .filter_map(|hop| match hop.request.auth {
            RequestAuth::S3SigV2 {
                content_md5: Some(ref digest),
                ..
            } => Some(digest.as_str()),
            RequestAuth::Session | RequestAuth::Anonymous | RequestAuth::S3SigV2 { .. } => None,
        })
        .collect();
    assert_eq!(
        digests.len(),
        3,
        "a Content-MD5 rides every part and no other call"
    );
    assert!(
        digests.first() != digests.get(1),
        "the digest is per part; the captured single-part upload hid that, got {digests:?}"
    );
    let adapter = adapter(cassette).with_part_size(128);
    futures::executor::block_on(adapter.submit(key(), fields(), NOW))
        .expect("the three-part chain replays");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "every part's signature, PUT and ETag were consumed"
    );
}

/// The create chain truncated at the first hop whose url carries `fragment`,
/// with that hop's recorded answer replaced. Hand-authored throughout: every
/// recorded hop answered 200, so no refusal shape comes off a capture.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn create_chain_answering(fragment: &str, response: HttpResponse) -> Cassette {
    let mut interactions = vec![create_render()];
    interactions.extend(staging_chain(usize::MAX));
    let at = interactions
        .iter()
        .position(|hop| hop.request.url.contains(fragment))
        .expect("the create chain issues that hop");
    interactions.truncate(at.saturating_add(1));
    if let Some(last) = interactions.last_mut() {
        last.response = response;
    }
    Cassette { interactions }
}

#[test]
fn every_signed_s3_call_reads_the_clock_so_a_long_upload_never_signs_a_stale_date() {
    // No capture covers a multi-part upload: the one recorded file went as a
    // single 224 KB part in seconds, which made one clock read for the whole
    // upload and one per call indistinguishable. A product the form accepts
    // up to four gibibytes of outlives SigV2's fifteen-minute skew window,
    // and every part after that window is rejected after its bytes have gone.
    let cassette = create_cassette(128, "/Product/test-17511712");
    let clocks: Vec<&str> = cassette
        .interactions
        .iter()
        .map(|hop| hop.request.url.as_str())
        .filter(|url| url.contains("/uploads/time"))
        .collect();
    assert_eq!(
        clocks.len(),
        5,
        "one clock read per signed call: the initiate, three parts and the completion, got \
         {clocks:?}"
    );
    assert_eq!(
        clocks.iter().collect::<BTreeSet<_>>().len(),
        clocks.len(),
        "two reads sharing a url share a cache entry, and a cached instant is the stale one, \
         got {clocks:?}"
    );
    let stamps: Vec<&str> = cassette
        .interactions
        .iter()
        .filter(|hop| hop.request.url.contains("/uploads/sign_auth"))
        .filter_map(|hop| hop.request.url.split("&datetime=").nth(1))
        .collect();
    assert_eq!(
        stamps.iter().collect::<BTreeSet<_>>().len(),
        5,
        "each signature is minted under the instant its own clock read returned, got {stamps:?}"
    );
    let adapter = adapter(cassette).with_part_size(128);
    futures::executor::block_on(adapter.submit(key(), fields(), NOW))
        .expect("the three-part chain replays");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "and the flow issued every clock read the chain records"
    );
}

#[test]
fn a_lost_response_before_the_final_post_is_safe_to_retry_rather_than_a_halt() {
    // The form render is a GET, the reservation and the S3 calls write only
    // to the seller's own staging area, and neither creates a product. A
    // response lost on one of them leaves at worst an orphaned staged object;
    // calling it an ambiguity would halt an entire inventory over a timeout
    // on a hop that created nothing.
    for lose_at in [0_usize, 1, 4] {
        let refused = futures::executor::block_on(
            losing(
                create_cassette(usize::MAX, "/Product/test-17511712"),
                lose_at,
            )
            .submit(key(), fields(), NOW),
        );
        assert!(
            matches!(refused, Err(AdapterError::NotSent(_))),
            "hop {lose_at} created nothing, so its lost answer is safe to retry, got {refused:?}"
        );
    }
}

#[test]
fn a_lost_response_on_the_final_post_stays_an_ambiguity() {
    let cassette = create_cassette(usize::MAX, "/Product/test-17511712");
    let last = cassette
        .interactions
        .len()
        .checked_sub(1)
        .expect("the chain ends with the form post");
    let refused = futures::executor::block_on(losing(cassette, last).submit(key(), fields(), NOW));
    assert_eq!(
        refused,
        Err(AdapterError::Ambiguous(AmbiguityCause::ResponseEventLost)),
        "the final POST is the one hop that may have created the record it could not report"
    );
}

#[test]
fn a_session_that_lapses_on_the_clock_read_parks_rather_than_refusing_the_item() {
    // Hand-authored. The token set ages across the upload — nearly three
    // minutes in the capture — so a session expiring mid-upload is the
    // ordinary shape, and a terminal rejection there discards an item that
    // only needs re-auth.
    let refused = futures::executor::block_on(
        adapter(create_chain_answering("/uploads/time", text(401, ""))).submit(
            key(),
            fields(),
            NOW,
        ),
    );
    assert_eq!(
        refused,
        Err(AdapterError::SessionExpired),
        "every sibling hop parks a 401 for re-auth, and the plain-text hops are no different"
    );
}

#[test]
fn a_rate_limited_signing_oracle_backs_off_rather_than_refusing_the_item() {
    // Hand-authored. The oracle is asked once per signed S3 call, so a
    // multi-part upload is exactly what would draw a 429 out of it.
    let refused = futures::executor::block_on(
        adapter(create_chain_answering("/uploads/sign_auth", text(429, ""))).submit(
            key(),
            fields(),
            NOW,
        ),
    );
    assert_eq!(
        refused,
        Err(AdapterError::RateLimited { retry_after: None }),
        "a rate limit is a rate limit whichever hop reports it"
    );
}

#[test]
fn a_plain_text_hop_answering_markup_two_hundred_is_read_as_the_bounce_it_is() {
    // Hand-authored. `/uploads/time` answers an RFC 1123 date and nothing
    // else, so a page in its place was answered by the front door.
    let refused = futures::executor::block_on(
        adapter(create_chain_answering(
            "/uploads/time",
            text(200, "<html><body><h1>Sign In</h1></body></html>"),
        ))
        .submit(key(), fields(), NOW),
    );
    assert_eq!(
        refused,
        Err(AdapterError::SessionExpired),
        "a sign-in page answered 200 is not a clock reading, whatever the status says"
    );
}

#[test]
fn a_render_the_scrape_cannot_read_is_reported_as_drift_by_the_preflight() {
    // Hand-authored. Only the drift report reaches the machine's drift path:
    // the driver treats every other preflight error as a transient and
    // abandons the run, so a render whose token input moved would be retried
    // on the next schedule instead of halting the inventory for a look.
    let mut render = create_render();
    let body = render
        .response
        .text()
        .replace("name=\"data[_Token][key]\"", "name=\"other\"");
    render.response = HttpResponse::plain(200, body.into_bytes());
    let refused = futures::executor::block_on(
        adapter(Cassette {
            interactions: vec![render],
        })
        .assert_form_schema(FormId(Uuid([2; 16]))),
    );
    let Err(AdapterError::SchemaDrift(drift)) = refused else {
        panic!("a render the scrape cannot read is drift, not a transient, got {refused:?}");
    };
    assert!(
        drift
            .removed
            .iter()
            .any(|entry| entry.contains("data[_Token][key]")),
        "the report names the anchor that moved, got {:?}",
        drift.removed
    );
}

#[test]
fn the_publish_edit_moves_the_status_selector_and_echoes_the_existing_thumbnails() {
    let render = edit_render();
    let page = tam_marketplace_tpt::form::scrape_form_page(&render.response.text())
        .expect("the committed edit render parses");
    let listing =
        write_model::listing_from_field_set(&fields()).expect("the projection parses back");
    let authorship = attested();
    let body = write_model::edit_fields(&write_model::EditSubmission {
        tokens: page.tokens(),
        listing: &listing,
        thumbs: page.thumbs(),
        status: write_model::StatusUser::Live,
        authorship: &authorship,
    });
    let value = |name: &str| {
        body.iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(
        value("data[Item][status_user]").as_deref(),
        Some("1"),
        "publishing is the edit form with the status selector moved"
    );
    assert_eq!(
        value("data[ItemDigital][thumb1]").as_deref(),
        Some("THUMBPLACEHOLDER1/aa+bb="),
        "an edit that dropped the handle would drop the product's thumbnail"
    );
    let target = FormTarget::EditDigital(ProductId(PRODUCT_ID));
    let cassette = Cassette {
        interactions: vec![
            render,
            Interaction {
                request: endpoints::submit_form_request(target, body),
                response: with_header(
                    302,
                    "",
                    ResponseHeader::Location,
                    "/Product/Fractions-pack-17511712",
                ),
            },
        ],
    };
    let adapter = adapter(cassette);
    let landing = futures::executor::block_on(adapter.publish(ProductId(PRODUCT_ID), &fields()))
        .expect("the recorded publish replays");
    assert_eq!(
        landing.product,
        ProductId(PRODUCT_ID),
        "the edit route names the product it edits, and the redirect must agree"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "two hops, both consumed"
    );
}

#[test]
fn a_publish_that_redirects_to_another_product_is_an_ambiguity() {
    let render = edit_render();
    let page = tam_marketplace_tpt::form::scrape_form_page(&render.response.text())
        .expect("the render parses");
    let listing = write_model::listing_from_field_set(&fields()).expect("it parses back");
    let authorship = attested();
    let target = FormTarget::EditDigital(ProductId(PRODUCT_ID));
    let body = write_model::edit_fields(&write_model::EditSubmission {
        tokens: page.tokens(),
        listing: &listing,
        thumbs: page.thumbs(),
        status: write_model::StatusUser::Live,
        authorship: &authorship,
    });
    let cassette = Cassette {
        interactions: vec![
            render,
            Interaction {
                request: endpoints::submit_form_request(target, body),
                response: with_header(302, "", ResponseHeader::Location, "/Product/other-9"),
            },
        ],
    };
    let refused =
        futures::executor::block_on(adapter(cassette).publish(ProductId(PRODUCT_ID), &fields()));
    assert_eq!(
        refused,
        Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier)),
        "a redirect naming a product we did not edit is not something this flow can reconcile"
    );
}

#[test]
fn the_structural_probe_reads_the_form_and_creates_nothing() {
    let cassette = Cassette {
        interactions: vec![create_render()],
    };
    let probe = adapter(cassette);
    let fingerprint = futures::executor::block_on(probe.assert_form_schema(FormId(Uuid([2; 16]))))
        .expect("the render declares every field this adapter writes");
    let again = adapter(Cassette {
        interactions: vec![create_render()],
    });
    let repeat = futures::executor::block_on(again.assert_form_schema(FormId(Uuid([2; 16]))))
        .expect("the same render fingerprints the same way");
    assert_eq!(
        fingerprint, repeat,
        "a fingerprint that moved without the form moving would cry drift every hour"
    );
    assert_eq!(
        probe.transport().remaining(),
        0,
        "the probe is one GET; it creates, submits and deletes nothing"
    );
}

#[test]
fn a_render_that_stops_declaring_a_field_this_adapter_writes_is_drift() {
    // Hand-authored: no capture contains a drifted render. The unlocked list
    // is the form's own statement of its shape, so dropping one entry from it
    // is exactly what an upstream field removal would look like.
    let mut render = create_render();
    let body = render
        .response
        .text()
        .replace("ItemsProperty.copyright_declaration%7C", "");
    render.response = HttpResponse::plain(200, body.into_bytes());
    let refused = futures::executor::block_on(
        adapter(Cassette {
            interactions: vec![render],
        })
        .assert_form_schema(FormId(Uuid([2; 16]))),
    );
    let Err(AdapterError::SchemaDrift(drift)) = refused else {
        panic!("a field we write that the form no longer declares is drift, got {refused:?}");
    };
    assert_eq!(
        drift.removed,
        vec!["ItemsProperty.copyright_declaration".to_owned()],
        "the report names the field that moved, not merely that something did"
    );
    assert!(
        drift.added.is_empty(),
        "the unlocked list is a superset by construction, so extra names are not drift"
    );
}

#[test]
fn a_submit_without_an_authorship_attestation_never_reaches_the_network() {
    let unattested = TptAdapter::new(
        CassetteTransport::new(Cassette {
            interactions: vec![],
        }),
        OneFile(file()),
        InstantPause,
    );
    let refused = futures::executor::block_on(unattested.submit(key(), fields(), NOW));
    let Err(AdapterError::Rejected { code, detail }) = refused else {
        panic!("the copyright declaration is the seller's, got {refused:?}");
    };
    assert_eq!(code, FailureCode::UploadRejected);
    assert!(
        detail.0.contains("authorship attestation"),
        "the refusal names what is missing, got {detail:?}"
    );
    assert_eq!(
        unattested.transport().remaining(),
        0,
        "and nothing was sent while it was missing"
    );
}

#[test]
fn a_projection_that_is_not_exactly_one_file_is_refused_before_the_form_is_rendered() {
    let mut many = fields();
    many.files.push(FileId(Uuid([2; 16])));
    let refused = futures::executor::block_on(
        adapter(Cassette {
            interactions: vec![],
        })
        .submit(key(), many, NOW),
    );
    let Err(AdapterError::Rejected { detail, .. }) = refused else {
        panic!("only the product slot is captured, got {refused:?}");
    };
    assert!(
        detail.0.contains("uncaptured"),
        "the refusal says the other slots are unproven rather than unsupported, got {detail:?}"
    );
    let mut none = fields();
    none.files.clear();
    assert!(
        futures::executor::block_on(
            adapter(Cassette {
                interactions: vec![]
            })
            .submit(key(), none, NOW)
        )
        .is_err(),
        "a create with no file has nothing to put in the product slot"
    );
}

#[test]
fn a_sign_in_interstitial_on_the_form_render_is_an_expired_session() {
    // Hand-authored. Every recorded request in both HARs succeeded, so no
    // capture contains a sign-in interstitial; this is the shape a lapsed
    // cookie jar produces on a page TPT gates.
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::form_page_request(FormTarget::CreateDigital),
            response: text(
                200,
                "<html><body><h1>Sign In</h1><a href=\"/sign-in\">continue</a></body></html>",
            ),
        }],
    };
    let refused = futures::executor::block_on(adapter(cassette).submit(key(), fields(), NOW));
    assert_eq!(
        refused,
        Err(AdapterError::SessionExpired),
        "a page that is not the form is not schema drift when it says why"
    );
}

#[test]
fn a_cloudflare_managed_challenge_on_the_form_render_is_a_challenge() {
    // Hand-authored. cf_clearance was carried into both captures from an
    // earlier session and never minted inside one, so no recording contains
    // a challenge; this is Cloudflare's own interstitial shape.
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::form_page_request(FormTarget::CreateDigital),
            response: text(
                403,
                "<html><head><title>Just a moment...</title></head><body>\
                 <script src=\"/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1\">\
                 </script></body></html>",
            ),
        }],
    };
    let refused = futures::executor::block_on(adapter(cassette).submit(key(), fields(), NOW));
    assert_eq!(
        refused,
        Err(AdapterError::Challenge(
            ChallengeKind::JavaScriptInterstitial
        )),
        "a challenge is parked and answered, never retried as a transport fault"
    );
}

#[test]
fn a_queue_job_that_reports_an_error_refuses_and_says_nothing_was_created() {
    // Hand-authored. No failure terminal was captured — every recorded poll
    // ended at status 2 with error null — so this is the documented error
    // member populated, which is the only failure shape the payload allows.
    let mut interactions = vec![create_render()];
    let staging = staging_chain(usize::MAX);
    let upto = staging
        .iter()
        .position(|hop| hop.request.url.contains("/queue/results"))
        .expect("the chain polls the queue");
    interactions.extend(staging.into_iter().take(upto));
    interactions.push(poll(
        PROCESS_JOB,
        1,
        0,
        r#"{"status":2,"data":{"error":"conversion failed","key":null}}"#,
    ));
    let adapter = adapter(Cassette { interactions });
    let refused = futures::executor::block_on(adapter.submit(key(), fields(), NOW));
    let Err(AdapterError::Rejected { code, detail }) = refused else {
        panic!("a failed processing job is a refusal, got {refused:?}");
    };
    assert_eq!(code, FailureCode::UploadRejected);
    assert!(
        detail.0.contains("conversion failed"),
        "the refusal names what the queue said, got {detail:?}"
    );
}

#[test]
fn a_final_post_answered_two_hundred_with_the_form_again_is_ambiguous() {
    // Hand-authored. No capture contains a refused submit, so the shape of
    // one is unknown; a blackholed CakePHP form re-renders the page. Reading
    // that as a clean rejection would be inventing evidence that nothing was
    // created, which is exactly what this must not do.
    let mut cassette = create_cassette(usize::MAX, "/Product/test-17511712");
    let last = cassette
        .interactions
        .len()
        .checked_sub(1)
        .expect("the chain ends with the submit");
    if let Some(submit) = cassette.interactions.get_mut(last) {
        submit.response = text(
            200,
            "<html><body><form id=\"ItemAddForm\" action=\"/My-Products/New/Digital-Next\">\
             </form></body></html>",
        );
    }
    let refused = futures::executor::block_on(adapter(cassette).submit(key(), fields(), NOW));
    assert_eq!(
        refused,
        Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier)),
        "the write may have landed, so it is reconciled or halted and never retried"
    );
}

#[test]
fn a_final_post_that_redirects_without_a_product_id_is_also_ambiguous() {
    let mut cassette = create_cassette(usize::MAX, "/My-Products");
    let _ = &mut cassette;
    let refused = futures::executor::block_on(adapter(cassette).submit(key(), fields(), NOW));
    assert_eq!(
        refused,
        Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier)),
        "a 302 whose Location names no product yields no durable identifier"
    );
}

#[test]
fn a_read_back_addresses_one_product_and_reports_what_the_wire_said() {
    let product = ProductId(PRODUCT_ID);
    let row = serde_json::json!({
        "id": PRODUCT_ID.to_string(),
        "name": "Fractions pack",
        "canonicalSlug": "Fractions-pack-17511712",
        "price": "$0.00",
        "isFree": true,
        "status": "ACTIVE",
        "taxonomyTags": [{"id": "math"}, {"id": "4th-grade"}],
        "categories": [{"id": 1_361_944, "name": "Complete Topic"}],
    });
    let body = serde_json::json!({"data": {"seller": {"resources": {
        "results": [row],
        "pageInfo": {"totalResultsCount": 1, "currentPage": 1, "totalPageCount": 1},
    }}}});
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::product_by_id_request(product),
            response: text(200, &body.to_string()),
        }],
    };
    let adapter = adapter(cassette);
    let observed = futures::executor::block_on(adapter.read_back(
        ListingLocator::Durable(RemoteListingId::Tpt {
            product_id: PRODUCT_ID,
        }),
        FetchReason::VerifyAttempt {
            attempt: WriteAttemptId(Uuid([6; 16])),
        },
        Timestamp(1_787_896_700_000),
    ))
    .expect("the recorded single-product read replays");
    assert_eq!(
        observed.id,
        RemoteListingId::Tpt {
            product_id: PRODUCT_ID
        },
        "the read is addressed by the durable identifier a receipt already holds"
    );
    assert_eq!(
        observed
            .fields
            .iter()
            .find(|(key, _)| *key == FieldKey::Title)
            .map(|(_, value)| value.as_str()),
        Some("Fractions pack"),
        "the title comes back for the diff report to compare"
    );
    assert_eq!(
        observed.lifecycle,
        RemoteLifecycle::Live {
            since: Timestamp(1_787_896_700_000)
        },
        "ACTIVE is the only live state on file, and the instant is the reader's own"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "one product, one request"
    );
}

/// `MyProductListings` is the only witness TPT offers, and its read lags a
/// write by up to twenty seconds. Answering `Ambiguous` for a page that
/// simply does not carry the product halted the tenant's inventory on the
/// read-back that immediately follows every create.
#[test]
fn a_product_absent_from_the_catalogue_reads_back_absent() {
    let product = ProductId(PRODUCT_ID);
    let body = serde_json::json!({"data": {"seller": {"resources": {
        "results": [],
        "pageInfo": {"totalResultsCount": 0, "currentPage": 1, "totalPageCount": 1},
    }}}});
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::product_by_id_request(product),
            response: text(200, &body.to_string()),
        }],
    };
    let adapter = adapter(cassette);
    let observed = futures::executor::block_on(adapter.read_back(
        ListingLocator::Durable(RemoteListingId::Tpt {
            product_id: PRODUCT_ID,
        }),
        FetchReason::VerifyAttempt {
            attempt: WriteAttemptId(Uuid([6; 16])),
        },
        Timestamp(1_787_896_700_000),
    ))
    .expect("a product the catalogue does not carry is an observation, not an ambiguity");
    assert_eq!(
        observed.lifecycle,
        RemoteLifecycle::Absent,
        "a page that parsed and named no such product says the product is not there"
    );
    assert_eq!(
        observed.id,
        RemoteListingId::Tpt {
            product_id: PRODUCT_ID
        },
        "the observation names the product that was asked about"
    );
    assert!(
        observed.fields.is_empty(),
        "an absent product carries no field values to diff"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "one product, one request, absent or not"
    );
}

#[test]
fn a_read_back_under_the_probe_capability_is_refused() {
    let adapter = adapter(Cassette {
        interactions: vec![],
    });
    let refused = futures::executor::block_on(adapter.read_back(
        ListingLocator::Durable(RemoteListingId::Tpt {
            product_id: PRODUCT_ID,
        }),
        FetchReason::StructuralProbe {
            grant: tam_marketplace::CanaryGrant {
                inventory: InventoryId::Tpt,
                decided_at: Timestamp(1),
            },
        },
        Timestamp(2),
    ));
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "the probe resolves selectors against a form and reads no listing, got {refused:?}"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "and it issues no request at all"
    );
}

#[test]
fn a_signing_string_naming_another_object_never_becomes_a_request() {
    let mine = ticket();
    let theirs = UploadTicket::new(
        BUCKET.to_owned(),
        "ffff-99999999-2026-08-28/product/someone-elses.png".to_owned(),
    );
    let signable = s3::string_to_sign(&theirs, &S3Operation::Initiate, None, "image/png", AMZ_DATE);
    assert!(
        endpoints::sign_auth_request(&mine, &signable, AMZ_DATE).is_err(),
        "the oracle signs whatever it is given, so the scope check is ours to make"
    );
}

fn revise_plan(to: ListingState) -> RevisePlan {
    RevisePlan {
        subject: RemoteListingId::Tpt {
            product_id: PRODUCT_ID,
        },
        fields: fields(),
        transition: LifecycleTransition {
            // Unused on TPT: the edit form is a full replace whichever side of
            // the draft line the product is on.
            from: ListingState::Draft,
            to,
        },
    }
}

/// The trait method delegates to the edit render rather than re-deriving one:
/// the cassette's submit interaction is built from the committed render's own
/// tokens and thumbnail handles, so a `revise` wired to a fresh create render
/// would not match it. The body's own content — the status selector and the
/// echoed handles — is pinned by
/// `the_publish_edit_moves_the_status_selector_and_echoes_the_existing_thumbnails`,
/// which this inherits its severity from.
#[test]
fn revise_moves_the_status_selector_and_echoes_the_existing_thumbnails() {
    for (label, to, status) in [
        (
            "a publish",
            ListingState::Live,
            write_model::StatusUser::Live,
        ),
        (
            "an edit that stays a draft",
            ListingState::Draft,
            write_model::StatusUser::Draft,
        ),
    ] {
        let target = FormTarget::EditDigital(ProductId(PRODUCT_ID));
        let cassette = Cassette {
            interactions: vec![
                edit_render(),
                Interaction {
                    request: endpoints::submit_form_request(target, edit_body(&fields(), status)),
                    response: with_header(
                        302,
                        "",
                        ResponseHeader::Location,
                        "/Product/Fractions-pack-17511712",
                    ),
                },
            ],
        };
        let adapter = adapter(cassette);
        let evidence = futures::executor::block_on(adapter.revise(revise_plan(to), NOW))
            .unwrap_or_else(|error| panic!("{label}: the recorded edit replays: {error:?}"));
        assert_eq!(
            evidence.landed,
            Some(RemoteListingId::Tpt {
                product_id: PRODUCT_ID
            }),
            "{label}: the landing names the product the redirect agreed on"
        );
        assert_eq!(
            evidence.landed_on_route.as_deref(),
            Some("/Product/Fractions-pack-17511712"),
            "{label}: the evidence names where the edit landed"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: the render and the submit, and no read-back inside the call"
        );
    }
}

/// The six fields a TPT edit blanks. A created product carries none of them,
/// so the blanking is a no-op on every listing Phase 3 can revise; this fails
/// the day either side starts carrying a value, which is the day adopting a
/// listing this system did not create makes the blanking live.
#[test]
fn an_edit_reposts_exactly_what_a_create_posts_for_the_fields_it_does_not_carry() {
    let page = tam_marketplace_tpt::form::scrape_form_page(&create_render().response.text())
        .expect("the committed create render parses");
    let listing =
        write_model::listing_from_field_set(&fields()).expect("the projection parses back");
    let authorship = attested();
    let created = write_model::create_fields(&write_model::CreateSubmission {
        tokens: page.tokens(),
        listing: &listing,
        product: &ProcessedHandle::new(PROCESSED_HANDLE.to_owned()),
        thumbs_collection_key: COLLECTION_KEY,
        authorship: &authorship,
    });
    let edited = edit_body(&fields(), write_model::StatusUser::Draft);
    let value = |body: &[(String, String)], name: &str| {
        body.iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.clone())
    };
    for name in [
        "data[ItemsProperty][pages]",
        "data[ItemsLocalization][common_core_id]",
        "data[ItemsLocalization][country_id_flag]",
        "data[ItemsProperty][duration]",
        "data[ItemsProperty][answer_key]",
    ] {
        assert_eq!(
            value(&edited, name),
            value(&created, name),
            "an edit reposts {name} exactly as the create posted it, so a revise of a \
             product this system created loses nothing"
        );
    }
    assert_eq!(
        value(&created, "data[ItemTaxCode][tax_code_id]"),
        None,
        "the create posts no tax code at all"
    );
    assert_eq!(
        value(&edited, "data[ItemTaxCode][tax_code_id]").as_deref(),
        Some(""),
        "and the edit posts an empty one, so a created product has none to lose"
    );
}

#[test]
fn a_removal_posts_remove_resource_and_accepts_only_its_own_id_back() {
    let product = ProductId(PRODUCT_ID);
    let plan = |subject| RemovalPlan {
        attempt: WriteAttemptId(Uuid([0x61; 16])),
        subject,
        // Unused on TPT: `RemoveResource` takes a product id and nothing else.
        state: ListingState::Live,
    };
    let removing = adapter(delete_cassette(product, delete_answer(PRODUCT_ID)));
    let evidence = futures::executor::block_on(removing.remove(
        plan(RemoteListingId::Tpt {
            product_id: PRODUCT_ID,
        }),
        NOW,
    ))
    .expect("the recorded delete replays through the trait");
    assert_eq!(
        evidence.landed,
        Some(RemoteListingId::Tpt {
            product_id: PRODUCT_ID
        }),
        "the removal states the listing whose disappearance the driver will poll for"
    );
    let answer: [u8; 32] = Sha256::digest(delete_answer(PRODUCT_ID).body).into();
    assert_eq!(
        evidence.response_body_digest,
        Some(ContentHash(answer)),
        "the digest is taken over the raw answer the mutation returned, which is what \
         makes the write evidence checkable against the recording"
    );
    assert_eq!(
        removing.transport().remaining(),
        0,
        "one mutation and nothing else: no render, no read-back inside the call"
    );

    let elsewhere =
        futures::executor::block_on(adapter(delete_cassette(product, delete_answer(9))).remove(
            plan(RemoteListingId::Tpt {
                product_id: PRODUCT_ID,
            }),
            NOW,
        ));
    assert_eq!(
        elsewhere,
        Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier)),
        "an echo naming a product we did not delete settles nothing about this one"
    );
}

/// The `RemoveResource` answer, whose only payload is the id it removed.
fn delete_answer(id: u64) -> HttpResponse {
    text(
        200,
        &serde_json::json!({
            "data": {"resourceDelete": {"__typename": "Product", "id": id.to_string()}}
        })
        .to_string(),
    )
}

fn delete_cassette(asked: ProductId, answer: HttpResponse) -> Cassette {
    Cassette {
        interactions: vec![Interaction {
            request: endpoints::remove_resource_request(asked),
            response: answer,
        }],
    }
}

#[test]
fn a_delete_posts_the_captured_mutation_and_settles_on_the_echoed_id() {
    let product = ProductId(PRODUCT_ID);
    let adapter = adapter(delete_cassette(product, delete_answer(PRODUCT_ID)));
    futures::executor::block_on(adapter.delete(product)).expect("the recorded delete replays");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "a delete is one mutation and nothing else: no render, no read-back inside the call"
    );
}

#[test]
fn a_delete_whose_answer_names_another_product_or_none_is_an_ambiguity() {
    // Hand-authored. The captured delete echoed the id it was given, so
    // neither shape below comes off a recording; both are answers this flow
    // must not read as a confirmation.
    let product = ProductId(PRODUCT_ID);
    let elsewhere = futures::executor::block_on(
        adapter(delete_cassette(product, delete_answer(9))).delete(product),
    );
    assert_eq!(
        elsewhere,
        Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier)),
        "an echo naming a product we did not delete settles nothing about this one"
    );
    let silent = futures::executor::block_on(
        adapter(delete_cassette(
            product,
            text(200, r#"{"data":{"resourceDelete":null}}"#),
        ))
        .delete(product),
    );
    assert_eq!(
        silent,
        Err(AdapterError::Ambiguous(
            AmbiguityCause::ReadBackIndeterminate
        )),
        "the mutation was posted, so an unreadable answer leaves the record's fate unknown \
         rather than proving it survives"
    );
}

/// The same projection with one description in place of another, which is
/// what an edit that changes a field looks like from the seam.
fn described(body: &str) -> FieldSet {
    let mut fields = fields();
    for (key, value) in &mut fields.entries {
        if *key == FieldKey::Description {
            body.clone_into(value);
        }
    }
    fields
}

/// The edit body one field set produces against the committed edit render.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn edit_body(fields: &FieldSet, status: write_model::StatusUser) -> Vec<(String, String)> {
    let page = tam_marketplace_tpt::form::scrape_form_page(&edit_render().response.text())
        .expect("the committed edit render parses");
    let listing = write_model::listing_from_field_set(fields).expect("the projection parses back");
    write_model::edit_fields(&write_model::EditSubmission {
        tokens: page.tokens(),
        listing: &listing,
        thumbs: page.thumbs(),
        status,
        authorship: &attested(),
    })
}

#[test]
fn an_update_rewrites_the_description_and_leaves_the_product_where_it_was() {
    let rewritten = described("<p>a second draft</p>");
    let body = edit_body(&rewritten, write_model::StatusUser::Draft);
    let value = |name: &str| {
        body.iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(
        value("data[Item][description]").as_deref(),
        Some("<p>a second draft</p>"),
        "the edit carries the new body, which is the only thing this update changes"
    );
    assert_eq!(
        value("data[Item][status_user]").as_deref(),
        Some("0"),
        "an update that is not a publish leaves the draft a draft"
    );
    assert_eq!(
        value("data[Item][free]").as_deref(),
        Some("1"),
        "a free listing's edit posts the flag; the live form refuses the body without it"
    );
    let unchanged = edit_body(&fields(), write_model::StatusUser::Draft);
    let moved: Vec<&str> = body
        .iter()
        .zip(unchanged.iter())
        .filter(|(after, before)| after != before)
        .map(|(after, _)| after.0.as_str())
        .collect();
    assert_eq!(
        (moved, body.len()),
        (vec!["data[Item][description]"], unchanged.len()),
        "an edit is a full replace, so every other name is posted back exactly as it stood"
    );
    let target = FormTarget::EditDigital(ProductId(PRODUCT_ID));
    let adapter = adapter(Cassette {
        interactions: vec![
            edit_render(),
            Interaction {
                request: endpoints::submit_form_request(target, body),
                response: with_header(
                    302,
                    "",
                    ResponseHeader::Location,
                    "/Product/Fractions-pack-17511712",
                ),
            },
        ],
    });
    let landing = futures::executor::block_on(adapter.update(
        ProductId(PRODUCT_ID),
        &rewritten,
        write_model::StatusUser::Draft,
    ))
    .expect("the recorded edit replays");
    assert_eq!(
        landing.product,
        ProductId(PRODUCT_ID),
        "the edit route names the product it edits, and the redirect must agree"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the render and the post, both consumed"
    );
}

/// The regression the live run of 2026-08-29 exposed: a refused edit answers
/// 302 to the very route it was posted to, and that route ends in the product
/// id being edited, so the landing check alone agreed with it.
#[test]
fn an_edit_bounced_back_to_its_own_form_is_not_reported_as_a_landing() {
    let rewritten = described("<p>a second draft</p>");
    let body = edit_body(&rewritten, write_model::StatusUser::Draft);
    let target = FormTarget::EditDigital(ProductId(PRODUCT_ID));
    let adapter = adapter(Cassette {
        interactions: vec![
            edit_render(),
            Interaction {
                request: endpoints::submit_form_request(target, body),
                response: with_header(
                    302,
                    "",
                    ResponseHeader::Location,
                    "/itemsDigital/editNext/17511712",
                ),
            },
        ],
    });
    let outcome = futures::executor::block_on(adapter.update(
        ProductId(PRODUCT_ID),
        &rewritten,
        write_model::StatusUser::Draft,
    ));
    assert!(
        matches!(
            outcome,
            Err(AdapterError::Rejected {
                code: FailureCode::SubmitNoConfirmation,
                ..
            })
        ),
        "a redirect back to the edit form is the form refusing the body, got {outcome:?}"
    );
}

#[test]
fn a_paid_create_posts_the_price_the_free_create_zeroed() {
    let paid = tam_types::PriceIntent::Paid(
        tam_types::Money::new(300, tam_types::Currency::Usd).expect("a positive amount is money"),
    );
    let projection = ProjectedListing {
        price: paid,
        ..projected()
    };
    let fields = write_model::project_fields(&projection).expect("a paid listing projects");
    let listing = write_model::listing_from_field_set(&fields).expect("and parses back");
    let page = tam_marketplace_tpt::form::scrape_form_page(&create_render().response.text())
        .expect("the committed render parses");
    let authorship = attested();
    let body = write_model::create_fields(&write_model::CreateSubmission {
        tokens: page.tokens(),
        listing: &listing,
        product: &ProcessedHandle::new(PROCESSED_HANDLE.to_owned()),
        thumbs_collection_key: COLLECTION_KEY,
        authorship: &authorship,
    });
    let value = |name: &str| {
        body.iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(
        (
            value("data[Item][free]"),
            value("data[Item][price]"),
            value("data[Item][license_price]"),
            value("data[Item][status_user]"),
        ),
        (Some("0"), Some("3.00"), Some("2.70"), Some("0")),
        "the paid amounts are the captured edit's, and a paid create still lands a draft"
    );
}

/// The synthetic store the read fixture carries. Declared here so the canary
/// below can assert the fixture's store identity without ever holding the
/// live one it replaced.
const SYNTHETIC_STORE_ID: &str = "90000001";
const SYNTHETIC_STORE_NAME: &str = "Sample Teaching Studio";
const SYNTHETIC_STORE_URL: &str = "/store/sample-teaching-studio";

/// Every `AKIA`-prefixed access key id the text carries: `AKIA` and sixteen
/// uppercase alphanumerics. Scanned as a shape, because a canary that names
/// the value it forbids commits exactly what it forbids.
fn access_key_ids(body: &str) -> Vec<&str> {
    body.match_indices("AKIA")
        .filter_map(|(at, _)| body.get(at..at.saturating_add(20)))
        .filter(|token| {
            token
                .bytes()
                .skip(4)
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
        .collect()
}

/// Every run of hexadecimal characters at least `least` long. The committed
/// token placeholders are one character repeated; a live CakePHP `_Token`
/// hash or CSRF value is not, and that difference is visible without this
/// file holding either.
fn hex_runs(body: &str, least: usize) -> Vec<String> {
    let mut runs: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in body.chars() {
        if character.is_ascii_hexdigit() {
            current.push(character);
            continue;
        }
        if current.chars().count() >= least {
            runs.push(current.clone());
        }
        current.clear();
    }
    if current.chars().count() >= least {
        runs.push(current);
    }
    runs
}

/// Every object of one GraphQL type a recorded response body carries, wherever
/// it sits in the envelope.
fn typed_blocks<'a>(value: &'a Value, typename: &str) -> Vec<&'a Value> {
    let mut found: Vec<&Value> = Vec::new();
    let mut stack: Vec<&Value> = vec![value];
    while let Some(node) = stack.pop() {
        match *node {
            Value::Object(ref map) => {
                if map.get("__typename").and_then(Value::as_str) == Some(typename) {
                    found.push(node);
                }
                stack.extend(map.values());
            }
            Value::Array(ref items) => stack.extend(items.iter()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    found
}

/// Every value the committed analytics fixture reports. A sales count has no
/// shape that tells a live figure from a synthetic one, so the guard is an
/// allow-list of the numbers this repository deliberately made up: a fixture
/// re-mined from a fresh capture fails until its totals are synthesised too.
const SYNTHETIC_TOTALS: [i64; 3] = [111, 0, 222];

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn recorded_bodies(fixture: &str) -> Vec<String> {
    let cassette: Cassette = serde_json::from_str(fixture).expect("a committed fixture parses");
    cassette
        .interactions
        .into_iter()
        .map(|hop| hop.response.text().into_owned())
        .collect()
}

/// Every fixture this crate commits, read and write halves alike.
const FIXTURES: [(&str, &str); 4] = [
    (
        "create_form_page.json",
        include_str!("cassettes/create_form_page.json"),
    ),
    (
        "edit_form_page.json",
        include_str!("cassettes/edit_form_page.json"),
    ),
    (
        "my_product_listings.json",
        include_str!("cassettes/my_product_listings.json"),
    ),
    (
        "all_time_stats.json",
        include_str!("cassettes/all_time_stats.json"),
    ),
];

#[test]
fn no_committed_fixture_carries_a_live_token_or_key_id() {
    for (name, body) in FIXTURES {
        for token in access_key_ids(body) {
            assert_eq!(
                token, AWS_KEY_ID,
                "{name} carries an access key id that is not the placeholder; the captured one \
                 is a real IAM key identifier belonging to TPT"
            );
        }
        for run in hex_runs(body, 32) {
            let first = run.chars().next();
            assert!(
                run.chars().all(|character| Some(character) == first),
                "{name} carries a {}-character hex run that is not one character repeated, \
                 which is the shape of a live token rather than of a placeholder (the value is \
                 deliberately not printed)",
                run.chars().count()
            );
        }
    }
}

#[test]
fn no_committed_fixture_carries_a_live_analytics_total() {
    let mut seen = 0_usize;
    for (name, fixture) in FIXTURES {
        for body in recorded_bodies(fixture) {
            let Ok(parsed) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            for stat in typed_blocks(&parsed, "StoreResourceStats") {
                seen = seen.saturating_add(1);
                let total = stat.get("totalValue").and_then(Value::as_i64);
                assert!(
                    total.is_some_and(|value| SYNTHETIC_TOTALS.contains(&value)),
                    "{name} reports a per-resource total this repository did not synthesise; \
                     what a seller's resources earned or sold is theirs"
                );
            }
        }
    }
    assert!(
        seen > 0,
        "the analytics fixture carries a total per resource, and finding none means the walk \
         stopped looking rather than that the fixtures are clean"
    );
}

#[test]
fn no_committed_fixture_carries_a_live_store_identity() {
    let mut seen = 0_usize;
    for (name, fixture) in FIXTURES {
        for body in recorded_bodies(fixture) {
            let Ok(parsed) = serde_json::from_str::<Value>(&body) else {
                continue;
            };
            for store in typed_blocks(&parsed, "Store") {
                seen = seen.saturating_add(1);
                let field = |key: &str| store.get(key).and_then(Value::as_str).map(str::to_owned);
                assert_eq!(
                    field("id").as_deref(),
                    Some(SYNTHETIC_STORE_ID),
                    "{name} names a store this fixture did not synthesise"
                );
                assert_eq!(
                    field("name").as_deref(),
                    Some(SYNTHETIC_STORE_NAME),
                    "{name} names a store this fixture did not synthesise"
                );
                assert_eq!(
                    field("url").as_deref(),
                    Some(SYNTHETIC_STORE_URL),
                    "{name} names a store this fixture did not synthesise"
                );
            }
        }
    }
    assert!(
        seen > 0,
        "the catalogue fixture carries a store on every row, and finding none means the walk \
         stopped looking rather than that the fixtures are clean"
    );
}
