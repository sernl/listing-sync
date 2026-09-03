//! The crawl's tests: what a capture records, and what a second capture
//! decides.
//!
//! Every fixture here is a recorded shape rather than a captured response: no
//! HAR of TPT's education-standards tree exists in this repository, so the node
//! shapes come from the selection sets in
//! `docs/research/rethink/tpt-product-model.md`, and the codes and statements
//! come from the committed ingest.

use super::*;
use crate::tpt::load_tpt_node_ids;

const ANCHOR: &str = "Demonstrate command of the conventions of standard English grammar and usage when writing or speaking.";
const AT: &str = "2026-10-03T00:00:00Z";
const CCSS_ROOT: TptNodeId = TptNodeId(3054);

fn crawled(id: u64, name: &str, statement: &str) -> CrawledStandard {
    CrawledStandard {
        tpt_node_id: TptNodeId(id),
        name: name.to_owned(),
        statement: statement.to_owned(),
        ancestry: vec![CCSS_ROOT],
    }
}

#[test]
fn the_four_subtrees_are_the_roots_the_form_offers() {
    let roots: Vec<u64> = crate::model::FRAMEWORKS
        .into_iter()
        .map(|framework| tpt_jurisdiction(framework).0)
        .collect();
    assert_eq!(
        roots,
        vec![3054, 3055, 3326, 5785],
        "CCSS, NGSS, TEKS and VA SOL out of the 166 roots"
    );
}

#[test]
fn the_four_differences_two_transcriptions_share_are_absorbed_and_no_more() {
    let published =
        "Demonstrate command of the \u{201c}conventions\u{201d} \u{2014} of standard English.";
    let transcribed = "demonstrate  command of the \"conventions\"\n- of standard english";
    assert_eq!(
        normalise_statement(published),
        normalise_statement(transcribed),
        "whitespace, case, curly quotes, an em dash and a trailing stop are one statement"
    );
    assert_ne!(
        normalise_statement(ANCHOR),
        normalise_statement("Demonstrate command of the conventions."),
        "a shortened statement is a different statement"
    );
    assert_ne!(
        normalise_statement("apply mathematics to problems arising in everyday life;"),
        normalise_statement("apply mathematics to problems arising in everyday life"),
        "a trailing semicolon is the source's own punctuation and is not absorbed"
    );
}

#[test]
fn a_rewrapped_statement_hashes_the_same_and_a_reworded_one_does_not() {
    let wrapped = "Demonstrate command of the conventions\n   of standard English grammar and usage when writing or speaking.";
    assert_eq!(
        statement_hash(wrapped),
        statement_hash(ANCHOR),
        "a re-indent is not a reword"
    );
    assert_ne!(
        statement_hash("Demonstrate command of the conventions."),
        statement_hash(ANCHOR),
        "a shortened statement is a different statement"
    );
}

fn table(rows: &[String]) -> TptNodeIdTable {
    load_tpt_node_ids(&rows.join("\n")).expect("the fixture bindings load")
}

fn binding_line(guid: &str, id: u64, name: &str, statement: &str, at: &str) -> String {
    let binding = TptBinding {
        framework: Framework::Ccss,
        source_guid: guid.to_owned(),
        subject: "English Language Arts".to_owned(),
        code: "CCSS.ELA-Literacy.CCRA.L.1".to_owned(),
        tpt_node_id: TptNodeId(id),
        tpt_name: name.to_owned(),
        statement_sha256: statement_hash(statement),
        verified_at: at.to_owned(),
    };
    serde_json::to_string(&binding).expect("a binding serialises")
}

#[test]
fn a_capture_that_agrees_stores_and_posts() {
    let bound = table(&[binding_line("g1", 9002, "CCRA.L.1", ANCHOR, AT)]);
    let diff = diff_capture(&bound, &[crawled(9002, "CCRA.L.1", ANCHOR)]);
    assert_eq!(diff.agreed, 1);
    assert_eq!(gate(&diff), GateVerdict::StoreAndPost);
}

