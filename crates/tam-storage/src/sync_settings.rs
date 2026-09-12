//! The inbound clock and what it publishes: how often one shop is re-read,
//! where a pulled resource goes afterwards, and the two reads the Marketplace
//! Sync page draws from both.
//!
//! The activity log and the multi-listed list live here rather than beside
//! the tables they read, and deliberately: both are page reads over three
//! unrelated tables, and the page is this one. Putting the activity join on
//! `ImportRunRepo` would have made an import's repository answer questions
//! about jobs and schedules, which is how a repository stops being about one
//! thing.
//!
//! Nothing here decides *when*. A setting is an interval and a last pull; the
//! comparison against now is the pass's, in one place, so the schedule clock
//! and the sync clock are read by one caller and cannot disagree about the
//! instant.

use sqlx::PgPool;
use tam_types::{InventoryId, JobId, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    inventory_from_db, inventory_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db,
    uuid_to_db,
};
use crate::{pin_org, StorageError};

/// How many activity lines one page answers.
pub const ACTIVITY_LISTED_MAX: i64 = 50;

/// One shop's inbound setting, with the rules that publish what it brings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSettingRecord {
    pub inventory: InventoryId,
    pub enabled: bool,
    pub interval_secs: u32,
    pub last_pull_at: Option<Timestamp>,
    /// Where a resource this shop brings in is published, as the seller's own
    /// rules name it.
    pub publish_to: Vec<InventoryId>,
    /// The template a rule fills a pulled resource from, or `None` where the
    /// seller has named none.
    ///
    /// One per source rather than one per rule, because the control is one
    /// picker on the shop's card: `auto_publish_rule` carries the column on
    /// every row of the set and this read takes the first, so the finer grain
    /// is available to a later control without a migration.
    pub template: Option<Uuid>,
}

impl SyncSettingRecord {
    /// Whether the pass owes this shop a read.
    ///
    /// A setting that has never pulled is due immediately, which is what
    /// `last_pull_at IS NULL` means rather than the epoch: a seller who has
    /// just switched this on gets their shop read on the next pass rather
    /// than in six hours.
    #[must_use]
    pub fn due(&self, now: Timestamp) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(last) = self.last_pull_at else {
            return true;
        };
        let interval_ms = i64::from(self.interval_secs).saturating_mul(1_000);
        last.0.saturating_add(interval_ms) <= now.0
    }
}

/// One line of the activity log, before it is worded.
///
/// Structured rather than a sentence, because the sentence is the API's: the
/// marketplace's seller-facing name lives beside the rest of the copy and a
/// storage layer composing English would be a second place it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityKind {
    /// A resource a pull brought into the catalogue.
    Pulled {
        run: Uuid,
        product: ProductId,
        title: String,
        source: InventoryId,
    },
    /// A resource an auto-publish rule sent on, once its job settled.
    Published {
        job: JobId,
        product: ProductId,
        title: String,
        target: InventoryId,
    },
    /// One schedule's tick, on one marketplace.
    ScheduleTick {
        schedule: Uuid,
        name: String,
        inventory: InventoryId,
        sent: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    pub at: Timestamp,
    pub kind: ActivityKind,
}

/// One resource that is live on more than one marketplace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiListedRow {
    pub product: ProductId,
    pub title: String,
    pub inventories: Vec<InventoryId>,
}

pub struct SyncSettingRepo {
    pool: PgPool,
}

