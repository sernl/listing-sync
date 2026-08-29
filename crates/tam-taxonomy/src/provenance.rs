//! The shape check an edge passes before it becomes durable.
//!
//! A projection edge names the identifier a target marketplace will be posted,
//! and the write model that posts it has no way to tell an identifier that
//! marketplace issued from one another marketplace did — the wire carries the
//! string and nothing else. The adapter refuses what it can recognise as
//! foreign, one listing at a time and after the seller has already asked for
//! the cross-listing; this is the same test, run where the edge is authored
//! rather than where it is spent.
//!
//! Two paths author one. The operator seeder derives the relation from the
//! committed captures and writes it in bulk, so it checks every edge before
//! any of them becomes durable. The reconciliation queue's own resolution
//! writes one edge at a time from what a human named through the API, and its
//! answer shape makes the native id optional, so it checks the single edge it
//! is about to write. Both call `check_native_ids`, which is why it is total
//! over a slice rather than shaped for either caller: an edge that reaches
//! `projection_edge` unchecked is global — the table carries no organisation
//! — and permanent, because the uniqueness indexes refuse a corrected row and
//! nothing deletes one.
//!
//! Only TPT's tag namespace is checked here, because it is the only target
//! whose identifiers have a recognisable shape: Tes addresses everything it
//! binds by number, which `tam-marketplace-tes` parses at projection, and Etsy
//! binds nothing. The check is registry-driven rather than axis-listed, so a
//! fifth TPT axis bound to `taxonomyTags` is covered the day it is declared.

use tam_domain::registry::registry;
use tam_domain::{ProjectionEdge, VocabularyId};
use tam_types::natives::is_tpt_tag_slug;
use tam_types::{CanonicalTermId, InventoryId};

/// The native field TPT's registry binds every one of its equivalence axes
/// to, and the one namespace on that platform addressed by slug rather than
/// by number. Named here rather than matched on the axis, so the set of
/// checked axes is read off the registry instead of transcribed beside it.
const TPT_TAG_FIELD: &str = "taxonomyTags";

/// One edge whose target identifier is not the shape its target vocabulary
/// issues: the term it starts from, the vocabulary it points into, and the
/// identifier that would have been posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignNativeId {
    pub term: CanonicalTermId,
    pub target: VocabularyId,
    /// Absent where the edge carries no identifier at all, which is equally
    /// unpostable: TPT addresses its tags by identifiers it issued, and there
    /// is nothing to send in place of one.
    pub native_id: Option<String>,
}

/// Every edge the relation would have seeded under an identifier its target
/// cannot answer to.
///
/// The whole set rather than the first, because these arrive from a
/// derivation over a polled vocabulary: one wrong join produces a hundred of
/// them, and an operator fixing them one seed run at a time learns nothing
/// the first run could not have told them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignNativeIds(pub Vec<ForeignNativeId>);

impl core::fmt::Display for ForeignNativeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let VocabularyId(inventory, kind) = self.target;
        let term = self.term.0.to_hyphenated();
        match &self.native_id {
            Some(native) => write!(
                f,
                "the edge from term {term} into ({inventory:?}, {kind:?}) carries the \
                 identifier {native:?}, which is not the slug shape TPT issues its taxonomy \
                 tags in",
            ),
            None => write!(
                f,
                "the edge from term {term} into ({inventory:?}, {kind:?}) carries no \
                 identifier at all, and TPT addresses its taxonomy tags by identifiers it \
                 issued",
            ),
        }
    }
}

impl core::fmt::Display for ForeignNativeIds {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(
            f,
            "{} projection edge(s) carry an identifier their target marketplace did not issue:",
            self.0.len()
        )?;
        for edge in &self.0 {
            writeln!(f, "  {edge}")?;
        }
        Ok(())
    }
}

impl core::error::Error for ForeignNativeIds {}

/// Whether a vocabulary is one of TPT's slug-addressed tag axes.
///
/// A TPT target the registry binds no `taxonomyTags` axis for is exempt, and
/// today the exemption is empty: TPT binds four axes and binds all four to
/// that one field, so every TPT vocabulary a projection can reach is a tag
/// axis. `categories` — the seller's own numeric shelves — is a native field
/// with no axis bound to it, so no edge can target it, which is why a numeric
/// identifier arriving on a TPT axis is foreign rather than a shelf.
#[must_use]
pub fn is_tpt_tag_vocabulary(vocabulary: VocabularyId) -> bool {
    let VocabularyId(inventory, kind) = vocabulary;
    inventory == InventoryId::Tpt
        && registry(inventory)
            .axis(kind)
            .is_some_and(|binding| binding.native == TPT_TAG_FIELD)
}

/// Checks every edge's target identifier against the shape its target
/// vocabulary issues, before any of them is written.
///
/// Pure, and total over the edge list: the caller runs it on whatever a
/// derivation produced and seeds nothing if it fails.
pub fn check_native_ids(edges: &[ProjectionEdge]) -> Result<(), ForeignNativeIds> {
    let foreign: Vec<ForeignNativeId> = edges
        .iter()
        .filter(|edge| is_tpt_tag_vocabulary(edge.to.vocabulary))
        .filter(|edge| !edge.to.native_id.as_deref().is_some_and(is_tpt_tag_slug))
        .map(|edge| ForeignNativeId {
            term: edge.from,
            target: edge.to.vocabulary,
            native_id: edge.to.native_id.clone(),
        })
        .collect();
    if foreign.is_empty() {
        Ok(())
    } else {
        Err(ForeignNativeIds(foreign))
    }
}

