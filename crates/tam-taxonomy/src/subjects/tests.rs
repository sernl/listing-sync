//! The subject inversion's tests, kept beside it as a child module so the
//! derivation itself stays readable.
//!
//! Every test here runs over the three committed captures and the authored
//! pairing, with no database, because the whole derivation is pure. The
//! counterparts in `tests/crosswalk.rs` are left in place rather than edited:
//! both directions stay asserted while the re-seed is in flight.

use std::collections::{BTreeMap, BTreeSet};

use super::{derive_subject_crosswalk, tpt_term_id, SubjectCrosswalk, SubjectMismatchReason};
use crate::project::{ingest, project};
use crate::tes::{canonical_id, derive_crosswalk, parse_tree, TesTree};
use tam_domain::{EdgeKind, TermKind, TermProjection, VocabularyId};
use tam_types::{CanonicalTermId, InventoryId, Timestamp};

/// `InventoryId` and `TermKind` are not `Ord`, so the sets below key on a
/// grouping rank rather than on the enums themselves. Nothing durable is
/// encoded here; the ordinal that must never renumber lives in
/// `tam-marketplace`.
const fn rank(vocabulary: VocabularyId) -> (u8, u8) {
    let inventory = match vocabulary.0 {
        InventoryId::Tes => 0,
        InventoryId::Etsy => 1,
        InventoryId::Tpt => 2,
    };
    let kind = match vocabulary.1 {
        TermKind::Subject => 0,
        TermKind::Topic => 1,
        TermKind::ResourceType => 2,
        TermKind::Phase => 3,
        TermKind::Licence => 4,
    };
    (inventory, kind)
}

const TES: &str = include_str!("../../../../docs/design/data/tes-taxonomy-GB.json");
const TPT: &str = include_str!("../../../../docs/design/data/tpt-vocabulary.json");
const PAIRS: &str = include_str!("../../../../docs/design/data/tpt-tes-subject-pairs.json");
const AT: Timestamp = Timestamp(1_787_000_000_000);

fn tree() -> TesTree {
    parse_tree(TES).expect("the Tes capture parses")
}

fn crosswalk() -> SubjectCrosswalk {
    derive_subject_crosswalk(&tree(), TPT, PAIRS, AT).expect("the committed captures derive")
}

fn tpt_edges(derived: &SubjectCrosswalk) -> Vec<&tam_domain::ProjectionEdge> {
    derived
        .edges
        .iter()
        .filter(|edge| edge.to.vocabulary.0 == InventoryId::Tpt)
        .collect()
}

/// The kill gate. The re-derivation may not lose a mapping the current
/// direction holds, and the test is severe because it exhausts the finite
/// domain rather than sampling it: every edge the Tes-to-Tes derivation
/// emits must reappear, on the same term, at the same path, with a kind no
/// weaker. It fails under any implementation that drops a pairing, renumbers
/// a canonical id, or narrows an edge.
#[test]
fn no_edge_the_tes_derivation_holds_is_lost_by_the_inversion() {
    let base = derive_crosswalk(&tree(), AT);
    let derived = crosswalk();
    let held: BTreeSet<([u8; 16], (u8, u8), Vec<String>, bool)> = derived
        .edges
        .iter()
        .map(|edge| {
            (
                edge.from.0 .0,
                rank(edge.to.vocabulary),
                edge.to.segments.clone(),
                edge.kind == EdgeKind::Exact,
            )
        })
        .collect();
    assert!(!base.edges.is_empty(), "the base derivation emits edges");
    for edge in &base.edges {
        assert_eq!(
            edge.kind,
            EdgeKind::Exact,
            "the base derivation emits Exact edges alone, so `no weaker` means Exact"
        );
        assert!(
            held.contains(&(
                edge.from.0 .0,
                rank(edge.to.vocabulary),
                edge.to.segments.clone(),
                true,
            )),
            "the inversion drops or weakens the edge onto {:?} in {:?}",
            edge.to.segments,
            edge.to.vocabulary.0
        );
    }
}

/// Identity is the one thing the inversion may not move: `canonical_term.id`
/// is referenced from `projection_edge`, `projection_no_counterpart` and
/// `reconciliation_item`, and from every stored product.
#[test]
fn every_existing_canonical_id_survives_unchanged() {
    let base = derive_crosswalk(&tree(), AT);
    let derived = crosswalk();
    let after: BTreeMap<[u8; 16], (TermKind, Option<CanonicalTermId>)> = derived
        .terms
        .iter()
        .map(|term| (term.id.0 .0, (term.kind, term.parent)))
        .collect();
    for term in &base.terms {
        let Some(&(kind, parent)) = after.get(&term.id.0 .0) else {
            panic!("the inversion drops the canonical term for {}", term.label);
        };
        assert_eq!(kind, term.kind, "a term's kind moved: {}", term.label);
        assert_eq!(parent, term.parent, "a term's parent moved: {}", term.label);
    }
}

