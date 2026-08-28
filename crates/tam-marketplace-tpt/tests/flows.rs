//! The TPT read flows driven end to end against cassettes mined from the
//! 2026-08-28 HAR captures: every request the flow issues must match the
//! recording in order, and every test asserts the cassette is fully consumed,
//! so a hop that silently vanished fails too.
//!
//! The catalogue fixture keeps the recorded envelope and nothing the seller
//! owns. The store identity, product names, slugs, prices, sale counts, asset
//! names and thumbnail urls are synthetic; the wire shape, the resource ids
//! the analytics fixture shares, and the flat taxonomy vocabulary are the
//! capture's own, because those are what the parser is being held to. The
//! canaries in `write_flows.rs` hold that line by shape.

use serde_json::{json, Value};
use tam_marketplace::cassette::{Cassette, CassetteTransport, Interaction};
use tam_marketplace::transport::HttpResponse;
use tam_marketplace::{
    AdapterError, FetchReason, FileContent, FileSource, FileSourceError, FirstPartyExport,
};
use tam_marketplace_tpt::endpoints::{
    self, AllTimeMetric, MetricResolution, ResolvedMetric, ResolvedStatsQuery, StatsWindow,
};
use tam_marketplace_tpt::read_model::ProductId;
use tam_marketplace_tpt::{InstantPause, TptAdapter};
use tam_types::{FailureCode, FileId, InventoryId};

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
        vec![2.0, 3.0, 0.0],
        "the recorded sales counts survive the Relay envelope"
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
    // reduced to the fields this adapter selects.
    let response = ok(&json!({"data": {"totals": {"edges": [
        {"cursor": "cmFuazox", "node": {"resourceId": "12854712", "totalValue": 41}}
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

#[test]
fn the_two_deferred_capabilities_report_themselves_as_uncaptured() {
    let adapter = adapter(
        Cassette {
            interactions: vec![],
        },
        2,
    );
    let bundle = futures::executor::block_on(FirstPartyExport::download_resource_bundle(
        &adapter,
        &export(),
        ProductId(12_854_712),
    ));
    assert_eq!(
        bundle,
        Err(AdapterError::Uncaptured {
            capability: tam_marketplace_tpt::flows::BUNDLE_DOWNLOAD,
        }),
        "no capture contains a TPT download, and an absent capability is not a refusal"
    );
    let import = futures::executor::block_on(FirstPartyExport::fetch_for_import(
        &adapter,
        &export(),
        ProductId(12_854_712),
    ));
    assert_eq!(
        import,
        Err(AdapterError::Uncaptured {
            capability: tam_marketplace_tpt::flows::IMPORT_CANONICALISATION,
        }),
        "canonicalisation waits on the import-run generalisation, and says so"
    );
    assert_eq!(
        adapter.transport().remaining(),
        0,
        "a deferred capability sends nothing"
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
