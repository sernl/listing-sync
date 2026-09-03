//! The projection function in both directions.
//!
//! Outbound, `project` decides on the edge set alone: no fallback, no
//! nearest-neighbour search and no model proposal, per the five-row table in
//! `docs/design/taxonomy-projection.md`. Inbound, `ingest` is the reverse of
//! the `Exact` relation only, because inverting a `Broader` edge would restore
//! the distinction the edge dropped.

use tam_domain::equivalence::{
    resolved_by, satisfied_by, settled_by, AxisOutcome, Election, ElectionAnswer, ElectionRule,
    ElectionTrigger, Loss, OverrideKind, PricingBranch, ProjectionOverride, SettledElection,
    VocabularyGap,
};
use tam_domain::registry::{registry, AxisBinding, Cardinality};
use tam_domain::{
    EdgeKind, GradeDeclaration, ProjectionEdge, TermProjection, VocabularyId, VocabularyPath,
};
use tam_types::{CanonicalTermId, InventoryId, ProductId};

/// A term whose projection has no correct answer, carrying the projection so
/// the caller can raise a reconciliation item naming which of the two it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedTerm {
    pub term: CanonicalTermId,
    pub projection: TermProjection,
}

/// The outcome of projecting a whole term list into one vocabulary.
/// `included` is what the listing carries, `loss` is the broadening the seller
/// sees in the field diff before publish, `blocked` is what raises
/// reconciliation items, and `omitted` is the terms a `NoCounterpart` record
/// says to drop.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TermsOutcome {
    pub included: Vec<VocabularyPath>,
    pub loss: Vec<TermProjection>,
    pub blocked: Vec<BlockedTerm>,
    pub omitted: Vec<CanonicalTermId>,
}

/// Projects one canonical term into one target vocabulary.
///
/// `Narrower` edges never participate: a narrower target invents a distinction
/// the canonical term does not carry, so it is not an invertible shortcut and
/// its presence neither produces a projection nor disturbs one.
#[must_use]
pub fn project(
    term: CanonicalTermId,
    vocabulary: VocabularyId,
    edges: &[ProjectionEdge],
) -> TermProjection {
    let mut exact: Vec<&VocabularyPath> = Vec::new();
    let mut broader: Vec<&VocabularyPath> = Vec::new();
    for edge in edges {
        if edge.from != term || edge.to.vocabulary != vocabulary {
            continue;
        }
        match edge.kind {
            EdgeKind::Exact => exact.push(&edge.to),
            EdgeKind::Broader => broader.push(&edge.to),
            EdgeKind::Narrower => {}
        }
    }

    match (exact.as_slice(), broader.as_slice()) {
        ([], []) => TermProjection::Absent,
        ([to], rest) if rest.iter().all(|path| *path == *to) => {
            TermProjection::Exact { to: (*to).clone() }
        }
        ([], [first, rest @ ..]) if rest.iter().all(|path| *path == *first) => {
            TermProjection::Broadened {
                to: (*first).clone(),
                dropped: vec![term],
            }
        }
        (_, _) => TermProjection::Ambiguous {
            candidates: distinct(exact.into_iter().chain(broader)),
        },
    }
}

/// Projects a term list, folding each `TermProjection` into the four buckets
/// the publish gate reads. A term whose projection is `Absent` and which
/// carries a `NoCounterpart` record for this vocabulary proceeds by omission;
/// an `Ambiguous` never omits, because the choice between two legitimate
/// targets is a decision rather than a gap.
#[must_use]
pub fn project_terms(
    terms: &[CanonicalTermId],
    vocabulary: VocabularyId,
    edges: &[ProjectionEdge],
    no_counterparts: &[(CanonicalTermId, VocabularyId)],
) -> TermsOutcome {
    let mut outcome = TermsOutcome::default();
    for &term in terms {
        match project(term, vocabulary, edges) {
            TermProjection::Exact { to } => push_distinct(&mut outcome.included, &to),
            TermProjection::Broadened { to, dropped } => {
                push_distinct(&mut outcome.included, &to);
                merge_loss(&mut outcome.loss, &to, dropped);
            }
            TermProjection::Ambiguous { candidates } => outcome.blocked.push(BlockedTerm {
                term,
                projection: TermProjection::Ambiguous { candidates },
            }),
            TermProjection::Absent => {
                if no_counterparts
                    .iter()
                    .any(|&(recorded, target)| recorded == term && target == vocabulary)
                {
                    outcome.omitted.push(term);
                } else {
                    outcome.blocked.push(BlockedTerm {
                        term,
                        projection: TermProjection::Absent,
                    });
                }
            }
        }
    }
    outcome
}

