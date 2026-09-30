//! The seller's inbox, the notices the console posts into it, and the one
//! switch that governs its mail.
//!
//! Every route reads the organisation and the user from the session's
//! [`OrgContext`] and nowhere else, so no request carries an organisation or a
//! user a caller could substitute — which is the whole of why the preference
//! write cannot name somebody else's row, and why a notice is only ever its
//! poster's.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    NewNotice, NotificationCursor, NotificationRecord, NotificationRepo, NotificationSource,
};
use tam_types::{
    InventoryId, Marketplace, NoticeTone, NotificationCounts, NotificationKind, Timestamp, Uuid,
};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// Pages this size unless the caller asks otherwise, and never larger. The
/// same pair the job listing uses, because the console pages both the same way.
const PAGE_LIMIT_DEFAULT: i64 = 50;
const PAGE_LIMIT_MAX: i64 = 200;

/// The longest a notice's title, body and client id may be, in characters;
/// the same bounds the table's `notification_notice_shape` holds.
const TITLE_MAX: usize = 200;
const BODY_MAX: usize = 1000;
const CLIENT_ID_MAX: usize = 64;

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
        APIErrorEntry::new("We can't find that notification.")
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(|parsed| Uuid(*parsed.as_bytes()))
        .map_err(|_unused| missing())
}

/// What a row is about, tagged `source` beside the row's own fields.
///
/// A run's `inventory` and `marketplace` are always present and are null
/// together, for an import and only for an import; a client reading either
/// therefore needs no optional-field branch and no second lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SourceView {
    Run {
        kind: NotificationKind,
        subject_id: Uuid,
        inventory: Option<InventoryId>,
        marketplace: Option<Marketplace>,
        counts: NotificationCounts,
    },
    Notice {
        title: String,
        body: String,
    },
}

/// One inbox row as the console renders it: the bell's line and the page's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationView {
    pub id: Uuid,
    pub tone: NoticeTone,
    #[serde(flatten)]
    pub source: SourceView,
    pub created_at: Timestamp,
    pub read_at: Option<Timestamp>,
}

