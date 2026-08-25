//! The captured Tes trees, the GB-NZ crosswalk derived from them, and the Tes
//! main-age-range table.
//!
//! The crosswalk is a deterministic id transform rather than a mapping
//! problem, per `docs/notes/probes/10-taxonomy-crosswalk.md`: Tes carries a
//! market-neutral `mapTo` id on every node, the market ids are one prefix over
//! a shared suffix, and the descriptions are already identical. The derivation
//! therefore pairs on `mapTo` and validates the other two independently, so a
//! drifted capture degrades to residue and a reconciliation item rather than a
//! wrong edge.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use tam_domain::{
    AgeInterval, CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_types::{CanonicalTermId, InventoryId, Timestamp, NAMESPACE_TAM_TAXONOMY};

/// One country's captured tree, as written by `probes/crawl-taxonomy.py`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TesTree {
    #[serde(rename = "_source")]
    pub source: String,
    pub country: String,
    pub subject_count: usize,
    pub topic_count: usize,
    pub subjects: Vec<TesSubject>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TesSubject {
    pub id: u64,
    pub description: String,
    #[serde(default)]
    pub leaf_depth: Option<String>,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub map_to: Vec<String>,
    #[serde(default)]
    pub parent_id: Option<u64>,
    #[serde(default)]
    pub topics: Vec<TesTopic>,
    /// The crawl's own record that this subject's children could not be read.
    #[serde(rename = "_err", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TesTopic {
    pub id: u64,
    pub description: String,
    #[serde(default)]
    pub leaf_depth: Option<String>,
    #[serde(default)]
    pub phases: Vec<String>,
    #[serde(default)]
    pub map_to: Vec<String>,
    #[serde(default)]
    pub parent_id: Option<u64>,
}

impl TesSubject {
    /// A crawl error means the children were never read, which is not the same
    /// fact as a subject having none, so the derivation treats such a subject
    /// as childless rather than asserting an empty topic list.
    #[must_use]
    pub fn children(&self) -> &[TesTopic] {
        if self.error.is_some() {
            &[]
        } else {
            &self.topics
        }
    }
}

/// Parses one captured tree. The captures are compiled into the test binary
/// rather than read at runtime, so this takes the text and performs no I/O.
pub fn parse_tree(json: &str) -> Result<TesTree, serde_json::Error> {
    serde_json::from_str(json)
}

/// A node that took part in no pair, kept so the seed report names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidueNode {
    pub market: InventoryId,
    pub kind: TermKind,
    pub native_id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub node: ResidueNode,
    /// The node it would have paired with, where one was found.
    pub counterpart: Option<ResidueNode>,
    pub reason: MismatchReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchReason {
    /// The node's id carries no market prefix, so it yields neither a suffix
    /// to pair on nor a name to derive a canonical id from. Nothing in either
    /// capture lacks one; the variant exists so a drifted crawl degrades to
    /// residue rather than to a wrong edge.
    NoMarketPrefix,
    /// Both markets carry a `mapTo` and the two disagree. Tes's own
    /// market-neutral id contradicting the suffix pairing means the capture
    /// drifted, so the node keeps its GB edge alone.
    MapToDisagreement,
    DescriptionMismatch,
    /// The paired node sits under a subject that is not this node's subject's
    /// pair, so the two trees disagree about the parent.
    ParentMismatch,
    /// The subject seeded no canonical term, so a topic under it has no parent
    /// to hang from.
    ParentUnpaired,
    /// Two canonical terms name one target path, which the reverse-`Exact` law
    /// forbids. Neither edge is emitted, because choosing between them is the
    /// decision the design reserves for a human.
    DuplicatePath,
}

impl core::fmt::Display for MismatchReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoMarketPrefix => f.write_str("the node's id carries no market prefix"),
            Self::MapToDisagreement => f.write_str("the markets carry disagreeing mapTo ids"),
            Self::DescriptionMismatch => f.write_str("the paired descriptions differ"),
            Self::ParentMismatch => f.write_str("the paired node sits under a different subject"),
            Self::ParentUnpaired => f.write_str("the subject seeded no canonical term"),
            Self::DuplicatePath => f.write_str("two canonical terms name one target path"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Residue {
    pub gb_only: Vec<ResidueNode>,
    pub nz_only: Vec<ResidueNode>,
    pub mismatched: Vec<Mismatch>,
}

/// The canonical terms and their edges into both market vocabularies, plus
/// everything the derivation refused to pair.
///
/// `terms` is ordered subjects first and topics second, each group ascending
/// by GB native id, so a seeding insert satisfies the parent foreign key in
/// one pass. `edges` follows the same order with a node's GB edge before its
/// NZ edge. `residue` is in capture order, with the reverse-uniqueness
/// refusals last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crosswalk {
    pub terms: Vec<CanonicalTerm>,
    pub edges: Vec<ProjectionEdge>,
    pub residue: Residue,
}

/// A malformed capture, refused wholesale rather than degraded to residue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrosswalkError {
    DuplicateKey { country: String, key: String },
}

