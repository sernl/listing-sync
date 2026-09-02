//! Where a marketplace login happens, and how the device knows it worked.
//!
//! The login runs in a webview this application owns, on the seller's machine,
//! because you can no longer attach to the seller's existing browser: Chrome
//! 136 stopped honouring `--remote-debugging-port` against the default profile,
//! and reading cookies out of the profile is the infostealer pattern that
//! App-Bound Encryption exists to block. Section 7 of
//! `docs/notes/design/client-side-architecture.md` records both.
//!
//! The logged-in condition is per marketplace and is read off the cookie jar
//! rather than off the page, because the jar is what the adapter will actually
//! authenticate with. Getting it from the page would mean agreeing with the
//! markup; getting it from the jar means agreeing with the transport.

use tam_types::Marketplace;

use crate::session::CookieJar;

/// A marketplace whose automation is sanctioned by an official API, and which
/// therefore has no business opening a login window on the seller's device.
///
/// This is the two-branch rule made unrepresentable rather than remembered:
/// there is no way to obtain a [`LoginTarget`] for an `OfficialApi`
/// marketplace, so no code path in this crate can capture a session for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotSellerDevice(pub Marketplace);

impl core::fmt::Display for NotSellerDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:?} publishes an official API, so its automation runs server-side under that \
             token and this device never logs in to it",
            self.0
        )
    }
}

impl core::error::Error for NotSellerDevice {}

/// One marketplace's login page and the jar that proves the login took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginTarget {
    pub marketplace: Marketplace,
    /// The page the login webview opens. Both are confirmed by the D12 probe;
    /// the Tes one is the sign-in form rather than `/login`, which is a chooser
    /// page carrying no password input at all.
    pub login_url: &'static str,
    /// The origin the captured cookies are read for.
    pub cookie_origin: &'static str,
    /// Every one of these must be present.
    pub required: &'static [&'static str],
    /// At least one of these must be present, when the slice is non-empty.
    pub required_any: &'static [&'static str],
}

impl LoginTarget {
    /// Whether this jar constitutes a logged-in session.
    ///
    /// A conservative reading: it asks for the cookies the adapter cannot work
    /// without, and stops there. It is a first-contact heuristic against the
    /// committed fixtures rather than a live-verified condition, and
    /// `docs/notes/design/desktop-client.md` records that.
    #[must_use]
    pub fn is_logged_in(&self, jar: &CookieJar) -> bool {
        let all_present = self.required.iter().all(|name| jar.contains(name));
        let any_present =
            self.required_any.is_empty() || self.required_any.iter().any(|name| jar.contains(name));
        all_present && any_present
    }
}

/// TeachersPayTeachers. The CSRF cookie is not optional: `TptSession` refuses
/// a jar without it, because the adapter mirrors its value into
/// `x-csrf-token` and cannot authorise a single request otherwise.
const TPT: LoginTarget = LoginTarget {
    marketplace: Marketplace::Tpt,
    login_url: "https://www.teacherspayteachers.com/Login",
    cookie_origin: "https://www.teacherspayteachers.com",
    required: &[TPT_CSRF_COOKIE],
    required_any: &["sessionKey", "TPT"],
};

/// The same name `crates/tam-marketplace-tpt/src/session.rs` exports as
/// `CSRF_COOKIE`, repeated rather than imported: this crate does not otherwise
/// depend on the adapter, and one string is not worth that edge.
const TPT_CSRF_COOKIE: &str = "csrfToken";

/// Tes. The session cookie name is taken from the committed cassette fixtures
/// and the broker's vault tests, which are the only evidence in this tree.
const TES: LoginTarget = LoginTarget {
    marketplace: Marketplace::Tes,
    login_url:
        "https://www.tes.com/authn/sign-in?rtn=https%3A%2F%2Fwww.tes.com%2Fteaching-resources",
    cookie_origin: "https://www.tes.com",
    required: &["TESSession"],
    required_any: &[],
};

/// The login page for a marketplace, or a refusal for one that has an API.
pub const fn login_target(marketplace: Marketplace) -> Result<LoginTarget, NotSellerDevice> {
    match marketplace {
        Marketplace::Tpt => Ok(TPT),
        Marketplace::Tes => Ok(TES),
        Marketplace::Etsy => Err(NotSellerDevice(marketplace)),
    }
}

#[cfg(test)]
mod tests {
    use super::{login_target, NotSellerDevice};
    use crate::session::{Cookie, CookieJar};
    use tam_types::{Marketplace, TransportClass};

    fn jar(names: &[&str]) -> CookieJar {
        CookieJar::new(
            names
                .iter()
                .map(|name| Cookie {
                    name: (*name).to_owned(),
                    value: "value".to_owned(),
                })
                .collect(),
        )
    }

    #[test]
    fn a_login_target_exists_for_exactly_the_seller_device_marketplaces() {
        for marketplace in Marketplace::ALL {
            let target = login_target(marketplace);
            match marketplace.transport_class() {
                TransportClass::SellerDevice => assert!(
                    target.is_ok(),
                    "{marketplace:?} has no official API, so its login must happen on the \
                     seller's device"
                ),
                TransportClass::OfficialApi => assert_eq!(
                    target,
                    Err(NotSellerDevice(marketplace)),
                    "{marketplace:?} is the sanctioned branch; opening a login window for it \
                     would put a marketplace session on the device that the server should be \
                     holding a token for"
                ),
            }
        }
    }

    #[test]
    fn tpt_needs_the_csrf_cookie_and_a_session_cookie() {
        let target = login_target(Marketplace::Tpt).expect("tpt is a seller-device marketplace");
        assert!(target.is_logged_in(&jar(&["csrfToken", "sessionKey"])));
        assert!(
            target.is_logged_in(&jar(&["csrfToken", "TPT", "_ga"])),
            "either session cookie spelling counts, and unrelated cookies do not interfere"
        );
        assert!(
            !target.is_logged_in(&jar(&["sessionKey"])),
            "without csrfToken the adapter cannot authorise one request, so this is not a \
             usable session however logged in the page looks"
        );
        assert!(
            !target.is_logged_in(&jar(&["csrfToken"])),
            "a CSRF cookie is handed out before login too, so on its own it proves nothing"
        );
        assert!(!target.is_logged_in(&jar(&[])));
    }

    #[test]
    fn tes_needs_its_session_cookie() {
        let target = login_target(Marketplace::Tes).expect("tes is a seller-device marketplace");
        assert!(target.is_logged_in(&jar(&["TESSession", "siteCountry"])));
        assert!(
            !target.is_logged_in(&jar(&["siteCountry"])),
            "the country cookie is set for anonymous visitors too"
        );
    }

    #[test]
    fn the_confirmed_login_urls_are_the_ones_the_probe_verified() {
        assert_eq!(
            login_target(Marketplace::Tpt).expect("tpt").login_url,
            "https://www.teacherspayteachers.com/Login"
        );
        assert_eq!(
            login_target(Marketplace::Tes).expect("tes").login_url,
            "https://www.tes.com/authn/sign-in?rtn=https%3A%2F%2Fwww.tes.com%2Fteaching-resources",
            "the /login page is a Drupal chooser with no password input; this is the sign-in form"
        );
    }
}
