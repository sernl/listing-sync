//! The activity log and the multi-listed list: the two reads at the bottom of
//! Marketplace Sync.
//!
//! Both answer in the seller's own words rather than in the ledger's. A line
//! is composed here, on the server, from a structured row the storage layer
//! read -- `"Fractions Pack" pulled from TPT` -- because the alternative is a
//! client assembling English out of three event kinds and a product lookup,
//! which is how one page ends up saying something a different page would not.
//!
//! The cursor is an instant rather than an opaque token, and deliberately:
//! three sources are merged by time, so "older than this" is the only thing
//! that can page all three at once. It is exact rather than approximate --
//! `at` of the oldest line handed back -- so a line is never skipped, and a
//! line sharing an instant with the page boundary may repeat, which is the
//! cheaper wrong answer for a log.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{system_label_name, ActivityKind, SyncSettingRepo, ACTIVITY_LISTED_MAX};
use tam_types::{InventoryId, ProductId, Timestamp};

use crate::error::APIError;
use crate::jobs::{storage_fault, validation};
use crate::{AppState, OrgContext};

// ------------------------------------------------------------------- wire

/// One line of the log, already worded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityLineView {
    pub at: Timestamp,
    pub line: String,
    /// Where the line goes when a seller clicks it, or nothing where it goes
    /// nowhere: a schedule's tick has no page of its own yet, and a link to
    /// the page you are on is worse than none.
    pub href: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityView {
    pub activity: Vec<ActivityLineView>,
    /// The cursor for the next page, or nothing when this page is the end.
    pub cursor: Option<Timestamp>,
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

// ---------------------------------------------------------------- handlers

pub(crate) async fn activity(
    State(state): State<AppState>,
    context: OrgContext,
    Query(params): Query<ActivityParams>,
) -> Result<Json<ActivityView>, APIError> {
    let cursor = match params.cursor.as_deref() {
        None | Some("") => None,
        Some(raw) => Some(Timestamp(raw.parse::<i64>().map_err(|_unused| {
            validation("a cursor is the `at` of the oldest line of the previous page")
        })?)),
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
        .then(|| rows.last().map(|last| last.at))
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
            format!("\"{title}\" pulled from {}", name_of(*source)),
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
        line,
        href,
    }
}

const fn name_of(inventory: InventoryId) -> &'static str {
    system_label_name(inventory.marketplace())
}

#[cfg(test)]
mod tests {
    use super::{line_of, ActivityKind};
    use tam_storage::ActivityRow;
    use tam_types::{InventoryId, JobId, ProductId, Timestamp, Uuid};

    /// The three lines, in the seller's words and with the marketplace spelled
    /// the way their own chip spells it.
    ///
    /// Pinned because these strings are the feature: a log that says "Tpt" or
    /// "import_run_item settled" is a log nobody reads.
    #[test]
    fn each_kind_is_one_sentence_a_seller_reads() {
        let pulled = line_of(&ActivityRow {
            at: Timestamp(1_000),
            kind: ActivityKind::Pulled {
                run: Uuid([0x11; 16]),
                product: ProductId(Uuid([0x22; 16])),
                title: "Fractions Pack".to_owned(),
                source: InventoryId::Tpt,
            },
        });
        assert_eq!(pulled.line, "\"Fractions Pack\" pulled from TPT");
        assert_eq!(
            pulled.href.as_deref(),
            Some("/imports/runs/11111111-1111-1111-1111-111111111111")
        );

        let published = line_of(&ActivityRow {
            at: Timestamp(2_000),
            kind: ActivityKind::Published {
                job: JobId(Uuid([0x33; 16])),
                product: ProductId(Uuid([0x22; 16])),
                title: "Fractions Pack".to_owned(),
                target: InventoryId::Tes,
            },
        });
        assert_eq!(published.line, "\"Fractions Pack\" published to Tes");

        let tick = line_of(&ActivityRow {
            at: Timestamp(3_000),
            kind: ActivityKind::ScheduleTick {
                schedule: Uuid([0x44; 16]),
                name: "Friday drop".to_owned(),
                inventory: InventoryId::Tpt,
                sent: 4,
            },
        });
        assert_eq!(
            tick.line,
            "Schedule \"Friday drop\" sent 4 resources to TPT"
        );
        assert_eq!(tick.href, None, "a tick has no page of its own yet");
    }

    /// One resource is a resource. The plural is the only arithmetic in this
    /// module and it is the one a seller would notice.
    #[test]
    fn one_resource_is_not_one_resources() {
        let tick = line_of(&ActivityRow {
            at: Timestamp(3_000),
            kind: ActivityKind::ScheduleTick {
                schedule: Uuid([0x44; 16]),
                name: "Friday drop".to_owned(),
                inventory: InventoryId::Tes,
                sent: 1,
            },
        });
        assert_eq!(tick.line, "Schedule \"Friday drop\" sent 1 resource to Tes");
    }
}
