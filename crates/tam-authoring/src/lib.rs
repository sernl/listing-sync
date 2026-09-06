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
///
/// Every vocabulary id is read as `i64` rather than as the `u8` its
/// vocabulary holds. A form can hold any integer, and reading the narrow type
/// at the decode refused the whole draft for one field's value — a fixture
/// tax code of 81111 took the resource page down with it — so an id outside
/// the range, like one inside it that no member has, is refused by its
/// control's name in [`draft_refusals`] while the rest of the draft is read.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DraftInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub payload_hash: Option<String>,
    /// Whether this draft is bound for a marketplace.
    ///
    /// D32: a resource kept on Teachouse alone may carry no file, and a file
    /// becomes necessary the moment a marketplace is named — for a draft there
    /// and for a live listing alike, so this is about the destination rather
    /// than the lifecycle state. The model describes a product and cannot know
    /// where it is going, so the caller states it.
    ///
    /// Defaulting to false is what makes a saved template deserialise
    /// correctly: a template is a partial draft nobody has chosen a
    /// destination for, and holding it to a marketplace's rules would refuse
    /// the half-filled form the template surface exists to store.
    #[serde(default)]
    pub for_marketplace: bool,
    #[serde(default)]
    pub preview_hash: Option<String>,
    #[serde(default)]
    pub video_preview_hash: Option<String>,
    /// `1`, `2` or `3`; absent reads as `1`, which is TPT's own default and
    /// the only one whose slots stay hidden.
    #[serde(default)]
    pub thumbnail_mode: Option<i64>,
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
    pub tax_code_id: Option<i64>,
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
    /// `data[ItemsLocalization][country_id_flag]`, the Categories group's last
    /// control. Absent is a draft that states nothing about it, which is not
    /// the same as a seller who left the checkbox unticked: a form that
    /// rendered the control sends the box's own state either way, so `false`
    /// arrives only as an answer.
    #[serde(default)]
    pub appropriate_for_country: Option<bool>,
    #[serde(default)]
    pub standards: Vec<StandardInput>,
    #[serde(default)]
    pub teaching_duration_id: Option<i64>,
    #[serde(default)]
    pub pages_or_slides: Option<u32>,
    #[serde(default)]
    pub answer_key_id: Option<i64>,
    /// `1` or `2`. Absent is the ordinary state of a blank form and is what
    /// the copyright refusal names, because ours pre-selects nothing.
    #[serde(default)]
    pub copyright_declaration_id: Option<i64>,
    #[serde(default)]
    pub status_user: Option<i64>,
}

