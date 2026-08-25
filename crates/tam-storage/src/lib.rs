//! Organisation-first repositories over PostgreSQL with row-level security.
//!
//! Every repository method takes the tenant as its first positional parameter
//! and pins it into the connection as a transaction-local `app.current_org`
//! setting, so the row-level-security policies in `migrations/` are a boundary
//! a forgotten `WHERE` clause cannot cross. The queries still filter by
//! `org_id` explicitly; the policies are the backstop the tenancy tests prove,
//! not the only fence.

#![forbid(unsafe_code)]

mod codec;
mod product;

pub use product::{ProductRecord, ProductRepo, ProductSummary};

use sqlx::{Postgres, Transaction};
use tam_types::OrgId;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("timestamp {millis}ms is outside the representable range")]
    TimestampOutOfRange { millis: i64 },
    #[error("stored row violates a domain invariant: {reason}")]
    CorruptRow { reason: String },
    #[error("the aggregate names a different organisation than the call pinned")]
    OrgMismatch,
    #[error("the aggregate is inconsistent: {reason}")]
    Inconsistent { reason: String },
}

/// Declares the tenant for the rest of this transaction. `set_config` with
/// `is_local = true` resets at transaction end, so a pooled connection never
/// leaks one tenant's pin into the next request.
pub(crate) async fn pin_org(
    tx: &mut Transaction<'_, Postgres>,
    org: OrgId,
) -> Result<(), StorageError> {
    let org_text = codec::uuid_to_db(org.0).to_string();
    sqlx::query!("SELECT set_config('app.current_org', $1, true)", org_text)
        .fetch_one(&mut **tx)
        .await?;
    Ok(())
}
