//! The TPT read flows driven end to end against cassettes mined from the
//! 2026-08-28 HAR captures: every request the flow issues must match the
//! recording in order, and every test asserts the cassette is fully consumed,
//! so a hop that silently vanished fails too.
//!
//! Both fixtures keep the recorded envelope and nothing the seller owns. The
//! store identity, product names, slugs, prices, sale counts, asset names,
//! thumbnail urls and the analytics totals are synthetic; the wire shape, the
//! resource ids the two fixtures share, and the flat taxonomy vocabulary are
//! the capture's own, because those are what the parser is being held to. The
//! resource ids stay because they appear in public storefront urls and are
//! not the seller's to lose; what those resources earned is. The canaries in
//! `write_flows.rs` hold that line by shape.

use serde_json::{json, Value};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::{HttpResponse, ResponseHeader};
use tam_marketplace::{
    AdapterError, CanaryGrant, FetchReason, FileContent, FileSource, FileSourceError,
    FirstPartyExport, ListingState, RemoteListingId,
};
use tam_marketplace_tpt::endpoints::{
    self, AllTimeMetric, MetricResolution, ResolvedMetric, ResolvedStatsQuery, StatsWindow,
};
use tam_marketplace_tpt::read_model::ProductId;
use tam_marketplace_tpt::{listing_state_from_status, InstantPause, TptAdapter};
use tam_types::{CopyFormat, FailureCode, FileId, ImportedPrice, InventoryId, Timestamp};

/// The one reason that justifies an enumeration-shaped read.
fn export() -> FetchReason {
    FetchReason::FirstPartyExport {
        inventory: InventoryId::Tpt,
    }
}

fn ok(body: &Value) -> HttpResponse {
    HttpResponse::plain(200, body.to_string().into_bytes())
}

fn page(rows: &Value, current: u64, total: u64, pages: u64) -> Value {
    json!({"data": {"seller": {"resources": {
        "results": rows,
        "pageInfo": {
            "totalResultsCount": total,
            "currentPage": current,
            "totalPageCount": pages,
        },
    }}}})
}

/// A file source no read flow reaches. The read half of the adapter takes
/// one because the write half needs one, and a read that fetched a file
/// would be a read this crate does not perform.
struct NoFiles;

impl FileSource for NoFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Missing(file)))
    }
}

fn adapter(
    cassette: Cassette,
    page_limit: u32,
) -> TptAdapter<CassetteTransport, NoFiles, InstantPause> {
    TptAdapter::new(CassetteTransport::new(cassette), NoFiles, InstantPause)
        .with_page_limit(page_limit)
}

#[test]
fn the_catalogue_walk_pages_to_the_reported_total() {
    let cassette: Cassette =
        serde_json::from_str(include_str!("cassettes/my_product_listings.json"))
            .expect("the committed MyProductListings fixture parses");
    let adapter = adapter(cassette, 2);
    let entries = futures::executor::block_on(TptAdapter::list_own_resources(&adapter, &export()))
        .expect("the recorded two-page walk completes");
    assert_eq!(
        entries.len(),
        3,
        "the walk collects every row the pageInfo total names"
    );
    assert_eq!(
        entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        vec![
            ProductId(12_854_712),
            ProductId(11_039_236),
            ProductId(12_873_800)
        ],
        "the numeric ids come off the wire's string ids in page order"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "the fixture recorded exactly the walk's hops"
    );
}

#[test]
fn a_money_string_is_parsed_and_never_compared_as_text() {
    let cassette: Cassette =
        serde_json::from_str(include_str!("cassettes/my_product_listings.json"))
            .expect("the committed MyProductListings fixture parses");
    let entries = futures::executor::block_on(TptAdapter::list_own_resources(
        &adapter(cassette, 2),
        &export(),
    ))
    .expect("the recorded walk completes");
    let minor: Vec<i64> = entries
        .iter()
        .map(|entry| entry.price.minor_units)
        .collect();
    assert_eq!(
        minor,
        vec![1200, 0, 850],
        "$12.00, $0.00 and $8.50 order correctly only once parsed: as text $12.00 sorts below \
         $8.50"
    );
    assert!(
        entries.iter().all(|entry| entry.price.symbol == "$"),
        "the symbol is carried verbatim and never resolved to a currency"
    );
    assert!(
        entries.get(1).is_some_and(|entry| entry.is_free),
        "the free product is the one the wire flagged, not the one priced at zero"
    );
}

