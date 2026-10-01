//! Every resource bound in the system. Nothing outside this crate declares one.
//!
//! Admission rule: a constant belongs here only when exceeding it is a
//! shared-resource incident affecting tenants other than the one that caused
//! it, and when no type, database constraint, or OS-level limit already bounds
//! it. A number that bounds only its own caller is an ordinary `const` next to
//! that caller. This crate is deliberately small; a large limits module is a
//! maintenance surface impersonating discipline.
//!
//! Every constant admitted to the resource-bound set carries a provenance
//! marker in its doc comment, a factual claim about where the number came
//! from; the plan table's figures are covered by the marker on
//! `Plan::capabilities` and `PLANS`.
//! `MEASURED` cites a recorded observation, named in the comment.
//! `SIZED` means derived by arithmetic from a quantity that is known
//! independently of measurement, such as the box's RAM or a protocol limit.
//! `DECIDED` is a deliberate operating point citing a dated `decisions.md`
//! entry and naming the trigger that re-opens it; neither a guess nor a
//! measurement.
//! `UNCALIBRATED` is a guess, and is a release blocker for the first paying
//! deployment rather than a wish. The test below pins how many of them exist,
//! so lowering the budget is a deliberate edit and raising it cannot pass
//! unnoticed. Drive it to zero before taking money.
//!
//! This crate's only dependency is `serde`, and it acquires no other. The
//! plan table below is a wire vocabulary as well as a bound — the same rows
//! answer `GET /v1/plans`, the console and the landing build — so the derive
//! lives where the numbers do rather than in a second struct that could
//! disagree with them. `serde` is already inside the pure-core fence that
//! `just purity` polices, so no crate acquires a runtime, a client or a
//! database handle by depending on this one.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// What an organisation holds. The closed set, and the only axis any feature
/// is gated on.
///
/// Three paid tiers above the free one, weakest first: `Starter`,
/// `Subscriber` (sold as "Pro" since the 2026-09-30 founder review; first
/// sold as "Sync", and the spelling predates both and is kept because stored
/// grants carry it; the price keys say `pro_*`) and `Studio`
/// (`docs/notes/design/research/2026-09-27-subscription-tiers.md`, which also
/// withdraws the founding offer and the paid onboarding session).
///
/// `migration_only` is gone, and its absence is the one structural change the
/// 2026-09-20 pricing re-evaluation makes: a pack buyer is a Free account
/// carrying a balance of moves rather than a plan of their own, which is one
/// fewer plan to gate, render and explain. Migration 0084 narrows the `plan`
/// CHECK to this set and turns every live `migration_only` grant into the
/// ledger credit it always meant.
///
/// `Free` is the default in every direction a plan can go missing: an
/// organisation with no grant, a grant that has expired, a device token
/// minted by an older server. Falling back to the smallest plan is the
/// fail-closed direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    #[default]
    Free,
    Starter,
    Subscriber,
    Studio,
}

impl Plan {
    /// Every plan, weakest first.
    ///
    /// The order is the precedence order [`Plan::strength`] renders, so a
    /// reader of either sees the same ladder. Rust cannot check on stable
    /// that this array is total over the enum, so the forcing function is the
    /// exhaustive `match` in `all_is_total_over_the_enum`, which fails to
    /// compile when a variant is added. `wildcard_enum_match_arm` is denied
    /// workspace-wide, so that match cannot be silenced with `_`.
    pub const ALL: [Self; 4] = [Self::Free, Self::Starter, Self::Subscriber, Self::Studio];

    /// The wire spelling, which is also the spelling the `plan` CHECK
    /// constraint enumerates after migration 0090.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Starter => "starter",
            Self::Subscriber => "subscriber",
            Self::Studio => "studio",
        }
    }

    /// The plan a stored spelling names, or `None` for a spelling this build
    /// does not know.
    ///
    /// `None` rather than a fallback to `Free`: a row carrying a plan this
    /// binary cannot read is a deployment running behind its own database,
    /// and silently downgrading a paying tenant is worse than saying so.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|plan| plan.as_str() == raw)
    }

    /// How plans compare when an organisation holds more than one unexpired
    /// grant. Larger wins.
    ///
    /// Not `Ord` on the enum, because the derive would key on declaration
    /// order and make the precedence an accident of where a variant was
    /// typed. Studio outranks Subscriber outranks Starter outranks Free: an
    /// operator grant of
    /// Studio made beside a live subscription must not be held down to the
    /// subscription's narrower set.
    #[must_use]
    pub const fn strength(self) -> u8 {
        match self {
            Self::Free => 0,
            Self::Starter => 1,
            Self::Subscriber => 2,
            Self::Studio => 3,
        }
    }

    /// What this plan grants.
    ///
    /// DECIDED (decisions.md, "Plans, capabilities and the pricing
    /// re-evaluation, 2026-09-12", as amended by
    /// `docs/notes/design/research/2026-09-20-pricing-model-re-evaluation.md`
    /// section 3.2, by `2026-09-27-subscription-tiers.md` section 0, which
    /// adds Starter and sells Studio, and by
    /// `2026-09-29-pricing-structure-review.md` section 5, which opens one
    /// collection to Look and lifts the label cap on every paid plan, and by
    /// `docs/notes/design/entitlement-enforcement.md` section 1, which puts a
    /// resource ceiling on every plan below Studio and counts watermarked
    /// previews per month, and by the addendum to the review dated
    /// 2026-09-30, the founder's own tier changes: Look's previews become five
    /// for the account's lifetime, previews climb 20 / 50 / 100 a month and
    /// stop being unlimited anywhere, Starter holds 250 resources and Pro 500,
    /// Pro keeps ten templates and ten collections, and Starter carries no AI
    /// fill):
    /// every figure here is that table's. Section 8 of the tiers note and
    /// section 9 of the review name the triggers that re-open them.
    ///
    /// `rung` is read only by `Studio`, where it is an operator-set monthly
    /// move allowance above the published hundred, granted with a reason for
    /// a founder trial or a large migration; it never lowers one. No other
    /// plan reads it, because a pack is no longer a plan: it is a credit in
    /// `move_ledger`, and the balance is a query rather than a capability.
    #[must_use]
    pub const fn capabilities(self, rung: Option<u32>) -> Capabilities {
        match self {
            // Look: a small shop brought in whole, and five moves to watch
            // one listing actually appear on the other side. There is no
            // other trial, so import is not gated at all; what bounds it is
            // the resource ceiling. A hundred resources is a typical first
            // shop and still shows every tool (enforcement note section 1).
            Self::Free => Capabilities {
                resources_max: 100,
                marketplaces_max: u32::MAX,
                // DECIDED (2026-09-26 re-evaluation): Look stores covers, not
                // bundles, so even 500 resources fit in ~150 MiB; the cut
                // quadruples the Look accounts Garage holds at quota.
                storage_bytes_max: 256 << 20,
                import_spreadsheet: true,
                import_marketplace: true,
                duplicate_review: true,
                publish_marketplaces_max: u32::MAX,
                moves_per_month: 0,
                moves_accrual_cap: 0,
                free_moves_lifetime: 5,
                pack_edit_days: 90,
                scheduling: false,
                sync_pull_interval_secs: None,
                auto_publish_rules: false,
                templates_max: 1,
                // One of each organising object, so a trial seller can try
                // grouping resources before paying (review section 5).
                collections_max: 1,
                labels_max: 5,
                analytics: false,
                export: true,
                devices_max: 5,
                ai_fills_per_month: 0,
                // Five for the account's whole life rather than five a
                // month: enough to see what a watermarked preview does to a
                // listing, not a standing allowance (founder, 2026-09-30).
                previews_per_month: 0,
                previews_lifetime: 5,
                uploads_in_flight_max: 1,
                support: Support::Guides,
            },
            // Starter: one teacher adding about a resource a week. Edits go
            // out once a day; statistics and automatic rules start at Pro.
            // Two hundred and fifty resources is two and a half times Look,
            // and twenty previews a month covers every new resource.
            Self::Starter => Capabilities {
                resources_max: 250,
                marketplaces_max: u32::MAX,
                storage_bytes_max: 5 << 30,
                import_spreadsheet: true,
                import_marketplace: true,
                duplicate_review: true,
                publish_marketplaces_max: u32::MAX,
                moves_per_month: 10,
                moves_accrual_cap: 30,
                free_moves_lifetime: 0,
                pack_edit_days: 90,
                scheduling: true,
                sync_pull_interval_secs: Some(24 * 3_600),
                auto_publish_rules: false,
                templates_max: 5,
                collections_max: 5,
                // Labels cost nothing to keep and are not a ladder axis:
                // capped on Look, unlimited on every paid plan (review §5).
                labels_max: u32::MAX,
                analytics: false,
                export: true,
                devices_max: 5,
                // No AI fill on Starter: the offer starts at Pro.
                ai_fills_per_month: 0,
                previews_per_month: 20,
                previews_lifetime: 0,
                uploads_in_flight_max: 2,
                support: Support::Email2Days,
            },
            Self::Subscriber => Capabilities {
                resources_max: 500,
                marketplaces_max: u32::MAX,
                storage_bytes_max: 20 << 30,
                import_spreadsheet: true,
                import_marketplace: true,
                duplicate_review: true,
                publish_marketplaces_max: u32::MAX,
                moves_per_month: 25,
                moves_accrual_cap: 75,
                free_moves_lifetime: 0,
                pack_edit_days: 90,
                scheduling: true,
                sync_pull_interval_secs: Some(6 * 3_600),
                auto_publish_rules: true,
                templates_max: 10,
                collections_max: 10,
                labels_max: u32::MAX,
                analytics: true,
                export: true,
                devices_max: 5,
                ai_fills_per_month: 200,
                previews_per_month: 50,
                previews_lifetime: 0,
                uploads_in_flight_max: 3,
                support: Support::Email2Days,
            },
            Self::Studio => Capabilities {
                resources_max: u32::MAX,
                marketplaces_max: u32::MAX,
                storage_bytes_max: 200 << 30,
                import_spreadsheet: true,
                import_marketplace: true,
                duplicate_review: true,
                publish_marketplaces_max: u32::MAX,
                moves_per_month: match rung {
                    Some(granted) if granted > 100 => granted,
                    _ => 100,
                },
                moves_accrual_cap: 300,
                free_moves_lifetime: 0,
                pack_edit_days: 90,
                scheduling: true,
                sync_pull_interval_secs: Some(3_600),
                auto_publish_rules: true,
                templates_max: u32::MAX,
                collections_max: u32::MAX,
                labels_max: u32::MAX,
                analytics: true,
                export: true,
                devices_max: 5,
                ai_fills_per_month: 600,
                // A ceiling rather than none: every preview is a render and
                // a stored image, and a hundred a month covers a whole shop
                // refreshed over a term (founder, 2026-09-30).
                previews_per_month: 100,
                previews_lifetime: 0,
                uploads_in_flight_max: 3,
                support: Support::Email1Day,
            },
        }
    }
}