/// One axis's projection, as the caller states it. Bundled because the arity
/// would otherwise exceed the workspace argument limit.
///
/// The binding is a parameter rather than a lookup through
/// `registry(inventory)`, deliberately: every populated cardinality on the
/// five real inventories is `One` or `Many { cap: None }`, so a cap overflow
/// has no fixture anywhere, and injecting the binding is what makes the
/// no-truncation invariant testable at all.
#[derive(Debug, Clone, Copy)]
pub struct AxisRequest<'a> {
    pub product: ProductId,
    pub inventory: InventoryId,
    pub binding: AxisBinding,
    pub terms: &'a [CanonicalTermId],
    /// The source-declared path each term was recognised from, parallel to
    /// `terms`, and empty where the axis arrives as canonical ids alone.
    ///
    /// A `Narrow` election names the seller's own value and keys a standing
    /// answer on that value's native id, so a term with no source path stays a
    /// gap rather than becoming a question that cannot say what it is about.
    pub sources: &'a [VocabularyPath],
    /// Which side of the free/paid gate the product sits on. A `Supply`
    /// question is unanswerable without it, because the branch decides which
    /// values the target will even accept.
    pub pricing: PricingBranch,
    /// The tenant's standing answers. Consulted before any election is
    /// returned, so a question the seller has already answered as a policy is
    /// never asked a second time and never reaches the queue at all.
    pub rules: &'a [ElectionRule],
    /// What this product's own settled questions were answered with. Without
    /// it an answered election revives the item it parked, re-raises the
    /// identical question and parks again, so the decision surface is inert
    /// and only a standing rule can ever release anything.
    pub settled: &'a [SettledElection],
}

/// Projects one whole axis into one target inventory, naming a gap, an
/// election, a loss and an unrecognised value apart.
///
/// `project_terms` is the term-level primitive underneath and stays exactly as
/// it was; what this adds is the axis-level facts a term never carries — the
/// target's cardinality, its requiredness, and the product the question is
/// about.
/// The relation alone decides, which is every caller that predates the
/// override layer and every one that has no tenant to speak for.
#[must_use]
pub fn project_axis(
    request: AxisRequest<'_>,
    edges: &[ProjectionEdge],
    no_counterparts: &[(CanonicalTermId, VocabularyId)],
) -> AxisOutcome {
    project_axis_with_overrides(request, edges, no_counterparts, &[])
}

/// The same projection with one tenant's own decisions consulted first.
///
/// The overrides arrive as a parameter rather than on `AxisRequest` so that
/// adding the layer breaks no existing construction of that struct: a caller
/// that knows nothing about overrides keeps compiling and keeps behaving
/// exactly as it did.
///
/// They are already scoped to the org by the caller, as `rules` is: this
/// function knows a target and an axis and deliberately not a tenant.
#[must_use]
pub fn project_axis_with_overrides(
    request: AxisRequest<'_>,
    edges: &[ProjectionEdge],
    no_counterparts: &[(CanonicalTermId, VocabularyId)],
    overrides: &[ProjectionOverride],
) -> AxisOutcome {
    let target = VocabularyId(request.inventory, request.binding.axis);
    // Precedence, and it is total in one direction: a term the seller has
    // decided for is not put to the relation at all. Filtering here rather
    // than folding the override in beside the global edges is what makes it a
    // decision rather than a second claimant -- `project` would see two paths
    // for one term and return `Ambiguous`, which is the opposite of settling.
    let overridden: Vec<&ProjectionOverride> = request
        .terms
        .iter()
        .filter_map(|&term| {
            overrides
                .iter()
                .find(|entry| entry.answers(request.inventory, request.binding.axis, term))
        })
        .collect();
    let remaining: Vec<CanonicalTermId> = request
        .terms
        .iter()
        .copied()
        .filter(|&term| {
            !overrides
                .iter()
                .any(|entry| entry.answers(request.inventory, request.binding.axis, term))
        })
        .collect();
    let terms = project_terms(&remaining, target, edges, no_counterparts);

    let mut outcome = AxisOutcome {
        resolved: terms.included,
        loss: terms.loss.iter().filter_map(broadening).collect(),
        omitted: terms.omitted,
        decided_by_seller: !overridden.is_empty(),
        ..AxisOutcome::default()
    };
    // Before the cardinality election below, because an overridden value
    // counts against the target's cap exactly as a resolved one does: the
    // seller choosing a value does not exempt the target from taking one.
    for entry in overridden {
        push_distinct(&mut outcome.resolved, &entry.to);
        if entry.kind == OverrideKind::Broader {
            merge_axis_loss(&mut outcome.loss, &entry.to, entry.from);
        }
    }
    let raise = |trigger| Election {
        product: request.product,
        inventory: request.inventory,
        axis: request.binding.axis,
        trigger,
    };
    for blocked in terms.blocked {
        match narrowing(&blocked, &request, edges) {
            Some(trigger) => outcome.elections.push(raise(trigger)),
            None => outcome.gaps.push(VocabularyGap {
                term: blocked.term,
                target,
                projection: blocked.projection,
            }),
        }
    }

    if let Some(trigger) =
        elect_over_cardinality(request.binding.cardinality, &mut outcome.resolved)
    {
        outcome.elections.push(raise(trigger));
    } else if requires_a_value(&request) && request.terms.is_empty() {
        outcome.elections.push(raise(ElectionTrigger::Supply {
            pricing: request.pricing,
        }));
    }

    // The seller's answers settle last, over every election this axis raised,
    // because an answer is about a trigger rather than about the path that
    // produced it. This product's own settled question comes before the
    // standing policy, being the more specific fact. An answered election
    // resolves into the same set the relation would have resolved into, so
    // nothing downstream can tell a decided answer from a translated one --
    // which is the point: the seller decided, and the decision is now data
    // like any other.
    let mut standing = Vec::new();
    for election in core::mem::take(&mut outcome.elections) {
        let decided = settled_by(request.settled, &election)
            .and_then(|chosen| {
                resolved_against(
                    &election.trigger,
                    &ElectionAnswer::Ordering {
                        prefer: chosen.to_vec(),
                    },
                    target,
                    edges,
                )
            })
            .or_else(|| {
                satisfied_by(request.rules, &election)
                    .and_then(|answer| resolved_against(&election.trigger, answer, target, edges))
            });
        match decided {
            Some(paths) => {
                for path in &paths {
                    push_distinct(&mut outcome.resolved, path);
                }
            }
            None => standing.push(election),
        }
    }
    outcome.elections = standing;
    outcome
}