impl core::fmt::Display for CrosswalkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DuplicateKey { country, key } => {
                write!(f, "{country} carries the key {key} twice")
            }
        }
    }
}

impl core::error::Error for CrosswalkError {}

/// Derives the canonical taxonomy and both market projections from the two
/// captures.
///
/// Every GB node seeds one canonical term whose id is UUIDv5 over
/// `tes:{gb_native_id}`, so a reseed is idempotent. The GB native id is the
/// name because it is total over the seeding domain and durable, where
/// `mapTo` is measurably not total: the `Languages` subject carries an empty
/// one in both captures.
///
/// Nodes pair on the measured prefix transform — GB `1`+suffix against NZ
/// `7`+suffix — and a pair holds when the descriptions are byte-identical,
/// any two non-empty `mapTo` lists agree, and, for a topic, it sits under the
/// pair of this topic's subject. Failing any of the three yields the GB edge
/// alone and a residue record, so the projection reads `Absent` rather than
/// wrong.
///
/// A last pass withdraws every edge onto a path two terms both claim. Tes
/// carries such a pair (two distinct `Whole school` topics share one
/// description), and emitting both would violate the reverse-`Exact` law the
/// partial unique index enforces, so the seed would be rejected at insert.
/// Withdrawing both leaves the projection `Absent` and the choice with the
/// reconciliation queue.
pub fn derive_crosswalk(
    gb: &TesTree,
    nz: &TesTree,
    decided_at: Timestamp,
) -> Result<Crosswalk, CrosswalkError> {
    let gb_index = TreeIndex::build(gb, Market::Gb)?;
    let nz_index = TreeIndex::build(nz, Market::Nz)?;
    let mut out = Accumulator::new(&gb.source, &nz.source, decided_at);
    pair_subjects(gb, &nz_index, &mut out);
    pair_topics(gb, &nz_index, &mut out);
    collect_nz_residue(nz, &gb_index, &mut out);
    Ok(out.finish())
}

fn pair_subjects(gb: &TesTree, nz_index: &TreeIndex<'_>, out: &mut Accumulator<'_>) {
    for subject in &gb.subjects {
        let Some(key) = node_key(subject.id, Market::Gb) else {
            out.residue.mismatched.push(Mismatch {
                node: subject_residue(Market::Gb, subject),
                counterpart: None,
                reason: MismatchReason::NoMarketPrefix,
            });
            out.subjects.insert(subject.id, SubjectOutcome::Unseeded);
            continue;
        };
        let at = Placement {
            kind_rank: kind_rank(TermKind::Subject),
            native: subject.id,
        };
        let canonical = out.seed(&NodeSeed {
            kind: TermKind::Subject,
            at,
            parent: None,
            label: &subject.description,
        });
        out.route(canonical, at, Market::Gb, subject_path(Market::Gb, subject));

        let counterpart = nz_index.subjects.get(&key).copied();
        let Some(nz_subject) = counterpart else {
            out.residue
                .gb_only
                .push(subject_residue(Market::Gb, subject));
            out.subjects.insert(
                subject.id,
                SubjectOutcome::Seeded {
                    canonical,
                    nz_pair: None,
                },
            );
            continue;
        };
        if let Some(reason) = subject_defect(subject, nz_subject) {
            out.residue.mismatched.push(Mismatch {
                node: subject_residue(Market::Gb, subject),
                counterpart: Some(subject_residue(Market::Nz, nz_subject)),
                reason,
            });
            out.subjects.insert(
                subject.id,
                SubjectOutcome::Seeded {
                    canonical,
                    nz_pair: None,
                },
            );
            continue;
        }
        out.route(
            canonical,
            at,
            Market::Nz,
            subject_path(Market::Nz, nz_subject),
        );
        out.subjects.insert(
            subject.id,
            SubjectOutcome::Seeded {
                canonical,
                nz_pair: Some(nz_subject.id),
            },
        );
    }
}

fn pair_topics(gb: &TesTree, nz_index: &TreeIndex<'_>, out: &mut Accumulator<'_>) {
    for subject in &gb.subjects {
        let outcome = out.subjects.get(&subject.id).copied();
        for topic in subject.children() {
            let Some(key) = node_key(topic.id, Market::Gb) else {
                out.residue.mismatched.push(Mismatch {
                    node: topic_residue(Market::Gb, topic),
                    counterpart: None,
                    reason: MismatchReason::NoMarketPrefix,
                });
                continue;
            };
            let Some(SubjectOutcome::Seeded { canonical, nz_pair }) = outcome else {
                out.residue.mismatched.push(Mismatch {
                    node: topic_residue(Market::Gb, topic),
                    counterpart: None,
                    reason: MismatchReason::ParentUnpaired,
                });
                continue;
            };
            let at = Placement {
                kind_rank: kind_rank(TermKind::Topic),
                native: topic.id,
            };
            let seeded = out.seed(&NodeSeed {
                kind: TermKind::Topic,
                at,
                parent: Some(canonical),
                label: &topic.description,
            });
            out.route(
                seeded,
                at,
                Market::Gb,
                topic_path(Market::Gb, subject, topic),
            );

            let Some(&(nz_subject, nz_topic)) = nz_index.topics.get(&key) else {
                out.residue.gb_only.push(topic_residue(Market::Gb, topic));
                continue;
            };
            if let Some(reason) = topic_defect(topic, nz_topic, nz_pair, nz_subject.id) {
                out.residue.mismatched.push(Mismatch {
                    node: topic_residue(Market::Gb, topic),
                    counterpart: Some(topic_residue(Market::Nz, nz_topic)),
                    reason,
                });
                continue;
            }
            out.route(
                seeded,
                at,
                Market::Nz,
                topic_path(Market::Nz, nz_subject, nz_topic),
            );
        }
    }
}

