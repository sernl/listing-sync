//! The subject and topic axes, re-based on TPT.
//!
//! The phase axis was always TPT's and the grade derivation inverted when
//! `grades.rs` landed. Subject and topic are the half that had not: every
//! canonical subject and topic id is UUIDv5 over `tes:{gb_native_id}` and the
//! whole relation is derived from the two Tes captures alone. This module is
//! that inversion.
//!
//! It does not replace `tes::derive_crosswalk`. It consumes its output,
//! because the GB-to-NZ half is still needed, the inbound direction still
//! ingests Tes ids, and every canonical id that relation minted is referenced
//! from `projection_edge`, `projection_no_counterpart` and
//! `reconciliation_item`. Identity is the one thing this pass may not move: a
//! facet denoting a Tes node reuses that node's canonical term, and only a
//! facet denoting no single node mints an id of its own. What does flip is
//! label authority, which costs nothing because the seeder re-seeds with `ON
//! CONFLICT (id) DO UPDATE SET label`.
//!
//! The pairing between TPT's 140 `PreK-12-Subject-Area` facets and Tes's 43
//! subjects and 453 topics is authored, as `GRADE_PAIRS` is authored, and
//! lives in `docs/design/data/tpt-tes-subject-pairs.json` rather than in a
//! const so that a founder amending a row edits data and re-runs. Only the
//! twenty roots are authored. Each root's row states the root's whole
//! denotation, and its children are paired by normalised description against
//! the Tes nodes inside that denotation, so the 120 children cost no
//! authoring and a child matching nothing degrades to residue rather than to
//! a guessed edge.
//!
//! Two facts about the captures shape the rest, and neither is what a
//! hierarchy-aligned reading would predict. The levels cross: eight TPT
//! *children* denote a Tes *subject* (`biology`, `chemistry`, `physics`,
//! `geography`, `economics`, `psychology`, `drama`, `music`) while only
//! `physical-education` denotes one from the root, and two TPT roots
//! (`health`, `speaking-and-listening`) denote Tes *topics*. A facet's
//! canonical `TermKind` is therefore taken from what it denotes rather than
//! from its `parentId` depth, which TPT's own wire permits because its
//! Subject and Topic axes bind one flat `taxonomyTags` field. And a child's
//! exact claim withdraws its root's broader claim on the same node, because a
//! term holding an `Exact` and a `Broader` edge onto two different TPT paths
//! projects `Ambiguous` and blocks.
//!
//! A hidden facet seeds nothing at all: it reads back on existing products
//! and is never offered on create, so an `Exact` edge would post a retired
//! slug on the next publish, and the alternative — a term reachable from
//! neither direction — would deliver nothing. Its slug arrives as an
//! unrecognised path and is retained verbatim on the product, which loses no
//! information, and because a minted id is derived from the slug, un-hiding a
//! facet later mints the same id it would have had now.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, NoCounterpart, ProjectionEdge, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_types::{CanonicalTermId, InventoryId, Timestamp};

use crate::grades::{derived, TaxonomyTag, TptVocabulary};
use crate::tes::{
    canonical_id, derive_crosswalk, Crosswalk, Residue, ResidueNode, TesSubject, TesTopic, TesTree,
};

/// The category whose facets this derivation reads. TPT files a grade, an
/// audience and a file format in the same flat namespace, so the axis is a
/// fact of the facet's own `category`.
const SUBJECT_CATEGORY: &str = "PreK-12-Subject-Area";

/// The canonical subject and topic axes as the three captures and the
/// authored pairing state them: everything `derive_crosswalk` produced, plus
/// the TPT side.
///
/// `terms` and `edges` contain the Tes derivation's own output unchanged
/// except for the labels a pairing flips, which is what makes the kill gate a
/// property of the type rather than of the algorithm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectCrosswalk {
    pub terms: Vec<CanonicalTerm>,
    pub edges: Vec<ProjectionEdge>,
    pub no_counterparts: Vec<NoCounterpart>,
    pub residue: SubjectResidue,
    /// What the market-to-market half refused, carried through unchanged so
    /// that consuming `derive_crosswalk` does not cost the seeder the report
    /// it prints today.
    pub tes_residue: Residue,
}

