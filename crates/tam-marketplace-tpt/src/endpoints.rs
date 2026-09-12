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
use tam_marketplace::transport::{HttpRequest, Method, RequestAuth, RequestBody};

use crate::read_model::{ProductId, STATS_ALIAS};
use crate::s3::{
    sign_auth_scope, AwsKeyId, S3Error, S3Operation, S3Signature, StringToSign, UploadSlot,
    UploadTicket,
};
use crate::upload::{ProcessedHandle, QueueJob, UploadHandle};

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

/// The seller's own product whole, verbatim from the `UploadPageProductQuery`
/// POST the edit form issues (`tpt-capture-edit-live-product.har`, entry 51),
/// with one word added.
///
/// `status` is selected beside `statusUser` because that is the field the
/// draft line is actually read off. `statusUser` is observed once in the
/// whole capture set, with one value; a `Product`-shaped `status` is observed
/// 908 times with two, and `read_back` already derives the lifecycle from it.
/// Deriving a removal's lifecycle from `statusUser` would ship an unvalidated
/// Draft arm into the one operation that deletes the seller's listing.
///
/// Adding the field is safe and needs no upstream registration: `status` is a
/// field on the same `Product` type that `MyResourceFields` already selects,
/// the query text is ours rather than a persisted-query hash, and the crate
/// carries no operation allowlist.
///
/// The catalogue query is not an alternative for this read: it carries no
/// `description`, and the description is the field canonicalisation exists
/// for.
pub const UPLOAD_PAGE_PRODUCT_QUERY: &str = r"query UploadPageProductQuery($id: ID!, $useResourceCatalog: Boolean) {
  products(ids: [$id], useResourceCatalog: $useResourceCatalog) {
    name
    copyrightInfringement {
      name
      __typename
    }
    description
    isFree
    price
    discountprice
    licenseprice
    teachingDuration
    answerKey
    statusUser
    status
    filePreview {
      pageCount
      previewUrl
      __typename
    }
    copyrightDeclaration
    itemType
    commonCoreStandards {
      name
      id: sphinxId
      parentIds
      __typename
    }
    taxonomyTags {
      id
      __typename
    }
    categories {
      id
      name
      __typename
    }
    localization {
      countryId
      countryIdFlag
      country {
        name
        __typename
      }
      __typename
    }
    audience
    audienceText
    videoType
    videoTypeText
    taxCode {
      id
      __typename
    }
    author {
      id
      __typename
    }
    __typename
  }
}
";

/// The edit form's own read of one product. `useResourceCatalog` is the value
/// the capture carries.
#[must_use]
pub fn upload_page_product_request(id: ProductId) -> HttpRequest {
    graphql(
        Service::Graph,
        "UploadPageProductQuery",
        UPLOAD_PAGE_PRODUCT_QUERY,
        &json!({
            "id": id.0.to_string(),
            "useResourceCatalog": true,
        }),
    )
}

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

/// Which product form a render addresses. The create and edit forms are one
/// CakePHP form rendered in add and edit mode, which is why one scrape serves
/// both and one type names both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormTarget {
    CreateDigital,
    EditDigital(ProductId),
}

const CREATE_DIGITAL_PATH: &str = "/My-Products/New/Digital-Next";
const EDIT_DIGITAL_PREFIX: &str = "/itemsDigital/editNext/";

impl FormTarget {
    #[must_use]
    pub fn path(self) -> String {
        match self {
            Self::CreateDigital => CREATE_DIGITAL_PATH.to_owned(),
            Self::EditDigital(product) => format!("{EDIT_DIGITAL_PREFIX}{product}"),
        }
    }

    #[must_use]
    pub fn url(self) -> String {
        format!("{ORIGIN}{}", self.path())
    }
}

/// True where a URL addresses a product form, which is how the live transport
/// tells a document navigation from an XHR: the two renders and the two
/// submits that answer them are the only session hops the browser makes as a
/// navigation, and the capture gives a navigation a different header envelope.
#[must_use]
pub fn is_product_form(url: &str) -> bool {
    url.starts_with(ORIGIN)
        && url.get(ORIGIN.len()..).is_some_and(|rest| {
            rest.starts_with(CREATE_DIGITAL_PATH) || rest.starts_with(EDIT_DIGITAL_PREFIX)
        })
}

