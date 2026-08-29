//! The authorship attestation a connection carries, read by the engine role.
//!
//! The attestation is the seller's own declaration of who authored what a
//! connection publishes, sealed onto the connection row by the broker's link
//! step. It lives here rather than in process configuration because a
//! configured attestation speaks for exactly one seller while the lease scan
//! is cross-tenant: the worker would otherwise attest one seller's authorship
//! on another's listing.
//!
//! It is not a credential and is deliberately not inside the sealed envelope.
//! An attestation the API path could never read would be an attestation
//! nobody could show the seller back.

use sqlx::PgPool;
use tam_types::{Marketplace, OrgId, Timestamp};

use crate::codec::{marketplace_to_db, timestamp_from_db, uuid_to_db};
use crate::StorageError;

/// Who a seller attested authorship to, and when they attested it.
///
/// The instant is the seller's, carried from the link rather than read from
/// any server clock: a minted instant would be this system attesting on their
/// behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorshipRecord {
    pub name: String,
    pub attested_at: Timestamp,
}

pub struct AuthorshipRepo {
    pool: PgPool,
}

impl AuthorshipRepo {
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
    /// Read over the engine role without a tenant pin, matching the lease
    /// scan that produced the item; the tenant is named explicitly in the
    /// predicate instead, which is the same discipline the broker follows.
    pub async fn for_connection(
        &self,
        org: OrgId,
        marketplace: Marketplace,
    ) -> Result<Option<AuthorshipRecord>, StorageError> {
        let row = sqlx::query!(
            "SELECT authorship_name, authorship_attested_at FROM connection \
             WHERE org_id = $1 AND marketplace = $2 AND state = 'linked'",
            uuid_to_db(org.0),
            marketplace_to_db(marketplace),
        )
        .fetch_optional(&self.pool)
        .await?;
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
}