#[test]
fn the_flat_taxonomy_and_the_seller_shelves_come_back_verbatim() {
    let cassette: Cassette =
        serde_json::from_str(include_str!("cassettes/my_product_listings.json"))
            .expect("the committed MyProductListings fixture parses");
    let entries = futures::executor::block_on(TptAdapter::list_own_resources(
        &adapter(cassette, 2),
        &export(),
    ))
    .expect("the recorded walk completes");
    let first = entries.first().expect("the walk collected rows");
    assert!(
        first.taxonomy_tags.contains(&"4th-grade".to_owned())
            && first.taxonomy_tags.contains(&"math".to_owned())
            && first.taxonomy_tags.contains(&"pdf".to_owned()),
        "grade, subject and format share one undifferentiated namespace, got {:?}",
        first.taxonomy_tags
    );
    assert_eq!(
        first.categories.first().map(|shelf| shelf.id.clone()),
        Some("1359903".to_owned()),
        "a category id arrives as a JSON number here and a string elsewhere; both read as one"
    );
    assert_eq!(
        first.last_modified_at.as_deref(),
        Some("2025-01-19T02:01:33"),
        "the modification stamp is carried in the wire's own format, uninterpreted"
    );
}

#[test]
fn a_catalogue_read_without_the_export_capability_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        2,
    );
    let reason = FetchReason::VerifyAttempt {
        attempt: tam_marketplace::WriteAttemptId(tam_types::Uuid([3; 16])),
    };
    let refused = futures::executor::block_on(TptAdapter::list_own_resources(&adapter, &reason));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::Other,
                ..
            })
        ),
        "an enumeration is the first-party-export capability and nothing else, got {refused:?}"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "a refused read issues no request at all"
    );
}

#[test]
fn a_walk_that_runs_dry_before_the_total_refuses_rather_than_truncating() {
    let cassette = Cassette {
        interactions: vec![
            Interaction {
                request: endpoints::my_product_listings_request(2, 0),
                response: ok(&page(&json!([{"id": "1", "price": "$1.00"}]), 1, 9, 5)),
            },
            Interaction {
                request: endpoints::my_product_listings_request(2, 2),
                response: ok(&page(&json!([]), 2, 9, 5)),
            },
        ],
    };
    let adapter = adapter(cassette, 2);
    let refused = futures::executor::block_on(TptAdapter::list_own_resources(&adapter, &export()));
    let Err(AdapterError::Rejected { code, detail }) = refused else {
        panic!("a walk short of its reported total must refuse, got {refused:?}");
    };
    assert_eq!(
        code,
        FailureCode::VerificationMismatch,
        "a short catalogue is a verification failure, not a marketplace refusal"
    );
    assert!(
        detail.0.contains("truncated"),
        "the refusal says what an importer would otherwise have believed, got {detail:?}"
    );
}

#[test]
fn an_unparseable_price_fails_the_page_rather_than_dropping_the_row() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::my_product_listings_request(2, 0),
            response: ok(&page(&json!([{"id": "1", "price": "$1.0.0"}]), 1, 1, 1)),
        }],
    };
    let refused = futures::executor::block_on(TptAdapter::list_own_resources(
        &adapter(cassette, 2),
        &export(),
    ));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::VerificationMismatch,
                ..
            })
        ),
        "a row we cannot price must fail the read, not vanish from it, got {refused:?}"
    );
}

