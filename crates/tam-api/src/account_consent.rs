//! What a person agreed to when their account was made, and again each time
//! the terms change: the record behind the two boxes on the sign-up form.
//!
//! Three writers' worth of routes, one record (migration 0103):
//!
//! - `POST /internal/consent`: the identity service, the moment a sign-up has
//!   created the account. Fenced by a shared secret, like the identity
//!   service's own `/internal/address`; with no secret configured, or a wrong
//!   one offered, the route answers the 404 any unknown path does.
//! - `GET /{version}/consent/status` and `POST /{version}/consent`: the
//!   signed-in console, which asks on every load whether the person has
//!   agreed to the current terms and, when they have not, shows the same two
//!   boxes until they do.
//! - `GET /{version}/admin/consents` and `GET /{version}/admin/users/{subject}/consent`:
//!   the operator's Users page and user detail, read on the backoffice pool.
//!
//! Not to be confused with [`crate::consent`], which is a seller's permission
//! for one marketplace.

use std::net::IpAddr;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tam_storage::{AccountConsentRepo, NewAccountConsent};
use tam_types::{Timestamp, Uuid};

use crate::admin::{backoffice, parse_id};
use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::{AppState, OperatorContext, OrgContext};

/// The Terms of Service's effective date: the version a sign-up agrees to and
/// the one the console asks a signed-in person to accept when it moves.
///
/// The one place this date is written. tam-typegen emits it for the console
/// (`web/src/lib/generated/legal.ts`) and for the landing page
/// (`apps/landing/src/legal.generated.js`), whose Terms page prints it as its
/// effective date, so the page a person reads and the version their agreement
/// records cannot disagree. Moving it is part of changing the terms, and makes
/// every signed-in person agree again on their next visit.
pub const TERMS_VERSION: &str = "2026-10-04";

/// The Privacy Policy's last substantive change, printed on its page. The
/// sign-up's first box covers both documents and records [`TERMS_VERSION`];
/// a privacy change that should ask everyone again moves that date too.
pub const PRIVACY_VERSION: &str = "2026-10-02";

/// The sentence every refusal of an unticked box answers with, matching the
/// identity service's own.
pub const CONSENT_REFUSAL: &str = "Please tick both boxes to continue.";

/// The header the internal route is fenced by: the same name the identity
/// service's address route reads.
pub const INTERNAL_SECRET_HEADER: &str = "x-tam-internal-secret";

/// The shared secret `POST /internal/consent` is fenced by, held so it cannot
/// reach a log line or a `Debug` render of the configuration that carries it.
#[derive(Clone, PartialEq, Eq)]
pub struct InternalSecret(String);

impl InternalSecret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Compared over SHA-256 digests in constant time, as the identity
    /// service compares its own, so neither the secret nor its length is an
    /// oracle.
    fn matches(&self, offered: &str) -> bool {
        let configured = ring::digest::digest(&ring::digest::SHA256, self.0.as_bytes());
        let offered = ring::digest::digest(&ring::digest::SHA256, offered.as_bytes());
        configured.as_ref().ct_eq(offered.as_ref()).into()
    }
}

impl core::fmt::Debug for InternalSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("InternalSecret(redacted)")
    }
}

/// The three statements and the terms version they were read under, as both
/// the sign-up form and the console's re-consent send them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentBody {
    pub terms_privacy: bool,
    pub ip_ownership: bool,
    pub age_18: bool,
    pub version: String,
}

impl ConsentBody {
    fn all_ticked(&self) -> bool {
        self.terms_privacy && self.ip_ownership && self.age_18
    }
}

/// What the identity service sends once the account exists.
#[derive(Debug, Clone, Deserialize)]
pub struct InternalConsentBody {
    /// The identity subject, `auth."user".id`.
    pub subject: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    pub consent: ConsentBody,
}

/// Whether the signed-in person has agreed to the current terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentStatusView {
    pub current_version: String,
    /// The newest version they have agreed to, if any.
    pub accepted_version: Option<String>,
    /// Whether the console must ask before anything else.
    pub required: bool,
}

