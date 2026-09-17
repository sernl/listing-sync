//! Editable seller rules, durable previews, append-only approvals and frozen
//! queued outputs.
//!
//! Snapshots copy rule definitions rather than following mutable references.
//! Approvals store the complete target choice, but assign authorship only to
//! fields in the preview's explicit patch.
//!
//! Approval rechecks the source fingerprint, captured rule revisions and
//! previous target choice under policy and product locks. The tenant policy
//! lock also excludes newly created or enabled rules from slipping into an
//! implicit rule set between validation and commit.
//!
//! Frozen outputs are immutable and written in the transaction that queues
//! their item.

use std::fmt::Write as _;

use chrono::NaiveDate;
use sqlx::PgPool;
use tam_domain::seller_rules::{
    FrozenRuleOutput, RuleKind, RuleMatch, RuleUse, SellerRuleDefinition, SellerRuleRecord,
    SellerTermChoice, TargetFields,
};
use tam_domain::{CanonicalProduct, DeclarationSource};
use tam_types::{Currency, InventoryId, OrgId, PriceIntent, ProductId, Timestamp, UserId};

use crate::codec::{
    copy_format_to_db, currency_from_db, currency_to_db, inventory_from_db, inventory_to_db,
    price_from_db, term_kind_to_db, timestamp_from_db, timestamp_to_db, uuid_from_db, uuid_to_db,
    PriceColumns,
};
use crate::product::{get_product_in_tx, lock_product, RightsColumns};
use crate::{pin_org, StorageError};

/// The namespace the source fingerprint is drawn in. A constant of this
/// module, so two deployments of the same build agree and no other v5 identity
/// in this tree can collide with one.
const FINGERPRINT_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x9e, 0x0b, 0x4a, 0x21, 0x7c, 0x3d, 0x4f, 0x88, 0xb1, 0x52, 0x6d, 0xc4, 0x0a, 0x91, 0x3e, 0x77,
]);

/// A digest of the resource as the evaluator read it.
///
/// Not the product's `updated_at`: a caller can pass a stale timestamp, and
/// the column moves for edits the evaluation never looked at — a cover
/// redrawn, a file renamed. What a proposal depends on is the title, the body
/// and its format, the price, the rights, the subjects, the grades and the
/// native residue, and this covers exactly those. A resource whose price
/// changed after the seller was shown a conversion of it produces a different
/// fingerprint and the confirmation is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFingerprint(pub uuid::Uuid);

/// The digest of what an evaluation reads off one resource.
///
/// The encoding is structural rather than a concatenation of renderings.
/// Every scalar is length-prefixed, every field is introduced by a tag, every
/// option states whether it is there at all, and every collection states how
/// many members it has before it writes them. All four are load-bearing: one
/// grade path of five segments and two paths that spell the same five strings
/// match different conditions, and a digest that flattened both to the same
/// byte stream would let a confirmation pass against facts the seller never
/// saw. The renderings inside are the column renderings this crate already
/// owns rather than `Debug`, which is a formatting convenience and not a
/// contract.
#[must_use]
pub fn fingerprint(product: &CanonicalProduct) -> SourceFingerprint {
    let out = &mut String::with_capacity(512);

    digest_tag(out, "title");
    digest_text(out, &product.title.0);

    digest_tag(out, "body");
    digest_text(out, &product.body.body);
    digest_text(out, copy_format_to_db(product.body.format));

    digest_tag(out, "price");
    let price = PriceColumns::from_intent(product.price);
    digest_text(out, price.kind);
    digest_optional(out, price.minor_units.map(|n| n.to_string()).as_deref());
    digest_optional(out, price.currency);

    digest_tag(out, "rights");
    let rights = RightsColumns::encode(&product.rights);
    digest_text(out, rights.state);
    digest_optional(out, rights.inventory.as_deref());
    digest_tag(out, "rights_path");
    match &rights.segments {
        None => digest_absent(out),
        Some(segments) => {
            digest_present(out);
            digest_count(out, segments.len());
            for segment in segments {
                digest_text(out, segment);
            }
        }
    }
    digest_optional(out, rights.native_id.as_deref());

    digest_tag(out, "subjects");
    digest_count(out, product.subjects.len());
    for subject in &product.subjects {
        digest_text(out, &uuid_to_db(subject.0).to_string());
    }

    digest_tag(out, "grades");
    digest_declaration_source(out, product.grades.source);
    digest_count(out, product.grades.raw.len());
    for path in &product.grades.raw {
        digest_tag(out, "path");
        digest_text(out, inventory_to_db(path.vocabulary.0));
        digest_text(out, term_kind_to_db(path.vocabulary.1));
        digest_count(out, path.segments.len());
        for segment in &path.segments {
            digest_text(out, segment);
        }
        digest_optional(out, path.native_id.as_deref());
    }

    digest_tag(out, "derived");
    match product.grades.derived {
        None => digest_absent(out),
        Some(derived) => {
            digest_present(out);
            digest_text(out, &derived.low_years().to_string());
            digest_text(out, &derived.high_years().to_string());
        }
    }

    digest_tag(out, "residue");
    digest_count(out, product.native_residue.len());
    for term in &product.native_residue {
        digest_tag(out, "term");
        digest_text(out, inventory_to_db(term.inventory));
        digest_optional(out, term.kind.map(term_kind_to_db));
        digest_count(out, term.segments.len());
        for segment in &term.segments {
            digest_text(out, segment);
        }
        digest_optional(out, term.native_id.as_deref());
    }

    SourceFingerprint(uuid::Uuid::new_v5(&FINGERPRINT_NAMESPACE, out.as_bytes()))
}

/// One scalar, length-prefixed so no value can run into the next.
fn digest_text(out: &mut String, value: &str) {
    let _unused: core::fmt::Result = write!(out, "{}:{value}\u{1f}", value.len());
}

/// Which field comes next. Two resources differing only in *which* axis
/// holds a value encode differently because of this.
fn digest_tag(out: &mut String, tag: &str) {
    let _unused: core::fmt::Result = write!(out, "@{tag}\u{1e}");
}

/// How many members the collection that follows has, so a boundary between
/// two members cannot be spelled the same way as one longer member.
fn digest_count(out: &mut String, count: usize) {
    let _unused: core::fmt::Result = write!(out, "*{count}\u{1e}");
}

fn digest_absent(out: &mut String) {
    out.push('-');
}

fn digest_present(out: &mut String) {
    out.push('+');
}

/// An option's discriminant before its value, so absence is a state of its
/// own rather than the empty string a present value could also hold.
fn digest_optional(out: &mut String, value: Option<&str>) {
    match value {
        None => digest_absent(out),
        Some(value) => {
            digest_present(out);
            digest_text(out, value);
        }
    }
}

/// Where the grade declaration came from: part of what the evaluator reads,
/// and a discriminant rather than a string the vocabulary could also spell.
fn digest_declaration_source(out: &mut String, source: DeclarationSource) {
    match source {
        DeclarationSource::Seller => digest_text(out, "seller"),
        DeclarationSource::Imported { vocabulary } => {
            digest_text(out, "imported");
            digest_text(out, inventory_to_db(vocabulary.0));
            digest_text(out, term_kind_to_db(vocabulary.1));
        }
    }
}

/// Which rules a listing asks for, beyond the direction and search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleState {
    #[default]
    All,
    Enabled,
    Disabled,
}

/// One page of the rule list, as the console asks for it. Every filter is
/// applied by the statement: a console that paginated in memory would report
/// counts for a page rather than for the tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleQuery {
    pub kind: Option<RuleKind>,
    pub state: RuleState,
    pub source: Option<InventoryId>,
    pub target: Option<InventoryId>,
    pub search: Option<String>,
    /// One-based, as the wire states it.
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleCounts {
    pub all: i64,
    pub enabled: i64,
    pub disabled: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePage {
    pub rows: Vec<SellerRuleRecord>,
    pub page: u32,
    pub has_next: bool,
    /// The tenant's counts under every filter except the state one, so a
    /// console showing "enabled 3 / disabled 1" does not have to ask three
    /// times and does not report the page it is looking at.
    pub counts: RuleCounts,
}

/// One edit to one rule. A value rather than five parameters, because the
/// revision and the definition are one statement — "replace what stands at
/// revision N with this" — and a caller able to pass them separately is a
/// caller able to pass them in the wrong order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleEdit<'a> {
    pub id: uuid::Uuid,
    pub revision: i64,
    pub definition: &'a SellerRuleDefinition,
    pub author: UserId,
    pub at: Timestamp,
}

/// A rule edit refused because its identity or revision no longer matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleWriteRefusal {
    Stale { current: i64 },
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDelete {
    Deleted,
    Stale { current: i64 },
    Missing,
}

/// An official rate as this deployment fetched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReferenceQuote {
    pub source: Currency,
    pub target: Currency,
    pub rate_micros: i64,
    pub as_of: NaiveDate,
    pub provider: String,
    pub source_url: String,
    pub fetched_at: Timestamp,
}

/// The same quote, stored, with the identity a rule refers to it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceQuote {
    pub id: uuid::Uuid,
    pub source: Currency,
    pub target: Currency,
    pub rate_micros: i64,
    pub as_of: NaiveDate,
    pub provider: String,
    pub source_url: String,
    pub fetched_at: Timestamp,
}

/// Where the preview's rule set came from.
///
/// It matters at confirmation and nowhere else. An `Explicit` set is exactly
/// what the client named — saved rules by identifier, or a draft alone — and
/// nothing can join it, so the recheck is "do these still stand as
/// captured?". An `Implicit` set was derived from "every enabled rule in this
/// direction", and a rule created or enabled since then *would* be part of
/// the answer now: the seller approved a proposal computed without it, so the
/// set having grown is as stale as one of its members having changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    Implicit,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStatus {
    Proposed,
    Unchanged,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewDecision {
    Pending,
    Accepted,
    Rejected,
}

