//! Fill the TPT node-id table: walk the four education-standards subtrees
//! under the founder's own session and write
//! `docs/design/data/standards/tpt-node-ids.jsonl`.
//!
//! The founder runs this by hand and nothing else runs it. TPT is a no-API
//! marketplace, so under D1 the server may never issue one of these requests,
//! and the crawl has exactly two lawful homes: a job on a seller's device, or
//! a founder-run capture on the founder's own machine. Decision 5 of
//! `docs/notes/design/standards-ingestion.md` takes the second, because the
//! table is TPT-global rather than a tenant's — every seller who tags
//! `8.F.B.5` posts the same node id — so a device crawl would repeat one
//! global enumeration once per seller and return nothing seller-specific for
//! the traffic it spends, while concentrating the bulk-enumeration exposure on
//! one account the founder controls is the point rather than a side effect.
//!
//! Usage:
//!
//! ```text
//! tam-standards-crawl --i-am-the-founder <out-dir> <crawled-at-rfc3339> [jar-path]
//! ```
//!
//! The acknowledgement flag is required and has no default. This binary is
//! never run from CI, never from a server, and never on a seller's behalf, and
//! a flag that must be typed is what makes each of those a deliberate act
//! rather than an available one.
//!
//! The timestamp is an argument rather than a clock read, matching
//! `tam-standards-fetch`: time enters as data, and it is what every binding
//! records as the moment the crawl saw its triple agree. The jar is the
//! founder's own exported cookie jar, read the way the crate's supervised live
//! examples read it.
//!
//! What is pure lives elsewhere and is tested there: request shaping and node
//! parsing in `tam_marketplace_tpt::standards`, the join and the gate
//! arithmetic in `tam_standards::crawl`. This file is the walk, the file
//! write, and the report the founder reads before committing anything.

#![forbid(unsafe_code)]

use std::io::Read as _;
use std::time::Duration;

use tam_marketplace::transport::{HttpResponse, Transport as _};
use tam_marketplace_tpt::classify::classify_graphql_read;
use tam_marketplace_tpt::standards::{
    self, JurisdictionRoot, StandardNodeId, TreeNode, BINDABLE_NODE_TYPE,
};
use tam_marketplace_tpt::{ReqwestTransport, TptSession};
use tam_standards::bind::{
    bind_walk, BindOutcome, CatalogueIndex, CatalogueRow, JurisdictionScope, Residue, Unresolved,
};
use tam_standards::crawl::{
    is_utc_timestamp, tpt_jurisdiction, tpt_jurisdiction_notation, CrawledStandard,
};
use tam_standards::{
    load_framework, parse_manifest, Framework, StandardsDocument, TptBinding, FRAMEWORKS,
};

type Failure = Box<dyn std::error::Error>;

const ACKNOWLEDGEMENT: &str = "--i-am-the-founder";
const DEFAULT_JAR: &str = "probes/local/tpt-cookies.jar";
const USAGE: &str =
    "usage: tam-standards-crawl --i-am-the-founder <out-dir> <crawled-at-rfc3339> [jar-path]";

/// The committed corpus, compiled in rather than read, so the join runs
/// against the ingest this binary was built from and not against whatever a
/// working copy happens to hold.
const MANIFEST: &str = include_str!("../../../docs/design/data/standards/manifest.json");
const CCSS: &[u8] = include_bytes!("../../../docs/design/data/standards/ccss.jsonl");
const NGSS: &[u8] = include_bytes!("../../../docs/design/data/standards/ngss.jsonl");
const TEKS: &[u8] = include_bytes!("../../../docs/design/data/standards/teks.jsonl");
const VA_SOL: &[u8] = include_bytes!("../../../docs/design/data/standards/va-sol.jsonl");

/// Between calls. TPT's own picker makes 58 expansions in one seller's
/// session, so a walk of comparable size is only unremarkable if it also runs
/// at a comparable pace.
const BETWEEN_CALLS: Duration = Duration::from_secs(1);

