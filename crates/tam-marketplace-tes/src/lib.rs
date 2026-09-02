//! The Tes adapter: a typed client for Tes's cookie-authenticated internal
//! JSON API, behind the marketplace seam. No browser exists anywhere in this
//! tree; the uploader is a JSON REST API plus a presigned direct-to-S3 form
//! POST, which is the M0 finding the decision record promotes here.
//!
//! Every write outcome is classified by positive assertion: `Committed`-class
//! success requires the response to be JSON carrying the expected id, and the
//! transport's 2xx is never the verdict — the misleading-204 delete is the
//! canonical counterexample.

#![forbid(unsafe_code)]

pub mod classify;
pub mod endpoints;
pub mod flows;
pub mod identity;
#[cfg(feature = "live")]
pub mod live;
pub mod schema;
pub mod session;

pub use endpoints::{CatalogueEntry, DraftId};
pub use flows::{route_name, NotATesInventory, TesAdapter};
pub use identity::{read_seller_user_id, seller_user_id, SellerId};
#[cfg(feature = "live")]
pub use live::{GatewayTransport, ReqwestTransport};
pub use session::{SessionError, TesSession};
