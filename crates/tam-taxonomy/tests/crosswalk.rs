//! The crosswalk over the real captured GB and NZ trees.
//!
//! The captures are a finite domain, so the totality and round-trip property
//! the engineering charter wanted proven is discharged here by exhausting it:
//! every derived edge ingests back to the term it came from and projects
//! forward to the path it names. That is the trade the charter anticipated in
//! place of a Kani harness over an unbounded model.
//!
//! The counts below are pinned rather than recomputed. A capture that drifts
//! must fail this test and be re-measured, which is the point: the numbers
//! come from `docs/notes/probes/10-taxonomy-crosswalk.md` and the crawl it
//! describes.

use tam_domain::{TermKind, TermProjection};
use tam_taxonomy::tes::{Crosswalk, MismatchReason};
use tam_taxonomy::{derive_crosswalk, ingest, parse_tree, project};
use tam_types::{InventoryId, Timestamp};

const GB_CAPTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/data/tes-taxonomy-GB.json"
));
const NZ_CAPTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/data/tes-taxonomy-NZ.json"
));

const CRAWLED_AT: Timestamp = Timestamp(1_787_000_000_000);

#[expect(
    clippy::expect_used,
    reason = "a capture that will not parse or derive is a broken fixture"
)]
fn crosswalk() -> Crosswalk {
    let gb = parse_tree(GB_CAPTURE).expect("the GB capture parses");
    let nz = parse_tree(NZ_CAPTURE).expect("the NZ capture parses");
    assert_eq!(gb.subjects.len(), 43, "the GB capture carries 43 subjects");
    assert_eq!(nz.subjects.len(), 43, "the NZ capture carries 43 subjects");
    derive_crosswalk(&gb, &nz, CRAWLED_AT).expect("the captures derive")
}

fn markets_of(derived: &Crosswalk, term: tam_types::CanonicalTermId) -> Vec<InventoryId> {
    derived
        .edges
        .iter()
        .filter(|edge| edge.from == term)
        .map(|edge| edge.to.vocabulary.0)
        .collect()
}

#[test]
fn every_subject_pairs() {
    let derived = crosswalk();
    let subjects: Vec<_> = derived
        .terms
        .iter()
        .filter(|term| term.kind == TermKind::Subject)
        .collect();
    assert_eq!(
        subjects.len(),
        43,
        "every GB subject yields a market-neutral key and seeds a canonical term"
    );
    let paired = subjects
        .iter()
        .filter(|term| {
            markets_of(&derived, term.id) == vec![InventoryId::TesGb, InventoryId::TesNz]
        })
        .count();
    assert_eq!(
        paired, 43,
        "all 43 pair: zero suffix or description mismatches, as the probe measured"
    );
    assert!(
        derived
            .residue
            .mismatched
            .iter()
            .all(|entry| entry.node.kind != TermKind::Subject),
        "no subject-level defect survives, including the mapTo-less Languages"
    );
}

#[test]
fn the_topic_residue_is_the_measured_handful() {
    let derived = crosswalk();
    let gb_only: Vec<&str> = derived
        .residue
        .gb_only
        .iter()
        .map(|node| node.description.as_str())
        .collect();
    assert_eq!(
        gb_only,
        vec!["Cells", "Micro-organisms"],
        "the 453-versus-451 topic difference the probe measured, named"
    );
    assert!(
        derived.residue.nz_only.is_empty(),
        "NZ carries no topic GB lacks"
    );

    assert_eq!(
        derived.residue.mismatched.len(),
        4,
        "the only defect is the contested path, withdrawn from both markets"
    );
    assert!(
        derived
            .residue
            .mismatched
            .iter()
            .all(|entry| entry.reason == MismatchReason::DuplicatePath),
        "the prefix transform holds across every node: no mapTo disagreement, no \
         description mismatch, no parent mismatch, every id market-prefixed"
    );
}

#[test]
fn the_contested_path_is_withdrawn_from_both_markets() {
    let derived = crosswalk();
    let withdrawn: Vec<(&str, &str)> = derived
        .residue
        .mismatched
        .iter()
        .filter(|entry| entry.reason == MismatchReason::DuplicatePath)
        .map(|entry| {
            (
                entry.node.native_id.as_str(),
                entry.node.description.as_str(),
            )
        })
        .collect();
    assert_eq!(
        withdrawn,
        vec![
            ("1001345", "Behaviour and classroom management"),
            ("7001345", "Behaviour and classroom management"),
            ("1001347", "Behaviour and classroom management"),
            ("7001347", "Behaviour and classroom management"),
        ],
        "Tes carries two distinct Whole school topics under one description, so \
         neither may claim the path and the queue decides"
    );
}

#[test]
fn the_derivation_seeds_the_pinned_counts() {
    let derived = crosswalk();
    assert_eq!(
        derived.terms.len(),
        496,
        "43 GB subjects and their 453 topics"
    );
    assert_eq!(
        derived.edges.len(),
        986,
        "494 GB and 492 NZ: two GB-only topics keep the GB edge alone, and the \
         contested path withdraws both claimants from each market"
    );
    let gb_edges = derived
        .edges
        .iter()
        .filter(|edge| edge.to.vocabulary.0 == InventoryId::TesGb)
        .count();
    assert_eq!(
        gb_edges, 494,
        "every seeded term but the two contesting one path carries its GB edge"
    );
}

#[test]
fn every_derived_edge_round_trips() {
    let derived = crosswalk();
    for edge in &derived.edges {
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
}

#[test]
fn the_terms_insert_before_their_parents_do_not() {
    let derived = crosswalk();
    let mut seen = std::collections::BTreeSet::new();
    for term in &derived.terms {
        if let Some(parent) = term.parent {
            assert!(
                seen.contains(&parent.0 .0),
                "a topic's subject sorts before it, so one insert pass satisfies the foreign key"
            );
        }
        seen.insert(term.id.0 .0);
    }
}
