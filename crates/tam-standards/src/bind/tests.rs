//! The join's tests.
//!
//! Every fixture here is a recorded shape rather than a captured response: no
//! HAR of TPT's education-standards tree exists in this repository, so the node
//! shapes come from the selection sets in
//! `docs/research/rethink/tpt-product-model.md`, while the codes, alt codes and
//! statements are the committed ingest's own.

use super::*;
use crate::crawl::tpt_jurisdiction;

const ANCHOR: &str = "Demonstrate command of the conventions of standard English grammar and usage when writing or speaking.";
const MATHS: &str = "apply mathematics to problems arising in everyday life;";
const SCIENCE: &str = "ask questions and define problems based on observations;";
const AT: &str = "2026-10-03T00:00:00Z";

fn scope(framework: Framework) -> JurisdictionScope {
    JurisdictionScope {
        framework,
        root: tpt_jurisdiction(framework),
    }
}

fn under(root: TptNodeId, id: u64, name: &str, statement: &str) -> CrawledStandard {
    CrawledStandard {
        tpt_node_id: TptNodeId(id),
        name: name.to_owned(),
        statement: statement.to_owned(),
        ancestry: vec![root],
    }
}

fn row<'a>(
    source_guid: &'a str,
    subject: &'a str,
    code: &'a str,
    alt_code: Option<&'a str>,
    statement: &'a str,
) -> CatalogueRow<'a> {
    CatalogueRow {
        source_guid,
        subject,
        code,
        alt_code,
        statement,
    }
}

/// Code correspondence has one definition and the join reads it through the
/// index, so this asserts it there rather than through a predicate beside it.
#[test]
fn a_namespace_prefix_is_dropped_and_a_code_level_is_not() {
    let catalogue = CatalogueIndex::build(vec![
        row(
            "anchor",
            "English Language Arts",
            "CCSS.ELA-Literacy.CCRA.L.1",
            Some("CCR.L.1"),
            ANCHOR,
        ),
        row("texas", "Mathematics (2012-)", "5.10.A", None, MATHS),
        row("virginia", "Science (2018-)", "5.3.B", None, SCIENCE),
    ]);
    let reached = |name: &str| {
        catalogue
            .candidates(name)
            .into_iter()
            .map(|found| found.source_guid)
            .collect::<Vec<&str>>()
    };
    assert_eq!(
        reached("CCRA.L.1"),
        vec!["anchor"],
        "TPT names the node without the mirror's namespace"
    );
    assert_eq!(
        reached("CCR.L.1"),
        vec!["anchor"],
        "and the mirror's own bare form is a name the row answers to, which is the \
         alt_code the removed predicate was blind to"
    );
    assert_eq!(
        reached("5.3B"),
        vec!["virginia"],
        "the fold is the equivalence a teacher's typing already gets"
    );
    assert!(
        reached("10.A").is_empty(),
        "dropping a numbered level would bind one standard to another's node"
    );
    assert!(reached("").is_empty(), "an unnamed node names no standard");
}

/// F7's shape: an unrelated row carries the name exactly and disagrees, while
/// the row that corresponds by suffix agrees. Offering only the exact tier
/// reports the node as disagreeing with a statement it was never shown.
#[test]
fn an_exact_hit_that_disagrees_does_not_hide_a_suffix_hit_that_agrees() {
    let catalogue = CatalogueIndex::build(vec![
        row("unrelated", "Mathematics", "CCRA.L.1", None, MATHS),
        row(
            "anchor",
            "English Language Arts",
            "CCSS.ELA-Literacy.CCRA.L.1",
            Some("CCR.L.1"),
            ANCHOR,
        ),
    ]);
    let scope = scope(Framework::Ccss);
    let outcome = bind_walk(
        &scope,
        &[under(scope.root, 9002, "CCRA.L.1", ANCHOR)],
        &catalogue,
        AT,
    );
    assert!(
        outcome.residue.is_empty(),
        "the row that agrees was offered, so nothing is refused, got {:?}",
        outcome.residue
    );
    assert_eq!(
        outcome
            .bindings
            .iter()
            .map(|binding| binding.source_guid.as_str())
            .collect::<Vec<&str>>(),
        vec!["anchor"],
        "the statement discriminates, and the unrelated row that carries the name exactly \
         simply does not agree"
    );
}

