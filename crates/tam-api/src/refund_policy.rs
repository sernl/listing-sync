//! The refund policy of the Terms (`teachouse.io/terms/#refunds`), as one
//! pure function of what was bought and when it is asked about.
//!
//! - A Move Pack none of whose moves were used is refunded in full within 14
//!   days of buying it, and not at all after that or once a move is used.
//! - A monthly plan's month is not refunded once it has started.
//! - A yearly plan refunds the whole months that have not started, less one
//!   month's fee: `price × max(0, unused whole months − 1) ÷ 12`, rounded
//!   down to the cent.
//!
//! Months and days are New Zealand's ([`SITE_TIMEZONE`]), counted from the
//! period's start: month *k* starts on the same day and time of day *k − 1*
//! calendar months later (the last day of a shorter month, where the day is
//! missing from it), and it counts as used from that instant. A start time a
//! daylight-saving change skips moves forward by the hour the clocks did, and
//! one the change repeats is its first occurrence. The 14 days of a pack end
//! at the same time of day 14 New Zealand days after the purchase.
//!
//! Nothing here reads Stripe or the database: [`crate::refund_quote`] finds
//! the purchase behind a charge and asks this module what it is owed, so the
//! policy is tested on its own, edge by edge.

use chrono::{DateTime, Days, LocalResult, Months, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tam_types::Timestamp;

use crate::payments::money;
use crate::time::SITE_TIMEZONE;

/// The months a yearly plan is divided into, and the denominator of its
/// refund.
pub const MONTHS_PER_YEAR: i64 = 12;

/// How long an unused Move Pack stays refundable, in New Zealand days.
pub const PACK_REFUND_DAYS: u64 = 14;

const MILLIS_PER_HOUR: i64 = 3_600_000;

/// What a charge paid for, as far as the policy distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseKind {
    Pack,
    Monthly,
    Yearly,
}

/// Which rule of the policy decided a quote. Stored beside every refund
/// issued against a quote (migration 0107), so the record says which rule
/// the amount came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundBasis {
    /// A Move Pack within its 14 days with no move used: in full.
    PackUnused,
    /// A Move Pack with at least one move used: nothing.
    PackUsed,
    /// A Move Pack bought more than 14 days ago: nothing.
    PackWindowClosed,
    /// A monthly plan's month that has started: nothing.
    MonthlyStarted,
    /// A monthly plan's month that has not started yet: in full.
    MonthlyNotStarted,
    /// A yearly plan inside its year: the unused whole months less one.
    YearlyUnusedMonths,
    /// A yearly plan whose year is over: nothing.
    YearlyEnded,
    /// A charge nothing could match to a plan or a pack. Never answered by
    /// [`policy_refund`]; the quote route's answer when there is no purchase
    /// to apply the policy to.
    Unmatched,
}

impl RefundBasis {
    pub const ALL: [Self; 8] = [
        Self::PackUnused,
        Self::PackUsed,
        Self::PackWindowClosed,
        Self::MonthlyStarted,
        Self::MonthlyNotStarted,
        Self::YearlyUnusedMonths,
        Self::YearlyEnded,
        Self::Unmatched,
    ];

    /// The stored spelling, which migration 0107's CHECKs enumerate.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PackUnused => "pack_unused",
            Self::PackUsed => "pack_used",
            Self::PackWindowClosed => "pack_window_closed",
            Self::MonthlyStarted => "monthly_started",
            Self::MonthlyNotStarted => "monthly_not_started",
            Self::YearlyUnusedMonths => "yearly_unused_months",
            Self::YearlyEnded => "yearly_ended",
            Self::Unmatched => "unmatched",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|basis| basis.as_str() == raw)
    }
}

/// One purchase as the policy reads it.
///
/// `price_cents` is what the charge took. `period_start` is when the plan's
/// period began, or when the pack was bought; `period_end` is when the
/// period ends, or when the pack's moves expire. `pack_moves_used` is read
/// only for a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purchase<'a> {
    pub kind: PurchaseKind,
    pub price_cents: i64,
    pub currency: &'a str,
    pub period_start: Timestamp,
    pub period_end: Timestamp,
    pub pack_moves_used: i64,
}

