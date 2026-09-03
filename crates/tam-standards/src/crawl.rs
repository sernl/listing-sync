//! The node-id crawl: what a capture records, and what a second capture proves
//! about whether a stored id is safe to post.
//!
//! Pure, so every part of it is exercised against recorded inputs without a
//! session. That is the point rather than a convenience: the kill gate in
//! `docs/notes/design/standards-ingestion.md` decides whether storing an id and
//! posting it later is a viable design at all, and a gate that could only be
//! run by crawling TPT could not be tested before it was depended on.
//!
//! Two checks live here and they are deliberately not conflated. The identity
//! check asks whether a node id still returns the name it returned when it was
//! bound; a changed name means the id now denotes a different standard, which
//! is the failure the gate exists to catch. The drift check asks whether the
//! statement still hashes the same under an unchanged name; that is TPT
//! rewording its prose, which is a reconciliation item and a refreshed hash
//! rather than a failure.
//!
//! Turning a crawled node into a binding is `crate::bind`, next door. The crawl
//! binary composes the two; neither needs the other's reasons.

use std::collections::BTreeMap;

use crate::format::content_hash;
use crate::model::Framework;
use crate::tpt::{TptBinding, TptNodeId, TptNodeIdTable};

/// TPT's own jurisdiction root per framework: the four subtrees a crawl walks,
/// out of the 166 roots `EducationStandardsJurisdictionsQuery` returns
/// (`docs/research/rethink/tpt-product-model.md:360`).
#[must_use]
pub const fn tpt_jurisdiction(framework: Framework) -> TptNodeId {
    match framework {
        Framework::Ccss => TptNodeId(3054),
        Framework::Ngss => TptNodeId(3055),
        Framework::Teks => TptNodeId(3326),
        Framework::VaSol => TptNodeId(5785),
    }
}

/// The notation TPT's own enumeration gives each of those four roots, polled
/// live on 2026-08-29 and recorded in `docs/design/data/tpt-vocabulary.json`.
///
/// A crawl reads it back before expanding anything, because an id is only an
/// id: `sphinxId` names a search index, the same fact the kill gate exists for,
/// and the only evidence that 3326 still means Texas is TPT saying so.
#[must_use]
pub const fn tpt_jurisdiction_notation(framework: Framework) -> &'static str {
    match framework {
        Framework::Ccss => "ccss",
        Framework::Ngss => "ngss",
        Framework::Teks => "teks",
        Framework::VaSol => "va sol",
    }
}

/// One node as a capture recorded it: the id the form would post, the name TPT
/// calls it, the statement TPT returned for it, and where in TPT's tree it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawledStandard {
    pub tpt_node_id: TptNodeId,
    /// TPT's `name`, which at a leaf is the published standard code.
    pub name: String,
    /// TPT's `descriptionText`, as returned.
    pub statement: String,
    /// TPT's `parentIds`, the full ancestor chain rather than one parent
    /// (`docs/research/rethink/tpt-product-model.md:352`). The join reads it to
    /// refuse a node that does not sit under the root the walk expanded, so a
    /// capture whose chain omitted the root would put every node in the residue
    /// under that reason: a visible refusal rather than a silent mis-scope.
    pub ancestry: Vec<TptNodeId>,
}

/// Reduce a statement to the form two transcriptions of one standard share.
///
/// Four differences are absorbed and no more: whitespace, letter case, the
/// Unicode quotation marks and dashes a publishing pipeline substitutes for
/// their ASCII forms, and a single trailing full stop. Anything wider is a real
/// difference in what the two sides say a standard says, and it belongs in the
/// residue where a human reads it rather than inside an equality that hides it.
///
/// Applied to both sides of every statement comparison and to the bytes the
/// recorded hash is taken over, so the equivalence that decides a binding is
/// the same one that later decides whether TPT reworded it.
#[must_use]
pub fn normalise_statement(statement: &str) -> String {
    let folded: String = statement
        .chars()
        .map(|character| match character {
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' | '\u{2032}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' => '"',
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect();
    let collapsed = folded.split_whitespace().collect::<Vec<&str>>().join(" ");
    collapsed
        .strip_suffix('.')
        .unwrap_or(collapsed.as_str())
        .to_owned()
}

/// The hash a binding records for the statement TPT returned, taken over the
/// normalised form for the reason above.
#[must_use]
pub fn statement_hash(statement: &str) -> String {
    content_hash(normalise_statement(statement).as_bytes())
}

/// A bound id whose node now answers to a different name. This is the failure
/// the kill gate is about: the id denotes a different standard, and a product
/// posted with it is tagged wrongly and silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedId {
    pub tpt_node_id: TptNodeId,
    pub source_guid: String,
    pub bound_name: String,
    pub crawled_name: String,
}

/// A bound id whose node answers to the same name and a different statement.
/// TPT reworded a standard: a reconciliation item and a refreshed hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftedStatement {
    pub tpt_node_id: TptNodeId,
    pub source_guid: String,
    pub bound_hash: String,
    pub crawled_hash: String,
}

/// What a second capture said about the bindings the first one wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CrawlDiff {
    /// Bindings whose name and statement both still hold.
    pub agreed: usize,
    pub moved: Vec<MovedId>,
    pub drifted: Vec<DriftedStatement>,
    /// Bound ids the new capture carries no node for. Not an identity failure:
    /// the capture answered no question about them, and a walk that stopped
    /// short would otherwise read as evidence.
    pub absent: Vec<TptNodeId>,
}

impl CrawlDiff {
    /// The bindings the capture actually answered for, which is the denominator
    /// the decision rule reads.
    #[must_use]
    pub fn checked(&self) -> usize {
        self.agreed
            .saturating_add(self.moved.len())
            .saturating_add(self.drifted.len())
    }
}

