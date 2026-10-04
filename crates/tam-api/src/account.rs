//! A seller deletes their own account: `DELETE /v1/account`.
//!
//! The order is the point, because every step but the last is irreversible
//! and the last is the one that forgets who the seller was:
//!
//! 1. Refuse what this route will not do, before anything moves: an operator
//!    steps down first (422), an organisation someone else is in is not
//!    erased (409), and the typed confirmation has to name the organisation
//!    or say `DELETE` (422).
//! 2. The identity service checks the proof — the current password for an
//!    account that has one, and otherwise a sign-in younger than five
//!    minutes — and answers the address and name the goodbye goes to, which
//!    this request holds until the end.
//! 3. Stripe cancels the subscription now, not at the period's end, and the
//!    answer is recorded so the erasure reads it as ended.
//! 4. The identity service deletes the account: sessions, linked accounts
//!    and passkeys with it.
//! 5. [`erase`] removes the platform user and their organisation and writes
//!    the `account_deletion` row in the same transaction (migration 0104).
//! 6. Only then does the goodbye mail go, to the address captured in step 2,
//!    so it never lands for a deletion that stopped halfway. A relay that
//!    refuses it is logged and reported in the answer; the deletion stands.
//!
//! The identity service, the relay and the address are the serving binary's
//! (`tam-server`'s `offboarding.rs`), reached through [`Offboarding`], so this
//! crate holds no identity-service client and no mail template. A deployment
//! without one answers 503 and deletes nothing.
//!
//! Organisations are one person each in practice: signing in provisions an
//! organisation per identity (`SessionRepo::provision_for_auth_subject`) and
//! nothing adds a member. `app_user.org_id` is not unique, though, so the
//! schema admits a second member, and an organisation that has one is
//! refused here as the operator's deletion refuses it, because erasing it
//! would erase the other member's work.

use core::fmt::Write as _;
use std::{future::Future, pin::Pin, sync::Arc};

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use tam_storage::{
    BillingRepo, DeletionRecord, Erased, ErasureRefusal, ErasureRepo, OrgRepo, SessionRepo,
    Standing,
};
use tam_types::{OrgId, Uuid};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::session::OrgContext;
use crate::AppState;

/// The word a seller with no organisation name to type can type instead.
pub const CONFIRM_WORD: &str = "DELETE";

/// The longest reason the deletion keeps, in characters (migration 0104).
pub const REASON_MAX_CHARS: usize = 1000;

/// The cookie the identity service's session rides on, under either of the
/// names better-auth gives it: `__Secure-` prefixed over HTTPS, bare in
/// development. Forwarded to the identity service so it can say how old this
/// browser's sign-in is; nothing here reads its value.
const IDENTITY_SESSION_COOKIE: &str = "better-auth.session_token";

// --------------------------------------------------------------------- port

/// The person being deleted, as the identity service holds them, for the
/// length of one request.
#[derive(Clone, PartialEq, Eq)]
pub struct Departing {
    pub email: String,
    pub name: Option<String>,
}

impl Departing {
    /// The first word of the name, for the goodbye's greeting.
    #[must_use]
    pub fn first_name(&self) -> Option<&str> {
        self.name.as_deref()?.split_whitespace().next()
    }
}

impl core::fmt::Debug for Departing {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Departing(redacted)")
    }
}

/// What the seller offered as proof that it is them.
#[derive(Clone, Copy)]
pub struct Proof<'a> {
    /// The current password, for an account that has one.
    pub password: Option<&'a str>,
    /// This browser's identity-service session cookie(s), as `name=value`
    /// pairs, for an account that signs in another way.
    pub session_cookie: Option<&'a str>,
}

impl core::fmt::Debug for Proof<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Proof")
            .field("password", &self.password.map(|_| "redacted"))
            .field("session_cookie", &self.session_cookie.map(|_| "redacted"))
            .finish()
    }
}

/// The identity service's verdict on a [`Proof`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reauthentication {
    Confirmed(Departing),
    /// The account has a password and none was offered.
    PasswordRequired,
    /// The password offered is not the account's.
    PasswordWrong,
    /// No password on the account, and this browser's sign-in is older than
    /// five minutes, or is not this account's.
    NotFresh,
    /// The identity service holds no such subject.
    NoIdentity,
}

/// A step the identity service or the relay could not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffboardingFault(pub String);

impl core::fmt::Display for OffboardingFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type OffboardingFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, OffboardingFault>> + Send + 'a>>;

