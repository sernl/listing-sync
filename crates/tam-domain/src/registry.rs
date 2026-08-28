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
    },
    // `RefdataStore.resourceTypes` holds 33 rows and the uploader's type
    // select filters them to `99000 < id < 99010`, so these nine are the
    // writable set; the other 24 are legacy or attachment-level types that
    // read back on older resources and are never offered. The ids are the
    // wire tokens and the labels are catalogued in
    // docs/design/data/tes-vocabulary.json.
    NativeField {
        name: "mainType",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "99001", "99002", "99003", "99004", "99005", "99006", "99007", "99008", "99009",
        ]),
    },
    // A pointer into whichever age vocabulary the country selects rather than
    // a vocabulary of its own: the uploader takes `ageRanges` for GB and
    // `yearGroups` everywhere else, and the 2026-08-29 poll read `mainAge` as
    // an `ageRanges` id on a GB resource without establishing what it carries
    // on the other branch. No single closed set is on file.
    NativeField {
        name: "mainAge",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
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
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
            "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30",
        ]),
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
const TPT: InventoryRegistry = InventoryRegistry {
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
};

const TPT_NATIVES: &[NativeField] = &[
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
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
    // Seller-owned shelves, so there is no platform-wide vocabulary to hold:
    // the members are whatever this seller created. The product read names
    // them `categories` with numeric ids and the gateway names the same
    // entities `customCategories` with the ids stringified; both writes post
    // those same numeric ids as a repeated `data[Category][Category][]`.
    NativeField {
        name: "categories",
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Numeric,
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
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["1", "2", "3", "4", "5"]),
    },
    // The `ResourceType` enum, recovered from the statistics page bundle and
    // corrected to five members by the poll's sweep of the enum literals
    // across the chunk set, which found the `EASEL` member the earlier read
    // missed. Only DIGITAL_PRODUCT occurs across this seller's 154 products,
    // and the member is chosen on the create-type page rather than in a form
    // select.
    NativeField {
        name: "itemType",
        direction: FieldDirection::ReadOnly,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "DIGITAL_PRODUCT",
            "BUNDLE",
            "ONLINE_RESOURCE",
            "EASEL",
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
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
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
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["0", "1"]),
    },
    NativeField {
        // 1 ORIGINAL_WORK and 2 USED_COPYRIGHTED_MATERIALS, with no unset
        // member: both writes posted 1 and read back as ORIGINAL_WORK. A
        // legal attestation of authorship: the connector posts it only where
        // the seller has made one, and never as a constant.
        name: "ItemsProperty.copyright_declaration",
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["1", "2"]),
    },
    NativeField {
        // 0 NA, 1 INCLUDED, 2 NOT_INCLUDED, 3 DOES_NOT_APPLY,
        // 4 INCLUDED_WITH_RUBRIC, 5 RUBRIC_ONLY. The read-side label map also
        // carries an UNKNOWN with no posted counterpart, so that one is a
        // read sentinel rather than a member of the written set.
        name: "ItemsProperty.answer_key",
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&["0", "1", "2", "3", "4", "5"]),
    },
    NativeField {
        // 0 NA through 22 OTHER, a scale of durations from 30 minutes to a
        // lifelong tool, so the captured 0 and 6 read as N/A and 1 hour.
        // UNKNOWN is again read-side only.
        name: "ItemsProperty.duration",
        direction: FieldDirection::Both,
        required: false,
        vocabulary: NativeVocabulary::Closed(&[
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15",
            "16", "17", "18", "19", "20", "21", "22",
        ]),
    },
    NativeField {
        // 1 on the create and 3 on the edit, with no source stating what
        // either means; the poll found no enumeration for it either. Each
        // write reproduces the value its own capture carried rather than
        // generalising from one of them.
        name: "Item.generate_thumbnail",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
    NativeField {
        // 0 on the create, 1 on the edit. The read side returns both a
        // countryId of 153 and a countryIdFlag; only the flag is ever posted,
        // and the poll found no country vocabulary in any chunk.
        name: "ItemsLocalization.country_id_flag",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
    NativeField {
        // 1 on the create alongside price 0; the edit form omits the field
        // entirely, so the two endpoints disagree on whether it exists.
        name: "Item.free",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
    NativeField {
        // Posted as the literal 0 on the create while a generated thumbnail
        // collection was also posted. Whether it counts manual thumbnails,
        // selects between generated and manual, or means something else is
        // unsettled, so the captured value is reproduced rather than derived.
        name: "thumbs",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::ClosedUncaptured,
    },
    NativeField {
        // The two opaque handles the create consumes: a 576-character
        // processed-asset key from the upload queue and an 88-character
        // thumbnail collection key. Server-issued encrypted envelopes over an
        // object path, with no vocabulary and no client-side derivation.
        name: "ItemDigital.product",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
    },
    NativeField {
        // See `ItemDigital.product`.
        name: "thumbs_collection_key",
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Unmeasured,
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
            6,
            "the licence, the curriculum read, and the four remaining written natives"
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
