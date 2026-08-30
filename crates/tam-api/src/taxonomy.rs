//! The canonical taxonomy as authoring data: the terms a create body's
//! `subjects` field names.
//!
//! [`crate::vocabulary`] serves what one marketplace's own field table admits.
//! This serves the relation above those tables — our terms, which the
//! projection carries into whichever platform a product is authored for — and
//! a form needs both. Without it `subjects` is a list of UUIDs nothing over
//! this API enumerates, so the field cannot be authored at all.
//!
//! Global reference data, read without pinning a tenant because
//! `canonical_term` is ours rather than an organisation's; the session is
//! still required, like every other read on this surface.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::TermKind;
use tam_storage::TaxonomyRepo;
use tam_types::CanonicalTermId;

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

#[derive(Debug, Clone, Deserialize)]
pub struct TermsParams {
    /// One `TermKind` token, spelled as the JSON bodies spell it. Absent
    /// serves every kind the relation holds; each term names its own, so a
    /// client that wants two kinds asks twice or filters what it has.
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermsView {
    pub terms: Vec<TermView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermView {
    /// The identifier `CreateProductBody.subjects` carries.
    pub id: CanonicalTermId,
    pub kind: TermKind,
    pub label: String,
    /// The broader term this one sits under: a topic names its subject.
    /// Absent for a root, which every subject is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<CanonicalTermId>,
}

/// The wire spelling of an axis in a query parameter: the same token the JSON
/// bodies carry, so a client holds one vocabulary rather than two.
fn parse_kind(raw: &str) -> Option<TermKind> {
    TermKind::ALL
        .into_iter()
        .find(|kind| serde_json::to_value(kind).ok() == Some(raw.into()))
}

pub(crate) async fn list_terms(
    State(state): State<AppState>,
    _context: OrgContext,
    Query(params): Query<TermsParams>,
) -> Result<Json<TermsView>, APIError> {
    let kind = match params.kind.as_deref() {
        None => None,
        Some(raw) => Some(parse_kind(raw).ok_or_else(|| {
            APIError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                APIErrorEntry::new("no such term kind").kind(APIErrorKind::Validation),
            )
        })?),
    };
    let terms = TaxonomyRepo::new(state.pool.clone())
        .terms_of_kind(kind)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(TermsView {
        terms: terms
            .into_iter()
            .map(|term| TermView {
                id: term.id,
                kind: term.kind,
                label: term.label,
                parent: term.parent,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::parse_kind;
    use tam_domain::TermKind;

    #[test]
    fn the_query_spelling_is_the_same_token_the_json_bodies_carry() {
        for kind in TermKind::ALL {
            let token = serde_json::to_value(kind).expect("an axis serialises");
            let token = token.as_str().expect("as a string");
            assert_eq!(
                parse_kind(token),
                Some(kind),
                "the filter spelling round-trips through the wire spelling"
            );
        }
        assert_eq!(
            parse_kind("resourceType"),
            None,
            "a spelling this server never issues is not accepted"
        );
    }
}
