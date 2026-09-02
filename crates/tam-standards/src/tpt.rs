//! The code-to-TPT-node-id table: defined, loadable, and empty.
//!
//! TPT binds a standard by an opaque internal numeric node id, read back as
//! `sphinxId`, rather than by the published code (`docs/research/rethink/
//! tpt-product-model.md`). The create form posts
//! `data[ItemsCommonCoreStandard][common_core_standard_id][]`, so a published
//! code alone cannot post a selection and this table is what makes the feature
//! work on the wire.
//!
//! It is empty because filling it needs a live TPT session to crawl
//! `EducationStandardsQuery` one subtree at a time, which is founder-gated and
//! out of scope here. The structure exists now so that the ingest it joins
//! against is shaped for it rather than retrofitted to it.
//!
//! Two fields exist for the risk the recon named rather than for the join. An
//! id aliased as `sphinxId` is a search-index identifier, and search indexes
//! get rebuilt; if the ids move, a posted edge tags the wrong standard
//! silently. So a binding records the statement it was verified against and
//! when, which is what lets a caller refuse to post an id outside the current
//! crawl window and degrade to "standards not projected" rather than to a
//! wrong tag. Nothing here repairs a mismatch: a code whose statement no
//! longer hashes the same is a reconciliation item, not a value to correct in
//! place.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::format::LoadError;
use crate::model::Framework;

/// TPT's own numeric node id for a standard, the value
/// `common_core_standard_id[]` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TptNodeId(pub u64);

/// One code bound to one TPT node id, with the evidence the binding was true
/// when it was taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TptBinding {
    pub framework: Framework,
    /// The mirror's own identifier for the node this binds: the key, and the
    /// only unambiguous name a standard has on our side.
    ///
    /// No key built from the published code holds. In the ingest of
    /// 2026-09-02, 697 of 4,872 TEKS codes name a different standard under a
    /// different subject, and even (framework, subject, code) collides on 414
    /// Texas keys, 331 of them carrying different statements. Binding on
    /// a code would post whichever of them this table happened to hold. The
    /// ambiguity is resolved once, by the crawl, and recorded here.
    pub source_guid: String,
    /// The mirrored set's subject label, carried so a reviewer can read a
    /// binding without joining, and never part of the key.
    pub subject: String,
    /// The published code, exactly as the ingest carries it. Also not part of
    /// the key.
    pub code: String,
    pub tpt_node_id: TptNodeId,
    /// SHA-256 of the statement TPT returned for this node at crawl time.
    /// A later crawl that reads a different statement under the same id has
    /// found a moved id, not a corrected one.
    pub statement_sha256: String,
    /// RFC 3339, in UTC: when the crawl last saw this triple agree.
    pub verified_at: String,
}

/// Every binding the last crawl produced, keyed so a projection can ask for
/// one code at a time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TptNodeIdTable {
    bindings: BTreeMap<String, TptBinding>,
}

impl TptNodeIdTable {
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// The binding for a code, or nothing.
    ///
    /// Nothing is the answer the whole table gives today, and a caller that
    /// treats it as a reason to omit the standards field rather than to guess
    /// an id behaves correctly both now and after the crawl lands.
    pub fn get(&self, source_guid: &str) -> Option<&TptBinding> {
        self.bindings.get(source_guid)
    }