/// What the pairing could not place, in the shape `tes::Residue` defines for
/// the market-to-market half.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SubjectResidue {
    /// Facets that mint a canonical term and reach no Tes node.
    pub tpt_only: Vec<ResidueNode>,
    /// Tes nodes no facet denotes, which are the no-counterpart records.
    pub tes_only: Vec<ResidueNode>,
    pub mismatched: Vec<SubjectMismatch>,
    /// Hidden facets, which seed neither a term nor an edge. Reported so that
    /// a vocabulary the relation deliberately ignores is a figure in the seed
    /// output rather than a silent absence.
    pub skipped_hidden: Vec<ResidueNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectMismatch {
    pub node: ResidueNode,
    pub counterpart: Option<ResidueNode>,
    pub reason: SubjectMismatchReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectMismatchReason {
    /// A child of this root denotes the node exactly, so the root's broader
    /// claim on it is withdrawn. Holding both would give the node's term an
    /// `Exact` and a `Broader` edge onto two TPT paths, which projects
    /// `Ambiguous` and blocks the publish.
    WithdrawnByChild,
    /// Two facets denote one Tes node. Neither claim is emitted, because
    /// choosing between them is the decision the design reserves for a human.
    ContestedNode,
}

impl core::fmt::Display for SubjectMismatchReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WithdrawnByChild => f.write_str("a child of this root denotes the node exactly"),
            Self::ContestedNode => f.write_str("two facets denote one Tes node"),
        }
    }
}

/// A malformed input, refused wholesale rather than degraded to residue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectError {
    Parse {
        file: &'static str,
        detail: String,
    },
    /// The pairing names a slug the capture holds no subject-area facet for.
    UnknownFacet {
        slug: String,
    },
    UnknownTesNode {
        id: u64,
    },
    /// The node exists in the capture but seeded no canonical term, so there
    /// is nothing for the pairing to attach an edge to.
    UnseededTesNode {
        id: u64,
    },
    MalformedRow {
        slug: String,
        detail: &'static str,
    },
    /// Ruling C: a hidden facet holds no edge, so a pairing that gives it one
    /// is a contradiction rather than a weaker claim.
    HiddenFacetPaired {
        slug: String,
    },
    /// Two facets name one target path. The reverse-`Exact` index admits one
    /// claimant per path, so the seed would be rejected at insert.
    DuplicateTptPath {
        first: String,
        second: String,
    },
}

impl core::fmt::Display for SubjectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse { file, detail } => write!(f, "{file} is not the expected shape: {detail}"),
            Self::UnknownFacet { slug } => write!(
                f,
                "the pairing names {slug}, which is no {SUBJECT_CATEGORY} facet"
            ),
            Self::UnknownTesNode { id } => write!(
                f,
                "the pairing names Tes node {id}, which the GB capture does not hold"
            ),
            Self::UnseededTesNode { id } => write!(
                f,
                "the pairing names Tes node {id}, which seeded no canonical term"
            ),
            Self::MalformedRow { slug, detail } => write!(f, "the row for {slug} {detail}"),
            Self::HiddenFacetPaired { slug } => write!(
                f,
                "{slug} is hidden and therefore holds no edge, so it may not be paired"
            ),
            Self::DuplicateTptPath { first, second } => {
                write!(f, "{first} and {second} name one TPT path")
            }
        }
    }
}

impl core::error::Error for SubjectError {}

