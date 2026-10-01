//! Reading and writing the one instant format this crate parses by hand, and
//! the company's calendar.
//!
//! The instant parser is hand-written rather than delegated to a date library
//! because the accepted shape is narrow: a calendar date, a time, an optional
//! fractional part truncated to milliseconds, and either `Z` or a numeric
//! offset. Anything else is refused rather than guessed at, so a format we
//! have not seen fails visibly instead of landing a wrong instant in a column
//! something orders by.
//!
//! It lived beside the Paddle webhook until the billing rail moved to Stripe,
//! which stamps its events with Unix seconds and needs none of this. The
//! parser stayed because it is still the definition `export::rfc3339` writes
//! the inverse of, and a renderer with no reader is a format nobody can check.
//!
//! The calendar is [`SITE_TIMEZONE`]'s. A day an operator types — a sale's
//! first day, a theme's last — is a day in New Zealand, where Teachouse
//! operates, and begins at midnight there: 1 October starts at 11:00 UTC on
//! 30 September while New Zealand keeps daylight time. Turning a day into an
//! instant needs that zone's rules, which is the one thing here `chrono-tz`
//! answers.

use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use tam_types::Timestamp;

/// The zone every calendar day on the admin pages is a day in.
pub const SITE_TIMEZONE: Tz = chrono_tz::Pacific::Auckland;

/// Days from 0001-01-01, `chrono`'s day one, to 1970-01-01.
const EPOCH_DAYS_FROM_CE: i64 = 719_163;

const MILLIS_PER_SEC: i64 = 1_000;

/// Why one timestamp could not be read as an instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAnInstant;

impl core::fmt::Display for NotAnInstant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("the timestamp is not an RFC 3339 instant")
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian civil date, by Howard
/// Hinnant's `days_from_civil`, which is exact over the whole range these
/// values can take and needs no table.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    // `div_euclid` floors, which is exactly what the reference algorithm's
    // `y >= 0 ? y : y - 399` emulates with truncating division.
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_shift = if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * (month + month_shift) + 2).div_euclid(5) + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era.div_euclid(4) - year_of_era.div_euclid(100) + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Hinnant's `civil_from_days`, the inverse of [`days_from_civil`], exact over the range a stored timestamp
/// can hold.
pub(crate) const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era.div_euclid(1_460) + day_of_era.div_euclid(36_524)
        - day_of_era.div_euclid(146_096))
    .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era.div_euclid(4) - year_of_era.div_euclid(100));
    let month_phase = (5 * day_of_year + 2).div_euclid(153);
    let date = day_of_year - (153 * month_phase + 2).div_euclid(5) + 1;
    let month = if month_phase < 10 {
        month_phase + 3
    } else {
        month_phase - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, date)
}

/// The instant one site day — days since 1970-01-01 — begins: midnight in
/// [`SITE_TIMEZONE`]. `None` only outside `chrono`'s range of years.
///
/// New Zealand moves its clocks at 02:00 and 03:00 and never at midnight, so
/// a midnight always exists and is never repeated; `earliest` is asked for
/// rather than that being assumed.
fn site_midnight(day: i64) -> Option<Timestamp> {
    let since_ce = i32::try_from(day.checked_add(EPOCH_DAYS_FROM_CE)?).ok()?;
    let midnight = NaiveDate::from_num_days_from_ce_opt(since_ce)?.and_hms_opt(0, 0, 0)?;
    SITE_TIMEZONE
        .from_local_datetime(&midnight)
        .earliest()
        .map(|at| Timestamp(at.timestamp_millis()))
}

/// The instant one `YYYY-MM-DD` site day begins: midnight in New Zealand.
///
/// A date that does not exist — 2026-02-30 — is refused rather than rolled
/// into March, by [`days_from_date`].
pub fn site_day_start(raw: &str) -> Result<Timestamp, NotAnInstant> {
    site_midnight(days_from_date(raw)?).ok_or(NotAnInstant)
}

/// The instant one `YYYY-MM-DD` site day ends: midnight in New Zealand at the
/// start of the next day. Not the start plus 24 hours, which is a day of 23
/// or 25 hours when the clocks change.
pub fn site_day_end(raw: &str) -> Result<Timestamp, NotAnInstant> {
    let next = days_from_date(raw)?.checked_add(1).ok_or(NotAnInstant)?;
    site_midnight(next).ok_or(NotAnInstant)
}

/// The site day an instant falls on, as days since 1970-01-01: the date a
/// calendar in New Zealand shows at that instant.
#[must_use]
pub fn site_day(at: Timestamp) -> i64 {
    match Utc.timestamp_millis_opt(at.0).single() {
        Some(utc) => {
            i64::from(utc.with_timezone(&SITE_TIMEZONE).date_naive().num_days_from_ce())
                - EPOCH_DAYS_FROM_CE
        }
        // Hundreds of thousands of years out, where `chrono` stops; no stored
        // instant is there, and the UTC day is the nearest honest answer.
        None => at.0.div_euclid(86_400 * MILLIS_PER_SEC),
    }
}

