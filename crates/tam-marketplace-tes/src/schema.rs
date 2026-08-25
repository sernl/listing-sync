//! The JSON-API analog of the selector-pack pre-flight: the draft object's
//! sorted field-name set, canonically encoded and hashed into a
//! `FormSchemaFingerprint`. A field appearing or vanishing is the maintenance
//! leading indicator the kill-gate section instruments here — a schema that
//! drifted is a break that has not surfaced yet.

use serde_json::Value;
use sha2::{Digest, Sha256};
use tam_marketplace::{FormId, FormSchemaFingerprint, SchemaDrift};
use tam_types::ContentHash;

/// The draft fields this adapter WRITES, which must all exist in the observed
/// schema for a submit to be safe. This is deliberately the written subset,
/// not the full draft shape: the authoritative full baseline is recorded and
/// maintained by the scheduled canary (M1d) from live observation, because a
/// hand-maintained copy of somebody else's schema is a drift detector that
/// itself drifts.
pub const WRITTEN_DRAFT_FIELDS: [&str; 10] = [
    "ageRanges",
    "ages",
    "categories",
    "descriptionRaw",
    "descriptionRawType",
    "licence",
    "mainAge",
    "mainType",
    "title",
    "yearGroups",
];

/// The top-level field names of a draft object, sorted and deduplicated.
#[must_use]
pub fn field_names(draft: &Value) -> Vec<String> {
    let mut names: Vec<String> = draft
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

/// Order-independent by construction: the canonical encoding is the sorted
/// name list joined by newlines, so two reads of one schema fingerprint
/// identically whatever order the server serialised the object in.
#[must_use]
pub fn fingerprint(fields: &[String]) -> FormSchemaFingerprint {
    let mut sorted: Vec<&str> = fields.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut hasher = Sha256::new();
    for name in sorted {
        hasher.update(name.as_bytes());
        hasher.update(b"\n");
    }
    FormSchemaFingerprint(ContentHash(hasher.finalize().into()))
}

/// Compares an observed field set against an expected one, naming exactly
/// what appeared and what vanished. `None` means no drift.
#[must_use]
pub fn drift(form: FormId, expected: &[String], observed: &[String]) -> Option<Box<SchemaDrift>> {
    let added: Vec<String> = observed
        .iter()
        .filter(|name| !expected.contains(name))
        .cloned()
        .collect();
    let removed: Vec<String> = expected
        .iter()
        .filter(|name| !observed.contains(name))
        .cloned()
        .collect();
    if added.is_empty() && removed.is_empty() {
        return None;
    }
    Some(Box::new(SchemaDrift {
        form,
        expected: fingerprint(expected),
        observed: fingerprint(observed),
        added,
        removed,
    }))
}

/// The written-subset assertion: every field this adapter writes must exist
/// in the observed draft, or a submit would type values into a form that no
/// longer carries them.
#[must_use]
pub fn missing_written_fields(observed: &[String]) -> Vec<&'static str> {
    WRITTEN_DRAFT_FIELDS
        .into_iter()
        .filter(|field| !observed.iter().any(|name| name == field))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{drift, field_names, fingerprint, missing_written_fields};
    use serde_json::json;
    use tam_marketplace::FormId;
    use tam_types::Uuid;

    fn owned(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_fingerprint_is_stable_under_field_order() {
        let forward = fingerprint(&owned(&["a", "b", "c"]));
        let shuffled = fingerprint(&owned(&["c", "a", "b"]));
        assert_eq!(
            forward, shuffled,
            "serialisation order must not read as schema drift"
        );
    }

    #[test]
    fn a_changed_field_set_changes_the_fingerprint() {
        assert_ne!(
            fingerprint(&owned(&["a", "b"])),
            fingerprint(&owned(&["a", "b", "c"])),
            "a new field must move the fingerprint or the pre-flight is blind"
        );
    }

    #[test]
    fn drift_names_exactly_what_appeared_and_vanished() {
        let form = FormId(Uuid([9; 16]));
        let report =
            drift(form, &owned(&["a", "b"]), &owned(&["b", "c"])).expect("a changed set is drift");
        assert_eq!(report.added, vec!["c".to_owned()], "the new field is named");
        assert_eq!(
            report.removed,
            vec!["a".to_owned()],
            "the vanished field is named"
        );
        assert!(
            drift(form, &owned(&["a"]), &owned(&["a"])).is_none(),
            "an identical set is not drift"
        );
    }

    #[test]
    fn field_names_come_sorted_from_the_object() {
        let names = field_names(&json!({"z": 1, "a": 2, "m": 3}));
        assert_eq!(
            names,
            owned(&["a", "m", "z"]),
            "names are canonically ordered regardless of the wire"
        );
    }

    #[test]
    fn a_draft_missing_a_written_field_is_named() {
        let observed = owned(&["title", "licence"]);
        let missing = missing_written_fields(&observed);
        assert!(
            missing.contains(&"descriptionRaw"),
            "a field we write that the form lost must be named before any submit"
        );
        assert!(
            !missing.contains(&"title"),
            "a present field is not missing"
        );
    }
}