/// Derives the subject and topic axes from the two Tes captures, the TPT
/// vocabulary capture and the authored pairing.
///
/// Pure: every capture arrives as text or as an already-parsed tree and the
/// caller does the I/O, so `tam-taxonomy` stays inside the purity gate.
pub fn derive_subject_crosswalk(
    tes: &TesTree,
    tpt_json: &str,
    pairs_json: &str,
    decided_at: Timestamp,
) -> Result<SubjectCrosswalk, SubjectError> {
    let base = derive_crosswalk(tes, decided_at);
    let tpt: TptVocabulary =
        serde_json::from_str(tpt_json).map_err(|error| SubjectError::Parse {
            file: "tpt-vocabulary.json",
            detail: error.to_string(),
        })?;
    let file: PairFile = serde_json::from_str(pairs_json).map_err(|error| SubjectError::Parse {
        file: "tpt-tes-subject-pairs.json",
        detail: error.to_string(),
    })?;
    let facets = subject_area_facets(&tpt.taxonomy_tags.options);
    let index = TesIndex::build(tes, &base);
    let plan = Plan::build(&file.pairs, &facets, &index)?;
    plan.emit(
        base,
        &Emission {
            facets: &facets,
            index: &index,
            decided_by: Decider::Imported {
                source: tpt.source.clone(),
            },
            decided_at,
        },
    )
}

/// Everything the emission reads and does not change, so that the one
/// function taking all of it takes one argument.
struct Emission<'a, 'b> {
    facets: &'a BTreeMap<String, Facet>,
    index: &'a TesIndex<'b>,
    decided_by: Decider,
    decided_at: Timestamp,
}

#[derive(Debug, Clone, Deserialize)]
struct PairFile {
    pairs: Vec<PairRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct PairRow {
    tpt: String,
    #[serde(default)]
    kind: Option<PairKind>,
    tes: Vec<PairNode>,
}

/// How a facet stands to the Tes nodes its row names, stated from the facet's
/// side. The edge each one emits is not the same word, because which side
/// holds the canonical term decides which way the edge points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PairKind {
    /// Co-extensive with the one named node, whose canonical term this facet
    /// reuses and which gains an `Exact` edge into TPT.
    Exact,
    /// Broader than each named node, each of whose terms gains a `Broader`
    /// edge into this facet's TPT path.
    Broader,
    /// Narrower than the one named node, whose `Exact` claim is already
    /// taken, so this facet's own term reaches it through a `Broader` edge.
    Narrower,
}

#[derive(Debug, Clone, Deserialize)]
struct PairNode {
    id: u64,
}

struct Facet {
    slug: String,
    name: String,
    parent: Option<String>,
    hidden: bool,
}

fn subject_area_facets(tags: &BTreeMap<String, TaxonomyTag>) -> BTreeMap<String, Facet> {
    tags.iter()
        .filter(|(_, tag)| tag.category.as_deref() == Some(SUBJECT_CATEGORY))
        .map(|(slug, tag)| {
            (
                slug.clone(),
                Facet {
                    slug: slug.clone(),
                    name: tag.name.clone(),
                    parent: tag.parent_id.clone(),
                    hidden: tag.is_hidden.unwrap_or(false),
                },
            )
        })
        .collect()
}

/// The Tes capture addressed by native id, restricted to the nodes that
/// actually seeded a canonical term, so a pairing can never attach an edge to
/// a term the base derivation declined to mint.
struct TesIndex<'a> {
    subjects: BTreeMap<u64, &'a TesSubject>,
    topics: BTreeMap<u64, (&'a TesSubject, &'a TesTopic)>,
    seeded: BTreeSet<u64>,
}

impl<'a> TesIndex<'a> {
    fn build(tes: &'a TesTree, base: &Crosswalk) -> Self {
        let minted: BTreeSet<[u8; 16]> = base.terms.iter().map(|term| term.id.0 .0).collect();
        let mut subjects = BTreeMap::new();
        let mut topics = BTreeMap::new();
        let mut seeded = BTreeSet::new();
        for subject in &tes.subjects {
            subjects.insert(subject.id, subject);
            if minted.contains(&canonical_id(subject.id).0 .0) {
                seeded.insert(subject.id);
            }
            for topic in subject.children() {
                topics.insert(topic.id, (subject, topic));
                if minted.contains(&canonical_id(topic.id).0 .0) {
                    seeded.insert(topic.id);
                }
            }
        }
        Self {
            subjects,
            topics,
            seeded,
        }
    }

    fn kind(&self, id: u64) -> Option<TermKind> {
        if self.subjects.contains_key(&id) {
            Some(TermKind::Subject)
        } else if self.topics.contains_key(&id) {
            Some(TermKind::Topic)
        } else {
            None
        }
    }

