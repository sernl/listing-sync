//! The help guides: one global document set, written by an operator and read
//! by every seller.
//!
//! Everything here runs on the application pool, which owns the table by
//! owning the database (db/init/01-app-role.sql), so the write path needs no
//! grant and the read path needs no second connection. The backoffice role is
//! granted nothing on `guide` (migration 0073): a guide is not one tenant's
//! data, so there is no fence for a cross-tenant reader to cross.
//!
//! No method here takes an organisation, and none can: `guide` carries no
//! `org_id`. The one organisation this module does name is the reserved
//! platform one that owns guide pictures, and it is named by slug rather than
//! by a compiled-in identifier so a database that has one already keeps it.

use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::StorageError;

/// Whether a guide is visible to sellers.
///
/// Two states and no third: a guide is being written or it is published.
/// Withdrawal is a return to `Draft`, which is why there is no `Archived` —
/// an unpublished guide reads exactly as one that was never published, and a
/// third state would be a second way of spelling the same visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideStatus {
    Draft,
    Published,
}

impl GuideStatus {
    /// The column's spelling, which is also the wire's: the API passes this
    /// string through rather than translating it, so the check constraint,
    /// the JSON and the console's union are one vocabulary.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }

    /// Reads the column back, or `None` for a spelling the constraint would
    /// not have accepted.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            _ => None,
        }
    }
}

/// One guide whole, body included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideRecord {
    pub slug: String,
    pub title: String,
    /// Markdown as the operator typed it. The rendering is the API's, done on
    /// the way out, so a guide is never stored as the HTML of the day it was
    /// saved.
    pub body: String,
    pub status: GuideStatus,
    pub updated_by: Option<UserId>,
    pub updated_at: Timestamp,
}

/// One guide without its body: what a listing renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideHead {
    pub slug: String,
    pub title: String,
    pub status: GuideStatus,
    pub updated_by: Option<UserId>,
    pub updated_at: Timestamp,
}

/// What one write says a guide now is: everything about it except where it
/// lives.
///
/// The slug is not here because it is the address rather than a field. A
/// create takes one beside this ([`NewGuide`]); an edit addresses the guide
/// by the slug it already has and never moves it, since renaming would break
/// every link already pointing at the guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuideEdit<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub status: GuideStatus,
    pub updated_by: UserId,
    pub at: Timestamp,
}

/// A guide as it is first written: an address and what goes at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewGuide<'a> {
    pub slug: &'a str,
    pub edit: GuideEdit<'a>,
}

/// What a create settled on.
///
/// A slug another guide already holds is an ordinary answer the caller turns
/// into a sentence, not a fault, for the reason [`crate::org::OrgWrite`]
/// gives: checking before the write is a race and the unique index is the
/// arbiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideWrite {
    Stored,
    SlugTaken,
}

/// The slug of the reserved organisation that owns guide pictures.
///
/// Refused to every tenant by the validator in `crates/tam-api/src/org.rs`
/// since slugs existed, so nothing can be holding it when the first upload
/// asks for it.
pub const PLATFORM_ORG_SLUG: &str = "guides";

/// The name that organisation carries. Ours rather than a tenant's, and it is
/// the one an operator sees if they ever read the row.
const PLATFORM_ORG_NAME: &str = "Teachouse";

pub struct GuideRepo {
    pool: PgPool,
}

