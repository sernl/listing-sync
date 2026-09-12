//! Collections: a named, ordered set of resources the seller acts on
//! together.
//!
//! A second dimension beside labels, and the reason both exist is that they
//! answer different questions. A label is a word a resource carries and a
//! resource carries many; a collection is a list a seller assembled in an
//! order they chose, and the order is the whole point — "the autumn unit, in
//! teaching sequence" is not a filter. So membership is a row with a position
//! rather than a term on the product, and the write is a whole-set replace:
//! the console renders the list and sends it back, which is `set_for_product`'s
//! rule in [`crate::labels`] and for the same reason.
//!
//! Two reads carry more than the tables hold, and deliberately. The listing
//! answers a member count and the marketplaces its members are bound on,
//! because a list page showing neither would need one read per row to say
//! anything a seller can choose by; both are aggregates over the same join,
//! so the page costs three statements rather than three per collection.
//!
//! Every read filters `product.deleted_at IS NULL`. A product is soft-deleted,
//! so the membership row of a binned resource survives, and that is correct:
//! it is out of every collection while it is in the bin and back in its place
//! when it is restored, with nothing having been moved.

use sqlx::PgPool;
use tam_types::{InventoryId, OrgId, ProductId, Timestamp, Uuid};

use crate::codec::{
    inventory_from_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
};
use crate::{pin_org, Given, StorageError};

/// How many collections one organisation may keep.
///
/// A ceiling on the listing rather than on the rate, for
/// `TEMPLATES_PER_ORG_MAX`'s reason: [`ResourceCollectionRepo::list`] answers
/// whole, because a picker that arrives in pages is not a picker, and the bound
/// on the table is therefore the bound on that answer. The plan's own
/// `collections_max` sits under this on every plan but the unbounded one,
/// which is the plan this number exists for.
///
/// [`ResourceCollectionRepo::list`]: ResourceCollectionRepo::list
pub const COLLECTIONS_PER_ORG_MAX: i64 = 200;

/// The unique index migration 0072 folds collection names under, named here
/// because its violation is a write's own answer rather than a fault.
const ONE_PER_NAME: &str = "collection_one_per_name";

/// One collection as a list row renders it: its own fields, how many
/// resources it holds, and which marketplaces those resources are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSummary {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// Members whose resource is not in the bin, which is what the seller
    /// counts when they look at the collection.
    pub count: i64,
    /// The union, over those members, of the marketplaces they are bound on.
    pub inventories: Vec<InventoryId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// One collection's own row, which is what a write answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRecord {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// One member, in the order the seller put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionMemberRow {
    pub product: ProductId,
    pub title: String,
    pub position: i32,
    /// The marketplaces this resource is bound on.
    pub inventories: Vec<InventoryId>,
}

/// What a new collection carries. The identifier and the instant are the
/// caller's, so a test drives this with a fixed clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCollection<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub created_at: Timestamp,
}

/// Which of a collection's two parts an edit replaces.
///
/// The name cannot be cleared and the note can, which is why one is an option
/// and the other is [`Given`]: `Set(None)` is the seller deleting their note,
/// and a plain option would make that inexpressible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionEdit<'a> {
    name: Option<&'a str>,
    description: Given<&'a str>,
}

impl<'a> CollectionEdit<'a> {
    /// The edit these two optional parts describe, or `None` where they
    /// describe no edit at all — which must stay unwritable, because it would
    /// still move `updated_at` and misstate when the seller last touched the
    /// collection.
    #[must_use]
    pub const fn of(name: Option<&'a str>, description: Given<&'a str>) -> Option<Self> {
        if name.is_none() && !description.named() {
            return None;
        }
        Some(Self { name, description })
    }
}

/// What a create did. Neither refusal is an error variant, for
/// `TemplateWrite`'s reason: a name already in use and a full shelf are
/// answers a seller acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionWrite {
    Saved(CollectionRecord),
    /// This organisation already has a collection of that name, compared
    /// case-insensitively.
    NameTaken,
    /// This organisation holds [`COLLECTIONS_PER_ORG_MAX`] already.
    TooMany,
}

/// What an edit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionChange {
    Saved(CollectionRecord),
    Missing,
    NameTaken,
}

pub struct ResourceCollectionRepo {
    pool: PgPool,
}

