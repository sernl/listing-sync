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

use sqlx::PgPool;
use tam_types::{OrgId, UserId, Uuid};

use crate::codec::{uuid_from_db, uuid_to_db};
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

pub struct ErasureRepo {
    pool: PgPool,
}

impl ErasureRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Delete the platform user this identity subject names, and their
    /// organisation, or say why not. Nothing is written on a refusal.
    pub async fn erase_account(
        &self,
        subject: Uuid,
    ) -> Result<Result<Erased, ErasureRefusal>, StorageError> {
        let mut tx = self.pool.begin().await?;
        let Some(target) = sqlx::query!(
            "SELECT u.id, u.org_id, o.name FROM app_user u \
             JOIN organisation o ON o.id = u.org_id \
             WHERE u.auth_subject = $1 \
             FOR UPDATE OF o",
            uuid_to_db(subject),
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(Err(ErasureRefusal::NoPlatformUser));
        };
        let user = UserId(uuid_from_db(target.id));
        let org = OrgId(uuid_from_db(target.org_id));
        pin_org(&mut tx, org).await?;

        let operator = sqlx::query!(
            "SELECT 1 AS \"present!\" FROM platform_operator \
             WHERE user_id = $1 AND revoked_at IS NULL",
            target.id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if operator.is_some() {
            return Ok(Err(ErasureRefusal::ActiveOperator));
        }

        let other_members = sqlx::query_scalar!(
            "SELECT count(*) AS \"count!\" FROM app_user WHERE org_id = $1 AND id <> $2",
            target.org_id,
            target.id,
        )
        .fetch_one(&mut *tx)
        .await?;
        if other_members > 0 {
            return Ok(Err(ErasureRefusal::SharedOrganisation { other_members }));
        }

        let subscription = sqlx::query_scalar!(
            "SELECT status FROM billing_subscription WHERE org_id = $1",
            target.org_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(status) = subscription {
            if !ENDED_SUBSCRIPTION_STATES.contains(&status.as_str()) {
                return Ok(Err(ErasureRefusal::LiveSubscription { status }));
            }
        }

        let rows = sqlx::query_scalar!(
            "SELECT erase_organisation($1) AS \"rows!\"",
            target.org_id
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Ok(Erased {
            user,
            org,
            org_name: target.name,
            rows,
        }))
    }
}
