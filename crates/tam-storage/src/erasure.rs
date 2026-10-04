//! Deleting a seller: the platform user, and the organisation that was theirs
//! alone.
//!
//! An organisation is a tenant, and every tenant row hangs off it, so deleting
//! a user whose organisation has nobody else in it means deleting the
//! organisation too — anything less leaves a tenant nobody can sign in to.
//! Three cases are refused instead, each for a reason the operator can act on:
//!
//! - the organisation has another member, because their work is in it and
//!   deleting it would delete that too;
//! - the user is an active operator, because the marking is granted and
//!   withdrawn by the `tam-admin` one-shot on the box, not from the console
//!   (and an operator deleting themselves is the same refusal);
//! - a subscription is still live at the billing provider, because deleting
//!   the row that names it would leave the seller being charged for an
//!   account that no longer exists. The operator cancels it first.
//!
//! Every check and the erasure itself run in one transaction with the
//! organisation row locked, so a member joining or a checkout completing
//! between the check and the delete is not a window. The deletion is
//! `erase_organisation` (migration 0092), which walks the catalogue so a
//! tenant table added later is erased without this file changing.
//!
//! Two callers, one path. An operator deletes a seller from the console
//! ([`ErasureRepo::erase_account`]); a seller deletes themselves
//! ([`ErasureRepo::erase_departing`]), which is the same erasure with an
//! `account_deletion` row written after it in the same transaction (migration
//! 0104). The self-service route reads [`ErasureRepo::inspect`] first, so it
//! can refuse before it cancels anything at Stripe or touches the identity
//! service, and the erasure checks again under the lock.

use sqlx::{PgPool, Postgres, Transaction};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::codec::{timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// Subscription states after which the provider charges nothing more.
/// Everything else — active, trialing, past_due, unpaid, incomplete, paused —
/// is a subscription the provider still holds open against the customer.
const ENDED_SUBSCRIPTION_STATES: [&str; 2] = ["canceled", "incomplete_expired"];

/// Why an account was not deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureRefusal {
    /// No platform user carries this identity subject. The identity account
    /// may still exist; there is simply nothing on this side to delete.
    NoPlatformUser,
    /// The user holds an operator marking nobody has withdrawn.
    ActiveOperator,
    /// Someone else is in the organisation.
    SharedOrganisation { other_members: i64 },
    /// The billing provider still holds a subscription open.
    LiveSubscription { status: String },
}

/// What was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erased {
    pub user: UserId,
    pub org: OrgId,
    pub org_name: String,
    /// Every row that left, the organisation's included. For the log line.
    pub rows: i64,
}

/// The subscription an account's organisation holds, as last recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSubscription {
    pub provider_subscription_id: String,
    pub status: String,
}

impl HeldSubscription {
    /// Whether the provider still charges for it.
    #[must_use]
    pub fn is_live(&self) -> bool {
        !ENDED_SUBSCRIPTION_STATES.contains(&self.status.as_str())
    }
}

/// An account that may be erased once its subscription has ended: the
/// checks that do not depend on billing have passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub user: UserId,
    pub org: OrgId,
    pub org_name: String,
    pub subscription: Option<HeldSubscription>,
}

/// What a seller's own deletion leaves behind (migration 0104).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionRecord {
    /// Lower-case hex SHA-256 of the trimmed, lower-cased address.
    pub email_hash: String,
    pub requested_at: Timestamp,
    /// The subscription Stripe was told to cancel on the way out, if any.
    pub stripe_subscription_id: Option<String>,
    pub reason: Option<String>,
}

pub struct ErasureRepo {
    pool: PgPool,
}