/// A source value the relation places below several target values is a
/// question rather than a gap.
///
/// One Tes GB age band covers several US year groups, which the seeded
/// relation records as `Narrower` edges. `project` will never derive across
/// them — inverting one restores the distinction the band dropped — so the
/// term projects `Absent`, and calling that a vocabulary gap would ask the
/// seller to author an equivalence that does not exist. What is missing is not
/// an edge; it is which of the year groups this particular listing means.
fn narrowing(
    blocked: &BlockedTerm,
    request: &AxisRequest<'_>,
    edges: &[ProjectionEdge],
) -> Option<ElectionTrigger> {
    if !matches!(blocked.projection, TermProjection::Absent) {
        return None;
    }
    let target = VocabularyId(request.inventory, request.binding.axis);
    let index = request
        .terms
        .iter()
        .position(|term| *term == blocked.term)?;
    let from = request.sources.get(index)?.clone();
    let candidates = distinct(
        edges
            .iter()
            .filter(|edge| {
                edge.kind == EdgeKind::Narrower
                    && edge.from == blocked.term
                    && edge.to.vocabulary == target
            })
            .map(|edge| &edge.to),
    );
    if candidates.is_empty() {
        return None;
    }
    Some(ElectionTrigger::Narrow { from, candidates })
}

/// The grade axis enters as the source's own paths rather than as canonical
/// ids, so it ingests before it projects.
///
/// A path the relation does not recognise is neither a gap nor a loss:
/// `reconciliation_item.term` is `NOT NULL REFERENCES canonical_term`, so an
/// unrecognised path cannot become a queue item at all, and calling it a loss
/// would assert the target has no such field when the truth is that we do not
/// know what the value is. It is carried out separately rather than dropped.
#[must_use]
pub fn ingest_grades(declaration: &GradeDeclaration, edges: &[ProjectionEdge]) -> GradeIngest {
    let mut ingest = GradeIngest::default();
    for path in &declaration.raw {
        let recognised = match path.native_id.as_deref() {
            Some(native) => ingest_by_native_id(native, path.vocabulary, edges),
            None => self::ingest(path, edges),
        };
        match recognised {
            Some(term) => {
                ingest.terms.push(term);
                ingest.sources.push(path.clone());
            }
            None => ingest.unrecognised.push(path.clone()),
        }
    }
    ingest
}

/// What a grade declaration became: the terms the relation recognised, the
/// source path each came from, and the paths it did not recognise at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GradeIngest {
    pub terms: Vec<CanonicalTermId>,
    /// Parallel to `terms`, so an election can name the seller's own value.
    pub sources: Vec<VocabularyPath>,
    pub unrecognised: Vec<VocabularyPath>,
}

/// Requiredness is read off the `NativeField` the binding names rather than
/// restated on the binding, so the two declarations cannot disagree. An axis
/// bound to a field the registry does not hold is not required, which the
/// registry's own test forbids from arising.
fn requires_a_value(request: &AxisRequest<'_>) -> bool {
    registry(request.inventory)
        .native(request.binding.native)
        .is_some_and(|native| native.required)
}

/// Cap overflow is a decision, never a truncation. The resolved set is taken
/// whole into the election, so no partially-narrowed set exists anywhere for
/// a caller to publish by accident.
fn elect_over_cardinality(
    cardinality: Cardinality,
    resolved: &mut Vec<VocabularyPath>,
) -> Option<ElectionTrigger> {
    match cardinality {
        Cardinality::One if resolved.len() > 1 => Some(ElectionTrigger::ElectOne {
            from: core::mem::take(resolved),
        }),
        Cardinality::Many { cap: Some(cap) } if resolved.len() > cap.limit => {
            Some(ElectionTrigger::OverCap {
                cap: cap.limit,
                from: core::mem::take(resolved),
            })
        }
        Cardinality::One | Cardinality::Many { .. } => None,
    }
}

/// Records a broadening the seller chose, merged onto the path it broadened
/// to so that four terms broadened onto one value disclose one loss naming
/// four rather than four losses naming one each, which is what
/// `project_terms` already does for the relation's own broadenings.
fn merge_axis_loss(loss: &mut Vec<Loss>, to: &VocabularyPath, dropped: CanonicalTermId) {
    for existing in loss.iter_mut() {
        if let Loss::Broadened {
            to: at,
            dropped: terms,
        } = existing
        {
            if at == to {
                if !terms.contains(&dropped) {
                    terms.push(dropped);
                }
                return;
            }
        }
    }
    loss.push(Loss::Broadened {
        to: to.clone(),
        dropped: vec![dropped],
    });
}

/// `project_terms` carries its loss as the `TermProjection` that produced it;
/// an axis outcome carries it as a `Loss`, which is the form the seller sees
/// and the mapping records.
pub(crate) fn broadening(projection: &TermProjection) -> Option<Loss> {
    match projection {
        TermProjection::Broadened { to, dropped } => Some(Loss::Broadened {
            to: to.clone(),
            dropped: dropped.clone(),
        }),
        TermProjection::Exact { .. }
        | TermProjection::Ambiguous { .. }
        | TermProjection::Absent => None,
    }
}

