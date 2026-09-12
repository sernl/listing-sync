//! Named starting points for a new resource: a partial create-form draft an
//! organisation saved to fill the next one in with.
//!
//! One repository, one tenant pin, five statements. The draft crosses this
//! boundary as an opaque [`serde_json::Value`] rather than as a typed draft,
//! and that is deliberate: the shape it must satisfy is the create form's,
//! which `tam-api` already owns and validates against `tam_authoring`, and a
//! second definition here is a second thing to keep in step with a form that
//! moves when TPT's does. What this module guarantees is the tenancy, the
//! naming and the ceilings; what the document says is the API's to decide
//! before it arrives.
//!
//! Two write outcomes rather than one, and neither is an error variant: a name
//! already in use and a full shelf are answers a seller acts on, and a
//! `Result` carrying them would put them on the same footing as a lost
//! connection.

use sqlx::PgPool;
use tam_types::{InventoryId, OrgId, Timestamp, Uuid};

use crate::codec::{
    inventory_from_db, inventory_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db,
    uuid_to_db,
};
use crate::{pin_org, Given, StorageError};

/// How many templates one organisation may keep.
///
/// A ceiling on the listing rather than on the rate, because [`list`] answers
/// whole: it has no cursor and no page, for the reason `LabelRepo::list` has
/// none — a seller picks a template from a menu, and a menu that arrives in
/// pages is not a menu. So the bound on the table is the bound on that answer,
/// and it has to exist somewhere. A hundred is far past a seller with a
/// template per unit type and far short of a response nobody can read.
///
/// [`list`]: ResourceTemplateRepo::list
pub const TEMPLATES_PER_ORG_MAX: i64 = 100;

/// The unique index migration 0056 folds template names under, named here
/// because its violation is a write's own answer rather than a fault.
const ONE_PER_NAME: &str = "resource_template_one_per_name";

/// One template as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTemplateRecord {
    pub id: Uuid,
    pub name: String,
    /// The seller's own note about when to reach for this template, which is
    /// not the resource description the draft carries.
    pub description: Option<String>,
    /// The marketplace this template is written for, or `None` for one that
    /// is written for none.
    pub scope: Option<InventoryId>,
    /// The partial `DraftInput` this template prefills a form with, verbatim.
    pub draft: serde_json::Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// One template without its draft, which is what a picker needs.
///
/// A separate type rather than a record whose draft is sometimes absent,
/// because the listing and the fetch answer different questions and a shared
/// shape with a hole in it makes every reader check which one it holds. The
/// listing is unpaged, so the draft's own ceiling would otherwise multiply by
/// [`TEMPLATES_PER_ORG_MAX`]: a tenant that filled its shelf with drafts at the
/// API's own byte cap would make every picker open serialise several megabytes,
/// which is a hundred writes buying an unbounded number of expensive reads.
///
/// The note and the scope do travel, unlike the draft: they are one short
/// string and one token, and they are what a picker row has to render for the
/// seller to choose between two templates without opening either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTemplateSummary {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub scope: Option<InventoryId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// What a new template carries. The identifier and the instant are the
/// caller's, so a test drives this with a fixed clock and no row is stamped
/// from an ambient one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewResourceTemplate<'a> {
    pub id: Uuid,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub scope: Option<InventoryId>,
    pub draft: &'a serde_json::Value,
    pub created_at: Timestamp,
}

/// Which of a template's four parts an edit replaces.
///
/// A struct with a private constructor rather than the sum this was while a
/// template had two parts: four parts make fifteen non-empty combinations, and
/// a sum over them is a type nobody can read. What the sum bought is kept by
/// [`TemplateEdit::of`] answering `None` for an edit that names nothing —
/// which still has to be impossible to write, because it would move
/// `updated_at` and misstate when the seller last touched the template.
///
/// The two nullable parts are [`Given`], and that is the whole point: `Kept`
/// leaves the stored value alone and `Set(None)` clears it. A plain option
/// would make "the seller deleted their note" inexpressible, which is the same
/// mistake as merging a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateEdit<'a> {
    name: Option<&'a str>,
    draft: Option<&'a serde_json::Value>,
    description: Given<&'a str>,
    scope: Given<InventoryId>,
}

