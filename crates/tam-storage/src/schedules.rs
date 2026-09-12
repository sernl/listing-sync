//! Publishing on a timetable: the schedule, the members one tick resolves,
//! and the record of what each tick did.
//!
//! The cadence arithmetic lives here beside the rows rather than at the API,
//! because it is a pure function of four stored columns -- the minute, the
//! zone, the repeat and the last tick -- and both callers need it: the pass
//! asks "is this due" and the console's view asks "when next". Two copies of
//! that would be two answers to the same question, and the one that drifted
//! would be the one nobody watches.
//!
//! Nothing here mints a job. `record_run` writes what a tick did *after* the
//! job exists, and its primary key is the fence that stops a second pass in
//! the same minute doing it again: the insert is `ON CONFLICT DO NOTHING`, so
//! idempotency is a constraint rather than a check-then-act.

use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use sqlx::PgPool;
use tam_types::{InventoryId, JobId, OrgId, ProductId, Timestamp, Title, Uuid};

use crate::codec::{
    inventory_from_db, inventory_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db,
    uuid_to_db,
};
use crate::sync_requests::SyncIntent;
use crate::{pin_org, StorageError};

/// How many ticks one schedule's runs list answers with. A tick is one row
/// per member per marketplace, so this is bounded in ticks rather than rows
/// and the handler groups them.
pub const SCHEDULE_RUNS_LISTED_MAX: i64 = 500;

/// How often a schedule fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleRepeat {
    /// The next due minute, and never again.
    Once,
    Daily,
    Weekly,
}

impl ScheduleRepeat {
    /// The closed set, in a stable order, for the vocabulary generator.
    pub const ALL: [Self; 3] = [Self::Once, Self::Daily, Self::Weekly];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
        }
    }

    /// # Errors
    ///
    /// A column value outside the migration's CHECK, which is a corrupt row.
    pub fn from_column(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "once" => Ok(Self::Once),
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            other => Err(StorageError::CorruptRow {
                reason: format!("unknown schedule repeat {other:?}"),
            }),
        }
    }
}

/// Which resources one tick sends.
///
/// Two spellings rather than one plus a nullable list, for `MigrationSelection`'s
/// reason: a label is re-resolved at every tick and grows as the seller labels
/// more work, while a tick list is frozen at what they chose. Reading those as
/// one would silently change what a schedule sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleSelection {
    Label(String),
    Products(Vec<ProductId>),
}

/// A schedule as the seller states it: everything the two write routes take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleWrite {
    pub name: String,
    pub selection: ScheduleSelection,
    pub inventories: Vec<InventoryId>,
    pub intent: SyncIntent,
    pub at_minute_of_day: u16,
    pub timezone: String,
    pub repeat: ScheduleRepeat,
    pub weekday: Option<u8>,
    pub republish_on_update: bool,
    pub enabled: bool,
}

/// A schedule as stored: what the seller said, plus what has happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRecord {
    pub id: Uuid,
    pub name: String,
    pub selection: ScheduleSelection,
    pub inventories: Vec<InventoryId>,
    pub intent: SyncIntent,
    pub at_minute_of_day: u16,
    pub timezone: String,
    pub repeat: ScheduleRepeat,
    pub weekday: Option<u8>,
    pub republish_on_update: bool,
    pub enabled: bool,
    pub last_run_at: Option<Timestamp>,
}

/// What one tick did to one member on one marketplace, as the pass writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleOutcome {
    /// Minted into this job.
    Sent(JobId),
    /// Not sent, and the sentence the seller is shown.
    Skipped(String),
}

/// One row of a tick, as the pass hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRunWrite {
    pub product: ProductId,
    pub inventory: InventoryId,
    pub outcome: ScheduleOutcome,
}

/// One row of a tick, as the runs list reads it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRunRow {
    pub tick: Timestamp,
    pub product: ProductId,
    pub title: String,
    pub inventory: InventoryId,
    pub outcome: ScheduleOutcome,
}

/// One member of a tick: the resource, and what a refusal sentence names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleMember {
    pub product: ProductId,
    pub title: Title,
    /// The catalogue's own last write, which `republish_on_update` compares
    /// against the target mapping's.
    pub updated_at: Timestamp,
}

