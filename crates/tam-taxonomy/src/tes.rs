//! The captured Tes tree, the canonical terms derived from it, and the Tes
//! main-age-range table.
//!
//! Tes is one inventory since 2026-09-12 (`docs/design/decisions.md`, "Tes is
//! one marketplace with no regions"), so the taxonomy is the GB tree and the
//! GB-to-NZ pairing that used to live here is deleted. The NZ capture is kept
//! as evidence under `docs/design/data/historical/`, not as seed data.

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

/// A node whose edge the derivation withdrew, kept so the seed report names
/// it.
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
    pub reason: MismatchReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchReason {
    /// Two canonical terms name one target path, which the reverse-`Exact` law
    /// forbids. Neither edge is emitted, because choosing between them is the
    /// decision the design reserves for a human.
    DuplicatePath,
}

impl core::fmt::Display for MismatchReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DuplicatePath => f.write_str("two canonical terms name one target path"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Residue {
    pub mismatched: Vec<Mismatch>,
}

/// The canonical terms and their edges into the Tes vocabulary, plus
/// everything the derivation refused to emit.
///
/// `terms` is ordered subjects first and topics second, each group ascending
/// by native id, so a seeding insert satisfies the parent foreign key in one
/// pass. `edges` follows the same order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crosswalk {
    pub terms: Vec<CanonicalTerm>,
    pub edges: Vec<ProjectionEdge>,
    pub residue: Residue,
}

/// Derives the canonical taxonomy and the Tes projection from the capture.
///
/// Every node seeds one canonical term whose id is UUIDv5 over
/// `tes:{native_id}`, so a reseed is idempotent. The native id is the name
/// because it is total over the seeding domain and durable, where `mapTo` is
/// measurably not total: the `Languages` subject carries an empty one.
///
/// A last pass withdraws every edge onto a path two terms both claim. Tes
/// carries such a pair (two distinct `Whole school` topics share one
/// description), and emitting both would violate the reverse-`Exact` law the
/// partial unique index enforces, so the seed would be rejected at insert.
/// Withdrawing both leaves the projection `Absent` and the choice with the
/// reconciliation queue.
#[must_use]
pub fn derive_crosswalk(tree: &TesTree, decided_at: Timestamp) -> Crosswalk {
    let mut out = Accumulator::new(&tree.source, decided_at);
    for subject in &tree.subjects {
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
        out.route(canonical, at, subject_path(subject));
        for topic in subject.children() {
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
            out.route(seeded, at, topic_path(subject, topic));
        }
    }
    out.finish()
}

const fn kind_rank(kind: TermKind) -> u8 {
    match kind {
        TermKind::Subject => 0,
        TermKind::Topic => 1,
        TermKind::ResourceType => 2,
        TermKind::Phase => 3,
        TermKind::Licence => 4,
    }
}

fn subject_path(subject: &TesSubject) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tes, TermKind::Subject),
        segments: vec![subject.description.clone()],
        native_id: Some(subject.id.to_string()),
    }
}

fn topic_path(subject: &TesSubject, topic: &TesTopic) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tes, TermKind::Topic),
        segments: vec![subject.description.clone(), topic.description.clone()],
        native_id: Some(topic.id.to_string()),
    }
}

