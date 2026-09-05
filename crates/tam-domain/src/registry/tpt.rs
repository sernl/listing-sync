//! TPT, whose whole equivalence surface is one flat facet namespace, and
//! which binds no licence axis at all.
//!
//! The `label` and `placement` on each field below come from
//! `docs/research/rethink/tpt-create-form-dom.md`, a full-page DOM snapshot of
//! `GET /My-Products/New/Digital-Next` read on 2026-09-03, which records every
//! section heading and every control label the create form renders. A field
//! that snapshot places nowhere carries no placement: the read-only fields the
//! form has no control for, the item type chosen on the create-type page
//! before the form, and the two opaque server-issued upload handles.
//!
//! The ordinal is the field's position among that section's own controls
//! rather than among its native fields, so the gaps are information: the
//! Categories section renders five pickers of which four feed `taxonomyTags`,
//! which is why `categories` sits at four, and the Files section's first slot
//! belongs to the upload boxes, which are the canonical `files` field and
//! carry no placement of their own.

use super::{
    AxisAbsent, AxisBinding, CanonicalFields, Cardinality, CountCap, Delegation, FieldDirection,
    FieldGroup, FieldSpec, FormPlacement, InventoryRegistry, LengthCap, NativeField,
    NativeVocabulary, NonDelegable,
};
use crate::product::FormGroup;
use crate::TermKind;
use tam_types::{InventoryId, LengthUnit};

/// Populated from the eight 2026-08-28 HAR captures of the founder's own TPT
/// seller account, analysed for the M7 connector: six read captures, one
/// product create and one product edit. Everything here is a read the
/// captures contain, a field one of the two writes posted, or a constant the
/// form hands its own client, extended by the 2026-08-29 live poll of the
/// create and edit vocabularies, which reached the option sets themselves
/// through live GraphQL operations and the `JSON.parse` blobs the
/// `tpt-frontend` chunks carry. The polled catalog is
/// docs/design/data/tpt-vocabulary.json.
///
/// No capture contains a refused write — every recorded request succeeded —
/// so nothing here is declared required. The written fields below record what
/// the wire carried, not what the server would insist on.
pub(super) const TPT: InventoryRegistry = InventoryRegistry {
    inventory: InventoryId::Tpt,
    canonical: CanonicalFields {
        // 80 is the longest of 352 observed titles and is also the value of
        // the title-length constant in chunk `tpt-frontend.1.1564`, so the
        // sample and the client's own constant agree. Neither is a refusal
        // anyone has measured and the counting unit is unverified either way,
        // and declaring a cap here would start truncating titles at
        // projection, so no cap is declared without a founder decision.
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
        // file formats; the create form offers a separate 19-value grade
        // selector, and the poll settled its relationship to those tags. Each
        // of the nineteen legacy grade ids the form posts is a facet's own
        // `legacyId`: seventeen resolve into `category: Grade-Level` and the
        // remaining two, Homeschool (17) and Staff (19), into
        // `category: audience`, which is where TPT itself files them.
        grades: FieldSpec::UNRECORDED,
        // The create form caps each upload slot separately — 4 GiB for the
        // product, 30 MiB for the preview, 4 MiB for each of four thumbnails,
        // 1 GiB for the video preview, and that last one moves with the
        // per-account `double_video_file_size_limit` variant. One cap for the
        // whole set would name none of them.
        files: FieldSpec::UNRECORDED,
    },
    natives: TPT_NATIVES,
    equivalence_axes: TPT_AXES,
    absent_axes: TPT_ABSENT_AXES,
};

