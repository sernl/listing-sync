//! The TPT adapter: a typed client for TeachersPayTeachers' two
//! cookie-authenticated GraphQL services and for the legacy CakePHP product
//! form beside them, behind the marketplace seam.
//!
//! The read half is `MyProductListings` on `/graph/graphql`, which enumerates
//! the seller's own catalogue, and `storeResourceTotalsAllTimeStats` on
//! `/gateway/graphql`, which reads its per-resource analytics.
//!
//! The write half is a plain form POST behind a cookie session, and the
//! captures settle that it needs no browser: no captcha token, no bot-management
//! token and no JavaScript-derived value appears on either the create or the
//! edit. What it does need is out of band. A create is thirteen hops — render
//! the form for its `SecurityComponent` token triple and its published AWS
//! key id, reserve an S3 object, then read TPT's clock and sign through its
//! own oracle once per S3 call, store the bytes, and walk two async jobs that
//! exchange the staged object for the two opaque handles the form consumes —
//! before the forty-three-field multipart navigation that answers 302 with the
//! new product id in its `Location`. The capture read the clock once for the
//! whole upload; a multi-part upload outlives AWS's skew window, so this reads
//! it per signed call and the count is two higher than the recording's.
//! An update is the forty-eight-field edit form, and publishing is that same
//! form with its status selector moved. A delete is the `RemoveResource`
//! mutation back on `/graph/graphql`, whose answer echoes the id it removed.
//!
//! Two things are deliberately absent and one is inferred. A create takes
//! exactly one file into the product slot, because the preview, video and
//! manual-thumbnail slots are exercised by nothing. An unrecognised answer to
//! the final POST is an ambiguity rather than a rejection, because no capture
//! contains a refusal and inferring the shape of one would be invention. The
//! inference is the paid create: both captured creates are free, so a price on
//! the create form is the captured paid edit's money fields moved onto the
//! form that declares the same names, reachable only from a projection that
//! deliberately carries one.

#![forbid(unsafe_code)]

pub mod classify;
pub mod endpoints;
pub mod flows;
pub mod form;
pub mod identity;
#[cfg(feature = "live")]
pub mod live;
pub mod read_model;
pub mod s3;
pub mod session;
pub mod upload;
pub mod write_model;

pub use classify::SubmitLanding;
pub use endpoints::{
    AllTimeMetric, FormTarget, MetricResolution, ResolvedMetric, ResolvedStatsQuery, StatsWindow,
};
pub use flows::{listing_state_from_status, TptAdapter};
pub use form::{TptFormPage, TptFormTokens};
pub use identity::{read_seller_store_id, seller_store_id, StoreId};
#[cfg(feature = "live")]
pub use live::{GatewayTransport, ReqwestTransport};
pub use read_model::{
    parse_upload_page_product, ProductId, ResourceStat, TptCatalogueEntry, TptCategory, TptPrice,
    UploadPageProduct,
};
pub use s3::{UploadTicket, PART_SIZE};
pub use session::{SessionError, TptSession};
pub use upload::{InstantPause, ProcessedHandle};
pub use write_model::{
    AuthorshipDeclaration, ListingPrice, PaidPrice, PriceError, StatusUser, TaxCode, TptListing,
};

#[cfg(test)]
mod guard;
