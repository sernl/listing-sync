//! The TPT write path driven end to end against cassettes.
//!
//! Two committed fixtures carry the only recorded shape a write depends on:
//! `create_form_page.json` and `edit_form_page.json` are renders of the two
//! product forms, reduced to what the scrape reads. Every token in them is a
//! placeholder of the captured shape — no real `_Token` hash, no real CSRF
//! value, no real AWS access key id, no seller id and no asset handle is
//! committed anywhere in this repository.
//!
//! The remaining hops are built here from the endpoint builders rather than
//! hand-authored as JSON, which is what makes the placeholder signatures
//! consistent: the flow derives each `StringToSign` from the ticket and the
//! bytes, the recorded `sign_auth` answer is a placeholder, and the recorded
//! S3 request carries that same placeholder because the flow put it there.
//! Every negative fixture below is hand-authored and says so; no capture
//! contains a refusal of any kind.

use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestAuth, RequestBody, ResponseHeader,
};
use tam_marketplace::{
    AdapterError, AmbiguityCause, ChallengeKind, FetchReason, FieldSet, FileContent, FileSource,
    FileSourceError, FormId, IdempotencyKey, ListingLocator, MarketplaceAdapter, NativeTerm,
    ProjectedListing, RemoteLifecycle, RemoteListingId, WriteAttemptId,
};
use tam_marketplace_tpt::endpoints::{self, FormTarget, SignedS3Call, UploadReservation};
use tam_marketplace_tpt::s3::{self, S3Operation, S3Signature, UploadSlot, UploadTicket};
use tam_marketplace_tpt::upload::{cache_buster, Hop, ProcessedHandle, QueueJob, UploadHandle};
use tam_marketplace_tpt::write_model::{self, AuthorshipDeclaration};
use tam_marketplace_tpt::{InstantPause, ProductId, TptAdapter};
use tam_types::{FailureCode, FieldKey, FileId, InventoryId, OrgId, Timestamp, Uuid};

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

fn org() -> OrgId {
    OrgId(Uuid([9; 16]))
}

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
                native_id: Some("1361944".to_owned()),
                segments: vec!["Complete Topic".to_owned()],
            },
        ],
        grades: vec![NativeTerm {
            native_id: Some("4th-grade".to_owned()),
            segments: vec!["Grade 4".to_owned()],
        }],
        ages: None,
        files: vec![FileId(Uuid([1; 16]))],
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

/// One signing round trip: the request the flow will build for this
/// operation, and the placeholder signature the oracle answers with.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn sign(operation: &S3Operation, content_md5: Option<&str>) -> Interaction {
    let ticket = ticket();
    let signable = s3::string_to_sign(&ticket, operation, content_md5, "image/png", AMZ_DATE);
    Interaction {
        request: endpoints::sign_auth_request(&ticket, &signable, AMZ_DATE)
            .expect("this operation is in scope for its own ticket"),
        response: text(200, SIGNATURE),
    }
}

fn s3_call(operation: &S3Operation, content_md5: Option<&str>, body: RequestBody) -> HttpRequest {
    let ticket = ticket();
    let key_id = s3::AwsKeyId::new(AWS_KEY_ID.to_owned());
    let signature = S3Signature::new(SIGNATURE.to_owned());
    endpoints::s3_request(
        &SignedS3Call {
            ticket: &ticket,
            operation,
            key_id: &key_id,
            signature: &signature,
            amz_date: AMZ_DATE,
            content_type: "image/png",
            content_md5,
        },
        body,
    )
}