/// The zone name did not parse against the compiled-in database.
///
/// Its own error rather than a [`StorageError`], because it is the route's
/// validation answer and not a fault: a seller whose browser reported a zone
/// this build's database does not know is told so.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{name} is not a time zone this server knows")]
pub struct UnknownTimezone {
    pub name: String,
}

/// The zone, parsed.
///
/// # Errors
///
/// A name outside the compiled-in IANA database.
pub fn timezone_of(name: &str) -> Result<Tz, UnknownTimezone> {
    name.parse::<Tz>().map_err(|_| UnknownTimezone {
        name: name.to_owned(),
    })
}

/// How far either direction the tick search walks.
///
/// Eight days rather than seven: a weekly schedule's own weekday is at most
/// seven days away, and the extra day is the headroom a zone transition
/// straddling midnight needs.
const SEARCH_DAYS: i64 = 8;

/// The instant this schedule's minute falls at on one local date.
///
/// `None` where the zone has no such local time at all, which happens on the
/// spring-forward date for a schedule set inside the skipped hour. The answer
/// is then the same wall time an hour later -- the first instant that exists
/// after the one the seller chose -- rather than nothing: a daily schedule at
/// half past one must not silently miss the day the clocks go forward. An
/// ambiguous time, on the autumn date, takes the earlier of its two
/// instants, so a schedule fires once and on the first pass of that hour.
fn tick_on(date: NaiveDate, minute_of_day: u16, tz: Tz) -> Option<Timestamp> {
    // `div_euclid` rather than `/`: the workspace denies the integer-division
    // operator, and the euclidean pair is the one whose remainder is never
    // negative.
    let hours = u32::from(minute_of_day).div_euclid(60);
    let minutes = u32::from(minute_of_day).rem_euclid(60);
    let naive = date.and_hms_opt(hours, minutes, 0)?;
    let resolved = match tz.from_local_datetime(&naive).earliest() {
        Some(resolved) => resolved,
        None => tz
            .from_local_datetime(&naive.checked_add_signed(chrono::TimeDelta::hours(1))?)
            .earliest()?,
    };
    Some(Timestamp(resolved.timestamp_millis()))
}

/// Whether this schedule fires on this local date at all.
fn fires_on(schedule: &ScheduleRecord, date: NaiveDate) -> bool {
    match schedule.repeat {
        ScheduleRepeat::Once | ScheduleRepeat::Daily => true,
        ScheduleRepeat::Weekly => schedule
            .weekday
            .is_some_and(|weekday| u32::from(weekday) == date.weekday().num_days_from_sunday()),
    }
}

/// Every instant this schedule fires at within a day of `around`, in order.
fn ticks_around(schedule: &ScheduleRecord, around: Timestamp) -> Vec<Timestamp> {
    let Ok(tz) = timezone_of(&schedule.timezone) else {
        return Vec::new();
    };
    let Some(anchor) = Utc.timestamp_millis_opt(around.0).earliest() else {
        return Vec::new();
    };
    let today = anchor.with_timezone(&tz).date_naive();
    let mut ticks = Vec::new();
    for offset in -SEARCH_DAYS..=SEARCH_DAYS {
        let Some(date) = today.checked_add_signed(chrono::TimeDelta::days(offset)) else {
            continue;
        };
        if !fires_on(schedule, date) {
            continue;
        }
        if let Some(tick) = tick_on(date, schedule.at_minute_of_day, tz) {
            ticks.push(tick);
        }
    }
    ticks.sort_unstable_by_key(|tick| tick.0);
    ticks
}

/// The tick this schedule owes work for now, if it owes any.
///
/// The latest fired instant at or before `now` that is strictly later than
/// the last one materialised. A pass that ran late lands on the scheduled
/// instant rather than on its own, which is what makes the tick a stable
/// idempotency key; a pass that missed a day fires once for the most recent
/// tick rather than catching up on every one it slept through, because a
/// seller who left the server off overnight wants Friday's drop and not
/// Monday's again.
#[must_use]
pub fn due_tick(schedule: &ScheduleRecord, now: Timestamp) -> Option<Timestamp> {
    if !schedule.enabled {
        return None;
    }
    if schedule.repeat == ScheduleRepeat::Once && schedule.last_run_at.is_some() {
        return None;
    }
    let last = schedule.last_run_at.map_or(i64::MIN, |at| at.0);
    ticks_around(schedule, now)
        .into_iter()
        .rfind(|tick| tick.0 <= now.0 && tick.0 > last)
}

