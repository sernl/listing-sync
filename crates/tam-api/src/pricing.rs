//! Sales, one-off discounts and discount codes: what the admin Pricing page
//! writes, what the price list announces, and which Stripe discount a
//! checkout opens with.
//!
//! Each discount is one Stripe Coupon, created here when the operator saves
//! it and named after its own row (`teachouse-<uuid>`), so the dashboard says
//! which row a coupon belongs to and a retried create collides with the first
//! instead of minting a second. A code is additionally one Promotion Code over
//! its coupon. Terms never change after the coupon exists — Stripe will not
//! change a coupon's amount or duration either — so the only later write is
//! ending one, which deletes the coupon and deactivates its codes in Stripe
//! at the same moment.
//!
//! A sale's banner and theme are presentation rather than terms, and live in
//! `site_setting` under `sale.<id>` beside the site's own theme and banner.
//!
//! Windows are calendar dates on the wire and half-open instants in storage:
//! `until` is inclusive on the page and turns into the midnight (UTC) after
//! it, so one comparison decides "open" everywhere.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_limits::PriceKey;
use tam_storage::{
    CodeWrite, Discount, DiscountAmount, DiscountCode, DiscountDuration, DiscountKind,
    DiscountRepo, SiteSettingRepo, StorageError,
};
use tam_types::{Timestamp, Uuid};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::session::OperatorContext;
use crate::stripe::{
    self, CheckoutDiscount, CouponAmount, CouponDuration, CouponRequest, PromotionCodeRequest,
};
use crate::time::{date_of, date_start};
use crate::AppState;

const MILLIS_PER_DAY: i64 = 86_400_000;

/// The `site_setting` key prefix a sale's presentation is stored under.
pub const SALE_SETTING_PREFIX: &str = "sale.";

/// The currency every price in `tam-limits` is quoted in, and so the one an
/// amount-off coupon is minted in.
const CURRENCY: &str = "usd";

const NAME_MAX: usize = 80;
const BANNER_MAX: usize = 140;

/// The themes a sale may be tied to: the site themes SiteOps' `/v1/site`
/// knows, less `none`.
const THEMES: [&str; 2] = ["halloween", "christmas"];

/// Whether a price key is a plan, which is what "every plan" — a sale, or a
/// discount naming no keys — reaches. Every recurring key is a plan
/// subscription; packs are single charges and are never on sale.
#[must_use]
pub const fn is_plan(key: PriceKey) -> bool {
    key.recurring()
}

/// What `amount` takes off one key's list price, in cents.
fn saving(amount: DiscountAmount, key: PriceKey) -> u64 {
    let list = u64::from(key.list_cents());
    match amount {
        DiscountAmount::Percent(percent) => percent_of(list, percent),
        DiscountAmount::Cents(cents) => u64::from(cents).min(list),
    }
}

/// `percent` of `cents`, rounded to the nearest whole cent, halves up.
#[expect(
    clippy::integer_division,
    reason = "whole cents: the added half makes the truncating division round to nearest"
)]
fn percent_of(cents: u64, percent: u32) -> u64 {
    (cents * u64::from(percent) + 50) / 100
}

fn coupon_id(discount: Uuid) -> String {
    format!("teachouse-{}", discount.to_hyphenated())
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}

// -------------------------------------------------------------------- views

/// A sale the price list announces: open now, and the only one that is,
/// because two sales may not overlap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleView {
    pub percent_off: u32,
    /// The last day of the sale, inclusive, `YYYY-MM-DD` in UTC.
    pub until: String,
    pub banner: String,
    /// Where the banner links, when it links anywhere.
    pub banner_href: Option<String>,
}

