//! Schedules: publishing a selection of resources to a marketplace at a
//! minute the seller chose.
//!
//! Five routes and nothing clever. A schedule is a row the seller writes and
//! the pass reads; these routes never fire one, and the pass never validates
//! one. That split is what makes the timetable testable: the arithmetic is a
//! pure function over the stored columns (`tam_storage::due_tick`), and every
//! refusal a seller can meet is here, before a minute ever arrives.
//!
//! The runs list is the record of what ticks did, grouped by tick and
//! marketplace, because that is how the console draws it: one line per
//! marketplace per firing, with the resources a lowering refused listed under
//! it. A refusal is a row rather than a failed tick -- one live Tes listing
//! that cannot be revised must not cost the seller the other nineteen
//! resources of the drop.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    next_tick, timezone_of, ScheduleOutcome, ScheduleRecord, ScheduleRepeat, ScheduleRepo,
    ScheduleRunRow, ScheduleSelection, ScheduleWrite, SyncIntent,
};
use tam_types::{InventoryId, OrgId, ProductId, Timestamp, Uuid};

use crate::entitlement::feature_refusal;
use crate::error::APIError;
use crate::jobs::{storage_fault, validation};
use crate::{AppState, OrgContext};

/// The sentence a plan without the timetable answers with, in the console's
/// own words -- `entitlement.ts::featureReason` carries the same one, so the
/// refusal a route makes and the hint a disabled control shows are one
/// sentence rather than two that drifted.
const NO_SCHEDULING: &str = "Your plan does not include scheduling. Upgrade to publish on a \
                             timetable.";

/// And the one for republishing, which is the auto-publish capability rather
/// than the timetable: a seller may hold the clock and not the rewrite.
const NO_REPUBLISH: &str = "Your plan does not include republishing a listing when its resource \
                            changes. Upgrade to use it.";

// ------------------------------------------------------------------- wire

/// How often a schedule fires, on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleRepeatView {
    Once,
    Daily,
    Weekly,
}

impl ScheduleRepeatView {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::Once, Self::Daily, Self::Weekly];

    #[must_use]
    pub const fn of(repeat: ScheduleRepeat) -> Self {
        match repeat {
            ScheduleRepeat::Once => Self::Once,
            ScheduleRepeat::Daily => Self::Daily,
            ScheduleRepeat::Weekly => Self::Weekly,
        }
    }

    #[must_use]
    pub const fn stored(self) -> ScheduleRepeat {
        match self {
            Self::Once => ScheduleRepeat::Once,
            Self::Daily => ScheduleRepeat::Daily,
            Self::Weekly => ScheduleRepeat::Weekly,
        }
    }
}