/// When this schedule fires next, for the console's own line.
///
/// `None` for a schedule that is off, and for a `once` schedule that has
/// already run: both are "never again", and the console renders an absent
/// next run rather than a date in the past.
#[must_use]
pub fn next_tick(schedule: &ScheduleRecord, now: Timestamp) -> Option<Timestamp> {
    if !schedule.enabled {
        return None;
    }
    if schedule.repeat == ScheduleRepeat::Once && schedule.last_run_at.is_some() {
        return None;
    }
    ticks_around(schedule, now)
        .into_iter()
        .find(|tick| tick.0 > now.0)
}

pub struct ScheduleRepo {
    pool: PgPool,
}

/// One member row, named rather than anonymous because both selection
/// branches read the same three columns and two inferred record types are not
/// one type.
struct MemberRow {
    id: uuid::Uuid,
    title: String,
    updated_at: chrono::DateTime<Utc>,
}

struct ScheduleRow {
    id: uuid::Uuid,
    name: String,
    selection_kind: String,
    selection_label: Option<String>,
    intent: String,
    at_minute_of_day: i32,
    timezone: String,
    repeat: String,
    weekday: Option<i16>,
    republish_on_update: bool,
    enabled: bool,
    last_run_at: Option<chrono::DateTime<Utc>>,
}