#[test]
fn a_graphql_error_envelope_answered_two_hundred_fails_the_walk() {
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::my_product_listings_request(2, 0),
            response: ok(&json!({"data": null, "errors": [{"message": "Unauthorized"}]})),
        }],
    };
    let refused = futures::executor::block_on(TptAdapter::list_own_resources(
        &adapter(cassette, 2),
        &export(),
    ));
    let Err(AdapterError::Rejected { detail, .. }) = refused else {
        panic!("a 200 carrying an errors array is not a catalogue, got {refused:?}");
    };
    assert!(
        detail.0.contains("Unauthorized"),
        "the rejection names what the service said, got {detail:?}"
    );
}

#[test]
fn the_all_time_statistics_read_maps_one_metric_per_resource() {
    let cassette: Cassette = serde_json::from_str(include_str!("cassettes/all_time_stats.json"))
        .expect("the committed statistics fixture parses");
    let adapter = adapter(cassette, 2);
    let ids = [
        ProductId(11_039_236),
        ProductId(11_050_756),
        ProductId(12_854_516),
    ];
    let stats = futures::executor::block_on(adapter.all_time_stats(
        &export(),
        AllTimeMetric::SalesCount,
        &ids,
    ))
    .expect("the recorded batch replays");
    assert_eq!(
        stats.iter().map(|stat| stat.resource).collect::<Vec<_>>(),
        ids.to_vec(),
        "every id in the batch comes back, in the ranking the gateway returned"
    );
    assert_eq!(
        stats
            .iter()
            .map(|stat| stat.total_value)
            .collect::<Vec<_>>(),
        vec![111.0, 0.0, 222.0],
        "the counts survive the Relay envelope in the ranking it returned, and the zero among \
         them survives as a zero rather than being dropped as an absent node"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "one batch is one request"
    );
}

#[test]
fn the_time_resolved_statistics_read_asks_the_gateway_for_a_window() {
    let query = ResolvedStatsQuery {
        metric: ResolvedMetric::ResourceViews,
        resolution: MetricResolution::Day,
        // The window the recorded StoreResources request carried, in the unix
        // seconds every gateway timestamp uses.
        window: StatsWindow {
            from_unix_seconds: 1_785_556_800,
            to_unix_seconds: 1_787_975_999,
        },
    };
    let ids = [ProductId(12_854_712)];
    // The Relay envelope the recorded StoreResources response answers with,
    // reduced to the fields this adapter selects and carrying a synthetic
    // total: what the seller's resources actually earned or were viewed is
    // theirs, and no assertion here depends on the figure.
    let response = ok(&json!({"data": {"totals": {"edges": [
        {"cursor": "cmFuazox", "node": {"resourceId": "12854712", "totalValue": 333}}
    ]}}}));
    let cassette = Cassette {
        interactions: vec![Interaction {
            request: endpoints::resolved_stats_request(query, &ids),
            response,
        }],
    };
    let adapter = adapter(cassette, 2);
    let stats = futures::executor::block_on(adapter.resolved_stats(&export(), query, &ids))
        .expect("the resolved batch replays");
    assert_eq!(
        stats.first().map(|stat| stat.resource),
        Some(ProductId(12_854_712)),
        "the resolved read maps per resource exactly as the all-time read does"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "one window is one request"
    );
}

#[test]
fn a_statistics_batch_outside_the_proven_bounds_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        2,
    );
    let empty = futures::executor::block_on(adapter.all_time_stats(
        &export(),
        AllTimeMetric::Earnings,
        &[],
    ));
    assert!(
        matches!(empty, Err(AdapterError::Rejected { .. })),
        "a statistics read names the resources it is about, got {empty:?}"
    );
    let oversized: Vec<ProductId> = (0..=u64::try_from(endpoints::STATS_BATCH_MAX).unwrap_or(100))
        .map(ProductId)
        .collect();
    let refused = futures::executor::block_on(adapter.all_time_stats(
        &export(),
        AllTimeMetric::Earnings,
        &oversized,
    ));
    let Err(AdapterError::Rejected { detail, .. }) = refused else {
        panic!("a batch past the proven ceiling must refuse, got {refused:?}");
    };
    assert!(
        detail.0.contains("untested"),
        "the refusal says the ceiling is unproven rather than asserting one, got {detail:?}"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "neither refusal reaches the network"
    );
}

