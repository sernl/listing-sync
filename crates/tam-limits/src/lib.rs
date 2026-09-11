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
/// `Studio` exists in code and is never sold: decisions.md, "Plans,
/// capabilities and the pricing re-evaluation, 2026-09-12" defers it until a
/// fifth of subscribers exceed 300 resources or hit the migration cap twice
/// in a quarter. Declaring it now is what makes that trigger a row in
/// [`PLANS`] with `sold: false` rather than a second pricing model invented
/// under pressure.
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
    Subscriber,
    MigrationOnly,
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
    pub const ALL: [Self; 4] = [
        Self::Free,
        Self::MigrationOnly,
        Self::Subscriber,
        Self::Studio,
    ];

    /// The wire spelling, which is also the spelling the `plan` CHECK
    /// constraint in migration 0069 enumerates.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Subscriber => "subscriber",
            Self::MigrationOnly => "migration_only",
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
    /// typed. Studio outranks Subscriber outranks MigrationOnly outranks
    /// Free: a one-off import bought by an organisation that later subscribes
    /// must not hold the subscription down to the one-off's narrower set.
    #[must_use]
    pub const fn strength(self) -> u8 {
        match self {
            Self::Free => 0,
            Self::MigrationOnly => 1,
            Self::Subscriber => 2,
            Self::Studio => 3,
        }
    }

    /// What this plan grants, at the rung it was bought at.
    ///
    /// DECIDED (decisions.md, "Plans, capabilities and the pricing
    /// re-evaluation, 2026-09-12", against the gating matrix in
    /// `docs/notes/design/research/2026-09-12-pricing-and-tiers.md` section
    /// 6): every figure here is the founder's, carried unchanged. The
    /// migration cap re-opens on the first subscriber who hits it twice in a
    /// quarter, which is also the trigger that ships `Studio`.
    ///
    /// `rung` is read only by `MigrationOnly`, whose whole product is the
    /// volume bought; every other plan ignores it. A `MigrationOnly` grant
    /// with no rung grants nothing, which is the honest reading of a purchase
    /// whose price we could not map: the buyer is refused and support can see
    /// why, rather than silently receiving the largest rung.
    #[must_use]
    pub const fn capabilities(self, rung: Option<u32>) -> Capabilities {
        match self {
            Self::Free => Capabilities {
                resources_max: 20,
                marketplaces_max: 1,
                storage_bytes_max: 1 << 30,
                import_spreadsheet: true,
                import_marketplace: false,
                duplicate_review: false,
                publish_marketplaces_max: 1,
                edit_days_after_purchase: None,
                migrations_per_month: 0,
                scheduling: false,
                sync_pull_interval_secs: None,
                auto_publish_rules: false,
                templates_max: 1,
                collections_max: 0,
                labels_max: 5,
                analytics: false,
                export: true,
                devices_max: 1,
                ai_fills_per_month: 0,
                support: Support::Guides,
            },
            Self::Subscriber => Capabilities {
                resources_max: 400,
                marketplaces_max: u32::MAX,
                storage_bytes_max: 20 << 30,
                import_spreadsheet: true,
                import_marketplace: true,
                duplicate_review: true,
                publish_marketplaces_max: u32::MAX,
                edit_days_after_purchase: None,
                migrations_per_month: 20,
                scheduling: true,
                sync_pull_interval_secs: Some(6 * 3_600),
                auto_publish_rules: true,
                templates_max: 20,
                collections_max: 20,
                labels_max: 20,
                analytics: true,
                export: true,
                devices_max: 2,
                ai_fills_per_month: 200,
                support: Support::Email2Days,
            },
            // One publish pass over the imported set, and thirty days in
            // which to correct it. `publish_marketplaces_max` is every
            // marketplace because a move has two sides; what bounds the pass
            // is the rung, which is also the migration allowance.
            Self::MigrationOnly => {
                let bought = match rung {
                    Some(bought) => bought,
                    None => 0,
                };
                Capabilities {
                    resources_max: bought,
                    marketplaces_max: u32::MAX,
                    storage_bytes_max: 5 << 30,
                    import_spreadsheet: true,
                    import_marketplace: true,
                    duplicate_review: true,
                    publish_marketplaces_max: u32::MAX,
                    edit_days_after_purchase: Some(30),
                    migrations_per_month: bought,
                    scheduling: false,
                    sync_pull_interval_secs: None,
                    auto_publish_rules: false,
                    templates_max: 1,
                    collections_max: 0,
                    labels_max: 0,
                    analytics: false,
                    export: true,
                    devices_max: 1,
                    ai_fills_per_month: 0,
                    support: Support::Email30DaysAfterPurchase,
                }
            }
            Self::Studio => Capabilities {
                resources_max: u32::MAX,
                marketplaces_max: u32::MAX,
                storage_bytes_max: 200 << 30,
                import_spreadsheet: true,
                import_marketplace: true,
                duplicate_review: true,
                publish_marketplaces_max: u32::MAX,
                edit_days_after_purchase: None,
                migrations_per_month: 100,
                scheduling: true,
                sync_pull_interval_secs: Some(3_600),
                auto_publish_rules: true,
                templates_max: u32::MAX,
                collections_max: u32::MAX,
                labels_max: 50,
                analytics: true,
                export: true,
                devices_max: 3,
                ai_fills_per_month: 600,
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
    #[serde(rename = "email_30_days_after_purchase")]
    Email30DaysAfterPurchase,
}

impl Support {
    /// Every level, for the same reason [`Plan::ALL`] exists.
    pub const ALL: [Self; 4] = [
        Self::Guides,
        Self::Email2Days,
        Self::Email1Day,
        Self::Email30DaysAfterPurchase,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Guides => "guides",
            Self::Email2Days => "email_2_days",
            Self::Email1Day => "email_1_day",
            Self::Email30DaysAfterPurchase => "email_30_days_after_purchase",
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
/// match into each of the nine gate sites. The two genuinely absent
/// quantities are `Option`: no edit deadline, and no sync cadence at all.
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
    /// How long after the purchase a marketplace listing may still be edited
    /// or deleted. `None` is unlimited, which is every recurring plan.
    pub edit_days_after_purchase: Option<u32>,
    /// Resources, not batches: the cap counts the resources a month's copies
    /// and moves name, because a per-batch cap is gamed by batching.
    pub migrations_per_month: u32,
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
    pub support: Support,
}

/// One row of the price list.
///
/// `monthly_cents` and `yearly_cents` are absent for the two plans that carry
/// no recurring price: Free, which charges nothing, and Catalogue Import,
/// which is priced by the rung ladder below rather than by the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRow {
    pub id: Plan,
    pub name: &'static str,
    pub monthly_cents: Option<u32>,
    pub yearly_cents: Option<u32>,
    pub trial_days: u32,
    /// Whether a checkout may route to this plan. False for Studio, which is
    /// priced and deferred: the figures are published so the trigger has
    /// something to ship, and no surface offers it.
    pub sold: bool,
}

/// One rung of the Catalogue Import ladder: a resource ceiling and its price.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rung {
    pub up_to: u32,
    pub price_cents: u32,
}

/// The Founding 100 overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Founding {
    pub discount_year_one_pct: u32,
    pub discount_ongoing_pct: u32,
    /// How many years the ongoing discount runs before it lapses, capped by
    /// the founder's decision of 2026-09-12.
    pub ongoing_years: u32,
    pub free_imports: u32,
    pub places: u32,
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
/// DECIDED (decisions.md, "Plans, capabilities and the pricing re-evaluation,
/// 2026-09-12"): every price the founder set on 2026-09-11 stands. Re-opened
/// by the Studio trigger, which is the only pending pricing decision.
pub const PLANS: [PlanRow; 4] = [
    PlanRow {
        id: Plan::Free,
        name: "Free",
        monthly_cents: None,
        yearly_cents: None,
        trial_days: 0,
        sold: true,
    },
    PlanRow {
        id: Plan::Subscriber,
        name: "Teachouse Subscription",
        monthly_cents: Some(2_400),
        yearly_cents: Some(24_000),
        trial_days: 14,
        sold: true,
    },
    PlanRow {
        id: Plan::MigrationOnly,
        name: "Catalogue Import",
        monthly_cents: None,
        yearly_cents: None,
        trial_days: 0,
        sold: true,
    },
    PlanRow {
        id: Plan::Studio,
        name: "Studio",
        monthly_cents: Some(4_400),
        yearly_cents: Some(44_000),
        trial_days: 0,
        sold: false,
    },
];

/// The one-off Catalogue Import ladder, cheapest rung first.
///
/// DECIDED (decisions.md, 2026-09-12): the founder's four rungs plus the two
/// the research added, because the measured dual-lister holds about 764
/// listings and the published ladder stopped short of its best customer. A
/// rung counts resources committed to the catalogue after duplicate merges.
pub const IMPORT_LADDER: [Rung; 5] = [
    Rung {
        up_to: 20,
        price_cents: 4_700,
    },
    Rung {
        up_to: 50,
        price_cents: 7_700,
    },
    Rung {
        up_to: 100,
        price_cents: 12_700,
    },
    Rung {
        up_to: 250,
        price_cents: 24_700,
    },
    Rung {
        up_to: 500,
        price_cents: 39_700,
    },
];

/// What a catalogue above the top rung is offered: a conversation, not a
/// price. Carried here rather than written into the page's copy so the server
/// and the two clients say the same words.
pub const LADDER_ABOVE: &str = "Talk to us";

/// The Founding 100 offer, whose four numbers the founder kept unchanged.
pub const FOUNDING: Founding = Founding {
    discount_year_one_pct: 25,
    discount_ongoing_pct: 20,
    ongoing_years: 3,
    free_imports: 20,
    places: 100,
};

/// The AI auto-fill packaging: bundled with a fair-use cap and a small
/// add-on, no credit currency.
pub const AI: AiOffer = AiOffer {
    status: AiStatus::ComingSoon,
    included_fills: 200,
    add_on_fills: 100,
    add_on_cents: 500,
};

pub mod http {
    /// SIZED against RAM, not against traffic: at this ceiling the concurrent
    /// request limit cannot buffer more than a small fraction of the box's
    /// memory. Applies to every route except the ingestion upload routes.
    /// Revisit once the spike records real listing-metadata payload sizes.
    pub const REQUEST_BODY_BYTES_MAX: u64 = 2 * 1024 * 1024;

    /// MEASURED against the Tes per-file ceiling recorded in the M-1 outcomes
    /// (decisions.md): supported files up to 200 MB. 256 MB admits the largest
    /// Tes-legal file with multipart headroom.
    pub const UPLOAD_BODY_BYTES_MAX: u64 = 256 * 1024 * 1024;
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

    /// DECIDED (decisions.md, "Limits calibration, 2026-08-28"): held at the
    /// serialised end until the per-job RAM footprint alongside the connection
    /// pool is measured, which re-opens it. Automation runs here, on our own
    /// infrastructure, so it is inside this bound rather than outside it.
    pub const CONCURRENT_JOBS_GLOBAL_MAX: u32 = 8;
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
    use super::{http, ingest, job, Plan, Support, IMPORT_LADDER, PLANS};

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
                Plan::Free | Plan::Subscriber | Plan::MigrationOnly | Plan::Studio => {}
            }
        }
        assert_eq!(
            Plan::ALL.len(),
            4,
            "a variant was added to Plan without being added to Plan::ALL"
        );
        for support in Support::ALL {
            match support {
                Support::Guides
                | Support::Email2Days
                | Support::Email1Day
                | Support::Email30DaysAfterPurchase => {}
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

    /// The recurring ladder has to climb, or the plans are three names for
    /// one product. `MigrationOnly` is left out on purpose: its allowance is
    /// the rung bought rather than a place on this ladder.
    #[test]
    fn every_recurring_plan_grants_strictly_more_resources_than_the_one_below() {
        let ladder = [Plan::Free, Plan::Subscriber, Plan::Studio];
        let allowances: Vec<u32> = ladder
            .iter()
            .map(|plan| plan.capabilities(None).resources_max)
            .collect();
        for pair in allowances.windows(2) {
            assert!(
                pair[1] > pair[0],
                "resource allowances must increase: {pair:?}"
            );
        }
    }

    /// A rung is the whole product `migration_only` sells, so the capability
    /// derivation must carry it rather than round it to a tier.
    #[test]
    fn a_catalogue_import_grants_exactly_the_rung_it_was_bought_at() {
        for rung in IMPORT_LADDER {
            let caps = Plan::MigrationOnly.capabilities(Some(rung.up_to));
            assert_eq!(
                caps.resources_max, rung.up_to,
                "the {} rung must grant {} resources",
                rung.price_cents, rung.up_to
            );
            assert_eq!(
                caps.migrations_per_month, rung.up_to,
                "the rung is also the one migration pass it pays for"
            );
        }
        assert_eq!(
            Plan::MigrationOnly.capabilities(None).resources_max,
            0,
            "a one-off purchase whose price we could not map grants nothing, \
             rather than silently granting the largest rung"
        );
    }

    /// A plan a checkout can route to must have somewhere to route: either a
    /// recurring price on its row or the one-off ladder. Studio is the one
    /// row carrying a price nothing sells, which is what deferring it means.
    #[test]
    fn every_sold_plan_names_a_price_and_the_only_unsold_one_is_studio() {
        for row in PLANS {
            let priced = row.monthly_cents.is_some();
            assert_eq!(
                priced,
                row.yearly_cents.is_some(),
                "{} names one recurring price and not the other",
                row.id.as_str()
            );
            if row.sold {
                let reachable = priced || row.id == Plan::Free || row.id == Plan::MigrationOnly;
                assert!(
                    reachable,
                    "{} is sold but no price names it",
                    row.id.as_str()
                );
            } else {
                assert_eq!(
                    row.id,
                    Plan::Studio,
                    "Studio is the only deferred plan; anything else unsold is a pricing \
                     decision that never reached decisions.md"
                );
            }
        }
        assert!(
            PLANS.iter().filter(|row| !row.sold).count() == 1,
            "exactly one plan is deferred"
        );
        assert!(
            IMPORT_LADDER
                .windows(2)
                .all(|pair| pair[1].up_to > pair[0].up_to
                    && pair[1].price_cents > pair[0].price_cents),
            "a ladder whose price does not climb with its volume is not a ladder"
        );
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
