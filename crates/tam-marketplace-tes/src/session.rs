//! The Tes session: a cookie header held as constructor state on the live
//! transport, so it never appears in a request value and therefore never in
//! a cassette.

/// The session cookies as one header value. `Debug` redacts, because a
/// session header in a log line is a credential leak.
pub struct TesSession {
    cookie_header: String,
}

impl core::fmt::Debug for TesSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TesSession(redacted)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    NoCookies,
}

impl core::fmt::Display for SessionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoCookies => f.write_str("no cookies parsed from the session source"),
        }
    }
}

impl core::error::Error for SessionError {}

impl TesSession {
    pub fn from_cookie_header(header: String) -> Result<Self, SessionError> {
        if header.trim().is_empty() {
            return Err(SessionError::NoCookies);
        }
        Ok(Self {
            cookie_header: header,
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
        Ok(Self {
            cookie_header: pairs.join("; "),
        })
    }

    #[must_use]
    pub fn header_value(&self) -> &str {
        &self.cookie_header
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionError, TesSession};

    #[test]
    fn a_netscape_jar_parses_including_httponly_lines() {
        let jar = "# Netscape HTTP Cookie File\n\
                   .tes.com\tTRUE\t/\tTRUE\t0\tsiteCountry\tGB\n\
                   #HttpOnly_.tes.com\tTRUE\t/\tTRUE\t0\tTESSession\tabc123\n";
        let session = TesSession::from_netscape_jar(jar).expect("the jar parses");
        assert_eq!(
            session.header_value(),
            "siteCountry=GB; TESSession=abc123",
            "both plain and HttpOnly cookies join the header in file order"
        );
    }

    #[test]
    fn an_empty_jar_is_refused() {
        assert_eq!(
            TesSession::from_netscape_jar("# only comments\n").err(),
            Some(SessionError::NoCookies),
            "a session with no cookies must be refused, not sent"
        );
    }

    #[test]
    fn debug_redacts_the_header() {
        let session = TesSession::from_cookie_header("TESSession=secret".to_owned())
            .expect("a non-empty header is a session");
        assert_eq!(
            format!("{session:?}"),
            "TesSession(redacted)",
            "a session must never print its cookies"
        );
    }
}
