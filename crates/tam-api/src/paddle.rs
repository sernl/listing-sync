//! Paddle's wire concerns, kept pure: the webhook signature, and the instant
//! format its notifications stamp themselves with.
//!
//! Nothing here opens a socket, reads a clock or touches a database. The
//! caller supplies the raw bytes, the header, the secret and the current
//! instant, so every branch below is reachable from a unit test with a
//! constructed vector and no network.
//!
//! The scheme is Paddle's, documented at
//! <https://developer.paddle.com/webhooks/signature-verification>: the
//! `Paddle-Signature` header carries a `ts` and one or more `h1` values, and
//! each `h1` is HMAC-SHA256 over `ts`, a colon, and the raw request body —
//! the bytes as received, before any JSON parse, because a re-serialised body
//! is a different message.

use hmac::{Hmac, Mac};
use subtle::ConstantTimeEq;
use tam_types::Timestamp;

/// How far a notification's own timestamp may sit from our clock, in either
/// direction, before it is refused as a replay.
///
/// Paddle's official SDKs default to five seconds. That is too tight here and
/// deliberately widened: this service runs in New Zealand and Paddle delivers
/// from the northern hemisphere, so a notification crosses the Pacific before
/// it is verified, and five seconds turns ordinary transit and a little clock
/// skew into dropped billing events. Sixty seconds still bounds a captured
/// signature's usefulness to a minute while leaving that transit ample room.
/// The bound is symmetric because a timestamp ahead of our clock is skew in
/// the other direction, and treating it as acceptable would remove the bound
/// entirely for anyone able to set it.
pub const SIGNATURE_TOLERANCE_SECS: i64 = 60;

const MILLIS_PER_SEC: i64 = 1_000;

/// Why one `Paddle-Signature` header did not authenticate its body.
///
/// The variants exist for tests and for a log line the operator reads; the
/// route answers the same refusal for all of them, because telling a caller
/// which check failed tells an attacker which one to work on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureRefusal {
    /// The header is not `ts=<integer>;h1=<hex>[;h1=<hex>...]`.
    HeaderMalformed,
    /// The timestamp parses but sits further than [`SIGNATURE_TOLERANCE_SECS`]
    /// from the instant the caller supplied, in one direction or the other.
    TimestampOutOfTolerance,
    /// The header is well formed and timely, and no `h1` it carries equals
    /// the digest of these bytes under this secret.
    NoDigestMatched,
}

impl core::fmt::Display for SignatureRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::HeaderMalformed => "the Paddle-Signature header is malformed",
            Self::TimestampOutOfTolerance => "the Paddle-Signature timestamp is outside tolerance",
            Self::NoDigestMatched => "no Paddle-Signature digest matched the body",
        })
    }
}

/// The parsed header: the signed timestamp, and every digest offered for it.
///
/// More than one `h1` appears while a webhook secret is being rotated, when
/// Paddle signs each notification under both the old secret and the new one.
/// Accepting any of them is what makes a rotation a configuration change
/// rather than an outage.
struct ParsedSignature<'a> {
    /// The timestamp exactly as it appeared, because it is signed material:
    /// re-rendering it from the parsed integer would change the message for
    /// any spelling that is not the canonical one.
    ts_raw: &'a str,
    ts_secs: i64,
    digests: Vec<&'a str>,
}

fn parse(header: &str) -> Result<ParsedSignature<'_>, SignatureRefusal> {
    let mut ts: Option<(&str, i64)> = None;
    let mut digests = Vec::new();
    for pair in header.split(';') {
        let (key, value) = pair
            .trim()
            .split_once('=')
            .ok_or(SignatureRefusal::HeaderMalformed)?;
        match key.trim() {
            "ts" => {
                let raw = value.trim();
                let secs = raw
                    .parse::<i64>()
                    .map_err(|_| SignatureRefusal::HeaderMalformed)?;
                // A second ts would leave which one was signed ambiguous, and
                // an attacker choosing the ambiguity is the whole attack.
                if ts.replace((raw, secs)).is_some() {
                    return Err(SignatureRefusal::HeaderMalformed);
                }
            }
            "h1" => digests.push(value.trim()),
            _ => return Err(SignatureRefusal::HeaderMalformed),
        }
    }
    let (ts_raw, ts_secs) = ts.ok_or(SignatureRefusal::HeaderMalformed)?;
    if digests.is_empty() {
        return Err(SignatureRefusal::HeaderMalformed);
    }
    Ok(ParsedSignature {
        ts_raw,
        ts_secs,
        digests,
    })
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(rendered, "{byte:02x}");
    }
    rendered
}

