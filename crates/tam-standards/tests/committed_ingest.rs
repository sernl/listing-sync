//! The committed ingest, checked against the manifest that describes it.
//!
//! The files are compiled in rather than read, so the crate stays free of I/O
//! and the test cannot pass against a file that is not the committed one.
//!
//! Two kinds of assertion, and both earn their place. The manifest checks are
//! self-consistency: the bytes hash to what the manifest declares, the row
//! count matches, and no row names a set the manifest lacks. The count checks
//! are against the sources research's independently measured expectations, so
//! an ingest that silently lost half a framework fails here rather than in a
//! seller's picker.

use tam_standards::{
    load_framework, load_tpt_node_ids, parse_manifest, required_notices, Framework, GradeLevel,
    StandardsDocument, StandardsIndex, CCSS_COPYRIGHT_NOTICE, FRAMEWORKS,
};

const MANIFEST: &str = include_str!("../../../docs/design/data/standards/manifest.json");
const CCSS: &[u8] = include_bytes!("../../../docs/design/data/standards/ccss.jsonl");
const NGSS: &[u8] = include_bytes!("../../../docs/design/data/standards/ngss.jsonl");
const TEKS: &[u8] = include_bytes!("../../../docs/design/data/standards/teks.jsonl");
const VA_SOL: &[u8] = include_bytes!("../../../docs/design/data/standards/va-sol.jsonl");
const TPT_NODE_IDS: &str = include_str!("../../../docs/design/data/standards/tpt-node-ids.jsonl");

fn bytes_of(framework: Framework) -> &'static [u8] {
    match framework {
        Framework::Ccss => CCSS,
        Framework::Ngss => NGSS,
        Framework::Teks => TEKS,
        Framework::VaSol => VA_SOL,
    }
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a committed file that does not load must fail loudly"
)]
fn documents() -> Vec<StandardsDocument> {
    let manifest = parse_manifest(MANIFEST).expect("the committed manifest parses");
    FRAMEWORKS
        .into_iter()
        .map(|framework| {
            load_framework(&manifest, framework, bytes_of(framework))
                .unwrap_or_else(|error| panic!("{framework} did not load: {error}"))
        })
        .collect()
}

#[test]
fn every_committed_file_round_trips_against_the_manifest_hash() {
    let documents = documents();
    assert_eq!(documents.len(), 4, "all four frameworks are committed");
    for document in &documents {
        let manifest = &document.manifest;
        assert_eq!(
            document.nodes.len(),
            manifest.node_count,
            "{} row count",
            manifest.framework
        );
        assert_eq!(
            u64::try_from(bytes_of(manifest.framework).len()).expect("file size fits a u64"),
            manifest.file_bytes,
            "{} byte count",
            manifest.framework
        );
        assert!(
            !manifest.file_sha256.is_empty() && manifest.file_sha256.len() == 64,
            "{} carries a full SHA-256",
            manifest.framework
        );
        assert!(
            !manifest.sets.is_empty(),
            "{} names the sets its rows came from",
            manifest.framework
        );
    }
}

/// The counts the sources research measured independently, as inclusive
/// bounds. Deliberately tight: these are frozen or slow-moving corpora, and a
/// change large enough to leave these ranges is a change that must be read
/// rather than absorbed.
#[test]
fn the_addressable_counts_match_the_sources_research() {
    let expected = [
        (Framework::Ccss, 1_500_usize, 1_600_usize),
        (Framework::Ngss, 200, 220),
        (Framework::Teks, 3_800, 5_200),
        (Framework::VaSol, 3_900, 4_400),
    ];
    for document in documents() {
        let framework = document.framework();
        let distinct: std::collections::BTreeSet<&str> = document
            .addressable()
            .filter_map(|node| node.code.as_deref())
            .collect();
        let (_, low, high) = expected
            .iter()
            .find(|(named, _, _)| *named == framework)
            .copied()
            .expect("every framework has an expected range");
        assert!(
            (low..=high).contains(&distinct.len()),
            "{framework}: {} distinct addressable codes, expected {low}..={high}",
            distinct.len()
        );
    }
}

