//! What TPT's create form will accept, read out of the committed capture
//! rather than transcribed beside it.
//!
//! Two facts live here, and both are properties of the form rather than of
//! the vocabulary, which is why neither could be recovered from the facet set
//! alone. How many members each picker takes, and which facets the form
//! offers a control for at all.
//!
//! The caps matter because `project_axis` treats a cap overflow as a decision
//! rather than a truncation: the whole resolved set goes into an election and
//! nothing is published partially narrowed. A cap absent is unmeasured and
//! never unlimited, so a picker whose stated cap a capture has already
//! exceeded reports none — enforcing it would raise a question about a set the
//! platform accepted, which is worse than not asking.
//!
//! The writable set matters because three of the twenty `Grade-Level` facets
//! are buyer-side roll-ups. `elementary`, `middle-school` and `high-school`
//! are the parents of the seventeen the form renders as checkboxes, they drive
//! browse filters, and the create form offers no control for any of them. They
//! are not `isHidden`: hidden means retired and still readable, while a
//! roll-up is current and unwritable. A projection that posted one would write
//! a slug into `data[TaxonomyTags][]` that no seller could have chosen.

use std::collections::BTreeSet;

use serde::Deserialize;
use tam_domain::registry::{Cardinality, CountCap};

/// One of the create form's own selection limits.
///
/// Named for the picker rather than for an equivalence axis, because the
/// correspondence is not one-to-one: `Tags` spans TPT's `theme`, `audience`
/// and `language` facets and `Thumbnails` counts upload slots rather than
/// vocabulary members, so neither is an axis the registry binds. Which
/// binding each should feed is a registry decision, not this module's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picker {
    /// The seventeen-checkbox grade grid.
    Grades,
    /// The `PreK-12-Subject-Area` multi-select.
    SubjectAreas,
    /// The `Tag (Theme, Audience, Language)` multi-select.
    Tags,
    /// The `Format` multi-select.
    Formats,
    /// The four `data[ItemDigital][thumb1..thumb4]` slots.
    Thumbnails,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    Parse(String),
    /// A picker the capture states no cap for. Absent is unmeasured rather
    /// than unlimited, so this is refused rather than defaulted.
    CapMissing(Picker),
}

impl core::fmt::Display for FormError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(detail) => {
                write!(f, "tpt-vocabulary.json is not the expected shape: {detail}")
            }
            Self::CapMissing(picker) => write!(
                f,
                "the capture states no selection cap for {picker:?}, and an absent cap is \
                 unmeasured rather than unlimited"
            ),
        }
    }
}

impl core::error::Error for FormError {}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capture {
    taxonomy_tags: FacetSet,
    constraints: Constraints,
}