/// A sale's presentation as `site_setting` stores it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct SalePresentation {
    banner: String,
    #[serde(default)]
    banner_href: Option<String>,
    #[serde(default)]
    theme: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscountState {
    /// Saved, and its window has not opened yet.
    Scheduled,
    /// Offered now.
    Open,
    /// Its window closed by itself.
    Over,
    /// An operator ended it early.
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscountCodeView {
    pub id: String,
    pub code: String,
    pub max_redemptions: Option<u32>,
    pub stripe_promotion_code_id: String,
    pub ended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscountView {
    pub id: String,
    /// `sale`, `one_off` or `code`.
    pub kind: String,
    pub name: String,
    pub percent_off: Option<u32>,
    pub amount_off_cents: Option<u32>,
    pub currency: String,
    /// `once` or `repeating`.
    pub duration: String,
    pub duration_months: Option<u32>,
    /// Empty means every plan.
    pub price_keys: Vec<String>,
    pub from: String,
    /// Inclusive.
    pub until: String,
    pub state: DiscountState,
    pub stripe_coupon_id: String,
    /// Whether Stripe still holds this coupon as redeemable. Absent when
    /// Stripe was not asked: no key configured, or it did not answer.
    pub in_stripe: Option<bool>,
    pub banner: Option<String>,
    pub banner_href: Option<String>,
    pub theme: Option<String>,
    pub codes: Vec<DiscountCodeView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingAdminView {
    pub discounts: Vec<DiscountView>,
    /// Every price key a discount may name, with its list price, so the page
    /// offers the vocabulary the server accepts rather than its own copy.
    pub price_keys: Vec<PriceKeyView>,
    /// Whether this deployment can create Stripe objects at all.
    pub stripe_configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceKeyView {
    pub key: PriceKey,
    pub list_cents: u32,
    pub plan: bool,
}

fn state_of(discount: &Discount, now: Timestamp) -> DiscountState {
    if discount.ended_at.is_some() {
        DiscountState::Ended
    } else if now.0 < discount.starts_at.0 {
        DiscountState::Scheduled
    } else if now.0 < discount.ends_at.0 {
        DiscountState::Open
    } else {
        DiscountState::Over
    }
}

fn last_day(discount: &Discount) -> String {
    date_of(Timestamp(discount.ends_at.0 - 1))
}

impl DiscountView {
    fn of(
        discount: &Discount,
        presentation: Option<SalePresentation>,
        codes: Vec<DiscountCodeView>,
        in_stripe: Option<bool>,
        now: Timestamp,
    ) -> Self {
        let (percent_off, amount_off_cents) = match discount.amount {
            DiscountAmount::Percent(percent) => (Some(percent), None),
            DiscountAmount::Cents(cents) => (None, Some(cents)),
        };
        let (duration, duration_months) = match discount.duration {
            DiscountDuration::Once => ("once", None),
            DiscountDuration::Repeating(months) => ("repeating", Some(months)),
        };
        let presentation = presentation.unwrap_or_default();
        Self {
            id: discount.id.to_hyphenated(),
            kind: discount.kind.as_str().to_owned(),
            name: discount.name.clone(),
            percent_off,
            amount_off_cents,
            currency: discount.currency.clone(),
            duration: duration.to_owned(),
            duration_months,
            price_keys: discount.price_keys.clone(),
            from: date_of(discount.starts_at),
            until: last_day(discount),
            state: state_of(discount, now),
            stripe_coupon_id: discount.stripe_coupon_id.clone(),
            in_stripe,
            banner: (!presentation.banner.is_empty()).then_some(presentation.banner),
            banner_href: presentation.banner_href,
            theme: presentation.theme,
            codes,
        }
    }
}

// ------------------------------------------------------------------ errors

fn invalid(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn conflict(message: &str) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn not_found() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new("There is no discount with that id.").kind(APIErrorKind::NotFound),
    )
}

fn unconfigured() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new("This deployment has no Stripe key, so it cannot create discounts."),
    )
}

fn storage(state: &AppState, error: &StorageError) -> APIError {
    state.internal(&error.to_string())
}

fn stripe_fault(state: &AppState, error: &stripe::StripeError) -> APIError {
    state.internal(&error.to_string())
}

// -------------------------------------------------------------- public read

/// The sale open at `now`, with its banner, or `None`.
pub(crate) async fn current_sale(
    state: &AppState,
    now: Timestamp,
) -> Result<Option<SaleView>, StorageError> {
    let open = DiscountRepo::new(state.pool.clone())
        .open_automatic(now)
        .await?;
    let Some(sale) = open.into_iter().find(|d| d.kind == DiscountKind::Sale) else {
        return Ok(None);
    };
    let DiscountAmount::Percent(percent_off) = sale.amount else {
        return Ok(None);
    };
    let presentation = presentation(state, sale.id).await?.unwrap_or_default();
    Ok(Some(SaleView {
        percent_off,
        until: last_day(&sale),
        banner: presentation.banner,
        banner_href: presentation.banner_href,
    }))
}

async fn presentation(
    state: &AppState,
    id: Uuid,
) -> Result<Option<SalePresentation>, StorageError> {
    let stored = SiteSettingRepo::new(state.pool.clone())
        .get(&format!("{SALE_SETTING_PREFIX}{}", id.to_hyphenated()))
        .await?;
    Ok(stored.and_then(|value| serde_json::from_value(value).ok()))
}

// ---------------------------------------------------------------- checkout

/// Refused when a typed code does not reach this checkout. One message for
/// unknown, ended, not yet open and wrong plan alike: which of them it was
/// is not the seller's problem, and telling a guesser which codes exist is
/// handing over the list.
fn code_refused() -> APIError {
    invalid("That code doesn't work for this plan. Check the spelling, or leave it empty.")
}

/// The discount a checkout for `key` opens with, owned so the request can
/// borrow it.
pub(crate) enum ChosenDiscount {
    Coupon(String),
    PromotionCode(String),
}

impl ChosenDiscount {
    pub(crate) fn as_checkout(&self) -> CheckoutDiscount<'_> {
        match self {
            Self::Coupon(id) => CheckoutDiscount::Coupon(id),
            Self::PromotionCode(id) => CheckoutDiscount::PromotionCode(id),
        }
    }
}