/// The three things only the serving binary can do for a deletion.
pub trait Offboarding: Send + Sync {
    /// Check the proof, and answer the address on success.
    fn reauthenticate<'a>(
        &'a self,
        subject: Uuid,
        proof: Proof<'a>,
    ) -> OffboardingFuture<'a, Reauthentication>;

    /// Send "Goodbye from Teachouse", after the deletion has committed.
    fn farewell<'a>(&'a self, to: &'a Departing) -> OffboardingFuture<'a, ()>;

    /// Delete the identity account, its sessions, linked accounts and
    /// passkeys. An identity already gone is success.
    fn delete_identity(&self, subject: Uuid) -> OffboardingFuture<'_, ()>;
}

/// The port as [`crate::Config`] carries it: comparable by identity, so the
/// configuration keeps its derived equality, and opaque in a `Debug` render.
#[derive(Clone)]
pub struct OffboardingPort(pub Arc<dyn Offboarding>);

impl core::fmt::Debug for OffboardingPort {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OffboardingPort")
    }
}

impl PartialEq for OffboardingPort {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for OffboardingPort {}

// ------------------------------------------------------------------ erasure

/// The one erasure both deletions go through: the operator's
/// (`DELETE /v1/admin/users/{subject}`) with no record, and the seller's own
/// with one. A refusal is the caller's to word, because the operator and the
/// seller are told different things about the same fact.
pub(crate) async fn erase(
    state: &AppState,
    subject: Uuid,
    record: Option<&DeletionRecord>,
) -> Result<Result<Erased, ErasureRefusal>, APIError> {
    let repo = ErasureRepo::new(state.pool.clone());
    let outcome = match record {
        Some(record) => repo.erase_departing(subject, record).await,
        None => repo.erase_account(subject).await,
    };
    outcome.map_err(|error| state.internal(&error.to_string()))
}

// -------------------------------------------------------------------- route

/// What the seller sends: the typed confirmation, the password where the
/// account has one, and a reason if they want to give one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteAccountBody {
    pub confirm: String,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// What the deletion did, for the console's last screen before it signs out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDeletedView {
    /// Whether a live subscription was cancelled at Stripe on the way out.
    pub subscription_cancelled: bool,
    /// Whether the goodbye mail was accepted by the relay.
    pub goodbye_sent: bool,
}

/// The seller deletes their own account. See the module note for the order.
///
/// Answers 200 with [`AccountDeletedView`] and an expired session cookie.
/// Refusals carry a `refusal` detail the console words from: `operator`
/// (422), `confirmation` (422), `reason` (422), `password_required` (422),
/// `shared_organisation` (409), `no_identity` (409), `password` (403) and
/// `reauthenticate` (403). 503 where the deployment cannot reach the
/// identity service or Stripe, with nothing deleted unless the message says
/// otherwise.
pub(crate) async fn delete_own_account(
    State(state): State<AppState>,
    context: OrgContext,
    headers: HeaderMap,
    Json(body): Json<DeleteAccountBody>,
) -> Result<Response, APIError> {
    let subject = SessionRepo::new(state.pool.clone())
        .auth_subject_of(context.user)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(|| {
            refused(
                StatusCode::CONFLICT,
                "This account was not created by signing up, so it can't delete itself. \
                 Reply to any Teachouse email and we'll do it for you.",
                "no_identity",
            )
        })?;
    let standing = ErasureRepo::new(state.pool.clone())
        .inspect(subject)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .map_err(|refusal| self_refusal(&refusal))?;
    check_confirmation(&state, context.org, &body.confirm).await?;
    let reason = reason_of(body.reason.as_deref())?;
    let port = state.config.offboarding.clone().ok_or_else(unavailable)?;

    let cookie = identity_cookie(&headers);
    let proof = Proof {
        password: body
            .password
            .as_deref()
            .filter(|password| !password.is_empty()),
        session_cookie: cookie.as_deref(),
    };
    let departing = match port
        .0
        .reauthenticate(subject, proof)
        .await
        .map_err(|fault| identity_unreachable(&fault))?
    {
        Reauthentication::Confirmed(departing) => departing,
        other @ (Reauthentication::PasswordRequired
        | Reauthentication::PasswordWrong
        | Reauthentication::NotFresh
        | Reauthentication::NoIdentity) => return Err(reauthentication_refused(&other)),
    };

    let cancelled = cancel_subscription(&state, &standing).await?;

    port.0.delete_identity(subject).await.map_err(|fault| {
        eprintln!(
            "tam-api: the identity of subject {} was not deleted: {fault}",
            subject.to_hyphenated()
        );
        refused(
            StatusCode::SERVICE_UNAVAILABLE,
            if cancelled.is_some() {
                "Your plan is cancelled, but we couldn't finish deleting your account. \
                 Try again in a minute."
            } else {
                "We couldn't finish deleting your account. Nothing else was deleted. \
                 Try again in a minute."
            },
            "unavailable",
        )
    })?;

    let record = DeletionRecord {
        email_hash: email_hash(&departing.email),
        requested_at: (state.wall)(),
        stripe_subscription_id: cancelled.clone(),
        reason,
    };
    let erased = erase(&state, subject, Some(&record))
        .await?
        .map_err(|refusal| self_refusal(&refusal))?;
    let goodbye_sent = match port.0.farewell(&departing).await {
        Ok(()) => true,
        Err(fault) => {
            eprintln!(
                "tam-api: subject {} is deleted, but the goodbye mail did not go: {fault}",
                subject.to_hyphenated()
            );
            false
        }
    };
    eprintln!(
        "tam-api: user {} deleted their own account and organisation {} ({} rows)",
        erased.user.0.to_hyphenated(),
        erased.org.0.to_hyphenated(),
        erased.rows,
    );
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, crate::session::cookie_header("", 0))],
        Json(AccountDeletedView {
            subscription_cancelled: cancelled.is_some(),
            goodbye_sent,
        }),
    )
        .into_response())
}

