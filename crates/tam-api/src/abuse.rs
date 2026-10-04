//! Abuse prevention on the request path and on the operator surface
//! (`docs/notes/design/abuse-prevention.md`; migrations 0105 and 0106).
//!
//! - The sign-up screen, `POST /internal/abuse/screen`: the identity service
//!   asks before it creates an account, and is refused a throwaway email
//!   domain, a sixth account from one address in a day, or an address a ban
//!   still refuses.
//! - The signals: what the request path records into the linkage ledger, each
//!   value digested under the server's pepper (`tam_secrets::abuse_digest`)
//!   before it reaches storage. A deployment without a key-encryption key
//!   records none, as it takes no storefront claims either.
//! - The suspension: the 403 every authenticated route answers a banned
//!   organisation with.
//! - The operator's Abuse page: `GET /{version}/admin/abuse/flags`,
//!   `GET /{version}/admin/abuse/orgs/{org}` and
//!   `POST /{version}/admin/abuse/flags/{id}/{action}`.

use std::collections::BTreeSet;
use std::net::IpAddr;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    AbuseAction, AbuseBackofficeRepo, AbuseDecision, AbuseOrgDetail, AbuseRepo, AbuseSearch,
    BannedKind, DeviceRepo, FlagKind, FlaggedOrg, Signal, SignalKind, SIGNUPS_PER_IP_PER_DAY,
};
use tam_types::{Marketplace, OrgId, Timestamp, Uuid};

use crate::account_consent::INTERNAL_SECRET_HEADER;
use crate::admin::{backoffice, parse_id};
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OperatorContext};

/// The sentence a suspended organisation is answered with, everywhere.
pub const SUSPENDED: &str = "This account is suspended. Email contact@teachouse.io.";
/// The sign-up screen's refusal of a throwaway email domain.
pub const DISPOSABLE_REFUSAL: &str = "Please use a school or personal email address.";
/// The sign-up screen's refusal of a sixth account from one address in a day.
pub const VELOCITY_REFUSAL: &str = "Too many accounts were made from this network today. Please try again tomorrow or email contact@teachouse.io.";

const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1_000;
const REASON_MAX_CHARS: usize = 500;

/// The bundled throwaway-domain list: `disposable-email-domains`
/// (CC0, <https://github.com/disposable-email-domains/disposable-email-domains>),
/// pinned at the commit its first line names. Bundled rather than fetched, so
/// sign-up depends on no third party and sends nobody's address anywhere.
const DISPOSABLE_DOMAINS: &str = include_str!("../data/disposable_email_domains.txt");

/// The list as a set, built once on first use.
static DISPOSABLE: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        DISPOSABLE_DOMAINS
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    });

/// Whether an address (or a bare domain) belongs to a throwaway mail service:
/// its domain, or any parent of it, is on the bundled list.
#[must_use]
pub fn is_disposable(email_or_domain: &str) -> bool {
    let domain = email_domain(email_or_domain);
    let mut candidate = domain.as_str();
    loop {
        if candidate.is_empty() {
            return false;
        }
        if DISPOSABLE.contains(candidate) {
            return true;
        }
        match candidate.split_once('.') {
            Some((_, parent)) if parent.contains('.') => candidate = parent,
            _ => return false,
        }
    }
}

/// The lower-cased domain of an address, or the input itself when it holds
/// no `@`.
#[must_use]
pub fn email_domain(email: &str) -> String {
    let trimmed = email.trim();
    let domain = trimmed
        .rsplit_once('@')
        .map_or(trimmed, |(_, domain)| domain);
    domain.trim_end_matches('.').to_ascii_lowercase()
}

/// The 403 a suspended organisation meets.
#[must_use]
pub fn suspended() -> APIError {
    APIError::new(
        StatusCode::FORBIDDEN,
        APIErrorEntry::new(SUSPENDED)
            .code(APIErrorCode::AccountSuspended)
            .kind(APIErrorKind::Unauthenticated),
    )
}