/// The route the product page's Download control points at. The control is a
/// plain `<a href target="_blank">` reading `Product.downloadurl` verbatim
/// from the page's own SSR state, with no minted token, no nonce and no XHR,
/// so a caller reproduces it as an ordinary document navigation.
const DOWNLOAD_PREFIX: &str = "/Download/";

/// The origin's sign-in gate, which is where a download redirects when the
/// session is not cleared for it.
const AUTHORIZATION_PATH: &str = "/Request-Authorization";

/// The content network an owned download redirects to.
///
/// Captured on the founder's own device on 2026-09-13, on their own
/// `product13042099`: an authenticated `GET /Download/<slug>-13042099`
/// answered `302` to
/// `https://rc-assets.teacherspayteachers.com/resources/13042099/assets/<opaque>?file_name=<name>.zip&verify=<token>`,
/// and a fetch of that url with credentials omitted answered `200`
/// `application/zip`, 14,110,742 bytes opening `PK\x03\x04`.
///
/// One host, exactly, and no suffix rule: this is the host that was
/// observed. A `.ends_with` test here would admit
/// `rc-assets.teacherspayteachers.com.example`, and widening to every https
/// host would make a `Location` the thing that decides where the seller's
/// own files are fetched from.
pub const ASSET_HOST: &str = "rc-assets.teacherspayteachers.com";

/// The only port the asset host is addressed on. An explicit `:443` is the
/// same destination written longhand; any other port names a different
/// service on a host we were pointed at, and is refused rather than reached.
const HTTPS_PORT: &str = "443";

/// The fixed segments either side of the resource id in the captured path,
/// `/resources/{id}/assets/{opaque}`.
const RESOURCES_SEGMENT: &str = "resources";
const ASSETS_SEGMENT: &str = "assets";

/// The seller's own copy of one of their products.
///
/// `downloadurl` is `/Download/{canonicalSlug}-{id}`, and the slug is
/// decorative here as it is on `/Product/{slug}-{id}`: the trailing number is
/// the identifier. It is threaded anyway because the catalogue read carries
/// it, and reproducing the url the page itself renders costs nothing; whether
/// the id alone resolves is unprobed.
#[must_use]
pub fn download_bundle_request(canonical_slug: &str, product: ProductId) -> HttpRequest {
    HttpRequest::get(format!(
        "{ORIGIN}{DOWNLOAD_PREFIX}{canonical_slug}-{}",
        product.0
    ))
}

/// True where a URL addresses the download route, which is how the live
/// transport gives this hop the document-navigation envelope the browser
/// sends it under rather than the fetch envelope every other bodyless GET
/// gets.
#[must_use]
pub fn is_download_path(url: &str) -> bool {
    url.starts_with(ORIGIN)
        && url
            .get(ORIGIN.len()..)
            .is_some_and(|rest| rest.starts_with(DOWNLOAD_PREFIX))
}

/// One signed asset url, exactly as the marketplace's `Location` spelled it.
///
/// A newtype for two reasons. The url carries a `verify` token minted for
/// one resource and one moment, so its `Debug` prints none of it: a refusal,
/// a log line or a panic message that formatted this would copy a live
/// credential somewhere it was never scoped for. And the url travels byte
/// for byte — no re-encoding, no query reordering, no normalisation —
/// because the token signs those bytes and any repair of them invalidates it.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedAssetUrl(String);

impl SignedAssetUrl {
    /// The bytes, at the one place that sends them.
    #[must_use]
    pub fn into_url(self) -> String {
        self.0
    }
}

impl core::fmt::Debug for SignedAssetUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SignedAssetUrl(<redacted>)")
    }
}

/// Why a redirect this download will not follow was refused.
///
/// A reason, and no url. What a refusal may say is which part of the shape
/// was wrong, never the destination: a rejected `Location` can still carry a
/// live `verify` token, and an error message is the one place a value is
/// certain to be written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectRefusal {
    /// Not an absolute `https://` url: plaintext, a protocol-relative
    /// authority, a relative reference, or a scheme this fetch does not
    /// speak.
    NotHttps,
    /// Userinfo in the authority, which is a credential in a url.
    CredentialsInUrl,
    /// A port other than 443.
    NonStandardPort,
    /// A fragment, which no request sends and which hides the tail of a url
    /// from a naive path test.
    Fragment,
    /// A host that is not the one captured asset host.
    ForeignHost,
    /// That host, and a path that is not this product's own asset.
    ForeignResource,
    /// Characters a URL parser would discard or reinterpret.
    NonCanonicalUrl,
}