/// The typed word: the organisation's name in the console's address, or
/// [`CONFIRM_WORD`]. Compared case-insensitively against the name, because
/// the name is lower case by construction and a phone capitalises the first
/// letter; the word itself is asked for in capitals and taken that way.
async fn check_confirmation(state: &AppState, org: OrgId, typed: &str) -> Result<(), APIError> {
    let typed = typed.trim();
    if typed == CONFIRM_WORD {
        return Ok(());
    }
    let slug = OrgRepo::new(state.pool.clone())
        .get(org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .and_then(|record| record.slug);
    match slug {
        Some(slug) if !typed.is_empty() && slug.eq_ignore_ascii_case(typed) => Ok(()),
        _ => Err(refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Type your account name, or DELETE, to confirm. Nothing was deleted.",
            "confirmation",
        )),
    }
}

fn reason_of(raw: Option<&str>) -> Result<Option<String>, APIError> {
    let Some(reason) = raw.map(str::trim).filter(|reason| !reason.is_empty()) else {
        return Ok(None);
    };
    if reason.chars().count() > REASON_MAX_CHARS {
        return Err(refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Keep the reason under 1,000 characters. Nothing was deleted.",
            "reason",
        ));
    }
    Ok(Some(reason.to_owned()))
}

/// Cancels a live subscription now, records what Stripe answered, and
/// answers the subscription's id when one was cancelled.
///
/// Read back from Stripe first, as the billing page's own cancel does: a
/// subscription that already ended there (a portal cancellation whose webhook
/// has not landed) is recorded as ended rather than cancelled twice.
async fn cancel_subscription(
    state: &AppState,
    standing: &Standing,
) -> Result<Option<String>, APIError> {
    let Some(held) = standing.subscription.as_ref().filter(|held| held.is_live()) else {
        return Ok(None);
    };
    let client = state.config.stripe.as_ref().ok_or_else(|| {
        refused(
            StatusCode::SERVICE_UNAVAILABLE,
            "We can't reach our payments provider to cancel your plan, so nothing was deleted. \
             Try again later.",
            "unavailable",
        )
    })?;
    let stripe_fault = |error: crate::stripe::StripeError| {
        eprintln!("tam-api: account deletion could not cancel at Stripe: {error}");
        refused(
            StatusCode::SERVICE_UNAVAILABLE,
            "We couldn't cancel your plan, so nothing was deleted. Try again in a minute.",
            "unavailable",
        )
    };
    let id = held.provider_subscription_id.as_str();
    let current = client
        .retrieve_subscription(id)
        .await
        .map_err(stripe_fault)?;
    let current_status = current.status.clone().unwrap_or_default();
    let (status, cancelled) = if ended(&current_status) {
        (current_status, None)
    } else {
        let answer = client.cancel_subscription(id).await.map_err(stripe_fault)?;
        (
            answer.status.unwrap_or_else(|| "canceled".to_owned()),
            Some(id.to_owned()),
        )
    };
    let _matched: bool = BillingRepo::new(state.pool.clone())
        .record_status(standing.org, id, &status, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(cancelled)
}

fn ended(status: &str) -> bool {
    crate::billing::ENDED.contains(&status)
}

/// The identity-service session cookie pairs this request carried, joined as
/// a `Cookie` header value, or `None`.
fn identity_cookie(headers: &HeaderMap) -> Option<String> {
    let pairs: Vec<&str> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|pair| {
            pair.split_once('=').is_some_and(|(name, _value)| {
                name == IDENTITY_SESSION_COOKIE
                    || name.strip_prefix("__Secure-") == Some(IDENTITY_SESSION_COOKIE)
            })
        })
        .collect();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

/// Lower-case hex SHA-256 of the trimmed, lower-cased address.
#[must_use]
pub fn email_hash(email: &str) -> String {
    let digest = sha2::Sha256::digest(email.trim().to_lowercase().as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _infallible = write!(out, "{byte:02x}");
    }
    out
}

fn refused(status: StatusCode, message: &str, refusal: &str) -> APIError {
    let kind = if status == StatusCode::SERVICE_UNAVAILABLE {
        APIErrorKind::Internal
    } else {
        APIErrorKind::Validation
    };
    APIError::new(
        status,
        APIErrorEntry::new(message)
            .kind(kind)
            .detail(serde_json::json!({ "refusal": refusal })),
    )
}

fn unavailable() -> APIError {
    refused(
        StatusCode::SERVICE_UNAVAILABLE,
        "Deleting your account isn't available right now. Nothing was deleted.",
        "unavailable",
    )
}

fn identity_unreachable(fault: &OffboardingFault) -> APIError {
    eprintln!("tam-api: account deletion could not reach the identity service: {fault}");
    unavailable()
}

/// The erasure's refusals, in the seller's words.
fn self_refusal(refusal: &ErasureRefusal) -> APIError {
    match refusal {
        ErasureRefusal::NoPlatformUser => refused(
            StatusCode::NOT_FOUND,
            "There is no account here to delete.",
            "no_platform_user",
        ),
        ErasureRefusal::ActiveOperator => refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "You're a Teachouse operator. Ask another operator to remove your operator \
             marking first, then delete your account. Nothing was deleted.",
            "operator",
        ),
        ErasureRefusal::SharedOrganisation { .. } => refused(
            StatusCode::CONFLICT,
            "Someone else works in this account, and deleting it would delete their work too. \
             Reply to any Teachouse email and we'll sort it out. Nothing was deleted.",
            "shared_organisation",
        ),
        ErasureRefusal::LiveSubscription { .. } => refused(
            StatusCode::CONFLICT,
            "Your plan is still running at Stripe. Try again in a minute. Nothing was deleted.",
            "live_subscription",
        ),
    }
}

