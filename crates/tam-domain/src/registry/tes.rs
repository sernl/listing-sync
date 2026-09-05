//! The three Tes inventories: one account against one JSON API reached
//! through the curriculum field (probe 04), so they share both axes and
//! differ only in the key and in which age vocabulary the country selects.
//!
//! The `placement` on each field below is the uploader's five-step wizard,
//! and two sources fix it. `docs/notes/probes/02-upload-request-shape.md`
//! names the steps — Description, Add Files, Categories, Licence, Publish —
//! and Tes's own author academy, quoted at
//! `docs/research/rethink/cross-marketplace-mapping-tpt-base.md:39`, says what
//! each holds: "title and description, file upload and resource type, tag and
//! categorise, price or licence, preview and agree to the Author Code". The
//! ordinal within a step is this file's own ordering and not a measurement;
//! `ageRanges` and `yearGroups` share one because the country picks one of
//! them and never both.

use super::{
    AxisBinding, CanonicalFields, Cardinality, Delegation, FieldDirection, FieldGroup,
    FormPlacement, InventoryRegistry, NativeField, NativeVocabulary, NonDelegable, TesStep,
};
use crate::TermKind;
use tam_types::InventoryId;

/// The axes are a parameter rather than a shared const because the three
/// registries genuinely differ on one: GB binds `Phase` to `ageRanges` and
/// the other two bind it to `yearGroups`, which is the country branch the
/// uploader takes.
const fn tes(
    inventory: InventoryId,
    equivalence_axes: &'static [AxisBinding],
) -> InventoryRegistry {
    InventoryRegistry {
        inventory,
        // No Tes cap has been measured. The design applies caps at projection
        // and never at authoring, so an absent cap is a projection that copies
        // verbatim, which is the honest behaviour for an unmeasured platform.
        canonical: CanonicalFields::UNRECORDED,
        natives: TES_NATIVES,
        equivalence_axes,
        // Tes binds all five axes, so nothing is measured absent here. TPT is
        // where the empty-handed case lives.
        absent_axes: &[],
    }
}

