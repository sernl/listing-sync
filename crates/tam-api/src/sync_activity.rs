//! The activity log and the multi-listed list: the two reads at the bottom of
//! Marketplace Sync.
//!
//! Both answer in the seller's own words rather than in the ledger's. A line
//! is composed here, on the server, from a structured row the storage layer
//! read -- `"Fractions Pack" pulled from TPT` -- because the alternative is a
//! client assembling English out of three event kinds and a product lookup,
//! which is how one page ends up saying something a different page would not.
//! The cursor is a token the server mints rather than a bare instant, and
//! deliberately: three sources are merged by time, so "older than this" is the
//! only thing that can page all three at once — but an instant alone does not
//! order the log. Two lines of one millisecond are ordinary, and a cursor of
//! "strictly older than the last instant" dropped the rest of that instant
//! while one of "that instant or older" repeated it forever. The token
//! therefore carries the instant *and* the last line's key, and the storage
//! layer orders and filters by the pair, so a page boundary can fall inside an
//! instant and lose nothing.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    system_label_name, ActivityCursor, ActivityKind, SyncSettingRepo, ACTIVITY_LISTED_MAX,
};
use tam_types::{InventoryId, ProductId, Timestamp};

use crate::error::APIError;
use crate::jobs::{storage_fault, validation};
use crate::{AppState, OrgContext};

// ------------------------------------------------------------------- wire

/// One line of the log, already worded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityLineView {
    pub at: Timestamp,
    /// This line's own identity, as the read that produced it minted it.
    ///
    /// The console keys its rendered list by this rather than by a position in
    /// the array it happens to hold: a list keyed by index re-keys every row
    /// the moment a page is replaced, and the rendered rows then belong to
    /// keys that named different lines a moment earlier.
    pub key: String,
    pub line: String,
    /// Where the line goes when a seller clicks it, or nothing where it goes
    /// nowhere: a schedule's tick has no page of its own yet, and a link to
    /// the page you are on is worse than none.
    pub href: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityView {
    pub activity: Vec<ActivityLineView>,
    /// The token for the next page, or nothing when this page is the end.
    /// Opaque: it carries the last line's instant and key together, because
    /// the instant alone does not order a log three sources merge into.
    pub cursor: Option<String>,
}