impl core::fmt::Display for RedirectRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotHttps => "the location is not an absolute https url",
            Self::CredentialsInUrl => "the location carries userinfo credentials",
            Self::NonStandardPort => "the location names a port other than 443",
            Self::Fragment => "the location carries a fragment",
            Self::ForeignHost => "the location names a host other than the captured asset host",
            Self::ForeignResource => "the location names another resource than the one requested",
            Self::NonCanonicalUrl => "the location is not canonically encoded",
        })
    }
}

/// Where a download's redirect points, which decides whether there is a
/// second hop to make and what may carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadRedirect {
    /// The origin's own sign-in gate. A 2026-08-29 live probe met this with a
    /// server-side jar that authenticates the GraphQL reads and the whole
    /// write path, so the download is gated on something those are not.
    Authorization,
    /// Elsewhere on the origin, which a session request may follow.
    SameOrigin(String),
    /// The captured asset hop: the one observed content-network host, and a
    /// path bound to the product that was asked for. Followed once, carrying
    /// nothing of ours.
    CapturedAsset(SignedAssetUrl),
    /// Anywhere else, and which part of the shape refused it.
    Refused(RedirectRefusal),
}

/// Reads a redirect's `Location` against the origin and against the one
/// captured asset host, for the product the download asked for.
///
/// A relative location is resolved against the origin and an absolute one on
/// the origin is kept. Anything else is measured against the captured asset
/// shape and refused with a reason if it does not match — including a
/// protocol-relative `//host/path`, which is a different host wearing a
/// leading slash.
///
/// The product id is a parameter because it is the binding. A `Location`
/// naming the right host and somebody else's resource is the one plausible
/// way this hop fetches a file nobody asked for, and the id in the path is
/// the only part of that url this code can check against the request that
/// produced it.
#[must_use]
pub fn download_redirect(location: &str, product: ProductId) -> DownloadRedirect {
    let path = if location.starts_with("//") {
        None
    } else if location.starts_with('/') {
        Some(location)
    } else {
        // The origin prefix must end the authority rather than merely begin
        // it, or `https://www.teacherspayteachers.com.example/x` reads as a
        // path on the origin.
        match location.strip_prefix(ORIGIN) {
            Some("") => Some("/"),
            Some(rest) if rest.starts_with(['/', '?', '#']) => Some(rest),
            Some(_) | None => None,
        }
    };
    match path {
        Some(path) if path.starts_with(AUTHORIZATION_PATH) => DownloadRedirect::Authorization,
        Some(path) => DownloadRedirect::SameOrigin(format!("{ORIGIN}{path}")),
        None => match captured_asset(location, product) {
            Ok(asset) => DownloadRedirect::CapturedAsset(asset),
            Err(why) => DownloadRedirect::Refused(why),
        },
    }
}

/// One `Location` measured against the captured asset hop, whole: the
/// authority by [`asset_target`] and the path by [`is_product_asset_path`].
/// The url handed back is the input's own bytes.
fn captured_asset(location: &str, product: ProductId) -> Result<SignedAssetUrl, RedirectRefusal> {
    let target = asset_target(location)?;
    let path = target.split('?').next().unwrap_or(target);
    if !is_product_asset_path(path, product) {
        return Err(RedirectRefusal::ForeignResource);
    }
    Ok(SignedAssetUrl(location.to_owned()))
}

