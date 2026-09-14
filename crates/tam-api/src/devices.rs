//! The device registry over the wire: register, heartbeat, list, sign out and
//! sign back in.
//!
//! Decision D14 in `docs/notes/design/vendoo-for-teachers-rethink.md` asks for
//! a "Your devices" page with per-device sign-out. Half of what such a page
//! shows is ours and half is better-auth's, and the two halves reach the
//! browser separately: these routes serve the seller's own machines and the
//! marketplaces each holds, and the identity service's client SDK serves the
//! browser login sessions. Nothing here reads the `auth` schema, which is the
//! charter's boundary rather than a convention — `tam_auth` is confined to that
//! schema and `tam_app` holds no privilege in it.
//!
//! Every route is org-scoped through [`OrgContext`], so the request carries no
//! organisation identifier a caller could substitute, and a device id names a
//! row only within the tenant the session speaks for.
//!
//! What revocation actually does, stated rather than implied: it marks the
//! device revoked here, and the device wipes its own marketplace sessions when
//! it next reaches [`heartbeat`]. A device that never reconnects keeps its
//! cookies until the marketplace itself expires them. There is no mechanism
//! that could do better — the sessions are on the seller's machine by design,
//! and D1 forbids the server from holding or reaching them — so the page says
//! so rather than implying an immediate wipe.
//!
//! And what reverses it: [`restore_device`], reached from a session on the
//! machine itself. Nothing the device does alone lifts a sign-out, so without
//! this route a machine the seller signed out could never work again -- it
//! would keep its identity, keep being answered `revoked`, and keep wiping the
//! marketplace logins the seller had just made on it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tam_domain::entitlement::Claims;
use tam_limits::Plan;
use tam_storage::{
    ConnectionFactsRepo, ConnectionRepo, DeviceRecord, DeviceRegistration, DeviceRepo,
    DeviceSessionRecord, DeviceSessionReport, DeviceSessionStatus,
};
use tam_types::{Marketplace, OrgId, Timestamp, TransportClass};

use crate::entitlement::{quota_refusal, QuotaKind};
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// The bounds migration 0042 puts on the columns, restated here so a device
/// that overruns one is refused as a validation error rather than surfacing a
/// constraint violation as a fault.
pub const ID_MAX_CHARS: usize = 64;
pub const NAME_MAX_CHARS: usize = 200;
pub const FACET_MAX_CHARS: usize = 64;
pub const LABEL_MAX_CHARS: usize = 200;
/// The name a seller declares authorship under. Bounded like the rest, and at
/// the same width as an account label because it is the same kind of thing: a
/// human name typed into a form.
pub const AUTHORSHIP_MAX_CHARS: usize = 200;

/// Milliseconds to the seconds RFC 7519 fixes `exp` to be. `div_euclid` rather
/// than `/`, which the workspace lint table denies.
const MILLIS_PER_SEC: i64 = 1_000;

/// An Ed25519 public key is thirty-two bytes, and the desktop build script that
/// embeds it wants exactly this many lowercase hex characters.
const PUBLIC_KEY_HEX_CHARS: usize = 64;

/// The Ed25519 key entitlement tokens are signed under, held so it cannot
/// reach a log line or a `Debug` render of the configuration that carries it.
///
/// PKCS#8 DER, and not by preference: `jsonwebtoken` is built here with
/// `default-features = false`, which drops its `use_pem` feature, so
/// `from_ed_pem` does not exist in this workspace and DER is the only form
/// there is.
///
/// The equality this derives is a byte comparison and is not constant-time. It
/// exists because [`crate::Config`] is compared in tests and nowhere else; no
/// request path compares a key.
///
/// Not zeroized on drop, and recorded as a follow-up rather than done here.
/// `tam-secrets` already zeroizes its own key material and would be the right
/// home for a wrapper, but neither type it exposes fits: `Secret` holds a
/// `String` and this is DER, and `Kek` is fixed at thirty-two bytes while a
/// PKCS#8 Ed25519 pair is eighty-three. Adding `zeroize` to this crate's own
/// manifest is a dependency decision rather than a cleanup. The exposure is
/// bounded and worth stating: the key lives for the whole process either way,
/// so what zeroizing would buy is only that a core dump or a swapped page taken
/// after a shutdown no longer carries it.
#[derive(Clone, PartialEq, Eq)]
pub struct EntitlementKey(Vec<u8>);