/// One resource that is live on more than one marketplace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiListedView {
    pub product: ProductId,
    pub title: String,
    pub inventories: Vec<InventoryId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiListedsView {
    pub multi: Vec<MultiListedView>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ActivityParams {
    pub cursor: Option<String>,
    pub limit: Option<String>,
}

/// Encodes the end of a page as the token the console carries back. The
/// format is a server detail; the contract is "opaque", exactly as it is for
/// the ledger's own cursor.
#[must_use]
pub fn encode_activity_cursor(cursor: &ActivityCursor) -> String {
    use core::fmt::Write;
    let mut token = format!("{:x}.", cursor.at.0);
    for byte in cursor.key.as_bytes() {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(token, "{byte:02x}");
    }
    token
}

/// Parses a token this server minted; anything else is `None` and the request
/// is refused as validation rather than guessed at. A guessed cursor is worse
/// here than elsewhere: it would silently answer a page from the middle of a
/// log the seller is reading as a history.
#[must_use]
pub fn decode_activity_cursor(raw: &str) -> Option<ActivityCursor> {
    let (millis_hex, key_hex) = raw.split_once('.')?;
    let at = Timestamp(i64::from_str_radix(millis_hex, 16).ok()?);
    if key_hex.is_empty() || !key_hex.len().is_multiple_of(2) {
        return None;
    }
    let mut key = Vec::with_capacity(key_hex.len() >> 1);
    for pair in key_hex.as_bytes().chunks_exact(2) {
        let pair = core::str::from_utf8(pair).ok()?;
        key.push(u8::from_str_radix(pair, 16).ok()?);
    }
    Some(ActivityCursor {
        at,
        key: String::from_utf8(key).ok()?,
    })
}

// ---------------------------------------------------------------- handlers

pub(crate) async fn activity(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<ActivityParams>,
) -> Result<Json<ActivityView>, APIError> {
    let cursor = match params.cursor.as_deref() {
        None | Some("") => None,
        Some(raw) => Some(
            decode_activity_cursor(raw)
                .ok_or_else(|| validation("This page link has expired. Reload the page."))?,
        ),
    };
    let limit = match params.limit.as_deref() {
        None | Some("") => ACTIVITY_LISTED_MAX,
        Some(raw) => raw
            .parse::<i64>()
            .ok()
            .filter(|limit| (1..=ACTIVITY_LISTED_MAX).contains(limit))
            .ok_or_else(|| {
                validation("a limit is between 1 and 50, which is what one page answers")
            })?,
    };
    let rows = SyncSettingRepo::new(state.pool.clone())
        .activity(context.org, cursor, limit)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    // A full page may have more behind it and a short one cannot, which is
    // the same rule `list_jobs` pages by.
    let next = (i64::try_from(rows.len()).unwrap_or(i64::MAX) == limit)
        .then(|| {
            rows.last().map(|last| {
                encode_activity_cursor(&ActivityCursor {
                    at: last.at,
                    key: last.key.clone(),
                })
            })
        })
        .flatten();
    Ok(Json(ActivityView {
        activity: rows.iter().map(line_of).collect(),
        cursor: next,
    }))
}

pub(crate) async fn multi_listed(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<MultiListedsView>, APIError> {
    let rows = SyncSettingRepo::new(state.pool.clone())
        .multi_listed(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(MultiListedsView {
        multi: rows
            .into_iter()
            .map(|row| MultiListedView {
                product: row.product,
                title: row.title,
                inventories: row.inventories,
            })
            .collect(),
    }))
}

// ------------------------------------------------------------------ words

/// One structured row as the sentence the seller reads.
///
/// The marketplace is named by `system_label_name`, which is the one place
/// this tree writes "TPT" rather than "Tpt": the import label, the activity
/// line and the chip all read it, so a fourth marketplace gets one word
/// rather than three.
fn line_of(row: &tam_storage::ActivityRow) -> ActivityLineView {
    let (line, href) = match &row.kind {
        ActivityKind::Pulled {
            run,
            title,
            source,
            product: _,
        } => (
            format!("\"{title}\" brought in from {}", name_of(*source)),
            Some(format!("/imports/runs/{}", run.to_hyphenated())),
        ),
        ActivityKind::Published {
            job,
            title,
            target,
            product: _,
        } => (
            format!("\"{title}\" published to {}", name_of(*target)),
            Some(format!("/jobs/{}", job.0.to_hyphenated())),
        ),
        ActivityKind::ScheduleTick {
            name,
            inventory,
            sent,
            schedule: _,
        } => (
            format!(
                "Schedule \"{name}\" sent {sent} {} to {}",
                if *sent == 1 { "resource" } else { "resources" },
                name_of(*inventory),
            ),
            None,
        ),
    };
    ActivityLineView {
        at: row.at,
        key: row.key.clone(),
        line,
        href,
    }
}

const fn name_of(inventory: InventoryId) -> &'static str {
    system_label_name(inventory.marketplace())
}

#[cfg(test)]
mod tests {
    use super::{decode_activity_cursor, encode_activity_cursor, line_of, ActivityKind};
    use tam_storage::{ActivityCursor, ActivityRow};
    use tam_types::{InventoryId, ProductId, Timestamp, Uuid};

    #[test]
    fn imported_resource_activity_links_to_its_run() {
        let pulled = line_of(&ActivityRow {
            at: Timestamp(1_000),
            key: "pulled:a:b".to_owned(),
            kind: ActivityKind::Pulled {
                run: Uuid([0x11; 16]),
                product: ProductId(Uuid([0x22; 16])),
                title: "Fractions Pack".to_owned(),
                source: InventoryId::Tpt,
            },
        });
        assert_eq!(
            pulled.href.as_deref(),
            Some("/imports/runs/11111111-1111-1111-1111-111111111111")
        );
    }

    /// Both halves survive the round trip, including the separators the keys
    /// are composed with: a token that lost the key would page by the instant
    /// alone, which is the ambiguity the key exists to remove.
    #[test]
    fn a_cursor_survives_its_own_encoding() {
        let cursor = ActivityCursor {
            at: Timestamp(1_726_000_000_123),
            key: "published:11111111-1111-1111-1111-111111111111:22222222-2222-2222-2222-222222222222:Tes"
                .to_owned(),
        };
        let token = encode_activity_cursor(&cursor);
        assert_eq!(decode_activity_cursor(&token), Some(cursor));
    }

    /// Anything this server did not mint is refused rather than read as a
    /// plausible instant.
    #[test]
    fn a_cursor_this_server_did_not_mint_is_refused() {
        for raw in [
            "",
            "1726000000123",
            "zz.7075",
            "1a2b.",
            "1a2b.706",
            "1a2b.ff",
        ] {
            assert_eq!(decode_activity_cursor(raw), None, "{raw} is not a cursor");
        }
    }
}
