//! The per-inventory field registry: the per-platform constraints on the
//! canonical six, the native fields that exist on one platform's wire and
//! nowhere else, and the equivalence axes each inventory binds.
//!
//! `FieldKey` stays closed at six and `FieldPolicies` stays a six-field
//! struct, so a platform-particular field gets a second axis here rather than
//! a seventh key. `CanonicalFields` mirrors the `FieldPolicies` shape, which
//! makes a new `FieldKey` a compile error in this module too.
//!
//! Every entry records a measured or documented fact. An unmeasured cap is
//! absent rather than guessed, and a vocabulary documented as closed upstream
//! whose members we have not captured says exactly that; no value here is
//! invented to fill a hole. The same discipline governs the axes: an axis
//! missing from `equivalence_axes` is unmeasured, and a measured absence is
//! an explicit `AxisAbsent` entry, because disclosing a loss and blocking on
//! an unknown are different answers.
//!
//! The per-inventory data lives in `tes`, `etsy` and `tpt` beside this file.
//! That split is a readability choice and not compliance with a limit:
//! `tam-limits` declares no line-count bound and nothing enforces one.

mod etsy;
pub mod listing_url;
mod tes;
mod tpt;

use crate::product::FormGroup;
use crate::TermKind;
use tam_types::{FieldKey, InventoryId, LengthUnit, TransportClass};

/// A length limit and the unit the marketplace counts it in. The unit is part
/// of the fact: a cap whose counting unit is unverified is not recordable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthCap {
    pub limit: usize,
    pub unit: LengthUnit,
}

/// How many members a set-valued field accepts, where a refusal has been
/// measured. A distinct type from `LengthCap` because one bounds text and
/// needs a counting unit to mean anything, while this one counts members and
/// has no unit to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountCap {
    pub limit: usize,
}

/// What one inventory declares about one of the canonical six.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    /// Absent means unmeasured, never unlimited.
    pub cap: Option<LengthCap>,
    /// True where the platform is documented or measured to refuse a create
    /// without the field. False records no such finding, which is not
    /// evidence that the field is optional.
    pub required: bool,
}

impl FieldSpec {
    /// Neither a cap nor a create-time requirement is on file.
    pub const UNRECORDED: Self = Self {
        cap: None,
        required: false,
    };

    /// Required at create, with no length cap on file.
    pub const REQUIRED: Self = Self {
        cap: None,
        required: true,
    };
}

/// One field per `FieldKey`, mirroring `FieldPolicies`, so a spec can be
/// neither missing nor unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalFields {
    pub title: FieldSpec,
    pub description: FieldSpec,
    pub price: FieldSpec,
    pub taxonomy: FieldSpec,
    pub grades: FieldSpec,
    pub files: FieldSpec,
}

impl CanonicalFields {
    /// Nothing recorded for any of the six.
    pub const UNRECORDED: Self = Self {
        title: FieldSpec::UNRECORDED,
        description: FieldSpec::UNRECORDED,
        price: FieldSpec::UNRECORDED,
        taxonomy: FieldSpec::UNRECORDED,
        grades: FieldSpec::UNRECORDED,
        files: FieldSpec::UNRECORDED,
    };

    #[must_use]
    pub const fn get(&self, key: FieldKey) -> &FieldSpec {
        match key {
            FieldKey::Title => &self.title,
            FieldKey::Description => &self.description,
            FieldKey::Price => &self.price,
            FieldKey::Taxonomy => &self.taxonomy,
            FieldKey::Grades => &self.grades,
            FieldKey::Files => &self.files,
        }
    }
}

/// Which way a native field crosses the adapter seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldDirection {
    /// The adapter writes it and does not read it back.
    Written,
    /// The adapter reads it and never writes it.
    ReadOnly,
    /// Written on create and read back on import.
    Both,
}

/// What is known about a native field's admissible values. The three closed
/// answers are deliberately distinct: holding a captured set, knowing a set is
/// closed without holding it, and having established nothing are different
/// facts, and collapsing them is how invented values get written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeVocabulary {
    /// Closed, and every member is captured here.
    Closed(&'static [&'static str]),
    /// Documented closed upstream, with the members not captured. Never
    /// substitute a guess: an unlisted member is refused by the platform.
    ClosedUncaptured,
    /// Free text, bounded only by the platform's own limits.
    Free,
    /// A numeric identifier or count, with no enumerable vocabulary.
    Numeric,
    /// Neither captured nor documented closed.
    Unmeasured,
}

