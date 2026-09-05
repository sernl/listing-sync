//! Where else a seller sells: the marketplaces they ask us to support.
//!
//! Two repositories over one table, because two roles reach it. The seller's
//! own write runs under `tam_app` through [`MarketplaceRequestRepo`],
//! pinned to their organisation like every other tenant statement here. The
//! operator listing runs under `tam_backoffice` through
//! [`MarketplaceRequestBackofficeRepo`], which pins nothing and reads every
//! tenant, exactly as far as migration 0055's SELECT grant and read policy
//! reach. Separate types rather than one repository with a cross-tenant
//! method: the pool a caller hands over decides what it can see, and naming
//! the crossing in the type is what stops the wrong pool reaching the wrong
//! method by accident.
//!
//! Nothing reads these rows to decide anything. Which marketplaces exist is
//! the closed `Marketplace` enum's answer; this is a message to a human.

use sqlx::PgPool;
use tam_types::{OrgId, Timestamp, UserId, Uuid};

use crate::codec::{timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// The most rows one operator page may carry, whatever it asks for. A ceiling
/// on this answer's own size rather than a page size: the caller chooses the
/// page and this is what stops it choosing the whole table.
///
/// Public because the route that pages this table has to clamp by the same
/// number to know whether a page filled. Two copies of it would let the route
/// ask for more than it can get and read the short answer as the end of the
/// walk.
pub const PAGE_LIMIT_MAX: i64 = 200;

/// How many requests one organisation may have standing.
///
/// A cap rather than a rate limit, because the harm is not the rate: this
/// table has exactly one reader, an operator listing, and a tenant that can
/// append to it without bound can push every other tenant's request off the
/// page and out of reach. Twenty is well past what a seller with a real list
/// of marketplaces needs and well short of a page, so a genuine request is
/// never refused and a flood never lands.
///
/// Nothing retires a row, so this is a lifetime cap rather than an open-request
/// one. The day an operator can answer a request is the day that distinction
/// exists; until then a seller who reaches twenty is someone to talk to.
pub const REQUESTS_PER_ORG_MAX: i64 = 20;

/// One request as it was written, answered back to whoever may read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceRequestRecord {
    pub org: OrgId,
    pub id: Uuid,
    pub requested_by: UserId,
    /// The address of the person who asked, where the reader is entitled to
    /// it. `None` on the seller's own write, which is answered to that person
    /// and needs no reminder of their own address; `Some` on the operator
    /// listing, which exists so that a request can be answered.
    pub requested_by_email: Option<String>,
    pub name: String,
    pub url: String,
    pub reason: String,
    pub created_at: Timestamp,
}

/// What a write did.
///
/// Three outcomes rather than a row and two error variants, because neither
/// refusal is a fault: both are answers a seller can act on, and a `Result`
/// carrying them would put them on the same footing as a lost connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketplaceRequestWrite {
    Recorded(MarketplaceRequestRecord),
    /// This organisation has already asked about this address.
    AlreadyAsked,
    /// This organisation holds [`REQUESTS_PER_ORG_MAX`] requests already.
    TooMany,
}

/// What a new request carries. The identifier and the instant are the
/// caller's, so a test drives this with a fixed clock and no row is stamped
/// from an ambient one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMarketplaceRequest<'a> {
    pub id: Uuid,
    pub requested_by: UserId,
    pub name: &'a str,
    pub url: &'a str,
    pub reason: &'a str,
    pub created_at: Timestamp,
}

/// The seller's own request, written under `tam_app` and fenced to their
/// organisation. A write and no read, because no route serves a tenant the
/// requests it has made: the answer to the write is the whole of what a seller
/// is shown.
pub struct MarketplaceRequestRepo {
    pool: PgPool,
}