/// The best automatic discount for one key at `now`: the open sale or
/// one-off discount that takes most off its list price, earliest-started on
/// a tie. `None` leaves Stripe's promotion-code field on its page.
#[must_use]
pub fn best_automatic(open: &[Discount], key: PriceKey, now: Timestamp) -> Option<&Discount> {
    open.iter()
        .filter(|d| d.kind != DiscountKind::Code && d.open_at(now))
        .filter(|d| d.covers(key.as_str(), is_plan(key)))
        .fold(None, |best: Option<&Discount>, d| match best {
            Some(held) if saving(held.amount, key) >= saving(d.amount, key) => Some(held),
            _ => Some(d),
        })
}

/// Decides the discount a checkout for `key` carries.
///
/// A typed code wins over an automatic discount: the seller asked for it by
/// name, and Stripe applies one discount per session.
pub(crate) async fn checkout_discount(
    state: &AppState,
    key: PriceKey,
    code: Option<&str>,
) -> Result<Option<ChosenDiscount>, APIError> {
    let now = (state.wall)();
    let repo = DiscountRepo::new(state.pool.clone());
    if let Some(code) = code.map(str::trim).filter(|code| !code.is_empty()) {
        let resolved = repo
            .resolve_code(code)
            .await
            .map_err(|error| storage(state, &error))?;
        return match resolved {
            Some((code, discount))
                if discount.open_at(now) && discount.covers(key.as_str(), is_plan(key)) =>
            {
                Ok(Some(ChosenDiscount::PromotionCode(
                    code.stripe_promotion_code_id,
                )))
            }
            _ => Err(code_refused()),
        };
    }
    let open = repo
        .open_automatic(now)
        .await
        .map_err(|error| storage(state, &error))?;
    Ok(best_automatic(&open, key, now)
        .map(|discount| ChosenDiscount::Coupon(discount.stripe_coupon_id.clone())))
}

// -------------------------------------------------------------- admin read

