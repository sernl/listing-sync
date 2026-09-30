//! The seller's inbox, and who wants it by mail.
//!
//! Two sources fill it. A finished run's row is written in the transaction
//! that settles the run, so the console fills whether or not a mail relay is
//! configured and whether or not the seller takes email; the outbox row beside
//! it is only the email channel. A notice is a toast the console showed and
//! the seller never looked at, posted back so the bell can say what they
//! missed; it belongs to that seller alone.
//!
//! Every read and write of the inbox pins the organisation and the user
//! (`pin_reader`), because the table's policy fences a notice on
//! `app.current_user` as well as `app.current_org`. A path that pins no user
//! sees the organisation's run rows and nothing else.
//!
//! Nothing here holds an address. `app_user.email` carries
//! `{subject}@subject.invalid` for every self-serve signup, and the real one is
//! the identity service's; what this module answers is `auth_subject`, which
//! the drainer resolves against that service for the length of one send.

use sqlx::{PgPool, Postgres, Transaction};
use tam_types::{
    InventoryId, JobSettledNotice, NoticeTone, NotificationCounts, NotificationKind, OrgId,
    Timestamp, UserId, Uuid,
};

use crate::codec::{
    inventory_from_db, inventory_to_db, marketplace_to_db, timestamp_from_db, timestamp_to_db,
    uuid_from_db, uuid_to_db,
};
use crate::jobs::{NewOutboxMessage, OutboxRepo};
use crate::{pin_org, StorageError};

/// The topic the completion mail is drained from, which the design's topic
/// table at `docs/design/schema.md` already names.
pub const JOB_SETTLED_TOPIC: &str = "email.job_settled";

/// What a row is about: a finished run, or a notice the console posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationSource {
    Run {
        kind: NotificationKind,
        /// The `sync_request` or the `import_batch` the run belongs to.
        subject: Uuid,
        /// The inventory written to, and null for an import. The marketplace
        /// is derived from it rather than stored beside it in this record, so
        /// the two cannot disagree in a reader's hands.
        inventory: Option<InventoryId>,
        counts: NotificationCounts,
    },
    Notice {
        tone: NoticeTone,
        title: String,
        body: String,
    },
}

impl NotificationSource {
    /// The tone the row reads in: a run's worst outcome, or the notice's own.
    #[must_use]
    pub const fn tone(&self) -> NoticeTone {
        match self {
            Self::Run { counts, .. } => counts.tone(),
            Self::Notice { tone, .. } => *tone,
        }
    }
}

/// One inbox row as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRecord {
    pub id: Uuid,
    pub source: NotificationSource,
    pub created_at: Timestamp,
    pub read_at: Option<Timestamp>,
}

/// A toast the seller did not look at, as the console posts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNotice {
    /// The console's own id for the toast; a second post of it is the first.
    pub client_id: String,
    pub tone: NoticeTone,
    pub title: String,
    pub body: String,
}

/// The keyset the list pages on, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationCursor {
    pub created_at: Timestamp,
    pub id: Uuid,
}

/// Who in an organisation wants the mail, and how the identity service knows
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipient {
    pub user: UserId,
    pub subject: Uuid,
}

