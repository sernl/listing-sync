//! The adapter flows driven end to end against cassettes: every request the
//! flow issues must match the recording in order, and every test asserts the
//! cassette is fully consumed, so a step that silently vanished fails too.

use base64::Engine;
use serde_json::{json, Value};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{FilePart, HttpResponse, RequestBody, ResponseHeader};
use tam_marketplace::{
    AdapterError, AgeSpan, AmbiguityCause, CanaryGrant, FetchReason, FieldDiffReport, FieldSet,
    FileContent, FileSource, FileSourceError, FormId, LifecycleTransition, ListingLocator,
    ListingState, MarketplaceAdapter, NativeAxis, NativeTerm, Outcome, ProjectedListing,
    RemoteLifecycle, RemoteListingId, RemovalPlan, RevisePlan, WriteAttemptId,
};
use tam_marketplace_tes::endpoints::{
    self, CatalogueEntry, DraftId, FreeLicence, TesListing, TesPrice, TesPricing,
};
use tam_marketplace_tes::{schema, TesAdapter};
use tam_types::{
    AttemptId, CopyFormat, Currency, FailureCode, FieldKey, FileId, InventoryId, Money,
    PriceIntent, TermKind, Timestamp, Uuid,
};

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
        InventoryId::Tes,
        CassetteTransport::new(cassette),
        StaticFiles(files),
    )
    .expect("Tes is a Tes inventory")
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
        description_raw: "A pack.".to_owned(),
        description_format: CopyFormat::Markdown,
        category_ids: vec![1_000_448],
        age_channel: tam_marketplace_tes::endpoints::TesAges::new(vec![4]),
        ages: vec![11, 12],
        main_type: Some(99_009),
        main_age: Some(4),
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
        body_format: Some(CopyFormat::Markdown),
        appropriate_for_country: None,
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
        body_format: CopyFormat::Markdown,
        // The licence the seller elected, in the branch's own token: the
        // projection resolves one per pricing branch because Tes gates the
        // write on it, and every fixture here is a projection that resolved.
        natives: vec![NativeAxis {
            axis: TermKind::Licence,
            value: NativeTerm {
                native_id: Some(
                    match price {
                        PriceIntent::Free => "CC-BY-SA",
                        PriceIntent::Paid(_) => "TES-PAID",
                    }
                    .to_owned(),
                ),
                segments: vec!["Licence".to_owned()],
            },
        }],
        appropriate_for_country: None,
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
        "CC-BY-SA",
        "and a free listing carries the licence the seller elected, not a default"
    );
}

/// D2. Every free listing we created was published under CC-BY -- a
/// perpetual, irrevocable grant -- because the adapter substituted one when
/// the projection carried none. The substitution is gone: with nothing
/// elected there is nothing to send, and a refusal is the only honest answer.
#[test]
fn a_free_listing_with_no_elected_licence_is_refused_rather_than_granted_cc_by() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Free);
    listing.natives.clear();
    let refused = adapter.project_fields(&listing);
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "issuing a rights grant the seller never chose is the failure this refusal \
         exists to prevent: {refused:?}"
    );
}

/// The other half of the same rule: an election that names a licence Tes
/// refuses with a price is not quietly overridden by the paid token.
#[test]
fn an_elected_creative_commons_licence_on_a_priced_listing_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Paid(money(500, Currency::Gbp)));
    listing.natives[0].value.native_id = Some("CC-BY-ND".to_owned());
    let refused = adapter.project_fields(&listing);
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "Tes refuses a Creative Commons value with a price, so this adapter does too \
         rather than posting a licence the seller did not elect: {refused:?}"
    );
}

/// F3, settled by the 2026-08-29 live probe: Tes accepts
/// `descriptionRawType: "html"`, echoes the type back and returns the markup
/// byte-intact. So a TPT-sourced body -- every one of which is HTML -- crosses
/// into Tes as itself. The refusal this replaces was the interim that stood
/// while the wire question was open.
#[test]
fn an_html_body_posts_under_the_html_type_rather_than_being_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Free);
    listing.body = "<p>A pack.</p>".to_owned();
    listing.body_format = CopyFormat::Html;
    let fields = adapter
        .project_fields(&listing)
        .expect("Tes takes either format, so an HTML body projects");
    assert_eq!(
        entry(&fields, FieldKey::Description),
        "<p>A pack.</p>",
        "the markup travels verbatim; nothing converts it"
    );
    assert_eq!(
        fields.body_format,
        Some(CopyFormat::Html),
        "and the declaration travels beside it, because the type posted is a function of it"
    );
}

/// The wire half of the same rule, on both tokens: the type is what the
/// listing declares rather than the constant `"md"` it used to be.
#[test]
fn the_description_type_follows_the_format_the_listing_declares() {
    for (format, token) in [(CopyFormat::Markdown, "md"), (CopyFormat::Html, "html")] {
        let listing = TesListing {
            description_format: format,
            ..sample_listing()
        };
        let RequestBody::Json(body) = endpoints::set_metadata_request(DRAFT, &listing).body else {
            panic!("the draft metadata is a JSON post");
        };
        assert_eq!(
            body["descriptionRawType"],
            json!(token),
            "{format:?} posts as {token:?}, and posting one format's bytes under the other's \
             declaration is what writes escaped markup into the seller's listing"
        );
    }
}

