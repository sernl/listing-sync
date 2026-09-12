//! What a plan grants, read once per request and enforced at every write it
//! bounds.
//!
//! The plan is not a column and not a Paddle status. It is the strongest
//! unexpired row in `entitlement_grant`, read by the [`OrgContext`] extractor
//! before any handler runs and carried on the context as a [`Capabilities`]
//! value. That is one extra indexed read per authenticated request, and it
//! buys the property the design asks for: the pricing page, the Account page
//! and every gate answer from one table, so no two surfaces can disagree
//! about what a plan holds.
//!
//! This module replaces `quota.rs`, which derived two numbers from the
//! recorded Paddle subscription because no plan existed to read. Deriving an
//! entitlement from a billing status was always a stand-in — it could not
//! express a one-off purchase, an operator grant or an expiry — and the
//! subscription table goes back to being what its migration says it is: a
//! record of what Paddle said.
//!
//! Every refusal here is the same 422 the two original quota refusals were,
//! carrying `detail.quota` naming the bound and, for a capability that is
//! absent rather than exhausted, `detail.feature` naming it. The sentences
//! are the seller's: what their plan includes, and what to do about it.
//!
//! [`OrgContext`]: crate::OrgContext

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_limits::{
    AiOffer, Capabilities, Founding, Plan, PlanRow, Rung, AI, FOUNDING, IMPORT_LADDER,
    LADDER_ABOVE, PLANS,
};
use tam_storage::{EntitlementRepo, Grant, Usage};
use tam_types::Timestamp;

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::version::APIVersion;
use crate::{AppState, OrgContext};

/// The entitlement one request runs under: the grant it came from, and what
/// that grant allows.
///
/// Both halves, because the two answer different questions. The gates read
/// `caps`; the Account page and the "Set by Teachouse until <date>" banner
/// read `grant`, which is the only thing that can say where an entitlement
/// came from and when it lapses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    pub grant: Grant,
    pub caps: Capabilities,
}

impl Entitlement {
    /// The entitlement an organisation with no live grant runs under.
    #[must_use]
    pub fn free() -> Self {
        Self::of(Grant::free())
    }

    #[must_use]
    pub fn of(grant: Grant) -> Self {
        let caps = grant.plan.capabilities(grant.rung);
        Self { grant, caps }
    }
}

/// Which bound was reached, so a refusal names one rather than saying
/// "quota".
///
/// `listings_max` and `storage_bytes_max` keep the spellings they have had
/// since the first two gates shipped, even though the capability behind the
/// first is now called `resources_max`: the string is a wire vocabulary the
/// console already branches on, and renaming it would be a client break for
/// no gain. The new members are named for the capability they bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaKind {
    Listings,
    StorageBytes,
    Marketplaces,
    MigrationsPerMonth,
    Templates,
    Labels,
    Collections,
    Devices,
    /// The plan does not include the capability at all, rather than having
    /// run out of it. `detail.feature` names which one.
    PlanFeature,
}

impl QuotaKind {
    pub const ALL: [Self; 9] = [
        Self::Listings,
        Self::StorageBytes,
        Self::Marketplaces,
        Self::MigrationsPerMonth,
        Self::Templates,
        Self::Labels,
        Self::Collections,
        Self::Devices,
        Self::PlanFeature,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Listings => "listings_max",
            Self::StorageBytes => "storage_bytes_max",
            Self::Marketplaces => "marketplaces_max",
            Self::MigrationsPerMonth => "migrations_per_month",
            Self::Templates => "templates_max",
            Self::Labels => "labels_max",
            Self::Collections => "collections_max",
            Self::Devices => "devices_max",
            Self::PlanFeature => "plan_feature",
        }
    }

    /// What the seller is told, in their words: what the plan includes, and
    /// the one thing they can do about it.
    ///
    /// A sentence per bound rather than one sentence with the bound's name
    /// substituted in, because "this plan's listing quota is full" is a
    /// sentence about our schema and "Your plan includes 20 resources" is a
    /// sentence about their catalogue.
    #[must_use]
    pub fn sentence(self, limit: u64) -> String {
        match self {
            Self::Listings => {
                format!("Your plan includes {limit} resources. Upgrade to add more.")
            }
            Self::StorageBytes => format!(
                "Your plan includes {} GB of files. Upgrade to add more.",
                limit >> 30
            ),
            Self::Marketplaces if limit == 1 => {
                "Your plan connects one marketplace. Upgrade to connect more.".to_owned()
            }
            Self::Marketplaces => {
                format!("Your plan connects {limit} marketplaces. Upgrade to connect more.")
            }
            Self::MigrationsPerMonth if limit == 0 => {
                "Your plan does not include copying or moving resources. Upgrade to use it."
                    .to_owned()
            }
            Self::MigrationsPerMonth => {
                format!("Your plan copies or moves {limit} resources a month. Upgrade to do more.")
            }
            Self::Templates if limit == 0 => {
                "Your plan does not include templates. Upgrade to use them.".to_owned()
            }
            Self::Templates if limit == 1 => {
                "Your plan includes one template. Upgrade to save more.".to_owned()
            }
            Self::Templates => {
                format!("Your plan includes {limit} templates. Upgrade to save more.")
            }
            Self::Labels if limit == 0 => {
                "Your plan does not include labels. Upgrade to use them.".to_owned()
            }
            Self::Labels => format!("Your plan includes {limit} labels. Upgrade to add more."),
            Self::Collections if limit == 0 => {
                "Your plan does not include collections. Upgrade to use them.".to_owned()
            }
            Self::Collections if limit == 1 => {
                "Your plan includes one collection. Upgrade to make more.".to_owned()
            }
            Self::Collections => {
                format!("Your plan includes {limit} collections. Upgrade to make more.")
            }
            Self::Devices if limit == 1 => {
                "Your plan covers one computer. Upgrade to add another.".to_owned()
            }
            Self::Devices => {
                format!("Your plan covers {limit} computers. Upgrade to add another.")
            }
            Self::PlanFeature => "Your plan does not include this. Upgrade to use it.".to_owned(),
        }
    }
}