pub(crate) async fn admin_view(
    State(state): State<AppState>,
    _operator: OperatorContext,
) -> Result<Json<PricingAdminView>, APIError> {
    let now = (state.wall)();
    let repo = DiscountRepo::new(state.pool.clone());
    let discounts = repo.list().await.map_err(|e| storage(&state, &e))?;
    let codes = repo.codes().await.map_err(|e| storage(&state, &e))?;
    let sales = SiteSettingRepo::new(state.pool.clone())
        .with_prefix(SALE_SETTING_PREFIX)
        .await
        .map_err(|e| storage(&state, &e))?;
    let sales: BTreeMap<String, SalePresentation> = sales
        .into_iter()
        .filter_map(|(key, value)| {
            let id = key.strip_prefix(SALE_SETTING_PREFIX)?.to_owned();
            Some((id, serde_json::from_value(value).ok()?))
        })
        .collect();

    // Stripe is asked, once, which coupons it still holds as redeemable, so
    // a deployment switched to a different Stripe account shows its missing
    // coupons instead of charging full price in silence. A failure here
    // costs the page one column, not the page.
    let in_stripe: Option<BTreeSet<String>> = match state.config.stripe.as_ref() {
        Some(client) => match client.list_coupons().await {
            Ok(coupons) => Some(
                coupons
                    .into_iter()
                    .filter(|coupon| coupon.valid)
                    .map(|coupon| coupon.id)
                    .collect(),
            ),
            Err(error) => {
                eprintln!("tam-api: the admin pricing read could not list coupons: {error}");
                None
            }
        },
        None => None,
    };

    let mut by_discount: HashMap<Uuid, Vec<DiscountCodeView>> = HashMap::new();
    for code in codes {
        by_discount
            .entry(code.discount_id)
            .or_default()
            .push(DiscountCodeView {
                id: code.id.to_hyphenated(),
                code: code.code,
                max_redemptions: code.max_redemptions,
                stripe_promotion_code_id: code.stripe_promotion_code_id,
                ended: code.ended_at.is_some(),
            });
    }
    let discounts = discounts
        .iter()
        .map(|discount| {
            DiscountView::of(
                discount,
                sales.get(&discount.id.to_hyphenated()).cloned(),
                by_discount.remove(&discount.id).unwrap_or_default(),
                in_stripe
                    .as_ref()
                    .map(|held| held.contains(&discount.stripe_coupon_id)),
                now,
            )
        })
        .collect();
    Ok(Json(PricingAdminView {
        discounts,
        price_keys: PriceKey::ALL
            .into_iter()
            .map(|key| PriceKeyView {
                key,
                list_cents: key.list_cents(),
                plan: is_plan(key),
            })
            .collect(),
        stripe_configured: state.config.stripe.is_some(),
    }))
}

// ------------------------------------------------------------- admin writes

/// The terms every kind shares.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermsBody {
    pub name: String,
    #[serde(default)]
    pub percent_off: Option<u32>,
    #[serde(default)]
    pub amount_off_cents: Option<u32>,
    /// `once` (default) or `repeating`.
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub duration_months: Option<u32>,
    /// Price keys; empty means every plan.
    #[serde(default)]
    pub price_keys: Vec<String>,
    /// First day, inclusive, `YYYY-MM-DD` (UTC).
    pub from: String,
    /// Last day, inclusive, `YYYY-MM-DD` (UTC).
    pub until: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleBody {
    pub name: String,
    pub percent_off: u32,
    pub from: String,
    pub until: String,
    pub banner: String,
    #[serde(default)]
    pub banner_href: Option<String>,
    /// `halloween`, `christmas`, or absent for a standalone sale.
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub duration_months: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeBody {
    pub code: String,
    #[serde(flatten)]
    pub terms: TermsBody,
    #[serde(default)]
    pub max_redemptions: Option<u32>,
}

/// Terms that passed every check, ready to become a coupon and a row.
struct Terms {
    name: String,
    amount: DiscountAmount,
    duration: DiscountDuration,
    price_keys: Vec<PriceKey>,
    starts_at: Timestamp,
    ends_at: Timestamp,
}

fn check_terms(body: &TermsBody, now: Timestamp) -> Result<Terms, APIError> {
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(invalid("Give it a name of up to 80 characters."));
    }
    let amount = match (body.percent_off, body.amount_off_cents) {
        (Some(percent), None) if (1..=100).contains(&percent) => DiscountAmount::Percent(percent),
        (None, Some(cents)) if cents > 0 => DiscountAmount::Cents(cents),
        (Some(_), None) => return Err(invalid("A percentage off is between 1 and 100.")),
        (None, Some(_)) => return Err(invalid("An amount off is more than zero.")),
        _ => {
            return Err(invalid(
                "Choose a percentage off or an amount off, not both.",
            ))
        }
    };
    let duration = match (
        body.duration.as_deref().unwrap_or("once"),
        body.duration_months,
    ) {
        ("once", None) => DiscountDuration::Once,
        ("repeating", Some(months)) if (1..=36).contains(&months) => {
            DiscountDuration::Repeating(months)
        }
        _ => {
            return Err(invalid(
                "Choose \"first payment only\", or a number of months between 1 and 36.",
            ))
        }
    };
    let mut price_keys = Vec::new();
    for raw in &body.price_keys {
        let Some(key) = PriceKey::parse(raw) else {
            return Err(invalid(&format!("{raw} is not a price we sell.")));
        };
        if !price_keys.contains(&key) {
            price_keys.push(key);
        }
    }
    let (Ok(starts_at), Ok(last)) = (date_start(&body.from), date_start(&body.until)) else {
        return Err(invalid("Dates are written YYYY-MM-DD."));
    };
    let ends_at = Timestamp(last.0 + MILLIS_PER_DAY);
    if ends_at.0 <= starts_at.0 {
        return Err(invalid("The last day comes on or after the first day."));
    }
    if ends_at.0 <= now.0 {
        return Err(invalid("That window has already closed."));
    }
    Ok(Terms {
        name: name.to_owned(),
        amount,
        duration,
        price_keys,
        starts_at,
        ends_at,
    })
}