/// The path and query of an absolute https url on the captured asset host,
/// and a reason for anything else.
///
/// Scheme, userinfo, port, fragment and host are decided here and nowhere
/// else, because the same question is asked twice — of a `Location` that
/// arrived and of a request about to leave — and two spellings of it would
/// eventually disagree about one of them.
fn asset_target(url: &str) -> Result<&str, RedirectRefusal> {
    if !url.is_ascii()
        || url.bytes().any(|byte| {
            byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || matches!(byte, b'\\' | b'"' | b'<' | b'>' | b'`' | b'\'')
        })
    {
        return Err(RedirectRefusal::NonCanonicalUrl);
    }
    if url.contains('#') {
        return Err(RedirectRefusal::Fragment);
    }
    let rest = url
        .strip_prefix("https://")
        .ok_or(RedirectRefusal::NotHttps)?;
    let authority_end = rest
        .find(['/', '?'])
        .ok_or(RedirectRefusal::ForeignResource)?;
    let (authority, target) = rest
        .split_at_checked(authority_end)
        .ok_or(RedirectRefusal::ForeignHost)?;
    if authority.contains('@') {
        return Err(RedirectRefusal::CredentialsInUrl);
    }
    let host = match authority.split_once(':') {
        None => authority,
        Some((host, port)) if port == HTTPS_PORT => host,
        Some(_) => return Err(RedirectRefusal::NonStandardPort),
    };
    if !host.eq_ignore_ascii_case(ASSET_HOST) {
        return Err(RedirectRefusal::ForeignHost);
    }
    Ok(target)
}

/// `/resources/{id}/assets/{opaque}`, for one id: the captured shape, with
/// the requested product's own decimal id in it and nothing after the asset.
fn is_product_asset_path(path: &str, product: ProductId) -> bool {
    let expected = product.0.to_string();
    let mut segments = path.strip_prefix('/').unwrap_or(path).split('/');
    segments.next() == Some(RESOURCES_SEGMENT)
        && segments.next() == Some(expected.as_str())
        && segments.next() == Some(ASSETS_SEGMENT)
        && segments.next().is_some_and(is_asset_segment)
        && segments.next().is_none()
}

/// An opaque, unreserved path segment: escaped separators and dot segments
/// must not be normalized into a different product's path.
fn is_asset_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

/// True where a url is an absolute https url on the captured asset host,
/// with no userinfo, no other port and no fragment.
///
/// The transport's half of the asset rule. It stops short of the resource
/// binding deliberately: which product a request is for is the flow's
/// knowledge, and a path-shape test here without it would read as a stronger
/// check than it is.
#[must_use]
pub fn is_asset_url(url: &str) -> bool {
    asset_target(url).is_ok()
}

/// The captured second hop, re-issued carrying nothing of ours.
///
/// [`RequestAuth::Redirected`] is what keeps the seller's cookie and CSRF
/// pair off it: the transport routes that authentication to the client
/// holding no jar and following no redirect, and refuses the pairing of a
/// session with this host outright. The body is empty and the method is GET
/// because that is the request the capture made, and the url is the
/// marketplace's own bytes unaltered — the `verify` token signs them.
#[must_use]
pub fn signed_asset_request(asset: SignedAssetUrl) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        url: asset.into_url(),
        body: RequestBody::Empty,
        auth: RequestAuth::Redirected,
    }
}

/// The second hop of a download whose redirect stayed on the origin.
/// Session-authenticated like the first, because it is the same origin and
/// the same navigation. No capture carries one; the asset hop above is what
/// the observed redirect produces.
#[must_use]
pub fn download_redirect_request(url: String) -> HttpRequest {
    HttpRequest::get(url)
}

/// Percent-encodes one `application/x-www-form-urlencoded` component. The
/// unreserved set is RFC 3986's; a space becomes `+`, which is the one place
/// form encoding departs from percent encoding.
fn form_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*byte));
            }
            b' ' => out.push('+'),
            other => {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(char::from(HEX[usize::from(other >> 4)]));
                out.push(char::from(HEX[usize::from(other & 0x0F)]));
            }
        }
    }
    out
}

/// The four XHR hops post `application/x-www-form-urlencoded`, which the seam
/// has no variant for; the body travels as raw bytes and this crate's live
/// transport labels a session-authenticated raw body as a form, because these
/// four are the only raw session bodies TPT sends.
fn form_body(fields: &[(&str, &str)]) -> RequestBody {
    let encoded = fields
        .iter()
        .map(|(name, value)| format!("{}={}", form_encode(name), form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    RequestBody::Bytes(encoded.into_bytes())
}

fn post_form(url: String, fields: &[(&str, &str)]) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url,
        body: form_body(fields),
        auth: RequestAuth::Session,
    }
}

