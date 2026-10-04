//! The purchase behind one Stripe charge, and what the refund policy
//! ([`crate::refund_policy`]) owes on it today.
//!
//! A charge names neither a plan nor a pack, so the purchase is traced:
//!
//! 1. The invoice it paid, which a subscription's charges have: named by
//!    the charge itself on API versions before 2025-03-31; found in the
//!    ledger's `invoice_paid` rows by charge or payment intent; or, on later
//!    versions, asked of Stripe's invoice payments by payment intent. The
//!    invoice's lines name the price (so the plan and its cadence) and the
//!    service period the policy counts months in.
//! 2. Otherwise the Checkout Session its payment intent completed, which a
//!    pack's charge has. Its line items name the pack, and the move ledger
//!    says how much of the pack is left.
//!
//! A charge neither traces to is quoted at nothing with
//! [`RefundBasis::Unmatched`]: an operator refunds it by judgement, and says
//! why. The price the policy reads is what the charge took, tax and any
//! discount included, because that is what is being returned.
//!
//! The quote never exceeds what Stripe says is left on the charge, so a
//! refund already made (here or in Stripe's dashboard) counts against it.

use serde::{Deserialize, Serialize};
use tam_limits::{PriceKey, PACKS};
use tam_storage::{EntitlementRepo, PackUse, PaymentRepo};
use tam_types::{OrgId, Timestamp};

use crate::billing::InvoiceObject;
use crate::error::APIError;
use crate::payments::{charge_org, money, not_found, storage, stripe_fault};
use crate::refund_policy::{policy_refund, Purchase, PurchaseKind, RefundBasis, RefundQuote};
use crate::stripe::{self, Client, StripeError};
use crate::AppState;

const MILLIS_PER_SEC: i64 = 1_000;
const MILLIS_PER_DAY: i64 = 86_400_000;

/// How long a pack's moves last, which is the pack's `period_end`.
const PACK_VALIDITY_DAYS: i64 = 365;

/// One charge's quote, as the Payments page's refund sheet and the billing
/// page's "Ask for a refund" read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteView {
    pub charge_id: String,
    pub org_id: Option<String>,
    /// What the charge bought, as a person says it: "Pro yearly plan",
    /// "Move Pack of 50 moves". Absent for an unmatched charge.
    pub what: Option<String>,
    pub paid_cents: i64,
    pub currency: String,
    pub paid_at: Timestamp,
    /// What Stripe says is left to refund on the charge.
    pub remaining_cents: i64,
    /// What the policy gives, capped at `remaining_cents`.
    pub amount_cents: i64,
    pub basis: RefundBasis,
    /// The policy's sentence about this charge, ending with what was already
    /// refunded where that capped the amount.
    pub explanation: String,
    /// Whether refunding this charge ends the plan today, as the Terms say a
    /// yearly plan's refund does.
    pub ends_plan: bool,
}

/// What a charge was traced to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Bought {
    /// A plan's period, by its subscription.
    Plan { subscription: String, key: PriceKey },
    /// A Move Pack, by the Checkout Session that bought it.
    Pack { session: String, key: PriceKey },
}

/// One charge, traced and quoted.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub(crate) charge: stripe::Charge,
    pub(crate) org: Option<OrgId>,
    pub(crate) bought: Option<Bought>,
    pub(crate) view: QuoteView,
}

impl Resolved {
    /// The yearly subscription refunding this charge ends, if it is one.
    pub(crate) fn yearly_subscription(&self) -> Option<&str> {
        match &self.bought {
            Some(Bought::Plan { subscription, key })
                if cadence(*key) == Some(PurchaseKind::Yearly) =>
            {
                Some(subscription)
            }
            Some(Bought::Plan { .. } | Bought::Pack { .. }) | None => None,
        }
    }

    /// The Checkout Session of the pack this charge bought, if it is one.
    pub(crate) fn pack_session(&self) -> Option<&str> {
        match &self.bought {
            Some(Bought::Pack { session, .. }) => Some(session),
            Some(Bought::Plan { .. }) | None => None,
        }
    }
}

/// The policy's kind for a price key: a pack, or a plan's cadence.
const fn cadence(key: PriceKey) -> Option<PurchaseKind> {
    match key {
        PriceKey::StarterMonthly | PriceKey::ProMonthly | PriceKey::StudioMonthly => {
            Some(PurchaseKind::Monthly)
        }
        PriceKey::StarterYearly | PriceKey::ProYearly | PriceKey::StudioYearly => {
            Some(PurchaseKind::Yearly)
        }
        PriceKey::Pack20
        | PriceKey::Pack50
        | PriceKey::Pack100
        | PriceKey::Pack250
        | PriceKey::Pack500 => None,
    }
}

