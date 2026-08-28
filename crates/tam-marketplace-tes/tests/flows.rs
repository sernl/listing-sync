//! The adapter flows driven end to end against cassettes: every request the
//! flow issues must match the recording in order, and every test asserts the
//! cassette is fully consumed, so a step that silently vanished fails too.

use base64::Engine;
use serde_json::{json, Value};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{FilePart, HttpResponse};
use tam_marketplace::{
    AdapterError, AgeSpan, AmbiguityCause, FetchReason, FieldSet, FileContent, FileSource,
    FileSourceError, FormId, ListingLocator, MarketplaceAdapter, NativeTerm, ProjectedListing,
    RemoteLifecycle, RemoteListingId, WriteAttemptId,
};
use tam_marketplace_tes::endpoints::{
    self, CatalogueEntry, DraftId, FreeLicence, TesListing, TesPrice, TesPricing,
};
use tam_marketplace_tes::{schema, TesAdapter};
use tam_types::{
    Currency, FailureCode, FieldKey, FileId, InventoryId, Money, OrgId, PriceIntent, Timestamp,
    Uuid,
};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
/// The driver's clock reading a submit is handed. Tes ignores it — its JSON
/// API stamps its own instants — so any fixed instant is the honest value.
const NOW: Timestamp = Timestamp(1_756_000_000_000);
const DRAFT: DraftId = DraftId(9001);

struct StaticFiles(Vec<(FileId, FileContent)>);

impl FileSource for StaticFiles {
    async fn fetch(&self, file: FileId) -> Result<FileContent, FileSourceError> {
        self.0
            .iter()
            .find(|(id, _)| *id == file)
            .map(|(_, content)| content.clone())
            .ok_or(FileSourceError::Missing(file))
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn adapter(
    cassette: Cassette,
    files: Vec<(FileId, FileContent)>,
) -> TesAdapter<CassetteTransport, StaticFiles> {
    TesAdapter::new(
        InventoryId::TesGb,
        CassetteTransport::new(cassette),
        StaticFiles(files),
    )
    .expect("TesGb is a Tes inventory")
}

fn ok(body: &Value) -> HttpResponse {
    HttpResponse::plain(200, body.to_string().into_bytes())
}

fn status(code: u16) -> HttpResponse {
    HttpResponse::plain(code, Vec::new())
}

fn sample_listing() -> TesListing {
    TesListing {
        title: "Fractions pack".to_owned(),
        description_markdown: "A pack.".to_owned(),
        category_ids: vec![1_000_448],
        age_range_ids: vec![4],
        ages: vec![11, 12],
        main_type: 99_009,
        main_age: 4,
        pricing: TesPricing::Free(FreeLicence::CcBy),
    }
}

/// The same listing priced, which is the only difference a paid write makes
/// to the metadata the draft and the publish both carry.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn paid_listing(minor_units: i64) -> TesListing {
    TesListing {
        pricing: TesPricing::Paid(
            TesPrice::new(minor_units).expect("the fixture price is a price"),
        ),
        ..sample_listing()
    }
}

fn sample_field_set(file: FileId) -> FieldSet {
    FieldSet {
        entries: vec![
            (FieldKey::Title, "Fractions pack".to_owned()),
            (FieldKey::Description, "A pack.".to_owned()),
            (FieldKey::Price, "CC-BY".to_owned()),
            (
                FieldKey::Taxonomy,
                json!({"categories": [1_000_448], "mainType": 99_009}).to_string(),
            ),
            (
                FieldKey::Grades,
                json!({"ageRanges": [4], "ages": [11, 12], "mainAge": 4}).to_string(),
            ),
        ],
        files: vec![file],
    }
}

fn presign_response() -> Value {
    let policy = base64::engine::general_purpose::STANDARD.encode(
        json!({
            "conditions": [
                {"bucket": "tes-uploads"},
                ["starts-with", "$name", ""],
                ["starts-with", "$Content-Type", ""],
            ]
        })
        .to_string(),
    );
    json!([{
        "tempId": "TEMP-0",
        "s3pending": {"key": "k/9001", "params": {"policy": policy, "signature": "sig"}},
    }])
}

fn draft_state(id: i64, draft: bool) -> Value {
    json!({
        "id": id,
        "draft": draft,
        "title": "Fractions pack",
        "descriptionRaw": "A pack.",
    })
}

#[test]
fn the_full_submit_flow_replays_and_lands() {
    let file_id = FileId(Uuid([0x21; 16]));
    let content = FileContent {
        file_name: "pack.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        bytes: b"%PDF-1.4 tiny".to_vec(),
    };
    let presign_body = presign_response();
    let upload = endpoints::parse_presign(&presign_body, "pack.pdf", "application/pdf")
        .expect("the fixture presign parses");
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::create_draft_request(),
                response: ok(&json!({"id": 9001})),
            },
            Interaction {
                request: endpoints::set_metadata_request(DRAFT, &sample_listing()),
                response: ok(&json!({"id": 9001, "title": "Fractions pack"})),
            },
            Interaction {
                request: endpoints::presign_request(DRAFT, "pack.pdf", "TEMP-0"),
                response: ok(&presign_body),
            },
            Interaction {
                request: endpoints::s3_upload_request(
                    &upload,
                    FilePart {
                        part_name: "file".to_owned(),
                        file_name: "pack.pdf".to_owned(),
                        content_type: "application/pdf".to_owned(),
                        bytes: content.bytes.clone(),
                    },
                ),
                response: status(204),
            },
            Interaction {
                request: endpoints::confirm_request(DRAFT, &upload),
                response: ok(&json!([{"isUploaded": true, "s3pending": {"key": "k/9001"}}])),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, true)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![(file_id, content)]);
    let evidence = futures::executor::block_on(adapter.submit(
        ORG,
        tam_marketplace::IdempotencyKey(Uuid([1; 16])),
        sample_field_set(file_id),
        NOW,
    ))
    .expect("the recorded flow lands");
    assert_eq!(
        evidence.landed_on_route.as_deref(),
        Some("https://www.tes.com/api/v2/resources/9001"),
        "the durable identifier rides on the observed route"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "every recorded interaction was exercised"
    );
}