fn reauthentication_refused(verdict: &Reauthentication) -> APIError {
    match verdict {
        Reauthentication::PasswordRequired => refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter your password to confirm. Nothing was deleted.",
            "password_required",
        ),
        Reauthentication::PasswordWrong => refused(
            StatusCode::FORBIDDEN,
            "That password isn't right. Nothing was deleted.",
            "password",
        ),
        Reauthentication::NoIdentity => refused(
            StatusCode::CONFLICT,
            "We couldn't find your sign-in. Reply to any Teachouse email and we'll finish \
             deleting your account. Nothing else was deleted.",
            "no_identity",
        ),
        Reauthentication::NotFresh | Reauthentication::Confirmed(_) => refused(
            StatusCode::FORBIDDEN,
            "Sign in again, then come straight back to delete your account. Nothing was deleted.",
            "reauthenticate",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{email_hash, identity_cookie};
    use axum::http::{header, HeaderMap, HeaderValue};

    #[test]
    fn the_address_is_hashed_after_trimming_and_lower_casing() {
        assert_eq!(
            email_hash("  Seller@Example.TEST "),
            email_hash("seller@example.test")
        );
        assert_eq!(
            email_hash("seller@example.test").len(),
            64,
            "lower-case hex SHA-256"
        );
        assert!(email_hash("a@b.test")
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()));
    }

    #[test]
    fn only_the_identity_session_cookie_is_forwarded() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static(
                "tam_session=abc; __Secure-better-auth.session_token=tok.sig; theme=dark",
            ),
        );
        assert_eq!(
            identity_cookie(&headers).as_deref(),
            Some("__Secure-better-auth.session_token=tok.sig")
        );

        let mut development = HeaderMap::new();
        development.insert(
            header::COOKIE,
            HeaderValue::from_static("better-auth.session_token=dev.sig"),
        );
        assert_eq!(
            identity_cookie(&development).as_deref(),
            Some("better-auth.session_token=dev.sig")
        );

        let mut none = HeaderMap::new();
        none.insert(header::COOKIE, HeaderValue::from_static("tam_session=abc"));
        assert_eq!(
            identity_cookie(&none),
            None,
            "the API's own cookie stays here"
        );
    }
}
