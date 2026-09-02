//! The four lookups a standards picker runs, and nothing else.
//!
//! The sources research fixed the shape from what a teacher actually does, in
//! the order they do it: type a code fragment they have in front of them from
//! a lesson plan, filter by grade and subject and browse, and search the
//! statement text when they know the concept but not the code. Eleven thousand
//! addressable codes rules out a flat list, so each of those is a method here
//! rather than a filter the caller writes over `nodes`.
//!
//! Code search is the one with a subtlety. A teacher types `5.3B` where the
//! mirror wrote `5.3.B`, and `RL.2.1` where the mirror wrote
//! `CCSS.ELA-Literacy.RL.2.1`. So every code is folded to its letters and
//! digits before it is compared -- separators carry no information a teacher
//! reproduces reliably -- and both the published code and the bare
//! `altStatementNotation` are indexed, because the bare form is not a prefix
//! of the published one.

use crate::format::StandardsDocument;
use crate::grades::GradeLevel;
use crate::model::{Framework, StandardNode};

/// One search result, joined back to the document that explains it.
#[derive(Debug, Clone, Copy)]
pub struct Match<'a> {
    pub framework: Framework,
    pub document: &'a StandardsDocument,
    pub node: &'a StandardNode,
}

impl<'a> Match<'a> {
    pub fn subject(&self) -> Option<&'a str> {
        self.document.subject_of(self.node)
    }
}

/// Fold a code to the characters a teacher reproduces: its letters and digits,
/// upper-cased. `5.3B`, `5.3.B` and `5-3-b` all fold to `53B`.
pub fn fold_code(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_uppercase)
        .collect()
}

fn fold_text(text: &str) -> String {
    text.to_lowercase()
}

/// A code-prefix index over one or more loaded frameworks.
///
/// Built once and queried many times: the fold is not free, and a picker runs
/// a query per keystroke.
#[derive(Debug)]
pub struct StandardsIndex<'a> {
    documents: &'a [StandardsDocument],
    /// Folded code, then the document and node it names. Sorted by the folded
    /// code so a prefix query is a binary search rather than a scan.
    codes: Vec<(String, usize, usize)>,
}

impl<'a> StandardsIndex<'a> {
    pub fn build(documents: &'a [StandardsDocument]) -> Self {
        let mut codes = Vec::new();
        for (document_index, document) in documents.iter().enumerate() {
            for (node_index, node) in document.nodes.iter().enumerate() {
                for code in [node.code.as_deref(), node.alt_code.as_deref()]
                    .into_iter()
                    .flatten()
                {
                    let folded = fold_code(code);
                    if !folded.is_empty() {
                        codes.push((folded, document_index, node_index));
                    }
                }
            }
        }
        codes.sort();
        codes.dedup();
        StandardsIndex { documents, codes }
    }

