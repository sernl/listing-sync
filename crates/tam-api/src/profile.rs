//! The seller's own profile: the picture they set for themselves, and where
//! they stand with the console's guided tour.
//!
//! Every route reads the organisation and the user from the session's
//! [`OrgContext`] and nowhere else, and none takes a user in its path or its
//! body, so no request can name another user's picture to read or to write.
//! The bytes are never accepted here: they arrive through `POST /uploads`,
//! where the picture check already runs, and the write names the handle that
//! upload answered.

use std::collections::HashMap;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{
    AvatarWrite, BlobError, BlobRepo, ProductRepo, ProfileRepo, TourOutcome, TourState,
};
use tam_types::{ContentHash, OrgId, Timestamp, UserId};

use crate::catalogue::{hex_encode, image_bytes_only, parse_hash, refuse_unheld};
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::resources::image_answer;
use crate::{AppState, OrgContext};

/// How a refusal names the picture, so the sentence the slot check composes
/// reads as this slot's rather than a thumbnail's.
const AVATAR_SLOT: &str = "A profile picture";

fn storage_fault(state: &AppState, error: &tam_storage::StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn missing(what: &str) -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(what)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn no_such_user() -> APIError {
    missing("We can't find that person in your account.")
}

/// Where the user stands with the guided tour, as the console decides from
/// it whether to offer the tour. `settled_at` is when they completed or
/// skipped it, and is absent in the two states where they have done neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourView {
    pub state: TourStateView,
    pub settled_at: Option<Timestamp>,
}

/// `due` for an account made since the tour shipped, `predates` for one made
/// before it; see migration 0088.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourStateView {
    Due,
    Predates,
    Completed,
    Skipped,
}

impl TourView {
    #[must_use]
    pub const fn of(tour: TourState) -> Self {
        let (state, settled_at) = match tour {
            TourState::Due => (TourStateView::Due, None),
            TourState::Predates => (TourStateView::Predates, None),
            TourState::Completed(at) => (TourStateView::Completed, Some(at)),
            TourState::Skipped(at) => (TourStateView::Skipped, Some(at)),
        };
        Self { state, settled_at }
    }
}

/// The signed-in user's own profile as the console reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub user: UserId,
    /// The handle of the picture this user set, or `None` where they have set
    /// none. A handle rather than bytes: the picture is fetched by the img
    /// element from its own route, and this is what tells a client whether
    /// there is one to ask for and when it changed.
    pub avatar_hash: Option<String>,
    pub tour: TourView,
}

impl ProfileView {
    fn of(user: UserId, avatar: Option<ContentHash>, tour: TourState) -> Self {
        Self {
            user,
            avatar_hash: avatar.map(|hash| hex_encode(&hash.0)),
            tour: TourView::of(tour),
        }
    }
}

/// The one field a picture write carries. Unknown fields are refused rather
/// than ignored, so a body that tries to name a user is a 422 and not a
/// silently narrowed request.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvatarBody {
    pub hash: String,
}

/// How the user ended the tour: walked to its last step, or closed it early.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourOutcomeBody {
    Completed,
    Skipped,
}

/// The one field a tour write carries, refused with anything else beside it
/// for the reason [`AvatarBody`] gives.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TourBody {
    pub outcome: TourOutcomeBody,
}

/// The whole profile of one user, read after any write so the answer is
/// the row as it now stands.
async fn read_profile(
    state: &AppState,
    org: OrgId,
    user: UserId,
) -> Result<Json<ProfileView>, APIError> {
    let repo = ProfileRepo::new(state.pool.clone());
    let avatar = repo
        .avatar(org, user)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(no_such_user)?;
    let tour = repo
        .tour(org, user)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(no_such_user)?;
    Ok(Json(ProfileView::of(user, avatar, tour)))
}

