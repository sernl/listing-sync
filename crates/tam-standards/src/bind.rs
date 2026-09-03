//! The join: how a node TPT returned becomes a binding against a row of our
//! own catalogue.
//!
//! Beside `crate::crawl` rather than inside it, because walking TPT's tree and
//! binding to the mirror are different responsibilities and only the crawl
//! binary needs both. This module never asks what a capture is worth; that is
//! the gate's question.
//!
//! The join is harder than a code comparison, because a published code is not
//! an identity on either side. TPT names a node `CCRA.L.1` where the mirror
//! codes the same standard `CCSS.ELA-Literacy.CCRA.L.1` and gives it a third
//! form, `CCR.L.1`, in `alt_code`, so equality alone binds nothing in Common
//! Core. And `1.1.A` is one standard in Texas mathematics and another in Texas
//! science, so equality alone binds the wrong one there. What the two sides do
//! share is the statement, which is why a binding needs the code to correspond
//! and the statement to agree, and why everything else is reported rather than
//! guessed at.
//!
//! Two invariants shape the result and they are not symmetric. One node binding
//! many rows is allowed and expected: the mirror repeats a Common Core anchor
//! standard across eleven grade sets, and each of those rows is a row a seller
//! tags. One row bound by two nodes is refused, because a row bound twice
//! cannot be tagged at all — there would be no answer to which id to post — so
//! a contested row goes to the residue with both claimants named.
//!
//! Scope is what keeps the second invariant reachable. Verbatim adoptions put
//! one statement under more than one jurisdiction, and TPT carries a tree for
//! every state beside the Common Core one, so an unscoped join would let a
//! state node and a Common Core node both claim one mirror row. Every candidate
//! is therefore scoped to the jurisdiction the node sits under, twice over: the
//! catalogue index a walk consults holds one framework's rows and no other, and
//! a node whose own ancestor chain does not carry the walked root is refused
//! before its code is even read.

use std::collections::BTreeMap;

use crate::crawl::{normalise_statement, statement_hash, CrawledStandard};
use crate::model::Framework;
use crate::search::fold_code;
use crate::tpt::{TptBinding, TptNodeId};

/// Which mirror jurisdiction a walked TPT subtree corresponds to.
///
/// An input to the join rather than something it derives, so the
/// correspondence is stated once by the caller that chose which subtree to
/// walk. The coarse correspondence is the four roots the create form offers.
/// A finer one — a TPT subject or domain node to a mirror set — does not exist
/// until a capture does, and this shape is what a later one narrows rather
/// than replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JurisdictionScope {
    pub framework: Framework,
    /// The jurisdiction root the walk expanded, which every node it answered
    /// with must sit under.
    pub root: TptNodeId,
}

/// One catalogue row a crawled node could name, borrowed from a loaded
/// framework rather than copied out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogueRow<'a> {
    pub source_guid: &'a str,
    pub subject: &'a str,
    pub code: &'a str,
    pub alt_code: Option<&'a str>,
    pub statement: &'a str,
}

/// Every dotted suffix of a code whose removed prefix carries no digit.
///
/// This is what lets `CCSS.ELA-Literacy.CCRA.L.1` be named `CCRA.L.1`, and
/// what stops `5.10.A` from being named `10.A`: TPT drops a namespace prefix,
/// which is letters, and never drops a level of the code itself, which is
/// where the digits are.
fn dot_suffixes(code: &str) -> Vec<&str> {
    code.match_indices('.')
        .filter_map(|(index, _)| {
            let boundary = index.saturating_add(1);
            let prefix = code.get(..boundary)?;
            if prefix.chars().any(|character| character.is_ascii_digit()) {
                return None;
            }
            code.get(boundary..)
        })
        .filter(|suffix| !suffix.is_empty())
        .collect()
}

