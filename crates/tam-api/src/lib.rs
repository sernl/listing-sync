//! The HTTP API as a library, so integration tests drive the router
//! in-process with no bound port.
//!
//! The crate owns the router, the extractors and the error mapping; a binary
//! that serves it owns a listener and nothing else. Every route mounted under
//! `/{version}` receives its version through the [`APIVersion`] extractor, so
//! an unknown version is refused with the same structured body as any other
//! fault rather than falling through to a bare not-found.

#![forbid(unsafe_code)]

pub mod error;
pub mod version;

use axum::{http::StatusCode, routing::get, Json, Router};
use serde::{Deserialize, Serialize};

pub use crate::{
    error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind, Disclosure},
    version::{APIVersion, VersionError},
};

/// Everything the serving binary decides and the library consumes. One value
/// crosses the boundary so a decision cannot arrive ambiently; handlers read
/// it from router state. Its `Default` is the fail-closed posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Config {
    pub disclosure: Disclosure,
}

/// What a versioned health probe answers with. The version is echoed back
/// because it is the extractor's own output, which is what the probe exists
/// to exercise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub version: APIVersion,
}

/// The whole API surface this build serves, configured by the binary.
pub fn router(config: Config) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/{version}/healthz", get(versioned_healthz))
        .with_state(config)
}

/// The unversioned liveness probe, which predates any version negotiation and
/// therefore takes no extractor.
async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn versioned_healthz(version: APIVersion) -> Json<Health> {
    Json(Health { version })
}
