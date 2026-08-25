//! The tenant boundary as one extractor: the session cookie resolves to the
//! organisation and user a request speaks for, or the request is refused
//! with the structured 401 before any handler runs. The stream route is the
//! one deliberate exception — a failed SSE response permanently stops the
//! browser's `EventSource` from reconnecting, so it answers a dead session
//! with 204, the specified stop signal, through [`StreamAuth`].

use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, StatusCode};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, UserId};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::AppState;

/// The cookie the session token travels in. HttpOnly and SameSite are the
/// serving deployment's to set when it mints; the API only ever reads.
pub const SESSION_COOKIE: &str = "tam_session";

/// Who an authenticated request speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrgContext {
    pub org: OrgId,
    pub user: UserId,
}

/// Pulls the session token out of a Cookie header's pair list, tolerating
/// the whitespace variants browsers actually send.
#[must_use]
pub fn token_from_cookie_header(header: &str) -> Option<SessionToken> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        if name.trim() == SESSION_COOKIE {
            SessionToken::from_hex(value.trim())
        } else {
            None
        }
    })
}

fn unauthenticated() -> APIError {
    APIError::new(
        StatusCode::UNAUTHORIZED,
        APIErrorEntry::new("a valid session is required")
            .code(APIErrorCode::SessionRequired)
            .kind(APIErrorKind::Unauthenticated),
    )
}

async fn resolve(parts: &Parts, state: &AppState) -> Result<Option<OrgContext>, APIError> {
    let Some(token) = parts
        .headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(token_from_cookie_header)
    else {
        return Ok(None);
    };
    let identity = SessionRepo::new(state.pool.clone())
        .resolve(&token, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(identity.map(|identity| OrgContext {
        org: identity.org,
        user: identity.user,
    }))
}

impl FromRequestParts<AppState> for OrgContext {
    type Rejection = APIError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, APIError> {
        resolve(parts, state).await?.ok_or_else(unauthenticated)
    }
}

/// The stream route's authentication: identical resolution, but a missing or
/// dead session is `Stop` — rendered as a bare 204 — because 401 as a stream
/// response would permanently halt the client's reconnection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamAuth {
    Live(OrgContext),
    Stop,
}

impl FromRequestParts<AppState> for StreamAuth {
    type Rejection = APIError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, APIError> {
        Ok(resolve(parts, state).await?.map_or(Self::Stop, Self::Live))
    }
}

#[cfg(test)]
mod tests {
    use super::{token_from_cookie_header, SESSION_COOKIE};
    use tam_storage::SessionToken;

    #[test]
    fn the_cookie_parser_finds_the_session_among_others() {
        let token = SessionToken([0x5C; 32]);
        let header = format!("theme=dark; {SESSION_COOKIE}={} ; other=1", token.to_hex());
        assert_eq!(
            token_from_cookie_header(&header),
            Some(token),
            "the session cookie is found regardless of position and spacing"
        );
    }

    #[test]
    fn a_missing_or_malformed_cookie_reads_as_no_session() {
        assert_eq!(token_from_cookie_header(""), None, "empty header");
        assert_eq!(
            token_from_cookie_header("theme=dark"),
            None,
            "no session pair"
        );
        assert_eq!(
            token_from_cookie_header(&format!("{SESSION_COOKIE}=zzzz")),
            None,
            "malformed hex is no session rather than an error"
        );
    }
}