impl ResourceCollectionRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every collection this organisation has, in the order the list shows
    /// them, with each one's live member count and marketplace marks.
    ///
    /// Three statements rather than one join: the count and the marketplace
    /// set are aggregates at different grains — one per collection and one per
    /// (collection, marketplace) — and one query answering both would either
    /// multiply the count by the marketplace cardinality or need a subselect
    /// per row.
    pub async fn list(&self, org: OrgId) -> Result<Vec<CollectionSummary>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, name, description, created_at, updated_at FROM collection \
             WHERE org_id = $1 ORDER BY lower(name)",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let counts = sqlx::query!(
            "SELECT cm.collection_id, count(*) AS \"held!\" \
               FROM collection_member cm \
               JOIN product p ON p.org_id = cm.org_id AND p.id = cm.product_id \
              WHERE cm.org_id = $1 AND p.deleted_at IS NULL \
              GROUP BY cm.collection_id",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let marks = sqlx::query!(
            "SELECT DISTINCT cm.collection_id, m.inventory \
               FROM collection_member cm \
               JOIN product p ON p.org_id = cm.org_id AND p.id = cm.product_id \
               JOIN mapping m ON m.org_id = cm.org_id AND m.product_id = cm.product_id \
              WHERE cm.org_id = $1 AND p.deleted_at IS NULL AND m.binding_state = 'bound' \
              ORDER BY cm.collection_id, m.inventory",
            org_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(CollectionSummary {
                    id: uuid_from_db(row.id),
                    name: row.name,
                    description: row.description,
                    count: counts
                        .iter()
                        .find(|held| held.collection_id == row.id)
                        .map_or(0, |held| held.held),
                    inventories: marks
                        .iter()
                        .filter(|mark| mark.collection_id == row.id)
                        .map(|mark| inventory_from_db(&mark.inventory))
                        .collect::<Result<Vec<_>, StorageError>>()?,
                    created_at: timestamp_from_db(row.created_at),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }

    /// One collection's own row, or `None` where this organisation has no such
    /// row — which is the same answer another organisation's collection gives,
    /// because the pin is what decides it.
    pub async fn get(
        &self,
        org: OrgId,
        id: Uuid,
    ) -> Result<Option<CollectionRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, name, description, created_at, updated_at FROM collection \
             WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.map(|row| CollectionRecord {
            id: uuid_from_db(row.id),
            name: row.name,
            description: row.description,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// One collection's members, in the seller's own order, each with the
    /// marketplaces it is bound on.
    ///
    /// Ordered by position and then by identifier, so a stored order two
    /// members happen to share still reads the same way twice.
    pub async fn members(
        &self,
        org: OrgId,
        collection: Uuid,
    ) -> Result<Vec<CollectionMemberRow>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let collection_db = uuid_to_db(collection);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT cm.product_id, cm.position, p.title \
               FROM collection_member cm \
               JOIN product p ON p.org_id = cm.org_id AND p.id = cm.product_id \
              WHERE cm.org_id = $1 AND cm.collection_id = $2 AND p.deleted_at IS NULL \
              ORDER BY cm.position, cm.product_id",
            org_db,
            collection_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        let marks = sqlx::query!(
            "SELECT m.product_id, m.inventory \
               FROM collection_member cm \
               JOIN mapping m ON m.org_id = cm.org_id AND m.product_id = cm.product_id \
              WHERE cm.org_id = $1 AND cm.collection_id = $2 AND m.binding_state = 'bound' \
              ORDER BY m.product_id, m.inventory",
            org_db,
            collection_db,
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(CollectionMemberRow {
                    product: ProductId(uuid_from_db(row.product_id)),
                    title: row.title,
                    position: row.position,
                    inventories: marks
                        .iter()
                        .filter(|mark| mark.product_id == row.product_id)
                        .map(|mark| inventory_from_db(&mark.inventory))
                        .collect::<Result<Vec<_>, StorageError>>()?,
                })
            })
            .collect()
    }

    /// Which collections hold one resource, in the order the list shows them.
    ///
    /// The reverse read `collection_member_by_product` exists for: the
    /// resource page asks it once per open.
    pub async fn for_product(
        &self,
        org: OrgId,
        product: ProductId,
    ) -> Result<Vec<CollectionRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT c.id, c.name, c.description, c.created_at, c.updated_at \
               FROM collection_member cm \
               JOIN collection c ON c.org_id = cm.org_id AND c.id = cm.collection_id \
              WHERE cm.org_id = $1 AND cm.product_id = $2 \
              ORDER BY lower(c.name)",
            uuid_to_db(org.0),
            uuid_to_db(product.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .map(|row| CollectionRecord {
                id: uuid_from_db(row.id),
                name: row.name,
                description: row.description,
                created_at: timestamp_from_db(row.created_at),
                updated_at: timestamp_from_db(row.updated_at),
            })
            .collect())
    }

    /// How many collections this organisation holds, which is what the plan's
    /// own allowance is measured against.
    pub async fn count(&self, org: OrgId) -> Result<i64, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = sqlx::query_scalar!(
            "SELECT count(*) AS \"total!\" FROM collection WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(held)
    }

    /// Saves one new collection, or says why it was not saved.
    ///
    /// The count is read in the same transaction as the insert, under the
    /// tenant pin, and the insert runs before the ceiling is answered, both
    /// for `ResourceTemplateRepo::create`'s reasons: a seller at the ceiling
    /// re-using a name they already hold is told the name is taken, which is
    /// the refusal they can act on.
    pub async fn create(
        &self,
        org: OrgId,
        collection: &NewCollection<'_>,
    ) -> Result<CollectionWrite, StorageError> {
        let at = timestamp_to_db(collection.created_at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = sqlx::query_scalar!(
            "SELECT count(*) AS \"total!\" FROM collection WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        let written = sqlx::query!(
            "INSERT INTO collection (org_id, id, name, description, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) \
             RETURNING id, name, description, created_at, updated_at",
            uuid_to_db(org.0),
            uuid_to_db(collection.id),
            collection.name,
            collection.description,
            at,
        )
        .fetch_one(&mut *tx)
        .await;
        let row = match written {
            Ok(row) => row,
            Err(sqlx::Error::Database(database)) if database.constraint() == Some(ONE_PER_NAME) => {
                return Ok(CollectionWrite::NameTaken)
            }
            Err(error) => return Err(error.into()),
        };
        if held >= COLLECTIONS_PER_ORG_MAX {
            // Rolled back rather than committed: the insert above ran only to
            // learn whether the name was free.
            return Ok(CollectionWrite::TooMany);
        }
        tx.commit().await?;
        Ok(CollectionWrite::Saved(CollectionRecord {
            id: uuid_from_db(row.id),
            name: row.name,
            description: row.description,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Renames a collection, replaces its note, or both.
    ///
    /// `GREATEST` rather than the caller's instant alone, because
    /// `collection_updated_after_created` is a CHECK and a wall clock that
    /// stepped backwards between the create and the edit would otherwise turn
    /// an ordinary rename into a fault.
    pub async fn update(
        &self,
        org: OrgId,
        id: Uuid,
        edit: &CollectionEdit<'_>,
        at: Timestamp,
    ) -> Result<CollectionChange, StorageError> {
        let at = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "UPDATE collection \
                SET name = COALESCE($3::text, name), \
                    description = CASE WHEN $4 THEN $5::text ELSE description END, \
                    updated_at = GREATEST($6, created_at) \
              WHERE org_id = $1 AND id = $2 \
             RETURNING id, name, description, created_at, updated_at",
            uuid_to_db(org.0),
            uuid_to_db(id),
            edit.name,
            edit.description.named(),
            edit.description.value(),
            at,
        )
        .fetch_optional(&mut *tx)
        .await;
        let row = match written {
            Ok(row) => row,
            Err(sqlx::Error::Database(database)) if database.constraint() == Some(ONE_PER_NAME) => {
                return Ok(CollectionChange::NameTaken)
            }
            Err(error) => return Err(error.into()),
        };
        let Some(row) = row else {
            return Ok(CollectionChange::Missing);
        };
        tx.commit().await?;
        Ok(CollectionChange::Saved(CollectionRecord {
            id: uuid_from_db(row.id),
            name: row.name,
            description: row.description,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Replaces a collection's membership with this ordered list, answering
    /// whether there was a collection to replace it on.
    ///
    /// A whole-set replace in one transaction, so nobody reads a collection
    /// holding half of a reorder. The position is the caller's index, which is
    /// what makes the wire shape a list rather than a list of pairs: an order
    /// the client states twice is an order two clients can disagree about.
    ///
    /// Every product named must already be this organisation's; the caller
    /// resolves the selection against the catalogue first, which is where a
    /// stale tab naming a deleted resource is dropped rather than refused.
    pub async fn set_members(
        &self,
        org: OrgId,
        collection: Uuid,
        products: &[ProductId],
        at: Timestamp,
    ) -> Result<bool, StorageError> {
        let org_db = uuid_to_db(org.0);
        let collection_db = uuid_to_db(collection);
        let at_db = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        // The touch is what decides whether there was a collection at all, so
        // the membership write below cannot land on a row that does not exist.
        let present = sqlx::query!(
            "UPDATE collection SET updated_at = GREATEST($3, created_at) \
              WHERE org_id = $1 AND id = $2 RETURNING id",
            org_db,
            collection_db,
            at_db,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if present.is_none() {
            return Ok(false);
        }
        sqlx::query!(
            "DELETE FROM collection_member WHERE org_id = $1 AND collection_id = $2",
            org_db,
            collection_db,
        )
        .execute(&mut *tx)
        .await?;
        for (index, product) in products.iter().enumerate() {
            sqlx::query!(
                "INSERT INTO collection_member \
                     (org_id, collection_id, product_id, position, added_at) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (org_id, collection_id, product_id) DO UPDATE \
                     SET position = excluded.position",
                org_db,
                collection_db,
                uuid_to_db(product.0),
                i32::try_from(index).unwrap_or(i32::MAX),
                at_db,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Removes one collection and its membership, answering whether there was
    /// one to remove, so a client that asks twice is told the second time
    /// rather than shown a success that did nothing.
    ///
    /// The membership goes with it by cascade; no resource is touched, because
    /// a collection is a way of looking at the catalogue and not part of it.
    pub async fn delete(&self, org: OrgId, id: Uuid) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let removed = sqlx::query!(
            "DELETE FROM collection WHERE org_id = $1 AND id = $2 RETURNING id",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(removed.is_some())
    }
}