/// The `YYYY-MM-DD` site day an instant falls on.
#[must_use]
pub fn site_date_of(at: Timestamp) -> String {
    let (year, month, day) = civil_from_days(site_day(at));
    format!("{year:04}-{month:02}-{day:02}")
}

fn digits(raw: &str, width: usize) -> Result<i64, NotAnInstant> {
    if raw.len() != width || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotAnInstant);
    }
    raw.parse().map_err(|_| NotAnInstant)
}

/// Reads a calendar date, `YYYY-MM-DD`, as days since 1970-01-01.
///
/// Strict about the day of the month, unlike the instant parser's coarse
/// `1..=31`: a date an operator types into a form is shown back to them, and
/// `2026-02-30` accepted would be shown as a day that does not exist.
pub fn days_from_date(raw: &str) -> Result<i64, NotAnInstant> {
    let (year, month_day) = raw.split_once('-').ok_or(NotAnInstant)?;
    let (month, day) = month_day.split_once('-').ok_or(NotAnInstant)?;
    let (year, month, day) = (digits(year, 4)?, digits(month, 2)?, digits(day, 2)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(NotAnInstant),
    };
    if !(1..=length).contains(&day) {
        return Err(NotAnInstant);
    }
    Ok(days_from_civil(year, month, day))
}


/// Reads an RFC 3339 instant as milliseconds since the epoch.
///
/// A leap second (`:60`) is accepted and carried arithmetically into the
/// following second. An ordering fence needs monotonicity, not a calendar.
pub fn instant_from_rfc3339(raw: &str) -> Result<Timestamp, NotAnInstant> {
    let (date, rest) = raw.split_once('T').ok_or(NotAnInstant)?;
    let (year, month_day) = date.split_once('-').ok_or(NotAnInstant)?;
    let (month, day) = month_day.split_once('-').ok_or(NotAnInstant)?;
    let (year, month, day) = (digits(year, 4)?, digits(month, 2)?, digits(day, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(NotAnInstant);
    }

    let (clock, offset_secs) = split_offset(rest)?;
    let (hour, minute_rest) = clock.split_once(':').ok_or(NotAnInstant)?;
    let (minute, second_rest) = minute_rest.split_once(':').ok_or(NotAnInstant)?;
    let (second, fraction) = match second_rest.split_once('.') {
        Some((second, fraction)) => (second, fraction),
        None => (second_rest, ""),
    };
    let (hour, minute, second) = (digits(hour, 2)?, digits(minute, 2)?, digits(second, 2)?);
    if hour > 23 || minute > 59 || second > 60 {
        return Err(NotAnInstant);
    }
    let millis = fraction_millis(fraction)?;

    let seconds = days_from_civil(year, month, day)
        .checked_mul(86_400)
        .and_then(|days| days.checked_add(hour * 3_600 + minute * 60 + second))
        .and_then(|secs| secs.checked_sub(offset_secs))
        .ok_or(NotAnInstant)?;
    seconds
        .checked_mul(MILLIS_PER_SEC)
        .and_then(|millis_part| millis_part.checked_add(millis))
        .map(Timestamp)
        .ok_or(NotAnInstant)
}

/// Splits the time-of-day from its zone designator, answering the offset in
/// seconds to subtract to reach UTC.
fn split_offset(rest: &str) -> Result<(&str, i64), NotAnInstant> {
    if let Some(clock) = rest.strip_suffix('Z').or_else(|| rest.strip_suffix('z')) {
        return Ok((clock, 0));
    }
    let split = rest
        .rfind(['+', '-'])
        .filter(|index| *index > 0)
        .ok_or(NotAnInstant)?;
    let (clock, zone) = rest.split_at_checked(split).ok_or(NotAnInstant)?;
    let signed = i64::from(zone.starts_with('-')) * -2 + 1;
    let (hours, minutes) = zone
        .get(1..)
        .ok_or(NotAnInstant)?
        .split_once(':')
        .ok_or(NotAnInstant)?;
    let (hours, minutes) = (digits(hours, 2)?, digits(minutes, 2)?);
    if hours > 23 || minutes > 59 {
        return Err(NotAnInstant);
    }
    Ok((clock, signed * (hours * 3_600 + minutes * 60)))
}

/// The fractional second as whole milliseconds, truncating rather than
/// rounding: a sub-millisecond source truncates in order and rounds out of
/// it across a boundary, and the columns this feeds are compared for
/// ordering.
fn fraction_millis(fraction: &str) -> Result<i64, NotAnInstant> {
    if fraction.is_empty() {
        return Ok(0);
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotAnInstant);
    }
    let mut millis = 0;
    for place in 0..3 {
        let digit = fraction
            .as_bytes()
            .get(place)
            .map_or(0, |byte| i64::from(byte - b'0'));
        millis = millis * 10 + digit;
    }
    Ok(millis)
}

#[cfg(test)]
mod tests {
    use super::{
        days_from_date, instant_from_rfc3339, site_date_of, site_day, site_day_end, site_day_start,
        NotAnInstant,
    };
    use tam_types::Timestamp;

