//! What a projection could not settle on its own, named apart.
//!
//! Four things can go wrong projecting one axis and they have four different
//! remedies, so they are four types rather than one error. A *gap* is a
//! question about a vocabulary pair: answering it writes a durable edge and
//! every later product finds it waiting. An *election* is a question about
//! one product against one target: it cannot deduplicate across products, so
//! its reuse mechanism is a standing rule instead. A *loss* is not a question
//! at all — it is disclosed and never blocks. And an *unrecognised* source
//! value is none of the three: `reconciliation_item.term` references
//! `canonical_term`, so a value the relation has never seen cannot become a
//! queue item, and calling it a loss would assert the target has no such
//! field when the truth is that we do not know what the value is.

use crate::registry::{registry, AxisBinding, Delegation, NonDelegable};
use crate::{Decider, EdgeKind, TermKind, TermProjection, VocabularyId, VocabularyPath};
use tam_types::{CanonicalTermId, InventoryId, OrgId, ProductId, Timestamp};

/// Which side of the free/paid gate a product sits on. Carried on a `Supply`
/// trigger because Tes's own API gates the write on it — a Creative Commons
/// value with a price is refused, and `TES-PAID` without one is too — so a
/// standing rule that ignored the branch would be unwritable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingBranch {
    Free,
    Paid,
}

impl PricingBranch {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Paid => "paid",
        }
    }
}

/// A question about a vocabulary pair rather than about a product. Answering
/// one writes an edge, which is why the queue behind it drains: the first
/// product asks and every later one finds the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyGap {
    pub term: CanonicalTermId,
    pub target: VocabularyId,
    /// `Absent` or `Ambiguous`, carried so the queue item names which of the
    /// two it was rather than flattening both to "unmapped".
    pub projection: TermProjection,
}

/// A decision the source data cannot supply and no edge can settle. Distinct
/// from a `VocabularyGap`, which is a question about a vocabulary; this is a
/// question about this product against this target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Election {
    pub product: ProductId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger: ElectionTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElectionTrigger {
    /// The target requires a value in an axis the source never carried.
    /// TPT-to-Tes licence: TPT has no licence field, so nothing was stated,
    /// and Tes refuses a create without one.
    Supply { pricing: PricingBranch },
    /// The target takes one and the resolved set holds several. Tes
    /// `primaryCategory` and `mainType`.
    ElectOne { from: Vec<VocabularyPath> },
    /// The target's measured count cap is smaller than the resolved set.
    /// Which to keep is the seller's; truncating would publish a listing
    /// silently narrower than the one they authored.
    OverCap {
        cap: usize,
        from: Vec<VocabularyPath>,
    },
    /// The source value is broader than any single target value, and the
    /// target takes several. One Tes GB age band covers several US year
    /// groups, which is a `Narrower` relation that `project` will never
    /// derive across, so the seller picks. Distinct from `Ambiguous`, which
    /// is two competing edges of the same kind and is a defect in the
    /// relation rather than a question for a seller.
    Narrow {
        from: VocabularyPath,
        candidates: Vec<VocabularyPath>,
    },
}

/// The trigger's discriminant, which is `election_item`'s dedup column and
/// `election_rule`'s. Named apart from the trigger itself because the payload
/// is a question and the kind is a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElectionTriggerKind {
    Supply,
    ElectOne,
    OverCap,
    Narrow,
}

impl ElectionTriggerKind {
    pub const ALL: [Self; 4] = [Self::Supply, Self::ElectOne, Self::OverCap, Self::Narrow];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supply => "supply",
            Self::ElectOne => "elect_one",
            Self::OverCap => "over_cap",
            Self::Narrow => "narrow",
        }
    }
}

impl ElectionTrigger {
    #[must_use]
    pub const fn kind(&self) -> ElectionTriggerKind {
        match *self {
            Self::Supply { .. } => ElectionTriggerKind::Supply,
            Self::ElectOne { .. } => ElectionTriggerKind::ElectOne,
            Self::OverCap { .. } => ElectionTriggerKind::OverCap,
            Self::Narrow { .. } => ElectionTriggerKind::Narrow,
        }
    }

    /// What discriminates one standing answer from another within an axis.
    ///
    /// `Supply` keys on the pricing branch, because the write is gated on it.
    /// `Narrow` keys on the source value's own native id: a band answer is a
    /// fact about a vocabulary member, and without the key a seller migrating
    /// five hundred GB listings answers the same band question five hundred
    /// times. `ElectOne` and `OverCap` ask about one product's own resolved
    /// set, which generalises to nothing, so they carry no key and no
    /// standing rule can answer them.
    #[must_use]
    pub fn key(&self) -> Option<String> {
        match self {
            Self::Supply { pricing } => Some(pricing.as_str().to_owned()),
            // Total, because the key is a database column tied to the trigger
            // kind by a CHECK: a narrow question stored keyless would be a
            // standing answer to every band rather than to this one.
            Self::Narrow { from, .. } => Some(
                from.native_id
                    .clone()
                    .unwrap_or_else(|| from.segments.join("/")),
            ),
            Self::ElectOne { .. } | Self::OverCap { .. } => None,
        }
    }
}

/// A fact about what a projection could not carry, surfaced to the seller
/// before publish and recorded against the mapping after it. Every variant
/// names something that existed on the source and does not exist on the
/// target, so a loss is always attributable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loss {
    /// A canonical term projected onto a broader target. TPT grades into Tes
    /// GB age bands.
    Broadened {
        to: VocabularyPath,
        dropped: Vec<CanonicalTermId>,
    },
    /// The source carried a value in an axis the target was measured to have
    /// no field for. The Tes-to-TPT licence drop: TPT has nowhere to put a
    /// Creative Commons grant, so no edge could ever exist and no queue item
    /// should ever be raised.
    NoTargetField {
        axis: TermKind,
        value: VocabularyPath,
    },
    /// Several source values collapsed onto one target value, or onto an
    /// undifferentiated one. The values survive and the distinction does not.
    Collapsed {
        to: VocabularyPath,
        from: Vec<VocabularyPath>,
    },
    /// The seller elected a subset because the target took fewer than the set
    /// held. Recorded with its decider so the choice is auditable rather than
    /// looking like a silent drop.
    Elected {
        kept: Vec<VocabularyPath>,
        dropped: Vec<VocabularyPath>,
        by: Decider,
    },
}