#[test]
fn a_confirm_that_never_flips_is_a_missing_confirmation() {
    let file_id = FileId(Uuid([0x21; 16]));
    let content = FileContent {
        file_name: "pack.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        bytes: b"x".to_vec(),
    };
    let presign_body = presign_response();
    let upload = endpoints::parse_presign(&presign_body, "pack.pdf", "application/pdf")
        .expect("the fixture presign parses");
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::presign_request(DRAFT, "pack.pdf", "TEMP-0"),
                response: ok(&presign_body),
            },
            Interaction {
                request: endpoints::s3_upload_request(
                    &upload,
                    FilePart {
                        part_name: "file".to_owned(),
                        file_name: "pack.pdf".to_owned(),
                        content_type: "application/pdf".to_owned(),
                        bytes: content.bytes.clone(),
                    },
                ),
                response: status(204),
            },
            Interaction {
                request: endpoints::confirm_request(DRAFT, &upload),
                response: ok(&json!([{"isUploaded": false}])),
            },
        ],
    };
    let adapter = adapter(cassette, vec![(file_id, content.clone())]);
    let refused = futures::executor::block_on(adapter.upload_file(DRAFT, 0, &content));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::SubmitNoConfirmation,
                ..
            })
        ),
        "2xx on every hop with isUploaded false is the M0 trap and must fail"
    );
}

#[test]
fn delete_reports_success_only_after_the_read_returns_not_found() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::delete_draft_request(DRAFT),
                response: status(204),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: status(404),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    futures::executor::block_on(adapter.delete_draft(DRAFT)).expect("a verified delete succeeds");
    assert_eq!(adapter.transport().remaining(), 0, "both hops ran");
}

