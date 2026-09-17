//! Target-only seller policies with fixed-point money.
//!
//! Conditions read source facts; evaluations return target patches. Present
//! fields are explicit proposals, including same-valued choices. Inherited
//! fields stay absent so partial approvals preserve their authorship.
//!
//! Disagreeing rules block unless an explicit override resolves the field.
//! The final price/licence pair is checked against the merged target choices.
//!
//! Rates are decimal strings on the wire and integer millionths in memory.
//! Conversion uses `i128` intermediates and integer rounding for reproducible
//! prices. Nothing here saves or applies a rule: `auto_apply` records separate
//! seller opt-ins, and presets opt into no automatic use.

use crate::registry::{registry, NativeVocabulary};
use crate::{CanonicalProduct, RightsDeclaration, TermKind, VocabularyPath};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tam_types::natives::is_tpt_tag_slug;
use tam_types::{
    CopyFormat, Currency, CurrencyRule, ImportedTerm, InventoryId, ListingCopy, Money, PriceIntent,
    Rounding, Timestamp, UserId, Uuid,
};

// ------------------------------------------------------------ fixed point

/// Millionths in one unit of rate. A rate crosses the wire as a decimal
/// string and lives in memory as this many micros, so `0.75` is exactly
/// `750_000` and no parse of the same string can yield a different number.
const MICROS_PER_UNIT: i128 = 1_000_000;

/// The most fraction digits a rate may carry. Six is the resolution the
/// micros representation holds exactly; a seventh digit would be silently
/// dropped, which is the defect this refuses rather than rounds.
const RATE_FRACTION_DIGITS_MAX: usize = 6;

/// Minor units in one major unit, which is what makes a charm price a
/// question about the last two digits.
const MINOR_UNITS_PER_MAJOR: i128 = 100;

/// The minor-unit remainder a charm price ends on: `.99`.
const CHARM_REMAINDER: i128 = 99;

/// The Tes paid floor and ceiling in minor units, from the captured reference
/// data recorded at `docs/research/rethink/cross-marketplace-mapping-tpt-base.md:46`:
/// "price as an integer in minor units with a floor of 1.00 everywhere except
/// the US at 1.50 and a ceiling of 300.00". GBP is the only currency a Tes
/// inventory prices in (`InventoryId::currency_rule`), so the GB floor is the
/// one that applies here and the US 1.50 branch has no reachable caller.
///
/// A converted price outside the range is refused and never clamped: clamping
/// would post a price the seller did not choose, under a rule they approved
/// for a different number.
const TES_PAID_MIN_MINOR_UNITS: i64 = 100;
const TES_PAID_MAX_MINOR_UNITS: i64 = 30_000;

/// The Tes licence values a *paid* resource may carry, and the ones a *free*
/// resource may carry. Both are subsets of the captured `licence` vocabulary
/// in `crate::registry`, split by the gate the reference data records: "a
/// required `licence` from five writable values, gated by price so that a free
/// resource picks among CC-BY, CC-BY-SA and CC-BY-ND and a paid one takes
/// TES-PAID or the TES-PAID-SCHOOL" (same capture).
///
/// The school tier is nonetheless absent from the paid list, because this
/// system's own write boundary cannot carry it: the Tes adapter's
/// `paid_price_token` admits `TES-PAID` alone and its `pricing_from_token`
/// cannot decode the school tier, so a rule proposing it would be previewed,
/// approved and frozen only to be refused after the terms were fixed. It is
/// refused here instead, and nothing is substituted for it — a rule that
/// quietly turned the school tier into the individual licence would make the
/// seller's grant for them. Admitting it again is a change to the Tes
/// adapter's writable pricing model, and this list follows that change rather
/// than anticipating it.
///
/// The two legacy values the registry also holds, `TES-V1` and `TES-V2`, are
/// out for a neighbouring reason: the Tes editor rewrites them to `CC-BY-SA`
/// on load and offers neither on write, so proposing one is proposing a grant
/// the platform will not accept.
const TES_LICENCE_PAID: [&str; 1] = ["TES-PAID"];
const TES_LICENCE_FREE: [&str; 3] = ["CC-BY", "CC-BY-ND", "CC-BY-SA"];

// ------------------------------------------------------------ vocabulary

/// Which half of the feature a rule belongs to. Derived from the action
/// rather than authored beside it, so a pricing action cannot be filed under
/// `mapping` in a list filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Pricing,
    Mapping,
}

/// A use a seller can opt a definition into applying automatically. Three
/// separate opt-ins because they are three different risks: a copy creates a
/// listing, a move retires the source, a cross-list keeps both live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleUse {
    Copy,
    Move,
    CrossList,
}

impl RuleUse {
    /// The closed set, in a stable order.
    pub const ALL: [Self; 3] = [Self::Copy, Self::Move, Self::CrossList];
}

/// Which source prices a rule is about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingCondition {
    #[default]
    Any,
    Free,
    Paid,
}

/// Whether a multi-valued constraint needs every value or any one of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionMode {
    #[default]
    All,
    Any,
}

/// A constraint on one source axis.
///
/// An axis this model retains no source declaration for is refused by
/// [`validate_definition`] rather than matching nothing quietly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeCondition {
    pub axis: TermKind,
    /// For [`TermKind::Subject`] these are canonical term identifiers: a
    /// canonical product holds subjects as `CanonicalTermId`s, and source
    /// subject tokens survive only where the import left them in
    /// `native_residue`. Both are matched; a caller must not present this as
    /// a source-native picker.
    pub values: Vec<String>,
    pub mode: ConditionMode,
}

/// What a rule matches. Every dimension left empty matches everything, and
/// the dimensions combine with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleConditions {
    #[serde(default)]
    pub pricing: PricingCondition,
    /// Source resource types, as the source's own native ids or captured
    /// labels. Any one of them matches.
    #[serde(default)]
    pub resource_types: Vec<String>,
    /// Literal, case-insensitive text sought in the *visible* description.
    /// Markup is removed before matching (see [`visible_description`]), so a
    /// keyword never matches an HTML tag name or an attribute value.
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub keyword_mode: ConditionMode,
    #[serde(default)]
    pub attributes: Vec<AttributeCondition>,
}

/// What a rule does to the target listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleAction {
    Pricing {
        /// A positive decimal with at most six fraction digits, parsed by
        /// [`parse_rate`]. Held as the seller typed it so the rule reads back
        /// as it was authored.
        rate: String,
        rounding: Rounding,
        /// A server-stored official quote this rate was taken from, where one
        /// was. Never a provider claim authored by a client: the identifier
        /// addresses a row we fetched and stored, and a rate that disagrees
        /// with that row is refused where the row is read.
        reference: Option<Uuid>,
    },
    Mapping {
        licence: Option<String>,
        resource_type: Option<String>,
    },
}

impl RuleAction {
    #[must_use]
    pub const fn kind(&self) -> RuleKind {
        match self {
            Self::Pricing { .. } => RuleKind::Pricing,
            Self::Mapping { .. } => RuleKind::Mapping,
        }
    }
}

/// A rule as the seller authored it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SellerRuleDefinition {
    pub title: String,
    pub description: String,
    pub enabled: bool,
    pub source: InventoryId,
    pub target: InventoryId,
    /// The uses this definition is opted into. Empty means suggestion-only:
    /// it appears in a preview the seller asks for and never applies itself.
    #[serde(default)]
    pub auto_apply: Vec<RuleUse>,
    #[serde(default)]
    pub conditions: RuleConditions,
    pub action: RuleAction,
}

impl SellerRuleDefinition {
    #[must_use]
    pub const fn kind(&self) -> RuleKind {
        self.action.kind()
    }

    /// Whether the seller opted this definition into automatic application
    /// for one use.
    #[must_use]
    pub fn auto_applies_to(&self, use_: RuleUse) -> bool {
        self.enabled && self.auto_apply.contains(&use_)
    }
}

/// A stored rule: the definition, who wrote it, and the revision a stale
/// write is detected against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SellerRuleRecord {
    pub id: Uuid,
    pub revision: i64,
    pub definition: SellerRuleDefinition,
    pub author: UserId,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// The provenance of one proposal: which rule, at which revision, and the
/// words the seller gave it.
///
/// The words are copied rather than referenced: why a queued item carries the
/// price it carries must still read correctly after the rule is retitled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleMatch {
    pub id: Uuid,
    pub revision: i64,
    pub title: String,
    pub description: String,
}

impl RuleMatch {
    #[must_use]
    pub fn of(record: &SellerRuleRecord) -> Self {
        Self {
            id: record.id,
            revision: record.revision,
            title: record.definition.title.clone(),
            description: record.definition.description.clone(),
        }
    }
}

/// What a target listing should carry, per field.
///
/// Every field is optional and `None` means unchanged. That is what makes a
/// mapping decision approvable before a pricing one — a Tes licence can be
/// accepted on a paid USD resource with no GBP price yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetFields {
    pub price: Option<PriceIntent>,
    pub licence: Option<String>,
    pub resource_type: Option<String>,
}

impl TargetFields {
    /// Whether this proposes nothing at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.price.is_none() && self.licence.is_none() && self.resource_type.is_none()
    }

    /// This proposal laid over `base`, field by field, because a price
    /// approval and a licence approval are separate decisions and replacing
    /// the record would drop whichever was approved first.
    #[must_use]
    pub fn merge_over(&self, base: &Self) -> Self {
        Self {
            price: self.price.or(base.price),
            licence: self.licence.clone().or_else(|| base.licence.clone()),
            resource_type: self
                .resource_type
                .clone()
                .or_else(|| base.resource_type.clone()),
        }
    }
}

/// A native target term a person chose, with the person recorded.
///
/// The author is part of the value: a licence with no identified chooser is
/// not a grant, and this makes that state unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SellerTermChoice {
    pub native_id: String,
    pub author: UserId,
}

/// The exact terms one queued item was enqueued under.
///
/// `price` is not optional. Every newly enqueued non-remove item gets a price,
/// because "the price this item will post" is a question with an answer at
/// enqueue time and an option here would make it a question answered later, by
/// whatever the rules said then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenRuleOutput {
    pub price: PriceIntent,
    pub licence: Option<SellerTermChoice>,
    pub resource_type: Option<SellerTermChoice>,
    pub matches: Vec<RuleMatch>,
}

impl FrozenRuleOutput {
    /// The approved fields frozen against the canonical price.
    ///
    /// `canonical` is used where no target price was approved, which is the
    /// same-inventory and mapping-only cases: the item posts the price the
    /// product already has rather than none.
    #[must_use]
    pub fn freeze(
        fields: &TargetFields,
        canonical: PriceIntent,
        author: UserId,
        matches: Vec<RuleMatch>,
    ) -> Self {
        let choice = |native_id: &String| SellerTermChoice {
            native_id: native_id.clone(),
            author,
        };
        Self {
            price: fields.price.unwrap_or(canonical),
            licence: fields.licence.as_ref().map(choice),
            resource_type: fields.resource_type.as_ref().map(choice),
            matches,
        }
    }

