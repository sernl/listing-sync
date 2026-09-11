//! The tenant boundary as one extractor: the session cookie resolves to the
//! organisation and user a request speaks for, or the request is refused
//! with the structured 401 before any handler runs. The stream route is the
//! one deliberate exception — a failed SSE response permanently stops the
//! browser's `EventSource` from reconnecting, so it answers a dead session
//! with 204, the specified stop signal, through [`StreamAuth`].

use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, StatusCode};
use tam_storage::{EntitlementRepo, NewTenant, OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId};

use crate::entitlement::Entitlement;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::AppState;

/// The cookie the session token travels in. HttpOnly and SameSite are the
/// serving deployment's to set when it mints; the API only ever reads.
pub const SESSION_COOKIE: &str = "tam_session";

/// Who an authenticated request speaks for, and what their plan allows.
///
/// The entitlement is resolved here rather than by each handler, because
/// here is the only place every authenticated request passes through: a gate
/// a handler has to remember to ask for is a gate that is missing from
/// whichever handler ships next. It costs one indexed read of one
/// organisation's live grants per request, beside the session resolution
/// that already happens.
///
/// No longer `Copy`: a grant carries the Paddle identifier it came from, and
/// a heap string is worth more than the convenience of an implicit copy on a
/// value every handler takes by move anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgContext {
    pub org: OrgId,
    pub user: UserId,
    pub entitlement: Entitlement,
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
    let now = (state.wall)();
    let identity = SessionRepo::new(state.pool.clone())
        .resolve(&token, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let Some(identity) = identity else {
        return Ok(None);
    };
    let grant = EntitlementRepo::new(state.pool.clone())
        .current(identity.org, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Some(OrgContext {
        org: identity.org,
        user: identity.user,
        entitlement: Entitlement::of(grant),
    }))
}

impl FromRequestParts<AppState> for OrgContext {
    type Rejection = APIError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, APIError> {
        resolve(parts, state).await?.ok_or_else(unauthenticated)
    }
}

/// Who an operator request speaks for.
///
/// The user and no organisation, deliberately. Every route taking this
/// extractor is org-independent, and a field naming one would be an
/// invitation to scope a cross-tenant read by accident.
///
/// The marking is read from `platform_operator` on every request rather than
/// carried in the session or asserted by a token, so a revocation takes
/// effect on the next request rather than at the next login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperatorContext {
    pub user: UserId,
}

impl FromRequestParts<AppState> for OperatorContext {
    type Rejection = APIError;

    /// A caller who is not an operator is refused exactly as one with no
    /// session at all: the same 401, the same code, the same body.
    ///
    /// Two reasons, and the second is the one that decides it. The session
    /// store already answers this way — an unknown token and an expired one
    /// are both `None`, and the caller cannot tell them apart — so this
    /// matches the refusal the surrounding code already makes. And a distinct
    /// 403 would turn every `/admin` route into an oracle: a seller probing
    /// one would learn from the status alone that their session is live and
    /// that the operator surface is real, which is precisely the pair of
    /// facts an attacker enumerating this platform wants confirmed.
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, APIError> {
        let context = resolve(parts, state).await?.ok_or_else(unauthenticated)?;
        let active = OperatorRepo::new(state.pool.clone())
            .is_active(context.user)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        if !active {
            return Err(unauthenticated());
        }
        Ok(Self { user: context.user })
    }
}

/// The stream route's authentication: identical resolution, but a missing or
/// dead session is `Stop` — rendered as a bare 204 — because 401 as a stream
/// response would permanently halt the client's reconnection.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// What the exchange accepts: either the identity service's login assertion
/// or the token `tam-mint-session` printed, told apart by shape.
///
/// The exchange turns whichever it is into the HttpOnly cookie the browser
/// will carry: the client cannot set an HttpOnly cookie itself, so the server
/// does, with `Max-Age` honest to the session's own expiry. `Secure` is
/// unconditional — localhost is a secure context in the browsers that matter,
/// and production is TLS.
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

/// How long a session minted from an identity assertion lives.
///
/// No longer than the identity service's own default session expiry, so a
/// client that fails to call `DELETE /{version}/session` on sign-out cannot
/// leave an API session outliving the identity that authorised it
/// (`docs/notes/design/better-auth-integration.md`, "Where the two systems
/// can diverge").
const ASSERTED_SESSION_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// What the exchange settled on: the token the cookie will carry, and the
/// identity that token now names.
struct Exchanged {
    token_hex: String,
    org: OrgId,
    user: UserId,
    expires_at: Timestamp,
}

