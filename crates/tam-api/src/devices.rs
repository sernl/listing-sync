//! The device registry over the wire: register, heartbeat, list and sign out.
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

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    ConnectionFactsRepo, DeviceRecord, DeviceRegistration, DeviceRepo, DeviceSessionRecord,
    DeviceSessionReport, DeviceSessionStatus,
};
use tam_types::{Marketplace, Timestamp, TransportClass};

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

#[derive(Debug, Deserialize)]
pub struct HeartbeatBody {
    /// Everything this device holds, not a delta. A marketplace absent here is
    /// taken as no longer held, because the page's job is saying what a device
    /// holds now and a delta would leave a stale row standing after a
    /// disconnect performed offline.
    pub sessions: Vec<HeartbeatSession>,
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
    let record = DeviceRepo::new(state.pool.clone())
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
    let beat = DeviceRepo::new(state.pool.clone())
        .heartbeat(context.org, device, &sessions, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(missing)?;
    Ok(Json(HeartbeatView {
        revoked: beat.revoked(),
        revoked_at: beat.revoked_at,
    }))
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