/// How quickly support answers, which is a plan's promise rather than a
/// bound, and is in the table because the pricing page renders it from the
/// same row every other figure comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Support {
    #[serde(rename = "guides")]
    Guides,
    #[serde(rename = "email_2_days")]
    Email2Days,
    #[serde(rename = "email_1_day")]
    Email1Day,
}

impl Support {
    /// Every level, for the same reason [`Plan::ALL`] exists.
    pub const ALL: [Self; 3] = [Self::Guides, Self::Email2Days, Self::Email1Day];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guides => "guides",
            Self::Email2Days => "email_2_days",
            Self::Email1Day => "email_1_day",
        }
    }
}

/// Everything one plan grants, as one value.
///
/// One struct returned from an exhaustive `match` rather than a lookup keyed
/// by a discriminant: adding a plan is then a compile error at the one site
/// that matters, and no caller needs an index, a bounds check or a fallback.
/// Every field is required for the same reason — an `Option` per capability
/// would let a plan answer "unspecified" to a question every gate has to ask.
///
/// `u32::MAX` means "no ceiling" on the count fields. A sentinel rather than
/// an `Option<u32>` because every reader of those fields is a comparison, and
/// `used >= max` is correct at the sentinel while an `Option` would push a
/// match into each of the nine gate sites. The one genuinely absent quantity
/// is an `Option`: no sync cadence at all.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the eight flags are the price list's own rows, not a state machine; \
              collapsing them into a bitset or sub-structs would make the struct \
              disagree with the table every surface renders from it"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub resources_max: u32,
    pub marketplaces_max: u32,
    /// The blob ceiling, which is the one capability measured in bytes and
    /// the only survivor of the deleted `TierQuota`. The per-tier request
    /// rates that sat beside it went with it: they were never enforced, and
    /// rate limiting is a shared-resource bound rather than a plan axis, so
    /// reinstating one belongs beside the limiter and not in a price list.
    pub storage_bytes_max: u64,
    pub import_spreadsheet: bool,
    pub import_marketplace: bool,
    pub duplicate_review: bool,
    /// How many marketplaces one resource may be published to. `u32::MAX` is
    /// every marketplace; `0` would be none, which no plan holds.
    pub publish_marketplaces_max: u32,
    /// How many moves this plan credits at the start of each billing period,
    /// which is zero on every plan that is not a subscription. A move is one
    /// resource committed to the other marketplace, counted at commit after
    /// duplicate merge, whether it lands as a draft or live.
    pub moves_per_month: u32,
    /// The ceiling the monthly credit accrues to. A subscriber who moves
    /// nothing for four months holds this, not four months of allowance:
    /// the roll-up is a buffer for an uneven month rather than a balance to
    /// hoard and then cancel against.
    pub moves_accrual_cap: u32,
    /// How many moves an organisation is given once, ever, the first time a
    /// storefront binds to it. Granted against the storefront rather than
    /// against the organisation — `storefront_allowance` is the record — so
    /// a second organisation naming the same shop is given nothing.
    pub free_moves_lifetime: u32,
    /// How long after a move the listing it made may still be edited or
    /// deleted through us. Ninety days on every plan: the window is the
    /// pack's promise, and a pack is bought on any plan.
    pub pack_edit_days: u32,
    pub scheduling: bool,
    /// How often the device re-enumerates a shop. `None` is no sync pulls.
    pub sync_pull_interval_secs: Option<u32>,
    pub auto_publish_rules: bool,
    pub templates_max: u32,
    pub collections_max: u32,
    pub labels_max: u32,
    pub analytics: bool,
    /// Never false on any plan, and a field rather than an omission: the
    /// decision that a seller who cannot get their catalogue out will not put
    /// one in is worth being able to point at, and a test pins it.
    pub export: bool,
    pub devices_max: u32,
    pub ai_fills_per_month: u32,
    /// Watermarked previews made in the preview maker, counted per UTC
    /// calendar month in `usage_counter` (migration 0098). Zero on a plan
    /// whose previews are a lifetime allowance instead.
    pub previews_per_month: u32,
    /// Watermarked previews for the organisation's whole life, on a plan
    /// that carries no monthly allowance: the sum of every month's
    /// `usage_counter` row, so no second counter can drift from the first.
    /// Zero on every plan that counts previews per month; see
    /// [`Capabilities::previews`] for which of the two a gate reads.
    pub previews_lifetime: u32,
    /// Uploads one organisation may have open at once. The upload route is
    /// the one server path that is memory-heavy and holds a pool connection
    /// across the object-store write, so this is the fairness bound between
    /// accounts sharing a replica (2026-09-26 re-evaluation, "can free
    /// accounts starve paid ones").
    pub uploads_in_flight_max: u32,
    pub support: Support,
}

/// Which window a plan counts its watermarked previews in, and how many the
/// window holds.
///
/// Read by every gate and every usage figure through
/// [`Capabilities::previews`] rather than off the two fields, so no reader
/// has to know that a zero monthly allowance beside a lifetime one means
/// "count the lifetime" rather than "none".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewAllowance {
    /// This many for the organisation's whole life.
    Lifetime(u32),
    /// This many each UTC calendar month.
    Monthly(u32),
}

impl PreviewAllowance {
    /// The ceiling, whichever window it is counted in.
    #[must_use]
    pub const fn cap(self) -> u32 {
        match self {
            Self::Lifetime(cap) | Self::Monthly(cap) => cap,
        }
    }
}

impl Capabilities {
    /// The window this plan counts watermarked previews in: the lifetime
    /// allowance where the plan carries one, the month otherwise.
    #[must_use]
    pub const fn previews(&self) -> PreviewAllowance {
        if self.previews_lifetime > 0 {
            PreviewAllowance::Lifetime(self.previews_lifetime)
        } else {
            PreviewAllowance::Monthly(self.previews_per_month)
        }
    }
}

