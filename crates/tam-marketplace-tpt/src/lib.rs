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
//! edit. What it does need is out of band. A create is eleven hops — render
//! the form for its `SecurityComponent` token triple and its published AWS
//! key id, reserve an S3 object, sign each S3 call through TPT's own signing
//! oracle, store the bytes, then walk two async jobs that exchange the staged
//! object for the two opaque handles the form consumes — before the
//! forty-three-field multipart navigation that answers 302 with the new
//! product id in its `Location`. Publishing is the forty-eight-field edit form
//! with its status selector moved.
//!
//! Three things are deliberately absent. A paid create is refused, because no
//! paid create is captured and the form's own minimum price names no
//! currency. A create takes exactly one file into the product slot, because
//! the preview, video and manual-thumbnail slots are exercised by nothing. And
//! an unrecognised answer to the final POST is an ambiguity rather than a
//! rejection, because no capture contains a refusal and inferring the shape of
//! one would be invention.

#![forbid(unsafe_code)]

pub mod classify;
pub mod endpoints;
pub mod flows;
pub mod form;
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
pub use flows::TptAdapter;
pub use form::{TptFormPage, TptFormTokens};
pub use live::ReqwestTransport;
pub use read_model::{ProductId, ResourceStat, TptCatalogueEntry, TptCategory, TptPrice};
pub use s3::{UploadTicket, PART_SIZE};
pub use session::{SessionError, TptSession};
pub use upload::{InstantPause, ProcessedHandle};
pub use write_model::{AuthorshipDeclaration, StatusUser, TaxCode, TptListing};

#[cfg(test)]
mod guard {
    /// The seller's payout configuration is reachable from the same session
    /// this connector holds, and nothing this connector does needs it. The
    /// scan is over this crate's own sources rather than over a list of
    /// constants, so a query text, a url fragment or a field name added
    /// anywhere in the crate trips it.
    #[test]
    fn no_endpoint_in_this_crate_addresses_the_sellers_money() {
        const FORBIDDEN: [&str; 4] = ["payout", "hyperwallet", "bank", "SellerPayoutPreferences"];
        const SOURCES: [(&str, &str); 9] = [
            ("classify.rs", include_str!("classify.rs")),
            ("endpoints.rs", include_str!("endpoints.rs")),
            ("flows.rs", include_str!("flows.rs")),
            ("form.rs", include_str!("form.rs")),
            ("live.rs", include_str!("live.rs")),
            ("read_model.rs", include_str!("read_model.rs")),
            ("s3.rs", include_str!("s3.rs")),
            ("session.rs", include_str!("session.rs")),
            ("write_model.rs", include_str!("write_model.rs")),
        ];
        for (name, source) in SOURCES {
            let lowered = source.to_lowercase();
            for needle in FORBIDDEN {
                assert!(
                    !lowered.contains(&needle.to_lowercase()),
                    "{name} names {needle:?}: this connector reads and writes products, and the \
                     seller's money is not its business"
                );
            }
        }
    }
}