impl GuideRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Every guide, newest edit first: the operator's listing.
    pub async fn list(&self) -> Result<Vec<GuideHead>, StorageError> {
        self.heads(false).await
    }

    /// Published guides only, newest edit first: the seller's listing.
    pub async fn published(&self) -> Result<Vec<GuideHead>, StorageError> {
        self.heads(true).await
    }

    /// The two listings, which differ by one predicate and nothing else.
    ///
    /// One statement with the predicate as a parameter rather than two
    /// statements: the columns, the decode and the ordering are the whole
    /// listing, and a second copy of them is a second thing to keep in step
    /// for the sake of one `WHERE` clause.
    async fn heads(&self, published_only: bool) -> Result<Vec<GuideHead>, StorageError> {
        let rows = sqlx::query!(
            "SELECT slug, title, status, updated_by, updated_at \
             FROM guide WHERE NOT $1 OR status = 'published' \
             ORDER BY updated_at DESC, slug",
            published_only,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(GuideHead {
                    slug: row.slug,
                    title: row.title,
                    status: status_from_db(&row.status)?,
                    updated_by: row.updated_by.map(|id| UserId(uuid_from_db(id))),
                    updated_at: timestamp_from_db(row.updated_at),
                })
            })
            .collect()
    }

    /// One guide by its slug, whatever its status. The operator's read; the
    /// seller's route filters on the status it got back rather than asking a
    /// different question, so a draft and a slug nobody has used answer the
    /// reader identically.
    pub async fn get(&self, slug: &str) -> Result<Option<GuideRecord>, StorageError> {
        let row = sqlx::query!(
            "SELECT slug, title, body, status, updated_by, updated_at \
             FROM guide WHERE slug = $1",
            slug,
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let status = status_from_db(&row.status)?;
        Ok(Some(GuideRecord {
            slug: row.slug,
            title: row.title,
            body: row.body,
            status,
            updated_by: row.updated_by.map(|id| UserId(uuid_from_db(id))),
            updated_at: timestamp_from_db(row.updated_at),
        }))
    }

    /// Writes a guide that does not exist yet.
    pub async fn create(&self, new: &NewGuide<'_>) -> Result<GuideWrite, StorageError> {
        let at = timestamp_to_db(new.edit.at)?;
        let done = sqlx::query!(
            "INSERT INTO guide \
             (id, slug, title, body, status, updated_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
            uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
            new.slug,
            new.edit.title,
            new.edit.body,
            new.edit.status.as_str(),
            uuid_to_db(new.edit.updated_by.0),
            at,
        )
        .execute(&self.pool)
        .await;
        match done {
            Ok(_) => Ok(GuideWrite::Stored),
            Err(sqlx::Error::Database(database)) if database.constraint() == Some("guide_slug") => {
                Ok(GuideWrite::SlugTaken)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Rewrites an existing guide, answering whether there was one to
    /// rewrite.
    pub async fn update(&self, slug: &str, edit: &GuideEdit<'_>) -> Result<bool, StorageError> {
        let done = sqlx::query!(
            "UPDATE guide SET title = $2, body = $3, status = $4, updated_by = $5, \
                 updated_at = $6 \
             WHERE slug = $1",
            slug,
            edit.title,
            edit.body,
            edit.status.as_str(),
            uuid_to_db(edit.updated_by.0),
            timestamp_to_db(edit.at)?,
        )
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Deletes a guide, answering whether there was one to delete.
    pub async fn delete(&self, slug: &str) -> Result<bool, StorageError> {
        let done = sqlx::query!("DELETE FROM guide WHERE slug = $1", slug)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }
}

fn status_from_db(raw: &str) -> Result<GuideStatus, StorageError> {
    GuideStatus::parse(raw).ok_or_else(|| StorageError::CorruptRow {
        reason: format!("unknown guide status {raw:?}"),
    })
}

/// The organisation guide pictures belong to, created if this database has
/// none yet.
///
/// Looked up by slug rather than by a compiled-in identifier, so a database
/// that already carries the row keeps it whatever its id is. The insert names
/// the slug the API reserves and `ON CONFLICT DO NOTHING` makes two uploads
/// racing produce one organisation rather than one error.
///
/// Deliberately not called by a read: a `GET` that created a tenant would be
/// a write dressed as a read. The reader asks [`platform_org`] instead and
/// answers "no such picture" where nobody has uploaded one.
pub async fn ensure_platform_org(pool: &PgPool, at: Timestamp) -> Result<OrgId, StorageError> {
    sqlx::query!(
        "INSERT INTO organisation (id, name, slug, slug_deferred, created_at) \
         VALUES ($1, $2, $3, false, $4) ON CONFLICT DO NOTHING",
        uuid_to_db(Uuid(*uuid::Uuid::new_v4().as_bytes())),
        PLATFORM_ORG_NAME,
        PLATFORM_ORG_SLUG,
        timestamp_to_db(at)?,
    )
    .execute(pool)
    .await?;
    let row = sqlx::query!(
        "SELECT id FROM organisation WHERE slug = $1",
        PLATFORM_ORG_SLUG,
    )
    .fetch_one(pool)
    .await?;
    Ok(OrgId(uuid_from_db(row.id)))
}

/// The organisation guide pictures belong to, or `None` where no upload has
/// created it yet.
pub async fn platform_org(pool: &PgPool) -> Result<Option<OrgId>, StorageError> {
    let row = sqlx::query!(
        "SELECT id FROM organisation WHERE slug = $1",
        PLATFORM_ORG_SLUG,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| OrgId(uuid_from_db(row.id))))
}