/// Creates the coupon in Stripe and records its row, deleting the coupon
/// again if the row cannot be written so no coupon outlives its record.
async fn create_discount(
    state: &AppState,
    operator: &OperatorContext,
    kind: DiscountKind,
    terms: &Terms,
    products: &[String],
) -> Result<Discount, APIError> {
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let id = fresh_uuid();
    let coupon = coupon_id(id);
    let amount = match terms.amount {
        DiscountAmount::Percent(percent) => CouponAmount::Percent(percent),
        DiscountAmount::Cents(cents) => CouponAmount::Cents {
            cents,
            currency: CURRENCY,
        },
    };
    let duration = match terms.duration {
        DiscountDuration::Once => CouponDuration::Once,
        DiscountDuration::Repeating(months) => CouponDuration::Repeating(months),
    };
    let created = client
        .create_coupon(&CouponRequest {
            id: &coupon,
            name: &terms.name,
            amount,
            duration,
            redeem_by: Some(Timestamp(terms.ends_at.0 - 1_000)),
            products,
        })
        .await
        .map_err(|error| stripe_fault(state, &error))?;
    let discount = Discount {
        id,
        kind,
        name: terms.name.clone(),
        amount: terms.amount,
        currency: CURRENCY.to_owned(),
        duration: terms.duration,
        price_keys: terms
            .price_keys
            .iter()
            .map(|key| key.as_str().to_owned())
            .collect(),
        starts_at: terms.starts_at,
        ends_at: terms.ends_at,
        stripe_coupon_id: created.id,
        ended_at: None,
        created_by: Some(operator.user),
        created_at: (state.wall)(),
    };
    if let Err(error) = DiscountRepo::new(state.pool.clone())
        .insert(&discount)
        .await
    {
        let _undone = client.delete_coupon(&discount.stripe_coupon_id).await;
        return Err(storage(state, &error));
    }
    Ok(discount)
}

/// The Stripe products a coupon restricted to `keys` names, found by asking
/// Stripe which product each mapped price sells. Stripe restricts coupons
/// by product, so a restriction to one price of a product reaches its other
/// prices too; the checkout's own check is what holds the price-key line.
async fn products_for(state: &AppState, keys: &[PriceKey]) -> Result<Vec<String>, APIError> {
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let mut products = Vec::new();
    for key in keys {
        let Some(price) = state.config.stripe_price_map.price_for(*key) else {
            return Err(invalid(&format!(
                "{} has no Stripe price on this deployment yet.",
                key.as_str()
            )));
        };
        let product = client
            .product_of_price(price)
            .await
            .map_err(|error| stripe_fault(state, &error))?;
        if !products.contains(&product) {
            products.push(product);
        }
    }
    Ok(products)
}

fn view(
    state: &AppState,
    discount: &Discount,
    presentation: Option<SalePresentation>,
) -> DiscountView {
    DiscountView::of(
        discount,
        presentation,
        Vec::new(),
        Some(true),
        (state.wall)(),
    )
}