    fn description(&self, id: u64) -> Option<&str> {
        self.subjects
            .get(&id)
            .map(|subject| subject.description.as_str())
            .or_else(|| {
                self.topics
                    .get(&id)
                    .map(|(_, topic)| topic.description.as_str())
            })
    }

    /// Every node a facet's children may be matched against: a named subject
    /// contributes itself and its topics, a named topic only itself.
    fn scope(&self, named: &[u64]) -> Vec<u64> {
        let mut scope = Vec::new();
        for &id in named {
            scope.push(id);
            if let Some(subject) = self.subjects.get(&id) {
                scope.extend(subject.children().iter().map(|topic| topic.id));
            }
        }
        scope
    }

    fn residue(&self, id: u64) -> Option<ResidueNode> {
        Some(ResidueNode {
            market: InventoryId::Tes,
            kind: self.kind(id)?,
            native_id: id.to_string(),
            description: self.description(id)?.to_owned(),
        })
    }
}

/// One facet's whole relation to the Tes side, after the authored rows and
/// the derived child matches have both been read and the withdrawals applied.
#[derive(Default)]
struct Binding {
    /// The node whose canonical term this facet reuses.
    reuses: Option<u64>,
    /// The nodes whose terms gain a `Broader` edge into this facet.
    broadens: Vec<u64>,
    /// The node this facet's own term reaches through a `Broader` edge.
    narrows: Option<u64>,
    kind: Option<TermKind>,
}

impl Binding {
    fn reaches_nothing(&self) -> bool {
        self.reuses.is_none() && self.broadens.is_empty() && self.narrows.is_none()
    }
}

/// What the two pairing passes accumulate between them: which facet reaches
/// each Tes node, the nodes two facets both reach, and the report rows the
/// refusals earn.
#[derive(Default)]
struct Claims {
    by_node: BTreeMap<u64, String>,
    contested: BTreeSet<u64>,
    mismatched: Vec<SubjectMismatch>,
}

impl Claims {
    fn contest(&mut self, node: u64, against: Option<&Facet>, index: &TesIndex<'_>) {
        self.contested.insert(node);
        if let Some(record) = index.residue(node) {
            self.mismatched.push(SubjectMismatch {
                node: record,
                counterpart: against.map(facet_residue),
                reason: SubjectMismatchReason::ContestedNode,
            });
        }
    }
}

struct Plan {
    bindings: BTreeMap<String, Binding>,
    mismatched: Vec<SubjectMismatch>,
}