#[test]
fn a_statistics_read_without_the_export_capability_is_refused() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        2,
    );
    let reason = FetchReason::VerifyAttempt {
        attempt: tam_marketplace::WriteAttemptId(tam_types::Uuid([4; 16])),
    };
    let refused = futures::executor::block_on(adapter.all_time_stats(
        &reason,
        AllTimeMetric::Earnings,
        &[ProductId(1)],
    ));
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "analytics are a first-party read like the catalogue, got {refused:?}"
    );
}

/// The product and slug every download test uses. Both are synthetic: no
/// seller owns this id, and the slug names no real resource. What is the
/// capture's own here is the url shape and the archive's leading bytes.
const DOWNLOADED: ProductId = ProductId(90_000_042);
const DOWNLOADED_SLUG: &str = "Sample-Fractions-Pack-90000042";

/// A synthetic archive. Only its first four bytes matter to the classifier,
/// and those four are what every ZIP opens with.
fn archive() -> Vec<u8> {
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(b"synthetic archive body");
    bytes
}

fn catalogue_row() -> Value {
    json!([{
        "id": DOWNLOADED.0.to_string(),
        "price": "$4.00",
        "canonicalSlug": DOWNLOADED_SLUG,
    }])
}

/// The read that names the slug the download url is built from.
fn slug_read(response: HttpResponse) -> Vec<Interaction> {
    vec![
        Interaction {
            request: endpoints::product_by_id_request(DOWNLOADED),
            response: ok(&page(&catalogue_row(), 1, 1, 1)),
        },
        Interaction {
            request: endpoints::download_bundle_request(DOWNLOADED_SLUG, DOWNLOADED),
            response,
        },
    ]
}

/// G-O3. The download is the product page's own control reproduced: the
/// catalogue read names the slug, and the archive comes back from
/// `/Download/{slug}-{id}` under the session.
///
/// Cassette-verified, and live-unverified on purpose. A 2026-08-29 probe of
/// the founder's own product met a redirect to the sign-in gate with a jar
/// that authenticates every other hop this crate makes, so no live run can
/// witness this path until a session carries the browser clearance it is
/// gated on.
#[test]
fn the_download_fetches_the_sellers_own_archive_through_the_page_control() {
    let adapter = adapter(
        Cassette {
            interactions: slug_read(HttpResponse::plain(200, archive())),
        },
        1,
    );
    let bundle = futures::executor::block_on(FirstPartyExport::download_resource_bundle(
        &adapter,
        &export(),
        DOWNLOADED,
    ))
    .expect("the recorded download yields the archive");
    assert_eq!(bundle, archive(), "the bytes cross untouched");
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "two hops and no others: the slug read and the download itself"
    );
}

/// The reason an archive is asserted positively rather than inferred from a
/// status. An ungated download answers a sign-in page at 200, and a status
/// check alone would hand that HTML to the importer and store it as the
/// seller's product.
#[test]
fn a_sign_in_page_answering_two_hundred_is_not_a_bundle() {
    let page = b"<html><head><title>Sign In</title></head><body>sign-in</body></html>".to_vec();
    let adapter = adapter(
        Cassette {
            interactions: slug_read(HttpResponse::plain(200, page)),
        },
        1,
    );
    let refused = futures::executor::block_on(FirstPartyExport::download_resource_bundle(
        &adapter,
        &export(),
        DOWNLOADED,
    ));
    let Err(AdapterError::Rejected { code, detail }) = refused else {
        panic!("a page where the archive should be is a rejection, got {refused:?}");
    };
    assert_eq!(
        code,
        FailureCode::ChallengePresented,
        "the gate is a browser clearance, not a lapsed session, so the remedy named is not \
         a re-auth"
    );
    assert!(
        detail.0.contains("clearance"),
        "and the refusal says what the session is missing, got {}",
        detail.0
    );
}

