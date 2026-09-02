//! The create form's own vocabulary: every controlled list its controls
//! render, served from the committed TPT capture rather than transcribed into
//! TypeScript.
//!
//! [`crate::vocabulary`] serves one marketplace's field table — what a native
//! field admits, which axes bind to it, which are measured absent. This serves
//! the canonical base model instead: TPT is the base by founder decision, so
//! the grade grid, the three multi-selects, the three listboxes, the thumbnail
//! radio, the copyright radio and the status switch are the product's own
//! controls rather than one platform's projection of them, and their members
//! come from `docs/design/data/tpt-vocabulary.json` in every case.
//!
//! Two rules the payload exists to enforce.
//!
//! Every option carries its wire id and its label together, and the list is
//! ordered by id. TPT's own Answer Key menu is ordered differently from its
//! ids — positions 3 and 5 carry ids 4 and 3 — so a client that inferred an id
//! from a menu position would write "Included with Rubric" as "Does Not
//! Apply", invisibly. `menu_index` is served beside the id for any client that
//! wants TPT's familiar order, and nothing converts the other way.
//!
//! Selection caps are served as measured, and absent means unmeasured rather
//! than unlimited. `tam_taxonomy::TptForm` reads them from the same capture
//! and drops the one a capture has contradicted, so a client rendering a
//! counter against `subject_areas` finds no number and renders a plain count.

use std::sync::OnceLock;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::product::{
    AnswerKey, CopyrightDeclaration, ListingStatus, SelectionCaps, StandardsFramework, TaxCode,
    TeachingDuration, ThumbnailMode, ADDITIONAL_LICENCE_PERCENTAGE, COPYRIGHT_PREAMBLE,
    FREE_RESOURCE_PAGE_GUIDANCE, TITLE_MAX_UTF16_UNITS,
};
use tam_taxonomy::TptForm;

use crate::error::APIError;
use crate::{AppState, OrgContext};

/// The capture, compiled in. The same file `tam-taxonomy` reads its own two
/// facts out of, so the form's members and the projection's caps cannot come
/// from different revisions of it.
const CAPTURE: &str = include_str!("../../../../docs/design/data/tpt-vocabulary.json");
pub mod check;
pub mod standards;

pub(crate) use check::{check_draft, record_of, DraftHead};
pub use check::{
    verdict, AdvisoryView, CheckView, DraftInput, RefusalView, StandardInput, TptBaseInput,
};
pub(crate) use standards::standards_search;
pub use standards::{StandardView, StandardsSearchParams, StandardsSearchView, StandardsState};

// ------------------------------------------------------------ the capture

#[derive(Debug, Clone, Deserialize)]
struct Capture {
    #[serde(rename = "taxonomyTags")]
    taxonomy_tags: FacetSet,
    #[serde(rename = "taxCodes")]
    tax_codes: KeyedOptions<TaxCodeRow>,
    #[serde(rename = "teachingDuration")]
    teaching_duration: KeyedOptions<LabelledRow>,
    #[serde(rename = "answerKey")]
    answer_key: KeyedOptions<LabelledRow>,
    constraints: Constraints,
}

#[derive(Debug, Clone, Deserialize)]
struct FacetSet {
    options: std::collections::BTreeMap<String, Facet>,
}

#[derive(Debug, Clone, Deserialize)]
struct KeyedOptions<T> {
    options: std::collections::BTreeMap<String, T>,
}

