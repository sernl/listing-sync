//! The projection function in both directions.
//!
//! Outbound, `project` decides on the edge set alone: no fallback, no
//! nearest-neighbour search and no model proposal, per the five-row table in
//! `docs/design/taxonomy-projection.md`. Inbound, `ingest` is the reverse of
//! the `Exact` relation only, because inverting a `Broader` edge would restore
//! the distinction the edge dropped.

use tam_domain::{EdgeKind, ProjectionEdge, TermProjection, VocabularyId, VocabularyPath};
use tam_types::CanonicalTermId;

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

#[cfg(test)]
mod tests {
    use super::{ingest, project, project_terms, BlockedTerm};
    use tam_domain::{
        Decider, EdgeKind, ProjectionEdge, TermKind, TermProjection, VocabularyId, VocabularyPath,
    };
    use tam_types::{CanonicalTermId, InventoryId, Timestamp, Uuid};

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
}
