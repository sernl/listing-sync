//! Discounts: the platform's sale periods, one-off discounts and typed codes,
//! each one a Stripe Coupon whose identifier is kept here.
//!
//! Everything runs on the application pool, which owns these tables by owning
//! the database, and nothing takes an organisation: a discount is offered to
//! every tenant, so there is no fence to pin (migration 0091).
//!
//! Terms are written once. The only change a stored discount accepts is being
//! ended, because Stripe's coupon is immutable in the same way and a row whose
//! terms drifted from its coupon would advertise one price and charge
//! another.

use sqlx::PgPool;
use tam_types::{Timestamp, UserId, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::StorageError;

/// How a discount reaches a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountKind {
    /// Automatic on every plan while open, and announced with a banner.
    Sale,
    /// Automatic on the price keys it names while open.
    OneOff,
    /// Only through one of its typed codes.
    Code,
}

impl DiscountKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sale => "sale",
            Self::OneOff => "one_off",
            Self::Code => "code",
        }
    }

    fn parse(raw: &str) -> Result<Self, StorageError> {
        match raw {
            "sale" => Ok(Self::Sale),
            "one_off" => Ok(Self::OneOff),
            "code" => Ok(Self::Code),
            other => Err(StorageError::CorruptRow {
                reason: format!("discount.kind {other:?} is not a known kind"),
            }),
        }
    }
}

/// What a discount takes off. One of the two, never both: migration 0091's
/// `discount_one_amount` holds the same line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountAmount {
    Percent(u32),
    /// Cents in the discount's currency.
    Cents(u32),
}