/// The kinds of loss, for the client vocabulary and for a report that counts
/// them without carrying their payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossKind {
    Broadened,
    NoTargetField,
    Collapsed,
    Elected,
}

impl LossKind {
    pub const ALL: [Self; 4] = [
        Self::Broadened,
        Self::NoTargetField,
        Self::Collapsed,
        Self::Elected,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Broadened => "broadened",
            Self::NoTargetField => "no_target_field",
            Self::Collapsed => "collapsed",
            Self::Elected => "elected",
        }
    }
}

impl Loss {
    #[must_use]
    pub const fn kind(&self) -> LossKind {
        match *self {
            Self::Broadened { .. } => LossKind::Broadened,
            Self::NoTargetField { .. } => LossKind::NoTargetField,
            Self::Collapsed { .. } => LossKind::Collapsed,
            Self::Elected { .. } => LossKind::Elected,
        }
    }
}

/// The outcome of projecting one axis's whole source set into one target
/// inventory. Partial resolution is normal: what resolved is carried, what
/// did not is named, and the caller decides whether to enqueue it or render
/// it. Nothing here knows about a job, a seam or an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AxisOutcome {
    pub resolved: Vec<VocabularyPath>,
    /// Never blocks. A loss is disclosed, not decided.
    pub loss: Vec<Loss>,
    /// Blocks. Deduplicated per (term, target vocabulary); answered once and
    /// reused by every later product.
    pub gaps: Vec<VocabularyGap>,
    /// Blocks. Per (product, inventory, axis, trigger); answerable in advance
    /// by a standing rule.
    pub elections: Vec<Election>,
    /// Source values the relation does not recognise at all. Neither a gap
    /// nor a loss, and carried out rather than dropped.
    pub unrecognised: Vec<VocabularyPath>,
    /// A `NoCounterpart` record says to drop these, and it proceeds.
    pub omitted: Vec<CanonicalTermId>,
    /// Whether a seller's own override decided any part of this outcome, so
    /// the field diff can say "you set this" rather than "the relation says
    /// this". It also bounds the blast radius of a re-poll: a later change to
    /// the global relation cannot silently change what an overridden term
    /// published as, and the seller can be told which of their listings rest
    /// on their own decision rather than ours.
    pub decided_by_seller: bool,
}

impl AxisOutcome {
    /// The publish gate. `loss` is deliberately not consulted: that is the
    /// whole difference between disclosing and deciding.
    #[must_use]
    pub fn is_publishable(&self) -> bool {
        self.gaps.is_empty() && self.elections.is_empty()
    }
}

/// What may settle an equivalence the edge relation did not settle. There is
/// no `Auto` variant: a clean edge resolves in `project` before any mode is
/// consulted, so a suggestion cannot attach to a clean projection because a
/// clean projection is a different type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The seller decides. No suggestion is computed, so none can be shown as
    /// ours.
    SellerDecides,
    /// The seller asked us to decide. Only here does a suggestion exist.
    BestFit,
}

/// Total, and `Never` wins over any opt-in: a legal axis is the seller's
/// whatever they have asked us to do elsewhere.
#[must_use]
pub const fn resolution_for(binding: AxisBinding, opted_in: bool) -> Mode {
    match (binding.delegation, opted_in) {
        (Delegation::Never(_), _) | (Delegation::ByOptIn, false) => Mode::SellerDecides,
        (Delegation::ByOptIn, true) => Mode::BestFit,
    }
}

/// One value a best-fit ranking may suggest: a value the projection already
/// resolved, and the edge the relation reached it by.
///
/// The edge is carried rather than looked up because the ranking is pure and
/// the relation is a database read. It is what makes `Exact` outrank
/// `Broader`: a target value the source names exactly is a better answer than
/// one that merely contains it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: VocabularyPath,
    pub edge: EdgeKind,
}

/// One value of the resolved set with its rank recorded: the same shape as a
/// [`Candidate`] except that the edge may be absent.
///
/// Absent means the candidate set named no edge for this value. It is still
/// in the resolved set, which is what admits it to the ranking; asserting an
/// edge kind for it would be a claim the relation never made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub path: VocabularyPath,
    pub edge: Option<EdgeKind>,
}

/// What best fit would pick, and what picking it leaves behind.
///
/// Never an answer. An election resolves only on explicit confirmation, so
/// this is offered pre-selected and the question stands until the seller
/// says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The values suggested, highest ranked first, and never more than the
    /// target will take.
    pub keep: Vec<Ranked>,
    /// The candidates the target's cardinality left behind, so a suggestion
    /// that drops something names what it dropped rather than dropping it
    /// quietly. Empty where nothing was dropped.
    pub dropped: Vec<Ranked>,
}

/// Rank order: an exact edge beats a broader one, a broader one beats a
/// narrower one, and a value the candidate set named no edge for ranks below
/// all three.
///
/// `Narrower` has a rank because a `Narrow` trigger's candidates are reached
/// by narrower edges and nothing else; there it makes every candidate equal
/// and the seller's own order decides, which is the intended outcome.
const fn edge_rank(edge: Option<EdgeKind>) -> u8 {
    match edge {
        Some(EdgeKind::Exact) => 0,
        Some(EdgeKind::Broader) => 1,
        Some(EdgeKind::Narrower) => 2,
        None => 3,
    }
}

/// The resolved set one trigger asks the seller to choose from, and how many
/// of it the target will take. `Supply` has neither: the source never carried
/// a value, so there is no resolved set and nothing to rank.
fn offered(trigger: &ElectionTrigger) -> Option<(&[VocabularyPath], usize)> {
    match trigger {
        ElectionTrigger::Supply { .. } => None,
        ElectionTrigger::ElectOne { from } => Some((from, 1)),
        ElectionTrigger::OverCap { cap, from } => Some((from, *cap)),
        // A band that covers eight year groups means eight, so a narrow
        // suggestion keeps every candidate under the band.
        ElectionTrigger::Narrow { candidates, .. } => Some((candidates, candidates.len())),
    }
}

