//! Seller-defined labels: the organisation's own vocabulary, and which items
//! carry which.
//!
//! Labels are named rather than identified by the client. A seller types a
//! word; the same word is the same label across their catalogue, which is what
//! makes filtering by it mean anything. So a write names labels by text and
//! this module resolves each to a row, minting one the first time it is seen.
//! The client never learns a label identifier and never has to.

use sqlx::PgPool;
use tam_types::{OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// One label as the console renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRecord {
    pub name: String,
    pub colour: Colour,
}

/// The closed palette migration 0046 constrains the column to.
///
/// Closed so the console's rendering is total: a colour added here fails the
/// web lane rather than reaching a seller as an unstyled chip. Which colour a
/// label gets is derived from its name rather than chosen, because the write
/// surface names labels by text and has nowhere for a seller to pick one; a
/// derived colour still gives the visual scanning coloured labels exist for,
/// and it is stable, so one label looks the same everywhere it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    Slate,
    Red,
    Amber,
    Green,
    Teal,
    Blue,
    Violet,
    Pink,
}

impl Colour {
    pub const ALL: [Self; 8] = [
        Self::Slate,
        Self::Red,
        Self::Amber,
        Self::Green,
        Self::Teal,
        Self::Blue,
        Self::Violet,
        Self::Pink,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Slate => "slate",
            Self::Red => "red",
            Self::Amber => "amber",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Blue => "blue",
            Self::Violet => "violet",
            Self::Pink => "pink",
        }
    }

    /// The colour a stored row names, or `None` for a value outside the closed
    /// set. Named for the column rather than as `from_str`, which would shadow
    /// the standard trait method of that name.
    #[must_use]
    pub fn from_column(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == raw)
    }

    /// The colour a label's own text earns it.
    ///
    /// Case-folded first, so "Autumn term" and "autumn term" — which
    /// `label_one_per_name` already treats as one label — cannot disagree
    /// about their colour if one is ever written before the other.
    #[must_use]
    pub fn of_name(name: &str) -> Self {
        let sum = name.to_lowercase().bytes().fold(0u32, |sum, byte| {
            sum.wrapping_mul(31).wrapping_add(byte.into())
        });
        let index = (sum % 8) as usize;
        Self::ALL[index]
    }
}

/// How a label's text is compared and stored.
///
/// Trimmed because trailing space is invisible and would mint a second label
/// nobody could tell from the first. Not case-folded: the seller's own
/// capitalisation is what they see, and `label_one_per_name` is what stops two
/// spellings existing at once.
#[must_use]
pub fn normalise(name: &str) -> String {
    name.trim().to_owned()
}

pub struct LabelRepo {
    pool: PgPool,
}