/// What the policy owes on one purchase, which rule said so, and the
/// sentence that says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundQuote {
    pub amount_cents: i64,
    pub basis: RefundBasis,
    pub explanation: String,
}

/// The refund the policy gives `purchase` at `now`.
#[must_use]
pub fn policy_refund(purchase: &Purchase<'_>, now: Timestamp) -> RefundQuote {
    match purchase.kind {
        PurchaseKind::Pack => pack_refund(purchase, now),
        PurchaseKind::Monthly => monthly_refund(purchase, now),
        PurchaseKind::Yearly => yearly_refund(purchase, now),
    }
}

/// `price × max(0, unused − 1) ÷ 12`, rounded down to the cent.
#[must_use]
pub fn yearly_amount(price_cents: i64, unused_months: i64) -> i64 {
    let kept = unused_months.saturating_sub(1).clamp(0, MONTHS_PER_YEAR);
    price_cents
        .max(0)
        .saturating_mul(kept)
        .div_euclid(MONTHS_PER_YEAR)
}

/// How many of a year's months have started at `now`, counting from
/// `start` in New Zealand: 0 before the start, 12 from the twelfth month on.
#[must_use]
pub fn months_started(start: Timestamp, now: Timestamp) -> i64 {
    let local = nz_local(start);
    let mut started = 0;
    for k in 0..12_u32 {
        let Some(month_start) = local.checked_add_months(Months::new(k)) else {
            break;
        };
        if nz_instant(month_start).0 > now.0 {
            break;
        }
        started = i64::from(k).saturating_add(1);
    }
    started
}

/// The instant an unused pack bought at `bought` stops being refundable:
/// the same time of day, [`PACK_REFUND_DAYS`] New Zealand days later.
#[must_use]
pub fn pack_deadline(bought: Timestamp) -> Timestamp {
    nz_local(bought)
        .checked_add_days(Days::new(PACK_REFUND_DAYS))
        .map_or(Timestamp(i64::MAX), nz_instant)
}

/// "3 October 2026", the day `at` falls on in New Zealand.
#[must_use]
pub fn nz_long_date(at: Timestamp) -> String {
    nz_local(at).format("%-d %B %Y").to_string()
}

fn pack_refund(purchase: &Purchase<'_>, now: Timestamp) -> RefundQuote {
    let bought = nz_long_date(purchase.period_start);
    if now.0 >= pack_deadline(purchase.period_start).0 {
        return RefundQuote {
            amount_cents: 0,
            basis: RefundBasis::PackWindowClosed,
            explanation: format!(
                "This Move Pack was bought on {bought}, more than {PACK_REFUND_DAYS} days ago, \
                 so it isn't refunded."
            ),
        };
    }
    if purchase.pack_moves_used > 0 {
        let used = purchase.pack_moves_used;
        let (noun, verb) = if used == 1 {
            ("move", "has")
        } else {
            ("moves", "have")
        };
        return RefundQuote {
            amount_cents: 0,
            basis: RefundBasis::PackUsed,
            explanation: format!(
                "{used} {noun} from this Move Pack {verb} been used, so it isn't refunded."
            ),
        };
    }
    let price = purchase.price_cents.max(0);
    RefundQuote {
        amount_cents: price,
        basis: RefundBasis::PackUnused,
        explanation: format!(
            "This Move Pack was bought on {bought} and none of its moves have been used, \
             so it is refunded in full: {}.",
            money(price, purchase.currency)
        ),
    }
}

fn monthly_refund(purchase: &Purchase<'_>, now: Timestamp) -> RefundQuote {
    let start = nz_long_date(purchase.period_start);
    if now.0 < purchase.period_start.0 {
        let price = purchase.price_cents.max(0);
        return RefundQuote {
            amount_cents: price,
            basis: RefundBasis::MonthlyNotStarted,
            explanation: format!(
                "This month of the plan starts on {start}, so it hasn't started and is \
                 refunded in full: {}.",
                money(price, purchase.currency)
            ),
        };
    }
    RefundQuote {
        amount_cents: 0,
        basis: RefundBasis::MonthlyStarted,
        explanation: format!(
            "This month of the plan started on {start}, and a month that has started isn't \
             refunded. Cancelling stops the next renewal."
        ),
    }
}