/// How many values one native field takes. Optionality is `required`, not a
/// cardinality: `One` with `required: false` is a field that may be omitted
/// and refuses a set, which is exactly Tes `mainType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// The platform refuses a set. Tes `primaryCategory` and `mainType`.
    One,
    /// A set. `cap` is the platform's own selection limit where one is
    /// measured; absent means unmeasured, never unlimited.
    Many { cap: Option<CountCap> },
}

/// Whether the seller may hand this axis or field to a best-fit computation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delegation {
    /// The seller decides by default and may opt into best-fit.
    ByOptIn,
    /// The seller decides, always. No opt-in admits a computed answer.
    Never(NonDelegable),
}

/// Why an axis or field refuses delegation, as a value rather than a comment,
/// so a second non-delegable one must state which bar it clears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonDelegable {
    /// A rights grant or an attestation of authorship. Choosing one is making
    /// it, and the decision record confines us to the seller's own
    /// instruction on legal content.
    LegalContent,
}

/// One step of the Tes uploader wizard, in the order it walks them.
///
/// Tes's uploader is five ordered screens rather than one page of headings,
/// captured as "Description, Add Files, Categories, Licence, Publish" in
/// `docs/notes/probes/02-upload-request-shape.md`. They are a different shape
/// from TPT's nine sections and are not a translation of them: TPT names a
/// Price section Tes folds into Licence, and Tes names a Publish step TPT
/// spells Product Status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TesStep {
    Description,
    Files,
    Categories,
    Licence,
    Publish,
}

impl TesStep {
    pub const ALL: [Self; 5] = [
        Self::Description,
        Self::Files,
        Self::Categories,
        Self::Licence,
        Self::Publish,
    ];

    /// The step as the uploader's own wizard names it.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Description => "Description",
            Self::Files => "Add Files",
            Self::Categories => "Categories",
            Self::Licence => "Licence",
            Self::Publish => "Publish",
        }
    }
}

/// The section of one platform's own authoring form that holds a field.
///
/// Two arms rather than one flat list, because the two platforms' form shapes
/// are separate facts and neither is the other's translation. TPT's arm
/// carries [`FormGroup`] itself rather than restating its nine values, so a
/// section added there is a compile error here instead of a second list to
/// keep in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldGroup {
    Tes(TesStep),
    Tpt(FormGroup),
}

impl FieldGroup {
    /// Every group, in the order a form renders them: Tes's five steps in
    /// wizard order, then TPT's nine sections in TPT's own order. No
    /// inventory carries groups from both arms, so one order serves both.
    pub const ALL: [Self; 14] = [
        Self::Tes(TesStep::Description),
        Self::Tes(TesStep::Files),
        Self::Tes(TesStep::Categories),
        Self::Tes(TesStep::Licence),
        Self::Tes(TesStep::Publish),
        Self::Tpt(FormGroup::Name),
        Self::Tpt(FormGroup::Files),
        Self::Tpt(FormGroup::Description),
        Self::Tpt(FormGroup::Price),
        Self::Tpt(FormGroup::Categories),
        Self::Tpt(FormGroup::EducationStandards),
        Self::Tpt(FormGroup::Details),
        Self::Tpt(FormGroup::Copyright),
        Self::Tpt(FormGroup::ProductStatus),
    ];

    /// The token this group is named by on the wire. Namespaced by
    /// marketplace because "categories" names a Tes wizard step and a TPT
    /// section that hold different fields.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Tes(TesStep::Description) => "tes_description",
            Self::Tes(TesStep::Files) => "tes_files",
            Self::Tes(TesStep::Categories) => "tes_categories",
            Self::Tes(TesStep::Licence) => "tes_licence",
            Self::Tes(TesStep::Publish) => "tes_publish",
            Self::Tpt(group) => match group {
                FormGroup::Name => "tpt_name",
                FormGroup::Files => "tpt_files",
                FormGroup::Description => "tpt_description",
                FormGroup::Price => "tpt_price",
                FormGroup::Categories => "tpt_categories",
                FormGroup::EducationStandards => "tpt_education_standards",
                FormGroup::Details => "tpt_details",
                FormGroup::Copyright => "tpt_copyright",
                FormGroup::ProductStatus => "tpt_product_status",
            },
        }
    }

    /// The heading as the platform's own form reads it.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Tes(step) => step.heading(),
            Self::Tpt(group) => group.heading(),
        }
    }
}

