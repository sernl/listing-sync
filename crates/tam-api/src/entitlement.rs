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
    AiOffer, Capabilities, Pack, Plan, PlanRow, PreviewAllowance, PriceKey, AI, PACKS, PACK_ABOVE,
    PLANS,
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
    /// Watermarked previews made this UTC month.
    Previews,
    /// Watermarked previews made over the organisation's life, on a plan
    /// that counts them that way rather than a month at a time.
    PreviewsLifetime,
    /// AI description fills made this UTC month.
    AiFills,
    /// The plan does not include the capability at all, rather than having
    /// run out of it. `detail.feature` names which one.
    PlanFeature,
}

impl QuotaKind {
    pub const ALL: [Self; 12] = [
        Self::Listings,
        Self::StorageBytes,
        Self::Marketplaces,
        Self::Moves,
        Self::Templates,
        Self::Labels,
        Self::Collections,
        Self::Devices,
        Self::Previews,
        Self::PreviewsLifetime,
        Self::AiFills,
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
            Self::Previews => "previews_per_month",
            Self::PreviewsLifetime => "previews_lifetime",
            Self::AiFills => "ai_fills_per_month",
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
            // Whole gigabytes where the ceiling has them, whole megabytes
            // below that: Look's 256 MiB would otherwise read as "0 GB".
            Self::StorageBytes if limit >= 1 << 30 => format!(
                "Your plan includes {} GB of files. Upgrade to add more.",
                limit >> 30
            ),
            Self::StorageBytes => format!(
                "Your plan includes {} MB of files. Upgrade to add more.",
                limit >> 20
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
            // A monthly allowance says when it comes back as well as how to
            // get more now, because unlike a standing count it renews.
            Self::Previews if limit == 0 => {
                "Your plan does not include watermarked previews. Upgrade to make them.".to_owned()
            }
            Self::Previews => format!(
                "Your plan includes {limit} watermarked previews a month. \
                 Upgrade to make more, or wait until next month."
            ),
            // A lifetime allowance never comes back, so the sentence offers
            // only the upgrade.
            Self::PreviewsLifetime => format!(
                "Your plan includes {limit} watermarked previews to try. Upgrade to make more."
            ),
            Self::AiFills if limit == 0 => {
                "Your plan does not include AI description fills. Upgrade to use them.".to_owned()
            }
            Self::AiFills => format!(
                "Your plan includes {limit} AI description fills a month. \
                 Upgrade to make more, or wait until next month."
            ),
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

/// Refuses a write that has just taken the organisation's live resources past
/// its plan's ceiling, in the write's own transaction so the refusal rolls
/// the write back.
///
/// Checked after the insert or restore rather than before it, because every
/// path that adds a live resource — a create, a spreadsheet row, a shop
/// import, a migrate leg, a re-import or merge undo that brings one back —
/// can then ask the same question in the same words, and a restore that
/// turned out to be a no-op adds nothing and is never refused. The caller
/// holds the organisation's catalogue lock (`begin_guarded`) or the request
/// lock of a migrate leg, so two writes cannot both take the last place.
///
/// The grandfather rule falls out of the comparison: an organisation already
/// over a lowered ceiling keeps every resource it has, and the next one it
/// adds is refused.
///
/// # Errors
///
/// The `listings_max` 422, or a storage fault.
pub(crate) async fn refuse_past_resource_cap_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    org: tam_types::OrgId,
    resources_max: u32,
) -> Result<(), APIError> {
    if resources_max == u32::MAX {
        return Ok(());
    }
    let live = tam_storage::live_count_in(tx, org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if live > i64::from(resources_max) {
        return Err(quota_refusal(
            QuotaKind::Listings,
            live.saturating_sub(1),
            u64::from(resources_max),
        ));
    }
    Ok(())
}

/// One spend of `amount` watermarked previews against the window the plan
/// counts them in, and the refusal that window answers with.
fn preview_charge(
    state: &AppState,
    caps: &Capabilities,
    amount: u32,
) -> (tam_storage::MonthlyCharge, QuotaKind) {
    let (window, kind) = match caps.previews() {
        PreviewAllowance::Lifetime(_) => (
            tam_storage::CounterWindow::Lifetime,
            QuotaKind::PreviewsLifetime,
        ),
        PreviewAllowance::Monthly(_) => (tam_storage::CounterWindow::Month, QuotaKind::Previews),
    };
    (
        tam_storage::MonthlyCharge {
            kind: tam_storage::MonthlyKind::Preview,
            amount,
            cap: caps.previews().cap(),
            window,
            at: (state.wall)(),
        },
        kind,
    )
}

/// Spends `amount` watermarked previews of the plan's allowance — this
/// month's, or the account's lifetime five on Look — in the write's own
/// transaction, or refuses the write.
///
/// Every preview a resource gains counts — attached to a saved resource,
/// carried by a create, or swapped in for an older one — because the server
/// cannot tell a preview drawn by the preview maker from one the seller drew
/// elsewhere, and counting only one kind would be an allowance anyone could
/// step around. Deleting a preview gives nothing back: it was made.
///
/// # Errors
///
/// The `previews_per_month` or `previews_lifetime` 422, or a storage fault.
pub(crate) async fn spend_previews_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    org: tam_types::OrgId,
    caps: &Capabilities,
    amount: u32,
) -> Result<(), APIError> {
    // Counted on an unlimited plan too, so what every organisation makes is
    // on record whatever it pays.
    if amount == 0 {
        return Ok(());
    }
    let (charge, refusal) = preview_charge(state, caps, amount);
    match tam_storage::spend_monthly_in(tx, org, charge)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        tam_storage::MonthlySpend::Granted { .. } => Ok(()),
        tam_storage::MonthlySpend::Refused { used } => {
            Err(quota_refusal(refusal, used, u64::from(charge.cap)))
        }
    }
}

/// A spend of previews made ahead of a write that runs in a transaction of
/// its own (adding or swapping a file on a saved resource), to be handed back
/// with [`PreviewSpend::refund`] where that write then fails.
#[must_use = "a spend whose write failed must be refunded"]
pub(crate) struct PreviewSpend(Option<tam_storage::MonthlyCharge>);

impl PreviewSpend {
    /// Spends one preview, or refuses with the plan's preview 422.
    pub(crate) async fn one(
        state: &AppState,
        org: tam_types::OrgId,
        caps: &Capabilities,
    ) -> Result<Self, APIError> {
        let (charge, refusal) = preview_charge(state, caps, 1);
        match EntitlementRepo::new(state.pool.clone())
            .spend_monthly(org, charge)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
        {
            tam_storage::MonthlySpend::Granted { .. } => Ok(Self(Some(charge))),
            tam_storage::MonthlySpend::Refused { used } => {
                Err(quota_refusal(refusal, used, u64::from(charge.cap)))
            }
        }
    }