#[test]
fn the_misleading_204_is_refused() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::delete_draft_request(DRAFT),
                response: status(204),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, false)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.delete_draft(DRAFT));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                ..
            })
        ),
        "a 204 with the draft still readable is not a delete, whatever the status said; \
         verifying against the resource route instead of the draft route reported a false gone live"
    );
}

#[test]
fn publish_proves_itself_by_reading_back() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::publish_request(DRAFT, &sample_listing()),
                response: ok(&json!({})),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, false)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let state = futures::executor::block_on(adapter.publish(DRAFT, &sample_listing()))
        .expect("a verified publish succeeds");
    assert_eq!(state["draft"], false, "the read-back is the verdict");
}

#[test]
fn an_unconfirmed_publish_is_ambiguous() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::publish_request(DRAFT, &sample_listing()),
                response: ok(&json!({})),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, true)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let unproven = futures::executor::block_on(adapter.publish(DRAFT, &sample_listing()));
    assert!(
        matches!(unproven, Err(AdapterError::Ambiguous(_))),
        "a publish the read-back cannot confirm proves nothing"
    );
}

/// The seam-side listing every projection test renders, carrying the numeric
/// native ids Tes addresses its taxonomy by.
fn projected(price: PriceIntent) -> ProjectedListing {
    ProjectedListing {
        title: "Fractions pack".to_owned(),
        body: "A pack.".to_owned(),
        price,
        taxonomy: vec![NativeTerm {
            native_id: Some("1000448".to_owned()),
            segments: vec!["Mathematics".to_owned()],
        }],
        grades: vec![NativeTerm {
            native_id: Some("4".to_owned()),
            segments: vec!["Secondary".to_owned()],
        }],
        ages: Some(AgeSpan {
            low_years: 11,
            high_years: 12,
        }),
        files: vec![FileId(Uuid([0x21; 16]))],
    }
}

/// What the projection put under one key, or the marker for an entry it never
/// rendered — which an assertion reports as the mismatch it is rather than as
/// a panic in a helper.
fn entry(fields: &FieldSet, key: FieldKey) -> String {
    fields
        .entries
        .iter()
        .find(|(field, _)| *field == key)
        .map_or_else(|| "<absent>".to_owned(), |(_, value)| value.clone())
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn money(minor_units: i64, currency: Currency) -> Money {
    Money::new(minor_units, currency).expect("the fixture price is positive")
}

#[test]
fn a_paid_projection_carries_the_tes_paid_licence_and_its_price_in_minor_units() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let fields = adapter
        .project_fields(&projected(PriceIntent::Paid(money(500, Currency::Gbp))))
        .expect("a paid listing projects into the GB inventory");
    assert_eq!(
        entry(&fields, FieldKey::Price),
        "TES-PAID:500",
        "the paid licence travels with the amount the API refuses it without"
    );
    let free = adapter
        .project_fields(&projected(PriceIntent::Free))
        .expect("a free listing projects");
    assert_eq!(
        entry(&free, FieldKey::Price),
        "CC-BY",
        "and a free listing is still the bare Creative Commons token"
    );
}

#[test]
fn a_price_denominated_in_another_currency_than_the_inventorys_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let refused = adapter.project_fields(&projected(PriceIntent::Paid(money(500, Currency::Usd))));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                ..
            })
        ),
        "the Tes wire carries a bare amount, so a dollar price into the GB inventory would \
         be posted as pounds: {refused:?}"
    );
}