fn collect_nz_residue(nz: &TesTree, gb_index: &TreeIndex<'_>, out: &mut Accumulator<'_>) {
    for subject in &nz.subjects {
        match node_key(subject.id, Market::Nz) {
            None => out.residue.mismatched.push(Mismatch {
                node: subject_residue(Market::Nz, subject),
                counterpart: None,
                reason: MismatchReason::NoMarketPrefix,
            }),
            Some(key) if !gb_index.subjects.contains_key(&key) => {
                out.residue
                    .nz_only
                    .push(subject_residue(Market::Nz, subject));
            }
            Some(_) => {}
        }
        for topic in subject.children() {
            match node_key(topic.id, Market::Nz) {
                None => out.residue.mismatched.push(Mismatch {
                    node: topic_residue(Market::Nz, topic),
                    counterpart: None,
                    reason: MismatchReason::NoMarketPrefix,
                }),
                Some(key) if !gb_index.topics.contains_key(&key) => {
                    out.residue.nz_only.push(topic_residue(Market::Nz, topic));
                }
                Some(_) => {}
            }
        }
    }
}

fn subject_defect(gb: &TesSubject, nz: &TesSubject) -> Option<MismatchReason> {
    if map_to_disagrees(&gb.map_to, &nz.map_to) {
        return Some(MismatchReason::MapToDisagreement);
    }
    if gb.description != nz.description {
        return Some(MismatchReason::DescriptionMismatch);
    }
    None
}

fn topic_defect(
    gb: &TesTopic,
    nz: &TesTopic,
    nz_pair: Option<u64>,
    nz_parent: u64,
) -> Option<MismatchReason> {
    if map_to_disagrees(&gb.map_to, &nz.map_to) {
        return Some(MismatchReason::MapToDisagreement);
    }
    if gb.description != nz.description {
        return Some(MismatchReason::DescriptionMismatch);
    }
    if nz_pair != Some(nz_parent) {
        return Some(MismatchReason::ParentMismatch);
    }
    None
}

/// `mapTo` validates a pair rather than forming it: an empty list on either
/// side blocks nothing, because Tes leaves it empty on a real node.
///
/// The two sides are compared against each other rather than against a
/// `"9"`-prefixed rendering of the GB id. That rendering holds for most nodes
/// but is measurably not a rule: 36 GB nodes, among them every `Languages`
/// language, carry a `mapTo` naming an id that appears nowhere in either tree.
/// Their NZ counterparts carry the same value, so the two markets agree and
/// the pairing is sound; requiring the derived form would send all 36 to
/// residue over a scheme Tes does not follow.
fn map_to_disagrees(gb: &[String], nz: &[String]) -> bool {
    !gb.is_empty() && !nz.is_empty() && gb != nz
}

const fn kind_rank(kind: TermKind) -> u8 {
    match kind {
        TermKind::Subject => 0,
        TermKind::Topic => 1,
        TermKind::ResourceType => 2,
        TermKind::Phase => 3,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Market {
    Gb,
    Nz,
}

impl Market {
    const fn rank(self) -> u8 {
        match self {
            Self::Gb => 0,
            Self::Nz => 1,
        }
    }

    const fn inventory(self) -> InventoryId {
        match self {
            Self::Gb => InventoryId::TesGb,
            Self::Nz => InventoryId::TesNz,
        }
    }

    /// The market's id prefix over the shared suffix, measured across all 43
    /// subjects by `docs/notes/probes/10-taxonomy-crosswalk.md`.
    const fn prefix(self) -> char {
        match self {
            Self::Gb => '1',
            Self::Nz => '7',
        }
    }
}

fn subject_path(market: Market, subject: &TesSubject) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(market.inventory(), TermKind::Subject),
        segments: vec![subject.description.clone()],
        native_id: Some(subject.id.to_string()),
    }
}

fn topic_path(market: Market, subject: &TesSubject, topic: &TesTopic) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(market.inventory(), TermKind::Topic),
        segments: vec![subject.description.clone(), topic.description.clone()],
        native_id: Some(topic.id.to_string()),
    }
}