fn yearly_refund(purchase: &Purchase<'_>, now: Timestamp) -> RefundQuote {
    if now.0 >= purchase.period_end.0 {
        return RefundQuote {
            amount_cents: 0,
            basis: RefundBasis::YearlyEnded,
            explanation: format!(
                "This yearly plan's year ended on {}, so there is nothing left to refund.",
                nz_long_date(purchase.period_end)
            ),
        };
    }
    let started = months_started(purchase.period_start, now);
    let unused = MONTHS_PER_YEAR.saturating_sub(started);
    let kept = unused.saturating_sub(1).max(0);
    let amount = yearly_amount(purchase.price_cents, unused);
    let price = money(purchase.price_cents.max(0), purchase.currency);
    let position = if started == 0 {
        format!(
            "This yearly plan starts on {}, so all 12 months are unused",
            nz_long_date(purchase.period_start)
        )
    } else {
        match unused {
            0 => format!("This is month {started} of the yearly plan, so every month has started"),
            1 => format!("This is month {started} of the yearly plan, so only month 12 is unused"),
            _ => format!(
                "This is month {started} of the yearly plan, so months {} to 12 are unused: \
                 {unused} whole months",
                started.saturating_add(1)
            ),
        }
    };
    let outcome = if kept == 0 {
        "Less one month's fee, there is nothing left to refund.".to_owned()
    } else {
        format!(
            "Less one month's fee, the refund is {kept} × {price} ÷ 12 = {}.",
            money(amount, purchase.currency)
        )
    };
    RefundQuote {
        amount_cents: amount,
        basis: RefundBasis::YearlyUnusedMonths,
        explanation: format!("{position}. {outcome}"),
    }
}

/// The New Zealand wall-clock time of an instant.
fn nz_local(at: Timestamp) -> NaiveDateTime {
    DateTime::<Utc>::from_timestamp_millis(at.0)
        .unwrap_or_default()
        .with_timezone(&SITE_TIMEZONE)
        .naive_local()
}