/// Where a field sits on the platform's own authoring form.
///
/// A form shape rather than a field fact, which is why it is optional: a
/// field the platform's form has no control for carries none, and so does
/// every field of an inventory whose form was never captured.
///
/// `ordinal` is the field's position among its section's own controls, and
/// orders nothing across sections: the render order is `FieldGroup::ALL`
/// first and this second. It counts controls rather than native fields, so a
/// gap is a slot held by a control that is not a native field of its own —
/// several pickers feeding one wire field, or one of the canonical six.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormPlacement {
    pub group: FieldGroup,
    pub ordinal: u8,
}

/// A field particular to one platform's wire, named as that wire names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeField {
    pub name: &'static str,
    /// The words the platform's own form heads this control with, where a
    /// capture recorded them, and `None` where none did.
    ///
    /// `None` is not a hole to fill: the caller shows the wire name, exactly
    /// as a value with no captured label shows its own token. Inventing a
    /// reading of `mainType` would put a claim on a seller's screen that no
    /// capture supports, and a field assembled from several controls — TPT's
    /// `taxonomyTags`, which five pickers feed — has no single label to
    /// record.
    pub label: Option<&'static str>,
    /// Where the platform's own form puts this field, where a capture placed
    /// it. Absent means the form has no control for it, or that no form
    /// capture exists for this inventory at all.
    pub placement: Option<FormPlacement>,
    pub direction: FieldDirection,
    /// Read as `FieldSpec::required`: a recorded refusal, not an absence.
    pub required: bool,
    pub vocabulary: NativeVocabulary,
    /// Legal content is a property of a field, not only of an axis: TPT's
    /// `copyright_declaration` is an attestation with no equivalence axis
    /// above it, and the one mechanism that says "a machine may never supply
    /// this" has to reach it.
    pub delegation: Delegation,
}

/// One equivalence axis as one inventory binds it. An axis absent from an
/// inventory's `equivalence_axes` is *unmeasured*, not absent on the platform;
/// measured absence is an explicit `AxisAbsent` entry. The distinction mirrors
/// `NativeVocabulary`'s existing three-way one and is what keeps Etsy blocking
/// rather than silently disclosing a loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisBinding {
    pub axis: TermKind,
    /// The `NativeField.name` this axis lands in. The correspondence is real
    /// and `every_bound_axis_names_a_native_field_beside_it` holds it.
    /// Requiredness is not restated here: it is read off that `NativeField`,
    /// so the two declarations cannot disagree.
    pub native: &'static str,
    pub cardinality: Cardinality,
    pub delegation: Delegation,
}

/// An axis measured to be absent from an inventory. TPT holds no licence
/// field anywhere on its wire, which is the whole legal exemplar declared as
/// data: a licence projected into TPT is a disclosed loss, while a licence
/// projected into an inventory that merely declares no binding is unmeasured
/// and blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisAbsent(pub TermKind);

/// One inventory's axes: the constraints on the canonical six, the native
/// fields beside them, and the equivalence axes those fields carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryRegistry {
    pub inventory: InventoryId,
    pub canonical: CanonicalFields,
    pub natives: &'static [NativeField],
    pub equivalence_axes: &'static [AxisBinding],
    pub absent_axes: &'static [AxisAbsent],
}

impl InventoryRegistry {
    /// The binding for one axis, or `None` where this inventory declares
    /// none. `None` does not distinguish measured absence from silence; ask
    /// `declares_absent` for that.
    #[must_use]
    pub fn axis(&self, axis: TermKind) -> Option<AxisBinding> {
        self.equivalence_axes
            .iter()
            .copied()
            .find(|binding| binding.axis == axis)
    }

    /// Whether this inventory was measured to lack the axis.
    #[must_use]
    pub fn declares_absent(&self, axis: TermKind) -> bool {
        self.absent_axes.iter().any(|absent| absent.0 == axis)
    }