/// Four axes into one field. TPT's 358 facets are one flat namespace where a
/// grade, a subject, a resource type and an audience are all the same kind of
/// thing, so the axis is a fact of the facet's own `category` rather than of
/// where it lands on the wire.
///
/// One of the four declares a cap, and the asymmetry is the evidence rather
/// than an oversight. The create form states a limit on each of its pickers,
/// but a stated limit is a claim until a write is refused: the 2026-08-30
/// create posted four `PreK-12-Subject-Area` slugs against a form that says
/// three, and TPT accepted them, so declaring the subject cap here would raise
/// an election about a set the platform already took. Grade is the one where
/// the form's number and every capture agree, so it is the one declared. Topic
/// and resource type have no picker of their own on the form and therefore no
/// measurement at all. The numbers live in
/// docs/design/data/tpt-vocabulary.json under `constraints.selectionCaps`,
/// which `tam_taxonomy::TptForm` reads; four is restated here because a
/// registry const cannot read a file, and
/// `the_declared_grade_cap_is_the_one_the_capture_states` holds the two
/// against each other.
const TPT_AXES: &[AxisBinding] = &[
    AxisBinding {
        axis: TermKind::Subject,
        native: "taxonomyTags",
        cardinality: Cardinality::Many { cap: None },
        delegation: Delegation::ByOptIn,
    },
    AxisBinding {
        axis: TermKind::Topic,
        native: "taxonomyTags",
        cardinality: Cardinality::Many { cap: None },
        delegation: Delegation::ByOptIn,
    },
    AxisBinding {
        axis: TermKind::ResourceType,
        native: "taxonomyTags",
        cardinality: Cardinality::Many { cap: None },
        delegation: Delegation::ByOptIn,
    },
    AxisBinding {
        axis: TermKind::Phase,
        native: "taxonomyTags",
        cardinality: Cardinality::Many {
            cap: Some(CountCap { limit: 4 }),
        },
        delegation: Delegation::ByOptIn,
    },
];

/// The legal exemplar, declared as data. The eight HAR captures and the
/// 2026-08-29 poll of the create and edit vocabularies between them reached
/// every field either write posts and every field the read returns, and none
/// of them is a licence: TPT sells under its own terms of service and offers
/// the seller no rights-grant choice. So a licence projected into TPT is a
/// disclosed loss rather than a question, which is the difference between
/// this list and simply declaring no binding.
const TPT_ABSENT_AXES: &[AxisAbsent] = &[AxisAbsent(TermKind::Licence)];

