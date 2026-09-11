//! The seller-facing label for a native vocabulary value, read out of the
//! committed captures rather than transcribed beside them.
//!
//! The field registry holds the wire tokens, because a token is what an
//! adapter posts: Tes `mainType` is `99001`, its GB age bands are `1` through
//! `7`, and TPT's teaching duration is `0` through `22`. None of those is a
//! thing a seller can pick from, and the labels the uploader shows were
//! captured in the same poll that established the tokens, so they are already
//! on file in `docs/design/data`. This module joins the two.
//!
//! Reading the captures rather than restating them is the same discipline the
//! grade derivation follows: a re-poll changes the labels the form renders and
//! leaves no transcription behind to drift. Nothing here decides anything, so
//! an option whose capture carries no label yields none, and the caller shows
//! the token. Inventing a reading of `1` would put a claim on a seller's
//! screen that no capture supports.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;
use tam_types::{InventoryId, Marketplace};

const TES_CAPTURE: &str = include_str!("../../../docs/design/data/tes-vocabulary.json");
const TPT_CAPTURE: &str = include_str!("../../../docs/design/data/tpt-vocabulary.json");

/// Which captured option set holds the labels for one registry native field,
/// as `(the registry's field name, the capture's key)`.
///
/// Only the fields whose tokens are opaque are here. Tes `curriculum` and TPT
/// `itemType` are captured as plain arrays whose members are their own names —
/// `American`, `DIGITAL_PRODUCT` — and a value that is already its own label
/// needs no join.
const TES_FIELDS: [(&str, &str); 4] = [
    ("ageRanges", "ageRanges"),
    ("yearGroups", "yearGroups"),
    ("licence", "licences"),
    ("mainType", "resourceTypes"),
];

/// TPT's half of the same table. `ItemsProperty.copyright_declaration` and
/// `Item.status_user` are deliberately absent: their captures hold only the
/// platform's own `enum` token — `ORIGINAL_WORK`, `ACTIVE` — and no
/// seller-facing label, so there is nothing here to serve.
const TPT_FIELDS: [(&str, &str); 3] = [
    ("ItemTaxCode.tax_code_id", "taxCodes"),
    ("ItemsProperty.duration", "teachingDuration"),
    ("ItemsProperty.answer_key", "answerKey"),
];

/// The keys a captured option spells its label under. Four rather than one
/// because the captures name the field as their own source named it:
/// `ageRanges` carries `label`, `licences` carries `title`, `yearGroups`
/// carries `group`, `taxCodes` carries `name`.
const LABEL_KEYS: [&str; 4] = ["label", "title", "group", "name"];

type LabelTable = BTreeMap<&'static str, BTreeMap<String, String>>;

static TES_LABELS: OnceLock<LabelTable> = OnceLock::new();
static TPT_LABELS: OnceLock<LabelTable> = OnceLock::new();

fn label_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(fields) => LABEL_KEYS
            .into_iter()
            .find_map(|key| fields.get(key).and_then(Value::as_str))
            .map(str::to_owned),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    }
}