/// A grouping key only, never a durable one: the idempotency ordinal in
/// tam-marketplace is the encoding that must never renumber.
const fn inventory_rank(inventory: InventoryId) -> u8 {
    match inventory {
        InventoryId::TesGb => 0,
        InventoryId::TesUs => 1,
        InventoryId::TesNz => 2,
        InventoryId::Etsy => 3,
        InventoryId::Tpt => 4,
    }
}

fn claim_key(to: &VocabularyPath) -> (u8, u8, Vec<String>) {
    (
        inventory_rank(to.vocabulary.0),
        kind_rank(to.vocabulary.1),
        to.segments.clone(),
    )
}

fn path_residue(to: &VocabularyPath) -> ResidueNode {
    ResidueNode {
        market: to.vocabulary.0,
        kind: to.vocabulary.1,
        native_id: to.native_id.clone().unwrap_or_default(),
        description: to.segments.last().cloned().unwrap_or_default(),
    }
}

fn subject_residue(market: Market, subject: &TesSubject) -> ResidueNode {
    ResidueNode {
        market: market.inventory(),
        kind: TermKind::Subject,
        native_id: subject.id.to_string(),
        description: subject.description.clone(),
    }
}

fn topic_residue(market: Market, topic: &TesTopic) -> ResidueNode {
    ResidueNode {
        market: market.inventory(),
        kind: TermKind::Topic,
        native_id: topic.id.to_string(),
        description: topic.description.clone(),
    }
}

/// The pairing key: the node's id with its market prefix stripped. The
/// measured prefix transform makes that suffix the one identity a GB and an NZ
/// node share, and it is total over both captures where `mapTo` is not.
fn node_key(id: u64, market: Market) -> Option<String> {
    id.to_string()
        .strip_prefix(market.prefix())
        .map(str::to_owned)
}

/// The canonical id is derived from the GB native id, which is total over the
/// seeding domain and durable, so reseeding is idempotent and no later capture
/// can re-key a term that already exists.
fn canonical_id(gb_native_id: u64) -> CanonicalTermId {
    let namespace = uuid::Uuid::from_bytes(NAMESPACE_TAM_TAXONOMY.0);
    let derived = uuid::Uuid::new_v5(&namespace, format!("tes:{gb_native_id}").as_bytes());
    CanonicalTermId(tam_types::Uuid(derived.into_bytes()))
}

struct TreeIndex<'a> {
    subjects: BTreeMap<String, &'a TesSubject>,
    topics: BTreeMap<String, (&'a TesSubject, &'a TesTopic)>,
}

impl<'a> TreeIndex<'a> {
    fn build(tree: &'a TesTree, market: Market) -> Result<Self, CrosswalkError> {
        let mut subjects = BTreeMap::new();
        let mut topics = BTreeMap::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for subject in &tree.subjects {
            if let Some(key) = node_key(subject.id, market) {
                if !seen.insert(key.clone()) {
                    return Err(duplicate(tree, &key));
                }
                subjects.insert(key, subject);
            }
            for topic in subject.children() {
                if let Some(key) = node_key(topic.id, market) {
                    if !seen.insert(key.clone()) {
                        return Err(duplicate(tree, &key));
                    }
                    topics.insert(key, (subject, topic));
                }
            }
        }
        Ok(Self { subjects, topics })
    }
}