#[test]
fn a_reworded_statement_is_drift_rather_than_a_moved_id() {
    let bound = table(&[binding_line("g1", 9002, "CCRA.L.1", ANCHOR, AT)]);
    let diff = diff_capture(&bound, &[crawled(9002, "CCRA.L.1", "Reworded by TPT.")]);
    assert!(
        diff.moved.is_empty(),
        "the id still names the same standard"
    );
    assert_eq!(diff.drifted.len(), 1, "the statement changed under it");
    assert_eq!(
        gate(&diff),
        GateVerdict::StoreAndPost,
        "the decision rule reads the identity check alone"
    );
}

#[test]
fn one_moved_id_in_a_hundred_kills_stored_ids_and_one_in_two_hundred_does_not() {
    let mut hundred: Vec<String> = (0..99)
        .map(|index| {
            binding_line(
                &format!("agree-{index}"),
                9000 + index,
                "CCRA.L.1",
                ANCHOR,
                AT,
            )
        })
        .collect();
    hundred.push(binding_line("moved", 9999, "CCRA.L.1", ANCHOR, AT));
    let mut capture: Vec<CrawledStandard> = (0..99)
        .map(|index| crawled(9000 + index, "CCRA.L.1", ANCHOR))
        .collect();
    capture.push(crawled(9999, "RL.2.1", ANCHOR));

    let diff = diff_capture(&table(&hundred), &capture);
    assert_eq!(diff.moved.len(), 1);
    assert_eq!(diff.checked(), 100);
    assert_eq!(
        gate(&diff),
        GateVerdict::ResolveAtPostTime,
        "one in a hundred is the threshold, and the threshold kills"
    );

    let mut two_hundred = hundred.clone();
    let mut wider = capture.clone();
    for index in 100..200 {
        two_hundred.push(binding_line(
            &format!("agree-{index}"),
            9000 + index,
            "CCRA.L.1",
            ANCHOR,
            AT,
        ));
        wider.push(crawled(9000 + index, "CCRA.L.1", ANCHOR));
    }
    let wider_diff = diff_capture(&table(&two_hundred), &wider);
    assert_eq!(wider_diff.moved.len(), 1);
    assert_eq!(
        gate(&wider_diff),
        GateVerdict::StoreAndPostWithQuarterlyCapture,
        "one in two hundred stores and posts on a tightened cadence"
    );
}

#[test]
fn a_capture_that_answered_for_nothing_is_inconclusive() {
    let bound = table(&[binding_line("g1", 9002, "CCRA.L.1", ANCHOR, AT)]);
    let diff = diff_capture(&bound, &[]);
    assert_eq!(diff.absent, vec![TptNodeId(9002)]);
    assert_eq!(
        gate(&diff),
        GateVerdict::Inconclusive,
        "a walk that stopped short is not evidence that nothing moved"
    );
}

#[test]
fn the_window_admits_the_current_capture_and_refuses_an_older_binding() {
    let bound = table(&[
        binding_line("current", 9002, "CCRA.L.1", ANCHOR, AT),
        binding_line("stale", 9003, "CCRA.L.2", ANCHOR, "2026-09-03T00:00:00Z"),
    ]);
    let window = CrawlWindow::opened_at(AT).expect("a shaped timestamp opens a window");
    assert_eq!(postable(&bound, &window, "current"), Ok(TptNodeId(9002)));
    assert_eq!(
        postable(&bound, &window, "stale"),
        Err(NotPostable::OutsideCrawlWindow),
        "an id no current capture vouches for is not posted under any branch"
    );
    assert_eq!(
        postable(&bound, &window, "never-crawled"),
        Err(NotPostable::Unbound),
        "unbound and stale are different answers to a seller"
    );
}

#[test]
fn a_window_refuses_a_timestamp_whose_comparison_would_be_meaningless() {
    assert!(
        CrawlWindow::opened_at("2026-10-03").is_none(),
        "a date is not the fixed-width instant a byte comparison orders"
    );
    assert!(CrawlWindow::opened_at("2026-10-03T00:00:00+01:00").is_none());
}