/// The seam the two halves meet at. The cassette is the assertion: a submit
/// that dropped the declaration would build the `"md"` request and diverge
/// here.
#[test]
fn a_field_set_declaring_html_submits_the_draft_under_the_html_type() {
    let html = TesListing {
        description_raw: "<p>A pack.</p>".to_owned(),
        description_format: CopyFormat::Html,
        ..sample_listing()
    };
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::create_draft_request(),
                response: ok(&json!({"id": 9001})),
            },
            Interaction {
                request: endpoints::set_metadata_request(DRAFT, &html),
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
            (FieldKey::Description, "<p>A pack.</p>".to_owned()),
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
        files: vec![],
        body_format: Some(CopyFormat::Html),
        appropriate_for_country: None,
    };
    futures::executor::block_on(adapter.submit(
        tam_marketplace::IdempotencyKey(Uuid([4; 16])),
        fields,
        NOW,
    ))
    .expect("the HTML submit lands");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the draft was written under the type the field set declared"
    );
}

/// And a description with no declaration beside it is refused rather than
/// assumed to be markdown, which is the same rule the licence and the
/// category ids answer to.
#[test]
fn a_description_without_a_declared_format_is_refused_rather_than_assumed() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let fields = FieldSet {
        body_format: None,
        ..sample_field_set_without_files()
    };
    let refused = futures::executor::block_on(adapter.submit(
        tam_marketplace::IdempotencyKey(Uuid([5; 16])),
        fields,
        NOW,
    ));
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "guessing a body's format from its bytes is the failure the declaration prevents: \
         {refused:?}"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "and nothing was sent on the strength of the guess"
    );
}

/// P.1, settled by the 2026-08-29 capture. `16+` is closed on the wire at
/// `{16,17,18}`, so a 16+-only listing states its real ages instead of
/// omitting the pair the way the interim did. The band is what `mainAge`
/// names; the interval that used to fill both fields put an age in a field
/// Tes reads as a band id.
#[test]
fn a_sixteen_plus_declaration_posts_the_bands_own_closed_age_set() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Free);
    listing.grades[0].native_id = Some("6".to_owned());
    // The interval a 16+ band derives is nothing, which is what made the
    // interim omit the pair; the wire fields do not come from it any more.
    listing.ages = None;
    let fields = adapter
        .project_fields(&listing)
        .expect("a 16+ declaration projects");
    let grades: Value =
        serde_json::from_str(&entry(&fields, FieldKey::Grades)).expect("grades are JSON");
    assert_eq!(
        grades["ages"],
        json!([16, 17, 18]),
        "Tes publishes humanAges [16,17,18] for the band, so this is a read value: {grades}"
    );
    assert_eq!(
        grades["mainAge"],
        json!(6),
        "and `mainAge` names the band, not an age in years: {grades}"
    );
}

/// The other half of the narrowing: the pair is still omitted where the
/// declaration really has no ages -- band 7 is "Age not applicable" and
/// publishes an empty set -- because an empty list beside `mainAge: 0` names
/// age zero as surely as `mainType: 0` named a real resource type.
#[test]
fn a_declaration_with_no_ages_at_all_still_omits_the_pair() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Free);
    listing.grades[0].native_id = Some("7".to_owned());
    let fields = adapter
        .project_fields(&listing)
        .expect("the not-applicable band is a declaration like any other");
    let grades: Value =
        serde_json::from_str(&entry(&fields, FieldKey::Grades)).expect("grades are JSON");
    assert!(
        grades.get("ages").is_none() && grades.get("mainAge").is_none(),
        "neither half of the pair is invented, got {grades}"
    );

    let spanless = TesListing {
        age_channel: tam_marketplace_tes::endpoints::TesAges::new(vec![7]),
        ages: vec![],
        main_age: None,
        ..sample_listing()
    };
    let RequestBody::Json(body) = endpoints::set_metadata_request(DRAFT, &spanless).body else {
        panic!("the draft metadata is a JSON post");
    };
    assert!(
        body.get("ages").is_none() && body.get("mainAge").is_none(),
        "and the wire carries neither, got {body}"
    );
    assert!(
        body.get("additionalAge").is_none(),
        "nor an additional band, which names what it is additional to: {body}"
    );
    assert!(
        body.get("ageRanges").is_some(),
        "the declaration itself still travels: the bands are what the seller stated"
    );
}

/// The union rule where the projection meets it: two disjoint bands post
/// their union, and the contiguous fill this replaces would have claimed
/// every age between them.
#[test]
fn two_disjoint_bands_project_their_union_rather_than_the_span_between_them() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Free);
    listing.grades = vec![
        NativeTerm {
            native_id: Some("2".to_owned()),
            segments: vec!["Primary".to_owned()],
        },
        NativeTerm {
            native_id: Some("6".to_owned()),
            segments: vec!["Post-16".to_owned()],
        },
    ];
    let fields = adapter
        .project_fields(&listing)
        .expect("two bands project as readily as one");
    let grades: Value =
        serde_json::from_str(&entry(&fields, FieldKey::Grades)).expect("grades are JSON");
    assert_eq!(
        grades["ages"],
        json!([5, 6, 7, 16, 17, 18]),
        "the capture posts exactly this for bands 2 and 6; a fill would claim 8 through 15: \
         {grades}"
    );
}