/// Compare a capture against the bindings in force.
///
/// The identity check reads `tpt_name` rather than `code`, because the two are
/// not the same string: TPT names the node `CCRA.L.1` where the mirror codes it
/// `CCSS.ELA-Literacy.CCRA.L.1`. Comparing the bound code to the crawled name
/// would report every Common Core binding as moved and kill the feature on an
/// artefact of our own spelling.
#[must_use]
pub fn diff_capture(bound: &TptNodeIdTable, crawled: &[CrawledStandard]) -> CrawlDiff {
    let capture: BTreeMap<TptNodeId, &CrawledStandard> = crawled
        .iter()
        .map(|node| (node.tpt_node_id, node))
        .collect();
    let mut diff = CrawlDiff::default();
    for binding in bound.iter() {
        let Some(node) = capture.get(&binding.tpt_node_id) else {
            diff.absent.push(binding.tpt_node_id);
            continue;
        };
        if node.name != binding.tpt_name {
            diff.moved.push(MovedId {
                tpt_node_id: binding.tpt_node_id,
                source_guid: binding.source_guid.clone(),
                bound_name: binding.tpt_name.clone(),
                crawled_name: node.name.clone(),
            });
            continue;
        }
        let crawled_hash = statement_hash(&node.statement);
        if crawled_hash == binding.statement_sha256 {
            diff.agreed = diff.agreed.saturating_add(1);
        } else {
            diff.drifted.push(DriftedStatement {
                tpt_node_id: binding.tpt_node_id,
                source_guid: binding.source_guid.clone(),
                bound_hash: binding.statement_sha256.clone(),
                crawled_hash,
            });
        }
    }
    diff
}

/// What the second capture decides about storing ids at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateVerdict {
    /// No id moved. Store and post, with the per-post verification as the
    /// standing check.
    StoreAndPost,
    /// Fewer than one in a hundred moved. Store and post, and take the capture
    /// quarterly rather than on the slower cadence.
    StoreAndPostWithQuarterlyCapture,
    /// One in a hundred or more moved. Stored ids are dead; a device resolves
    /// the id at post time by expanding the subtree from the code, which costs
    /// a handful of requests against the picker's own 58. A degradation rather
    /// than a project kill.
    ResolveAtPostTime,
    /// The capture answered for no binding in force, so nothing was measured.
    /// Named rather than folded into `StoreAndPost`, because a gate that ran
    /// against nothing has not run.
    Inconclusive,
}

/// The decision rule, read across the whole re-crawl and off the identity check
/// alone. Drift moves no id and does not enter it.
#[must_use]
pub fn gate(diff: &CrawlDiff) -> GateVerdict {
    let checked = diff.checked();
    if checked == 0 {
        return GateVerdict::Inconclusive;
    }
    let moved = diff.moved.len();
    if moved == 0 {
        return GateVerdict::StoreAndPost;
    }
    if moved.saturating_mul(100) < checked {
        return GateVerdict::StoreAndPostWithQuarterlyCapture;
    }
    GateVerdict::ResolveAtPostTime
}

/// The RFC 3339 UTC shape every timestamp in this pair carries: fixed width,
/// second resolution, `Z`. Fixed width is what makes a byte comparison a time
/// comparison, which is what the window below rests on.
///
/// Public because a crawl checks its own argument against it before spending a
/// request, and a second definition of the shape is a second definition of what
/// the window orders.
#[must_use]
pub fn is_utc_timestamp(text: &str) -> bool {
    text.len() == 20
        && text.ends_with('Z')
        && text
            .as_bytes()
            .iter()
            .zip(b"0000-00-00T00:00:00Z")
            .all(|(found, pattern)| match pattern {
                b'0' => found.is_ascii_digit(),
                other => found == other,
            })
}

/// The span a caller will post bindings from: everything verified at or after
/// the capture that opened it.
///
/// A binding outside it is not posted under any branch of the gate. An id
/// aliased as `sphinxId` names a search index, search indexes get rebuilt, and
/// an id no current capture vouches for may now denote a different standard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawlWindow {
    since: String,
}

impl CrawlWindow {
    /// Refuses a timestamp that is not the shape above, rather than admitting
    /// one whose comparison would be meaningless.
    #[must_use]
    pub fn opened_at(timestamp: &str) -> Option<Self> {
        is_utc_timestamp(timestamp).then(|| Self {
            since: timestamp.to_owned(),
        })
    }

    #[must_use]
    pub fn since(&self) -> &str {
        &self.since
    }

    #[must_use]
    pub fn admits(&self, binding: &TptBinding) -> bool {
        is_utc_timestamp(&binding.verified_at)
            && binding.verified_at.as_str() >= self.since.as_str()
    }
}

/// Why a tagged standard cannot be posted to TPT.
///
/// The adapter names the same two reasons in its own vocabulary; the decision
/// is made here, where the table and the window are, and carried across by the
/// caller. Neither crate depends on the other, which is what keeps an adapter
/// from resolving a code by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotPostable {
    /// No crawl has bound this standard. Every standard today.
    Unbound,
    /// A binding exists and the current window does not vouch for it.
    OutsideCrawlWindow,
}

/// The id to post for one tagged standard, or why there is none.
pub fn postable(
    bound: &TptNodeIdTable,
    window: &CrawlWindow,
    source_guid: &str,
) -> Result<TptNodeId, NotPostable> {
    let binding = bound.get(source_guid).ok_or(NotPostable::Unbound)?;
    if window.admits(binding) {
        Ok(binding.tpt_node_id)
    } else {
        Err(NotPostable::OutsideCrawlWindow)
    }
}

#[cfg(test)]
mod tests;