/// Which resources a schedule sends.
///
/// Untagged, and the two shapes are distinguishable by their one field:
/// `{"label": "Ready"}` or `{"products": [...]}`. The same spelling
/// `MigrationSelection` uses, for the same reason -- a seller who named four
/// resources has not named a label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScheduleSelectionView {
    Label { label: String },
    Products { products: Vec<ProductId> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleView {
    pub id: Uuid,
    pub name: String,
    pub selection: ScheduleSelectionView,
    pub inventories: Vec<InventoryId>,
    /// `draft` or `live`, in `sync_request.intent`'s own spelling.
    pub intent: String,
    pub at_minute_of_day: u16,
    pub timezone: String,
    pub repeat: ScheduleRepeatView,
    pub weekday: Option<u8>,
    pub republish_on_update: bool,
    pub enabled: bool,
    /// When this will fire next, computed rather than stored: the console
    /// renders it and nothing writes it. Absent for a schedule that is off
    /// and for a `once` one that has already run, which are both "never
    /// again" rather than a date in the past.
    pub next_run_at: Option<Timestamp>,
    pub last_run_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulesView {
    pub schedules: Vec<ScheduleView>,
}

/// The body both write routes take: a [`ScheduleView`] minus the three fields
/// the server owns.
///
/// One body for create and replace, so the two cannot describe two different
/// schedules -- `MigrationBody`'s own rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleBody {
    pub name: String,
    pub selection: ScheduleSelectionView,
    pub inventories: Vec<InventoryId>,
    pub intent: String,
    pub at_minute_of_day: u16,
    pub timezone: String,
    pub repeat: ScheduleRepeatView,
    #[serde(default)]
    pub weekday: Option<u8>,
    #[serde(default)]
    pub republish_on_update: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// A schedule the seller did not say to switch off is on: the form's toggle
/// ships defaulted to on, and a body omitting it is one from a client that
/// has no toggle yet rather than a request to write a dormant row.
const fn yes() -> bool {
    true
}

/// One resource a tick did not send, and the sentence it was refused with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedMemberView {
    pub product: ProductId,
    pub title: String,
    pub reason: String,
}

/// One marketplace's share of one tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRunView {
    pub tick: Timestamp,
    pub inventory: InventoryId,
    /// The job the sent resources were minted into. Absent where the tick
    /// sent nothing, which is a tick every member was refused on.
    pub job: Option<Uuid>,
    pub sent: u32,
    pub skipped: Vec<SkippedMemberView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleRunsView {
    pub runs: Vec<ScheduleRunView>,
}

// ---------------------------------------------------------------- handlers

pub(crate) async fn list_schedules(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<SchedulesView>, APIError> {
    held(&context)?;
    let now = (state.wall)();
    let schedules = ScheduleRepo::new(state.pool.clone())
        .list(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(SchedulesView {
        schedules: schedules
            .iter()
            .map(|record| view_of(record, now))
            .collect(),
    }))
}

pub(crate) async fn create_schedule(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<ScheduleBody>,
) -> Result<(StatusCode, Json<ScheduleView>), APIError> {
    held(&context)?;
    let write = parse(&context, &body)?;
    let now = (state.wall)();
    let id = Uuid(*uuid::Uuid::new_v4().as_bytes());
    let repo = ScheduleRepo::new(state.pool.clone());
    if !repo
        .create(context.org, id, &write, now)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        return Err(validation(
            "a schedule of that name already exists; give this one a different name",
        ));
    }
    Ok((
        StatusCode::CREATED,
        Json(reread(&state, &repo, context.org, id, now).await?),
    ))
}

pub(crate) async fn update_schedule(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, schedule)): Path<(String, String)>,
    Json(body): Json<ScheduleBody>,
) -> Result<Json<ScheduleView>, APIError> {
    held(&context)?;
    let id = parse_id(&schedule)?;
    let write = parse(&context, &body)?;
    let now = (state.wall)();
    let repo = ScheduleRepo::new(state.pool.clone());
    if !repo
        .update(context.org, id, &write)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        return Err(missing());
    }
    Ok(Json(reread(&state, &repo, context.org, id, now).await?))
}

pub(crate) async fn delete_schedule(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, schedule)): Path<(String, String)>,
) -> Result<StatusCode, APIError> {
    held(&context)?;
    let id = parse_id(&schedule)?;
    if ScheduleRepo::new(state.pool.clone())
        .delete(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(missing())
    }
}