/// One row of the price list.
///
/// The prices and the keys that buy them are absent for Free, which charges
/// nothing. Packs are not rows here: a pack buys a balance of moves on
/// whatever plan the buyer already holds, so it is priced in [`PACKS`] and
/// keyed by [`PriceKey`] rather than named as a plan.
///
/// The keys ride on the row so a card can open the checkout for the cadence
/// it shows without a second table mapping plans to keys;
/// `every_paid_row_names_the_keys_that_grant_it` pins that each key grants
/// the row it sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRow {
    pub id: Plan,
    pub name: &'static str,
    pub monthly_cents: Option<u32>,
    pub yearly_cents: Option<u32>,
    pub monthly_key: Option<PriceKey>,
    pub yearly_key: Option<PriceKey>,
    pub trial_days: u32,
    /// The one paid plan a pricing page raises and badges "Recommended".
    /// Exactly one row carries it, which `exactly_one_paid_plan_is_recommended`
    /// pins, so no surface names the plan it features by hand.
    pub recommended: bool,
    /// Who the plan is for, in one teacher-facing sentence under its name.
    pub tagline: &'static str,
}

/// Everything a checkout can be opened for, as one closed vocabulary.
///
/// The processor's own price identifiers are opaque strings it mints and we
/// never choose; `--stripe-price-map` maps each of them onto one of these,
/// so the vocabulary crossing the wire and the vocabulary the server gates
/// on are the same closed set rather than two string tables that can drift.
/// A key here is not a plan: a pack grants no plan at all, which is why
/// [`PriceKey::plan`] answers an `Option`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PriceKey {
    #[serde(rename = "starter_monthly")]
    StarterMonthly,
    #[serde(rename = "starter_yearly")]
    StarterYearly,
    #[serde(rename = "pro_monthly")]
    ProMonthly,
    #[serde(rename = "pro_yearly")]
    ProYearly,
    #[serde(rename = "studio_monthly")]
    StudioMonthly,
    #[serde(rename = "studio_yearly")]
    StudioYearly,
    #[serde(rename = "pack_20")]
    Pack20,
    #[serde(rename = "pack_50")]
    Pack50,
    #[serde(rename = "pack_100")]
    Pack100,
    #[serde(rename = "pack_250")]
    Pack250,
    #[serde(rename = "pack_500")]
    Pack500,
}

impl PriceKey {
    /// Every key, in the order a pricing page reads them.
    pub const ALL: [Self; 11] = [
        Self::StarterMonthly,
        Self::StarterYearly,
        Self::ProMonthly,
        Self::ProYearly,
        Self::StudioMonthly,
        Self::StudioYearly,
        Self::Pack20,
        Self::Pack50,
        Self::Pack100,
        Self::Pack250,
        Self::Pack500,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StarterMonthly => "starter_monthly",
            Self::StarterYearly => "starter_yearly",
            Self::ProMonthly => "pro_monthly",
            Self::ProYearly => "pro_yearly",
            Self::StudioMonthly => "studio_monthly",
            Self::StudioYearly => "studio_yearly",
            Self::Pack20 => "pack_20",
            Self::Pack50 => "pack_50",
            Self::Pack100 => "pack_100",
            Self::Pack250 => "pack_250",
            Self::Pack500 => "pack_500",
        }
    }

    /// The key a stored or configured spelling names, or `None` for one this
    /// build does not know. `None` rather than a fallback, for the reason
    /// [`Plan::parse`] gives: a price map naming a key we cannot read is a
    /// deployment behind its own configuration, and guessing which SKU was
    /// bought is worse than refusing to guess.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.as_str() == raw)
    }

    /// The plan a recurring key grants, or `None` for a pack, which credits
    /// moves on whatever plan the buyer holds.
    #[must_use]
    pub const fn plan(self) -> Option<Plan> {
        match self {
            Self::StarterMonthly | Self::StarterYearly => Some(Plan::Starter),
            Self::ProMonthly | Self::ProYearly => Some(Plan::Subscriber),
            Self::StudioMonthly | Self::StudioYearly => Some(Plan::Studio),
            Self::Pack20 | Self::Pack50 | Self::Pack100 | Self::Pack250 | Self::Pack500 => None,
        }
    }

    /// Whether this key bills again by itself: exactly the keys that grant a
    /// plan, which are the ones a billing portal can cancel.
    #[must_use]
    pub const fn recurring(self) -> bool {
        self.plan().is_some()
    }

    /// What this key charges before any discount, in US cents, read off the
    /// row in [`PLANS`] or [`PACKS`] that names it rather than restated.
    /// `every_key_has_a_list_price` pins that no key falls through to zero.
    #[must_use]
    pub const fn list_cents(self) -> u32 {
        let mut index = 0;
        while index < PLANS.len() {
            let row = PLANS[index];
            if let (Some(key), Some(cents)) = (row.monthly_key, row.monthly_cents) {
                if key as u8 == self as u8 {
                    return cents;
                }
            }
            if let (Some(key), Some(cents)) = (row.yearly_key, row.yearly_cents) {
                if key as u8 == self as u8 {
                    return cents;
                }
            }
            index += 1;
        }
        let mut index = 0;
        while index < PACKS.len() {
            if PACKS[index].key as u8 == self as u8 {
                return PACKS[index].price_cents;
            }
            index += 1;
        }
        0
    }
}

/// One pack: a count of moves and what it costs.
///
/// `per_move_cents` is carried rather than computed at every render, because
/// it is the figure the page compares rungs by and three surfaces would
/// otherwise round it three ways. `packs_are_a_ladder_priced_per_move` pins
/// it against the division.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pack {
    pub key: PriceKey,
    pub moves: u32,
    pub price_cents: u32,
    pub per_move_cents: u32,
}

/// What the AI auto-fill offer promises, which today is that it is coming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiOffer {
    pub status: AiStatus,
    pub included_fills: u32,
    pub add_on_fills: u32,
    pub add_on_cents: u32,
}

/// Where the AI auto-fill offer stands. One variant today, and a closed set
/// rather than a free string so the day it ships is a compile error at every
/// surface that renders "coming soon".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiStatus {
    ComingSoon,
}

impl AiStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ComingSoon => "coming_soon",
        }
    }
}

/// The price list, in the order a pricing page reads it.
///
/// DECIDED (`docs/notes/design/research/2026-09-27-subscription-tiers.md`
/// section 0): Pro (first sold as Sync) keeps the price decisions.md
/// approved on 2026-09-12; Starter and Studio sit on the category's entry
/// and "pro" rungs. Every
/// yearly price is about a third off twelve monthly ones, so each card can
/// say "$20 a month, billed yearly" and mean it. Section 8 of the note names
/// what re-opens these. `2026-09-29-pricing-structure-review.md` sections 3
/// and 4 re-test the prices and the yearly saving against the category and
/// keep both; section 6 makes Pro the recommended plan. The 2026-09-30
/// addendum renames Sync to Pro and leaves every price where it is, and 0.18.0 renames the `sync_*` keys
/// to `pro_*` so our key and Stripe's product, nickname and lookup key
/// read alike.
///
/// No trial days on any row. Look is the trial, there is no other, and a
/// 14-day clock beside a free plan that never expires was two offers where
/// the seller only ever understood one.
pub const PLANS: [PlanRow; 4] = [
    PlanRow {
        id: Plan::Free,
        name: "Look",
        monthly_cents: None,
        yearly_cents: None,
        monthly_key: None,
        yearly_key: None,
        trial_days: 0,
        recommended: false,
        tagline:
            "For a small shop: bring your resources in, make five previews and try five moves.",
    },
    PlanRow {
        id: Plan::Starter,
        name: "Starter",
        monthly_cents: Some(1_200),
        yearly_cents: Some(9_600),
        monthly_key: Some(PriceKey::StarterMonthly),
        yearly_key: Some(PriceKey::StarterYearly),
        trial_days: 0,
        recommended: false,
        tagline: "For teachers who add a resource now and then.",
    },
    PlanRow {
        id: Plan::Subscriber,
        name: "Pro",
        monthly_cents: Some(2_900),
        yearly_cents: Some(24_000),
        monthly_key: Some(PriceKey::ProMonthly),
        yearly_key: Some(PriceKey::ProYearly),
        trial_days: 0,
        recommended: true,
        tagline: "For teachers who add resources every week.",
    },
    PlanRow {
        id: Plan::Studio,
        name: "Studio",
        monthly_cents: Some(5_900),
        yearly_cents: Some(48_000),
        monthly_key: Some(PriceKey::StudioMonthly),
        yearly_key: Some(PriceKey::StudioYearly),
        trial_days: 0,
        recommended: false,
        tagline: "For big catalogues and whole-shop moves.",
    },
];

