//! The analytics capture pass: the shortlist of metrics it reads, the
//! batching the marketplace's own cap dictates, the fan-out across metrics,
//! and the snapshot rows the answers become.
//!
//! Everything here is driven by a caller that supplies the reader, the
//! listings and the instant, so the whole orchestration is exercisable without
//! a socket. The binary beside it owns the pools, the broker lease and the
//! clock, which is the only part that cannot be.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use tam_engine_driver::vocabulary::{BoundListing, MetricSnapshot};
use tam_marketplace::transport::Transport;
use tam_marketplace::{AdapterError, FetchReason, FileSource, Pause};
use tam_marketplace_tpt::endpoints::STATS_BATCH_MAX;
use tam_marketplace_tpt::{AllTimeMetric, ProductId, ResourceStat, TptAdapter};
use tam_types::{InventoryId, MappingId, Timestamp};

/// The metrics a pass captures, each with the name it is stored under.
///
/// A shortlist off the twelve all-time metrics the adapter names, because one
/// request reads one metric for one batch: the full set is twelve times the
/// request volume against a marketplace we would rather touch less. The stored
/// names are written out rather than derived from the enum, so widening the
/// shortlist is a decision taken here and the column's vocabulary never moves
/// because a Rust identifier was renamed.
pub const CAPTURED_METRICS: [(AllTimeMetric, &str); 3] = [
    (AllTimeMetric::SalesCount, "sales_count"),
    (AllTimeMetric::Earnings, "earnings"),
    (AllTimeMetric::ResourceViews, "resource_views"),
];

/// How long a snapshot is kept. Retention lives here rather than in
/// `tam-limits` because the capture is the only pass that writes or erases
/// these rows; promoting it to the shared limits table is a founder decision
/// and not a consequence of this crate existing.
///
/// SIZED to hold a full year of comparison plus the month a seller needs to
/// compare this year's season against last year's on the day it starts.
pub const SNAPSHOT_RETENTION_DAYS: i64 = 400;

/// How many rows one retention pass erases. Local for the same reason the
/// window above is.
///
/// SIZED against what a pass can create: listings times three metrics times
/// one capture a day, so this clears far more than a day's growth for any
/// tenant count this fleet will see before the limit is revisited, while
/// bounding the lock footprint of a single statement.
pub const SNAPSHOT_PRUNE_BATCH: i64 = 10_000;

const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// Snapshots observed before this are prunable. Either arithmetic failure
/// clamps to the epoch, which prunes nothing: an overflow must not be able to
/// erase a seller's history.
#[must_use]
pub fn retention_cutoff(now: Timestamp) -> Timestamp {
    let millis = SNAPSHOT_RETENTION_DAYS
        .checked_mul(MILLIS_PER_DAY)
        .and_then(|window| now.0.checked_sub(window))
        .unwrap_or(0);
    Timestamp(millis.max(0))
}

/// The read a capture declares itself as: the marketplace's own first-party
/// export of the seller's data, which is the only tier that admits a
/// statistics read at all.
#[must_use]
pub const fn capture_reason() -> FetchReason {
    FetchReason::FirstPartyExport {
        inventory: InventoryId::Tpt,
    }
}

/// The one adapter capability a capture needs.
///
/// A local trait rather than a use of `TptAdapter` directly, so the fan-out
/// below runs against a fake in a unit test. It is deliberately not a
/// promotion of `all_time_stats` onto the `MarketplaceAdapter` seam: TPT is
/// still the only platform whose analytics can be read, and this trait exists
/// for testability rather than to claim a second implementor.
pub trait StatsReader {
    fn all_time_stats(
        &self,
        reason: &FetchReason,
        metric: AllTimeMetric,
        ids: &[ProductId],
    ) -> impl core::future::Future<Output = Result<Vec<ResourceStat>, AdapterError>>;
}

impl<T: Transport, F: FileSource, P: Pause> StatsReader for TptAdapter<T, F, P> {
    fn all_time_stats(
        &self,
        reason: &FetchReason,
        metric: AllTimeMetric,
        ids: &[ProductId],
    ) -> impl core::future::Future<Output = Result<Vec<ResourceStat>, AdapterError>> {
        TptAdapter::all_time_stats(self, reason, metric, ids)
    }
}