/// Verifies one notification's signature over the bytes as received.
///
/// `now` is the caller's instant rather than a clock read here, which is what
/// lets the tolerance test drive both edges of the window without waiting.
pub fn verify(
    header: &str,
    body: &[u8],
    secret: &str,
    now: Timestamp,
) -> Result<(), SignatureRefusal> {
    let signature = parse(header)?;
    let now_secs = now.0.div_euclid(MILLIS_PER_SEC);
    let skew = now_secs.saturating_sub(signature.ts_secs).saturating_abs();
    if skew > SIGNATURE_TOLERANCE_SECS {
        return Err(SignatureRefusal::TimestampOutOfTolerance);
    }

    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|_| SignatureRefusal::NoDigestMatched)?;
    mac.update(signature.ts_raw.as_bytes());
    mac.update(b":");
    mac.update(body);
    let expected = hex(&mac.finalize().into_bytes());

    // Every candidate is compared, and each comparison is length-checked and
    // then constant-time, so neither the number of digests offered nor how
    // far a wrong one agrees is observable in the time this takes.
    let matched = signature
        .digests
        .iter()
        .fold(subtle::Choice::from(0u8), |found, candidate| {
            found | candidate.as_bytes().ct_eq(expected.as_bytes())
        });
    if bool::from(matched) {
        Ok(())
    } else {
        Err(SignatureRefusal::NoDigestMatched)
    }
}

/// Why one notification's `occurred_at` could not be read as an instant.
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

