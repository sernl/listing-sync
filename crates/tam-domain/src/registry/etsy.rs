//! Etsy, whose connector is not built. Every entry comes from the published
//! API reference rather than from a capture, and no equivalence axis is
//! declared, which is what keeps an Etsy projection blocking.
//!
//! No field carries a label or a placement, for the same reason: an API
//! reference names wire fields and not the words a seller reads or the form
//! that holds them. Both stay absent until a capture supplies them, and a
//! form renders the wire names rather than a reading invented here.

use super::{
    CanonicalFields, Delegation, FieldDirection, FieldSpec, InventoryRegistry, LengthCap,
    NativeField, NativeVocabulary,
};
use tam_types::{InventoryId, LengthUnit};

/// Etsy's published API reference for `createDraftListing` is the source for
/// every entry: the required create fields, the title cap, and the three
/// required natives.
pub(super) const ETSY: InventoryRegistry = InventoryRegistry {
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
    // No axis is declared and none is recorded absent: the connector is not
    // built, so nothing about Etsy's equivalence surface is measured. That is
    // the unmeasured case, and it blocks rather than disclosing a loss.
    equivalence_axes: &[],
    absent_axes: &[],
};

const ETSY_NATIVES: &[NativeField] = &[
    NativeField {
        name: "quantity",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: true,
        vocabulary: NativeVocabulary::Numeric,
        delegation: Delegation::ByOptIn,
    },
    NativeField {
        name: "who_made",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: true,
        vocabulary: NativeVocabulary::Closed(&["i_did", "someone_else", "collective"]),
        delegation: Delegation::ByOptIn,
    },
    // Closed upstream and required on create, with the members not captured
    // here. Etsy refuses an unlisted value, so a guess is a failed create.
    NativeField {
        name: "when_made",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: true,
        vocabulary: NativeVocabulary::ClosedUncaptured,
        delegation: Delegation::ByOptIn,
    },
    // Etsy documents at most thirteen tags of at most twenty characters each.
    // A collection cardinality has no home in `LengthCap`, and inventing one
    // for a single field would be worse than recording the limit here until
    // the connector needs to enforce it.
    NativeField {
        name: "tags",
        label: None,
        placement: None,
        direction: FieldDirection::Written,
        required: false,
        vocabulary: NativeVocabulary::Free,
        delegation: Delegation::ByOptIn,
    },
];

#[cfg(test)]
mod tests {
    use super::ETSY;
    use crate::registry::{registry, LengthCap};
    use tam_types::{InventoryId, LengthUnit};

    fn cap(limit: usize, unit: LengthUnit) -> LengthCap {
        LengthCap { limit, unit }
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
    fn etsy_declares_no_axis_and_records_no_absence_because_none_is_measured() {
        assert!(
            ETSY.equivalence_axes.is_empty() && ETSY.absent_axes.is_empty(),
            "the connector is unbuilt, so nothing about Etsy's equivalence surface is a \
             finding either way, and an Etsy projection blocks"
        );
    }
}