#[derive(Debug, Clone, Deserialize)]
struct Facet {
    name: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default, rename = "parentId")]
    parent_id: Option<String>,
    #[serde(default, rename = "legacyId")]
    legacy_id: Option<String>,
    #[serde(default, rename = "isHidden")]
    is_hidden: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct TaxCodeRow {
    #[serde(rename = "taxCode")]
    tax_code: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LabelledRow {
    label: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Constraints {
    #[serde(rename = "titleMaxLength")]
    title_max_length: usize,
    #[serde(rename = "descriptionMaxLength")]
    description_max_length: usize,
    #[serde(rename = "minPrice")]
    min_price: f64,
    #[serde(rename = "uploadSlots")]
    upload_slots: UploadSlots,
}

#[derive(Debug, Clone, Deserialize)]
struct UploadSlots {
    product: Slot,
    preview: Slot,
    thumbnails: Slot,
    #[serde(rename = "videoPreview")]
    video_preview: Slot,
}

#[derive(Debug, Clone, Deserialize)]
struct Slot {
    #[serde(rename = "maxSizeBytes")]
    max_size_bytes: u64,
    #[serde(default, rename = "fileExtensions")]
    file_extensions: Vec<String>,
}

/// Parsed once. The capture is a compiled-in constant, so a parse failure is a
/// build-time fact the tests catch rather than a request-time one.
fn capture() -> Option<&'static Capture> {
    static PARSED: OnceLock<Option<Capture>> = OnceLock::new();
    PARSED
        .get_or_init(|| serde_json::from_str(CAPTURE).ok())
        .as_ref()
}
// -------------------------------------------------------------- the views

/// One member of a `data[TaxonomyTags][]` picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetView {
    /// The slug, which is both the checkbox id's suffix and the wire value.
    pub slug: String,
    pub label: String,
    /// The broader facet this one sits under, where the capture records one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// False only where the capture measured that the create form renders no
    /// control for this facet. The three `Grade-Level` roll-ups are the whole
    /// set: `elementary`, `middle-school` and `high-school` drive buyer-facing
    /// browse filters and no seller can choose one, so a projection that
    /// posted one would write a slug nobody picked.
    pub seller_writable: bool,
}

/// One option of a listbox, radio group or switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionView {
    /// The wire value, as text so one shape serves every list.
    pub id: String,
    pub label: String,
    /// Where TPT's own menu puts this option, when that differs from the id
    /// order. Served so a client can render TPT's familiar order without ever
    /// deriving an id from a position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub menu_index: Option<u8>,
    /// The Avalara code behind a tax-code row, for the seller who recognises
    /// it. Never the label: the label is the full description TPT shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl OptionView {
    fn plain(id: u8, label: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            label: label.into(),
            menu_index: None,
            code: None,
        }
    }
}

/// The price floor as minor units.
///
/// The capture states it as a major-unit decimal, `minPrice: 0.95`, and TPT
/// sells in USD alone, so the minor unit is the cent. A value outside a
/// plausible price range reports no floor rather than a number arithmetic
/// invented, because a floor is refused against and a wrong one refuses a
/// price the marketplace would have taken.
fn floor_minor_units(major: f64) -> i64 {
    let cents = (major * 100.0).round();
    if !cents.is_finite() || !(0.0..=1_000_000.0).contains(&cents) {
        return 0;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "guarded above: the value is integral after round() and inside 0..=1e6, which i64 holds exactly"
    )]
    let minor = cents as i64;
    minor
}

/// A picker's measured limit. Absent means unmeasured, never unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grades: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_areas: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formats: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnails: Option<usize>,
}

/// One upload target, with the cap the form states on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotView {
    pub label: String,
    pub max_size_bytes: u64,
    pub file_extensions: Vec<String>,
}

/// The numbers the form states beside its own controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsView {
    /// UTF-16 code units, which is the unit `maxlength` counts in.
    pub title_max_utf16_units: usize,
    pub description_max_length: usize,
    pub min_price_minor_units: i64,
    /// The Multiple Licenses pre-fill, as a percentage of the price. A
    /// pre-fill and never a rule: TPT's own help centre says the seller may
    /// choose any discount, so a projection that derived this figure would
    /// overwrite a seller's own on every sync.
    pub additional_licence_percentage: i64,
    /// "Free resources should be 10 pages or fewer." Guidance on a tooltip
    /// rather than a validated bound, so a client states it and never blocks
    /// on it.
    pub free_resource_page_guidance: u32,
    pub product_file: SlotView,
    pub preview: SlotView,
    pub video_preview: SlotView,
    pub thumbnail: SlotView,
}

/// The copyright group: the preamble and the two attestations, both quoted in
/// full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyrightView {
    pub preamble: String,
    pub options: Vec<OptionView>,
    /// Always false. TPT pre-selects value `1` on a blank form; ours must
    /// pre-select nothing and refuse submission until the seller chooses,
    /// because the attestation is their legal statement and a default makes
    /// it ours. Served rather than assumed so the rule is one the client
    /// reads rather than one it remembers.
    pub preselect: bool,
}

/// One education-standards jurisdiction the form offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkView {
    pub jurisdiction_id: u32,
    pub name: String,
    pub button_label: String,
}

