//! The per-inventory field registry: the per-platform constraints on the
//! canonical six, and the native fields that exist on one platform's wire and
//! nowhere else.
//!
//! `FieldKey` stays closed at six and `FieldPolicies` stays a six-field
//! struct, so a platform-particular field gets a second axis here rather than
//! a seventh key. `CanonicalFields` mirrors the `FieldPolicies` shape, which
//! makes a new `FieldKey` a compile error in this module too.
//!
//! Every entry records a measured or documented fact. An unmeasured cap is
//! absent rather than guessed, and a vocabulary documented as closed upstream
//! whose members we have not captured says exactly that; no value here is
//! invented to fill a hole.

use tam_types::{FieldKey, InventoryId, LengthUnit};

/// A length limit and the unit the marketplace counts it in. The unit is part
/// of the fact: a cap whose counting unit is unverified is not recordable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthCap {
    pub limit: usize,
    pub unit: LengthUnit,
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

/// A field particular to one platform's wire, named as that wire names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeField {
    pub name: &'static str,
    pub direction: FieldDirection,
    /// Read as `FieldSpec::required`: a recorded refusal, not an absence.
    pub required: bool,
    pub vocabulary: NativeVocabulary,
}

/// One inventory's two axes: the constraints on the canonical six, and the
/// native fields beside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryRegistry {
    pub inventory: InventoryId,
    pub canonical: CanonicalFields,
    pub natives: &'static [NativeField],
}

/// Total over `InventoryId::ALL` by exhaustive match, so a new inventory is a
/// compile error rather than a silent absence.
#[must_use]
pub const fn registry(inventory: InventoryId) -> &'static InventoryRegistry {
    match inventory {
        InventoryId::TesGb => &TES_GB,
        InventoryId::TesUs => &TES_US,
        InventoryId::TesNz => &TES_NZ,
        InventoryId::Etsy => &ETSY,
        InventoryId::Tpt => &TPT,
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

/// The three Tes inventories are one account against one JSON API reached
/// through the curriculum field (probe 04), so they share both axes and
/// differ only in the key.
const fn tes(inventory: InventoryId) -> InventoryRegistry {
    InventoryRegistry {
        inventory,
        // No Tes cap has been measured. The design applies caps at projection
        // and never at authoring, so an absent cap is a projection that copies
        // verbatim, which is the honest behaviour for an unmeasured platform.
        canonical: CanonicalFields::UNRECORDED,
        natives: TES_NATIVES,
    }
}

const TES_NATIVES: &[NativeField] = &[
    // Restated rather than imported: the pure core must not depend on an
    // adapter crate. Source of the four values and the publish-time refusal:
    // `TesLicence` and the publish builder in
    // crates/tam-marketplace-tes/src/endpoints.rs. Written by the draft
    // metadata request and read back by the first-party import.
    NativeField {
        name: "licence",
        direction: FieldDirection::Both,
        required: true,
        vocabulary: NativeVocabulary::Closed(&["CC-BY", "CC-BY-SA", "CC-BY-ND", "TES-PAID"]),
    },
    // The market-targeting field the GB-to-NZ wedge depends on. Read on import
    // and discarded, and absent from the written-field manifest, so it is
    // declared read-only until a founder-supervised capture of the uploader
    // setting one. The thirteen values are the dropdown transcribed in
    // docs/notes/probes/04-dual-inventory-access.md.
    NativeField {
        name: "curriculum",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "None",
            "No curriculum",
            "American",
            "Australian",
            "Canadian",
            "English",
            "International",
            "Irish",
            "New Zealand",
            "Northern Irish",
            "Scottish",
            "Welsh",
            "Zambian",
        ]),
    },
    // Category and age-range native ids, drawn from the vocabularies the
    // taxonomy hub owns rather than from any set this registry could hold.
    NativeField {
        name: "mainType",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
    },
    NativeField {
        name: "mainAge",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
    },
    // The adapter writes the empty list on every draft, so the accepted values
    // have never been exercised and none are on file; the import reads the
    // field back off the resource state, so it crosses in both directions.
    NativeField {
        name: "yearGroups",
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
    },
    // The adapter writes the constant "md"; whether the API accepts anything
    // else is uncaptured, and one written value is not evidence of a set.
    NativeField {
        name: "descriptionRawType",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
    },
];

const TES_GB: InventoryRegistry = tes(InventoryId::TesGb);
const TES_US: InventoryRegistry = tes(InventoryId::TesUs);
const TES_NZ: InventoryRegistry = tes(InventoryId::TesNz);