    /// A deterministic rendering of the semantic output, for mixing into an
    /// item idempotency key. Owned here so there is one answer to what "the
    /// same output" means. `matches` is excluded deliberately: retitling a
    /// rule changes nothing that will be posted, so it must not rekey an
    /// unchanged item.
    #[must_use]
    pub fn digest_material(&self) -> String {
        let mut out = String::with_capacity(64);
        out.push_str("price=");
        match self.price {
            PriceIntent::Free => out.push_str("free"),
            PriceIntent::Paid(money) => {
                out.push_str("paid:");
                out.push_str(&money.minor_units().to_string());
                out.push(':');
                out.push_str(money.currency().code());
            }
        }
        for (tag, choice) in [
            ("licence", self.licence.as_ref()),
            ("resource_type", self.resource_type.as_ref()),
        ] {
            out.push('\n');
            out.push_str(tag);
            out.push('=');
            match choice {
                None => out.push('-'),
                Some(choice) => out.push_str(&escape_token(&choice.native_id)),
            }
        }
        out
    }
}

/// Backslash and newline escaped, so no native id can forge a field boundary
/// in [`FrozenRuleOutput::digest_material`].
fn escape_token(raw: &str) -> Cow<'_, str> {
    if raw.bytes().any(|byte| matches!(byte, b'\\' | b'\n')) {
        Cow::Owned(raw.replace('\\', "\\\\").replace('\n', "\\n"))
    } else {
        Cow::Borrowed(raw)
    }
}

/// What the evaluator concluded for one product against one direction.
///
/// `fields` is a patch and not a picture: it holds the fields this decision
/// *changes*, so a field the target already carried is absent however
/// squarely a rule agreed with it. A caller wanting the whole target reads
/// [`TargetFields::merge_over`] against its own base, and a caller recording
/// who granted what reads this.
///
/// `blockers` is never traded for a fallback. A matching rule that cannot be
/// applied — an unparseable rate, a licence the target has no field for, a
/// conversion outside the target's measured range — produces a sentence here
/// and leaves the field unset, so the preview shows a refusal rather than a
/// number nobody chose.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleEvaluation {
    pub fields: TargetFields,
    pub matches: Vec<RuleMatch>,
    pub blockers: Vec<String>,
}

/// Explicit per-field resolutions, and the fields a decision already taken
/// has settled.
///
/// The three authored fields are one seller answering one disagreement the
/// rules could not, which is what makes them different from a rule: they
/// resolve one field once rather than standing for every future product, and
/// they stay visible in the preview.
///
/// [`Self::price`] is not one of those. It carries a price that has already
/// been approved, which is a different kind of fact: it is not previewed, not
/// authored here and not accepted again, so it never crosses the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleOverrides {
    /// A one-off rate, applied at nearest-penny rounding: a charm ending is a
    /// standing policy, not what a seller resolving one conflict asked for.
    pub rate: Option<String>,
    /// The price this evaluation is not free to recompute, because a seller
    /// already approved it.
    ///
    /// It settles the price field outright, ahead of every automatic
    /// proposal: their disagreements and their conversion failures are
    /// answers to a question that is no longer open, and reporting them would
    /// block a licence the same evaluation resolved perfectly well. It wins
    /// over [`Self::rate`] for the same reason — a decision already taken
    /// outranks one being previewed.
    ///
    /// Never serialised. A request that could name the approved price could
    /// name any price and have the rules decline to argue with it.
    #[serde(skip)]
    pub price: Option<PriceIntent>,
    pub licence: Option<String>,
    pub resource_type: Option<String>,
}

impl RuleOverrides {
    /// No overrides, as a const so the automatic path can pass a reference
    /// without allocating one per product.
    pub const NONE: Self = Self {
        rate: None,
        price: None,
        licence: None,
        resource_type: None,
    };
}

/// A named, described suggestion. Listing one saves nothing: the seller edits
/// what they like and the definition becomes a rule only when they create it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulePreset {
    pub id: String,
    pub definition: SellerRuleDefinition,
    /// What the seller is being told before they accept, in full sentences.
    /// Where it quotes a marketplace's own words it says whose they are.
    pub notice: String,
    /// The documents the notice rests on.
    pub sources: Vec<String>,
}

// ------------------------------------------------------------ refusals

/// Every way a rule can be inadmissible. A value rather than a string so the
/// API can map one to a status and the evaluator can render one into a
/// blocker sentence without two vocabularies of refusal existing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    TitleBlank,
    RateBlank,
    RateMalformed {
        rate: String,
    },
    RateNotPositive {
        rate: String,
    },
    RateTooPrecise {
        digits: usize,
    },
    RateOutOfRange {
        rate: String,
    },
    SameInventory {
        inventory: InventoryId,
    },
    DuplicateAutoApply {
        duplicate: RuleUse,
    },
    KeywordBlank,
    ResourceTypeBlank,
    AttributeValuesEmpty {
        axis: TermKind,
    },
    AttributeValueBlank {
        axis: TermKind,
    },
    UnsupportedAxis {
        axis: TermKind,
    },
    SourceAxisUnclassifiable {
        source: InventoryId,
        axis: TermKind,
    },
    MappingEmpty,
    PriceCurrencyUnsupported {
        inventory: InventoryId,
    },
    SourceCurrencyMismatch {
        expected: Currency,
        found: Currency,
    },
    PaidBecameZero,
    PriceOverflow,
    PriceOutOfRange {
        minor_units: i64,
        min: i64,
        max: i64,
    },
    AxisAbsentOnTarget {
        target: InventoryId,
        axis: TermKind,
    },
    AxisUnmeasuredOnTarget {
        target: InventoryId,
        axis: TermKind,
    },
    VocabularyUnmeasured {
        target: InventoryId,
        axis: TermKind,
    },
    NotInTargetVocabulary {
        target: InventoryId,
        axis: TermKind,
        value: String,
    },
    LicenceNotWritable {
        value: String,
    },
    LicenceContradictsPrice {
        value: String,
        price_is_free: bool,
    },
}

impl core::fmt::Display for RuleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TitleBlank => f.write_str("a rule needs a title to be recognised by"),
            Self::RateBlank => f.write_str("a pricing rule needs a rate"),
            // "digits, optionally a point and digits" is the whole grammar: one
            // spelling per rate is what keeps a stored rule comparable with a
            // re-entered one.
            Self::RateMalformed { rate } => write!(f, "{rate:?} is not a decimal rate"),
            Self::RateNotPositive { rate } => write!(f, "{rate:?} is not a positive rate"),
            Self::RateTooPrecise { digits } => write!(
                f,
                "a rate carries at most {RATE_FRACTION_DIGITS_MAX} fraction digits, not {digits}"
            ),
            Self::RateOutOfRange { rate } => {
                write!(f, "{rate:?} is larger than any rate this converts at")
            }
            Self::SameInventory { inventory } => {
                write!(f, "a rule crosses two inventories, not {inventory:?} twice")
            }
            Self::DuplicateAutoApply { duplicate } => {
                write!(f, "{duplicate:?} is opted into twice")
            }
            Self::KeywordBlank => f.write_str("a blank keyword matches every description"),
            Self::ResourceTypeBlank => f.write_str("a blank resource type names nothing"),
            Self::AttributeValuesEmpty { axis } => {
                write!(f, "the {axis:?} condition lists no values")
            }
            Self::AttributeValueBlank { axis } => {
                write!(f, "the {axis:?} condition holds a blank value")
            }
            // Refused rather than admitted and matched against nothing, which
            // would read as a rule that mysteriously never fires.
            Self::UnsupportedAxis { axis } => {
                write!(f, "no source declaration is retained for {axis:?}")
            }
            // Refused for the same reason and on the same principle: the
            // source keeps values of this axis as bare identifiers, and
            // nothing measured says which of them answer it.
            Self::SourceAxisUnclassifiable { source, axis } => write!(
                f,
                "{source:?} retains no {axis:?} declaration this system can recognise"
            ),
            Self::MappingEmpty => {
                f.write_str("a mapping rule sets a licence, a resource type, or both")
            }
            Self::PriceCurrencyUnsupported { inventory } => {
                write!(f, "{inventory:?} has no single measured currency")
            }
            Self::SourceCurrencyMismatch { expected, found } => write!(
                f,
                "the source prices in {expected} and this price is in {found}",
                expected = expected.code(),
                found = found.code()
            ),
            // Free is a decision the seller makes, never an arithmetic result.
            Self::PaidBecameZero => f.write_str("the conversion takes a paid resource to nothing"),
            Self::PriceOverflow => {
                f.write_str("the conversion does not fit in the money this system holds")
            }
            // Named in full because the seller has to decide what to do about
            // it: the price is refused, never moved to the nearest allowed one.
            Self::PriceOutOfRange {
                minor_units,
                min,
                max,
            } => write!(
                f,
                "the target accepts {min} to {max} minor units and the conversion gives \
                 {minor_units}, which is refused rather than clamped"
            ),
            Self::AxisAbsentOnTarget { target, axis } => {
                write!(f, "{target:?} carries no {axis:?} field on its wire at all")
            }
            Self::AxisUnmeasuredOnTarget { target, axis } => {
                write!(f, "nothing has been measured about {axis:?} on {target:?}")
            }
            Self::VocabularyUnmeasured { target, axis } => {
                write!(f, "the {axis:?} vocabulary on {target:?} is not captured")
            }
            Self::NotInTargetVocabulary {
                target,
                axis,
                value,
            } => write!(f, "{value:?} is not a {axis:?} value {target:?} issues"),
            Self::LicenceNotWritable { value } => {
                write!(f, "{value:?} is not a licence value this system can write")
            }
            Self::LicenceContradictsPrice {
                value,
                price_is_free,
            } => {
                let price = if *price_is_free { "free" } else { "paid" };
                write!(f, "{value:?} is not a licence a {price} resource may carry")
            }
        }
    }
}

impl core::error::Error for RuleError {}

// ------------------------------------------------------------ arithmetic

/// A decimal rate string as an exact count of micros.
///
/// One spelling per rate, which is why a leading point, a leading plus, a
/// thousands separator and a seventh fraction digit are all refused rather
/// than normalised: two strings that parsed to one number would let a stored
/// rule and a re-entered rate compare unequal while meaning the same thing.
///
/// # Errors
///
/// [`RuleError::RateBlank`], [`RuleError::RateMalformed`],
/// [`RuleError::RateTooPrecise`], [`RuleError::RateNotPositive`] or
/// [`RuleError::RateOutOfRange`], each naming what was wrong with the input.
pub fn parse_rate(rate: &str) -> Result<i64, RuleError> {
    let trimmed = rate.trim();
    if trimmed.is_empty() {
        return Err(RuleError::RateBlank);
    }
    let malformed = || RuleError::RateMalformed {
        rate: trimmed.to_owned(),
    };
    let (whole, fraction) = trimmed.split_once('.').unwrap_or((trimmed, ""));
    if whole.is_empty() || !is_ascii_digits(whole) || !is_ascii_digits(fraction) {
        return Err(malformed());
    }
    if fraction.len() > RATE_FRACTION_DIGITS_MAX {
        return Err(RuleError::RateTooPrecise {
            digits: fraction.len(),
        });
    }
    let out_of_range = || RuleError::RateOutOfRange {
        rate: trimmed.to_owned(),
    };
    let units: i128 = whole.parse().map_err(|_ignored| out_of_range())?;
    let mut padded = fraction.to_owned();
    while padded.len() < RATE_FRACTION_DIGITS_MAX {
        padded.push('0');
    }
    let micros = padded.parse::<i128>().map_err(|_ignored| out_of_range())?;
    let total = units
        .checked_mul(MICROS_PER_UNIT)
        .and_then(|scaled| scaled.checked_add(micros))
        .ok_or_else(out_of_range)?;
    if total <= 0 {
        return Err(RuleError::RateNotPositive {
            rate: trimmed.to_owned(),
        });
    }
    i64::try_from(total).map_err(|_ignored| out_of_range())
}

