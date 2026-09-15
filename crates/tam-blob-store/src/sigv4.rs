//! AWS Signature Version 4 over the object requests [`crate::S3ObjectStore`]
//! makes. Hand-rolled against the published canonical-request rules rather
//! than taken from an SDK: what this crate signs is two verbs over one bucket,
//! and the SDK that would sign them arrives with a transport, a retry policy
//! and a credential-discovery chain this deployment does not use.
//!
//! The instant is an argument rather than a clock read here, which is what
//! lets the test below sign AWS's own published example and compare the
//! documented signature byte for byte.

use hmac::{Hmac, Mac};
use sha2::{Digest as _, Sha256};

use crate::Credentials;

/// The algorithm identifier that names both the signature and its scope.
const ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// Why a request could not be signed. Neither arm is reachable from a
/// well-formed call, and both are carried rather than swallowed because a
/// silently unsigned request is a request the endpoint refuses with no reason
/// anyone can read.
#[derive(Debug)]
pub(crate) enum SigningError {
    /// The instant given is outside the range a calendar date exists for.
    Clock,
    /// The HMAC construction refused the key it was handed.
    Key,
}

impl core::fmt::Display for SigningError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Clock => f.write_str("the instant is not a calendar date"),
            Self::Key => f.write_str("the signing key was refused"),
        }
    }
}

/// The two renderings of one instant a signature needs: the stamp the request
/// carries, and the day the signing key is derived for.
pub(crate) struct SigningTime {
    pub(crate) stamp: String,
    pub(crate) day: String,
}

impl SigningTime {
    pub(crate) fn from_unix_seconds(seconds: i64) -> Result<Self, SigningError> {
        let moment = chrono::DateTime::from_timestamp(seconds, 0).ok_or(SigningError::Clock)?;
        Ok(Self {
            stamp: moment.format("%Y%m%dT%H%M%SZ").to_string(),
            day: moment.format("%Y%m%d").to_string(),
        })
    }
}

/// One request as the signature reads it. Header names are lowercase, values
/// trimmed, and the slice sorted by name: the canonical form is defined in
/// that order, and sorting it here would hide a caller that signed a header it
/// then did not send.
pub(crate) struct CanonicalRequest<'a> {
    pub(crate) method: &'a str,
    pub(crate) path: &'a str,
    pub(crate) query: &'a str,
    pub(crate) headers: &'a [(&'a str, &'a str)],
    pub(crate) payload_sha256: &'a str,
}

impl CanonicalRequest<'_> {
    fn signed_headers(&self) -> String {
        self.headers
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(";")
    }

    fn render(&self) -> String {
        let mut lines = Vec::with_capacity(self.headers.len() + 5);
        lines.push(self.method.to_owned());
        lines.push(self.path.to_owned());
        lines.push(self.query.to_owned());
        for (name, value) in self.headers {
            lines.push(format!("{name}:{value}"));
        }
        // The blank line the canonical form puts between the headers and the
        // list of the ones signed.
        lines.push(String::new());
        lines.push(self.signed_headers());
        lines.push(self.payload_sha256.to_owned());
        lines.join("\n")
    }
}

/// The `Authorization` header value the request carries.
pub(crate) fn authorization(
    request: &CanonicalRequest<'_>,
    credentials: &Credentials,
    region: &str,
    service: &str,
    time: &SigningTime,
) -> Result<String, SigningError> {
    let scope = scope(region, service, time);
    let signature = signature(request, credentials, region, service, time)?;
    Ok(format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={}, Signature={signature}",
        credentials.access_key,
        request.signed_headers()
    ))
}

fn scope(region: &str, service: &str, time: &SigningTime) -> String {
    format!("{}/{region}/{service}/aws4_request", time.day)
}

fn signature(
    request: &CanonicalRequest<'_>,
    credentials: &Credentials,
    region: &str,
    service: &str,
    time: &SigningTime,
) -> Result<String, SigningError> {
    let to_sign = format!(
        "{ALGORITHM}\n{}\n{}\n{}",
        time.stamp,
        scope(region, service, time),
        hex(&Sha256::digest(request.render().as_bytes()))
    );
    let mut key = hmac(
        format!("AWS4{}", credentials.secret_key).as_bytes(),
        time.day.as_bytes(),
    )?;
    for step in [region.as_bytes(), service.as_bytes(), b"aws4_request"] {
        key = hmac(&key, step)?;
    }
    Ok(hex(&hmac(&key, to_sign.as_bytes())?))
}

fn hmac(key: &[u8], message: &[u8]) -> Result<[u8; 32], SigningError> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).map_err(|_never| SigningError::Key)?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().into())
}

/// The lowercase hex every field of a signature is rendered in.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(rendered, "{byte:02x}");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{authorization, CanonicalRequest, SigningTime};
    use crate::Credentials;

    /// AWS's own worked example — `get-vanilla` from the Signature Version 4
    /// test suite in the signing documentation — signed here and compared with
    /// the header that documentation prints. A signer that agrees with this
    /// agrees with every endpoint that implements the same specification;
    /// nothing short of a published vector proves that.
    #[test]
    fn the_published_aws_example_signs_to_its_documented_header() {
        let credentials = Credentials {
            access_key: "AKIDEXAMPLE".to_owned(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned(),
        };
        // 2015-08-30T12:36:00Z, the instant the example signs under.
        let time = SigningTime::from_unix_seconds(1_440_938_160).expect("a calendar date");
        assert_eq!(time.stamp, "20150830T123600Z", "the example's own stamp");
        let empty = super::hex(&<sha2::Sha256 as sha2::Digest>::digest(b""));
        let header = authorization(
            &CanonicalRequest {
                method: "GET",
                path: "/",
                query: "",
                headers: &[
                    ("host", "example.amazonaws.com"),
                    ("x-amz-date", "20150830T123600Z"),
                ],
                payload_sha256: &empty,
            },
            &credentials,
            "us-east-1",
            "service",
            &time,
        )
        .expect("the example signs");
        assert_eq!(
            header,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
            "the documented Authorization header for get-vanilla"
        );
    }
}