/// One form render, the sole source of the token triple, the CSRF pair, the
/// published AWS key id and — on an edit — the existing asset handles.
#[must_use]
pub fn form_page_request(target: FormTarget) -> HttpRequest {
    HttpRequest::get(target.url())
}

/// What `/uploads/upload_file` is told about the file it is reserving a key
/// for. `item_id` is empty on a create and the product id on an edit; only
/// the empty form is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadReservation<'a> {
    pub slot: UploadSlot,
    pub file_name: &'a str,
    pub size: usize,
    /// The file's own modification instant in milliseconds, as the browser's
    /// `File.lastModified` reports it. Passed in as data; this crate reads no
    /// clock.
    pub last_modified_ms: i64,
    pub item_id: Option<ProductId>,
}

/// Reserves an S3 object key. The bucket and path in the answer are the
/// server's choice and are never computed locally.
#[must_use]
pub fn upload_file_request(reservation: &UploadReservation<'_>) -> HttpRequest {
    let size = reservation.size.to_string();
    let last_modified = reservation.last_modified_ms.to_string();
    let item_id = reservation
        .item_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    post_form(
        format!("{ORIGIN}/uploads/upload_file"),
        &[
            ("file_type", reservation.slot.as_str()),
            ("file_name", reservation.file_name),
            ("size", &size),
            ("lastModified", &last_modified),
            ("item_id", &item_id),
            ("path", ""),
        ],
    )
}

/// TPT's own clock, in RFC 1123. Used verbatim as `x-amz-date` and inside the
/// `StringToSign`, because AWS rejects a signature whose date has skewed and
/// the server that signs it is the one whose clock matters.
#[must_use]
pub fn time_request(request_time_ms: i64) -> HttpRequest {
    HttpRequest::get(format!(
        "{ORIGIN}/uploads/time?requestTime={request_time_ms}"
    ))
}

/// Asks TPT to sign one string. The ticket is taken alongside the string and
/// the scope is re-checked here, at the last point before a request value
/// exists: the oracle itself performs no such check, so a caller that
/// assembled a resource elsewhere gets a refusal rather than a signature.
pub fn sign_auth_request(
    ticket: &UploadTicket,
    to_sign: &StringToSign,
    datetime: &str,
) -> Result<HttpRequest, S3Error> {
    sign_auth_scope(ticket, to_sign)?;
    Ok(HttpRequest::get(format!(
        "{ORIGIN}/uploads/sign_auth?to_sign={}&datetime={}",
        form_encode(to_sign.as_str()),
        form_encode(datetime),
    )))
}

/// Everything one signed S3 call carries beside its body.
#[derive(Debug, Clone, Copy)]
pub struct SignedS3Call<'a> {
    pub ticket: &'a UploadTicket,
    pub operation: &'a S3Operation,
    pub key_id: &'a AwsKeyId,
    pub signature: &'a S3Signature,
    pub amz_date: &'a str,
    /// Bound into the signature, so it must be the file's real type on every
    /// one of the three calls — including the completion, whose body is XML
    /// and whose content type is nonetheless the file's.
    pub content_type: &'a str,
    pub content_md5: Option<&'a str>,
}

/// One S3 request under a per-request SigV2 signature. Path-style addressing,
/// no cookies, and the signature carried as request state so it cannot
/// outlive the one call it authorises.
#[must_use]
pub fn s3_request(call: &SignedS3Call<'_>, body: RequestBody) -> HttpRequest {
    let method = match *call.operation {
        S3Operation::UploadPart { .. } => Method::Put,
        S3Operation::Initiate | S3Operation::Complete { .. } => Method::Post,
    };
    HttpRequest {
        method,
        url: call.ticket.object_url(&call.operation.query()),
        body,
        auth: RequestAuth::S3SigV2 {
            access_key_id: call.key_id.as_str().to_owned(),
            signature: call.signature.as_str().to_owned(),
            amz_date: call.amz_date.to_owned(),
            content_md5: call.content_md5.map(str::to_owned),
            content_type: call.content_type.to_owned(),
        },
    }
}

