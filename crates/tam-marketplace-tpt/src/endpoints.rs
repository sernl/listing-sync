//! Typed builders and parsers for the two TPT GraphQL services, mined from
//! the 2026-08-28 HAR captures of the founder's own seller account. Builders
//! return seam `HttpRequest` values so cassettes and the live transport are
//! interchangeable.
//!
//! TPT is not one API. `/graph/graphql` owns the seller and product domain
//! and paginates by offset; `/gateway/graphql` owns store analytics and
//! paginates Relay-style. They share a cookie session and a CSRF header and
//! differ by one request header, which is why [`Service`] is a type rather
//! than a string a caller assembles. What comes back is parsed in
//! [`crate::read_model`].

use serde_json::{json, Value};
use tam_marketplace::transport::HttpRequest;

use crate::read_model::{ProductId, STATS_ALIAS};

pub const ORIGIN: &str = "https://www.teacherspayteachers.com";

/// Which of the origin's two GraphQL services a request addresses. The
/// gateway's extra `x-gateway-auth-version` header is transport-side
/// constructor state, so the service is recoverable from the URL alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Graph,
    Gateway,
}

impl Service {
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Graph => "/graph/graphql",
            Self::Gateway => "/gateway/graphql",
        }
    }
}

/// True where a URL addresses the gateway service, which is how the live
/// transport decides to add `x-gateway-auth-version: 2` without the seam
/// carrying a headers field.
#[must_use]
pub fn is_gateway(url: &str) -> bool {
    url.starts_with(ORIGIN)
        && url
            .get(ORIGIN.len()..)
            .is_some_and(|rest| rest.starts_with(Service::Gateway.path()))
}

/// Every request appends `?opname=<operationName>`, which is how the client
/// itself addresses both services.
fn url(service: Service, operation: &str) -> String {
    format!("{ORIGIN}{}?opname={operation}", service.path())
}

/// The client sends the full query text on every call: TPT uses no persisted
/// queries and no hash allowlist, so nothing here needs registering upstream.
fn graphql(service: Service, operation: &str, query: &str, variables: &Value) -> HttpRequest {
    HttpRequest::post_json(
        url(service, operation),
        json!({
            "operationName": operation,
            "variables": variables,
            "extensions": {},
            "query": query,
        }),
    )
}

/// The seller-owns-these enumeration query, verbatim from the `MyProductListings`
/// POST recorded in `tpt-products-manage.har`, `MyResourceFields` fragment
/// included. `sellerId` is deliberately absent from the variables: the server
/// infers the seller from the session, and supplying one is untested.
pub const MY_PRODUCT_LISTINGS_QUERY: &str = r"query MyProductListings($limit: Int!, $offset: Int!, $filters: [SellerResourcesFilter]!, $orderBy: SellerResourcesSortOrder!, $sellerId: ID, $resourceIds: [ID!]) {
  seller(sellerId: $sellerId) {
    resources(
      filters: $filters
      orderBy: $orderBy
      paginationParameters: {limit: $limit, offset: $offset}
      resourceIds: $resourceIds
    ) {
      results {
        ...MyResourceFields
        __typename
      }
      pageInfo {
        totalResultsCount
        currentPage
        totalPageCount
        __typename
      }
      __typename
    }
    __typename
  }
}

fragment MyResourceFields on Product {
  id
  name
  canonicalSlug
  kind
  itemType
  statusAdminType {
    id
    __typename
  }
  price
  saleprice
  licenseprice
  discountprice
  isFree
  isFeatured
  images {
    home
    large
    medium
    original
    small
    __typename
  }
  status
  statusAdmin
  statusCopyright
  sold
  evaluationRating {
    scoreAverage
    count
    __typename
  }
  filePreview {
    name
    format
    filesize
    __typename
  }
  resourceProperties {
    assessmentIds
    activityIds
    canConvertToEaselActivity
    isStandaloneEasel
    __typename
  }
  bundle {
    includedItemTypes
    includedOnlineResourceTypes {
      description
      identifier
      __typename
    }
    digitalActivities {
      totalEaselAssessments
      totalEaselActivities
      totalConvertibleEaselActivities
      __typename
    }
    zip {
      status
      __typename
    }
    __typename
  }
  onlineResource {
    onlineResourceType {
      description
      __typename
    }
    __typename
  }
  mainVideo {
    duration
    status
    __typename
  }
  previewVideo {
    id
    __typename
  }
  lastModifiedAt
  postDate
  categories {
    id
    name
    __typename
  }
  taxonomyTags {
    id
    __typename
  }
  author {
    id
    name
    url
    __typename
  }
  __typename
}
";

