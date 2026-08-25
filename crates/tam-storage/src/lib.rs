//! Organisation-first repositories over PostgreSQL with row-level security.
//!
//! Every repository method takes the tenant as its first positional parameter
//! and pins it into the connection as a transaction-local `app.current_org`
//! setting, so the row-level-security policy in `migrations/0001_product.sql`
//! is a boundary a forgotten `WHERE` clause cannot cross. The queries still
//! filter by `org_id` explicitly; the policy is the backstop the tenancy
//! tests prove, not the only fence.

#![forbid(unsafe_code)]

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use tam_types::{ListingCopy, Money, OrgId, PriceIntent, ProductId, Timestamp, Title};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("timestamp {millis}ms is outside the representable range")]
    TimestampOutOfRange { millis: i64 },
    #[error("stored row violates a domain invariant: {reason}")]
    CorruptRow { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProduct {
    pub id: ProductId,
    pub title: Title,
    pub body: ListingCopy,
    pub price: PriceIntent,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductRecord {
    pub org: OrgId,
    pub id: ProductId,
    pub title: Title,
    pub body: ListingCopy,
    pub price: PriceIntent,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub struct ProductRepo {
    pool: PgPool,
}

impl ProductRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, org: OrgId, product: &NewProduct) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let price = PriceColumns::from_intent(product.price);
        sqlx::query!(
            "INSERT INTO product \
             (org_id, id, title, body, price_kind, price_minor_units, price_currency, \
              created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            uuid_to_db(org.0),
            uuid_to_db(product.id.0),
            product.title.0,
            product.body.body,
            price.kind,
            price.minor_units,
            price.currency,
            timestamp_to_db(product.created_at)?,
            timestamp_to_db(product.updated_at)?,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn get(
        &self,
        org: OrgId,
        id: ProductId,
    ) -> Result<Option<ProductRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_as!(
            ProductRow,
            "SELECT org_id, id, title, body, price_kind, price_minor_units, price_currency, \
             created_at, updated_at \
             FROM product \
             WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL",
            uuid_to_db(org.0),
            uuid_to_db(id.0),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(ProductRecord::try_from).transpose()
    }

    pub async fn list(&self, org: OrgId) -> Result<Vec<ProductRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query_as!(
            ProductRow,
            "SELECT org_id, id, title, body, price_kind, price_minor_units, price_currency, \
             created_at, updated_at \
             FROM product \
             WHERE org_id = $1 AND deleted_at IS NULL \
             ORDER BY created_at, id",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter().map(ProductRecord::try_from).collect()
    }
}

/// Declares the tenant for the rest of this transaction. `set_config` with
/// `is_local = true` resets at transaction end, so a pooled connection never
/// leaks one tenant's pin into the next request.
async fn pin_org(tx: &mut Transaction<'_, Postgres>, org: OrgId) -> Result<(), StorageError> {
    let org_text = uuid_to_db(org.0).to_string();
    sqlx::query!("SELECT set_config('app.current_org', $1, true)", org_text)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}

struct ProductRow {
    org_id: uuid::Uuid,
    id: uuid::Uuid,
    title: String,
    body: String,
    price_kind: String,
    price_minor_units: Option<i64>,
    price_currency: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ProductRow> for ProductRecord {
    type Error = StorageError;

    fn try_from(row: ProductRow) -> Result<Self, Self::Error> {
        Ok(Self {
            org: OrgId(uuid_from_db(row.org_id)),
            id: ProductId(uuid_from_db(row.id)),
            title: Title(row.title),
            body: ListingCopy { body: row.body },
            price: price_from_db(&row.price_kind, row.price_minor_units, row.price_currency)?,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        })
    }
}

const PRICE_KIND_FREE: &str = "free";
const PRICE_KIND_PAID: &str = "paid";
const CURRENCY_GBP: &str = "gbp";
const CURRENCY_USD: &str = "usd";

struct PriceColumns {
    kind: &'static str,
    minor_units: Option<i64>,
    currency: Option<&'static str>,
}

impl PriceColumns {
    fn from_intent(price: PriceIntent) -> Self {
        match price {
            PriceIntent::Free => Self {
                kind: PRICE_KIND_FREE,
                minor_units: None,
                currency: None,
            },
            PriceIntent::Paid(money) => Self {
                kind: PRICE_KIND_PAID,
                minor_units: Some(money.minor_units()),
                currency: Some(match money.currency() {
                    tam_types::Currency::Gbp => CURRENCY_GBP,
                    tam_types::Currency::Usd => CURRENCY_USD,
                }),
            },
        }
    }
}

fn price_from_db(
    kind: &str,
    minor_units: Option<i64>,
    currency: Option<String>,
) -> Result<PriceIntent, StorageError> {
    match (kind, minor_units, currency) {
        (k, None, None) if k == PRICE_KIND_FREE => Ok(PriceIntent::Free),
        (k, Some(units), Some(cur)) if k == PRICE_KIND_PAID => {
            let currency = match cur.as_str() {
                c if c == CURRENCY_GBP => tam_types::Currency::Gbp,
                c if c == CURRENCY_USD => tam_types::Currency::Usd,
                other => {
                    return Err(StorageError::CorruptRow {
                        reason: format!("unknown currency {other:?}"),
                    })
                }
            };
            Money::new(units, currency)
                .map(PriceIntent::Paid)
                .map_err(|_| StorageError::CorruptRow {
                    reason: format!("non-positive paid amount {units}"),
                })
        }
        (k, units, cur) => Err(StorageError::CorruptRow {
            reason: format!("inconsistent price columns ({k:?}, {units:?}, {cur:?})"),
        }),
    }
}

fn uuid_to_db(id: tam_types::Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

fn uuid_from_db(id: uuid::Uuid) -> tam_types::Uuid {
    tam_types::Uuid(id.into_bytes())
}

fn timestamp_to_db(at: Timestamp) -> Result<DateTime<Utc>, StorageError> {
    DateTime::<Utc>::from_timestamp_millis(at.0)
        .ok_or(StorageError::TimestampOutOfRange { millis: at.0 })
}

fn timestamp_from_db(at: DateTime<Utc>) -> Timestamp {
    Timestamp(at.timestamp_millis())
}