/// D3. `mainType: 0` names a real Tes resource type, and it was on every
/// listing we ever created. The axis is unrouted, so the key is absent.
#[test]
fn an_unprojected_resource_type_omits_main_type_rather_than_sending_zero() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let fields = adapter
        .project_fields(&projected(PriceIntent::Free))
        .expect("a free listing projects");
    let taxonomy: Value =
        serde_json::from_str(&entry(&fields, FieldKey::Taxonomy)).expect("the taxonomy is JSON");
    assert!(
        taxonomy.get("mainType").is_none(),
        "an optional field nothing projected into is absent, never zero: {taxonomy}"
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
        body_format: Some(CopyFormat::Markdown),
        appropriate_for_country: None,
    };
    futures::executor::block_on(adapter.submit(
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
            body_format: Some(CopyFormat::Markdown),
            appropriate_for_country: None,
        };
        let refused = futures::executor::block_on(adapter.submit(
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

fn revise_plan(transition: LifecycleTransition) -> RevisePlan {
    RevisePlan {
        subject: RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        },
        fields: sample_field_set_without_files(),
        transition,
    }
}

fn sample_field_set_without_files() -> FieldSet {
    FieldSet {
        files: vec![],
        ..sample_field_set(FileId(Uuid([0x21; 16])))
    }
}

fn removal_plan(state: ListingState) -> RemovalPlan {
    RemovalPlan {
        attempt: WriteAttemptId(Uuid([0x61; 16])),
        subject: RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        },
        state,
    }
}

/// The four-case dispatch, on the two cells a capture settles. Collapsing it
/// to one route would silently publish an edit, or silently fail to publish
/// a publish.
#[test]
fn revise_to_live_publishes_and_revise_to_draft_reposts_the_metadata() {
    for (label, transition, request, route) in [
        (
            "an edit that keeps the draft",
            LifecycleTransition {
                from: ListingState::Draft,
                to: ListingState::Draft,
            },
            endpoints::set_metadata_request(DRAFT, &sample_listing()),
            "https://www.tes.com/api/v2/resources/9001/draft",
        ),
        (
            "a publish",
            LifecycleTransition {
                from: ListingState::Draft,
                to: ListingState::Live,
            },
            endpoints::publish_request(DRAFT, &sample_listing()),
            "https://www.tes.com/api/v2/resources/9001/draft/publish",
        ),
    ] {
        let cassette = Cassette {
            interactions: vec![Interaction {
                request,
                response: ok(&draft_state(9001, false)),
            }],
        };
        let adapter = adapter(cassette, vec![]);
        let evidence = futures::executor::block_on(adapter.revise(revise_plan(transition), NOW))
            .unwrap_or_else(|error| panic!("{label}: the cell posts its route: {error:?}"));
        assert_eq!(
            evidence.landed_on_route.as_deref(),
            Some(route),
            "{label}: the evidence names the route the write went to"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: the cell posts and classifies and reads nothing back — verification is \
             the driver's, because only the driver holds a budget to poll with"
        );
    }
}

/// The exact invention the charter forbids: no capture shows `POST
/// /{id}/draft` against a published resource, and the `/draft` route is a
/// draft overlay, so guessing would edit the overlay and leave the live
/// listing standing.
#[test]
fn an_edit_of_a_published_resource_is_uncaptured_rather_than_guessed() {
    for (label, to, capability) in [
        (
            "editing a live listing",
            ListingState::Live,
            "tes.edit_published",
        ),
        ("unpublishing", ListingState::Draft, "tes.unpublish"),
    ] {
        // One interaction the wrong implementation would consume — a
        // published edit approximated as a draft metadata post. `remaining()`
        // over an empty cassette is `len().saturating_sub(cursor)`, which
        // reads zero whether nothing was sent or five things were, so the
        // untouched claim needs something for the counter to still be
        // holding.
        let adapter = adapter(
            Cassette {
                interactions: vec![Interaction {
                    request: endpoints::set_metadata_request(DRAFT, &sample_listing()),
                    response: ok(&draft_state(9001, false)),
                }],
            },
            vec![],
        );
        let refused = futures::executor::block_on(adapter.revise(
            revise_plan(LifecycleTransition {
                from: ListingState::Live,
                to,
            }),
            NOW,
        ));
        assert_eq!(
            refused,
            Err(AdapterError::Uncaptured { capability }),
            "{label}: a capability no capture settles is refused, not approximated"
        );
        assert_eq!(
            adapter.transport().remaining(),
            1,
            "{label}: and the refusal sends nothing at all — the interaction it would \
             have consumed is still there"
        );
    }
}

/// The state selects the route and nothing probes for it — the 2026-08-29
/// incident, where a publish-lagged read said "draft" for a live listing and
/// only its overlay was removed.
#[test]
fn a_removal_deletes_the_route_the_stated_state_lives_at() {
    for (label, state, deletion, route) in [
        (
            "a draft",
            ListingState::Draft,
            endpoints::delete_draft_request(DRAFT),
            "https://www.tes.com/api/v2/resources/9001/draft",
        ),
        (
            "a published resource",
            ListingState::Live,
            endpoints::delete_resource_request(DRAFT),
            "https://www.tes.com/api/v2/resources/9001",
        ),
    ] {
        let cassette = Cassette {
            interactions: vec![Interaction {
                request: deletion,
                response: status(204),
            }],
        };
        let adapter = adapter(cassette, vec![]);
        let evidence = futures::executor::block_on(adapter.remove(removal_plan(state), NOW))
            .unwrap_or_else(|error| {
                panic!("{label}: the stated state decides the route: {error:?}")
            });
        assert_eq!(
            evidence.landed_on_route.as_deref(),
            Some(route),
            "{label}: the evidence names the delete that was posted"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: the removal posts and does not confirm; the disappearance is the \
             driver's to poll for, on a route that lags this write by seconds"
        );
    }
}

/// `landed` becomes the bind's remote-id columns. A revise that minted the
/// route it posted rather than the canonical resource identity would make
/// `settle` report `DivergentLanding` against the mapping it had just
/// successfully revised.
#[test]
fn a_revise_and_a_create_mint_the_same_landed_identifier() {
    let created = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::create_draft_request(),
                response: ok(&json!({"id": 9001})),
            },
            Interaction {
                request: endpoints::set_metadata_request(DRAFT, &sample_listing()),
                response: ok(&draft_state(9001, true)),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, true)),
            },
        ],
    };
    let submitted = futures::executor::block_on(adapter(created, vec![]).submit(
        tam_marketplace::IdempotencyKey(Uuid([2; 16])),
        sample_field_set_without_files(),
        NOW,
    ))
    .expect("the create lands");

    let revised = Cassette {
        interactions: vec![Interaction {
            request: endpoints::set_metadata_request(DRAFT, &sample_listing()),
            response: ok(&draft_state(9001, true)),
        }],
    };
    let evidence = futures::executor::block_on(adapter(revised, vec![]).revise(
        revise_plan(LifecycleTransition {
            from: ListingState::Draft,
            to: ListingState::Draft,
        }),
        NOW,
    ))
    .expect("the revise lands");

    assert_eq!(
        evidence.landed, submitted.landed,
        "a create and a revise of one resource must mint one identity, byte for byte"
    );
    assert_eq!(
        evidence.landed,
        Some(RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        }),
        "and it is the canonical resource url, never the route the write went to"
    );
}