/// What this schedule's ticks did.
pub(crate) async fn schedule_runs(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, schedule)): Path<(String, String)>,
) -> Result<Json<ScheduleRunsView>, APIError> {
    held(&context)?;
    let id = parse_id(&schedule)?;
    let repo = ScheduleRepo::new(state.pool.clone());
    if repo
        .get(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .is_none()
    {
        return Err(missing());
    }
    let rows = repo
        .runs(context.org, id)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    Ok(Json(ScheduleRunsView { runs: group(&rows) }))
}

// ----------------------------------------------------------------- shared

/// Whether this plan holds the timetable at all.
fn held(context: &OrgContext) -> Result<(), APIError> {
    if context.entitlement.caps.scheduling {
        Ok(())
    } else {
        Err(feature_refusal("scheduling", NO_SCHEDULING))
    }
}

#[must_use]
pub(crate) fn view_of(record: &ScheduleRecord, now: Timestamp) -> ScheduleView {
    ScheduleView {
        id: record.id,
        name: record.name.clone(),
        selection: match &record.selection {
            ScheduleSelection::Label(label) => ScheduleSelectionView::Label {
                label: label.clone(),
            },
            ScheduleSelection::Products(products) => ScheduleSelectionView::Products {
                products: products.clone(),
            },
        },
        inventories: record.inventories.clone(),
        intent: record.intent.as_str().to_owned(),
        at_minute_of_day: record.at_minute_of_day,
        timezone: record.timezone.clone(),
        repeat: ScheduleRepeatView::of(record.repeat),
        weekday: record.weekday,
        republish_on_update: record.republish_on_update,
        enabled: record.enabled,
        next_run_at: next_tick(record, now),
        last_run_at: record.last_run_at,
    }
}

/// The written row read back, rather than the body echoed.
///
/// `next_run_at` is computed from what was stored, and a view assembled from
/// the request would be the one place those two could disagree.
async fn reread(
    state: &AppState,
    repo: &ScheduleRepo,
    org: OrgId,
    id: Uuid,
    now: Timestamp,
) -> Result<ScheduleView, APIError> {
    let record = repo
        .get(org, id)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(missing)?;
    Ok(view_of(&record, now))
}

/// Every refusal a schedule body can meet, in one place.
fn parse(context: &OrgContext, body: &ScheduleBody) -> Result<ScheduleWrite, APIError> {
    let name = body.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(validation(
            "a schedule needs a name, and a name is at most 120 characters",
        ));
    }
    if body.inventories.is_empty() {
        return Err(validation(
            "a schedule sends to at least one marketplace; this one names none",
        ));
    }
    let intent = match body.intent.as_str() {
        "draft" => SyncIntent::Draft,
        "live" => SyncIntent::Live,
        _ => return Err(validation("intent is \"draft\" or \"live\"")),
    };
    if body.at_minute_of_day > 1439 {
        return Err(validation(
            "a time of day is minutes past midnight, so it is between 0 and 1439",
        ));
    }
    // Against the compiled-in zone database rather than a pattern: a name
    // that looks like a zone and is not one would store fine and then fire at
    // an hour nobody chose, or never fire at all.
    timezone_of(&body.timezone).map_err(|unknown| validation(&unknown.to_string()))?;
    let repeat = body.repeat.stored();
    let weekday = match (repeat, body.weekday) {
        (ScheduleRepeat::Weekly, Some(weekday)) if weekday <= 6 => Some(weekday),
        (ScheduleRepeat::Weekly, _) => {
            return Err(validation(
                "a weekly schedule names the day it runs on, as 0 for Sunday through 6 for \
                 Saturday",
            ))
        }
        // Refused rather than dropped: a body carrying a weekday under
        // `daily` is a client that thinks it set one, and storing the row
        // without it would fire every day while the form showed Friday.
        (ScheduleRepeat::Once | ScheduleRepeat::Daily, Some(_)) => {
            return Err(validation(
                "only a weekly schedule has a weekday; this one repeats otherwise",
            ))
        }
        (ScheduleRepeat::Once | ScheduleRepeat::Daily, None) => None,
    };
    let selection = match &body.selection {
        ScheduleSelectionView::Label { label } => {
            let label = label.trim().to_owned();
            if label.is_empty() || label.chars().count() > 120 {
                return Err(validation(
                    "a label selection names a label, and a label is at most 120 characters",
                ));
            }
            ScheduleSelection::Label(label)
        }
        ScheduleSelectionView::Products { products } => {
            if products.is_empty() {
                return Err(validation(
                    "a selection is either a label or a list of resources; this list is empty",
                ));
            }
            ScheduleSelection::Products(products.clone())
        }
    };
    // The second capability, and its own refusal: republishing a live listing
    // when its resource changes is the auto-publish feature rather than the
    // clock, so a plan holding the clock alone is told which one it is short.
    if body.republish_on_update && !context.entitlement.caps.auto_publish_rules {
        return Err(feature_refusal("auto_publish_rules", NO_REPUBLISH));
    }
    Ok(ScheduleWrite {
        name,
        selection,
        inventories: body.inventories.clone(),
        intent,
        at_minute_of_day: body.at_minute_of_day,
        timezone: body.timezone.clone(),
        repeat,
        weekday,
        republish_on_update: body.republish_on_update,
        enabled: body.enabled,
    })
}