/// `POST /v1/admin/pricing/sales`: a percentage off every plan for a window,
/// announced with a banner on the pricing pages.
pub(crate) async fn create_sale(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<SaleBody>,
) -> Result<(StatusCode, Json<DiscountView>), APIError> {
    let now = (state.wall)();
    let terms = check_terms(
        &TermsBody {
            name: body.name.clone(),
            percent_off: Some(body.percent_off),
            amount_off_cents: None,
            duration: body.duration.clone(),
            duration_months: body.duration_months,
            price_keys: Vec::new(),
            from: body.from.clone(),
            until: body.until.clone(),
        },
        now,
    )?;
    let banner = body.banner.trim();
    if banner.is_empty() || banner.chars().count() > BANNER_MAX {
        return Err(invalid("Write a banner of up to 140 characters."));
    }
    let banner_href = body
        .banner_href
        .as_deref()
        .map(str::trim)
        .filter(|href| !href.is_empty());
    if banner_href.is_some_and(|href| !(href.starts_with('/') || href.starts_with("https://"))) {
        return Err(invalid("A banner link starts with / or https://."));
    }
    let theme = body.theme.as_deref().filter(|theme| *theme != "none");
    if theme.is_some_and(|theme| !THEMES.contains(&theme)) {
        return Err(invalid(
            "A sale is tied to the halloween or christmas theme, or to none.",
        ));
    }

    // One sale at a time: the pricing pages show one banner and one struck
    // price, and two overlapping sales would leave which one a checkout gets
    // to the order rows came back in.
    let clash = DiscountRepo::new(state.pool.clone())
        .list()
        .await
        .map_err(|e| storage(&state, &e))?
        .into_iter()
        .find(|d| {
            d.kind == DiscountKind::Sale
                && d.ended_at.is_none()
                && d.starts_at.0 < terms.ends_at.0
                && terms.starts_at.0 < d.ends_at.0
        });
    if let Some(clash) = clash {
        return Err(conflict(&format!(
            "\"{}\" already runs {} to {}. End it first, or pick other dates.",
            clash.name,
            date_of(clash.starts_at),
            last_day(&clash)
        )));
    }

    let discount = create_discount(&state, &operator, DiscountKind::Sale, &terms, &[]).await?;
    let presentation = SalePresentation {
        banner: banner.to_owned(),
        banner_href: banner_href.map(str::to_owned),
        theme: theme.map(str::to_owned),
    };
    let value =
        serde_json::to_value(&presentation).map_err(|error| state.internal(&error.to_string()))?;
    let written = SiteSettingRepo::new(state.pool.clone())
        .set(
            &format!("{SALE_SETTING_PREFIX}{}", discount.id.to_hyphenated()),
            &value,
            operator.user,
            now,
        )
        .await;
    if let Err(error) = written {
        // A sale with no banner would strike prices with nothing saying why;
        // withdraw it rather than leave it half-made.
        if let Some(client) = state.config.stripe.as_ref() {
            let _undone = client.delete_coupon(&discount.stripe_coupon_id).await;
        }
        let _ended = DiscountRepo::new(state.pool.clone())
            .end(discount.id, now)
            .await;
        return Err(storage(&state, &error));
    }
    Ok((
        StatusCode::CREATED,
        Json(view(&state, &discount, Some(presentation))),
    ))
}

/// `POST /v1/admin/pricing/discounts`: a one-off discount applied without a
/// code to the price keys it names while its window is open.
pub(crate) async fn create_one_off(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<TermsBody>,
) -> Result<(StatusCode, Json<DiscountView>), APIError> {
    let terms = check_terms(&body, (state.wall)())?;
    let discount = create_discount(&state, &operator, DiscountKind::OneOff, &terms, &[]).await?;
    Ok((StatusCode::CREATED, Json(view(&state, &discount, None))))
}