/// The whole out-of-band upload, from the key reservation to the two
/// handles the product form consumes.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn staging_chain(part_size: usize) -> Vec<Interaction> {
    let payload = file().bytes;
    let mut chain = vec![
        Interaction {
            request: endpoints::upload_file_request(&UploadReservation {
                slot: UploadSlot::Product,
                file_name: "fractions.png",
                size: payload.len(),
                last_modified_ms: NOW.0,
                item_id: None,
            }),
            response: text(
                200,
                &format!(
                    r#"{{"key":"{UPLOAD_HANDLE}","bucket":"{BUCKET}","path":"{OBJECT_PATH}"}}"#
                ),
            ),
        },
        Interaction {
            request: endpoints::time_request(NOW.0),
            response: text(200, AMZ_DATE),
        },
        sign(&S3Operation::Initiate, None),
        Interaction {
            request: s3_call(&S3Operation::Initiate, None, RequestBody::Empty),
            response: text(
                200,
                &format!(
                    "<?xml version=\"1.0\"?><InitiateMultipartUploadResult><Bucket>{BUCKET}\
                     </Bucket><Key>{OBJECT_PATH}</Key><UploadId>{UPLOAD_ID}</UploadId>\
                     </InitiateMultipartUploadResult>"
                ),
            ),
        },
    ];

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
        chain.push(sign(&operation, Some(&digest)));
        chain.push(Interaction {
            request: s3_call(
                &operation,
                Some(&digest),
                RequestBody::Bytes(bytes.to_vec()),
            ),
            response: with_header(200, "", ResponseHeader::ETag, &etag),
        });
        completed.push((part.number, etag));
    }

    let complete = S3Operation::Complete {
        upload_id: UPLOAD_ID.to_owned(),
    };
    chain.push(sign(&complete, None));
    chain.push(Interaction {
        request: s3_call(
            &complete,
            None,
            RequestBody::Bytes(s3::complete_multipart_body(&completed).into_bytes()),
        ),
        response: text(
            200,
            "<CompleteMultipartUploadResult><Location>s3</Location><ETag>\"e-1\"</ETag>\
             </CompleteMultipartUploadResult>",
        ),
    });

    let staged = UploadHandle::new(UPLOAD_HANDLE.to_owned());
    let processed = ProcessedHandle::new(PROCESSED_HANDLE.to_owned());
    chain.push(Interaction {
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
    });
    chain.push(poll(
        PROCESS_JOB,
        1,
        0,
        &format!(r#"{{"status":2,"data":{{"error":null,"key":"{PROCESSED_HANDLE}"}}}}"#),
    ));
    chain.push(Interaction {
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
    });
    chain.push(poll(THUMBS_JOB, 2, 0, r#"{"status":0,"data":[]}"#));
    chain.push(poll(
        THUMBS_JOB,
        2,
        1,
        &format!(
            r#"{{"status":2,"data":{{"thumbnails":[{{"original":"https://example.invalid/0.jpg"}}],"collection_key":"{COLLECTION_KEY}"}}}}"#
        ),
    ));
    chain
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
    let evidence = futures::executor::block_on(adapter.submit(org(), key(), fields(), NOW))
        .expect("the recorded create chain replays");
    assert_eq!(
        hops, 15,
        "the render, the reservation, the clock, three signed S3 calls each with its own \
         signature, the two enqueues, three queue polls and the form post"
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
    futures::executor::block_on(adapter.submit(org(), key(), fields(), NOW))
        .expect("the three-part chain replays");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "every part's signature, PUT and ETag were consumed"
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
    let fingerprint =
        futures::executor::block_on(probe.assert_form_schema(org(), FormId(Uuid([2; 16]))))
            .expect("the render declares every field this adapter writes");
    let again = adapter(Cassette {
        interactions: vec![create_render()],
    });
    let repeat =
        futures::executor::block_on(again.assert_form_schema(org(), FormId(Uuid([2; 16]))))
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
        .assert_form_schema(org(), FormId(Uuid([2; 16]))),
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
    let refused = futures::executor::block_on(unattested.submit(org(), key(), fields(), NOW));
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
        .submit(org(), key(), many, NOW),
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
            .submit(org(), key(), none, NOW)
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
    let refused =
        futures::executor::block_on(adapter(cassette).submit(org(), key(), fields(), NOW));
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
    let refused =
        futures::executor::block_on(adapter(cassette).submit(org(), key(), fields(), NOW));
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
    let refused = futures::executor::block_on(adapter.submit(org(), key(), fields(), NOW));
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
    let refused =
        futures::executor::block_on(adapter(cassette).submit(org(), key(), fields(), NOW));
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
    let refused =
        futures::executor::block_on(adapter(cassette).submit(org(), key(), fields(), NOW));
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
        org(),
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

#[test]
fn a_read_back_under_the_probe_capability_is_refused() {
    let adapter = adapter(Cassette {
        interactions: vec![],
    });
    let refused = futures::executor::block_on(adapter.read_back(
        org(),
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

#[test]
fn no_committed_fixture_carries_a_live_token_or_identifier() {
    const FIXTURES: [(&str, &str); 2] = [
        (
            "create_form_page.json",
            include_str!("cassettes/create_form_page.json"),
        ),
        (
            "edit_form_page.json",
            include_str!("cassettes/edit_form_page.json"),
        ),
    ];
    // The live values from the 2026-08-28 captures. None of them may appear
    // in this repository, and the AWS key id in particular is a real IAM key
    // identifier belonging to TPT.
    const LIVE: [&str; 4] = [
        "AKIAJA34TRRDFU7AB2DQ",
        "21268787",
        "188ccbafbf906aafd1059e452f9e3e1b7c32255d",
        "live.digital.upload/de07-21268787",
    ];
    for (name, body) in FIXTURES {
        for needle in LIVE {
            assert!(
                !body.contains(needle),
                "{name} carries {needle:?}, which is a live upstream value"
            );
        }
    }
}