fn bytes_of(framework: Framework) -> &'static [u8] {
    match framework {
        Framework::Ccss => CCSS,
        Framework::Ngss => NGSS,
        Framework::Teks => TEKS,
        Framework::VaSol => VA_SOL,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Invocation {
    out_dir: String,
    crawled_at: String,
    jar: String,
}

/// Parse the command line, refusing anything the founder did not ask for.
fn invocation(arguments: &[String]) -> Result<Invocation, String> {
    if !arguments.iter().any(|argument| argument == ACKNOWLEDGEMENT) {
        return Err(format!(
            "this binary crawls TPT under your own session and is never run from CI or on a \
             seller's behalf; pass {ACKNOWLEDGEMENT} to say that is what you are doing\n{USAGE}"
        ));
    }
    let positional: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| !argument.starts_with("--"))
        .collect();
    let out_dir = positional
        .first()
        .copied()
        .ok_or_else(|| format!("missing out-dir\n{USAGE}"))?;
    let crawled_at = positional.get(1).copied().ok_or_else(|| {
        format!("missing crawled-at; pass `date -u +%Y-%m-%dT%H:%M:%SZ`\n{USAGE}")
    })?;
    if !is_utc_timestamp(crawled_at) {
        return Err(format!(
            "`{crawled_at}` is not an RFC 3339 UTC timestamp; expected 2026-10-03T12:00:00Z"
        ));
    }
    Ok(Invocation {
        out_dir: out_dir.to_owned(),
        crawled_at: crawled_at.to_owned(),
        jar: positional.get(2).copied().unwrap_or(DEFAULT_JAR).to_owned(),
    })
}

/// The nodes to expand after a jurisdiction's own level, in the order the
/// answer returned them and each once. The root itself is dropped: expanding a
/// node from its own answer is a loop.
fn expandable(root: StandardNodeId, nodes: &[TreeNode]) -> Vec<StandardNodeId> {
    let mut wanted: Vec<StandardNodeId> = Vec::new();
    for node in nodes {
        if node.id != root && !wanted.contains(&node.id) {
            wanted.push(node.id);
        }
    }
    wanted
}

/// The bindable nodes of a walk, each id once.
///
/// A node of any other level is the tree rather than a standard, and a
/// `standard` carrying no statement is kept: it cannot bind, and it belongs in
/// the residue report rather than in silence.
fn standards_of(nodes: &[TreeNode]) -> Vec<CrawledStandard> {
    let mut seen: Vec<StandardNodeId> = Vec::new();
    let mut standards = Vec::new();
    for node in nodes {
        if node.node_type.as_deref() != Some(BINDABLE_NODE_TYPE) || seen.contains(&node.id) {
            continue;
        }
        seen.push(node.id);
        standards.push(CrawledStandard {
            tpt_node_id: tam_standards::TptNodeId(node.id.0),
            name: node.name.clone(),
            statement: node.statement.clone().unwrap_or_default(),
            ancestry: node
                .parent_ids
                .iter()
                .map(|parent| tam_standards::TptNodeId(parent.0))
                .collect(),
        });
    }
    standards
}

/// One framework's catalogue rows: the nodes a seller can tag, with the
/// subject their set declares.
fn catalogue_rows(document: &StandardsDocument) -> Vec<CatalogueRow<'_>> {
    document
        .addressable()
        .filter_map(|node| {
            Some(CatalogueRow {
                source_guid: node.source_guid.as_str(),
                subject: document.subject_of(node).unwrap_or_default(),
                code: node.code.as_deref()?,
                alt_code: node.alt_code.as_deref(),
                statement: node.statement.as_str(),
            })
        })
        .collect()
}

/// One binding per line, sorted by the key the table is keyed on, newline
/// terminated. The same determinism the ingest has, for the same reason: two
/// runs over the same answers produce the same bytes.
fn render_bindings(bindings: &mut [TptBinding]) -> Result<String, serde_json::Error> {
    bindings.sort_by(|left, right| {
        (left.framework, &left.source_guid).cmp(&(right.framework, &right.source_guid))
    });
    let mut rendered = String::new();
    for binding in &*bindings {
        rendered.push_str(&serde_json::to_string(binding)?);
        rendered.push('\n');
    }
    Ok(rendered)
}

/// What the founder reads before committing the file: what bound, and what did
/// not, per framework and per reason.
fn report(framework: Framework, rows: usize, outcome: &BindOutcome) -> String {
    let mut bound_rows: Vec<&str> = outcome
        .bindings
        .iter()
        .map(|binding| binding.source_guid.as_str())
        .collect();
    bound_rows.sort_unstable();
    bound_rows.dedup();
    let mut outside = 0_usize;
    let mut unnamed = 0_usize;
    let mut disagreed = 0_usize;
    let mut contested = 0_usize;
    for item in &outcome.residue {
        match &item.reason {
            Unresolved::OutsideJurisdiction => outside = outside.saturating_add(1),
            Unresolved::NoCatalogueRow => unnamed = unnamed.saturating_add(1),
            Unresolved::StatementDisagrees { .. } => disagreed = disagreed.saturating_add(1),
            Unresolved::RowClaimedTwice { .. } => contested = contested.saturating_add(1),
        }
    }
    format!(
        "{framework}: {} bindings over {} of {rows} taggable rows; residue {outside} outside the walked root, {unnamed} no row carries, {disagreed} no row states alike, {contested} claims on rows two nodes both reached",
        outcome.bindings.len(),
        bound_rows.len(),
    )
}