pub(super) const TPT_NATIVES: &[NativeField] = &[
    // One flat namespace, and the poll reached it whole: 358 facets,
    // referentially closed, at most two deep, where a grade (4th-grade), an
    // audience (homeschool), a subject (math), a resource type (unit-plans)
    // and a file format (pdf) are all the same kind of thing and only the
    // facet's own `category` tells them apart. 23 of the 358 are hidden —
    // retired values that read back on existing products and are never
    // offered on write. 358 slugs is far past what belongs in a const, so the
    // members live in docs/design/data/tpt-vocabulary.json and are not
    // captured here, which is the difference `ClosedUncaptured` names: an
    // unlisted slug is refused by the platform. Both writes post them as a
    // repeated `data[TaxonomyTags][]` array of the same slugs the read
    // returns, six on the create and eight on the edit, so the field crosses
    // both ways.
    NativeField {
        name: "taxonomyTags",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Categories),
            ordinal: 0,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    // Seller-owned shelves, so there is no platform-wide vocabulary to hold:
    // the members are whatever this seller created. The product read names
    // them `categories` with numeric ids and the gateway names the same
    // entities `customCategories` with the ids stringified; both writes post
    // those same numeric ids as a repeated `data[Category][Category][]`.
    NativeField {
        name: "categories",
        label: Some("Custom Category"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Categories),
            ordinal: 4,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
        delegation: Delegation::ByOptIn,
    },
    // `TaxCodesQuery` on /graph/graphql returned the complete five-row
    // `taxData` set: id 1 DA051011 digital audio, 2 DB031013 digital books,
    // 3 DI010200 digital images, 4 DV010200 videos, 5 DO010000 other digital
    // goods. The captured edit settles which half travels: it posted
    // `data[ItemTaxCode][tax_code_id] = 2` for a product the read reported as
    // `taxCode {id: "2"}` and DB031013, so the field carries the row id. The
    // create posted the field not at all.
    NativeField {
        name: "ItemTaxCode.tax_code_id",
        label: Some("Tax Code"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Price),
            ordinal: 4,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["1", "2", "3", "4", "5"]),
        delegation: Delegation::ByOptIn,
    },
    // The `ResourceType` enum, recovered from the statistics page bundle and
    // corrected to five members by the poll's sweep of the enum literals
    // across the chunk set, which found the `EASEL` member the earlier read
    // missed. Only DIGITAL_PRODUCT occurs across this seller's 154 products,
    // and the member is chosen on the create-type page rather than in a form
    // select.
    NativeField {
        name: "itemType",
        label: None,
        placement: None,
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "DIGITAL_PRODUCT",
            "BUNDLE",
            "ONLINE_RESOURCE",
            "EASEL",
            "VIDEO",
        ]),
        delegation: Delegation::ByOptIn,
    },
    // Every one of the 154 products read ACTIVE, on `status`, `statusAdmin`
    // and `statusCopyright` alike, so one store's steady state is all that is
    // on file. The seller filter offers ACTIVE, INACTIVE and FEATURED, but a
    // filter vocabulary is a different upstream enum from a product's status.
    NativeField {
        name: "status",
        label: None,
        placement: None,
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
        delegation: Delegation::ByOptIn,
    },
    // PDF and ZIP across 154 products, always equal to `filePreview.format`.
    // Two values from one store is a sample of what this seller uploads.
    NativeField {
        name: "kind",
        label: None,
        placement: None,
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
        delegation: Delegation::ByOptIn,
    },
    // `EducationStandardsJurisdictionsQuery` returned 166 jurisdiction roots
    // — Common Core, NGSS, TEKS and the rest — and the poll catalogued them,
    // but the form field takes a standard id from a leaf below one of those
    // roots. `EducationStandardsQuery($id, $depth)` expands a jurisdiction on
    // demand and full expansion across all 166 runs to thousands of nodes, so
    // the leaf ids are deliberately not captured. The set is closed upstream
    // and its members are not held here, which is exactly what an unlisted
    // value being refused means.
    NativeField {
        name: "ItemsCommonCoreStandard.common_core_standard_id",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::EducationStandards),
            ordinal: 0,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    // The eight small-integer fields the two writes post. Each is a member of
    // an enumeration the read side names in words — `statusUser: ACTIVE`,
    // `answerKey: INCLUDED`, `copyrightDeclaration: ORIGINAL_WORK`,
    // `teachingDuration: HOURS_1` — against an integer the form posts, and
    // the captures exercised at most two members of each. The poll recovered
    // four of those enumerations whole from the constants module in chunk
    // `tpt-frontend.1.1564`, cross-checked against the read-side label map,
    // so those four now hold the ids the form posts rather than the two
    // members a capture happened to carry. For the other four nothing
    // enumerates the set, and an unlisted member is refused by the platform,
    // which is exactly what `ClosedUncaptured` records.
    NativeField {
        // 0 NOT_ACTIVE and 1 ACTIVE, which settles the captured create as a
        // draft and the captured edit as live.
        name: "Item.status_user",
        label: Some("Make Listing Active"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::ProductStatus),
            ordinal: 0,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["0", "1"]),
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // 1 ORIGINAL_WORK and 2 USED_COPYRIGHTED_MATERIALS, with no unset
        // member: both writes posted 1 and read back as ORIGINAL_WORK. A
        // legal attestation of authorship: the connector posts it only where
        // the seller has made one, and never as a constant.
        name: "ItemsProperty.copyright_declaration",
        label: Some("Intellectual Property Rights"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Copyright),
            ordinal: 0,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["1", "2"]),
        // An attestation of authorship is legal content whatever axis sits
        // above it, and none does: this field is why delegation lives on the
        // field as well as on the axis.
        delegation: Delegation::Never(NonDelegable::LegalContent),
    },
    NativeField {
        // 0 NA, 1 INCLUDED, 2 NOT_INCLUDED, 3 DOES_NOT_APPLY,
        // 4 INCLUDED_WITH_RUBRIC, 5 RUBRIC_ONLY. The read-side label map also
        // carries an UNKNOWN with no posted counterpart, so that one is a
        // read sentinel rather than a member of the written set.
        name: "ItemsProperty.answer_key",
        label: Some("Answer Key"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Details),
            ordinal: 2,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["0", "1", "2", "3", "4", "5"]),
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // 0 NA through 22 OTHER, a scale of durations from 30 minutes to a
        // lifelong tool, so the captured 0 and 6 read as N/A and 1 hour.
        // UNKNOWN is again read-side only.
        name: "ItemsProperty.duration",
        label: Some("Teaching Duration"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Details),
            ordinal: 0,
        }),
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15",
            "16", "17", "18", "19", "20", "21", "22",
        ]),
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // 1 on the create and 3 on the edit, with no source stating what
        // either means; the poll found no enumeration for it either. Each
        // write reproduces the value its own capture carried rather than
        // generalising from one of them.
        name: "Item.generate_thumbnail",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Files),
            ordinal: 1,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // 0 on the create, 1 on the edit. The read side returns both a
        // countryId of 153 and a countryIdFlag; only the flag is ever posted,
        // and the poll found no country vocabulary in any chunk.
        name: "ItemsLocalization.country_id_flag",
        label: None,
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Categories),
            ordinal: 5,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // 1 on the create alongside price 0; the edit form omits the field
        // entirely, so the two endpoints disagree on whether it exists.
        name: "Item.free",
        label: Some("Free Resource"),
        placement: Some(FormPlacement {
            group: FieldGroup::Tpt(FormGroup::Price),
            ordinal: 0,
        }),
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // Posted as the literal 0 on the create while a generated thumbnail
        // collection was also posted. Whether it counts manual thumbnails,
        // selects between generated and manual, or means something else is
        // unsettled, so the captured value is reproduced rather than derived.
        name: "thumbs",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // The two opaque handles the create consumes: a 576-character
        // processed-asset key from the upload queue and an 88-character
        // thumbnail collection key. Server-issued encrypted envelopes over an
        // object path, with no vocabulary and no client-side derivation.
        name: "ItemDigital.product",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        // See `ItemDigital.product`.
        name: "thumbs_collection_key",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
        delegation: Delegation::ByOptIn,
    },
];