/// What best fit suggests for one election, or `None` where it declines.
///
/// It declines in four situations and each is a refusal to invent. The axis
/// refuses delegation, which `resolution_for` decides and `Never` wins
/// outright — the licence is never suggested however the seller has opted in
/// elsewhere. The seller has not opted in, so no suggestion is ours to make.
/// The trigger is a `Supply`, where the source carried nothing and there is
/// no resolved set to rank. Or the resolved set is empty, where the
/// projection settled nothing and the question stands.
///
/// The ranking is strictly over the set the trigger carries and never over
/// the target's whole vocabulary, which is the constraint that keeps a
/// suggestion from becoming an invention. Order is the edge kind first and
/// the seller's own stated order second: the sort is stable, so candidates of
/// one kind keep the order the seller gave them. A candidate `candidates`
/// names no edge for still ranks and still may be suggested — it is in the
/// resolved set, which is what admits it — but it ranks below every candidate
/// whose edge is known.
#[must_use]
pub fn best_fit(
    binding: AxisBinding,
    opted_in: bool,
    trigger: &ElectionTrigger,
    candidates: &[Candidate],
) -> Option<Suggestion> {
    if resolution_for(binding, opted_in) != Mode::BestFit {
        return None;
    }
    let (paths, cap) = offered(trigger)?;
    let mut ranked: Vec<Ranked> = paths
        .iter()
        .map(|path| Ranked {
            path: path.clone(),
            edge: candidates
                .iter()
                .find(|candidate| candidate.path == *path)
                .map(|candidate| candidate.edge),
        })
        .collect();
    ranked.sort_by_key(|candidate| edge_rank(candidate.edge));
    let dropped = ranked.split_off(cap.min(ranked.len()));
    (!ranked.is_empty()).then_some(Suggestion {
        keep: ranked,
        dropped,
    })
}

/// Which of the two projecting kinds an override may assert.
///
/// `Narrower` is absent rather than forgotten: a narrower edge never
/// participates in an outbound projection, so an override producing one would
/// be a decision the seller could make and never observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideKind {
    Exact,
    Broader,
}

/// One seller's own mapping decision, which wins over every global edge for
/// the same term and target.
///
/// The global relation is a fact about two vocabularies and is ours; this is a
/// decision about one catalogue and is theirs. Precedence is total and in one
/// direction: where an override exists for `(org, inventory, axis, term)` the
/// relation is not consulted for that pair at all, and where none exists the
/// relation decides exactly as it did before.
///
/// An override never suppresses a loss and never converts a block into a
/// proceed on a non-delegable axis. `resolution_for` already makes
/// `Delegation::Never` win over any opt-in, and an override is an opt-in of
/// the strongest kind, so `new` refuses the licence axis and the table refuses
/// it again in a CHECK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionOverride {
    pub org: OrgId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub from: CanonicalTermId,
    pub to: VocabularyPath,
    pub kind: OverrideKind,
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}

/// The fields `ProjectionOverride::new` validates, named rather than
/// positional, in the shape `NewElectionRule` uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProjectionOverride {
    pub org: OrgId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub from: CanonicalTermId,
    pub to: VocabularyPath,
    pub kind: OverrideKind,
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionOverrideError {
    /// Issuing a rights grant on the seller's behalf is not a preference we
    /// can be given, whatever they have asked us to do elsewhere.
    LicenceNeverOverridden,
    /// A path with no segments names nothing, so the projection would resolve
    /// to a value no target can be told.
    EmptyPath,
    /// The override names an axis this inventory does not bind, so no field on
    /// that marketplace could carry the answer.
    ///
    /// The same refusal `ElectionRule::new` makes, for the same reason: an
    /// answer for an axis the target never asks about is durable, invisible
    /// and unreachable, and a seller who set it would be owed an explanation
    /// nothing could give.
    UnboundAxis,
}

impl core::fmt::Display for ProjectionOverrideError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LicenceNeverOverridden => {
                f.write_str("the licence axis is never delegated, so it is never overridden")
            }
            Self::EmptyPath => f.write_str("an override names a path with no segments"),
            Self::UnboundAxis => {
                f.write_str("the override names an axis this marketplace does not bind")
            }
        }
    }
}

impl core::error::Error for ProjectionOverrideError {}

impl ProjectionOverride {
    /// The domain half of the two-layer refusal. The database CHECK is the
    /// other half, because this one passes for anything that writes the row
    /// directly.
    pub fn new(request: NewProjectionOverride) -> Result<Self, ProjectionOverrideError> {
        if request.axis == TermKind::Licence {
            return Err(ProjectionOverrideError::LicenceNeverOverridden);
        }
        if request.to.segments.is_empty() {
            return Err(ProjectionOverrideError::EmptyPath);
        }
        if registry(request.inventory).axis(request.axis).is_none() {
            return Err(ProjectionOverrideError::UnboundAxis);
        }
        Ok(Self {
            org: request.org,
            inventory: request.inventory,
            axis: request.axis,
            from: request.from,
            to: request.to,
            kind: request.kind,
            decided_by: request.decided_by,
            decided_at: request.decided_at,
        })
    }

    /// Whether this override speaks for one term projecting into one target.
    #[must_use]
    pub fn answers(&self, inventory: InventoryId, axis: TermKind, term: CanonicalTermId) -> bool {
        self.inventory == inventory && self.axis == axis && self.from == term
    }
}