    fn at(raw: &str) -> Timestamp {
        instant_from_rfc3339(raw).unwrap_or(Timestamp(i64::MIN))
    }

    /// A New Zealand day begins at its own midnight: 13 hours ahead of UTC in
    /// daylight time, 12 in standard time.
    #[test]
    fn a_site_day_begins_at_midnight_in_new_zealand() {
        assert_eq!(site_day_start("2026-10-01"), Ok(at("2026-09-30T11:00:00Z")));
        assert_eq!(site_day_start("2026-07-01"), Ok(at("2026-06-30T12:00:00Z")));
        // Daylight time began at 02:00 on Sunday 27 September 2026.
        assert_eq!(site_day_start("2026-09-27"), Ok(at("2026-09-26T12:00:00Z")));
        assert_eq!(site_day_start("2026-09-28"), Ok(at("2026-09-27T11:00:00Z")));
        assert_eq!(site_day_start("2026-02-30"), Err(NotAnInstant));
        // 4 April 2027 has 25 hours: daylight time ends at 03:00 that day.
        assert_eq!(site_day_end("2027-04-04"), Ok(at("2027-04-04T12:00:00Z")));
        assert_eq!(site_day_start("2027-04-04"), Ok(at("2027-04-03T11:00:00Z")));
    }

    #[test]
    fn the_site_day_turns_at_midnight_in_new_zealand() {
        assert_eq!(site_day(at("2026-09-30T10:59:59Z")), days_from_date("2026-09-30").unwrap_or(0));
        assert_eq!(site_day(at("2026-09-30T11:00:00Z")), days_from_date("2026-10-01").unwrap_or(0));
        assert_eq!(site_date_of(at("2026-09-30T11:00:00Z")), "2026-10-01");
        assert_eq!(site_date_of(at("2026-10-31T10:59:59.999Z")), "2026-10-31");
    }

    #[test]
    fn the_epoch_and_a_known_instant_read_back_exactly() {
        assert_eq!(
            instant_from_rfc3339("1970-01-01T00:00:00Z"),
            Ok(Timestamp(0)),
            "the epoch is zero"
        );
        assert_eq!(
            instant_from_rfc3339("2026-08-30T12:34:56Z"),
            Ok(Timestamp(1_788_093_296_000)),
            "a known instant reads back as the milliseconds it is"
        );
    }

    #[test]
    fn microsecond_precision_truncates_to_milliseconds() {
        assert_eq!(
            instant_from_rfc3339("2026-08-30T12:34:56.789123Z"),
            Ok(Timestamp(1_788_093_296_789)),
            "microseconds truncate rather than round, so ordering is preserved"
        );
        assert_eq!(
            instant_from_rfc3339("2026-08-30T12:34:56.7Z"),
            Ok(Timestamp(1_788_093_296_700)),
            "a short fraction is padded on the right, not the left"
        );
    }

    #[test]
    fn a_numeric_offset_is_carried_back_to_utc() {
        assert_eq!(
            instant_from_rfc3339("2026-08-30T12:34:56+00:00"),
            instant_from_rfc3339("2026-08-30T12:34:56Z"),
            "a zero offset and Z name one instant"
        );
        assert_eq!(
            instant_from_rfc3339("2026-08-31T00:34:56+12:00"),
            instant_from_rfc3339("2026-08-30T12:34:56Z"),
            "a New Zealand offset resolves to the same instant"
        );
        assert_eq!(
            instant_from_rfc3339("2026-08-30T07:34:56-05:00"),
            instant_from_rfc3339("2026-08-30T12:34:56Z"),
            "and so does a western one"
        );
    }

    #[test]
    fn a_leap_year_boundary_is_exact() {
        assert_eq!(
            instant_from_rfc3339("2024-02-29T00:00:00Z"),
            Ok(Timestamp(1_709_164_800_000)),
            "the civil-date algorithm handles the century rules without a table"
        );
    }

    #[test]
    fn instants_order_the_way_their_text_does() {
        let earlier =
            instant_from_rfc3339("2026-08-30T12:34:56.100000Z").expect("the earlier instant reads");
        let later =
            instant_from_rfc3339("2026-08-30T12:34:56.200000Z").expect("the later instant reads");
        assert!(
            earlier < later,
            "the ordering fence in storage depends on this: {earlier:?} < {later:?}"
        );
    }

    #[test]
    fn shapes_that_are_not_instants_are_refused_rather_than_guessed_at() {
        for shape in [
            "",
            "2026-08-30",
            "2026-08-30T12:34:56",
            "2026-8-30T12:34:56Z",
            "2026-13-01T00:00:00Z",
            "2026-08-30T24:00:00Z",
            "2026-08-30T12:61:00Z",
            "2026-08-30T12:34:56.12x4Z",
            "not-a-timestamp",
        ] {
            assert_eq!(
                instant_from_rfc3339(shape),
                Err(NotAnInstant),
                "{shape:?} must refuse rather than land a wrong instant"
            );
        }
    }
}