/// Whether every byte is an ASCII digit. An empty string is vacuously true,
/// which is what the absent-fraction case wants.
fn is_ascii_digits(raw: &str) -> bool {
    raw.bytes().all(|byte| byte.is_ascii_digit())
}

/// One price converted from the source inventory's currency into the target
/// inventory's, at an exact rate.
///
/// `Free` converts to `Free` at every rate: a free resource has no amount to
/// scale, and making it paid is a decision rather than a multiplication.
///
/// # Errors
///
/// Refuses rather than approximating: a source or target inventory with no
/// single measured currency ([`RuleError::PriceCurrencyUnsupported`]), a price
/// denominated in some other currency ([`RuleError::SourceCurrencyMismatch`]),
/// a non-positive rate, a product that does not fit
/// ([`RuleError::PriceOverflow`]), a conversion that lands on nothing
/// ([`RuleError::PaidBecameZero`]) and one outside the target's measured
/// accepted range ([`RuleError::PriceOutOfRange`]).
pub fn convert_price(
    price: PriceIntent,
    source: InventoryId,
    target: InventoryId,
    rate_micros: i64,
    rounding: Rounding,
) -> Result<PriceIntent, RuleError> {
    let source_currency = fixed_currency(source)?;
    let target_currency = fixed_currency(target)?;
    if rate_micros <= 0 {
        return Err(RuleError::RateNotPositive {
            rate: rate_micros.to_string(),
        });
    }
    let money = match price {
        PriceIntent::Free => return Ok(PriceIntent::Free),
        PriceIntent::Paid(money) => money,
    };
    if money.currency() != source_currency {
        return Err(RuleError::SourceCurrencyMismatch {
            expected: source_currency,
            found: money.currency(),
        });
    }
    // Micro-minor-units: the exact product, before any rounding decision.
    let exact = i128::from(money.minor_units())
        .checked_mul(i128::from(rate_micros))
        .ok_or(RuleError::PriceOverflow)?;
    let minor = match rounding {
        Rounding::Nearest => nearest_minor_units(exact),
        Rounding::UpToCharm => charm_minor_units(exact),
    }
    .ok_or(RuleError::PriceOverflow)?;
    if minor <= 0 {
        return Err(RuleError::PaidBecameZero);
    }
    let minor = i64::try_from(minor).map_err(|_ignored| RuleError::PriceOverflow)?;
    check_target_range(target, minor)?;
    Money::new(minor, target_currency)
        .map(PriceIntent::Paid)
        .map_err(|_ignored| RuleError::PaidBecameZero)
}

/// The currency an inventory prices in, where exactly one is measured.
///
/// A seller-scoped or unmeasured currency is refused rather than guessed: a
/// conversion into an unknown denomination produces a number that looks like a
/// price and is comparable with nothing.
fn fixed_currency(inventory: InventoryId) -> Result<Currency, RuleError> {
    match inventory.currency_rule() {
        CurrencyRule::Fixed(currency) => Ok(currency),
        CurrencyRule::SellerScoped | CurrencyRule::Unmeasured => {
            Err(RuleError::PriceCurrencyUnsupported { inventory })
        }
    }
}

/// Half up to a penny. The addend is half a minor unit in micros, so a value
/// exactly on the half goes up, which is the rounding a seller reads as
/// "nearest" on a price list.
fn nearest_minor_units(exact: i128) -> Option<i128> {
    exact
        .checked_add(MICROS_PER_UNIT.div_euclid(2))
        .map(|shifted| shifted.div_euclid(MICROS_PER_UNIT))
}

/// The smallest amount ending in `.99` that is not below the exact
/// conversion.
///
/// Deliberately not nearest-then-charm. Rounding 12.004 to the nearest penny
/// gives 12.00 and charming that gives 11.99, which is *below* the converted
/// price the seller approved a rate for; computing the charm against the
/// unrounded value instead can only ever move the price up, so a charm ending
/// never quietly discounts.
fn charm_minor_units(exact: i128) -> Option<i128> {
    let whole = exact.div_euclid(MICROS_PER_UNIT);
    let ceiling = if exact.rem_euclid(MICROS_PER_UNIT) == 0 {
        whole
    } else {
        whole.checked_add(1)?
    };
    // Drop to the whole major unit below `ceiling` and add .99. That is never
    // under `ceiling`, because the part dropped is at most 99.
    ceiling
        .checked_sub(ceiling.rem_euclid(MINOR_UNITS_PER_MAJOR))
        .and_then(|major| major.checked_add(CHARM_REMAINDER))
}

/// The measured accepted paid range of the target, where one is recorded.
///
/// Only Tes has one on file here. TPT's own `min_price` was captured without a
/// currency beside it and its floor is enforced at the adapter's write model,
/// which is the layer that knows which dollar it read; restating an
/// unmeasured floor in the pure core would make a guess look like a fact.
fn check_target_range(target: InventoryId, minor_units: i64) -> Result<(), RuleError> {
    let (min, max) = match target {
        InventoryId::Tes => (TES_PAID_MIN_MINOR_UNITS, TES_PAID_MAX_MINOR_UNITS),
        InventoryId::Tpt | InventoryId::Etsy => return Ok(()),
    };
    if minor_units < min || minor_units > max {
        return Err(RuleError::PriceOutOfRange {
            minor_units,
            min,
            max,
        });
    }
    Ok(())
}

// ------------------------------------------------------------ validation

/// Whether a definition is admissible at all.
///
/// Run before a rule is stored and again by [`evaluate`] before a stored rule
/// is applied, because a rule can become inadmissible without being edited: a
/// vocabulary capture changes, an inventory's measured range changes, and a
/// rule that was valid when written must refuse rather than post whatever it
/// now means.
///
/// # Errors
///
/// A [`RuleError`] naming the first inadmissible thing found.
pub fn validate_definition(definition: &SellerRuleDefinition) -> Result<(), RuleError> {
    if definition.title.trim().is_empty() {
        return Err(RuleError::TitleBlank);
    }
    if definition.source == definition.target {
        return Err(RuleError::SameInventory {
            inventory: definition.source,
        });
    }
    for (index, use_) in definition.auto_apply.iter().enumerate() {
        if definition.auto_apply[index + 1..].contains(use_) {
            return Err(RuleError::DuplicateAutoApply { duplicate: *use_ });
        }
    }
    validate_conditions(&definition.conditions, definition.source)?;
    validate_action(definition)
}

/// Every constraint names something it could match, on a source that could
/// declare it.
fn validate_conditions(conditions: &RuleConditions, source: InventoryId) -> Result<(), RuleError> {
    if conditions
        .keywords
        .iter()
        .any(|word| word.trim().is_empty())
    {
        return Err(RuleError::KeywordBlank);
    }
    if conditions
        .resource_types
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err(RuleError::ResourceTypeBlank);
    }
    if !conditions.resource_types.is_empty() {
        source_can_declare(source, TermKind::ResourceType)?;
    }
    for condition in &conditions.attributes {
        if !axis_is_matchable(condition.axis) {
            return Err(RuleError::UnsupportedAxis {
                axis: condition.axis,
            });
        }
        if condition.values.is_empty() {
            return Err(RuleError::AttributeValuesEmpty {
                axis: condition.axis,
            });
        }
        if condition.values.iter().any(|value| value.trim().is_empty()) {
            return Err(RuleError::AttributeValueBlank {
                axis: condition.axis,
            });
        }
        source_can_declare(source, condition.axis)?;
    }
    Ok(())
}

/// The axes a canonical product actually retains a source declaration for.
///
/// `Topic` is absent because nothing in [`CanonicalProduct`] holds source
/// topics: they survive only inside a taxonomy relation this pure module has
/// no access to. A condition on an unmatchable axis is refused rather than
/// admitted and matched against nothing, which would read to the seller as a
/// rule that mysteriously never fires.
const fn axis_is_matchable(axis: TermKind) -> bool {
    match axis {
        TermKind::Subject | TermKind::ResourceType | TermKind::Phase | TermKind::Licence => true,
        TermKind::Topic => false,
    }
}

/// Whether one source's retained data can answer a condition on one axis.
///
/// Three axes live in typed fields of the canonical product — subjects in
/// `subjects`, grades in `grades`, the grant in `rights` — so where the value
/// is kept is what establishes which axis it answers. A resource type
/// survives only inside the untyped `native_residue`, so it needs a
/// vocabulary that can classify an identifier *as* a resource type, and a
/// source with none is refused here rather than admitted to match nothing.
/// That is the same discipline [`axis_is_matchable`] applies to `Topic`,
/// applied one level down: at the source rather than at the model.
fn source_can_declare(source: InventoryId, axis: TermKind) -> Result<(), RuleError> {
    let established = match axis {
        TermKind::Subject | TermKind::Phase | TermKind::Licence => true,
        TermKind::ResourceType => classified_axis_vocabulary(source, axis).is_some(),
        TermKind::Topic => false,
    };
    if established {
        Ok(())
    } else {
        Err(RuleError::SourceAxisUnclassifiable { source, axis })
    }
}

/// The captured members of one inventory's vocabulary for one axis, where
/// that vocabulary can classify a value *as* that axis.
///
/// Two conditions, and both are about evidence rather than convenience. The
/// members have to be captured, because `ClosedUncaptured` states a closed set
/// nobody here holds. And the native field has to carry this axis alone: TPT
/// binds Subject, Topic, ResourceType and Phase to one `taxonomyTags` field
/// where only the facet's own category tells them apart, a capture that lives
/// in `docs/design/data/tpt-vocabulary.json` and that this pure module has no
/// reader for. Membership of a shared namespace establishes that TPT issued
/// the slug and nothing about which axis it answers.
fn classified_axis_vocabulary(
    inventory: InventoryId,
    axis: TermKind,
) -> Option<&'static [&'static str]> {
    let entry = registry(inventory);
    let binding = entry.axis(axis)?;
    if entry
        .equivalence_axes
        .iter()
        .filter(|other| other.native == binding.native)
        .count()
        > 1
    {
        return None;
    }
    match entry.native(binding.native)?.vocabulary {
        NativeVocabulary::Closed(members) => Some(members),
        NativeVocabulary::ClosedUncaptured
        | NativeVocabulary::Free
        | NativeVocabulary::Numeric
        | NativeVocabulary::Unmeasured => None,
    }
}

/// The action's own admissibility, including whether the target can carry it.
fn validate_action(definition: &SellerRuleDefinition) -> Result<(), RuleError> {
    match &definition.action {
        RuleAction::Pricing { rate, .. } => {
            parse_rate(rate)?;
            // A rate is only meaningful between two measured currencies, and
            // refusing here is what stops a rule from being stored against a
            // direction in which it could never produce a price.
            fixed_currency(definition.source)?;
            fixed_currency(definition.target)?;
            Ok(())
        }
        RuleAction::Mapping {
            licence,
            resource_type,
        } => {
            if licence.is_none() && resource_type.is_none() {
                return Err(RuleError::MappingEmpty);
            }
            if let Some(value) = licence {
                check_native_value(definition.target, TermKind::Licence, value)?;
                check_licence_is_writable(definition.target, value)?;
            }
            if let Some(value) = resource_type {
                check_native_value(definition.target, TermKind::ResourceType, value)?;
            }
            Ok(())
        }
    }
}

