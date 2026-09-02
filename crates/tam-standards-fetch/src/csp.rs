//! The Common Standards Project's wire shape, and the normalisation from it
//! into `tam-standards`.
//!
//! One mirror rather than four owners, by decision. Common Core publishes no
//! machine-readable form at all today, NGSS never did, Virginia blocks
//! automated clients, and D19 ingests TEKS from this mirror under its CC BY
//! licence rather than from TEA's own feed, whose terms of service we never
//! agree to. What that buys is one parser; what it costs is that every row is
//! a third party's transcription, which the manifest's `source` field records
//! so a reconciliation item raised against a row can say so.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use tam_standards::{
    addressable, dotted_prefixes, DocumentRef, Framework, GradeBand, Licence, NodeShape, SetRef,
    StandardNode,
};

#[derive(Debug, Deserialize)]
pub(crate) struct Envelope<T> {
    pub(crate) data: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JurisdictionDetail {
    pub(crate) standard_sets: Vec<SetSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetSummary {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) subject: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetDetail {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) subject: Option<String>,
    #[serde(default)]
    pub(crate) normalized_subject: Option<String>,
    #[serde(default)]
    pub(crate) education_levels: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) license: Option<WireLicence>,
    #[serde(default)]
    pub(crate) document: Option<WireDocument>,
    #[serde(default)]
    pub(crate) standards: BTreeMap<String, WireNode>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireLicence {
    // Null rather than absent on 29 of the mirrored Texas sets, so these are
    // optional in the wire shape and flattened to the empty string in the
    // model, where "declared nothing" and "declared an empty title" are the
    // same fact.
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(rename = "URL", default)]
    pub(crate) url: Option<String>,
    #[serde(rename = "rightsHolder", default)]
    pub(crate) rights_holder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireDocument {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) valid: Option<String>,
    #[serde(default)]
    pub(crate) source_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireNode {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) asn_identifier: Option<String>,
    #[serde(default)]
    pub(crate) depth: u32,
    #[serde(default)]
    pub(crate) statement_notation: Option<String>,
    #[serde(default)]
    pub(crate) alt_statement_notation: Option<String>,
    #[serde(default)]
    pub(crate) statement_label: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) parent_id: Option<String>,
    #[serde(default)]
    pub(crate) ancestor_ids: Vec<String>,
    #[serde(default)]
    pub(crate) exact_match: Vec<String>,
}