/// Every name a row answers to, folded: its code and its bare code exactly, and
/// the namespace-stripped suffixes of its code as fallbacks.
///
/// The one definition of code correspondence. The index is built from it and
/// the join reads only the index, so no second rule exists that could tighten
/// or loosen on its own — which is what a predicate beside this one, blind to
/// `alt_code` and called by nothing, had become.
fn keys_of(row: &CatalogueRow<'_>) -> (Vec<String>, Vec<String>) {
    let folded = |code: &str| {
        let key = fold_code(code);
        (!key.is_empty()).then_some(key)
    };
    let exact = [Some(row.code), row.alt_code]
        .into_iter()
        .flatten()
        .filter_map(folded)
        .collect();
    let suffix = dot_suffixes(row.code)
        .into_iter()
        .filter_map(folded)
        .collect();
    (exact, suffix)
}

/// Index one row under one key, once.
///
/// The guard is the invariant rather than a precaution: a row reaches the same
/// key by more than one route, and a key offering it twice makes `bind_walk`
/// write the same binding twice, which produces a file `load_tpt_node_ids`
/// refuses as a key bound twice. Two routes are known. A row whose `code` and
/// `alt_code` fold alike is indexed twice in the exact tier; and a code with a
/// segment made entirely of punctuation is indexed twice in the suffix tier,
/// because the fold drops that segment and two of its suffixes become one key.
/// The committed NGSS row coded `ESS.!.A` is the second, whose suffixes `!.A`
/// and `A` are both the key `A`.
fn index_under(map: &mut BTreeMap<String, Vec<usize>>, key: &str, position: usize) {
    let positions = map.entry(key.to_owned()).or_default();
    if !positions.contains(&position) {
        positions.push(position);
    }
}

/// One framework's catalogue rows, keyed by the forms a TPT name can take.
///
/// Two tiers. A row's own code and bare code are exact keys; its
/// namespace-stripped suffixes are fallback keys. Both are offered for a name
/// and the exact tier is offered first, which orders the answer without
/// filtering it: the statement is the discriminator, and a suffix-tier row
/// withheld because some unrelated row carried the name exactly would be
/// reported as disagreeing with a statement it was never shown.
///
/// One index holds one framework's rows, which is the first half of the
/// jurisdiction scope: a walk of the Texas subtree is offered no Common Core
/// row to bind, whatever a code collision would otherwise allow.
#[derive(Debug)]
pub struct CatalogueIndex<'a> {
    rows: Vec<CatalogueRow<'a>>,
    exact: BTreeMap<String, Vec<usize>>,
    suffix: BTreeMap<String, Vec<usize>>,
}

impl<'a> CatalogueIndex<'a> {
    #[must_use]
    pub fn build(rows: Vec<CatalogueRow<'a>>) -> Self {
        let mut exact: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut suffix: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (position, row) in rows.iter().enumerate() {
            let (exact_keys, suffix_keys) = keys_of(row);
            for key in exact_keys {
                index_under(&mut exact, &key, position);
            }
            for key in suffix_keys {
                index_under(&mut suffix, &key, position);
            }
        }
        Self {
            rows,
            exact,
            suffix,
        }
    }

    #[must_use]
    pub fn rows(&self) -> &[CatalogueRow<'a>] {
        &self.rows
    }

    /// Every row a name reaches, exact tier first and each row once. A row
    /// whose code is entirely punctuation before its last segment can sit in
    /// both tiers under one key, which is why the union is deduplicated here as
    /// well as within each tier.
    fn positions(&self, tpt_name: &str) -> Vec<usize> {
        let folded = fold_code(tpt_name);
        if folded.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<usize> = Vec::new();
        for tier in [self.exact.get(&folded), self.suffix.get(&folded)] {
            for position in tier.into_iter().flatten() {
                if !found.contains(position) {
                    found.push(*position);
                }
            }
        }
        found
    }

    /// Every row a TPT name could be naming, exact tier first.
    #[must_use]
    pub fn candidates(&self, tpt_name: &str) -> Vec<CatalogueRow<'a>> {
        self.positions(tpt_name)
            .into_iter()
            .filter_map(|position| self.rows.get(position).copied())
            .collect()
    }
}