fn residue_lines(residue: &[Residue]) -> Vec<String> {
    residue
        .iter()
        .map(|item| match &item.reason {
            Unresolved::OutsideJurisdiction => format!(
                "  {} node {} `{}`: its ancestor chain does not carry the root this walk expanded",
                item.framework, item.tpt_node_id.0, item.name
            ),
            Unresolved::NoCatalogueRow => format!(
                "  {} node {} `{}`: no catalogue row carries this code",
                item.framework, item.tpt_node_id.0, item.name
            ),
            Unresolved::StatementDisagrees { candidates } => format!(
                "  {} node {} `{}`: {candidates} row(s) carry this code and none states it alike",
                item.framework, item.tpt_node_id.0, item.name
            ),
            Unresolved::RowClaimedTwice {
                source_guid,
                others,
            } => format!(
                "  {} node {} `{}`: row {source_guid} is claimed by {} too, so neither binds it",
                item.framework,
                item.tpt_node_id.0,
                item.name,
                others
                    .iter()
                    .map(|other| other.0.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
        })
        .collect()
}

/// Check that a root still names the jurisdiction the walk assumes.
///
/// The four ids are constants here, and TPT's id space is a search-index
/// artefact rather than a stable public identifier, so expanding one on the
/// strength of its number alone makes exactly the bet the kill gate exists to
/// refuse. The enumeration answers it in one request, and a mismatch stops the
/// crawl rather than filling the table from whatever that id names now.
fn confirm_root(framework: Framework, roots: &[JurisdictionRoot]) -> Result<(), String> {
    let expected = tpt_jurisdiction(framework);
    let notation = tpt_jurisdiction_notation(framework);
    let Some(root) = roots.iter().find(|root| root.id.0 == expected.0) else {
        return Err(format!(
            "TPT's enumeration no longer carries root {}, which is the subtree {framework} is \
             expanded from",
            expected.0
        ));
    };
    match root.notation.as_deref() {
        Some(found) if found.eq_ignore_ascii_case(notation) => Ok(()),
        Some(found) => Err(format!(
            "root {} notates `{found}` where {framework} expects `{notation}`, so that id now \
             names a different jurisdiction",
            expected.0
        )),
        None => Err(format!(
            "root {} carries no notation, so nothing confirms it is still {framework}",
            expected.0
        )),
    }
}

/// The table is the founder's data, and an empty walk would write an empty file
/// over it. A crawl that answered nothing for a framework refuses rather than
/// deleting every binding the last capture made.
fn refuse_empty_walk(framework: Framework, crawled: usize) -> Result<(), String> {
    if crawled == 0 {
        return Err(format!(
            "{framework} answered no standards, so nothing is written and the existing table \
             is left as it stands"
        ));
    }
    Ok(())
}

/// The same refusal for a walk that answered and bound nothing, which would
/// blank the table just as thoroughly.
fn refuse_empty_table(bindings: usize) -> Result<(), String> {
    if bindings == 0 {
        return Err(
            "no walk produced a binding, so nothing is written and the existing table is \
                    left as it stands"
                .to_owned(),
        );
    }
    Ok(())
}

/// TPT's own enumeration of its 166 jurisdiction roots, which is the only
/// listing of them that exists.
async fn enumerate_jurisdictions(
    transport: &ReqwestTransport,
) -> Result<Vec<JurisdictionRoot>, Failure> {
    let response = transport
        .send(standards::jurisdictions_request())
        .await
        .map_err(|error| format!("enumerating jurisdictions: {error:?}"))?;
    let body = classify_graphql_read(&response)
        .map_err(|error| format!("enumerating jurisdictions: {error:?}"))?;
    Ok(standards::parse_jurisdictions(&body)?)
}

async fn expand(
    transport: &ReqwestTransport,
    node: StandardNodeId,
    depth: Option<u32>,
) -> Result<Vec<TreeNode>, Failure> {
    let response: HttpResponse = transport
        .send(standards::subtree_request(node, depth))
        .await
        .map_err(|error| format!("expanding node {node}: {error:?}"))?;
    let body = classify_graphql_read(&response)
        .map_err(|error| format!("expanding node {node}: {error:?}"))?;
    Ok(standards::parse_subtree(&body)?)
}

/// Walk one jurisdiction the way the picker does: one level at the root, then
/// each of its children whole. Expanding the root itself in one call is not a
/// shape any capture recorded, and a walk that asked for it would be asking
/// TPT for something its own client never asks for.
async fn crawl_framework(
    transport: &ReqwestTransport,
    framework: Framework,
) -> Result<Vec<CrawledStandard>, Failure> {
    let root = StandardNodeId(tpt_jurisdiction(framework).0);
    let top = expand(transport, root, Some(1)).await?;
    let subtrees = expandable(root, &top);
    let mut nodes = top;
    eprintln!(
        "{framework}: root {root} has {} subtrees to expand",
        subtrees.len()
    );
    for node in subtrees {
        tokio::time::sleep(BETWEEN_CALLS).await;
        nodes.extend(expand(transport, node, None).await?);
    }
    Ok(standards_of(&nodes))
}

fn arguments() -> Vec<String> {
    std::env::args().skip(1).collect()
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let invocation = invocation(&arguments())?;

    let mut jar = String::new();
    std::fs::File::open(&invocation.jar)?.read_to_string(&mut jar)?;
    let session = TptSession::from_netscape_jar(&jar)?;
    let transport = ReqwestTransport::new(&session)?;

    let roots = enumerate_jurisdictions(&transport).await?;
    eprintln!(
        "TPT's enumeration answered with {} jurisdiction roots",
        roots.len()
    );
    for framework in FRAMEWORKS {
        confirm_root(framework, &roots)?;
    }

    let manifest = parse_manifest(MANIFEST)?;
    let mut bindings: Vec<TptBinding> = Vec::new();
    let mut residue: Vec<Residue> = Vec::new();
    for framework in FRAMEWORKS {
        let document = load_framework(&manifest, framework, bytes_of(framework))?;
        let rows = catalogue_rows(&document);
        let taggable = rows.len();
        let catalogue = CatalogueIndex::build(rows);

        let crawled = crawl_framework(&transport, framework).await?;
        refuse_empty_walk(framework, crawled.len())?;
        eprintln!("{framework}: {} standards crawled", crawled.len());

        let scope = JurisdictionScope {
            framework,
            root: tpt_jurisdiction(framework),
        };
        let outcome = bind_walk(&scope, &crawled, &catalogue, &invocation.crawled_at);
        eprintln!("{}", report(framework, taggable, &outcome));
        bindings.extend(outcome.bindings);
        residue.extend(outcome.residue);
    }

    refuse_empty_table(bindings.len())?;
    let rendered = render_bindings(&mut bindings)?;
    let path = std::path::Path::new(&invocation.out_dir).join("tpt-node-ids.jsonl");
    std::fs::write(&path, rendered.as_bytes())?;
    eprintln!(
        "wrote {} ({} bindings, {} bytes)",
        path.display(),
        bindings.len(),
        rendered.len()
    );

    if !residue.is_empty() {
        eprintln!("{} crawled nodes bound nothing:", residue.len());
        for line in residue_lines(&residue) {
            eprintln!("{line}");
        }
        eprintln!(
            "nothing above is a defect to fix in the file: an unbound node stays unbound, and a \
             seller's tag for it is carried in our own catalogue and omitted from a TPT publish \
             with a visible loss record."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One jurisdiction's top level and one subject's subtree, in the shape
    /// `docs/research/rethink/tpt-product-model.md:341` records: a flat list
    /// carrying every level, with the leaves typed `standard`.
    const TOP_LEVEL: &str = r#"{"data":{"educationStandards":{"children":[
      {"id":3052,"name":"English Language Arts","depth":1,"type":"subject","parentIds":[3054]},
      {"id":3053,"name":"Mathematics","depth":1,"type":"subject","parentIds":[3054]}
    ]}}}"#;
    const SUBTREE: &str = r#"{"data":{"educationStandards":{"children":[
      {"id":8001,"name":"Language","depth":2,"type":"domain","parentIds":[3054,3052]},
      {"id":9002,"name":"CCRA.L.1","depth":4,"type":"standard","parentIds":[3054,3052,8001],
       "descriptionText":"Demonstrate command of the conventions of standard English grammar and usage when writing or speaking."},
      {"id":9002,"name":"CCRA.L.1","depth":4,"type":"standard","parentIds":[3054,3052,8001],
       "descriptionText":"Demonstrate command of the conventions of standard English grammar and usage when writing or speaking."}
    ]}}}"#;
    /// The same level with the root echoed back and one child repeated, which
    /// is what `expandable`'s two guards exist for.
    const TOP_LEVEL_WITH_ROOT_AND_REPEAT: &str = r#"{"data":{"educationStandards":{"children":[
      {"id":3054,"name":"Common Core State Standards","depth":0,"type":"jurisdiction","parentIds":[]},
      {"id":3052,"name":"English Language Arts","depth":1,"type":"subject","parentIds":[3054]},
      {"id":3053,"name":"Mathematics","depth":1,"type":"subject","parentIds":[3054]},
      {"id":3052,"name":"English Language Arts","depth":1,"type":"subject","parentIds":[3054]}
    ]}}}"#;
    const ANCHOR: &str = "Demonstrate command of the conventions of standard English grammar and usage when writing or speaking.";
    const AT: &str = "2026-10-03T00:00:00Z";

    fn arguments(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|argument| (*argument).to_owned()).collect()
    }

    fn nodes(fixture: &str) -> Vec<TreeNode> {
        let body: serde_json::Value = serde_json::from_str(fixture).expect("fixture");
        standards::parse_subtree(&body).expect("the recorded shape parses")
    }

    #[test]
    fn the_crawl_refuses_to_run_without_the_founder_acknowledgement() {
        let refusal = invocation(&arguments(&["docs/design/data/standards", AT]))
            .expect_err("an unacknowledged crawl must not run");
        assert!(
            refusal.contains(ACKNOWLEDGEMENT),
            "the refusal names the flag it wants, got {refusal}"
        );
    }

    #[test]
    fn the_timestamp_is_checked_before_anything_is_crawled() {
        let refusal = invocation(&arguments(&[
            ACKNOWLEDGEMENT,
            "docs/design/data/standards",
            "2026-10-03",
        ]))
        .expect_err("a date is not the instant a binding records");
        assert!(refusal.contains("RFC 3339"), "got {refusal}");
    }

    #[test]
    fn the_jar_defaults_and_is_overridable() {
        let default = invocation(&arguments(&[
            ACKNOWLEDGEMENT,
            "docs/design/data/standards",
            AT,
        ]))
        .expect("the acknowledged form parses");
        assert_eq!(default.jar, DEFAULT_JAR);
        assert_eq!(default.out_dir, "docs/design/data/standards");
        assert_eq!(default.crawled_at, AT);

        let named = invocation(&arguments(&[
            ACKNOWLEDGEMENT,
            "docs/design/data/standards",
            AT,
            "/tmp/jar",
        ]))
        .expect("a named jar parses");
        assert_eq!(named.jar, "/tmp/jar");
    }

    #[test]
    fn the_walk_expands_the_roots_children_once_each_and_never_the_root() {
        let root = StandardNodeId(3054);
        let plan = expandable(root, &nodes(TOP_LEVEL_WITH_ROOT_AND_REPEAT));
        assert_eq!(
            plan,
            vec![StandardNodeId(3052), StandardNodeId(3053)],
            "one level at the root, then each subject whole and each of them once"
        );
        assert!(
            !plan.contains(&root),
            "expanding the root from its own answer is a loop, and this answer echoes it back"
        );
    }

    #[test]
    fn only_bindable_levels_are_crawled_and_each_id_once() {
        let crawled = standards_of(&nodes(SUBTREE));
        assert_eq!(
            crawled.len(),
            1,
            "the domain is the tree, and the repeated leaf is one node"
        );
        assert_eq!(crawled[0].name, "CCRA.L.1");
        assert_eq!(crawled[0].statement, ANCHOR);
    }

    #[test]
    fn a_recorded_walk_binds_the_catalogue_and_renders_deterministically() {
        let mut walked = nodes(TOP_LEVEL);
        walked.extend(nodes(SUBTREE));
        let crawled = standards_of(&walked);

        let catalogue = CatalogueIndex::build(vec![
            CatalogueRow {
                source_guid: "grade-5",
                subject: "English Language Arts",
                code: "CCSS.ELA-Literacy.CCRA.L.1",
                alt_code: Some("CCR.L.1"),
                statement: ANCHOR,
            },
            CatalogueRow {
                source_guid: "grade-4",
                subject: "English Language Arts",
                code: "CCSS.ELA-Literacy.CCRA.L.1",
                alt_code: Some("CCR.L.1"),
                statement: ANCHOR,
            },
        ]);
        let scope = JurisdictionScope {
            framework: Framework::Ccss,
            root: tpt_jurisdiction(Framework::Ccss),
        };
        let outcome = bind_walk(&scope, &crawled, &catalogue, AT);
        assert!(
            outcome.residue.is_empty(),
            "every crawled standard resolved, got {:?}",
            outcome.residue
        );

        let mut bindings = outcome.bindings;
        let rendered = render_bindings(&mut bindings).expect("bindings render");
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2, "one line per bound row");
        assert!(
            rendered.ends_with('\n'),
            "the file is newline terminated like every other committed one"
        );
        assert!(
            lines[0].contains("grade-4") && lines[1].contains("grade-5"),
            "sorted by the key the table is keyed on, got {lines:?}"
        );
        assert!(
            lines[0].contains(r#""tpt_node_id":9002"#)
                && lines[0].contains(r#""tpt_name":"CCRA.L.1""#),
            "the id the form posts and the name the identity check reads, got {}",
            lines[0]
        );

        let summary = report(
            Framework::Ccss,
            2,
            &BindOutcome {
                bindings,
                residue: vec![],
            },
        );
        assert!(
            summary.contains("2 bindings over 2 of 2 taggable rows"),
            "got {summary}"
        );
    }

    #[test]
    fn a_root_that_no_longer_notates_its_jurisdiction_stops_the_crawl() {
        let confirmed = vec![
            JurisdictionRoot {
                id: StandardNodeId(3054),
                name: "Common Core State Standards".to_owned(),
                notation: Some("ccss".to_owned()),
            },
            JurisdictionRoot {
                id: StandardNodeId(3326),
                name: "Something Else Entirely".to_owned(),
                notation: Some("nsw".to_owned()),
            },
            JurisdictionRoot {
                id: StandardNodeId(5785),
                name: "Virginia Standards of Learning".to_owned(),
                notation: None,
            },
        ];
        assert_eq!(
            confirm_root(Framework::Ccss, &confirmed),
            Ok(()),
            "the root TPT still notates as ccss is the one we expand for Common Core"
        );

        let moved = confirm_root(Framework::Teks, &confirmed)
            .expect_err("a root that notates something else must not be expanded");
        assert!(
            moved.contains("3326") && moved.contains("nsw") && moved.contains("teks"),
            "the refusal names the id, what it says now and what we expected, got {moved}"
        );

        let unnotated = confirm_root(Framework::VaSol, &confirmed)
            .expect_err("a root carrying no notation confirms nothing");
        assert!(unnotated.contains("5785"), "got {unnotated}");

        let gone = confirm_root(Framework::Ngss, &confirmed)
            .expect_err("a root the enumeration no longer carries cannot be expanded");
        assert!(gone.contains("3055"), "got {gone}");
    }

    #[test]
    fn an_empty_walk_writes_nothing_over_the_founders_table() {
        let empty = refuse_empty_walk(Framework::Teks, 0)
            .expect_err("a framework that answered nothing must not reach the writer");
        assert!(
            empty.contains("left as it stands"),
            "the refusal says the existing table is kept, got {empty}"
        );
        assert_eq!(refuse_empty_walk(Framework::Teks, 1), Ok(()));

        let nothing_bound = refuse_empty_table(0)
            .expect_err("a run that bound nothing would blank the table just as thoroughly");
        assert!(
            nothing_bound.contains("left as it stands"),
            "got {nothing_bound}"
        );
        assert_eq!(refuse_empty_table(1), Ok(()));
    }

    #[test]
    fn the_residue_report_says_which_refusal_each_node_met() {
        let lines = residue_lines(&[
            Residue {
                framework: Framework::Teks,
                tpt_node_id: tam_standards::TptNodeId(7001),
                name: "9.9.Z".to_owned(),
                reason: Unresolved::NoCatalogueRow,
            },
            Residue {
                framework: Framework::Teks,
                tpt_node_id: tam_standards::TptNodeId(7002),
                name: "1.1.A".to_owned(),
                reason: Unresolved::StatementDisagrees { candidates: 2 },
            },
        ]);
        assert!(lines[0].contains("no catalogue row carries this code"));
        assert!(lines[1].contains("2 row(s) carry this code"));
    }
}
