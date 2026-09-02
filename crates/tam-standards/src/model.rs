//! The normalised standards record: one shape for four frameworks whose
//! owners publish four different ones.
//!
//! Two rules from the sources research (`docs/research/rethink/
//! education-standards-sources.md`, "Normalisation") govern every type here.
//! The source's own vocabulary is captured verbatim rather than mapped into a
//! house taxonomy at ingest, so `subject` and `node_type` are the labels the
//! mirror published and nothing else. And the hierarchy comes from the
//! source's parent links rather than from parsing the code, because Virginia's
//! four subjects use four incompatible code grammars and a single regex
//! cannot read them.
//!
//! The grade axis is the one derivation, because it is what a seller searches
//! by and every framework encodes it differently. `GradeBand` therefore keeps
//! the source's own level codes beside the interval derived from them, and
//! names the codes that did not parse rather than dropping them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::grades::GradeBand;

/// The four frameworks TPT's create form offers, and the only four ingested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Framework {
    #[serde(rename = "CCSS")]
    Ccss,
    #[serde(rename = "NGSS")]
    Ngss,
    #[serde(rename = "TEKS")]
    Teks,
    #[serde(rename = "VA_SOL")]
    VaSol,
}

/// Every framework, in the order the on-disk files are written and the
/// manifest lists them.
pub const FRAMEWORKS: [Framework; 4] = [
    Framework::Ccss,
    Framework::Ngss,
    Framework::Teks,
    Framework::VaSol,
];

impl Framework {
    pub fn as_str(self) -> &'static str {
        match self {
            Framework::Ccss => "CCSS",
            Framework::Ngss => "NGSS",
            Framework::Teks => "TEKS",
            Framework::VaSol => "VA_SOL",
        }
    }

    /// The stem of the framework's data file, under `docs/design/data/standards`.
    pub fn file_stem(self) -> &'static str {
        match self {
            Framework::Ccss => "ccss",
            Framework::Ngss => "ngss",
            Framework::Teks => "teks",
            Framework::VaSol => "va-sol",
        }
    }

    /// Derived rather than stored on every row: the jurisdiction is a function
    /// of the framework, and a column that repeats one of three values across
    /// nineteen thousand rows records nothing the framework does not.
    pub fn jurisdiction(self) -> Jurisdiction {
        match self {
            Framework::Ccss | Framework::Ngss => Jurisdiction::Us,
            Framework::Teks => Jurisdiction::Tx,
            Framework::VaSol => Jurisdiction::Va,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "CCSS" => Some(Framework::Ccss),
            "NGSS" => Some(Framework::Ngss),
            "TEKS" => Some(Framework::Teks),
            "VA_SOL" => Some(Framework::VaSol),
            _ => None,
        }
    }
}

impl fmt::Display for Framework {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Jurisdiction {
    #[serde(rename = "US")]
    Us,
    #[serde(rename = "TX")]
    Tx,
    #[serde(rename = "VA")]
    Va,
}

impl Jurisdiction {
    pub fn as_str(self) -> &'static str {
        match self {
            Jurisdiction::Us => "US",
            Jurisdiction::Tx => "TX",
            Jurisdiction::Va => "VA",
        }
    }
}

impl fmt::Display for Jurisdiction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The licence a mirror declares on the set it served.
///
/// Carried per set rather than per framework because the Common Standards
/// Project declares it per set and the rights holders differ across them --
/// D2L Corporation on the Achievement Standards Network lineage, Common
/// Curriculum, Inc. elsewhere -- and a CC BY attribution names the holder.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct Licence {
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub rights_holder: String,
}

/// The owner's own document behind a mirrored set: what a hand-check is run
/// against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRef {
    /// Absent on the 124 mirrored sets whose `document` carries only a title
    /// and a source URL. Optional rather than defaulted, because an empty
    /// string here would read as an owner document with a blank identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

/// One mirrored standard set, and the join target of a node's `set` index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_subject: Option<String>,
    #[serde(default, skip_serializing_if = "GradeBand::is_empty")]
    pub grade_band: GradeBand,
    pub source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<DocumentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<Licence>,
}