impl ScheduleRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The organisation's schedules, oldest first, each with its marketplaces
    /// and its frozen tick list.
    ///
    /// Three statements rather than one per schedule: the two child sets are
    /// read for the whole page and matched in memory, which is what
    /// `MappingRepo::heads_for_products` does for the same reason.
    pub async fn list(&self, org: OrgId) -> Result<Vec<ScheduleRecord>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            ScheduleRow,
            "SELECT id, name, selection_kind, selection_label, intent, at_minute_of_day, \
                    timezone, repeat, weekday, republish_on_update, enabled, last_run_at \
             FROM schedule WHERE org_id = $1 ORDER BY created_at, id",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let marketplaces = sqlx::query!(
            "SELECT schedule_id, inventory FROM schedule_marketplace \
             WHERE org_id = $1 ORDER BY schedule_id, inventory",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let members = sqlx::query!(
            "SELECT schedule_id, product_id FROM schedule_product \
             WHERE org_id = $1 ORDER BY schedule_id, product_id",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut schedules = Vec::with_capacity(rows.len());
        for row in rows {
            let inventories = marketplaces
                .iter()
                .filter(|entry| entry.schedule_id == row.id)
                .map(|entry| inventory_from_db(&entry.inventory))
                .collect::<Result<Vec<_>, StorageError>>()?;
            let products: Vec<ProductId> = members
                .iter()
                .filter(|entry| entry.schedule_id == row.id)
                .map(|entry| ProductId(uuid_from_db(entry.product_id)))
                .collect();
            schedules.push(record_of(row, inventories, products)?);
        }
        Ok(schedules)
    }

    /// One schedule, or nothing: another organisation's is nothing, because
    /// the pin is what the read is scoped by.
    pub async fn get(
        &self,
        org: OrgId,
        schedule: Uuid,
    ) -> Result<Option<ScheduleRecord>, StorageError> {
        Ok(self
            .list(org)
            .await?
            .into_iter()
            .find(|record| record.id == schedule))
    }

    /// Writes a schedule and its two child sets in one transaction.
    ///
    /// The name collision is the unique index's rather than a read before the
    /// write, so two tabs saving the same name cannot both be told it was
    /// free. `Ok(false)` is that collision.
    pub async fn create(
        &self,
        org: OrgId,
        id: Uuid,
        write: &ScheduleWrite,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let inserted = sqlx::query!(
            "INSERT INTO schedule (org_id, id, name, selection_kind, selection_label, intent, \
                                   at_minute_of_day, timezone, repeat, weekday, \
                                   republish_on_update, enabled, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             ON CONFLICT DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(id),
            write.name,
            selection_kind_of(&write.selection),
            selection_label_of(&write.selection),
            write.intent.as_str(),
            i32::from(write.at_minute_of_day),
            write.timezone,
            write.repeat.as_str(),
            write.weekday.map(i16::from),
            write.republish_on_update,
            write.enabled,
            timestamp_to_db(now)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        write_children(&mut tx, org, id, write).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Replaces a schedule and both child sets.
    ///
    /// A whole-value replace rather than a patch, because the route's body is
    /// the whole schedule: a field absent from a form that did not render it
    /// must not be read as a removal, and the way to keep that impossible is
    /// to have no partial write at all.
    ///
    /// `last_run_at` is deliberately untouched. A seller who moves the time
    /// has not un-run this morning's tick, and clearing it would send the
    /// day's resources a second time.
    pub async fn update(
        &self,
        org: OrgId,
        id: Uuid,
        write: &ScheduleWrite,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let updated = sqlx::query!(
            "UPDATE schedule SET name = $3, selection_kind = $4, selection_label = $5, \
                    intent = $6, at_minute_of_day = $7, timezone = $8, repeat = $9, \
                    weekday = $10, republish_on_update = $11, enabled = $12 \
             WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
            write.name,
            selection_kind_of(&write.selection),
            selection_label_of(&write.selection),
            write.intent.as_str(),
            i32::from(write.at_minute_of_day),
            write.timezone,
            write.repeat.as_str(),
            write.weekday.map(i16::from),
            write.republish_on_update,
            write.enabled,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query!(
            "DELETE FROM schedule_marketplace WHERE org_id = $1 AND schedule_id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM schedule_product WHERE org_id = $1 AND schedule_id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .execute(&mut *tx)
        .await?;
        write_children(&mut tx, org, id, write).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Deletes a schedule; its marketplaces, its tick list and its run
    /// history cascade with it.
    pub async fn delete(&self, org: OrgId, id: Uuid) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let deleted = sqlx::query!(
            "DELETE FROM schedule WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted > 0)
    }

    /// The resources one tick sends, resolved now.
    ///
    /// A label re-resolves, so a resource labelled since the last tick is
    /// included; a tick list is read back as stored. Deleted resources are
    /// excluded either way, which is the same tombstone rule every catalogue
    /// read follows.
    pub async fn members(
        &self,
        org: OrgId,
        schedule: &ScheduleRecord,
    ) -> Result<Vec<ScheduleMember>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = match &schedule.selection {
            // The catalogue page's own label predicate, case-insensitively as
            // `label_one_per_name` is, so a schedule and a filtered catalogue
            // view agree about which resources carry a label.
            ScheduleSelection::Label(label) => {
                sqlx::query_as!(
                    MemberRow,
                    "SELECT id, title, updated_at FROM product \
                     WHERE org_id = $1 AND deleted_at IS NULL \
                       AND EXISTS ( \
                             SELECT 1 FROM product_label pl \
                             JOIN label l ON l.org_id = pl.org_id AND l.id = pl.label_id \
                             WHERE pl.org_id = product.org_id AND pl.product_id = product.id \
                               AND lower(l.name) = lower($2)) \
                     ORDER BY created_at, id",
                    org_db,
                    label,
                )
                .fetch_all(&mut *tx)
                .await?
            }
            ScheduleSelection::Products(_) => {
                sqlx::query_as!(
                    MemberRow,
                    "SELECT p.id, p.title, p.updated_at FROM product p \
                     JOIN schedule_product sp \
                       ON sp.org_id = p.org_id AND sp.product_id = p.id \
                     WHERE p.org_id = $1 AND sp.schedule_id = $2 AND p.deleted_at IS NULL \
                     ORDER BY p.created_at, p.id",
                    org_db,
                    uuid_to_db(schedule.id),
                )
                .fetch_all(&mut *tx)
                .await?
            }
        };
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| ScheduleMember {
                product: ProductId(uuid_from_db(row.id)),
                title: Title(row.title),
                updated_at: timestamp_from_db(row.updated_at),
            })
            .collect())
    }

    /// Records what one tick did, and moves the schedule's last tick.
    ///
    /// One transaction, and the insert conflicts rather than checks: the
    /// primary key of `schedule_run` is `(schedule, tick, product, inventory)`,
    /// so a second pass in the same minute writes nothing. `last_run_at` is
    /// set to the tick rather than to the pass's own instant for the same
    /// reason the key is the tick.
    #[expect(
        clippy::too_many_arguments,
        reason = "the rows are addressed by tenant, schedule and tick, and the write carries \
                  its own rows and its instant; a struct over those five would name the call"
    )]
    pub async fn record_run(
        &self,
        org: OrgId,
        schedule: Uuid,
        tick: Timestamp,
        rows: &[ScheduleRunWrite],
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let org_db = uuid_to_db(org.0);
        let schedule_db = uuid_to_db(schedule);
        let tick_db = timestamp_to_db(tick)?;
        let recorded_at = timestamp_to_db(now)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        for row in rows {
            let (state, job, reason) = match &row.outcome {
                ScheduleOutcome::Sent(job) => ("sent", Some(uuid_to_db(job.0)), None),
                ScheduleOutcome::Skipped(reason) => ("skipped", None, Some(reason.as_str())),
            };
            sqlx::query!(
                "INSERT INTO schedule_run (org_id, schedule_id, tick, product_id, inventory, \
                                           state, job_id, reason, recorded_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT DO NOTHING",
                org_db,
                schedule_db,
                tick_db,
                uuid_to_db(row.product.0),
                inventory_to_db(row.inventory),
                state,
                job,
                reason,
                recorded_at,
            )
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query!(
            "UPDATE schedule SET last_run_at = $3 \
             WHERE org_id = $1 AND id = $2 \
               AND (last_run_at IS NULL OR last_run_at < $3)",
            org_db,
            schedule_db,
            tick_db,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether this tick has already been materialised for this marketplace.
    ///
    /// Read before the mint rather than relying on the insert alone, because
    /// the job is minted first: without this a pass repeating a tick would
    /// create a second job whose `schedule_run` rows then conflicted away,
    /// leaving a job nothing points at. The job's own request key would make
    /// it a replay rather than a duplicate, so this is the cheaper answer and
    /// not the only fence.
    pub async fn tick_recorded(
        &self,
        org: OrgId,
        schedule: Uuid,
        tick: Timestamp,
        inventory: InventoryId,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM schedule_run \
             WHERE org_id = $1 AND schedule_id = $2 AND tick = $3 AND inventory = $4)",
            uuid_to_db(org.0),
            uuid_to_db(schedule),
            timestamp_to_db(tick)?,
            inventory_to_db(inventory),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(found.unwrap_or(false))
    }

    /// One schedule's runs, newest tick first, with each member's title.
    pub async fn runs(
        &self,
        org: OrgId,
        schedule: Uuid,
    ) -> Result<Vec<ScheduleRunRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT r.tick, r.product_id, r.inventory, r.state, r.job_id, r.reason, \
                    COALESCE(p.title, '') AS \"title!\" \
             FROM schedule_run r \
             LEFT JOIN product p ON p.org_id = r.org_id AND p.id = r.product_id \
             WHERE r.org_id = $1 AND r.schedule_id = $2 \
             ORDER BY r.tick DESC, r.inventory, p.title LIMIT $3",
            uuid_to_db(org.0),
            uuid_to_db(schedule),
            SCHEDULE_RUNS_LISTED_MAX,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let outcome = match (row.state.as_str(), row.job_id, row.reason) {
                    ("sent", Some(job), _) => ScheduleOutcome::Sent(JobId(uuid_from_db(job))),
                    ("skipped", _, Some(reason)) => ScheduleOutcome::Skipped(reason),
                    (state, _, _) => {
                        return Err(StorageError::CorruptRow {
                            reason: format!("schedule run state {state:?} names neither leg"),
                        })
                    }
                };
                Ok(ScheduleRunRow {
                    tick: timestamp_from_db(row.tick),
                    product: ProductId(uuid_from_db(row.product_id)),
                    title: row.title,
                    inventory: inventory_from_db(&row.inventory)?,
                    outcome,
                })
            })
            .collect()
    }
}

