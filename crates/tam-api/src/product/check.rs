//! What the create form refuses, as the server answers it.
//!
//! The rules themselves live in `tam-authoring`, which the browser compiles to
//! `wasm32` and calls through `tam-core-wasm`, so a seller reading a message
//! as they type is reading this endpoint's own answer rather than a client's
//! restatement of it (D28). What stays here is the half that needs a server:
//! the handler, the sidecar record, and the conversions from a refusal to an
//! `APIError`.
//!
//! Nothing here is written or enqueued.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use tam_domain::product::{
    AnswerKey, AuthoringError, CategoryGroup, CopyrightDeclaration, DetailGroup, FacetSlug,
    ListingStatus, StandardAlignment, StandardsFramework, TaxCode, TeachingDuration, ThumbnailMode,
    UploadRef,
};
use tam_storage::TptBaseRecord;

use tam_authoring::member_of;
pub use tam_authoring::{
    refusal_of, verdict, AdvisoryView, CheckView, DraftHead, DraftInput, RefusalView,
    StandardInput, TptBaseInput,
};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// The draft as the sidecar stores it.
///
/// Refuses the same shapes the smart constructors do, because a value that
/// cannot be a member of its vocabulary must not reach a column whose CHECK
/// would refuse it as a 500 rather than as an answer the seller can act on.
pub(crate) fn record_of(draft: &DraftInput) -> Result<TptBaseRecord, APIError> {
    let handles = |raw: &[String]| -> Result<Vec<UploadRef>, APIError> {
        raw.iter()
            .map(|hash| UploadRef::new(hash).map_err(|error| shape_refusal(&error)))
            .collect()
    };
    let slugs = |raw: &[String]| -> Result<Vec<FacetSlug>, APIError> {
        raw.iter()
            .map(|slug| FacetSlug::new(slug).map_err(|error| shape_refusal(&error)))
            .collect()
    };
    Ok(TptBaseRecord {
        thumbnail_mode: match draft.thumbnail_mode {
            None => ThumbnailMode::AutoGenerate,
            Some(id) => member_of(id, ThumbnailMode::from_wire_id).ok_or_else(|| {
                validation_refusal(&format!("{id} is not a thumbnail mode this form offers"))
            })?,
        },
        thumbnails: handles(&draft.thumbnail_hashes)?,
        video_preview: draft
            .video_preview_hash
            .as_deref()
            .map(|hash| UploadRef::new(hash).map_err(|error| shape_refusal(&error)))
            .transpose()?,
        additional_licence_minor_units: draft.additional_licence_minor_units,
        bundle_discount_minor_units: draft.bundle_discount_minor_units,
        tax_code: member(draft.tax_code_id, TaxCode::from_wire_id, "tax code")?,
        categories: CategoryGroup {
            // The grades are stored on `product.grades` as the verbatim
            // declaration every other reader already uses, so the sidecar
            // holds no second copy of them.
            grades: vec![],
            subject_areas: slugs(&draft.subject_areas)?,
            tags: slugs(&draft.tags)?,
            formats: slugs(&draft.formats)?,
            custom_categories: draft.custom_categories.clone(),
            appropriate_for_country: draft.appropriate_for_country,
        },
        standards: draft
            .standards
            .iter()
            .map(|input| {
                StandardsFramework::from_jurisdiction_id(input.framework)
                    .map(|framework| StandardAlignment {
                        framework,
                        code: input.code.clone(),
                        tpt_node_id: input.tpt_node_id,
                    })
                    .ok_or_else(|| {
                        validation_refusal(&format!(
                            "jurisdiction {} is not one the create form offers",
                            input.framework
                        ))
                    })
            })
            .collect::<Result<Vec<_>, APIError>>()?,
        details: DetailGroup {
            teaching_duration: member(
                draft.teaching_duration_id,
                TeachingDuration::from_wire_id,
                "teaching duration",
            )?,
            pages_or_slides: draft.pages_or_slides.filter(|pages| *pages > 0),
            answer_key: member(draft.answer_key_id, AnswerKey::from_wire_id, "answer key")?,
        },
        copyright: member(
            draft.copyright_declaration_id,
            CopyrightDeclaration::from_wire_id,
            "copyright declaration",
        )?,
        status: match draft.status_user {
            None => ListingStatus::Draft,
            Some(id) => member_of(id, ListingStatus::from_wire_id).ok_or_else(|| {
                validation_refusal(&format!("{id} is not a listing status this form offers"))
            })?,
        },
    })
}

/// A stored wire id back through the vocabulary's own membership test.
///
/// The sidecar's CHECK constraints bound these columns, so an id outside the
/// set must be refused here as an answer the seller can act on rather than
/// reaching the database and returning as a fault.
fn member<T>(
    held: Option<i64>,
    of: impl FnOnce(u8) -> Option<T>,
    what: &str,
) -> Result<Option<T>, APIError> {
    match held {
        None => Ok(None),
        Some(id) => member_of(id, of)
            .map(Some)
            .ok_or_else(|| validation_refusal(&format!("{id} is not a {what} this form offers"))),
    }
}

fn shape_refusal(error: &AuthoringError) -> APIError {
    validation_refusal(&refusal_of(error).message)
}

fn validation_refusal(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

/// What the form would refuse, decided by the server rather than mirrored
/// from it.
///
/// The client runs the same rules so a seller sees a message as they type,
/// but this is the authority: a client that drifted, or one nobody wrote,
/// still gets the same answer. Nothing is written and nothing is enqueued.
pub(crate) async fn check_draft(
    State(_state): State<AppState>,
    _context: OrgContext,
    Json(draft): Json<DraftInput>,
) -> Result<Json<CheckView>, APIError> {
    Ok(Json(verdict(&draft)))
}