/// A standing answer the seller has given, which satisfies every future
/// election matching its trigger without asking again. The decision stays
/// theirs and becomes durable, which is what makes seller-decides compatible
/// with not re-asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElectionRule {
    pub org: OrgId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger_kind: ElectionTriggerKind,
    /// Matches `ElectionTrigger::key`. `None` is the row's `''` sentinel.
    pub trigger_key: Option<String>,
    pub answer: ElectionAnswer,
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElectionAnswer {
    /// A literal target value. "My free Tes resources are CC-BY-SA."
    Value { path: VocabularyPath },
    /// A declared preference order over the resolved set, so the seller
    /// states a policy instead of answering per product. This is the seller's
    /// own ordering, never a computed best fit.
    Ordering { prefer: Vec<VocabularyPath> },
    /// The seller delegated this trigger to best fit. Admissible only where
    /// the registry's `Delegation` permits it, which licence never does.
    Delegate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElectionRuleError {
    /// Delegating a legal axis would be us issuing the seller's rights grant.
    NotDelegable(NonDelegable),
    /// The rule names an axis this inventory does not bind.
    UnboundAxis,
}

/// What a caller hands `ElectionRule::new`. Bundled because the constructor
/// would otherwise exceed the workspace argument limit, and because a rule is
/// meaningless with any of these missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewElectionRule {
    pub org: OrgId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger_kind: ElectionTriggerKind,
    pub trigger_key: Option<String>,
    pub answer: ElectionAnswer,
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}

impl ElectionRule {
    /// The domain half of the two-layer legal backstop, mirroring the pairing
    /// of `Mapping::check` against `mapping_remote_id_marketplace`: this
    /// refuses the combination and a CHECK refuses the row, because the
    /// domain check alone passes if anything writes the row directly.
    pub fn new(request: NewElectionRule) -> Result<Self, ElectionRuleError> {
        let binding = registry(request.inventory)
            .axis(request.axis)
            .ok_or(ElectionRuleError::UnboundAxis)?;
        if let (Delegation::Never(why), ElectionAnswer::Delegate) =
            (binding.delegation, &request.answer)
        {
            return Err(ElectionRuleError::NotDelegable(why));
        }
        let NewElectionRule {
            org,
            inventory,
            axis,
            trigger_kind,
            trigger_key,
            answer,
            decided_by,
            decided_at,
        } = request;
        Ok(Self {
            org,
            inventory,
            axis,
            trigger_kind,
            trigger_key,
            answer,
            decided_by,
            decided_at,
        })
    }

    /// Whether this rule speaks to that election. Matching is on the key
    /// rather than on the payload, so a rule answers every future election
    /// with the same discriminant and never one with a different one.
    #[must_use]
    pub fn answers(&self, election: &Election) -> bool {
        self.inventory == election.inventory
            && self.axis == election.axis
            && self.trigger_kind == election.trigger.kind()
            && self.trigger_key == election.trigger.key()
    }
}

/// What a standing answer settles one election to, or `None` where it cannot
/// settle this one and the question stands.
///
/// An answer that names values the trigger never offered is not applied: a
/// rule written against one product's candidate set would otherwise inject a
/// value into a later product the relation never resolved for it. `Delegate`
/// resolves nothing here — best fit is a computation over the candidates that
/// this function has no inputs for, and a legal axis refuses delegation
/// outright — so a delegated trigger stays a question rather than becoming a
/// silent pick.
#[must_use]
pub fn resolved_by(
    trigger: &ElectionTrigger,
    answer: &ElectionAnswer,
) -> Option<Vec<VocabularyPath>> {
    let among = |offered: &[VocabularyPath], chosen: &[VocabularyPath], cap: usize| {
        let kept: Vec<VocabularyPath> = chosen
            .iter()
            .filter(|path| offered.contains(path))
            .take(cap)
            .cloned()
            .collect();
        (!kept.is_empty()).then_some(kept)
    };
    match (trigger, answer) {
        // Supply asks for a value the source never carried, so the answer is
        // the value itself and there is no candidate set to check it against.
        (ElectionTrigger::Supply { .. }, ElectionAnswer::Value { path }) => {
            Some(vec![path.clone()])
        }
        (ElectionTrigger::Supply { .. }, ElectionAnswer::Ordering { prefer }) => {
            prefer.first().map(|path| vec![path.clone()])
        }
        (ElectionTrigger::ElectOne { from }, ElectionAnswer::Value { path }) => {
            among(from, core::slice::from_ref(path), 1)
        }
        (ElectionTrigger::ElectOne { from }, ElectionAnswer::Ordering { prefer }) => {
            among(from, prefer, 1)
        }
        (ElectionTrigger::OverCap { cap, from }, ElectionAnswer::Ordering { prefer }) => {
            among(from, prefer, *cap)
        }
        (ElectionTrigger::OverCap { cap, from }, ElectionAnswer::Value { path }) => {
            among(from, core::slice::from_ref(path), *cap)
        }
        (ElectionTrigger::Narrow { candidates, .. }, ElectionAnswer::Value { path }) => {
            among(candidates, core::slice::from_ref(path), candidates.len())
        }
        (ElectionTrigger::Narrow { candidates, .. }, ElectionAnswer::Ordering { prefer }) => {
            among(candidates, prefer, candidates.len())
        }
        (
            ElectionTrigger::Supply { .. }
            | ElectionTrigger::ElectOne { .. }
            | ElectionTrigger::OverCap { .. }
            | ElectionTrigger::Narrow { .. },
            ElectionAnswer::Delegate,
        ) => None,
    }
}

/// One question this product already had answered, read back so a revived
/// item resolves instead of asking it again.
///
/// Distinct from `ElectionRule` and not derivable from one: a rule is a policy
/// the seller states over every future product, and an answer is what they
/// said about this one. The queue's own settled rows are the durable record of
/// the second, so a projection that consults only rules cannot see the answer
/// that unparked it and re-raises the identical question forever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledElection {
    pub product: ProductId,
    pub inventory: InventoryId,
    pub axis: TermKind,
    pub trigger_kind: ElectionTriggerKind,
    pub trigger_key: Option<String>,
    /// The values the seller named, in the order they named them. Read as an
    /// `Ordering`, which is what makes one stored shape answer all four
    /// triggers: a supply takes the first, an elect-one takes the first the
    /// candidates still offer, an over-cap takes as many as the cap allows,
    /// and a narrow takes every one the band still covers.
    pub chosen: Vec<VocabularyPath>,
}