const fn selection_kind_of(selection: &ScheduleSelection) -> &'static str {
    match selection {
        ScheduleSelection::Label(_) => "label",
        ScheduleSelection::Products(_) => "products",
    }
}

fn selection_label_of(selection: &ScheduleSelection) -> Option<&str> {
    match selection {
        ScheduleSelection::Label(label) => Some(label.as_str()),
        ScheduleSelection::Products(_) => None,
    }
}

async fn write_children(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    id: Uuid,
    write: &ScheduleWrite,
) -> Result<(), StorageError> {
    for inventory in &write.inventories {
        sqlx::query!(
            "INSERT INTO schedule_marketplace (org_id, schedule_id, inventory) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(id),
            inventory_to_db(*inventory),
        )
        .execute(&mut **tx)
        .await?;
    }
    if let ScheduleSelection::Products(products) = &write.selection {
        for product in products {
            sqlx::query!(
                "INSERT INTO schedule_product (org_id, schedule_id, product_id) \
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                uuid_to_db(org.0),
                uuid_to_db(id),
                uuid_to_db(product.0),
            )
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

fn record_of(
    row: ScheduleRow,
    inventories: Vec<InventoryId>,
    products: Vec<ProductId>,
) -> Result<ScheduleRecord, StorageError> {
    let selection = match (row.selection_kind.as_str(), row.selection_label) {
        ("label", Some(label)) => ScheduleSelection::Label(label),
        ("products", None) => ScheduleSelection::Products(products),
        (kind, _) => {
            return Err(StorageError::CorruptRow {
                reason: format!("schedule selection {kind:?} does not match its label column"),
            })
        }
    };
    Ok(ScheduleRecord {
        id: uuid_from_db(row.id),
        name: row.name,
        selection,
        inventories,
        intent: match row.intent.as_str() {
            "draft" => SyncIntent::Draft,
            "live" => SyncIntent::Live,
            other => {
                return Err(StorageError::CorruptRow {
                    reason: format!("unknown schedule intent {other:?}"),
                })
            }
        },
        at_minute_of_day: u16::try_from(row.at_minute_of_day).unwrap_or(0),
        timezone: row.timezone,
        repeat: ScheduleRepeat::from_column(&row.repeat)?,
        weekday: row.weekday.and_then(|weekday| u8::try_from(weekday).ok()),
        republish_on_update: row.republish_on_update,
        enabled: row.enabled,
        last_run_at: row.last_run_at.map(timestamp_from_db),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        due_tick, next_tick, ScheduleRecord, ScheduleRepeat, ScheduleSelection, SyncIntent,
    };
    use tam_types::{Timestamp, Uuid};

    /// 2026-09-11 is a Friday; 12:00 UTC that day.
    const FRIDAY_NOON: Timestamp = Timestamp(1_789_128_000_000);

    fn schedule(repeat: ScheduleRepeat, minute: u16, timezone: &str) -> ScheduleRecord {
        ScheduleRecord {
            id: Uuid([0x11; 16]),
            name: "Friday drop".to_owned(),
            selection: ScheduleSelection::Label("Ready".to_owned()),
            inventories: vec![],
            intent: SyncIntent::Live,
            at_minute_of_day: minute,
            timezone: timezone.to_owned(),
            repeat,
            weekday: match repeat {
                ScheduleRepeat::Weekly => Some(5),
                ScheduleRepeat::Once | ScheduleRepeat::Daily => None,
            },
            republish_on_update: false,
            enabled: true,
            last_run_at: None,
        }
    }

    /// A minute already past today is owed now, and the tick is the scheduled
    /// instant rather than the pass's.
    ///
    /// The instant matters more than the fact: it is the idempotency key, so
    /// a pass forty seconds late and one on time have to agree on it.
    #[test]
    fn a_daily_schedule_owes_the_minute_that_has_already_passed_today() {
        let daily = schedule(ScheduleRepeat::Daily, 9 * 60, "UTC");
        let tick = due_tick(&daily, FRIDAY_NOON).expect("nine has passed by noon");
        assert_eq!(
            tick,
            Timestamp(FRIDAY_NOON.0 - 3 * 60 * 60 * 1_000),
            "the tick is nine o'clock, not noon"
        );
    }

    /// The same tick is owed once. Recording it is what stops the second
    /// pass, and this is the arithmetic half of that: with `last_run_at` at
    /// the tick, nothing is due until tomorrow.
    #[test]
    fn a_recorded_tick_is_not_owed_again_and_the_next_one_is_tomorrow() {
        let mut daily = schedule(ScheduleRepeat::Daily, 9 * 60, "UTC");
        daily.last_run_at = Some(Timestamp(FRIDAY_NOON.0 - 3 * 60 * 60 * 1_000));
        assert_eq!(due_tick(&daily, FRIDAY_NOON), None);
        assert_eq!(
            next_tick(&daily, FRIDAY_NOON),
            Some(Timestamp(FRIDAY_NOON.0 + 21 * 60 * 60 * 1_000)),
            "nine tomorrow"
        );
    }

    /// A weekly schedule fires on its own weekday and not on the six others.
    #[test]
    fn a_weekly_schedule_owes_its_own_weekday_and_the_next_is_seven_days_on() {
        let weekly = schedule(ScheduleRepeat::Weekly, 9 * 60, "UTC");
        assert!(
            due_tick(&weekly, FRIDAY_NOON).is_some(),
            "Friday's nine has passed"
        );
        let mut ran = weekly.clone();
        ran.last_run_at = due_tick(&weekly, FRIDAY_NOON);
        assert_eq!(
            next_tick(&ran, FRIDAY_NOON),
            Some(Timestamp(FRIDAY_NOON.0 + (7 * 24 - 3) * 60 * 60 * 1_000)),
            "next Friday, not tomorrow"
        );
    }

    /// The zone is the seller's, not the server's: nine in New York is one in
    /// the afternoon UTC in September, so at noon UTC today's has not happened
    /// and the last one that did was yesterday's.
    ///
    /// This is the whole reason the column is a zone name and not an offset.
    #[test]
    fn the_minute_is_read_in_the_sellers_own_zone() {
        let daily = schedule(ScheduleRepeat::Daily, 9 * 60, "America/New_York");
        assert_eq!(
            due_tick(&daily, FRIDAY_NOON),
            Some(Timestamp(FRIDAY_NOON.0 - 23 * 60 * 60 * 1_000)),
            "yesterday's nine in New York, which is the last one that passed"
        );
        assert_eq!(
            next_tick(&daily, FRIDAY_NOON),
            Some(Timestamp(FRIDAY_NOON.0 + 60 * 60 * 1_000)),
            "today's nine in New York is still an hour away"
        );
    }

    /// A `once` schedule fires at the next minute it is owed and never again,
    /// and the record of that firing is what says so.
    #[test]
    fn a_once_schedule_is_owed_exactly_one_tick() {
        let once = schedule(ScheduleRepeat::Once, 9 * 60, "UTC");
        let tick = due_tick(&once, FRIDAY_NOON).expect("nine has passed");
        let mut ran = once;
        ran.last_run_at = Some(tick);
        assert_eq!(due_tick(&ran, FRIDAY_NOON), None);
        assert_eq!(next_tick(&ran, FRIDAY_NOON), None);
    }

    /// A schedule that is off owes nothing and promises nothing, rather than
    /// accumulating ticks it would fire on being switched back on.
    #[test]
    fn a_disabled_schedule_owes_nothing() {
        let mut daily = schedule(ScheduleRepeat::Daily, 9 * 60, "UTC");
        daily.enabled = false;
        assert_eq!(due_tick(&daily, FRIDAY_NOON), None);
        assert_eq!(next_tick(&daily, FRIDAY_NOON), None);
    }

    /// A zone this build's database does not know fires nothing rather than
    /// falling back to UTC, which would publish at the wrong hour without
    /// anyone being told.
    #[test]
    fn an_unknown_zone_fires_nothing() {
        let daily = schedule(ScheduleRepeat::Daily, 9 * 60, "Mars/Olympus_Mons");
        assert_eq!(due_tick(&daily, FRIDAY_NOON), None);
        assert_eq!(next_tick(&daily, FRIDAY_NOON), None);
    }
}