/// "Pro yearly plan", "Move Pack of 50 moves".
pub(crate) fn label_for(key: PriceKey) -> String {
    let plan = |name: &str, every: &str| format!("{name} {every} plan");
    match key {
        PriceKey::StarterMonthly => plan("Starter", "monthly"),
        PriceKey::StarterYearly => plan("Starter", "yearly"),
        PriceKey::ProMonthly => plan("Pro", "monthly"),
        PriceKey::ProYearly => plan("Pro", "yearly"),
        PriceKey::StudioMonthly => plan("Studio", "monthly"),
        PriceKey::StudioYearly => plan("Studio", "yearly"),
        PriceKey::Pack20
        | PriceKey::Pack50
        | PriceKey::Pack100
        | PriceKey::Pack250
        | PriceKey::Pack500 => PACKS.iter().find(|pack| pack.key == key).map_or_else(
            || "Move Pack".to_owned(),
            |pack| format!("Move Pack of {} moves", pack.moves),
        ),
    }
}

fn at(unix_secs: i64) -> Timestamp {
    Timestamp(unix_secs.saturating_mul(MILLIS_PER_SEC))
}

/// A Stripe refusal that means "nothing here" rather than a fault: the
/// object or the list does not exist on this account's API version.
const fn absent(error: &StripeError) -> bool {
    matches!(error, StripeError::Api { status, .. } if *status >= 400 && *status < 500)
}

/// The invoice a charge paid, by the three routes the module header names.
async fn invoice_of(
    state: &AppState,
    client: &Client,
    charge: &stripe::Charge,
) -> Result<Option<InvoiceObject>, APIError> {
    let read = |raw: serde_json::Value| serde_json::from_value::<InvoiceObject>(raw).ok();
    if let Some(invoice) = &charge.invoice {
        return match client.retrieve_invoice(invoice).await {
            Ok(raw) => Ok(read(raw)),
            Err(error) if absent(&error) => Ok(None),
            Err(error) => Err(stripe_fault(state, &error)),
        };
    }
    let mut ids = vec![charge.id.clone()];
    ids.extend(charge.payment_intent.clone());
    let ledger = PaymentRepo::new(state.pool.clone())
        .invoice_paid_by(&ids)
        .await
        .map_err(|error| storage(state, &error))?;
    if let Some(raw) = ledger {
        return Ok(read(raw));
    }
    let Some(intent) = &charge.payment_intent else {
        return Ok(None);
    };
    let paid = match client.invoice_paid_by_intent(intent).await {
        Ok(paid) => paid,
        Err(error) if absent(&error) => None,
        Err(error) => return Err(stripe_fault(state, &error)),
    };
    let Some(invoice) = paid else {
        return Ok(None);
    };
    match client.retrieve_invoice(&invoice).await {
        Ok(raw) => Ok(read(raw)),
        Err(error) if absent(&error) => Ok(None),
        Err(error) => Err(stripe_fault(state, &error)),
    }
}

/// The pack a charge's Checkout Session bought, and when.
async fn pack_of(
    state: &AppState,
    client: &Client,
    charge: &stripe::Charge,
) -> Result<Option<(String, PriceKey, Timestamp)>, APIError> {
    let Some(intent) = &charge.payment_intent else {
        return Ok(None);
    };
    let session = match client.checkout_session_for_intent(intent).await {
        Ok(session) => session,
        Err(error) if absent(&error) => None,
        Err(error) => return Err(stripe_fault(state, &error)),
    };
    let Some(session) = session else {
        return Ok(None);
    };
    let key = session
        .purchased()
        .into_iter()
        .find_map(|(price, _quantity)| {
            state
                .config
                .stripe_price_map
                .key_for(price)
                .filter(|key| cadence(*key).is_none())
        });
    let bought = if session.created > 0 {
        session.created
    } else {
        charge.created
    };
    Ok(key.map(|key| (session.id, key, at(bought))))
}

/// The moves of a pack the ledger can show were used: those it stamped as
/// drawn on the pack, or, where the balance has fallen below the pack's
/// size, at least the shortfall, since the pack's own moves cannot all be
/// standing then. Never more than the pack held.
fn pack_moves_used(pack: &PackUse) -> i64 {
    pack.drawn
        .max(pack.moves.saturating_sub(pack.balance))
        .clamp(0, pack.moves.max(0))
}

fn unmatched() -> RefundQuote {
    RefundQuote {
        amount_cents: 0,
        basis: RefundBasis::Unmatched,
        explanation: "This payment couldn't be matched to a plan or a Move Pack, so the refund \
                      policy gives no amount."
            .to_owned(),
    }
}