pub(super) const TES_NATIVES: &[NativeField] = &[
    // Restated rather than imported: the pure core must not depend on an
    // adapter crate. The seven values are `RefdataStore.licences` as the
    // 2026-08-29 live poll read it, catalogued in
    // docs/design/data/tes-vocabulary.json; `GET /api/refdata/v2/licences`
    // returns the first five and omits the legacy pair. Price gates the
    // write: a free resource picks among the three Creative Commons values
    // and a paid one is `TES-PAID`. `TES-PAID-SCHOOL` is the school tier, and
    // the editor rewrites `TES-V1` and `TES-V2` to `CC-BY-SA` on load, so
    // both read back on older resources and neither is offered on write. The
    // adapter's `TesLicence` in crates/tam-marketplace-tes/src/endpoints.rs
    // writes four of the seven. Written by the draft metadata request and
    // read back by the first-party import.
    NativeField {
        name: "licence",
        label: Some("Licence"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Licence),
            ordinal: 0,
        }),
        direction: FieldDirection::Both,
        required: true,
        vocabulary: NativeVocabulary::Closed(&[
            "CC-BY",
            "CC-BY-ND",
            "CC-BY-SA",
            "TES-PAID",
            "TES-PAID-SCHOOL",
            "TES-V1",
            "TES-V2",
        ]),
        // Choosing a licence is issuing a rights grant, so no opt-in reaches
        // it: the seller's own instruction is the only admissible source.
        delegation: Delegation::Never(NonDelegable::LegalContent),
    },
    // The market-targeting field the GB-to-NZ wedge depends on. Read on import
    // and discarded, and absent from the written-field manifest, so it is
    // declared read-only until a founder-supervised capture of the uploader
    // setting one. The twelve values are
    // `TaxonomyStore["resource-orientations"]` as the 2026-08-29 live poll
    // read it off the uploader page, superseding the thirteen-line
    // transcription in docs/notes/probes/04-dual-inventory-access.md, which
    // wrote the one synthetic option down twice as `None` and
    // `No curriculum` and omitted `[rest of world]`. The uploader prepends
    // that synthetic `none`, labelled "No curriculum", and filters
    // `[rest of world]` out of the picker, so eleven of the twelve are
    // selectable. This is the first of three cascading selects: the
    // orientation narrows a 48-value `framework`, itself filtered by the
    // resource's ages, which narrows a 23-value `authority`. The adapter
    // writes none of the three, so the other two levels are catalogued in
    // docs/design/data/tes-vocabulary.json rather than held here.
    NativeField {
        name: "curriculum",
        label: Some("Curriculum"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Categories),
            ordinal: 0,
        }),
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
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
            "[rest of world]",
        ]),
        delegation: Delegation::ByOptIn,
    },
    // `RefdataStore.resourceTypes` holds 33 rows and the uploader's type
    // select filters them to `99000 < id < 99010`, so these nine are the
    // writable set; the other 24 are legacy or attachment-level types that
    // read back on older resources and are never offered. The ids are the
    // wire tokens and the labels are catalogued in
    // docs/design/data/tes-vocabulary.json.
    NativeField {
        name: "mainType",
        label: Some("Resource type"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Files),
            ordinal: 0,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "99001", "99002", "99003", "99004", "99005", "99006", "99007", "99008", "99009",
        ]),
        delegation: Delegation::ByOptIn,
    },
    // A pointer into whichever age vocabulary the country selects rather than
    // a vocabulary of its own: the uploader takes `ageRanges` for GB and
    // `yearGroups` everywhere else, and the 2026-08-29 poll read `mainAge` as
    // an `ageRanges` id on a GB resource without establishing what it carries
    // on the other branch. No single closed set is on file.
    NativeField {
        name: "mainAge",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Categories),
            ordinal: 2,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
        delegation: Delegation::ByOptIn,
    },
    // The non-GB age field, and an alternative to `ageRanges` rather than a
    // companion: the uploader picks one or the other by country, taking
    // `ageRanges` for GB and `yearGroups` everywhere else. The thirty ids are
    // `GET /api/refdata/v2/year-groups` as the 2026-08-29 poll read it — 1
    // through 15 the GB school years from Nursery and Reception up to 13, and
    // 16 through 30 the US grades from Pre-K and Kindergarten up to 12th,
    // with 30 the not-applicable sentinel that `ageRanges` spells 7. The
    // adapter still writes the empty list on every draft, so none of the
    // thirty has been exercised on the write side; the import reads the field
    // back off the resource state, so it crosses in both directions.
    NativeField {
        name: "yearGroups",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Categories),
            ordinal: 1,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
            "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30",
        ]),
        delegation: Delegation::ByOptIn,
    },
    // The adapter writes the constant "md"; whether the API accepts anything
    // else is uncaptured, and one written value is not evidence of a set.
    NativeField {
        name: "descriptionRawType",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Description),
            ordinal: 0,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
        delegation: Delegation::ByOptIn,
    },
    // The GB age vocabulary, and an alternative to `yearGroups` rather than a
    // companion: the uploader takes `ageRanges` for GB and `yearGroups`
    // everywhere else. The seven ids are `ageRanges.options` as the
    // 2026-08-29 poll read them, catalogued in
    // docs/design/data/tes-vocabulary.json — 1 through 5 the closed bands
    // from 3-5 to 14-16, 6 the half-open 16+, and 7 the not-applicable
    // sentinel that `yearGroups` spells 30. Written by the draft metadata
    // request and read back off the resource state.
    NativeField {
        name: "ageRanges",
        label: Some("Main age range"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Categories),
            ordinal: 1,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["1", "2", "3", "4", "5", "6", "7"]),
        delegation: Delegation::ByOptIn,
    },
    // The subject and topic channel, posted as `categories: [{id}]` of
    // taxonomy node ids and read back off the resource state. The tree is
    // fetched per node from `GET /taxonomy/v4/{country}/{id}` rather than
    // enumerated, so there is no set to capture here.
    //
    // `docs/design/data/tes-vocabulary.json` records the selection limit as
    // "~10 subject/topic pairs per resource". A tilde is a guess, and under
    // this module's own admission rule the cardinality stays uncapped until a
    // founder-supervised capture measures the refusal.
    NativeField {
        name: "categories",
        label: Some("Subjects and topics"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Categories),
            ordinal: 3,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
        delegation: Delegation::ByOptIn,
    },
    // Exactly one of the declared `categories`, posted alongside them by the
    // publish request and by nothing else. `publish_request` writes the first
    // category by sort order today, which is a decision rather than a read,
    // and Phase 4 turns it into an election with that value pre-selected.
    NativeField {
        name: "primaryCategory",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tes(TesStep::Categories),
            ordinal: 4,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
        delegation: Delegation::ByOptIn,
    },
];