/// Whether one native value is one the target inventory issues for one axis.
///
/// Reads the measured registry rather than a list restated here, so a
/// vocabulary capture is the single source. The three unmeasured answers
/// refuse: `ClosedUncaptured` admits only a value whose shape a provenance
/// test recognises, and a free, numeric or unmeasured vocabulary can check
/// nothing at all.
fn check_native_value(target: InventoryId, axis: TermKind, value: &str) -> Result<(), RuleError> {
    let inventory = registry(target);
    let Some(binding) = inventory.axis(axis) else {
        return Err(if inventory.declares_absent(axis) {
            RuleError::AxisAbsentOnTarget { target, axis }
        } else {
            RuleError::AxisUnmeasuredOnTarget { target, axis }
        });
    };
    let Some(native) = inventory.native(binding.native) else {
        return Err(RuleError::AxisUnmeasuredOnTarget { target, axis });
    };
    match native.vocabulary {
        NativeVocabulary::Closed(values) => {
            if values.contains(&value) {
                Ok(())
            } else {
                Err(RuleError::NotInTargetVocabulary {
                    target,
                    axis,
                    value: value.to_owned(),
                })
            }
        }
        // TPT's whole facet namespace arrives in one uncaptured array, and
        // `is_tpt_tag_slug` is the provenance test that already decides which
        // identifiers TPT can have issued. Reused rather than restated, and
        // left exactly as it is.
        NativeVocabulary::ClosedUncaptured if target == InventoryId::Tpt => {
            if is_tpt_tag_slug(value) {
                Ok(())
            } else {
                Err(RuleError::NotInTargetVocabulary {
                    target,
                    axis,
                    value: value.to_owned(),
                })
            }
        }
        NativeVocabulary::ClosedUncaptured
        | NativeVocabulary::Free
        | NativeVocabulary::Numeric
        | NativeVocabulary::Unmeasured => Err(RuleError::VocabularyUnmeasured { target, axis }),
    }
}

/// The legacy Tes licences read back and are never offered on write.
fn check_licence_is_writable(target: InventoryId, value: &str) -> Result<(), RuleError> {
    if target == InventoryId::Tes
        && !TES_LICENCE_PAID.contains(&value)
        && !TES_LICENCE_FREE.contains(&value)
    {
        return Err(RuleError::LicenceNotWritable {
            value: value.to_owned(),
        });
    }
    Ok(())
}

/// The target's own price gate on a licence, checked against the price the
/// target listing will carry.
///
/// Tes gates its licence vocabulary by price: a free resource picks a Creative
/// Commons value and a paid one takes the `TES-PAID` tier. A proposal on the
/// wrong side of that gate is refused here rather than posted and rejected by
/// the platform, and rather than "corrected" to the other branch — which
/// licence a resource carries is the seller's grant to make.
///
/// The price is the *target's*, not the source's, and the distinction is the
/// whole of why this takes one. A rule selects resources by what the source
/// published; the platform gates the grant by what the listing being written
/// will charge. An approved GBP price on a source that has since gone free,
/// or a conversion that makes a free resource paid, moves the second without
/// moving the first.
fn check_licence_against_price(
    target: InventoryId,
    value: &str,
    price: PriceIntent,
) -> Result<(), RuleError> {
    if target != InventoryId::Tes {
        return Ok(());
    }
    let price_is_free = price == PriceIntent::Free;
    let permitted: &[&str] = if price_is_free {
        &TES_LICENCE_FREE
    } else {
        &TES_LICENCE_PAID
    };
    if permitted.contains(&value) {
        Ok(())
    } else {
        Err(RuleError::LicenceContradictsPrice {
            value: value.to_owned(),
            price_is_free,
        })
    }
}

// ------------------------------------------------------------ evaluation

/// What every matching rule, plus any explicit override, says one product's
/// target listing should carry — as a *patch* against what it carries
/// already.
///
/// The product is read and never written. Rules whose direction is not the one
/// asked about are skipped rather than silently applied; a rule that no longer
/// validates becomes a blocker rather than a proposal; agreeing proposals
/// coalesce; and differing ones leave the field unset with a blocker naming the
/// disagreement, unless the matching override resolves that one field.
///
/// `base` is what the target already carries. It is read for two things and
/// copied into the result for none: the price a licence is judged against
/// where this evaluation proposes none, and the value a resolved field has to
/// differ from to be a change at all. A caller that records an author per
/// present field therefore never hands one seller's grant to whoever approved
/// the price.
///
/// The two sides of a rule read different prices, deliberately. Conditions
/// ask which resources a rule is about, and that is a question about the
/// source listing, so they read `product.price` and the source's own
/// declarations — unchanged, whatever any rule proposes. The licence gate asks
/// whether a grant may be made on the listing being written, so it reads the
/// price that listing will carry: this evaluation's own price, else the
/// approved one, else the source's.
#[must_use]
pub fn evaluate(
    product: &CanonicalProduct,
    direction: (InventoryId, InventoryId),
    rules: &[SellerRuleRecord],
    overrides: &RuleOverrides,
    base: &TargetFields,
) -> RuleEvaluation {
    let (source, target) = direction;
    // Once per product rather than once per rule: markup removal and case
    // folding are the same answer for every keyword.
    let haystack = visible_description(&product.body).to_lowercase();
    let mut evaluation = RuleEvaluation::default();
    let mut proposals = Proposals::default();
    for record in rules {
        let definition = &record.definition;
        if definition.source != source || definition.target != target {
            continue;
        }
        if let Err(error) = validate_definition(definition) {
            let title = &definition.title;
            evaluation
                .blockers
                .push(format!("rule {title:?} cannot be applied: {error}"));
            continue;
        }
        if !conditions_match(&definition.conditions, product, source, &haystack) {
            continue;
        }
        evaluation.matches.push(RuleMatch::of(record));
        collect_proposals(record, product, direction, &mut proposals);
    }
    // The price is settled first because the licence gate reads it, and it is
    // read here rather than by the caller's later merge: a gate applied
    // before the merge is a gate applied to a price that is not the one the
    // listing will carry.
    let price = resolve_price(
        &proposals.prices,
        overrides,
        product,
        direction,
        &mut evaluation.blockers,
    );
    let target_price = (target, price.or(base.price).unwrap_or(product.price));
    let licence = resolve_term(
        TermKind::Licence,
        &proposals.licences,
        overrides.licence.as_deref(),
        target_price,
        &mut evaluation.blockers,
    );
    let resource_type = resolve_term(
        TermKind::ResourceType,
        &proposals.resource_types,
        overrides.resource_type.as_deref(),
        target_price,
        &mut evaluation.blockers,
    );
    if licence.is_none() {
        check_inherited_licence(base, target_price, &mut evaluation.blockers);
    }
    evaluation.fields = TargetFields {
        price,
        licence,
        resource_type,
    };
    evaluation
}

/// The licence the target already carries, re-judged against the price this
/// evaluation lands on.
///
/// A pricing rule can move a listing across the gate its licence was approved
/// under: a conversion that makes a free resource paid leaves a Creative
/// Commons grant on a resource Tes will sell, and an approved GBP price on a
/// source since gone free leaves TES-PAID on one it will give away. Neither is
/// a field this evaluation proposes, so neither is reached by the resolution
/// above, and without this the adapter is the first thing to notice — after
/// the terms are frozen.
fn check_inherited_licence(
    base: &TargetFields,
    target_price: (InventoryId, PriceIntent),
    blockers: &mut Vec<String>,
) {
    let (target, price) = target_price;
    let Some(value) = base.licence.as_deref() else {
        return;
    };
    if let Err(error) = checked_term(TermKind::Licence, target, value, price) {
        blockers.push(format!(
            "the approved licence {value:?} does not fit the price this listing would carry: \
             {error}"
        ));
    }
}

/// The per-field candidate lists one pass over the rules fills.
///
/// A price candidate keeps its conversion *result* rather than only its
/// successes, and a licence candidate is ungated at this point. Both are
/// deferred for the same reason: whether a rule's refusal matters is decided
/// by the resolution, which is the only thing that knows whether the field was
/// answered another way and what price the gate should read.
#[derive(Default)]
struct Proposals {
    prices: Vec<Proposal<Result<PriceIntent, RuleError>>>,
    licences: Vec<Proposal<String>>,
    resource_types: Vec<Proposal<String>>,
}

/// One rule's proposal for one field, with the rule's own title kept beside it
/// so a refusal or a disagreement names which rule said what.
struct Proposal<Value> {
    title: String,
    value: Value,
}

/// One matching rule's proposals, unresolved and ungated.
fn collect_proposals(
    record: &SellerRuleRecord,
    product: &CanonicalProduct,
    direction: (InventoryId, InventoryId),
    into: &mut Proposals,
) {
    let (source, target) = direction;
    let definition = &record.definition;
    let title = definition.title.clone();
    match &definition.action {
        RuleAction::Pricing { rate, rounding, .. } => into.prices.push(Proposal {
            title,
            value: parse_rate(rate)
                .and_then(|micros| convert_price(product.price, source, target, micros, *rounding)),
        }),
        RuleAction::Mapping {
            licence,
            resource_type,
        } => {
            if let Some(value) = licence {
                into.licences.push(Proposal {
                    title: title.clone(),
                    value: value.clone(),
                });
            }
            if let Some(value) = resource_type {
                into.resource_types.push(Proposal {
                    title,
                    value: value.clone(),
                });
            }
        }
    }
}

/// The price the proposals, the approved value and any override settle on.
///
/// An approved price returns before a single rule's arithmetic is read: it is
/// the field's answer, and a conflict or a conversion failure about a settled
/// field is noise that would block the fields beside it. A one-off rate wins
/// over the rules for the neighbouring reason — it is the seller answering the
/// disagreement the rules could not — and is applied at nearest-penny
/// rounding, with its own failure a blocker rather than a fallback.
fn resolve_price(
    prices: &[Proposal<Result<PriceIntent, RuleError>>],
    overrides: &RuleOverrides,
    product: &CanonicalProduct,
    direction: (InventoryId, InventoryId),
    blockers: &mut Vec<String>,
) -> Option<PriceIntent> {
    let (source, target) = direction;
    if let Some(approved) = overrides.price {
        return Some(approved);
    }
    if let Some(rate) = overrides.rate.as_deref() {
        return match parse_rate(rate).and_then(|micros| {
            convert_price(product.price, source, target, micros, Rounding::Nearest)
        }) {
            Ok(price) => Some(price),
            Err(error) => {
                blockers.push(format!(
                    "the one-off rate {rate:?} cannot price this resource: {error}"
                ));
                None
            }
        };
    }
    let mut converted: Vec<PriceIntent> = Vec::with_capacity(prices.len());
    for proposal in prices {
        match &proposal.value {
            Ok(price) => converted.push(*price),
            Err(error) => {
                let title = &proposal.title;
                blockers.push(format!(
                    "rule {title:?} cannot price this resource: {error}"
                ));
            }
        }
    }
    let first = *converted.first()?;
    if converted.iter().all(|price| *price == first) {
        return Some(first);
    }
    blockers.push(format!(
        "matching rules propose different target prices ({}); approve one with an explicit rate",
        rendered_prices(&converted)
    ));
    None
}

/// The distinct proposals, in first-seen order, for a blocker sentence that
/// names what disagreed rather than only that something did.
fn rendered_prices(prices: &[PriceIntent]) -> String {
    let mut seen: Vec<String> = Vec::new();
    for price in prices {
        let rendered = render_price(*price);
        if !seen.contains(&rendered) {
            seen.push(rendered);
        }
    }
    seen.join(", ")
}