impl LabelRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every label this organisation has, in the order a person reads them.
    pub async fn list(&self, org: OrgId) -> Result<Vec<LabelRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT name, colour FROM label WHERE org_id = $1 ORDER BY lower(name)",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| decode(row.name, &row.colour))
            .collect()
    }

    /// The labels one item carries.
    pub async fn for_product(
        &self,
        org: OrgId,
        product: ProductId,
    ) -> Result<Vec<LabelRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT l.name, l.colour FROM product_label pl \
             JOIN label l ON l.org_id = pl.org_id AND l.id = pl.label_id \
             WHERE pl.org_id = $1 AND pl.product_id = $2 \
             ORDER BY lower(l.name)",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| decode(row.name, &row.colour))
            .collect()
    }

    /// Replaces the labels one item carries, minting any the organisation has
    /// not used before.
    ///
    /// Replace rather than merge, because the console renders the whole set and
    /// sends it back: a merge would make removing the last label impossible to
    /// express. The whole thing is one transaction, so an item is never seen
    /// carrying half of an edit.
    pub async fn set_for_product(
        &self,
        org: OrgId,
        product: ProductId,
        names: &[String],
        at: Timestamp,
    ) -> Result<Vec<LabelRecord>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let product_db = uuid_to_db(product.0);
        let at_db = timestamp_to_db(at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let detached = sqlx::query_scalar!(
            "DELETE FROM product_label WHERE org_id = $1 AND product_id = $2 \
             RETURNING label_id",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;

        for name in names {
            // The insert is the resolution: the unique index on the folded name
            // decides whether this is a label the organisation already has, so
            // two requests naming a new label at once agree on one row rather
            // than racing to create two.
            let label = sqlx::query!(
                "INSERT INTO label (org_id, id, name, colour, created_at) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (org_id, lower(name)) DO UPDATE SET name = label.name \
                 RETURNING id",
                org_db,
                uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
                name.as_str(),
                Colour::of_name(name).as_str(),
                at_db,
            )
            .fetch_one(&mut *tx)
            .await?;

            sqlx::query!(
                "INSERT INTO product_label (org_id, product_id, label_id, applied_at) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                org_db,
                product_db,
                label.id,
                at_db,
            )
            .execute(&mut *tx)
            .await?;
        }

        sweep_abandoned(&mut tx, org, &detached).await?;

        let rows = sqlx::query!(
            "SELECT l.name, l.colour FROM product_label pl \
             JOIN label l ON l.org_id = pl.org_id AND l.id = pl.label_id \
             WHERE pl.org_id = $1 AND pl.product_id = $2 \
             ORDER BY lower(l.name)",
            org_db,
            product_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| decode(row.name, &row.colour))
            .collect()
    }

    /// Gives one label a new name, keeping every item that carries it.
    ///
    /// One statement on the `label` row rather than a move between two rows:
    /// `product_label` names the label by identifier, so the carriers follow
    /// the rename without being visited and there is no window in which an
    /// item carries neither name.
    ///
    /// The colour is recomputed, because it is derived from the name rather
    /// than chosen. Keeping the old one would leave a label whose colour no
    /// name explains, and the next writer to mint that name elsewhere would
    /// disagree with this row about what colour it is.
    ///
    /// The refusal is the unique index's rather than a read before the write,
    /// so two renames racing onto one name cannot both be told it is free.
    pub async fn rename(
        &self,
        org: OrgId,
        from: &str,
        to: &str,
    ) -> Result<LabelRename, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let renamed = sqlx::query!(
            "UPDATE label SET name = $3, colour = $4 \
              WHERE org_id = $1 AND lower(name) = lower($2) \
             RETURNING name, colour",
            uuid_to_db(org.0),
            from,
            to,
            Colour::of_name(to).as_str(),
        )
        .fetch_optional(&mut *tx)
        .await;
        let row = match renamed {
            Ok(row) => row,
            Err(sqlx::Error::Database(database)) if database.constraint() == Some(ONE_PER_NAME) => {
                return Ok(LabelRename::Taken)
            }
            Err(error) => return Err(error.into()),
        };
        let Some(row) = row else {
            return Ok(LabelRename::Missing);
        };
        let record = decode(row.name, &row.colour)?;
        tx.commit().await?;
        Ok(LabelRename::Renamed(record))
    }

    /// Removes one label from this organisation's vocabulary, and with it from
    /// every item carrying it.
    ///
    /// The carriers go by `product_label`'s cascade, which migration 0046
    /// argues for: the alternative is a filter offering a label no item can be
    /// found by, or an item carrying a label with no name.
    ///
    /// Answers whether a label of that name was there to remove, so a client
    /// that asks twice is told the second time rather than shown a success
    /// that did nothing.
    pub async fn delete(&self, org: OrgId, name: &str) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let removed = sqlx::query!(
            "DELETE FROM label WHERE org_id = $1 AND lower(name) = lower($2) RETURNING id",
            uuid_to_db(org.0),
            name,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(removed.is_some())
    }
}

/// The unique index migration 0046 folds label names under, named here because
/// its violation is the rename's own refusal rather than a fault.
const ONE_PER_NAME: &str = "label_one_per_name";

/// What a rename did, which the caller must branch on rather than infer from
/// a row count: a name nobody holds and a name somebody else holds are
/// different answers to the seller, and both move zero rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelRename {
    /// The label carries the new name, and every item that carried it still
    /// does.
    Renamed(LabelRecord),
    /// This organisation has no label of the old name.
    Missing,
    /// Another label of this organisation already carries the new name.
    Taken,
}

/// Drops the labels this write left on no item at all.
///
/// Narrowed to the labels the caller just detached rather than sweeping the
/// organisation, because an organisation-wide sweep can strip a label another
/// request attached between that request's insert and this delete: the two
/// transactions do not see each other's uncommitted rows, so the sweep's
/// `NOT EXISTS` can be true for a label that is about to have a carrier. The
/// narrow form still races on exactly the labels this write touched, which is
/// the residual noted in the G6 entry: two writes abandoning and re-attaching
/// one label at the same instant can leave it deleted with a carrier, and the
/// foreign key's cascade then removes that carrier's attachment rather than
/// leaving a dangling row. The cost of that race is a label a seller retypes;
/// the cost of taking a lock wide enough to prevent it is every label write
/// serialising behind every other.
pub(crate) async fn sweep_abandoned(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    detached: &[uuid::Uuid],
) -> Result<(), StorageError> {
    if detached.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        "DELETE FROM label WHERE org_id = $1 AND id = ANY($2) \
         AND NOT EXISTS (SELECT 1 FROM product_label pl \
                         WHERE pl.org_id = label.org_id AND pl.label_id = label.id)",
        uuid_to_db(org.0),
        detached,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// One stored row as the console renders it, refusing a colour outside the
/// closed set rather than rendering an unstyled chip.
fn decode(name: String, colour: &str) -> Result<LabelRecord, StorageError> {
    Ok(LabelRecord {
        name,
        colour: Colour::from_column(colour).ok_or(StorageError::CorruptRow {
            reason: format!("label colour {colour:?} is not one of the closed set"),
        })?,
    })
}