impl<'a> TemplateEdit<'a> {
    /// The edit these four optional parts describe, or `None` where they
    /// describe no edit at all.
    #[must_use]
    pub const fn of(
        name: Option<&'a str>,
        draft: Option<&'a serde_json::Value>,
        description: Given<&'a str>,
        scope: Given<InventoryId>,
    ) -> Option<Self> {
        if name.is_none() && draft.is_none() && !description.named() && !scope.named() {
            return None;
        }
        Some(Self {
            name,
            draft,
            description,
            scope,
        })
    }
}

/// What a create did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateWrite {
    Saved(ResourceTemplateRecord),
    /// This organisation already has a template of that name, compared
    /// case-insensitively.
    NameTaken,
    /// This organisation holds [`TEMPLATES_PER_ORG_MAX`] templates already.
    TooMany,
}

/// What an edit did. `TooMany` is absent because an edit adds no row, and
/// `Missing` is present because an identifier naming nothing and a name
/// already in use are different answers that both move zero rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateChange {
    Saved(ResourceTemplateRecord),
    Missing,
    NameTaken,
}

pub struct ResourceTemplateRepo {
    pool: PgPool,
}

impl ResourceTemplateRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every template this organisation has, in the order the picker shows
    /// them, without their drafts.
    ///
    /// Whole rather than paged: [`TEMPLATES_PER_ORG_MAX`] is what bounds the
    /// row count, and the folded-name index migration 0056 creates is what
    /// serves the order without a second one. The draft is left in the column
    /// rather than carried here, because the row count alone does not bound a
    /// response whose every row may hold sixty-four kilobytes; [`get`] is what
    /// fetches one whole, once the seller has chosen it.
    ///
    /// [`get`]: ResourceTemplateRepo::get
    pub async fn list(&self, org: OrgId) -> Result<Vec<ResourceTemplateSummary>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let rows = sqlx::query!(
            "SELECT id, name, description, scope, created_at, updated_at FROM resource_template \
             WHERE org_id = $1 ORDER BY lower(name)",
            uuid_to_db(org.0),
        )
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|row| {
                Ok(ResourceTemplateSummary {
                    id: uuid_from_db(row.id),
                    name: row.name,
                    description: row.description,
                    scope: row.scope.as_deref().map(inventory_from_db).transpose()?,
                    created_at: timestamp_from_db(row.created_at),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }

    /// One template by identifier, or `None` where this organisation has no
    /// such row — which is the same answer a template belonging to another
    /// organisation gives, because the pin is what decides it.
    pub async fn get(
        &self,
        org: OrgId,
        id: Uuid,
    ) -> Result<Option<ResourceTemplateRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, name, description, scope, draft, created_at, updated_at \
             FROM resource_template WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(ResourceTemplateRecord {
            id: uuid_from_db(row.id),
            name: row.name,
            description: row.description,
            scope: row.scope.as_deref().map(inventory_from_db).transpose()?,
            draft: row.draft,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Saves one new template, or says why it was not saved.
    ///
    /// The count is read in the same transaction as the insert, under the
    /// tenant pin, so it is this organisation's own. The name refusal is the
    /// unique index's rather than a read before the write, so two creates
    /// racing onto one name cannot both be told it is free.
    ///
    /// The residual is the cap's own race: every create in flight reads the
    /// count before any of them commits, so a tenant issuing a hundred parallel
    /// creates against a shelf holding ninety-nine can reach a hundred and
    /// ninety-nine. The overshoot is bounded by concurrency rather than by one
    /// row, and a second burst gains nothing, because every later create then
    /// reads a count at or above the ceiling. Serialising every write on this
    /// table to close that would spend a real fence on a bound that already
    /// holds within a constant factor.
    ///
    /// `updated_at` starts equal to `created_at`: a template nobody has edited
    /// was last changed when it was made, and a null would make every reader
    /// restate that.
    pub async fn create(
        &self,
        org: OrgId,
        template: &NewResourceTemplate<'_>,
    ) -> Result<TemplateWrite, StorageError> {
        let at = timestamp_to_db(template.created_at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let held = sqlx::query_scalar!(
            "SELECT count(*) AS \"total!\" FROM resource_template WHERE org_id = $1",
            uuid_to_db(org.0),
        )
        .fetch_one(&mut *tx)
        .await?;
        // The insert runs before the ceiling is answered, so that a seller at
        // exactly the ceiling who re-saves a name they already hold is told the
        // name is taken rather than that the shelf is full. Both are true and
        // only the first is one they can act on: clearing space would not let
        // that write land.
        let written = sqlx::query!(
            "INSERT INTO resource_template \
             (org_id, id, name, description, scope, draft, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             RETURNING id, name, description, scope, draft, created_at, updated_at",
            uuid_to_db(org.0),
            uuid_to_db(template.id),
            template.name,
            template.description,
            template.scope.map(inventory_to_db),
            template.draft,
            at,
        )
        .fetch_one(&mut *tx)
        .await;
        let row = match written {
            Ok(row) => row,
            Err(sqlx::Error::Database(database)) if database.constraint() == Some(ONE_PER_NAME) => {
                return Ok(TemplateWrite::NameTaken)
            }
            Err(error) => return Err(error.into()),
        };
        if held >= TEMPLATES_PER_ORG_MAX {
            // Rolled back rather than committed: the insert above ran only to
            // learn whether the name was free.
            return Ok(TemplateWrite::TooMany);
        }
        tx.commit().await?;
        Ok(TemplateWrite::Saved(ResourceTemplateRecord {
            id: uuid_from_db(row.id),
            name: row.name,
            description: row.description,
            scope: row.scope.as_deref().map(inventory_from_db).transpose()?,
            draft: row.draft,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Replaces any of a template's four parts.
    ///
    /// A part the caller leaves absent keeps the value it has, expressed in
    /// one statement rather than as a read followed by a write: two edits
    /// arriving together then interleave at the row rather than each
    /// overwriting the other's whole record with a copy it read beforehand.
    /// `COALESCE` serves the two parts that cannot be cleared and a `CASE` on
    /// a given-flag serves the two that can, because `COALESCE` cannot tell a
    /// null meaning "leave it" from a null meaning "clear it".
    ///
    /// The draft is replaced whole rather than merged, because the console
    /// renders every field and sends them all back: a merge would make
    /// clearing a field impossible to express.
    ///
    /// `GREATEST` rather than the caller's instant alone, because
    /// `resource_template_updated_after_created` is a CHECK and a wall clock
    /// that steps backwards between a create and an edit would otherwise turn
    /// an ordinary rename into a fault. Clamped to the row's own creation
    /// instant, which reads as "last touched when it was made" — the one
    /// statement about a backwards clock that is not a lie.
    pub async fn update(
        &self,
        org: OrgId,
        id: Uuid,
        edit: &TemplateEdit<'_>,
        at: Timestamp,
    ) -> Result<TemplateChange, StorageError> {
        let at = timestamp_to_db(at)?;
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let written = sqlx::query!(
            "UPDATE resource_template \
                SET name = COALESCE($3::text, name), \
                    draft = COALESCE($4::jsonb, draft), \
                    description = CASE WHEN $5 THEN $6::text ELSE description END, \
                    scope = CASE WHEN $7 THEN $8::text ELSE scope END, \
                    updated_at = GREATEST($9, created_at) \
              WHERE org_id = $1 AND id = $2 \
             RETURNING id, name, description, scope, draft, created_at, updated_at",
            uuid_to_db(org.0),
            uuid_to_db(id),
            edit.name,
            edit.draft,
            edit.description.named(),
            edit.description.value(),
            edit.scope.named(),
            edit.scope.value().map(inventory_to_db),
            at,
        )
        .fetch_optional(&mut *tx)
        .await;
        let row = match written {
            Ok(row) => row,
            Err(sqlx::Error::Database(database)) if database.constraint() == Some(ONE_PER_NAME) => {
                return Ok(TemplateChange::NameTaken)
            }
            Err(error) => return Err(error.into()),
        };
        let Some(row) = row else {
            return Ok(TemplateChange::Missing);
        };
        tx.commit().await?;
        Ok(TemplateChange::Saved(ResourceTemplateRecord {
            id: uuid_from_db(row.id),
            name: row.name,
            description: row.description,
            scope: row.scope.as_deref().map(inventory_from_db).transpose()?,
            draft: row.draft,
            created_at: timestamp_from_db(row.created_at),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Removes one template, answering whether there was one to remove, so a
    /// client that asks twice is told the second time rather than shown a
    /// success that did nothing.
    ///
    /// Nothing references a template, so nothing cascades: it prefills a form
    /// and the resource that form creates carries no trace of which template
    /// filled it.
    pub async fn delete(&self, org: OrgId, id: Uuid) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let removed = sqlx::query!(
            "DELETE FROM resource_template WHERE org_id = $1 AND id = $2 RETURNING id",
            uuid_to_db(org.0),
            uuid_to_db(id),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(removed.is_some())
    }
}