/// A price as a seller reads it, for blocker prose only.
fn render_price(price: PriceIntent) -> String {
    match price {
        PriceIntent::Free => "free".to_owned(),
        PriceIntent::Paid(money) => {
            let minor = money.minor_units();
            format!(
                "{} {}.{:02}",
                money.currency().code(),
                minor.div_euclid(100),
                minor.rem_euclid(100)
            )
        }
    }
}

/// The native term the proposals and any override settle on, each candidate
/// checked against the target and the target's price before it counts.
///
/// A candidate the target refuses is a blocker naming its rule and is not a
/// party to the disagreement: two rules do not conflict over a value only one
/// of them could have set.
fn resolve_term(
    axis: TermKind,
    candidates: &[Proposal<String>],
    override_value: Option<&str>,
    target_price: (InventoryId, PriceIntent),
    blockers: &mut Vec<String>,
) -> Option<String> {
    let (target, price) = target_price;
    let noun = axis_noun(axis);
    if let Some(value) = override_value {
        return match checked_term(axis, target, value, price) {
            Ok(()) => Some(value.to_owned()),
            Err(error) => {
                blockers.push(format!(
                    "the one-off {noun} choice {value:?} is refused: {error}"
                ));
                None
            }
        };
    }
    let mut admissible: Vec<&str> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        match checked_term(axis, target, &candidate.value, price) {
            Ok(()) => admissible.push(candidate.value.as_str()),
            Err(error) => {
                let title = &candidate.title;
                blockers.push(format!("rule {title:?} cannot set a {noun} here: {error}"));
            }
        }
    }
    let first = *admissible.first()?;
    if admissible.iter().all(|value| *value == first) {
        return Some(first.to_owned());
    }
    let mut distinct: Vec<&str> = Vec::new();
    for value in &admissible {
        if !distinct.contains(value) {
            distinct.push(value);
        }
    }
    blockers.push(format!(
        "matching rules propose different {noun} values ({}); choose one explicitly",
        distinct.join(", ")
    ));
    None
}

/// The axis as a sentence reads it, so a blocker is prose rather than the
/// debug rendering of an enum.
const fn axis_noun(axis: TermKind) -> &'static str {
    match axis {
        TermKind::Licence => "licence",
        TermKind::ResourceType => "resource type",
        TermKind::Subject => "subject",
        TermKind::Topic => "topic",
        TermKind::Phase => "phase",
    }
}

/// Everything a target term has to clear: the target's vocabulary, the
/// writability of a licence, and the target's price gate.
fn checked_term(
    axis: TermKind,
    target: InventoryId,
    value: &str,
    price: PriceIntent,
) -> Result<(), RuleError> {
    check_native_value(target, axis, value)?;
    if axis == TermKind::Licence {
        check_licence_is_writable(target, value)?;
        check_licence_against_price(target, value, price)?;
    }
    Ok(())
}

// ------------------------------------------------------------ matching

/// Whether every dimension of a condition set holds for this product.
fn conditions_match(
    conditions: &RuleConditions,
    product: &CanonicalProduct,
    source: InventoryId,
    folded_description: &str,
) -> bool {
    pricing_matches(conditions.pricing, product.price)
        && (conditions.resource_types.is_empty()
            || conditions
                .resource_types
                .iter()
                .any(|value| source_declares(product, source, TermKind::ResourceType, value)))
        && keywords_match(
            &conditions.keywords,
            conditions.keyword_mode,
            folded_description,
        )
        && conditions
            .attributes
            .iter()
            .all(|condition| attribute_matches(condition, product, source))
}

/// The canonical source price decides this, never a converted one: a rule that
/// selects free resources is asking about what the seller published on the
/// source, and matching against a price some other rule proposed would make
/// the set of matching rules depend on the order they were evaluated in.
const fn pricing_matches(condition: PricingCondition, price: PriceIntent) -> bool {
    match condition {
        PricingCondition::Any => true,
        PricingCondition::Free => matches!(price, PriceIntent::Free),
        PricingCondition::Paid => matches!(price, PriceIntent::Paid(_)),
    }
}

fn keywords_match(keywords: &[String], mode: ConditionMode, folded_description: &str) -> bool {
    if keywords.is_empty() {
        return true;
    }
    let mut hit = |keyword: &String| folded_description.contains(&keyword.to_lowercase());
    match mode {
        ConditionMode::All => keywords.iter().all(&mut hit),
        ConditionMode::Any => keywords.iter().any(&mut hit),
    }
}

fn attribute_matches(
    condition: &AttributeCondition,
    product: &CanonicalProduct,
    source: InventoryId,
) -> bool {
    let mut hit = |value: &String| source_declares(product, source, condition.axis, value);
    match condition.mode {
        ConditionMode::All => condition.values.iter().all(&mut hit),
        ConditionMode::Any => condition.values.iter().any(&mut hit),
    }
}

/// Whether the source declared one value on one axis, reading only what a
/// canonical product actually retains.
///
/// Each axis has its own home and they are not interchangeable. Subjects are
/// canonical term identifiers, so a subject condition matches a canonical id
/// and also any source subject the import left in `native_residue`. Grades
/// live in the seller's own `GradeDeclaration`, kept verbatim. A licence lives
/// in `RightsDeclaration`, and `Unstated` matches nothing rather than matching
/// a plausible default. Resource types exist only where the import kept them
/// in the residue, which is where TPT's flat facet array lands.
fn source_declares(
    product: &CanonicalProduct,
    source: InventoryId,
    axis: TermKind,
    value: &str,
) -> bool {
    let in_residue = product
        .native_residue
        .iter()
        .filter(|term| term.inventory == source && residue_carries_axis(source, axis, term))
        .any(|term| residue_names(term, value));
    if in_residue {
        return true;
    }
    match axis {
        TermKind::Subject => product
            .subjects
            .iter()
            .any(|subject| subject.0.to_hyphenated().eq_ignore_ascii_case(value)),
        TermKind::Phase => product
            .grades
            .raw
            .iter()
            .filter(|path| path.vocabulary.0 == source)
            .any(|path| path_names(path, value)),
        TermKind::Licence => match &product.rights {
            RightsDeclaration::Unstated => false,
            RightsDeclaration::Declared { source: path } => {
                path.vocabulary.0 == source && path_names(path, value)
            }
        },
        TermKind::ResourceType | TermKind::Topic => false,
    }
}

/// Whether one retained residue term is a value of one axis.
///
/// What ingestion keeps is deliberately untyped: `tam_import::residue_of`
/// retains exactly the terms a read left without a kind, and TPT's adapter
/// emits its whole flat taxonomy that way. So a term that states its own kind
/// is the easy case and the real one is a bare identifier, where membership of
/// a captured vocabulary this inventory binds to this axis alone
/// ([`classified_axis_vocabulary`]) is the only thing that classifies it.
///
/// An identifier nothing classifies matches no axis rather than every one.
/// TPT files a grade, a subject, a resource type and a file format in one
/// namespace, so letting an unclassified slug answer a resource-type condition
/// would fire the rule on `pdf`.
fn residue_carries_axis(source: InventoryId, axis: TermKind, term: &ImportedTerm) -> bool {
    match term.kind {
        Some(kind) => kind == axis,
        None => classified_axis_vocabulary(source, axis).is_some_and(|members| {
            term.native_id
                .as_deref()
                .is_some_and(|native| members.contains(&native))
        }),
    }
}

/// A native identifier matches exactly; a label matches without regard to
/// ASCII case, because a label is words a person reads and an identifier is a
/// token a platform issued.
fn residue_names(term: &ImportedTerm, value: &str) -> bool {
    term.native_id.as_deref() == Some(value)
        || term
            .segments
            .iter()
            .any(|segment| segment.eq_ignore_ascii_case(value))
}

fn path_names(path: &VocabularyPath, value: &str) -> bool {
    path.native_id.as_deref() == Some(value)
        || path
            .segments
            .iter()
            .any(|segment| segment.eq_ignore_ascii_case(value))
}

/// The words a buyer would read in a description, with markup removed.
///
/// A source body may be HTML (TPT's is), and matching against the raw bytes
/// would match tag names and attribute values as though the seller had
/// written them. Tags become a space so adjacent words do not fuse, and the
/// named and numeric entities are decoded. A Markdown body is borrowed, so
/// the common case allocates nothing. Not a sanitiser and not a renderer.
#[must_use]
pub fn visible_description(copy: &ListingCopy) -> Cow<'_, str> {
    match copy.format {
        CopyFormat::Markdown => Cow::Borrowed(copy.body.as_str()),
        CopyFormat::Html => Cow::Owned(decode_entities(&strip_tags(&copy.body))),
    }
}

/// Everything between `<` and the next `>` replaced by one space. An unclosed
/// `<` swallows the rest, which is the fail-closed direction: text inside a
/// truncated tag was never visible either.
fn strip_tags(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut inside = false;
    for character in body.chars() {
        match character {
            '<' => {
                inside = true;
                out.push(' ');
            }
            '>' if inside => inside = false,
            _ if inside => {}
            _ => out.push(character),
        }
    }
    out
}

/// The named entities a description actually carries, plus numeric
/// references. An unrecognised entity is left verbatim rather than dropped: a
/// keyword match on text we could not decode is better than a silent hole.
fn decode_entities(text: &str) -> String {
    const NAMED: [(&str, char); 6] = [
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("apos", '\''),
        ("nbsp", ' '),
    ];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        out.push_str(before);
        let entity = after
            .bytes()
            .take(9)
            .position(|byte| byte == b';')
            .and_then(|end| after.split_at_checked(end + 1))
            .and_then(|(encoded, remaining)| {
                encoded.strip_suffix(';').map(|body| (body, remaining))
            });
        let Some((body, remaining)) = entity else {
            out.push('&');
            rest = after;
            continue;
        };
        rest = remaining;
        if let Some((_name, decoded)) = NAMED.iter().find(|(name, _)| *name == body) {
            out.push(*decoded);
        } else if let Some(decoded) = numeric_entity(body) {
            out.push(decoded);
        } else {
            out.push('&');
            out.push_str(body);
            out.push(';');
        }
    }
    out.push_str(rest);
    out
}

/// A decimal or hexadecimal numeric character reference, or nothing.
fn numeric_entity(body: &str) -> Option<char> {
    let digits = body.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    char::from_u32(code)
}

// ------------------------------------------------------------ presets

/// Editable suggestions with no automatic-use opt-ins.
///
/// No resource-type preset is offered: TPT's source types are uncaptured, so
/// there is no measured correspondence to the writable Tes types.
#[must_use]
pub fn presets() -> Vec<RulePreset> {
    vec![
        manual_rate_preset(),
        paid_licence_preset(),
        free_licence_preset(),
    ]
}

/// The URLs the notices rest on, as the assessment at
/// `docs/notes/legal/marketplace-terms-assessment.md` records them.
const TES_LICENCE_SUMMARY_URL: &str =
    "https://www.tes.com/en-gb/policies/summary-teaching-resources-licence";
const TES_SELLING_FAQ_URL: &str = "https://www.tes.com/en-au/policies/help/selling-resources-faq";
const TPT_TERMS_URL: &str = "https://www.teacherspayteachers.com/Terms-of-Service";
const ECB_REFERENCE_RATES_URL: &str = "https://www.ecb.europa.eu/stats/policy_and_exchange_rates/euro_reference_exchange_rates/html/index.en.html";