impl NotificationView {
    fn of(record: &NotificationRecord) -> Self {
        let source = match &record.source {
            NotificationSource::Run {
                kind,
                subject,
                inventory,
                counts,
            } => SourceView::Run {
                kind: *kind,
                subject_id: *subject,
                inventory: *inventory,
                marketplace: inventory.map(InventoryId::marketplace),
                counts: *counts,
            },
            NotificationSource::Notice { title, body, .. } => SourceView::Notice {
                title: title.clone(),
                body: body.clone(),
            },
        };
        Self {
            id: record.id,
            tone: record.source.tone(),
            source,
            created_at: record.created_at,
            read_at: record.read_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NotificationPage {
    pub notifications: Vec<NotificationView>,
    pub next_cursor: Option<String>,
    /// Every unread row this reader has, not only the ones on this page: the
    /// bell's badge.
    pub unread: u64,
}

#[derive(Debug, Deserialize)]
pub struct PageParams {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

/// A toast the seller did not look at, posted as it leaves the screen.
#[derive(Debug, Serialize, Deserialize)]
pub struct NoticeBody {
    /// The console's own id for the toast; posting it again is the same row.
    pub client_id: String,
    pub tone: NoticeTone,
    pub title: String,
    #[serde(default)]
    pub body: String,
}

impl NoticeBody {
    fn validated(self) -> Result<NewNotice, APIError> {
        let client_id = self.client_id.trim().to_owned();
        if client_id.is_empty() || client_id.chars().count() > CLIENT_ID_MAX {
            return Err(validation("This notice has no usable id."));
        }
        let title = self.title.trim().to_owned();
        if title.is_empty() {
            return Err(validation("A notice needs something to say."));
        }
        if title.chars().count() > TITLE_MAX {
            return Err(validation("This notice's title is too long."));
        }
        let body = self.body.trim().to_owned();
        if body.chars().count() > BODY_MAX {
            return Err(validation("This notice's text is too long."));
        }
        Ok(NewNotice {
            client_id,
            tone: self.tone,
            title,
            body,
        })
    }
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
pub struct DismissAck {
    pub dismissed: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PreferencesView {
    pub notify_email: bool,
    /// Whether they take the operators' news and tips
    /// (`crate::mail_campaigns`); off once they follow an unsubscribe link.
    pub marketing_email: bool,
}

/// A change to either switch; a field left out is left as it is.
#[derive(Debug, Deserialize)]
pub struct PreferencesPatch {
    #[serde(default)]
    pub notify_email: Option<bool>,
    #[serde(default)]
    pub marketing_email: Option<bool>,
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

/// What this seller may read of the inbox, newest first, with their unread
/// count.
pub(crate) async fn list(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<PageParams>,
) -> Result<Json<NotificationPage>, APIError> {
    let cursor = match params.cursor.as_deref() {
        None => None,
        Some(raw) => Some(
            decode_cursor(raw)
                .ok_or_else(|| validation("This page link has expired. Reload the page."))?,
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
    let repo = NotificationRepo::new(state.pool.clone());
    let mut rows = repo
        .list(context.org, context.user, cursor, limit.saturating_add(1))
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let unread = repo
        .unread(context.org, context.user)
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
        unread,
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
        .mark_read_through(context.org, context.user, body.through, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(ReadAck { marked }))
}

/// Keeps a toast the seller did not look at, for the bell. Answers the row,
/// whether this post wrote it or an earlier one with the same client id did.
pub(crate) async fn post_notice(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<NoticeBody>,
) -> Result<Json<NotificationView>, APIError> {
    let notice = body.validated()?;
    let record = NotificationRepo::new(state.pool.clone())
        .post_notice(context.org, context.user, &notice, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(NotificationView::of(&record)))
}

/// Marks one row read. Idempotent: a row read already answers zero.
pub(crate) async fn mark_one_read(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<Json<ReadAck>, APIError> {
    let id = parse_id(&id)?;
    let marked = NotificationRepo::new(state.pool.clone())
        .mark_read(context.org, context.user, id, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(missing)?;
    Ok(Json(ReadAck {
        marked: u64::from(marked),
    }))
}

/// Marks everything this seller has unread, read.
pub(crate) async fn mark_all_read(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ReadAck>, APIError> {
    let marked = NotificationRepo::new(state.pool.clone())
        .mark_all_read(context.org, context.user, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ReadAck { marked }))
}

/// Takes one row out of the inbox. Idempotent: dismissing it again is still
/// dismissed.
pub(crate) async fn dismiss(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    let id = parse_id(&id)?;
    let found = NotificationRepo::new(state.pool.clone())
        .dismiss(context.org, context.user, id, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if found {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing())
    }
}

/// Takes every read row out of the inbox: the page's Delete read.
pub(crate) async fn dismiss_read(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<DismissAck>, APIError> {
    let dismissed = NotificationRepo::new(state.pool.clone())
        .dismiss_read(context.org, context.user, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(DismissAck { dismissed }))
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
    let marketing_email =
        crate::mail_campaigns::marketing_preference(&state, &context, None).await?;
    Ok(Json(PreferencesView {
        notify_email,
        marketing_email,
    }))
}

/// Sets either, for the requesting user and no other: the body carries the
/// values and nothing that could name a different row.
pub(crate) async fn update_preferences(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<PreferencesPatch>,
) -> Result<Json<PreferencesView>, APIError> {
    let repo = NotificationRepo::new(state.pool.clone());
    let notify_email = match body.notify_email {
        Some(wanted) => {
            repo.set_notify_email(context.org, context.user, wanted)
                .await
        }
        None => repo.notify_email(context.org, context.user).await,
    }
    .map_err(|error| storage_fault(&state, &error))?
    .ok_or_else(missing)?;
    let marketing_email =
        crate::mail_campaigns::marketing_preference(&state, &context, body.marketing_email).await?;
    Ok(Json(PreferencesView {
        notify_email,
        marketing_email,
    }))
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
