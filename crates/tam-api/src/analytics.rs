//! The analytics summary: the newest captured figure for every listing this
//! organisation has captured one for.
//!
//! Org-scoped through [`OrgContext`] like every other route, so the request
//! carries no organisation identifier a caller could substitute. A tenant
//! whose first capture has not run answers an empty list, which is honest and
//! is what a first render needs.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{AnalyticsRepo, LatestMetric};
use tam_types::{InventoryId, MappingId, Timestamp};

use crate::error::APIError;
use crate::{AppState, OrgContext};

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalyticsSummary {
    pub listings: Vec<ListingMetricsView>,
}

/// One listing's newest figures.
///
/// `observed_at` travels per listing rather than once per response, so the
/// console states how stale a figure is instead of presenting a captured
/// number as a live one.
///
/// The metric names are the strings the capture stored rather than a closed
/// vocabulary. The captured set is a shortlist off the twelve the adapter
/// names, and a client that renders the keys it recognises keeps working when
/// the shortlist widens; a closed union here would make widening it a
/// coordinated release.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListingMetricsView {
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub observed_at: Timestamp,
    pub metrics: BTreeMap<String, f64>,
}

/// Folds one row per (mapping, metric) into one row per mapping.
///
/// The listing's instant is the *oldest* of the figures it carries, not the
/// newest. A pass stamps every metric it captures with one instant, so the two
/// agree whenever a pass completed; they diverge exactly when one metric's
/// last successful read is older than another's, and reporting the newest
/// there would present a stale number under a fresh instant — the one thing
/// this field exists to prevent. The oldest understates the freshness of the
/// rest, which is the error that cannot mislead.
#[must_use]
pub fn summarise(rows: Vec<LatestMetric>) -> Vec<ListingMetricsView> {
    let mut by_mapping: BTreeMap<[u8; 16], ListingMetricsView> = BTreeMap::new();
    for row in rows {
        let entry = by_mapping
            .entry(row.mapping.0 .0)
            .or_insert_with(|| ListingMetricsView {
                mapping: row.mapping,
                inventory: row.inventory,
                observed_at: row.observed_at,
                metrics: BTreeMap::new(),
            });
        entry.observed_at = entry.observed_at.min(row.observed_at);
        entry.metrics.insert(row.metric, row.total_value);
    }
    by_mapping.into_values().collect()
}

pub(crate) async fn analytics_summary(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<AnalyticsSummary>, APIError> {
    let rows = AnalyticsRepo::new(state.pool.clone())
        .latest(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(AnalyticsSummary {
        listings: summarise(rows),
    }))
}

#[cfg(test)]
mod tests {
    use super::summarise;
    use tam_storage::LatestMetric;
    use tam_types::{InventoryId, MappingId, Timestamp, Uuid};

    fn row(mapping: u8, metric: &str, at: i64, value: f64) -> LatestMetric {
        LatestMetric {
            mapping: MappingId(Uuid([mapping; 16])),
            inventory: InventoryId::Tpt,
            metric: metric.to_owned(),
            observed_at: Timestamp(at),
            total_value: value,
        }
    }

    #[test]
    fn every_metric_of_one_listing_lands_in_one_row() {
        let listings = summarise(vec![
            row(1, "sales_count", 500, 17.0),
            row(1, "resource_views", 500, 1234.0),
            row(1, "earnings", 500, 42.5),
        ]);
        assert_eq!(listings.len(), 1, "one mapping is one row");
        assert_eq!(
            listings[0].metrics.len(),
            3,
            "all three metrics travel in that row's map"
        );
        assert_eq!(
            listings[0].metrics.get("resource_views"),
            Some(&1234.0),
            "the stored metric name is the key the client reads"
        );
    }

    #[test]
    fn a_row_states_the_instant_of_its_stalest_figure() {
        let listings = summarise(vec![
            row(1, "sales_count", 9_000, 17.0),
            row(1, "resource_views", 1_000, 1234.0),
        ]);
        assert_eq!(
            listings[0].observed_at,
            Timestamp(1_000),
            "the older figure sets the row's instant, so no number is shown fresher than it is"
        );
    }

    #[test]
    fn listings_do_not_bleed_into_each_other() {
        let listings = summarise(vec![
            row(2, "sales_count", 500, 3.0),
            row(1, "sales_count", 500, 17.0),
        ]);
        assert_eq!(listings.len(), 2, "two mappings are two rows");
        assert_eq!(
            listings
                .iter()
                .map(|listing| listing.metrics["sales_count"])
                .collect::<Vec<_>>(),
            vec![17.0, 3.0],
            "each row carries its own mapping's figure, ordered by mapping id"
        );
    }

    #[test]
    fn a_tenant_with_no_captures_answers_an_empty_list() {
        assert!(
            summarise(Vec::new()).is_empty(),
            "no captures is an empty list rather than a fabricated zero"
        );
    }
}