/// One resource inside one proposal, as it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPreviewRow {
    pub product: ProductId,
    /// What the resource was when the proposal was computed.
    pub fingerprint: SourceFingerprint,
    /// The approval that was the resource's target choice then, and `None`
    /// for the positive statement that there was none.
    pub previous_choice: Option<uuid::Uuid>,
    pub title: String,
    pub source_price: PriceIntent,
    pub before: TargetFields,
    /// The merged target fields: what the row shows the seller the resource
    /// would carry. Kept for display and never for authorship, because a
    /// field it merely inherited is not something this approver decided.
    pub proposed: TargetFields,
    /// Exactly what this evaluation changed, as the evaluator returned it.
    /// This is what an approval's field authorship is drawn from: a field
    /// absent here keeps the value *and the author* it already had, and a
    /// field present here is this approver's decision even where it restates
    /// the value that was already stored.
    pub patch: TargetFields,
    pub matches: Vec<RuleMatch>,
    pub blockers: Vec<String>,
    pub status: PreviewStatus,
}

/// One proposal as it is written. The rules travel whole because the
/// confirmation is checked against them and a rule edited in between has to
/// be a refusal rather than a different proposal.
#[derive(Debug, Clone)]
pub struct NewPreview<'a> {
    pub id: uuid::Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub request: &'a serde_json::Value,
    pub captured_rules: &'a [SellerRuleRecord],
    pub scope: RuleScope,
    pub actor: UserId,
    pub at: Timestamp,
    pub rows: &'a [NewPreviewRow],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPreviewRow {
    pub product: ProductId,
    pub fingerprint: SourceFingerprint,
    pub previous_choice: Option<uuid::Uuid>,
    pub title: String,
    pub source_price: PriceIntent,
    pub before: TargetFields,
    pub proposed: TargetFields,
    pub patch: TargetFields,
    pub matches: Vec<RuleMatch>,
    pub blockers: Vec<String>,
    pub status: PreviewStatus,
    pub decision: PreviewDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPreview {
    pub id: uuid::Uuid,
    pub source: InventoryId,
    pub target: InventoryId,
    pub request: serde_json::Value,
    pub rules: Vec<SellerRuleRecord>,
    pub scope: RuleScope,
    pub actor: UserId,
    pub created_at: Timestamp,
    pub rows: Vec<StoredPreviewRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionKind {
    Accept,
    Reject,
}

/// Which rows a decision addresses.
///
/// `All` is every *pending* row of the preview: a row the seller already
/// decided is not part of "the rest of them", and reading it as one would
/// make "reject these two, then accept the rest" a conflict instead of the
/// ordinary workflow it is. `Products` is exactly what the client named,
/// already-decided rows included, so naming one carrying the opposite
/// decision is still a conflict rather than a silent reversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    All,
    Products(Vec<ProductId>),
}

/// Why one confirmation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    /// The resource is not what the proposal was computed from.
    SourceChanged,
    /// A rule the preview captured has been edited or deleted.
    RuleRevised { rule: uuid::Uuid },
    /// Another approval for this resource and direction landed in between.
    ChoiceChanged,
    /// Named explicitly by a client that was told the row could not be
    /// accepted.
    Blocked,
    /// Named explicitly, and the preview holds no such row — or the resource
    /// has since been deleted.
    Missing,
}

/// One refusal. `product` is absent where the fact is about the whole
/// proposal rather than one of its rows, which is what a revised rule is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleRow {
    pub product: Option<ProductId>,
    pub reason: StaleReason,
}

/// What a confirmation did.
///
/// The counts are of rows this call decided, plus — for an explicit
/// selection only — rows that already carried the same decision, which is
/// what makes a retried request idempotent rather than a second approval. An
/// `All` decision reports no replays, because it never reaches an
/// already-decided row in the first place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionReport {
    pub accepted: usize,
    pub rejected: usize,
    /// Rows of this preview still pending after the write.
    pub remaining: usize,
    /// Pending rows an `All` accept passed over because they are blocked,
    /// which is why `remaining` is not zero.
    pub blocked: usize,
}

/// One decision on one proposal: which preview, who, accept or reject, and
/// over which rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDecision<'a> {
    pub preview: uuid::Uuid,
    pub actor: UserId,
    pub kind: DecisionKind,
    pub selection: &'a Selection,
    pub at: Timestamp,
}

/// What a confirmation did, or why it did nothing.
///
/// `Stale` and `Conflict` write nothing at all: a partial application of a
/// proposal is a state no seller asked for and no preview describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionOutcome {
    Applied(DecisionReport),
    Stale(Vec<StaleRow>),
    /// Rows already carrying the opposite decision. A replay of the same
    /// decision is `Applied`; reversing one is not something this call does.
    Conflict(Vec<ProductId>),
    UnknownPreview,
}

/// The current target choice for one resource and direction: the whole
/// choice, with the author of each field.
///
/// The authors are separate columns rather than one row-level author because
/// they genuinely differ: approving a converted price for a resource whose
/// licence was approved last week must not restate the licence as this
/// approver's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedChoice {
    pub id: uuid::Uuid,
    pub price: Option<PriceIntent>,
    pub price_author: Option<UserId>,
    pub licence: Option<SellerTermChoice>,
    pub resource_type: Option<SellerTermChoice>,
    pub matches: Vec<RuleMatch>,
    pub approved_at: Timestamp,
}

impl AcceptedChoice {
    /// The same choice in the domain's partial-field shape, for an evaluator
    /// that does not care who approved what.
    #[must_use]
    pub fn fields(&self) -> TargetFields {
        TargetFields {
            price: self.price,
            licence: self.licence.as_ref().map(|c| c.native_id.clone()),
            resource_type: self.resource_type.as_ref().map(|c| c.native_id.clone()),
        }
    }
}

/// One resource's confirmed policy answer, as a request freezes it.
///
/// Two shapes because confirmation knows two different amounts of the
/// answer. Where the resource is already in this catalogue, the request
/// evaluates it under the rules being frozen and keeps the *output*: the
/// exact price and native values the drain will publish, so a source edit
/// between confirmation and drain cannot change what the seller confirmed.
/// Where the resource is only named by a native read that has not happened
/// yet, there is nothing to evaluate against, so the request keeps the
/// *accepted choice* and the drain evaluates against that.
///
/// An `Output`'s price is the automatic answer, not a manual grant, which is
/// why it carries no price author. Its native values keep the authors they
/// were approved by, because those are approvals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestChoice {
    Accepted(AcceptedChoice),
    Output(FrozenRuleOutput),
}

/// The definitions and choices one request was confirmed against.
#[derive(Debug, Clone)]
pub struct NewRequestSnapshot<'a> {
    pub source: InventoryId,
    pub target: InventoryId,
    pub rules: &'a [SellerRuleRecord],
    pub choices: &'a [(ProductId, RequestChoice)],
    pub at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRuleSnapshot {
    pub source: InventoryId,
    pub target: InventoryId,
    pub rules: Vec<SellerRuleRecord>,
    pub choices: Vec<(ProductId, RequestChoice)>,
    pub captured_at: Timestamp,
}

const STATUS_PROPOSED: &str = "proposed";
const STATUS_UNCHANGED: &str = "unchanged";
const STATUS_BLOCKED: &str = "blocked";
const DECISION_PENDING: &str = "pending";
const DECISION_ACCEPTED: &str = "accepted";
const DECISION_REJECTED: &str = "rejected";

const fn status_to_db(status: PreviewStatus) -> &'static str {
    match status {
        PreviewStatus::Proposed => STATUS_PROPOSED,
        PreviewStatus::Unchanged => STATUS_UNCHANGED,
        PreviewStatus::Blocked => STATUS_BLOCKED,
    }
}

fn status_from_db(raw: &str) -> Result<PreviewStatus, StorageError> {
    match raw {
        STATUS_PROPOSED => Ok(PreviewStatus::Proposed),
        STATUS_UNCHANGED => Ok(PreviewStatus::Unchanged),
        STATUS_BLOCKED => Ok(PreviewStatus::Blocked),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown preview status {other:?}"),
        }),
    }
}

const fn decision_to_db(decision: PreviewDecision) -> &'static str {
    match decision {
        PreviewDecision::Pending => DECISION_PENDING,
        PreviewDecision::Accepted => DECISION_ACCEPTED,
        PreviewDecision::Rejected => DECISION_REJECTED,
    }
}

fn decision_from_db(raw: &str) -> Result<PreviewDecision, StorageError> {
    match raw {
        DECISION_PENDING => Ok(PreviewDecision::Pending),
        DECISION_ACCEPTED => Ok(PreviewDecision::Accepted),
        DECISION_REJECTED => Ok(PreviewDecision::Rejected),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown preview decision {other:?}"),
        }),
    }
}

const fn scope_to_db(scope: RuleScope) -> &'static str {
    match scope {
        RuleScope::Implicit => "implicit",
        RuleScope::Explicit => "explicit",
    }
}

fn scope_from_db(raw: &str) -> Result<RuleScope, StorageError> {
    match raw {
        "implicit" => Ok(RuleScope::Implicit),
        "explicit" => Ok(RuleScope::Explicit),
        other => Err(StorageError::CorruptRow {
            reason: format!("unknown preview rule scope {other:?}"),
        }),
    }
}

const fn kind_to_db(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Pricing => "pricing",
        RuleKind::Mapping => "mapping",
    }
}

const fn use_to_db(use_: RuleUse) -> &'static str {
    match use_ {
        RuleUse::Copy => "copy",
        RuleUse::Move => "move",
        RuleUse::CrossList => "cross_list",
    }
}

/// A rendering that failed is a bug in this crate, not a corrupt row: the
/// value being rendered came from memory.
///
/// Both helpers take the `serde_json` result rather than the value, so
/// neither names a `serde` trait in a bound — this crate links `serde_json`
/// and not `serde`, and the domain's derives are what make the call legal at
/// each site.
fn to_json(
    rendered: serde_json::Result<serde_json::Value>,
    what: &str,
) -> Result<serde_json::Value, StorageError> {
    rendered.map_err(|why| StorageError::Inconsistent {
        reason: format!("{what} does not render as json: {why}"),
    })
}

/// A stored value that will not parse is a corrupt row, which is a different
/// fault from a rendering that failed and is reported as one.
fn from_json<T>(parsed: serde_json::Result<T>, what: &str) -> Result<T, StorageError> {
    parsed.map_err(|why| StorageError::CorruptRow {
        reason: format!("stored {what} does not parse: {why}"),
    })
}

