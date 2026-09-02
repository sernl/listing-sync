//! What the create form refuses, decided by the server rather than mirrored
//! from a client.
//!
//! The client runs the same rules inline so a seller reads a message as they
//! type; this is the authority. A client that drifted, or one nobody wrote,
//! gets the same answer, and nothing here is written or enqueued.
//!
//! The draft crossing the wire is deliberately the shape of the form rather
//! than the shape of a create body: a half-filled draft is exactly what this
//! describes, and a wire type that could only carry a complete product would
//! have nothing to say about one.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::product::{
    suggested_additional_licence, AnswerKey, AuthoringError, AuthoringWarning, CategoryGroup,
    CopyrightDeclaration, DetailGroup, FacetSlug, FileGroup, FormGroup, ListingStatus, PaidPrice,
    PriceGroup, ProductName, StandardAlignment, StandardsFramework, TaxCode, TeachingDuration,
    ThumbnailMode, TptBaseProduct, UploadRef,
};
use tam_storage::TptBaseRecord;
use tam_types::{CopyFormat, Currency, ListingCopy, Money};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::product::selection_caps;
use crate::{AppState, OrgContext};

// ------------------------------------------------------------ the check

/// The draft as the form holds it, before anything is written.
///
/// Deliberately the shape of the form rather than the shape of the wire: a
/// client sends what its controls hold and the server answers what the model
/// refuses, so the two sides cannot disagree about a rule by restating it
/// differently. Every field is optional or defaulted, because a half-filled
/// draft is exactly what this endpoint exists to describe.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DraftInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub payload_hash: Option<String>,
    #[serde(default)]
    pub preview_hash: Option<String>,
    #[serde(default)]
    pub video_preview_hash: Option<String>,
    /// `1`, `2` or `3`; absent reads as `1`, which is TPT's own default and
    /// the only one whose slots stay hidden.
    #[serde(default)]
    pub thumbnail_mode: Option<u8>,
    #[serde(default)]
    pub thumbnail_hashes: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub free: bool,
    #[serde(default)]
    pub price_minor_units: Option<i64>,
    #[serde(default)]
    pub additional_licence_minor_units: Option<i64>,
    #[serde(default)]
    pub bundle_discount_minor_units: Option<i64>,
    #[serde(default)]
    pub tax_code_id: Option<u8>,
    #[serde(default)]
    pub grades: Vec<String>,
    #[serde(default)]
    pub subject_areas: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub custom_categories: Vec<String>,
    #[serde(default)]
    pub standards: Vec<StandardInput>,
    #[serde(default)]
    pub teaching_duration_id: Option<u8>,
    #[serde(default)]
    pub pages_or_slides: Option<u32>,
    #[serde(default)]
    pub answer_key_id: Option<u8>,
    /// `1` or `2`. Absent is the ordinary state of a blank form and is what
    /// the copyright refusal names, because ours pre-selects nothing.
    #[serde(default)]
    pub copyright_declaration_id: Option<u8>,
    #[serde(default)]
    pub status_user: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StandardInput {
    pub framework: u32,
    pub code: String,
    #[serde(default)]
    pub tpt_node_id: Option<u64>,
}

/// One thing the form must refuse, named by the group whose heading holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalView {
    /// `name`, `files`, `description`, `price`, `categories`,
    /// `education_standards`, `details`, `copyright` or `product_status`.
    pub group: String,
    /// The control's own label where the refusal is about one picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    pub message: String,
}

/// Something worth saying that is not a refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryView {
    pub group: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckView {
    pub submittable: bool,
    pub refusals: Vec<RefusalView>,
    pub advisories: Vec<AdvisoryView>,
}