impl Plan {
    fn build(
        rows: &[PairRow],
        facets: &BTreeMap<String, Facet>,
        index: &TesIndex<'_>,
    ) -> Result<Self, SubjectError> {
        let mut bindings: BTreeMap<String, Binding> = BTreeMap::new();
        let mut claims = Claims::default();

        for row in rows {
            let facet = facets
                .get(&row.tpt)
                .ok_or_else(|| SubjectError::UnknownFacet {
                    slug: row.tpt.clone(),
                })?;
            let named: Vec<u64> = row.tes.iter().map(|node| node.id).collect();
            for &id in &named {
                if index.kind(id).is_none() {
                    return Err(SubjectError::UnknownTesNode { id });
                }
                if !index.seeded.contains(&id) {
                    return Err(SubjectError::UnseededTesNode { id });
                }
            }
            let Some(kind) = row.kind else {
                if !named.is_empty() {
                    return Err(SubjectError::MalformedRow {
                        slug: row.tpt.clone(),
                        detail: "states no relation yet names Tes nodes",
                    });
                }
                bindings.insert(row.tpt.clone(), Binding::default());
                continue;
            };
            if facet.hidden {
                return Err(SubjectError::HiddenFacetPaired {
                    slug: row.tpt.clone(),
                });
            }
            let levels: Vec<TermKind> = named.iter().filter_map(|&id| index.kind(id)).collect();
            let Some(&level) = levels.first() else {
                return Err(SubjectError::MalformedRow {
                    slug: row.tpt.clone(),
                    detail: "states a relation yet names no Tes node",
                });
            };
            if levels.iter().any(|&other| other != level) {
                return Err(SubjectError::MalformedRow {
                    slug: row.tpt.clone(),
                    detail: "names Tes nodes at more than one level",
                });
            }
            let mut binding = Binding {
                kind: Some(level),
                ..Binding::default()
            };
            match kind {
                PairKind::Exact | PairKind::Narrower => {
                    let [only] = named[..] else {
                        return Err(SubjectError::MalformedRow {
                            slug: row.tpt.clone(),
                            detail: "is exact or narrower and must name exactly one Tes node",
                        });
                    };
                    if kind == PairKind::Exact {
                        binding.reuses = Some(only);
                    } else {
                        binding.narrows = Some(only);
                    }
                }
                PairKind::Broader => binding.broadens.clone_from(&named),
            }
            // A `narrower` row reaches its node without giving that node's own
            // term an edge, so it is not a claim and cannot contest one.
            if kind != PairKind::Narrower {
                for &id in &named {
                    if let Some(first) = claims.by_node.insert(id, row.tpt.clone()) {
                        claims.contest(id, Some(facet), index);
                        claims.by_node.insert(id, first);
                    }
                }
            }
            bindings.insert(row.tpt.clone(), binding);
        }

        Self::pair_children(&mut bindings, facets, index, &mut claims);
        Self::withdraw(&mut bindings, facets, index, &mut claims);
        for binding in bindings.values_mut() {
            binding.broadens.retain(|id| !claims.contested.contains(id));
            if binding
                .reuses
                .is_some_and(|id| claims.contested.contains(&id))
            {
                binding.reuses = None;
            }
        }
        Ok(Self {
            bindings,
            mismatched: claims.mismatched,
        })
    }

    /// The derived half. Each child of a root is matched by normalised
    /// description against the Tes nodes inside that root's denotation; a
    /// child matching one node denotes it exactly, a child matching several
    /// is broader than all of them, and a child matching none reaches
    /// nothing.
    fn pair_children(
        bindings: &mut BTreeMap<String, Binding>,
        facets: &BTreeMap<String, Facet>,
        index: &TesIndex<'_>,
        claims: &mut Claims,
    ) {
        let mut derived_bindings: Vec<(String, Binding)> = Vec::new();
        for (slug, facet) in facets {
            let Some(parent) = facet.parent.as_ref() else {
                continue;
            };
            if facet.hidden {
                continue;
            }
            let scope = bindings.get(parent).map_or_else(Vec::new, |binding| {
                let mut named = binding.broadens.clone();
                named.extend(binding.reuses);
                named.extend(binding.narrows);
                index.scope(&named)
            });
            let wanted = normalise(&facet.name);
            let matches: Vec<u64> = scope
                .into_iter()
                .filter(|&id| {
                    index
                        .description(id)
                        .is_some_and(|description| normalise(description) == wanted)
                })
                .collect();
            let mut binding = Binding::default();
            match matches[..] {
                [] => {}
                [only] => {
                    binding.kind = index.kind(only);
                    binding.reuses = Some(only);
                }
                _ => {
                    binding.kind = matches.first().copied().and_then(|id| index.kind(id));
                    binding.broadens.clone_from(&matches);
                }
            }
            for &id in &matches {
                // A root's broader claim on a node its own child denotes
                // exactly is not a contest: `withdraw` settles it in the
                // child's favour, which is the finer of the two claims.
                if let Some(first) = claims.by_node.insert(id, slug.clone()) {
                    if &first != parent {
                        claims.contest(id, facets.get(&first), index);
                    }
                }
            }
            derived_bindings.push((slug.clone(), binding));
        }
        for (slug, binding) in derived_bindings {
            bindings.insert(slug, binding);
        }
    }

