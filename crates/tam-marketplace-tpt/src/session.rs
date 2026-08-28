//! The TPT session: the cookie jar and the CSRF token mirrored out of it,
//! held as constructor state on the live transport so neither appears in a
//! request value and therefore neither can reach a cassette.
//!
//! TPT's CSRF is a classic double submit: the `csrfToken` cookie value is
//! copied verbatim into an `x-csrf-token` header. The token appears nowhere
//! in the page markup, so a connector obtains it only by reading its own jar,
//! which is why this type derives it rather than accepting it separately.

/// The cookie name whose value is mirrored into `x-csrf-token`.
pub const CSRF_COOKIE: &str = "csrfToken";

/// `Debug` redacts, because a session header in a log line is a credential leak.
pub struct TptSession {
    cookie_header: String,
    csrf_token: String,
}

impl core::fmt::Debug for TptSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TptSession(redacted)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    NoCookies,
    NoCsrfToken,
}

impl core::fmt::Display for SessionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoCookies => f.write_str("no cookies parsed from the session source"),
            Self::NoCsrfToken => write!(
                f,
                "the jar carries no {CSRF_COOKIE} cookie, so no request can be authorised"
            ),
        }
    }
}

impl core::error::Error for SessionError {}

fn csrf_from_pairs<'a>(pairs: impl Iterator<Item = &'a str>) -> Option<String> {
    for pair in pairs {
        if let Some((name, value)) = pair.trim().split_once('=') {
            if name == CSRF_COOKIE {
                return Some(value.to_owned());
            }
        }
    }
    None
}

impl TptSession {
    pub fn from_cookie_header(header: String) -> Result<Self, SessionError> {
        if header.trim().is_empty() {
            return Err(SessionError::NoCookies);
        }
        let csrf_token = csrf_from_pairs(header.split(';')).ok_or(SessionError::NoCsrfToken)?;
        Ok(Self {
            cookie_header: header,
            csrf_token,
        })
    }

    /// Parses a Netscape cookie jar (fields: domain, flag, path, secure,
    /// expiry, name, value), tolerating curl's `#HttpOnly_` prefix.
    pub fn from_netscape_jar(text: &str) -> Result<Self, SessionError> {
        let mut pairs = Vec::new();
        for line in text.lines() {
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            if let (Some(name), Some(value)) = (fields.get(5), fields.get(6)) {
                pairs.push(format!("{name}={value}"));
            }
        }
        if pairs.is_empty() {
            return Err(SessionError::NoCookies);
        }
        Self::from_cookie_header(pairs.join("; "))
    }

    #[must_use]
    pub fn header_value(&self) -> &str {
        &self.cookie_header
    }

    /// The value the live transport mirrors into `x-csrf-token`.
    #[must_use]
    pub fn csrf_token(&self) -> &str {
        &self.csrf_token
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionError, TptSession};

    #[test]
    fn a_netscape_jar_parses_and_mirrors_the_csrf_cookie() {
        let jar = "# Netscape HTTP Cookie File\n\
                   .teacherspayteachers.com\tTRUE\t/\tTRUE\t0\tsessionKey\tabc123\n\
                   #HttpOnly_.teacherspayteachers.com\tTRUE\t/\tTRUE\t0\tcsrfToken\tdeadbeef\n";
        let session = TptSession::from_netscape_jar(jar).expect("the jar parses");
        assert_eq!(
            session.header_value(),
            "sessionKey=abc123; csrfToken=deadbeef",
            "both plain and HttpOnly cookies join the header in file order"
        );
        assert_eq!(
            session.csrf_token(),
            "deadbeef",
            "the header value is the cookie value verbatim, which is the double submit"
        );
    }

    #[test]
    fn a_jar_without_the_csrf_cookie_is_refused() {
        let jar = ".teacherspayteachers.com\tTRUE\t/\tTRUE\t0\tsessionKey\tabc123\n";
        assert_eq!(
            TptSession::from_netscape_jar(jar).err(),
            Some(SessionError::NoCsrfToken),
            "a session that cannot mirror a token must be refused, not sent"
        );
    }

    #[test]
    fn an_empty_jar_is_refused() {
        assert_eq!(
            TptSession::from_netscape_jar("# only comments\n").err(),
            Some(SessionError::NoCookies),
            "a session with no cookies must be refused, not sent"
        );
    }

    #[test]
    fn debug_redacts_the_header() {
        let session = TptSession::from_cookie_header("csrfToken=secret".to_owned())
            .expect("a header carrying the token is a session");
        assert_eq!(
            format!("{session:?}"),
            "TptSession(redacted)",
            "a session must never print its cookies"
        );
    }
}
