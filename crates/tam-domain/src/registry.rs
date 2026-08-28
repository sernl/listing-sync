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

/// Populated from the six 2026-08-28 HAR captures of the founder's own TPT
/// seller account, analysed for the M7 connector. Everything here is a read
/// the captures contain or a constant the create form hands its own client;
/// no product write is captured, so nothing about the write's mandatory-field
/// set is declared.
const TPT: InventoryRegistry = InventoryRegistry {
    inventory: InventoryId::Tpt,
    canonical: CanonicalFields {
        // The sampled 80-character title cap is real but unproven: 80 is the
        // longest of 352 observed titles, not a refusal anyone has measured,
        // and the counting unit is unverified either way. No cap is declared.
        title: FieldSpec::UNRECORDED,
        description: FieldSpec {
            // `description_max_length: 45000` from the `var cfg` bootstrap the
            // create page at /My-Products/New/Digital-Next hands its own
            // client. Recorded in UTF-16 code units because a browser-side
            // validator counts `String.length`; the server's own enforcement
            // is untested, so this is the client's cap, not a measured one.
            cap: Some(LengthCap {
                limit: 45_000,
                unit: LengthUnit::Utf16CodeUnits,
            }),
            required: false,
        },
        // The same bootstrap carries `min_price: 0.95` with no currency: the
        // store observed is New Zealand-based, every money field renders with
        // a bare `$`, and nothing states which dollar. A minimum has no home
        // in `LengthCap` and inventing an axis for one field would be worse
        // than recording it here until the write path needs to enforce it.
        price: FieldSpec::UNRECORDED,
        taxonomy: FieldSpec::UNRECORDED,
        // TPT has no grades field. Grades arrive inside the flat
        // `taxonomyTags` slug array alongside subjects, resource types and
        // file formats; the create form offers a separate 14-value grade
        // selector, whose relationship to those tags is uncaptured.
        grades: FieldSpec::UNRECORDED,
        // The create form caps each upload slot separately — 4 GiB for the
        // product, 30 MiB for the preview, 4 MiB for each of four thumbnails,
        // 1 GiB for the video preview, and that last one moves with the
        // per-account `double_video_file_size_limit` variant. One cap for the
        // whole set would name none of them.
        files: FieldSpec::UNRECORDED,
    },
    natives: TPT_NATIVES,
};

const TPT_NATIVES: &[NativeField] = &[
    // One flat namespace: the nine slugs observed across 154 products span
    // grades (4th-grade), audience (homeschool), subject (math), resource type
    // (unit-plans) and file format (pdf) without distinguishing them. Nine
    // slugs from one store is a sample, not a vocabulary, and TPT's own
    // platform tag set is uncaptured. The create form's posted-field
    // whitelist names a `TaxonomyTags` field, so the write very likely sets
    // these, but no captured write does, so the read is all that is declared.
    NativeField {
        name: "taxonomyTags",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
    },
    // Seller-owned shelves, so there is no platform-wide vocabulary to hold:
    // the members are whatever this seller created. The product read names
    // them `categories` with numeric ids and the gateway names the same
    // entities `customCategories` with the ids stringified.
    NativeField {
        name: "categories",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
    },
    // `TaxCodesQuery` on /graph/graphql returned the complete five-row
    // `taxData` set: id 1 DA051011 digital audio, 2 DB031013 digital books,
    // 3 DI010200 digital images, 4 DV010200 videos, 5 DO010000 other digital
    // goods. The create form posts `data[ItemTaxCode][tax_code_id]`, and no
    // capture shows whether that field carries the id or the code, so the
    // write must settle which of the two it sends before it sends one.
    NativeField {
        name: "taxCode",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "DA051011", "DB031013", "DI010200", "DV010200", "DO010000",
        ]),
    },
    // The `ResourceType` enum, recovered whole from the statistics page
    // bundle. Only DIGITAL_PRODUCT occurs across this seller's 154 products.
    NativeField {
        name: "itemType",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "DIGITAL_PRODUCT",
            "BUNDLE",
            "ONLINE_RESOURCE",
            "VIDEO",
        ]),
    },
    // Every one of the 154 products read ACTIVE, on `status`, `statusAdmin`
    // and `statusCopyright` alike, so one store's steady state is all that is
    // on file. The seller filter offers ACTIVE, INACTIVE and FEATURED, but a
    // filter vocabulary is a different upstream enum from a product's status.
    NativeField {
        name: "status",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
    },
    // PDF and ZIP across 154 products, always equal to `filePreview.format`.
    // Two values from one store is a sample of what this seller uploads.
    NativeField {
        name: "kind",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
    },
    // `EducationStandardsQuery` returned 166 jurisdiction roots — Common Core,
    // NGSS, TEKS and the rest — but the form field takes a standard id from
    // within a jurisdiction, and neither those ids nor the jurisdiction-to-id
    // mapping is captured. The set is closed upstream and its members are not
    // held here, which is exactly what an unlisted value being refused means.
    NativeField {
        name: "ItemsCommonCoreStandard.common_core_standard_id",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
];

#[cfg(test)]
mod tests {
    use super::{
        registry, truncate, FieldDirection, FieldSpec, LengthCap, NativeVocabulary, TES_GB,
        TES_NATIVES, TPT,
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
    fn the_tpt_title_cap_stays_undeclared_because_eighty_is_a_sample() {
        assert_eq!(
            TPT.canonical.title.cap, None,
            "the longest observed title is not a refusal anyone measured"
        );
    }

    #[test]
    fn the_tpt_description_cap_is_the_one_the_create_form_hands_its_client() {
        assert_eq!(
            TPT.canonical.description.cap,
            Some(cap(45_000, LengthUnit::Utf16CodeUnits)),
            "45000 comes from the create page's own cfg bootstrap, counted as a JS validator does"
        );
    }

    #[test]
    fn no_tpt_field_is_declared_required_because_no_write_is_captured() {
        for key in [
            FieldKey::Title,
            FieldKey::Description,
            FieldKey::Price,
            FieldKey::Taxonomy,
            FieldKey::Grades,
            FieldKey::Files,
        ] {
            assert!(
                !TPT.canonical.get(key).required,
                "no capture contains a TPT product write, so {key:?} records no refusal"
            );
        }
        for native in TPT.natives {
            assert!(
                !native.required,
                "{} likewise records no refusal, only a wire name",
                native.name
            );
        }
    }

    #[test]
    fn the_tpt_tax_code_vocabulary_is_the_five_taxdata_rows() {
        let tax = TPT
            .natives
            .iter()
            .find(|native| native.name == "taxCode")
            .map(|native| native.vocabulary);
        let Some(NativeVocabulary::Closed(values)) = tax else {
            panic!("TaxCodesQuery returned a complete set, got {tax:?}");
        };
        assert_eq!(values.len(), 5, "taxData carried exactly five rows");
        assert_eq!(
            values.first(),
            Some(&"DA051011"),
            "recorded in the order the query returned them"
        );
    }

    #[test]
    fn the_tpt_taxonomy_is_one_flat_read_only_namespace() {
        let tags = TPT
            .natives
            .iter()
            .find(|native| native.name == "taxonomyTags");
        assert_eq!(
            tags.map(|native| (native.direction, native.vocabulary)),
            Some((FieldDirection::ReadOnly, NativeVocabulary::Unmeasured)),
            "tags come back on read; whether a write sets them is uncaptured, and nine slugs \
             from one store is a sample rather than a vocabulary"
        );
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