fn duplicate(tree: &TesTree, key: &str) -> CrosswalkError {
    CrosswalkError::DuplicateKey {
        country: tree.country.clone(),
        key: key.to_owned(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubjectOutcome {
    Unseeded,
    Seeded {
        canonical: CanonicalTermId,
        nz_pair: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy)]
struct Placement {
    kind_rank: u8,
    native: u64,
}

struct NodeSeed<'a> {
    kind: TermKind,
    at: Placement,
    parent: Option<CanonicalTermId>,
    label: &'a str,
}

struct Accumulator<'a> {
    gb_source: &'a str,
    nz_source: &'a str,
    decided_at: Timestamp,
    terms: Vec<(u8, u64, CanonicalTerm)>,
    edges: Vec<(u8, u64, u8, ProjectionEdge)>,
    residue: Residue,
    subjects: BTreeMap<u64, SubjectOutcome>,
}

impl<'a> Accumulator<'a> {
    fn new(gb_source: &'a str, nz_source: &'a str, decided_at: Timestamp) -> Self {
        Self {
            gb_source,
            nz_source,
            decided_at,
            terms: Vec::new(),
            edges: Vec::new(),
            residue: Residue::default(),
            subjects: BTreeMap::new(),
        }
    }

    fn seed(&mut self, node: &NodeSeed<'_>) -> CanonicalTermId {
        let id = canonical_id(node.at.native);
        self.terms.push((
            node.at.kind_rank,
            node.at.native,
            CanonicalTerm {
                id,
                kind: node.kind,
                parent: node.parent,
                label: node.label.to_owned(),
            },
        ));
        id
    }

    fn route(&mut self, from: CanonicalTermId, at: Placement, market: Market, to: VocabularyPath) {
        let source = match market {
            Market::Gb => self.gb_source,
            Market::Nz => self.nz_source,
        };
        self.edges.push((
            at.kind_rank,
            at.native,
            market.rank(),
            ProjectionEdge {
                from,
                to,
                kind: EdgeKind::Exact,
                decided_by: Decider::Imported {
                    source: source.to_owned(),
                },
                decided_at: self.decided_at,
            },
        ));
    }

    fn finish(mut self) -> Crosswalk {
        self.terms
            .sort_by_key(|&(kind_rank, native, _)| (kind_rank, native));
        self.edges
            .sort_by_key(|&(kind_rank, native, market, _)| (kind_rank, native, market));

        let mut claims: BTreeMap<(u8, u8, Vec<String>), BTreeSet<[u8; 16]>> = BTreeMap::new();
        for (_, _, _, edge) in &self.edges {
            claims
                .entry(claim_key(&edge.to))
                .or_default()
                .insert(edge.from.0 .0);
        }

        let mut edges = Vec::with_capacity(self.edges.len());
        for (_, _, _, edge) in self.edges {
            let contested = claims
                .get(&claim_key(&edge.to))
                .is_some_and(|claimants| claimants.len() > 1);
            if contested {
                self.residue.mismatched.push(Mismatch {
                    node: path_residue(&edge.to),
                    counterpart: None,
                    reason: MismatchReason::DuplicatePath,
                });
            } else {
                edges.push(edge);
            }
        }

        Crosswalk {
            terms: self.terms.into_iter().map(|(_, _, term)| term).collect(),
            edges,
            residue: self.residue,
        }
    }
}

/// One row of the Tes main-age-range field, transcribed from the measured
/// uploader vocabulary in `docs/design/data/tes-vocabulary.json`. `16+` and
/// `Age not applicable` carry no bounds, which is the fact rather than a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TesAgeRange {
    pub native_id: &'static str,
    pub label: &'static str,
    pub bounds: Option<(u8, u8)>,
}

pub const TES_MAIN_AGE_RANGES: [TesAgeRange; 7] = [
    TesAgeRange {
        native_id: "1",
        label: "3-5",
        bounds: Some((3, 5)),
    },
    TesAgeRange {
        native_id: "2",
        label: "5-7",
        bounds: Some((5, 7)),
    },
    TesAgeRange {
        native_id: "3",
        label: "7-11",
        bounds: Some((7, 11)),
    },
    TesAgeRange {
        native_id: "4",
        label: "11-14",
        bounds: Some((11, 14)),
    },
    TesAgeRange {
        native_id: "5",
        label: "14-16",
        bounds: Some((14, 16)),
    },
    TesAgeRange {
        native_id: "6",
        label: "16+",
        bounds: None,
    },
    TesAgeRange {
        native_id: "7",
        label: "Age not applicable",
        bounds: None,
    },
];