    /// Which branch of the automation rule this inventory's marketplace falls
    /// in. Read off the marketplace rather than restated here, so the registry
    /// and `tam_types::Marketplace` cannot disagree about a transport.
    #[must_use]
    pub const fn transport_class(&self) -> TransportClass {
        self.inventory.marketplace().transport_class()
    }

    /// The native field one of this inventory's wire names refers to.
    #[must_use]
    pub fn native(&self, name: &str) -> Option<NativeField> {
        self.natives
            .iter()
            .copied()
            .find(|native| native.name == name)
    }

    /// The form sections this inventory's fields fall in, in render order and
    /// without repeats. Empty where no form capture placed any field, which
    /// is Etsy.
    #[must_use]
    pub fn groups(&self) -> Vec<FieldGroup> {
        FieldGroup::ALL
            .into_iter()
            .filter(|group| {
                self.natives.iter().any(|native| {
                    native
                        .placement
                        .is_some_and(|placement| placement.group == *group)
                })
            })
            .collect()
    }
}

/// Total over `InventoryId::ALL` by exhaustive match, so a new inventory is a
/// compile error rather than a silent absence.
#[must_use]
pub const fn registry(inventory: InventoryId) -> &'static InventoryRegistry {
    match inventory {
        InventoryId::Tes => &tes::TES,
        InventoryId::Etsy => &etsy::ETSY,
        InventoryId::Tpt => &tpt::TPT,
    }
}

