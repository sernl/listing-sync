//! The lease's cookie jar: the sealed credential parsed into pairs, updated
//! by the `Set-Cookie` headers the upstream sends back, and rendered into the
//! `Cookie` header the gateway injects on the next hop.
//!
//! This type exists because a session that cannot absorb its own renewals can
//! only age. The Tes longevity probe keeps a session authenticated for days
//! by handing curl `-b`/`-c` so every `Set-Cookie` persists; the broker's
//! sealed copy, which forwarded responses and dropped their `Set-Cookie`
//! headers, went stale within a day against the same account. The difference
//! is entirely this jar.
//!
//! Two deletion shapes are honoured — `Max-Age` at or below zero, and an
//! empty value, which is the form `name=; Expires=<past>` takes. A deletion
//! expressed only through a past `Expires` date and a non-empty value is not
//! recognised, because recognising it needs an HTTP-date parser and no new
//! dependency is worth a case no observed response uses; such a cookie stays
//! in the jar at its previous value until the upstream overwrites it.

/// Insertion-ordered `name=value` pairs. Order is preserved across updates so
/// that a renewal which changes no value renders byte-identically and the
/// reseal debounce sees no change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CookieJar {
    pairs: Vec<(String, String)>,
}

impl CookieJar {
    /// Parses a `Cookie` request header: `name=value` pairs separated by
    /// `;`. A pair carrying no `=` is skipped rather than guessed at.
    pub(crate) fn from_cookie_header(header: &str) -> Self {
        let mut jar = Self { pairs: Vec::new() };
        for pair in header.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=') {
                let name = name.trim();
                if !name.is_empty() {
                    jar.set(name, value.trim());
                }
            }
        }
        jar
    }

    /// The `Cookie` header this jar renders to.
    pub(crate) fn to_cookie_header(&self) -> String {
        self.pairs
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// One cookie's current value, which is how the Tpt lease finds the
    /// `csrfToken` it must mirror into a header.
    pub(crate) fn value(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn set(&mut self, name: &str, value: &str) {
        match self.pairs.iter_mut().find(|(key, _)| key == name) {
            Some(existing) => value.clone_into(&mut existing.1),
            None => self.pairs.push((name.to_owned(), value.to_owned())),
        }
    }

    fn remove(&mut self, name: &str) {
        self.pairs.retain(|(key, _)| key != name);
    }

    /// Applies one `Set-Cookie` response header. Returns whether the jar's
    /// rendered value changed, which is what the reseal debounce reads: a
    /// renewal restating a value the jar already holds is not a change and
    /// must not cost a database write.
    pub(crate) fn apply_set_cookie(&mut self, header: &str) -> bool {
        let mut attributes = header.split(';');
        let Some((name, value)) = attributes
            .next()
            .and_then(|pair| pair.trim().split_once('='))
        else {
            return false;
        };
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let value = value.trim();
        let deleted = value.is_empty() || expired_by_max_age(attributes);

        let before = self.value(name).map(str::to_owned);
        if deleted {
            if before.is_none() {
                return false;
            }
            self.remove(name);
            return true;
        }
        if before.as_deref() == Some(value) {
            return false;
        }
        self.set(name, value);
        true
    }
}

/// `Max-Age` at or below zero is a deletion. A malformed value is not treated
/// as one: an unparseable attribute is a reason to leave the jar alone, not a
/// reason to drop a live session cookie.
fn expired_by_max_age<'a>(attributes: impl Iterator<Item = &'a str>) -> bool {
    for attribute in attributes {
        if let Some((key, value)) = attribute.trim().split_once('=') {
            if key.trim().eq_ignore_ascii_case("max-age") {
                return value.trim().parse::<i64>().is_ok_and(|age| age <= 0);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::CookieJar;

    #[test]
    fn a_cookie_header_round_trips_through_the_jar() {
        let jar = CookieJar::from_cookie_header("a=1; b=2;  c=3 ");
        assert_eq!(
            jar.to_cookie_header(),
            "a=1; b=2; c=3",
            "parsing and rendering must agree so an unchanged jar reseals to the same bytes"
        );
    }

    #[test]
    fn a_renewal_replaces_the_value_in_place_and_reports_the_change() {
        let mut jar = CookieJar::from_cookie_header("session=old; other=keep");
        assert!(
            jar.apply_set_cookie("session=new; Path=/; HttpOnly; Secure"),
            "a changed value is a change"
        );
        assert_eq!(
            jar.to_cookie_header(),
            "session=new; other=keep",
            "the renewed cookie keeps its position, so an unrelated renewal cannot reorder the header"
        );
    }

    #[test]
    fn a_restated_value_is_not_a_change_and_must_not_cost_a_reseal() {
        let mut jar = CookieJar::from_cookie_header("session=same");
        assert!(
            !jar.apply_set_cookie("session=same; Path=/"),
            "a Set-Cookie restating the value the jar already holds changes nothing"
        );
    }

    #[test]
    fn a_new_cookie_is_appended() {
        let mut jar = CookieJar::from_cookie_header("a=1");
        assert!(
            jar.apply_set_cookie("b=2"),
            "a cookie the jar lacks is a change"
        );
        assert_eq!(jar.to_cookie_header(), "a=1; b=2");
    }

    #[test]
    fn max_age_zero_deletes_and_a_second_deletion_is_not_a_change() {
        let mut jar = CookieJar::from_cookie_header("doomed=1; kept=2");
        assert!(
            jar.apply_set_cookie("doomed=1; Max-Age=0"),
            "a deletion of a cookie the jar holds is a change"
        );
        assert_eq!(jar.to_cookie_header(), "kept=2");
        assert!(
            !jar.apply_set_cookie("doomed=1; Max-Age=0"),
            "deleting what is already gone changes nothing, so it must not reseal"
        );
    }

    #[test]
    fn an_empty_value_deletes_which_is_the_shape_expires_in_the_past_takes() {
        let mut jar = CookieJar::from_cookie_header("doomed=1; kept=2");
        assert!(
            jar.apply_set_cookie("doomed=; Expires=Thu, 01 Jan 1970 00:00:00 GMT"),
            "the standard deletion shape must clear the cookie"
        );
        assert_eq!(jar.to_cookie_header(), "kept=2");
    }

    #[test]
    fn a_malformed_max_age_does_not_drop_a_live_cookie() {
        let mut jar = CookieJar::from_cookie_header("session=live");
        assert!(
            !jar.apply_set_cookie("session=live; Max-Age=not-a-number"),
            "an unparseable attribute is no reason to change the jar"
        );
        assert_eq!(
            jar.value("session"),
            Some("live"),
            "the session cookie must survive an attribute the parser cannot read"
        );
    }

    #[test]
    fn a_header_without_an_equals_sign_is_ignored() {
        let mut jar = CookieJar::from_cookie_header("a=1");
        assert!(
            !jar.apply_set_cookie("garbage"),
            "a pair with no value is not a cookie"
        );
        assert_eq!(jar.to_cookie_header(), "a=1");
    }

    #[test]
    fn the_csrf_cookie_is_readable_by_name_for_the_tpt_double_submit() {
        let jar = CookieJar::from_cookie_header("sessionKey=abc; csrfToken=deadbeef");
        assert_eq!(
            jar.value("csrfToken"),
            Some("deadbeef"),
            "the Tpt lease mirrors this value into x-csrf-token, so the jar must expose it"
        );
    }
}