fn manual_rate_preset() -> RulePreset {
    RulePreset {
        id: "tpt-to-tes-manual-usd-gbp-0-75".to_owned(),
        definition: SellerRuleDefinition {
            title: "TPT to Tes at a manual 0.75 estimate".to_owned(),
            description: "Multiply the USD price by 0.75 and round to the nearest penny."
                .to_owned(),
            enabled: true,
            source: InventoryId::Tpt,
            target: InventoryId::Tes,
            auto_apply: vec![],
            conditions: RuleConditions {
                pricing: PricingCondition::Paid,
                ..RuleConditions::default()
            },
            action: RuleAction::Pricing {
                rate: "0.75".to_owned(),
                rounding: Rounding::Nearest,
                reference: None,
            },
        },
        notice: "0.75 is a round manual estimate, not a quoted exchange rate: it is here so a \
                 seller can start without waiting on a quote, and it will drift from the market. \
                 For a dated official figure, take the European Central Bank reference rate this \
                 system fetches and stores, which records the day it was published. Two \
                 marketplace facts bound what a converted price may be. Tes sets \"The sale value \
                 of your Premium Content shall be set by you at the point of upload, subject to a \
                 minimum and maximum price\" (Tes Additional Terms), and the captured Tes \
                 reference data puts that range at GBP 1.00 to GBP 300.00, so a conversion \
                 outside it is refused rather than moved to the nearest allowed price. TPT \
                 states \"You may not charge more on TPT for a resource that is offered for free \
                 or less elsewhere\", so converting downwards can put the TPT price above the Tes \
                 one; check the result against that rule before you approve it."
            .to_owned(),
        sources: vec![
            ECB_REFERENCE_RATES_URL.to_owned(),
            TES_SELLING_FAQ_URL.to_owned(),
            TPT_TERMS_URL.to_owned(),
        ],
    }
}

fn paid_licence_preset() -> RulePreset {
    RulePreset {
        id: "tpt-to-tes-paid-tes-paid".to_owned(),
        definition: SellerRuleDefinition {
            title: "Paid TPT resources take the Tes paid licence".to_owned(),
            description: "Set the Tes licence to TES-PAID on resources the source sells."
                .to_owned(),
            enabled: true,
            source: InventoryId::Tpt,
            target: InventoryId::Tes,
            auto_apply: vec![],
            conditions: RuleConditions {
                pricing: PricingCondition::Paid,
                ..RuleConditions::default()
            },
            action: RuleAction::Mapping {
                licence: Some("TES-PAID".to_owned()),
                resource_type: None,
            },
        },
        notice: "TES-PAID is the individual teaching licence Tes requires on a resource it \
                 sells; its own summary describes the grant you make as \"a non-exclusive, \
                 sub-licensable, fully paid-up, royalty-bearing (in accordance with the clauses \
                 below), worldwide, perpetual and irrevocable (save as set out herein) licence\", \
                 against the free-content grant which is \"non-exclusive, sub-licensable, \
                 worldwide, fully paid-up, royalty-free, perpetual and irrevocable (save as set \
                 out below)\" — the difference is the royalty, and it is why the paid and free \
                 licence choices are not interchangeable. TPT publishes no licence field at all, \
                 so nothing here is a translation of a TPT grant and no equivalence between the \
                 two platforms' terms is claimed; this is a new grant you are making on Tes. \
                 Read the summary before accepting. TES-PAID is the only paid Tes licence this \
                 system can write: the school tier reads back on existing resources and the Tes \
                 write path cannot post it, so a rule proposing it is refused rather than \
                 quietly turned into this one."
            .to_owned(),
        sources: vec![
            TES_LICENCE_SUMMARY_URL.to_owned(),
            TES_SELLING_FAQ_URL.to_owned(),
        ],
    }
}