#[derive(Debug, Clone, Deserialize)]
struct FacetSet {
    options: std::collections::BTreeMap<String, Facet>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Facet {
    /// Absent means writable. Only a facet measured to have no control on the
    /// form carries the marker, so silence is the ordinary case rather than an
    /// unmeasured one.
    #[serde(default)]
    seller_writable: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Constraints {
    selection_caps: SelectionCaps,
    upload_slots: UploadSlots,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionCaps {
    grades: Option<usize>,
    subject_areas: Option<usize>,
    tags: Option<usize>,
    formats: Option<usize>,
    /// Pickers a capture has been observed to exceed, named by the key they
    /// are stated under.
    #[serde(default)]
    contradicted_by_capture: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct UploadSlots {
    thumbnails: ThumbnailSlots,
}

#[derive(Debug, Clone, Deserialize)]
struct ThumbnailSlots {
    slots: Option<usize>,
}

/// The create form's measured limits and its unwritable facets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptForm {
    caps: SelectionCapsResolved,
    unwritable: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionCapsResolved {
    grades: Option<usize>,
    subject_areas: Option<usize>,
    tags: Option<usize>,
    formats: Option<usize>,
    thumbnails: Option<usize>,
}

impl TptForm {
    /// Reads both facts out of the committed capture.
    ///
    /// Pure: the capture arrives as text and the caller does the I/O, so
    /// `tam-taxonomy` stays inside the purity gate.
    pub fn read(capture: &str) -> Result<Self, FormError> {
        let parsed: Capture =
            serde_json::from_str(capture).map_err(|error| FormError::Parse(error.to_string()))?;
        let stated = &parsed.constraints.selection_caps;
        let contradicted = |key: &str, value: Option<usize>| {
            if stated
                .contradicted_by_capture
                .iter()
                .any(|named| named == key)
            {
                None
            } else {
                value
            }
        };
        Ok(Self {
            caps: SelectionCapsResolved {
                grades: contradicted("grades", stated.grades),
                subject_areas: contradicted("subjectAreas", stated.subject_areas),
                tags: contradicted("tags", stated.tags),
                formats: contradicted("formats", stated.formats),
                thumbnails: contradicted(
                    "thumbnails",
                    parsed.constraints.upload_slots.thumbnails.slots,
                ),
            },
            unwritable: parsed
                .taxonomy_tags
                .options
                .into_iter()
                .filter(|(_, facet)| facet.seller_writable == Some(false))
                .map(|(slug, _)| slug)
                .collect(),
        })
    }

    /// How many members one picker accepts, where the capture states a limit
    /// no capture has contradicted.
    #[must_use]
    pub fn cap(&self, picker: Picker) -> Option<CountCap> {
        let limit = match picker {
            Picker::Grades => self.caps.grades,
            Picker::SubjectAreas => self.caps.subject_areas,
            Picker::Tags => self.caps.tags,
            Picker::Formats => self.caps.formats,
            Picker::Thumbnails => self.caps.thumbnails,
        };
        limit.map(|limit| CountCap { limit })
    }

    /// The same limit in the form `AxisRequest` reads, so a caller binding
    /// one of these pickers to an equivalence axis hands the measured cap to
    /// the projection rather than restating it.
    ///
    /// `Many` in every case: none of these pickers refuses a set, and a
    /// contradicted or unstated cap yields `Many { cap: None }`, which is what
    /// makes the overflow path silent rather than wrong.
    #[must_use]
    pub fn cardinality(&self, picker: Picker) -> Cardinality {
        Cardinality::Many {
            cap: self.cap(picker),
        }
    }

    /// The measured limit, or a refusal. For a caller that must not proceed
    /// on an unmeasured cap.
    pub fn required_cap(&self, picker: Picker) -> Result<CountCap, FormError> {
        self.cap(picker).ok_or(FormError::CapMissing(picker))
    }

    /// Whether the create form offers the seller a control for this facet.
    ///
    /// True for every slug the capture does not mark otherwise, including
    /// slugs the capture does not hold at all: this answers "did we measure
    /// that the form withholds it", and an unknown slug is refused by the
    /// adapter's own shape guard rather than by this.
    #[must_use]
    pub fn is_seller_writable(&self, slug: &str) -> bool {
        !self.unwritable.contains(slug)
    }

    /// Every facet the form offers no control for, ascending by slug.
    pub fn unwritable(&self) -> impl Iterator<Item = &str> {
        self.unwritable.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::{FormError, Picker, TptForm};
    use crate::grades::derive_grade_crosswalk;
    use crate::project::{project_axis, AxisRequest};
    use tam_domain::equivalence::{ElectionTrigger, PricingBranch};
    use tam_domain::registry::{registry, AxisBinding, Cardinality, CountCap};
    use tam_domain::{TermKind, VocabularyId};
    use tam_types::{CanonicalTermId, InventoryId, ProductId, Timestamp, Uuid};

    const TPT: &str = include_str!("../../../docs/design/data/tpt-vocabulary.json");
    const TES: &str = include_str!("../../../docs/design/data/tes-vocabulary.json");
    const AT: Timestamp = Timestamp(1_700_000_000_000);
    const PRODUCT: ProductId = ProductId(Uuid([0x0f; 16]));

    fn form() -> TptForm {
        TptForm::read(TPT).expect("the committed capture states the form's own limits")
    }

    #[test]
    fn the_four_measured_caps_are_the_numbers_the_form_states() {
        let form = form();
        for (picker, limit, why) in [
            (Picker::Grades, 4, "Select up to four grades"),
            (
                Picker::Tags,
                6,
                "Select up to six tags, since the March 2025 overhaul",
            ),
            (Picker::Formats, 3, "three formats"),
            (
                Picker::Thumbnails,
                4,
                "four thumb boxes under generate_thumbnail mode 2",
            ),
        ] {
            assert_eq!(
                form.cap(picker),
                Some(CountCap { limit }),
                "{picker:?} is capped at {limit}: {why}"
            );
        }
    }

    /// The one cap a capture contradicts. Stating it would raise an election
    /// about a set TPT already accepted, so it reports unmeasured and the
    /// refusal path says so rather than defaulting.
    #[test]
    fn the_subject_area_cap_the_capture_exceeded_reports_unmeasured() {
        let form = form();
        assert_eq!(
            form.cap(Picker::SubjectAreas),
            None,
            "the form says three and the 2026-08-30 create posted four, so three is a claim \
             rather than a refusal"
        );
        assert_eq!(
            form.cardinality(Picker::SubjectAreas),
            Cardinality::Many { cap: None },
            "an unmeasured cap never truncates and never asks"
        );
        assert_eq!(
            form.required_cap(Picker::SubjectAreas),
            Err(FormError::CapMissing(Picker::SubjectAreas)),
            "a caller that must not proceed unmeasured is refused rather than defaulted"
        );
    }

    /// The cap as the projection spends it: over the limit is a question about
    /// the whole set, never a set silently cut to length.
    #[test]
    fn a_grade_set_over_the_measured_cap_elects_rather_than_truncating() {
        let crosswalk = derive_grade_crosswalk(TPT, TES, AT).expect("the captures derive");
        let form = form();
        let terms: Vec<CanonicalTermId> = [3_u64, 4, 5, 6, 7]
            .into_iter()
            .map(crate::grades::tpt_grade_term_id)
            .collect();
        let binding = AxisBinding {
            cardinality: form.cardinality(Picker::Grades),
            ..registry(InventoryId::Tpt)
                .axis(TermKind::Phase)
                .expect("TPT binds a phase axis")
        };
        let outcome = project_axis(
            AxisRequest {
                product: PRODUCT,
                inventory: InventoryId::Tpt,
                binding,
                terms: &terms,
                sources: &[],
                pricing: PricingBranch::Free,
                rules: &[],
                settled: &[],
            },
            &crosswalk.edges,
            &[],
        );
        let [election] = outcome.elections.as_slice() else {
            panic!("five grades against a cap of four is one question, got {outcome:?}");
        };
        let ElectionTrigger::OverCap { cap, from } = &election.trigger else {
            panic!("a set over the cap is an OverCap trigger");
        };
        assert_eq!(*cap, 4, "the form's own limit, read from the capture");
        assert_eq!(
            from.len(),
            5,
            "the whole resolved set goes into the question, so no four-grade subset exists \
             for a caller to publish by accident"
        );
        assert!(
            outcome.resolved.is_empty(),
            "nothing publishes until the seller says which four"
        );
    }

    /// The registry restates the grade cap as a const because a const cannot
    /// read a file. This is the join that keeps the restatement honest: if a
    /// re-poll moves the form's number, the two disagree here rather than
    /// silently at a seller's publish.
    #[test]
    fn the_declared_grade_cap_is_the_one_the_capture_states() {
        assert_eq!(
            registry(InventoryId::Tpt)
                .axis(TermKind::Phase)
                .map(|binding| binding.cardinality),
            Some(form().cardinality(Picker::Grades)),
            "the registry's declared grade cap and the capture's own selectionCaps.grades \
             are one number stated twice"
        );
    }

    #[test]
    fn the_three_buyer_side_roll_ups_are_the_whole_unwritable_set() {
        let form = form();
        assert_eq!(
            form.unwritable().collect::<Vec<_>>(),
            vec!["elementary", "high-school", "middle-school"],
            "seventeen of the twenty Grade-Level facets render as checkboxes, and the three \
             that do not are the band roll-ups that drive buyer-facing filters"
        );
        for writable in [
            "preschool",
            "4th-grade",
            "not-grade-specific",
            "higher-education",
        ] {
            assert!(
                form.is_seller_writable(writable),
                "{writable} is one of the seventeen the form renders"
            );
        }
    }

    /// The enforcement, stated over the relation the seeder actually writes: a
    /// roll-up reaching `data[TaxonomyTags][]` would put a slug on a seller's
    /// listing that no seller could have picked, and the grade derivation is
    /// the only thing that authors an edge into `(Tpt, Phase)`.
    #[test]
    fn no_seeded_edge_projects_into_a_facet_the_form_withholds() {
        let crosswalk = derive_grade_crosswalk(TPT, TES, AT).expect("the captures derive");
        let form = form();
        let target = VocabularyId(InventoryId::Tpt, TermKind::Phase);
        let mut checked = 0_usize;
        for edge in &crosswalk.edges {
            if edge.to.vocabulary != target {
                continue;
            }
            let native = edge
                .to
                .native_id
                .as_deref()
                .expect("a TPT path is addressed or it cannot be posted");
            assert!(
                form.is_seller_writable(native),
                "{native:?} is a buyer-side roll-up with no control on the create form, so \
                 posting it would write a slug the seller could not have chosen"
            );
            checked += 1;
        }
        assert_eq!(
            checked, 32,
            "nineteen grade options claimed once each, and the thirteen band coverings"
        );
    }
}
