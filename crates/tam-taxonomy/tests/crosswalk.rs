//! The derivation over the real captured Tes tree.
//!
//! The capture is a finite domain, so the totality and round-trip property
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

const TES_CAPTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/data/tes-taxonomy-GB.json"
));

const CRAWLED_AT: Timestamp = Timestamp(1_787_000_000_000);

#[expect(
    clippy::expect_used,
    reason = "a capture that will not parse is a broken fixture"
)]
fn crosswalk() -> Crosswalk {
    let tes = parse_tree(TES_CAPTURE).expect("the Tes capture parses");
    assert_eq!(tes.subjects.len(), 43, "the capture carries 43 subjects");
    derive_crosswalk(&tes, CRAWLED_AT)
}

#[test]
fn the_contested_path_is_withdrawn() {
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
            ("1001347", "Behaviour and classroom management"),
        ],
        "Tes carries two distinct Whole school topics under one description, so \
         neither may claim the path and the queue decides"
    );
}

#[test]
fn the_derivation_seeds_the_pinned_counts() {
    let derived = crosswalk();
    assert_eq!(derived.terms.len(), 496, "43 subjects and their 453 topics");
    assert_eq!(
        derived.edges.len(),
        494,
        "every seeded term but the two contesting one path carries its edge"
    );
    assert!(
        derived
            .edges
            .iter()
            .all(|edge| edge.to.vocabulary.0 == InventoryId::Tes),
        "one Tes inventory, so every edge lands in it"
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
fn the_derivation_seeds_both_kinds() {
    let derived = crosswalk();
    let subjects = derived
        .terms
        .iter()
        .filter(|term| term.kind == TermKind::Subject)
        .count();
    assert_eq!(subjects, 43, "one canonical subject per captured subject");
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
