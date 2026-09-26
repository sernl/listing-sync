//! What a plan grants, read once per request and enforced at every write it
//! bounds.
//!
//! The plan is not a column and not a billing status. It is the strongest
//! unexpired row in `entitlement_grant`, read by the [`OrgContext`] extractor
//! before any handler runs and carried on the context as a [`Capabilities`]
//! value. That is one extra indexed read per authenticated request, and it
//! buys the property the design asks for: the pricing page, the Account page
//! and every gate answer from one table, so no two surfaces can disagree
//! about what a plan holds.
//!
//! This module replaces `quota.rs`, which derived two numbers from the
//! recorded subscription because no plan existed to read. Deriving an
//! entitlement from a billing status was always a stand-in — it could not
//! express a one-off purchase, an operator grant or an expiry — and the
//! subscription table goes back to being what its migration says it is: a
//! record of what the billing provider said.
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
    AiOffer, Capabilities, Founding, Pack, Plan, PlanRow, PriceKey, Service, AI, FOUNDING, PACKS,
    PACK_ABOVE, PLANS, SERVICES,
};
use tam_storage::{EntitlementRepo, Grant, MoveBalance, Usage};
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
    /// The move balance, which is the one bound that is a balance rather
    /// than a ceiling: it is bought and spent rather than reset.
    Moves,
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
        Self::Moves,
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
            Self::Moves => "moves",
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
            Self::Moves if limit == 0 => {
                "Your plan includes no moves. Buy a pack to move resources.".to_owned()
            }
            Self::Moves => {
                format!("Your plan includes {limit} moves a month. Buy a pack to move more.")
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

/// A move the balance cannot pay for.
///
/// A struct rather than four positional arguments, because the two gates
/// that raise it — the migration confirm and the sync submit — must say the
/// same thing, and the shape is what makes that true rather than a comment
/// asking for it.
///
/// The balance is not a monthly counter and the refusal must not read like
/// one: there is no date on which more arrive unless the seller is paying
/// for a subscription, so the sentence says what to do rather than when to
/// come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveRefusal {
    pub available: i64,
    pub requested: i64,
    /// When the soonest part of the balance lapses, where any of it does.
    /// Carried so the console can warn before it happens rather than after.
    pub expiring_soonest: Option<Timestamp>,
}

impl MoveRefusal {
    #[must_use]
    pub fn of(balance: &MoveBalance, requested: i64) -> Self {
        Self {
            available: balance.available,
            requested,
            expiring_soonest: balance.expiring_soonest,
        }
    }

    #[must_use]
    pub fn sentence(self) -> String {
        if self.available == 0 {
            "You have no moves left. Buy a pack, or choose Sync.".to_owned()
        } else if self.available == 1 {
            format!(
                "You have one move left and asked to move {}. Buy a pack or pick fewer.",
                self.requested
            )
        } else {
            format!(
                "You have {} moves left and asked to move {}. Buy a pack or pick fewer.",
                self.available, self.requested
            )
        }
    }
}

impl From<MoveRefusal> for APIError {
    fn from(refusal: MoveRefusal) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(&refusal.sentence())
                .code(APIErrorCode::QuotaExceeded)
                .kind(APIErrorKind::Validation)
                .detail(serde_json::json!({
                    "quota": QuotaKind::Moves.as_str(),
                    "available": refusal.available,
                    "requested": refusal.requested,
                    "expiring_soonest": refusal.expiring_soonest,
                })),
        )
    }
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
            // At no rung, which is what a price list shows: the only plan
            // that reads one is Studio, whose rung is an operator's decision
            // about one tenant rather than a figure on a public page.
            capabilities: row.id.capabilities(None),
            sold: row.sold,
        }
    }
}

/// One service as the pricing page reads it.
///
/// Owned rather than [`Service`] itself, for [`PlanRowView`]'s reason: the
/// constant carries `&'static str`, and a view a client deserialises cannot
/// borrow from a lifetime it does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceView {
    pub key: PriceKey,
    pub name: String,
    pub price_cents: u32,
}

impl ServiceView {
    fn of(service: Service) -> Self {
        Self {
            key: service.key,
            name: service.name.to_owned(),
            price_cents: service.price_cents,
        }
    }
}