#[test]
fn a_paid_field_set_parses_back_into_the_priced_draft_the_projection_named() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::create_draft_request(),
                response: ok(&json!({"id": 9001})),
            },
            // The cassette is the assertion: a token read back as free would
            // build a CC-BY metadata request and diverge here.
            Interaction {
                request: endpoints::set_metadata_request(DRAFT, &paid_listing(500)),
                response: ok(&json!({"id": 9001})),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, true)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let fields = FieldSet {
        entries: vec![
            (FieldKey::Title, "Fractions pack".to_owned()),
            (FieldKey::Description, "A pack.".to_owned()),
            (FieldKey::Price, "TES-PAID:500".to_owned()),
            (
                FieldKey::Taxonomy,
                json!({"categories": [1_000_448], "mainType": 99_009}).to_string(),
            ),
            (
                FieldKey::Grades,
                json!({"ageRanges": [4], "ages": [11, 12], "mainAge": 4}).to_string(),
            ),
        ],
        files: vec![],
    };
    futures::executor::block_on(adapter.submit(
        ORG,
        tam_marketplace::IdempotencyKey(Uuid([2; 16])),
        fields,
        NOW,
    ))
    .expect("the paid submit lands");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the priced draft was written exactly as the projection named it"
    );
}

#[test]
fn a_paid_token_without_a_usable_amount_is_refused_rather_than_freed() {
    for token in ["TES-PAID", "TES-PAID:0", "TES-PAID:-1", "TES-PAID:free"] {
        let adapter = adapter(
            Cassette {
                interactions: vec![],
            },
            vec![],
        );
        let fields = FieldSet {
            entries: vec![
                (FieldKey::Title, "T".to_owned()),
                (FieldKey::Description, "D".to_owned()),
                (FieldKey::Price, token.to_owned()),
                (FieldKey::Taxonomy, json!({"categories": []}).to_string()),
                (FieldKey::Grades, json!({"ageRanges": []}).to_string()),
            ],
            files: vec![],
        };
        let refused = futures::executor::block_on(adapter.submit(
            ORG,
            tam_marketplace::IdempotencyKey(Uuid([3; 16])),
            fields,
            NOW,
        ));
        assert!(
            matches!(
                refused,
                Err(AdapterError::Rejected {
                    code: FailureCode::UploadRejected,
                    ..
                })
            ),
            "{token:?} names no price, and a listing quietly demoted to free is the one failure \
             that costs the seller money: {refused:?}"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{token:?}: the refusal happens before the first request"
        );
    }
}

#[test]
fn a_paid_publish_carries_the_price_to_the_drafts_own_publish_route() {
    let listing = paid_listing(500);
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::publish_request(DRAFT, &listing),
                response: ok(&json!({})),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, false)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let state = futures::executor::block_on(adapter.publish(DRAFT, &listing))
        .expect("a verified paid publish succeeds");
    assert_eq!(
        state["draft"], false,
        "the read-back is the verdict here too"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the publish went to the captured route carrying the captured body"
    );
}

#[test]
fn a_published_delete_takes_the_resource_route_and_proves_it_by_a_404_read() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::delete_resource_request(DRAFT),
                response: status(204),
            },
            Interaction {
                request: endpoints::read_resource_request(DRAFT),
                response: status(404),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    futures::executor::block_on(adapter.delete_published(DRAFT))
        .expect("a verified published delete succeeds");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the delete and its proof both ran on the route a published resource lives at"
    );
}

#[test]
fn a_published_resource_still_readable_after_its_delete_is_a_mismatch() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::delete_resource_request(DRAFT),
                response: status(204),
            },
            Interaction {
                request: endpoints::read_resource_request(DRAFT),
                response: ok(&draft_state(9001, false)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.delete_published(DRAFT));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                ..
            })
        ),
        "a 204 with the resource still readable is not a delete on this route either"
    );
}

