//! The seller's own account picture: which sealed blob the console draws for
//! them, on their `app_user` row.
//!
//! `app_user` is classified global in `rls_matrix.rs` and carries no row
//! policy, so every predicate here names the organisation as well as the
//! user, exactly as the notification preference does: the pair is the whole
//! of the fence, and a session can only ever supply its own.

use sqlx::PgPool;
use tam_types::{ContentHash, OrgId, UserId};

use crate::codec::{hash_from_db, hash_to_db, uuid_to_db};
use crate::StorageError;

/// What a picture write settled on.
///
/// Three outcomes rather than a `bool` and a fault, because a hash this
/// organisation holds no blob for is an ordinary answer the route renders a
/// sentence for. The foreign key `app_user_avatar_blob_held` decides it: a
/// check before the write would race a prune, and the constraint is the
/// arbiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarWrite {
    /// The row now names this hash, or names none.
    Stored(Option<ContentHash>),
    /// This organisation has sealed no blob under the named hash.
    NotHeld,
    /// This organisation holds no such user.
    NoSuchUser,
}

pub struct ProfileRepo {
    pool: PgPool,
}

impl ProfileRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The picture this user has set. `None` where this organisation holds no
    /// such user; `Some(None)` where it does and they have set none.
    pub async fn avatar(
        &self,
        org: OrgId,
        user: UserId,
    ) -> Result<Option<Option<ContentHash>>, StorageError> {
        let row = sqlx::query!(
            "SELECT avatar_hash FROM app_user WHERE id = $1 AND org_id = $2",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| row.avatar_hash.as_deref().map(hash_from_db).transpose())
            .transpose()
    }

    /// Sets or clears it, for one user of one organisation and nobody else.
    pub async fn set_avatar(
        &self,
        org: OrgId,
        user: UserId,
        avatar: Option<ContentHash>,
    ) -> Result<AvatarWrite, StorageError> {
        let written = sqlx::query!(
            "UPDATE app_user SET avatar_hash = $3 WHERE id = $1 AND org_id = $2 \
             RETURNING avatar_hash",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
            avatar.map(hash_to_db),
        )
        .fetch_optional(&self.pool)
        .await;
        match written {
            Ok(Some(row)) => Ok(AvatarWrite::Stored(
                row.avatar_hash.as_deref().map(hash_from_db).transpose()?,
            )),
            Ok(None) => Ok(AvatarWrite::NoSuchUser),
            Err(sqlx::Error::Database(database))
                if database.constraint() == Some("app_user_avatar_blob_held") =>
            {
                Ok(AvatarWrite::NotHeld)
            }
            Err(error) => Err(StorageError::Db(error)),
        }
    }
}