/// Subject and Topic share one wire field: `categories` carries both, and the
/// depth of the node is what tells them apart, so the axes differ in what
/// they mean and not in where they land.
const fn tes_axes(phase_native: &'static str) -> [AxisBinding; 5] {
    [
        AxisBinding {
            axis: TermKind::Subject,
            native: "categories",
            cardinality: Cardinality::Many { cap: None },
            delegation: Delegation::ByOptIn,
        },
        AxisBinding {
            axis: TermKind::Topic,
            native: "categories",
            cardinality: Cardinality::Many { cap: None },
            delegation: Delegation::ByOptIn,
        },
        AxisBinding {
            axis: TermKind::ResourceType,
            native: "mainType",
            cardinality: Cardinality::One,
            delegation: Delegation::ByOptIn,
        },
        AxisBinding {
            axis: TermKind::Phase,
            native: phase_native,
            cardinality: Cardinality::Many { cap: None },
            delegation: Delegation::ByOptIn,
        },
        AxisBinding {
            axis: TermKind::Licence,
            native: "licence",
            cardinality: Cardinality::One,
            delegation: Delegation::Never(NonDelegable::LegalContent),
        },
    ]
}

/// GB takes `ageRanges`; every other country takes `yearGroups`.
const TES_GB_AXES: [AxisBinding; 5] = tes_axes("ageRanges");
const TES_US_AXES: [AxisBinding; 5] = tes_axes("yearGroups");

pub(super) const TES_GB: InventoryRegistry = tes(InventoryId::TesGb, &TES_GB_AXES);
pub(super) const TES_US: InventoryRegistry = tes(InventoryId::TesUs, &TES_US_AXES);
// NZ is on the non-GB branch of the country fork, so it shares US's axes.
pub(super) const TES_NZ: InventoryRegistry = tes(InventoryId::TesNz, &TES_US_AXES);

#[cfg(test)]
mod tests {
    use super::{TES_GB, TES_NATIVES, TES_NZ, TES_US};
    use crate::registry::{
        registry, AxisBinding, Cardinality, Delegation, FieldDirection, NativeVocabulary,
        NonDelegable,
    };
    use crate::TermKind;
    use tam_types::{FieldKey, InventoryId};
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

    fn tes_vocabulary(name: &str) -> Option<NativeVocabulary> {
        TES_NATIVES
            .iter()
            .find(|native| native.name == name)
            .map(|native| native.vocabulary)
    }

    #[test]
    fn the_tes_licence_vocabulary_is_the_seven_rows_the_refdata_store_holds() {
        assert_eq!(
            tes_vocabulary("licence"),
            Some(NativeVocabulary::Closed(&[
                "CC-BY",
                "CC-BY-ND",
                "CC-BY-SA",
                "TES-PAID",
                "TES-PAID-SCHOOL",
                "TES-V1",
                "TES-V2",
            ])),
            "restated from the polled RefdataStore.licences without depending on the adapter, \
             whose TesLicence writes four of the seven"
        );
        assert_eq!(
            TES_GB.natives.len(),
            9,
            "the licence, the curriculum read, and the seven remaining age, category and \
             format fields the uploader posts"
        );
    }

    #[test]
    fn the_tes_licence_vocabulary_holds_the_school_tier_and_both_legacy_values() {
        let Some(NativeVocabulary::Closed(values)) = tes_vocabulary("licence") else {
            panic!("the licence vocabulary is captured closed");
        };
        for extra in ["TES-PAID-SCHOOL", "TES-V1", "TES-V2"] {
            assert!(
                values.contains(&extra),
                "{extra} is a real row the four-value record left out"
            );
        }
    }

    #[test]
    fn the_curriculum_vocabulary_is_the_orientation_select_the_uploader_filters() {
        let curriculum = tes_vocabulary("curriculum");
        let Some(NativeVocabulary::Closed(values)) = curriculum else {
            panic!("the curriculum vocabulary is captured closed, got {curriculum:?}");
        };
        assert_eq!(
            values.len(),
            12,
            "resource-orientations holds twelve rows, of which the uploader offers eleven"
        );
        assert_eq!(
            (values.first(), values.last()),
            (Some(&"American"), Some(&"[rest of world]")),
            "the eleven selectable nationals, then the row the picker filters out"
        );
        assert!(
            !values.contains(&"No curriculum"),
            "the uploader prepends that label over a synthetic none, so it is not a row"
        );
    }