/// Hands the staged object to the async processor. The answer's body says
/// only `{"success":true}`; the job id is in the `x-queue-tracking-id`
/// response header.
#[must_use]
pub fn process_file_request(
    handle: &UploadHandle,
    item_id: Option<ProductId>,
    cache_buster: &str,
) -> HttpRequest {
    let item_id = item_id.map(|id| id.to_string()).unwrap_or_default();
    post_form(
        format!("{ORIGIN}/uploads/process_file?rand={cache_buster}"),
        &[("key", handle.as_str()), ("item_id", &item_id)],
    )
}

/// Polls one async job. `retryCount` is sent empty, which is what all five
/// recorded polls did; its increment semantics are uncaptured.
#[must_use]
pub fn queue_results_request(job: &QueueJob, cache_buster: &str) -> HttpRequest {
    post_form(
        format!("{ORIGIN}/queue/results?rand={cache_buster}&retryCount="),
        &[("job", job.as_str())],
    )
}

/// Requests thumbnail generation from the processed asset — the processed
/// handle, never the staged one.
#[must_use]
pub fn generate_thumbs_request(
    handle: &ProcessedHandle,
    item_id: Option<ProductId>,
    cache_buster: &str,
) -> HttpRequest {
    let item_id = item_id.map(|id| id.to_string()).unwrap_or_default();
    post_form(
        format!("{ORIGIN}/converter/generate_thumbs?rand={cache_buster}"),
        &[
            ("key", handle.as_str()),
            ("item_id", &item_id),
            ("item_type_id", "0"),
        ],
    )
}

/// The product form itself: a multipart navigation carrying no CSRF header,
/// because the pair travels in the body and the cookie. The transport must
/// not follow the redirect it answers with — the `Location` is the only place
/// the new product id appears.
#[must_use]
pub fn submit_form_request(target: FormTarget, fields: Vec<(String, String)>) -> HttpRequest {
    HttpRequest::post_multipart(target.url(), fields, None, RequestAuth::Session)
}

/// One of the seller's own products by id, through the same
/// `MyProductListings` operation the catalogue walk uses: the captured query
/// text declares a `resourceIds` variable, and naming one product is the
/// narrowest read that query supports.
#[must_use]
pub fn product_by_id_request(product: ProductId) -> HttpRequest {
    graphql(
        Service::Graph,
        "MyProductListings",
        MY_PRODUCT_LISTINGS_QUERY,
        &json!({
            "limit": 1,
            "offset": 0,
            "filters": [],
            "orderBy": CATALOGUE_ORDER,
            "resourceIds": [product.0.to_string()],
        }),
    )
}

/// The delete mutation, verbatim from the `RemoveResource` POST recorded in
/// `tpt-delete-product.har`. The operation declares `sellerId` and the
/// capture leaves it out of the variables, so this does too: the server
/// infers the seller from the session, exactly as the enumeration read does.
///
/// The identifier travels quoted here and unquoted on the analytics read.
/// Each request sends what its own capture sent.
pub const REMOVE_RESOURCE_MUTATION: &str = r"mutation RemoveResource($id: ID!, $sellerId: ID) {
  resourceDelete(input: {id: $id}, sellerId: $sellerId) {
    id
    __typename
  }
}
";