/// A literal needle for `LIKE`, with the pattern's own metacharacters
/// neutralised. Without this a seller searching for `50%` matches every rule
/// they have.
fn like_needle(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    for ch in raw.to_lowercase().chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    format!("%{out}%")
}

/// The four columns one field's approval occupies: the native value and the
/// author of that value, present or absent together.
struct TermColumns {
    native_id: Option<String>,
    author: Option<uuid::Uuid>,
}

impl TermColumns {
    fn encode(choice: Option<&SellerTermChoice>) -> Self {
        Self {
            native_id: choice.map(|c| c.native_id.clone()),
            author: choice.map(|c| uuid_to_db(c.author.0)),
        }
    }

    fn decode(
        native_id: Option<String>,
        author: Option<uuid::Uuid>,
        what: &str,
    ) -> Result<Option<SellerTermChoice>, StorageError> {
        match (native_id, author) {
            (None, None) => Ok(None),
            (Some(native_id), Some(author)) => Ok(Some(SellerTermChoice {
                native_id,
                author: UserId(uuid_from_db(author)),
            })),
            _ => Err(StorageError::CorruptRow {
                reason: format!("{what} is present without its author, or the reverse"),
            }),
        }
    }
}

/// The money triple as an *optional* amount: an approval may state a licence
/// and no price, which is not the same as stating a free one.
struct OptionalPriceColumns {
    kind: Option<&'static str>,
    minor_units: Option<i64>,
    currency: Option<&'static str>,
}

impl OptionalPriceColumns {
    fn encode(price: Option<&PriceIntent>) -> Self {
        match price {
            None => Self {
                kind: None,
                minor_units: None,
                currency: None,
            },
            Some(price) => {
                let columns = PriceColumns::from_intent(*price);
                Self {
                    kind: Some(columns.kind),
                    minor_units: columns.minor_units,
                    currency: columns.currency,
                }
            }
        }
    }
}

fn optional_price_from_db(
    kind: Option<String>,
    minor_units: Option<i64>,
    currency: Option<String>,
) -> Result<Option<PriceIntent>, StorageError> {
    match kind {
        None => Ok(None),
        Some(kind) => price_from_db(&kind, minor_units, currency).map(Some),
    }
}

pub struct SellerRuleRepo {
    pool: PgPool,
}

