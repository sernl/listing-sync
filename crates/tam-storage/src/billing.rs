//! One organisation's Paddle subscription as Paddle last described it.
//!
//! State tracking and nothing else: this module records what a webhook said
//! and answers what it recorded. No entitlement is decided here, and no
//! surface is gated on the answer yet.
//!
//! The write is monotone in the event's own instant rather than in arrival
//! order. Paddle retries deliveries and a retried older notification can
//! overtake a newer one, so an upsert that trusted arrival order would
//! regress a live subscription to a state it had already left and leave it
//! there until the next event happened to arrive. `apply` therefore compares
//! the incoming `occurred_at` against the stored one and declines to
//! overwrite with anything older, which makes replay and reordering
//! harmless without needing a de-duplication table.

use sqlx::PgPool;
use tam_types::{OrgId, Timestamp};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// A subscription's state as one notification described it.
///
/// `status` is the vocabulary Paddle sent, stored verbatim. The column is
/// open for the reason migration 0038 states: Paddle owns that vocabulary and
/// versions it on its own schedule, so a value we have not seen before must
/// land in a readable row rather than abort a webhook Paddle will then retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionState {
    pub paddle_subscription_id: String,
    pub paddle_customer_id: String,
    pub status: String,
    /// Absent where Paddle carried no billing period — a trial, or a
    /// subscription cancelled outright. Absent rather than fabricated.
    pub current_period_end: Option<Timestamp>,
    /// The instant Paddle stamped on the notification, not the instant we
    /// wrote it. The ordering fence in [`BillingRepo::apply`] is this field.
    pub occurred_at: Timestamp,
}

pub struct BillingRepo {
    pool: PgPool,
}

impl BillingRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Applies one notification's state, answering whether it was applied.
    ///
    /// `false` means the stored row already carries a strictly newer
    /// `occurred_at`, so this notification is stale and was ignored. That is
    /// an ordinary outcome rather than a fault: it is what a retried or
    /// reordered delivery looks like, and the caller answers Paddle 200
    /// either way.
    ///
    /// An event bearing the same instant as the stored row is applied rather
    /// than declined, so a corrected redelivery of one event still lands.
    ///
    /// A notification naming a `paddle_subscription_id` another organisation
    /// already holds raises the unique-violation rather than attaching a
    /// paying customer to a second tenant; migration 0038 argues why that
    /// constraint crosses the tenant fence.
    pub async fn apply(
        &self,
        org: OrgId,
        state: &SubscriptionState,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let applied = sqlx::query!(
            "INSERT INTO billing_subscription \
             (org_id, paddle_subscription_id, paddle_customer_id, status, \
              current_period_end, occurred_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (org_id) DO UPDATE SET \
                 paddle_subscription_id = EXCLUDED.paddle_subscription_id, \
                 paddle_customer_id     = EXCLUDED.paddle_customer_id, \
                 status                 = EXCLUDED.status, \
                 current_period_end     = EXCLUDED.current_period_end, \
                 occurred_at            = EXCLUDED.occurred_at, \
                 updated_at             = EXCLUDED.updated_at \
             WHERE EXCLUDED.occurred_at >= billing_subscription.occurred_at",
            uuid_to_db(org.0),
            state.paddle_subscription_id,
            state.paddle_customer_id,
            state.status,
            state.current_period_end.map(timestamp_to_db).transpose()?,
            timestamp_to_db(state.occurred_at)?,
            timestamp_to_db(now)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(applied == 1)
    }

    /// This organisation's subscription state, or `None` where it has never
    /// had one. `None` is the honest answer for a tenant that never reached
    /// checkout, and is distinct from a cancelled subscription, which is a
    /// row carrying Paddle's cancelled status.
    pub async fn get(&self, org: OrgId) -> Result<Option<SubscriptionState>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT paddle_subscription_id, paddle_customer_id, status, \
                    current_period_end, occurred_at \
               FROM billing_subscription WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| SubscriptionState {
            paddle_subscription_id: row.paddle_subscription_id,
            paddle_customer_id: row.paddle_customer_id,
            status: row.status,
            current_period_end: row.current_period_end.map(timestamp_from_db),
            occurred_at: timestamp_from_db(row.occurred_at),
        }))
    }
}