/// Reads Paddle's `occurred_at` — an RFC 3339 instant — as milliseconds since
/// the epoch.
///
/// Hand-written rather than delegated to a date library because the only
/// dependencies this milestone may add are the three signature primitives,
/// and because the accepted shape is narrow: a calendar date, a time, an
/// optional fractional part truncated to milliseconds, and either `Z` or a
/// numeric offset. Anything else is refused rather than guessed at, so a
/// format we have not seen fails visibly instead of landing a wrong instant
/// in the column the ordering fence reads.
///
/// A leap second (`:60`) is accepted and carried arithmetically into the
/// following second. The ordering fence needs monotonicity, not a calendar.
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
/// rounding: Paddle sends microseconds, and the column this feeds is compared
/// for ordering, where truncation preserves order and rounding can invert it
/// across a boundary.
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
    use super::{instant_from_rfc3339, verify, SignatureRefusal, SIGNATURE_TOLERANCE_SECS};
    use hmac::{Hmac, Mac};
    use tam_types::Timestamp;

    const SECRET: &str = "pdl_ntfset_01hv8_the_development_secret";
    const BODY: &[u8] = br#"{"event_type":"subscription.created","data":{"id":"sub_01"}}"#;
    const TS: i64 = 1_800_000_000;

    fn digest(ts: i64, body: &[u8], secret: &str) -> String {
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes())
            .expect("HMAC accepts a key of any length");
        mac.update(ts.to_string().as_bytes());
        mac.update(b":");
        mac.update(body);
        super::hex(&mac.finalize().into_bytes())
    }

    fn header(ts: i64, body: &[u8], secret: &str) -> String {
        format!("ts={ts};h1={}", digest(ts, body, secret))
    }

    fn at(secs: i64) -> Timestamp {
        Timestamp(secs * 1_000)
    }

    #[test]
    fn a_correctly_signed_body_verifies() {
        assert_eq!(
            verify(&header(TS, BODY, SECRET), BODY, SECRET, at(TS)),
            Ok(()),
            "the vector this module constructs is the vector it accepts"
        );
    }

    #[test]
    fn a_signature_under_another_secret_is_refused() {
        assert_eq!(
            verify(&header(TS, BODY, "another-secret"), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "only the configured secret authenticates a notification"
        );
    }

    #[test]
    fn a_tampered_body_is_refused() {
        let signed = header(TS, BODY, SECRET);
        let tampered = br#"{"event_type":"subscription.created","data":{"id":"sub_02"}}"#;
        assert_eq!(
            verify(&signed, tampered, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "the digest covers the body, so one changed byte refuses"
        );
    }

    #[test]
    fn a_body_of_the_same_length_signed_for_a_different_timestamp_is_refused() {
        let signed = header(TS, BODY, SECRET);
        let restamped = signed.replace(&TS.to_string(), &(TS + 1).to_string());
        assert_eq!(
            verify(&restamped, BODY, SECRET, at(TS + 1)),
            Err(SignatureRefusal::NoDigestMatched),
            "the timestamp is signed material, so moving it invalidates the digest"
        );
    }

    #[test]
    fn a_stale_timestamp_is_refused_beyond_the_tolerance() {
        let stale = TS - SIGNATURE_TOLERANCE_SECS - 1;
        assert_eq!(
            verify(&header(stale, BODY, SECRET), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::TimestampOutOfTolerance),
            "a captured signature stops being useful once the window closes"
        );
    }

    #[test]
    fn a_future_timestamp_is_refused_beyond_the_tolerance() {
        let ahead = TS + SIGNATURE_TOLERANCE_SECS + 1;
        assert_eq!(
            verify(&header(ahead, BODY, SECRET), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::TimestampOutOfTolerance),
            "the window is symmetric; skew forward is skew"
        );
    }

    #[test]
    fn the_tolerance_edges_are_inside_the_window() {
        for edge in [TS - SIGNATURE_TOLERANCE_SECS, TS + SIGNATURE_TOLERANCE_SECS] {
            assert_eq!(
                verify(&header(edge, BODY, SECRET), BODY, SECRET, at(TS)),
                Ok(()),
                "the bound is inclusive, so a Pacific crossing of exactly \
                 {SIGNATURE_TOLERANCE_SECS}s is accepted"
            );
        }
    }

    #[test]
    fn any_one_of_several_digests_authenticates_during_rotation() {
        let old = digest(TS, BODY, "the-retiring-secret");
        let new = digest(TS, BODY, SECRET);
        assert_eq!(
            verify(&format!("ts={TS};h1={old};h1={new}"), BODY, SECRET, at(TS)),
            Ok(()),
            "the new secret's digest is accepted though it is not the first offered"
        );
        assert_eq!(
            verify(&format!("ts={TS};h1={new};h1={old}"), BODY, SECRET, at(TS)),
            Ok(()),
            "and accepted when it is first, so order does not decide"
        );
    }

    #[test]
    fn several_wrong_digests_are_still_refused() {
        let a = digest(TS, BODY, "wrong-one");
        let b = digest(TS, BODY, "wrong-two");
        assert_eq!(
            verify(&format!("ts={TS};h1={a};h1={b}"), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "accepting any digest is not accepting every digest"
        );
    }

    #[test]
    fn malformed_header_shapes_are_refused_before_any_digest_is_computed() {
        let good = digest(TS, BODY, SECRET);
        for (shape, why) in [
            (String::new(), "the empty header"),
            (format!("h1={good}"), "no timestamp at all"),
            (format!("ts={TS}"), "a timestamp with no digest"),
            (
                format!("ts=notanumber;h1={good}"),
                "an unparsable timestamp",
            ),
            (format!("ts={TS};h1={good};ts={TS}"), "two timestamps"),
            (
                format!("ts={TS},h1={good}"),
                "comma separators, which are Stripe's scheme rather than Paddle's",
            ),
            (format!("ts={TS};{good}"), "a pair carrying no '='"),
            (
                format!("ts={TS};h2={good}"),
                "an unknown key, which must not be skipped past",
            ),
        ] {
            assert_eq!(
                verify(&shape, BODY, SECRET, at(TS)),
                Err(SignatureRefusal::HeaderMalformed),
                "{why} is malformed"
            );
        }
    }

    #[test]
    fn whitespace_around_the_pairs_is_tolerated() {
        let good = digest(TS, BODY, SECRET);
        assert_eq!(
            verify(&format!(" ts={TS} ; h1={good} "), BODY, SECRET, at(TS)),
            Ok(()),
            "padding around the separators does not change the signed material"
        );
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
    fn paddles_microsecond_precision_truncates_to_milliseconds() {
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
                Err(super::NotAnInstant),
                "{shape:?} must refuse rather than land a wrong instant"
            );
        }
    }
}