impl SettledElection {
    /// Whether this settled row speaks to that election. The key is the
    /// election's own dedup key, so an answer settles the question it was
    /// asked about and never a neighbouring one -- a free-branch licence
    /// answer does not settle the paid-branch question the price flip raised.
    #[must_use]
    pub fn answers(&self, election: &Election) -> bool {
        self.product == election.product
            && self.inventory == election.inventory
            && self.axis == election.axis
            && self.trigger_kind == election.trigger.kind()
            && self.trigger_key == election.trigger.key()
    }
}

/// What the seller already said about this very product, or `None` where they
/// said nothing about this question.
///
/// Consulted before the standing rules, because an answer about this product
/// is the more specific fact. In practice the two never disagree -- a rule
/// that matches suppresses the raise, so the question is never asked and never
/// answered -- and the order is stated rather than left to iteration.
#[must_use]
pub fn settled_by<'settled>(
    settled: &'settled [SettledElection],
    election: &Election,
) -> Option<&'settled [VocabularyPath]> {
    settled
        .iter()
        .find(|answer| answer.answers(election))
        .map(|answer| answer.chosen.as_slice())
}

/// The first of the three idempotency mechanisms, and the only pure one: an
/// election a standing rule answers is never enqueued at all, so the common
/// path writes nothing.
///
/// A keyless trigger is still answerable — an `Ordering` over a subject set
/// is a policy the seller may state once — so matching is on the key's value
/// including its absence, not on the key's presence.
#[must_use]
pub fn satisfied_by<'rules>(
    rules: &'rules [ElectionRule],
    election: &Election,
) -> Option<&'rules ElectionAnswer> {
    rules
        .iter()
        .find(|rule| rule.answers(election))
        .map(|rule| &rule.answer)
}

#[cfg(test)]
mod tests {
    use super::{
        best_fit, resolution_for, satisfied_by, Candidate, Election, ElectionAnswer, ElectionRule,
        ElectionRuleError, ElectionTrigger, ElectionTriggerKind, Mode, NewElectionRule,
        NewProjectionOverride, OverrideKind, PricingBranch, ProjectionOverride,
        ProjectionOverrideError, Suggestion,
    };
    use crate::registry::{registry, AxisBinding, Cardinality, Delegation, NonDelegable};
    use crate::{Decider, EdgeKind, TermKind, VocabularyId, VocabularyPath};
    use proptest::prelude::*;
    use tam_types::{CanonicalTermId, InventoryId, OrgId, ProductId, Timestamp, Uuid};

    const ORG: OrgId = OrgId(Uuid([0x01; 16]));
    const PRODUCT: ProductId = ProductId(Uuid([0x02; 16]));

    fn decider() -> Decider {
        Decider::Imported {
            source: "test".to_owned(),
        }
    }