    #[test]
    fn the_tes_main_type_vocabulary_is_the_nine_the_uploader_can_write() {
        assert_eq!(
            tes_vocabulary("mainType"),
            Some(NativeVocabulary::Closed(&[
                "99001", "99002", "99003", "99004", "99005", "99006", "99007", "99008", "99009",
            ])),
            "the type select keeps 99000 < id < 99010, leaving the other 24 resourceTypes \
             rows readable but never offered"
        );
    }

    #[test]
    fn the_tes_year_groups_vocabulary_holds_both_countries_ranges() {
        let year_groups = tes_vocabulary("yearGroups");
        let Some(NativeVocabulary::Closed(values)) = year_groups else {
            panic!("the yearGroups vocabulary is captured closed, got {year_groups:?}");
        };
        assert_eq!(
            values.len(),
            30,
            "fifteen GB school years and fifteen US grade equivalents"
        );
        assert_eq!(
            (values.first(), values.last()),
            (Some(&"1"), Some(&"30")),
            "1 is Nursery and 30 is the not-applicable sentinel of the US half"
        );
    }

    #[test]
    fn the_tes_main_age_holds_no_set_because_the_country_picks_its_vocabulary() {
        assert_eq!(
            tes_vocabulary("mainAge"),
            Some(NativeVocabulary::Numeric),
            "mainAge points into ageRanges for GB and yearGroups elsewhere, and the poll \
             read it only on the GB branch"
        );
    }

    #[test]
    fn every_tes_inventory_binds_all_five_axes_and_records_none_absent() {
        for inventory in [InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz] {
            let entry = registry(inventory);
            let bound: Vec<TermKind> = entry
                .equivalence_axes
                .iter()
                .map(|binding| binding.axis)
                .collect();
            assert_eq!(
                bound,
                vec![
                    TermKind::Subject,
                    TermKind::Topic,
                    TermKind::ResourceType,
                    TermKind::Phase,
                    TermKind::Licence,
                ],
                "{inventory:?} carries every axis on its own wire"
            );
            assert!(
                entry.absent_axes.is_empty(),
                "{inventory:?} lacks no axis, so it records no absence"
            );
        }
    }

    #[test]
    fn the_tes_licence_axis_refuses_delegation_whatever_the_seller_opts_into() {
        assert_eq!(
            TES_GB.axis(TermKind::Licence),
            Some(AxisBinding {
                axis: TermKind::Licence,
                native: "licence",
                cardinality: Cardinality::One,
                delegation: Delegation::Never(NonDelegable::LegalContent),
            }),
            "issuing a rights grant is the seller's, and the binding is where that is data"
        );
    }

    #[test]
    fn the_country_fork_is_the_one_axis_the_three_registries_disagree_on() {
        let phase = |entry: &super::InventoryRegistry| {
            entry.axis(TermKind::Phase).map(|binding| binding.native)
        };
        assert_eq!(
            (phase(&TES_GB), phase(&TES_US), phase(&TES_NZ)),
            (Some("ageRanges"), Some("yearGroups"), Some("yearGroups")),
            "the uploader takes ageRanges for GB and yearGroups everywhere else"
        );
        for other in [TermKind::Subject, TermKind::Topic, TermKind::ResourceType] {
            assert_eq!(
                TES_GB.axis(other),
                TES_US.axis(other),
                "{other:?} is the same binding on both branches"
            );
        }
    }

    #[test]
    fn the_two_age_vocabularies_are_alternatives_and_never_companions() {
        let ages = tes_vocabulary("ageRanges");
        let Some(NativeVocabulary::Closed(values)) = ages else {
            panic!("ageRanges.options is captured closed, got {ages:?}");
        };
        assert_eq!(
            values,
            ["1", "2", "3", "4", "5", "6", "7"],
            "five closed bands, the half-open 16+, and the not-applicable sentinel"
        );
        assert!(
            TES_GB
                .equivalence_axes
                .iter()
                .filter(|binding| binding.axis == TermKind::Phase)
                .count()
                == 1,
            "one Phase binding per inventory, because the country picks one field"
        );
    }

    #[test]
    fn the_primary_category_is_one_where_the_categories_beside_it_are_many() {
        let Some(primary) = TES_GB.native("primaryCategory") else {
            panic!("primaryCategory is a written field of the publish request");
        };
        assert_eq!(
            primary.direction,
            FieldDirection::Written,
            "publish_request posts it and no read returns it"
        );
        assert_eq!(
            TES_GB.axis(TermKind::Subject).map(|b| b.cardinality),
            Some(Cardinality::Many { cap: None }),
            "a tilde in the vocabulary catalogue is a guess, so the cap stays unmeasured"
        );
    }
}