    /// A child's exact claim beats its root's broader claim on the same node.
    fn withdraw(
        bindings: &mut BTreeMap<String, Binding>,
        facets: &BTreeMap<String, Facet>,
        index: &TesIndex<'_>,
        claims: &mut Claims,
    ) {
        let taken: BTreeMap<u64, String> = facets
            .values()
            .filter_map(|facet| {
                let parent = facet.parent.as_ref()?;
                let node = bindings.get(&facet.slug)?.reuses?;
                Some((node, parent.clone()))
            })
            .collect();
        for (node, root) in taken {
            let Some(binding) = bindings.get_mut(&root) else {
                continue;
            };
            if !binding.broadens.contains(&node) {
                continue;
            }
            binding.broadens.retain(|&id| id != node);
            if let Some(record) = index.residue(node) {
                claims.mismatched.push(SubjectMismatch {
                    node: record,
                    counterpart: facets.get(&root).map(facet_residue),
                    reason: SubjectMismatchReason::WithdrawnByChild,
                });
            }
        }
    }

    fn emit(
        self,
        base: Crosswalk,
        at: &Emission<'_, '_>,
    ) -> Result<SubjectCrosswalk, SubjectError> {
        let Emission {
            facets,
            index,
            decided_by,
            decided_at,
        } = at;
        let decided_at = *decided_at;
        let mut paths: BTreeMap<String, VocabularyPath> = BTreeMap::new();
        let mut by_path: BTreeMap<(u8, Vec<String>), String> = BTreeMap::new();
        for (slug, binding) in &self.bindings {
            let Some(facet) = facets.get(slug) else {
                continue;
            };
            if facet.hidden {
                continue;
            }
            let kind = binding.kind.unwrap_or_else(|| depth_kind(facet));
            let path = tpt_path(facet, kind, facets);
            if let Some(first) =
                by_path.insert((kind_rank(kind), path.segments.clone()), facet.slug.clone())
            {
                return Err(SubjectError::DuplicateTptPath {
                    first,
                    second: facet.slug.clone(),
                });
            }
            paths.insert(slug.clone(), path);
        }

        let Crosswalk {
            mut terms,
            mut edges,
            residue: tes_residue,
        } = base;
        let mut residue = SubjectResidue::default();
        let mut minted: BTreeMap<String, CanonicalTermId> = BTreeMap::new();
        let mut roots = Vec::new();
        let mut children = Vec::new();

        for (slug, binding) in &self.bindings {
            let Some(facet) = facets.get(slug) else {
                continue;
            };
            if facet.hidden {
                residue.skipped_hidden.push(facet_residue(facet));
                continue;
            }
            if let Some(node) = binding.reuses {
                let id = canonical_id(node);
                minted.insert(slug.clone(), id);
                if let Some(term) = terms.iter_mut().find(|term| term.id == id) {
                    term.label.clone_from(&facet.name);
                }
            } else {
                minted.insert(slug.clone(), tpt_term_id(slug));
                if binding.reaches_nothing() {
                    residue.tpt_only.push(facet_residue(facet));
                }
            }
        }

        for (slug, binding) in &self.bindings {
            let (Some(facet), Some(path)) = (facets.get(slug), paths.get(slug)) else {
                continue;
            };
            if binding.reuses.is_some() {
                continue;
            }
            let id = minted[slug];
            let parent = facet
                .parent
                .as_ref()
                .and_then(|parent| minted.get(parent))
                .copied();
            let term = CanonicalTerm {
                id,
                kind: path.vocabulary.1,
                parent,
                label: facet.name.clone(),
            };
            if facet.parent.is_some() {
                children.push(term);
            } else {
                roots.push(term);
            }
        }
        terms.extend(roots);
        terms.extend(children);

        for (slug, binding) in &self.bindings {
            let Some(path) = paths.get(slug) else {
                continue;
            };
            let id = minted[slug];
            edges.push(ProjectionEdge {
                from: id,
                to: path.clone(),
                kind: EdgeKind::Exact,
                decided_by: decided_by.clone(),
                decided_at,
            });
            if let Some(node) = binding.narrows {
                for to in tes_paths(&edges, canonical_id(node)) {
                    edges.push(ProjectionEdge {
                        from: id,
                        to,
                        kind: EdgeKind::Broader,
                        decided_by: decided_by.clone(),
                        decided_at,
                    });
                }
            }
            for &node in &binding.broadens {
                edges.push(ProjectionEdge {
                    from: canonical_id(node),
                    to: path.clone(),
                    kind: EdgeKind::Broader,
                    decided_by: decided_by.clone(),
                    decided_at,
                });
            }
        }

        let reached: BTreeSet<u64> = self
            .bindings
            .values()
            .flat_map(|binding| binding.broadens.iter().copied().chain(binding.reuses))
            .collect();
        let mut no_counterparts = Vec::new();
        for &node in &index.seeded {
            let Some(kind) = index.kind(node) else {
                continue;
            };
            if reached.contains(&node) {
                continue;
            }
            if let Some(record) = index.residue(node) {
                residue.tes_only.push(record);
            }
            no_counterparts.push(NoCounterpart {
                term: canonical_id(node),
                target: VocabularyId(InventoryId::Tpt, kind),
                decided_by: decided_by.clone(),
                decided_at,
            });
        }
        residue.mismatched = self.mismatched;

        Ok(SubjectCrosswalk {
            terms,
            edges,
            no_counterparts,
            residue,
            tes_residue,
        })
    }
}

