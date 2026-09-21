//! Reading and writing the one instant format this crate parses by hand.
//!
//! Hand-written rather than delegated to a date library because this crate
//! holds no calendar dependency and the accepted shape is narrow: a calendar
//! date, a time, an optional fractional part truncated to milliseconds, and
//! either `Z` or a numeric offset. Anything else is refused rather than
//! guessed at, so a format we have not seen fails visibly instead of landing
//! a wrong instant in a column something orders by.
//!
//! It lived beside the Paddle webhook until the billing rail moved to Stripe,
//! which stamps its events with Unix seconds and needs none of this. The
//! parser stayed because it is still the definition `export::rfc3339` writes
//! the inverse of, and a renderer with no reader is a format nobody can check.

use tam_types::Timestamp;

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

fn digits(raw: &str, width: usize) -> Result<i64, NotAnInstant> {
    if raw.len() != width || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotAnInstant);
    }
    raw.parse().map_err(|_| NotAnInstant)
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
    use super::{instant_from_rfc3339, NotAnInstant};
    use tam_types::Timestamp;

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
