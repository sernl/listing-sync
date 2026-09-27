//! The seller's own account picture and where they stand with the guided
//! tour: which sealed blob the console draws for them, and whether the tour
//! is still to be offered, both on their `app_user` row.
//!
//! `app_user` is classified global in `rls_matrix.rs` and carries no row
//! policy, so every predicate here names the organisation as well as the
//! user, exactly as the notification preference does: the pair is the whole
//! of the fence, and a session can only ever supply its own.

use sqlx::PgPool;
use tam_types::{ContentHash, OrgId, Timestamp, UserId};

use crate::codec::{hash_from_db, hash_to_db, timestamp_from_db, timestamp_to_db, uuid_to_db};
use crate::StorageError;

/// Where a user stands with the console's guided tour. See migration 0088
/// for why "not seen" is two states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourState {
    /// A user created since the tour shipped, who has not ended it.
    Due,
    /// A user who existed before the tour shipped and has not ended it.
    Predates,
    /// Walked to the last step, at this time.
    Completed(Timestamp),
    /// Closed before the last step, at this time.
    Skipped(Timestamp),
}

/// How a user ended the tour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourOutcome {
    Completed,
    Skipped,
}

impl TourOutcome {
    const fn as_db(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Skipped => "skipped",
        }
    }
}

fn tour_from_db(
    state: &str,
    settled_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<TourState, StorageError> {
    match (state, settled_at.map(timestamp_from_db)) {
        ("due", None) => Ok(TourState::Due),
        ("predates", None) => Ok(TourState::Predates),
        ("completed", Some(at)) => Ok(TourState::Completed(at)),
        ("skipped", Some(at)) => Ok(TourState::Skipped(at)),
        (other, _) => Err(StorageError::CorruptRow {
            reason: format!("app_user.tour_state {other:?} does not match its settled time"),
        }),
    }
}

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

    /// Where this user stands with the tour. `None` where this organisation
    /// holds no such user.
    pub async fn tour(&self, org: OrgId, user: UserId) -> Result<Option<TourState>, StorageError> {
        let row = sqlx::query!(
            "SELECT tour_state, tour_settled_at FROM app_user WHERE id = $1 AND org_id = $2",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| tour_from_db(&row.tour_state, row.tour_settled_at))
            .transpose()
    }

    /// Records how this user ended the tour, and when. A later ending
    /// overwrites an earlier one: a tour restarted from Help and then closed
    /// is closed as of now. `None` where this organisation holds no such user.
    pub async fn settle_tour(
        &self,
        org: OrgId,
        user: UserId,
        outcome: TourOutcome,
        at: Timestamp,
    ) -> Result<Option<TourState>, StorageError> {
        let row = sqlx::query!(
            "UPDATE app_user SET tour_state = $3, tour_settled_at = $4 \
             WHERE id = $1 AND org_id = $2 \
             RETURNING tour_state, tour_settled_at",
            uuid_to_db(user.0),
            uuid_to_db(org.0),
            outcome.as_db(),
            timestamp_to_db(at)?,
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| tour_from_db(&row.tour_state, row.tour_settled_at))
            .transpose()
    }
}