/// What the charge bought and the policy's quote on it.
async fn trace(
    state: &AppState,
    client: &Client,
    charge: &stripe::Charge,
    org: Option<OrgId>,
    now: Timestamp,
) -> Result<(Option<Bought>, RefundQuote), APIError> {
    let purchase = |kind, start: Timestamp, end: Timestamp, used: i64| Purchase {
        kind,
        price_cents: charge.amount,
        currency: &charge.currency,
        period_start: start,
        period_end: end,
        pack_moves_used: used,
    };
    if let Some(invoice) = invoice_of(state, client, charge).await? {
        let key = invoice
            .price()
            .and_then(|price| state.config.stripe_price_map.key_for(price));
        let (start, end) = invoice.period();
        if let (Some(subscription), Some(key), Some(start), Some(end)) =
            (invoice.subscription_id(), key, start, end)
        {
            if let Some(kind) = cadence(key) {
                let quote = policy_refund(&purchase(kind, at(start), at(end), 0), now);
                let bought = Bought::Plan {
                    subscription: subscription.to_owned(),
                    key,
                };
                return Ok((Some(bought), quote));
            }
        }
        return Ok((None, unmatched()));
    }
    let Some((session, key, bought_at)) = pack_of(state, client, charge).await? else {
        return Ok((None, unmatched()));
    };
    let bought = Bought::Pack {
        session: session.clone(),
        key,
    };
    let Some(org) = org else {
        return Ok((Some(bought), unmatched()));
    };
    let ledger = EntitlementRepo::new(state.pool.clone())
        .pack_use(org, &session, now)
        .await
        .map_err(|error| storage(state, &error))?;
    let Some(pack) = ledger else {
        return Ok((Some(bought), unmatched()));
    };
    let end = Timestamp(
        bought_at
            .0
            .saturating_add(PACK_VALIDITY_DAYS.saturating_mul(MILLIS_PER_DAY)),
    );
    let quote = policy_refund(
        &purchase(PurchaseKind::Pack, bought_at, end, pack_moves_used(&pack)),
        now,
    );
    Ok((Some(bought), quote))
}

/// Reads one charge from Stripe, traces it, and quotes it at `now`.
pub(crate) async fn resolve(
    state: &AppState,
    client: &Client,
    charge_id: &str,
    now: Timestamp,
) -> Result<Resolved, APIError> {
    let charge = match client.retrieve_charge(charge_id).await {
        Ok(charge) => charge,
        Err(StripeError::Api { status: 404, .. }) => {
            return Err(not_found("There is no payment with that id in Stripe."))
        }
        Err(error) => return Err(stripe_fault(state, &error)),
    };
    let repo = PaymentRepo::new(state.pool.clone());
    let org = charge_org(state, &repo, &charge)
        .await
        .map_err(|error| storage(state, &error))?;
    let (bought, quote) = trace(state, client, &charge, org, now).await?;
    let remaining = charge.amount.saturating_sub(charge.amount_refunded).max(0);
    let amount = quote.amount_cents.min(remaining);
    let explanation = if quote.amount_cents > remaining {
        format!(
            "{} {} has already been refunded on this payment, so {} is left to refund.",
            quote.explanation,
            money(charge.amount_refunded, &charge.currency),
            money(remaining, &charge.currency)
        )
    } else {
        quote.explanation
    };
    let ends_plan = matches!(
        &bought,
        Some(Bought::Plan { key, .. }) if cadence(*key) == Some(PurchaseKind::Yearly)
    ) && amount > 0;
    let view = QuoteView {
        charge_id: charge.id.clone(),
        org_id: org.map(|org| org.0.to_hyphenated()),
        what: bought.as_ref().map(|bought| match bought {
            Bought::Plan { key, .. } | Bought::Pack { key, .. } => label_for(*key),
        }),
        paid_cents: charge.amount,
        currency: charge.currency.clone(),
        paid_at: at(charge.created),
        remaining_cents: remaining,
        amount_cents: amount,
        basis: quote.basis,
        explanation,
        ends_plan,
    };
    Ok(Resolved {
        charge,
        org,
        bought,
        view,
    })
}

#[cfg(test)]
mod tests {
    use super::{label_for, pack_moves_used};
    use tam_limits::PriceKey;
    use tam_storage::PackUse;

    fn pack(moves: i64, drawn: i64, balance: i64) -> PackUse {
        PackUse {
            moves,
            expires_at: None,
            drawn,
            refunded: false,
            balance,
        }
    }

    #[test]
    fn a_pack_is_unused_while_nothing_was_drawn_on_it_and_the_balance_covers_it() {
        assert_eq!(pack_moves_used(&pack(20, 0, 20)), 0);
        assert_eq!(
            pack_moves_used(&pack(20, 0, 95)),
            0,
            "moves spent from older credits that expire sooner leave the pack untouched"
        );
    }

    #[test]
    fn a_pack_is_used_by_what_was_drawn_on_it_or_the_balance_shortfall() {
        assert_eq!(pack_moves_used(&pack(20, 1, 40)), 1);
        assert_eq!(
            pack_moves_used(&pack(20, 0, 19)),
            1,
            "a balance below the pack's size means one of its moves is gone"
        );
        assert_eq!(
            pack_moves_used(&pack(20, 0, 0)),
            20,
            "never more than the pack held"
        );
    }

    #[test]
    fn a_purchase_is_named_as_a_teacher_says_it() {
        assert_eq!(label_for(PriceKey::ProYearly), "Pro yearly plan");
        assert_eq!(label_for(PriceKey::StarterMonthly), "Starter monthly plan");
        assert_eq!(label_for(PriceKey::Pack50), "Move Pack of 50 moves");
    }
}