/// Why a crawled node produced no binding, or lost one it would have had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// The node's own ancestor chain does not carry the root this walk
    /// expanded, so it belongs to some other jurisdiction's tree and is not
    /// this framework's to bind.
    OutsideJurisdiction,
    /// No catalogue row of this framework carries the code TPT named.
    NoCatalogueRow,
    /// Rows carry the code and none of their statements agree with TPT's.
    /// Binding one of them anyway is how a product acquires a tag naming a
    /// standard nobody chose.
    StatementDisagrees { candidates: usize },
    /// The row this node would bind is claimed by another node of the same
    /// walk. A row bound twice has no answer to which id to post, so neither
    /// claim is kept and both are named here.
    RowClaimedTwice {
        source_guid: String,
        others: Vec<TptNodeId>,
    },
}

/// One crawled node the join refused, with what it would take to explain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residue {
    pub framework: Framework,
    pub tpt_node_id: TptNodeId,
    pub name: String,
    pub reason: Unresolved,
}

/// What one walk's join produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BindOutcome {
    pub bindings: Vec<TptBinding>,
    pub residue: Vec<Residue>,
}

/// One node's claim on one catalogue row, before the contested ones are
/// dropped.
struct Claim {
    node: usize,
    row: usize,
}

fn refuse(scope: &JurisdictionScope, node: &CrawledStandard, reason: Unresolved) -> Residue {
    Residue {
        framework: scope.framework,
        tpt_node_id: node.tpt_node_id,
        name: node.name.clone(),
        reason,
    }
}

/// Bind one walk's crawled nodes to the catalogue rows of the framework it
/// corresponds to.
///
/// Two passes, because the one-row-one-node invariant is not decidable node by
/// node. The first pass turns each node into the rows it agrees with, or into
/// residue. The second drops every row two nodes reached and names both
/// claimants, leaving the rows exactly one node reached as bindings.
#[must_use]
pub fn bind_walk(
    scope: &JurisdictionScope,
    crawled: &[CrawledStandard],
    catalogue: &CatalogueIndex<'_>,
    verified_at: &str,
) -> BindOutcome {
    let mut residue = Vec::new();
    let mut claims: Vec<Claim> = Vec::new();

    for (position, node) in crawled.iter().enumerate() {
        if !node.ancestry.contains(&scope.root) {
            residue.push(refuse(scope, node, Unresolved::OutsideJurisdiction));
            continue;
        }
        let candidates = catalogue.positions(&node.name);
        if candidates.is_empty() {
            residue.push(refuse(scope, node, Unresolved::NoCatalogueRow));
            continue;
        }
        let statement = normalise_statement(&node.statement);
        let agreeing: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|row| {
                catalogue
                    .rows
                    .get(*row)
                    .is_some_and(|found| normalise_statement(found.statement) == statement)
            })
            .collect();
        if agreeing.is_empty() {
            residue.push(refuse(
                scope,
                node,
                Unresolved::StatementDisagrees {
                    candidates: candidates.len(),
                },
            ));
            continue;
        }
        for row in agreeing {
            claims.push(Claim {
                node: position,
                row,
            });
        }
    }

    let mut claimants: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for claim in &claims {
        let nodes = claimants.entry(claim.row).or_default();
        if !nodes.contains(&claim.node) {
            nodes.push(claim.node);
        }
    }

    let mut bindings = Vec::new();
    for claim in &claims {
        let Some(nodes) = claimants.get(&claim.row) else {
            continue;
        };
        let (Some(node), Some(row)) = (crawled.get(claim.node), catalogue.rows.get(claim.row))
        else {
            continue;
        };
        if nodes.len() > 1 {
            let others = nodes
                .iter()
                .filter(|other| **other != claim.node)
                .filter_map(|other| crawled.get(*other).map(|found| found.tpt_node_id))
                .collect();
            residue.push(refuse(
                scope,
                node,
                Unresolved::RowClaimedTwice {
                    source_guid: row.source_guid.to_owned(),
                    others,
                },
            ));
            continue;
        }
        bindings.push(TptBinding {
            framework: scope.framework,
            source_guid: row.source_guid.to_owned(),
            subject: row.subject.to_owned(),
            code: row.code.to_owned(),
            tpt_node_id: node.tpt_node_id,
            tpt_name: node.name.clone(),
            statement_sha256: statement_hash(&node.statement),
            verified_at: verified_at.to_owned(),
        });
    }

    BindOutcome { bindings, residue }
}

#[cfg(test)]
mod tests;