fn refusal_of(error: &AuthoringError) -> RefusalView {
    let (control, message) = match error {
        AuthoringError::NameMissing => (None, "A product needs a title.".to_owned()),
        AuthoringError::NameTooLong { used, limit } => (
            None,
            format!("The title is {used} characters and the form takes {limit}."),
        ),
        AuthoringError::EmptySlug => (None, "A category value cannot be blank.".to_owned()),
        AuthoringError::MalformedUploadRef => (
            None,
            "A file handle is not one this server issued; upload the file again.".to_owned(),
        ),
        AuthoringError::OverCap { picker, chosen, cap } => (
            Some(picker.label().to_owned()),
            format!(
                "{} takes up to {cap}, and {chosen} are chosen.",
                picker.label()
            ),
        ),
        AuthoringError::PickerEmpty { picker } => (
            Some(picker.label().to_owned()),
            format!("{} is required; choose at least one.", picker.label()),
        ),
        AuthoringError::ThumbnailsWithoutUploadNow { mode } => (
            None,
            format!(
                "Thumbnails were attached under the option \"{}\", which shows no slots for them.",
                mode.label()
            ),
        ),
        AuthoringError::PriceCurrencyMismatch => (
            None,
            "The price and the additional-licence price are in different currencies.".to_owned(),
        ),
        AuthoringError::CopyrightUnstated => (
            None,
            "Choose one of the two copyright attestations. Nothing is pre-selected, because              the statement is yours to make."
                .to_owned(),
        ),
    };
    RefusalView {
        group: error.group().token().to_owned(),
        control,
        message,
    }
}

fn advisory_of(warning: AuthoringWarning) -> AdvisoryView {
    match warning {
        AuthoringWarning::FreeResourceOverPageGuidance { pages, guidance } => AdvisoryView {
            group: FormGroup::Price.token().to_owned(),
            message: format!(
                "TPT advises that free resources be {guidance} pages or fewer, and this one                  states {pages}. It is guidance rather than a rule, so nothing is blocked."
            ),
        },
    }
}

/// Keeps a smart constructor's refusal instead of dropping the value
/// silently, so a draft with three malformed fields reports three messages.
fn collect<T>(result: Result<T, AuthoringError>, errors: &mut Vec<AuthoringError>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(error);
            None
        }
    }
}

/// Reads a draft into the pure model, collecting the shape refusals the
/// smart constructors raise on the way.
fn read_draft(draft: &DraftInput) -> (Option<TptBaseProduct>, Vec<AuthoringError>) {
    let mut errors = Vec::new();
    let name = collect(ProductName::new(&draft.name), &mut errors);
    let payload = draft
        .payload_hash
        .as_deref()
        .and_then(|hash| collect(UploadRef::new(hash), &mut errors));
    let slugs = |raw: &[String], errors: &mut Vec<AuthoringError>| -> Vec<FacetSlug> {
        raw.iter()
            .filter_map(|value| match FacetSlug::new(value) {
                Ok(slug) => Some(slug),
                Err(error) => {
                    errors.push(error);
                    None
                }
            })
            .collect()
    };

    let categories = CategoryGroup {
        grades: slugs(&draft.grades, &mut errors),
        subject_areas: slugs(&draft.subject_areas, &mut errors),
        tags: slugs(&draft.tags, &mut errors),
        formats: slugs(&draft.formats, &mut errors),
        custom_categories: draft.custom_categories.clone(),
    };
    let thumbnails: Vec<UploadRef> = draft
        .thumbnail_hashes
        .iter()
        .filter_map(|hash| match UploadRef::new(hash) {
            Ok(handle) => Some(handle),
            Err(error) => {
                errors.push(error);
                None
            }
        })
        .collect();

    let (Some(name), Some(payload)) = (name, payload) else {
        return (None, errors);
    };
    let price = match price_of(draft) {
        Ok(price) => price,
        Err(error) => {
            errors.push(error);
            return (None, errors);
        }
    };
    (
        Some(TptBaseProduct {
            name,
            files: FileGroup {
                payload,
                preview: draft
                    .preview_hash
                    .as_deref()
                    .and_then(|hash| UploadRef::new(hash).ok()),
                video_preview: draft
                    .video_preview_hash
                    .as_deref()
                    .and_then(|hash| UploadRef::new(hash).ok()),
                thumbnail_mode: draft
                    .thumbnail_mode
                    .and_then(ThumbnailMode::from_wire_id)
                    .unwrap_or(ThumbnailMode::AutoGenerate),
                thumbnails,
            },
            description: ListingCopy {
                body: draft.description.clone(),
                format: CopyFormat::Markdown,
            },
            price,
            categories,
            standards: draft
                .standards
                .iter()
                .filter_map(|input| {
                    StandardsFramework::from_jurisdiction_id(input.framework).map(|framework| {
                        StandardAlignment {
                            framework,
                            code: input.code.clone(),
                            tpt_node_id: input.tpt_node_id,
                        }
                    })
                })
                .collect(),
            details: DetailGroup {
                teaching_duration: draft
                    .teaching_duration_id
                    .and_then(TeachingDuration::from_wire_id),
                pages_or_slides: draft.pages_or_slides,
                answer_key: draft.answer_key_id.and_then(AnswerKey::from_wire_id),
            },
            copyright: draft
                .copyright_declaration_id
                .and_then(CopyrightDeclaration::from_wire_id),
            status: draft
                .status_user
                .and_then(ListingStatus::from_wire_id)
                .unwrap_or(ListingStatus::Draft),
        }),
        errors,
    )
}