/// The TPT products this tenant's bound mappings name, indexed by the
/// identifier the marketplace answers with.
///
/// A binding on another marketplace is skipped rather than refused: the caller
/// asks the mapping repo for one inventory, so a foreign identifier here would
/// be a corrupt row, and dropping it costs one listing's figures rather than
/// the tenant's whole pass.
#[must_use]
pub fn tpt_resources(listings: &[BoundListing]) -> BTreeMap<u64, MappingId> {
    listings
        .iter()
        .filter_map(|listing| match listing.remote {
            tam_marketplace::RemoteListingId::Tpt { product_id } => {
                Some((product_id, listing.mapping))
            }
            tam_marketplace::RemoteListingId::Tes { .. }
            | tam_marketplace::RemoteListingId::Etsy { .. } => None,
        })
        .collect()
}

/// The resource ids of an index, in the order a request sends them.
#[must_use]
pub fn resource_ids(index: &BTreeMap<u64, MappingId>) -> Vec<ProductId> {
    index.keys().copied().map(ProductId).collect()
}

/// Splits resource ids into requests at the batch size proven on the wire.
///
/// The adapter refuses an over-large batch rather than attempting one, so this
/// is what keeps a tenant with more than a hundred listings from failing every
/// pass instead of taking two requests.
#[must_use]
pub fn batches(ids: &[ProductId]) -> Vec<&[ProductId]> {
    ids.chunks(STATS_BATCH_MAX).collect()
}

/// Turns one metric's answers into snapshot rows.
///
/// A stat naming a resource the index does not hold is dropped. The gateway
/// answers about the resources it was asked about, so such a row is either a
/// listing unbound between the read and the answer or an echo we did not ask
/// for; writing it would need a mapping id that does not exist.
#[must_use]
pub fn snapshot_rows(
    index: &BTreeMap<u64, MappingId>,
    metric: &str,
    stats: &[ResourceStat],
    observed_at: Timestamp,
) -> Vec<MetricSnapshot> {
    stats
        .iter()
        .filter_map(|stat| {
            index.get(&stat.resource.0).map(|mapping| MetricSnapshot {
                mapping: *mapping,
                metric: metric.to_owned(),
                observed_at: observed_at.0,
                total_value: stat.total_value,
            })
        })
        .collect()
}

/// One metric's read that did not answer, named so a pass can report what it
/// is missing rather than presenting a partial capture as a whole one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFailure {
    pub metric: &'static str,
    pub error: AdapterError,
}

impl core::fmt::Display for CaptureFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: adapter: {:?}", self.metric, self.error)
    }
}

/// What one tenant's pass produced: the rows to write, and the reads that
/// did not answer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CaptureOutcome {
    pub snapshots: Vec<MetricSnapshot>,
    pub failures: Vec<CaptureFailure>,
}

