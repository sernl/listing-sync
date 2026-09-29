//! The operators' mail to sellers (migration 0097): campaigns, the recipient
//! rows that are each recipient's outbox entry, the pictures a body shows, and
//! the seller's marketing opt-out.
//!
//! Everything here runs on the application pool. The three tables are global
//! and the backoffice role is granted nothing on them; `app_user` and
//! `organisation` are the application role's to read across tenants already
//! (both are classified global in `tests/rls_matrix.rs`). The plan each
//! organisation holds is the one read this surface needs from the backoffice
//! pool, and it is [`crate::BackofficeRepo::plans`], not a method here.
//!
//! No address passes through this module. A recipient row names the identity
//! subject the drainer resolves at send time, as the completion mail does.

use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::codec::{hash_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::StorageError;

/// One seller an audience is chosen from: every app user, with what the
/// filters and the recipient row need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudienceMember {
    pub user: UserId,
    /// `None` for a user with no sign-in identity, who has no address the
    /// identity service could answer and so is never queued.
    pub auth_subject: Option<Uuid>,
    pub org: OrgId,
    pub org_name: String,
    pub opted_out: bool,
}

/// A campaign as the operator route writes it.
#[derive(Debug, Clone)]
pub struct NewCampaign<'a> {
    pub id: Uuid,
    pub subject: &'a str,
    pub body_html: &'a str,
    pub link_url: Option<&'a str>,
    pub link_label: Option<&'a str>,
    pub audience: &'a serde_json::Value,
    pub test: bool,
    pub created_by: UserId,
    pub created_by_label: &'a str,
    pub at: Timestamp,
}

/// One recipient as queued: the subject to resolve and the two values the
/// mail says about them, frozen at queue time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRecipient {
    pub user: UserId,
    pub auth_subject: Uuid,
    pub org_name: String,
    pub plan: String,
}

/// How a campaign's recipients stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CampaignCounts {
    pub total: i64,
    pub queued: i64,
    pub sent: i64,
    pub failed: i64,
    pub skipped: i64,
}

/// One campaign as stored, with its counts: live ones counted from the
/// recipient rows, deleted ones from the snapshot taken as they went.
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignRecord {
    pub id: Uuid,
    pub subject: String,
    pub body_html: Option<String>,
    pub link_url: Option<String>,
    pub link_label: Option<String>,
    pub audience: serde_json::Value,
    pub test: bool,
    pub created_by: UserId,
    pub created_by_label: String,
    pub created_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
    pub counts: CampaignCounts,
}

/// One recipient row as the detail view reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientRecord {
    pub user: UserId,
    pub auth_subject: Uuid,
    pub org_name: String,
    pub plan: String,
    pub status: String,
    pub attempts: i32,
    pub provider_id: Option<String>,
    pub error: Option<String>,
    pub updated_at: Timestamp,
}

/// One recipient the drainer has claimed, with everything the send needs
/// except the address.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedMail {
    pub campaign: Uuid,
    pub user: UserId,
    pub auth_subject: Uuid,
    pub org_name: String,
    pub plan: String,
    /// Including this one.
    pub attempts: i32,
    pub subject: String,
    pub body_html: String,
    pub link_url: Option<String>,
    pub link_label: Option<String>,
    pub audience: serde_json::Value,
    pub opted_out: bool,
    pub marketing_token: Uuid,
}

/// What became of one claimed send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailOutcome {
    Sent {
        provider_id: String,
    },
    Failed {
        error: String,
    },
    Skipped {
        reason: String,
    },
    /// Put back in the queue, due again at `at`.
    Retry {
        error: String,
        at: Timestamp,
    },
}

pub struct MailCampaignRepo {
    pool: PgPool,
}

/// How many campaigns the log lists at most.
const LIST_MAX: i64 = 200;