/// The move packs, smallest first.
///
/// DECIDED (decisions.md, 2026-09-12, semantics amended 2026-09-20): the five
/// prices are unchanged — they are readable, approved and live on the page —
/// and what changed is what they buy. A rung was a resource ceiling on a plan
/// of its own; a pack is a balance of moves on whatever plan the buyer holds,
/// debited at commit and expiring twelve months after purchase.
pub const PACKS: [Pack; 5] = [
    Pack {
        key: PriceKey::Pack20,
        moves: 20,
        price_cents: 4_700,
        per_move_cents: 235,
    },
    Pack {
        key: PriceKey::Pack50,
        moves: 50,
        price_cents: 7_700,
        per_move_cents: 154,
    },
    Pack {
        key: PriceKey::Pack100,
        moves: 100,
        price_cents: 12_700,
        per_move_cents: 127,
    },
    Pack {
        key: PriceKey::Pack250,
        moves: 250,
        price_cents: 24_700,
        per_move_cents: 98,
    },
    Pack {
        key: PriceKey::Pack500,
        moves: 500,
        price_cents: 39_700,
        per_move_cents: 79,
    },
];

/// How long a bought pack's moves last. Twelve months from purchase, which
/// is the listing-credit precedent the market pack records.
pub const PACK_VALID_MONTHS: u32 = 12;

/// What a catalogue above the top pack is offered: a conversation, not a
/// price. Carried here rather than written into the page's copy so the server
/// and the two clients say the same words.
pub const PACK_ABOVE: &str = "Talk to us";

/// The AI auto-fill packaging: bundled with a fair-use cap and a small
/// add-on, no credit currency.
pub const AI: AiOffer = AiOffer {
    status: AiStatus::ComingSoon,
    included_fills: 200,
    add_on_fills: 100,
    add_on_cents: 500,
};

/// One value per plan, keyed by the plan's wire name, so a client reads
/// `included[plan.id]` rather than trusting an array's order to match
/// [`Plan::ALL`]. A struct rather than a map so a fifth plan is a compile
/// error at every row of [`PLAN_FEATURES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerPlan<T> {
    pub free: T,
    pub starter: T,
    pub subscriber: T,
    pub studio: T,
}

impl<T: Copy> PerPlan<T> {
    #[must_use]
    pub const fn get(&self, plan: Plan) -> T {
        match plan {
            Plan::Free => self.free,
            Plan::Starter => self.starter,
            Plan::Subscriber => self.subscriber,
            Plan::Studio => self.studio,
        }
    }
}

/// What one plan gets of one feature: `false` (not on this plan), `true`
/// (included, and on a counted feature, with no ceiling) or a number read
/// in the feature's [`FeatureUnit`]. Untagged, so the wire carries the bare
/// `boolean | number` a comparison table renders as a tick, a dash or a
/// figure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Included {
    Flag(bool),
    Limit(u32),
}

impl Included {
    /// A count read off [`Capabilities`], where `0` is "not on this plan"
    /// and `u32::MAX` is "no ceiling".
    #[must_use]
    pub const fn counted(n: u32) -> Self {
        match n {
            0 => Self::Flag(false),
            u32::MAX => Self::Flag(true),
            limit => Self::Limit(limit),
        }
    }
}

/// How a number in an [`Included`] cell reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureUnit {
    /// A plain count: "500", "20".
    Count,
    /// A count renewed each billing month: "25 a month".
    PerMonth,
    /// A size in mebibytes, which a client renders as MB or GB.
    Megabytes,
    /// An interval in hours, where fewer is better: "every 6 hours".
    EveryHours,
}

impl FeatureUnit {
    pub const ALL: [Self; 4] = [
        Self::Count,
        Self::PerMonth,
        Self::Megabytes,
        Self::EveryHours,
    ];
}

/// The comparison table's sections, in the order it reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureGroup {
    Moving,
    Automation,
    Catalogue,
    Insight,
    Support,
}

impl FeatureGroup {
    pub const ALL: [Self; 5] = [
        Self::Moving,
        Self::Automation,
        Self::Catalogue,
        Self::Insight,
        Self::Support,
    ];

    /// The heading a comparison table draws above the group's rows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Moving => "Moves and updates",
            Self::Automation => "Automations",
            Self::Catalogue => "Your catalogue",
            Self::Insight => "Statistics and AI",
            Self::Support => "Help",
        }
    }
}

/// Every row of the plan comparison, as one closed vocabulary.
///
/// Each key reads its cells off [`Capabilities`] in [`FeatureKey::included`]
/// where a capability exists, so the table a seller compares is the table
/// the server enforces; the rows with no capability behind them are the
/// features every plan carries and no gate withholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKey {
    Moves,
    MovesRollover,
    CopyOrMove,
    EditSync,
    Scheduling,
    AutoPublishRules,
    TermAndPriceRules,
    Resources,
    Storage,
    Import,
    DuplicateReview,
    RichText,
    WatermarkedPreviews,
    Templates,
    Collections,
    Labels,
    Export,
    Analytics,
    AiFill,
    DesktopApp,
    EmailSupport,
    PrioritySupport,
}

impl FeatureKey {
    /// Every key, for the same reason [`Plan::ALL`] exists.
    pub const ALL: [Self; 22] = [
        Self::Moves,
        Self::MovesRollover,
        Self::CopyOrMove,
        Self::EditSync,
        Self::Scheduling,
        Self::AutoPublishRules,
        Self::TermAndPriceRules,
        Self::Resources,
        Self::Storage,
        Self::Import,
        Self::DuplicateReview,
        Self::RichText,
        Self::WatermarkedPreviews,
        Self::Templates,
        Self::Collections,
        Self::Labels,
        Self::Export,
        Self::Analytics,
        Self::AiFill,
        Self::DesktopApp,
        Self::EmailSupport,
        Self::PrioritySupport,
    ];

    /// What a plan holding `caps` gets of this feature.
    ///
    /// The rows answering a constant `true` are the core: import, the
    /// editor, rules and the desktop app are the product itself, cost
    /// nothing per seller, and are what a trial has to show
    /// (`2026-09-29-pricing-structure-review.md` section 5). Watermarked
    /// previews left the core in the enforcement note: every plan makes
    /// them, and how many a month climbs the ladder. Look's cell is the dash
    /// of a plan with no monthly allowance; its five lifetime previews are
    /// `previews_lifetime`, which a table says in place of the dash, as it
    /// does Look's trial moves.
    #[must_use]
    #[expect(
        clippy::integer_division,
        reason = "the pull interval is a whole number of hours on every plan, which \
                  `every_edit_sync_interval_is_whole_hours` pins"
    )]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "200 GiB is 204,800 MiB, far inside u32; the shift runs before the cast"
    )]
    pub const fn included(self, caps: &Capabilities) -> Included {
        match self {
            Self::Moves => Included::counted(caps.moves_per_month),
            Self::MovesRollover => Included::counted(caps.moves_accrual_cap),
            Self::EditSync => match caps.sync_pull_interval_secs {
                Some(secs) => Included::Limit(secs / 3_600),
                None => Included::Flag(false),
            },
            Self::Scheduling => Included::Flag(caps.scheduling),
            Self::AutoPublishRules => Included::Flag(caps.auto_publish_rules),
            Self::Resources => Included::counted(caps.resources_max),
            Self::Storage => Included::counted((caps.storage_bytes_max >> 20) as u32),
            Self::Import => Included::Flag(caps.import_spreadsheet && caps.import_marketplace),
            Self::DuplicateReview => Included::Flag(caps.duplicate_review),
            Self::Templates => Included::counted(caps.templates_max),
            Self::Collections => Included::counted(caps.collections_max),
            Self::Labels => Included::counted(caps.labels_max),
            Self::Export => Included::Flag(caps.export),
            Self::Analytics => Included::Flag(caps.analytics),
            Self::AiFill => Included::counted(caps.ai_fills_per_month),
            Self::WatermarkedPreviews => Included::counted(caps.previews_per_month),
            Self::DesktopApp => Included::Flag(caps.devices_max > 0),
            Self::EmailSupport => Included::Flag(!matches!(caps.support, Support::Guides)),
            Self::PrioritySupport => Included::Flag(matches!(caps.support, Support::Email1Day)),
            Self::CopyOrMove | Self::TermAndPriceRules | Self::RichText => Included::Flag(true),
        }
    }
}

/// One row of the plan comparison: what the feature is called, where it
/// sits, how its numbers read, whether it is sold before it is built, and
/// what each plan gets of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanFeature {
    pub key: FeatureKey,
    pub label: &'static str,
    pub group: FeatureGroup,
    /// `None` on a yes-or-no row, where every cell is a flag.
    pub unit: Option<FeatureUnit>,
    /// Sold before it is built. A table draws a hollow mark rather than a
    /// tick, so a tick never claims a feature exists.
    pub soon: bool,
    pub included: PerPlan<Included>,
}