    /// Every binding whose published code matches, which may be more than one.
    ///
    /// Returned as a list rather than an option because a code is ambiguous in
    /// two of the four frameworks, and a caller that has only a code has to
    /// see that rather than be handed an arbitrary one of them.
    pub fn candidates_for_code(&self, framework: Framework, code: &str) -> Vec<&TptBinding> {
        self.bindings
            .values()
            .filter(|binding| binding.framework == framework && binding.code == code)
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TptBinding> {
        self.bindings.values()
    }
}

/// Read the committed bindings file: one binding per line, blank lines
/// ignored, and a duplicate key refused rather than resolved.
///
/// A key bound twice means two crawls disagree about which node TPT means,
/// and picking either would post a tag nobody chose.
pub fn load_tpt_node_ids(jsonl: &str) -> Result<TptNodeIdTable, LoadError> {
    let mut bindings = BTreeMap::new();
    for (offset, line) in jsonl.lines().enumerate() {
        let number = offset + 1;
        if line.trim().is_empty() {
            continue;
        }
        let binding: TptBinding = serde_json::from_str(line).map_err(|error| LoadError::Line {
            line: number,
            message: error.to_string(),
        })?;
        let key = binding.source_guid.clone();
        if let Some(existing) = bindings.insert(key, binding) {
            return Err(LoadError::Line {
                line: number,
                message: format!(
                    "{} node `{}` (code `{}`) is bound twice; the earlier binding named TPT node {}",
                    existing.framework, existing.source_guid, existing.code, existing.tpt_node_id.0
                ),
            });
        }
    }
    Ok(TptNodeIdTable { bindings })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINDING: &str = r#"{"framework":"CCSS","subject":"Mathematics","code":"CCSS.Math.Content.8.F.B.5","source_guid":"A6F2D78E7F294A2EA04C158DD1A47EC4","tpt_node_id":91234,"statement_sha256":"00","verified_at":"2026-09-03T00:00:00Z"}"#;
    const TEKS_MATH: &str = r#"{"framework":"TEKS","subject":"Mathematics (2012-)","code":"1.1.A","source_guid":"M1","tpt_node_id":11,"statement_sha256":"00","verified_at":"2026-09-03T00:00:00Z"}"#;
    const TEKS_SCIENCE: &str = r#"{"framework":"TEKS","subject":"Science (2020-)","code":"1.1.A","source_guid":"S1","tpt_node_id":22,"statement_sha256":"00","verified_at":"2026-09-03T00:00:00Z"}"#;

    #[test]
    fn the_committed_table_is_empty_and_loads() {
        let table = load_tpt_node_ids("").expect("an empty table is a valid table");
        assert!(table.is_empty(), "nothing is bound until the crawl runs");
        assert_eq!(
            table.get("A6F2D78E7F294A2EA04C158DD1A47EC4"),
            None,
            "an unbound code answers with nothing rather than a guess"
        );
    }

    #[test]
    fn a_binding_round_trips_through_the_line_format() {
        let table = load_tpt_node_ids(BINDING).expect("load");
        assert_eq!(table.len(), 1);
        let binding = table
            .get("A6F2D78E7F294A2EA04C158DD1A47EC4")
            .expect("the node is bound");
        assert_eq!(binding.tpt_node_id, TptNodeId(91234));
        assert_eq!(binding.verified_at, "2026-09-03T00:00:00Z");
    }

    #[test]
    fn blank_lines_are_ignored() {
        let table = load_tpt_node_ids(&format!("\n{BINDING}\n\n")).expect("load");
        assert_eq!(table.len(), 1, "a blank line is not a binding");
    }

    #[test]
    fn one_code_under_two_subjects_is_two_bindings_rather_than_a_collision() {
        let table = load_tpt_node_ids(&format!("{TEKS_MATH}\n{TEKS_SCIENCE}")).expect("load");
        assert_eq!(
            table.len(),
            2,
            "TEKS 1.1.A names a different standard per subject"
        );
        assert_eq!(
            table.get("M1").map(|binding| binding.tpt_node_id),
            Some(TptNodeId(11))
        );
        assert_eq!(
            table.get("S1").map(|binding| binding.tpt_node_id),
            Some(TptNodeId(22)),
            "the science standard must not be shadowed by the mathematics one"
        );
        let ambiguous = table.candidates_for_code(Framework::Teks, "1.1.A");
        assert_eq!(
            ambiguous.len(),
            2,
            "a caller holding only the code must see both rather than one of them"
        );
    }

    #[test]
    fn a_key_bound_twice_is_refused_rather_than_resolved() {
        let error = load_tpt_node_ids(&format!("{BINDING}\n{BINDING}"))
            .expect_err("two bindings for one code must not load");
        assert!(
            matches!(error, LoadError::Line { line: 2, .. }),
            "the second line is named, got {error:?}"
        );
    }
}