/// The signed-in user's profile.
pub(crate) async fn profile_view(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ProfileView>, APIError> {
    read_profile(&state, context.org, context.user).await
}

/// Records that the signed-in user completed or skipped the guided tour, so
/// the console stops offering it. Answers the profile as it now stands.
///
/// The console does not call this while an operator is impersonating: the
/// session here is the impersonated user's, and the tour is theirs to end.
pub(crate) async fn settle_tour(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<TourBody>,
) -> Result<Json<ProfileView>, APIError> {
    let outcome = match body.outcome {
        TourOutcomeBody::Completed => TourOutcome::Completed,
        TourOutcomeBody::Skipped => TourOutcome::Skipped,
    };
    ProfileRepo::new(state.pool.clone())
        .settle_tour(context.org, context.user, outcome, (state.wall)())
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(no_such_user)?;
    read_profile(&state, context.org, context.user).await
}

/// Sets the picture to bytes this organisation uploaded.
///
/// The same three checks the thumbnail slots make, in the same order: the
/// handle must name bytes this organisation holds, the bytes are bounded and
/// read back to prove they are a picture, and only then is the row written.
/// The foreign key behind the row makes the first check true of the column
/// as well as of this route.
pub(crate) async fn set_avatar(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<AvatarBody>,
) -> Result<Json<ProfileView>, APIError> {
    let hash = parse_hash(&body.hash)
        .ok_or_else(|| validation("We can't read that picture. Upload it again."))?;
    let lengths: HashMap<ContentHash, i64> = ProductRepo::new(state.pool.clone())
        .stored_hashes(context.org, &[hash])
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .into_iter()
        .collect();
    refuse_unheld(&[hash], &lengths)?;
    image_bytes_only(&state, context.org, &[(hash, AVATAR_SLOT)], &lengths).await?;
    let written = ProfileRepo::new(state.pool.clone())
        .set_avatar(context.org, context.user, Some(hash))
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    settled(&state, context, written).await
}

/// Clears the picture. The bytes stay sealed and counted, as every upload's
/// do: nothing in this system deletes a blob.
pub(crate) async fn clear_avatar(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<ProfileView>, APIError> {
    let written = ProfileRepo::new(state.pool.clone())
        .set_avatar(context.org, context.user, None)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    settled(&state, context, written).await
}

async fn settled(
    state: &AppState,
    context: OrgContext,
    written: AvatarWrite,
) -> Result<Json<ProfileView>, APIError> {
    match written {
        AvatarWrite::Stored(_) => read_profile(state, context.org, context.user).await,
        // Held a moment ago and gone by the write: the constraint is the
        // arbiter, and its answer is the one the unheld check gives.
        AvatarWrite::NotHeld => Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("We can't find that upload. Upload the file again.")
                .code(APIErrorCode::UploadRejected)
                .kind(APIErrorKind::Validation),
        )),
        AvatarWrite::NoSuchUser => Err(no_such_user()),
    }
}

/// The bytes of the signed-in user's own picture, and no other user's: the
/// route names nobody, so there is nothing to substitute.
///
/// The row before the store, as the product cover does, so a user with no
/// picture is answered 404 wherever the bytes would have come from rather
/// than 503 on a deployment without an object store.
pub(crate) async fn avatar(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<([(header::HeaderName, &'static str); 3], Vec<u8>), APIError> {
    let hash = ProfileRepo::new(state.pool.clone())
        .avatar(context.org, context.user)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(no_such_user)?
        .ok_or_else(|| missing("You haven't set a picture."))?;
    let Some(blobs) = state.blobs.clone() else {
        return Err(APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("File storage isn't available right now. Try again later.")
                .kind(APIErrorKind::Internal),
        ));
    };
    let bytes = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .get(context.org, hash)
        .await
        .map_err(|error| match error {
            // The row is keyed to the blob by a foreign key, so a blob the store
            // cannot produce is ours to see rather than a picture to report unset.
            BlobError::Missing => {
                state.internal("a profile row names a blob this store does not hold")
            }
            fault @ (BlobError::Storage(_) | BlobError::Store(_) | BlobError::Crypto(_)) => {
                state.internal(&format!("{fault}"))
            }
        })?;
    image_answer(bytes)
}