const fn feature(
    key: FeatureKey,
    label: &'static str,
    group: FeatureGroup,
    unit: Option<FeatureUnit>,
) -> PlanFeature {
    PlanFeature {
        key,
        label,
        group,
        unit,
        soon: matches!(key, FeatureKey::AiFill) && matches!(AI.status, AiStatus::ComingSoon),
        included: PerPlan {
            free: key.included(&Plan::Free.capabilities(None)),
            starter: key.included(&Plan::Starter.capabilities(None)),
            subscriber: key.included(&Plan::Subscriber.capabilities(None)),
            studio: key.included(&Plan::Studio.capabilities(None)),
        },
    }
}

/// The plan comparison, grouped and in reading order.
///
/// DECIDED (`docs/notes/design/research/2026-09-29-pricing-structure-review.md`
/// section 5, the feature matrix): which features are core, which are
/// counted per plan and which start at a plan. The cells are computed from
/// [`Plan::capabilities`] at compile time, so this table cannot promise a
/// plan more or less than its gates allow; only the labels and the order
/// are written here.
pub const PLAN_FEATURES: [PlanFeature; 22] = [
    feature(
        FeatureKey::Moves,
        "Moves",
        FeatureGroup::Moving,
        Some(FeatureUnit::PerMonth),
    ),
    feature(
        FeatureKey::MovesRollover,
        "Unused moves carry over, up to",
        FeatureGroup::Moving,
        Some(FeatureUnit::Count),
    ),
    feature(
        FeatureKey::CopyOrMove,
        "Copy or move resources between marketplaces",
        FeatureGroup::Moving,
        None,
    ),
    feature(
        FeatureKey::EditSync,
        "Edits sent to every marketplace",
        FeatureGroup::Moving,
        Some(FeatureUnit::EveryHours),
    ),
    feature(
        FeatureKey::Scheduling,
        "Schedule when a listing goes live",
        FeatureGroup::Automation,
        None,
    ),
    feature(
        FeatureKey::AutoPublishRules,
        "Automatic publishing rules",
        FeatureGroup::Automation,
        None,
    ),
    feature(
        FeatureKey::TermAndPriceRules,
        "Term mapping and price rules",
        FeatureGroup::Automation,
        None,
    ),
    feature(
        FeatureKey::Resources,
        "Resources",
        FeatureGroup::Catalogue,
        Some(FeatureUnit::Count),
    ),
    feature(
        FeatureKey::Storage,
        "Storage for covers and previews",
        FeatureGroup::Catalogue,
        Some(FeatureUnit::Megabytes),
    ),
    feature(
        FeatureKey::Import,
        "Import from your shops or a spreadsheet",
        FeatureGroup::Catalogue,
        None,
    ),
    feature(
        FeatureKey::DuplicateReview,
        "Find and merge duplicates",
        FeatureGroup::Catalogue,
        None,
    ),
    feature(
        FeatureKey::RichText,
        "Rich-text descriptions",
        FeatureGroup::Catalogue,
        None,
    ),
    feature(
        FeatureKey::WatermarkedPreviews,
        "Watermarked previews",
        FeatureGroup::Catalogue,
        Some(FeatureUnit::PerMonth),
    ),
    feature(
        FeatureKey::Templates,
        "Templates",
        FeatureGroup::Catalogue,
        Some(FeatureUnit::Count),
    ),
    feature(
        FeatureKey::Collections,
        "Collections",
        FeatureGroup::Catalogue,
        Some(FeatureUnit::Count),
    ),
    feature(
        FeatureKey::Labels,
        "Labels",
        FeatureGroup::Catalogue,
        Some(FeatureUnit::Count),
    ),
    feature(
        FeatureKey::Export,
        "Export to a spreadsheet",
        FeatureGroup::Catalogue,
        None,
    ),
    feature(
        FeatureKey::Analytics,
        "Statistics on every shop",
        FeatureGroup::Insight,
        None,
    ),
    feature(
        FeatureKey::AiFill,
        "AI description fill",
        FeatureGroup::Insight,
        Some(FeatureUnit::PerMonth),
    ),
    feature(
        FeatureKey::DesktopApp,
        "Desktop app for your own devices",
        FeatureGroup::Support,
        None,
    ),
    feature(
        FeatureKey::EmailSupport,
        "Email support",
        FeatureGroup::Support,
        None,
    ),
    feature(
        FeatureKey::PrioritySupport,
        "Priority support, answered within a day",
        FeatureGroup::Support,
        None,
    ),
];

pub mod http {
    /// SIZED against RAM, not against traffic: at this ceiling the concurrent
    /// request limit cannot buffer more than a small fraction of the box's
    /// memory. Applies to every route except the ingestion upload routes.
    /// Revisit once the spike records real listing-metadata payload sizes.
    pub const REQUEST_BODY_BYTES_MAX: u64 = 2 * 1024 * 1024;

    /// BOUNDED by the edge, not the marketplace: Cloudflare's free plan
    /// refuses request bodies over 100 MB, so a larger ceiling here is
    /// unreachable through teachouse.io and only sizes the buffers a replica
    /// holds (the upload path keeps ~3 copies). 96 MiB is the largest body
    /// the edge passes, with multipart headroom. Tes-legal files up to 200 MB
    /// still reach Tes: the desktop sends bundles from the seller's own disk.
    pub const UPLOAD_BODY_BYTES_MAX: u64 = 96 * 1024 * 1024;
}

pub mod ingest {
    /// SIZED: the absolute ceiling on bytes written out of an archive,
    /// independent of the ratio check below. Bounding compressed input is not
    /// a zip-bomb defence; bounding decompressed output is.
    pub const ARCHIVE_UNCOMPRESSED_BYTES_MAX: u64 = 1024 * 1024 * 1024;

    /// DECIDED (decisions.md, "Limits calibration, 2026-08-28"): uncompressed
    /// divided by compressed, checked incrementally rather than after the fact.
    /// Deliberately loose so a false rejection is unlikely before the ratio
    /// distribution of real seller bundles is sampled; the customer-zero import
    /// is the first sample and re-opens this number.
    pub const ARCHIVE_COMPRESSION_RATIO_MAX: u64 = 200;
}

pub mod job {
    use std::time::Duration;

    /// SIZED against `WALL_CLOCK_MAX` at the backoff base: the deadline
    /// outlives the full retry budget, pinned by the wall-clock test below.
    /// Which faults are worth retrying at all is M0's tested fault taxonomy.
    pub const ATTEMPTS_MAX: u32 = 5;

    /// SIZED against support response time, not against publish duration: a
    /// wedged job must free its lease well inside one working day. Attempts
    /// multiplied by exponential backoff can exceed any sane duration at a
    /// small attempt count, so this deadline is enforced alongside the count
    /// rather than derived from it.
    pub const WALL_CLOCK_MAX: Duration = Duration::from_mins(30);
}

pub mod import {
    /// DECIDED (founder, 2026-09-05, the spreadsheet-import decision set): the
    /// largest spreadsheet one upload may carry.
    ///
    /// Not `http::UPLOAD_BODY_BYTES_MAX` at 256 MiB, because an xlsx is a zip
    /// and a zip of cells at that size is a parse bomb whose expansion the
    /// ingest pipeline's own archive bounds never see -- this upload
    /// deliberately does not go through the ingest route. Not
    /// `http::REQUEST_BODY_BYTES_MAX` at 2 MiB either, which is the general
    /// route ceiling this one exceeds on purpose: a five-hundred-row workbook
    /// with a dropdown per vocabulary column is comfortably past it.
    ///
    /// Re-opened by the first real seller workbook that is refused here.
    pub const SPREADSHEET_BYTES_MAX: u64 = 8 * 1024 * 1024;

    /// DECIDED (founder, 2026-09-05, the spreadsheet-import decision set): how
    /// many rows one upload may carry across every tab.
    ///
    /// Chosen against `Tier::quota().listings_max` -- Free 100, Pro 5 000 --
    /// and against the reject-the-whole-upload model, which makes a refused
    /// five-thousand-row sheet both a slow parse and a bad answer. It sits
    /// above the founder's own catalogue scale.
    pub const ROWS_PER_UPLOAD_MAX: usize = 500;

    /// DECIDED (founder, 2026-09-05, the spreadsheet-import decision set): how
    /// long an unsettled batch survives before the sweep settles it and
    /// releases the bytes attached to its rows.
    ///
    /// A retention window like `ledger::JOB_EVENT_RETENTION_DAYS` beside it,
    /// and it is here for the same reason: a batch nobody finished charges
    /// storage against a tenant's quota for bytes that belong to no product,
    /// which the seller cannot see and cannot free. Deleting a seller's own
    /// uploaded bytes is why this number needed a word rather than a default.
    pub const BATCH_EXPIRY_DAYS: i64 = 14;
}

