//! What the create form refuses, as one pure function every caller runs.
//!
//! Lifted out of `tam-api` so there is exactly one definition of the rules.
//! The server answers `POST /v1/authoring/check` by calling [`verdict`]; the
//! browser answers the same question by calling the same function compiled to
//! `wasm32` through `tam-core-wasm`. A client cannot drift from the server
//! without drifting from itself, which is the property the split exists for
//! (D28). `tam-api` keeps the handler, the sidecar record and the `APIError`
//! conversions, because those are the half that genuinely needs a server.
//!
//! Two kinds of refusal meet here and the distinction is deliberate. A model
//! refusal is a fact about a product and is raised by
//! [`TptBaseProduct::check`]. A draft refusal is a fact about a half-filled
//! form that the model cannot state, because `TptBaseProduct` requires a
//! payload by type, has no variant for a priced listing without a tax code,
//! and holds no price floor. Both reach the seller as one list.

use serde::{Deserialize, Serialize};
use tam_domain::product::{
    suggested_additional_licence, AnswerKey, AuthoringError, AuthoringWarning, CategoryGroup,
    CopyrightDeclaration, DetailGroup, FacetSlug, FileGroup, FormGroup, ListingStatus, PaidPrice,
    PriceGroup, ProductName, SelectionCaps, StandardAlignment, StandardsFramework, TaxCode,
    TeachingDuration, ThumbnailMode, TptBaseProduct, UploadRef,
};
use tam_types::{CopyFormat, Currency, ListingCopy, Money};

/// The committed capture every number below is measured from, compiled in so
/// the browser reads what the server reads rather than being told it.
const CAPTURE: &str = include_str!("../../../docs/design/data/tpt-vocabulary.json");

// --------------------------------------------------------- what the capture says

/// The `constraints` block, read for the one number the pure model does not
/// hold. Its siblings are served by `tam-api`'s vocabulary view and are not
/// restated here.
#[derive(Debug, Clone, Deserialize)]
struct Captured {
    constraints: CapturedConstraints,
}

#[derive(Debug, Clone, Deserialize)]
struct CapturedConstraints {
    #[serde(rename = "minPrice")]
    min_price: f64,
}

/// TPT's own price floor in cents, as the capture states it.
///
/// `min_price: 0.95` in the create page's `var cfg` bootstrap. Zero where the
/// capture does not parse, which refuses nothing: a floor nobody could read is
/// not a floor to enforce.
#[must_use]
pub fn min_price_minor_units() -> i64 {
    let Ok(read) = serde_json::from_str::<Captured>(CAPTURE) else {
        return 0;
    };
    let cents = (read.constraints.min_price * 100.0).round();
    if !cents.is_finite() || !(0.0..=1_000_000.0).contains(&cents) {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "guarded above: integral after round() and inside 0..=1e6, which i64 holds exactly"
    )]
    let minor = cents as i64;
    minor
}

/// The caps as the committed capture states them.
///
/// Read here rather than accepted from a caller, because caps supplied from
/// outside are precisely the channel through which two callers of one rule
/// stop agreeing about it. A capture that does not parse yields the default,
/// where every cap is unmeasured and so refuses nothing.
#[must_use]
pub fn selection_caps() -> SelectionCaps {
    let Ok(form) = tam_taxonomy::TptForm::read(CAPTURE) else {
        return SelectionCaps::default();
    };
    let cap = |picker| {
        form.cap(picker)
            .map(|tam_domain::registry::CountCap { limit }| limit)
    };
    SelectionCaps {
        grades: cap(tam_taxonomy::Picker::Grades),
        subject_areas: cap(tam_taxonomy::Picker::SubjectAreas),
        tags: cap(tam_taxonomy::Picker::Tags),
        formats: cap(tam_taxonomy::Picker::Formats),
        thumbnails: cap(tam_taxonomy::Picker::Thumbnails),
    }
}

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