#[test]
fn delete_routes_each_state_to_the_endpoint_it_lives_at() {
    for (label, probe, deletion, proof) in [
        (
            "a never-published draft, which 404s on the resource route",
            status(404),
            endpoints::delete_draft_request(DRAFT),
            endpoints::read_draft_request(DRAFT),
        ),
        (
            "a published resource, which does not",
            ok(&draft_state(9001, false)),
            endpoints::delete_resource_request(DRAFT),
            endpoints::read_resource_request(DRAFT),
        ),
    ] {
        let cassette = Cassette {
            interactions: vec![
                Interaction {
                    request: endpoints::read_resource_request(DRAFT),
                    response: probe,
                },
                Interaction {
                    request: deletion,
                    response: status(204),
                },
                Interaction {
                    request: proof,
                    response: status(404),
                },
            ],
        };
        let adapter = adapter(cassette, vec![]);
        let deleted = futures::executor::block_on(adapter.delete(DRAFT));
        assert!(
            deleted.is_ok(),
            "{label}: the state decides the route, and this one deleted where it lives: \
             {deleted:?}"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: the state was probed, then deleted where it lives, then proved gone"
        );
    }
}

#[test]
fn a_delete_whose_state_cannot_be_read_names_the_condition_rather_than_a_route() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::read_resource_request(DRAFT),
            response: status(401),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.delete(DRAFT));
    assert_eq!(
        refused,
        Err(AdapterError::SessionExpired),
        "a lapsed session is not a resource that happens to be absent, and no delete is guessed \
         from it"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "nothing was deleted on the strength of an unreadable state"
    );
}

#[test]
fn an_edit_reposts_the_draft_metadata_carrying_the_change() {
    let edited = TesListing {
        title: "Fractions pack, second edition".to_owned(),
        description_markdown: "A better pack.".to_owned(),
        ..sample_listing()
    };
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::set_metadata_request(DRAFT, &edited),
            response: ok(&json!({"id": 9001, "title": "Fractions pack, second edition"})),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    futures::executor::block_on(adapter.update(DRAFT, &edited)).expect("the edit lands");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "an edit is the draft metadata POST restated, and nothing else"
    );
}

#[test]
fn an_edit_the_response_does_not_name_is_never_a_landing() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::set_metadata_request(DRAFT, &sample_listing()),
            response: ok(&json!({"id": 9002})),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let unproven = futures::executor::block_on(adapter.update(DRAFT, &sample_listing()));
    assert!(
        matches!(unproven, Err(AdapterError::Ambiguous(_))),
        "a 200 naming another resource proves nothing about this one: {unproven:?}"
    );
}

#[test]
fn the_preflight_names_a_vanished_written_field() {
    let probe = DraftId(77);
    let mut incomplete = json!({"id": 77, "draft": true});
    for field in schema::WRITTEN_DRAFT_FIELDS {
        if field != "descriptionRaw" {
            incomplete[field] = json!(null);
        }
    }
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::create_draft_request(),
                response: ok(&json!({"id": 77})),
            },
            Interaction {
                request: endpoints::set_metadata_request(probe, &endpoints::probe_listing()),
                response: ok(&json!({"id": 77})),
            },
            Interaction {
                request: endpoints::read_draft_request(probe),
                response: ok(&incomplete),
            },
            Interaction {
                request: endpoints::delete_draft_request(probe),
                response: status(204),
            },
            Interaction {
                request: endpoints::read_draft_request(probe),
                response: status(404),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let drifted =
        futures::executor::block_on(adapter.assert_form_schema(ORG, FormId(Uuid([7; 16]))));
    let Err(AdapterError::SchemaDrift(report)) = drifted else {
        panic!("a draft missing a written field must be drift, got {drifted:?}");
    };
    assert_eq!(
        report.removed,
        vec!["descriptionRaw".to_owned()],
        "the vanished field is named, and the probe draft was still deleted first"
    );
    assert_eq!(adapter.transport().remaining(), 0, "the probe cleaned up");
}