#[cfg(test)]
mod tests {
    use super::TPT;
    use crate::registry::{
        AxisAbsent, Cardinality, CountCap, Delegation, FieldDirection, LengthCap, NativeVocabulary,
        NonDelegable,
    };
    use crate::TermKind;
    use tam_types::{FieldKey, LengthUnit};

    fn cap(limit: usize, unit: LengthUnit) -> LengthCap {
        LengthCap { limit, unit }
    }
    #[test]
    fn the_tpt_title_cap_stays_undeclared_though_the_sample_and_the_constant_agree() {
        assert_eq!(
            TPT.canonical.title.cap, None,
            "80 is the longest observed title and the client's own constant, and neither is a \
             refusal anyone measured against a verified counting unit"
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
    fn no_tpt_field_is_declared_required_because_no_refusal_is_captured() {
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
                "both captured writes succeeded, so {key:?} records no refusal"
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
    fn the_tpt_tax_code_field_carries_the_row_id_the_edit_posted() {
        let tax = TPT
            .natives
            .iter()
            .find(|native| native.name == "ItemTaxCode.tax_code_id")
            .map(|native| native.vocabulary);
        let Some(NativeVocabulary::Closed(values)) = tax else {
            panic!("TaxCodesQuery returned a complete set, got {tax:?}");
        };
        assert_eq!(
            values,
            ["1", "2", "3", "4", "5"],
            "taxData carried five rows"
        );
        assert!(
            !values.contains(&"DA051011"),
            "the captured edit posted 2 for DB031013, so the field carries the id not the code"
        );
    }

    #[test]
    fn the_tpt_taxonomy_is_one_flat_namespace_closed_upstream_and_held_elsewhere() {
        let tags = TPT
            .natives
            .iter()
            .find(|native| native.name == "taxonomyTags");
        assert_eq!(
            tags.map(|native| (native.direction, native.vocabulary)),
            Some((FieldDirection::Both, NativeVocabulary::ClosedUncaptured)),
            "both writes post the same slugs the read returns, and the 358 facets are closed \
             upstream but catalogued in docs/design/data rather than in this const"
        );
    }

    fn tpt_vocabulary(name: &str) -> Option<NativeVocabulary> {
        TPT.natives
            .iter()
            .find(|native| native.name == name)
            .map(|native| native.vocabulary)
    }

    #[test]
    fn the_four_polled_tpt_enum_to_id_pairs_hold_the_ids_the_form_posts() {
        // Restated from the create form's own constants module rather than
        // imported, as everything else in this module is.
        const CAPTURED: [(&str, &[&str]); 4] = [
            ("Item.status_user", &["0", "1"]),
            ("ItemsProperty.copyright_declaration", &["1", "2"]),
            ("ItemsProperty.answer_key", &["0", "1", "2", "3", "4", "5"]),
            (
                "ItemsProperty.duration",
                &[
                    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14",
                    "15", "16", "17", "18", "19", "20", "21", "22",
                ],
            ),
        ];
        for (name, ids) in CAPTURED {
            assert_eq!(
                tpt_vocabulary(name),
                Some(NativeVocabulary::Closed(ids)),
                "{name} holds the whole posted scale, not the one or two members a capture \
                 happened to carry"
            );
        }
    }

    #[test]
    fn the_two_unpolled_tpt_enum_to_id_pairs_stay_closed_but_uncaptured() {
        const UNCAPTURED: [&str; 2] = [
            "Item.generate_thumbnail",
            "ItemsLocalization.country_id_flag",
        ];
        for name in UNCAPTURED {
            assert_eq!(
                tpt_vocabulary(name),
                Some(NativeVocabulary::ClosedUncaptured),
                "nothing enumerates {name}, and an unlisted member is refused by the platform"
            );
        }
    }

    #[test]
    fn the_tpt_item_type_vocabulary_carries_the_easel_member_too() {
        assert_eq!(
            tpt_vocabulary("itemType"),
            Some(NativeVocabulary::Closed(&[
                "DIGITAL_PRODUCT",
                "BUNDLE",
                "ONLINE_RESOURCE",
                "EASEL",
                "VIDEO",
            ])),
            "the chunk sweep found five ResourceType members where the statistics page \
             bundle showed four"
        );
    }

    #[test]
    fn the_tpt_write_path_fields_are_all_present_in_the_registry() {
        let written = TPT
            .natives
            .iter()
            .filter(|native| {
                matches!(
                    native.direction,
                    FieldDirection::Written | FieldDirection::Both
                )
            })
            .count();
        assert_eq!(
            written, 14,
            "the registry is the per-inventory record of what the adapter puts on the wire"
        );
    }

    #[test]
    fn tpt_binds_four_axes_into_one_field_and_records_the_licence_absent() {
        let bound: Vec<TermKind> = TPT
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
            ],
            "one flat facet namespace carries all four, and the category tells them apart"
        );
        for binding in TPT.equivalence_axes {
            assert_eq!(
                binding.native, "taxonomyTags",
                "{:?} lands in the same array as the rest",
                binding.axis
            );
        }
        assert_eq!(
            TPT.absent_axes,
            [AxisAbsent(TermKind::Licence)],
            "TPT offers the seller no rights-grant choice, and that measured absence is what \
             makes a projected licence a disclosed loss rather than a question"
        );
        assert_eq!(
            TPT.axis(TermKind::Licence),
            None,
            "a measured absence is not also a binding"
        );
    }

    /// One of the four axes carries a cap, and the other three carry none for
    /// three different reasons. Asserting all four together is what keeps a
    /// later editor from tidying the asymmetry away.
    #[test]
    fn only_the_grade_axis_declares_a_cardinality_cap() {
        let cap = |axis| TPT.axis(axis).map(|binding| binding.cardinality);
        assert_eq!(
            cap(TermKind::Phase),
            Some(Cardinality::Many {
                cap: Some(CountCap { limit: 4 })
            }),
            "the create form says four grades and no capture contradicts it"
        );
        for axis in [TermKind::Subject, TermKind::Topic, TermKind::ResourceType] {
            assert_eq!(
                cap(axis),
                Some(Cardinality::Many { cap: None }),
                "{axis:?} is either contradicted by a capture or never measured, and an \
                 absent cap is unmeasured rather than unlimited"
            );
        }
    }

    #[test]
    fn the_tpt_copyright_declaration_refuses_delegation_with_no_axis_above_it() {
        let Some(field) = TPT.native("ItemsProperty.copyright_declaration") else {
            panic!("both writes posted it and both reads returned it");
        };
        assert_eq!(
            field.delegation,
            Delegation::Never(NonDelegable::LegalContent),
            "an attestation of authorship is legal content, and no equivalence axis covers it, \
             which is why delegation is a property of the field too"
        );
    }
}