impl SellerRuleRepo {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Writes a new rule at revision 1.
    pub async fn create(
        &self,
        org: OrgId,
        author: UserId,
        definition: &SellerRuleDefinition,
        at: Timestamp,
    ) -> Result<SellerRuleRecord, StorageError> {
        let id = uuid::Uuid::new_v4();
        let at_db = timestamp_to_db(at)?;
        let json = to_json(serde_json::to_value(definition), "a rule definition")?;
        let auto_apply: Vec<String> = definition
            .auto_apply
            .iter()
            .map(|use_| use_to_db(*use_).to_owned())
            .collect();

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        // Every write to the policy takes this, and so does every
        // confirmation: a rule inserted or enabled between a confirmation's
        // rule-set check and its write would be a rule the seller's proposal
        // never saw, and no row lock excludes a row that does not exist yet.
        lock_policy(&mut tx, org).await?;
        sqlx::query!(
            "INSERT INTO seller_rule (org_id, id, revision, kind, title, description, enabled, \
             source, target, auto_apply, definition, author, created_at, updated_at) \
             VALUES ($1, $2, 1, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)",
            uuid_to_db(org.0),
            id,
            kind_to_db(definition.kind()),
            definition.title,
            definition.description,
            definition.enabled,
            inventory_to_db(definition.source),
            inventory_to_db(definition.target),
            &auto_apply,
            json,
            uuid_to_db(author.0),
            at_db,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(SellerRuleRecord {
            id: uuid_from_db(id),
            revision: 1,
            definition: definition.clone(),
            author,
            created_at: at,
            updated_at: at,
        })
    }

    pub async fn get(
        &self,
        org: OrgId,
        id: uuid::Uuid,
    ) -> Result<Option<SellerRuleRecord>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query_as!(
            RuleRow,
            "SELECT id, revision, definition, author, created_at, updated_at \
             FROM seller_rule WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(decode_rule).transpose()
    }

    /// One page of the tenant's rules, filtered, searched and counted by the
    /// statement.
    ///
    /// `has_next` comes from asking for one row more than the page holds,
    /// which is one statement rather than a second count that could disagree
    /// with the page beside it.
    pub async fn list(&self, org: OrgId, query: &RuleQuery) -> Result<RulePage, StorageError> {
        let org_db = uuid_to_db(org.0);
        let kind = query.kind.map(kind_to_db);
        let enabled = match query.state {
            RuleState::All => None,
            RuleState::Enabled => Some(true),
            RuleState::Disabled => Some(false),
        };
        let source = query.source.map(inventory_to_db);
        let target = query.target.map(inventory_to_db);
        let needle = query.search.as_deref().map(like_needle);
        let page = query.page.max(1);
        let size = i64::from(query.page_size.clamp(1, 200));
        let offset = i64::from(page - 1) * size;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let rows = sqlx::query_as!(
            RuleRow,
            "SELECT id, revision, definition, author, created_at, updated_at \
             FROM seller_rule \
             WHERE org_id = $1 \
               AND ($2::text IS NULL OR kind = $2) \
               AND ($3::bool IS NULL OR enabled = $3) \
               AND ($4::text IS NULL OR source = $4) \
               AND ($5::text IS NULL OR target = $5) \
               AND ($6::text IS NULL OR search_text LIKE $6 ESCAPE '\\') \
             ORDER BY created_at DESC, id \
             LIMIT $7 OFFSET $8",
            org_db,
            kind,
            enabled,
            source,
            target,
            needle,
            size + 1,
            offset,
        )
        .fetch_all(&mut *tx)
        .await?;

        // The state filter is deliberately absent here: the console renders
        // three tabs and each one's count has to be the count of the other
        // tabs' contents too.
        let counts = sqlx::query!(
            "SELECT count(*) AS \"all!\", \
                    count(*) FILTER (WHERE enabled) AS \"enabled!\", \
                    count(*) FILTER (WHERE NOT enabled) AS \"disabled!\" \
             FROM seller_rule \
             WHERE org_id = $1 \
               AND ($2::text IS NULL OR kind = $2) \
               AND ($3::text IS NULL OR source = $3) \
               AND ($4::text IS NULL OR target = $4) \
               AND ($5::text IS NULL OR search_text LIKE $5 ESCAPE '\\')",
            org_db,
            kind,
            source,
            target,
            needle,
        )
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        let has_next = i64::try_from(rows.len()).unwrap_or(i64::MAX) > size;
        let wanted = usize::try_from(size).unwrap_or(usize::MAX);
        Ok(RulePage {
            rows: rows
                .into_iter()
                .take(wanted)
                .map(decode_rule)
                .collect::<Result<Vec<_>, _>>()?,
            page,
            has_next,
            counts: RuleCounts {
                all: counts.all,
                enabled: counts.enabled,
                disabled: counts.disabled,
            },
        })
    }

    /// Replaces a rule's definition and bumps its revision.
    ///
    /// The row is locked before the revision is compared, so two edits racing
    /// produce one write and one `Stale` rather than two writes at the same
    /// revision. Nothing already approved or queued moves: those hold their
    /// own copies.
    pub async fn update(
        &self,
        org: OrgId,
        edit: &RuleEdit<'_>,
    ) -> Result<Result<SellerRuleRecord, RuleWriteRefusal>, StorageError> {
        let &RuleEdit {
            id,
            revision,
            definition,
            author,
            at,
        } = edit;
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(at)?;
        let json = to_json(serde_json::to_value(definition), "a rule definition")?;
        let auto_apply: Vec<String> = definition
            .auto_apply
            .iter()
            .map(|use_| use_to_db(*use_).to_owned())
            .collect();

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        lock_policy(&mut tx, org).await?;

        let Some(current) = sqlx::query!(
            "SELECT revision, created_at FROM seller_rule \
             WHERE org_id = $1 AND id = $2 FOR UPDATE",
            org_db,
            id,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(Err(RuleWriteRefusal::Missing));
        };

        if current.revision != revision {
            tx.commit().await?;
            return Ok(Err(RuleWriteRefusal::Stale {
                current: current.revision,
            }));
        }

        let next = revision + 1;
        sqlx::query!(
            "UPDATE seller_rule \
             SET revision = $3, kind = $4, title = $5, description = $6, enabled = $7, \
                 source = $8, target = $9, auto_apply = $10, definition = $11, \
                 author = $12, updated_at = $13 \
             WHERE org_id = $1 AND id = $2",
            org_db,
            id,
            next,
            kind_to_db(definition.kind()),
            definition.title,
            definition.description,
            definition.enabled,
            inventory_to_db(definition.source),
            inventory_to_db(definition.target),
            &auto_apply,
            json,
            uuid_to_db(author.0),
            at_db,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(Ok(SellerRuleRecord {
            id: uuid_from_db(id),
            revision: next,
            definition: definition.clone(),
            author,
            created_at: timestamp_from_db(current.created_at),
            updated_at: at,
        }))
    }

    /// Deletes a rule. Nothing it ever proposed goes with it: previews,
    /// approvals and frozen snapshots hold their own copies and carry no
    /// foreign key onto this table, which is why they are unaffected rather
    /// than merely unlikely to be affected.
    pub async fn delete(
        &self,
        org: OrgId,
        id: uuid::Uuid,
        revision: i64,
    ) -> Result<RuleDelete, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        lock_policy(&mut tx, org).await?;

        let Some(current) = sqlx::query!(
            "SELECT revision FROM seller_rule WHERE org_id = $1 AND id = $2 FOR UPDATE",
            org_db,
            id,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(RuleDelete::Missing);
        };

        if current.revision != revision {
            tx.commit().await?;
            return Ok(RuleDelete::Stale {
                current: current.revision,
            });
        }

        sqlx::query!(
            "DELETE FROM seller_rule WHERE org_id = $1 AND id = $2",
            org_db,
            id,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(RuleDelete::Deleted)
    }

    /// Stores an official quote, or returns the one already stored for that
    /// provider, pair and day.
    ///
    /// Immutable by trigger, so a second fetch never rewrites the first: a
    /// rule referring to this identifier was approved against the number
    /// stored here, and changing it under the rule would change what the
    /// seller agreed to.
    ///
    /// The rate is part of the natural key for that same reason. An identical
    /// refetch reuses the identifier, because it is the same observation; a
    /// provider restating the day's rate is a different observation and gets
    /// its own row and its own identifier, rather than being answered with
    /// the superseded amount under the guise of "already stored".
    pub async fn record_reference(
        &self,
        org: OrgId,
        quote: &NewReferenceQuote,
    ) -> Result<ReferenceQuote, StorageError> {
        let org_db = uuid_to_db(org.0);
        let source = currency_to_db(quote.source);
        let target = currency_to_db(quote.target);
        let fetched_at = timestamp_to_db(quote.fetched_at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        sqlx::query!(
            "INSERT INTO seller_rule_reference \
             (org_id, id, source_currency, target_currency, rate_micros, as_of, provider, \
              source_url, fetched_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT ON CONSTRAINT seller_rule_reference_natural DO NOTHING",
            org_db,
            uuid::Uuid::new_v4(),
            source,
            target,
            quote.rate_micros,
            quote.as_of,
            quote.provider,
            quote.source_url,
            fetched_at,
        )
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query!(
            "SELECT id, source_currency, target_currency, rate_micros, as_of, provider, \
                    source_url, fetched_at \
             FROM seller_rule_reference \
             WHERE org_id = $1 AND provider = $2 AND source_currency = $3 \
               AND target_currency = $4 AND as_of = $5 AND rate_micros = $6",
            org_db,
            quote.provider,
            source,
            target,
            quote.as_of,
            quote.rate_micros,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(ReferenceQuote {
            id: row.id,
            source: currency_from_db(&row.source_currency)?,
            target: currency_from_db(&row.target_currency)?,
            rate_micros: row.rate_micros,
            as_of: row.as_of,
            provider: row.provider,
            source_url: row.source_url,
            fetched_at: timestamp_from_db(row.fetched_at),
        })
    }

    pub async fn reference(
        &self,
        org: OrgId,
        id: uuid::Uuid,
    ) -> Result<Option<ReferenceQuote>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let row = sqlx::query!(
            "SELECT id, source_currency, target_currency, rate_micros, as_of, provider, \
                    source_url, fetched_at \
             FROM seller_rule_reference WHERE org_id = $1 AND id = $2",
            uuid_to_db(org.0),
            id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;

        row.map(|row| {
            Ok(ReferenceQuote {
                id: row.id,
                source: currency_from_db(&row.source_currency)?,
                target: currency_from_db(&row.target_currency)?,
                rate_micros: row.rate_micros,
                as_of: row.as_of,
                provider: row.provider,
                source_url: row.source_url,
                fetched_at: timestamp_from_db(row.fetched_at),
            })
        })
        .transpose()
    }

    /// Writes one proposal and its rows as one decision.
    pub async fn save_preview(
        &self,
        org: OrgId,
        preview: NewPreview<'_>,
    ) -> Result<(), StorageError> {
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(preview.at)?;
        let rules = to_json(
            serde_json::to_value(preview.captured_rules),
            "the captured rules",
        )?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        sqlx::query!(
            "INSERT INTO seller_rule_preview \
             (org_id, id, source, target, request, captured_rules, rule_scope, actor, \
              created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            org_db,
            preview.id,
            inventory_to_db(preview.source),
            inventory_to_db(preview.target),
            preview.request,
            rules,
            scope_to_db(preview.scope),
            uuid_to_db(preview.actor.0),
            at_db,
        )
        .execute(&mut *tx)
        .await?;

        for row in preview.rows {
            let price = PriceColumns::from_intent(row.source_price);
            sqlx::query!(
                "INSERT INTO seller_rule_preview_row \
                 (org_id, preview_id, product_id, title, source_price_kind, \
                  source_price_minor, source_price_ccy, source_fingerprint, previous_choice, \
                  before, proposed, patch, matches, blockers, status, decision) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, \
                  'pending')",
                org_db,
                preview.id,
                uuid_to_db(row.product.0),
                row.title,
                price.kind,
                price.minor_units,
                price.currency,
                row.fingerprint.0,
                row.previous_choice,
                to_json(serde_json::to_value(&row.before), "a preview row's before")?,
                to_json(
                    serde_json::to_value(&row.proposed),
                    "a preview row's proposal"
                )?,
                to_json(serde_json::to_value(&row.patch), "a preview row's patch")?,
                to_json(
                    serde_json::to_value(&row.matches),
                    "a preview row's matches"
                )?,
                to_json(
                    serde_json::to_value(&row.blockers),
                    "a preview row's blockers"
                )?,
                status_to_db(row.status),
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn preview(
        &self,
        org: OrgId,
        id: uuid::Uuid,
    ) -> Result<Option<StoredPreview>, StorageError> {
        let org_db = uuid_to_db(org.0);
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;

        let Some(header) = sqlx::query!(
            "SELECT source, target, request, captured_rules, rule_scope, actor, created_at \
             FROM seller_rule_preview WHERE org_id = $1 AND id = $2",
            org_db,
            id,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };

        let rows = fetch_preview_rows(&mut tx, org_db, id).await?;
        tx.commit().await?;

        Ok(Some(StoredPreview {
            id,
            source: inventory_from_db(&header.source)?,
            target: inventory_from_db(&header.target)?,
            request: header.request,
            rules: from_json(
                serde_json::from_value(header.captured_rules),
                "captured rules",
            )?,
            scope: scope_from_db(&header.rule_scope)?,
            actor: UserId(uuid_from_db(header.actor)),
            created_at: timestamp_from_db(header.created_at),
            rows: rows
                .into_iter()
                .map(decode_preview_row)
                .collect::<Result<Vec<_>, StorageError>>()?,
        }))
    }

    /// Records the seller's decision on a proposal's rows, in one
    /// transaction, under the tenant's policy lock and the products' own
    /// locks.
    ///
    /// Everything is checked before anything is written. An accept recomputes
    /// each resource's fingerprint from the row the lock holds, compares every
    /// rule the preview captured against the rule as it now stands, and
    /// compares the resource's current latest approval against the one the
    /// preview recorded. Any disagreement returns [`DecisionOutcome::Stale`]
    /// and the transaction is rolled back, so a confirmation is all or none.
    /// The policy lock is what makes the rule comparison mean anything: it is
    /// taken before the comparison and held through the commit, and every
    /// rule write takes it too, so no rule can be edited, created or enabled
    /// in between.
    ///
    /// Three things about which rows are addressed. `All` is the preview's
    /// pending rows and nothing else. An explicit selection naming one
    /// resource twice decides it once. And a pending row whose patch states
    /// nothing — an ordinary unmatched resource — is decided without
    /// appending an approval, because an approval of nothing is not something
    /// the record holds.
    ///
    /// A row already carrying the decision being made is a replay: it is
    /// counted and nothing is written, so a client that retries a request
    /// whose response it lost does not approve twice. A row carrying the
    /// opposite decision is [`DecisionOutcome::Conflict`].
    pub async fn decide(
        &self,
        org: OrgId,
        decision: &RuleDecision<'_>,
    ) -> Result<DecisionOutcome, StorageError> {
        let &RuleDecision {
            preview,
            actor,
            kind,
            selection,
            at,
        } = decision;
        let org_db = uuid_to_db(org.0);
        let at_db = timestamp_to_db(at)?;

        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        // Taken before the preview's own row and before any product lock, so
        // every holder of both takes them in one order. This is what makes
        // the rule-set recheck below a decision the tenant's own writers
        // cannot overtake, phantoms included.
        lock_policy(&mut tx, org).await?;

        // The preview's own row serialises two decisions on one proposal, so
        // the read of "which rows are still pending" cannot be overtaken by
        // the write that answers it.
        let Some(header) = sqlx::query!(
            "SELECT source, target, captured_rules, rule_scope FROM seller_rule_preview \
             WHERE org_id = $1 AND id = $2 FOR UPDATE",
            org_db,
            preview,
        )
        .fetch_optional(&mut *tx)
        .await?
        else {
            tx.commit().await?;
            return Ok(DecisionOutcome::UnknownPreview);
        };

        let source = inventory_from_db(&header.source)?;
        let target = inventory_from_db(&header.target)?;
        let captured: Vec<SellerRuleRecord> = from_json(
            serde_json::from_value(header.captured_rules),
            "captured rules",
        )?;
        let scope = scope_from_db(&header.rule_scope)?;

        let stored = fetch_preview_rows(&mut tx, org_db, preview).await?;
        let mut rows = stored
            .into_iter()
            .map(decode_preview_row)
            .collect::<Result<Vec<_>, StorageError>>()?;
        rows.sort_by_key(|row| uuid_to_db(row.product.0));

        // Which rows this decision addresses, and what the client asked for
        // that is not there.
        let mut stale: Vec<StaleRow> = Vec::new();
        let chosen: Vec<StoredPreviewRow> = match selection {
            // Every pending row, and only those. A row the seller already
            // decided is not part of "the rest of them", and reading it as
            // one made "reject these two, then accept the rest" a conflict
            // instead of the workflow it is.
            Selection::All => rows
                .into_iter()
                .filter(|row| matches!(row.decision, PreviewDecision::Pending))
                .collect(),
            Selection::Products(products) => {
                // Named twice is named once. Two occurrences of one resource
                // would be checked against one previous choice and then
                // appended as two approvals with two identities, and the
                // report would count one decision twice.
                let mut picked: Vec<StoredPreviewRow> = Vec::with_capacity(products.len());
                for product in products {
                    if picked.iter().any(|row| row.product == *product)
                        || stale.iter().any(|row| row.product == Some(*product))
                    {
                        continue;
                    }
                    match rows.iter().find(|row| row.product == *product) {
                        Some(row) => picked.push(row.clone()),
                        None => stale.push(StaleRow {
                            product: Some(*product),
                            reason: StaleReason::Missing,
                        }),
                    }
                }
                picked.sort_by_key(|row| uuid_to_db(row.product.0));
                picked
            }
        };

        let explicit = matches!(selection, Selection::Products(_));
        let mut conflict = Vec::new();
        let mut replayed = 0_usize;
        let mut blocked = 0_usize;
        let mut pending = Vec::new();

        for row in chosen {
            match (row.decision, kind) {
                (PreviewDecision::Accepted, DecisionKind::Accept)
                | (PreviewDecision::Rejected, DecisionKind::Reject) => replayed += 1,
                (PreviewDecision::Accepted, DecisionKind::Reject)
                | (PreviewDecision::Rejected, DecisionKind::Accept) => conflict.push(row.product),
                (PreviewDecision::Pending, _) => {
                    if matches!(kind, DecisionKind::Accept)
                        && matches!(row.status, PreviewStatus::Blocked)
                    {
                        // An `all` accept passes over blocked rows and says
                        // so; naming one explicitly is asking for something
                        // the preview already refused.
                        if explicit {
                            stale.push(StaleRow {
                                product: Some(row.product),
                                reason: StaleReason::Blocked,
                            });
                        } else {
                            blocked += 1;
                        }
                    } else {
                        pending.push(row);
                    }
                }
            }
        }

        if !conflict.is_empty() {
            tx.rollback().await?;
            return Ok(DecisionOutcome::Conflict(conflict));
        }
        if !stale.is_empty() {
            tx.rollback().await?;
            return Ok(DecisionOutcome::Stale(stale));
        }

        // Every pending row this call reached is decided, whether or not it
        // has an approval to append.
        let (mut accepted, mut rejected) = match kind {
            DecisionKind::Accept => (pending.len(), 0),
            DecisionKind::Reject => (0, pending.len()),
        };

        // The rows that write an approval, which is not every row a seller
        // may accept: a resource no rule matched carries an empty patch, and
        // appending an approval stating nothing is refused by the record
        // itself. Rechecking and locking exactly these keeps a world that
        // moved around an empty row from refusing a decision that would not
        // have written anything about it anyway.
        let writing: Vec<&StoredPreviewRow> = match kind {
            DecisionKind::Accept => pending
                .iter()
                .filter(|row| patch_states_something(&row.patch))
                .collect(),
            DecisionKind::Reject => Vec::new(),
        };

        if !writing.is_empty() {
            // The rules first, because a revised rule invalidates the whole
            // proposal rather than one of its rows: the preview's other rows
            // were computed against the same definitions.
            let frame = CapturedRules {
                scope,
                source,
                target,
                rules: &captured,
            };
            if let Some(reason) = rules_moved(&mut tx, org_db, &frame).await? {
                tx.rollback().await?;
                return Ok(DecisionOutcome::Stale(vec![StaleRow {
                    product: None,
                    reason,
                }]));
            }

            // Locks taken in product-identifier order, which is the order
            // every multi-product write in this crate takes them in, so two
            // confirmations over overlapping selections queue rather than
            // deadlock.
            for row in &writing {
                if !lock_product(&mut tx, org_db, uuid_to_db(row.product.0)).await? {
                    stale.push(StaleRow {
                        product: Some(row.product),
                        reason: StaleReason::Missing,
                    });
                }
            }
            if !stale.is_empty() {
                tx.rollback().await?;
                return Ok(DecisionOutcome::Stale(stale));
            }

            // The validated reads are kept rather than taken again below:
            // the write pass needs exactly the choice this pass compared
            // against, and re-reading it would be both a second statement per
            // row and a second chance to see a different answer.
            let mut previously = Vec::with_capacity(writing.len());
            for row in &writing {
                let Some(record) = get_product_in_tx(&mut tx, org, row.product).await? else {
                    stale.push(StaleRow {
                        product: Some(row.product),
                        reason: StaleReason::Missing,
                    });
                    continue;
                };
                if fingerprint(&record.product) != row.fingerprint {
                    stale.push(StaleRow {
                        product: Some(row.product),
                        reason: StaleReason::SourceChanged,
                    });
                    continue;
                }
                let current =
                    latest_accepted_fields_in_tx(&mut tx, org, row.product, target).await?;
                if current.as_ref().map(|choice| choice.id) != row.previous_choice {
                    stale.push(StaleRow {
                        product: Some(row.product),
                        reason: StaleReason::ChoiceChanged,
                    });
                    continue;
                }
                previously.push(current);
            }
            if !stale.is_empty() {
                tx.rollback().await?;
                return Ok(DecisionOutcome::Stale(stale));
            }

            for (row, previous) in writing.iter().zip(&previously) {
                append_application(
                    &mut tx,
                    org_db,
                    AppliedChoice {
                        product: row.product,
                        source,
                        target,
                        previous: previous.as_ref(),
                        patch: &row.patch,
                        matches: &row.matches,
                        preview,
                        actor,
                        at: at_db,
                    },
                )
                .await?;
            }
        }

        for row in &pending {
            sqlx::query!(
                "UPDATE seller_rule_preview_row \
                 SET decision = $4, decided_at = $5, decided_by = $6 \
                 WHERE org_id = $1 AND preview_id = $2 AND product_id = $3 \
                   AND decision = 'pending'",
                org_db,
                preview,
                uuid_to_db(row.product.0),
                decision_to_db(match kind {
                    DecisionKind::Accept => PreviewDecision::Accepted,
                    DecisionKind::Reject => PreviewDecision::Rejected,
                }),
                at_db,
                uuid_to_db(actor.0),
            )
            .execute(&mut *tx)
            .await?;
        }

        let remaining = sqlx::query!(
            "SELECT count(*) AS \"pending!\" FROM seller_rule_preview_row \
             WHERE org_id = $1 AND preview_id = $2 AND decision = 'pending'",
            org_db,
            preview,
        )
        .fetch_one(&mut *tx)
        .await?
        .pending;

        tx.commit().await?;

        match kind {
            DecisionKind::Accept => accepted += replayed,
            DecisionKind::Reject => rejected += replayed,
        }
        Ok(DecisionOutcome::Applied(DecisionReport {
            accepted,
            rejected,
            remaining: usize::try_from(remaining).unwrap_or(usize::MAX),
            blocked,
        }))
    }

    /// The resource's current target choice, on the pool.
    ///
    /// The twin of [`latest_accepted_fields_in_tx`] for a caller that is only
    /// reading — building a preview's `before`, rendering a console page.
    /// Anything deciding on the answer has to use the in-transaction form
    /// under the product's lock.
    pub async fn latest_accepted_fields(
        &self,
        org: OrgId,
        product: ProductId,
        target: InventoryId,
    ) -> Result<Option<AcceptedChoice>, StorageError> {
        let mut tx = self.pool.begin().await?;
        pin_org(&mut tx, org).await?;
        let choice = latest_accepted_fields_in_tx(&mut tx, org, product, target).await?;
        tx.commit().await?;
        Ok(choice)
    }
}

/// Serialises this tenant's whole policy for the rest of the transaction.
///
/// The decision it protects is "is this the rule set the seller was shown",
/// and the rows that answer it include rows that do not exist yet: a rule
/// created or enabled after a confirmation compared the set would have taken
/// part in a proposal nobody saw, and `FOR UPDATE` over the rules currently
/// in the direction cannot exclude an insert. Advisory and transaction-scoped
/// for that reason, in the form [`crate::import_runs`] already uses for a
/// decision spread over rows rather than held in one.
///
/// Taken by every rule write, by every confirmation, and by the runtime as it
/// captures the policy a request is carried out against — each of them before
/// any preview or product lock, which is the ordering that keeps the set
/// deadlock-free.
pub(crate) async fn lock_policy(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
) -> Result<(), StorageError> {
    let key = uuid_to_db(org.0).to_string();
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('seller-policy:' || $1, 0))",
        key,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

/// The resource's current target choice, inside a transaction the caller
/// owns.
///
/// One indexed read rather than a fold over history, because every
/// application row carries the whole choice: the latest row *is* the answer,
/// including the fields it inherited from earlier approvals and the authors
/// they were approved by.
///
/// Keyed on the target and not on the source-target pair. A listing on one
/// marketplace has one price and one licence; which catalogue a proposal was
/// computed from is provenance, kept on the row and never part of the lookup.
/// Keying on the pair would let two sources hold two contradictory current
/// choices for one listing, and the runtime would have to pick.
pub async fn latest_accepted_fields_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    product: ProductId,
    target: InventoryId,
) -> Result<Option<AcceptedChoice>, StorageError> {
    let row = sqlx::query_as!(
        ApplicationRow,
        "SELECT id, sequence, price_kind, price_minor_units, price_currency, price_author, \
                licence_native_id, licence_author, resource_type_native_id, \
                resource_type_author, matches, approved_at \
         FROM seller_rule_application \
         WHERE org_id = $1 AND product_id = $2 AND target = $3 \
         ORDER BY sequence DESC LIMIT 1",
        uuid_to_db(org.0),
        uuid_to_db(product.0),
        inventory_to_db(target),
    )
    .fetch_optional(&mut **tx)
    .await?;

    row.map(decode_application).transpose()
}

/// The current target choice for many resources at once, in one statement.
///
/// `None` is every resource this tenant has ever had a choice approved for in
/// this target, which is what a native read needs before it knows which
/// resources it is about to find. `Some` narrows to the named resources, and
/// a resource with no approval is absent from the answer rather than present
/// with an empty one.
///
/// `DISTINCT ON` over the latest-choice index, so this is one scan of the
/// tenant's own rows and not a lookup per resource: the alternative — list
/// the products, then ask per product — grows a statement per resource and
/// reads the products for no other reason.
pub async fn accepted_choices_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    target: InventoryId,
    products: Option<&[ProductId]>,
) -> Result<Vec<(ProductId, AcceptedChoice)>, StorageError> {
    let wanted: Option<Vec<uuid::Uuid>> =
        products.map(|products| products.iter().map(|id| uuid_to_db(id.0)).collect());

    let rows = sqlx::query_as!(
        ChoiceRow,
        "SELECT DISTINCT ON (product_id) \
                product_id, id, sequence, price_kind, price_minor_units, price_currency, \
                price_author, licence_native_id, licence_author, resource_type_native_id, \
                resource_type_author, matches, approved_at \
         FROM seller_rule_application \
         WHERE org_id = $1 AND target = $2 \
           AND ($3::uuid[] IS NULL OR product_id = ANY($3)) \
         ORDER BY product_id, sequence DESC",
        uuid_to_db(org.0),
        inventory_to_db(target),
        wanted.as_deref(),
    )
    .fetch_all(&mut **tx)
    .await?;

    rows.into_iter()
        .map(|row| {
            let (product, application) = row.split();
            Ok((product, decode_application(application)?))
        })
        .collect()
}

/// The rules that apply to this direction without being asked, for one use.
///
/// `auto_apply` is a set column rather than a field inside the definition
/// json, so this is an indexed containment test: the runtime asks it once per
/// enqueue and a jsonb scan of every rule the tenant owns would be the wrong
/// shape for that.
pub async fn enabled_rules_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    source: InventoryId,
    target: InventoryId,
    use_: RuleUse,
) -> Result<Vec<SellerRuleRecord>, StorageError> {
    let rows = sqlx::query_as!(
        RuleRow,
        "SELECT id, revision, definition, author, created_at, updated_at \
         FROM seller_rule \
         WHERE org_id = $1 AND enabled AND source = $2 AND target = $3 \
           AND auto_apply @> ARRAY[$4]::text[] \
         ORDER BY created_at, id",
        uuid_to_db(org.0),
        inventory_to_db(source),
        inventory_to_db(target),
        use_to_db(use_),
    )
    .fetch_all(&mut **tx)
    .await?;

    rows.into_iter().map(decode_rule).collect()
}

/// The columns one frozen request choice occupies, whichever shape it is.
///
/// One encoder rather than two inserts, because the two shapes differ in
/// exactly three columns — the approval's identity, whether the price is
/// optional, and who authored it — and a second statement would be a second
/// place for the column list to drift.
struct ChoiceColumns<'a> {
    application: Option<uuid::Uuid>,
    price: OptionalPriceColumns,
    price_author: Option<uuid::Uuid>,
    licence: Option<&'a SellerTermChoice>,
    resource_type: Option<&'a SellerTermChoice>,
    matches: serde_json::Value,
    approved_at: chrono::DateTime<chrono::Utc>,
}

impl<'a> ChoiceColumns<'a> {
    fn encode(choice: &'a RequestChoice, at: Timestamp) -> Result<Self, StorageError> {
        match choice {
            RequestChoice::Accepted(accepted) => Ok(Self {
                application: Some(accepted.id),
                price: OptionalPriceColumns::encode(accepted.price.as_ref()),
                price_author: accepted.price_author.map(|author| uuid_to_db(author.0)),
                licence: accepted.licence.as_ref(),
                resource_type: accepted.resource_type.as_ref(),
                matches: to_json(
                    serde_json::to_value(&accepted.matches),
                    "a frozen choice's matches",
                )?,
                // An approval's own instant, which is not this confirmation's:
                // the seller approved this choice when they approved it.
                approved_at: timestamp_to_db(accepted.approved_at)?,
            }),
            RequestChoice::Output(output) => Ok(Self {
                application: None,
                price: OptionalPriceColumns::encode(Some(&output.price)),
                // The amount is what the rules computed, not something a
                // person granted, and naming an author here would label an
                // automatic answer a manual decision.
                price_author: None,
                licence: output.licence.as_ref(),
                resource_type: output.resource_type.as_ref(),
                matches: to_json(
                    serde_json::to_value(&output.matches),
                    "a frozen output's matches",
                )?,
                // The output came into existence at confirmation.
                approved_at: timestamp_to_db(at)?,
            }),
        }
    }
}

/// Freezes the definitions and choices a request was confirmed against.
///
/// Definitions and choices travel in one call because they are one fact: what
/// the seller confirmed, at the instant they confirmed it. Two calls could
/// capture a rule from before an edit and a choice from after one.
///
/// A choice is written as one of the two shapes [`RequestChoice`] names, told
/// apart by `application_id`: present for an approval this request carries
/// forward, absent for an evaluated output frozen at confirmation. An output
/// states an exact price and no price author, because the amount is the
/// automatic answer and not a manual grant; the CHECKs on the table say the
/// same thing, so neither shape can be written half-formed.
pub async fn save_request_snapshot_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    request: uuid::Uuid,
    snapshot: &NewRequestSnapshot<'_>,
) -> Result<(), StorageError> {
    let org_db = uuid_to_db(org.0);
    let at_db = timestamp_to_db(snapshot.at)?;

    sqlx::query!(
        "INSERT INTO sync_request_rule_snapshot \
         (org_id, request_id, source, target, rules, captured_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
        org_db,
        request,
        inventory_to_db(snapshot.source),
        inventory_to_db(snapshot.target),
        to_json(serde_json::to_value(&snapshot.rules), "the confirmed rules")?,
        at_db,
    )
    .execute(&mut **tx)
    .await?;

    for (product, choice) in snapshot.choices {
        let columns = ChoiceColumns::encode(choice, snapshot.at)?;
        let licence = TermColumns::encode(columns.licence);
        let resource_type = TermColumns::encode(columns.resource_type);
        sqlx::query!(
            "INSERT INTO sync_request_rule_choice \
             (org_id, request_id, product_id, application_id, price_kind, price_minor_units, \
              price_currency, price_author, licence_native_id, licence_author, \
              resource_type_native_id, resource_type_author, matches, approved_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            org_db,
            request,
            uuid_to_db(product.0),
            columns.application,
            columns.price.kind,
            columns.price.minor_units,
            columns.price.currency,
            columns.price_author,
            licence.native_id,
            licence.author,
            resource_type.native_id,
            resource_type.author,
            columns.matches,
            columns.approved_at,
        )
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

pub async fn request_snapshot_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    request: uuid::Uuid,
) -> Result<Option<RequestRuleSnapshot>, StorageError> {
    let org_db = uuid_to_db(org.0);
    let Some(header) = sqlx::query!(
        "SELECT source, target, rules, captured_at FROM sync_request_rule_snapshot \
         WHERE org_id = $1 AND request_id = $2",
        org_db,
        request,
    )
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };

    let rows = sqlx::query!(
        "SELECT product_id, application_id, price_kind, price_minor_units, price_currency, \
                price_author, licence_native_id, licence_author, resource_type_native_id, \
                resource_type_author, matches, approved_at \
         FROM sync_request_rule_choice \
         WHERE org_id = $1 AND request_id = $2 \
         ORDER BY product_id",
        org_db,
        request,
    )
    .fetch_all(&mut **tx)
    .await?;

    let mut choices = Vec::with_capacity(rows.len());
    for row in rows {
        let price =
            optional_price_from_db(row.price_kind, row.price_minor_units, row.price_currency)?;
        let licence = TermColumns::decode(
            row.licence_native_id,
            row.licence_author,
            "a frozen licence",
        )?;
        let resource_type = TermColumns::decode(
            row.resource_type_native_id,
            row.resource_type_author,
            "a frozen resource type",
        )?;
        let matches = from_json(
            serde_json::from_value(row.matches),
            "a frozen choice's matches",
        )?;

        // The discriminant is the approval's identity: an output has none,
        // because no approval states it. An output without its exact price is
        // a corrupt row and not an occasion to invent a free listing — the
        // whole point of freezing the output is that the amount is known.
        let choice = if let Some(application) = row.application_id {
            RequestChoice::Accepted(AcceptedChoice {
                id: application,
                price,
                price_author: row.price_author.map(|id| UserId(uuid_from_db(id))),
                licence,
                resource_type,
                matches,
                approved_at: timestamp_from_db(row.approved_at),
            })
        } else {
            let Some(price) = price else {
                return Err(StorageError::CorruptRow {
                    reason: "a frozen request output states no price".to_owned(),
                });
            };
            if row.price_author.is_some() {
                return Err(StorageError::CorruptRow {
                    reason: "a frozen request output states a price author".to_owned(),
                });
            }
            RequestChoice::Output(FrozenRuleOutput {
                price,
                licence,
                resource_type,
                matches,
            })
        };
        choices.push((ProductId(uuid_from_db(row.product_id)), choice));
    }

    Ok(Some(RequestRuleSnapshot {
        source: inventory_from_db(&header.source)?,
        target: inventory_from_db(&header.target)?,
        rules: from_json(serde_json::from_value(header.rules), "confirmed rules")?,
        choices,
        captured_at: timestamp_from_db(header.captured_at),
    }))
}

/// Freezes what one queued item will publish.
///
/// Insert-only: the table's trigger refuses an UPDATE, so a second save for
/// one item raises a unique violation rather than quietly changing what a
/// lease already in flight believes it is sending. The price is a typed
/// triple with a totality CHECK, so an engine reading it back gets the exact
/// amount it was queued with and never a rate to re-apply.
pub async fn save_item_output_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    item: uuid::Uuid,
    output: &FrozenRuleOutput,
) -> Result<(), StorageError> {
    let price = PriceColumns::from_intent(output.price);
    let licence = TermColumns::encode(output.licence.as_ref());
    let resource_type = TermColumns::encode(output.resource_type.as_ref());

    sqlx::query!(
        "INSERT INTO job_item_rule_output \
         (org_id, item_id, price_kind, price_minor_units, price_currency, \
          licence_native_id, licence_author, resource_type_native_id, resource_type_author, \
          matches) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        uuid_to_db(org.0),
        item,
        price.kind,
        price.minor_units,
        price.currency,
        licence.native_id,
        licence.author,
        resource_type.native_id,
        resource_type.author,
        to_json(
            serde_json::to_value(&output.matches),
            "a frozen output's matches"
        )?,
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn item_output_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: OrgId,
    item: uuid::Uuid,
) -> Result<Option<FrozenRuleOutput>, StorageError> {
    let row = sqlx::query!(
        "SELECT price_kind, price_minor_units, price_currency, licence_native_id, \
                licence_author, resource_type_native_id, resource_type_author, matches \
         FROM job_item_rule_output WHERE org_id = $1 AND item_id = $2",
        uuid_to_db(org.0),
        item,
    )
    .fetch_optional(&mut **tx)
    .await?;

    let Some(row) = row else { return Ok(None) };
    Ok(Some(FrozenRuleOutput {
        price: price_from_db(&row.price_kind, row.price_minor_units, row.price_currency)?,
        licence: TermColumns::decode(
            row.licence_native_id,
            row.licence_author,
            "a frozen licence",
        )?,
        resource_type: TermColumns::decode(
            row.resource_type_native_id,
            row.resource_type_author,
            "a frozen resource type",
        )?,
        matches: from_json(
            serde_json::from_value(row.matches),
            "a frozen output's matches",
        )?,
    }))
}

/// What one queued item will publish, on a pool the caller holds.
///
/// The engine's read: `seed::prepare_item` runs on the worker's own pool and
/// runs twice for one lease, and both readings have to be the same financial
/// answer. They are, because nothing here is recomputed — the row was written
/// once and cannot be updated.
///
/// `None` is a pre-0083 item, queued before any of this existed. It is not an
/// item whose output was lost: no output was ever frozen for it, and the
/// caller keeps the behaviour that item was queued under rather than
/// inventing an approval nobody gave.
pub async fn item_output(
    pool: &PgPool,
    org: OrgId,
    item: uuid::Uuid,
) -> Result<Option<FrozenRuleOutput>, StorageError> {
    let mut tx = pool.begin().await?;
    pin_org(&mut tx, org).await?;
    let output = item_output_in_tx(&mut tx, org, item).await?;
    tx.commit().await?;
    Ok(output)
}

/// The rule set one proposal was computed from, and how it was formed.
struct CapturedRules<'a> {
    scope: RuleScope,
    source: InventoryId,
    target: InventoryId,
    rules: &'a [SellerRuleRecord],
}

/// Whether the rule set the proposal was computed from still stands.
///
/// Two questions, because a preview's set was formed in one of two ways.
///
/// For an explicit set — rules the client named, or a draft alone — the only
/// question is whether each captured rule still exists at the revision it was
/// captured at. A rule created elsewhere is not part of what the client asked
/// for and does not make the answer wrong.
///
/// For an implicit set — every enabled rule in this direction — the set
/// itself is part of what was computed, so a rule created since, or one
/// enabled since, or one whose direction now points here, would have taken
/// part in a proposal the seller never saw. Comparing the captured
/// identifiers and revisions against the direction as it now stands catches
/// all three, and catches deletion and revision as the same comparison.
async fn rules_moved(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    captured: &CapturedRules<'_>,
) -> Result<Option<StaleReason>, StorageError> {
    let &CapturedRules {
        scope,
        source,
        target,
        rules: captured,
    } = captured;
    if matches!(scope, RuleScope::Implicit) {
        let current = sqlx::query!(
            "SELECT id, revision FROM seller_rule \
             WHERE org_id = $1 AND enabled AND source = $2 AND target = $3 \
             ORDER BY id",
            org_db,
            inventory_to_db(source),
            inventory_to_db(target),
        )
        .fetch_all(&mut **tx)
        .await?;

        // A rule that joined the direction since the preview. Reported as the
        // newcomer rather than as a vague "something changed", so the console
        // can name it.
        for row in &current {
            if !captured
                .iter()
                .any(|rule| uuid_to_db(rule.id) == row.id && rule.revision == row.revision)
            {
                return Ok(Some(StaleReason::RuleRevised { rule: row.id }));
            }
        }
        // A rule that left it, by deletion, by being disabled, or by being
        // pointed somewhere else.
        for rule in captured {
            let id = uuid_to_db(rule.id);
            if !current
                .iter()
                .any(|row| row.id == id && row.revision == rule.revision)
            {
                return Ok(Some(StaleReason::RuleRevised { rule: id }));
            }
        }
        return Ok(None);
    }

    for rule in captured {
        let id = uuid_to_db(rule.id);
        let current = sqlx::query!(
            "SELECT revision FROM seller_rule WHERE org_id = $1 AND id = $2",
            org_db,
            id,
        )
        .fetch_optional(&mut **tx)
        .await?;

        // A deleted rule and an edited one are the same answer to a
        // confirmation: what the seller was shown is not what would be
        // applied.
        if current.map(|row| row.revision) != Some(rule.revision) {
            return Ok(Some(StaleReason::RuleRevised { rule: id }));
        }
    }
    Ok(None)
}

/// One approval about to be appended.
struct AppliedChoice<'a> {
    product: ProductId,
    source: InventoryId,
    target: InventoryId,
    previous: Option<&'a AcceptedChoice>,
    /// What this approval decides, and only that. See [`merge_choice`].
    patch: &'a TargetFields,
    matches: &'a [RuleMatch],
    preview: uuid::Uuid,
    actor: UserId,
    at: chrono::DateTime<chrono::Utc>,
}