pub mod ledger {
    /// SIZED between `job::WALL_CLOCK_MAX` at the floor and the resync
    /// contract at the ceiling: a job's events are complete within half an
    /// hour of enqueue, and a client resuming below the pruning watermark is
    /// resynced rather than failed, so retention is an audit window rather
    /// than a correctness bound. A month sits three orders above the floor.
    pub const JOB_EVENT_RETENTION_DAYS: i64 = 30;

    /// SIZED against the drainer's own 30-second backoff base: polling at a
    /// third of it keeps the poll from dominating a retried message's latency.
    pub const OUTBOX_DRAIN_INTERVAL_SECS: u64 = 10;

    /// SIZED against the retention window: hourly is 720 passes inside a
    /// month, so the batch below has to cover only a fraction of a window's
    /// events per pass for the pruner to keep pace with the ledger.
    pub const PRUNE_INTERVAL_SECS: u64 = 3_600;

    /// SIZED against the outbound pacing ceiling: at the interval above this
    /// erases 240,000 rows a day, several times the events
    /// `marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX` can cause in one, so a
    /// tenant's backlog cannot outrun the pruner while bounding the lock
    /// footprint of any single pass.
    pub const PRUNE_BATCH: i64 = 10_000;
}

pub mod llm {
    /// SIZED as a loss ceiling rather than from a token price: this is the
    /// per-tenant daily spend the business is willing to lose to a runaway
    /// loop before a human looks. Recompute against the provider's actual
    /// per-token price once one is chosen. Spend is a first-class bounded
    /// resource here, not a proxy for request count.
    pub const CENTS_PER_TENANT_PER_DAY_MAX: u32 = 500;
}

pub mod marketplace {
    use std::num::NonZeroU32;

    /// DECIDED (decisions.md, "Limits calibration, 2026-08-28"), the only
    /// constant here with a legal rather than an operational justification.
    /// Tes publishes no throttle and none has been observed; where one is
    /// published, as on Etsy, the lower figure binds. Raising it requires the
    /// written-terms answer in the charter's section 1.
    pub const OUTBOUND_REQUESTS_PER_MINUTE_MAX: NonZeroU32 = NonZeroU32::new(30).unwrap();
}

/// Relationships between constants, checked at compile time rather than at run
/// time. Each is a claim that could actually be false after an edit; a claim
/// the declaration already guarantees is not written here.
///
/// The pointer-width claim below is `cfg`-scoped to non-wasm targets. It is a
/// claim about the axum boundary, which exists only in the server binaries;
/// `wasm32` is a 32-bit client target that performs no such conversion, so on
/// wasm the assertion would fail for a boundary the build does not contain.
const _: () = {
    assert!(
        http::UPLOAD_BODY_BYTES_MAX >= http::REQUEST_BODY_BYTES_MAX,
        "the upload ceiling must not be tighter than the ordinary body ceiling"
    );
    assert!(
        ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX >= http::UPLOAD_BODY_BYTES_MAX,
        "an archive at the upload ceiling must be able to expand at all"
    );
    assert!(
        http::UPLOAD_BODY_BYTES_MAX * ingest::ARCHIVE_COMPRESSION_RATIO_MAX
            > ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
        "the ratio cap must be able to bind before the absolute cap, or one of them is dead code"
    );
    assert!(
        import::SPREADSHEET_BYTES_MAX > http::REQUEST_BODY_BYTES_MAX,
        "the spreadsheet route exceeds the general body ceiling deliberately, and says so"
    );
    assert!(
        import::SPREADSHEET_BYTES_MAX < http::UPLOAD_BODY_BYTES_MAX,
        "a spreadsheet is a zip parsed into cells, so it is bounded well under the payload ceiling"
    );
    #[cfg(not(target_family = "wasm"))]
    assert!(
        usize::BITS >= 64,
        "byte bounds are u64 and are converted to usize at the axum boundary"
    );
};

#[cfg(test)]
mod tests {
    use super::{
        http, ingest, job, Capabilities, FeatureGroup, FeatureKey, FeatureUnit, Included, Plan,
        PreviewAllowance, PriceKey, Support, PACKS, PLANS, PLAN_FEATURES,
    };

    /// The pricing page raises one plan. Two would be no recommendation and
    /// a free one would be recommending that nobody pays.
    #[test]
    fn exactly_one_paid_plan_is_recommended() {
        let recommended: Vec<Plan> = PLANS
            .iter()
            .filter(|row| row.recommended)
            .map(|row| row.id)
            .collect();
        assert_eq!(recommended.len(), 1, "recommended: {recommended:?}");
        assert_ne!(
            recommended[0],
            Plan::Free,
            "the recommended plan must be one that is paid for"
        );
        for row in PLANS {
            let tagline = row.tagline.trim();
            assert!(
                tagline.ends_with('.') && !tagline.trim_end_matches('.').contains(". "),
                "{}'s tagline must be one sentence: {tagline:?}",
                row.id.as_str()
            );
        }
    }

    /// How far a cell reaches, on a scale where a stronger plan must never
    /// score lower. An interval counts down: an hourly sync is more than a
    /// daily one.
    fn reach(cell: Included, unit: Option<FeatureUnit>) -> u64 {
        match cell {
            Included::Flag(false) => 0,
            Included::Flag(true) => u64::MAX,
            Included::Limit(n) if unit == Some(FeatureUnit::EveryHours) => u64::from(u32::MAX - n),
            Included::Limit(n) => u64::from(n),
        }
    }

    /// No row of the comparison gets worse as the plans climb: an upgrade
    /// that lost a tick or a count anywhere would be a downgrade in one row
    /// the seller did not look at.
    #[test]
    fn the_feature_matrix_never_gets_worse_up_the_ladder() {
        for row in PLAN_FEATURES {
            for (below, above) in Plan::ALL.iter().zip(Plan::ALL.iter().skip(1)) {
                let (lower, upper) = (row.included.get(*below), row.included.get(*above));
                assert!(
                    reach(upper, row.unit) >= reach(lower, row.unit),
                    "{:?}: {} has {upper:?} but {} has {lower:?}",
                    row.key,
                    above.as_str(),
                    below.as_str()
                );
            }
        }
    }

    /// The matrix is one row per key, grouped in reading order, with numbers
    /// only where the row says how to read them.
    #[test]
    fn the_feature_matrix_is_consistent() {
        for key in FeatureKey::ALL {
            match key {
                FeatureKey::Moves
                | FeatureKey::MovesRollover
                | FeatureKey::CopyOrMove
                | FeatureKey::EditSync
                | FeatureKey::Scheduling
                | FeatureKey::AutoPublishRules
                | FeatureKey::TermAndPriceRules
                | FeatureKey::Resources
                | FeatureKey::Storage
                | FeatureKey::Import
                | FeatureKey::DuplicateReview
                | FeatureKey::RichText
                | FeatureKey::WatermarkedPreviews
                | FeatureKey::Templates
                | FeatureKey::Collections
                | FeatureKey::Labels
                | FeatureKey::Export
                | FeatureKey::Analytics
                | FeatureKey::AiFill
                | FeatureKey::DesktopApp
                | FeatureKey::EmailSupport
                | FeatureKey::PrioritySupport => {}
            }
            assert_eq!(
                PLAN_FEATURES.iter().filter(|row| row.key == key).count(),
                1,
                "{key:?} must be exactly one row of the comparison"
            );
        }
        let groups: Vec<FeatureGroup> = PLAN_FEATURES.iter().map(|row| row.group).collect();
        let mut order = groups.clone();
        order.dedup();
        assert_eq!(
            order,
            FeatureGroup::ALL.to_vec(),
            "rows must sit together under their group, groups in reading order, none empty"
        );
        for row in PLAN_FEATURES {
            assert!(!row.label.trim().is_empty(), "{:?} has no label", row.key);
            for plan in Plan::ALL {
                let cell = row.included.get(plan);
                if row.unit.is_none() {
                    assert!(
                        matches!(cell, Included::Flag(_)),
                        "{:?} is a yes-or-no row but {} carries a number",
                        row.key,
                        plan.as_str()
                    );
                }
            }
            assert_eq!(
                row.soon,
                row.key == FeatureKey::AiFill,
                "only the AI fill is sold before it is built; {:?} says otherwise",
                row.key
            );
        }
        // Core: what a trial has to show and no gate withholds.
        for key in [
            FeatureKey::CopyOrMove,
            FeatureKey::Import,
            FeatureKey::DuplicateReview,
            FeatureKey::RichText,
            FeatureKey::TermAndPriceRules,
            FeatureKey::Export,
            FeatureKey::DesktopApp,
        ] {
            for plan in Plan::ALL {
                assert_eq!(
                    key.included(&plan.capabilities(None)),
                    Included::Flag(true),
                    "{key:?} is core and must be on {}",
                    plan.as_str()
                );
            }
        }
        // Counted, but never withheld: every plan makes some previews, Look
        // from its lifetime allowance, whose cell is the dash a comparison
        // table replaces with the lifetime figure.
        for plan in Plan::ALL {
            assert!(
                plan.capabilities(None).previews().cap() > 0,
                "{} must be able to make a watermarked preview",
                plan.as_str()
            );
        }
    }