/// Etsy's published API reference for `createDraftListing` is the source for
/// every entry: the required create fields, the title cap, and the three
/// required natives.
const ETSY: InventoryRegistry = InventoryRegistry {
    inventory: InventoryId::Etsy,
    canonical: CanonicalFields {
        title: FieldSpec {
            // Etsy's own wording is "140 characters" and names no counting
            // unit; recorded as codepoints, which is the reading its examples
            // support, and re-measurable when the connector is built.
            cap: Some(LengthCap {
                limit: 140,
                unit: LengthUnit::Codepoints,
            }),
            required: true,
        },
        description: FieldSpec::REQUIRED,
        price: FieldSpec::REQUIRED,
        taxonomy: FieldSpec::REQUIRED,
        grades: FieldSpec::UNRECORDED,
        files: FieldSpec::UNRECORDED,
    },
    natives: ETSY_NATIVES,
};

const ETSY_NATIVES: &[NativeField] = &[
    NativeField {
        name: "quantity",
        direction: FieldDirection::Written,
        required: true,
        vocabulary: NativeVocabulary::Numeric,
    },
    NativeField {
        name: "who_made",
        direction: FieldDirection::Written,
        required: true,
        vocabulary: NativeVocabulary::Closed(&["i_did", "someone_else", "collective"]),
    },
    // Closed upstream and required on create, with the members not captured
    // here. Etsy refuses an unlisted value, so a guess is a failed create.
    NativeField {
        name: "when_made",
        direction: FieldDirection::Written,
        required: true,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
    // Etsy documents at most thirteen tags of at most twenty characters each.
    // A collection cardinality has no home in `LengthCap`, and inventing one
    // for a single field would be worse than recording the limit here until
    // the connector needs to enforce it.
    NativeField {
        name: "tags",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Free,
    },
];

/// Sparse until the M7 first contact measures it. The sampled 80-character
/// title cap is real but its counting unit is unverified — the 352-title
/// sample contained no astral-plane characters — so no cap is declared, and
/// no native field has been observed on the wire yet.
const TPT: InventoryRegistry = InventoryRegistry {
    inventory: InventoryId::Tpt,
    canonical: CanonicalFields::UNRECORDED,
    natives: &[],
};

#[cfg(test)]
mod tests {
    use super::{
        registry, truncate, FieldSpec, LengthCap, NativeVocabulary, TES_GB, TES_NATIVES, TPT,
    };
    use tam_types::{FieldKey, InventoryId, LengthUnit};

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

    #[test]
    fn no_tes_inventory_declares_a_cap_because_none_is_measured() {
        for inventory in [InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz] {
            let canonical = registry(inventory).canonical;
            for key in [
                FieldKey::Title,
                FieldKey::Description,
                FieldKey::Price,
                FieldKey::Taxonomy,
                FieldKey::Grades,
                FieldKey::Files,
            ] {
                assert_eq!(
                    canonical.get(key).cap,
                    None,
                    "no Tes cap is measured, so none is declared for {key:?}"
                );
            }
        }
    }

    #[test]
    fn the_etsy_title_cap_is_the_documented_one_hundred_and_forty_codepoints() {
        assert_eq!(
            registry(InventoryId::Etsy).canonical.title.cap,
            Some(cap(140, LengthUnit::Codepoints)),
            "Etsy documents a 140-character title limit"
        );
    }

    #[test]
    fn tpt_declares_nothing_until_first_contact_measures_it() {
        assert_eq!(
            TPT.canonical,
            super::CanonicalFields::UNRECORDED,
            "the sampled 80-character cap has an unverified unit, so nothing is declared"
        );
        assert!(TPT.natives.is_empty(), "no TPT native is on the wire yet");
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
    fn the_tes_licence_vocabulary_is_the_four_values_the_api_validates() {
        let licence = TES_NATIVES
            .iter()
            .find(|native| native.name == "licence")
            .map(|native| native.vocabulary);
        assert_eq!(
            licence,
            Some(NativeVocabulary::Closed(&[
                "CC-BY", "CC-BY-SA", "CC-BY-ND", "TES-PAID"
            ])),
            "restated from the adapter's TesLicence without depending on it"
        );
        assert_eq!(
            TES_GB.natives.len(),
            6,
            "the licence, the curriculum read, and the four remaining written natives"
        );
    }

    #[test]
    fn the_curriculum_vocabulary_holds_the_thirteen_captured_values() {
        let curriculum = TES_NATIVES
            .iter()
            .find(|native| native.name == "curriculum")
            .map(|native| native.vocabulary);
        let Some(NativeVocabulary::Closed(values)) = curriculum else {
            panic!("the curriculum vocabulary is captured closed, got {curriculum:?}");
        };
        assert_eq!(
            values.len(),
            13,
            "probe 04 transcribed thirteen dropdown values"
        );
        assert_eq!(
            (values.first(), values.last()),
            (Some(&"None"), Some(&"Zambian")),
            "transcribed in the order the dropdown offers them"
        );
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
}