#[test]
fn an_anchor_standard_in_two_sets_binds_both_rows_to_one_node() {
    let catalogue = CatalogueIndex::build(vec![
        row(
            "grade-4",
            "English Language Arts",
            "CCSS.ELA-Literacy.CCRA.L.1",
            Some("CCR.L.1"),
            ANCHOR,
        ),
        row(
            "grade-5",
            "English Language Arts",
            "CCSS.ELA-Literacy.CCRA.L.1",
            Some("CCR.L.1"),
            ANCHOR,
        ),
    ]);
    let scope = scope(Framework::Ccss);
    let outcome = bind_walk(
        &scope,
        &[under(scope.root, 9002, "CCRA.L.1", ANCHOR)],
        &catalogue,
        AT,
    );
    assert!(
        outcome.residue.is_empty(),
        "both rows resolved, got {:?}",
        outcome.residue
    );
    assert_eq!(
        outcome
            .bindings
            .iter()
            .map(|binding| binding.source_guid.as_str())
            .collect::<Vec<&str>>(),
        vec!["grade-4", "grade-5"],
        "the mirror repeats an anchor standard per grade set and each row is tagged separately"
    );
    assert!(
        outcome
            .bindings
            .iter()
            .all(|binding| binding.tpt_node_id == TptNodeId(9002) && binding.tpt_name == "CCRA.L.1"),
        "one node, and the name TPT itself used is what the identity check will read"
    );
}

#[test]
fn a_row_two_nodes_both_reach_is_bound_by_neither() {
    let catalogue = CatalogueIndex::build(vec![row(
        "grade-4",
        "English Language Arts",
        "CCSS.ELA-Literacy.CCRA.L.1",
        Some("CCR.L.1"),
        ANCHOR,
    )]);
    let scope = scope(Framework::Ccss);
    let outcome = bind_walk(
        &scope,
        &[
            under(scope.root, 9002, "CCRA.L.1", ANCHOR),
            under(scope.root, 9003, "CCR.L.1", ANCHOR),
        ],
        &catalogue,
        AT,
    );
    assert!(
        outcome.bindings.is_empty(),
        "a row bound twice has no answer to which id to post, got {:?}",
        outcome.bindings
    );
    assert_eq!(
        outcome.residue.len(),
        2,
        "both claimants are named rather than one of them silently winning"
    );
    assert_eq!(
        outcome.residue[0].reason,
        Unresolved::RowClaimedTwice {
            source_guid: "grade-4".to_owned(),
            others: vec![TptNodeId(9003)],
        },
        "each claimant's residue names the row and the other claimant"
    );
}

#[test]
fn a_node_from_another_jurisdictions_tree_is_refused_before_its_code_is_read() {
    let catalogue = CatalogueIndex::build(vec![row(
        "grade-4",
        "English Language Arts",
        "CCSS.ELA-Literacy.CCRA.L.1",
        Some("CCR.L.1"),
        ANCHOR,
    )]);
    let scope = scope(Framework::Ccss);
    let elsewhere = CrawledStandard {
        tpt_node_id: TptNodeId(9002),
        // A name no row in this index carries, so the two orderings give
        // different reasons and the assertion can tell them apart.
        name: "9.9.Z".to_owned(),
        statement: ANCHOR.to_owned(),
        ancestry: vec![tpt_jurisdiction(Framework::Teks)],
    };
    let outcome = bind_walk(&scope, &[elsewhere], &catalogue, AT);
    assert!(
        outcome.bindings.is_empty(),
        "a node under another jurisdiction's tree is not this jurisdiction's to bind"
    );
    assert_eq!(
        outcome.residue[0].reason,
        Unresolved::OutsideJurisdiction,
        "the ancestry is read before the code: reading the code first would report that no \
         row carries it, which says nothing about the jurisdiction"
    );
}