/// The inbound direction: the reverse of the `Exact` relation, matching on
/// vocabulary and segments because the segments are the path's identity and a
/// marketplace need not expose a native id.
///
/// Two `Exact` edges from different terms onto one path cannot exist under the
/// partial unique index on `projection_edge`, so reaching that state means the
/// relation is corrupt. It reads as unmapped rather than picking a winner, so
/// the path fails closed into the reconciliation queue.
#[must_use]
pub fn ingest(path: &VocabularyPath, edges: &[ProjectionEdge]) -> Option<CanonicalTermId> {
    let mut found: Option<CanonicalTermId> = None;
    for edge in edges {
        match edge.kind {
            EdgeKind::Exact => {}
            EdgeKind::Broader | EdgeKind::Narrower => continue,
        }
        if edge.to.vocabulary != path.vocabulary || edge.to.segments != path.segments {
            continue;
        }
        if let Some(claimed) = found {
            if claimed != edge.from {
                return None;
            }
        } else {
            found = Some(edge.from);
        }
    }
    found
}

/// One stored answer read back against the target vocabulary it names.
///
/// The seller answers with a value, and `AnswerPath` makes the target's own
/// native id optional, so an answer given as segments alone is stored and read
/// back carrying none. That is not a token: `native_id` is a fact about the
/// value's place in the target vocabulary rather than part of what the seller
/// decided, so it is resolved from the relation here rather than stored beside
/// the answer, which also keeps it current across a re-poll.
///
/// Two things depend on it. A supply answer resolves into a path the seam
/// writes as the target's own id, and one without it reaches the adapter as a
/// value the wire has no name for. And `resolved_by` checks every other
/// trigger's answer against the candidate set by whole-path equality, which an
/// answer missing the id its candidates carry can never satisfy -- so the
/// question would stand however often it was answered.
fn with_native_id(
    path: &VocabularyPath,
    target: VocabularyId,
    edges: &[ProjectionEdge],
) -> VocabularyPath {
    if path.native_id.is_some() || path.vocabulary != target {
        return path.clone();
    }
    let mut named = edges
        .iter()
        .filter(|edge| {
            edge.kind == EdgeKind::Exact
                && edge.to.vocabulary == target
                && edge.to.segments == path.segments
        })
        .filter_map(|edge| edge.to.native_id.as_ref());
    let Some(native) = named.next() else {
        return path.clone();
    };
    // Two members under one path and different ids is a defect in the
    // relation, and picking either would be us deciding which value the seller
    // named. Left as it arrived, the axis refuses downstream rather than
    // publishing a guess.
    if named.any(|other| other != native) {
        return path.clone();
    }
    VocabularyPath {
        native_id: Some(native.clone()),
        ..path.clone()
    }
}

/// `resolved_by` over an answer whose paths have been read back against the
/// target vocabulary first. Every consumption of a stored answer goes through
/// here, so the two shapes the API accepts -- with the native id and without
/// it -- resolve to the same value rather than only the first one working.
fn resolved_against(
    trigger: &ElectionTrigger,
    answer: &ElectionAnswer,
    target: VocabularyId,
    edges: &[ProjectionEdge],
) -> Option<Vec<VocabularyPath>> {
    let named = match answer {
        ElectionAnswer::Value { path } => ElectionAnswer::Value {
            path: with_native_id(path, target, edges),
        },
        ElectionAnswer::Ordering { prefer } => ElectionAnswer::Ordering {
            prefer: prefer
                .iter()
                .map(|path| with_native_id(path, target, edges))
                .collect(),
        },
        ElectionAnswer::Delegate => ElectionAnswer::Delegate,
    };
    resolved_by(trigger, &named)
}

fn push_distinct(into: &mut Vec<VocabularyPath>, path: &VocabularyPath) {
    if !into.iter().any(|seen| seen == path) {
        into.push(path.clone());
    }
}

fn distinct<'a>(paths: impl Iterator<Item = &'a VocabularyPath>) -> Vec<VocabularyPath> {
    let mut collected = Vec::new();
    for path in paths {
        push_distinct(&mut collected, path);
    }
    collected
}

/// One loss record per target path: two source terms broadening onto the same
/// target is one broadening in the seller's field diff, carrying both dropped
/// terms.
fn merge_loss(
    loss: &mut Vec<TermProjection>,
    target: &VocabularyPath,
    dropped: Vec<CanonicalTermId>,
) {
    let recorded = loss.iter_mut().find_map(|entry| match entry {
        TermProjection::Broadened { to, dropped: seen } if to == target => Some(seen),
        TermProjection::Broadened { .. }
        | TermProjection::Exact { .. }
        | TermProjection::Ambiguous { .. }
        | TermProjection::Absent => None,
    });
    match recorded {
        Some(seen) => {
            for term in dropped {
                if !seen.contains(&term) {
                    seen.push(term);
                }
            }
        }
        None => loss.push(TermProjection::Broadened {
            to: target.clone(),
            dropped,
        }),
    }
}

/// The inbound direction by the marketplace's own numeric id rather than by
/// segments: the import reads `categories: [{id}]`, and the id is what the
/// edge's `native_id` retains. Reverse of `Exact` edges only, and a
/// duplicated claim (impossible under the database's reverse-uniqueness
/// index) reads as unmapped rather than picking a winner.
#[must_use]
pub fn ingest_by_native_id(
    native_id: &str,
    vocabulary: VocabularyId,
    edges: &[ProjectionEdge],
) -> Option<CanonicalTermId> {
    let mut matches = edges.iter().filter(|edge| {
        edge.kind == EdgeKind::Exact
            && edge.to.vocabulary == vocabulary
            && edge.to.native_id.as_deref() == Some(native_id)
    });
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first.from)
}