/// TPT sells in USD alone, so the denomination is not a choice a TPT-base
/// draft makes.
fn price_of(draft: &DraftInput) -> Result<PriceGroup, AuthoringError> {
    if draft.free {
        return Ok(PriceGroup::Free);
    }
    let currency = Currency::Usd;
    let amount = draft
        .price_minor_units
        .and_then(|minor| Money::new(minor, currency).ok());
    let Some(price) = amount else {
        // A price the seller has not typed yet is not a currency mismatch and
        // not a refusal of its own: the free-versus-paid control is answered
        // and the amount is simply blank, which the client's own required
        // marker covers. Treated as free here so the rest of the report still
        // reaches the seller rather than stopping at the first blank field.
        return Ok(PriceGroup::Free);
    };
    let additional = draft
        .additional_licence_minor_units
        .and_then(|minor| Money::new(minor, currency).ok())
        .or_else(|| suggested_additional_licence(price))
        .unwrap_or(price);
    let discount = draft
        .bundle_discount_minor_units
        .and_then(|minor| Money::new(minor, currency).ok());
    let tax_code = draft
        .tax_code_id
        .and_then(TaxCode::from_wire_id)
        // Never defaulted (D7). A paid draft with no tax code is reported by
        // the client's own required marker; the model has no variant for a
        // priced listing without one, so the check falls back to describing
        // the rest of the draft rather than inventing a designation.
        .unwrap_or(TaxCode::OtherDigitalGoods);
    PaidPrice::new(price, additional, discount, tax_code).map(PriceGroup::Paid)
}

// ------------------------------------------------- the sidecar's own input

/// The TPT-base fields a create or an edit carries beyond what `product`
/// already stores.
///
/// Deliberately a subset rather than a whole draft: the create body already
/// carries the title, the description, the price, the payload handles and the
/// grades, and a second copy of any of them is a second thing to disagree
/// with. [`TptBaseInput::into_draft`] merges the two into the one shape the
/// validator reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TptBaseInput {
    #[serde(default)]
    pub thumbnail_mode: Option<u8>,
    #[serde(default)]
    pub thumbnail_hashes: Vec<String>,
    #[serde(default)]
    pub video_preview_hash: Option<String>,
    #[serde(default)]
    pub additional_licence_minor_units: Option<i64>,
    #[serde(default)]
    pub bundle_discount_minor_units: Option<i64>,
    #[serde(default)]
    pub tax_code_id: Option<u8>,
    #[serde(default)]
    pub subject_areas: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub custom_categories: Vec<String>,
    #[serde(default)]
    pub standards: Vec<StandardInput>,
    #[serde(default)]
    pub teaching_duration_id: Option<u8>,
    #[serde(default)]
    pub pages_or_slides: Option<u32>,
    #[serde(default)]
    pub answer_key_id: Option<u8>,
    #[serde(default)]
    pub copyright_declaration_id: Option<u8>,
    #[serde(default)]
    pub status_user: Option<u8>,
}