/// The page size the TPT client itself hard-codes.
pub const CATALOGUE_PAGE_LIMIT: u32 = 100;

/// The sort order the recorded unfiltered enumeration used.
const CATALOGUE_ORDER: &str = "MOST_RECENTLY_POSTED";

#[must_use]
pub fn my_product_listings_request(limit: u32, offset: u32) -> HttpRequest {
    graphql(
        Service::Graph,
        "MyProductListings",
        MY_PRODUCT_LISTINGS_QUERY,
        &json!({
            "limit": limit,
            "offset": offset,
            "filters": [],
            "orderBy": CATALOGUE_ORDER,
        }),
    )
}

/// The metrics observed on `storeResourceTotalsAllTimeStats`, each recovered
/// from a query the seller-statistics page issues or carries: `SalesStatsAllTimeQuery`
/// (sales, licences, conversions, earnings), `ActivityStatsAllTimeQuery`
/// (downloads through wishlists) and `ReviewsStatsAllTimeQuery` (rating, votes).
/// GraphQL enum members cannot travel as variables, so the member name is
/// interpolated into the query text exactly as the TPT client does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllTimeMetric {
    SalesCount,
    SoldLicenses,
    ResourceConversions,
    Earnings,
    Downloads,
    PreviewDownloads,
    ResourceViews,
    VideoPlays,
    PreviewPlays,
    Wishlisted,
    Rating,
    Votes,
}

impl AllTimeMetric {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SalesCount => "SALES_COUNT",
            Self::SoldLicenses => "SOLD_LICENSES",
            Self::ResourceConversions => "RESOURCE_CONVERSIONS",
            Self::Earnings => "EARNINGS",
            Self::Downloads => "DOWNLOADS",
            Self::PreviewDownloads => "PREVIEW_DOWNLOADS",
            Self::ResourceViews => "RESOURCE_VIEWS",
            Self::VideoPlays => "VIDEO_PLAYS",
            Self::PreviewPlays => "PREVIEW_PLAYS",
            Self::Wishlisted => "WISHLISTED",
            Self::Rating => "RATING",
            Self::Votes => "VOTES",
        }
    }
}

/// The metrics observed on `storeResourceStatsNext`, whose enum is a
/// different type upstream from [`AllTimeMetric`] and whose members overlap
/// only partly. Earnings and views were observed on the wire; the five Easel
/// members come from `EaselResourceStatsAllTimeQuery` in the statistics page
/// bundle and have not been seen executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedMetric {
    Earnings,
    ResourceViews,
    TotalAdds,
    TotalAssigns,
    UniqueTeachers,
    StudentsReached,
    AssignRate,
}

impl ResolvedMetric {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Earnings => "EARNINGS",
            Self::ResourceViews => "RESOURCE_VIEWS",
            Self::TotalAdds => "TOTAL_ADDS",
            Self::TotalAssigns => "TOTAL_ASSIGNS",
            Self::UniqueTeachers => "UNIQUE_TEACHERS",
            Self::StudentsReached => "STUDENTS_REACHED",
            Self::AssignRate => "ASSIGN_RATE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricResolution {
    Day,
    Month,
}

impl MetricResolution {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "DAY",
            Self::Month => "MONTH",
        }
    }
}

/// The analytics window, in the unix seconds every gateway timestamp uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsWindow {
    pub from_unix_seconds: i64,
    pub to_unix_seconds: i64,
}

/// The batch size proven on the wire: the statistics page passed one hundred
/// resource ids in a single `resourceIds` array and the gateway answered.
/// The true ceiling is untested, so a larger batch is refused rather than
/// attempted.
pub const STATS_BATCH_MAX: usize = 100;

/// Serialises ids as unquoted integers, which is what the captured request
/// does despite the `[ID]` declaration: the gateway coerces an integer to an
/// ID, and mirroring the capture keeps the request byte-shaped like a real one.
fn resource_id_values(ids: &[ProductId]) -> Vec<Value> {
    ids.iter().map(|id| json!(id.0)).collect()
}

fn all_time_query(metric: AllTimeMetric) -> String {
    format!(
        "query ResourceTotalsAllTime($resourceIds: [ID]) {{\n  \
         {STATS_ALIAS}: storeResourceTotalsAllTimeStats(\n    \
         metricName: {}\n    filters: {{resourceIds: $resourceIds}}\n  ) {{\n    \
         edges {{\n      cursor\n      node {{\n        resourceId\n        \
         totalValue\n        __typename\n      }}\n      __typename\n    }}\n    \
         __typename\n  }}\n}}\n",
        metric.as_str()
    )
}