#[test]
fn a_stated_delete_takes_its_states_route_without_probing_for_it() {
    for (label, state, deletion, proof) in [
        (
            "a draft",
            ListingState::Draft,
            endpoints::delete_draft_request(DRAFT),
            endpoints::read_draft_request(DRAFT),
        ),
        (
            "a published resource",
            ListingState::Live,
            endpoints::delete_resource_request(DRAFT),
            endpoints::read_resource_request(DRAFT),
        ),
    ] {
        let cassette = Cassette {
            interactions: vec![
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
        let deleted = futures::executor::block_on(adapter.delete(DRAFT, state));
        assert!(
            deleted.is_ok(),
            "{label}: the stated state decides the route: {deleted:?}"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: the delete and its proof, and NO state probe before them — a probe of \
             /resources/{{id}} lags a publish and misroutes a live listing to the draft delete"
        );
    }
}

#[test]
fn a_published_delete_is_stated_even_while_the_resource_route_still_404s() {
    // The 2026-08-29 live failure, as a cassette: seconds after a confirmed
    // publish the resource route still answered 404, the dispatcher read that
    // as "this is a draft", and the draft delete cleared only the overlay
    // while the live paid listing stayed up. A stated Published never asks.
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
    futures::executor::block_on(adapter.delete(DRAFT, ListingState::Live))
        .expect("a freshly published resource deletes on the route it was published to");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "nothing was read to decide the route, so nothing lagging could misdirect it"
    );
}

#[test]
fn presence_is_read_on_the_route_the_state_lives_at() {
    for (label, state, read, response, expected) in [
        (
            "a draft that is still there",
            ListingState::Draft,
            endpoints::read_draft_request(DRAFT),
            ok(&draft_state(9001, true)),
            true,
        ),
        (
            "a draft that is gone",
            ListingState::Draft,
            endpoints::read_draft_request(DRAFT),
            status(404),
            false,
        ),
        (
            "a published resource that is still there",
            ListingState::Live,
            endpoints::read_resource_request(DRAFT),
            ok(&draft_state(9001, false)),
            true,
        ),
        (
            "a published resource that is gone",
            ListingState::Live,
            endpoints::read_resource_request(DRAFT),
            status(404),
            false,
        ),
    ] {
        let cassette = Cassette {
            interactions: vec![Interaction {
                request: read,
                response,
            }],
        };
        let adapter = adapter(cassette, vec![]);
        assert_eq!(
            futures::executor::block_on(adapter.is_present(DRAFT, state)),
            Ok(expected),
            "{label}: only the state's own route can witness whether it is there"
        );
        assert_eq!(
            adapter.transport().remaining(),
            0,
            "{label}: one read, on that route and no other"
        );
    }
}

#[test]
fn probing_for_a_state_reads_a_resource_route_that_lags_a_publish() {
    // The documented hazard, pinned so it cannot be reintroduced silently as
    // the default: this path is for reconciliation, which meets listings it
    // did not create, and never for a caller that just wrote.
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::read_resource_request(DRAFT),
                response: status(404),
            },
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
    futures::executor::block_on(adapter.delete_probing_state(DRAFT))
        .expect("a 404 on the resource route probes as a draft");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the probe answered 404 and the draft route was taken — which is right for a real draft \
         and wrong for a resource published moments ago, hence the stated-state path above"
    );
}