/// The whole target choice an approval leaves behind, each field with the
/// author who approved that field.
struct MergedChoice {
    price: Option<PriceIntent>,
    price_author: Option<UserId>,
    licence: Option<SellerTermChoice>,
    resource_type: Option<SellerTermChoice>,
}

/// This approval's patch over the choice that was already there.
///
/// The patch is what the approver decided, so a field present in it is
/// theirs — including where it restates the value already stored, which is a
/// fresh grant of the same terms and not an inheritance. A field absent from
/// it keeps the value *and the author* it already had, which is how a
/// price-only approval over a licence approved last month leaves the licence
/// attributed to whoever approved it.
///
/// Deliberately not the merged fields the preview showed the seller. Those
/// carry every inherited field as well, and drawing authorship from them
/// makes each approver the author of everything they were shown: a
/// pricing-only accept would restate somebody else's rights decision as its
/// own.
fn merge_choice(
    previous: Option<&AcceptedChoice>,
    patch: &TargetFields,
    actor: UserId,
) -> MergedChoice {
    let (price, price_author) = match patch.price {
        Some(price) => (Some(price), Some(actor)),
        None => (
            previous.and_then(|prev| prev.price),
            previous.and_then(|prev| prev.price_author),
        ),
    };
    MergedChoice {
        price,
        price_author,
        licence: merge_term(
            patch.licence.as_ref(),
            previous.and_then(|prev| prev.licence.as_ref()),
            actor,
        ),
        resource_type: merge_term(
            patch.resource_type.as_ref(),
            previous.and_then(|prev| prev.resource_type.as_ref()),
            actor,
        ),
    }
}

