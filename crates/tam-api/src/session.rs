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

/// The exchange that turns the mint tool's printed token into the HttpOnly
/// cookie the browser will carry: the client cannot set an HttpOnly cookie
/// itself, so the server does, with `Max-Age` honest to the session's own
/// expiry. `Secure` is unconditional — localhost is a secure context in the
/// browsers that matter, and production is TLS.
#[derive(Debug, serde::Deserialize)]
pub struct ExchangeBody {
    pub token: String,
}

fn cookie_header(token_hex: &str, max_age_seconds: i64) -> String {
    format!(
        "{SESSION_COOKIE}={token_hex}; Path=/; HttpOnly; SameSite=Lax; Secure; \
         Max-Age={max_age_seconds}"
    )
}

pub(crate) async fn exchange(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Json(body): axum::Json<ExchangeBody>,
) -> Result<axum::response::Response, APIError> {
    use axum::response::IntoResponse;
    let raw = body.token.trim();
    let raw = raw
        .strip_prefix(&format!("{SESSION_COOKIE}="))
        .unwrap_or(raw);
    let token = SessionToken::from_hex(raw).ok_or_else(unauthenticated)?;
    let now = (state.wall)();
    let identity = SessionRepo::new(state.pool.clone())
        .resolve(&token, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(unauthenticated)?;
    #[expect(
        clippy::integer_division,
        reason = "milliseconds to whole cookie seconds; truncation only shortens the cookie's life toward safety"
    )]
    let max_age = identity.expires_at.0.saturating_sub(now.0) / 1_000;
    let body = crate::Whoami {
        org: identity.org,
        user: identity.user,
    };
    Ok((
        [(axum::http::header::SET_COOKIE, cookie_header(raw, max_age))],
        axum::Json(body),
    )
        .into_response())
}

/// Logout: expire the session server-side and clear the cookie. Always 204 —
/// logging out twice is not an error anyone needs reported.
pub(crate) async fn logout(
    axum::extract::State(state): axum::extract::State<AppState>,
    parts: axum::http::HeaderMap,
) -> Result<axum::response::Response, APIError> {
    use axum::response::IntoResponse;
    if let Some(token) = parts
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(token_from_cookie_header)
    {
        SessionRepo::new(state.pool.clone())
            .expire(&token)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [(axum::http::header::SET_COOKIE, cookie_header("", 0))],
    )
        .into_response())
}