/// The 422 every bound answers with. One shape, so a client branches on
/// `detail.quota` rather than on the sentence.
#[must_use]
pub fn quota_refusal(kind: QuotaKind, used: i64, limit: u64) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(&kind.sentence(limit))
            .code(APIErrorCode::QuotaExceeded)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "quota": kind.as_str(),
                "used": used,
                "limit": limit,
            })),
    )
}

/// The refusal for a capability the plan does not hold at all.
///
/// `feature` is the `Capabilities` field name, so the console can map a
/// refusal back to the row of the pricing table that explains it without
/// parsing a sentence.
#[must_use]
pub fn feature_refusal(feature: &str, sentence: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(sentence)
            .code(APIErrorCode::QuotaExceeded)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "quota": QuotaKind::PlanFeature.as_str(),
                "feature": feature,
            })),
    )
}

/// The migration cap's own refusal, which carries what is left and when the
/// counter returns to zero.
///
/// A month's allowance is the one bound a seller can wait out, so the
/// sentence says when rather than only that they may not now.
#[must_use]
pub fn migration_refusal(used: i64, limit: u32, requested: i64, resets_at: Timestamp) -> APIError {
    let remaining = i64::from(limit).saturating_sub(used).max(0);
    let sentence = if limit == 0 {
        "Your plan does not include copying or moving resources. Upgrade to use it.".to_owned()
    } else if remaining == 0 {
        format!(
            "Your plan copies or moves {limit} resources a month, and this month's are used. \
             The next {limit} are available on {}.",
            day_of(resets_at)
        )
    } else {
        format!(
            "Your plan copies or moves {limit} resources a month. You have {remaining} left \
             this month and asked for {requested}; the next {limit} are available on {}.",
            day_of(resets_at)
        )
    };
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(&sentence)
            .code(APIErrorCode::QuotaExceeded)
            .kind(APIErrorKind::Validation)
            .detail(serde_json::json!({
                "quota": QuotaKind::MigrationsPerMonth.as_str(),
                "used": used,
                "limit": limit,
                "requested": requested,
                "resets_at": resets_at,
            })),
    )
}

/// The reset date as the sentence spells it: a day in UTC, which is the
/// boundary the counter actually turns on.
///
/// The civil-date arithmetic is written out rather than taken from a date
/// library, for the reason `paddle::instant_from_rfc3339` writes out the
/// forward direction: this crate parses and renders exactly two instants and
/// takes no calendar dependency for them. The algorithm is Howard Hinnant's
/// `civil_from_days`, the inverse of the one beside it.
fn day_of(at: Timestamp) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let (year, month, day) = civil_from_days(at.0.div_euclid(86_400_000));
    let name = MONTHS
        .get(usize::try_from(month).unwrap_or(1).saturating_sub(1))
        .copied()
        .unwrap_or("January");
    format!("{day} {name} {year}")
}

/// Days since 1970-01-01 to a civil year, month and day, proleptic
/// Gregorian.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era.div_euclid(1_460) + day_of_era.div_euclid(36_524)
        - day_of_era.div_euclid(146_096))
    .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era.div_euclid(4) - year_of_era.div_euclid(100));
    let shifted_month = (5 * day_of_year + 2).div_euclid(153);
    let day = day_of_year - (153 * shifted_month + 2).div_euclid(5) + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

// ------------------------------------------------------------- GET /v1/plans

/// One plan as the pricing page and the Account page read it: the row, and
/// what it grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRowView {
    pub id: Plan,
    pub name: String,
    pub monthly_cents: Option<u32>,
    pub yearly_cents: Option<u32>,
    pub trial_days: u32,
    pub sold: bool,
    pub capabilities: Capabilities,
}