/// Builds one marketplace's table. A capture that does not parse, or a field
/// whose option set is not a keyed object, contributes nothing: this is
/// display material, and a form that shows tokens is a worse form rather than
/// a broken one.
fn table(capture: &str, fields: &[(&'static str, &str)]) -> LabelTable {
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(capture) else {
        return LabelTable::new();
    };
    let mut out = LabelTable::new();
    for (field, key) in fields {
        let Some(Value::Object(options)) = root.get(*key).and_then(|set| set.get("options")) else {
            continue;
        };
        let labelled: BTreeMap<String, String> = options
            .iter()
            .filter_map(|(id, value)| label_of(value).map(|label| (id.clone(), label)))
            .collect();
        if !labelled.is_empty() {
            out.insert(*field, labelled);
        }
    }
    out
}

/// The label the capture carries for one native field's value, or `None` where
/// it carries none.
///
/// Keyed on the marketplace rather than the inventory: Tes GB, US and NZ share
/// one field table and one captured vocabulary, and the country fork lives in
/// which field a resource fills rather than in what the options mean.
#[must_use]
pub fn native_label(inventory: InventoryId, field: &str, native_id: &str) -> Option<&'static str> {
    let table = match inventory.marketplace() {
        Marketplace::Tes => TES_LABELS.get_or_init(|| table(TES_CAPTURE, &TES_FIELDS)),
        Marketplace::Tpt => TPT_LABELS.get_or_init(|| table(TPT_CAPTURE, &TPT_FIELDS)),
        // No Etsy vocabulary has been polled, and its one captured closed set
        // is the self-labelling `who_made`.
        Marketplace::Etsy => return None,
    };
    table.get(field)?.get(native_id).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::{native_label, TES_FIELDS, TPT_FIELDS};
    use crate::tes::TES_MAIN_AGE_RANGES;
    use tam_domain::registry::{registry, NativeVocabulary};
    use tam_types::InventoryId;

    #[test]
    fn the_opaque_tes_tokens_reach_a_form_as_words() {
        assert_eq!(
            native_label(InventoryId::Tes, "mainType", "99001"),
            Some("Assembly"),
            "the resource-type select is nine five-digit ids and nine names"
        );
        assert_eq!(
            native_label(InventoryId::Tes, "ageRanges", "3"),
            Some("7-11"),
            "the band a seller picks, not its row number"
        );
        assert_eq!(
            native_label(InventoryId::Tes, "yearGroups", "3"),
            Some("1"),
            "year group 3 is named 1, which is exactly why it is not age band 3"
        );
        assert_eq!(
            native_label(InventoryId::Tes, "licence", "TES-PAID"),
            Some("Teaching Resource Licence"),
            "the licence title the refdata store holds"
        );
    }

    #[test]
    fn the_tpt_scales_carry_their_captured_labels() {
        assert_eq!(
            native_label(InventoryId::Tpt, "ItemsProperty.duration", "6"),
            Some("1 Hour"),
            "the captured 6 is an hour rather than a rank, in the title case the 2026-09-03 \
             DOM read shows the seller"
        );
        assert_eq!(
            native_label(InventoryId::Tpt, "ItemsProperty.answer_key", "1"),
            Some("Included"),
            "the answer-key scale is captured with labels"
        );
    }

    #[test]
    fn an_option_whose_capture_holds_no_label_yields_none() {
        for field in ["Item.status_user", "ItemsProperty.copyright_declaration"] {
            assert_eq!(
                native_label(InventoryId::Tpt, field, "1"),
                None,
                "{field} is captured as a machine token alone, and a reading of 1 would be \
                 invented rather than measured"
            );
        }
        assert_eq!(
            native_label(InventoryId::Etsy, "who_made", "i_did"),
            None,
            "no Etsy vocabulary has been polled, and this token is its own name"
        );
        assert_eq!(
            native_label(InventoryId::Tes, "mainType", "43788"),
            None,
            "a legacy read-only type is not a member of the writable set"
        );
    }

    /// The one transcribed copy of a captured label in this crate, checked
    /// against the capture it was transcribed from.
    #[test]
    fn the_age_range_table_still_agrees_with_the_capture() {
        for row in TES_MAIN_AGE_RANGES {
            assert_eq!(
                native_label(InventoryId::Tes, "ageRanges", row.native_id),
                Some(row.label),
                "the const table and docs/design/data/tes-vocabulary.json name band {} \
                 differently",
                row.native_id
            );
        }
    }

    /// A field renamed in the registry, or an option set renamed in a re-poll,
    /// silently stops labelling anything. This is what catches that.
    #[test]
    fn every_mapped_field_is_a_native_field_that_holds_captured_values() {
        let mapped = [
            (InventoryId::Tes, TES_FIELDS.as_slice()),
            (InventoryId::Tpt, TPT_FIELDS.as_slice()),
        ];
        for (inventory, fields) in mapped {
            for (field, key) in fields {
                let native = registry(inventory)
                    .native(field)
                    .unwrap_or_else(|| panic!("{field} is a native field of {inventory:?}"));
                let NativeVocabulary::Closed(values) = native.vocabulary else {
                    panic!("{field} holds a captured closed set, or it needs no labels");
                };
                for value in values {
                    assert!(
                        native_label(inventory, field, value).is_some(),
                        "{key} carries no label for {field} value {value}"
                    );
                }
            }
        }
    }
}