/// The newest Terms acceptance of one identity subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentSummaryView {
    pub subject: Uuid,
    pub document_version: String,
    pub accepted_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentSummariesView {
    pub consents: Vec<ConsentSummaryView>,
}

/// One stored statement, as the operator's user detail shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRowView {
    pub kind: String,
    pub document_version: String,
    pub accepted_at: Timestamp,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectConsentView {
    pub subject: Uuid,
    pub consents: Vec<ConsentRowView>,
}

fn refusal(status: StatusCode, message: &str) -> APIError {
    APIError::new(
        status,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

/// The 404 an unconfigured deployment or a wrong secret answers, identical to
/// an unknown path's so a caller cannot tell the three apart.
fn not_found() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("not found").kind(APIErrorKind::NotFound),
    )
}

/// `YYYY-MM-DD`, shape only: the column's own check says the same.
fn is_version(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}

fn checked(consent: &ConsentBody) -> Result<(), APIError> {
    if !consent.all_ticked() {
        return Err(refusal(StatusCode::UNPROCESSABLE_ENTITY, CONSENT_REFUSAL));
    }
    if !is_version(&consent.version) {
        return Err(refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the terms version is not a date",
        ));
    }
    Ok(())
}

/// The client's address on the signed-in path, read the way the identity
/// service reads it (`auth/src/client-ip.ts`): Cloudflare's header first,
/// then the first hop of `x-forwarded-for`. Unparseable is absent rather than
/// a refusal: the address is evidence beside the agreement, not a condition
/// of it.
fn client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    header("cf-connecting-ip")
        .or_else(|| header("x-forwarded-for").and_then(|list| list.split(',').next()))
        .and_then(|raw| raw.trim().parse().ok())
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .filter(|agent| !agent.is_empty())
}

