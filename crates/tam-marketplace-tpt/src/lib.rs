//! The TPT adapter: a typed client for TeachersPayTeachers' two
//! cookie-authenticated GraphQL services, behind the marketplace seam.
//!
//! This crate is the M7 read slice and only that. `MyProductListings` on
//! `/graph/graphql` enumerates the seller's own catalogue and
//! `storeResourceTotalsAllTimeStats` on `/gateway/graphql` reads its
//! per-resource analytics; both are fully captured and neither needs a
//! browser to describe. The write path is a legacy CakePHP multipart form
//! whose token triple, out-of-band upload and bot-management handling no
//! capture contains, so it is not attempted here — see
//! `docs/design/plans/2026-08-28-m7-tpt-connector.md`.

#![forbid(unsafe_code)]

pub mod classify;
pub mod endpoints;
pub mod flows;
pub mod live;
pub mod read_model;
pub mod session;

pub use endpoints::{
    AllTimeMetric, MetricResolution, ResolvedMetric, ResolvedStatsQuery, StatsWindow,
};
pub use flows::TptAdapter;
pub use live::ReqwestTransport;
pub use read_model::{ProductId, ResourceStat, TptCatalogueEntry, TptCategory, TptPrice};
pub use session::{SessionError, TptSession};