/// Which invoices a discount reaches. `Once` is the first invoice of a
/// subscription or the one payment; `Repeating` the first `n` invoices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscountDuration {
    Once,
    Repeating(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discount {
    pub id: Uuid,
    pub kind: DiscountKind,
    pub name: String,
    pub amount: DiscountAmount,
    pub currency: String,
    pub duration: DiscountDuration,
    /// Price keys, in `tam-limits` vocabulary. Empty means every plan.
    pub price_keys: Vec<String>,
    pub starts_at: Timestamp,
    /// Exclusive.
    pub ends_at: Timestamp,
    pub stripe_coupon_id: String,
    pub ended_at: Option<Timestamp>,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
}

impl Discount {
    /// Whether this discount is offered at `now`: inside its window and not
    /// ended early.
    #[must_use]
    pub fn open_at(&self, now: Timestamp) -> bool {
        self.ended_at.is_none() && self.starts_at.0 <= now.0 && now.0 < self.ends_at.0
    }

    /// Whether it reaches one price key. `plan` says whether the key is a
    /// plan, which is what an empty key set means.
    #[must_use]
    pub fn covers(&self, key: &str, plan: bool) -> bool {
        if self.price_keys.is_empty() {
            plan
        } else {
            self.price_keys.iter().any(|named| named == key)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscountCode {
    pub id: Uuid,
    pub discount_id: Uuid,
    pub code: String,
    pub stripe_promotion_code_id: String,
    pub max_redemptions: Option<u32>,
    pub ended_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// Why a code could not be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeWrite {
    Stored,
    /// A live code already spells this, case aside.
    Taken,
}

pub struct DiscountRepo {
    pool: PgPool,
}

struct DiscountRow {
    id: uuid::Uuid,
    kind: String,
    name: String,
    percent_off: Option<i32>,
    amount_off_cents: Option<i32>,
    currency: String,
    duration: String,
    duration_months: Option<i32>,
    price_keys: Vec<String>,
    starts_at: chrono::DateTime<chrono::Utc>,
    ends_at: chrono::DateTime<chrono::Utc>,
    stripe_coupon_id: String,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
    created_by: Option<uuid::Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn non_negative(value: i32, column: &str) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::CorruptRow {
        reason: format!("discount.{column} is negative"),
    })
}

impl TryFrom<DiscountRow> for Discount {
    type Error = StorageError;

    fn try_from(row: DiscountRow) -> Result<Self, StorageError> {
        let amount = match (row.percent_off, row.amount_off_cents) {
            (Some(percent), None) => DiscountAmount::Percent(non_negative(percent, "percent_off")?),
            (None, Some(cents)) => DiscountAmount::Cents(non_negative(cents, "amount_off_cents")?),
            _ => {
                return Err(StorageError::CorruptRow {
                    reason: "discount carries both or neither of percent_off and amount_off_cents"
                        .to_owned(),
                })
            }
        };
        let duration = match (row.duration.as_str(), row.duration_months) {
            ("once", None) => DiscountDuration::Once,
            ("repeating", Some(months)) => {
                DiscountDuration::Repeating(non_negative(months, "duration_months")?)
            }
            (other, _) => {
                return Err(StorageError::CorruptRow {
                    reason: format!("discount.duration {other:?} is not a known duration"),
                })
            }
        };
        Ok(Self {
            id: uuid_from_db(row.id),
            kind: DiscountKind::parse(&row.kind)?,
            name: row.name,
            amount,
            currency: row.currency,
            duration,
            price_keys: row.price_keys,
            starts_at: timestamp_from_db(row.starts_at),
            ends_at: timestamp_from_db(row.ends_at),
            stripe_coupon_id: row.stripe_coupon_id,
            ended_at: row.ended_at.map(timestamp_from_db),
            created_by: row.created_by.map(|id| UserId(uuid_from_db(id))),
            created_at: timestamp_from_db(row.created_at),
        })
    }
}

struct DiscountCodeRow {
    id: uuid::Uuid,
    discount_id: uuid::Uuid,
    code: String,
    stripe_promotion_code_id: String,
    max_redemptions: Option<i32>,
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl TryFrom<DiscountCodeRow> for DiscountCode {
    type Error = StorageError;

    fn try_from(row: DiscountCodeRow) -> Result<Self, StorageError> {
        Ok(Self {
            id: uuid_from_db(row.id),
            discount_id: uuid_from_db(row.discount_id),
            code: row.code,
            stripe_promotion_code_id: row.stripe_promotion_code_id,
            max_redemptions: row
                .max_redemptions
                .map(|n| non_negative(n, "max_redemptions"))
                .transpose()?,
            ended_at: row.ended_at.map(timestamp_from_db),
            created_at: timestamp_from_db(row.created_at),
        })
    }
}

fn to_i32(value: u32, column: &str) -> Result<i32, StorageError> {
    i32::try_from(value).map_err(|_| StorageError::Inconsistent {
        reason: format!("discount.{column} exceeds the column's range"),
    })
}

impl DiscountRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records a discount whose coupon Stripe has already created.
    pub async fn insert(&self, discount: &Discount) -> Result<(), StorageError> {
        let (percent, cents) = match discount.amount {
            DiscountAmount::Percent(percent) => (Some(to_i32(percent, "percent_off")?), None),
            DiscountAmount::Cents(cents) => (None, Some(to_i32(cents, "amount_off_cents")?)),
        };
        let (duration, months) = match discount.duration {
            DiscountDuration::Once => ("once", None),
            DiscountDuration::Repeating(months) => {
                ("repeating", Some(to_i32(months, "duration_months")?))
            }
        };
        sqlx::query!(
            "INSERT INTO discount (id, kind, name, percent_off, amount_off_cents, currency, duration, duration_months, price_keys, starts_at, ends_at, stripe_coupon_id, ended_at, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
            uuid_to_db(discount.id),
            discount.kind.as_str(),
            discount.name,
            percent,
            cents,
            discount.currency,
            duration,
            months,
            &discount.price_keys,
            timestamp_to_db(discount.starts_at)?,
            timestamp_to_db(discount.ends_at)?,
            discount.stripe_coupon_id,
            discount.ended_at.map(timestamp_to_db).transpose()?,
            discount.created_by.map(|user| uuid_to_db(user.0)),
            timestamp_to_db(discount.created_at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every discount, newest window first. The admin page reads all of
    /// them: ended and expired ones are the history of what was offered.
    pub async fn list(&self) -> Result<Vec<Discount>, StorageError> {
        let rows = sqlx::query_as!(
            DiscountRow,
            "SELECT id, kind, name, percent_off, amount_off_cents, currency, duration, duration_months, price_keys, starts_at, ends_at, stripe_coupon_id, ended_at, created_by, created_at FROM discount ORDER BY starts_at DESC, created_at DESC"
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Discount::try_from).collect()
    }

    /// The discounts a checkout at `now` may apply without a code: sales and
    /// one-offs whose window holds `now` and which were not ended.
    pub async fn open_automatic(&self, now: Timestamp) -> Result<Vec<Discount>, StorageError> {
        let rows = sqlx::query_as!(
            DiscountRow,
            "SELECT id, kind, name, percent_off, amount_off_cents, currency, duration, duration_months, price_keys, starts_at, ends_at, stripe_coupon_id, ended_at, created_by, created_at FROM discount WHERE kind IN ('sale', 'one_off') AND ended_at IS NULL AND starts_at <= $1 AND ends_at > $1 ORDER BY starts_at, created_at",
            timestamp_to_db(now)?,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Discount::try_from).collect()
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<Discount>, StorageError> {
        let row = sqlx::query_as!(
            DiscountRow,
            "SELECT id, kind, name, percent_off, amount_off_cents, currency, duration, duration_months, price_keys, starts_at, ends_at, stripe_coupon_id, ended_at, created_by, created_at FROM discount WHERE id = $1",
            uuid_to_db(id),
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(Discount::try_from).transpose()
    }

    /// Ends a discount and every code on it, answering whether anything was
    /// still live. Idempotent: ending an ended discount changes nothing.
    pub async fn end(&self, id: Uuid, at: Timestamp) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        let ended = sqlx::query!(
            "UPDATE discount SET ended_at = $2 WHERE id = $1 AND ended_at IS NULL",
            uuid_to_db(id),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query!(
            "UPDATE discount_code SET ended_at = $2 WHERE discount_id = $1 AND ended_at IS NULL",
            uuid_to_db(id),
            timestamp_to_db(at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ended == 1)
    }

    /// Whether a live code already spells `code`, case aside. Asked before
    /// Stripe is called, so a clash is refused without leaving an orphan
    /// promotion code behind; the unique index is still the fence.
    pub async fn code_taken(&self, code: &str) -> Result<bool, StorageError> {
        let taken = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM discount_code WHERE lower(code) = lower($1) AND ended_at IS NULL) AS "taken!""#,
            code,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(taken)
    }

    pub async fn insert_code(&self, code: &DiscountCode) -> Result<CodeWrite, StorageError> {
        let written = sqlx::query!(
            "INSERT INTO discount_code (id, discount_id, code, stripe_promotion_code_id, max_redemptions, ended_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (lower(code)) WHERE ended_at IS NULL DO NOTHING",
            uuid_to_db(code.id),
            uuid_to_db(code.discount_id),
            code.code,
            code.stripe_promotion_code_id,
            code.max_redemptions.map(|n| to_i32(n, "max_redemptions")).transpose()?,
            code.ended_at.map(timestamp_to_db).transpose()?,
            timestamp_to_db(code.created_at)?,
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(if written == 1 {
            CodeWrite::Stored
        } else {
            CodeWrite::Taken
        })
    }

    pub async fn codes(&self) -> Result<Vec<DiscountCode>, StorageError> {
        let rows = sqlx::query_as!(
            DiscountCodeRow,
            "SELECT id, discount_id, code, stripe_promotion_code_id, max_redemptions, ended_at, created_at FROM discount_code ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(DiscountCode::try_from).collect()
    }

    /// The live code spelling `code`, case aside, with its discount.
    pub async fn resolve_code(
        &self,
        code: &str,
    ) -> Result<Option<(DiscountCode, Discount)>, StorageError> {
        let Some(row) = sqlx::query_as!(
            DiscountCodeRow,
            "SELECT id, discount_id, code, stripe_promotion_code_id, max_redemptions, ended_at, created_at FROM discount_code WHERE lower(code) = lower($1) AND ended_at IS NULL",
            code,
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let code = DiscountCode::try_from(row)?;
        Ok(self.get(code.discount_id).await?.map(|discount| (code, discount)))
    }
}