/// The instant a New Zealand wall-clock time names. A time the spring
/// change skips is read an hour later, where the clocks went; a time the
/// autumn change repeats is its first occurrence.
fn nz_instant(local: NaiveDateTime) -> Timestamp {
    let resolve = |naive: NaiveDateTime| match SITE_TIMEZONE.from_local_datetime(&naive) {
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Some(at.timestamp_millis()),
        LocalResult::None => None,
    };
    let skipped = local.checked_add_signed(chrono::TimeDelta::milliseconds(MILLIS_PER_HOUR));
    Timestamp(
        resolve(local)
            .or_else(|| skipped.and_then(resolve))
            .unwrap_or(i64::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        months_started, nz_instant, pack_deadline, policy_refund, yearly_amount, Purchase,
        PurchaseKind, RefundBasis,
    };
    use chrono::NaiveDate;
    use tam_types::Timestamp;

    const PRICE: i64 = 24_000;
    const MINUTE: i64 = 60_000;
    const DAY: i64 = 86_400_000;

    /// A New Zealand wall-clock time as an instant.
    fn nz(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Timestamp {
        nz_instant(
            NaiveDate::from_ymd_opt(year, month, day)
                .and_then(|date| date.and_hms_opt(hour, minute, 0))
                .expect("a real calendar time"),
        )
    }

    fn utc(raw: &str) -> Timestamp {
        Timestamp(
            chrono::DateTime::parse_from_rfc3339(raw)
                .expect("an RFC 3339 instant")
                .timestamp_millis(),
        )
    }

    fn yearly(start: Timestamp, end: Timestamp) -> Purchase<'static> {
        Purchase {
            kind: PurchaseKind::Yearly,
            price_cents: PRICE,
            currency: "usd",
            period_start: start,
            period_end: end,
            pack_moves_used: 0,
        }
    }

    fn pack(bought: Timestamp, used: i64) -> Purchase<'static> {
        Purchase {
            kind: PurchaseKind::Pack,
            price_cents: 2_900,
            currency: "usd",
            period_start: bought,
            period_end: Timestamp(bought.0 + 365 * DAY),
            pack_moves_used: used,
        }
    }

    fn year_from(start: Timestamp) -> Purchase<'static> {
        yearly(start, Timestamp(start.0 + 366 * DAY))
    }

    #[test]
    fn a_refund_on_the_first_day_leaves_eleven_unused_months() {
        let start = nz(2026, 10, 5, 9, 0);
        let quote = policy_refund(&year_from(start), Timestamp(start.0 + 3_600_000));
        assert_eq!(months_started(start, Timestamp(start.0 + 3_600_000)), 1);
        assert_eq!(quote.basis, RefundBasis::YearlyUnusedMonths);
        assert_eq!(quote.amount_cents, 20_000, "10 × $240 ÷ 12");
        assert!(
            quote
                .explanation
                .contains("months 2 to 12 are unused: 11 whole months"),
            "{}",
            quote.explanation
        );
    }

    #[test]
    fn the_terms_worked_example_is_seven_twelfths() {
        // Pro yearly at $240, asked during month 4: months 5 to 12 unused.
        let start = nz(2026, 1, 15, 10, 0);
        let quote = policy_refund(&year_from(start), nz(2026, 4, 20, 12, 0));
        assert_eq!(quote.amount_cents, 14_000);
        assert_eq!(
            quote.explanation,
            "This is month 4 of the yearly plan, so months 5 to 12 are unused: 8 whole months. \
             Less one month's fee, the refund is 7 × $240.00 ÷ 12 = $140.00."
        );
    }

    #[test]
    fn from_month_eleven_there_is_nothing_left() {
        let start = nz(2026, 1, 15, 10, 0);
        let eleven = policy_refund(&year_from(start), nz(2026, 11, 15, 10, 0));
        assert_eq!(eleven.amount_cents, 0);
        assert!(eleven.explanation.contains("only month 12 is unused"));
        assert!(eleven.explanation.contains("nothing left to refund"));
        let twelve = policy_refund(&year_from(start), nz(2026, 12, 31, 23, 0));
        assert_eq!(twelve.amount_cents, 0);
        assert_eq!(twelve.basis, RefundBasis::YearlyUnusedMonths);
        assert!(twelve.explanation.contains("every month has started"));
        let ten = policy_refund(&year_from(start), nz(2026, 10, 15, 10, 0));
        assert_eq!(ten.amount_cents, 2_000, "month 10: 2 unused, 1 × $240 ÷ 12");
    }

    #[test]
    fn a_year_that_is_over_refunds_nothing() {
        let start = nz(2025, 6, 1, 8, 0);
        let end = nz(2026, 6, 1, 8, 0);
        let quote = policy_refund(&yearly(start, end), end);
        assert_eq!(quote.basis, RefundBasis::YearlyEnded);
        assert_eq!(quote.amount_cents, 0);
        assert!(quote.explanation.contains("ended on 1 June 2026"));
    }

    #[test]
    fn a_month_is_used_from_the_instant_it_starts() {
        let start = nz(2026, 1, 15, 10, 0);
        let month_five = nz(2026, 5, 15, 10, 0);
        assert_eq!(months_started(start, Timestamp(month_five.0 - 1)), 4);
        assert_eq!(months_started(start, month_five), 5);
        assert_eq!(
            policy_refund(&year_from(start), Timestamp(month_five.0 - 1)).amount_cents,
            14_000
        );
        assert_eq!(
            policy_refund(&year_from(start), month_five).amount_cents,
            12_000
        );
    }

    #[test]
    fn before_the_start_no_month_is_used_and_one_month_is_still_kept() {
        let start = nz(2026, 11, 1, 0, 0);
        let quote = policy_refund(&year_from(start), nz(2026, 10, 30, 0, 0));
        assert_eq!(months_started(start, nz(2026, 10, 30, 0, 0)), 0);
        assert_eq!(quote.amount_cents, 22_000, "11 × $240 ÷ 12");
        assert!(quote.explanation.contains("all 12 months are unused"));
    }

    #[test]
    fn a_month_end_start_lands_on_the_last_day_of_a_short_month() {
        // Leap year: month 2 of a 31 January start begins on 29 February.
        let leap = nz(2028, 1, 31, 10, 0);
        assert_eq!(months_started(leap, nz(2028, 2, 29, 9, 59)), 1);
        assert_eq!(months_started(leap, nz(2028, 2, 29, 10, 0)), 2);
        // Common year: on 28 February.
        let common = nz(2027, 1, 31, 10, 0);
        assert_eq!(months_started(common, nz(2027, 2, 28, 9, 59)), 1);
        assert_eq!(months_started(common, nz(2027, 2, 28, 10, 0)), 2);
        // And month 3 is back on the 31st.
        assert_eq!(months_started(common, nz(2027, 3, 31, 9, 59)), 2);
        assert_eq!(months_started(common, nz(2027, 3, 31, 10, 0)), 3);
    }

    #[test]
    fn a_leap_day_start_counts_in_new_zealand_days() {
        let start = nz(2028, 2, 29, 12, 0);
        assert_eq!(months_started(start, nz(2028, 3, 29, 11, 59)), 1);
        assert_eq!(months_started(start, nz(2028, 3, 29, 12, 0)), 2);
        assert_eq!(months_started(start, nz(2029, 1, 29, 12, 0)), 12);
    }

    #[test]
    fn months_are_new_zealand_months_not_utc_months() {
        // 00:30 on 1 March in Auckland is still 28 February in UTC.
        let start = nz(2027, 3, 1, 0, 30);
        assert_eq!(start, utc("2027-02-28T11:30:00Z"));
        // Month 2 starts at 00:30 on 1 April in Auckland (still NZDT, UTC+13,
        // until the first Sunday in April), which is 31 March in UTC, not
        // 1 April UTC.
        assert_eq!(months_started(start, utc("2027-03-31T11:29:00Z")), 1);
        assert_eq!(months_started(start, utc("2027-03-31T11:30:00Z")), 2);
    }

    #[test]
    fn a_start_time_the_spring_change_skips_moves_forward_an_hour() {
        // 02:30 on 27 September 2026 does not exist in Auckland: the clocks
        // go from 02:00 NZST to 03:00 NZDT. Month 2 of a 27 August 02:30
        // start therefore begins at 03:30 NZDT, 14:30 UTC the day before.
        let start = nz(2026, 8, 27, 2, 30);
        assert_eq!(start, utc("2026-08-26T14:30:00Z"));
        assert_eq!(months_started(start, utc("2026-09-26T14:29:59Z")), 1);
        assert_eq!(months_started(start, utc("2026-09-26T14:30:00Z")), 2);
    }

    #[test]
    fn a_start_time_the_autumn_change_repeats_is_its_first_occurrence() {
        // 02:30 on 4 April 2027 happens twice in Auckland; the first is NZDT
        // (UTC+13), 13:30 UTC on 3 April.
        let start = nz(2027, 3, 4, 2, 30);
        assert_eq!(months_started(start, utc("2027-04-03T13:29:59Z")), 1);
        assert_eq!(months_started(start, utc("2027-04-03T13:30:00Z")), 2);
    }

    #[test]
    fn the_yearly_amount_rounds_down_to_the_cent() {
        assert_eq!(
            yearly_amount(19_900, 11),
            16_583,
            "10 × $199 ÷ 12 = $165.833…"
        );
        assert_eq!(yearly_amount(24_000, 1), 0);
        assert_eq!(yearly_amount(24_000, 0), 0);
        assert_eq!(yearly_amount(24_000, 12), 22_000);
        assert_eq!(yearly_amount(-5, 12), 0, "a negative price refunds nothing");
    }

    #[test]
    fn a_monthly_plan_is_not_refunded_once_its_month_has_started() {
        let start = nz(2026, 10, 1, 9, 0);
        let monthly = Purchase {
            kind: PurchaseKind::Monthly,
            price_cents: 2_900,
            currency: "usd",
            period_start: start,
            period_end: nz(2026, 11, 1, 9, 0),
            pack_moves_used: 0,
        };
        let started = policy_refund(&monthly, start);
        assert_eq!(started.amount_cents, 0);
        assert_eq!(started.basis, RefundBasis::MonthlyStarted);
        assert!(started.explanation.contains("started on 1 October 2026"));
        let later = policy_refund(&monthly, nz(2026, 10, 2, 9, 0));
        assert_eq!(later.amount_cents, 0);
        let early = policy_refund(&monthly, Timestamp(start.0 - MINUTE));
        assert_eq!(early.basis, RefundBasis::MonthlyNotStarted);
        assert_eq!(early.amount_cents, 2_900);
    }

    #[test]
    fn an_unused_pack_is_refunded_in_full_through_day_fourteen() {
        let bought = nz(2026, 10, 1, 9, 0);
        let first = policy_refund(&pack(bought, 0), bought);
        assert_eq!(first.basis, RefundBasis::PackUnused);
        assert_eq!(first.amount_cents, 2_900);
        assert!(first.explanation.contains("bought on 1 October 2026"));
        let last = policy_refund(&pack(bought, 0), nz(2026, 10, 15, 8, 59));
        assert_eq!(last.amount_cents, 2_900, "still day 14");
    }

    #[test]
    fn an_unused_pack_on_day_fifteen_refunds_nothing() {
        let bought = nz(2026, 10, 1, 9, 0);
        let late = policy_refund(&pack(bought, 0), nz(2026, 10, 15, 9, 0));
        assert_eq!(late.basis, RefundBasis::PackWindowClosed);
        assert_eq!(late.amount_cents, 0);
        assert!(late.explanation.contains("more than 14 days ago"));
    }

    #[test]
    fn a_pack_with_one_move_used_refunds_nothing() {
        let bought = nz(2026, 10, 1, 9, 0);
        let one = policy_refund(&pack(bought, 1), nz(2026, 10, 2, 9, 0));
        assert_eq!(one.basis, RefundBasis::PackUsed);
        assert_eq!(one.amount_cents, 0);
        assert_eq!(
            one.explanation,
            "1 move from this Move Pack has been used, so it isn't refunded."
        );
        let many = policy_refund(&pack(bought, 3), nz(2026, 10, 2, 9, 0));
        assert!(many
            .explanation
            .starts_with("3 moves from this Move Pack have"));
    }

    #[test]
    fn the_pack_window_is_fourteen_new_zealand_days_across_daylight_saving() {
        // Bought at noon NZST on 20 September 2026; the clocks go forward on
        // the 27th, so 14 Auckland days later is 23 hours short of 14 × 24.
        let bought = nz(2026, 9, 20, 12, 0);
        assert_eq!(bought, utc("2026-09-20T00:00:00Z"));
        assert_eq!(pack_deadline(bought), utc("2026-10-03T23:00:00Z"));
        let after = policy_refund(&pack(bought, 0), utc("2026-10-03T23:30:00Z"));
        assert_eq!(
            after.basis,
            RefundBasis::PackWindowClosed,
            "noon on 4 October in Auckland has passed even though 14 × 24 hours have not"
        );
        let before = policy_refund(&pack(bought, 0), utc("2026-10-03T22:59:00Z"));
        assert_eq!(before.basis, RefundBasis::PackUnused);
    }

    #[test]
    fn the_release_notes_worked_examples_hold() {
        // Pro yearly at $240, started at noon on 20 September 2026 in Auckland.
        let start = nz(2026, 9, 20, 12, 0);
        let plan = yearly(start, nz(2027, 9, 20, 12, 0));
        assert_eq!(
            policy_refund(&plan, nz(2026, 9, 20, 15, 0)).amount_cents,
            20_000
        );
        assert_eq!(
            policy_refund(&plan, nz(2027, 1, 15, 21, 0)).amount_cents,
            14_000
        );
        assert_eq!(policy_refund(&plan, nz(2027, 7, 25, 9, 0)).amount_cents, 0);
    }

    #[test]
    fn every_basis_round_trips_its_stored_spelling() {
        for basis in RefundBasis::ALL {
            assert_eq!(RefundBasis::parse(basis.as_str()), Some(basis));
            assert_eq!(
                serde_json::to_value(basis).expect("serialises"),
                serde_json::json!(basis.as_str()),
                "the wire spelling is the stored one"
            );
        }
        assert_eq!(RefundBasis::parse("generous"), None);
    }
}