impl SyncSettingRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every setting this organisation has written, with its rules.
    ///
    /// A marketplace with no row is absent rather than defaulted here: the
    /// route pads the list from the connection set, because "which shops
    /// could be synced" is a question about connections and not about this
    /// table.
    pub async fn list(&self, org: OrgId) -> Result<Vec<SyncSettingRecord>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT inventory, enabled, interval_secs, last_pull_at \
             FROM marketplace_sync_setting WHERE org_id = $1 ORDER BY inventory",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let rules = sqlx::query!(
            "SELECT source_inventory, target_inventory, template_id FROM auto_publish_rule \
             WHERE org_id = $1 ORDER BY source_inventory, target_inventory",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                let inventory = inventory_from_db(&row.inventory)?;
                let mut mine = rules
                    .iter()
                    .filter(|rule| rule.source_inventory == row.inventory);
                let publish_to = mine
                    .clone()
                    .map(|rule| inventory_from_db(&rule.target_inventory))
                    .collect::<Result<Vec<_>, StorageError>>()?;
                Ok(SyncSettingRecord {
                    inventory,
                    enabled: row.enabled,
                    interval_secs: u32::try_from(row.interval_secs).unwrap_or(u32::MAX),
                    last_pull_at: row.last_pull_at.map(timestamp_from_db),
                    publish_to,
                    template: mine.find_map(|rule| rule.template_id).map(uuid_from_db),
                })
            })
            .collect()
    }

    /// Writes one shop's setting and replaces its rule set.
    ///
    /// A replace rather than an add-and-remove pair, for `set_for_product`'s
    /// reason: the control is a set of ticks and the body is the whole set,
    /// so a rule the seller unticked is one this statement no longer writes.
    /// `last_pull_at` is untouched -- changing the interval does not mean the
    /// shop was just read.
    ///
    /// The template is written on every rule row of the source's set, because
    /// the control that names it is one picker on the shop's card: the column
    /// admits a template per target and no surface states one yet.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is addressed by tenant and marketplace and carries its three stored \
                  values and the template its rules fill from; a struct over those six would \
                  name the call"
    )]
    pub async fn upsert(
        &self,
        org: OrgId,
        inventory: InventoryId,
        enabled: bool,
        interval_secs: u32,
        publish_to: &[InventoryId],
        template: Option<Uuid>,
    ) -> Result<(), StorageError> {
        let org_db = uuid_to_db(org.0);
        let inventory_db = inventory_to_db(inventory);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO marketplace_sync_setting \
                 (org_id, inventory, enabled, interval_secs) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (org_id, inventory) DO UPDATE \
                 SET enabled = $3, interval_secs = $4",
            org_db,
            inventory_db,
            enabled,
            i32::try_from(interval_secs).unwrap_or(i32::MAX),
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM auto_publish_rule WHERE org_id = $1 AND source_inventory = $2",
            org_db,
            inventory_db,
        )
        .execute(&mut *tx)
        .await?;
        for target in publish_to {
            sqlx::query!(
                "INSERT INTO auto_publish_rule \
                     (org_id, source_inventory, target_inventory, template_id) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                org_db,
                inventory_db,
                inventory_to_db(*target),
                template.map(uuid_to_db),
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The pass minted a run for this shop.
    ///
    /// Set when the run is opened rather than when it finishes, because the
    /// interval measures how often we ask the seller's machine to read and
    /// not how long their machine took: a shop whose read takes an hour must
    /// not then be asked again the minute it lands.
    pub async fn mark_pulled(
        &self,
        org: OrgId,
        inventory: InventoryId,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        sqlx::query!(
            "UPDATE marketplace_sync_setting SET last_pull_at = $3 \
             WHERE org_id = $1 AND inventory = $2",
            uuid_to_db(org.0),
            inventory_to_db(inventory),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records that a rule published one resource, and answers whether this
    /// call is the one that did it.
    ///
    /// The primary key is `(run, product, target)`, which is the idempotency
    /// the design states: `false` is a row that already existed, so the
    /// caller minted a job it must not mint again. Written after the job so
    /// the row never names one that does not exist.
    #[expect(
        clippy::too_many_arguments,
        reason = "the row is its own primary key -- tenant, run, resource and target -- plus \
                  the job it names and the instant; a struct over those six would name the call"
    )]
    pub async fn record_auto_publish(
        &self,
        org: OrgId,
        run: Uuid,
        product: ProductId,
        target: InventoryId,
        job: JobId,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO auto_publish_run \
                 (org_id, run_id, product_id, target_inventory, job_id, recorded_at) \
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(run),
            uuid_to_db(product.0),
            inventory_to_db(target),
            uuid_to_db(job.0),
            timestamp_to_db(now)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(written > 0)
    }

    /// Whether a rule has already published this resource out of this run.
    pub async fn auto_published(
        &self,
        org: OrgId,
        run: Uuid,
        product: ProductId,
        target: InventoryId,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let found = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM auto_publish_run \
             WHERE org_id = $1 AND run_id = $2 AND product_id = $3 AND target_inventory = $4)",
            uuid_to_db(org.0),
            uuid_to_db(run),
            uuid_to_db(product.0),
            inventory_to_db(target),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(found.unwrap_or(false))
    }

    /// One page of the activity log, newest first, strictly older than the
    /// cursor.
    ///
    /// Three reads merged in memory rather than one `UNION ALL`: each of the
    /// three has its own join and its own instant column, and a union would
    /// have to cast all three into one row shape and then be decoded back
    /// out. Each read takes the same limit, so the merge has at least `limit`
    /// candidates from every source and the truncation is correct.
    pub async fn activity(
        &self,
        org: OrgId,
        cursor: Option<Timestamp>,
        limit: i64,
    ) -> Result<Vec<ActivityRow>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let before = cursor.map(timestamp_to_db).transpose()?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let pulled = sqlx::query!(
            "SELECT i.run_id, i.product_id, i.settled_at, r.source, \
                    COALESCE(p.title, '') AS \"title!\" \
             FROM import_run_item i \
             JOIN import_run r ON r.org_id = i.org_id AND r.id = i.run_id \
             LEFT JOIN product p ON p.org_id = i.org_id AND p.id = i.product_id \
             WHERE i.org_id = $1 AND i.state = 'imported' AND i.settled_at IS NOT NULL \
               AND i.product_id IS NOT NULL AND r.source IS NOT NULL \
               AND ($2::timestamptz IS NULL OR i.settled_at < $2) \
             ORDER BY i.settled_at DESC LIMIT $3",
            org_db,
            before,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        let published = sqlx::query!(
            "SELECT a.product_id, a.target_inventory, a.job_id, \
                    COALESCE(p.title, '') AS \"title!\", \
                    MAX(i.settled_at) AS \"settled_at!\" \
             FROM auto_publish_run a \
             JOIN job_item i ON i.org_id = a.org_id AND i.job_id = a.job_id \
             LEFT JOIN product p ON p.org_id = a.org_id AND p.id = a.product_id \
             WHERE a.org_id = $1 \
             GROUP BY a.product_id, a.target_inventory, a.job_id, p.title \
             HAVING COUNT(*) FILTER (WHERE i.settled_at IS NULL) = 0 \
                AND ($2::timestamptz IS NULL OR MAX(i.settled_at) < $2) \
             ORDER BY MAX(i.settled_at) DESC LIMIT $3",
            org_db,
            before,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        let ticks = sqlx::query!(
            "SELECT r.schedule_id, r.tick, r.inventory, s.name, \
                    COUNT(*) FILTER (WHERE r.state = 'sent') AS \"sent!\" \
             FROM schedule_run r \
             JOIN schedule s ON s.org_id = r.org_id AND s.id = r.schedule_id \
             WHERE r.org_id = $1 AND ($2::timestamptz IS NULL OR r.tick < $2) \
             GROUP BY r.schedule_id, r.tick, r.inventory, s.name \
             ORDER BY r.tick DESC LIMIT $3",
            org_db,
            before,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let mut lines: Vec<ActivityRow> = Vec::new();
        for row in pulled {
            let (Some(at), Some(product), Some(source)) =
                (row.settled_at, row.product_id, row.source.as_deref())
            else {
                continue;
            };
            lines.push(ActivityRow {
                at: timestamp_from_db(at),
                kind: ActivityKind::Pulled {
                    run: uuid_from_db(row.run_id),
                    product: ProductId(uuid_from_db(product)),
                    title: row.title,
                    source: inventory_from_db(source)?,
                },
            });
        }
        for row in published {
            lines.push(ActivityRow {
                at: timestamp_from_db(row.settled_at),
                kind: ActivityKind::Published {
                    job: JobId(uuid_from_db(row.job_id)),
                    product: ProductId(uuid_from_db(row.product_id)),
                    title: row.title,
                    target: inventory_from_db(&row.target_inventory)?,
                },
            });
        }
        for row in ticks {
            lines.push(ActivityRow {
                at: timestamp_from_db(row.tick),
                kind: ActivityKind::ScheduleTick {
                    schedule: uuid_from_db(row.schedule_id),
                    name: row.name,
                    inventory: inventory_from_db(&row.inventory)?,
                    sent: u32::try_from(row.sent).unwrap_or(u32::MAX),
                },
            });
        }
        lines.sort_unstable_by_key(|line| core::cmp::Reverse(line.at.0));
        lines.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(lines)
    }

    /// The resources that are live on more than one marketplace.
    ///
    /// Bound mappings only, which is what "listed" means everywhere else on
    /// this surface: an unbound or severed mapping is a marketplace the
    /// seller added and nothing has written yet, and counting it would tell
    /// them a resource is in two shops when it is in one.
    pub async fn multi_listed(&self, org: OrgId) -> Result<Vec<MultiListedRow>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT m.product_id, m.inventory, p.title \
             FROM mapping m \
             JOIN product p ON p.org_id = m.org_id AND p.id = m.product_id \
             WHERE m.org_id = $1 AND m.binding_state = 'bound' AND p.deleted_at IS NULL \
             ORDER BY p.title, p.id, m.inventory",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut listed: Vec<MultiListedRow> = Vec::new();
        for row in rows {
            let product = ProductId(uuid_from_db(row.product_id));
            let inventory = inventory_from_db(&row.inventory)?;
            match listed.last_mut() {
                Some(last) if last.product == product => last.inventories.push(inventory),
                _ => listed.push(MultiListedRow {
                    product,
                    title: row.title,
                    inventories: vec![inventory],
                }),
            }
        }
        listed.retain(|row| row.inventories.len() > 1);
        Ok(listed)
    }
}

#[cfg(test)]
mod tests {
    use super::{SyncSettingRecord, Timestamp};
    use tam_types::InventoryId;

    const NOW: Timestamp = Timestamp(1_789_128_000_000);
    const SIX_HOURS: u32 = 6 * 3_600;

    fn setting(enabled: bool, last_pull_at: Option<Timestamp>) -> SyncSettingRecord {
        SyncSettingRecord {
            inventory: InventoryId::Tpt,
            enabled,
            interval_secs: SIX_HOURS,
            last_pull_at,
            publish_to: vec![],
            template: None,
        }
    }

    /// A setting just switched on is due now, not in six hours: the seller
    /// pressed the switch to have their shop read.
    #[test]
    fn a_setting_that_has_never_pulled_is_due() {
        assert!(setting(true, None).due(NOW));
    }

    /// The interval is measured from the last pull, and a shop read inside it
    /// is not read again.
    #[test]
    fn the_interval_is_measured_from_the_last_pull() {
        let one_hour_ago = Timestamp(NOW.0 - 3_600 * 1_000);
        assert!(
            !setting(true, Some(one_hour_ago)).due(NOW),
            "five hours of the six are left"
        );
        let seven_hours_ago = Timestamp(NOW.0 - 7 * 3_600 * 1_000);
        assert!(setting(true, Some(seven_hours_ago)).due(NOW));
    }

    /// A shop the seller switched off is never due, whatever its interval and
    /// however long ago it was last read.
    #[test]
    fn a_disabled_setting_is_never_due() {
        assert!(!setting(false, None).due(NOW));
        assert!(!setting(false, Some(Timestamp(0))).due(NOW));
    }
}
