//! The API version as a request-parts extractor.
//!
//! Adopted from the founder's reference repository, `sheroz/axum-rest-api-sample`.
//! A handler takes [`APIVersion`] as an argument and receives a value from a
//! closed set, so versioning is decided once here rather than by string
//! handling inside every handler, and a route reached with a version this
//! build does not serve is refused before the handler body runs.

use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Path},
    http::{request::Parts, StatusCode},
    RequestPartsExt,
};
use serde::{Deserialize, Serialize};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};

/// The path segment the extractor reads.
pub const VERSION_PARAM: &str = "version";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum APIVersion {
    V1,
    V2,
}

impl APIVersion {
    /// Every version this build serves, in the order a client should read them.
    pub const SUPPORTED: [Self; 2] = [Self::V1, Self::V2];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

impl core::fmt::Display for APIVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl core::str::FromStr for APIVersion {
    type Err = VersionError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::SUPPORTED
            .into_iter()
            .find(|candidate| candidate.as_str() == raw)
            .ok_or_else(|| VersionError::Unsupported(raw.to_owned()))
    }
}

/// The three ways a request fails to name a version this build serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    Unsupported(String),
    Missing,
    Unreadable,
}

impl core::fmt::Display for VersionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported(raw) => write!(f, "unsupported api version: {raw}"),
            Self::Missing => f.write_str("the route matched without a version path parameter"),
            Self::Unreadable => f.write_str("the path parameters could not be read"),
        }
    }
}

impl core::error::Error for VersionError {}

impl VersionError {
    const fn code(&self) -> APIErrorCode {
        match self {
            Self::Unsupported(_) => APIErrorCode::UnsupportedApiVersion,
            Self::Missing => APIErrorCode::VersionParameterMissing,
            Self::Unreadable => APIErrorCode::VersionParameterUnreadable,
        }
    }
}

impl From<VersionError> for APIError {
    fn from(error: VersionError) -> Self {
        let supported = APIVersion::SUPPORTED.map(APIVersion::as_str).join(", ");
        let entry = APIErrorEntry::new(&error.to_string())
            .code(error.code())
            .kind(APIErrorKind::Validation)
            .reason(&format!("supported versions: {supported}"));
        Self::new(StatusCode::BAD_REQUEST, entry)
    }
}

impl<S> FromRequestParts<S> for APIVersion
where
    S: Send + Sync,
{
    type Rejection = APIError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let params: Path<HashMap<String, String>> = parts
            .extract()
            .await
            .map_err(|_| VersionError::Unreadable)?;
        let raw = params.get(VERSION_PARAM).ok_or(VersionError::Missing)?;
        Ok(raw.parse()?)
    }
}

#[cfg(test)]
mod tests {
    use super::{APIVersion, VersionError};
    use crate::error::{APIError, APIErrorCode, APIErrorKind};

    #[test]
    fn every_supported_version_parses_back_to_itself() {
        for version in APIVersion::SUPPORTED {
            match version {
                APIVersion::V1 | APIVersion::V2 => {}
            }
            let parsed: APIVersion = version
                .as_str()
                .parse()
                .expect("a supported version parses from its own str");
            assert_eq!(
                parsed, version,
                "the version str and the parser must agree for {version}"
            );
        }
    }

    #[test]
    fn an_unknown_version_names_what_it_refused() {
        let refused = "v9".parse::<APIVersion>();
        assert_eq!(
            refused,
            Err(VersionError::Unsupported("v9".to_owned())),
            "the refusal carries the token it refused, so the client can see the typo"
        );
    }

    #[test]
    fn the_wire_name_matches_the_str() {
        for version in APIVersion::SUPPORTED {
            let encoded = serde_json::to_string(&version).expect("a version serialises");
            assert_eq!(
                encoded,
                format!("\"{}\"", version.as_str()),
                "the serde name and as_str must agree for {version}"
            );
        }
    }

    #[test]
    fn a_refusal_becomes_a_bad_request_naming_the_supported_set() {
        let error = APIError::from(VersionError::Unsupported("v9".to_owned()));
        assert_eq!(
            error.status, 400,
            "an unsupported version is the caller's error, not the server's"
        );
        let entry = error.errors.first().expect("the refusal carries one entry");
        assert_eq!(
            entry.code,
            Some(APIErrorCode::UnsupportedApiVersion),
            "the client matches on the code rather than the message"
        );
        assert_eq!(
            entry.kind,
            Some(APIErrorKind::Validation),
            "an unsupported version is a validation fault"
        );
        assert_eq!(
            entry.reason.as_deref(),
            Some("supported versions: v1, v2"),
            "the refusal states which versions would have worked"
        );
    }

    #[test]
    fn a_missing_parameter_is_distinguishable_from_an_unsupported_one() {
        let error = APIError::from(VersionError::Missing);
        let entry = error.errors.first().expect("the refusal carries one entry");
        assert_eq!(
            entry.code,
            Some(APIErrorCode::VersionParameterMissing),
            "a route mounted without a version parameter is our bug, and says so"
        );
    }
}