    fn path(segment: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
            segments: vec![segment.to_owned()],
            native_id: Some(segment.to_owned()),
        }
    }

    fn licence_election(pricing: PricingBranch) -> Election {
        Election {
            product: PRODUCT,
            inventory: InventoryId::TesGb,
            axis: TermKind::Licence,
            trigger: ElectionTrigger::Supply { pricing },
        }
    }

    fn rule(request: NewElectionRule) -> ElectionRule {
        match ElectionRule::new(request) {
            Ok(built) => built,
            Err(error) => panic!("the fixture is admissible, got {error:?}"),
        }
    }

    fn supply_rule(pricing: PricingBranch, answer: ElectionAnswer) -> ElectionRule {
        rule(NewElectionRule {
            org: ORG,
            inventory: InventoryId::TesGb,
            axis: TermKind::Licence,
            trigger_kind: ElectionTriggerKind::Supply,
            trigger_key: Some(pricing.as_str().to_owned()),
            answer,
            decided_by: decider(),
            decided_at: Timestamp(0),
        })
    }

    #[test]
    fn a_standing_answer_settles_the_branch_it_was_given_for_and_no_other() {
        let rules = [supply_rule(
            PricingBranch::Free,
            ElectionAnswer::Value {
                path: path("CC-BY-SA"),
            },
        )];
        assert_eq!(
            satisfied_by(&rules, &licence_election(PricingBranch::Free)),
            Some(&ElectionAnswer::Value {
                path: path("CC-BY-SA")
            }),
            "answered once as a policy, and never asked again"
        );
        assert_eq!(
            satisfied_by(&rules, &licence_election(PricingBranch::Paid)),
            None,
            "Tes refuses a Creative Commons value with a price, so the free answer must \
             not settle a paid listing"
        );
    }

    #[test]
    fn a_rule_for_another_axis_or_inventory_answers_nothing() {
        let rules = [supply_rule(
            PricingBranch::Free,
            ElectionAnswer::Value {
                path: path("CC-BY"),
            },
        )];
        let mut elsewhere = licence_election(PricingBranch::Free);
        elsewhere.inventory = InventoryId::TesUs;
        assert_eq!(
            satisfied_by(&rules, &elsewhere),
            None,
            "keyed per inventory"
        );
        let mut other_axis = licence_election(PricingBranch::Free);
        other_axis.axis = TermKind::Subject;
        assert_eq!(satisfied_by(&rules, &other_axis), None, "keyed per axis");
    }

    #[test]
    fn a_legal_axis_refuses_delegation_at_the_constructor() {
        assert_eq!(
            ElectionRule::new(NewElectionRule {
                org: ORG,
                inventory: InventoryId::TesGb,
                axis: TermKind::Licence,
                trigger_kind: ElectionTriggerKind::Supply,
                trigger_key: Some("free".to_owned()),
                answer: ElectionAnswer::Delegate,
                decided_by: decider(),
                decided_at: Timestamp(0),
            }),
            Err(ElectionRuleError::NotDelegable(NonDelegable::LegalContent)),
            "delegating a licence would be us issuing the seller's rights grant"
        );
    }

    #[test]
    fn a_legal_axis_still_takes_the_sellers_own_literal_answer() {
        let answered = supply_rule(
            PricingBranch::Free,
            ElectionAnswer::Value {
                path: path("CC-BY-ND"),
            },
        );
        assert_eq!(
            answered.trigger_key.as_deref(),
            Some("free"),
            "the refusal is of delegation, not of a standing answer the seller authored"
        );
    }

    #[test]
    fn a_rule_naming_an_axis_the_inventory_does_not_bind_is_refused() {
        assert_eq!(
            ElectionRule::new(NewElectionRule {
                org: ORG,
                inventory: InventoryId::Tpt,
                axis: TermKind::Licence,
                trigger_kind: ElectionTriggerKind::Supply,
                trigger_key: Some("free".to_owned()),
                answer: ElectionAnswer::Value {
                    path: path("CC-BY")
                },
                decided_by: decider(),
                decided_at: Timestamp(0),
            }),
            Err(ElectionRuleError::UnboundAxis),
            "TPT has no licence field, so there is nothing for a rule to answer"
        );
    }

    #[test]
    fn a_trigger_carries_a_key_exactly_where_a_standing_answer_generalises() {
        let keys: Vec<(ElectionTriggerKind, Option<String>)> = vec![
            ElectionTrigger::Supply {
                pricing: PricingBranch::Paid,
            },
            ElectionTrigger::ElectOne {
                from: vec![path("Maths")],
            },
            ElectionTrigger::OverCap {
                cap: 1,
                from: vec![path("Maths")],
            },
            ElectionTrigger::Narrow {
                from: path("3"),
                candidates: vec![path("Year 3"), path("Year 4")],
            },
        ]
        .into_iter()
        .map(|trigger| (trigger.kind(), trigger.key()))
        .collect();
        assert_eq!(
            keys,
            vec![
                (ElectionTriggerKind::Supply, Some("paid".to_owned())),
                (ElectionTriggerKind::ElectOne, None),
                (ElectionTriggerKind::OverCap, None),
                (ElectionTriggerKind::Narrow, Some("3".to_owned())),
            ],
            "supply keys on the pricing branch and narrow on the source value's own id, so \
             a seller migrating five hundred banded listings answers once; the other two \
             ask about one product's own set and generalise to nothing"
        );
    }

    #[test]
    fn every_trigger_kind_is_reachable_from_a_trigger() {
        for kind in ElectionTriggerKind::ALL {
            let built = match kind {
                ElectionTriggerKind::Supply => ElectionTrigger::Supply {
                    pricing: PricingBranch::Free,
                },
                ElectionTriggerKind::ElectOne => ElectionTrigger::ElectOne { from: vec![] },
                ElectionTriggerKind::OverCap => ElectionTrigger::OverCap {
                    cap: 0,
                    from: vec![],
                },
                ElectionTriggerKind::Narrow => ElectionTrigger::Narrow {
                    from: path("1"),
                    candidates: vec![],
                },
            };
            assert_eq!(built.kind(), kind, "the discriminant round-trips");
        }
    }

    fn any_delegation() -> impl Strategy<Value = Delegation> {
        prop_oneof![
            Just(Delegation::ByOptIn),
            Just(Delegation::Never(NonDelegable::LegalContent)),
        ]
    }

    proptest! {
        /// The one property the whole legal direction rests on: no opt-in,
        /// from any tenant, in any configuration, promotes a `Never` axis to
        /// best fit.
        #[test]
        fn never_wins_over_every_opt_in(delegation in any_delegation(), opted_in: bool) {
            let binding = AxisBinding {
                axis: TermKind::Licence,
                native: "licence",
                cardinality: Cardinality::One,
                delegation,
            };
            let mode = resolution_for(binding, opted_in);
            match delegation {
                Delegation::Never(_) => prop_assert_eq!(mode, Mode::SellerDecides),
                Delegation::ByOptIn => prop_assert_eq!(
                    mode,
                    if opted_in { Mode::BestFit } else { Mode::SellerDecides }
                ),
            }
        }

        /// Delegation is refused on exactly the bindings the registry marks,
        /// over every real inventory and axis rather than the one exemplar.
        #[test]
        fn the_constructor_refuses_delegation_on_exactly_the_registrys_legal_axes(
            inventory in prop::sample::select(InventoryId::ALL.as_slice()),
            axis in prop::sample::select(
                [
                    TermKind::Subject,
                    TermKind::Topic,
                    TermKind::ResourceType,
                    TermKind::Phase,
                    TermKind::Licence,
                ].as_slice()
            ),
        ) {
            let built = ElectionRule::new(NewElectionRule {
                org: ORG,
                inventory,
                axis,
                trigger_kind: ElectionTriggerKind::Supply,
                trigger_key: Some("free".to_owned()),
                answer: ElectionAnswer::Delegate,
                decided_by: decider(),
                decided_at: Timestamp(0),
            });
            match registry(inventory).axis(axis).map(|binding| binding.delegation) {
                None => prop_assert_eq!(built.err(), Some(ElectionRuleError::UnboundAxis)),
                Some(Delegation::Never(why)) => {
                    prop_assert_eq!(built.err(), Some(ElectionRuleError::NotDelegable(why)));
                }
                Some(Delegation::ByOptIn) => prop_assert!(built.is_ok()),
            }
        }
    }

    fn an_override(axis: TermKind) -> NewProjectionOverride {
        NewProjectionOverride {
            org: ORG,
            inventory: InventoryId::TesGb,
            axis,
            from: CanonicalTermId(Uuid([0x03; 16])),
            to: path("Science"),
            kind: OverrideKind::Exact,
            decided_by: decider(),
            decided_at: Timestamp(0),
        }
    }

    /// The domain half of the two-layer refusal. `Delegation::Never` wins over
    /// any opt-in, and an override is an opt-in of the strongest kind:
    /// issuing a rights grant on the seller's behalf is not a preference we
    /// can be given, however they have configured everything else.
    #[test]
    fn an_override_may_never_settle_the_licence_axis() {
        assert_eq!(
            ProjectionOverride::new(an_override(TermKind::Licence)),
            Err(ProjectionOverrideError::LicenceNeverOverridden)
        );
        for axis in [
            TermKind::Subject,
            TermKind::Topic,
            TermKind::ResourceType,
            TermKind::Phase,
        ] {
            assert!(
                ProjectionOverride::new(an_override(axis)).is_ok(),
                "{axis:?} is delegable and must be overridable"
            );
        }
    }

    /// A path with no segments names nothing, so the projection would resolve
    /// to a value no target could be told.
    #[test]
    fn an_override_may_not_name_an_empty_path() {
        let mut request = an_override(TermKind::Subject);
        request.to.segments.clear();
        assert_eq!(
            ProjectionOverride::new(request),
            Err(ProjectionOverrideError::EmptyPath)
        );
    }

    /// An answer for an axis the target never asks about is durable, invisible
    /// and unreachable, which is why `ElectionRule::new` refuses one; an
    /// override is the same kind of answer and earns the same refusal. Etsy
    /// binds no equivalence axis at all, so every axis is unbound there.
    #[test]
    fn an_override_may_not_name_an_axis_the_marketplace_does_not_bind() {
        for axis in [
            TermKind::Subject,
            TermKind::Topic,
            TermKind::ResourceType,
            TermKind::Phase,
        ] {
            let mut request = an_override(axis);
            request.inventory = InventoryId::Etsy;
            assert_eq!(
                ProjectionOverride::new(request),
                Err(ProjectionOverrideError::UnboundAxis),
                "Etsy binds no {axis:?} field, so there is nothing for an override to answer"
            );
        }
    }

    /// The refusal is a property of the axis rather than of the delegation
    /// mode the caller happens to hold, so it cannot be configured away.
    #[test]
    fn the_licence_refusal_agrees_with_the_delegation_the_registry_declares() {
        let binding = registry(InventoryId::TesGb)
            .equivalence_axes
            .iter()
            .find(|binding| binding.axis == TermKind::Licence)
            .copied()
            .expect("Tes binds the licence axis");
        assert!(matches!(binding.delegation, Delegation::Never(_)));
        assert_eq!(resolution_for(binding, true), Mode::SellerDecides);
    }

    // ------------------------------------------------------------ best fit

    /// A delegable axis with room for exactly one value: Tes takes one
    /// `mainType`, which is the `ElectOne` shape the ranking exists for.
    fn resource_type() -> AxisBinding {
        match registry(InventoryId::TesGb).axis(TermKind::ResourceType) {
            Some(binding) => binding,
            None => panic!("Tes binds a resource type"),
        }
    }

    fn licence_axis() -> AxisBinding {
        match registry(InventoryId::TesGb).axis(TermKind::Licence) {
            Some(binding) => binding,
            None => panic!("Tes binds the licence axis"),
        }
    }

    fn typed(segment: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesGb, TermKind::ResourceType),
            segments: vec![segment.to_owned()],
            native_id: Some(segment.to_owned()),
        }
    }

    fn candidate(segment: &str, edge: EdgeKind) -> Candidate {
        Candidate {
            path: typed(segment),
            edge,
        }
    }

    fn kept(suggestion: &Suggestion) -> Vec<String> {
        suggestion
            .keep
            .iter()
            .filter_map(|ranked| ranked.path.native_id.clone())
            .collect()
    }

    fn dropped(suggestion: &Suggestion) -> Vec<String> {
        suggestion
            .dropped
            .iter()
            .filter_map(|ranked| ranked.path.native_id.clone())
            .collect()
    }

    /// The constraint the whole design rests on: a suggestion is a ranking of
    /// what the projection already resolved, so it can never name a value the
    /// resolved set did not hold. A candidate list carrying a value the
    /// trigger never offered is the invention this guards against.
    #[test]
    fn a_suggestion_never_names_a_value_the_resolved_set_did_not_hold() {
        let trigger = ElectionTrigger::ElectOne {
            from: vec![typed("99009")],
        };
        let Some(suggestion) = best_fit(
            resource_type(),
            true,
            &trigger,
            &[
                candidate("99001", EdgeKind::Exact),
                candidate("99009", EdgeKind::Broader),
            ],
        ) else {
            panic!("one resolved value is still a set of one");
        };
        assert_eq!(
            [kept(&suggestion), dropped(&suggestion)].concat(),
            ["99009".to_owned()],
            "99001 has the better edge and is not in the resolved set, so it is not offered"
        );
    }

    /// A supply asks for a value the source never carried, so there is no
    /// resolved set and nothing to rank. Best fit declines rather than
    /// reaching into the target's vocabulary for something plausible.
    #[test]
    fn a_supply_trigger_gets_no_suggestion_because_nothing_resolved() {
        assert_eq!(
            best_fit(
                resource_type(),
                true,
                &ElectionTrigger::Supply {
                    pricing: PricingBranch::Free
                },
                &[candidate("99001", EdgeKind::Exact)],
            ),
            None,
            "a candidate list handed in from outside is not a resolved set"
        );
    }

    /// The legal backstop, at the one place a computed answer could appear.
    /// `Never` wins over the opt-in exactly as `resolution_for` says, so this
    /// holds whatever the seller has asked us to do elsewhere.
    #[test]
    fn a_legal_axis_gets_no_suggestion_whatever_the_opt_in_says() {
        let licence = |segment: &str| VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
            segments: vec![segment.to_owned()],
            native_id: Some(segment.to_owned()),
        };
        let trigger = ElectionTrigger::ElectOne {
            from: vec![licence("CC-BY"), licence("CC-BY-SA")],
        };
        for opted_in in [false, true] {
            assert_eq!(
                best_fit(
                    licence_axis(),
                    opted_in,
                    &trigger,
                    &[Candidate {
                        path: licence("CC-BY"),
                        edge: EdgeKind::Exact,
                    }],
                ),
                None,
                "choosing a licence is issuing a rights grant, so no opt-in reaches it"
            );
        }
    }

    /// Best fit exists only where the seller asked for it. Without the opt-in
    /// the same delegable axis yields nothing, so a suggestion on screen is
    /// always one the seller invited.
    #[test]
    fn no_opt_in_means_no_suggestion_on_an_otherwise_delegable_axis() {
        let trigger = ElectionTrigger::ElectOne {
            from: vec![typed("99001"), typed("99009")],
        };
        assert_eq!(
            best_fit(
                resource_type(),
                false,
                &trigger,
                &[candidate("99001", EdgeKind::Exact)]
            ),
            None,
            "the default is that the seller decides"
        );
    }

    /// The cap case, and the one place the seller most needs to see what a
    /// suggestion leaves behind: kept and dropped together are the whole
    /// resolved set, so nothing goes missing without being named.
    #[test]
    fn an_over_cap_suggestion_keeps_exactly_the_cap_and_names_the_rest() {
        let trigger = ElectionTrigger::OverCap {
            cap: 2,
            from: vec![
                typed("99001"),
                typed("99005"),
                typed("99007"),
                typed("99009"),
            ],
        };
        let Some(suggestion) = best_fit(
            resource_type(),
            true,
            &trigger,
            &[
                candidate("99001", EdgeKind::Broader),
                candidate("99005", EdgeKind::Exact),
                candidate("99007", EdgeKind::Broader),
                candidate("99009", EdgeKind::Exact),
            ],
        ) else {
            panic!("four resolved values against a cap of two is the over-cap question");
        };
        assert_eq!(
            (kept(&suggestion), dropped(&suggestion)),
            (
                vec!["99005".to_owned(), "99009".to_owned()],
                vec!["99001".to_owned(), "99007".to_owned()]
            ),
            "the two exact edges are kept in the seller's own order and the two broader \
             ones are named as dropped rather than vanishing"
        );
    }

    /// A band that covers eight year groups means eight, so the narrow
    /// suggestion is every candidate under the band and nothing is dropped.
    #[test]
    fn a_narrow_suggestion_keeps_every_candidate_under_the_band() {
        let trigger = ElectionTrigger::Narrow {
            from: typed("3"),
            candidates: vec![typed("99001"), typed("99005"), typed("99007")],
        };
        let Some(suggestion) = best_fit(resource_type(), true, &trigger, &[]) else {
            panic!("a band with candidates under it is rankable");
        };
        assert_eq!(
            (kept(&suggestion).len(), dropped(&suggestion)),
            (3, Vec::new()),
            "narrowing keeps the whole set the band covers"
        );
    }

    /// Edge kind first, and the seller's own order second. The input is given
    /// in an order the ranking must reverse on the first key and preserve on
    /// the second, so a sort on either key alone fails it.
    #[test]
    fn an_exact_edge_outranks_a_broader_one_and_ties_keep_the_sellers_order() {
        let trigger = ElectionTrigger::OverCap {
            cap: 4,
            from: vec![
                typed("99009"),
                typed("99007"),
                typed("99005"),
                typed("99001"),
            ],
        };
        let Some(suggestion) = best_fit(
            resource_type(),
            true,
            &trigger,
            &[
                candidate("99009", EdgeKind::Broader),
                candidate("99007", EdgeKind::Exact),
                candidate("99005", EdgeKind::Broader),
                candidate("99001", EdgeKind::Exact),
            ],
        ) else {
            panic!("four resolved values under a cap of four still rank");
        };
        assert_eq!(
            kept(&suggestion),
            vec![
                "99007".to_owned(),
                "99001".to_owned(),
                "99009".to_owned(),
                "99005".to_owned()
            ],
            "the two exact edges come first, and within each kind the seller's own order \
             stands"
        );
    }

    /// A value in the resolved set that the candidate list names no edge for
    /// is still offered — being resolved is what admits it — but it ranks
    /// below every value whose edge is known, and it claims no edge of its
    /// own rather than borrowing the weakest one.
    #[test]
    fn a_value_with_no_named_edge_ranks_last_and_claims_none() {
        let trigger = ElectionTrigger::OverCap {
            cap: 2,
            from: vec![typed("99009"), typed("99001")],
        };
        let Some(suggestion) = best_fit(
            resource_type(),
            true,
            &trigger,
            &[candidate("99001", EdgeKind::Narrower)],
        ) else {
            panic!("both values are in the resolved set");
        };
        assert_eq!(
            suggestion
                .keep
                .iter()
                .map(|ranked| (ranked.path.native_id.clone(), ranked.edge))
                .collect::<Vec<_>>(),
            vec![
                (Some("99001".to_owned()), Some(EdgeKind::Narrower)),
                (Some("99009".to_owned()), None)
            ],
            "the known edge ranks first even at its weakest, and the unknown one says so"
        );
    }

    /// Where the projection resolved nothing there is nothing to rank and the
    /// question stands, which is the same answer a supply gets by a different
    /// route.
    #[test]
    fn an_empty_resolved_set_yields_no_suggestion() {
        for trigger in [
            ElectionTrigger::ElectOne { from: Vec::new() },
            ElectionTrigger::OverCap {
                cap: 3,
                from: Vec::new(),
            },
            ElectionTrigger::Narrow {
                from: typed("3"),
                candidates: Vec::new(),
            },
        ] {
            assert_eq!(
                best_fit(resource_type(), true, &trigger, &[]),
                None,
                "{trigger:?} resolved nothing, so there is nothing to suggest"
            );
        }
    }

    /// A cap of zero takes nothing, so there is no suggestion to make rather
    /// than an empty one to render. The registry declares no such cap today
    /// and the arithmetic is what would go wrong if one arrived.
    #[test]
    fn a_cap_of_nothing_yields_no_suggestion_rather_than_an_empty_one() {
        assert_eq!(
            best_fit(
                resource_type(),
                true,
                &ElectionTrigger::OverCap {
                    cap: 0,
                    from: vec![typed("99001")],
                },
                &[candidate("99001", EdgeKind::Exact)],
            ),
            None,
            "a target that takes none is not answered by suggesting none"
        );
    }
}