/// Reads every captured metric over every batch of one tenant's listings.
///
/// A failed read costs its own metric and batch and nothing else. The
/// alternative — abandoning the tenant on the first refusal — would make a
/// single rate-limited request throw away the figures already in hand, and the
/// table is append-only, so a partial pass is a smaller gap in one series
/// rather than a corrupt row anywhere.
///
/// Every row carries the one `observed_at` the caller supplied, so the whole
/// pass reads as the single instant it is rather than as a smear across
/// however long the requests took.
pub async fn capture_listings<R: StatsReader>(
    reader: &R,
    listings: &[BoundListing],
    observed_at: Timestamp,
) -> CaptureOutcome {
    let index = tpt_resources(listings);
    let ids = resource_ids(&index);
    let reason = capture_reason();
    let mut outcome = CaptureOutcome::default();
    for (metric, name) in CAPTURED_METRICS {
        for batch in batches(&ids) {
            match reader.all_time_stats(&reason, metric, batch).await {
                Ok(stats) => {
                    outcome
                        .snapshots
                        .extend(snapshot_rows(&index, name, &stats, observed_at));
                }
                Err(error) => outcome.failures.push(CaptureFailure {
                    metric: name,
                    error,
                }),
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::{
        batches, capture_listings, resource_ids, retention_cutoff, snapshot_rows, tpt_resources,
        AllTimeMetric, CaptureOutcome, FetchReason, ProductId, ResourceStat, StatsReader,
        CAPTURED_METRICS, SNAPSHOT_RETENTION_DAYS, STATS_BATCH_MAX,
    };
    use std::collections::BTreeMap;
    use tam_engine_driver::vocabulary::BoundListing;
    use tam_marketplace::{AdapterError, RemoteListingId};
    use tam_types::{FailureCode, FailureDetail, MappingId, Timestamp, Uuid};

    const NOW: Timestamp = Timestamp(1_756_512_000_000);

    fn bound(mapping: u8, product_id: u64) -> BoundListing {
        BoundListing {
            mapping: MappingId(Uuid([mapping; 16])),
            remote: RemoteListingId::Tpt { product_id },
        }
    }

    fn tes(mapping: u8) -> BoundListing {
        BoundListing {
            mapping: MappingId(Uuid([mapping; 16])),
            remote: RemoteListingId::Tes {
                url: "https://example.test/resource/1".to_owned(),
            },
        }
    }

    /// Answers a fixed value for every id it is asked about, recording the
    /// requests so a test can state how the pass split them.
    struct Echo {
        value: f64,
        seen: core::cell::RefCell<Vec<(AllTimeMetric, usize)>>,
    }

    impl Echo {
        fn new(value: f64) -> Self {
            Self {
                value,
                seen: core::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl StatsReader for Echo {
        async fn all_time_stats(
            &self,
            reason: &FetchReason,
            metric: AllTimeMetric,
            ids: &[ProductId],
        ) -> Result<Vec<ResourceStat>, AdapterError> {
            assert!(
                matches!(reason, FetchReason::FirstPartyExport { .. }),
                "a capture reads as a first-party export and nothing else"
            );
            self.seen.borrow_mut().push((metric, ids.len()));
            Ok(ids
                .iter()
                .map(|id| ResourceStat {
                    resource: *id,
                    total_value: self.value,
                })
                .collect())
        }
    }

    /// Refuses one named metric and answers the rest.
    struct RefusesOne(&'static str);

    impl StatsReader for RefusesOne {
        async fn all_time_stats(
            &self,
            _reason: &FetchReason,
            metric: AllTimeMetric,
            ids: &[ProductId],
        ) -> Result<Vec<ResourceStat>, AdapterError> {
            let refused = CAPTURED_METRICS
                .iter()
                .any(|(candidate, name)| *candidate == metric && *name == self.0);
            if refused {
                return Err(AdapterError::Rejected {
                    code: FailureCode::Other,
                    detail: FailureDetail("the gateway said no".to_owned()),
                });
            }
            Ok(ids
                .iter()
                .map(|id| ResourceStat {
                    resource: *id,
                    total_value: 1.0,
                })
                .collect())
        }
    }

    #[test]
    fn only_tpt_bindings_enter_the_index() {
        let index = tpt_resources(&[bound(1, 11), tes(2), bound(3, 33)]);
        assert_eq!(
            index.keys().copied().collect::<Vec<_>>(),
            vec![11, 33],
            "a binding on another marketplace names no TPT resource and is dropped"
        );
        assert_eq!(
            index.get(&33),
            Some(&MappingId(Uuid([3; 16]))),
            "each resource id routes back to the mapping that binds it"
        );
    }

    #[test]
    fn a_batch_never_exceeds_the_size_proven_on_the_wire() {
        let ids: Vec<ProductId> = (0..251).map(ProductId).collect();
        let split = batches(&ids);
        assert_eq!(
            split.len(),
            3,
            "251 ids is three requests at a hundred each"
        );
        assert!(
            split.iter().all(|batch| batch.len() <= STATS_BATCH_MAX),
            "the adapter refuses a larger batch rather than attempting one"
        );
        assert_eq!(
            split.iter().map(|batch| batch.len()).sum::<usize>(),
            ids.len(),
            "every id is asked about exactly once"
        );
    }

    #[test]
    fn an_empty_listing_set_issues_no_request() {
        assert!(
            batches(&[]).is_empty(),
            "the adapter refuses an empty batch, so a tenant with no listings asks nothing"
        );
    }

    #[test]
    fn a_stat_naming_an_unknown_resource_is_dropped() {
        let index = tpt_resources(&[bound(1, 11)]);
        let rows = snapshot_rows(
            &index,
            "sales_count",
            &[
                ResourceStat {
                    resource: ProductId(11),
                    total_value: 17.0,
                },
                ResourceStat {
                    resource: ProductId(99),
                    total_value: 4.0,
                },
            ],
            NOW,
        );
        assert_eq!(
            rows.len(),
            1,
            "only the resource the index names is written"
        );
        assert_eq!(
            (rows[0].mapping, rows[0].total_value, rows[0].observed_at),
            (MappingId(Uuid([1; 16])), 17.0, NOW.0),
            "the row carries the mapping, the untouched value and the pass's instant"
        );
    }

    #[tokio::test]
    async fn a_pass_writes_one_row_per_listing_per_metric() {
        let reader = Echo::new(7.0);
        let listings = [bound(1, 11), bound(2, 22)];
        let outcome = capture_listings(&reader, &listings, NOW).await;
        assert!(outcome.failures.is_empty(), "nothing refused");
        assert_eq!(
            outcome.snapshots.len(),
            listings.len() * CAPTURED_METRICS.len(),
            "two listings and three metrics is six rows"
        );
        let metrics: std::collections::BTreeSet<&str> = outcome
            .snapshots
            .iter()
            .map(|snapshot| snapshot.metric.as_str())
            .collect();
        assert_eq!(
            metrics,
            CAPTURED_METRICS
                .iter()
                .map(|(_, name)| *name)
                .collect::<std::collections::BTreeSet<_>>(),
            "every captured metric is stored under the name the shortlist gives it"
        );
        assert!(
            outcome
                .snapshots
                .iter()
                .all(|snapshot| snapshot.observed_at == NOW.0),
            "one pass is one instant, however long its requests took"
        );
    }

    #[tokio::test]
    async fn the_fan_out_asks_once_per_metric_per_batch() {
        let reader = Echo::new(1.0);
        let listings: Vec<BoundListing> = (0..150_u64)
            .map(|index| BoundListing {
                mapping: MappingId(Uuid([u8::try_from(index % 251).unwrap_or(0); 16])),
                remote: RemoteListingId::Tpt {
                    product_id: index + 1,
                },
            })
            .collect();
        let outcome = capture_listings(&reader, &listings, NOW).await;
        let seen = reader.seen.borrow();
        assert_eq!(
            seen.len(),
            CAPTURED_METRICS.len() * 2,
            "150 listings is two batches, asked for each of the three metrics"
        );
        assert_eq!(
            seen.iter().map(|(_, size)| *size).collect::<Vec<_>>(),
            vec![100, 50, 100, 50, 100, 50],
            "each metric is asked over the same split"
        );
        assert!(!outcome.snapshots.is_empty(), "the pass produced rows");
    }

    #[tokio::test]
    async fn a_refused_metric_costs_only_itself() {
        let outcome = capture_listings(&RefusesOne("earnings"), &[bound(1, 11)], NOW).await;
        assert_eq!(outcome.failures.len(), 1, "exactly one read was refused");
        assert_eq!(
            outcome.failures[0].metric, "earnings",
            "the failure names the metric that did not answer"
        );
        assert_eq!(
            outcome.snapshots.len(),
            CAPTURED_METRICS.len() - 1,
            "the metrics that did answer are still written"
        );
        assert!(
            outcome
                .snapshots
                .iter()
                .all(|snapshot| snapshot.metric != "earnings"),
            "no row is fabricated for the metric that failed"
        );
    }

    #[tokio::test]
    async fn a_tenant_with_no_bound_listings_captures_nothing() {
        let outcome = capture_listings(&Echo::new(1.0), &[tes(1)], NOW).await;
        assert_eq!(
            outcome,
            CaptureOutcome::default(),
            "no TPT listing means no request and no row, not an error"
        );
    }

    #[test]
    fn the_retention_cutoff_is_the_window_behind_the_instant_given() {
        let day = 24 * 60 * 60 * 1000;
        assert_eq!(
            retention_cutoff(Timestamp(SNAPSHOT_RETENTION_DAYS * day + 5)),
            Timestamp(5),
            "the cutoff sits exactly one window behind"
        );
    }

    #[test]
    fn a_clock_inside_the_window_prunes_nothing() {
        assert_eq!(
            retention_cutoff(Timestamp(0)),
            Timestamp(0),
            "an underflow clamps to the epoch, which erases nothing"
        );
        assert_eq!(
            retention_cutoff(Timestamp(i64::MIN)),
            Timestamp(0),
            "so does an arithmetic failure"
        );
    }

    #[test]
    fn the_index_is_asked_about_in_a_stable_order() {
        let index: BTreeMap<u64, MappingId> = tpt_resources(&[bound(3, 33), bound(1, 11)]);
        assert_eq!(
            resource_ids(&index),
            vec![ProductId(11), ProductId(33)],
            "requests are byte-identical across passes with the same listings"
        );
    }
}