fn refused(message: &str, refusal: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message)
            .code(APIErrorCode::SignupRefused)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({ "refusal": refusal })),
    )
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn not_found() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("not found").kind(APIErrorKind::NotFound),
    )
}

fn missing(what: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(what)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

/// One value's keyed digest, or none where this deployment holds no key.
fn digest(state: &AppState, kind: &str, value: &str) -> Option<[u8; 32]> {
    state
        .blobs
        .as_ref()
        .map(|blobs| tam_secrets::abuse_digest(&blobs.kek, kind, value))
}

/// An email address's digest, normalised the way `account::email_hash` is.
fn email_digest(state: &AppState, email: &str) -> Option<[u8; 32]> {
    digest(
        state,
        BannedKind::Email.as_str(),
        &email.trim().to_lowercase(),
    )
}

/// A device id's digest, the value both the ledger and a ban hold.
fn device_digest(state: &AppState, device: &str) -> Option<[u8; 32]> {
    digest(state, SignalKind::DeviceFingerprint.as_str(), device)
}

/// The client's address, read the way the identity service reads it
/// (`auth/src/client-ip.ts`): Cloudflare's header first, then the first hop
/// of `x-forwarded-for`.
#[must_use]
pub fn client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    header("cf-connecting-ip")
        .or_else(|| header("x-forwarded-for").and_then(|list| list.split(',').next()))
        .and_then(|raw| raw.trim().parse().ok())
}

async fn record(state: &AppState, org: OrgId, kind: SignalKind, value: &str, at: Timestamp) {
    let Some(value) = digest(state, kind.as_str(), value) else {
        return;
    };
    // Evidence, not a gate: a ledger write that fails must not fail the
    // sign-in or the registration it rides on, so it is logged and dropped.
    if let Err(error) = AbuseRepo::new(state.pool.clone())
        .record_signal(Signal {
            org,
            kind,
            value: &value,
            at,
        })
        .await
    {
        eprintln!("tam-api: an abuse signal was not recorded: {error}");
    }
}

/// A sign-in reached the session exchange: its address and browser.
pub(crate) async fn record_sign_in(
    state: &AppState,
    org: OrgId,
    headers: &HeaderMap,
    at: Timestamp,
) {
    if let Some(ip) = client_ip(headers) {
        record(state, org, SignalKind::Ip, &ip.to_string(), at).await;
    }
    if let Some(agent) = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .filter(|agent| !agent.is_empty())
    {
        record(state, org, SignalKind::UserAgentHash, agent, at).await;
    }
}

/// A tenant was made for a new identity subject: the domain it signed up
/// with, and a flag where that domain is a throwaway one the screen did not
/// see (an account made before the screen, or around it).
pub(crate) async fn record_new_tenant(state: &AppState, org: OrgId, subject: Uuid, at: Timestamp) {
    let repo = AbuseRepo::new(state.pool.clone());
    let email = match repo.signup_email(subject).await {
        Ok(Some(email)) => email,
        Ok(None) => return,
        Err(error) => {
            eprintln!("tam-api: a new tenant's sign-up address could not be read: {error}");
            return;
        }
    };
    let domain = email_domain(&email);
    record(state, org, SignalKind::EmailDomain, &domain, at).await;
    if is_disposable(&domain) {
        if let Err(error) = repo
            .raise(
                org,
                FlagKind::DisposableEmail,
                "Signed up with a throwaway email address.",
                at,
            )
            .await
        {
            eprintln!("tam-api: a throwaway-email flag was not raised: {error}");
        }
    }
}

