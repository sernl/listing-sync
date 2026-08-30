//! The organisation's own settings: the name the seller sees, and the rename
//! the settings surface issues. Both routes read the organisation from the
//! session's [`OrgContext`] and nowhere else, so the request carries no
//! organisation identifier a caller could substitute.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::OrgRepo;
use tam_types::OrgId;

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

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

/// The longest name the settings field accepts, counted in characters rather
/// than bytes so a name written in accented or non-Latin letters is measured
/// the way the seller who typed it sees it.
pub const NAME_MAX_CHARS: usize = 120;

#[derive(Debug, Serialize, Deserialize)]
pub struct OrgView {
    pub id: OrgId,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub name: String,
}

fn validated_name(raw: &str) -> Result<&str, APIError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(validation("an organisation name cannot be empty"));
    }
    if trimmed.chars().count() > NAME_MAX_CHARS {
        return Err(validation(&format!(
            "an organisation name is at most {NAME_MAX_CHARS} characters"
        )));
    }
    Ok(trimmed)
}

pub(crate) async fn org_view(
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<OrgView>, APIError> {
    let record = OrgRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        .ok_or_else(|| missing("no such organisation"))?;
    Ok(Json(OrgView {
        id: record.id,
        name: record.name,
    }))
}

/// The rename answers with the stored representation rather than 204: the
/// name is trimmed on the way in, so a client that echoed what it sent would
/// render a value the server did not keep.
pub(crate) async fn rename_org(
    State(state): State<AppState>,
    context: OrgContext,
    Json(body): Json<RenameBody>,
) -> Result<Json<OrgView>, APIError> {
    let name = validated_name(&body.name)?;
    let renamed = OrgRepo::new(state.pool.clone())
        .rename(context.org, name)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    if !renamed {
        return Err(missing("no such organisation"));
    }
    Ok(Json(OrgView {
        id: context.org,
        name: name.to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::{validated_name, NAME_MAX_CHARS};
    use crate::error::APIErrorKind;
    use axum::http::StatusCode;

    #[test]
    fn surrounding_whitespace_is_trimmed_rather_than_stored() {
        assert_eq!(
            validated_name("  Riverbend Resources \n").ok(),
            Some("Riverbend Resources"),
            "the stored name is the trimmed one"
        );
    }

    #[test]
    fn a_name_that_is_only_whitespace_is_refused_as_empty() {
        for raw in ["", "   ", "\t\n"] {
            let refused = validated_name(raw).expect_err("a blank name is refused");
            assert_eq!(
                refused.status_code(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "a blank name is the caller's error, not a fault: {raw:?}"
            );
            assert_eq!(
                refused.errors[0].kind,
                Some(APIErrorKind::Validation),
                "the refusal carries the validation kind the client branches on"
            );
        }
    }

    #[test]
    fn the_length_bound_is_counted_in_characters_and_after_trimming() {
        // Two bytes per character, so a byte-counting bound would refuse the
        // name a character-counting one accepts.
        let longest = "\u{e9}".repeat(NAME_MAX_CHARS);
        assert!(
            validated_name(&format!(" {longest} ")).is_ok(),
            "{NAME_MAX_CHARS} characters is accepted, and the surrounding space is not counted"
        );
        let over = "\u{e9}".repeat(NAME_MAX_CHARS + 1);
        let refused = validated_name(&over).expect_err("one character too many is refused");
        assert_eq!(
            refused.status_code(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "an overlong name is the caller's error"
        );
        assert!(
            refused.errors[0]
                .message
                .contains(&NAME_MAX_CHARS.to_string()),
            "the refusal states the bound it applied: {}",
            refused.errors[0].message
        );
    }
}
