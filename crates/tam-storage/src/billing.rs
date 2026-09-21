//! One organisation's subscription as the billing provider last described
//! it, and the service bookings it has bought.
//!
//! State tracking and nothing else: this module records what a webhook said
//! and answers what it recorded. No entitlement is decided here.
//!
//! The write is monotone in the event's own instant rather than in arrival
//! order. Stripe retries deliveries and a retried older event can overtake a
//! newer one, so an upsert that trusted arrival order would regress a live
//! subscription to a state it had already left and leave it there until the
//! next event happened to arrive. `apply` therefore compares the incoming
//! `occurred_at` against the stored one and declines to overwrite with
//! anything older, which makes replay and reordering harmless without needing
//! a de-duplication table.

use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// A subscription's state as one event described it.
///
/// `status` is the vocabulary the provider sent, stored verbatim. The column
/// is open for the reason migration 0038 states: the provider owns that
/// vocabulary and versions it on its own schedule, so a value we have not
/// seen before must land in a readable row rather than abort a webhook the
/// provider will then retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionState {
    pub provider_subscription_id: String,
    pub provider_customer_id: String,
    pub status: String,
    /// The price this subscription renews at, where the event named one.
    ///
    /// Kept so the billing page can answer the cadence and the founding
    /// question from the price map alone, without a second read of the
    /// provider. Absent where the event carried no line item, which is what
    /// a cancellation looks like.
    pub provider_price_id: Option<String>,
    /// Absent where the provider carried no billing period — a trial, or a
    /// subscription cancelled outright. Absent rather than fabricated.
    pub current_period_end: Option<Timestamp>,
    /// The instant the provider stamped on the event, not the instant we
    /// wrote it. The ordering fence in [`BillingRepo::apply`] is this field.
    pub occurred_at: Timestamp,
}

/// One service purchase: time bought, not a capability granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceBooking {
    pub id: Uuid,
    /// The service's price key, `move_with_me` today.
    pub key: String,
    /// The provider's own identifier for the purchase — a checkout session —
    /// which is what makes a replayed delivery book once.
    pub provider_ref: String,
    pub created_at: Timestamp,
}

pub struct BillingRepo {
    pool: PgPool,
}

impl BillingRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Applies one event's state, answering whether it was applied.
    ///
    /// `false` means the stored row already carries a strictly newer
    /// `occurred_at`, so this event is stale and was ignored. That is an
    /// ordinary outcome rather than a fault: it is what a retried or
    /// reordered delivery looks like, and the caller answers the provider 200
    /// either way.
    ///
    /// An event bearing the same instant as the stored row is applied rather
    /// than declined, so a corrected redelivery of one event still lands.
    ///
    /// An event naming a `provider_subscription_id` another organisation
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
            "INSERT INTO billing_subscription (org_id, provider, provider_subscription_id, provider_customer_id, provider_price_id, status, current_period_end, occurred_at, updated_at) VALUES ($1, 'stripe', $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (org_id) DO UPDATE SET provider = EXCLUDED.provider, provider_subscription_id = EXCLUDED.provider_subscription_id, provider_customer_id = EXCLUDED.provider_customer_id, provider_price_id = COALESCE(EXCLUDED.provider_price_id, billing_subscription.provider_price_id), status = EXCLUDED.status, current_period_end = EXCLUDED.current_period_end, occurred_at = EXCLUDED.occurred_at, updated_at = EXCLUDED.updated_at WHERE EXCLUDED.occurred_at >= billing_subscription.occurred_at",
            uuid_to_db(org.0),
            state.provider_subscription_id,
            state.provider_customer_id,
            state.provider_price_id,
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
    /// row carrying the provider's cancelled status.
    pub async fn get(&self, org: OrgId) -> Result<Option<SubscriptionState>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT provider_subscription_id, provider_customer_id, provider_price_id, status, current_period_end, occurred_at FROM billing_subscription WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| SubscriptionState {
            provider_subscription_id: row.provider_subscription_id,
            provider_customer_id: row.provider_customer_id,
            provider_price_id: row.provider_price_id,
            status: row.status,
            current_period_end: row.current_period_end.map(timestamp_from_db),
            occurred_at: timestamp_from_db(row.occurred_at),
        }))
    }

    /// Records one service purchase, answering whether it was new.
    ///
    /// `false` means this provider reference is already booked, which is what
    /// a retried webhook delivery looks like. Idempotent on `provider_ref`
    /// rather than on the row identifier, because the provider's identifier
    /// is the one both deliveries agree on.
    pub async fn record_booking(
        &self,
        org: OrgId,
        booking: &ServiceBooking,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "INSERT INTO service_booking (org_id, id, key, provider_ref, created_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (provider_ref) DO NOTHING",
            uuid_to_db(org.0),
            uuid_to_db(booking.id),
            booking.key,
            booking.provider_ref,
            timestamp_to_db(booking.created_at)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(written == 1)
    }
}