fn fresh_uuid() -> tam_types::Uuid {
    tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes())
}

/// Two v4 UUIDs concatenated: 32 bytes from the generator the workspace
/// already trusts for row identity, which is what `tam-mint-session` mints.
fn fresh_session_token() -> SessionToken {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    SessionToken(bytes)
}

/// The address a self-serve signup's `app_user` row carries.
///
/// The assertion carries no email: the contract's claim set is `sub`, `iss`,
/// `aud`, `iat`, `exp` and `email_verified`, and the address stays owned by
/// the identity service, which lets a user change it. `app_user.email` is
/// nonetheless `NOT NULL UNIQUE`, so the row needs something. A
/// subject-derived address under the reserved `.invalid` top-level domain
/// (RFC 2606) is unique by construction and unroutable by definition, which
/// is the honest rendering of "this system does not hold your address".
fn placeholder_email(subject: tam_types::Uuid) -> String {
    format!("{}@subject.invalid", uuid::Uuid::from_bytes(subject.0))
}

/// The name a self-serve signup's organisation carries until its owner sets
/// one. Derived from the subject for the same reason the address is.
fn provisional_org_name(subject: tam_types::Uuid) -> String {
    format!("org-{}", uuid::Uuid::from_bytes(subject.0))
}

/// The break-glass path: the token `tam-mint-session` printed already names a
/// session row, so the exchange resolves it and re-uses that same token in
/// the cookie rather than minting a second one.
async fn from_minted_token(
    state: &AppState,
    raw: &str,
    now: Timestamp,
) -> Result<Exchanged, APIError> {
    let token = SessionToken::from_hex(raw).ok_or_else(unauthenticated)?;
    let identity = SessionRepo::new(state.pool.clone())
        .resolve(&token, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(unauthenticated)?;
    Ok(Exchanged {
        token_hex: raw.to_owned(),
        org: identity.org,
        user: identity.user,
        expires_at: identity.expires_at,
    })
}

/// The identity-service path: verify the assertion, resolve its subject to a
/// tenant — provisioning one on first sight, which is self-serve signup — and
/// mint a fresh session.
///
/// Nothing in the assertion names the organisation. The subject is looked up
/// in `app_user`, so the tenant boundary is drawn from our own table and a
/// forged claim has nothing to widen.
async fn from_assertion(
    state: &AppState,
    raw: &str,
    now: Timestamp,
) -> Result<Exchanged, APIError> {
    let bridge = state.auth.as_ref().ok_or_else(unauthenticated)?;
    let verified = bridge.verify(raw, now).await.ok_or_else(unauthenticated)?;

    let repo = SessionRepo::new(state.pool.clone());
    let linked = repo
        .user_by_auth_subject(verified.subject)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let (org, user) = match linked {
        Some(linked) => linked,
        None => repo
            .provision_for_auth_subject(
                NewTenant {
                    subject: verified.subject,
                    org: OrgId(fresh_uuid()),
                    user: UserId(fresh_uuid()),
                    email: &placeholder_email(verified.subject),
                    org_name: &provisional_org_name(verified.subject),
                },
                now,
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?,
    };

    let token = fresh_session_token();
    let expires_at = Timestamp(now.0.saturating_add(ASSERTED_SESSION_TTL_MS));
    repo.mint(&token, user, expires_at, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Exchanged {
        token_hex: token.to_hex(),
        org,
        user,
        expires_at,
    })
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
    let now = (state.wall)();
    // Shape alone decides the path, and neither falls back to the other: a
    // JWT has dots, the mint tool's token is 64 hex characters, and no string
    // is both. A fallback would let a failed assertion be retried as a token.
    let settled = if raw.contains('.') {
        from_assertion(&state, raw, now).await?
    } else {
        from_minted_token(&state, raw, now).await?
    };
    #[expect(
        clippy::integer_division,
        reason = "milliseconds to whole cookie seconds; truncation only shortens the cookie's life toward safety"
    )]
    let max_age = settled.expires_at.0.saturating_sub(now.0) / 1_000;
    // A fresh signup reaches here having just had its organisation created,
    // so this is the first answer that can tell the console whether the claim
    // screen stands before it. Reading it here rather than leaving the client
    // to a second call is what keeps the gate from flashing the console first.
    let (slug, slug_prompt) = crate::org::slug_state(&state, settled.org).await?;
    let body = crate::Whoami {
        org: settled.org,
        user: settled.user,
        slug,
        slug_prompt,
    };
    Ok((
        [(
            axum::http::header::SET_COOKIE,
            cookie_header(&settled.token_hex, max_age),
        )],
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