/// One native field of the merged choice: this approver's grant where the
/// patch states one, and the stored value with its own author where it does
/// not.
fn merge_term(
    patch: Option<&String>,
    previous: Option<&SellerTermChoice>,
    actor: UserId,
) -> Option<SellerTermChoice> {
    match patch {
        Some(native_id) => Some(SellerTermChoice {
            native_id: native_id.clone(),
            author: actor,
        }),
        None => previous.cloned(),
    }
}

/// Appends the resource's new whole target choice, as [`merge_choice`]
/// computes it.
///
/// The row is the whole choice rather than the patch, so the current choice
/// stays one indexed read, and each field carries the author of that field
/// rather than of the row.
async fn append_application(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    choice: AppliedChoice<'_>,
) -> Result<(), StorageError> {
    let actor_db = uuid_to_db(choice.actor.0);
    let merged = merge_choice(choice.previous, choice.patch, choice.actor);

    let price_author = merged.price_author.map(|author| uuid_to_db(author.0));
    let price_columns = OptionalPriceColumns::encode(merged.price.as_ref());
    let licence_columns = TermColumns::encode(merged.licence.as_ref());
    let resource_type_columns = TermColumns::encode(merged.resource_type.as_ref());

    // The product's row is already locked by the caller, which is what makes
    // "one past the greatest" a safe way to number these.
    let sequence = sqlx::query!(
        "SELECT COALESCE(MAX(sequence), 0) AS \"highest!\" FROM seller_rule_application \
         WHERE org_id = $1 AND product_id = $2 AND target = $3",
        org_db,
        uuid_to_db(choice.product.0),
        inventory_to_db(choice.target),
    )
    .fetch_one(&mut **tx)
    .await?
    .highest
        + 1;

    sqlx::query!(
        "INSERT INTO seller_rule_application \
         (org_id, id, product_id, source, target, sequence, price_kind, price_minor_units, \
          price_currency, price_author, licence_native_id, licence_author, \
          resource_type_native_id, resource_type_author, matches, preview_id, author, \
          approved_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
        org_db,
        uuid::Uuid::new_v4(),
        uuid_to_db(choice.product.0),
        inventory_to_db(choice.source),
        inventory_to_db(choice.target),
        sequence,
        price_columns.kind,
        price_columns.minor_units,
        price_columns.currency,
        price_author,
        licence_columns.native_id,
        licence_columns.author,
        resource_type_columns.native_id,
        resource_type_columns.author,
        to_json(
            serde_json::to_value(&choice.matches),
            "an approval's matches"
        )?,
        choice.preview,
        actor_db,
        choice.at,
    )
    .execute(&mut **tx)
    .await?;

    Ok(())
}