/// A device registered: refused when a ban still names it, recorded
/// otherwise.
pub(crate) async fn screen_device(
    state: &AppState,
    org: OrgId,
    device: &str,
    at: Timestamp,
) -> Result<(), APIError> {
    let Some(value) = device_digest(state, device) else {
        return Ok(());
    };
    let repo = AbuseRepo::new(state.pool.clone());
    if repo
        .is_banned(BannedKind::DeviceFingerprint, &value, at)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        return Err(suspended());
    }
    if let Err(error) = repo
        .record_signal(Signal {
            org,
            kind: SignalKind::DeviceFingerprint,
            value: &value,
            at,
        })
        .await
    {
        eprintln!("tam-api: a device signal was not recorded: {error}");
    }
    Ok(())
}

/// Whether an organisation is suspended, for the paths that resolve one
/// without a session row (the identity exchange).
pub(crate) async fn refuse_suspended(state: &AppState, org: OrgId) -> Result<(), APIError> {
    let standing = AbuseRepo::new(state.pool.clone())
        .standing(org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if standing.suspends() {
        return Err(suspended());
    }
    Ok(())
}

/// Copies card fingerprints from the payments ledger (migration 0102) into
/// the linkage ledger, digested. `charge` narrows to one charge, which is
/// the webhook's on-event form; `None` is the nightly sweep.
pub async fn harvest_cards(state: &AppState, charge: Option<&str>) -> Result<usize, String> {
    let repo = AbuseRepo::new(state.pool.clone());
    let cards = repo
        .card_fingerprints(charge)
        .await
        .map_err(|error| error.to_string())?;
    let mut recorded = 0;
    for (org, fingerprint, at) in cards {
        let Some(value) = digest(state, SignalKind::PaymentFingerprint.as_str(), &fingerprint)
        else {
            return Ok(0);
        };
        repo.record_signal(Signal {
            org,
            kind: SignalKind::PaymentFingerprint,
            value: &value,
            at,
        })
        .await
        .map_err(|error| error.to_string())?;
        recorded += 1;
    }
    Ok(recorded)
}

/// What the identity service sends before it creates an account.
#[derive(Debug, Clone, Deserialize)]
pub struct ScreenBody {
    pub email: String,
    #[serde(default)]
    pub ip_address: Option<String>,
}

/// `POST /internal/abuse/screen`: 204 lets the sign-up proceed, 422 refuses
/// it with the sentence the identity service shows. Fenced by the same
/// shared secret as `/internal/consent`, with the same indistinguishable 404.
pub(crate) async fn screen(
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
    let body: ScreenBody = serde_json::from_slice(&body)
        .map_err(|error| validation(&format!("the screen body is not readable: {error}")))?;
    if is_disposable(&body.email) {
        return Err(refused(DISPOSABLE_REFUSAL, "disposable_email"));
    }
    let now = (state.wall)();
    let repo = AbuseRepo::new(state.pool.clone());
    if let Some(value) = email_digest(&state, &body.email) {
        if repo
            .is_banned(BannedKind::Email, &value, now)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
        {
            return Err(refused(SUSPENDED, "suspended"));
        }
    }
    if let Some(ip) = body
        .ip_address
        .as_deref()
        .and_then(|raw| raw.trim().parse::<IpAddr>().ok())
    {
        let made = repo
            .signups_from(ip, Timestamp(now.0.saturating_sub(MILLIS_PER_DAY)))
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        if made >= SIGNUPS_PER_IP_PER_DAY {
            return Err(refused(VELOCITY_REFUSAL, "ip_velocity"));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------- the operator

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseCountersView {
    pub open_orgs: i64,
    pub open_flags: i64,
    pub warned: i64,
    pub limited: i64,
    pub banned: i64,
    pub banned_identities: i64,
    pub signals: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseOrgRow {
    pub org: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub score: i64,
    pub kinds: Vec<String>,
    pub signal_kinds: Vec<String>,
    pub linked_orgs: i64,
    pub standing: String,
    pub open_flags: i64,
    pub last_flagged_at: Timestamp,
    pub flag: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseFlagsView {
    pub counters: AbuseCountersView,
    /// How the search was read: `none`, `email`, `ip`, `shop`, `hash` or
    /// `name`.
    pub query: String,
    pub orgs: Vec<AbuseOrgRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseFlagView {
    pub id: Uuid,
    pub kind: String,
    pub score: i32,
    pub reason: String,
    pub created_at: Timestamp,
    pub resolved_at: Option<Timestamp>,
    pub resolved_by: Option<Uuid>,
    pub action: String,
    pub action_reason: Option<String>,
    pub mail_sent_at: Option<Timestamp>,
    pub mail_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseSignalView {
    pub kind: String,
    /// The first twelve hex characters of the keyed digest: enough to search
    /// by and to tell two apart, and nothing anybody could reverse.
    pub value: String,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub shared_with: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedOrgView {
    pub org: Uuid,
    pub name: String,
    pub standing: String,
    pub shared: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbuseOrgView {
    pub org: Uuid,
    pub name: String,
    pub slug: Option<String>,
    pub created_at: Timestamp,
    pub standing: String,
    pub flags: Vec<AbuseFlagView>,
    pub signals: Vec<AbuseSignalView>,
    pub linked: Vec<LinkedOrgView>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FlagsQuery {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ActionBody {
    #[serde(default)]
    pub reason: Option<String>,
}

fn row_view(row: FlaggedOrg) -> AbuseOrgRow {
    AbuseOrgRow {
        org: row.org.0,
        name: row.name,
        slug: row.slug,
        score: row.score,
        kinds: row
            .kinds
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect(),
        signal_kinds: row
            .signal_kinds
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect(),
        linked_orgs: row.linked_orgs,
        standing: row.standing.as_str().to_owned(),
        open_flags: row.open_flags,
        last_flagged_at: row.last_flagged_at,
        flag: row.flag,
    }
}

fn org_view(detail: AbuseOrgDetail) -> AbuseOrgView {
    AbuseOrgView {
        org: detail.org.0,
        name: detail.name,
        slug: detail.slug,
        created_at: detail.created_at,
        standing: detail.standing.as_str().to_owned(),
        flags: detail
            .flags
            .into_iter()
            .map(|flag| AbuseFlagView {
                id: flag.id,
                kind: flag.kind.as_str().to_owned(),
                score: flag.score,
                reason: flag.reason,
                created_at: flag.created_at,
                resolved_at: flag.resolved_at,
                resolved_by: flag.resolved_by,
                action: flag.action.as_str().to_owned(),
                action_reason: flag.action_reason,
                mail_sent_at: flag.mail_sent_at,
                mail_error: flag.mail_error,
            })
            .collect(),
        signals: detail
            .signals
            .into_iter()
            .map(|signal| {
                let mut value = tam_secrets::hex_encode(&signal.value);
                value.truncate(12);
                AbuseSignalView {
                    kind: signal.kind.as_str().to_owned(),
                    value,
                    first_seen: signal.first_seen,
                    last_seen: signal.last_seen,
                    shared_with: signal.shared_with,
                }
            })
            .collect(),
        linked: detail
            .linked
            .into_iter()
            .map(|linked| LinkedOrgView {
                org: linked.org.0,
                name: linked.name,
                standing: linked.standing.as_str().to_owned(),
                shared: linked
                    .shared
                    .iter()
                    .map(|kind| kind.as_str().to_owned())
                    .collect(),
            })
            .collect(),
    }
}

/// How the operator's search is read, and the organisations it names.
async fn search(
    state: &AppState,
    repo: &AbuseBackofficeRepo,
    raw: &str,
) -> Result<(&'static str, Vec<OrgId>), APIError> {
    let fault = |error: tam_storage::StorageError| state.internal(&error.to_string());
    let query = raw.trim();
    if query.contains('@') {
        return Ok((
            "email",
            repo.search(AbuseSearch::Email(query))
                .await
                .map_err(fault)?,
        ));
    }
    if let Ok(ip) = query.parse::<IpAddr>() {
        let Some(value) = digest(state, SignalKind::Ip.as_str(), &ip.to_string()) else {
            return Ok(("ip", Vec::new()));
        };
        return Ok((
            "ip",
            repo.search(AbuseSearch::Digest(&value, &[SignalKind::Ip]))
                .await
                .map_err(fault)?,
        ));
    }
    if let Some((marketplace, shop)) = query.split_once(':') {
        let marketplace = match marketplace.trim().to_ascii_lowercase().as_str() {
            "tpt" => Some(Marketplace::Tpt),
            "tes" => Some(Marketplace::Tes),
            _ => None,
        };
        if let Some(marketplace) = marketplace {
            let Some(blobs) = state.blobs.as_ref() else {
                return Ok(("shop", Vec::new()));
            };
            let value = tam_secrets::account_digest(
                &blobs.kek,
                marketplace,
                tam_storage::ACCOUNT_KEY_VERSION,
                shop.trim(),
            );
            return Ok((
                "shop",
                repo.search(AbuseSearch::Digest(&value, &[SignalKind::ShopDigest]))
                    .await
                    .map_err(fault)?,
            ));
        }
    }
    if (12..=64).contains(&query.len()) && query.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok((
            "hash",
            repo.search(AbuseSearch::HexPrefix(query))
                .await
                .map_err(fault)?,
        ));
    }
    Ok((
        "name",
        repo.search(AbuseSearch::Name(query)).await.map_err(fault)?,
    ))
}

/// `GET /{version}/admin/abuse/flags`.
pub(crate) async fn list_flags(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Query(query): Query<FlagsQuery>,
) -> Result<Json<AbuseFlagsView>, APIError> {
    let pool = backoffice(&state)?;
    let repo = AbuseBackofficeRepo::new(pool);
    let fault = |error: tam_storage::StorageError| state.internal(&error.to_string());
    let all = match query.state.as_deref() {
        None | Some("open") => false,
        Some("all") => true,
        Some(_) => return Err(validation("state is open or all")),
    };
    let wanted = query.q.as_deref().filter(|raw| !raw.trim().is_empty());
    let (read, orgs) = match wanted {
        Some(raw) => {
            let (read, orgs) = search(&state, &repo, raw).await?;
            // A search looks through every decision, not only the open ones:
            // the operator is looking somebody up.
            let found = repo.flagged(true, Some(&orgs)).await.map_err(fault)?;
            (read, found)
        }
        None => ("none", repo.flagged(all, None).await.map_err(fault)?),
    };
    let counters = repo.counters().await.map_err(fault)?;
    Ok(Json(AbuseFlagsView {
        counters: AbuseCountersView {
            open_orgs: counters.open_orgs,
            open_flags: counters.open_flags,
            warned: counters.warned,
            limited: counters.limited,
            banned: counters.banned,
            banned_identities: counters.banned_identities,
            signals: counters.signals,
        },
        query: read.to_owned(),
        orgs: orgs.into_iter().map(row_view).collect(),
    }))
}

async fn detail(state: &AppState, org: OrgId) -> Result<AbuseOrgView, APIError> {
    let pool = backoffice(state)?;
    AbuseBackofficeRepo::new(pool)
        .org(org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .map(org_view)
        .ok_or_else(|| missing("no organisation with that id"))
}

/// `GET /{version}/admin/abuse/orgs/{org}`.
pub(crate) async fn org_detail(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, org)): Path<(String, String)>,
) -> Result<Json<AbuseOrgView>, APIError> {
    let org = OrgId(parse_id(&org)?);
    Ok(Json(detail(&state, org).await?))
}

/// What a ban refuses that only this process can digest: the organisation's
/// email addresses and the device ids it registered.
async fn ban_identities(
    state: &AppState,
    org: OrgId,
) -> Result<Vec<(BannedKind, [u8; 32])>, APIError> {
    let fault = |error: tam_storage::StorageError| state.internal(&error.to_string());
    let mut identities = BTreeSet::new();
    for email in AbuseRepo::new(state.pool.clone())
        .org_emails(org)
        .await
        .map_err(fault)?
    {
        if let Some(value) = email_digest(state, &email) {
            identities.insert((BannedKind::Email.as_str(), value));
        }
    }
    for device in DeviceRepo::new(state.pool.clone())
        .list(org)
        .await
        .map_err(fault)?
    {
        if let Some(value) = device_digest(state, &device.id) {
            identities.insert((BannedKind::DeviceFingerprint.as_str(), value));
        }
    }
    Ok(identities
        .into_iter()
        .map(|(kind, value)| {
            let kind = if kind == BannedKind::Email.as_str() {
                BannedKind::Email
            } else {
                BannedKind::DeviceFingerprint
            };
            (kind, value)
        })
        .collect())
}

/// `POST /{version}/admin/abuse/flags/{id}/{action}`.
pub(crate) async fn act(
    State(state): State<AppState>,
    operator: OperatorContext,
    Path((_version, flag, action)): Path<(String, String, String)>,
    body: Option<Json<ActionBody>>,
) -> Result<Json<AbuseOrgView>, APIError> {
    let flag = parse_id(&flag)?;
    let action = match action.as_str() {
        "dismiss" => AbuseAction::None,
        "warn" => AbuseAction::Warn,
        "limit" => AbuseAction::Limit,
        "ban" => AbuseAction::Ban,
        _ => return Err(validation("the action is dismiss, warn, limit or ban")),
    };
    let reason = body
        .and_then(|Json(body)| body.reason)
        .map(|reason| reason.trim().to_owned())
        .filter(|reason| !reason.is_empty());
    if reason
        .as_deref()
        .is_some_and(|reason| reason.chars().count() > REASON_MAX_CHARS)
    {
        return Err(validation("Keep the reason to 500 characters."));
    }
    if action == AbuseAction::Ban && reason.is_none() {
        return Err(validation("Say why this account is being suspended."));
    }
    let repo = AbuseRepo::new(state.pool.clone());
    let fault = |error: tam_storage::StorageError| state.internal(&error.to_string());
    let Some(org) = repo.flag_org(flag).await.map_err(fault)? else {
        return Err(missing("no flag with that id"));
    };
    let identities = if action == AbuseAction::Ban {
        ban_identities(&state, org).await?
    } else {
        Vec::new()
    };
    repo.decide(AbuseDecision {
        flag,
        action,
        reason: reason.as_deref(),
        by: operator.user.0,
        at: (state.wall)(),
        identities: &identities,
    })
    .await
    .map_err(fault)?
    .ok_or_else(|| missing("no flag with that id"))?;
    Ok(Json(detail(&state, org).await?))
}

#[cfg(test)]
mod tests {
    use super::{email_domain, is_disposable};

    #[test]
    fn a_listed_domain_and_its_subdomains_are_throwaway() {
        assert!(is_disposable("someone@mailinator.com"));
        assert!(is_disposable("Someone@MAILINATOR.com "));
        assert!(is_disposable("x@inbox.mailinator.com"));
        assert!(is_disposable("yopmail.com"));
    }

    #[test]
    fn schools_and_mailbox_providers_are_not() {
        for address in [
            "teacher@gmail.com",
            "teacher@outlook.com",
            "teacher@school.nz",
            "head@education.govt.nz",
            "",
            "@",
        ] {
            assert!(!is_disposable(address), "{address} is not throwaway");
        }
    }

    #[test]
    fn the_domain_is_lower_cased_and_unrooted() {
        assert_eq!(email_domain(" A@B.Example. "), "b.example");
        assert_eq!(email_domain("b.example"), "b.example");
    }
}