/// Derives the age interval a declaration spans, and only when every declared
/// range carries both bounds. An empty declaration, an unrecognised native id
/// and an unbounded row each derive nothing rather than an invented number;
/// the declaration itself remains the fact that re-emission uses.
///
/// The table's bounded rows are all non-inverted, so `AgeInterval::new` cannot
/// reject the derived pair; `.ok()` carries that without asserting it.
#[must_use]
pub fn derive_interval(paths: &[VocabularyPath]) -> Option<AgeInterval> {
    if paths.is_empty() {
        return None;
    }
    let mut low = u8::MAX;
    let mut high = u8::MIN;
    for path in paths {
        let native = path.native_id.as_deref()?;
        let row = TES_MAIN_AGE_RANGES
            .iter()
            .find(|range| range.native_id == native)?;
        let (row_low, row_high) = row.bounds?;
        low = low.min(row_low);
        high = high.max(row_high);
    }
    AgeInterval::new(low, high).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        derive_crosswalk, derive_interval, parse_tree, CrosswalkError, MismatchReason, TesSubject,
        TesTopic, TesTree,
    };
    use tam_domain::{TermKind, VocabularyId, VocabularyPath};
    use tam_types::{InventoryId, Timestamp};

    const AT: Timestamp = Timestamp(1_700_000_000_000);

    fn topic(id: u64, description: &str, map_to: &[&str]) -> TesTopic {
        TesTopic {
            id,
            description: description.to_owned(),
            leaf_depth: Some("1".to_owned()),
            phases: Vec::new(),
            map_to: map_to.iter().map(|entry| (*entry).to_owned()).collect(),
            parent_id: None,
        }
    }

    fn subject(id: u64, description: &str, map_to: &[&str], topics: Vec<TesTopic>) -> TesSubject {
        TesSubject {
            id,
            description: description.to_owned(),
            leaf_depth: Some("0".to_owned()),
            phases: Vec::new(),
            map_to: map_to.iter().map(|entry| (*entry).to_owned()).collect(),
            parent_id: None,
            topics,
            error: None,
        }
    }

    fn tree(country: &str, subjects: Vec<TesSubject>) -> TesTree {
        let topic_count = subjects.iter().map(|entry| entry.topics.len()).sum();
        TesTree {
            source: format!("capture {country}"),
            country: country.to_owned(),
            subject_count: subjects.len(),
            topic_count,
            subjects,
        }
    }

    fn range(native_id: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Phase),
            segments: vec!["age".to_owned()],
            native_id: Some(native_id.to_owned()),
        }
    }

    #[test]
    fn a_matching_pair_seeds_one_term_and_both_edges() {
        let gb = tree(
            "GB",
            vec![subject(1_000_454, "Maths", &["91000454"], vec![])],
        );
        let nz = tree(
            "NZ",
            vec![subject(7_000_454, "Maths", &["91000454"], vec![])],
        );
        let crosswalk =
            derive_crosswalk(&gb, &nz, AT).expect("the synthetic captures are well formed");
        assert_eq!(crosswalk.terms.len(), 1);
        assert_eq!(crosswalk.edges.len(), 2, "one edge into each market");
        assert_eq!(crosswalk.residue, super::Residue::default());
        assert_eq!(
            crosswalk.edges[0].to.vocabulary,
            VocabularyId(InventoryId::TesGb, TermKind::Subject),
            "the GB edge sorts before the NZ edge"
        );
        assert_eq!(
            crosswalk.edges[1].to.native_id.as_deref(),
            Some("7000454"),
            "each market's edge carries that market's own native id"
        );
    }

    #[test]
    fn a_map_to_disagreement_keeps_the_gb_edge_alone() {
        let gb = tree(
            "GB",
            vec![subject(1_000_454, "Maths", &["91000454"], vec![])],
        );
        let nz = tree(
            "NZ",
            vec![subject(7_000_454, "Maths", &["91000999"], vec![])],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(crosswalk.terms.len(), 1);
        assert_eq!(crosswalk.edges.len(), 1, "no NZ edge on a drifted capture");
        assert_eq!(
            crosswalk.residue.mismatched[0].reason,
            MismatchReason::MapToDisagreement
        );
        assert_eq!(
            crosswalk.residue.mismatched[0]
                .counterpart
                .as_ref()
                .map(|node| node.native_id.as_str()),
            Some("7000454"),
            "the residue names the node it refused to pair with"
        );
    }

    #[test]
    fn an_empty_map_to_blocks_no_pairing() {
        let gb = tree("GB", vec![subject(1_000_454, "Maths", &[], vec![])]);
        let nz = tree(
            "NZ",
            vec![subject(7_000_454, "Maths", &["91000454"], vec![])],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(
            crosswalk.edges.len(),
            2,
            "mapTo validates a pair rather than forming it"
        );
        assert_eq!(crosswalk.residue, super::Residue::default());
    }

    #[test]
    fn a_description_mismatch_keeps_the_gb_edge_alone() {
        let gb = tree(
            "GB",
            vec![subject(1_000_454, "Maths", &["91000454"], vec![])],
        );
        let nz = tree(
            "NZ",
            vec![subject(7_000_454, "Mathematics", &["91000454"], vec![])],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(crosswalk.edges.len(), 1);
        assert_eq!(
            crosswalk.residue.mismatched[0].reason,
            MismatchReason::DescriptionMismatch
        );
    }

    #[test]
    fn a_topic_pairing_across_subjects_is_a_parent_mismatch() {
        let gb = tree(
            "GB",
            vec![subject(
                1_000_454,
                "Maths",
                &["91000454"],
                vec![topic(1_000_732, "Time", &["91000732"])],
            )],
        );
        let nz = tree(
            "NZ",
            vec![
                subject(7_000_454, "Maths", &["91000454"], vec![]),
                subject(
                    7_000_500,
                    "Science",
                    &["91000500"],
                    vec![topic(7_000_732, "Time", &["91000732"])],
                ),
            ],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(
            crosswalk.residue.mismatched[0].reason,
            MismatchReason::ParentMismatch,
            "the two trees disagree about which subject the topic sits under"
        );
        assert_eq!(
            crosswalk.edges.len(),
            3,
            "the subject pairs and the topic keeps its GB edge alone"
        );
    }

    #[test]
    fn a_subject_without_a_map_to_pairs_on_its_id_suffix() {
        let gb = tree(
            "GB",
            vec![subject(
                1_100_000,
                "Languages",
                &[],
                vec![topic(1_000_015, "Arabic", &["91000015"])],
            )],
        );
        let nz = tree(
            "NZ",
            vec![subject(
                7_100_000,
                "Languages",
                &[],
                vec![topic(7_000_015, "Arabic", &["91000015"])],
            )],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(
            crosswalk.terms.len(),
            2,
            "the shared id suffix is the identity, and it is total where mapTo is not"
        );
        assert_eq!(
            crosswalk.edges.len(),
            4,
            "the subject and its topic each reach both markets"
        );
        assert_eq!(
            crosswalk.residue,
            super::Residue::default(),
            "an empty mapTo is a stable Tes fact, not a drifted capture"
        );
    }

    #[test]
    fn the_canonical_id_is_the_gb_native_id() {
        let gb = tree(
            "GB",
            vec![
                subject(1_100_000, "Languages", &[], vec![]),
                subject(1_000_454, "Maths", &["91000454"], vec![]),
            ],
        );
        let nz = tree("NZ", vec![]);
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        let namespace = uuid::Uuid::from_bytes(tam_types::NAMESPACE_TAM_TAXONOMY.0);
        for (index, native) in [(0_usize, "1000454"), (1, "1100000")] {
            let expected = uuid::Uuid::new_v5(&namespace, format!("tes:{native}").as_bytes());
            assert_eq!(
                crosswalk.terms[index].id.0 .0,
                expected.into_bytes(),
                "one naming rule over every node, mapTo or not"
            );
        }
    }

    #[test]
    fn a_node_with_no_market_prefix_seeds_nothing() {
        let gb = tree("GB", vec![subject(9_100_000, "Languages", &[], vec![])]);
        let nz = tree("NZ", vec![]);
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert!(
            crosswalk.terms.is_empty(),
            "an id without a market prefix yields no suffix to pair on"
        );
        assert_eq!(
            crosswalk.residue.mismatched[0].reason,
            MismatchReason::NoMarketPrefix
        );
    }

    #[test]
    fn a_topic_under_a_prefixless_subject_is_unparented() {
        let gb = tree(
            "GB",
            vec![subject(
                9_100_000,
                "Languages",
                &[],
                vec![topic(1_000_015, "Arabic", &["91000015"])],
            )],
        );
        let nz = tree("NZ", vec![]);
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        let reasons: Vec<MismatchReason> = crosswalk
            .residue
            .mismatched
            .iter()
            .map(|entry| entry.reason)
            .collect();
        assert_eq!(
            reasons,
            vec![
                MismatchReason::NoMarketPrefix,
                MismatchReason::ParentUnpaired
            ],
            "a topic whose subject seeded no term has no parent to hang from"
        );
    }

    #[test]
    fn seeding_totality_is_independent_of_pairing() {
        let gb = tree(
            "GB",
            vec![
                subject(1_000_454, "Maths", &["91000454"], vec![]),
                subject(1_000_500, "Science", &["91000500"], vec![]),
                subject(1_000_600, "History", &["91000600"], vec![]),
            ],
        );
        let nz = tree(
            "NZ",
            vec![
                subject(7_000_454, "Maths", &["91000454"], vec![]),
                subject(7_000_600, "Histoire", &["91000600"], vec![]),
            ],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(
            crosswalk.terms.len(),
            3,
            "naming on the GB id decouples seeding from pairing entirely"
        );
        let gb_edges = crosswalk
            .edges
            .iter()
            .filter(|edge| edge.to.vocabulary.0 == InventoryId::TesGb)
            .count();
        assert_eq!(
            gb_edges, 3,
            "an unpairable GB node still carries its GB edge"
        );
        assert_eq!(
            crosswalk.edges.len(),
            4,
            "only the pairing subject reaches NZ"
        );

        let target = VocabularyId(InventoryId::TesNz, TermKind::Subject);
        for (index, expected_pairing) in [(0_usize, true), (1, false), (2, false)] {
            let projected = crate::project(crosswalk.terms[index].id, target, &crosswalk.edges);
            assert_eq!(
                projected == tam_domain::TermProjection::Absent,
                !expected_pairing,
                "an unpaired term projects Absent and raises a reconciliation item"
            );
        }
    }

    #[test]
    fn a_map_to_that_is_not_the_gb_id_is_no_defect() {
        let gb = tree(
            "GB",
            vec![subject(1_100_025, "Afrikaans", &["91002025"], vec![])],
        );
        let nz = tree(
            "NZ",
            vec![subject(7_100_025, "Afrikaans", &["91002025"], vec![])],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(
            crosswalk.edges.len(),
            2,
            "36 real GB nodes carry a mapTo unrelated to their own id, and the \
             markets agree on it"
        );
        assert_eq!(crosswalk.residue, super::Residue::default());
    }

    #[test]
    fn a_gb_only_node_keeps_its_gb_edge() {
        let gb = tree(
            "GB",
            vec![subject(1_000_454, "Maths", &["91000454"], vec![])],
        );
        let nz = tree("NZ", vec![]);
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(crosswalk.terms.len(), 1);
        assert_eq!(crosswalk.edges.len(), 1);
        assert_eq!(crosswalk.residue.gb_only.len(), 1);
        assert_eq!(crosswalk.residue.gb_only[0].market, InventoryId::TesGb);
    }

    #[test]
    fn an_nz_only_node_seeds_nothing() {
        let gb = tree("GB", vec![]);
        let nz = tree(
            "NZ",
            vec![subject(7_000_454, "Maths", &["91000454"], vec![])],
        );
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert!(crosswalk.terms.is_empty(), "the canonical seed is GB's");
        assert!(crosswalk.edges.is_empty());
        assert_eq!(crosswalk.residue.nz_only.len(), 1);
    }

    #[test]
    fn a_duplicate_key_refuses_the_capture() {
        let gb = tree(
            "GB",
            vec![
                subject(1_000_454, "Maths", &["91000454"], vec![]),
                subject(1_000_454, "Numeracy", &["91000455"], vec![]),
            ],
        );
        let nz = tree("NZ", vec![]);
        assert_eq!(
            derive_crosswalk(&gb, &nz, AT),
            Err(CrosswalkError::DuplicateKey {
                country: "GB".to_owned(),
                key: "000454".to_owned()
            }),
            "a malformed file is refused rather than degraded to residue"
        );
    }

    #[test]
    fn the_order_is_subjects_then_topics_by_native_id() {
        let gb = tree(
            "GB",
            vec![
                subject(
                    1_100_000,
                    "Languages",
                    &["91100000"],
                    vec![topic(1_000_015, "Arabic", &["91000015"])],
                ),
                subject(1_000_454, "Maths", &["91000454"], vec![]),
            ],
        );
        let nz = tree("NZ", vec![]);
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        let labels: Vec<&str> = crosswalk
            .terms
            .iter()
            .map(|term| term.label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["Maths", "Languages", "Arabic"],
            "subjects ascending by native id, then topics, so a parent inserts first"
        );
        assert_eq!(crosswalk.terms[2].kind, TermKind::Topic);
        assert_eq!(
            crosswalk.terms[2].parent,
            Some(crosswalk.terms[1].id),
            "the topic hangs from its own subject"
        );
    }

    #[test]
    fn the_derivation_is_reproducible() {
        let gb = tree(
            "GB",
            vec![subject(
                1_000_454,
                "Maths",
                &["91000454"],
                vec![topic(1_000_732, "Time", &["91000732"])],
            )],
        );
        let nz = tree(
            "NZ",
            vec![subject(
                7_000_454,
                "Maths",
                &["91000454"],
                vec![topic(7_000_732, "Time", &["91000732"])],
            )],
        );
        let first = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        let again = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(first, again, "reseeding must be idempotent");
    }

    #[test]
    fn an_errored_subject_is_treated_as_childless() {
        let mut errored = subject(
            1_000_454,
            "Maths",
            &["91000454"],
            vec![topic(1_000_732, "Time", &["91000732"])],
        );
        errored.error = Some("504 from the taxonomy API".to_owned());
        let gb = tree("GB", vec![errored]);
        let nz = tree("NZ", vec![]);
        let crosswalk = derive_crosswalk(&gb, &nz, AT).expect("well formed");
        assert_eq!(
            crosswalk.terms.len(),
            1,
            "an unread child list is not an empty one"
        );
    }

    #[test]
    fn the_capture_shape_parses() {
        let parsed = parse_tree(
            r#"{"_source":"crawl","country":"GB","subjectCount":1,"topicCount":1,
                "subjects":[{"id":1000454,"description":"Maths","leafDepth":"0",
                "phases":["primary"],"mapTo":["91000454"],"parentId":null,
                "topics":[{"id":1000732,"description":"Time","leafDepth":"1",
                "phases":[],"mapTo":["91000732"],"parentId":1000454}]}]}"#,
        )
        .expect("the captured shape parses");
        assert_eq!(parsed.source, "crawl");
        assert_eq!(parsed.subjects[0].topics[0].description, "Time");
        assert_eq!(parsed.subjects[0].error, None);
    }

    #[test]
    fn the_interval_spans_every_declared_row() {
        assert_eq!(
            derive_interval(&[range("2"), range("4")])
                .map(|span| (span.low_years(), span.high_years())),
            Some((5, 14)),
            "the lowest low and the highest high"
        );
    }

    #[test]
    fn an_unbounded_row_derives_nothing() {
        assert_eq!(
            derive_interval(&[range("2"), range("6")]),
            None,
            "16+ has no upper bound and the derivation refuses to invent one"
        );
        assert_eq!(
            derive_interval(&[range("7")]),
            None,
            "age not applicable is a fact, not a range"
        );
    }

    #[test]
    fn an_unknown_native_id_derives_nothing() {
        assert_eq!(derive_interval(&[range("2"), range("99")]), None);
    }

    #[test]
    fn a_declaration_without_a_native_id_derives_nothing() {
        let mut anonymous = range("2");
        anonymous.native_id = None;
        assert_eq!(derive_interval(&[anonymous]), None);
    }

    #[test]
    fn an_empty_declaration_derives_nothing() {
        assert_eq!(derive_interval(&[]), None);
    }

    #[test]
    fn the_age_table_is_the_measured_vocabulary() {
        let labels: Vec<&str> = super::TES_MAIN_AGE_RANGES
            .iter()
            .map(|row| row.label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "3-5",
                "5-7",
                "7-11",
                "11-14",
                "14-16",
                "16+",
                "Age not applicable"
            ],
            "transcribed from docs/design/data/tes-vocabulary.json"
        );
    }
}
