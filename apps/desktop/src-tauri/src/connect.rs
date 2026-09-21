//! Where a marketplace login happens, and how the device knows it worked.
//!
//! The login runs in a webview this application owns, on the seller's machine,
//! because you can no longer attach to the seller's existing browser: Chrome
//! 136 stopped honouring `--remote-debugging-port` against the default profile,
//! and reading cookies out of the profile is the infostealer pattern that
//! App-Bound Encryption exists to block. Section 7 of
//! `docs/notes/design/client-side-architecture.md` records both.
//!
//! Cookie names identify login candidates, not authenticated principals. TES
//! also issues `TESSession` to anonymous visitors. The command confirms its
//! principal through the device-side transport before filing that candidate.

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

/// One marketplace's login page and the cookies needed to attempt authentication.
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
    /// Whether the jar contains the cookies the adapter needs.
    ///
    /// Necessary, not sufficient: the command must also apply the marketplace's
    /// confirmation gate before reporting a successful capture.
    #[must_use]
    pub fn has_login_cookies(&self, jar: &CookieJar) -> bool {
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

/// How a login ended, in the one word that survives a navigation.
///
/// A closed set rather than a string because on a phone it is the whole
/// channel between the capture and the sentence the seller reads: the page
/// that asked for the login is unloaded by the navigation to the marketplace,
/// so the promise it was holding is gone before there is anything to resolve
/// it with, and the answer has to come back in the address bar or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectVerdict {
    /// A session is filed on this device.
    Captured,
    /// The login deadline passed with no session in the jar.
    Deadline,
    /// The seller left the marketplace's pages before signing in, which on a
    /// phone is the back gesture and has no equivalent on a computer, where
    /// leaving means closing a window. The gesture walks the webview's
    /// history, so a seller who moved through several of the marketplace's own
    /// pages presses back once per page and this is reached when they arrive
    /// back at ours (`MainActivity.kt`, `handleBackNavigation`).
    Abandoned,
    /// The sign-in never appeared: a navigation the platform dropped, or a page
    /// that could not begin to load. The seller has something to do about it,
    /// which is to press Connect again.
    Refused,
    /// The sign-in was not saved on this device. The cause worth naming is a
    /// device signed out or revoked from the console, because the check-in that
    /// follows a capture wipes the store and the seller's own remedy is to sign
    /// in to Teachouse again here; a jar that could not be read lands here too,
    /// having saved nothing either.
    ///
    /// Split from [`Self::Refused`] because the two were one code and the
    /// console could only word one of them: a seller whose device had been
    /// signed out was told the sign-in could not be opened, which is the
    /// opposite of what happened and names nothing they can act on.
    NotKept,
    /// This machine had been signed out from the console, so no login was
    /// opened — or the one in flight was not kept, because the check-in that
    /// follows a capture wiped the store under it.
    ///
    /// Split from [`Self::NotKept`] because the remedy is a different one and
    /// the seller can only act on the remedy: `NotKept` says the sign-in was
    /// not saved and leaves them to press Connect again, which on a signed-out
    /// machine fails again every time. This says the machine itself is signed
    /// out, which is undone once, on the Machines page, and then the sign-in
    /// holds.
    SignedOut,
    /// The organisation has not agreed to the seller-device notice for this
    /// marketplace, so no login was opened. The remedy is on the Account
    /// page's permissions panel, on any machine, and then the sign-in holds.
    ConsentRequired,
    /// The shop this sign-in speaks for is already connected to another
    /// Teachouse account, so the server refused the claim and the capture was
    /// not kept.
    ///
    /// Its own verdict because it is the one refusal no action on this device
    /// clears: pressing Connect again signs in to the same shop and is
    /// refused again, and signing this machine back in does nothing for it.
    /// The remedy is a person.
    BoundElsewhere,
}

impl ConnectVerdict {
    /// Every verdict, beside the variants rather than in a test, so a new one
    /// is listed where it is declared. `Marketplace::ALL` is the same shape and
    /// is the reason this is a constant and not an iterator.
    pub const ALL: [Self; 8] = [
        Self::Captured,
        Self::Deadline,
        Self::Abandoned,
        Self::Refused,
        Self::NotKept,
        Self::SignedOut,
        Self::ConsentRequired,
        Self::BoundElsewhere,
    ];

    /// The word this verdict travels as.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Captured => "captured",
            Self::Deadline => "deadline",
            Self::Abandoned => "abandoned",
            Self::Refused => "refused",
            Self::NotKept => "notkept",
            Self::SignedOut => "signed_out",
            Self::ConsentRequired => "consent",
            Self::BoundElsewhere => "bound_elsewhere",
        }
    }
}

/// The query parameter carrying [`ConnectVerdict::code`].
pub const RETURN_PARAM: &str = "connect";

/// The query parameter naming which marketplace the verdict is about, so the
/// sentence can name it rather than saying "the marketplace".
pub const RETURN_MARKETPLACE_PARAM: &str = "marketplace";