#[test]
fn a_probed_delete_whose_state_cannot_be_read_names_the_condition_rather_than_a_route() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::read_resource_request(DRAFT),
            response: status(401),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.delete_probing_state(DRAFT));
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
        description_raw: "A better pack.".to_owned(),
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
    let drifted = futures::executor::block_on(adapter.assert_form_schema(FormId(Uuid([7; 16]))));
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
        futures::executor::block_on(adapter.assert_form_schema(FormId(Uuid([7; 16]))));
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
        ListingLocator::Durable(RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        }),
        FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
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

/// The union predicate: `resource_state` reads the draft route and falls
/// through to the resource route, so a 404 on both means neither answers.
/// Under the old behaviour that pair propagated `Rejected
/// { PreconditionElementAbsent }`, which the machine lists as inapplicable in
/// `AwaitingReadBack` and which therefore errored the whole run.
#[test]
fn a_resource_that_is_not_there_reads_back_absent() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: status(404),
            },
            Interaction {
                request: endpoints::read_resource_request(DRAFT),
                response: status(404),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let subject = RemoteListingId::Tes {
        url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
    };
    let observed = futures::executor::block_on(adapter.read_back(
        ListingLocator::Durable(subject.clone()),
        FetchReason::VerifyAttempt {
            attempt: WriteAttemptId(Uuid([0x61; 16])),
        },
        Timestamp(1_756_000_000_000),
    ))
    .expect("a listing that is not there is an observation, not a read failure");
    assert_eq!(
        observed.lifecycle,
        RemoteLifecycle::Absent,
        "both routes silent is the absence the live runner proves a deletion with"
    );
    assert_eq!(
        observed.id, subject,
        "the observation names the listing that was asked about"
    );
    assert!(
        observed.fields.is_empty(),
        "an absent listing carries no field values to diff"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "both routes were read; a single-route probe would leave one unspent"
    );
}

/// The four other `resource_state` callers still read a 404 as an error, so
/// the catch cannot have widened past `read_back`.
#[test]
fn a_publish_whose_state_read_404s_is_still_an_error() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::publish_request(DRAFT, &sample_listing()),
                response: status(200),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: status(404),
            },
            Interaction {
                request: endpoints::read_resource_request(DRAFT),
                response: status(404),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.publish(DRAFT, &sample_listing()));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: tam_types::FailureCode::PreconditionElementAbsent,
                ..
            })
        ),
        "a publish that cannot read its own resource back must not be told the \
         resource is legitimately absent, got {refused:?}"
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
            inventory: InventoryId::Tes,
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

/// The reconcile of a stranded create reads the catalogue under the fencing
/// row of the attempt it is settling, so the gate admits `VerifyAttempt`
/// beside the export capability.
#[test]
fn a_catalogue_read_is_admitted_by_the_fencing_row_of_the_attempt_it_reconciles() {
    let empty = json!([]);
    let limit = endpoints::CATALOGUE_PAGE_LIMIT;
    let adapter = adapter(
        Cassette {
            interactions: vec![
                Interaction {
                    request: endpoints::list_resources_request(0, limit),
                    response: ok(&empty),
                },
                Interaction {
                    request: endpoints::list_drafts_request(0, limit),
                    response: ok(&empty),
                },
            ],
        },
        vec![],
    );
    let entries =
        futures::executor::block_on(adapter.list_own_resources(&FetchReason::VerifyAttempt {
            attempt: WriteAttemptId(Uuid([3; 16])),
        }))
        .expect("a reconcile's enumeration is justified by the attempt it is settling");
    assert!(entries.is_empty(), "the seller's catalogue is empty here");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "and the walk ran in full rather than being turned back at the gate, which is the \
         only thing that distinguishes an admitted reason from a refused one"
    );
}

/// Two reasons justify an enumeration and no third does. This is the test a
/// later widening has to argue with, so it names every remaining variant
/// rather than a representative one.
#[test]
fn a_catalogue_read_refuses_every_reason_but_the_export_and_the_reconcile() {
    let receipt = match tam_marketplace::settle(
        AttemptId(Uuid([4; 16])),
        RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        },
        NOW,
        FieldDiffReport {
            normaliser_version: 0,
            mismatches: Vec::new(),
        },
    ) {
        Outcome::Committed { receipt, .. } | Outcome::Degraded { receipt, .. } => receipt,
        other @ (Outcome::Rejected { .. }
        | Outcome::Ambiguous { .. }
        | Outcome::Blocked { .. }
        | Outcome::Skipped { .. }) => {
            panic!("an empty diff report settles into the committed class: {other:?}")
        }
    };
    for reason in [
        FetchReason::VerifyWrite {
            receipt: receipt.clone(),
        },
        FetchReason::PollLifecycle { receipt },
        FetchReason::StructuralProbe {
            grant: CanaryGrant {
                inventory: InventoryId::Tes,
                decided_at: NOW,
            },
        },
    ] {
        let adapter = adapter(
            Cassette {
                interactions: vec![Interaction {
                    request: endpoints::list_resources_request(0, endpoints::CATALOGUE_PAGE_LIMIT),
                    response: ok(&json!([])),
                }],
            },
            vec![],
        );
        let refused = futures::executor::block_on(adapter.list_own_resources(&reason));
        assert!(
            matches!(
                refused,
                Err(AdapterError::Rejected {
                    code: FailureCode::Other,
                    ..
                })
            ),
            "an enumeration needs the export capability or a fencing row, and {reason:?} is \
             neither: {refused:?}"
        );
        assert_eq!(
            adapter.transport().remaining(),
            1,
            "and the refusal happens before the first request: the walk's own first page is \
             still unconsumed, which an empty cassette could not have shown"
        );
    }
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
            inventory: InventoryId::Tes,
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