/// One row as the list query returns it, before it is checked into a record.
struct NotificationRow {
    id: uuid::Uuid,
    kind: String,
    subject_id: Option<uuid::Uuid>,
    inventory: Option<String>,
    counts: Option<serde_json::Value>,
    tone: Option<String>,
    title: Option<String>,
    body: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    read_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl NotificationRow {
    fn record(self) -> Result<NotificationRecord, StorageError> {
        let source = if self.kind == NOTICE_KIND {
            let (Some(tone), Some(title), Some(body)) = (self.tone, self.title, self.body) else {
                return Err(StorageError::CorruptRow {
                    reason: "a notice without its tone, title or body".to_owned(),
                });
            };
            NotificationSource::Notice {
                tone: tone_from_db(&tone)?,
                title,
                body,
            }
        } else {
            let (Some(subject), Some(counts)) = (self.subject_id, self.counts) else {
                return Err(StorageError::CorruptRow {
                    reason: "a finished run without its subject or counts".to_owned(),
                });
            };
            NotificationSource::Run {
                kind: kind_from_db(&self.kind)?,
                subject: uuid_from_db(subject),
                inventory: self
                    .inventory
                    .as_deref()
                    .map(inventory_from_db)
                    .transpose()?,
                counts: counts_from_db(counts)?,
            }
        };
        Ok(NotificationRecord {
            id: uuid_from_db(self.id),
            source,
            created_at: timestamp_from_db(self.created_at),
            read_at: self.read_at.map(timestamp_from_db),
        })
    }
}

/// The `kind` a notice row carries.
const NOTICE_KIND: &str = "notice";

/// Pins the organisation and the user the inbox policy fences on, in the
/// transaction every statement below runs in.
async fn pin_reader(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    user: UserId,
) -> Result<(), StorageError> {
    pin_org(tx, org).await?;
    let user_text = uuid_to_db(user.0).to_string();
    sqlx::query!("SELECT set_config('app.current_user', $1, true)", user_text)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

pub struct NotificationRepo {
    pool: PgPool,
}

impl NotificationRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// What this user may read of the organisation's inbox, newest first:
    /// every finished run, and their own notices. A dismissed row is gone.
    pub async fn list(
        &self,
        org: OrgId,
        user: UserId,
        cursor: Option<NotificationCursor>,
        limit: i64,
    ) -> Result<Vec<NotificationRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let (after_at, after_id) = match cursor {
            Some(cursor) => (
                Some(timestamp_to_db(cursor.created_at)?),
                Some(uuid_to_db(cursor.id)),
            ),
            None => (None, None),
        };
        let rows = sqlx::query_as!(
            NotificationRow,
            "SELECT id, kind, subject_id, inventory, counts, tone, title, body, \
                    created_at, read_at \
               FROM notification \
              WHERE org_id = $1 AND dismissed_at IS NULL \
                AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3)) \
              ORDER BY created_at DESC, id DESC \
              LIMIT $4",
            uuid_to_db(org.0),
            after_at,
            after_id,
            limit,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter().map(NotificationRow::record).collect()
    }

    /// How many of the rows `list` would return are unread.
    pub async fn unread(&self, org: OrgId, user: UserId) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let count = sqlx::query_scalar!(
            "SELECT count(*) AS \"count!\" FROM notification \
              WHERE org_id = $1 AND read_at IS NULL AND dismissed_at IS NULL",
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// Marks every unread row at or before the named one read, in the same
    /// order the list pages in.
    ///
    /// `None` where this reader holds no such row, which is the answer an
    /// unknown id and another tenant's id both get.
    pub async fn mark_read_through(
        &self,
        org: OrgId,
        user: UserId,
        through: Uuid,
        at: Timestamp,
    ) -> Result<Option<u64>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let named = sqlx::query!(
            "SELECT created_at FROM notification WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(through),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(named) = named else {
            tx.commit().await?;
            return Ok(None);
        };
        let marked = sqlx::query!(
            "UPDATE notification SET read_at = $4 \
              WHERE org_id = $1 AND read_at IS NULL \
                AND (created_at, id) <= ($2, $3)",
            uuid_to_db(org.0),
            named.created_at,
            uuid_to_db(through),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(marked.rows_affected()))
    }

    /// Marks one row read. `None` where this reader holds no such row, or
    /// only a dismissed one; `Some(false)` where it was read already.
    pub async fn mark_read(
        &self,
        org: OrgId,
        user: UserId,
        id: Uuid,
        at: Timestamp,
    ) -> Result<Option<bool>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let row = sqlx::query!(
            "SELECT read_at FROM notification \
              WHERE org_id = $1 AND id = $2 AND dismissed_at IS NULL \
                FOR UPDATE",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        if row.read_at.is_some() {
            tx.commit().await?;
            return Ok(Some(false));
        }
        sqlx::query!(
            "UPDATE notification SET read_at = $3 WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(true))
    }

    /// Marks everything this reader has unread, read.
    pub async fn mark_all_read(
        &self,
        org: OrgId,
        user: UserId,
        at: Timestamp,
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let marked = sqlx::query!(
            "UPDATE notification SET read_at = $2 \
              WHERE org_id = $1 AND read_at IS NULL AND dismissed_at IS NULL",
            uuid_to_db(org.0),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(marked.rows_affected())
    }

    /// Takes one row out of the list. Idempotent: a row dismissed already
    /// answers as dismissed. `false` where this reader holds no such row.
    pub async fn dismiss(
        &self,
        org: OrgId,
        user: UserId,
        id: Uuid,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let touched = sqlx::query!(
            "UPDATE notification SET dismissed_at = coalesce(dismissed_at, $3) \
              WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(touched.rows_affected() > 0)
    }

    /// Takes every read row out of the list, and answers how many went.
    pub async fn dismiss_read(
        &self,
        org: OrgId,
        user: UserId,
        at: Timestamp,
    ) -> Result<u64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        let gone = sqlx::query!(
            "UPDATE notification SET dismissed_at = $2 \
              WHERE org_id = $1 AND read_at IS NOT NULL AND dismissed_at IS NULL",
            uuid_to_db(org.0),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(gone.rows_affected())
    }

    /// Keeps a toast this user did not look at. A second post of the same
    /// `client_id` writes nothing and answers the row the first one wrote,
    /// so two tabs, or a retry after a timeout, leave one notice.
    pub async fn post_notice(
        &self,
        org: OrgId,
        user: UserId,
        notice: &NewNotice,
        at: Timestamp,
    ) -> Result<NotificationRecord, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_reader(&mut tx, org, user).await?;
        sqlx::query!(
            "INSERT INTO notification \
             (org_id, id, kind, user_id, tone, title, body, client_id, created_at) \
             VALUES ($1, $2, 'notice', $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (org_id, user_id, client_id) WHERE client_id IS NOT NULL DO NOTHING",
            uuid_to_db(org.0),
            uuid::Uuid::new_v4(),
            uuid_to_db(user.0),
            tone_to_db(notice.tone),
            notice.title,
            notice.body,
            notice.client_id,
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query_as!(
            NotificationRow,
            "SELECT id, kind, subject_id, inventory, counts, tone, title, body, \
                    created_at, read_at \
               FROM notification \
              WHERE org_id = $1 AND user_id = $2 AND client_id = $3",
            uuid_to_db(org.0),
            uuid_to_db(user.0),
            notice.client_id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        row.record()
    }

    /// Whether this user wants the mail. `None` where this organisation holds
    /// no such user.
    pub async fn notify_email(
        &self,
        org: OrgId,
        user: UserId,
    ) -> Result<Option<bool>, StorageError> {
        let row = sqlx::query!(
            "SELECT notify_email FROM app_user WHERE id = $1 AND org_id = $2",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| row.notify_email))
    }

    /// Sets it, for one user of one organisation and nobody else.
    ///
    /// The organisation is in the predicate as well as the user, and it is not
    /// redundant. `app_user` is classified global in `rls_matrix.rs`, so it
    /// carries no row-level policy to fall back on: the whole of the fence on
    /// this write is the predicate here. Today nothing in a request can name
    /// another user -- the id comes from the resolved session and the body has
    /// one boolean field -- and this closes the class rather than resting it on
    /// every future caller of the method.
    pub async fn set_notify_email(
        &self,
        org: OrgId,
        user: UserId,
        wanted: bool,
    ) -> Result<Option<bool>, StorageError> {
        let row = sqlx::query!(
            "UPDATE app_user SET notify_email = $3 WHERE id = $1 AND org_id = $2 \
             RETURNING notify_email",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
            wanted,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| row.notify_email))
    }

    /// Who in this organisation to mail, skipping everyone who opted out.
    ///
    /// Runs on the engine pool, under the column-scoped `SELECT` migration 0063
    /// grants: `email` is deliberately not among the columns, because the value
    /// there is a placeholder for every self-serve signup.
    pub async fn recipients(&self, org: OrgId) -> Result<Vec<Recipient>, StorageError> {
        let rows = sqlx::query!(
            "SELECT id, auth_subject FROM app_user \
              WHERE org_id = $1 AND notify_email AND auth_subject IS NOT NULL \
              ORDER BY id",
            uuid_to_db(org.0),
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                row.auth_subject.map(|subject| Recipient {
                    user: UserId(uuid_from_db(row.id)),
                    subject: uuid_from_db(subject),
                })
            })
            .collect())
    }

    /// Records a finished spreadsheet import from a caller that owns no batch
    /// transaction of its own.
    ///
    /// The production import path does not come through here: `ImportBatchRepo`
    /// writes the same record inside the transaction that settles the batch, so
    /// that a settled batch and its inbox row commit together or not at all.
    /// This is that write for a caller holding neither, and it is what the
    /// storage tests drive to make a notification without building a whole
    /// spreadsheet.
    ///
    /// Idempotent at the database in both halves, so two writers racing the
    /// same batch produce one row and one message.
    pub async fn record_import(
        &self,
        org: OrgId,
        batch: Uuid,
        counts: NotificationCounts,
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = record(
            &mut tx,
            org,
            &Completion {
                kind: NotificationKind::Import,
                subject: batch,
                inventory: None,
                counts,
                at,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(written)
    }
}

/// One finished run, as the transaction that settled it knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Completion {
    pub(crate) kind: NotificationKind,
    pub(crate) subject: Uuid,
    pub(crate) inventory: Option<InventoryId>,
    pub(crate) counts: NotificationCounts,
    pub(crate) at: Timestamp,
}

/// Writes the inbox row, and the mail message where there is something to say.
///
/// The inbox row is unconditional and the outbox row is not: a run that changed
/// nothing is worth a line in the console and is not worth an email, which is
/// the whole of the fatigue rule on an hourly schedule.
///
/// Both writes are idempotent at the database — `notification_one_per_run` and
/// `outbox_dedupe` — so a race between a migration's two legs produces one of
/// each whichever leg wins, rather than one of each per leg.
pub(crate) async fn record(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
    completion: &Completion,
) -> Result<bool, StorageError> {
    let Completion {
        kind,
        subject,
        inventory,
        counts,
        at,
    } = *completion;
    let counts_json = serde_json::to_value(counts).map_err(|error| StorageError::Inconsistent {
        reason: format!("notification counts must serialise: {error}"),
    })?;
    // A plain insert whose unique violation is the answer, rather than
    // `ON CONFLICT DO NOTHING`. Postgres serves a conflict clause only to a
    // role holding SELECT on the target, and the engine holds INSERT alone
    // here by the founder's decision of 2026-09-07 -- so the conflict form
    // fails `permission denied` on the very path the notification exists for,
    // the settle reached from the maintenance loop. The index is still the
    // arbiter of one row per run; only who asks it has changed.
    let inserted = sqlx::query!(
        "INSERT INTO notification \
         (org_id, id, kind, subject_id, inventory, marketplace, counts, created_at, read_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL)",
        uuid_to_db(org.0),
        uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
        kind_to_db(kind),
        uuid_to_db(subject),
        inventory.map(inventory_to_db),
        inventory.map(|inventory| marketplace_to_db(inventory.marketplace())),
        counts_json,
        timestamp_to_db(at)?,
    )
    .execute(&mut **tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(database))
            if database.constraint() == Some("notification_one_per_run") =>
        {
            // Another writer got there first: a migration's other leg, or a
            // second import chunk finishing the same last row. One row per run
            // is the property, and it holds.
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    }
    if counts.is_empty() {
        return Ok(true);
    }
    let notice = JobSettledNotice {
        kind,
        subject_id: subject,
        inventory,
        marketplace: inventory.map(InventoryId::marketplace),
        counts,
        settled_at: at,
    };
    let payload = serde_json::to_value(&notice).map_err(|error| StorageError::Inconsistent {
        reason: format!("a settled-run notice must serialise: {error}"),
    })?;
    OutboxRepo::append(
        tx,
        &NewOutboxMessage {
            org,
            id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
            topic: JOB_SETTLED_TOPIC.to_owned(),
            dedupe_key: format!("{}:{}", kind_to_db(kind), uuid_to_db(subject)),
            payload,
            at,
        },
    )
    .await?;
    Ok(true)
}

pub(crate) const fn kind_to_db(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::Sync => "sync",
        NotificationKind::Migration => "migration",
        NotificationKind::Import => "import",
    }
}

fn kind_from_db(raw: &str) -> Result<NotificationKind, StorageError> {
    match raw {
        "sync" => Ok(NotificationKind::Sync),
        "migration" => Ok(NotificationKind::Migration),
        "import" => Ok(NotificationKind::Import),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown notification kind {other:?}"),
        }),
    }
}

fn counts_from_db(raw: serde_json::Value) -> Result<NotificationCounts, StorageError> {
    serde_json::from_value(raw).map_err(|error| StorageError::CorruptRow {
        reason: format!("stored notification counts do not parse: {error}"),
    })
}

const fn tone_to_db(tone: NoticeTone) -> &'static str {
    match tone {
        NoticeTone::Success => "success",
        NoticeTone::Info => "info",
        NoticeTone::Warning => "warning",
        NoticeTone::Error => "error",
    }
}

fn tone_from_db(raw: &str) -> Result<NoticeTone, StorageError> {
    match raw {
        "success" => Ok(NoticeTone::Success),
        "info" => Ok(NoticeTone::Info),
        "warning" => Ok(NoticeTone::Warning),
        "error" => Ok(NoticeTone::Error),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown notice tone {other:?}"),
        }),
    }
}