impl EntitlementKey {
    #[must_use]
    pub const fn new(der: Vec<u8>) -> Self {
        Self(der)
    }

    /// The one legitimate use: signing a token. Never logged, never echoed.
    fn signing_key(&self) -> EncodingKey {
        EncodingKey::from_ed_der(&self.0)
    }

    /// The public half this key signs under, as the sixty-four lowercase hex
    /// characters `TAM_ENTITLEMENT_PUBLIC_KEY` carries.
    ///
    /// Asked once at start-up, and it answers a different question from "can
    /// these bytes sign". `jsonwebtoken` signs through
    /// `Ed25519KeyPair::from_pkcs8_maybe_unchecked`, the constructor that
    /// deliberately skips the consistency check between the private seed and the
    /// public key embedded in the DER, so a successful signature proves only that
    /// *a* signature came out — never that it is the one the installed base
    /// verifies. `from_pkcs8` below is the checked constructor, and the hex it
    /// yields is what an operator compares against the repository variable.
    ///
    /// The failure this closes is silent otherwise: a restored backup or a
    /// half-finished rotation puts the wrong pair at the key path, every client
    /// fails verification, and every gate closes on the next check-in with
    /// nothing anywhere distinguishing it from a healthy deployment.
    pub fn public_key_hex(&self) -> Result<String, String> {
        use core::fmt::Write as _;
        let pair =
            ring::signature::Ed25519KeyPair::from_pkcs8(&self.0).map_err(|why| why.to_string())?;
        let mut hex = String::with_capacity(PUBLIC_KEY_HEX_CHARS);
        for byte in ring::signature::KeyPair::public_key(&pair).as_ref() {
            // infallible on String; the Result is the trait's, not the writer's
            let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
        }
        Ok(hex)
    }
}

impl core::fmt::Debug for EntitlementKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("EntitlementKey(redacted)")
    }
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn missing() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("no such device")
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

/// A trimmed, bounded, non-empty field, or the refusal naming the bound it
/// applied. Characters rather than bytes, matching `char_length` in the
/// migration and the way the human who typed a machine name sees it.
fn bounded<'a>(field: &str, raw: &'a str, max: usize) -> Result<&'a str, APIError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation(&format!("{field} cannot be empty")));
    }
    if trimmed.chars().count() > max {
        return Err(validation(&format!("{field} is at most {max} characters")));
    }
    Ok(trimmed)
}

#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
}

/// One marketplace session as a heartbeat reports it. The status is the closed
/// vocabulary `tam-storage` stores and the generator emits; an unknown word is
/// a validation refusal rather than a row nothing can render.
#[derive(Debug, Deserialize)]
pub struct HeartbeatSession {
    pub marketplace: Marketplace,
    pub account_label: Option<String>,
    pub status: String,
}

/// One file a device holds in its library, by digest hex.
#[derive(Debug, Deserialize)]
pub struct HeartbeatHolding {
    pub hash: String,
    pub byte_len: u64,
}