/// A term's label is the one thing the inversion does move, and only where a
/// facet denotes it exactly. `Cross-curricular topics` becomes `For All
/// Subjects` because TPT is the base and the re-seed updates the label in
/// place.
#[test]
fn label_authority_flips_on_an_exactly_denoted_term() {
    let derived = crosswalk();
    let term = derived
        .terms
        .iter()
        .find(|term| term.id == canonical_id(1_000_880))
        .expect("Cross-curricular topics seeds a term");
    assert_eq!(term.label, "For All Subjects");
    let untouched = derived
        .terms
        .iter()
        .find(|term| term.id == canonical_id(1_000_896))
        .expect("Mathematics seeds a term");
    assert_eq!(
        untouched.label, "Mathematics",
        "a term no facet denotes exactly keeps its Tes label"
    );
}

/// Every root either reaches the TPT vocabulary or is recorded as reaching
/// nothing, and never both. Mirrors `every_tpt_grade_is_paired_or_recorded_
/// absent_and_never_both`.
#[test]
fn every_tpt_subject_root_pairs_or_is_recorded_absent() {
    let derived = crosswalk();
    let reached: BTreeSet<&str> = tpt_edges(&derived)
        .iter()
        .filter_map(|edge| edge.to.native_id.as_deref())
        .collect();
    let absent: BTreeSet<&str> = derived
        .residue
        .tpt_only
        .iter()
        .map(|node| node.native_id.as_str())
        .chain(
            derived
                .residue
                .skipped_hidden
                .iter()
                .map(|node| node.native_id.as_str()),
        )
        .collect();
    for slug in [
        "art",
        "math",
        "science",
        "social-studies",
        "world-languages",
    ] {
        assert!(reached.contains(slug), "{slug} reaches no TPT path");
    }
    for slug in ["arts", "phonics", "trigonometry"] {
        assert!(
            !reached.contains(slug),
            "{slug} is hidden and must seed no edge"
        );
        assert!(absent.contains(slug), "{slug} is not recorded as absent");
    }
}

/// Ruling C, stated as a property rather than as a spot check: a hidden facet
/// seeds neither a term nor an edge, so its slug ingests to nothing and is
/// retained verbatim on the product instead.
#[test]
fn a_hidden_facet_seeds_neither_a_term_nor_an_edge() {
    let derived = crosswalk();
    assert_eq!(
        derived.residue.skipped_hidden.len(),
        7,
        "the capture carries seven hidden PreK-12-Subject-Area facets"
    );
    for hidden in &derived.residue.skipped_hidden {
        let id = tpt_term_id(&hidden.native_id);
        assert!(
            !derived.terms.iter().any(|term| term.id == id),
            "{} seeds a term",
            hidden.native_id
        );
        assert!(
            !derived
                .edges
                .iter()
                .any(|edge| edge.to.native_id.as_deref() == Some(hidden.native_id.as_str())),
            "{} seeds an edge",
            hidden.native_id
        );
    }
}

/// The withdrawal rule, which exists because a term holding an `Exact` and a
/// `Broader` edge onto two different TPT paths projects `Ambiguous` and
/// blocks the publish. Tes `Biology` is denoted exactly by the TPT child
/// `biology`, so the root `science` may not also claim it.
#[test]
fn a_child_exact_claim_withdraws_its_roots_broader_claim() {
    let derived = crosswalk();
    let withdrawn: BTreeSet<&str> = derived
        .residue
        .mismatched
        .iter()
        .filter(|mismatch| mismatch.reason == SubjectMismatchReason::WithdrawnByChild)
        .map(|mismatch| mismatch.node.native_id.as_str())
        .collect();
    for node in ["1000993", "1001002", "1001141", "1000792", "1001202"] {
        assert!(withdrawn.contains(node), "{node} is not withdrawn");
    }
    let biology = canonical_id(1_000_993);
    let into_tpt: Vec<_> = derived
        .edges
        .iter()
        .filter(|edge| edge.from == biology && edge.to.vocabulary.0 == InventoryId::Tpt)
        .collect();
    assert_eq!(
        into_tpt.len(),
        1,
        "Biology holds one edge into TPT, not one Exact and one Broader"
    );
    assert_eq!(into_tpt[0].kind, EdgeKind::Exact);
    assert_eq!(into_tpt[0].to.native_id.as_deref(), Some("biology"));
}

/// The round-trip law, re-run in the inverted direction. An `Exact` edge
/// ingests back to the term it came from and projects forward onto the same
/// path; a `Broader` edge broadens rather than resolving, which is the whole
/// difference between a legal many-to-one and an illegal one.
#[test]
fn every_derived_edge_round_trips() {
    let derived = crosswalk();
    for edge in &derived.edges {
        match edge.kind {
            EdgeKind::Exact => {
                assert_eq!(
                    ingest(&edge.to, &derived.edges),
                    Some(edge.from),
                    "the reverse of the exact relation is a function: {:?}",
                    edge.to.segments
                );
                assert_eq!(
                    project(edge.from, edge.to.vocabulary, &derived.edges),
                    TermProjection::Exact {
                        to: edge.to.clone()
                    },
                    "the forward projection names the same path: {:?}",
                    edge.to.segments
                );
            }
            EdgeKind::Broader => {
                assert_eq!(
                    project(edge.from, edge.to.vocabulary, &derived.edges),
                    TermProjection::Broadened {
                        to: edge.to.clone(),
                        dropped: vec![edge.from]
                    },
                    "a broadening resolves and discloses rather than blocking: {:?}",
                    edge.to.segments
                );
            }
            EdgeKind::Narrower => panic!("the derivation emits no Narrower edge"),
        }
    }
}