/// An empty or whitespace-only string is absent, not a value.
///
/// The mirror writes `statementNotation` as `""` on 848 of the rows in this
/// ingest rather than as null, and a code of `""` would otherwise pass every
/// `is_some` test, be marked addressable, and reach a picker as a standard
/// with nothing to post.
fn present(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

impl WireNode {
    fn code(&self) -> Option<String> {
        present(self.statement_notation.as_deref())
    }

    /// The owner's dereferenceable URI, where the mirror carries one.
    ///
    /// `exactMatch` is not a crosswalk: on a Common Core standard it holds the
    /// standard's own canonical URI beside the mirror's own identifier for it,
    /// asserting identity rather than equivalence to a counterpart elsewhere.
    /// Only the URI half is kept.
    fn uri(&self) -> Option<String> {
        self.exact_match
            .iter()
            .find(|candidate| candidate.starts_with("http://") || candidate.starts_with("https://"))
            .cloned()
    }
}

/// Every node of a framework, keyed by the mirror's own identifier, with the
/// set each was first seen in.
#[derive(Debug, Default)]
pub(crate) struct FrameworkNodes {
    ordered_sets: Vec<SetRef>,
    set_index: BTreeMap<String, usize>,
    nodes: BTreeMap<String, (usize, WireNode)>,
    /// Nodes seen again in a later set, which is ordinary here: the mirror
    /// serves overlapping grade and course sets over one corpus.
    pub(crate) duplicate_nodes: usize,
}

impl FrameworkNodes {
    pub(crate) fn absorb(&mut self, detail: SetDetail, source_url: String) {
        let index = *self.set_index.entry(detail.id.clone()).or_insert_with(|| {
            self.ordered_sets.push(SetRef {
                id: detail.id.clone(),
                title: detail.title.clone(),
                subject: detail.subject.clone().unwrap_or_default(),
                normalized_subject: detail.normalized_subject.clone(),
                grade_band: GradeBand::derive(detail.education_levels.as_deref().unwrap_or(&[])),
                source_url,
                document: detail.document.as_ref().map(|document| DocumentRef {
                    id: document.id.clone(),
                    title: document.title.clone(),
                    valid: document.valid.clone(),
                    source_url: document.source_url.clone(),
                }),
                licence: detail.license.as_ref().map(|licence| Licence {
                    title: licence.title.clone().unwrap_or_default(),
                    url: licence.url.clone().unwrap_or_default(),
                    rights_holder: licence.rights_holder.clone().unwrap_or_default(),
                }),
            });
            self.ordered_sets.len() - 1
        });
        for (key, node) in detail.standards {
            // The mirror keys `standards` by the node's own identifier and
            // repeats it in the body; a few rows carry only the key, so the
            // map key is the identifier and the body's copy is a duplicate of
            // it. Keying the collection by it makes that the one source.
            let id = node.id.clone().unwrap_or(key);
            if self.nodes.contains_key(&id) {
                self.duplicate_nodes += 1;
                continue;
            }
            self.nodes.insert(id, (index, node));
        }
    }

    pub(crate) fn sets(&self) -> &[SetRef] {
        &self.ordered_sets
    }

    /// Normalise into the committed shape.
    ///
    /// Both structural facts the addressability rule needs are computed here
    /// over the whole framework rather than per set, because a node's children
    /// and a code's extensions can live in a different set from the node
    /// itself.
    pub(crate) fn normalise(&self, framework: Framework) -> Vec<StandardNode> {
        let mut parents: BTreeSet<&str> = BTreeSet::new();
        for (_, node) in self.nodes.values() {
            if let Some(parent) = node.parent_id.as_deref() {
                parents.insert(parent);
            }
        }
        // Scoped per set, not per framework, because a published code is
        // only unique inside the set that served it. Texas mathematics and
        // Texas English both carry `K.2.A`, and only the English one has the
        // sub-expectations `K.2.A.i`; a framework-wide prefix table would let
        // the English children strike the mathematics standard off the
        // addressable list. The set is the tree the node belongs to, and its
        // children live in the same one.
        let mut prefixes_by_set: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for (set, node) in self.nodes.values() {
            prefixes_by_set
                .entry(*set)
                .or_default()
                .extend(dotted_prefixes(node.code().as_deref()));
        }

        self.nodes
            .iter()
            .map(|(guid, (set, node))| {
                let code = node.code();
                let node_type = present(node.statement_label.as_deref());
                let statement = node.description.clone().unwrap_or_default();
                let shape = NodeShape {
                    node_type: node_type.as_deref(),
                    code: code.as_deref(),
                    has_statement: !statement.trim().is_empty(),
                    is_leaf: !parents.contains(guid.as_str()),
                    extended_by_a_code: code.as_deref().is_some_and(|code| {
                        prefixes_by_set
                            .get(set)
                            .is_some_and(|prefixes| prefixes.contains(code))
                    }),
                };
                StandardNode {
                    set: *set,
                    source_guid: guid.clone(),
                    asn_identifier: node.asn_identifier.clone(),
                    parent_guid: node.parent_id.clone(),
                    depth: node.depth,
                    code: code.clone(),
                    alt_code: present(node.alt_statement_notation.as_deref()),
                    node_type: node_type.clone(),
                    statement,
                    hierarchy_path: self.hierarchy_path(node),
                    uri: node.uri(),
                    addressable: addressable(framework, shape),
                }
            })
            .collect()
    }

    /// The ancestor chain of codes, root first, read off the mirror's own
    /// links rather than parsed out of the code.
    ///
    /// `ancestorIds` is ordered nearest-first on the wire, so it is reversed
    /// here. Ancestors that carry no code contribute nothing rather than an
    /// empty rung: the chain is what a picker renders as a breadcrumb, and a
    /// blank rung in it is worse than a shorter one.
    fn hierarchy_path(&self, node: &WireNode) -> Vec<String> {
        node.ancestor_ids
            .iter()
            .rev()
            .filter_map(|ancestor| self.nodes.get(ancestor))
            .filter_map(|(_, ancestor)| ancestor.code())
            .collect()
    }

    pub(crate) fn distinct_codes(&self) -> usize {
        self.nodes
            .values()
            .filter_map(|(_, node)| node.code())
            .collect::<BTreeSet<String>>()
            .len()
    }
}

/// Every distinct licence declared across a framework's sets, sorted so two
/// runs list them in one order.
pub(crate) fn declared_licences(sets: &[SetRef]) -> Vec<Licence> {
    let mut licences: BTreeSet<Licence> = BTreeSet::new();
    for set in sets {
        if let Some(licence) = &set.licence {
            if !licence.title.is_empty() || !licence.rights_holder.is_empty() {
                licences.insert(licence.clone());
            }
        }
    }
    licences.into_iter().collect()
}