/// One node of a framework: a standard a seller can tag, or a container whose
/// text a standard's meaning depends on.
///
/// Containers are kept rather than flattened away. The Common Core identifier
/// documentation gives the reason in the owners' own words: cluster headings
/// were given identifiers precisely so that applications "can preserve the
/// meanings that arise from considering the cluster headings and the
/// individual content standards in conjunction with one another".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardNode {
    /// Index into the framework manifest's `sets`.
    pub set: usize,
    /// The mirror's own identifier for this node, and the row's identity.
    pub source_guid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asn_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_guid: Option<String>,
    pub depth: u32,
    /// The published code (`statementNotation`). Absent on the unlabelled
    /// container rows the mirrors carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The bare form (`altStatementNotation`): `8.F.5` beside
    /// `CCSS.Math.Content.8.F.B.5`, which is what a teacher types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_code: Option<String>,
    /// The source's own type label (`statementLabel`), verbatim. Null across
    /// the whole TEKS mirror, which is why `addressable` cannot be read off
    /// it for every framework.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_type: Option<String>,
    /// The statement, verbatim and never paraphrased. The Common Core grant
    /// names four verbs -- copy, publish, distribute, display -- and
    /// modification is not among them.
    pub statement: String,
    /// The ancestor chain of codes, root first, derived from the mirror's
    /// parent links.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hierarchy_path: Vec<String>,
    /// The owner's dereferenceable URI where the mirror carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Whether a seller tags at this node, decided by the per-framework rule
    /// in `addressable`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub addressable: bool,
}

/// What the addressability rule reads about one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeShape<'a> {
    pub node_type: Option<&'a str>,
    pub code: Option<&'a str>,
    /// No other node in the framework names this one as parent.
    pub is_leaf: bool,
    /// The node carries statement text.
    ///
    /// Two rows of the 2026-09-03 ingest carry a Texas code and no statement
    /// at all. They are not addressable: the picker rule is that a code is
    /// shown with its full statement or not at all, so a row with nothing to
    /// show is a blank line in a seller's picker and an untaggable tag.
    pub has_statement: bool,
    /// Some other code *in the same set* extends this one by a dot segment:
    /// `5.10` is extended by `5.10.A`, and `VS.13` by `VS.13.a`.
    ///
    /// The set, not the framework, is the scope: a published code is unique
    /// only inside the set that served it. Texas mathematics and Texas English
    /// both carry `K.2.A`, and only the English one has sub-expectations
    /// `K.2.A.i`, so a framework-wide prefix table would strike the
    /// mathematics standard off the addressable list.
    pub extended_by_a_code: bool,
}

/// Whether a node is the granularity a seller tags at.
///
/// The rule is per framework because the mirrors do not describe themselves
/// the same way, and the difference is a data-quality fact rather than a
/// modelling preference.
///
/// Common Core and NGSS carry `statementLabel` reliably, so the rule reads it:
/// Standard and Component for Common Core, Performance Expectation for NGSS.
/// Reading anything else there would be wrong rather than merely coarse --
/// NGSS's connection and crosscutting rows are leaves too, and there are
/// fifteen hundred of them against two hundred and eight performance
/// expectations.
///
/// The Texas and Virginia mirrors carry no usable label -- `statementLabel`
/// is null across the whole TEKS mirror -- and both flatten a parent standard
/// into a sibling of its own children: `5.10` and `5.10.A` share the strand as
/// parent, as do `VS.13` and `VS.13.a`. So neither the label nor the parent
/// link separates them, and the rule is the conjunction of the two structural
/// facts that do. A node is addressable there when nothing names it as parent
/// *and* no other code extends it by a dot segment. Each half is needed: the
/// parent link alone admits `VS.13`, and the code extension alone admits the
/// single-letter Virginia strand rows (`R`, `W`, `LU`) whose children are
/// coded `1.C.1` rather than `R.1`.
pub fn addressable(framework: Framework, shape: NodeShape<'_>) -> bool {
    if !shape.has_statement {
        return false;
    }
    match framework {
        Framework::Ccss => matches!(shape.node_type, Some("Standard" | "Component")),
        Framework::Ngss => shape.node_type == Some("Performance Expectation"),
        Framework::Teks | Framework::VaSol => {
            shape.code.is_some() && shape.is_leaf && !shape.extended_by_a_code
        }
    }
}