/// The whole download, including the hop the marketplace named.
///
/// Three requests: the manifest, the bundle route that answers a redirect, and
/// the signed url re-issued carrying nothing of ours. The third is the one
/// this test exists for — that it is made at all, and that it is made as
/// `Redirected` rather than under the session, since the transport's routing
/// rule is what keeps the seller's cookie off a host the marketplace chose.
#[test]
fn a_redirected_bundle_hop_is_re_issued_carrying_nothing_of_ours() {
    let manifest = json!({ "zipUrls": { DRAFT.0.to_string(): { "url": BUNDLE_PATH } } });
    let bundle = b"PK\x03\x04 the seller's own archive".to_vec();
    let signed = "https://d111111abcdef8.cloudfront.net/bundle?Signature=abc";
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&manifest),
            },
            Interaction {
                request: endpoints::download_bundle_request(BUNDLE_PATH),
                response: HttpResponse {
                    status: 302,
                    body: Vec::new(),
                    headers: vec![(ResponseHeader::Location, signed.to_owned())],
                },
            },
            Interaction {
                request: endpoints::redirected_bundle_request(signed.to_owned()),
                response: HttpResponse::plain(200, bundle.clone()),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let downloaded = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect("the re-issued hop returns the bundle");

    assert_eq!(
        downloaded, bundle,
        "the bytes are the archive the signed url served, byte for byte"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "three requests and no more: the manifest, the redirect, and the one hop it named"
    );
}

/// A redirect with nowhere to go is a refusal, not an ambiguity.
#[test]
fn a_bundle_redirect_with_no_location_refuses_by_name() {
    let manifest = json!({ "zipUrls": { DRAFT.0.to_string(): { "url": BUNDLE_PATH } } });
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&manifest),
            },
            Interaction {
                request: endpoints::download_bundle_request(BUNDLE_PATH),
                response: HttpResponse::plain(302, Vec::new()),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("a redirect naming nowhere is not bytes");

    assert!(
        !matches!(refused, AdapterError::Ambiguous(_)),
        "never ambiguous: that is the arm that halts a tenant's inventory, and this is a \
         condition the adapter can name exactly. Got: {refused:?}"
    );
    let AdapterError::Rejected { detail, .. } = &refused else {
        panic!("a redirect with no location is a rejection, and got: {refused:?}");
    };
    assert!(
        detail.0.contains("no location"),
        "the refusal says what was missing: {detail:?}"
    );
}

/// A `Location` naming the marketplace's own origin is refused by name.
///
/// Live it arrives in the trailing-dot spelling, which reqwest's policy reads
/// as a host change and hands back rather than following. Unrefused here it
/// reaches the transport, which declines it as `NotSent` — the class the seam
/// documents as the only one safe to retry — so a permanent condition would be
/// retried against a marketplace that keeps answering the same thing.
#[test]
fn a_bundle_redirect_back_into_the_session_origin_refuses_by_name() {
    let manifest = json!({ "zipUrls": { DRAFT.0.to_string(): { "url": BUNDLE_PATH } } });
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&manifest),
            },
            Interaction {
                request: endpoints::download_bundle_request(BUNDLE_PATH),
                response: HttpResponse {
                    status: 302,
                    body: Vec::new(),
                    headers: vec![(
                        ResponseHeader::Location,
                        "https://www.tes.com./api/v2/resources/9001".to_owned(),
                    )],
                },
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("a hop back into the session origin is not bytes");

    assert!(
        !matches!(
            refused,
            AdapterError::Ambiguous(_) | AdapterError::NotSent(_)
        ),
        "neither halted nor retried: this is permanent and the adapter can name it. Got: \
         {refused:?}"
    );
    let AdapterError::Rejected { code, detail } = &refused else {
        panic!("a hop back into our own origin is a rejection, and got: {refused:?}");
    };
    assert_eq!(
        *code,
        FailureCode::UnexpectedOrigin,
        "what failed is where the location pointed"
    );
    assert!(
        detail.0.contains("own origin"),
        "the refusal says the location named us: {detail:?}"
    );
}