/// The Tes paths one canonical term already reaches, read out of the base
/// derivation's own edges rather than rebuilt, so a `narrower` row cannot
/// name a path the Tes half does not hold.
fn tes_paths(edges: &[ProjectionEdge], term: CanonicalTermId) -> Vec<VocabularyPath> {
    edges
        .iter()
        .filter(|edge| edge.from == term && edge.kind == EdgeKind::Exact)
        .filter(|edge| matches!(edge.to.vocabulary.0, InventoryId::Tes))
        .map(|edge| edge.to.clone())
        .collect()
}

/// A facet's path: its own name under its parent's, addressed by the slug
/// TPT's wire posts and reads. `legacyId` addresses the create form's
/// selector instead and the two are not interchangeable on the wire.
fn tpt_path(facet: &Facet, kind: TermKind, facets: &BTreeMap<String, Facet>) -> VocabularyPath {
    let mut segments = Vec::new();
    if let Some(parent) = facet.parent.as_ref().and_then(|slug| facets.get(slug)) {
        segments.push(parent.name.clone());
    }
    segments.push(facet.name.clone());
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tpt, kind),
        segments,
        native_id: Some(facet.slug.clone()),
    }
}

fn facet_residue(facet: &Facet) -> ResidueNode {
    ResidueNode {
        market: InventoryId::Tpt,
        kind: depth_kind(facet),
        native_id: facet.slug.clone(),
        description: facet.name.clone(),
    }
}

/// The kind a facet takes when nothing it denotes decides one: TPT's own
/// depth, which is the only fact left.
const fn depth_kind(facet: &Facet) -> TermKind {
    if facet.parent.is_some() {
        TermKind::Topic
    } else {
        TermKind::Subject
    }
}

/// A grouping key only. The reverse-`Exact` index is on the target path, so
/// two facets at one kind may not name one segment list.
const fn kind_rank(kind: TermKind) -> u8 {
    match kind {
        TermKind::Subject => 0,
        TermKind::Topic => 1,
        TermKind::ResourceType => 2,
        TermKind::Phase => 3,
        TermKind::Licence => 4,
    }
}

/// Deterministic canonical ids in the scheme `grades.rs` uses, so a re-seed is
/// an explicit no-op and no later capture can re-key a term that already
/// exists. The prefix names the axis the id was minted for and not the kind
/// the term ended up with, which ruling B makes a separate decision.
pub fn tpt_term_id(slug: &str) -> CanonicalTermId {
    derived(&format!("tpt-subject:{slug}"))
}

/// The comparison the child pairing matches on. Tes and TPT write the same
/// concept with different punctuation and case -- `Speaking & Listening`
/// against `Speaking and listening` -- and neither carries an identifier the
/// other answers to, so the description is the only shared fact.
fn normalise(description: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for character in description.chars() {
        if character == '&' {
            if !word.is_empty() {
                words.push(core::mem::take(&mut word));
            }
            words.push("and".to_owned());
        } else if character.is_alphanumeric() {
            word.extend(character.to_lowercase());
        } else if !word.is_empty() {
            words.push(core::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words.join(" ")
}

#[cfg(test)]
mod tests;
