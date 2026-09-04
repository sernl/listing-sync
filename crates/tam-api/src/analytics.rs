//! The analytics summary: the newest captured figure for every listing this
//! organisation has captured one for.
//!
//! Org-scoped through [`OrgContext`] like every other route, so the request
//! carries no organisation identifier a caller could substitute. A tenant
//! whose first capture has not run answers an empty list, which is honest and
//! is what a first render needs.

use std::collections::BTreeMap;

use axum::extract::Path;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_engine_driver::vocabulary::{BoundListing, MetricSnapshot};
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

/// What a device is to capture, and the capture it sends back.
///
/// The routes sit here rather than beside the device registry because a reader
/// looking for anything analytics-shaped in this crate looks once. What makes
/// them device routes is the path and the two gates below, not the module.
///
/// Both are org-scoped through [`OrgContext`] like everything else on this
/// surface, so no caller names an organisation. Two further gates apply, and
/// they are the same two the work claim applies: the device must hold a
/// connected session for the marketplace, because a capture is a marketplace
/// request and must not be handed to a machine that cannot make it; and the
/// tenant's entitlement must stand, because D11 makes entitlement the gate on
/// work that runs on a seller's own hardware, and a capture spends the
/// connection's rate budget exactly as a write does.
///
/// A pull rather than a claim, which the snapshot table settles: it is keyed
/// `(org_id, mapping_id, metric, observed_at)` and written `ON CONFLICT DO
/// NOTHING`, and migration 0036 designs it append-only with a rate as the
/// difference between two rows. Two devices capturing the same listing
/// therefore corrupt nothing — the same instant collapses, and different
/// instants are two honest observations. What it does cost is a second walk of
/// the seller's catalogue against one connection's budget, which is recorded
/// as its own item rather than solved by a lease here.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadOrderView {
    pub listings: Vec<BoundListing>,
    /// Absent work is `listings` empty rather than a separate state: a tenant
    /// with nothing bound on Tpt has nothing to capture, and saying so as an
    /// empty list keeps the client's one code path.
    pub next_poll_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct CaptureBody {
    pub snapshots: Vec<MetricSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CaptureAcceptedView {
    pub written: u64,
}

/// How long a device waits before asking again. The capture is a daily-shaped
/// job rather than a queue, so this is deliberately far longer than the work
/// route's poll: a device that asks once an hour and finds nothing has cost
/// the marketplace nothing and the server one row read.
const READ_POLL_MS: u64 = 3_600_000;

/// The two gates, asked once and in the order that refuses most cheaply.
async fn may_capture(
    state: &AppState,
    org: tam_types::OrgId,
    device: &str,
) -> Result<bool, APIError> {
    let devices = tam_storage::DeviceRepo::new(state.pool.clone());
    if !devices
        .holds_connected_session(org, device, tam_types::Marketplace::Tpt)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        return Ok(false);
    }
    devices
        .entitlement_stands(org, tam_domain::ENTITLEMENT_GRACE_HOURS)
        .await
        .map_err(|error| state.internal(&error.to_string()))
}

/// What this device should capture now.
///
/// A device that may not capture is answered an empty list rather than a
/// refusal, and the distinction is deliberate: neither a lapsed plan nor a
/// signed-out session is an error the client can act on, and a 403 on a poll
/// that runs hourly would fill a log with something nobody will read.
pub(crate) async fn read_order(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
) -> Result<Json<ReadOrderView>, APIError> {
    let listings = if may_capture(&state, context.org, &device).await? {
        tam_storage::MappingRepo::new(state.pool.clone())
            .bound_listings(context.org, InventoryId::Tpt)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
            .into_iter()
            .map(BoundListing::from)
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(ReadOrderView {
        listings,
        next_poll_ms: READ_POLL_MS,
    }))
}

/// The capture, reported back.
///
/// The gates are re-asked rather than trusted from the order that prompted
/// this, because the two are separate requests and a plan can lapse or a
/// session end in between; a device that captured under an entitlement that
/// has since gone reports rows we decline to store rather than rows we store
/// on a right that expired.
pub(crate) async fn record_capture(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(body): Json<CaptureBody>,
) -> Result<Json<CaptureAcceptedView>, APIError> {
    if !may_capture(&state, context.org, &device).await? {
        return Ok(Json(CaptureAcceptedView { written: 0 }));
    }
    // Only this tenant's own mappings. `listing_metric_snapshot` carries a
    // foreign key onto `mapping`, so a snapshot naming a mapping this
    // organisation does not own would raise a constraint violation and surface
    // as a fault -- a caller's mistake reported as ours. Dropping it is both
    // the honest answer and the same one an ungated device gets: nothing
    // written, nothing claimed.
    let owned: Vec<MappingId> = tam_storage::MappingRepo::new(state.pool.clone())
        .bound_listings(context.org, InventoryId::Tpt)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .into_iter()
        .map(|listing| listing.mapping)
        .collect();
    let snapshots: Vec<tam_storage::MetricSnapshot> = body
        .snapshots
        .into_iter()
        .filter(|snapshot| owned.contains(&snapshot.mapping))
        .map(tam_storage::MetricSnapshot::from)
        .collect();
    let written = AnalyticsRepo::new(state.pool.clone())
        .record(context.org, &snapshots)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(CaptureAcceptedView { written }))
}