/// Everything the eight-group form needs to render its controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormVocabularyView {
    /// The seventeen writable grades in the column-major order TPT's own grid
    /// reads down, plus the three roll-ups marked unwritable so a client can
    /// explain their absence rather than silently omitting them.
    pub grades: Vec<FacetView>,
    /// How many cells each column of the grade grid holds, left to right.
    /// The arrangement is the segregation, so it is served rather than left
    /// to a client to guess: primary grades, middle grades, high-school
    /// grades, then the three non-grade bands.
    pub grade_columns: Vec<usize>,
    pub subject_areas: Vec<FacetView>,
    pub tags: Vec<FacetView>,
    pub formats: Vec<FacetView>,
    pub tax_codes: Vec<OptionView>,
    pub teaching_durations: Vec<OptionView>,
    /// In wire-id order. `menu_index` carries TPT's own position, which for
    /// this one list is not the same thing.
    pub answer_keys: Vec<OptionView>,
    pub thumbnail_modes: Vec<OptionView>,
    pub copyright: CopyrightView,
    pub statuses: Vec<OptionView>,
    pub standards_frameworks: Vec<FrameworkView>,
    pub caps: CapsView,
    pub limits: LimitsView,
}

// --------------------------------------------------------- the assembly

/// The grade grid's four columns, as the DOM read records them.
///
/// The capture holds no column information, so the sizes are stated here and
/// the members are derived: grades sort by their own `legacyId`, which runs
/// 1..16 then 23 and is exactly the grid's column-major order. A test pins the
/// total against the writable facet count, so a re-poll that adds or retires a
/// grade fails the build here rather than producing a ragged grid.
const GRADE_COLUMN_SIZES: [usize; 4] = [5, 5, 4, 3];

/// The one facet category whose members are grades.
const GRADE_CATEGORY: &str = "Grade-Level";

fn facets_of(capture: &Capture, form: &TptForm, category: &str) -> Vec<FacetView> {
    let mut found: Vec<FacetView> = capture
        .taxonomy_tags
        .options
        .iter()
        .filter(|(_, facet)| facet.category.as_deref() == Some(category))
        // A hidden facet is retired: it reads back on an existing product and
        // is never offered on write, so it has no control on this form.
        .filter(|(_, facet)| facet.is_hidden != Some(true))
        .map(|(slug, facet)| FacetView {
            slug: slug.clone(),
            label: facet.name.clone(),
            parent: facet.parent_id.clone(),
            seller_writable: form.is_seller_writable(slug),
        })
        .collect();
    found.sort_by(|left, right| left.label.cmp(&right.label));
    found
}

/// The grades in the order the grid reads down its columns.
///
/// `legacyId` order rather than alphabetical, because that is the order the
/// grid's cells run in, and the grid's arrangement is what makes the four
/// bands legible. The three roll-ups carry no `legacyId` and sort last, where
/// a client renders them as an explanation rather than as cells.
fn grades_of(capture: &Capture, form: &TptForm) -> Vec<FacetView> {
    let mut found: Vec<(Option<u32>, FacetView)> = capture
        .taxonomy_tags
        .options
        .iter()
        .filter(|(_, facet)| facet.category.as_deref() == Some(GRADE_CATEGORY))
        .filter(|(_, facet)| facet.is_hidden != Some(true))
        .map(|(slug, facet)| {
            (
                facet.legacy_id.as_deref().and_then(|id| id.parse().ok()),
                FacetView {
                    slug: slug.clone(),
                    label: facet.name.clone(),
                    parent: facet.parent_id.clone(),
                    seller_writable: form.is_seller_writable(slug),
                },
            )
        })
        .collect();
    found.sort_by(|left, right| match (left.0, right.0) {
        (Some(one), Some(other)) => one.cmp(&other),
        (Some(_), None) => core::cmp::Ordering::Less,
        (None, Some(_)) => core::cmp::Ordering::Greater,
        (None, None) => left.1.slug.cmp(&right.1.slug),
    });
    found.into_iter().map(|(_, facet)| facet).collect()
}

fn caps_of(form: &TptForm) -> CapsView {
    let cap = |picker| {
        form.cap(picker)
            .map(|tam_domain::registry::CountCap { limit }| limit)
    };
    CapsView {
        grades: cap(tam_taxonomy::Picker::Grades),
        subject_areas: cap(tam_taxonomy::Picker::SubjectAreas),
        tags: cap(tam_taxonomy::Picker::Tags),
        formats: cap(tam_taxonomy::Picker::Formats),
        thumbnails: cap(tam_taxonomy::Picker::Thumbnails),
    }
}

