//! The seller's completion inbox and the one switch that governs its mail.
//!
//! Every route reads the organisation and the user from the session's
//! [`OrgContext`] and nowhere else, so no request carries an organisation or a
//! user a caller could substitute — which is the whole of why the preference
//! write cannot name somebody else's row.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{NotificationCursor, NotificationRecord, NotificationRepo};
use tam_types::{InventoryId, Marketplace, NotificationCounts, NotificationKind, Timestamp, Uuid};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// Pages this size unless the caller asks otherwise, and never larger. The
/// same pair the job listing uses, because the console pages both the same way.
const PAGE_LIMIT_DEFAULT: i64 = 50;
const PAGE_LIMIT_MAX: i64 = 200;

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
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
        APIErrorEntry::new("no such notification")
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

/// One completion as the console renders it.
///
/// `inventory` and `marketplace` are always present and are null together, for
/// an import and only for an import; a client reading either therefore needs
/// no optional-field branch and no second lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationView {
    pub id: Uuid,
    pub kind: NotificationKind,
    pub subject_id: Uuid,
    pub inventory: Option<InventoryId>,
    pub marketplace: Option<Marketplace>,
    pub counts: NotificationCounts,
    pub created_at: Timestamp,
    pub read_at: Option<Timestamp>,
}

impl NotificationView {
    fn of(record: &NotificationRecord) -> Self {
        Self {
            id: record.id,
            kind: record.kind,
            subject_id: record.subject,
            inventory: record.inventory,
            marketplace: record.inventory.map(InventoryId::marketplace),
            counts: record.counts,
            created_at: record.created_at,
            read_at: record.read_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotificationPage {
    pub notifications: Vec<NotificationView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadBody {
    /// Everything at or before this one is marked read, in the order the list
    /// pages in.
    pub through: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReadAck {
    pub marked: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PreferencesView {
    pub notify_email: bool,
}

/// Encodes the keyset as the opaque token the client carries back. The format
/// is a server detail; the contract is "opaque".
#[must_use]
pub fn encode_cursor(cursor: &NotificationCursor) -> String {
    use core::fmt::Write as _;
    let mut token = format!("{:x}.", cursor.created_at.0);
    for byte in cursor.id.0 {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(token, "{byte:02x}");
    }
    token
}

/// Parses a token this server minted; anything else is `None` and the request
/// is refused as validation, never guessed at.
#[must_use]
pub fn decode_cursor(raw: &str) -> Option<NotificationCursor> {
    let (millis_hex, id_hex) = raw.split_once('.')?;
    let millis = i64::from_str_radix(millis_hex, 16).ok()?;
    if id_hex.len() != 32 {
        return None;
    }
    let mut id = [0u8; 16];
    for (index, slot) in id.iter_mut().enumerate() {
        let pair = id_hex.get(index * 2..index * 2 + 2)?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(NotificationCursor {
        created_at: Timestamp(millis),
        id: Uuid(id),
    })
}

/// The organisation's completions, newest first.
pub(crate) async fn list(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<PageParams>,
) -> Result<Json<NotificationPage>, APIError> {
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(
            decode_cursor(raw)
                .ok_or_else(|| validation("the cursor is not one this server issued"))?,
        ),
    };
    let limit = params
        .limit
        .unwrap_or(PAGE_LIMIT_DEFAULT)
        .clamp(1, PAGE_LIMIT_MAX);
    // One more than the page, so the cursor states that something follows
    // rather than that the page happened to fill. A seller holding exactly
    // fifty notifications would otherwise be drawn a "Load more" that fetches
    // nothing and then disappears.
    let mut rows = NotificationRepo::new(state.pool.clone())
        .list(context.org, cursor, limit.saturating_add(1))
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    let next_cursor = more
        .then(|| {
            rows.last().map(|last| {
                encode_cursor(&NotificationCursor {
                    created_at: last.created_at,
                    id: last.id,
                })
            })
        })
        .flatten();
    Ok(Json(NotificationPage {
        notifications: rows.iter().map(NotificationView::of).collect(),
        next_cursor,
    }))
}

/// Marks everything up to a given completion read.
///
/// Idempotent: a second call naming the same one marks nothing and answers
/// zero, which is what a console that re-sends on reconnect needs.
pub(crate) async fn mark_read(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<ReadBody>,
) -> Result<Json<ReadAck>, APIError> {
    let marked = NotificationRepo::new(state.pool.clone())
        .mark_read_through(context.org, body.through, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(ReadAck { marked }))
}

/// Whether this seller wants the mail.
pub(crate) async fn preferences(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<PreferencesView>, APIError> {
    let notify_email = NotificationRepo::new(state.pool.clone())
        .notify_email(context.org, context.user)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(PreferencesView { notify_email }))
}

/// Sets it, for the requesting user and no other: the body carries the value
/// and nothing that could name a different row.
pub(crate) async fn update_preferences(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<PreferencesView>,
) -> Result<Json<PreferencesView>, APIError> {
    let notify_email = NotificationRepo::new(state.pool.clone())
        .set_notify_email(context.org, context.user, body.notify_email)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(PreferencesView { notify_email }))
}

#[cfg(test)]
mod tests {
    use super::{decode_cursor, encode_cursor};
    use tam_storage::NotificationCursor;
    use tam_types::{Timestamp, Uuid};

    #[test]
    fn a_cursor_survives_its_own_round_trip() {
        let cursor = NotificationCursor {
            created_at: Timestamp(1_757_164_800_000),
            id: Uuid([0x3f; 16]),
        };
        let back = decode_cursor(&encode_cursor(&cursor)).expect("the token this server minted");
        assert_eq!(back, cursor, "the keyset must survive its own encoding");
    }

    #[test]
    fn a_token_this_server_did_not_mint_is_refused() {
        for raw in ["", ".", "zz.00", "1996d1a3f00.", "1996d1a3f00.abcd"] {
            assert!(
                decode_cursor(raw).is_none(),
                "{raw:?} is not a cursor and must not be guessed at"
            );
        }
    }
}