#[test]
fn every_addressable_row_carries_a_code_and_a_statement() {
    for document in documents() {
        let framework = document.framework();
        for node in document.addressable() {
            assert!(
                node.code.as_ref().is_some_and(|code| !code.is_empty()),
                "{framework}: an addressable row with no code cannot be posted ({})",
                node.source_guid
            );
            assert!(
                !node.statement.trim().is_empty(),
                "{framework}: addressable `{}` carries no statement",
                node.code.as_deref().unwrap_or("?")
            );
        }
    }
}

#[test]
fn every_row_joins_to_a_set_that_declares_its_provenance() {
    for document in documents() {
        let framework = document.framework();
        for node in &document.nodes {
            let set = document
                .set_of(node)
                .unwrap_or_else(|| panic!("{framework}: row {} names no set", node.source_guid));
            assert!(
                !set.source_url.is_empty(),
                "{framework}: set `{}` records no source URL",
                set.id
            );
        }
        assert!(
            !document.manifest.fetched_at.is_empty(),
            "{framework} records when it was fetched"
        );
        assert_eq!(
            document.manifest.source, "commonstandardsproject",
            "{framework} records which mirror it came from"
        );
    }
}

#[test]
fn a_teacher_typing_a_bare_code_finds_the_published_one() {
    let documents = documents();
    let index = StandardsIndex::build(&documents);

    let found = index.by_code_prefix("RL.2.1");
    assert!(
        found
            .iter()
            .any(|hit| hit.node.code.as_deref() == Some("CCSS.ELA-Literacy.RL.2.1")),
        "`RL.2.1` must reach the prefixed Common Core code"
    );

    for typed in ["5.4F", "5.4.F", "5-4-f"] {
        assert!(
            index
                .by_code_prefix(typed)
                .iter()
                .any(|hit| hit.framework == Framework::Teks),
            "`{typed}` must reach a TEKS standard"
        );
    }

    assert!(
        index
            .by_code_prefix("MS-LS1-1")
            .iter()
            .any(|hit| hit.framework == Framework::Ngss),
        "an NGSS performance expectation is reachable by its own code"
    );
}

#[test]
fn grade_subject_and_keyword_each_narrow_the_corpus() {
    let documents = documents();
    let index = StandardsIndex::build(&documents);

    let whole = documents.iter().map(|d| d.nodes.len()).sum::<usize>();
    let grade_five = index.by_grade(GradeLevel(5)).len();
    assert!(
        grade_five > 0 && grade_five < whole,
        "grade 5 narrows: {grade_five} of {whole}"
    );

    let mathematics = index.by_subject("mathematics").len();
    assert!(
        mathematics > 0 && mathematics < whole,
        "the vintage suffix must not defeat a subject search: {mathematics} of {whole}"
    );

    let fractions = index.by_keyword("equivalent fractions");
    assert!(
        !fractions.is_empty(),
        "a teacher who knows the concept but not the code finds something"
    );
    assert!(
        fractions
            .iter()
            .all(|hit| hit.node.statement.to_lowercase().contains("fractions")),
        "every keyword hit actually carries the term"
    );
}

#[test]
fn the_licence_notices_the_committed_data_obliges_are_available() {
    let documents = documents();
    for document in &documents {
        let framework = document.framework();
        for notice in required_notices(framework) {
            assert!(!notice.text.is_empty(), "{framework} notice text");
        }
        assert!(
            !document.manifest.licences.is_empty()
                || matches!(framework, Framework::Ccss | Framework::Ngss),
            "{framework} records the licence its mirror declared"
        );
    }
    assert!(
        required_notices(Framework::Ccss)
            .iter()
            .any(|notice| notice.text == CCSS_COPYRIGHT_NOTICE),
        "Common Core rows may not be displayed without the owners' notice"
    );
}

#[test]
fn the_tpt_node_id_table_is_committed_empty_and_loads() {
    let table = load_tpt_node_ids(TPT_NODE_IDS).expect("the committed table loads");
    assert!(
        table.is_empty(),
        "the crawl that fills it needs a live TPT session and is founder-gated"
    );
}
