//! The connection facts the engine role reads per item: the authorship
//! attestation, and whether the account behind the connection has been named.
//!
//! The attestation is the seller's own declaration of who authored what a
//! connection publishes. It has two writers: the broker's link step seals it
//! onto the connection row as part of linking, and [`ConnectionFactsRepo`]
//! writes it on its own from the declaration route, which is what lets it
//! outlive the broker. It lives here rather than in process configuration
//! because a configured attestation speaks for exactly one seller while the
//! lease scan is cross-tenant: the worker would otherwise attest one seller's
//! authorship on another's listing.
//!
//! It is not a credential and is deliberately not inside the sealed envelope.
//! An attestation the API path could never read would be an attestation
//! nobody could show the seller back.

use sqlx::PgPool;
use tam_types::{Marketplace, OrgId, Timestamp};

use crate::codec::{marketplace_to_db, timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::{pin_org, StorageError};

/// Who a seller attested authorship to, and when they attested it.
///
/// The instant means the same thing whichever writer put it here -- the moment
/// the seller declared -- and the two get it differently because the
/// declaration reaches them differently. On the broker path it is carried from
/// the link, where the seller made the declaration, and a server clock read
/// there would be this system minting an instant for a statement made
/// somewhere else. On [`ConnectionFactsRepo::declare_authorship`] it is
/// stamped server-side for the same reason rather than in spite of it: that
/// request *is* the declaration, so our receipt of it is when the seller made
/// it, and taking the instant off the body would let a caller date their own
/// statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorshipRecord {
    pub name: String,
    pub attested_at: Timestamp,
}

pub struct ConnectionFactsRepo {
    pool: PgPool,
}

impl ConnectionFactsRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The attestation on this tenant's linked connection, where one was
    /// declared.
    ///
    /// `None` covers both a connection carrying no attestation and no linked
    /// connection at all, because the caller's answer to each is the same:
    /// refuse the item and leave its lease to expire rather than write
    /// unattributed.
    ///
    /// Read under the tenant pin, which the API path needs and the engine
    /// role does not care about. `tam_engine` is BYPASSRLS and names the
    /// tenant in the predicate, so the pin is inert for the lease scan; `tam_app`
    /// reads `connection` under forced row-level security, where an unpinned
    /// read matches nothing on a fresh pooled connection and raises on one
    /// whose transaction-local pin has reverted to the empty string. Until this
    /// pin existed no work order could carry an attestation at all.
    pub async fn authorship_for(
        &self,
        org: OrgId,
        marketplace: Marketplace,
    ) -> Result<Option<AuthorshipRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT authorship_name, authorship_attested_at FROM connection \
             WHERE org_id = $1 AND marketplace = $2 AND state = 'linked'",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.and_then(|found| {
            // Both halves or neither: a name without an instant cannot say
            // when the seller attested, and an instant without a name says
            // nothing at all.
            match (found.authorship_name, found.authorship_attested_at) {
                (Some(name), Some(attested_at)) => Some(AuthorshipRecord {
                    name,
                    attested_at: timestamp_from_db(attested_at),
                }),
                (Some(_) | None, _) => None,
            }
        }))
    }

    /// Whether this tenant's linked connection still has no named account.
    ///
    /// The guard on the claim backfill. A connection is usable from the moment
    /// its credential is sealed and takes the global exclusivity lock later,
    /// the first time a live read names the storefront it speaks for; this is
    /// what stops that read being reissued on every item once it has.
    ///
    /// `false` for a connection that is not `linked` as well as for one already
    /// claimed, because neither should be reading an identity: the first has
    /// nothing to claim with and the second has nothing left to claim.
    pub async fn account_claim_pending(
        &self,
        org: OrgId,
        marketplace: Marketplace,
    ) -> Result<bool, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT platform_account_digest FROM connection \
             WHERE org_id = $1 AND marketplace = $2 AND state = 'linked'",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.is_some_and(|found| found.platform_account_digest.is_none()))
    }

    /// Records the seller's declaration of authorship for one marketplace.
    ///
    /// Keyed on the connection rather than on a device, which is what makes it
    /// survive one: the seller declares once and every machine they ever
    /// register writes under it.
    ///
    /// A marketplace with no connection row yet gets one in `unlinked`, which
    /// is truthful -- nothing has reported a session for it -- and grants
    /// nothing, because [`Self::authorship_for`] and the device claim both read
    /// `linked` rows only. An existing row keeps whatever state it stands in,
    /// `revoked` included: a declaration is a fact about the seller rather than
    /// about the link, so it neither links nor unlinks anything.
    pub async fn declare_authorship(
        &self,
        org: OrgId,
        marketplace: Marketplace,
        name: &str,
        at: Timestamp,
    ) -> Result<AuthorshipRecord, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            r#"INSERT INTO connection
                 (org_id, id, marketplace, state, created_at, updated_at,
                  authorship_name, authorship_attested_at)
               VALUES ($1, $2, $3, 'unlinked', now(), now(), $4, $5)
               ON CONFLICT (org_id, marketplace) DO UPDATE
                   SET authorship_name = EXCLUDED.authorship_name,
                       authorship_attested_at = EXCLUDED.authorship_attested_at,
                       updated_at = now()
               RETURNING authorship_name AS "name!",
                         authorship_attested_at AS "attested_at!""#,
            uuid_to_db(org.0),
            uuid::Uuid::new_v4(),
            marketplace_to_db(marketplace),
            name,
            timestamp_to_db(at)?,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(AuthorshipRecord {
            name: row.name,
            attested_at: timestamp_from_db(row.attested_at),
        })
    }
}