/// `POST /internal/consent`: the identity service records a sign-up's
/// agreement. 204 on success; the identity service deletes the account it
/// just made when this answers anything else.
///
/// The body is read as bytes and parsed only once the secret has matched, so
/// a caller without it meets the same 404 whatever it sends, rather than a
/// JSON refusal that would say the route is there.
pub(crate) async fn record_internal(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, APIError> {
    let Some(secret) = state.config.consent_secret.as_ref() else {
        return Err(not_found());
    };
    let offered = headers
        .get(INTERNAL_SECRET_HEADER)
        .and_then(|value| value.to_str().ok());
    if !offered.is_some_and(|offered| secret.matches(offered)) {
        return Err(not_found());
    }
    let body: InternalConsentBody = serde_json::from_slice(&body).map_err(|error| {
        refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("the consent body is not readable: {error}"),
        )
    })?;
    checked(&body.consent)?;
    let subject = parse_id(&body.subject)?;
    let ip_address = body
        .ip_address
        .as_deref()
        .and_then(|raw| raw.trim().parse().ok());
    AccountConsentRepo::new(state.pool.clone())
        .record(&NewAccountConsent {
            subject,
            email: body.email.as_deref(),
            document_version: &body.consent.version,
            accepted_at: (state.wall)(),
            ip_address,
            user_agent: body.user_agent.as_deref().filter(|agent| !agent.is_empty()),
        })
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /{version}/consent/status`.
///
/// A user provisioned before the identity service carries no subject and so
/// has no account to agree with; they are never asked, which is also what a
/// development login minted without one sees.
pub(crate) async fn status(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ConsentStatusView>, APIError> {
    let repo = AccountConsentRepo::new(state.pool.clone());
    let fault = |error: tam_storage::StorageError| state.internal(&error.to_string());
    let subject = repo
        .identity_of(context.user)
        .await
        .map_err(fault)?
        .and_then(|(subject, _)| subject);
    let Some(subject) = subject else {
        return Ok(Json(ConsentStatusView {
            current_version: TERMS_VERSION.to_owned(),
            accepted_version: None,
            required: false,
        }));
    };
    let accepted = repo.accepted(subject, TERMS_VERSION).await.map_err(fault)?;
    let accepted_version = repo.latest_version(subject).await.map_err(fault)?;
    Ok(Json(ConsentStatusView {
        current_version: TERMS_VERSION.to_owned(),
        accepted_version,
        required: !accepted,
    }))
}

/// `POST /{version}/consent`: the signed-in person agrees to the current
/// terms. Agreeing again to a version already on record writes nothing and
/// answers 204, so a double-click is not a second agreement.
pub(crate) async fn accept(
    State(state): State<AppState>,
    context: OrgContext,
    headers: HeaderMap,
    Json(body): Json<ConsentBody>,
) -> Result<StatusCode, APIError> {
    checked(&body)?;
    if body.version != TERMS_VERSION {
        return Err(refusal(
            StatusCode::CONFLICT,
            "The terms changed while this page was open. Reload to read the new version.",
        ));
    }
    let repo = AccountConsentRepo::new(state.pool.clone());
    let fault = |error: tam_storage::StorageError| state.internal(&error.to_string());
    let Some((Some(subject), email)) = repo.identity_of(context.user).await.map_err(fault)? else {
        return Err(refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This account has no sign-in to record an agreement against.",
        ));
    };
    if repo.accepted(subject, TERMS_VERSION).await.map_err(fault)? {
        return Ok(StatusCode::NO_CONTENT);
    }
    repo.record(&NewAccountConsent {
        subject,
        email: Some(&email),
        document_version: TERMS_VERSION,
        accepted_at: (state.wall)(),
        ip_address: client_ip(&headers),
        user_agent: user_agent(&headers),
    })
    .await
    .map_err(fault)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /{version}/admin/consents`: every subject's newest Terms acceptance,
/// for the Users page's Consent column. An account with no line here
/// predates the sign-up boxes.
pub(crate) async fn admin_summaries(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<ConsentSummariesView>, APIError> {
    let summaries = AccountConsentRepo::new(backoffice(&state)?)
        .summaries()
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(ConsentSummariesView {
        consents: summaries
            .into_iter()
            .map(|summary| ConsentSummaryView {
                subject: summary.subject,
                document_version: summary.document_version,
                accepted_at: summary.accepted_at,
            })
            .collect(),
    }))
}

/// `GET /{version}/admin/users/{subject}/consent`: every statement one
/// identity subject agreed to, newest first, with where it was agreed from.
pub(crate) async fn admin_subject(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, subject)): Path<(String, String)>,
) -> Result<Json<SubjectConsentView>, APIError> {
    let subject = parse_id(&subject)?;
    let rows = AccountConsentRepo::new(backoffice(&state)?)
        .for_subject(subject)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(SubjectConsentView {
        subject,
        consents: rows
            .into_iter()
            .map(|row| ConsentRowView {
                kind: row.kind.as_str().to_owned(),
                document_version: row.document_version,
                accepted_at: row.accepted_at,
                ip_address: row.ip_address,
                user_agent: row.user_agent,
                email: row.email,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{client_ip, is_version, InternalSecret, PRIVACY_VERSION, TERMS_VERSION};

    #[test]
    fn the_versions_are_dates() {
        assert!(is_version(TERMS_VERSION));
        assert!(is_version(PRIVACY_VERSION));
        assert!(!is_version("2026-1-04"));
        assert!(!is_version("latest"));
        assert!(!is_version("2026/10/04"));
    }

    #[test]
    fn the_secret_matches_only_itself_and_never_prints() {
        let secret = InternalSecret::new("s3cret".to_owned());
        assert!(secret.matches("s3cret"));
        assert!(!secret.matches("s3cre"));
        assert!(!secret.matches(""));
        assert_eq!(format!("{secret:?}"), "InternalSecret(redacted)");
    }

    #[test]
    fn the_client_address_prefers_cloudflare_then_the_first_hop() {
        let mut headers = HeaderMap::new();
        assert_eq!(client_ip(&headers), None);
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.4, 10.0.0.1"),
        );
        assert_eq!(
            client_ip(&headers).map(|ip| ip.to_string()).as_deref(),
            Some("198.51.100.4")
        );
        headers.insert("cf-connecting-ip", HeaderValue::from_static("2001:db8::1"));
        assert_eq!(
            client_ip(&headers).map(|ip| ip.to_string()).as_deref(),
            Some("2001:db8::1")
        );
    }
}
