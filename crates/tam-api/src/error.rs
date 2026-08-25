//! The structured error body every failing route returns, and the rule about
//! what a response may disclose.
//!
//! Adopted from the founder's reference repository, `sheroz/axum-rest-api-sample`,
//! with three substitutions. The code and kind are closed enums rather than
//! free strings, so a typo is a compile error instead of a value the client
//! cannot match on. The timestamp and the trace identifier enter as data
//! rather than being read from the clock or a random source, which the lint
//! table requires. The disclosure split is an explicit [`Disclosure`] value
//! rather than a `cfg!` read at the point of use, so both sides of it are
//! reachable from a test.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tam_types::Timestamp;

/// What a redacted entry says in place of the internals it withheld.
pub const REDACTED_INTERNAL_MESSAGE: &str = "internal error";

/// Whether an entry may carry internal detail across the trust boundary.
///
/// The variant is a value rather than a `cfg!` read inside each constructor so
/// that a test can exercise the withholding path from a debug build, where
/// every test necessarily runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    Full,
    Redacted,
}

impl Default for Disclosure {
    /// Redacted, so a path that never received configuration fails closed.
    /// There is deliberately no build-type signal here: this workspace ships
    /// `debug-assertions = true` in release, so `cfg!(debug_assertions)` (the
    /// reference repository's key) cannot tell production apart. `Full` is
    /// granted only by explicit configuration at the process boundary.
    fn default() -> Self {
        Self::Redacted
    }
}

/// The closed set of machine-readable error codes. The client matches on these,
/// so a variant is a cross-layer contract rather than a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum APIErrorCode {
    UnsupportedApiVersion,
    VersionParameterMissing,
    VersionParameterUnreadable,
    SessionRequired,
    IdempotencyKeyRequired,
    SyncMappingsInvalid,
    DuplicateSyncItem,
    ResourceMissing,
    BrokerUnavailable,
    Internal,
}

impl APIErrorCode {
    /// The closed set, in a stable order; the closed-set test and the
    /// vocabulary generator read this single source.
    pub const ALL: [Self; 10] = [
        Self::UnsupportedApiVersion,
        Self::VersionParameterMissing,
        Self::VersionParameterUnreadable,
        Self::SessionRequired,
        Self::IdempotencyKeyRequired,
        Self::SyncMappingsInvalid,
        Self::DuplicateSyncItem,
        Self::ResourceMissing,
        Self::BrokerUnavailable,
        Self::Internal,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedApiVersion => "unsupported_api_version",
            Self::VersionParameterMissing => "version_parameter_missing",
            Self::VersionParameterUnreadable => "version_parameter_unreadable",
            Self::SessionRequired => "session_required",
            Self::IdempotencyKeyRequired => "idempotency_key_required",
            Self::SyncMappingsInvalid => "sync_mappings_invalid",
            Self::DuplicateSyncItem => "duplicate_sync_item",
            Self::ResourceMissing => "resource_missing",
            Self::BrokerUnavailable => "broker_unavailable",
            Self::Internal => "internal",
        }
    }
}

impl core::fmt::Display for APIErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The coarse class a code belongs to, which is what a client branches on when
/// it does not recognise the code itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum APIErrorKind {
    Validation,
    Unauthenticated,
    NotFound,
    Internal,
}

impl APIErrorKind {
    /// The closed set, in a stable order, for the same two readers.
    pub const ALL: [Self; 4] = [
        Self::Validation,
        Self::Unauthenticated,
        Self::NotFound,
        Self::Internal,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Unauthenticated => "unauthenticated",
            Self::NotFound => "not_found",
            Self::Internal => "internal",
        }
    }
}

impl core::fmt::Display for APIErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One fault. A response carries a list of these, because a validation pass
/// reports every field it rejected rather than only the first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct APIErrorEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<APIErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<APIErrorKind>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<Timestamp>,
}