/// The run rows, grouped into one line per tick per marketplace.
///
/// The rows arrive newest tick first and ordered within a tick, so the
/// grouping is a fold rather than a map: a tick's job is the same for every
/// sent member of it, which is what makes `job` one field rather than a set.
fn group(rows: &[ScheduleRunRow]) -> Vec<ScheduleRunView> {
    let mut runs: Vec<ScheduleRunView> = Vec::new();
    for row in rows {
        let found = runs
            .iter()
            .position(|run| run.tick == row.tick && run.inventory == row.inventory);
        let index = if let Some(index) = found {
            index
        } else {
            runs.push(ScheduleRunView {
                tick: row.tick,
                inventory: row.inventory,
                job: None,
                sent: 0,
                skipped: Vec::new(),
            });
            runs.len().saturating_sub(1)
        };
        let Some(slot) = runs.get_mut(index) else {
            continue;
        };
        match &row.outcome {
            ScheduleOutcome::Sent(job) => {
                slot.job = Some(job.0);
                slot.sent = slot.sent.saturating_add(1);
            }
            ScheduleOutcome::Skipped(reason) => slot.skipped.push(SkippedMemberView {
                product: row.product,
                title: row.title.clone(),
                reason: reason.clone(),
            }),
        }
    }
    runs
}

fn parse_id(raw: &str) -> Result<Uuid, APIError> {
    Uuid::parse_hyphenated(raw).ok_or_else(|| validation("that is not a schedule identifier"))
}

/// Another organisation's schedule is missing rather than forbidden, which is
/// the posture every other org-scoped read here takes.
fn missing() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        crate::error::APIErrorEntry::new("no such schedule")
            .kind(crate::error::APIErrorKind::Validation),
    )
}

#[cfg(test)]
mod tests {
    use super::{group, ScheduleOutcome, ScheduleRunRow};
    use tam_types::{InventoryId, JobId, ProductId, Timestamp, Uuid};

    fn row(tick: i64, inventory: InventoryId, outcome: ScheduleOutcome) -> ScheduleRunRow {
        ScheduleRunRow {
            tick: Timestamp(tick),
            product: ProductId(Uuid([0x22; 16])),
            title: "Fractions Pack".to_owned(),
            inventory,
            outcome,
        }
    }

    /// One tick sending to two marketplaces is two lines, and the sent count
    /// is per line rather than per tick.
    ///
    /// The console draws one row per marketplace with its own job link, so a
    /// grouping that folded both into one would show a seller four resources
    /// sent to TPT when two went to Tes.
    #[test]
    fn a_tick_is_grouped_per_marketplace() {
        let job = JobId(Uuid([0x33; 16]));
        let runs = group(&[
            row(1_000, InventoryId::Tpt, ScheduleOutcome::Sent(job)),
            row(1_000, InventoryId::Tpt, ScheduleOutcome::Sent(job)),
            row(
                1_000,
                InventoryId::Tes,
                ScheduleOutcome::Skipped("Tes has no captured tes.edit_published".to_owned()),
            ),
        ]);
        assert_eq!(runs.len(), 2, "two marketplaces, two lines");
        assert_eq!(runs[0].sent, 2);
        assert_eq!(runs[0].job, Some(job.0));
        assert_eq!(runs[1].sent, 0, "Tes sent nothing");
        assert_eq!(runs[1].job, None, "and so has no job to link to");
        assert_eq!(runs[1].skipped.len(), 1);
    }

    /// Two ticks of the same schedule on the same marketplace stay two lines:
    /// the tick is part of the key, not only the marketplace.
    #[test]
    fn two_ticks_are_two_lines() {
        let job = JobId(Uuid([0x33; 16]));
        let runs = group(&[
            row(2_000, InventoryId::Tpt, ScheduleOutcome::Sent(job)),
            row(1_000, InventoryId::Tpt, ScheduleOutcome::Sent(job)),
        ]);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].tick, Timestamp(2_000), "newest tick first");
    }
}