    pub fn documents(&self) -> &'a [StandardsDocument] {
        self.documents
    }

    fn resolve(&self, document_index: usize, node_index: usize) -> Option<Match<'a>> {
        let document = self.documents.get(document_index)?;
        let node = document.nodes.get(node_index)?;
        Some(Match {
            framework: document.framework(),
            document,
            node,
        })
    }

    /// Every node whose published or bare code begins with `query`, once each.
    ///
    /// A node indexed under both its forms would otherwise be returned twice
    /// for a query that is a prefix of both.
    pub fn by_code_prefix(&self, query: &str) -> Vec<Match<'a>> {
        let folded = fold_code(query);
        if folded.is_empty() {
            return Vec::new();
        }
        let start = self
            .codes
            .partition_point(|(code, _, _)| code.as_str() < folded.as_str());
        let mut seen: Vec<(usize, usize)> = Vec::new();
        let mut matches = Vec::new();
        for (code, document_index, node_index) in self.codes.iter().skip(start) {
            if !code.starts_with(&folded) {
                break;
            }
            if seen.contains(&(*document_index, *node_index)) {
                continue;
            }
            seen.push((*document_index, *node_index));
            if let Some(found) = self.resolve(*document_index, *node_index) {
                matches.push(found);
            }
        }
        matches
    }

    /// Every node whose set covers `level`.
    ///
    /// The grade is a property of the mirrored set rather than of the node,
    /// which is why this is a document join and not a field test.
    pub fn by_grade(&self, level: GradeLevel) -> Vec<Match<'a>> {
        self.filter(|document, node| {
            document
                .set_of(node)
                .is_some_and(|set| set.grade_band.covers(level))
        })
    }

    /// Every node whose set's subject label contains `needle`, ignoring case.
    ///
    /// Containment rather than equality because the mirror's labels carry the
    /// vintage a seller does not type -- `Mathematics (2012-)` is what a
    /// search for `mathematics` has to find.
    pub fn by_subject(&self, needle: &str) -> Vec<Match<'a>> {
        let needle = fold_text(needle);
        if needle.is_empty() {
            return Vec::new();
        }
        self.filter(|document, node| {
            document
                .subject_of(node)
                .is_some_and(|subject| fold_text(subject).contains(&needle))
        })
    }

    /// Every node whose statement contains all of `query`'s whitespace-
    /// separated terms, ignoring case.
    ///
    /// All terms rather than the raw string, so `fractions equivalent` finds
    /// what `equivalent fractions` finds. The statement is searched as the
    /// owner wrote it; nothing here rewrites or truncates it.
    pub fn by_keyword(&self, query: &str) -> Vec<Match<'a>> {
        let terms: Vec<String> = query.split_whitespace().map(fold_text).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        self.filter(|_, node| {
            let statement = fold_text(&node.statement);
            terms.iter().all(|term| statement.contains(term.as_str()))
        })
    }

    fn filter(
        &self,
        keep: impl Fn(&'a StandardsDocument, &'a StandardNode) -> bool,
    ) -> Vec<Match<'a>> {
        let mut matches = Vec::new();
        for document in self.documents {
            for node in &document.nodes {
                if keep(document, node) {
                    matches.push(Match {
                        framework: document.framework(),
                        document,
                        node,
                    });
                }
            }
        }
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{FrameworkManifest, StandardsDocument};
    use crate::grades::GradeBand;
    use crate::model::{Jurisdiction, SetRef};

    fn document() -> StandardsDocument {
        let nodes = vec![
            node("CCSS.ELA-Literacy.RL.2.1", Some("RL.2.1"), "Ask and answer such questions as who, what, where, when, why, and how."),
            node("CCSS.Math.Content.4.NF.C.7", Some("4.NF.7"), "Compare two decimals to hundredths by reasoning about their size."),
            node("CCSS.Math.Content.5.NF.B.3", Some("5.NF.3"), "Interpret a fraction as division; solve word problems involving equivalent fractions."),
        ];
        StandardsDocument {
            manifest: FrameworkManifest {
                framework: Framework::Ccss,
                jurisdiction: Jurisdiction::Us,
                fetched_at: "2026-09-03T00:00:00Z".to_owned(),
                source: "commonstandardsproject".to_owned(),
                source_api: "https://api.commonstandardsproject.com".to_owned(),
                file: "ccss.jsonl".to_owned(),
                file_bytes: 0,
                file_sha256: String::new(),
                node_count: nodes.len(),
                distinct_code_count: nodes.len(),
                addressable_count: nodes.len(),
                licences: Vec::new(),
                sets: vec![SetRef {
                    id: "S1".to_owned(),
                    title: Some("Grade 4".to_owned()),
                    subject: "Mathematics (2012-)".to_owned(),
                    normalized_subject: Some("Math".to_owned()),
                    grade_band: GradeBand::derive(&["04".to_owned(), "05".to_owned()]),
                    source_url: "https://example.invalid/S1".to_owned(),
                    document: None,
                    licence: None,
                }],
            },
            nodes,
        }
    }

    fn node(code: &str, alt: Option<&str>, statement: &str) -> StandardNode {
        StandardNode {
            set: 0,
            source_guid: code.to_owned(),
            asn_identifier: None,
            parent_guid: None,
            depth: 1,
            code: Some(code.to_owned()),
            alt_code: alt.map(str::to_owned),
            node_type: Some("Standard".to_owned()),
            statement: statement.to_owned(),
            hierarchy_path: Vec::new(),
            uri: None,
            addressable: true,
        }
    }

    #[test]
    fn a_bare_code_finds_the_prefixed_one() {
        let documents = [document()];
        let index = StandardsIndex::build(&documents);
        let found = index.by_code_prefix("RL.2.1");
        assert_eq!(found.len(), 1, "the bare form must find the published code");
        assert_eq!(
            found[0].node.code.as_deref(),
            Some("CCSS.ELA-Literacy.RL.2.1")
        );
    }

    #[test]
    fn separators_a_teacher_omits_do_not_change_the_result() {
        let documents = [document()];
        let index = StandardsIndex::build(&documents);
        for typed in ["4.NF.7", "4NF7", "4-nf-7", "4.nf7"] {
            let found = index.by_code_prefix(typed);
            assert_eq!(found.len(), 1, "`{typed}` must find exactly one standard");
        }
    }

    #[test]
    fn a_node_indexed_under_both_forms_is_returned_once() {
        let documents = [document()];
        let index = StandardsIndex::build(&documents);
        // "4" prefixes both CCSS.Math...4.NF.C.7's bare form and nothing else.
        let found = index.by_code_prefix("4");
        assert_eq!(found.len(), 1, "one node, not one per indexed form");
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        let documents = [document()];
        let index = StandardsIndex::build(&documents);
        assert!(index.by_code_prefix("").is_empty(), "code");
        assert!(index.by_subject("").is_empty(), "subject");
        assert!(index.by_keyword("   ").is_empty(), "keyword");
    }

    #[test]
    fn grade_and_subject_read_the_set_rather_than_the_node() {
        let documents = [document()];
        let index = StandardsIndex::build(&documents);
        assert_eq!(
            index.by_grade(GradeLevel(4)).len(),
            3,
            "the whole set covers grade 4"
        );
        assert!(
            index.by_grade(GradeLevel(8)).is_empty(),
            "grade 8 is outside 4-5"
        );
        assert_eq!(
            index.by_subject("mathematics").len(),
            3,
            "the vintage suffix must not defeat a subject search"
        );
    }

    #[test]
    fn keyword_terms_match_in_any_order() {
        let documents = [document()];
        let index = StandardsIndex::build(&documents);
        let ordered = index.by_keyword("equivalent fractions");
        let reversed = index.by_keyword("fractions equivalent");
        assert_eq!(
            ordered.len(),
            1,
            "one statement mentions equivalent fractions"
        );
        assert_eq!(
            ordered.len(),
            reversed.len(),
            "term order must not change the result"
        );
    }
}