/// Every code that some other code in `codes` extends by a dot segment.
///
/// Built once per framework and asked of every node, because the question is
/// about the code set as a whole rather than about a node's own string.
/// Callers scope this to one set: see `NodeShape::extended_by_a_code`.
pub fn dotted_prefixes<'a>(
    codes: impl IntoIterator<Item = &'a str>,
) -> std::collections::BTreeSet<String> {
    let mut prefixes = std::collections::BTreeSet::new();
    for code in codes {
        let segments: Vec<&str> = code.split('.').collect();
        for taken in 1..segments.len() {
            prefixes.insert(segments[..taken].join("."));
        }
    }
    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jurisdiction_follows_the_framework() {
        assert_eq!(Framework::Ccss.jurisdiction(), Jurisdiction::Us);
        assert_eq!(Framework::Teks.jurisdiction(), Jurisdiction::Tx);
        assert_eq!(Framework::VaSol.jurisdiction(), Jurisdiction::Va);
    }

    #[test]
    fn framework_round_trips_through_its_wire_name() {
        for framework in FRAMEWORKS {
            assert_eq!(
                Framework::parse(framework.as_str()),
                Some(framework),
                "{framework} did not round-trip"
            );
        }
    }

    #[test]
    fn addressable_reads_the_label_where_the_mirror_carries_one() {
        let standard = NodeShape {
            node_type: Some("Standard"),
            code: Some("CCSS.Math.Content.8.F.B.5"),
            has_statement: true,
            is_leaf: true,
            extended_by_a_code: false,
        };
        assert!(
            addressable(Framework::Ccss, standard),
            "a Standard is tagged at"
        );
        let cluster = NodeShape {
            node_type: Some("Cluster"),
            ..standard
        };
        assert!(
            !addressable(Framework::Ccss, cluster),
            "a Cluster is context, not a tag"
        );
        let expectation = NodeShape {
            node_type: Some("Performance Expectation"),
            code: Some("MS-LS1-1"),
            has_statement: true,
            is_leaf: true,
            extended_by_a_code: false,
        };
        assert!(addressable(Framework::Ngss, expectation));
        let connection = NodeShape {
            node_type: Some("Connection Statement"),
            ..expectation
        };
        assert!(
            !addressable(Framework::Ngss, connection),
            "a leaf connection row is not a performance expectation"
        );
    }

    #[test]
    fn a_flattened_parent_standard_is_excluded_by_its_extended_code() {
        let knowledge_and_skills = NodeShape {
            node_type: None,
            code: Some("5.10"),
            has_statement: true,
            is_leaf: true,
            extended_by_a_code: true,
        };
        assert!(
            !addressable(Framework::Teks, knowledge_and_skills),
            "the mirror makes 5.10 a leaf; 5.10.A is what a seller tags"
        );
        let expectation = NodeShape {
            code: Some("5.10.A"),
            extended_by_a_code: false,
            ..knowledge_and_skills
        };
        assert!(addressable(Framework::Teks, expectation));
    }

    #[test]
    fn a_strand_container_is_excluded_by_its_children_even_with_an_unextended_code() {
        let strand = NodeShape {
            node_type: None,
            code: Some("LU"),
            has_statement: true,
            is_leaf: false,
            extended_by_a_code: false,
        };
        assert!(
            !addressable(Framework::VaSol, strand),
            "Virginia strand rows are coded LU while their children are coded 1.C.1"
        );
    }

    #[test]
    fn a_leaf_without_a_code_is_not_addressable() {
        let unnamed = NodeShape {
            node_type: None,
            code: None,
            has_statement: true,
            is_leaf: true,
            extended_by_a_code: false,
        };
        assert!(
            !addressable(Framework::VaSol, unnamed),
            "there is nothing to post"
        );
    }

    #[test]
    fn a_row_with_a_code_and_no_statement_is_not_addressable() {
        let blank = NodeShape {
            node_type: Some("Standard"),
            code: Some("P2"),
            has_statement: false,
            is_leaf: true,
            extended_by_a_code: false,
        };
        for framework in FRAMEWORKS {
            assert!(
                !addressable(framework, blank),
                "{framework}: a code with nothing to show cannot be tagged"
            );
        }
    }

    #[test]
    fn dotted_prefixes_names_every_extended_code_and_nothing_else() {
        let prefixes = dotted_prefixes(["5.10.A", "5.10", "5.1", "VS.13.a", "LU"]);
        assert!(prefixes.contains("5.10"), "5.10.A extends 5.10");
        assert!(prefixes.contains("VS.13"), "VS.13.a extends VS.13");
        assert!(
            !prefixes.contains("5.1"),
            "5.10 does not extend 5.1: the segment differs"
        );
        assert!(!prefixes.contains("LU"), "nothing extends LU");
    }
}