/// A grouping key only, never a durable one: the idempotency ordinal in
/// tam-marketplace is the encoding that must never renumber.
const fn inventory_rank(inventory: InventoryId) -> u8 {
    match inventory {
        InventoryId::Tes => 0,
        InventoryId::Etsy => 1,
        InventoryId::Tpt => 2,
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

/// The canonical id is derived from the Tes native id, which is total over the
/// seeding domain and durable, so reseeding is idempotent and no later capture
/// can re-key a term that already exists.
pub(crate) fn canonical_id(native_id: u64) -> CanonicalTermId {
    let namespace = uuid::Uuid::from_bytes(NAMESPACE_TAM_TAXONOMY.0);
    let derived = uuid::Uuid::new_v5(&namespace, format!("tes:{native_id}").as_bytes());
    CanonicalTermId(tam_types::Uuid(derived.into_bytes()))
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
    source: &'a str,
    decided_at: Timestamp,
    terms: Vec<(u8, u64, CanonicalTerm)>,
    edges: Vec<(u8, u64, ProjectionEdge)>,
    residue: Residue,
}

impl<'a> Accumulator<'a> {
    const fn new(source: &'a str, decided_at: Timestamp) -> Self {
        Self {
            source,
            decided_at,
            terms: Vec::new(),
            edges: Vec::new(),
            residue: Residue {
                mismatched: Vec::new(),
            },
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

    fn route(&mut self, from: CanonicalTermId, at: Placement, to: VocabularyPath) {
        self.edges.push((
            at.kind_rank,
            at.native,
            ProjectionEdge {
                from,
                to,
                kind: EdgeKind::Exact,
                decided_by: Decider::Imported {
                    source: self.source.to_owned(),
                },
                decided_at: self.decided_at,
            },
        ));
    }

    fn finish(mut self) -> Crosswalk {
        self.terms
            .sort_by_key(|&(kind_rank, native, _)| (kind_rank, native));
        self.edges
            .sort_by_key(|&(kind_rank, native, _)| (kind_rank, native));

        let mut claims: BTreeMap<(u8, u8, Vec<String>), BTreeSet<[u8; 16]>> = BTreeMap::new();
        for (_, _, edge) in &self.edges {
            claims
                .entry(claim_key(&edge.to))
                .or_default()
                .insert(edge.from.0 .0);
        }

        let mut edges = Vec::with_capacity(self.edges.len());
        for (_, _, edge) in self.edges {
            let contested = claims
                .get(&claim_key(&edge.to))
                .is_some_and(|claimants| claimants.len() > 1);
            if contested {
                self.residue.mismatched.push(Mismatch {
                    node: path_residue(&edge.to),
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

/// What one age row bounds. `16+` and `Age not applicable` are different
/// facts and an `Option<(u8, u8)>` records them as the same one: the JSON has
/// `ageLow: 16, ageHigh: null` for the first and nulls for both on the
/// second, so collapsing them loses a lower bound the vocabulary states.
///
/// A half-open band derives no interval — `AgeInterval` is a closed pair and
/// there is no measured upper age to close it with — so this type is read by
/// the crosswalk's covering computation, which needs the lower bound, and not
/// by `derive_interval`, which needs the pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeBounds {
    Between { low: u8, high: u8 },
    From { low: u8 },
    NotApplicable,
}

impl AgeBounds {
    /// The lowest age the band admits, where it states one.
    #[must_use]
    pub const fn low(self) -> Option<u8> {
        match self {
            Self::Between { low, .. } | Self::From { low } => Some(low),
            Self::NotApplicable => None,
        }
    }
}

/// One row of the Tes main-age-range field, transcribed from the measured
/// uploader vocabulary in `docs/design/data/tes-vocabulary.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TesAgeRange {
    pub native_id: &'static str,
    pub label: &'static str,
    pub bounds: AgeBounds,
}

pub const TES_MAIN_AGE_RANGES: [TesAgeRange; 7] = [
    TesAgeRange {
        native_id: "1",
        label: "3-5",
        bounds: AgeBounds::Between { low: 3, high: 5 },
    },
    TesAgeRange {
        native_id: "2",
        label: "5-7",
        bounds: AgeBounds::Between { low: 5, high: 7 },
    },
    TesAgeRange {
        native_id: "3",
        label: "7-11",
        bounds: AgeBounds::Between { low: 7, high: 11 },
    },
    TesAgeRange {
        native_id: "4",
        label: "11-14",
        bounds: AgeBounds::Between { low: 11, high: 14 },
    },
    TesAgeRange {
        native_id: "5",
        label: "14-16",
        bounds: AgeBounds::Between { low: 14, high: 16 },
    },
    TesAgeRange {
        native_id: "6",
        label: "16+",
        bounds: AgeBounds::From { low: 16 },
    },
    TesAgeRange {
        native_id: "7",
        label: "Age not applicable",
        bounds: AgeBounds::NotApplicable,
    },
];

/// The vocabulary `TES_MAIN_AGE_RANGES` is the membership of. Only a GB
/// resource's phase values are drawn from it.
const TES_MAIN_AGE_VOCABULARY: VocabularyId = VocabularyId(InventoryId::Tes, TermKind::Phase);

/// Derives the age interval a declaration spans, and only when every declared
/// range is closed. An empty declaration, an unrecognised native id, a
/// half-open band and the not-applicable row each derive nothing rather than
/// an invented number; the declaration itself remains the fact that
/// re-emission uses.
///
/// `From` is refused here on purpose even though the band states its lower
/// bound. This interval is the canonical claim about the ages a resource
/// suits, and a `16+` band states no upper one.
///
/// What Tes posts is a separate question and no longer reads this value: the
/// adapter derives the literal `ages` array from the declared bands' own
/// published age sets, where band 6 is closed at 18. A band closed on the
/// wire is not thereby closed as a claim about who the resource is for.
///
/// A declaration is read off this table only where it names the vocabulary the
/// table is the members of. The Tes editor takes `ageRanges` when the country
/// is GB and `yearGroups` otherwise, so `VocabularyId(Tes, Phase)` denotes
/// these seven bands while the same small integers under any other inventory
/// denote year groups: US year group 3 is grade 1, and row 3 here is ages
/// 7-11. A path from any other vocabulary derives nothing rather than a band
/// it never named.
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
        if path.vocabulary != TES_MAIN_AGE_VOCABULARY {
            return None;
        }
        let native = path.native_id.as_deref()?;
        let row = TES_MAIN_AGE_RANGES
            .iter()
            .find(|range| range.native_id == native)?;
        let AgeBounds::Between {
            low: row_low,
            high: row_high,
        } = row.bounds
        else {
            return None;
        };
        low = low.min(row_low);
        high = high.max(row_high);
    }
    AgeInterval::new(low, high).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_id, derive_crosswalk, derive_interval, parse_tree, AgeBounds, MismatchReason,
        TesSubject, TesTopic, TesTree,
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
            vocabulary: VocabularyId(InventoryId::Tes, TermKind::Phase),
            segments: vec!["age".to_owned()],
            native_id: Some(native_id.to_owned()),
        }
    }

    #[test]
    fn every_node_seeds_one_term_and_one_edge() {
        let tes = tree(
            "GB",
            vec![subject(
                1_000_454,
                "Maths",
                &["91000454"],
                vec![topic(1_000_732, "Time", &["91000732"])],
            )],
        );
        let crosswalk = derive_crosswalk(&tes, AT);
        assert_eq!(crosswalk.terms.len(), 2);
        assert_eq!(crosswalk.edges.len(), 2);
        assert_eq!(crosswalk.residue, super::Residue::default());
        assert_eq!(
            crosswalk.edges[0].to.vocabulary,
            VocabularyId(InventoryId::Tes, TermKind::Subject)
        );
        assert_eq!(
            crosswalk.edges[1].to.native_id.as_deref(),
            Some("1000732"),
            "the edge carries the node's own native id"
        );
    }

    #[test]
    fn the_canonical_id_is_the_tes_native_id() {
        let tes = tree(
            "GB",
            vec![subject(1_000_454, "Maths", &["91000454"], vec![])],
        );
        let crosswalk = derive_crosswalk(&tes, AT);
        assert_eq!(
            crosswalk.terms[0].id,
            canonical_id(1_000_454),
            "the native id names the term, so a reseed is a no-op"
        );
    }

    #[test]
    fn two_nodes_claiming_one_path_withdraw_both_edges() {
        let tes = tree(
            "GB",
            vec![subject(
                1_000_454,
                "Maths",
                &["91000454"],
                vec![
                    topic(1_001_345, "Whole school", &[]),
                    topic(1_001_347, "Whole school", &[]),
                ],
            )],
        );
        let crosswalk = derive_crosswalk(&tes, AT);
        assert_eq!(
            crosswalk.terms.len(),
            3,
            "both topics still seed a term; only their edges are withdrawn"
        );
        assert_eq!(
            crosswalk.edges.len(),
            1,
            "the subject keeps its edge and neither contesting topic does"
        );
        let reasons: Vec<MismatchReason> = crosswalk
            .residue
            .mismatched
            .iter()
            .map(|entry| entry.reason)
            .collect();
        assert_eq!(
            reasons,
            vec![MismatchReason::DuplicatePath; 2],
            "the reverse-Exact law admits one claimant per path, so the queue decides"
        );
    }

    #[test]
    fn the_order_is_subjects_then_topics_by_native_id() {
        let tes = tree(
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
        let crosswalk = derive_crosswalk(&tes, AT);
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
        let tes = tree(
            "GB",
            vec![subject(
                1_000_454,
                "Maths",
                &["91000454"],
                vec![topic(1_000_732, "Time", &["91000732"])],
            )],
        );
        assert_eq!(
            derive_crosswalk(&tes, AT),
            derive_crosswalk(&tes, AT),
            "reseeding must be idempotent"
        );
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
        let crosswalk = derive_crosswalk(&tree("GB", vec![errored]), AT);
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
    fn a_phase_from_another_vocabulary_never_derives_a_tes_age_band() {
        let elsewhere = |native: &str| VocabularyPath {
            vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
            segments: vec!["grade".to_owned()],
            native_id: Some(native.to_owned()),
        };
        assert_eq!(
            derive_interval(&[elsewhere("3")]),
            None,
            "the same small integer denotes a TPT grade there and an age band here"
        );
        assert_eq!(
            derive_interval(&[range("3"), elsewhere("4")]),
            None,
            "one foreign member is enough: the span would be read off two vocabularies"
        );
        assert_eq!(
            derive_interval(&[range("3")]).map(|span| (span.low_years(), span.high_years())),
            Some((7, 11)),
            "the Tes age-range path still derives its own band"
        );
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
    fn a_half_open_band_states_its_lower_bound_and_a_missing_one_states_nothing() {
        let bounds = |native: &str| {
            super::TES_MAIN_AGE_RANGES
                .iter()
                .find(|row| row.native_id == native)
                .map(|row| row.bounds)
        };
        assert_eq!(
            bounds("6"),
            Some(AgeBounds::From { low: 16 }),
            "the vocabulary records ageLow 16 with a null high, so 16+ has a lower bound"
        );
        assert_eq!(
            bounds("7"),
            Some(AgeBounds::NotApplicable),
            "both bounds are null, which is a different fact from a half-open band"
        );
        assert_eq!(
            (
                bounds("6").and_then(AgeBounds::low),
                bounds("7").and_then(AgeBounds::low)
            ),
            (Some(16), None),
            "the two rows an Option<(u8, u8)> collapsed now read apart"
        );
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