/// The founding offer as the pricing page reads it. Owned for the same
/// reason as [`ServiceView`]; `closes_at` is the borrowed field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundingView {
    pub discount_year_one_pct: u32,
    pub discount_ongoing_pct: u32,
    pub ongoing_years: u32,
    pub year_one_cents: u32,
    pub ongoing_cents: u32,
    pub closes_at: String,
    pub annual_only: bool,
    pub extra_moves: u32,
    pub places: u32,
}

impl FoundingView {
    fn of(founding: Founding) -> Self {
        Self {
            discount_year_one_pct: founding.discount_year_one_pct,
            discount_ongoing_pct: founding.discount_ongoing_pct,
            ongoing_years: founding.ongoing_years,
            year_one_cents: founding.year_one_cents,
            ongoing_cents: founding.ongoing_cents,
            closes_at: founding.closes_at.to_owned(),
            annual_only: founding.annual_only,
            extra_moves: founding.extra_moves,
            places: founding.places,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlansView {
    pub plans: Vec<PlanRowView>,
    pub packs: Vec<Pack>,
    pub pack_above: String,
    pub services: Vec<ServiceView>,
    pub founding: FoundingView,
    pub ai: AiOffer,
}

/// The price list. Unauthenticated, because the pricing page is public and a
/// price a seller cannot read before signing up is not a price list.
pub(crate) async fn plans_view(_version: APIVersion) -> Json<PlansView> {
    Json(PlansView {
        plans: PLANS.into_iter().map(PlanRowView::of).collect(),
        packs: PACKS.to_vec(),
        pack_above: PACK_ABOVE.to_owned(),
        services: SERVICES.into_iter().map(ServiceView::of).collect(),
        founding: FoundingView::of(FOUNDING),
        ai: AI,
    })
}

// ------------------------------------------------------- GET /v1/entitlement

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageView {
    pub resources: i64,
    pub marketplaces: i64,
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
            templates: usage.templates,
            labels: usage.labels,
            collections: usage.collections,
            devices: usage.devices,
        }
    }
}

/// The moves an organisation can spend, as the console reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveBalanceView {
    pub available: i64,
    pub expiring_soonest: Option<Timestamp>,
}

impl MoveBalanceView {
    #[must_use]
    pub const fn of(balance: MoveBalance) -> Self {
        Self {
            available: balance.available,
            expiring_soonest: balance.expiring_soonest,
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
    /// The balance, which is not a usage figure: it is bought and spent
    /// rather than counted against a ceiling, so it sits beside the usage
    /// block rather than inside it.
    pub moves: MoveBalanceView,
}

pub(crate) async fn entitlement_view(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<EntitlementView>, APIError> {
    let entitlements = EntitlementRepo::new(state.pool.clone());
    let usage = entitlements
        .usage(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let moves = entitlements
        .move_balance(context.org, (state.wall)())
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
        moves: MoveBalanceView::of(moves),
    }))
}

#[cfg(test)]
mod tests {
    use super::{MoveRefusal, QuotaKind};
    use tam_limits::Plan;

    #[test]
    fn every_bound_names_itself_on_the_wire_and_speaks_to_the_seller() {
        for kind in QuotaKind::ALL {
            match kind {
                QuotaKind::Listings
                | QuotaKind::StorageBytes
                | QuotaKind::Marketplaces
                | QuotaKind::Moves
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
            "Your plan includes 500 resources. Upgrade to add more."
        );
        assert_eq!(
            QuotaKind::StorageBytes.sentence(free.storage_bytes_max),
            "Your plan includes 1 GB of files. Upgrade to add more."
        );
    }

    /// A balance is not a monthly counter, so its refusal must never tell a
    /// seller to come back next month: on Free there is no next month, and a
    /// sentence promising one is the worst kind of wrong.
    #[test]
    fn a_move_refusal_says_what_to_do_rather_than_when_to_return() {
        let empty = MoveRefusal {
            available: 0,
            requested: 4,
            expiring_soonest: None,
        };
        assert_eq!(
            empty.sentence(),
            "You have no moves left. Buy a pack, or choose Sync."
        );
        let short = MoveRefusal {
            available: 3,
            requested: 9,
            expiring_soonest: None,
        };
        assert_eq!(
            short.sentence(),
            "You have 3 moves left and asked to move 9. Buy a pack or pick fewer."
        );
        let one = MoveRefusal {
            available: 1,
            requested: 2,
            expiring_soonest: None,
        };
        assert_eq!(
            one.sentence(),
            "You have one move left and asked to move 2. Buy a pack or pick fewer."
        );
    }
}