struct RuleRow {
    id: uuid::Uuid,
    revision: i64,
    definition: serde_json::Value,
    author: uuid::Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn decode_rule(row: RuleRow) -> Result<SellerRuleRecord, StorageError> {
    Ok(SellerRuleRecord {
        id: uuid_from_db(row.id),
        revision: row.revision,
        definition: from_json(serde_json::from_value(row.definition), "a rule definition")?,
        author: UserId(uuid_from_db(row.author)),
        created_at: timestamp_from_db(row.created_at),
        updated_at: timestamp_from_db(row.updated_at),
    })
}

struct ApplicationRow {
    id: uuid::Uuid,
    sequence: i64,
    price_kind: Option<String>,
    price_minor_units: Option<i64>,
    price_currency: Option<String>,
    price_author: Option<uuid::Uuid>,
    licence_native_id: Option<String>,
    licence_author: Option<uuid::Uuid>,
    resource_type_native_id: Option<String>,
    resource_type_author: Option<uuid::Uuid>,
    matches: serde_json::Value,
    approved_at: chrono::DateTime<chrono::Utc>,
}

fn decode_application(row: ApplicationRow) -> Result<AcceptedChoice, StorageError> {
    // The sequence is read and deliberately not carried out of here: it
    // orders rows and is not a fact about the choice, and a caller holding
    // one would be holding a number it could compare against a row it has
    // not locked.
    let _ = row.sequence;
    Ok(AcceptedChoice {
        id: row.id,
        price: optional_price_from_db(row.price_kind, row.price_minor_units, row.price_currency)?,
        price_author: row.price_author.map(|id| UserId(uuid_from_db(id))),
        licence: TermColumns::decode(row.licence_native_id, row.licence_author, "a licence")?,
        resource_type: TermColumns::decode(
            row.resource_type_native_id,
            row.resource_type_author,
            "a resource type",
        )?,
        matches: from_json(serde_json::from_value(row.matches), "an approval's matches")?,
        approved_at: timestamp_from_db(row.approved_at),
    })
}

/// The same twelve columns as [`ApplicationRow`], plus the resource they
/// belong to: the shape of the many-resource read.
struct ChoiceRow {
    product_id: uuid::Uuid,
    id: uuid::Uuid,
    sequence: i64,
    price_kind: Option<String>,
    price_minor_units: Option<i64>,
    price_currency: Option<String>,
    price_author: Option<uuid::Uuid>,
    licence_native_id: Option<String>,
    licence_author: Option<uuid::Uuid>,
    resource_type_native_id: Option<String>,
    resource_type_author: Option<uuid::Uuid>,
    matches: serde_json::Value,
    approved_at: chrono::DateTime<chrono::Utc>,
}

impl ChoiceRow {
    fn split(self) -> (ProductId, ApplicationRow) {
        (
            ProductId(uuid_from_db(self.product_id)),
            ApplicationRow {
                id: self.id,
                sequence: self.sequence,
                price_kind: self.price_kind,
                price_minor_units: self.price_minor_units,
                price_currency: self.price_currency,
                price_author: self.price_author,
                licence_native_id: self.licence_native_id,
                licence_author: self.licence_author,
                resource_type_native_id: self.resource_type_native_id,
                resource_type_author: self.resource_type_author,
                matches: self.matches,
                approved_at: self.approved_at,
            },
        )
    }
}

/// Whether a patch grants anything at all.
///
/// A preview row for a resource no rule matched carries an empty patch, and
/// an approval stating nothing is not an approval — the record refuses one by
/// CHECK. Accepting such a row is a decision the seller may record, and a
/// wider selection containing one must not fail on its account.
fn patch_states_something(patch: &TargetFields) -> bool {
    patch.price.is_some() || patch.licence.is_some() || patch.resource_type.is_some()
}

struct PreviewRowRow {
    product_id: uuid::Uuid,
    title: String,
    source_price_kind: String,
    source_price_minor: Option<i64>,
    source_price_ccy: Option<String>,
    source_fingerprint: uuid::Uuid,
    previous_choice: Option<uuid::Uuid>,
    before: serde_json::Value,
    proposed: serde_json::Value,
    patch: serde_json::Value,
    matches: serde_json::Value,
    blockers: serde_json::Value,
    status: String,
    decision: String,
}

async fn fetch_preview_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_db: uuid::Uuid,
    preview: uuid::Uuid,
) -> Result<Vec<PreviewRowRow>, StorageError> {
    Ok(sqlx::query_as!(
        PreviewRowRow,
        "SELECT product_id, title, source_price_kind, source_price_minor, source_price_ccy, \
                source_fingerprint, previous_choice, before, proposed, patch, matches, \
                blockers, status, decision \
         FROM seller_rule_preview_row \
         WHERE org_id = $1 AND preview_id = $2 \
         ORDER BY product_id",
        org_db,
        preview,
    )
    .fetch_all(&mut **tx)
    .await?)
}

