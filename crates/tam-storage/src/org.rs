//! The organisation row itself: the name the tenant carries and the slug it
//! claims, which the settings surface reads and writes.
//!
//! `organisation` is the tenancy root and carries no row-level-security
//! policy, unlike every table that references it. It cannot: the signup path
//! inserts the row before any `app.current_org` exists to pin, and the
//! engine's cross-tenant scan reads the table to enumerate tenants. So the
//! `WHERE id = $1` clause below is the whole tenant fence here rather than
//! the redundant filter it is in the repositories the policies also guard.

use sqlx::PgPool;
use tam_types::OrgId;

use crate::codec::{uuid_from_db, uuid_to_db};
use crate::StorageError;

/// One organisation as the settings surface sees it. The id is read back
/// from the row rather than echoed from the argument, so a read that somehow
/// answered with another tenant's row would be visible rather than masked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgRecord {
    pub id: OrgId,
    pub name: String,
    /// The handle the seller claimed, or `None` while they have claimed none.
    pub slug: Option<String>,
    /// Whether this organisation is allowed to carry no slug, which migration
    /// 0057 set true for every row that predated it and false for every row
    /// since. It separates an organisation that has not been asked yet from
    /// one that was asked and is mid-work; the two are otherwise identical.
    pub slug_deferred: bool,
}

/// What a write to the organisation settled on.
///
/// Three outcomes rather than a `bool` and a `StorageError`, because a slug
/// another organisation already holds is an ordinary answer the caller renders
/// a sentence for, not a fault. The unique index on `lower(slug)` is what
/// decides it: a check before the write is a race, and this is the arbiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrgWrite {
    Stored,
    SlugTaken,
    NoSuchOrg,
}

pub struct OrgRepo {
    pool: PgPool,
}

impl OrgRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The calling organisation's own row, or `None` when no such row exists.
    pub async fn get(&self, org: OrgId) -> Result<Option<OrgRecord>, StorageError> {
        let row = sqlx::query!(
            "SELECT id, name, slug, slug_deferred FROM organisation WHERE id = $1",
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| OrgRecord {
            id: OrgId(uuid_from_db(row.id)),
            name: row.name,
            slug: row.slug,
            slug_deferred: row.slug_deferred,
        }))
    }

    /// Every tenant, for a pass that visits each in turn under its own pin.
    ///
    /// No pin here and none needed: `organisation` is the tenancy root and
    /// carries no policy, for the reasons this module opens with. Ordered so a
    /// pass visits tenants the same way twice.
    pub async fn tenants(&self) -> Result<Vec<OrgId>, StorageError> {
        let rows = sqlx::query!("SELECT id FROM organisation ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| OrgId(uuid_from_db(row.id)))
            .collect())
    }

    /// Writes the name, the slug, or both, leaving whichever is `None`
    /// exactly as it was. The values are the caller's to validate; storage
    /// stores what it is given.
    ///
    /// One statement rather than two, and that is the whole reason `COALESCE`
    /// appears here. A rename and a claim issued separately can half-succeed:
    /// the name lands, the slug collides, and the organisation is left renamed
    /// by a request that answered 409. One `UPDATE` cannot, because the
    /// constraint violation rolls the whole statement back.
    ///
    /// A `None` slug therefore means "leave it", not "clear it". Nothing in
    /// this product clears a slug, and a column that could be emptied by an
    /// omitted field would be one PATCH away from an accident.
    pub async fn update(
        &self,
        org: OrgId,
        name: Option<&str>,
        slug: Option<&str>,
    ) -> Result<OrgWrite, StorageError> {
        let done = sqlx::query!(
            "UPDATE organisation \
                SET name = COALESCE($2, name), slug = COALESCE($3, slug) \
              WHERE id = $1",
            uuid_to_db(org.0),
            name,
            slug,
        )
        .execute(&self.pool)
        .await;
        match done {
            Ok(done) if done.rows_affected() == 1 => Ok(OrgWrite::Stored),
            Ok(_) => Ok(OrgWrite::NoSuchOrg),
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some("organisation_slug_lower") =>
            {
                Ok(OrgWrite::SlugTaken)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Whether some other organisation already holds this slug.
    ///
    /// Cross-tenant by construction, and it has to be: uniqueness is a
    /// property of the whole table. It answers a boolean and never the holder,
    /// which is the same stance `PlatformAccountAlreadyLinked` takes -- a
    /// verdict that named the holder would turn the constraint into a
    /// directory of every seller on the platform.
    ///
    /// The caller's own row is excluded, so a seller re-typing the slug they
    /// already hold is told it is free. That agrees with the write, which
    /// stores the same value over itself without the index firing.
    ///
    /// Advisory. The index decides, and a caller that renders this verdict
    /// must still render the refusal a write can answer with.
    pub async fn slug_taken(&self, org: OrgId, slug: &str) -> Result<bool, StorageError> {
        let row = sqlx::query!(
            "SELECT EXISTS ( \
                 SELECT 1 FROM organisation \
                  WHERE lower(slug) = lower($1) AND id <> $2 \
             ) AS \"taken!\"",
            slug,
            uuid_to_db(org.0),
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.taken)
    }
}