    /// Every capability, as a score on which a stronger plan must never be
    /// lower. The destructure names every field with no `..`, so a new
    /// capability is a compile error here until someone says how it climbs.
    fn cap_reach(caps: Capabilities) -> Vec<(&'static str, u64)> {
        let Capabilities {
            resources_max,
            marketplaces_max,
            storage_bytes_max,
            import_spreadsheet,
            import_marketplace,
            duplicate_review,
            publish_marketplaces_max,
            moves_per_month,
            moves_accrual_cap,
            // Descends by design: the trial's five moves are Look's alone,
            // and `only_the_free_plan_carries_the_lifetime_moves` pins it.
            free_moves_lifetime: _,
            // Descends for the same reason: Look's five lifetime previews
            // are its whole allowance, and
            // `only_look_counts_its_previews_for_life_and_no_plan_is_unlimited`
            // pins it.
            previews_lifetime: _,
            pack_edit_days,
            scheduling,
            sync_pull_interval_secs,
            auto_publish_rules,
            templates_max,
            collections_max,
            labels_max,
            analytics,
            export,
            devices_max,
            ai_fills_per_month,
            previews_per_month,
            uploads_in_flight_max,
            support,
        } = caps;
        vec![
            ("resources_max", u64::from(resources_max)),
            ("marketplaces_max", u64::from(marketplaces_max)),
            ("storage_bytes_max", storage_bytes_max),
            ("import_spreadsheet", u64::from(import_spreadsheet)),
            ("import_marketplace", u64::from(import_marketplace)),
            ("duplicate_review", u64::from(duplicate_review)),
            (
                "publish_marketplaces_max",
                u64::from(publish_marketplaces_max),
            ),
            ("moves_per_month", u64::from(moves_per_month)),
            ("moves_accrual_cap", u64::from(moves_accrual_cap)),
            ("pack_edit_days", u64::from(pack_edit_days)),
            ("scheduling", u64::from(scheduling)),
            (
                "sync_pull_interval_secs",
                sync_pull_interval_secs.map_or(0, |secs| u64::MAX - u64::from(secs)),
            ),
            ("auto_publish_rules", u64::from(auto_publish_rules)),
            ("templates_max", u64::from(templates_max)),
            ("collections_max", u64::from(collections_max)),
            ("labels_max", u64::from(labels_max)),
            ("analytics", u64::from(analytics)),
            ("export", u64::from(export)),
            ("devices_max", u64::from(devices_max)),
            ("ai_fills_per_month", u64::from(ai_fills_per_month)),
            ("previews_per_month", u64::from(previews_per_month)),
            ("uploads_in_flight_max", u64::from(uploads_in_flight_max)),
            (
                "support",
                match support {
                    Support::Guides => 0,
                    Support::Email2Days => 1,
                    Support::Email1Day => 2,
                },
            ),
        ]
    }

    /// No capability gets worse up the ladder, whether or not the comparison
    /// table shows it: an upgrade that lost anything would be a downgrade in
    /// a row the seller never saw.
    #[test]
    fn every_cap_climbs_the_ladder() {
        for (below, above) in Plan::ALL.iter().zip(Plan::ALL.iter().skip(1)) {
            let lower = cap_reach(below.capabilities(None));
            let upper = cap_reach(above.capabilities(None));
            for ((name, low), (_, high)) in lower.iter().zip(&upper) {
                assert!(
                    high >= low,
                    "{name}: {} grants less than {}",
                    above.as_str(),
                    below.as_str()
                );
            }
        }
    }

    #[test]
    fn every_edit_sync_interval_is_whole_hours() {
        for plan in Plan::ALL {
            if let Some(secs) = plan.capabilities(None).sync_pull_interval_secs {
                assert_eq!(secs % 3_600, 0, "{} syncs off the hour", plan.as_str());
            }
        }
    }

    /// The `UNCALIBRATED` markers are a countdown, not decoration.
    ///
    /// Pinning the count makes removing a marker a deliberate edit and makes
    /// adding one impossible to do quietly. Drive the number to zero before the
    /// first paying deployment; that is what makes the marker a release
    /// blocker rather than a wish.
    #[test]
    fn uncalibrated_markers_are_ratcheted() {
        let marked = include_str!("lib.rs")
            .lines()
            .filter(|line| line.trim_start().starts_with("/// UNCALIBRATED"))
            .count();
        assert_eq!(
            marked, UNCALIBRATED_BUDGET,
            "the uncalibrated-constant budget moved; lower it deliberately or calibrate the number"
        );
    }

    /// Counts doc-comment markers only, so prose and assertion messages that
    /// mention the marker do not inflate it.
    const UNCALIBRATED_BUDGET: usize = 0;

    #[test]
    fn all_is_total_over_the_enum() {
        for plan in Plan::ALL {
            match plan {
                Plan::Free | Plan::Starter | Plan::Subscriber | Plan::Studio => {}
            }
        }
        assert_eq!(
            Plan::ALL.len(),
            4,
            "a variant was added to Plan without being added to Plan::ALL"
        );
        for support in Support::ALL {
            match support {
                Support::Guides | Support::Email2Days | Support::Email1Day => {}
            }
        }
        for key in PriceKey::ALL {
            match key {
                PriceKey::StarterMonthly
                | PriceKey::StarterYearly
                | PriceKey::ProMonthly
                | PriceKey::ProYearly
                | PriceKey::StudioMonthly
                | PriceKey::StudioYearly
                | PriceKey::Pack20
                | PriceKey::Pack50
                | PriceKey::Pack100
                | PriceKey::Pack250
                | PriceKey::Pack500 => {}
            }
        }
        assert_eq!(PLANS.len(), Plan::ALL.len(), "every plan needs a price row");
        for plan in Plan::ALL {
            assert!(
                PLANS.iter().any(|row| row.id == plan),
                "{} has no row in PLANS, so no surface can price or name it",
                plan.as_str()
            );
        }
    }

    /// The ladder has to climb, or the plans are four names for one
    /// product. It climbs on three counted axes: moves, resources and
    /// watermarked previews, each strictly larger on every stronger plan.
    #[test]
    fn every_plan_grants_strictly_more_of_each_counted_axis_than_the_one_below() {
        let axes: [(&str, fn(Capabilities) -> u32); 3] = [
            ("moves_per_month", |caps| caps.moves_per_month),
            ("resources_max", |caps| caps.resources_max),
            ("previews_per_month", |caps| caps.previews_per_month),
        ];
        for (axis, read) in axes {
            let values: Vec<u32> = Plan::ALL
                .iter()
                .map(|plan| read(plan.capabilities(None)))
                .collect();
            for pair in values.windows(2) {
                assert!(pair[1] > pair[0], "{axis} must increase: {pair:?}");
            }
        }
        // The founder's floor: the free plan starts at a hundred resources.
        assert_eq!(Plan::Free.capabilities(None).resources_max, 100);
    }

    /// Previews are counted in exactly one window per plan: Look's five for
    /// the account's life, every paid plan a month at a time, and none of
    /// them without a ceiling (founder, 2026-09-30).
    #[test]
    fn only_look_counts_its_previews_for_life_and_no_plan_is_unlimited() {
        assert_eq!(
            Plan::Free.capabilities(None).previews(),
            PreviewAllowance::Lifetime(5)
        );
        assert_eq!(Plan::Free.capabilities(None).previews_per_month, 0);
        for plan in [Plan::Starter, Plan::Subscriber, Plan::Studio] {
            let caps = plan.capabilities(None);
            assert_eq!(
                caps.previews_lifetime,
                0,
                "{} counts per month",
                plan.as_str()
            );
            assert!(
                matches!(caps.previews(), PreviewAllowance::Monthly(cap) if cap < u32::MAX),
                "{} must count previews a month against a ceiling",
                plan.as_str()
            );
        }
    }