impl MarketplaceRequestRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records one request, or says why it was not recorded.
    ///
    /// Both bounds are read in the same transaction as the insert, under the
    /// tenant pin, so the count they read is this organisation's own and no
    /// other's. The address is compared case-folded, because a seller who
    /// pastes the same shop twice has asked once.
    ///
    /// The residual is a race: two requests arriving together each see the
    /// count before the other commits, so a tenant can hold twenty-one rows or
    /// two rows naming one address. Neither is worth the cost of removing.
    /// Serialising every write on this table, or taking a lock wide enough to
    /// order them, would spend a real fence on an off-by-one in a table whose
    /// only reader is a person reading a page of fifty; and the cap still
    /// bounds the flood it exists to bound, because a caller repeating the
    /// race has to win it every time to gain a row.
    ///
    /// The answer is the row as stored rather than as sent, because the API
    /// trims what a seller typed and a client echoing its own request would
    /// render values this row does not hold.
    pub async fn create(
        &self,
        org: OrgId,
        request: &NewMarketplaceRequest<'_>,
    ) -> Result<MarketplaceRequestWrite, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let standing = sqlx::query!(
            "SELECT count(*) AS \"total!\", \
                    count(*) FILTER (WHERE lower(url) = lower($2)) AS \"same_url!\" \
               FROM marketplace_request WHERE org_id = $1",
            uuid_to_db(org.0),
            request.url,
        )
        .fetch_one(&mut *tx)
        .await?;
        if standing.same_url > 0 {
            return Ok(MarketplaceRequestWrite::AlreadyAsked);
        }
        if standing.total >= REQUESTS_PER_ORG_MAX {
            return Ok(MarketplaceRequestWrite::TooMany);
        }
        let row = sqlx::query!(
            "INSERT INTO marketplace_request \
             (org_id, id, requested_by, name, url, reason, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING id, requested_by, name, url, reason, created_at",
            uuid_to_db(org.0),
            uuid_to_db(request.id),
            uuid_to_db(request.requested_by.0),
            request.name,
            request.url,
            request.reason,
            timestamp_to_db(request.created_at)?,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(MarketplaceRequestWrite::Recorded(
            MarketplaceRequestRecord {
                org,
                id: uuid_from_db(row.id),
                requested_by: UserId(uuid_from_db(row.requested_by)),
                requested_by_email: None,
                name: row.name,
                url: row.url,
                reason: row.reason,
                created_at: timestamp_from_db(row.created_at),
            },
        ))
    }
}

/// Every tenant's requests, for the operator surface only.
///
/// Constructed with the `tam_backoffice` pool. Handed the application's pool
/// instead it answers nothing at all rather than crossing the fence: the
/// statement runs unpinned under forced row-level security, which matches no
/// rows.
pub struct MarketplaceRequestBackofficeRepo {
    pool: PgPool,
}

impl MarketplaceRequestBackofficeRepo {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// One page of the newest requests across every tenant, oldest last.
    ///
    /// Keyset-paged on `(created_at, id)` in the shape the catalogue page uses,
    /// so an operator reads past the first page rather than being shown a
    /// prefix and nothing else. `limit` is clamped to [`PAGE_LIMIT_MAX`], which is
    /// why the parameter cannot be used to ask for the whole table.
    ///
    /// The email is joined rather than served as the raw identifier, because
    /// answering a request means writing to the person who made it and
    /// migration 0037 already grants this role the read that makes that
    /// possible. `LEFT JOIN`, so a request whose author has since been removed
    /// is still listed, without an address.
    pub async fn newest(
        &self,
        cursor: Option<crate::job_reads::LedgerCursor>,
        limit: i64,
    ) -> Result<Vec<MarketplaceRequestRecord>, StorageError> {
        let (cursor_at, cursor_id) = match cursor {
            Some(cursor) => (
                Some(timestamp_to_db(cursor.created_at)?),
                Some(uuid_to_db(cursor.id)),
            ),
            None => (None, None),
        };
        let rows = sqlx::query!(
            // `email?` because the join is outer and sqlx reads the column's
            // own NOT NULL rather than the join's: without the override a
            // request whose author has been removed decodes as a fault.
            "SELECT r.org_id, r.id, r.requested_by, u.email AS \"email?\", \
                    r.name, r.url, r.reason, r.created_at \
               FROM marketplace_request r \
               LEFT JOIN app_user u ON u.id = r.requested_by \
              WHERE ($2::timestamptz IS NULL OR (r.created_at, r.id) < ($2, $3)) \
              ORDER BY r.created_at DESC, r.id DESC LIMIT $1",
            limit.clamp(1, PAGE_LIMIT_MAX),
            cursor_at,
            cursor_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| MarketplaceRequestRecord {
                org: OrgId(uuid_from_db(row.org_id)),
                id: uuid_from_db(row.id),
                requested_by: UserId(uuid_from_db(row.requested_by)),
                requested_by_email: row.email,
                name: row.name,
                url: row.url,
                reason: row.reason,
                created_at: timestamp_from_db(row.created_at),
            })
            .collect())
    }
}