/// An expired signed url answers a page, and a page is not the seller's file.
///
/// This is the one that would otherwise be silent: the hop succeeds with 200,
/// the bytes are HTML, and without this check they would be uploaded to the
/// seller's other marketplace under their own name as their resource.
#[test]
fn a_signed_url_that_answers_a_page_rather_than_an_archive_is_refused() {
    let manifest = json!({ "zipUrls": { DRAFT.0.to_string(): { "url": BUNDLE_PATH } } });
    let signed = "https://d111111abcdef8.cloudfront.net/bundle?Signature=expired";
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&manifest),
            },
            Interaction {
                request: endpoints::download_bundle_request(BUNDLE_PATH),
                response: HttpResponse {
                    status: 302,
                    body: Vec::new(),
                    headers: vec![(ResponseHeader::Location, signed.to_owned())],
                },
            },
            Interaction {
                request: endpoints::redirected_bundle_request(signed.to_owned()),
                // Deliberately a page carrying the word the session-expiry
                // sniffer looks for. An expired signature answers exactly
                // this shape, and if the shared classifier saw it first the
                // run would park on a session that is perfectly good.
                response: HttpResponse::plain(
                    200,
                    b"<html><body>Please login to continue</body></html>".to_vec(),
                ),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("a page is not a bundle");

    assert!(
        !matches!(refused, AdapterError::SessionExpired),
        "the bytes decide what they are before anything reads them for what they say: a page \
         containing the word the sniffer looks for must not park the run on a good session"
    );
    let AdapterError::Rejected { code, detail } = &refused else {
        panic!("an expired signature is a rejection, and got: {refused:?}");
    };
    assert_eq!(
        *code,
        tam_types::FailureCode::VerificationMismatch,
        "what failed is that the bytes are not what they must be"
    );
    assert!(
        detail.0.contains("not an archive"),
        "the refusal says the bytes are wrong rather than that the request failed: {detail:?}"
    );
}

/// One hop, and the second redirect is where that is enforced.
#[test]
fn a_signed_url_that_redirects_again_is_refused_rather_than_followed() {
    let manifest = json!({ "zipUrls": { DRAFT.0.to_string(): { "url": BUNDLE_PATH } } });
    let signed = "https://d111111abcdef8.cloudfront.net/bundle?Signature=abc";
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&manifest),
            },
            Interaction {
                request: endpoints::download_bundle_request(BUNDLE_PATH),
                response: HttpResponse {
                    status: 302,
                    body: Vec::new(),
                    headers: vec![(ResponseHeader::Location, signed.to_owned())],
                },
            },
            Interaction {
                request: endpoints::redirected_bundle_request(signed.to_owned()),
                response: HttpResponse {
                    status: 302,
                    body: Vec::new(),
                    headers: vec![(
                        ResponseHeader::Location,
                        "https://elsewhere.example/again".to_owned(),
                    )],
                },
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("a chain this crate does not follow is not bytes");

    let AdapterError::Rejected { detail, .. } = &refused else {
        panic!("a second redirect is a rejection, and got: {refused:?}");
    };
    assert!(
        detail.0.contains("one hop"),
        "the refusal says the limit it hit, so nobody reads it as an outage: {detail:?}"
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
                inventory: InventoryId::Tes,
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
            interactions: vec![Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: ok(&json!([])),
            }],
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
        1,
        "the refusal happens before the first request: the manifest read is still \
         unconsumed, which an empty cassette could not have shown"
    );
}

/// A delete answering 404 is not a marketplace refusal. It is the ordinary
/// answer for a resource that is not on that route, which is what an
/// already-removed listing gives — and `post_delete`'s own contract is that
/// the status is not a verdict, so the driver's absence poll is what settles
/// a removal. Reading it as `UploadRejected` settled the item Failed and
/// skipped the poll, leaving the mapping bound to a listing that was gone
/// with no path left to sever it.
#[test]
fn a_delete_that_404s_is_evidence_for_the_poll_and_a_400_is_still_a_refusal() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::delete_draft_request(DRAFT),
            response: status(404),
        }],
    };
    let gone = adapter(cassette, vec![]);
    let evidence = futures::executor::block_on(gone.remove(removal_plan(ListingState::Draft), NOW))
        .unwrap_or_else(|error| panic!("a 404 delete is evidence, not a rejection: {error:?}"));
    assert_eq!(
        evidence.http_status,
        Some(404),
        "the evidence records the status verbatim; what changed is that it is no longer \
         read as a verdict"
    );

    let refused = Cassette {
        interactions: vec![Interaction {
            request: endpoints::delete_draft_request(DRAFT),
            response: status(400),
        }],
    };
    let refusing = adapter(refused, vec![]);
    let refusal =
        futures::executor::block_on(refusing.remove(removal_plan(ListingState::Draft), NOW));
    assert!(
        matches!(
            refusal,
            Err(AdapterError::Rejected {
                code: FailureCode::UploadRejected,
                ..
            })
        ),
        "only the 404 moved: a refused delete is still a refusal, so this is not the \
         classifier being opened up: {refusal:?}"
    );
}