/// The same numbers as the pure model reads them, so the API and the domain
/// check against one source.
#[must_use]
pub fn selection_caps() -> SelectionCaps {
    let Some(form) = form() else {
        return SelectionCaps::default();
    };
    let caps = caps_of(form);
    SelectionCaps {
        grades: caps.grades,
        subject_areas: caps.subject_areas,
        tags: caps.tags,
        formats: caps.formats,
        thumbnails: caps.thumbnails,
    }
}

fn form() -> Option<&'static TptForm> {
    static READ: OnceLock<Option<TptForm>> = OnceLock::new();
    READ.get_or_init(|| TptForm::read(CAPTURE).ok()).as_ref()
}

fn slot_view(label: &str, slot: &Slot) -> SlotView {
    SlotView {
        label: label.to_owned(),
        max_size_bytes: slot.max_size_bytes,
        file_extensions: slot.file_extensions.clone(),
    }
}

/// The whole payload, assembled from the capture.
///
/// `None` only where the compiled-in capture does not parse, which is a
/// build-time fact rather than a request-time one; the handler turns it into
/// an internal fault rather than serving a half-populated form.
#[must_use]
pub fn form_vocabulary() -> Option<FormVocabularyView> {
    let capture = capture()?;
    let form = form()?;
    let constraints = &capture.constraints;
    Some(FormVocabularyView {
        grades: grades_of(capture, form),
        grade_columns: GRADE_COLUMN_SIZES.to_vec(),
        subject_areas: facets_of(capture, form, "PreK-12-Subject-Area"),
        // TPT's own label is "Tag (Theme, Audience, Language)", and the three
        // facet categories behind it are exactly those. They are one picker on
        // the form, so they are one list here.
        tags: ["theme", "audience", "language"]
            .into_iter()
            .flat_map(|category| facets_of(capture, form, category))
            .collect(),
        formats: facets_of(capture, form, "Format"),
        tax_codes: TaxCode::ALL
            .into_iter()
            .map(|code| {
                let row = capture.tax_codes.options.get(&code.wire_id().to_string());
                OptionView {
                    id: code.wire_id().to_string(),
                    label: row
                        .map_or_else(|| code.avalara_code().to_owned(), |row| row.name.clone()),
                    menu_index: None,
                    code: Some(row.map_or_else(
                        || code.avalara_code().to_owned(),
                        |row| row.tax_code.clone(),
                    )),
                }
            })
            .collect(),
        teaching_durations: TeachingDuration::all()
            .map(|duration| {
                OptionView::plain(
                    duration.wire_id(),
                    capture
                        .teaching_duration
                        .options
                        .get(&duration.wire_id().to_string())
                        .map_or_else(|| duration.label().to_owned(), |row| row.label.clone()),
                )
            })
            .collect(),
        answer_keys: AnswerKey::ALL
            .into_iter()
            .map(|key| OptionView {
                id: key.wire_id().to_string(),
                label: capture
                    .answer_key
                    .options
                    .get(&key.wire_id().to_string())
                    .map_or_else(|| key.label().to_owned(), |row| row.label.clone()),
                menu_index: Some(key.menu_index()),
                code: None,
            })
            .collect(),
        thumbnail_modes: ThumbnailMode::ALL
            .into_iter()
            .map(|mode| OptionView::plain(mode.wire_id(), mode.label()))
            .collect(),
        copyright: CopyrightView {
            preamble: COPYRIGHT_PREAMBLE.to_owned(),
            options: CopyrightDeclaration::ALL
                .into_iter()
                .map(|declaration| {
                    OptionView::plain(declaration.wire_id(), declaration.statement())
                })
                .collect(),
            preselect: false,
        },
        statuses: ListingStatus::ALL
            .into_iter()
            .map(|status| {
                OptionView::plain(
                    status.wire_id(),
                    match status {
                        ListingStatus::Draft => "Draft, visible only to you",
                        ListingStatus::Live => "Make Listing Active",
                    },
                )
            })
            .collect(),
        standards_frameworks: StandardsFramework::ALL
            .into_iter()
            .map(|framework| FrameworkView {
                jurisdiction_id: framework.jurisdiction_id(),
                name: framework.name().to_owned(),
                button_label: framework.button_label().to_owned(),
            })
            .collect(),
        caps: caps_of(form),
        limits: LimitsView {
            title_max_utf16_units: constraints.title_max_length.min(TITLE_MAX_UTF16_UNITS),
            description_max_length: constraints.description_max_length,
            min_price_minor_units: floor_minor_units(constraints.min_price),
            additional_licence_percentage: ADDITIONAL_LICENCE_PERCENTAGE,
            free_resource_page_guidance: FREE_RESOURCE_PAGE_GUIDANCE,
            product_file: slot_view("Downloadable File", &constraints.upload_slots.product),
            preview: slot_view("Preview", &constraints.upload_slots.preview),
            video_preview: slot_view("Video Preview", &constraints.upload_slots.video_preview),
            thumbnail: slot_view("Thumbnail", &constraints.upload_slots.thumbnails),
        },
    })
}