fn free_licence_preset() -> RulePreset {
    RulePreset {
        id: "tpt-to-tes-free-cc-by-nd".to_owned(),
        definition: SellerRuleDefinition {
            title: "Free TPT resources take CC-BY-ND on Tes".to_owned(),
            description: "Set the Tes licence to CC-BY-ND on resources the source gives away."
                .to_owned(),
            enabled: true,
            source: InventoryId::Tpt,
            target: InventoryId::Tes,
            auto_apply: vec![],
            conditions: RuleConditions {
                pricing: PricingCondition::Free,
                ..RuleConditions::default()
            },
            action: RuleAction::Mapping {
                licence: Some("CC-BY-ND".to_owned()),
                resource_type: None,
            },
        },
        notice: "Tes requires a Creative Commons licence for free resources. CC-BY-ND is the \
                 closest writable suggestion, not an equivalent to TPT's terms: it permits \
                 commercial redistribution of unchanged copies with attribution. Those \
                 permissions cannot be revoked while the licence conditions are met. CC-BY and \
                 CC-BY-SA also permit sharing adaptations. Do not approve this preset unless \
                 you intend that wider grant."
            .to_owned(),
        sources: vec![
            TES_LICENCE_SUMMARY_URL.to_owned(),
            TES_SELLING_FAQ_URL.to_owned(),
            "https://creativecommons.org/licenses/by-nd/4.0/".to_owned(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        convert_price, evaluate, parse_rate, presets, validate_definition, visible_description,
        AttributeCondition, ConditionMode, FrozenRuleOutput, PricingCondition, RuleAction,
        RuleConditions, RuleError, RuleMatch, RuleOverrides, RuleUse, SellerRuleDefinition,
        SellerRuleRecord, TargetFields,
    };
    use crate::{CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration};
    use tam_types::{
        CanonicalTermId, CopyFormat, Currency, ImportedTerm, InventoryId, ListingCopy, Money,
        OrgId, PriceIntent, ProductId, Rounding, TermKind, Timestamp, Title, UserId, Uuid,
    };

    const T0: Timestamp = Timestamp(1_760_000_000_000);

    fn author() -> UserId {
        UserId(Uuid([0x0c; 16]))
    }

    fn paid(minor_units: i64, currency: Currency) -> PriceIntent {
        PriceIntent::Paid(Money::new(minor_units, currency).expect("a positive price"))
    }

    fn product(price: PriceIntent) -> CanonicalProduct {
        CanonicalProduct {
            id: ProductId(Uuid([0x01; 16])),
            org: OrgId(Uuid([0x02; 16])),
            title: Title("Fractions on a number line".to_owned()),
            body: ListingCopy {
                body: "Twelve task cards.".to_owned(),
                format: CopyFormat::Markdown,
            },
            payload: None,
            cover: None,
            previews: vec![],
            subjects: vec![],
            grades: GradeDeclaration {
                source: DeclarationSource::Seller,
                raw: vec![],
                derived: None,
            },
            price,
            rights: RightsDeclaration::Unstated,
            native_residue: vec![],
        }
    }

    fn definition(action: RuleAction) -> SellerRuleDefinition {
        SellerRuleDefinition {
            title: "TPT to Tes".to_owned(),
            description: "A rule under test.".to_owned(),
            enabled: true,
            source: InventoryId::Tpt,
            target: InventoryId::Tes,
            auto_apply: vec![],
            conditions: RuleConditions::default(),
            action,
        }
    }

    fn pricing(rate: &str) -> SellerRuleDefinition {
        definition(RuleAction::Pricing {
            rate: rate.to_owned(),
            rounding: Rounding::Nearest,
            reference: None,
        })
    }

    fn mapping(licence: &str) -> SellerRuleDefinition {
        definition(RuleAction::Mapping {
            licence: Some(licence.to_owned()),
            resource_type: None,
        })
    }

    fn record(byte: u8, definition: SellerRuleDefinition) -> SellerRuleRecord {
        SellerRuleRecord {
            id: Uuid([byte; 16]),
            revision: 1,
            definition,
            author: author(),
            created_at: T0,
            updated_at: T0,
        }
    }

    fn evaluated(
        price: PriceIntent,
        rules: &[SellerRuleRecord],
        overrides: &RuleOverrides,
    ) -> (TargetFields, Vec<String>, usize) {
        against(&product(price), rules, overrides, &TargetFields::default())
    }

    /// The whole call, against a product and a target that already carries
    /// something, because the base is half of what the evaluator now decides.
    fn against(
        product: &CanonicalProduct,
        rules: &[SellerRuleRecord],
        overrides: &RuleOverrides,
        base: &TargetFields,
    ) -> (TargetFields, Vec<String>, usize) {
        let evaluation = evaluate(
            product,
            (InventoryId::Tpt, InventoryId::Tes),
            rules,
            overrides,
            base,
        );
        (
            evaluation.fields,
            evaluation.blockers,
            evaluation.matches.len(),
        )
    }

    #[test]
    fn a_rate_is_exact_micros_and_a_second_spelling_of_one_is_refused() {
        assert_eq!(parse_rate("0.75"), Ok(750_000));
        assert_eq!(parse_rate(" 0.75 "), Ok(750_000));
        assert_eq!(parse_rate("1"), Ok(1_000_000));
        assert_eq!(parse_rate("0.000001"), Ok(1));
        assert_eq!(
            parse_rate("0.0000001"),
            Err(RuleError::RateTooPrecise { digits: 7 }),
            "a seventh digit cannot be held exactly, so it is refused rather than dropped"
        );
        assert_eq!(
            parse_rate("0"),
            Err(RuleError::RateNotPositive {
                rate: "0".to_owned()
            })
        );
        for spelling in [".75", "+0.75", "-0.75", "1,5", "0.7.5", "abc"] {
            assert_eq!(
                parse_rate(spelling),
                Err(RuleError::RateMalformed {
                    rate: spelling.to_owned()
                }),
                "{spelling:?} is a second spelling or no rate at all"
            );
        }
        assert_eq!(parse_rate("  "), Err(RuleError::RateBlank));
    }

    /// The founder's own worked example, and the one the API regression asserts
    /// end to end: USD 300.00 at 0.75 is GBP 225.00 exactly.
    #[test]
    fn three_hundred_dollars_at_the_manual_estimate_is_two_hundred_and_twenty_five_pounds() {
        assert_eq!(
            convert_price(
                paid(30_000, Currency::Usd),
                InventoryId::Tpt,
                InventoryId::Tes,
                750_000,
                Rounding::Nearest
            ),
            Ok(paid(22_500, Currency::Gbp))
        );
    }

    #[test]
    fn nearest_takes_a_half_penny_up_and_refuses_a_paid_resource_that_rounds_to_nothing() {
        let half = convert_price(
            paid(10, Currency::Gbp),
            InventoryId::Tes,
            InventoryId::Tpt,
            50_000,
            Rounding::Nearest,
        );
        assert_eq!(half, Ok(paid(1, Currency::Usd)), "0.5 of a penny rounds up");
        assert_eq!(
            convert_price(
                paid(9, Currency::Gbp),
                InventoryId::Tes,
                InventoryId::Tpt,
                50_000,
                Rounding::Nearest
            ),
            Err(RuleError::PaidBecameZero),
            "0.45 of a penny is not a price, and free is not an arithmetic result"
        );
    }

    #[test]
    fn free_stays_free_at_every_rate_and_rounding() {
        for rounding in [Rounding::Nearest, Rounding::UpToCharm] {
            assert_eq!(
                convert_price(
                    PriceIntent::Free,
                    InventoryId::Tpt,
                    InventoryId::Tes,
                    750_000,
                    rounding
                ),
                Ok(PriceIntent::Free)
            );
        }
    }

    /// USD 240.08 at 0.05 is GBP 12.004 exactly. Nearest gives 12.00, and
    /// charming *that* would give 11.99 — below the conversion the seller
    /// approved a rate for. The charm is computed against the unrounded value
    /// instead, so it can only move the price up.
    #[test]
    fn a_charm_ending_is_never_below_the_exact_conversion() {
        let charmed = convert_price(
            paid(24_008, Currency::Usd),
            InventoryId::Tpt,
            InventoryId::Tes,
            50_000,
            Rounding::UpToCharm,
        );
        assert_eq!(charmed, Ok(paid(1_299, Currency::Gbp)));
        assert_ne!(charmed, Ok(paid(1_199, Currency::Gbp)));
        assert_eq!(
            convert_price(
                paid(24_008, Currency::Usd),
                InventoryId::Tpt,
                InventoryId::Tes,
                50_000,
                Rounding::Nearest
            ),
            Ok(paid(1_200, Currency::Gbp))
        );
        assert_eq!(
            convert_price(
                paid(1_099, Currency::Gbp),
                InventoryId::Tes,
                InventoryId::Tpt,
                1_000_000,
                Rounding::UpToCharm
            ),
            Ok(paid(1_099, Currency::Usd)),
            "an amount already ending .99 is left alone"
        );
    }

    #[test]
    fn the_measured_tes_range_refuses_rather_than_clamping() {
        let convert = |minor_units: i64, rate: i64| {
            convert_price(
                paid(minor_units, Currency::Usd),
                InventoryId::Tpt,
                InventoryId::Tes,
                rate,
                Rounding::Nearest,
            )
        };
        assert_eq!(
            convert(200, 500_000),
            Ok(paid(100, Currency::Gbp)),
            "the floor itself is a price"
        );
        assert_eq!(
            convert(40_000, 750_000),
            Ok(paid(30_000, Currency::Gbp)),
            "the ceiling itself is a price"
        );
        assert_eq!(
            convert(132, 750_000),
            Err(RuleError::PriceOutOfRange {
                minor_units: 99,
                min: 100,
                max: 30_000
            }),
            "GBP 0.99 is refused, not raised to the floor"
        );
        assert_eq!(
            convert(40_002, 750_000),
            Err(RuleError::PriceOutOfRange {
                minor_units: 30_002,
                min: 100,
                max: 30_000
            }),
            "GBP 300.02 is refused, not lowered to the ceiling"
        );
    }

    #[test]
    fn an_overflowing_conversion_and_a_foreign_denomination_both_refuse() {
        assert_eq!(
            convert_price(
                paid(i64::MAX, Currency::Gbp),
                InventoryId::Tes,
                InventoryId::Tpt,
                2_000_000,
                Rounding::Nearest
            ),
            Err(RuleError::PriceOverflow)
        );
        assert_eq!(
            convert_price(
                paid(1_000, Currency::Gbp),
                InventoryId::Tpt,
                InventoryId::Tes,
                750_000,
                Rounding::Nearest
            ),
            Err(RuleError::SourceCurrencyMismatch {
                expected: Currency::Usd,
                found: Currency::Gbp
            }),
            "a GBP amount on a USD inventory is a mismatch, not a free conversion"
        );
        assert_eq!(
            convert_price(
                paid(1_000, Currency::Usd),
                InventoryId::Tpt,
                InventoryId::Etsy,
                750_000,
                Rounding::Nearest
            ),
            Err(RuleError::PriceCurrencyUnsupported {
                inventory: InventoryId::Etsy
            })
        );
        assert_eq!(
            convert_price(
                paid(1_000, Currency::Usd),
                InventoryId::Tpt,
                InventoryId::Tes,
                0,
                Rounding::Nearest
            ),
            Err(RuleError::RateNotPositive {
                rate: "0".to_owned()
            })
        );
    }

    #[test]
    fn agreeing_price_rules_coalesce_and_differing_ones_block() {
        let agreeing = [record(0xa1, pricing("0.75")), record(0xa2, pricing("0.75"))];
        let (fields, blockers, matched) =
            evaluated(paid(30_000, Currency::Usd), &agreeing, &RuleOverrides::NONE);
        assert_eq!(fields.price, Some(paid(22_500, Currency::Gbp)));
        assert!(
            blockers.is_empty(),
            "two rules saying the same thing do not disagree"
        );
        assert_eq!(matched, 2, "both are shown as provenance");

        let differing = [record(0xb1, pricing("0.75")), record(0xb2, pricing("0.80"))];
        let (fields, blockers, matched) = evaluated(
            paid(30_000, Currency::Usd),
            &differing,
            &RuleOverrides::NONE,
        );
        assert_eq!(
            (fields.price, matched, blockers.len()),
            (None, 2, 1),
            "a disagreement leaves the field unset rather than picking a winner"
        );
    }

    #[test]
    fn an_explicit_rate_resolves_a_price_disagreement_at_the_nearest_penny() {
        let differing = [record(0xb1, pricing("0.75")), record(0xb2, pricing("0.80"))];
        let overrides = RuleOverrides {
            rate: Some("0.5".to_owned()),
            ..RuleOverrides::default()
        };
        let (fields, blockers, matched) =
            evaluated(paid(30_000, Currency::Usd), &differing, &overrides);
        assert_eq!(fields.price, Some(paid(15_000, Currency::Gbp)));
        assert!(
            blockers.is_empty(),
            "the seller answered the disagreement, so there is nothing left to report"
        );
        assert_eq!(
            matched, 2,
            "the rules that matched stay visible in the preview"
        );
    }

    #[test]
    fn conflicting_licence_rules_block_and_an_explicit_choice_resolves_that_field_alone() {
        let rules = [
            record(0xc1, mapping("CC-BY")),
            record(0xc2, mapping("CC-BY-ND")),
        ];
        let (fields, blockers, _matched) =
            evaluated(PriceIntent::Free, &rules, &RuleOverrides::NONE);
        assert_eq!((fields.licence, blockers.len()), (None, 1));

        let overrides = RuleOverrides {
            licence: Some("CC-BY".to_owned()),
            ..RuleOverrides::default()
        };
        let (fields, blockers, _matched) = evaluated(PriceIntent::Free, &rules, &overrides);
        assert_eq!(fields.licence, Some("CC-BY".to_owned()));
        assert!(blockers.is_empty());
        assert_eq!(fields.price, None, "a licence decision proposes no price");
    }

    /// The school tier is real on read and this system's Tes write path
    /// cannot post it: `paid_price_token` admits TES-PAID alone. So a rule
    /// proposing it is refused where it is authored rather than approved,
    /// frozen and refused by the adapter — and nothing is substituted for it,
    /// because which grant a resource carries is the seller's to make.
    #[test]
    fn the_tes_school_licence_is_refused_and_never_silently_replaced() {
        assert_eq!(
            validate_definition(&mapping("TES-PAID-SCHOOL")),
            Err(RuleError::LicenceNotWritable {
                value: "TES-PAID-SCHOOL".to_owned()
            })
        );
        let (fields, blockers, matched) = evaluated(
            paid(30_000, Currency::Usd),
            &[record(0xc3, mapping("TES-PAID-SCHOOL"))],
            &RuleOverrides::NONE,
        );
        assert_eq!(
            (fields.licence, matched, blockers.len()),
            (None, 0, 1),
            "an inadmissible rule is a refusal rather than a proposal of TES-PAID"
        );

        let overrides = RuleOverrides {
            licence: Some("TES-PAID-SCHOOL".to_owned()),
            ..RuleOverrides::default()
        };
        let (fields, blockers, _matched) = evaluated(paid(30_000, Currency::Usd), &[], &overrides);
        assert_eq!(
            (fields.licence, blockers.len()),
            (None, 1),
            "a one-off choice of the school tier is refused on the same ground"
        );
    }

    #[test]
    fn a_licence_rule_pointed_at_tpt_is_refused_rather_than_pretended() {
        let mut into_tpt = mapping("TES-PAID");
        into_tpt.source = InventoryId::Tes;
        into_tpt.target = InventoryId::Tpt;
        assert_eq!(
            validate_definition(&into_tpt),
            Err(RuleError::AxisAbsentOnTarget {
                target: InventoryId::Tpt,
                axis: TermKind::Licence
            })
        );
        let evaluation = evaluate(
            &product(paid(1_000, Currency::Gbp)),
            (InventoryId::Tes, InventoryId::Tpt),
            &[record(0xd1, into_tpt)],
            &RuleOverrides::NONE,
            &TargetFields::default(),
        );
        assert_eq!(evaluation.fields.licence, None);
        assert_eq!(evaluation.blockers.len(), 1);
        assert!(
            evaluation.matches.is_empty(),
            "a rule that cannot be applied has not matched anything"
        );
    }

    #[test]
    fn the_target_price_gate_on_a_licence_is_enforced_both_ways() {
        let (fields, blockers, matched) = evaluated(
            PriceIntent::Free,
            &[record(0xe1, mapping("TES-PAID"))],
            &RuleOverrides::NONE,
        );
        assert_eq!((fields.licence, matched, blockers.len()), (None, 1, 1));

        let (fields, blockers, _matched) = evaluated(
            paid(30_000, Currency::Usd),
            &[record(0xe2, mapping("CC-BY-ND"))],
            &RuleOverrides::NONE,
        );
        assert_eq!((fields.licence, blockers.len()), (None, 1));

        let legacy = validate_definition(&mapping("TES-V1"));
        assert_eq!(
            legacy,
            Err(RuleError::LicenceNotWritable {
                value: "TES-V1".to_owned()
            }),
            "a value the editor rewrites on load is never proposed on write"
        );
    }

    #[test]
    fn a_mapping_is_approvable_while_the_price_is_still_refused() {
        let rules = [
            record(0xf1, mapping("TES-PAID")),
            record(0xf2, pricing("0.001")),
        ];
        let (fields, blockers, matched) =
            evaluated(paid(30_000, Currency::Usd), &rules, &RuleOverrides::NONE);
        assert_eq!(fields.licence, Some("TES-PAID".to_owned()));
        assert_eq!(
            (fields.price, matched, blockers.len()),
            (None, 2, 1),
            "the licence stands on its own while the conversion is out of the target's range"
        );
    }

    /// An approved price settles its own field and nothing else.
    ///
    /// Two rates that disagree are no longer a disagreement, a conversion
    /// that cannot be made is moot, and the licence rule beside them still
    /// resolves. The defect this pins is the opposite behaviour: the price
    /// conflict discarded the whole evaluation, so an item blocked on a
    /// licence election that a matching, admissible rule had answered.
    #[test]
    fn an_approved_price_settles_its_field_without_suppressing_a_licence_rule() {
        let approved = paid(300, Currency::Gbp);
        let rules = [
            record(0xb1, pricing("0.75")),
            record(0xb2, pricing("0.80")),
            record(0xb3, pricing("0.000001")),
            record(0xf3, mapping("TES-PAID")),
        ];
        let overrides = RuleOverrides {
            price: Some(approved),
            ..RuleOverrides::default()
        };
        let base = TargetFields {
            price: Some(approved),
            ..TargetFields::default()
        };
        let (fields, blockers, matched) = against(
            &product(paid(30_000, Currency::Usd)),
            &rules,
            &overrides,
            &base,
        );
        assert_eq!(
            fields.licence,
            Some("TES-PAID".to_owned()),
            "the licence is a different field and a different decision"
        );
        assert_eq!(
            fields.price,
            Some(approved),
            "the explicit price remains the supplied field; inherited fields alone stay absent"
        );
        assert!(
            blockers.is_empty(),
            "the price was answered before the rules were weighed, got {blockers:?}"
        );
        assert_eq!(
            matched, 4,
            "every matching rule stays visible as provenance"
        );
    }

    /// A licence is judged against the price the *listing* will carry, while
    /// the rule that proposes it is selected by the source's own price. The
    /// two come apart exactly where this used to publish a contradiction: a
    /// free source with an approved paid target price, and an approved paid
    /// licence on a source that has since gone free.
    #[test]
    fn a_licence_is_judged_against_the_merged_price_and_never_against_the_source_alone() {
        let priced = TargetFields {
            price: Some(paid(300, Currency::Gbp)),
            ..TargetFields::default()
        };
        let (fields, blockers, matched) = against(
            &product(PriceIntent::Free),
            &[record(0xe3, mapping("CC-BY"))],
            &RuleOverrides::NONE,
            &priced,
        );
        assert_eq!(matched, 1, "the rule asks about the source, which is free");
        assert_eq!(
            (fields.licence, blockers.len()),
            (None, 1),
            "a Tes listing that sells for GBP 3.00 may not carry CC-BY, whatever the source \
             gives away"
        );

        let granted = TargetFields {
            licence: Some("TES-PAID".to_owned()),
            ..TargetFields::default()
        };
        let (fields, blockers, _matched) = against(
            &product(PriceIntent::Free),
            &[record(0xe4, pricing("0.75"))],
            &RuleOverrides::NONE,
            &granted,
        );
        assert_eq!(fields.price, Some(PriceIntent::Free));
        assert_eq!(
            fields.licence, None,
            "an inherited field is never copied into the patch"
        );
        assert_eq!(
            blockers.len(),
            1,
            "the approved TES-PAID does not survive a free price, and the refusal is explicit \
             rather than left to the adapter"
        );
    }

    #[test]
    fn the_canonical_product_is_read_and_never_rewritten() {
        let before = product(paid(30_000, Currency::Usd));
        let evaluation = evaluate(
            &before,
            (InventoryId::Tpt, InventoryId::Tes),
            &[record(0xa1, pricing("0.75"))],
            &RuleOverrides::NONE,
            &TargetFields::default(),
        );
        assert_eq!(
            (before.price, &before.rights),
            (paid(30_000, Currency::Usd), &RightsDeclaration::Unstated),
            "the source price and the source rights are facts of the source listing"
        );
        assert_eq!(evaluation.fields.price, Some(paid(22_500, Currency::Gbp)));
    }

    #[test]
    fn a_keyword_matches_visible_words_and_never_markup() {
        let mut html = product(PriceIntent::Free);
        html.body = ListingCopy {
            body: "<div class=\"freebie\"><p>Algebra worksheet &amp; answer key</p></div>"
                .to_owned(),
            format: CopyFormat::Html,
        };
        assert_eq!(
            visible_description(&html.body).trim(),
            "Algebra worksheet & answer key"
        );
        let keyword = |word: &str| {
            let mut rule = mapping("CC-BY-ND");
            rule.conditions.keywords = vec![word.to_owned()];
            evaluate(
                &html,
                (InventoryId::Tpt, InventoryId::Tes),
                &[record(0x0a, rule)],
                &RuleOverrides::NONE,
                &TargetFields::default(),
            )
            .matches
            .len()
        };
        assert_eq!(
            keyword("algebra"),
            1,
            "a visible word matches, case-insensitively"
        );
        assert_eq!(
            keyword("worksheet & answer"),
            1,
            "an entity decodes to what a buyer reads"
        );
        assert_eq!(
            keyword("div"),
            0,
            "a tag name is not a word the seller wrote"
        );
        assert_eq!(keyword("freebie"), 0, "neither is an attribute value");
    }

    #[test]
    fn a_condition_can_only_name_an_axis_the_source_declaration_is_kept_for() {
        let mut topic = pricing("0.75");
        topic.conditions.attributes = vec![AttributeCondition {
            axis: TermKind::Topic,
            values: vec!["1000448".to_owned()],
            mode: ConditionMode::Any,
        }];
        assert_eq!(
            validate_definition(&topic),
            Err(RuleError::UnsupportedAxis {
                axis: TermKind::Topic
            })
        );

        let mut empty = pricing("0.75");
        empty.conditions.attributes = vec![AttributeCondition {
            axis: TermKind::Subject,
            values: vec![],
            mode: ConditionMode::All,
        }];
        assert_eq!(
            validate_definition(&empty),
            Err(RuleError::AttributeValuesEmpty {
                axis: TermKind::Subject
            })
        );

        let mut blank = pricing("0.75");
        blank.conditions.keywords = vec![" ".to_owned()];
        assert_eq!(validate_definition(&blank), Err(RuleError::KeywordBlank));
    }

    /// The direction a rule names is the direction it is evaluated in, so a
    /// condition is read against the source it was authored for.
    fn matched(fixture: &CanonicalProduct, rule: &SellerRuleDefinition) -> usize {
        evaluate(
            fixture,
            (rule.source, rule.target),
            &[record(0x44, rule.clone())],
            &RuleOverrides::NONE,
            &TargetFields::default(),
        )
        .matches
        .len()
    }

    #[test]
    fn a_subject_condition_reads_the_canonical_identifiers() {
        let subject = CanonicalTermId(Uuid([0x33; 16]));
        let mut fixture = product(paid(30_000, Currency::Usd));
        fixture.subjects = vec![subject];
        let subject_rule = |value: String| {
            let mut rule = pricing("0.75");
            rule.conditions.attributes = vec![AttributeCondition {
                axis: TermKind::Subject,
                values: vec![value],
                mode: ConditionMode::All,
            }];
            rule
        };
        assert_eq!(
            matched(&fixture, &subject_rule(subject.0.to_hyphenated())),
            1
        );
        assert_eq!(
            matched(&fixture, &subject_rule(Uuid([0x34; 16]).to_hyphenated())),
            0,
            "a subject the product does not carry does not match"
        );
    }

    /// Real imported residue is untyped: `tam_import::residue_of` keeps
    /// exactly the terms a read left without a kind, and TPT's adapter emits
    /// its whole flat taxonomy that way. The fixture this replaces
    /// manufactured `Some(ResourceType)` residue, a shape no ingestion path
    /// produces, so the condition it exercised could never fire on real data.
    #[test]
    fn an_untyped_residue_identifier_answers_only_the_axis_a_capture_establishes() {
        let mut imported = product(paid(30_000, Currency::Usd));
        imported.native_residue = vec![
            ImportedTerm {
                inventory: InventoryId::Tpt,
                kind: None,
                segments: vec!["Unit Plans".to_owned()],
                native_id: Some("unit-plans".to_owned()),
            },
            ImportedTerm {
                inventory: InventoryId::Tes,
                kind: None,
                segments: vec![],
                native_id: Some("99003".to_owned()),
            },
        ];

        // TPT files a grade, a subject, a resource type and a file format in
        // one uncaptured namespace, so nothing on file says `unit-plans` is a
        // resource type rather than any of the others. The condition is
        // refused where it is authored rather than admitted to fire on
        // whatever slug it happens to hit.
        let mut from_tpt = pricing("0.75");
        from_tpt.conditions.resource_types = vec!["unit-plans".to_owned()];
        assert_eq!(
            validate_definition(&from_tpt),
            Err(RuleError::SourceAxisUnclassifiable {
                source: InventoryId::Tpt,
                axis: TermKind::ResourceType
            })
        );
        let (_fields, blockers, matches) = against(
            &imported,
            &[record(0x45, from_tpt)],
            &RuleOverrides::NONE,
            &TargetFields::default(),
        );
        assert_eq!(
            (matches, blockers.len()),
            (0, 1),
            "an inadmissible condition is a refusal, never a silent match"
        );

        // Tes binds its resource type to `mainType` alone and the nine
        // writable ids are captured, so membership does establish the axis.
        let mut from_tes = pricing("0.75");
        from_tes.source = InventoryId::Tes;
        from_tes.target = InventoryId::Tpt;
        from_tes.conditions.resource_types = vec!["99003".to_owned()];
        assert_eq!(validate_definition(&from_tes), Ok(()));
        assert_eq!(matched(&imported, &from_tes), 1);
        let declared = |value: &str| {
            let mut rule = from_tes.clone();
            rule.conditions.resource_types = vec![value.to_owned()];
            matched(&imported, &rule)
        };
        assert_eq!(
            declared("99004"),
            0,
            "a measured id the source never declared does not match"
        );
        assert_eq!(
            declared("unit-plans"),
            0,
            "a TPT slug is not a Tes resource type"
        );
    }

    #[test]
    fn the_output_digest_follows_the_money_and_ignores_an_editorial_retitle() {
        let fields = TargetFields {
            price: Some(paid(22_500, Currency::Gbp)),
            licence: Some("TES-PAID".to_owned()),
            resource_type: None,
        };
        let named = |title: &str| {
            FrozenRuleOutput::freeze(
                &fields,
                paid(30_000, Currency::Usd),
                author(),
                vec![RuleMatch {
                    id: Uuid([0xa1; 16]),
                    revision: 1,
                    title: title.to_owned(),
                    description: String::new(),
                }],
            )
        };
        assert_eq!(
            named("TPT to Tes").digest_material(),
            named("TPT to Tes (2026 rates)").digest_material(),
            "renaming a rule changes nothing about what will be posted"
        );
        let cheaper = FrozenRuleOutput::freeze(
            &TargetFields {
                price: Some(paid(22_499, Currency::Gbp)),
                ..fields.clone()
            },
            paid(30_000, Currency::Usd),
            author(),
            vec![],
        );
        assert_ne!(
            cheaper.digest_material(),
            named("TPT to Tes").digest_material(),
            "one penny is a different output"
        );
    }

    #[test]
    fn a_frozen_output_falls_back_to_the_canonical_price_and_never_to_none() {
        let frozen = FrozenRuleOutput::freeze(
            &TargetFields::default(),
            paid(30_000, Currency::Usd),
            author(),
            vec![],
        );
        assert_eq!(frozen.price, paid(30_000, Currency::Usd));
        assert!(frozen.licence.is_none() && frozen.resource_type.is_none());
    }

    #[test]
    fn every_preset_is_admissible_and_opted_into_nothing() {
        let presets = presets();
        for preset in &presets {
            assert_eq!(
                validate_definition(&preset.definition),
                Ok(()),
                "preset {} is offered, so it must be admissible",
                preset.id
            );
            assert!(
                preset.definition.auto_apply.is_empty(),
                "preset {} is a suggestion until a seller opts it in",
                preset.id
            );
            for use_ in RuleUse::ALL {
                assert!(!preset.definition.auto_applies_to(use_));
            }
        }
    }

    #[test]
    fn the_licence_presets_propose_only_on_the_side_of_the_price_gate_they_name() {
        for preset in presets() {
            let RuleAction::Mapping { licence, .. } = &preset.definition.action else {
                continue;
            };
            let licence = licence.clone().expect("both licence presets set one");
            let gated_free = preset.definition.conditions.pricing == PricingCondition::Free;
            let (matching, other) = if gated_free {
                (PriceIntent::Free, paid(30_000, Currency::Usd))
            } else {
                (paid(30_000, Currency::Usd), PriceIntent::Free)
            };
            let rules = [record(0x55, preset.definition.clone())];
            let (fields, blockers, matched) = evaluated(matching, &rules, &RuleOverrides::NONE);
            assert_eq!(
                (fields.licence, matched, blockers.len()),
                (Some(licence), 1, 0),
                "preset {} proposes its licence on the price it names",
                preset.id
            );
            let (fields, _blockers, matched) = evaluated(other, &rules, &RuleOverrides::NONE);
            assert_eq!(
                (fields.licence, matched),
                (None, 0),
                "preset {} does not fire on the other side of the gate",
                preset.id
            );
        }
    }
}