impl APIErrorEntry {
    #[must_use]
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
            ..Self::default()
        }
    }

    /// A fault the caller cannot act on, carrying the internals only where
    /// [`Disclosure::Full`] permits it.
    ///
    /// The withheld text is not logged here, because this crate holds no
    /// logger; the caller logs `internals` against the same `trace_id` it
    /// passes in, and a caller that does not has destroyed the only copy.
    #[must_use]
    pub fn internal(disclosure: Disclosure, internals: &str, trace_id: &str) -> Self {
        let message = match disclosure {
            Disclosure::Full => internals,
            Disclosure::Redacted => REDACTED_INTERNAL_MESSAGE,
        };
        Self::new(message)
            .code(APIErrorCode::Internal)
            .kind(APIErrorKind::Internal)
            .trace_id(trace_id)
    }

    #[must_use]
    pub const fn code(mut self, code: APIErrorCode) -> Self {
        self.code = Some(code);
        self
    }

    #[must_use]
    pub const fn kind(mut self, kind: APIErrorKind) -> Self {
        self.kind = Some(kind);
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    #[must_use]
    pub fn reason(mut self, reason: &str) -> Self {
        self.reason = Some(reason.to_owned());
        self
    }

    #[must_use]
    pub fn instance(mut self, instance: &str) -> Self {
        self.instance = Some(instance.to_owned());
        self
    }

    #[must_use]
    pub fn trace_id(mut self, trace_id: &str) -> Self {
        self.trace_id = Some(trace_id.to_owned());
        self
    }

    #[must_use]
    pub const fn at(mut self, at: Timestamp) -> Self {
        self.at = Some(at);
        self
    }
}

/// The response body itself. `status` repeats the HTTP status inside the
/// payload, because a client reading a logged body has no envelope left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct APIError {
    pub status: u16,
    pub errors: Vec<APIErrorEntry>,
}

impl APIError {
    #[must_use]
    pub fn new(status: StatusCode, entry: APIErrorEntry) -> Self {
        Self {
            status: status.as_u16(),
            errors: vec![entry],
        }
    }

    #[must_use]
    pub fn with_entries(status: StatusCode, errors: Vec<APIErrorEntry>) -> Self {
        Self {
            status: status.as_u16(),
            errors,
        }
    }

    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl core::fmt::Display for APIError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} (", self.status)?;
        for (position, entry) in self.errors.iter().enumerate() {
            if position > 0 {
                f.write_str("; ")?;
            }
            f.write_str(&entry.message)?;
        }
        f.write_str(")")
    }
}

impl core::error::Error for APIError {}