/// Removes one of the seller's own products. The answer echoes the deleted
/// id, which is the only confirmation the mutation gives.
#[must_use]
pub fn remove_resource_request(product: ProductId) -> HttpRequest {
    graphql(
        Service::Graph,
        "RemoveResource",
        REMOVE_RESOURCE_MUTATION,
        &json!({ "id": product.0.to_string() }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        all_time_stats_request, download_redirect, form_page_request, is_asset_url, is_gateway,
        is_product_form, my_product_listings_request, remove_resource_request,
        signed_asset_request, AllTimeMetric, DownloadRedirect, FormTarget, RedirectRefusal,
        Service, ASSET_HOST, MY_PRODUCT_LISTINGS_QUERY, ORIGIN,
    };
    use crate::read_model::ProductId;
    use serde_json::{json, Value};
    use tam_marketplace::transport::{Method, RequestAuth, RequestBody};

    /// The product and the url shape the 2026-09-13 device capture carries.
    /// The token is synthetic; the path and the host are the observed ones.
    const OWNED: ProductId = ProductId(13_042_099);

    fn captured_location() -> String {
        format!(
            "https://{ASSET_HOST}/resources/13042099/assets/9f2c1b?file_name=worksheet.zip&verify=token"
        )
    }

    #[test]
    fn the_captured_asset_location_is_followed_byte_for_byte() {
        let location = captured_location();
        let DownloadRedirect::CapturedAsset(asset) = download_redirect(&location, OWNED) else {
            panic!("the captured shape is the one redirect this download follows");
        };
        let request = signed_asset_request(asset);
        assert_eq!(
            request.url, location,
            "the verify token signs these bytes, so nothing re-encodes or reorders them"
        );
        assert_eq!(
            (request.method, request.body, request.auth),
            (Method::Get, RequestBody::Empty, RequestAuth::Redirected),
            "the capture is a bodyless GET carrying none of the seller's session"
        );
        assert!(
            is_asset_url(&location),
            "and the transport's own half of the rule admits the same url"
        );
    }

    /// Neither a refusal nor a `Debug` line may carry the signed url.
    ///
    /// The token authorises a fetch of the seller's file by itself, and an
    /// error message is the one value certain to be written down somewhere
    /// nobody scoped for it.
    #[test]
    fn no_refusal_or_debug_line_repeats_the_signed_url() {
        let location = captured_location();
        let followed = download_redirect(&location, OWNED);
        let printed = format!("{followed:?}");
        assert!(
            !printed.contains("verify") && !printed.contains("9f2c1b"),
            "the url is redacted where it is printed, and printed: {printed}"
        );
        let refused = format!(
            "{:?} {}",
            download_redirect(&location, ProductId(90_000_042)),
            RedirectRefusal::ForeignResource
        );
        assert!(
            !refused.contains("verify") && !refused.contains(ASSET_HOST),
            "a refusal names the shape that was wrong and not the destination, and said: \
             {refused}"
        );
    }

    /// Every way of pointing somewhere else while resembling the captured
    /// hop. Each row is a refusal with a reason, because a `Location` decides
    /// nothing here beyond which of these it matches.
    #[test]
    fn a_redirect_that_is_not_the_captured_asset_is_refused_with_its_reason() {
        let expected = [
            (
                format!("http://{ASSET_HOST}/resources/13042099/assets/9f2c1b"),
                RedirectRefusal::NotHttps,
            ),
            (
                "//rc-assets.teacherspayteachers.com/resources/13042099/assets/9f2c1b".to_owned(),
                RedirectRefusal::NotHttps,
            ),
            (
                format!("https://{ASSET_HOST}.example/resources/13042099/assets/9f2c1b"),
                RedirectRefusal::ForeignHost,
            ),
            (
                format!("https://example.invalid/{ASSET_HOST}/resources/13042099/assets/9f2c1b"),
                RedirectRefusal::ForeignHost,
            ),
            (
                format!("https://{ASSET_HOST}@example.invalid/resources/13042099/assets/9f2c1b"),
                RedirectRefusal::CredentialsInUrl,
            ),
            (
                format!("https://{ASSET_HOST}:8443/resources/13042099/assets/9f2c1b"),
                RedirectRefusal::NonStandardPort,
            ),
            (
                format!("https://{ASSET_HOST}/resources/13042099/assets/9f2c1b#x"),
                RedirectRefusal::Fragment,
            ),
            (
                format!("https://{ASSET_HOST}/resources/90000042/assets/9f2c1b?verify=token"),
                RedirectRefusal::ForeignResource,
            ),
            (
                format!("https://{ASSET_HOST}/resources/13042099/assets/"),
                RedirectRefusal::ForeignResource,
            ),
            (
                format!("https://{ASSET_HOST}/resources/13042099/assets/..%2f..%2fresources"),
                RedirectRefusal::ForeignResource,
            ),
            (
                format!("https://{ASSET_HOST}/resources/13042099/assets/9f2c1b/extra"),
                RedirectRefusal::ForeignResource,
            ),
            (
                format!("https://{ASSET_HOST}/downloads/13042099"),
                RedirectRefusal::ForeignResource,
            ),
            (
                format!("https://{ASSET_HOST}"),
                RedirectRefusal::ForeignResource,
            ),
            (
                "Download/worksheet-13042099".to_owned(),
                RedirectRefusal::NotHttps,
            ),
        ];
        for (location, why) in expected {
            assert_eq!(
                download_redirect(&location, OWNED),
                DownloadRedirect::Refused(why),
                "{location} is not the captured asset hop"
            );
            assert!(
                !is_asset_url(&location) || matches!(why, RedirectRefusal::ForeignResource),
                "and the transport refuses it too, except where only the resource binding — \
                 which the transport cannot know — is what failed: {location}"
            );
        }
    }

    /// The origin's own answers, which are read before the asset rule and
    /// are not affected by it.
    #[test]
    fn the_origins_own_redirects_are_still_read_as_the_origins() {
        assert_eq!(
            download_redirect("/Request-Authorization?authModal=login", OWNED),
            DownloadRedirect::Authorization,
            "the sign-in gate is the condition the 2026-08-29 server-side probe met"
        );
        assert_eq!(
            download_redirect(&format!("{ORIGIN}/Request-Authorization"), OWNED),
            DownloadRedirect::Authorization,
            "spelled absolutely, it is the same gate"
        );
        assert_eq!(
            download_redirect("/Download/worksheet-13042099?attempt=2", OWNED),
            DownloadRedirect::SameOrigin(format!("{ORIGIN}/Download/worksheet-13042099?attempt=2")),
            "somewhere else on the origin is a hop the session may make"
        );
        assert_eq!(
            download_redirect(&format!("{ORIGIN}.example/Download/x"), OWNED),
            DownloadRedirect::Refused(RedirectRefusal::ForeignHost),
            "a host that merely begins with the origin is another host, not a path on ours"
        );
    }

    #[test]
    fn asset_paths_that_a_url_parser_would_rewrite_are_refused() {
        for segment in [
            r"x\..\..\..\90000042\assets\other",
            "%2e",
            ".%2e",
            "asset\tname",
            "asset\nname",
        ] {
            let location = format!(
                "https://rc-assets.teacherspayteachers.com/resources/13042099/assets/{segment}?verify=synthetic"
            );
            assert!(
                matches!(
                    download_redirect(&location, OWNED),
                    DownloadRedirect::Refused(_)
                ),
                "a noncanonical path must not escape the product binding"
            );
        }
    }

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
    fn both_product_form_urls_are_recognised_as_navigations() {
        assert!(
            is_product_form(&form_page_request(FormTarget::CreateDigital).url),
            "the create render is a document navigation"
        );
        assert!(
            is_product_form(&FormTarget::EditDigital(ProductId(13_042_099)).url()),
            "so is every edit render, whatever product it addresses"
        );
        assert!(
            !is_product_form(&format!("{ORIGIN}/uploads/time?requestTime=1")),
            "the clock read is a plain fetch, not a navigation"
        );
        assert!(
            !is_product_form(&my_product_listings_request(100, 0).url),
            "and a GraphQL call is an XHR"
        );
        assert!(
            !is_product_form("https://s3.amazonaws.com/My-Products/New/Digital-Next"),
            "the path alone decides nothing: another host is not this origin"
        );
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
    fn the_delete_mutation_quotes_its_id_and_names_no_seller() {
        let request = remove_resource_request(ProductId(17_512_457));
        assert!(
            request
                .url
                .ends_with("/graph/graphql?opname=RemoveResource"),
            "the delete is on the product service, not the analytics gateway, got {:?}",
            request.url
        );
        let RequestBody::Json(body) = request.body else {
            panic!("the mutation posts JSON");
        };
        assert_eq!(
            body.pointer("/variables/id"),
            Some(&json!("17512457")),
            "the capture quotes the identifier here and leaves it unquoted on the analytics read"
        );
        assert!(
            body.pointer("/variables/sellerId").is_none(),
            "the operation declares sellerId and the capture omits it; the session names the seller"
        );
        assert!(
            body.pointer("/query")
                .and_then(Value::as_str)
                .is_some_and(|query| query.contains("resourceDelete(input: {id: $id}")),
            "the mutation text is the captured one, field for field"
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