/// `POST /v1/admin/pricing/codes`: a code sellers type, over a coupon
/// restricted in Stripe to the products its price keys sell — the code also
/// works in the field on Stripe's own page, where our check does not run.
pub(crate) async fn create_code(
    State(state): State<AppState>,
    operator: OperatorContext,
    Json(body): Json<CodeBody>,
) -> Result<(StatusCode, Json<DiscountView>), APIError> {
    let now = (state.wall)();
    let code = body.code.trim();
    let shaped = (3..=40).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !shaped {
        return Err(invalid(
            "A code is 3 to 40 letters, digits, dashes or underscores.",
        ));
    }
    if body.max_redemptions == Some(0) {
        return Err(invalid(
            "A redemption limit is at least 1, or empty for none.",
        ));
    }
    let terms = check_terms(&body.terms, now)?;
    let repo = DiscountRepo::new(state.pool.clone());
    if repo
        .code_taken(code)
        .await
        .map_err(|e| storage(&state, &e))?
    {
        return Err(conflict(
            "That code is already in use. End the old one first.",
        ));
    }
    let covered: Vec<PriceKey> = if terms.price_keys.is_empty() {
        PriceKey::ALL
            .into_iter()
            .filter(|key| is_plan(*key))
            .collect()
    } else {
        terms.price_keys.clone()
    };
    let products = products_for(&state, &covered).await?;
    let discount =
        create_discount(&state, &operator, DiscountKind::Code, &terms, &products).await?;
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let promotion = client
        .create_promotion_code(&PromotionCodeRequest {
            coupon: &discount.stripe_coupon_id,
            code,
            expires_at: Some(Timestamp(terms.ends_at.0 - 1_000)),
            max_redemptions: body.max_redemptions,
        })
        .await;
    let promotion = match promotion {
        Ok(promotion) => promotion,
        Err(error) => {
            // The coupon is useless without its code; withdraw both.
            let _undone = client.delete_coupon(&discount.stripe_coupon_id).await;
            let _ended = repo.end(discount.id, now).await;
            return Err(stripe_fault(&state, &error));
        }
    };
    let stored = DiscountCode {
        id: fresh_uuid(),
        discount_id: discount.id,
        code: code.to_owned(),
        stripe_promotion_code_id: promotion.id,
        max_redemptions: body.max_redemptions,
        ended_at: None,
        created_at: now,
    };
    match repo
        .insert_code(&stored)
        .await
        .map_err(|e| storage(&state, &e))?
    {
        CodeWrite::Stored => {}
        CodeWrite::Taken => {
            let _undone = client
                .deactivate_promotion_code(&stored.stripe_promotion_code_id)
                .await;
            let _undone = client.delete_coupon(&discount.stripe_coupon_id).await;
            let _ended = repo.end(discount.id, now).await;
            return Err(conflict(
                "That code is already in use. End the old one first.",
            ));
        }
    }
    let mut shown = view(&state, &discount, None);
    shown.codes.push(DiscountCodeView {
        id: stored.id.to_hyphenated(),
        code: stored.code,
        max_redemptions: stored.max_redemptions,
        stripe_promotion_code_id: stored.stripe_promotion_code_id,
        ended: false,
    });
    Ok((StatusCode::CREATED, Json(shown)))
}

/// `POST /v1/admin/pricing/{id}/end`: stops offering a discount now.
///
/// Stripe first, then the row: a coupon deleted and a row still open would
/// show a price the checkout can no longer give, which the next save of the
/// page retries; the other order would leave a coupon redeemable on Stripe's
/// page with nothing here saying so. Subscriptions already carrying the
/// coupon keep their discount — deletion stops new redemptions only.
pub(crate) async fn end_discount(
    State(state): State<AppState>,
    _operator: OperatorContext,
    Path((_version, id)): Path<(String, String)>,
) -> Result<Json<DiscountView>, APIError> {
    let Some(id) = Uuid::parse_hyphenated(&id) else {
        return Err(not_found());
    };
    let repo = DiscountRepo::new(state.pool.clone());
    let Some(discount) = repo.get(id).await.map_err(|e| storage(&state, &e))? else {
        return Err(not_found());
    };
    let client = state.config.stripe.as_ref().ok_or_else(unconfigured)?;
    let codes: Vec<DiscountCode> = repo
        .codes()
        .await
        .map_err(|e| storage(&state, &e))?
        .into_iter()
        .filter(|code| code.discount_id == id && code.ended_at.is_none())
        .collect();
    for code in &codes {
        client
            .deactivate_promotion_code(&code.stripe_promotion_code_id)
            .await
            .map_err(|error| stripe_fault(&state, &error))?;
    }
    client
        .delete_coupon(&discount.stripe_coupon_id)
        .await
        .map_err(|error| stripe_fault(&state, &error))?;
    let now = (state.wall)();
    repo.end(id, now).await.map_err(|e| storage(&state, &e))?;
    let ended = Discount {
        ended_at: Some(discount.ended_at.unwrap_or(now)),
        ..discount
    };
    let presentation = presentation(&state, id)
        .await
        .map_err(|e| storage(&state, &e))?;
    Ok(Json(DiscountView::of(
        &ended,
        presentation,
        Vec::new(),
        Some(false),
        now,
    )))
}