/// The single-valued index rejects the seed otherwise: a term may hold at
/// most one projecting edge of each kind into each vocabulary.
#[test]
fn one_term_holds_at_most_one_projecting_edge_into_each_vocabulary() {
    let derived = crosswalk();
    let mut seen: BTreeSet<([u8; 16], (u8, u8), bool)> = BTreeSet::new();
    for edge in &derived.edges {
        let key = (
            edge.from.0 .0,
            rank(edge.to.vocabulary),
            edge.kind == EdgeKind::Exact,
        );
        assert!(
            seen.insert(key),
            "two edges of one kind leave the same term for {:?}",
            edge.to.vocabulary.0
        );
    }
}

/// No canonical term projects `Ambiguous` into any vocabulary the relation
/// seeds, which is what the withdrawal rule and the contest guard exist to
/// hold.
#[test]
fn the_seeded_subject_relation_holds_no_ambiguity() {
    let derived = crosswalk();
    let mut vocabularies: Vec<VocabularyId> = Vec::new();
    for edge in &derived.edges {
        if !vocabularies.contains(&edge.to.vocabulary) {
            vocabularies.push(edge.to.vocabulary);
        }
    }
    for term in &derived.terms {
        for &vocabulary in &vocabularies {
            let projection = project(term.id, vocabulary, &derived.edges);
            assert!(
                !matches!(projection, TermProjection::Ambiguous { .. }),
                "{} projects Ambiguous into {:?}",
                term.label,
                vocabulary.0
            );
        }
    }
}

/// The insert order law, which matters more after the inversion than before
/// it, because a minted TPT child now hangs from a minted TPT root.
#[test]
fn the_terms_insert_before_their_parents_do_not() {
    let derived = crosswalk();
    let mut seen: BTreeSet<[u8; 16]> = BTreeSet::new();
    for term in &derived.terms {
        if let Some(parent) = term.parent {
            assert!(
                seen.contains(&parent.0 .0),
                "the parent of {} sorts after it, so one insert pass breaks the foreign key",
                term.label
            );
        }
        seen.insert(term.id.0 .0);
    }
}

/// The reverse-`Exact` index admits one claimant per target path, and the
/// seed is rejected at insert otherwise.
#[test]
fn no_two_terms_claim_one_target_path_as_exact() {
    let derived = crosswalk();
    let mut claims: BTreeMap<((u8, u8), Vec<String>), [u8; 16]> = BTreeMap::new();
    for edge in &derived.edges {
        if edge.kind != EdgeKind::Exact {
            continue;
        }
        let key = (rank(edge.to.vocabulary), edge.to.segments.clone());
        if let Some(first) = claims.insert(key, edge.from.0 .0) {
            assert_eq!(
                first, edge.from.0 .0,
                "two terms claim {:?} as Exact",
                edge.to.segments
            );
        }
    }
}

/// The figures the three captures and the authored pairing state, pinned
/// rather than recomputed. A capture that drifts must fail here and be
/// re-measured, which is what the drift job exists to notice first.
#[test]
fn the_derivation_seeds_the_pinned_counts() {
    let derived = crosswalk();
    assert_eq!(
        derived.terms.len(),
        601,
        "496 Tes terms, plus one for each of the 133 writable facets that denotes no Tes node \
         exactly: 28 of the 133 reuse a term rather than minting one"
    );
    let subjects = derived
        .terms
        .iter()
        .filter(|term| term.kind == TermKind::Subject)
        .count();
    assert_eq!(
        subjects, 52,
        "43 Tes subjects and the 9 roots that mint a subject-kind term; `health` and \
         `speaking-and-listening` mint topic-kind terms because Tes carries them as topics"
    );
    let into_tpt = tpt_edges(&derived);
    assert_eq!(
        into_tpt.len(),
        158,
        "one Exact edge for each of the 133 writable facets, and 25 Broader edges from the \
         Tes side"
    );
    assert_eq!(
        into_tpt
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Exact)
            .count(),
        133,
        "140 subject-area facets less the 7 hidden ones, each claiming its own path once"
    );
    assert_eq!(
        derived.no_counterparts.len(),
        443,
        "14 of the 43 Tes subjects and 429 of the 453 Tes topics reach no facet"
    );
    assert_eq!(
        derived.residue.tpt_only.len(),
        95,
        "93 children and the roots `social-emotional` and `performing-arts` reach no Tes node"
    );
    assert_eq!(
        derived.residue.mismatched.len(),
        8,
        "the eight subjects a child denotes exactly, withdrawn from their roots"
    );
    assert_eq!(
        derived.edges.len(),
        653,
        "494 from the Tes half, 158 into TPT, and the one Tes path `not-subject-specific` \
         reaches through Broader"
    );
}