#[test]
fn the_probe_writes_before_asserting_because_an_empty_draft_omits_null_scalars() {
    let probe = DraftId(78);
    let mut populated = json!({"id": 78, "draft": true});
    for field in schema::WRITTEN_DRAFT_FIELDS {
        populated[field] = json!(null);
    }
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::create_draft_request(),
                response: ok(&json!({"id": 78})),
            },
            Interaction {
                request: endpoints::set_metadata_request(probe, &endpoints::probe_listing()),
                response: ok(&json!({"id": 78})),
            },
            Interaction {
                request: endpoints::read_draft_request(probe),
                response: ok(&populated),
            },
            Interaction {
                request: endpoints::delete_draft_request(probe),
                response: status(204),
            },
            Interaction {
                request: endpoints::read_draft_request(probe),
                response: status(404),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let fingerprint =
        futures::executor::block_on(adapter.assert_form_schema(ORG, FormId(Uuid([7; 16]))));
    assert!(
        fingerprint.is_ok(),
        "a probe that wrote every field must observe every field: {fingerprint:?}"
    );
    assert_eq!(adapter.transport().remaining(), 0, "the probe cleaned up");
}

#[test]
fn read_back_is_gated_and_maps_the_lifecycle() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::read_draft_request(DRAFT),
            response: ok(&draft_state(9001, false)),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let observed_at = Timestamp(1_756_000_000_000);
    let observed = futures::executor::block_on(adapter.read_back(
        ORG,
        ListingLocator::Durable(RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        }),
        FetchReason::FirstPartyExport {
            inventory: InventoryId::TesGb,
        },
        observed_at,
    ))
    .expect("a gated read succeeds");
    assert_eq!(
        observed.lifecycle,
        RemoteLifecycle::Live { since: observed_at },
        "a non-draft state is live as of the observation instant"
    );
    assert!(
        observed
            .fields
            .iter()
            .any(|(key, value)| *key == FieldKey::Title && value == "Fractions pack"),
        "the observed fields carry the title for the diff"
    );
}

#[test]
fn the_cassette_fixture_format_loads_from_disk() {
    let cassette: Cassette = serde_json::from_str(include_str!("cassettes/delete_verified.json"))
        .expect("the committed fixture parses");
    let adapter = adapter(cassette, vec![]);
    futures::executor::block_on(adapter.delete_draft(DRAFT))
        .expect("the fixture-driven verified delete succeeds");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the fixture recorded exactly the flow's hops"
    );
}

#[test]
fn a_missing_ambiguity_cause_is_not_invented() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::create_draft_request(),
            response: ok(&json!({"resourceId": 9001})),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.create_listing(&sample_listing(), &[]));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Ambiguous(AmbiguityCause::NoDurableIdentifier))
        ),
        "a 2xx create without an id is precisely the no-durable-identifier ambiguity"
    );
}

#[test]
fn the_catalogue_walk_pages_published_then_drafts_and_stops_on_an_empty_page() {
    let limit = endpoints::CATALOGUE_PAGE_LIMIT;
    let published_page = json!([
        {
            "id": 9001, "title": "Fractions pack", "licence": "TES-PAID",
            "price": 350, "draft": false, "url": "/teaching-resource/fractions-pack-9001"
        },
        {
            "id": 9002, "title": "Free starter", "licence": "CC-BY",
            "price": 0, "draft": false, "url": "/teaching-resource/free-starter-9002"
        },
    ]);
    let drafts_page = json!([
        {
            "id": 9003, "title": "Half written", "licence": "CC-BY-SA",
            "price": 0, "draft": true, "url": "/teaching-resource/half-written-9003"
        },
    ]);
    let empty = json!([]);
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::list_resources_request(0, limit),
                response: ok(&published_page),
            },
            Interaction {
                request: endpoints::list_resources_request(1, limit),
                response: ok(&empty),
            },
            Interaction {
                request: endpoints::list_drafts_request(0, limit),
                response: ok(&drafts_page),
            },
            Interaction {
                request: endpoints::list_drafts_request(1, limit),
                response: ok(&empty),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let entries =
        futures::executor::block_on(adapter.list_own_resources(&FetchReason::FirstPartyExport {
            inventory: InventoryId::TesGb,
        }))
        .expect("the first-party catalogue read succeeds");

    assert_eq!(
        entries,
        vec![
            CatalogueEntry {
                id: 9001,
                title: "Fractions pack".to_owned(),
                published: true,
                licence: Some("TES-PAID".to_owned()),
                price_pence: Some(350),
            },
            CatalogueEntry {
                id: 9002,
                title: "Free starter".to_owned(),
                published: true,
                licence: Some("CC-BY".to_owned()),
                price_pence: Some(0),
            },
            CatalogueEntry {
                id: 9003,
                title: "Half written".to_owned(),
                published: false,
                licence: Some("CC-BY-SA".to_owned()),
                price_pence: Some(0),
            },
        ],
        "both lists arrive in order, each row carrying the price in pence and its own draft state"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the walk stopped at the empty page of each list, and nowhere earlier"
    );
}