#[cfg(test)]
mod tests {
    use super::{best_automatic, is_plan};
    use tam_limits::PriceKey;
    use tam_storage::{Discount, DiscountAmount, DiscountDuration, DiscountKind};
    use tam_types::{Timestamp, Uuid};

    fn discount(byte: u8, kind: DiscountKind, amount: DiscountAmount, keys: &[&str]) -> Discount {
        Discount {
            id: Uuid([byte; 16]),
            kind,
            name: format!("d{byte}"),
            amount,
            currency: "usd".to_owned(),
            duration: DiscountDuration::Once,
            price_keys: keys.iter().map(|key| (*key).to_owned()).collect(),
            starts_at: Timestamp(0),
            ends_at: Timestamp(1_000),
            stripe_coupon_id: format!("c{byte}"),
            ended_at: None,
            created_by: None,
            created_at: Timestamp(0),
        }
    }

    fn a_plan() -> PriceKey {
        PriceKey::ALL
            .into_iter()
            .find(|key| is_plan(*key))
            .unwrap_or(PriceKey::ALL[0])
    }

    fn a_pack() -> PriceKey {
        PriceKey::ALL
            .into_iter()
            .find(|key| !is_plan(*key))
            .unwrap_or(PriceKey::ALL[0])
    }

    #[test]
    fn a_sale_reaches_plans_and_never_packs() {
        let sale = [discount(
            1,
            DiscountKind::Sale,
            DiscountAmount::Percent(25),
            &[],
        )];
        assert!(best_automatic(&sale, a_plan(), Timestamp(10)).is_some());
        assert!(best_automatic(&sale, a_pack(), Timestamp(10)).is_none());
    }

    #[test]
    fn the_larger_saving_wins_whichever_way_it_is_written() {
        let plan = a_plan();
        let list = plan.list_cents();
        let open = [
            discount(1, DiscountKind::Sale, DiscountAmount::Percent(10), &[]),
            // An amount worth more than 10 % of this key's list price: all of it.
            discount(
                2,
                DiscountKind::OneOff,
                DiscountAmount::Cents(list),
                &[plan.as_str()],
            ),
        ];
        let chosen =
            best_automatic(&open, plan, Timestamp(10)).map(|d| d.stripe_coupon_id.as_str());
        assert_eq!(chosen, Some("c2"), "the whole price off beats a tenth off");
    }

    #[test]
    fn codes_ended_and_closed_windows_are_never_automatic() {
        let plan = a_plan();
        let mut ended = discount(2, DiscountKind::Sale, DiscountAmount::Percent(50), &[]);
        ended.ended_at = Some(Timestamp(5));
        let open = [
            discount(1, DiscountKind::Code, DiscountAmount::Percent(90), &[]),
            ended,
        ];
        assert!(best_automatic(&open, plan, Timestamp(10)).is_none());
        let sale = [discount(
            3,
            DiscountKind::Sale,
            DiscountAmount::Percent(25),
            &[],
        )];
        assert!(
            best_automatic(&sale, plan, Timestamp(1_000)).is_none(),
            "the window is half-open: the end instant is outside it"
        );
    }

    #[test]
    fn a_one_off_reaches_only_the_keys_it_names() {
        let pack = a_pack();
        let open = [discount(
            1,
            DiscountKind::OneOff,
            DiscountAmount::Percent(20),
            &[pack.as_str()],
        )];
        assert!(best_automatic(&open, pack, Timestamp(10)).is_some());
        assert!(best_automatic(&open, a_plan(), Timestamp(10)).is_none());
    }
}