/// All-time per-resource totals for one metric over one batch of the seller's
/// own products. Modelled on the recorded `SalesStatsAllTimeQuery`, reduced
/// from its four aliases to the one this adapter asks for.
#[must_use]
pub fn all_time_stats_request(metric: AllTimeMetric, ids: &[ProductId]) -> HttpRequest {
    graphql(
        Service::Gateway,
        "ResourceTotalsAllTime",
        &all_time_query(metric),
        &json!({ "resourceIds": resource_id_values(ids) }),
    )
}

fn resolved_query(metric: ResolvedMetric, resolution: MetricResolution) -> String {
    format!(
        "query ResourceStatsResolved($resourceIds: [ID], $timeFrom: Int, $timeTo: Int) {{\n  \
         {STATS_ALIAS}: storeResourceStatsNext(\n    metricName: {}\n    timeFrom: $timeFrom\n    \
         timeTo: $timeTo\n    resolution: {}\n    filters: {{resourceIds: $resourceIds}}\n  ) {{\n    \
         edges {{\n      cursor\n      node {{\n        resourceId\n        totalValue\n        \
         __typename\n      }}\n      __typename\n    }}\n    __typename\n  }}\n}}\n",
        metric.as_str(),
        resolution.as_str()
    )
}

/// What a time-resolved statistics read asks for, apart from the resources
/// themselves: which metric, at what granularity, over which window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedStatsQuery {
    pub metric: ResolvedMetric,
    pub resolution: MetricResolution,
    pub window: StatsWindow,
}

/// Time-resolved per-resource totals. The root field, its resolution argument
/// and the unix-second window are the recorded `StoreResources` request; the
/// `resourceIds` filter is from `EaselResourceStatsAllTimeQuery` in the page
/// bundle, which selects the same root field but was never seen executed.
#[must_use]
pub fn resolved_stats_request(query: ResolvedStatsQuery, ids: &[ProductId]) -> HttpRequest {
    graphql(
        Service::Gateway,
        "ResourceStatsResolved",
        &resolved_query(query.metric, query.resolution),
        &json!({
            "resourceIds": resource_id_values(ids),
            "timeFrom": query.window.from_unix_seconds,
            "timeTo": query.window.to_unix_seconds,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        all_time_stats_request, is_gateway, my_product_listings_request, AllTimeMetric, Service,
        MY_PRODUCT_LISTINGS_QUERY, ORIGIN,
    };
    use crate::read_model::ProductId;
    use serde_json::{json, Value};
    use tam_marketplace::transport::RequestBody;

    #[test]
    fn the_two_services_are_told_apart_by_the_url_alone() {
        let graph = my_product_listings_request(100, 0);
        let gateway = all_time_stats_request(AllTimeMetric::Earnings, &[ProductId(1)]);
        assert!(
            !is_gateway(&graph.url),
            "the product read is the legacy graph service"
        );
        assert!(
            is_gateway(&gateway.url),
            "the analytics read is the gateway, which is what adds x-gateway-auth-version"
        );
        assert!(
            !is_gateway(&format!("{ORIGIN}/graph/graphql-not-the-gateway")),
            "a path that merely starts alike is not the gateway"
        );
        assert_eq!(Service::Gateway.path(), "/gateway/graphql");
    }

    #[test]
    fn the_enumeration_request_carries_its_full_query_text() {
        let request = my_product_listings_request(100, 0);
        let RequestBody::Json(body) = request.body else {
            panic!("the enumeration posts JSON");
        };
        assert_eq!(
            body.pointer("/variables/limit").and_then(Value::as_u64),
            Some(100),
            "the client's own page size travels as a variable"
        );
        assert!(
            body.pointer("/variables/sellerId").is_none(),
            "the seller is inferred from the session; supplying one is untested"
        );
        assert!(
            MY_PRODUCT_LISTINGS_QUERY.contains("fragment MyResourceFields on Product"),
            "TPT uses no persisted queries, so the fragment ships with every call"
        );
    }

    #[test]
    fn a_statistics_batch_sends_unquoted_integer_ids() {
        let request = all_time_stats_request(AllTimeMetric::SalesCount, &[ProductId(12_854_712)]);
        let RequestBody::Json(body) = request.body else {
            panic!("the statistics read posts JSON");
        };
        assert_eq!(
            body.pointer("/variables/resourceIds/0"),
            Some(&json!(12_854_712)),
            "the capture sends integers despite the [ID] declaration and the gateway coerces them"
        );
        assert!(
            body.pointer("/query")
                .and_then(Value::as_str)
                .is_some_and(|query| query.contains("metricName: SALES_COUNT")),
            "a GraphQL enum member cannot travel as a variable, so it is interpolated"
        );
    }
}