impl MailCampaignRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every app user with their organisation and whether they opted out.
    pub async fn audience_members(&self) -> Result<Vec<AudienceMember>, StorageError> {
        let rows = sqlx::query!(
            "SELECT u.id, u.auth_subject, u.marketing_opt_out, o.id AS \"org_id\", o.name AS \"org_name\" \
             FROM app_user u JOIN organisation o ON o.id = u.org_id \
             ORDER BY u.created_at, u.id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AudienceMember {
                user: UserId(uuid_from_db(row.id)),
                auth_subject: row.auth_subject.map(uuid_from_db),
                org: OrgId(uuid_from_db(row.org_id)),
                org_name: row.org_name,
                opted_out: row.marketing_opt_out,
            })
            .collect())
    }

    /// Writes a campaign and one queued row per recipient, in one
    /// transaction: a campaign is never visible half-queued.
    pub async fn create(
        &self,
        campaign: &NewCampaign<'_>,
        recipients: &[NewRecipient],
    ) -> Result<(), StorageError> {
        let at = timestamp_to_db(campaign.at)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "INSERT INTO mail_campaign (id, subject, body_html, link_url, link_label, audience, \
                 test, created_by, created_by_label, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            uuid_to_db(campaign.id),
            campaign.subject,
            campaign.body_html,
            campaign.link_url,
            campaign.link_label,
            campaign.audience,
            campaign.test,
            uuid_to_db(campaign.created_by.0),
            campaign.created_by_label,
            at,
        )
        .execute(&mut *tx)
        .await?;
        let users: Vec<uuid::Uuid> = recipients.iter().map(|r| uuid_to_db(r.user.0)).collect();
        let subjects: Vec<uuid::Uuid> = recipients
            .iter()
            .map(|r| uuid_to_db(r.auth_subject))
            .collect();
        let orgs: Vec<String> = recipients.iter().map(|r| r.org_name.clone()).collect();
        let plans: Vec<String> = recipients.iter().map(|r| r.plan.clone()).collect();
        sqlx::query!(
            "INSERT INTO mail_campaign_recipient \
                 (campaign_id, user_id, auth_subject, org_name, plan, next_attempt_at, updated_at) \
             SELECT $1, u, s, o, p, $6, $6 \
             FROM UNNEST($2::uuid[], $3::uuid[], $4::text[], $5::text[]) AS t(u, s, o, p)",
            uuid_to_db(campaign.id),
            &users,
            &subjects,
            &orgs,
            &plans,
            at,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes this operator's earlier test sends that have finished, so
    /// "send a test" does not grow a table nobody lists.
    pub async fn purge_finished_tests(&self, by: UserId) -> Result<u64, StorageError> {
        let done = sqlx::query!(
            "DELETE FROM mail_campaign c WHERE c.test AND c.created_by = $1 \
               AND NOT EXISTS (SELECT 1 FROM mail_campaign_recipient r \
                                WHERE r.campaign_id = c.id AND r.status = 'queued')",
            uuid_to_db(by.0),
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// The campaign log, newest first, test sends left out.
    pub async fn list(&self) -> Result<Vec<CampaignRecord>, StorageError> {
        let rows = sqlx::query!(
            "SELECT c.id, c.subject, c.body_html, c.link_url, c.link_label, c.audience, c.test, \
                    c.created_by, c.created_by_label, c.created_at, c.deleted_at, c.final_counts, \
                    count(r.user_id) AS \"total!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'queued') AS \"queued!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'sent') AS \"sent!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'failed') AS \"failed!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'skipped') AS \"skipped!\" \
             FROM mail_campaign c LEFT JOIN mail_campaign_recipient r ON r.campaign_id = c.id \
             WHERE NOT c.test \
             GROUP BY c.id ORDER BY c.created_at DESC, c.id LIMIT $1",
            LIST_MAX,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let live = CampaignCounts {
                    total: row.total,
                    queued: row.queued,
                    sent: row.sent,
                    failed: row.failed,
                    skipped: row.skipped,
                };
                Ok(CampaignRecord {
                    id: uuid_from_db(row.id),
                    subject: row.subject,
                    body_html: row.body_html,
                    link_url: row.link_url,
                    link_label: row.link_label,
                    audience: row.audience,
                    test: row.test,
                    created_by: UserId(uuid_from_db(row.created_by)),
                    created_by_label: row.created_by_label,
                    created_at: timestamp_from_db(row.created_at),
                    deleted_at: row.deleted_at.map(timestamp_from_db),
                    counts: counts_of(row.final_counts, live)?,
                })
            })
            .collect()
    }

    /// One campaign, test sends included, or `None`.
    pub async fn get(&self, id: Uuid) -> Result<Option<CampaignRecord>, StorageError> {
        let Some(row) = sqlx::query!(
            "SELECT c.id, c.subject, c.body_html, c.link_url, c.link_label, c.audience, c.test, \
                    c.created_by, c.created_by_label, c.created_at, c.deleted_at, c.final_counts, \
                    count(r.user_id) AS \"total!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'queued') AS \"queued!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'sent') AS \"sent!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'failed') AS \"failed!\", \
                    count(r.user_id) FILTER (WHERE r.status = 'skipped') AS \"skipped!\" \
             FROM mail_campaign c LEFT JOIN mail_campaign_recipient r ON r.campaign_id = c.id \
             WHERE c.id = $1 GROUP BY c.id",
            uuid_to_db(id),
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let live = CampaignCounts {
            total: row.total,
            queued: row.queued,
            sent: row.sent,
            failed: row.failed,
            skipped: row.skipped,
        };
        Ok(Some(CampaignRecord {
            id: uuid_from_db(row.id),
            subject: row.subject,
            body_html: row.body_html,
            link_url: row.link_url,
            link_label: row.link_label,
            audience: row.audience,
            test: row.test,
            created_by: UserId(uuid_from_db(row.created_by)),
            created_by_label: row.created_by_label,
            created_at: timestamp_from_db(row.created_at),
            deleted_at: row.deleted_at.map(timestamp_from_db),
            counts: counts_of(row.final_counts, live)?,
        }))
    }

    /// A campaign's recipient rows: failures first, then by organisation.
    pub async fn recipients(&self, id: Uuid) -> Result<Vec<RecipientRecord>, StorageError> {
        let rows = sqlx::query!(
            "SELECT user_id, auth_subject, org_name, plan, status, attempts, provider_id, error, \
                    updated_at \
             FROM mail_campaign_recipient WHERE campaign_id = $1 \
             ORDER BY (status = 'failed') DESC, org_name, user_id",
            uuid_to_db(id),
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| RecipientRecord {
                user: UserId(uuid_from_db(row.user_id)),
                auth_subject: uuid_from_db(row.auth_subject),
                org_name: row.org_name,
                plan: row.plan,
                status: row.status,
                attempts: row.attempts,
                provider_id: row.provider_id,
                error: row.error,
                updated_at: timestamp_from_db(row.updated_at),
            })
            .collect())
    }

    /// Puts every failed recipient of a live campaign back in the queue, due
    /// now, with a fresh attempt budget. Answers how many.
    pub async fn retry_failed(&self, id: Uuid, now: Timestamp) -> Result<u64, StorageError> {
        let done = sqlx::query!(
            "UPDATE mail_campaign_recipient SET status = 'queued', attempts = 0, error = NULL, \
                 leased_until = NULL, next_attempt_at = $2, updated_at = $2 \
             WHERE campaign_id = $1 AND status = 'failed'",
            uuid_to_db(id),
            timestamp_to_db(now)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// Deletes a campaign's body and every recipient row, keeping the log
    /// line with its counts frozen. `false` where there is no live campaign
    /// by that id. Anything still queued is not sent.
    pub async fn delete(&self, id: Uuid, by: UserId, now: Timestamp) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        let counts = sqlx::query!(
            "SELECT count(*) AS \"total!\", \
                    count(*) FILTER (WHERE status = 'queued') AS \"queued!\", \
                    count(*) FILTER (WHERE status = 'sent') AS \"sent!\", \
                    count(*) FILTER (WHERE status = 'failed') AS \"failed!\", \
                    count(*) FILTER (WHERE status = 'skipped') AS \"skipped!\" \
             FROM mail_campaign_recipient WHERE campaign_id = $1",
            uuid_to_db(id),
        )
        .fetch_one(&mut *tx)
        .await?;
        let frozen = serde_json::json!({
            "total": counts.total,
            "queued": counts.queued,
            "sent": counts.sent,
            "failed": counts.failed,
            "skipped": counts.skipped,
        });
        let marked = sqlx::query!(
            "UPDATE mail_campaign SET body_html = NULL, link_url = NULL, link_label = NULL, \
                 deleted_at = $2, deleted_by = $3, final_counts = $4 \
             WHERE id = $1 AND deleted_at IS NULL",
            uuid_to_db(id),
            timestamp_to_db(now)?,
            uuid_to_db(by.0),
            frozen,
        )
        .execute(&mut *tx)
        .await?;
        if marked.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query!(
            "DELETE FROM mail_campaign_recipient WHERE campaign_id = $1",
            uuid_to_db(id),
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Claims the oldest due queued recipient until `lease_until`, counting
    /// the attempt, or `None` when nothing is due.
    ///
    /// `SKIP LOCKED` and the lease together make a second drainer, or a pass
    /// that died mid-send, safe: the row is invisible to the next claim until
    /// the lease runs out, and then due again.
    pub async fn claim(
        &self,
        now: Timestamp,
        lease_until: Timestamp,
    ) -> Result<Option<ClaimedMail>, StorageError> {
        let row = sqlx::query!(
            "WITH due AS ( \
                 SELECT campaign_id, user_id FROM mail_campaign_recipient \
                 WHERE status = 'queued' AND next_attempt_at <= $1 \
                   AND (leased_until IS NULL OR leased_until <= $1) \
                 ORDER BY next_attempt_at, campaign_id, user_id \
                 LIMIT 1 FOR UPDATE SKIP LOCKED) \
             UPDATE mail_campaign_recipient r \
                SET leased_until = $2, attempts = r.attempts + 1, updated_at = $1 \
             FROM due, mail_campaign c, app_user u \
             WHERE r.campaign_id = due.campaign_id AND r.user_id = due.user_id \
               AND c.id = r.campaign_id AND u.id = r.user_id \
             RETURNING r.campaign_id, r.user_id, r.auth_subject, r.org_name, r.plan, r.attempts, \
                       c.subject, c.body_html, c.link_url, c.link_label, c.audience, \
                       u.marketing_opt_out, u.marketing_token",
            timestamp_to_db(now)?,
            timestamp_to_db(lease_until)?,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let body_html = row.body_html.ok_or_else(|| StorageError::CorruptRow {
            reason: "a queued recipient belongs to a campaign with no body".to_owned(),
        })?;
        Ok(Some(ClaimedMail {
            campaign: uuid_from_db(row.campaign_id),
            user: UserId(uuid_from_db(row.user_id)),
            auth_subject: uuid_from_db(row.auth_subject),
            org_name: row.org_name,
            plan: row.plan,
            attempts: row.attempts,
            subject: row.subject,
            body_html,
            link_url: row.link_url,
            link_label: row.link_label,
            audience: row.audience,
            opted_out: row.marketing_opt_out,
            marketing_token: uuid_from_db(row.marketing_token),
        }))
    }

    /// Records what became of one claimed send and releases the claim.
    pub async fn settle(
        &self,
        campaign: Uuid,
        user: UserId,
        outcome: &MailOutcome,
        now: Timestamp,
    ) -> Result<(), StorageError> {
        let (status, provider_id, error, due) = match outcome {
            MailOutcome::Sent { provider_id } => ("sent", Some(provider_id.as_str()), None, now),
            MailOutcome::Failed { error } => ("failed", None, Some(error.as_str()), now),
            MailOutcome::Skipped { reason } => ("skipped", None, Some(reason.as_str()), now),
            MailOutcome::Retry { error, at } => ("queued", None, Some(error.as_str()), *at),
        };
        sqlx::query!(
            "UPDATE mail_campaign_recipient \
                SET status = $3, provider_id = $4, error = $5, next_attempt_at = $6, \
                    leased_until = NULL, updated_at = $7 \
              WHERE campaign_id = $1 AND user_id = $2",
            uuid_to_db(campaign),
            uuid_to_db(user.0),
            status,
            provider_id,
            error,
            timestamp_to_db(due)?,
            timestamp_to_db(now)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Follows an unsubscribe link: the user holding this token opts out.
    /// `false` where no user holds it.
    pub async fn opt_out_by_token(&self, token: Uuid) -> Result<bool, StorageError> {
        let done = sqlx::query!(
            "UPDATE app_user SET marketing_opt_out = true WHERE marketing_token = $1",
            uuid_to_db(token),
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Whether this user takes the operators' news. `None` where this
    /// organisation holds no such user.
    pub async fn marketing_email(
        &self,
        org: OrgId,
        user: UserId,
    ) -> Result<Option<bool>, StorageError> {
        let row = sqlx::query!(
            "SELECT marketing_opt_out FROM app_user WHERE id = $1 AND org_id = $2",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| !row.marketing_opt_out))
    }

    /// Sets it for one user of one organisation, fenced by both for the
    /// reason `NotificationRepo::set_notify_email` is.
    pub async fn set_marketing_email(
        &self,
        org: OrgId,
        user: UserId,
        wanted: bool,
    ) -> Result<Option<bool>, StorageError> {
        let row = sqlx::query!(
            "UPDATE app_user SET marketing_opt_out = $3 WHERE id = $1 AND org_id = $2 \
             RETURNING marketing_opt_out",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
            !wanted,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| !row.marketing_opt_out))
    }

    /// Lists an uploaded picture as one the public mail-image route serves.
    pub async fn record_image(
        &self,
        hash: tam_types::ContentHash,
        by: UserId,
        at: Timestamp,
    ) -> Result<(), StorageError> {
        sqlx::query!(
            "INSERT INTO mail_image (hash, uploaded_by, uploaded_at) VALUES ($1, $2, $3) \
             ON CONFLICT (hash) DO NOTHING",
            hash_to_db(hash),
            uuid_to_db(by.0),
            timestamp_to_db(at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether a picture was uploaded for mail.
    pub async fn image_listed(&self, hash: tam_types::ContentHash) -> Result<bool, StorageError> {
        let row = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM mail_image WHERE hash = $1) AS \"listed!\"",
            hash_to_db(hash),
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}

/// The counts a campaign shows: the frozen snapshot once deleted, the live
/// ones before.
fn counts_of(
    frozen: Option<serde_json::Value>,
    live: CampaignCounts,
) -> Result<CampaignCounts, StorageError> {
    let Some(value) = frozen else {
        return Ok(live);
    };
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| StorageError::CorruptRow {
                reason: format!("mail_campaign.final_counts has no count for {name}"),
            })
    };
    Ok(CampaignCounts {
        total: field("total")?,
        queued: field("queued")?,
        sent: field("sent")?,
        failed: field("failed")?,
        skipped: field("skipped")?,
    })
}