/// What the create body already holds, so the merge names one source per
/// field rather than trusting whichever copy arrived last.
#[derive(Debug, Clone)]
pub(crate) struct DraftHead {
    pub name: String,
    pub description: String,
    pub free: bool,
    pub price_minor_units: Option<i64>,
    pub payload_hash: Option<String>,
    pub grades: Vec<String>,
}

impl TptBaseInput {
    pub(crate) fn into_draft(self, head: DraftHead) -> DraftInput {
        DraftInput {
            name: head.name,
            payload_hash: head.payload_hash,
            preview_hash: None,
            video_preview_hash: self.video_preview_hash,
            thumbnail_mode: self.thumbnail_mode,
            thumbnail_hashes: self.thumbnail_hashes,
            description: head.description,
            free: head.free,
            price_minor_units: head.price_minor_units,
            additional_licence_minor_units: self.additional_licence_minor_units,
            bundle_discount_minor_units: self.bundle_discount_minor_units,
            tax_code_id: self.tax_code_id,
            grades: head.grades,
            subject_areas: self.subject_areas,
            tags: self.tags,
            formats: self.formats,
            custom_categories: self.custom_categories,
            standards: self.standards,
            teaching_duration_id: self.teaching_duration_id,
            pages_or_slides: self.pages_or_slides,
            answer_key_id: self.answer_key_id,
            copyright_declaration_id: self.copyright_declaration_id,
            status_user: self.status_user,
        }
    }
}