pub(crate) async fn form_vocabulary_view(
    State(state): State<AppState>,
    _context: OrgContext,
) -> Result<Json<FormVocabularyView>, APIError> {
    form_vocabulary().map(Json).ok_or_else(|| {
        state.internal("the committed TPT capture did not parse; the form cannot be rendered")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        capture, form, form_vocabulary, selection_caps, GRADE_CATEGORY, GRADE_COLUMN_SIZES,
    };
    use tam_domain::product::{AnswerKey, TaxCode};

    fn rendered() -> super::FormVocabularyView {
        form_vocabulary().expect("the committed capture parses and the form reads")
    }

    #[test]
    fn the_committed_capture_parses_into_every_list_the_form_renders() {
        assert!(
            capture().is_some(),
            "the capture is compiled in, so a parse failure is a build fact"
        );
        let view = rendered();
        assert_eq!(
            view.grades.len(),
            20,
            "seventeen writable grades and three roll-ups"
        );
        assert_eq!(
            view.grades
                .iter()
                .filter(|grade| grade.seller_writable)
                .count(),
            17,
            "the create form renders seventeen checkboxes"
        );
        assert_eq!(view.tax_codes.len(), 5);
        assert_eq!(view.teaching_durations.len(), 23);
        assert_eq!(view.answer_keys.len(), 6);
        assert_eq!(view.thumbnail_modes.len(), 3);
        assert_eq!(view.copyright.options.len(), 2);
        assert_eq!(view.statuses.len(), 2);
        assert_eq!(view.standards_frameworks.len(), 4);
    }

    /// The grid's arrangement is the segregation, so the column sizes and the
    /// members have to agree: a re-poll that adds or retires a grade fails
    /// here rather than rendering a ragged grid.
    #[test]
    fn the_grade_columns_account_for_every_writable_grade() {
        let view = rendered();
        let writable: Vec<&str> = view
            .grades
            .iter()
            .filter(|grade| grade.seller_writable)
            .map(|grade| grade.slug.as_str())
            .collect();
        assert_eq!(
            GRADE_COLUMN_SIZES.iter().sum::<usize>(),
            writable.len(),
            "five, five, four and three is seventeen cells"
        );
        assert_eq!(
            [writable[0], writable[5], writable[10], writable[14]],
            ["preschool", "4th-grade", "9th-grade", "higher-education"],
            "each column starts where the DOM's own grid starts one: primary grades, middle \
             grades, high-school grades, then the three non-grade bands"
        );
    }

    /// The three roll-ups reach the client marked rather than omitted, so a
    /// form can say why a browse filter's band has no checkbox.
    #[test]
    fn the_buyer_side_roll_ups_are_served_and_marked_unwritable() {
        let view = rendered();
        let held: Vec<&str> = view
            .grades
            .iter()
            .filter(|grade| !grade.seller_writable)
            .map(|grade| grade.slug.as_str())
            .collect();
        assert_eq!(held, ["elementary", "high-school", "middle-school"]);
    }

    /// The trap, restated on the wire: the list is ordered by id and each
    /// option carries TPT's own menu position beside it, so nothing has to
    /// derive one from the other.
    #[test]
    fn the_answer_keys_are_served_in_id_order_carrying_their_menu_position() {
        let view = rendered();
        assert_eq!(
            view.answer_keys
                .iter()
                .map(|option| (option.id.as_str(), option.label.as_str(), option.menu_index))
                .collect::<Vec<_>>(),
            vec![
                ("0", "N/A", Some(0)),
                ("1", "Included", Some(1)),
                ("2", "Not Included", Some(2)),
                ("3", "Does Not Apply", Some(5)),
                ("4", "Included with Rubric", Some(3)),
                ("5", "Rubric Only", Some(4)),
            ],
            "id 3 is Does Not Apply and sits last in TPT's menu; id 4 is Included with Rubric \
             and sits fourth, which is the mis-map a position-derived id would make"
        );
        assert_eq!(
            AnswerKey::from_wire_id(4).map(AnswerKey::label),
            Some("Included with Rubric"),
            "and the domain and the capture agree about which is which"
        );
    }

    #[test]
    fn the_tax_codes_carry_the_full_description_and_the_avalara_code_apart() {
        let view = rendered();
        let first = view.tax_codes.first().expect("five rows");
        assert_eq!(
            (first.id.as_str(), first.code.as_deref()),
            ("1", Some("DA051011")),
            "the wire id is the row id and never the Avalara string"
        );
        assert_eq!(
            first.label, "Digital audio works sold to an end user with rights for permanent use",
            "the select shows TPT's full description rather than an abbreviation"
        );
        assert_eq!(
            TaxCode::ALL.len(),
            view.tax_codes.len(),
            "the domain's closed set and the capture's rows are one list"
        );
    }

    /// The one rule our form inverts.
    #[test]
    fn the_copyright_group_is_served_with_nothing_preselected() {
        let view = rendered();
        assert!(
            !view.copyright.preselect,
            "TPT pre-selects value 1 on a blank form; a default attestation would be ours \
             rather than the seller's"
        );
        assert!(
            view.copyright.options[0]
                .label
                .starts_with("I attest that this product"),
            "both statements are quoted in full, because an abbreviated attestation is a \
             different attestation"
        );
        assert!(view
            .copyright
            .preamble
            .starts_with("Intellectual Property Rights:"));
    }

    /// The caps the API serves and the caps the domain validates against are
    /// one reading of one file.
    #[test]
    fn the_served_caps_are_the_caps_the_model_checks_against() {
        let view = rendered();
        let caps = selection_caps();
        assert_eq!(
            (
                caps.grades,
                caps.subject_areas,
                caps.tags,
                caps.formats,
                caps.thumbnails
            ),
            (
                view.caps.grades,
                view.caps.subject_areas,
                view.caps.tags,
                view.caps.formats,
                view.caps.thumbnails
            ),
        );
        assert_eq!(
            (caps.grades, caps.tags, caps.formats, caps.thumbnails),
            (Some(4), Some(6), Some(3), Some(4)),
            "four grades, six tags, three formats and four thumbnail slots"
        );
        assert_eq!(
            caps.subject_areas, None,
            "the one cap a capture has already exceeded is served as unmeasured, so a client \
             renders a plain count rather than a counter against a number it would refuse on"
        );
        assert_eq!(
            form().and_then(|form| form.cap(tam_taxonomy::Picker::Grades)),
            Some(tam_domain::registry::CountCap { limit: 4 }),
            "and both readings come from `TptForm`, not from a literal"
        );
    }

    #[test]
    fn the_pickers_the_capture_names_are_the_categories_the_form_renders() {
        let view = rendered();
        assert_eq!(
            view.subject_areas.len(),
            133,
            "140 PreK-12-Subject-Area facets less the seven marked hidden; a hidden facet is \
             retired, reads back on an existing product and has no control on the form"
        );
        assert_eq!(
            view.tags.len(),
            46,
            "TPT's own label is `Tag (Theme, Audience, Language)`, so its 39 theme, 4 visible \
             audience and 3 language facets are one picker here as they are one there"
        );
        assert_eq!(view.formats.len(), 26, "Format facets");
        assert_eq!(GRADE_CATEGORY, "Grade-Level");
    }

    #[test]
    fn the_stated_limits_are_the_capture_s_own_numbers() {
        let limits = rendered().limits;
        assert_eq!(limits.title_max_utf16_units, 80);
        assert_eq!(limits.description_max_length, 45_000);
        assert_eq!(limits.min_price_minor_units, 95);
        assert_eq!(limits.additional_licence_percentage, 90);
        assert_eq!(limits.free_resource_page_guidance, 10);
        assert_eq!(limits.product_file.max_size_bytes, 4_294_967_296);
        assert_eq!(limits.preview.max_size_bytes, 31_457_280);
        assert_eq!(limits.video_preview.max_size_bytes, 1_073_741_824);
        assert_eq!(limits.thumbnail.max_size_bytes, 4_194_304);
    }
}