    /// Five moves once, on the free plan and nowhere else: a plan that is
    /// paid for already carries an allowance, and granting the lifetime five
    /// on top of it would be a second free tier inside a paid one.
    #[test]
    fn only_the_free_plan_carries_the_lifetime_moves() {
        assert_eq!(Plan::Free.capabilities(None).free_moves_lifetime, 5);
        for plan in [Plan::Starter, Plan::Subscriber, Plan::Studio] {
            assert_eq!(
                plan.capabilities(None).free_moves_lifetime,
                0,
                "{} pays for its moves",
                plan.as_str()
            );
        }
        for plan in Plan::ALL {
            let caps = plan.capabilities(None);
            assert_eq!(
                caps.pack_edit_days,
                90,
                "{} must honour the pack's edit window; a pack is bought on any plan",
                plan.as_str()
            );
            assert_eq!(
                caps.devices_max,
                5,
                "{} covers five computers; the figure is the same everywhere and is never shown",
                plan.as_str()
            );
            assert!(
                caps.moves_accrual_cap >= caps.moves_per_month,
                "{} accrues to less than one period's allowance, so the credit is lost on \
                 arrival",
                plan.as_str()
            );
        }
    }

    /// An operator may lift Studio's monthly allowance and may never lower
    /// it, which is the whole of what a rung means now that packs are a
    /// balance rather than a plan.
    #[test]
    fn a_studio_rung_only_ever_raises_the_monthly_allowance() {
        assert_eq!(Plan::Studio.capabilities(None).moves_per_month, 100);
        assert_eq!(Plan::Studio.capabilities(Some(20)).moves_per_month, 100);
        assert_eq!(Plan::Studio.capabilities(Some(400)).moves_per_month, 400);
        for plan in [Plan::Free, Plan::Starter, Plan::Subscriber] {
            assert_eq!(
                plan.capabilities(Some(500)).moves_per_month,
                plan.capabilities(None).moves_per_month,
                "{} reads no rung: a pack is a ledger credit, not a capability",
                plan.as_str()
            );
        }
    }

    /// The packs are a ladder, and the per-move figure on each is the one
    /// the page compares them by rather than a fourth number to maintain.
    #[test]
    fn packs_are_a_ladder_priced_per_move() {
        assert!(
            PACKS.windows(2).all(|pair| pair[1].moves > pair[0].moves
                && pair[1].price_cents > pair[0].price_cents
                && pair[1].per_move_cents < pair[0].per_move_cents),
            "a ladder whose price does not climb with its volume, or whose per-move rate does \
             not fall, is not a ladder"
        );
        for pack in PACKS {
            assert_eq!(
                pack.per_move_cents * pack.moves + pack.price_cents % pack.moves,
                pack.price_cents,
                "the {} pack's per-move rate disagrees with its own price",
                pack.key.as_str()
            );
            assert!(
                !pack.key.recurring(),
                "a pack is bought once; {} says otherwise",
                pack.key.as_str()
            );
        }
    }

    /// A paid row is bought by exactly its own two keys, one per cadence,
    /// and every recurring key belongs to some row: a key granting a plan the
    /// row does not name would sell one tier at another's price.
    #[test]
    fn every_paid_row_names_the_keys_that_grant_it() {
        for row in PLANS {
            let priced = row.monthly_cents.is_some();
            assert_eq!(
                priced,
                row.yearly_cents.is_some(),
                "{} names one recurring price and not the other",
                row.id.as_str()
            );
            assert_eq!(priced, row.id != Plan::Free, "only Free charges nothing");
            assert_eq!(row.monthly_key.is_some(), priced);
            assert_eq!(row.yearly_key.is_some(), priced);
            for key in [row.monthly_key, row.yearly_key].into_iter().flatten() {
                assert_eq!(
                    key.plan(),
                    Some(row.id),
                    "{} is on the {} row but grants another plan",
                    key.as_str(),
                    row.id.as_str()
                );
            }
        }
        for key in PriceKey::ALL.into_iter().filter(|key| key.recurring()) {
            assert!(
                PLANS
                    .iter()
                    .any(|row| row.monthly_key == Some(key) || row.yearly_key == Some(key)),
                "{} is recurring but no row sells it",
                key.as_str()
            );
        }
    }

    #[test]
    fn every_key_has_a_list_price() {
        for key in PriceKey::ALL {
            assert!(key.list_cents() > 0, "{} has no list price", key.as_str());
        }
        assert_eq!(PriceKey::ProYearly.list_cents(), 24_000);
        assert_eq!(PriceKey::Pack100.list_cents(), 12_700);
    }

    /// Each paid rung costs more than the one below at both cadences, and
    /// never charges more per move than the rung below it, so upgrading is
    /// never the worse deal on the axis the ladder climbs on.
    #[test]
    fn every_paid_rung_costs_more_and_no_more_per_move() {
        let paid: Vec<_> = PLANS.iter().filter(|row| row.id != Plan::Free).collect();
        for (lower, upper) in paid.iter().zip(paid.iter().skip(1)) {
            assert!(upper.monthly_cents > lower.monthly_cents);
            assert!(upper.yearly_cents > lower.yearly_cents);
            // Compared cross-multiplied, so no cents are lost to rounding.
            let monthly = |row: &super::PlanRow| row.monthly_cents.unwrap_or(0);
            let moves = |row: &super::PlanRow| row.id.capabilities(None).moves_per_month;
            assert!(
                monthly(upper) * moves(lower) <= monthly(lower) * moves(upper),
                "{} charges more per move than {}",
                upper.id.as_str(),
                lower.id.as_str()
            );
        }
        for row in paid {
            let monthly = row.monthly_cents.unwrap_or(0);
            let yearly = row.yearly_cents.unwrap_or(0);
            assert!(
                yearly < monthly * 12,
                "{} charges no less for paying a year up front",
                row.id.as_str()
            );
        }
    }

    /// The one capability the founder ruled is never gated.
    #[test]
    fn export_is_granted_on_every_plan() {
        for plan in Plan::ALL {
            assert!(
                plan.capabilities(Some(20)).export,
                "{} must be able to get its catalogue out",
                plan.as_str()
            );
        }
    }

    /// Precedence is what decides which of several unexpired grants an
    /// organisation is served under, so it must be a strict order rather than
    /// an artefact of declaration order.
    #[test]
    fn the_strength_order_is_strict_and_free_is_the_floor() {
        let mut strengths: Vec<u8> = Plan::ALL.iter().map(|plan| plan.strength()).collect();
        let ordered = strengths.clone();
        strengths.sort_unstable();
        strengths.dedup();
        assert_eq!(
            strengths, ordered,
            "Plan::ALL is ordered weakest first and no two plans tie"
        );
        assert_eq!(Plan::Free.strength(), 0, "no grant is the weakest position");
    }

    /// The spelling crosses the wire, the database CHECK and the generated
    /// client, so the three renderings have to agree.
    #[test]
    fn a_plans_spelling_round_trips_through_its_wire_name() {
        for plan in Plan::ALL {
            assert_eq!(Plan::parse(plan.as_str()), Some(plan));
            assert_eq!(
                serde_json::to_string(&plan).expect("a plan serialises"),
                format!("\"{}\"", plan.as_str()),
                "serde and as_str must spell a plan the same way"
            );
        }
        assert_eq!(
            Plan::parse("pro"),
            None,
            "a spelling this build does not know is refused rather than downgraded"
        );
        for support in Support::ALL {
            assert_eq!(
                serde_json::to_string(&support).expect("a support level serialises"),
                format!("\"{}\"", support.as_str())
            );
        }
        for key in PriceKey::ALL {
            assert_eq!(PriceKey::parse(key.as_str()), Some(key));
            assert_eq!(
                serde_json::to_string(&key).expect("a price key serialises"),
                format!("\"{}\"", key.as_str()),
                "the price map is keyed on this spelling on both sides of the wire"
            );
        }
        assert_eq!(
            PriceKey::parse("rung_50"),
            None,
            "the ladder's old spelling names nothing this build sells"
        );
        for (cell, wire) in [
            (Included::Flag(false), "false"),
            (Included::Flag(true), "true"),
            (Included::Limit(25), "25"),
        ] {
            assert_eq!(
                serde_json::to_string(&cell).expect("a cell serialises"),
                wire,
                "a comparison cell crosses the wire as a bare boolean or number"
            );
        }
    }

    #[test]
    fn the_wall_clock_deadline_outlives_the_full_retry_budget_at_the_backoff_base() {
        let base = std::time::Duration::from_millis(500);
        let worst = base * job::ATTEMPTS_MAX;
        assert!(
            job::WALL_CLOCK_MAX > worst,
            "the deadline must not fire before the retries are exhausted"
        );
    }

    #[test]
    fn a_maximal_upload_expanding_at_the_maximal_ratio_exceeds_the_absolute_cap() {
        let expanded = http::UPLOAD_BODY_BYTES_MAX * ingest::ARCHIVE_COMPRESSION_RATIO_MAX;
        assert!(
            expanded > ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
            "the absolute cap is the binding one for a maximal upload"
        );
    }
}