#[cfg(test)]
mod tests {
    use super::{check_native_ids, is_tpt_tag_vocabulary};
    use tam_domain::{Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath};
    use tam_types::{CanonicalTermId, InventoryId, Timestamp, Uuid};

    const TPT: &str = include_str!("../../../docs/design/data/tpt-vocabulary.json");
    const TES: &str = include_str!("../../../docs/design/data/tes-vocabulary.json");

    fn edge(vocabulary: VocabularyId, native: Option<&str>) -> ProjectionEdge {
        ProjectionEdge {
            from: CanonicalTermId(Uuid([7; 16])),
            to: VocabularyPath {
                vocabulary,
                segments: vec!["Algebra".to_owned()],
                native_id: native.map(str::to_owned),
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Imported {
                source: "a fixture".to_owned(),
            },
            decided_at: Timestamp(1),
        }
    }

    /// The founder-designated hole this module closes: a Tes-to-TPT subject
    /// edge seeded against the numeric half of the source vocabulary would
    /// reach a live listing as a tag TPT never issued.
    #[test]
    fn a_numeric_identifier_into_a_tpt_tag_axis_fails_the_check_and_names_the_edge() {
        let poisoned = edge(
            VocabularyId(InventoryId::Tpt, TermKind::Subject),
            Some("1000448"),
        );
        let Err(foreign) = check_native_ids(&[poisoned]) else {
            panic!("a numeric identifier is not a TPT tag slug");
        };
        let report = foreign.to_string();
        assert!(
            report.contains("1000448")
                && report.contains("Subject")
                && report.contains("Tpt")
                && report.contains(&CanonicalTermId(Uuid([7; 16])).0.to_hyphenated()),
            "the report names the offending id, the target axis and the source term, \
             got {report}"
        );
    }

    #[test]
    fn an_edge_carrying_no_identifier_at_all_fails_the_same_check() {
        let unaddressed = edge(VocabularyId(InventoryId::Tpt, TermKind::Topic), None);
        let Err(foreign) = check_native_ids(&[unaddressed]) else {
            panic!("there is nothing to post in place of an identifier");
        };
        assert!(
            foreign.to_string().contains("no identifier at all"),
            "the report says the edge is unaddressed, got {foreign}"
        );
    }

    /// Tes addresses everything it binds by number, so its own edges are not
    /// this check's business and must not be caught by it.
    #[test]
    fn an_edge_into_another_marketplace_is_left_alone() {
        let tes = edge(
            VocabularyId(InventoryId::TesNz, TermKind::Subject),
            Some("1000448"),
        );
        assert!(
            check_native_ids(&[tes]).is_ok(),
            "a numeric Tes category id is exactly what Tes issues"
        );
    }

    /// The exemption stated in [`is_tpt_tag_vocabulary`] is empty rather than
    /// a bypass: every axis TPT binds lands in the one slug namespace.
    #[test]
    fn every_axis_tpt_binds_is_a_tag_axis_so_nothing_is_exempt() {
        for axis in TermKind::ALL {
            let bound = tam_domain::registry::registry(InventoryId::Tpt)
                .axis(axis)
                .is_some();
            assert_eq!(
                bound,
                is_tpt_tag_vocabulary(VocabularyId(InventoryId::Tpt, axis)),
                "{axis:?} is bound by TPT but does not land in its tag namespace, so it needs \
                 a grammar of its own rather than this check's silence"
            );
        }
    }

    /// The committed vocabularies, seeded as the operator seeds them. Every
    /// TPT edge the derivation emits is a tag-axis edge addressed by a slug,
    /// so the real seed passes without exercising the exemption.
    #[test]
    fn the_committed_tpt_vocabulary_seeds_clean() {
        let grades = crate::grades::derive_grade_crosswalk(TPT, TES, Timestamp(1))
            .expect("the committed captures derive");
        let into_tpt = grades
            .edges
            .iter()
            .filter(|edge| edge.to.vocabulary.0 == InventoryId::Tpt)
            .count();
        assert_eq!(
            into_tpt, 32,
            "nineteen grade options claimed once each, and the thirteen band coverings"
        );
        assert!(
            grades.edges.iter().all(|edge| {
                edge.to.vocabulary.0 != InventoryId::Tpt
                    || is_tpt_tag_vocabulary(edge.to.vocabulary)
            }),
            "every TPT edge the derivation emits is a tag-axis edge"
        );
        if let Err(foreign) = check_native_ids(&grades.edges) {
            panic!("the committed grade crosswalk seeds clean, got {foreign}");
        }
    }

    /// The other two derivations the seeder runs, for completeness: neither
    /// targets TPT, and neither may start doing so without this check seeing
    /// it.
    #[test]
    fn the_licence_crosswalk_seeds_clean() {
        let licences = crate::licences::derive_licence_crosswalk(TES, Timestamp(1))
            .expect("the committed capture derives");
        if let Err(foreign) = check_native_ids(&licences.edges) {
            panic!("the licence crosswalk names no TPT target, got {foreign}");
        }
    }
}