impl ErasureRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The checks an erasure makes, read without writing, and what the
    /// account's organisation holds at the billing provider. A live
    /// subscription is not a refusal here: the self-service route cancels it
    /// before erasing, and the erasure itself still refuses one.
    pub async fn inspect(
        &self,
        subject: Uuid,
    ) -> Result<Result<Standing, ErasureRefusal>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let standing = standing(&mut tx, subject, false).await?;
        tx.rollback().await?;
        Ok(standing)
    }

    /// Delete the platform user this identity subject names, and their
    /// organisation, or say why not. Nothing is written on a refusal.
    pub async fn erase_account(
        &self,
        subject: Uuid,
    ) -> Result<Result<Erased, ErasureRefusal>, StorageError> {
        self.erase(subject, None).await
    }

    /// The seller's own deletion: [`Self::erase_account`], and the
    /// `account_deletion` row in the same transaction.
    pub async fn erase_departing(
        &self,
        subject: Uuid,
        record: &DeletionRecord,
    ) -> Result<Result<Erased, ErasureRefusal>, StorageError> {
        self.erase(subject, Some(record)).await
    }

    async fn erase(
        &self,
        subject: Uuid,
        record: Option<&DeletionRecord>,
    ) -> Result<Result<Erased, ErasureRefusal>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let target = match standing(&mut tx, subject, true).await? {
            Ok(target) => target,
            Err(refusal) => return Ok(Err(refusal)),
        };
        if let Some(subscription) = target.subscription.filter(HeldSubscription::is_live) {
            return Ok(Err(ErasureRefusal::LiveSubscription {
                status: subscription.status,
            }));
        }

        let rows = sqlx::query_scalar!(
            "SELECT erase_organisation($1) AS \"rows!\"",
            uuid_to_db(target.org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        if let Some(record) = record {
            sqlx::query!(
                "INSERT INTO account_deletion \
                 (subject, email_hash, org_id, requested_at, stripe_subscription_id, reason) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                uuid_to_db(subject),
                record.email_hash,
                uuid_to_db(target.org.0),
                timestamp_to_db(record.requested_at)?,
                record.stripe_subscription_id,
                record.reason,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Ok(Erased {
            user: target.user,
            org: target.org,
            org_name: target.org_name,
            rows,
        }))
    }
}

/// The account a subject names, pinned, with the operator and membership
/// checks made. `lock` takes the organisation row for the erasure that
/// follows; the read-only inspection does not.
async fn standing(
    tx: &mut Transaction<'_, Postgres>,
    subject: Uuid,
    lock: bool,
) -> Result<Result<Standing, ErasureRefusal>, StorageError> {
    let target = if lock {
        sqlx::query_as!(
            Target,
            "SELECT u.id, u.org_id, o.name FROM app_user u \
             JOIN organisation o ON o.id = u.org_id \
             WHERE u.auth_subject = $1 \
             FOR UPDATE OF o",
            uuid_to_db(subject),
        )
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query_as!(
            Target,
            "SELECT u.id, u.org_id, o.name FROM app_user u \
             JOIN organisation o ON o.id = u.org_id \
             WHERE u.auth_subject = $1",
            uuid_to_db(subject),
        )
        .fetch_optional(&mut **tx)
        .await?
    };
    let Some(target) = target else {
        return Ok(Err(ErasureRefusal::NoPlatformUser));
    };
    let org = OrgId(uuid_from_db(target.org_id));
    pin_org(tx, org).await?;

    let operator = sqlx::query!(
        "SELECT 1 AS \"present!\" FROM platform_operator \
         WHERE user_id = $1 AND revoked_at IS NULL",
        target.id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if operator.is_some() {
        return Ok(Err(ErasureRefusal::ActiveOperator));
    }

    let other_members = sqlx::query_scalar!(
        "SELECT count(*) AS \"count!\" FROM app_user WHERE org_id = $1 AND id <> $2",
        target.org_id,
        target.id,
    )
    .fetch_one(&mut **tx)
    .await?;
    if other_members > 0 {
        return Ok(Err(ErasureRefusal::SharedOrganisation { other_members }));
    }

    let subscription = sqlx::query!(
        "SELECT provider_subscription_id, status FROM billing_subscription WHERE org_id = $1",
        target.org_id,
    )
    .fetch_optional(&mut **tx)
    .await?
    .map(|row| HeldSubscription {
        provider_subscription_id: row.provider_subscription_id,
        status: row.status,
    });
    Ok(Ok(Standing {
        user: UserId(uuid_from_db(target.id)),
        org,
        org_name: target.name,
        subscription,
    }))
}

struct Target {
    id: sqlx::types::Uuid,
    org_id: sqlx::types::Uuid,
    name: String,
}