/// The condition the live probe actually met: a 302 to the sign-in gate, for
/// a jar that reads and writes everything else.
#[test]
fn a_download_redirected_to_the_sign_in_gate_names_the_clearance_it_needs() {
    let redirect = HttpResponse {
        status: 302,
        body: Vec::new(),
        headers: vec![(
            ResponseHeader::Location,
            "/Request-Authorization?authModal=login".to_owned(),
        )],
    };
    let adapter = adapter(
        Cassette {
            interactions: slug_read(redirect),
        },
        1,
    );
    let refused = futures::executor::block_on(FirstPartyExport::download_resource_bundle(
        &adapter,
        &export(),
        DOWNLOADED,
    ));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::ChallengePresented,
                ..
            })
        ),
        "the 2026-08-29 probe met exactly this, and it is not a condition a credential \
         refresh clears: {refused:?}"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "and the gate is read from the redirect rather than followed"
    );
}

/// A redirect off the origin is refused rather than followed. No capture
/// carries a signed download hop, so its shape is unknown; the transport
/// would refuse a session request to another host anyway, and inventing the
/// request that satisfies one is not a substitute for capturing it.
#[test]
fn a_download_redirected_off_the_origin_refuses_rather_than_guessing_the_hop() {
    let redirect = HttpResponse {
        status: 302,
        body: Vec::new(),
        headers: vec![(
            ResponseHeader::Location,
            "https://files.example.invalid/signed/object.zip".to_owned(),
        )],
    };
    let adapter = adapter(
        Cassette {
            interactions: slug_read(redirect),
        },
        1,
    );
    let refused = futures::executor::block_on(FirstPartyExport::download_resource_bundle(
        &adapter,
        &export(),
        DOWNLOADED,
    ));
    assert!(
        matches!(
            refused,
            Err(AdapterError::Rejected {
                code: FailureCode::UnexpectedOrigin,
                ..
            })
        ),
        "an uncaptured signed hop is named, not improvised: {refused:?}"
    );
}

/// The same gate every other first-party read answers to.
#[test]
fn a_download_under_any_other_reason_sends_nothing() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        1,
    );
    let refused = futures::executor::block_on(FirstPartyExport::download_resource_bundle(
        &adapter,
        &FetchReason::StructuralProbe {
            grant: CanaryGrant {
                inventory: InventoryId::Tpt,
                decided_at: Timestamp(0),
            },
        },
        DOWNLOADED,
    ));
    assert!(
        matches!(refused, Err(AdapterError::Rejected { .. })),
        "a file download is justified only by the first-party-export capability, got {refused:?}"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "and nothing left before the refusal"
    );
}

#[test]
fn the_adapter_names_the_one_tpt_inventory() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        2,
    );
    assert_eq!(
        adapter.inventory(),
        InventoryId::Tpt,
        "one crate, one inventory, per the design's crate table"
    );
}