/// Where a device's library can be reached and what it holds. Optional on
/// the body so an application from before the library existed still checks
/// in; absent, nothing about the library changes.
#[derive(Debug, Deserialize)]
pub struct HeartbeatLibrary {
    pub node_id: String,
    pub direct_addrs: Vec<String>,
    pub holdings: Vec<HeartbeatHolding>,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatBody {
    /// Everything this device holds, not a delta. A marketplace absent here is
    /// taken as no longer held, because the page's job is saying what a device
    /// holds now and a delta would leave a stale row standing after a
    /// disconnect performed offline.
    pub sessions: Vec<HeartbeatSession>,
    #[serde(default)]
    pub library: Option<HeartbeatLibrary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceSessionView {
    pub marketplace: Marketplace,
    pub account_label: Option<String>,
    pub linked_at: Timestamp,
    pub last_used_at: Timestamp,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    pub first_seen_at: Timestamp,
    pub last_seen_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
    /// The device was signed out and has not been heard from since, so it may
    /// still hold the marketplace cookies listed below. Derived from the two
    /// instants rather than stored, because it is a statement about what we
    /// know rather than a fact anybody wrote down.
    pub wipe_outstanding: bool,
    /// Whether this installation is new enough to run a sourced-payload item.
    ///
    /// Derived here rather than in the console, because the floor
    /// (`tam_domain::SOURCED_PAYLOAD_MIN_VERSION`) is the same figure the
    /// claim gates on: a browser comparing version strings for itself would be
    /// a second implementation of the gate, and the two would disagree the
    /// first time the floor moved.
    ///
    /// It is eligibility and nothing else. It says nothing about whether this
    /// device has been heard from, holds a marketplace login, or is working
    /// anything now; those are `last_seen_at`, `sessions` and the run's own
    /// owner. An older installation that is eligible stays eligible — a
    /// version difference on its own is not a reason to hide a machine.
    pub runs_sourced_payloads: bool,
    pub sessions: Vec<DeviceSessionView>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DevicesView {
    pub devices: Vec<DeviceView>,
}

/// What a heartbeat answers. `revoked` is the instruction the device acts on:
/// forget every marketplace session and stop working.
#[derive(Debug, Serialize, Deserialize)]
pub struct HeartbeatView {
    pub revoked: bool,
    pub revoked_at: Option<Timestamp>,
    /// The entitlement token this check-in grants, under decision D10.
    ///
    /// Skipped rather than serialised as null, so a reply that grants nothing
    /// is byte-identical to what this route answered before the mint existed,
    /// and `default` so a reply without it still deserialises. Neither this
    /// type nor the desktop client's own reply type denies unknown fields,
    /// which is what let the field be added at all without stranding the
    /// published 0.1.3 client: each end ignores what the other added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entitlement: Option<String>,
}

/// Whether we have asked a device to wipe and have not heard from it since.
///
/// Free rather than a method so the derivation is unit-testable without a
/// database row, which is the whole reason it is not computed in SQL.
#[must_use]
pub fn wipe_outstanding(revoked_at: Option<Timestamp>, last_seen_at: Timestamp) -> bool {
    revoked_at.is_some_and(|revoked| last_seen_at < revoked)
}

fn session_view(record: DeviceSessionRecord) -> DeviceSessionView {
    DeviceSessionView {
        marketplace: record.marketplace,
        account_label: record.account_label,
        linked_at: record.linked_at,
        last_used_at: record.last_used_at,
        status: record.status.as_str().to_owned(),
    }
}

fn device_view(record: DeviceRecord) -> DeviceView {
    DeviceView {
        wipe_outstanding: wipe_outstanding(record.revoked_at, record.last_seen_at),
        runs_sourced_payloads: tam_domain::runs_sourced_payloads(&record.app_version),
        id: record.id,
        name: record.name,
        os: record.os,
        arch: record.arch,
        app_version: record.app_version,
        first_seen_at: record.first_seen_at,
        last_seen_at: record.last_seen_at,
        revoked_at: record.revoked_at,
        sessions: record.sessions.into_iter().map(session_view).collect(),
    }
}

/// The status word as the storage vocabulary spells it, or a refusal.
fn status_of(raw: &str) -> Result<DeviceSessionStatus, APIError> {
    DeviceSessionStatus::ALL
        .into_iter()
        .find(|status| status.as_str() == raw)
        .ok_or_else(|| {
            let known: Vec<&str> = DeviceSessionStatus::ALL
                .iter()
                .map(|status| status.as_str())
                .collect();
            validation(&format!(
                "unknown session status {raw:?}; one of {}",
                known.join(", ")
            ))
        })
}

/// One reported session, validated into the shape the repository takes.
///
/// A marketplace with an official API is refused outright. Its automation runs
/// server-side under a sanctioned token and no device ever holds a session for
/// it, so a heartbeat naming one is either a client defect or a claim the
/// architecture forbids; recording it would put a row in the registry that the
/// two-branch rule says cannot exist.
fn reported(session: &HeartbeatSession) -> Result<DeviceSessionReport<'_>, APIError> {
    if session.marketplace.transport_class() == TransportClass::OfficialApi {
        return Err(validation(&format!(
            "{:?} publishes an official API, so its automation runs server-side \
             and no device holds a session for it",
            session.marketplace
        )));
    }
    let label = match session.account_label.as_deref() {
        Some(raw) => Some(bounded("an account label", raw, LABEL_MAX_CHARS)?),
        None => None,
    };
    Ok(DeviceSessionReport {
        marketplace: session.marketplace,
        account_label: label,
        status: status_of(&session.status)?,
    })
}

#[derive(Debug, Deserialize)]
pub struct DeclareAuthorshipBody {
    pub name: String,
}

/// The declaration as it stands after the request that made it.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthorshipView {
    pub marketplace: Marketplace,
    pub name: String,
    pub attested_at: Timestamp,
}

/// The marketplace as the path spells it, which is its serde name -- the same
/// derivation `/{version}/vocabulary/{inventory}` uses, so one spelling serves
/// the body, the path and the generated client vocabulary.
fn marketplace_of(raw: &str) -> Option<Marketplace> {
    Marketplace::ALL
        .into_iter()
        .find(|marketplace| serde_json::to_value(marketplace).ok() == Some(raw.into()))
}

/// The seller declares who authored what one marketplace connection
/// publishes.
///
/// Once per connection rather than once per device, which is what makes it
/// survive a machine being replaced: the seller declares here and every device
/// they ever register writes under it, reading it off the work order. TPT's
/// product form makes this declaration a required field, so without one the
/// adapter refuses every write -- correctly, because the copyright declaration
/// is the seller's statement and not a constant a connector may make for them.
///
/// The organisation comes from [`OrgContext`] and is not representable in the
/// body. The instant is stamped here rather than taken from the body, because
/// this request is the declaration: our receipt of it is when the seller made
/// it, and an instant a caller supplied would let them date their own
/// statement.
///
/// A marketplace with an official API is refused, for the reason [`reported`]
/// refuses one: its automation runs server-side under a sanctioned token, no
/// device ever composes a write for it, and nothing would ever read the
/// declaration back.
pub(crate) async fn declare_authorship(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, marketplace)): Path<(String, String)>,
    Json(body): Json<DeclareAuthorshipBody>,
) -> Result<Json<AuthorshipView>, APIError> {
    let marketplace = marketplace_of(&marketplace).ok_or_else(|| {
        APIError::new(
            StatusCode::NOT_FOUND,
            APIErrorEntry::new("no such marketplace")
                .code(APIErrorCode::ResourceMissing)
                .kind(APIErrorKind::NotFound),
        )
    })?;
    if marketplace.transport_class() == TransportClass::OfficialApi {
        return Err(validation(&format!(
            "{marketplace:?} publishes an official API, so its automation runs \
             server-side and no device composes a write to declare authorship on"
        )));
    }
    // The plan's marketplace allowance binds here, because this is the act
    // that adopts a marketplace: a seller declares who authored what they
    // publish, once per connection, and nothing else on this surface is a
    // seller saying "I sell here". Marketplaces this organisation already
    // has a live connection for never count against it, so a re-declaration
    // is never refused.
    let connected: Vec<Marketplace> = ConnectionRepo::new(state.pool.clone())
        .list(context.org, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .into_iter()
        .filter(|row| row.state != "revoked")
        .map(|row| row.marketplace)
        .collect();
    let mut distinct: Vec<Marketplace> = Vec::new();
    for held in connected {
        if !distinct.contains(&held) {
            distinct.push(held);
        }
    }
    let caps = context.entitlement.caps;
    if !distinct.contains(&marketplace)
        && distinct.len() >= usize::try_from(caps.marketplaces_max).unwrap_or(usize::MAX)
    {
        return Err(quota_refusal(
            QuotaKind::Marketplaces,
            i64::try_from(distinct.len()).unwrap_or(i64::MAX),
            u64::from(caps.marketplaces_max),
        ));
    }
    let name = bounded("an authorship name", &body.name, AUTHORSHIP_MAX_CHARS)?;
    let record = ConnectionFactsRepo::new(state.pool.clone())
        .declare_authorship(context.org, marketplace, name, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(AuthorshipView {
        marketplace,
        name: record.name,
        attested_at: record.attested_at,
    }))
}

pub(crate) async fn register(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<RegisterBody>,
) -> Result<Json<DeviceView>, APIError> {
    let registration = DeviceRegistration {
        id: bounded("a device id", &body.id, ID_MAX_CHARS)?,
        name: bounded("a device name", &body.name, NAME_MAX_CHARS)?,
        os: bounded("an operating system", &body.os, FACET_MAX_CHARS)?,
        arch: bounded("an architecture", &body.arch, FACET_MAX_CHARS)?,
        app_version: bounded("an application version", &body.app_version, FACET_MAX_CHARS)?,
    };
    // One computer per device row, and the plan says how many. Revoked
    // devices are excluded: a machine the seller signed out is in the record
    // rather than in the allowance.
    let devices = DeviceRepo::new(state.pool.clone());
    let held = devices
        .list(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let live = held
        .iter()
        .filter(|record| record.revoked_at.is_none() && record.id != registration.id)
        .count();
    let caps = context.entitlement.caps;
    if live >= usize::try_from(caps.devices_max).unwrap_or(usize::MAX) {
        return Err(quota_refusal(
            QuotaKind::Devices,
            i64::try_from(live).unwrap_or(i64::MAX),
            u64::from(caps.devices_max),
        ));
    }
    let record = devices
        .register(context.org, &registration, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(device_view(record)))
}

/// The device's check-in. It stamps the device seen, replaces what the device
/// holds, and tells the device whether it has been signed out.
///
/// A heartbeat never creates a device: an unregistered id is not-found, so a
/// client that lost its registration re-registers rather than silently
/// appearing in the seller's list under whatever id it now holds.
pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(body): Json<HeartbeatBody>,
) -> Result<Json<HeartbeatView>, APIError> {
    let device = bounded("a device id", &device, ID_MAX_CHARS)?;
    let sessions = body
        .sessions
        .iter()
        .map(reported)
        .collect::<Result<Vec<_>, APIError>>()?;
    let now = (state.wall)();
    let beat = DeviceRepo::new(state.pool.clone())
        .heartbeat(context.org, device, &sessions, now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(missing)?;
    if let Some(library) = &body.library {
        crate::library::record_report(&state, context.org, device, library, now).await?;
    }
    // A signed-out device is granted nothing, so a client that ignored
    // `revoked` still gets no entitlement out of the same answer.
    let entitlement = if beat.revoked() {
        None
    } else {
        mint_entitlement(
            &state,
            context.org,
            context.entitlement.grant.plan,
            device,
            now,
        )
        .await?
    };
    Ok(Json(HeartbeatView {
        revoked: beat.revoked(),
        revoked_at: beat.revoked_at,
        entitlement,
    }))
}

/// The entitlement this check-in grants, or none.
///
/// Decision D10 in full: Postgres decides and this only transports the
/// decision. The grant set is most of the work claim's own predicate asked as a
/// question rather than embedded in a claim — a registered unrevoked device, a
/// plan that has not lapsed past the grace, a linked connection, and no halt
/// over the marketplace — and it is a superset of what the claim will serve
/// rather than an equal set, because the connected-session and per-item
/// conditions are deliberately left out. Over-approximating is the safe
/// direction for a gate that can only refuse: too wide costs one refused claim,
/// too narrow stops an entitled seller working. `DeviceRepo::entitled_marketplaces`
/// carries the full reasoning.
///
/// Every reason to grant nothing answers `None` rather than an error, and the
/// device reads that as a closed gate. A heartbeat's first job is delivering a
/// revocation, so a deployment with no signing key, a lapsed plan and a halted
/// fleet all still get their answer rather than a fault.
async fn mint_entitlement(
    state: &AppState,
    org: OrgId,
    plan: Plan,
    device: &str,
    now: Timestamp,
) -> Result<Option<String>, APIError> {
    let Some(key) = state.config.entitlement_key.as_ref() else {
        return Ok(None);
    };
    let marketplaces = DeviceRepo::new(state.pool.clone())
        .entitled_marketplaces(org, device)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if marketplaces.is_empty() {
        return Ok(None);
    }
    let claims = Claims::mint(
        org.0.to_hyphenated(),
        device.to_owned(),
        marketplaces,
        plan,
        now.0.div_euclid(MILLIS_PER_SEC),
    );
    encode(&Header::new(Algorithm::EdDSA), &claims, &key.signing_key())
        .map(Some)
        .map_err(|error| state.internal(&format!("the entitlement token did not sign: {error}")))
}

pub(crate) async fn list_devices(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<DevicesView>, APIError> {
    let records = DeviceRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(DevicesView {
        devices: records.into_iter().map(device_view).collect(),
    }))
}

/// Signs one device out. Idempotent: a second call answers the first
/// decision's instant rather than restamping it, because the wipe is measured
/// against when the seller decided.
pub(crate) async fn revoke_device(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
) -> Result<Json<DeviceView>, APIError> {
    let device = bounded("a device id", &device, ID_MAX_CHARS)?;
    let repo = DeviceRepo::new(state.pool.clone());
    repo.revoke(context.org, device, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(missing)?;
    let record = repo
        .list(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .into_iter()
        .find(|record| record.id == device)
        .ok_or_else(missing)?;
    Ok(Json(device_view(record)))
}

/// Signs one device back in, undoing a sign-out the seller made from the
/// console.
///
/// The route exists because without it a signed-out machine was finished:
/// registration deliberately never clears the mark and the client only
/// re-registers when the server has forgotten it, so the app kept its
/// identity, kept being told `revoked`, and kept wiping its marketplace
/// sessions every cycle. Every connect attempt on that machine then failed
/// after the seller had typed a password.
///
/// Reversing it is the seller's act rather than the machine's, which is why
/// this is a session-authed route and not something the heartbeat does: a
/// revocation a machine lifted by restarting would not be the seller's
/// decision any more.
///
/// The device allowance is re-applied here, for the reason [`register`]
/// applies it: a restore adds a machine to the live fleet, so a seller on a
/// one-computer plan who signed out an old laptop and registered a new one
/// cannot have both back by restoring. The device being restored is excluded
/// from the count it is measured against, so restoring one that is not
/// signed out at all is never refused.
///
/// A restore is authorisation and not contact, so the record it answers with
/// carries the `last_seen_at` the device last actually spoke at: the console
/// says "last seen" against an instant before the sign-out until this machine
/// checks in, which it does on its next cadence or the moment the seller
/// presses Check in now. Stamping it here read as "checked in just now" for a
/// machine that had said nothing.
///
/// Not-found comes first deliberately: an unknown id is a different fact from
/// a full fleet, and answering the quota for it would tell the caller a
/// machine exists.
pub(crate) async fn restore_device(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
) -> Result<Json<DeviceView>, APIError> {
    let device = bounded("a device id", &device, ID_MAX_CHARS)?;
    let devices = DeviceRepo::new(state.pool.clone());
    let held = devices
        .list(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if !held.iter().any(|record| record.id == device) {
        return Err(missing());
    }
    let live = held
        .iter()
        .filter(|record| record.revoked_at.is_none() && record.id != device)
        .count();
    let caps = context.entitlement.caps;
    if live >= usize::try_from(caps.devices_max).unwrap_or(usize::MAX) {
        return Err(quota_refusal(
            QuotaKind::Devices,
            i64::try_from(live).unwrap_or(i64::MAX),
            u64::from(caps.devices_max),
        ));
    }
    let record = devices
        .restore(context.org, device)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(missing)?;
    Ok(Json(device_view(record)))
}

#[cfg(test)]
mod tests {
    use super::{
        bounded, marketplace_of, reported, status_of, wipe_outstanding, HeartbeatSession,
        FACET_MAX_CHARS,
    };
    use crate::error::APIErrorKind;
    use axum::http::StatusCode;
    use tam_storage::DeviceSessionStatus;
    use tam_types::{Marketplace, Timestamp};

    fn session(marketplace: Marketplace, status: &str) -> HeartbeatSession {
        HeartbeatSession {
            marketplace,
            account_label: None,
            status: status.to_owned(),
        }
    }

    #[test]
    fn the_public_half_is_derived_from_the_pair_rather_than_from_a_signature() {
        use core::fmt::Write as _;
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random)
            .expect("the test key generates");
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
            .expect("the generated key parses");
        let mut expected = String::new();
        for byte in ring::signature::KeyPair::public_key(&pair).as_ref() {
            let _unused: core::fmt::Result = write!(expected, "{byte:02x}");
        }

        let key = super::EntitlementKey::new(pkcs8.as_ref().to_vec());
        assert_eq!(
            key.public_key_hex().as_deref(),
            Ok(expected.as_str()),
            "the operator compares this against the fleet's repository variable, so it must be \
             the public half of this very pair"
        );
        assert_eq!(expected.len(), 64, "sixty-four lowercase hex characters");

        let nonsense = super::EntitlementKey::new(vec![0x30, 0x00, 0x01]);
        assert!(
            nonsense.public_key_hex().is_err(),
            "and a file that is not a key pair is a start-up fault, not a running server that \
             mints tokens nobody can verify"
        );
    }

    #[test]
    fn a_signing_key_cannot_reach_a_log_line() {
        let key = super::EntitlementKey::new(vec![0x11, 0x22, 0x33, 0x44]);
        let printed = format!("{key:?}");
        assert_eq!(printed, "EntitlementKey(redacted)");
        assert!(
            !printed.contains("11") && !printed.contains("22"),
            "the key's own bytes must not survive a Debug render, which is what a \
             configuration dump would print: {printed}"
        );
    }

    #[test]
    fn a_field_is_trimmed_and_measured_in_characters() {
        assert_eq!(
            bounded("a name", "  founder-pc \n", 200).ok(),
            Some("founder-pc")
        );
        // Two bytes per character, so a byte-counting bound would refuse what
        // a character-counting one accepts.
        let longest = "\u{e9}".repeat(FACET_MAX_CHARS);
        assert!(bounded("an architecture", &longest, FACET_MAX_CHARS).is_ok());
        let refused = bounded(
            "an architecture",
            &"\u{e9}".repeat(FACET_MAX_CHARS + 1),
            FACET_MAX_CHARS,
        )
        .expect_err("one character too many is refused");
        assert_eq!(refused.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            refused.errors[0]
                .message
                .contains(&FACET_MAX_CHARS.to_string()),
            "the refusal states the bound it applied: {}",
            refused.errors[0].message
        );
    }

    #[test]
    fn a_blank_field_is_the_callers_error_rather_than_a_stored_empty_string() {
        for raw in ["", "   ", "\t\n"] {
            let refused = bounded("a device id", raw, 64).expect_err("a blank id is refused");
            assert_eq!(refused.errors[0].kind, Some(APIErrorKind::Validation));
        }
    }

    #[test]
    fn every_stored_status_is_accepted_and_nothing_else_is() {
        for status in DeviceSessionStatus::ALL {
            assert_eq!(status_of(status.as_str()).ok(), Some(status));
        }
        let refused = status_of("expired").expect_err("an invented status is refused");
        assert_eq!(refused.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            refused.errors[0].message.contains("connected"),
            "the refusal names the vocabulary it wanted: {}",
            refused.errors[0].message
        );
    }

    #[test]
    fn a_heartbeat_may_not_claim_a_session_for_a_sanctioned_marketplace() {
        let refused = reported(&session(Marketplace::Etsy, "connected"))
            .expect_err("Etsy's automation runs server-side, so no device holds a session for it");
        assert_eq!(refused.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        for marketplace in [Marketplace::Tpt, Marketplace::Tes] {
            assert!(
                reported(&session(marketplace, "connected")).is_ok(),
                "{marketplace:?} is a seller-device marketplace and may be reported"
            );
        }
    }

    #[test]
    fn a_marketplace_path_segment_is_its_serde_name_and_nothing_else() {
        for marketplace in Marketplace::ALL {
            let spelled = serde_json::to_value(marketplace).ok();
            assert_eq!(
                marketplace_of(
                    spelled
                        .as_ref()
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                ),
                Some(marketplace),
                "the path spells a marketplace the way the body and the generated \
                 vocabulary already do"
            );
        }
        for raw in ["tpt", "TPT", "teacherspayteachers", ""] {
            assert_eq!(
                marketplace_of(raw),
                None,
                "{raw:?} is not a marketplace, and a route that guessed would declare \
                 authorship against the wrong connection"
            );
        }
    }

    #[test]
    fn a_wipe_is_outstanding_only_between_the_sign_out_and_the_next_check_in() {
        assert!(
            !wipe_outstanding(None, Timestamp(10)),
            "a device nobody signed out has nothing to wipe"
        );
        assert!(
            wipe_outstanding(Some(Timestamp(20)), Timestamp(10)),
            "signed out, and not heard from since: it may still hold its cookies"
        );
        assert!(
            !wipe_outstanding(Some(Timestamp(20)), Timestamp(20)),
            "a check-in at the revocation instant carried the instruction"
        );
        assert!(
            !wipe_outstanding(Some(Timestamp(20)), Timestamp(30)),
            "a later check-in is the evidence the device complied"
        );
    }
}