#[test]
fn two_subjects_sharing_a_code_bind_only_the_row_whose_statement_agrees() {
    let catalogue = CatalogueIndex::build(vec![
        row("maths-row", "Mathematics (2012-)", "1.1.A", None, MATHS),
        row("science-row", "Science (2020-)", "1.1.A", None, SCIENCE),
    ]);
    let scope = scope(Framework::Teks);
    let outcome = bind_walk(
        &scope,
        &[under(scope.root, 7001, "1.1.A", SCIENCE)],
        &catalogue,
        AT,
    );
    assert_eq!(
        outcome
            .bindings
            .iter()
            .map(|binding| binding.source_guid.as_str())
            .collect::<Vec<&str>>(),
        vec!["science-row"],
        "697 of 4,872 Texas codes name a different standard under a different subject"
    );
    assert!(
        outcome.residue.is_empty(),
        "the maths row is simply not this node, not a residue item"
    );
}

#[test]
fn a_node_no_row_carries_and_a_node_no_row_agrees_with_are_both_residue() {
    let catalogue = CatalogueIndex::build(vec![row(
        "maths-row",
        "Mathematics (2012-)",
        "1.1.A",
        None,
        MATHS,
    )]);
    let scope = scope(Framework::Teks);
    let outcome = bind_walk(
        &scope,
        &[
            under(
                scope.root,
                7001,
                "9.9.Z",
                "a standard the mirror does not carry",
            ),
            under(
                scope.root,
                7002,
                "1.1.A",
                "a statement neither side transcribed alike",
            ),
        ],
        &catalogue,
        AT,
    );
    assert!(
        outcome.bindings.is_empty(),
        "neither node resolved, got {:?}",
        outcome.bindings
    );
    assert_eq!(
        outcome
            .residue
            .iter()
            .map(|item| item.reason.clone())
            .collect::<Vec<Unresolved>>(),
        vec![
            Unresolved::NoCatalogueRow,
            Unresolved::StatementDisagrees { candidates: 1 },
        ],
        "the two refusals are told apart, because they need different answers"
    );
}

#[test]
fn a_row_whose_code_and_alt_code_fold_alike_is_bound_once() {
    let catalogue = CatalogueIndex::build(vec![row(
        "one-row",
        "Mathematics (2012-)",
        "1.1.A",
        Some("1.1.A"),
        MATHS,
    )]);
    assert_eq!(
        catalogue.candidates("1.1.A").len(),
        1,
        "one row is one candidate however many of its own forms carry the same code"
    );
    let scope = scope(Framework::Teks);
    let outcome = bind_walk(
        &scope,
        &[under(scope.root, 7001, "1.1.A", MATHS)],
        &catalogue,
        AT,
    );
    assert_eq!(
        outcome.bindings.len(),
        1,
        "a row bound twice writes a line load_tpt_node_ids refuses, so the index must never \
         offer one row twice under one key"
    );
}

#[test]
fn a_code_whose_segment_is_punctuation_is_bound_once() {
    let catalogue = CatalogueIndex::build(vec![row(
        "D943516F35F44030BE63383A6FD0BF81",
        "Science",
        "ESS.!.A",
        None,
        ANCHOR,
    )]);
    assert_eq!(
        catalogue.candidates("A").len(),
        1,
        "the committed NGSS row coded `ESS.!.A` has the suffixes `!.A` and `A`, and the fold \
         drops the punctuation segment so both are the key `A`"
    );
    let scope = scope(Framework::Ngss);
    let outcome = bind_walk(
        &scope,
        &[under(scope.root, 5001, "A", ANCHOR)],
        &catalogue,
        AT,
    );
    assert_eq!(
        outcome.bindings.len(),
        1,
        "the second route to one row twice, and the same refused file if it survived"
    );
}