fn decode_preview_row(row: PreviewRowRow) -> Result<StoredPreviewRow, StorageError> {
    Ok(StoredPreviewRow {
        product: ProductId(uuid_from_db(row.product_id)),
        fingerprint: SourceFingerprint(row.source_fingerprint),
        previous_choice: row.previous_choice,
        title: row.title,
        source_price: price_from_db(
            &row.source_price_kind,
            row.source_price_minor,
            row.source_price_ccy,
        )?,
        before: from_json(serde_json::from_value(row.before), "a preview row's before")?,
        proposed: from_json(
            serde_json::from_value(row.proposed),
            "a preview row's proposal",
        )?,
        patch: from_json(serde_json::from_value(row.patch), "a preview row's patch")?,
        matches: from_json(
            serde_json::from_value(row.matches),
            "a preview row's matches",
        )?,
        blockers: from_json(
            serde_json::from_value(row.blockers),
            "a preview row's blockers",
        )?,
        status: status_from_db(&row.status)?,
        decision: decision_from_db(&row.decision)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, merge_choice, patch_states_something, AcceptedChoice};
    use tam_domain::seller_rules::{SellerTermChoice, TargetFields};
    use tam_domain::{
        CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration, VocabularyId,
        VocabularyPath,
    };
    use tam_types::{
        CopyFormat, Currency, InventoryId, ListingCopy, Money, OrgId, PriceIntent, ProductId,
        TermKind, Timestamp, Title, UserId, Uuid,
    };

    const APPROVER: UserId = UserId(Uuid([0x0A; 16]));
    const EARLIER: UserId = UserId(Uuid([0x0B; 16]));

    fn product(grades: Vec<VocabularyPath>) -> CanonicalProduct {
        CanonicalProduct {
            id: ProductId(Uuid([0x07; 16])),
            org: OrgId(Uuid([0x08; 16])),
            title: Title("Fractions".to_owned()),
            body: ListingCopy {
                body: "A worksheet.".to_owned(),
                format: CopyFormat::Markdown,
            },
            payload: None,
            cover: None,
            previews: Vec::new(),
            subjects: Vec::new(),
            grades: GradeDeclaration {
                source: DeclarationSource::Seller,
                raw: grades,
                derived: None,
            },
            price: PriceIntent::Free,
            rights: RightsDeclaration::Unstated,
            native_residue: Vec::new(),
        }
    }

    fn path(inventory: InventoryId, segments: &[&str], native_id: Option<&str>) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Phase),
            segments: segments.iter().map(|s| (*s).to_owned()).collect(),
            native_id: native_id.map(str::to_owned),
        }
    }

    /// The confirmation guard is fingerprint equality, so two declarations
    /// that match different conditions have to digest differently. These two
    /// spell the same scalars in the same order and differ only in where the
    /// path boundaries fall, which is exactly what a flattened encoding lost.
    #[test]
    fn grade_paths_differing_only_in_their_boundaries_digest_differently() {
        let one = product(vec![path(
            InventoryId::Tpt,
            &["a", "x", "tes", "phase", "b"],
            Some("c"),
        )]);
        let two = product(vec![
            path(InventoryId::Tpt, &["a"], Some("x")),
            path(InventoryId::Tes, &["b"], Some("c")),
        ]);

        assert_ne!(
            fingerprint(&one),
            fingerprint(&two),
            "one five-segment path and two paths spelling the same strings are different sources"
        );
    }

    /// An absent native identifier is not one that happens to be empty: a
    /// condition on it matches one and not the other.
    #[test]
    fn an_absent_native_identifier_digests_differently_from_an_empty_one() {
        let absent = product(vec![path(InventoryId::Tpt, &["a"], None)]);
        let empty = product(vec![path(InventoryId::Tpt, &["a"], Some(""))]);

        assert_ne!(
            fingerprint(&absent),
            fingerprint(&empty),
            "no native identifier and an empty one are different sources"
        );
    }

    fn previous() -> AcceptedChoice {
        AcceptedChoice {
            id: uuid::Uuid::from_u128(9),
            price: Some(PriceIntent::Paid(
                Money::new(400, Currency::Gbp).expect("a positive amount"),
            )),
            price_author: Some(EARLIER),
            licence: Some(SellerTermChoice {
                native_id: "tes-paid".to_owned(),
                author: EARLIER,
            }),
            resource_type: None,
            matches: Vec::new(),
            approved_at: Timestamp(1_789_128_000_000),
        }
    }

    /// The distinction the patch exists for: a field this approval states is
    /// theirs even where the value is unchanged, and a field it does not
    /// state keeps its own author.
    #[test]
    fn authorship_follows_the_patch_and_not_the_value() {
        let previous = previous();
        let restated = merge_choice(
            Some(&previous),
            &TargetFields {
                price: None,
                licence: Some("tes-paid".to_owned()),
                resource_type: None,
            },
            APPROVER,
        );

        assert_eq!(
            restated.licence.as_ref().map(|choice| choice.author),
            Some(APPROVER),
            "restating the same licence is this approver's own grant"
        );
        assert_eq!(
            restated.price_author,
            Some(EARLIER),
            "the price this approval says nothing about keeps its author"
        );
        assert_eq!(
            restated.price, previous.price,
            "and keeps its value with it"
        );
    }

    /// A price-only approval must not restate somebody else's rights
    /// decision, which is what drawing authorship from the merged display
    /// fields did.
    #[test]
    fn a_price_only_approval_leaves_the_licence_author_alone() {
        let previous = previous();
        let merged = merge_choice(
            Some(&previous),
            &TargetFields {
                price: Some(PriceIntent::Free),
                licence: None,
                resource_type: None,
            },
            APPROVER,
        );

        assert_eq!(
            merged.price_author,
            Some(APPROVER),
            "the price is what this approval decided"
        );
        assert_eq!(
            merged.price,
            Some(PriceIntent::Free),
            "and it is the patch's value"
        );
        assert_eq!(
            merged.licence.as_ref().map(|choice| choice.author),
            Some(EARLIER),
            "the licence is inherited, author included"
        );
        assert_eq!(
            merged.licence.map(|choice| choice.native_id),
            Some("tes-paid".to_owned()),
            "the inherited licence keeps its value too"
        );
    }

    /// What keeps an accept over an unmatched resource from attempting an
    /// approval the record refuses.
    #[test]
    fn an_empty_patch_states_nothing_and_a_free_price_states_something() {
        assert!(
            !patch_states_something(&TargetFields::default()),
            "an unmatched resource's patch grants nothing"
        );
        assert!(
            patch_states_something(&TargetFields {
                price: Some(PriceIntent::Free),
                licence: None,
                resource_type: None,
            }),
            "a free price is a decision, not an absence"
        );
    }
}