impl PlanRowView {
    fn of(row: PlanRow) -> Self {
        Self {
            id: row.id,
            name: row.name.to_owned(),
            monthly_cents: row.monthly_cents,
            yearly_cents: row.yearly_cents,
            trial_days: row.trial_days,
            // At no rung, which is what a price list shows: a Catalogue
            // Import row's allowance is the rung the buyer picks from the
            // ladder below, so the row itself grants nothing until one is.
            capabilities: row.id.capabilities(None),
            sold: row.sold,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlansView {
    pub plans: Vec<PlanRowView>,
    pub import_ladder: Vec<Rung>,
    pub ladder_above: String,
    pub founding: Founding,
    pub ai: AiOffer,
}

/// The price list. Unauthenticated, because the pricing page is public and a
/// price a seller cannot read before signing up is not a price list.
pub(crate) async fn plans_view(_version: APIVersion) -> Json<PlansView> {
    Json(PlansView {
        plans: PLANS.into_iter().map(PlanRowView::of).collect(),
        import_ladder: IMPORT_LADDER.to_vec(),
        ladder_above: LADDER_ABOVE.to_owned(),
        founding: FOUNDING,
        ai: AI,
    })
}

// ------------------------------------------------------- GET /v1/entitlement

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageView {
    pub resources: i64,
    pub marketplaces: i64,
    pub migrations_this_month: i64,
    pub migrations_reset_at: Timestamp,
    pub templates: i64,
    pub labels: i64,
    pub collections: i64,
    pub devices: i64,
}

impl UsageView {
    fn of(usage: Usage) -> Self {
        Self {
            resources: usage.resources,
            marketplaces: usage.marketplaces,
            migrations_this_month: usage.migrations_this_month,
            migrations_reset_at: usage.migrations_reset_at,
            templates: usage.templates,
            labels: usage.labels,
            collections: usage.collections,
            devices: usage.devices,
        }
    }
}

/// What the Account page reads: the plan, where it came from, what it grants
/// and what has been used of it.
///
/// `granted_by` is absent for an organisation on Free with no grant at all,
/// because nobody granted it. That absence is what the console branches on to
/// decide between "Subscribe" and "Set by Teachouse until <date>".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementView {
    pub plan: Plan,
    pub rung: Option<u32>,
    pub granted_by: Option<String>,
    pub granted_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub capabilities: Capabilities,
    pub usage: UsageView,
}

pub(crate) async fn entitlement_view(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<EntitlementView>, APIError> {
    let usage = EntitlementRepo::new(state.pool.clone())
        .usage(context.org, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let grant = &context.entitlement.grant;
    Ok(Json(EntitlementView {
        plan: grant.plan,
        rung: grant.rung,
        granted_by: grant.granted_by.map(|by| by.as_str().to_owned()),
        granted_at: grant.granted_at,
        expires_at: grant.expires_at,
        capabilities: context.entitlement.caps,
        usage: UsageView::of(usage),
    }))
}

#[cfg(test)]
mod tests {
    use super::{day_of, QuotaKind};
    use tam_limits::Plan;
    use tam_types::Timestamp;

    #[test]
    fn every_bound_names_itself_on_the_wire_and_speaks_to_the_seller() {
        for kind in QuotaKind::ALL {
            match kind {
                QuotaKind::Listings
                | QuotaKind::StorageBytes
                | QuotaKind::Marketplaces
                | QuotaKind::MigrationsPerMonth
                | QuotaKind::Templates
                | QuotaKind::Labels
                | QuotaKind::Collections
                | QuotaKind::Devices
                | QuotaKind::PlanFeature => {}
            }
            let sentence = kind.sentence(20);
            assert!(
                sentence.starts_with("Your plan"),
                "{} says {sentence}, which is about us rather than about them",
                kind.as_str()
            );
            assert!(
                sentence.ends_with('.'),
                "{} does not finish its sentence",
                kind.as_str()
            );
        }
    }

    /// The free plan's own numbers reach the sentence, because a refusal that
    /// names the wrong ceiling sends a seller to the wrong page.
    #[test]
    fn the_refusal_quotes_the_plans_own_ceiling() {
        let free = Plan::Free.capabilities(None);
        assert_eq!(
            QuotaKind::Listings.sentence(u64::from(free.resources_max)),
            "Your plan includes 20 resources. Upgrade to add more."
        );
        assert_eq!(
            QuotaKind::StorageBytes.sentence(free.storage_bytes_max),
            "Your plan includes 1 GB of files. Upgrade to add more."
        );
    }

    #[test]
    fn the_reset_date_reads_as_a_day_rather_than_an_instant() {
        assert_eq!(day_of(Timestamp(1_790_812_800_000)), "1 October 2026");
    }
}