impl IntoResponse for APIError {
    fn into_response(self) -> Response {
        (self.status_code(), Json(self)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        APIError, APIErrorCode, APIErrorEntry, APIErrorKind, Disclosure, REDACTED_INTERNAL_MESSAGE,
    };
    use axum::http::StatusCode;
    use tam_types::Timestamp;

    /// The wire name is a cross-layer contract, and the derive and `as_str`
    /// are two sources for it. The single or-pattern arm carries no wildcard,
    /// so adding a variant fails compilation here rather than shipping a name
    /// the client has never seen.
    #[test]
    fn every_code_serialises_to_its_own_str() {
        for code in APIErrorCode::ALL {
            match code {
                APIErrorCode::UnsupportedApiVersion
                | APIErrorCode::VersionParameterMissing
                | APIErrorCode::VersionParameterUnreadable
                | APIErrorCode::SessionRequired
                | APIErrorCode::IdempotencyKeyRequired
                | APIErrorCode::SyncMappingsInvalid
                | APIErrorCode::DuplicateSyncItem
                | APIErrorCode::ResourceMissing
                | APIErrorCode::BrokerUnavailable
                | APIErrorCode::Internal => {}
            }
            let encoded = serde_json::to_string(&code).expect("an error code serialises");
            assert_eq!(
                encoded,
                format!("\"{}\"", code.as_str()),
                "the serde name and as_str must agree for {code}"
            );
        }
    }

    #[test]
    fn every_kind_serialises_to_its_own_str() {
        for kind in APIErrorKind::ALL {
            match kind {
                APIErrorKind::Validation
                | APIErrorKind::Unauthenticated
                | APIErrorKind::NotFound
                | APIErrorKind::Internal => {}
            }
            let encoded = serde_json::to_string(&kind).expect("an error kind serialises");
            assert_eq!(
                encoded,
                format!("\"{}\"", kind.as_str()),
                "the serde name and as_str must agree for {kind}"
            );
        }
    }

    #[test]
    fn the_builder_sets_only_what_it_is_given() {
        let entry = APIErrorEntry::new("the version is not one this build serves")
            .code(APIErrorCode::UnsupportedApiVersion)
            .kind(APIErrorKind::Validation)
            .reason("supported versions are v1 and v2")
            .instance("/v9/healthz")
            .at(Timestamp(1));

        assert_eq!(
            entry.code,
            Some(APIErrorCode::UnsupportedApiVersion),
            "the builder must record the code it was given"
        );
        assert_eq!(
            entry.kind,
            Some(APIErrorKind::Validation),
            "the builder must record the kind it was given"
        );
        assert_eq!(
            entry.at,
            Some(Timestamp(1)),
            "the timestamp enters as data and must survive the builder"
        );
        assert!(
            entry.detail.is_none() && entry.trace_id.is_none(),
            "an unset field stays unset rather than acquiring a default"
        );
    }

    #[test]
    fn an_unset_field_is_absent_from_the_wire() {
        let entry = APIErrorEntry::new("plain");
        let encoded = serde_json::to_value(&entry).expect("an entry serialises");
        let object = encoded.as_object().expect("an entry encodes as an object");
        assert_eq!(
            object.len(),
            1,
            "only the message is set, so only the message is serialised: {encoded}"
        );
    }

    #[test]
    fn full_disclosure_keeps_the_internals() {
        let entry =
            APIErrorEntry::internal(Disclosure::Full, "connection refused: 127.0.0.1", "t1");
        assert_eq!(
            entry.message, "connection refused: 127.0.0.1",
            "a full-disclosure build states the fault"
        );
        assert_eq!(
            entry.trace_id.as_deref(),
            Some("t1"),
            "the trace id is carried under either disclosure mode"
        );
    }

    #[test]
    fn redacted_disclosure_withholds_the_internals() {
        let entry =
            APIErrorEntry::internal(Disclosure::Redacted, "connection refused: 127.0.0.1", "t1");
        assert_eq!(
            entry.message, REDACTED_INTERNAL_MESSAGE,
            "a redacting build must not name the fault"
        );
        assert!(
            !entry.message.contains("127.0.0.1"),
            "a redacting build must not leak an internal address"
        );
        assert_eq!(
            entry.trace_id.as_deref(),
            Some("t1"),
            "the trace id survives redaction, or the log cannot be joined to the response"
        );
    }

    #[test]
    fn disclosure_defaults_to_redacted() {
        assert_eq!(
            Disclosure::default(),
            Disclosure::Redacted,
            "an unconfigured path must fail closed rather than disclose"
        );
    }

    #[test]
    fn the_status_is_carried_in_the_body_and_the_envelope() {
        let error = APIError::new(
            StatusCode::BAD_REQUEST,
            APIErrorEntry::new("unknown version"),
        );
        assert_eq!(error.status, 400, "the body repeats the HTTP status");
        assert_eq!(
            error.status_code(),
            StatusCode::BAD_REQUEST,
            "the envelope status is recovered from the body status"
        );
    }

    #[test]
    fn an_out_of_range_status_falls_back_rather_than_panicking() {
        let error = APIError {
            status: 7,
            errors: vec![],
        };
        assert_eq!(
            error.status_code(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "a status outside the HTTP range must not panic the response path"
        );
    }
}