/// Truncates to the cap at the cap's own unit, never splitting a UTF-8 code
/// point: whole scalar values are appended while the budget affords them, so
/// a cap falling mid-codepoint or mid-surrogate-pair floors to the boundary
/// below it.
#[must_use]
pub fn truncate(text: &str, cap: LengthCap) -> String {
    let mut kept = String::new();
    let mut used: usize = 0;
    for character in text.chars() {
        let width = match cap.unit {
            LengthUnit::Bytes => character.len_utf8(),
            LengthUnit::Utf16CodeUnits => character.len_utf16(),
            // N codepoints can never exceed N grapheme clusters, so counting
            // codepoints under-approximates a grapheme budget: the result can
            // be shorter than the cap allows but never longer than it permits.
            LengthUnit::Codepoints | LengthUnit::GraphemeClusters => 1,
        };
        let Some(next) = used.checked_add(width) else {
            break;
        };
        if next > cap.limit {
            break;
        }
        kept.push(character);
        used = next;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{
        registry, truncate, Delegation, FieldGroup, FieldSpec, LengthCap, NativeVocabulary,
    };
    use tam_types::{FieldKey, InventoryId, LengthUnit, TransportClass};

    fn cap(limit: usize, unit: LengthUnit) -> LengthCap {
        LengthCap { limit, unit }
    }

    #[test]
    fn every_inventory_resolves_to_a_registry_keyed_on_itself() {
        for inventory in InventoryId::ALL {
            assert_eq!(
                registry(inventory).inventory,
                inventory,
                "the registry is total over InventoryId::ALL and keyed on its own inventory"
            );
        }
    }

    /// The registry is where a reader finds the transport class, and every
    /// inventory of a no-API marketplace reads as the seller-device branch.
    /// The expected value is a literal rather than a second call, so an
    /// inventory re-pointed at another marketplace fails here.
    #[test]
    fn every_registered_inventory_reads_its_marketplaces_transport_class() {
        for inventory in InventoryId::ALL {
            let expected = match inventory {
                InventoryId::Tes | InventoryId::Tpt => TransportClass::SellerDevice,
                InventoryId::Etsy => TransportClass::OfficialApi,
            };
            assert_eq!(
                registry(inventory).transport_class(),
                expected,
                "{inventory:?} must read the branch its marketplace declares"
            );
        }
    }

    #[test]
    fn a_captured_closed_vocabulary_is_never_empty() {
        for inventory in InventoryId::ALL {
            for native in registry(inventory).natives {
                if let NativeVocabulary::Closed(values) = native.vocabulary {
                    assert!(
                        !values.is_empty(),
                        "{} claims a captured closed set, so it must hold one",
                        native.name
                    );
                }
            }
        }
    }

    #[test]
    fn every_bound_axis_names_a_native_field_beside_it() {
        for inventory in InventoryId::ALL {
            let entry = registry(inventory);
            for binding in entry.equivalence_axes {
                assert!(
                    entry.native(binding.native).is_some(),
                    "{inventory:?} binds {:?} to {}, which must be a native field of its own \
                     registry so requiredness has exactly one home",
                    binding.axis,
                    binding.native
                );
            }
        }
    }

    #[test]
    fn no_axis_is_both_bound_and_measured_absent() {
        for inventory in InventoryId::ALL {
            let entry = registry(inventory);
            for absent in entry.absent_axes {
                assert!(
                    entry.axis(absent.0).is_none(),
                    "{inventory:?} cannot both bind {:?} and record it absent",
                    absent.0
                );
            }
        }
    }

    #[test]
    fn a_bound_axis_delegates_exactly_as_the_field_it_lands_in_does() {
        for inventory in InventoryId::ALL {
            let entry = registry(inventory);
            for binding in entry.equivalence_axes {
                let Some(native) = entry.native(binding.native) else {
                    panic!("{} is asserted to exist by its own test", binding.native);
                };
                assert_eq!(
                    binding.delegation, native.delegation,
                    "{inventory:?}'s {:?} axis and its {} field must agree on delegation, or a \
                     legal refusal depends on which one a caller happened to ask",
                    binding.axis, binding.native
                );
            }
        }
    }

    /// The count and the names together, because the count alone would let a
    /// legal field trade places with an ordinary one. A form that pre-ticks
    /// best fit reads `delegation` to decide what the tick may cover, so this
    /// is what stops a new authoring surface rendering a rights grant as
    /// something a computation may supply.
    #[test]
    fn a_never_delegable_field_states_which_bar_it_clears() {
        let mut refused: Vec<(InventoryId, &str)> = Vec::new();
        for inventory in InventoryId::ALL {
            for native in registry(inventory).natives {
                if matches!(native.delegation, Delegation::Never(_)) {
                    refused.push((inventory, native.name));
                }
            }
        }
        assert_eq!(
            refused,
            vec![
                (InventoryId::Tes, "licence"),
                (InventoryId::Tpt, "ItemsProperty.copyright_declaration"),
            ],
            "the Tes licence, and TPT's copyright declaration"
        );
    }

    /// A required field is the one a create is refused without, so it is the
    /// one a form must be able to head with words and put somewhere. Etsy is
    /// exempted by a derived condition rather than by name: its entries come
    /// from a published API reference and no form of its was ever captured, so
    /// it places no field at all, and the moment one of its fields is placed
    /// its required ones are held to the same bar.
    #[test]
    fn every_required_native_of_a_captured_form_carries_a_label_and_a_placement() {
        for inventory in InventoryId::ALL {
            let entry = registry(inventory);
            if entry.groups().is_empty() {
                continue;
            }
            for native in entry.natives.iter().filter(|native| native.required) {
                assert!(
                    native.label.is_some() && native.placement.is_some(),
                    "{inventory:?} refuses a create without {}, so a form has to be able to \
                     head it with the platform's own words and know where to put it",
                    native.name
                );
            }
        }
    }

    /// `groups` is what a form renders its headings from, so a placed field
    /// whose group it omits would be a control with nowhere to go.
    #[test]
    fn every_placed_field_falls_in_a_group_its_inventory_lists() {
        for inventory in InventoryId::ALL {
            let entry = registry(inventory);
            let listed = entry.groups();
            for native in entry.natives {
                let Some(placement) = native.placement else {
                    continue;
                };
                assert!(
                    listed.contains(&placement.group),
                    "{inventory:?} places {} in {:?}, which its own group list omits",
                    native.name,
                    placement.group
                );
            }
        }
    }

    /// One inventory never draws groups from two platforms' forms, so
    /// `FieldGroup::ALL`'s single order is a render order for every one of
    /// them rather than an interleaving of two.
    #[test]
    fn an_inventory_draws_its_groups_from_one_platforms_form() {
        for inventory in InventoryId::ALL {
            let groups = registry(inventory).groups();
            let tes = groups
                .iter()
                .filter(|group| matches!(group, FieldGroup::Tes(_)))
                .count();
            assert!(
                tes == 0 || tes == groups.len(),
                "{inventory:?} mixes two platforms' form sections, so no single order \
                 renders it"
            );
        }
    }

    /// The token is the client's key into the heading list, so two groups
    /// sharing one would collapse two sections into one on the form. Tes and
    /// TPT both name a Categories section, which is exactly why the token is
    /// namespaced and this test exists.
    #[test]
    fn every_group_token_is_its_own() {
        let mut tokens: Vec<&str> = FieldGroup::ALL.iter().map(|group| group.token()).collect();
        let count = tokens.len();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(
            tokens.len(),
            count,
            "two sections sharing a token would render as one heading"
        );
        for group in FieldGroup::ALL {
            assert!(
                !group.heading().is_empty(),
                "{group:?} is rendered as a heading, so it must have words"
            );
        }
    }

    #[test]
    fn text_within_its_cap_survives_verbatim() {
        let text = "A worksheet on café algebra";
        for unit in [
            LengthUnit::Bytes,
            LengthUnit::Utf16CodeUnits,
            LengthUnit::Codepoints,
            LengthUnit::GraphemeClusters,
        ] {
            assert_eq!(
                truncate(text, cap(1_000, unit)),
                text,
                "a cap no input reaches copies verbatim at {unit:?}"
            );
        }
    }

    #[test]
    fn a_byte_cap_falling_mid_codepoint_floors_to_the_boundary() {
        // "café" is five bytes because é is two, so a four-byte budget lands
        // one byte inside é.
        assert_eq!(
            truncate("café", cap(4, LengthUnit::Bytes)),
            "caf",
            "the cap falls inside é, so it floors below it rather than splitting it"
        );
        assert_eq!(
            truncate("café", cap(5, LengthUnit::Bytes)),
            "café",
            "five bytes affords the whole two-byte scalar"
        );
    }

    #[test]
    fn a_utf16_cap_falling_mid_surrogate_pair_floors_to_the_boundary() {
        // U+1F600 occupies two UTF-16 code units, so a two-unit budget over
        // "a<emoji>" is spent by the "a" plus one half of the pair.
        let text = "a\u{1F600}b";
        assert_eq!(
            truncate(text, cap(2, LengthUnit::Utf16CodeUnits)),
            "a",
            "the cap falls between the surrogates, so the astral scalar is dropped whole"
        );
        assert_eq!(
            truncate(text, cap(3, LengthUnit::Utf16CodeUnits)),
            "a\u{1F600}",
            "three units affords the pair, and the following b does not fit"
        );
    }

    #[test]
    fn a_codepoint_cap_counts_scalar_values_and_not_bytes() {
        let text = "café\u{1F600}";
        assert_eq!(
            truncate(text, cap(5, LengthUnit::Codepoints)),
            text,
            "five scalars fit under five codepoints whatever they cost in bytes"
        );
        assert_eq!(
            truncate(text, cap(4, LengthUnit::Codepoints)),
            "café",
            "the fifth scalar is dropped whole"
        );
    }

    #[test]
    fn a_grapheme_cap_under_approximates_by_truncating_codepoints() {
        // "é" as e + U+0301 is one grapheme cluster over two codepoints, so a
        // two-cluster budget yields one cluster: short of the cap, never past.
        let text = "e\u{301}a";
        assert_eq!(
            truncate(text, cap(2, LengthUnit::GraphemeClusters)),
            "e\u{301}",
            "codepoint counting can only truncate early, never past the cap"
        );
    }

    #[test]
    fn a_zero_cap_yields_nothing_rather_than_a_partial_scalar() {
        assert_eq!(
            truncate("café", cap(0, LengthUnit::Bytes)),
            "",
            "no budget affords no character"
        );
    }

    #[test]
    fn the_unrecorded_spec_declares_neither_a_cap_nor_a_requirement() {
        assert_eq!(
            (FieldSpec::UNRECORDED.cap, FieldSpec::UNRECORDED.required),
            (None, false),
            "an absent finding is not a finding of absence, and reads as neither"
        );
    }

    #[test]
    fn the_canonical_six_carry_their_own_arity() {
        use tam_types::Arity;
        assert_eq!(
            [
                FieldKey::Title.cardinality(),
                FieldKey::Description.cardinality(),
                FieldKey::Price.cardinality(),
                FieldKey::Taxonomy.cardinality(),
                FieldKey::Grades.cardinality(),
                FieldKey::Files.cardinality(),
            ],
            [
                Arity::One,
                Arity::One,
                Arity::One,
                Arity::Many,
                Arity::Many,
                Arity::Many
            ],
            "arity is a fact of the canonical model rather than of any inventory"
        );
    }
}