#[test]
fn a_catalogue_read_without_the_export_capability_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let refused =
        futures::executor::block_on(adapter.list_own_resources(&FetchReason::VerifyAttempt {
            attempt: WriteAttemptId(Uuid([3; 16])),
        }));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::Other,
                ..
            })
        ),
        "an enumeration read needs the tier-one capability, not any reason at all"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the refusal happens before the first request, so nothing was sent"
    );
}

/// A bundle's first bytes are a ZIP local header, and the rest is chosen to
/// be undecodable as UTF-8: this fixture exists to prove the transport hands
/// the bytes back untouched rather than through a lossy decode.
fn zip_bytes() -> Vec<u8> {
    let mut bytes = b"PK\x03\x04\x14\x00\x08\x00\x08\x00".to_vec();
    bytes.extend((0u8..=255).cycle().take(1024));
    bytes.extend_from_slice(b"PK\x05\x06");
    bytes
}

const BUNDLE_PATH: &str = "/teaching-resource/download/9001/bundle";

#[test]
fn a_published_resource_downloads_its_bundle_byte_for_byte() {
    let bundle = zip_bytes();
    let manifest = json!({
        "zipUrls": {
            "9001": { "url": BUNDLE_PATH, "title": "Fractions pack" }
        }
    });
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&manifest),
            },
            Interaction {
                request: endpoints::download_bundle_request(BUNDLE_PATH),
                response: HttpResponse::plain(200, bundle.clone()),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let downloaded = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::TesGb,
        },
        DRAFT,
    ))
    .expect("a published resource downloads its bundle");

    assert_eq!(
        downloaded, bundle,
        "the bundle arrives byte for byte; a lossy transport would corrupt every archive"
    );
    assert_eq!(
        downloaded.get(..4),
        Some(b"PK\x03\x04".as_slice()),
        "the zip magic survives, which is what the pipeline's extraction depends on"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the manifest and the bundle it named, and nothing else"
    );
}

#[test]
fn a_draft_has_no_bundle_and_says_so_distinctly() {
    for (label, body) in [
        (
            "a manifest carrying no zipUrls",
            json!({ "zipUrls": {} }).to_string().into_bytes(),
        ),
        (
            "the html redirect the live route answers a draft with",
            b"<html><head><title>Not found</title></head></html>".to_vec(),
        ),
    ] {
        let cassette = Cassette {
            interactions: vec![Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: HttpResponse::plain(200, body),
            }],
        };
        let adapter = adapter(cassette, vec![]);
        let refused = futures::executor::block_on(adapter.download_resource_bundle(
            &FetchReason::FirstPartyExport {
                inventory: InventoryId::TesGb,
            },
            DRAFT,
        ));
        assert!(
            matches!(
                refused,
                Err(AdapterError::Rejected {
                    code: FailureCode::PreconditionElementAbsent,
                    ..
                })
            ),
            "{label}: an absent bundle is a stated rejection, never an ambiguity"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: the bundle itself was never requested"
        );
    }
}

#[test]
fn a_bundle_download_without_the_export_capability_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::VerifyAttempt {
            attempt: WriteAttemptId(Uuid([3; 16])),
        },
        DRAFT,
    ));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::Other,
                ..
            })
        ),
        "downloading a seller's files needs the tier-one capability"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the refusal happens before the first request"
    );
}