/// What the model refuses, as one pure function.
///
/// The handler and the create path both call this, so the endpoint that
/// reports a refusal and the endpoint that acts on one cannot come to
/// different conclusions.
#[must_use]
pub fn verdict(draft: &DraftInput) -> CheckView {
    let caps = selection_caps();
    let (product, mut errors) = read_draft(draft);
    let mut advisories = Vec::new();
    if let Some(product) = product {
        let report = product.check(caps);
        errors.extend(report.errors);
        advisories.extend(report.warnings.into_iter().map(advisory_of));
    }
    let refusals: Vec<RefusalView> = errors.iter().map(refusal_of).collect();
    CheckView {
        submittable: refusals.is_empty(),
        refusals,
        advisories,
    }
}

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
            Some(id) => ThumbnailMode::from_wire_id(id).ok_or_else(|| {
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
            Some(id) => ListingStatus::from_wire_id(id).ok_or_else(|| {
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
    held: Option<u8>,
    of: impl Fn(u8) -> Option<T>,
    what: &str,
) -> Result<Option<T>, APIError> {
    match held {
        None => Ok(None),
        Some(id) => of(id)
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

#[cfg(test)]
mod check_tests {
    use super::{advisory_of, read_draft, refusal_of, selection_caps, DraftInput};
    use tam_domain::product::AuthoringWarning;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Everything a complete draft holds, so each test below changes exactly
    /// one control and the report names that one.
    fn draft() -> DraftInput {
        DraftInput {
            name: "Fractions on a number line".to_owned(),
            payload_hash: Some(HASH.to_owned()),
            free: true,
            grades: vec!["3rd-grade".to_owned()],
            subject_areas: vec!["math".to_owned()],
            tags: vec!["centers".to_owned()],
            copyright_declaration_id: Some(1),
            ..DraftInput::default()
        }
    }

    fn report(draft: &DraftInput) -> Vec<super::RefusalView> {
        let (product, mut errors) = read_draft(draft);
        if let Some(product) = product {
            errors.extend(product.check(selection_caps()).errors);
        }
        errors.iter().map(refusal_of).collect()
    }

    #[test]
    fn a_complete_draft_is_submittable() {
        assert_eq!(report(&draft()), vec![], "nothing left to answer");
    }

    /// The one rule our form inverts against TPT's own.
    #[test]
    fn an_unstated_copyright_declaration_is_the_refusal_that_blocks_submission() {
        let mut blank = draft();
        blank.copyright_declaration_id = None;
        let refusals = report(&blank);
        assert_eq!(refusals.len(), 1, "one control is unanswered: {refusals:?}");
        assert_eq!(refusals[0].group, "copyright");
        assert!(
            refusals[0].message.contains("Nothing is pre-selected"),
            "the message says why there is no default, because the default is the thing TPT \
             does and we deliberately do not: {:?}",
            refusals[0].message
        );
    }

    #[test]
    fn a_cap_refusal_names_the_control_and_both_numbers() {
        let mut over = draft();
        over.grades = [
            "1st-grade",
            "2nd-grade",
            "3rd-grade",
            "4th-grade",
            "5th-grade",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let refusals = report(&over);
        assert_eq!(
            (
                refusals[0].group.as_str(),
                refusals[0].control.as_deref(),
                refusals[0].message.as_str()
            ),
            (
                "categories",
                Some("Grade Level"),
                "Grade Level takes up to 4, and 5 are chosen."
            ),
        );
    }

    /// The contradicted cap. Four subject areas is what the 2026-08-30 create
    /// actually posted, so refusing it would refuse a set TPT accepted.
    #[test]
    fn four_subject_areas_are_accepted_because_a_capture_posted_four() {
        let mut many = draft();
        many.subject_areas = ["math", "science", "ela", "art"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(report(&many), vec![]);
    }

    #[test]
    fn each_required_picker_left_empty_names_itself() {
        let mut bare = draft();
        bare.grades.clear();
        bare.subject_areas.clear();
        bare.tags.clear();
        let named: Vec<Option<String>> = report(&bare)
            .into_iter()
            .map(|refusal| refusal.control)
            .collect();
        assert_eq!(
            named,
            vec![
                Some("Grade Level".to_owned()),
                Some("Subject Area".to_owned()),
                Some("Tag".to_owned()),
            ],
            "three unanswered controls are three messages, not one at a time"
        );
    }

    #[test]
    fn thumbnails_under_a_mode_with_no_slots_are_refused_by_name() {
        let mut deferred = draft();
        deferred.thumbnail_mode = Some(3);
        deferred.thumbnail_hashes = vec![HASH.to_owned()];
        let refusals = report(&deferred);
        assert_eq!(refusals[0].group, "files");
        assert!(
            refusals[0].message.contains("Upload thumbnails later"),
            "the message quotes the radio the seller actually chose: {:?}",
            refusals[0].message
        );
    }

    #[test]
    fn a_malformed_handle_is_refused_before_anything_else_is_read() {
        let mut bad = draft();
        bad.payload_hash = Some("cafe".to_owned());
        let refusals = report(&bad);
        assert_eq!(refusals.len(), 1);
        assert_eq!(refusals[0].group, "files");
    }

    /// Guidance rather than a rule, so it arrives as an advisory.
    #[test]
    fn the_free_resource_page_guidance_is_an_advisory_and_not_a_refusal() {
        let advisory = advisory_of(AuthoringWarning::FreeResourceOverPageGuidance {
            pages: 24,
            guidance: 10,
        });
        assert_eq!(advisory.group, "price");
        assert!(advisory.message.contains("nothing is blocked"));
        let mut long = draft();
        long.pages_or_slides = Some(24);
        assert_eq!(report(&long), vec![], "and it blocks nothing");
    }

    #[test]
    fn a_title_over_the_form_s_own_cap_reports_both_numbers() {
        let mut long = draft();
        long.name = "a".repeat(81);
        let refusals = report(&long);
        assert_eq!(refusals[0].group, "name");
        assert_eq!(
            refusals[0].message,
            "The title is 81 characters and the form takes 80."
        );
    }
}