    /// How many previews the plan's window has already used, and the
    /// refusal to answer with where that is the whole allowance: the check
    /// the upload runs before it seals a byte, so a preview that could never
    /// be attached is not stored first.
    pub(crate) async fn refusal_before_upload(
        state: &AppState,
        org: tam_types::OrgId,
        caps: &Capabilities,
    ) -> Result<Option<APIError>, APIError> {
        let (charge, refusal) = preview_charge(state, caps, 1);
        let used = EntitlementRepo::new(state.pool.clone())
            .counted_used(org, charge.kind, charge.window, charge.at)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        Ok((used >= i64::from(charge.cap))
            .then(|| quota_refusal(refusal, used, u64::from(charge.cap))))
    }

    /// Nothing spent, for a write that adds no preview.
    pub(crate) const fn none() -> Self {
        Self(None)
    }

    /// Passes a write's result through, giving the spend back where it
    /// failed so a refused file does not use up the month.
    pub(crate) async fn settle<T>(
        self,
        state: &AppState,
        org: tam_types::OrgId,
        result: Result<T, APIError>,
    ) -> Result<T, APIError> {
        if let (Err(_), Some(charge)) = (&result, self.0) {
            EntitlementRepo::new(state.pool.clone())
                .refund_monthly(org, charge)
                .await
                .map_err(|error| state.internal(&error.to_string()))?;
        }
        result
    }
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
            "You have no moves left. Buy a pack, or choose Pro.".to_owned()
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
    pub monthly_key: Option<PriceKey>,
    pub yearly_key: Option<PriceKey>,
    pub trial_days: u32,
    pub recommended: bool,
    pub tagline: String,
    pub capabilities: Capabilities,
}

impl PlanRowView {
    fn of(row: PlanRow) -> Self {
        Self {
            id: row.id,
            name: row.name.to_owned(),
            monthly_cents: row.monthly_cents,
            yearly_cents: row.yearly_cents,
            monthly_key: row.monthly_key,
            yearly_key: row.yearly_key,
            trial_days: row.trial_days,
            recommended: row.recommended,
            tagline: row.tagline.to_owned(),
            // At no rung, which is what a price list shows: the only plan
            // that reads one is Studio, whose rung is an operator's decision
            // about one tenant rather than a figure on a public page.
            capabilities: row.id.capabilities(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlansView {
    pub plans: Vec<PlanRowView>,
    pub packs: Vec<Pack>,
    pub pack_above: String,
    pub ai: AiOffer,
    /// The sale open now, which every plan's checkout applies by itself.
    /// Absent outside a sale, and whenever the sale could not be read: the
    /// price list stays readable at list prices rather than failing.
    pub sale: Option<crate::pricing::SaleView>,
}

/// How long a shared cache may keep the price list. A sale opens or closes
/// at midnight and an operator may end one early; a minute is the most a
/// struck price may lag either.
const PLANS_MAX_AGE_SECS: u32 = 60;

/// The price list. Unauthenticated, because the pricing page is public and a
/// price a seller cannot read before signing up is not a price list.
pub(crate) async fn plans_view(
    _version: APIVersion,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let sale = crate::pricing::current_sale(&state, (state.wall)())
        .await
        .unwrap_or_else(|error| {
            eprintln!("tam-api: the price list could not read the current sale: {error}");
            None
        });
    let cache = format!("public, max-age={PLANS_MAX_AGE_SECS}");
    (
        [(axum::http::header::CACHE_CONTROL, cache)],
        Json(PlansView {
            plans: PLANS.into_iter().map(PlanRowView::of).collect(),
            packs: PACKS.to_vec(),
            pack_above: PACK_ABOVE.to_owned(),
            ai: AI,
            sale,
        }),
    )
}

// ------------------------------------------------------- GET /v1/entitlement

/// What has been used of each allowance, beside `capabilities`, which carries
/// the matching limit: `resources` against `resources_max`, `storage_bytes`
/// against `storage_bytes_max`, `previews` against `previews_per_month`,
/// `previews_lifetime` against `previews_lifetime` and so on. `previews`,
/// `ai_fills` and `moves_this_month` count the current UTC calendar month and
/// start again at `month_resets_at`; `previews_lifetime` is every month's
/// previews summed and never starts again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageView {
    pub resources: i64,
    pub marketplaces: i64,
    pub templates: i64,
    pub labels: i64,
    pub collections: i64,
    pub devices: i64,
    pub storage_bytes: i64,
    pub previews: i64,
    pub previews_lifetime: i64,
    pub ai_fills: i64,
    /// Moves this month's commits spent, from `move_ledger`.
    pub moves_this_month: i64,
    pub month_resets_at: Timestamp,
    /// Whether this organisation has bound a storefront, which is when the
    /// free lifetime moves land in the balance. Until then Look's five are
    /// still to come, and the console says so rather than reading "0 left".
    pub free_moves_unlocked: bool,
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
            storage_bytes: usage.storage_bytes,
            previews: usage.previews,
            previews_lifetime: usage.previews_lifetime,
            ai_fills: usage.ai_fills,
            moves_this_month: usage.moves_this_month,
            month_resets_at: usage.month_resets_at,
            free_moves_unlocked: usage.free_moves_unlocked,
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
        .usage(context.org, (state.wall)())
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
                | QuotaKind::Previews
                | QuotaKind::PreviewsLifetime
                | QuotaKind::AiFills
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
            "Your plan includes 100 resources. Upgrade to add more."
        );
        assert_eq!(
            QuotaKind::StorageBytes.sentence(free.storage_bytes_max),
            "Your plan includes 256 MB of files. Upgrade to add more."
        );
        assert_eq!(
            QuotaKind::StorageBytes.sentence(Plan::Subscriber.capabilities(None).storage_bytes_max),
            format!(
                "Your plan includes {} GB of files. Upgrade to add more.",
                Plan::Subscriber.capabilities(None).storage_bytes_max >> 30
            )
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
            "You have no moves left. Buy a pack, or choose Pro."
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