#[test]
fn the_import_read_yields_the_product_whole_and_states_what_it_did_not_carry() {
    let cassette: Cassette =
        serde_json::from_str(include_str!("cassettes/upload_page_product.json"))
            .expect("the committed UploadPageProductQuery fixture parses");
    let adapter = adapter(cassette, 1);
    let listing = futures::executor::block_on(TptAdapter::fetch_for_import(
        &adapter,
        &export(),
        ProductId(13_042_099),
    ))
    .expect("the recorded read canonicalises");

    assert_eq!(
        listing.remote,
        RemoteListingId::Tpt {
            product_id: 13_042_099
        },
        "the id is the caller's, because the response row carries none"
    );
    assert!(
        listing.body.starts_with("<p>"),
        "TPT stores and returns the body as HTML, which the catalogue read does not carry at all"
    );
    assert_eq!(
        listing.body_format,
        CopyFormat::Html,
        "declared rather than sniffed: guessing a body's format from its bytes is how a \
         listing acquires escaped markup nobody asked for"
    );
    assert_eq!(
        listing.price,
        ImportedPrice::Paid {
            minor_units: 300,
            denomination: "USD".to_owned(),
        },
        "TPT sells in USD and offers no other currency, so the denomination is the \
         inventory's rule rather than a reading of the dollar sign beside the amount"
    );
    assert_eq!(
        listing.rights, None,
        "TPT binds no licence field anywhere on its wire, which is the measured absence the \
         registry records rather than a gap in this read"
    );
    assert_eq!(
        listing.state, None,
        "the fixture is the capture's own selection set, which predates the status field, so \
         the honest answer is that the read did not carry it -- and a migrate's removal \
         refuses on None rather than deleting against a lifecycle nobody observed"
    );
}

#[test]
fn every_imported_value_travels_untagged_because_the_axis_is_the_relations_fact() {
    let cassette: Cassette =
        serde_json::from_str(include_str!("cassettes/upload_page_product.json"))
            .expect("the committed UploadPageProductQuery fixture parses");
    let listing = futures::executor::block_on(TptAdapter::fetch_for_import(
        &adapter(cassette, 1),
        &export(),
        ProductId(13_042_099),
    ))
    .expect("the recorded read canonicalises");

    assert!(
        listing.native.iter().all(|term| term.kind.is_none()),
        "a grade, a subject, a resource type and an audience are the same kind of thing in \
         TPT's flat namespace, and tagging one here would assert an axis binding this \
         adapter cannot know"
    );
    assert_eq!(
        listing
            .native
            .iter()
            .filter_map(|term| term.native_id.clone())
            .collect::<Vec<_>>(),
        vec![
            "4th-grade".to_owned(),
            "5th-grade".to_owned(),
            "6th-grade".to_owned(),
            "homeschool".to_owned(),
            "homeschool-curricula".to_owned(),
            "math".to_owned(),
            "unit-plans".to_owned(),
            "worksheets".to_owned(),
            "1368989".to_owned(),
            "1368990".to_owned(),
        ],
        "the eight facet slugs the capture carried, then the seller's own two shelves"
    );
}

#[test]
fn the_draft_line_is_read_off_status_and_refuses_a_value_nobody_has_observed() {
    // 908 observations across three captures carry two values. read_back's
    // lenient else-arm reads anything else as Draft, which is right for a
    // comparison and wrong here: a removal that guesses Draft posts a delete
    // to the wrong route.
    assert_eq!(
        listing_state_from_status("ACTIVE"),
        Some(ListingState::Live)
    );
    assert_eq!(
        listing_state_from_status("NOT_ACTIVE"),
        Some(ListingState::Draft)
    );
    assert_eq!(
        listing_state_from_status("PENDING"),
        None,
        "a third value would be a state nobody has seen, and a migrate refuses on it"
    );
}

#[test]
fn an_import_read_is_refused_without_the_first_party_export_capability() {
    let cassette: Cassette =
        serde_json::from_str(include_str!("cassettes/upload_page_product.json"))
            .expect("the committed UploadPageProductQuery fixture parses");
    let adapter = adapter(cassette, 1);
    let refused = futures::executor::block_on(TptAdapter::fetch_for_import(
        &adapter,
        &FetchReason::StructuralProbe {
            grant: CanaryGrant {
                inventory: InventoryId::Tpt,
                decided_at: Timestamp(0),
            },
        },
        ProductId(13_042_099),
    ));
    assert!(
        refused.is_err(),
        "an enumeration-shaped read of the seller's own data is the tier-one capability and \
         nothing else justifies one"
    );
    assert_eq!(
        adapter.transport().remaining(),
        1,
        "the refusal precedes the request rather than following it"
    );
}