/// The age field on the write path. The uploader offers one age field per
/// country and the two carry ids from different vocabularies, so posting a
/// year group under `ageRanges` would be a wrong field rather than a wrong
/// label: id 4 is the 11-14 band in one and 2nd grade in the other. Tes is
/// the `ageRanges` branch, and `yearGroups` never carries a written id.
#[test]
fn a_projection_posts_its_grades_as_age_ranges() {
    let tes = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let fields = tes
        .project_fields(&projected(PriceIntent::Free))
        .expect("a Tes listing projects");
    let grades: Value =
        serde_json::from_str(&entry(&fields, FieldKey::Grades)).expect("the grades are JSON");
    assert_eq!(grades["ageRanges"], serde_json::json!([4]));
    assert!(
        grades.get("yearGroups").is_none(),
        "the field set names one age field and does not mention the other"
    );
}

#[test]
fn a_grade_without_a_numeric_id_is_refused_rather_than_dropped() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        vec![],
    );
    let mut listing = projected(PriceIntent::Free);
    listing.grades.push(NativeTerm {
        native_id: None,
        segments: vec!["Key Stage 3".to_owned()],
    });
    let refused = adapter.project_fields(&listing);
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "publishing a listing with fewer grades than the seller authored, silently, is \
         the failure this refusal exists to prevent: {refused:?}"
    );
}

/// The first-party read of one resource, as the device's import reads it.
///
/// The price is the wire's own integer of minor units — the captured publish
/// body reads `"price": 500` for GBP 5.00 and the dashboard rows carry the
/// same number as `price_pence` — so it crosses without conversion. Until
/// 2026-09-07 this read multiplied it by a hundred, and the founder's £5.00
/// listings arrived as £500.00.
#[test]
fn an_import_read_carries_the_price_in_the_wires_own_minor_units() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::read_draft_request(DRAFT),
            response: ok(&json!({
                "id": 9001, "draft": false, "title": "Fractions pack",
                "descriptionRaw": "A pack.", "licence": "TES-PAID", "price": 500,
                "categories": [{ "id": 1_000_448 }], "ageRanges": [4], "mainType": 99_009
            })),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let listing = futures::executor::block_on(adapter.fetch_for_import(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect("the resource reads for import");

    assert_eq!(
        listing.price,
        tam_types::ImportedPrice::Paid {
            minor_units: 500,
            denomination: "GBP".to_owned(),
        },
        "500 on the wire is £5.00, and the denomination is the inventory's"
    );
    assert_eq!(
        listing.native_ids(TermKind::ResourceType),
        vec!["99009".to_owned()],
        "the declared resource type crosses with the read rather than being dropped"
    );
    assert_eq!(
        listing.native_ids(TermKind::Subject),
        vec!["1000448".to_owned()]
    );
}

/// A number with a fractional part is not a count of minor units, and reading
/// it as one by rounding is how a major-unit amount would pass as a hundredth
/// of itself.
#[test]
fn an_import_read_refuses_a_price_that_is_not_a_whole_number_of_minor_units() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::read_draft_request(DRAFT),
            response: ok(&json!({
                "id": 9001, "draft": false, "title": "Fractions pack",
                "licence": "TES-PAID", "price": 4.5
            })),
        }],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.fetch_for_import(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("4.5 is not a number of pence");
    let AdapterError::Rejected { detail, .. } = &refused else {
        panic!("a price this read cannot place is a refusal, and got: {refused:?}");
    };
    assert!(
        detail.0.contains("minor units"),
        "the refusal names the unit it expected: {detail:?}"
    );
}

/// The page a draft's manifest route redirects to is a Tes page, and a Tes
/// page carries the word the sign-in sniffer keys on. Read on its own that
/// page is a dead session, and the founder's 2026-09-07 import skipped both
/// of its drafts as `SessionExpired` on a session that read the next listing
/// fine. The state route settles it: a session that can read the draft is
/// alive, and the draft is what has no bundle.
#[test]
fn a_drafts_manifest_page_that_mentions_login_is_no_bundle_rather_than_a_dead_session() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: HttpResponse::plain(
                    200,
                    b"<html><a href=\"/login\">Log in</a> Page not found</html>".to_vec(),
                ),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: ok(&draft_state(9001, true)),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("a draft has no bundle");
    let AdapterError::Rejected { code, detail } = &refused else {
        panic!("an unpublished resource is a rejection naming the cause, and got: {refused:?}");
    };
    assert_eq!(*code, FailureCode::PreconditionElementAbsent);
    assert!(
        detail.0.contains("no published bundle"),
        "the seller reads why: {detail:?}"
    );
    assert_eq!(adapter.transport().remaining(), 0);
}

/// The same page on a session that really has lapsed stays a dead session,
/// because the state route cannot be read either.
#[test]
fn a_manifest_page_on_a_session_the_state_route_also_refuses_is_a_dead_session() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::download_manifest_request(DRAFT),
                response: HttpResponse::plain(200, b"<html>login</html>".to_vec()),
            },
            Interaction {
                request: endpoints::read_draft_request(DRAFT),
                response: status(401),
            },
            Interaction {
                request: endpoints::read_resource_request(DRAFT),
                response: status(401),
            },
        ],
    };
    let adapter = adapter(cassette, vec![]);
    let refused = futures::executor::block_on(adapter.download_resource_bundle(
        &FetchReason::FirstPartyExport {
            inventory: InventoryId::Tes,
        },
        DRAFT,
    ))
    .expect_err("nothing reads on a dead session");
    assert_eq!(refused, AdapterError::SessionExpired);
    assert_eq!(adapter.transport().remaining(), 0);
}