#[must_use]
pub fn refusal_of(error: &AuthoringError) -> RefusalView {
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
pub struct DraftHead {
    pub name: String,
    pub description: String,
    pub free: bool,
    pub price_minor_units: Option<i64>,
    pub payload_hash: Option<String>,
    pub grades: Vec<String>,
}

impl TptBaseInput {
    #[must_use]
    pub fn into_draft(self, head: DraftHead) -> DraftInput {
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

// ------------------------------------------------- what the draft refuses

/// A submission rule the pure model cannot state.
///
/// Each of the three is a fact about a half-filled form rather than about a
/// product, and each is one TPT's own create form enforces: `TptBaseProduct`
/// takes a payload by type, holds no price floor, and has no variant for a
/// priced listing without a tax code, so a draft missing any of them cannot be
/// turned into a product to ask about. They are refused here rather than in a
/// client, because the server is the authority and a rule only a client held
/// would let a bypassed client submit a product TPT refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftRefusal {
    /// `Downloadable File` is what buyers receive; TPT lists no product
    /// without one and neither does any other target.
    PayloadMissing,
    /// A paid listing whose price is blank or under the form's own floor.
    /// `min_price: 0.95` in the create page's `var cfg` bootstrap.
    PriceUnderFloor { stated: Option<i64>, floor: i64 },
    /// TPT marks Tax Code required on a priced listing and renders `Please
    /// select a tax code.` when it is blank. Refusing here is what D7 asks
    /// for rather than a departure from it: the seller is contractually
    /// answerable for the designation, so requiring them to choose is the
    /// opposite of choosing for them.
    TaxCodeUnstated,
}

fn refusal_of_draft(refusal: DraftRefusal) -> RefusalView {
    let (group, control, message) = match refusal {
        DraftRefusal::PayloadMissing => (
            FormGroup::Files,
            Some("Downloadable File"),
            "Upload the file buyers download; a product with none cannot be listed anywhere."
                .to_owned(),
        ),
        DraftRefusal::PriceUnderFloor {
            stated: None,
            floor: _,
        } => (
            FormGroup::Price,
            Some("Price"),
            "A paid listing needs a price, written in dollars and cents.".to_owned(),
        ),
        DraftRefusal::PriceUnderFloor {
            stated: Some(_),
            floor,
        } => (
            FormGroup::Price,
            Some("Price"),
            format!("TPT refuses a price below ${}.", major_units(floor)),
        ),
        DraftRefusal::TaxCodeUnstated => (
            FormGroup::Price,
            Some("Tax Code"),
            "Choose a tax code. It is never chosen for you: designating it is yours under \
             TPT's terms."
                .to_owned(),
        ),
    };
    RefusalView {
        group: group.token().to_owned(),
        control: control.map(ToOwned::to_owned),
        message,
    }
}

/// Cents as dollars and cents, for a message a seller reads.
///
/// `div_euclid` rather than `/`: the workspace denies the integer-division
/// operator, and the euclidean pair is the one whose remainder is never
/// negative, so the cents half needs no sign correction.
fn major_units(minor: i64) -> String {
    let (whole, part) = (minor.div_euclid(100), minor.rem_euclid(100));
    format!("{whole}.{part:02}")
}

/// The three submission rules, in the order the form's own groups read.
fn draft_refusals(draft: &DraftInput) -> Vec<DraftRefusal> {
    let mut found = Vec::new();
    if draft.payload_hash.is_none() {
        found.push(DraftRefusal::PayloadMissing);
    }
    if !draft.free {
        let floor = min_price_minor_units();
        let stated = draft.price_minor_units;
        if stated.is_none_or(|minor| minor < floor) {
            found.push(DraftRefusal::PriceUnderFloor { stated, floor });
        }
        if draft.tax_code_id.and_then(TaxCode::from_wire_id).is_none() {
            found.push(DraftRefusal::TaxCodeUnstated);
        }
    }
    found
}

// ------------------------------------------------------------ the verdict

/// What the form refuses, as one pure function.
///
/// The handler, the create path and the browser all call this, so the endpoint
/// that reports a refusal, the endpoint that acts on one and the page that
/// shows one cannot come to different conclusions.
#[must_use]
pub fn verdict(draft: &DraftInput) -> CheckView {
    verdict_with(draft, selection_caps())
}

/// The same decision against caps the caller states.
///
/// Exists so a test can hold the caps fixed. Nothing in production calls it
/// with anything but [`selection_caps`], for the reason given there.
#[must_use]
pub fn verdict_with(draft: &DraftInput, caps: SelectionCaps) -> CheckView {
    let (product, mut errors) = read_draft(draft);
    let mut advisories = Vec::new();
    if let Some(product) = product {
        let report = product.check(caps);
        errors.extend(report.errors);
        advisories.extend(report.warnings.into_iter().map(advisory_of));
    }
    let mut refusals: Vec<RefusalView> = errors.iter().map(refusal_of).collect();
    refusals.extend(draft_refusals(draft).into_iter().map(refusal_of_draft));
    CheckView {
        submittable: refusals.is_empty(),
        refusals,
        advisories,
    }
}

#[cfg(test)]
mod draft_rule_tests {
    use super::{verdict, DraftInput};

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A draft that satisfies every rule, so each test below removes exactly
    /// one answer and the verdict names that one.
    fn paid() -> DraftInput {
        DraftInput {
            name: "Fractions on a number line".to_owned(),
            payload_hash: Some(HASH.to_owned()),
            free: false,
            price_minor_units: Some(450),
            tax_code_id: Some(1),
            grades: vec!["3rd-grade".to_owned()],
            subject_areas: vec!["math".to_owned()],
            tags: vec!["centers".to_owned()],
            copyright_declaration_id: Some(1),
            ..DraftInput::default()
        }
    }

    fn controls(draft: &DraftInput) -> Vec<String> {
        verdict(draft)
            .refusals
            .into_iter()
            .filter_map(|refusal| refusal.control)
            .collect()
    }

    #[test]
    fn a_complete_paid_draft_is_submittable() {
        assert!(
            verdict(&paid()).submittable,
            "every rule the form states is answered, so nothing is refused"
        );
    }

    /// The defect this rule fixes: the draft reader returned early on a
    /// missing payload without recording anything, so a titled draft with no
    /// file came back submittable and only the client's own refusal caught it.
    #[test]
    fn a_draft_with_no_downloadable_file_is_refused() {
        let mut bare = paid();
        bare.payload_hash = None;
        let report = verdict(&bare);
        assert!(
            !report.submittable,
            "a product buyers cannot download is not one any target will take"
        );
        assert!(
            controls(&bare).contains(&"Downloadable File".to_owned()),
            "and the refusal names the control rather than the wire field"
        );
    }

    /// `min_price: 0.95` from the create page's own bootstrap, read from the
    /// committed capture rather than typed here.
    #[test]
    fn a_paid_draft_under_the_forms_own_floor_is_refused() {
        let mut cheap = paid();
        cheap.price_minor_units = Some(94);
        let report = verdict(&cheap);
        assert!(!report.submittable, "94 cents is under TPT's 95-cent floor");
        assert!(
            report
                .refusals
                .iter()
                .any(|refusal| refusal.message.contains("$0.95")),
            "and the message states the floor the capture holds, not a rounded one"
        );

        let mut exact = paid();
        exact.price_minor_units = Some(95);
        assert!(
            verdict(&exact).submittable,
            "the floor itself is accepted; it is a minimum rather than an exclusive bound"
        );
    }

    #[test]
    fn a_paid_draft_with_no_price_at_all_is_refused() {
        let mut blank = paid();
        blank.price_minor_units = None;
        assert!(
            !verdict(&blank).submittable,
            "a paid listing with no amount typed states no price to sell at"
        );
    }

    /// D7 read the way it is written: the seller is answerable for the
    /// designation, so the form requires them to make it rather than making
    /// one for them.
    #[test]
    fn a_paid_draft_with_no_tax_code_is_refused_by_the_control_that_holds_it() {
        let mut untaxed = paid();
        untaxed.tax_code_id = None;
        let report = verdict(&untaxed);
        assert!(
            !report.submittable,
            "TPT marks Tax Code required on a priced listing"
        );
        assert!(
            controls(&untaxed).contains(&"Tax Code".to_owned()),
            "and names the control, because the seller has to choose one themselves"
        );
    }

    /// Free resources are not taxed and have no price, so neither price rule
    /// applies to one. Help article 47530264334868.
    #[test]
    fn a_free_draft_needs_neither_a_price_nor_a_tax_code() {
        let mut free = paid();
        free.free = true;
        free.price_minor_units = None;
        free.tax_code_id = None;
        assert!(
            verdict(&free).submittable,
            "the free branch collects neither field, so requiring either would refuse a \
             product TPT accepts"
        );
    }

    #[test]
    fn the_capture_states_the_floor_the_bootstrap_carries() {
        assert_eq!(
            super::min_price_minor_units(),
            95,
            "read from the committed capture's constraints block, never typed here"
        );
    }
}

#[cfg(test)]
mod lifted_tests {
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