/// How a marketplace is spelled in the address the console is resumed at.
///
/// An exhaustive match rather than `Debug`, because the console reads this
/// string as its own `Marketplace` type and `Debug` is not a wire contract;
/// `the_marketplace_spelling_is_the_one_the_console_deserialises` holds the two
/// together.
const fn marketplace_code(marketplace: Marketplace) -> &'static str {
    match marketplace {
        Marketplace::Tes => "Tes",
        Marketplace::Tpt => "Tpt",
        Marketplace::Etsy => "Etsy",
    }
}

/// Where the console is resumed after a login that replaced it, and what it is
/// told when it gets there.
///
/// Pure, so the whole return leg is decided and tested on the host with no
/// webview. A query parameter rather than a command or an event because it
/// needs no capability and no grant, and because it can only choose a
/// sentence: whether a marketplace is connected is read from the server's own
/// connection list, so a hand-typed parameter changes copy and never state.
#[must_use]
pub fn return_url(base: &str, marketplace: Marketplace, verdict: ConnectVerdict) -> String {
    format!(
        "{}/marketplaces?{RETURN_PARAM}={}&{RETURN_MARKETPLACE_PARAM}={}",
        base.trim_end_matches('/'),
        verdict.code(),
        marketplace_code(marketplace),
    )
}

#[cfg(test)]
mod tests {
    use super::{login_target, return_url, ConnectVerdict, NotSellerDevice};
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
        assert!(target.has_login_cookies(&jar(&["csrfToken", "sessionKey"])));
        assert!(
            target.has_login_cookies(&jar(&["csrfToken", "TPT", "_ga"])),
            "either session cookie spelling counts, and unrelated cookies do not interfere"
        );
        assert!(
            !target.has_login_cookies(&jar(&["sessionKey"])),
            "without csrfToken the adapter cannot authorise one request, so this is not a \
             usable session however logged in the page looks"
        );
        assert!(
            !target.has_login_cookies(&jar(&["csrfToken"])),
            "a CSRF cookie is handed out before login too, so on its own it proves nothing"
        );
        assert!(!target.has_login_cookies(&jar(&[])));
    }

    #[test]
    fn tes_needs_its_session_cookie() {
        let target = login_target(Marketplace::Tes).expect("tes is a seller-device marketplace");
        assert!(target.has_login_cookies(&jar(&["TESSession", "siteCountry"])));
        assert!(
            !target.has_login_cookies(&jar(&["siteCountry"])),
            "the country cookie is set for anonymous visitors too"
        );
    }

    /// Every verdict reaches the console as its own address.
    ///
    /// Total over [`ConnectVerdict::ALL`] rather than a sample, because this is
    /// the only channel a phone's outcome travels down: a verdict with no
    /// address, or two verdicts sharing one, is a seller returned to the
    /// console with the wrong sentence or with none.
    #[test]
    fn every_verdict_returns_to_the_marketplaces_page_naming_itself() {
        let mut seen: Vec<String> = Vec::new();
        for verdict in ConnectVerdict::ALL {
            let url = return_url("https://teachouse.io", Marketplace::Tpt, verdict);
            assert!(
                url.starts_with("https://teachouse.io/marketplaces?"),
                "the return leg lands on the page the seller pressed Connect on, or they come \
                 back somewhere they did not leave. Got: {url}"
            );
            assert!(
                url.contains(&format!("connect={}", verdict.code())),
                "the verdict is what the address carries; without it the page has nothing to \
                 say. Got: {url}"
            );
            assert!(
                url.contains("marketplace=Tpt"),
                "and which marketplace it is about, or the sentence cannot name it. Got: {url}"
            );
            assert!(
                !seen.contains(&url),
                "two verdicts sharing one address would make one of them unreadable. Got: {url}"
            );
            seen.push(url);
        }
    }

    /// A base with a trailing slash produces the same address as one without.
    ///
    /// `base_url()` takes `TAM_CONTROL_PLANE` verbatim from the environment,
    /// so a developer's `https://host/` is a value this actually receives, and
    /// `//marketplaces` is a path the console does not serve.
    #[test]
    fn a_trailing_slash_on_the_base_does_not_become_a_second_one() {
        assert_eq!(
            return_url(
                "http://localhost:5173/",
                Marketplace::Tes,
                ConnectVerdict::Deadline
            ),
            return_url(
                "http://localhost:5173",
                Marketplace::Tes,
                ConnectVerdict::Deadline
            ),
        );
        assert!(!return_url(
            "http://localhost:5173/",
            Marketplace::Tes,
            ConnectVerdict::Deadline
        )
        .contains("//marketplaces"));
    }

    /// The marketplace in the address is spelled the way the console's own
    /// `Marketplace` type is.
    ///
    /// `marketplace_code` is a hand-written match and the console deserialises
    /// the parameter into a generated union, so nothing but this holds the two
    /// spellings together: a rename on either side would leave the return leg
    /// naming a marketplace the page cannot look up, and the sentence would go
    /// missing on exactly the outcome it exists for.
    #[test]
    fn the_marketplace_spelling_is_the_one_the_console_deserialises() {
        for marketplace in Marketplace::ALL {
            let serialised = serde_json::to_string(&marketplace).expect("a marketplace serialises");
            let quoted = serialised.trim_matches('"');
            assert!(
                return_url("https://x", marketplace, ConnectVerdict::Captured)
                    .ends_with(&format!("marketplace={quoted}")),
                "{marketplace:?} travels as its own serialised name"
            );
        }
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