#[cfg(test)]
mod tests {
    use super::{
        ingest, project, project_axis, project_axis_with_overrides, project_terms, AxisRequest,
        BlockedTerm,
    };
    use tam_domain::equivalence::{
        AxisOutcome, ElectionTrigger, ElectionTriggerKind, Loss, OverrideKind, PricingBranch,
        ProjectionOverride, SettledElection, VocabularyGap,
    };
    use tam_domain::registry::{AxisBinding, Cardinality, CountCap, Delegation};
    use tam_domain::{
        Decider, EdgeKind, ProjectionEdge, TermKind, TermProjection, VocabularyId, VocabularyPath,
    };
    use tam_types::{CanonicalTermId, InventoryId, OrgId, ProductId, Timestamp, Uuid};

    const TERM: CanonicalTermId = CanonicalTermId(Uuid([0x01; 16]));
    const OTHER: CanonicalTermId = CanonicalTermId(Uuid([0x02; 16]));
    const TARGET: VocabularyId = VocabularyId(InventoryId::TesNz, TermKind::Subject);
    const ELSEWHERE: VocabularyId = VocabularyId(InventoryId::TesUs, TermKind::Subject);

    fn path(segment: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: TARGET,
            segments: vec![segment.to_owned()],
            native_id: None,
        }
    }

    /// A vocabulary member as the relation holds one: the path and the
    /// target's own id for it.
    fn named(segment: &str, native: &str) -> VocabularyPath {
        VocabularyPath {
            native_id: Some(native.to_owned()),
            ..path(segment)
        }
    }

    fn edge(from: CanonicalTermId, to: VocabularyPath, kind: EdgeKind) -> ProjectionEdge {
        ProjectionEdge {
            from,
            to,
            kind,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: Timestamp(0),
        }
    }

    #[test]
    fn one_exact_edge_projects_exactly() {
        let edges = [edge(TERM, path("Maths"), EdgeKind::Exact)];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Exact { to: path("Maths") },
            "a single exact edge is the whole of the projection"
        );
    }

    #[test]
    fn two_exact_edges_are_ambiguous() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(TERM, path("Numeracy"), EdgeKind::Exact),
        ];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Ambiguous {
                candidates: vec![path("Maths"), path("Numeracy")]
            },
            "two exact targets have no correct answer and defer to a human"
        );
    }

    #[test]
    fn agreeing_broader_edges_broaden() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Broader),
            edge(TERM, path("Maths"), EdgeKind::Broader),
        ];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Broadened {
                to: path("Maths"),
                dropped: vec![TERM]
            },
            "broader edges naming one target are a broadening, not an ambiguity"
        );
    }

    #[test]
    fn disagreeing_broader_edges_are_ambiguous() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Broader),
            edge(TERM, path("Science"), EdgeKind::Broader),
        ];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Ambiguous {
                candidates: vec![path("Maths"), path("Science")]
            },
            "two broader targets are a choice, and the choice is the seller's"
        );
    }

    #[test]
    fn an_exact_edge_with_a_differing_broader_edge_is_ambiguous() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(TERM, path("Science"), EdgeKind::Broader),
        ];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Ambiguous {
                candidates: vec![path("Maths"), path("Science")]
            },
            "an exact and a broader naming different paths disagree"
        );
    }

    #[test]
    fn an_exact_edge_with_a_duplicate_broader_edge_reads_as_exact() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(TERM, path("Maths"), EdgeKind::Broader),
        ];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Exact { to: path("Maths") },
            "a broader edge duplicating the exact target is degenerate"
        );
    }

    const PRODUCT: ProductId = ProductId(Uuid([0x0f; 16]));

    fn binding(cardinality: Cardinality) -> AxisBinding {
        AxisBinding {
            axis: TermKind::Subject,
            native: "categories",
            cardinality,
            delegation: Delegation::ByOptIn,
        }
    }

    fn override_to(
        from: CanonicalTermId,
        to: VocabularyPath,
        kind: OverrideKind,
    ) -> ProjectionOverride {
        ProjectionOverride {
            org: OrgId(Uuid([0x0a; 16])),
            inventory: InventoryId::TesNz,
            axis: TermKind::Subject,
            from,
            to,
            kind,
            decided_by: Decider::Human {
                user: tam_types::UserId(Uuid([0x0b; 16])),
                org: OrgId(Uuid([0x0a; 16])),
            },
            decided_at: Timestamp(0),
        }
    }

    fn request(terms: &[CanonicalTermId], cardinality: Cardinality) -> AxisRequest<'_> {
        AxisRequest {
            product: PRODUCT,
            inventory: InventoryId::TesNz,
            binding: binding(cardinality),
            terms,
            sources: &[],
            pricing: PricingBranch::Free,
            rules: &[],
            settled: &[],
        }
    }

    const MANY: Cardinality = Cardinality::Many { cap: None };

    #[test]
    fn every_source_term_lands_in_exactly_one_bucket() {
        // The trichotomy's totality, enumerated rather than sampled: these
        // are every shape `project` can return for one term, plus the
        // no-counterpart branch, which is the only thing that turns an
        // `Absent` into an omission.
        let cases: [(&str, Vec<ProjectionEdge>, bool); 5] = [
            (
                "exact",
                vec![edge(TERM, path("Maths"), EdgeKind::Exact)],
                false,
            ),
            (
                "broadened",
                vec![edge(TERM, path("Maths"), EdgeKind::Broader)],
                false,
            ),
            (
                "ambiguous",
                vec![
                    edge(TERM, path("Maths"), EdgeKind::Broader),
                    edge(TERM, path("Science"), EdgeKind::Broader),
                ],
                false,
            ),
            ("absent", vec![], false),
            ("omitted", vec![], true),
        ];
        for (name, edges, recorded) in cases {
            let no_counterparts: Vec<(CanonicalTermId, VocabularyId)> = if recorded {
                vec![(TERM, TARGET)]
            } else {
                vec![]
            };
            let outcome = project_axis(request(&[TERM], MANY), &edges, &no_counterparts);
            let landed = usize::from(!outcome.resolved.is_empty())
                + outcome.gaps.len()
                + outcome.omitted.len();
            assert_eq!(
                landed, 1,
                "{name}: one source term is accounted for exactly once, and never twice \
                 or not at all"
            );
            assert!(
                outcome.unrecognised.is_empty(),
                "{name}: a canonical term id is by construction recognised; the bucket is \
                 for source paths the relation has never seen"
            );
        }
    }

    #[test]
    fn a_broadening_is_disclosed_and_does_not_block() {
        let edges = [edge(TERM, path("Maths"), EdgeKind::Broader)];
        let outcome = project_axis(request(&[TERM], MANY), &edges, &[]);
        assert_eq!(
            outcome.loss,
            vec![Loss::Broadened {
                to: path("Maths"),
                dropped: vec![TERM],
            }],
            "the broadening survives as a loss the seller sees"
        );
        assert!(
            outcome.is_publishable(),
            "a loss is disclosed, not decided, so it never holds up a publish"
        );
    }

    #[test]
    fn a_gap_names_which_of_the_two_unanswerable_shapes_it_was() {
        let ambiguous = [
            edge(TERM, path("Maths"), EdgeKind::Broader),
            edge(TERM, path("Science"), EdgeKind::Broader),
        ];
        let outcome = project_axis(request(&[TERM], MANY), &ambiguous, &[]);
        assert_eq!(
            outcome.gaps,
            vec![VocabularyGap {
                term: TERM,
                target: TARGET,
                projection: TermProjection::Ambiguous {
                    candidates: vec![path("Maths"), path("Science")],
                },
            }],
            "two competing edges are a defect in the relation, and the queue item says so"
        );
        assert!(
            !outcome.is_publishable(),
            "a gap blocks, which is what makes the queue a gate rather than a report"
        );
    }

    #[test]
    fn a_target_taking_one_elects_rather_than_picking() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(OTHER, path("Science"), EdgeKind::Exact),
        ];
        let outcome = project_axis(request(&[TERM, OTHER], Cardinality::One), &edges, &[]);
        assert_eq!(
            outcome.elections.len(),
            1,
            "one election for the axis, not one per value"
        );
        assert_eq!(
            outcome.elections[0].trigger,
            ElectionTrigger::ElectOne {
                from: vec![path("Maths"), path("Science")],
            },
            "the whole resolved set is the candidate list"
        );
    }

    /// The seller answers by naming a value, and the answer endpoint stores
    /// what they named without the target's own id for it. Every trigger but
    /// `Supply` checks the answer against the candidate set by whole-path
    /// equality, so an answer read back without the id its candidates carry
    /// matches nothing, settles nothing, and leaves the question standing
    /// however many times it is answered.
    #[test]
    fn an_answer_naming_a_value_without_its_id_settles_the_question_it_answers() {
        let subjects = [TERM, OTHER];
        let edges = [
            edge(TERM, named("Maths", "7000001"), EdgeKind::Exact),
            edge(OTHER, named("Science", "7000002"), EdgeKind::Exact),
        ];
        let settled = [SettledElection {
            product: PRODUCT,
            inventory: InventoryId::TesNz,
            axis: TermKind::Subject,
            trigger_kind: ElectionTriggerKind::ElectOne,
            trigger_key: None,
            chosen: vec![path("Science")],
        }];
        let mut ask = request(&subjects, Cardinality::One);
        ask.settled = &settled;
        let outcome = project_axis(ask, &edges, &[]);
        assert!(
            outcome.elections.is_empty(),
            "the seller picked one of the two, so the axis has nothing left to ask"
        );
        assert_eq!(
            outcome.resolved,
            vec![named("Science", "7000002")],
            "and what they picked travels under the target's own id, which the \
             relation names and the answer never did"
        );
    }

    #[test]
    fn an_over_cap_set_is_never_left_partially_truncated() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(OTHER, path("Science"), EdgeKind::Exact),
        ];
        let cardinality = Cardinality::Many {
            cap: Some(CountCap { limit: 1 }),
        };
        let outcome = project_axis(request(&[TERM, OTHER], cardinality), &edges, &[]);
        assert_eq!(
            outcome.elections[0].trigger,
            ElectionTrigger::OverCap {
                cap: 1,
                from: vec![path("Maths"), path("Science")],
            },
            "which to keep is the seller's, and the cap is stated in the question"
        );
        assert!(
            outcome.resolved.is_empty(),
            "a truncated set is exactly what a caller must not be able to publish by \
             accident, so none exists"
        );
    }

    #[test]
    fn a_set_within_its_cardinality_raises_nothing() {
        let edges = [edge(TERM, path("Maths"), EdgeKind::Exact)];
        for cardinality in [
            Cardinality::One,
            MANY,
            Cardinality::Many {
                cap: Some(CountCap { limit: 1 }),
            },
        ] {
            let outcome = project_axis(request(&[TERM], cardinality), &edges, &[]);
            assert_eq!(
                outcome,
                AxisOutcome {
                    resolved: vec![path("Maths")],
                    ..AxisOutcome::default()
                },
                "a set the target accepts is carried whole under {cardinality:?}"
            );
        }
    }

    #[test]
    fn a_required_axis_the_source_never_carried_asks_for_a_value() {
        let mut supply = request(&[], Cardinality::One);
        supply.binding.axis = TermKind::Licence;
        supply.binding.native = "licence";
        let outcome = project_axis(supply, &[], &[]);
        assert_eq!(
            outcome.elections.len(),
            1,
            "Tes refuses a create without a licence and nothing stated one"
        );
        assert_eq!(
            outcome.elections[0].trigger,
            ElectionTrigger::Supply {
                pricing: PricingBranch::Free,
            },
            "the pricing branch is carried because the API gates the write on it"
        );
    }

    #[test]
    fn an_optional_axis_the_source_never_carried_asks_nothing() {
        let outcome = project_axis(request(&[], MANY), &[], &[]);
        assert_eq!(
            outcome,
            AxisOutcome::default(),
            "an empty set on a field that records no refusal is a listing without \
             categories, not a question"
        );
    }

    #[test]
    fn narrower_edges_never_participate() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(TERM, path("Algebra"), EdgeKind::Narrower),
        ];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Exact { to: path("Maths") },
            "a narrower edge invents a distinction and cannot disturb an exact"
        );
        let narrower_only = [edge(TERM, path("Algebra"), EdgeKind::Narrower)];
        assert_eq!(
            project(TERM, TARGET, &narrower_only),
            TermProjection::Absent,
            "a narrower edge alone is not a projection"
        );
    }

    #[test]
    fn no_edges_is_absent() {
        assert_eq!(
            project(TERM, TARGET, &[]),
            TermProjection::Absent,
            "absent is a first-class answer, never a default term"
        );
    }

    #[test]
    fn edges_into_another_vocabulary_do_not_project() {
        let mut elsewhere = path("Maths");
        elsewhere.vocabulary = ELSEWHERE;
        let edges = [edge(TERM, elsewhere, EdgeKind::Exact)];
        assert_eq!(
            project(TERM, TARGET, &edges),
            TermProjection::Absent,
            "a vocabulary is per inventory and edges do not leak across them"
        );
    }

    #[test]
    fn two_terms_projecting_onto_one_path_include_it_once() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(OTHER, path("Maths"), EdgeKind::Exact),
        ];
        let outcome = project_terms(&[TERM, OTHER], TARGET, &edges, &[]);
        assert_eq!(
            outcome.included,
            vec![path("Maths")],
            "the listing carries each target path once"
        );
    }

    #[test]
    fn two_terms_broadening_onto_one_path_merge_into_one_loss() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Broader),
            edge(OTHER, path("Maths"), EdgeKind::Broader),
        ];
        let outcome = project_terms(&[TERM, OTHER], TARGET, &edges, &[]);
        assert_eq!(outcome.included, vec![path("Maths")]);
        assert_eq!(
            outcome.loss,
            vec![TermProjection::Broadened {
                to: path("Maths"),
                dropped: vec![TERM, OTHER]
            }],
            "one broadening in the field diff, carrying both dropped terms"
        );
        assert!(
            outcome.blocked.is_empty(),
            "a broadening proceeds and raises no reconciliation item"
        );
    }

    #[test]
    fn an_absent_term_with_a_no_counterpart_record_is_omitted() {
        let outcome = project_terms(&[TERM], TARGET, &[], &[(TERM, TARGET)]);
        assert_eq!(outcome.omitted, vec![TERM]);
        assert!(
            outcome.blocked.is_empty(),
            "a recorded no-counterpart proceeds by omission"
        );
    }

    #[test]
    fn a_no_counterpart_record_for_another_vocabulary_does_not_omit() {
        let outcome = project_terms(&[TERM], TARGET, &[], &[(TERM, ELSEWHERE)]);
        assert!(outcome.omitted.is_empty());
        assert_eq!(
            outcome.blocked,
            vec![BlockedTerm {
                term: TERM,
                projection: TermProjection::Absent
            }],
            "a no-counterpart record is per target vocabulary"
        );
    }

    #[test]
    fn an_ambiguous_term_never_omits() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(TERM, path("Numeracy"), EdgeKind::Exact),
        ];
        let outcome = project_terms(&[TERM], TARGET, &edges, &[(TERM, TARGET)]);
        assert!(
            outcome.omitted.is_empty(),
            "a no-counterpart record answers a gap, not a choice"
        );
        assert_eq!(
            outcome.blocked,
            vec![BlockedTerm {
                term: TERM,
                projection: TermProjection::Ambiguous {
                    candidates: vec![path("Maths"), path("Numeracy")]
                }
            }]
        );
    }

    #[test]
    fn ingest_reverses_an_exact_edge() {
        let edges = [edge(TERM, path("Maths"), EdgeKind::Exact)];
        assert_eq!(ingest(&path("Maths"), &edges), Some(TERM));
    }

    #[test]
    fn ingest_ignores_broader_and_narrower_edges() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Broader),
            edge(OTHER, path("Algebra"), EdgeKind::Narrower),
        ];
        assert_eq!(
            ingest(&path("Maths"), &edges),
            None,
            "inverting a broader edge would restore the distinction it dropped"
        );
        assert_eq!(ingest(&path("Algebra"), &edges), None);
    }

    #[test]
    fn ingest_matches_on_segments_and_ignores_the_native_id() {
        let edges = [edge(
            TERM,
            VocabularyPath {
                vocabulary: TARGET,
                segments: vec!["Maths".to_owned()],
                native_id: Some("1000454".to_owned()),
            },
            EdgeKind::Exact,
        )];
        assert_eq!(
            ingest(&path("Maths"), &edges),
            Some(TERM),
            "the segments are the path's identity"
        );
    }

    #[test]
    fn a_duplicated_reverse_target_fails_closed() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(OTHER, path("Maths"), EdgeKind::Exact),
        ];
        assert_eq!(
            ingest(&path("Maths"), &edges),
            None,
            "a corrupt relation reads as unmapped rather than picking a winner"
        );
    }

    #[test]
    fn an_unmapped_path_ingests_to_nothing() {
        assert_eq!(ingest(&path("Maths"), &[]), None);
    }

    /// Precedence, which is the whole of the override layer: where the seller
    /// has decided, the relation is not consulted for that pair at all.
    ///
    /// The failure this rules out is not "the relation wins" but something
    /// worse: an override folded in beside the global edge would give the
    /// term two paths in one vocabulary, which projects `Ambiguous` and
    /// blocks the publish. A seller stating a preference would stop their own
    /// listing going out.
    #[test]
    fn an_override_wins_over_every_global_edge_for_that_pair() {
        let edges = [edge(TERM, path("Maths"), EdgeKind::Exact)];
        let overrides = [override_to(TERM, path("Science"), OverrideKind::Exact)];
        let outcome = project_axis_with_overrides(request(&[TERM], MANY), &edges, &[], &overrides);
        assert_eq!(outcome.resolved, vec![path("Science")]);
        assert!(outcome.decided_by_seller);
        assert!(outcome.gaps.is_empty());
        assert!(outcome.elections.is_empty());
        assert!(
            outcome.is_publishable(),
            "an override settles rather than blocks"
        );
    }

    /// The other tenant's projection, which is the same call with an empty
    /// override set: one seller's decision is invisible to the relation every
    /// other seller reads.
    #[test]
    fn a_term_with_no_override_is_decided_by_the_relation() {
        let edges = [edge(TERM, path("Maths"), EdgeKind::Exact)];
        let outcome = project_axis(request(&[TERM], MANY), &edges, &[]);
        assert_eq!(outcome.resolved, vec![path("Maths")]);
        assert!(
            !outcome.decided_by_seller,
            "nothing was decided by a seller, so the field diff must not claim it was"
        );
    }

    /// An override for one term leaves every other term on the relation, so
    /// the layer is per-pair rather than per-axis.
    #[test]
    fn an_override_settles_its_own_term_and_no_other() {
        let edges = [
            edge(TERM, path("Maths"), EdgeKind::Exact),
            edge(OTHER, path("History"), EdgeKind::Exact),
        ];
        let overrides = [override_to(TERM, path("Science"), OverrideKind::Exact)];
        let outcome =
            project_axis_with_overrides(request(&[TERM, OTHER], MANY), &edges, &[], &overrides);
        assert_eq!(
            outcome.resolved,
            vec![path("History"), path("Science")],
            "the relation's term keeps its edge and the overridden one takes the seller's path"
        );
    }

    /// An override never suppresses a loss. A seller choosing a broader
    /// target is still broadening, and the disclosure is what makes the
    /// choice visible in the field diff rather than silent.
    #[test]
    fn an_overridden_broadening_still_discloses_the_loss() {
        let overrides = [override_to(TERM, path("Science"), OverrideKind::Broader)];
        let outcome = project_axis_with_overrides(request(&[TERM], MANY), &[], &[], &overrides);
        assert_eq!(outcome.resolved, vec![path("Science")]);
        assert_eq!(
            outcome.loss,
            vec![Loss::Broadened {
                to: path("Science"),
                dropped: vec![TERM],
            }]
        );
    }

    /// An overridden value counts against the target's cap exactly as a
    /// resolved one does. The seller choosing a value does not exempt the
    /// target from taking only one, so the overflow is still an election
    /// carrying the whole set rather than a truncation.
    #[test]
    fn an_override_counts_against_the_targets_cap() {
        let edges = [edge(OTHER, path("History"), EdgeKind::Exact)];
        let overrides = [override_to(TERM, path("Science"), OverrideKind::Exact)];
        let outcome = project_axis_with_overrides(
            request(&[TERM, OTHER], Cardinality::One),
            &edges,
            &[],
            &overrides,
        );
        assert!(outcome.resolved.is_empty());
        let [election] = &outcome.elections[..] else {
            panic!("one election over the two values: {:?}", outcome.elections);
        };
        let ElectionTrigger::ElectOne { from } = &election.trigger else {
            panic!("a single-valued axis elects one: {:?}", election.trigger);
        };
        assert_eq!(from, &vec![path("History"), path("Science")]);
    }
}