/// The vocabulary member a wire id names, or `None` where no member has it.
///
/// Every vocabulary's ids fit a byte, so an id beyond one is an id no member
/// has rather than a decode failure: the narrowing is the membership test's
/// first step, not the deserialiser's, and one caller cannot narrow
/// differently from another.
#[must_use]
pub fn member_of<T>(id: i64, of: impl FnOnce(u8) -> Option<T>) -> Option<T> {
    u8::try_from(id).ok().and_then(of)
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

/// Every sentence here is written for the teacher reading it, to the
/// founder's rule of 2026-09-11: one short sentence, saying what to do, and
/// no explanation of why a rule exists unless the teacher has to act on it.
/// The reasons stay in the doc comments, where the person who has to act on
/// them is a maintainer.
#[must_use]
pub fn refusal_of(error: &AuthoringError) -> RefusalView {
    let (control, message) = match error {
        AuthoringError::NameMissing => (None, "Add a name for your resource.".to_owned()),
        AuthoringError::NameTooLong { used, limit } => (
            None,
            format!("Shorten the name to {limit} characters; it is {used} now."),
        ),
        AuthoringError::EmptySlug => (
            None,
            "Choose your categories again; one came through blank.".to_owned(),
        ),
        AuthoringError::MalformedUploadRef => (None, "Upload that file again.".to_owned()),
        AuthoringError::OverCap {
            picker,
            chosen,
            cap,
        } => (
            Some(picker.label().to_owned()),
            format!(
                "Choose up to {cap} under {}; you have {chosen}.",
                picker.label()
            ),
        ),
        AuthoringError::PickerEmpty { picker } => (
            Some(picker.label().to_owned()),
            format!("Choose at least one under {}.", picker.label()),
        ),
        AuthoringError::ThumbnailsWithoutUploadNow { .. } => (
            None,
            "Choose \"Upload thumbnails now\", or remove the thumbnails.".to_owned(),
        ),
        AuthoringError::PriceCurrencyMismatch => (
            None,
            "Use one currency for the price and the additional licence price.".to_owned(),
        ),
        AuthoringError::CopyrightUnstated => (
            None,
            "Choose one of the two copyright statements.".to_owned(),
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
                "Free resources do best at {guidance} pages or fewer, and this one has {pages}."
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
        appropriate_for_country: draft.appropriate_for_country,
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
                    .and_then(|id| member_of(id, ThumbnailMode::from_wire_id))
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
                    .and_then(|id| member_of(id, TeachingDuration::from_wire_id)),
                pages_or_slides: draft.pages_or_slides,
                answer_key: draft
                    .answer_key_id
                    .and_then(|id| member_of(id, AnswerKey::from_wire_id)),
            },
            copyright: draft
                .copyright_declaration_id
                .and_then(|id| member_of(id, CopyrightDeclaration::from_wire_id)),
            status: draft
                .status_user
                .and_then(|id| member_of(id, ListingStatus::from_wire_id))
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
        .and_then(|id| member_of(id, TaxCode::from_wire_id))
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
///
/// Its ids stay `u8` where [`DraftInput`]'s are wide: this is a write's body,
/// and the sidecar row holds only a member, so the create refuses a value no
/// member has at its door rather than reading it in.
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
    /// `data[ItemsLocalization][country_id_flag]`, the Categories group's last
    /// control. Absent is a draft that states nothing about it, which is not
    /// the same as a seller who left the checkbox unticked: a form that
    /// rendered the control sends the box's own state either way, so `false`
    /// arrives only as an answer.
    #[serde(default)]
    pub appropriate_for_country: Option<bool>,
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
    /// Whether this create names a marketplace, which is the only thing that
    /// makes the payload necessary (D32). The create body holds the inventory
    /// list, so this is read from there rather than guessed at here.
    pub for_marketplace: bool,
}

impl TptBaseInput {
    #[must_use]
    pub fn into_draft(self, head: DraftHead) -> DraftInput {
        DraftInput {
            name: head.name,
            payload_hash: head.payload_hash,
            for_marketplace: head.for_marketplace,
            preview_hash: None,
            video_preview_hash: self.video_preview_hash,
            thumbnail_mode: self.thumbnail_mode.map(i64::from),
            thumbnail_hashes: self.thumbnail_hashes,
            description: head.description,
            free: head.free,
            price_minor_units: head.price_minor_units,
            additional_licence_minor_units: self.additional_licence_minor_units,
            bundle_discount_minor_units: self.bundle_discount_minor_units,
            tax_code_id: self.tax_code_id.map(i64::from),
            grades: head.grades,
            subject_areas: self.subject_areas,
            tags: self.tags,
            formats: self.formats,
            custom_categories: self.custom_categories,
            appropriate_for_country: self.appropriate_for_country,
            standards: self.standards,
            teaching_duration_id: self.teaching_duration_id.map(i64::from),
            pages_or_slides: self.pages_or_slides,
            answer_key_id: self.answer_key_id.map(i64::from),
            copyright_declaration_id: self.copyright_declaration_id.map(i64::from),
            status_user: self.status_user.map(i64::from),
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
    /// A stated id that no member of the control's vocabulary has, whether
    /// beyond the byte the ids fit in or inside it. Left unread rather than
    /// read as absent, and refused by the control's name, so a seller whose
    /// form offered the value learns which control holds it; the unstated
    /// rules above stay quiet about a control that is stated.
    IdUnknown { control: IdControl, stated: i64 },
}

/// The six controls whose value travels as a vocabulary row id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdControl {
    ThumbnailMode,
    TaxCode,
    TeachingDuration,
    AnswerKey,
    Copyright,
    Status,
}

impl IdControl {
    const fn group(self) -> FormGroup {
        match self {
            Self::ThumbnailMode => FormGroup::Files,
            Self::TaxCode => FormGroup::Price,
            Self::TeachingDuration | Self::AnswerKey => FormGroup::Details,
            Self::Copyright => FormGroup::Copyright,
            Self::Status => FormGroup::ProductStatus,
        }
    }

    /// The control's own label, as the form heads it.
    const fn label(self) -> &'static str {
        match self {
            Self::ThumbnailMode => "Thumbnails",
            Self::TaxCode => "Tax Code",
            Self::TeachingDuration => "Teaching Duration",
            Self::AnswerKey => "Answer Key",
            Self::Copyright => "Copyright",
            Self::Status => "Product Status",
        }
    }

    /// What one of its options is, in the sentence `tam-api` and
    /// `tam-storage` already use for a value no member has.
    const fn member(self) -> &'static str {
        match self {
            Self::ThumbnailMode => "thumbnail mode",
            Self::TaxCode => "tax code",
            Self::TeachingDuration => "teaching duration",
            Self::AnswerKey => "answer key",
            Self::Copyright => "copyright declaration",
            Self::Status => "listing status",
        }
    }
}

fn refusal_of_draft(refusal: DraftRefusal) -> RefusalView {
    let (group, control, message) = match refusal {
        DraftRefusal::PayloadMissing => (
            FormGroup::Files,
            Some("Downloadable File"),
            "Upload the file buyers will download.".to_owned(),
        ),
        DraftRefusal::PriceUnderFloor {
            stated: None,
            floor: _,
        } => (
            FormGroup::Price,
            Some("Price"),
            "Add a price in dollars and cents.".to_owned(),
        ),
        DraftRefusal::PriceUnderFloor {
            stated: Some(_),
            floor,
        } => (
            FormGroup::Price,
            Some("Price"),
            format!("Raise the price to at least ${}.", major_units(floor)),
        ),
        DraftRefusal::TaxCodeUnstated => (
            FormGroup::Price,
            Some("Tax Code"),
            "Choose a tax code.".to_owned(),
        ),
        DraftRefusal::IdUnknown { control, .. } => (
            control.group(),
            Some(control.label()),
            format!("Choose a {} from the list.", control.member()),
        ),
    };
    RefusalView {
        group: group.token().to_owned(),
        control: control.map(ToOwned::to_owned),
        message,
    }
}

/// The refusal a stated id earns when no member of its vocabulary has it.
fn unknown_id<T>(
    control: IdControl,
    stated: Option<i64>,
    of: impl FnOnce(u8) -> Option<T>,
) -> Option<DraftRefusal> {
    let stated = stated?;
    if member_of(stated, of).is_some() {
        return None;
    }
    Some(DraftRefusal::IdUnknown { control, stated })
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

/// What the form refuses about a price the seller has actually typed, for a
/// draft nobody has finished.
///
/// Exists because [`verdict`] cannot answer this question about a partial
/// draft. Its two price rules live in [`draft_refusals`], and both of those
/// fire on absence as well as on a value: a blank price and a blank tax code
/// are refusals of a submission, and a template is not one. The stated half is
/// different — a price the seller typed below TPT's floor is a value, and the
/// template surface has to refuse it exactly as `POST /{version}/products`
/// does, or a template prefills a form that will not submit.
///
/// `None` where the draft is free, where no price is typed, or where the price
/// typed clears the floor. `Some` carries the form's own sentence, rendered by
/// the same [`refusal_of_draft`] the check endpoint renders it with, so the two
/// surfaces cannot come to differently-worded conclusions about one rule.
///
/// Public where `DraftRefusal` and its renderer stay private: the caller needs
/// this one answer, and widening the refusal vocabulary would let a caller
/// assemble a verdict of its own instead of asking for one.
#[must_use]
pub fn stated_price_refusal(draft: &DraftInput) -> Option<RefusalView> {
    if draft.free {
        return None;
    }
    let stated = draft.price_minor_units?;
    let floor = min_price_minor_units();
    if stated >= floor {
        return None;
    }
    Some(refusal_of_draft(DraftRefusal::PriceUnderFloor {
        stated: Some(stated),
        floor,
    }))
}

/// The submission rules and the unknown-id refusals, in the order the form's
/// own groups read.
///
/// An unknown id is refused whether or not the draft is free, because the
/// sidecar refuses it on the create either way and the check must not call
/// submittable what the create will refuse.
fn draft_refusals(draft: &DraftInput) -> Vec<DraftRefusal> {
    let mut found = Vec::new();
    // Only where the draft is bound for a marketplace (D32). A resource kept
    // here is allowed to have no file yet, which is the whole of the change;
    // everything else about the rule, including the sentence it renders, is
    // unmoved.
    if draft.for_marketplace && draft.payload_hash.is_none() {
        found.push(DraftRefusal::PayloadMissing);
    }
    found.extend(unknown_id(
        IdControl::ThumbnailMode,
        draft.thumbnail_mode,
        ThumbnailMode::from_wire_id,
    ));
    if !draft.free {
        let floor = min_price_minor_units();
        let stated = draft.price_minor_units;
        if stated.is_none_or(|minor| minor < floor) {
            found.push(DraftRefusal::PriceUnderFloor { stated, floor });
        }
        if draft.tax_code_id.is_none() {
            found.push(DraftRefusal::TaxCodeUnstated);
        }
    }
    found.extend(unknown_id(
        IdControl::TaxCode,
        draft.tax_code_id,
        TaxCode::from_wire_id,
    ));
    found.extend(unknown_id(
        IdControl::TeachingDuration,
        draft.teaching_duration_id,
        TeachingDuration::from_wire_id,
    ));
    found.extend(unknown_id(
        IdControl::AnswerKey,
        draft.answer_key_id,
        AnswerKey::from_wire_id,
    ));
    found.extend(unknown_id(
        IdControl::Copyright,
        draft.copyright_declaration_id,
        CopyrightDeclaration::from_wire_id,
    ));
    found.extend(unknown_id(
        IdControl::Status,
        draft.status_user,
        ListingStatus::from_wire_id,
    ));
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
    use super::{verdict, DraftInput, RefusalView};

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

    /// D32, both directions, because the rule is about the destination and a
    /// test of one direction alone would pass on a rule that always refuses or
    /// one that never does.
    ///
    /// The defect the refusal itself fixes is older and still guarded: the
    /// draft reader returned early on a missing payload without recording
    /// anything, so a titled draft with no file came back submittable and only
    /// the client's own refusal caught it.
    #[test]
    fn a_draft_bound_for_a_marketplace_needs_a_downloadable_file() {
        let mut bare = paid();
        bare.payload_hash = None;
        bare.for_marketplace = true;
        let report = verdict(&bare);
        assert!(
            !report.submittable,
            "a marketplace will not list a product buyers cannot download"
        );
        assert!(
            controls(&bare).contains(&"Downloadable File".to_owned()),
            "and the refusal names the control rather than the wire field"
        );
    }

    #[test]
    fn a_draft_kept_here_needs_no_downloadable_file() {
        let mut kept = paid();
        kept.payload_hash = None;
        kept.for_marketplace = false;
        let report = verdict(&kept);
        assert!(
            report.submittable,
            "a resource kept on Teachouse may have no file yet, which is D32"
        );
        assert!(
            !controls(&kept).contains(&"Downloadable File".to_owned()),
            "and nothing asks for one"
        );
    }

    /// The same draft, refused and not refused by the destination alone.
    ///
    /// Holds the two tests above to one variable: if some other field of
    /// `paid()` started deciding the payload rule, both would still pass
    /// separately and this would not.
    #[test]
    fn the_destination_is_the_only_thing_that_decides_it() {
        let mut draft = paid();
        draft.payload_hash = None;
        draft.for_marketplace = false;
        let kept = verdict(&draft).submittable;
        draft.for_marketplace = true;
        let bound = verdict(&draft).submittable;
        assert_eq!(
            (kept, bound),
            (true, false),
            "one field moved and the verdict moved with it"
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

    /// The wave-5 render fixture offered tax codes with invented ids, 81111
    /// and 81112, and choosing one failed the whole decode with `invalid
    /// value: integer 81111, expected u8`, which the console drew as its
    /// error boundary. The value is now one refusal naming the one control.
    #[test]
    fn an_id_beyond_the_vocabulary_s_range_is_refused_by_the_control_that_holds_it() {
        let read: Result<DraftInput, _> = serde_json::from_str(r#"{"tax_code_id":81111}"#);
        assert!(
            read.is_ok(),
            "an id beyond a byte decodes, so the field can be refused rather than the draft"
        );
        let mut invented = paid();
        invented.tax_code_id = Some(81111);
        let report = verdict(&invented);
        assert!(
            !report.submittable,
            "a code no member has is not a designation"
        );
        let about_it: Vec<&RefusalView> = report
            .refusals
            .iter()
            .filter(|refusal| refusal.control.as_deref() == Some("Tax Code"))
            .collect();
        assert_eq!(
            about_it.len(),
            1,
            "one sentence for the one control, and not the unstated one as well: {:?}",
            report.refusals
        );
        assert_eq!(about_it[0].message, "Choose a tax code from the list.");
        assert_eq!(about_it[0].group, "price");
    }

    /// The same refusal for an id inside the byte that names no member, so
    /// the range and the vocabulary are one test rather than two.
    #[test]
    fn an_id_inside_the_range_that_no_member_has_is_refused_the_same_way() {
        let mut invented = paid();
        invented.tax_code_id = Some(200);
        let report = verdict(&invented);
        let about_it: Vec<&str> = report
            .refusals
            .iter()
            .filter(|refusal| refusal.control.as_deref() == Some("Tax Code"))
            .map(|refusal| refusal.message.as_str())
            .collect();
        assert_eq!(
            about_it,
            ["Choose a tax code from the list."],
            "200 fits a byte and is nobody's tax code, and is refused as one sentence"
        );
    }

    /// The six id controls, each refused by its own name and in the order the
    /// form's groups read, so no control can slip back to a silent default.
    #[test]
    fn every_id_control_left_unread_names_itself() {
        let mut invented = paid();
        invented.thumbnail_mode = Some(81111);
        invented.tax_code_id = Some(81111);
        invented.teaching_duration_id = Some(81111);
        invented.answer_key_id = Some(81111);
        invented.copyright_declaration_id = Some(81111);
        invented.status_user = Some(81111);
        assert_eq!(
            controls(&invented),
            vec![
                "Thumbnails".to_owned(),
                "Tax Code".to_owned(),
                "Teaching Duration".to_owned(),
                "Answer Key".to_owned(),
                "Copyright".to_owned(),
                "Product Status".to_owned(),
            ]
        );
    }

    /// A stated value is refused on a free draft as well, where the tax code
    /// control is hidden, because the sidecar refuses it on the create.
    #[test]
    fn an_unknown_id_is_refused_on_a_free_draft_too() {
        let mut free = paid();
        free.free = true;
        free.price_minor_units = None;
        free.tax_code_id = Some(81111);
        assert!(
            !verdict(&free).submittable,
            "the create's sidecar would refuse the row, so the check says so first"
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
        assert_eq!(
            refusals[0].message, "Choose one of the two copyright statements.",
            "the seller is told what to do; why nothing is pre-selected is ours to know"
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
                "Choose up to 4 under Grade Level; you have 5."
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
    fn thumbnails_under_a_mode_with_no_slots_are_refused() {
        let mut deferred = draft();
        deferred.thumbnail_mode = Some(3);
        deferred.thumbnail_hashes = vec![HASH.to_owned()];
        let refusals = report(&deferred);
        assert_eq!(refusals[0].group, "files");
        assert_eq!(
            refusals[0].message, "Choose \"Upload thumbnails now\", or remove the thumbnails.",
            "the two ways out of the contradiction, and no explanation of it"
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
        assert_eq!(
            advisory.message,
            "Free resources do best at 10 pages or fewer, and this one has 24."
        );
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
            "Shorten the name to 80 characters; it is 81 now."
        );
    }
}
