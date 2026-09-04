//! The sans-io HTTP seam adapters call through, so the cassette harness can
//! stand exactly where the network stands while this crate takes no runtime
//! or client dependency.
//!
//! `HttpRequest` deliberately has no headers field: authentication is
//! constructor state on the live transport, and the only auth a request value
//! may carry is an ephemeral per-request S3 signature, never a session secret,
//! so a recorded cassette cannot contain one by construction. `HttpResponse`
//! mirrors the rule from the other side with a closed header allow-list, which
//! leaves `Set-Cookie` no variant to be recorded in.

use serde::{Deserialize, Serialize};

use crate::ConnectFailure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// One part of a multipart form carrying file content. The S3 upload names
/// the part, its file name and its content type; everything else the policy
/// requires travels as ordinary form fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePart {
    pub part_name: String,
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RequestBody {
    Empty,
    Json(serde_json::Value),
    Multipart {
        fields: Vec<(String, String)>,
        file: Option<FilePart>,
    },
    /// A raw payload, for the S3 object PUT whose signature covers the bytes
    /// themselves. Recorded through the same helper a response body uses, so
    /// a text payload stays legible in a fixture and a binary one survives it.
    Bytes(#[serde(with = "body_bytes")] Vec<u8>),
}

/// The only authentication a request value may carry. Closed, and with no
/// general header field beside it, so a session secret is unrepresentable
/// here rather than merely discouraged: [`RequestAuth::Session`] names the
/// credential the live transport holds at construction and carries none of
/// it, and the one variant that does carry material carries an ephemeral
/// per-request signature over one object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestAuth {
    #[default]
    Session,
    /// A request whose authorisation travels in its own body — the S3
    /// POST-policy upload, where the policy and its signature are form
    /// fields. It must reach the network without our session attached.
    Anonymous,
    /// A hop the marketplace named rather than the caller: the destination of
    /// a redirect, re-issued deliberately.
    ///
    /// What separates it from [`Self::Anonymous`] is not the credential —
    /// neither carries one — but who chose the destination. An anonymous
    /// request goes where this crate decided to send it, so its host can be
    /// asserted against a constant. This one goes where a marketplace's
    /// `Location` pointed, which is a signed url on a content network whose
    /// host that marketplace may change without telling anybody, so no
    /// constant can be maintained for it and the rule has to be about the
    /// request's shape instead: https, one hop, nothing of ours attached, and
    /// a bound on what may come back.
    ///
    /// It exists because a session client must not follow a cross-host
    /// redirect — every default header it carries would follow with it — while
    /// the bytes behind that redirect are still the seller's own and still
    /// wanted. Re-issuing the hop here is what keeps both true.
    Redirected,
    S3SigV2 {
        access_key_id: String,
        signature: String,
        amz_date: String,
        content_md5: Option<String>,
        content_type: String,
    },
}

impl RequestAuth {
    /// The serde skip predicate that keeps every session-authenticated
    /// request byte-identical to the fixtures written before this field
    /// existed.
    #[must_use]
    pub const fn is_session(&self) -> bool {
        matches!(*self, Self::Session)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub body: RequestBody,
    #[serde(default, skip_serializing_if = "RequestAuth::is_session")]
    pub auth: RequestAuth,
}

impl HttpRequest {
    /// A session-authenticated read.
    #[must_use]
    pub const fn get(url: String) -> Self {
        Self {
            method: Method::Get,
            url,
            body: RequestBody::Empty,
            auth: RequestAuth::Session,
        }
    }

    /// A session-authenticated delete. The seam carries no body on one,
    /// which is what every measured delete route accepts.
    #[must_use]
    pub const fn delete(url: String) -> Self {
        Self {
            method: Method::Delete,
            url,
            body: RequestBody::Empty,
            auth: RequestAuth::Session,
        }
    }

    #[must_use]
    pub const fn post_json(url: String, value: serde_json::Value) -> Self {
        Self {
            method: Method::Post,
            url,
            body: RequestBody::Json(value),
            auth: RequestAuth::Session,
        }
    }

    /// A multipart form POST. `auth` is the caller's, because the two forms
    /// this seam sends differ on exactly that point: a marketplace form
    /// rides the session, and an S3 POST-policy upload must not.
    #[must_use]
    pub const fn post_multipart(
        url: String,
        fields: Vec<(String, String)>,
        file: Option<FilePart>,
        auth: RequestAuth,
    ) -> Self {
        Self {
            method: Method::Post,
            url,
            body: RequestBody::Multipart { fields, file },
            auth,
        }
    }

    /// A raw-body PUT under a per-request signature: the S3 object write.
    #[must_use]
    pub const fn put_signed(url: String, bytes: Vec<u8>, auth: RequestAuth) -> Self {
        Self {
            method: Method::Put,
            url,
            body: RequestBody::Bytes(bytes),
            auth,
        }
    }
}

/// A response body is bytes, never a string. `reqwest::Response::text()`
/// decodes lossily rather than failing, so carrying a body as `String` turns
/// every non-UTF-8 byte into U+FFFD in silence — measured on a real download
/// bundle, a 4364-byte ZIP came back 7810 bytes and unreadable. Text is a
/// view onto the bytes, taken by [`HttpResponse::text`] where a classifier
/// wants one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(with = "body_bytes")]
    pub body: Vec<u8>,
    /// The allow-listed headers a flow reads. Empty on every response whose
    /// flow reads none, which is what keeps the recorded shape identical to
    /// the fixtures written before the field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(ResponseHeader, String)>,
}

/// The response headers a flow may see. An allow-list rather than a map:
/// a live transport projects the real header map through these variants at
/// the boundary, so `Set-Cookie` has nowhere to land and a recording cannot
/// carry one however the marketplace answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseHeader {
    Location,
    ETag,
    QueueTrackingId,
}

impl HttpResponse {
    /// A response carrying no allow-listed header, which is every response
    /// whose flow reads only status and body.
    #[must_use]
    pub const fn plain(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }

    /// The body decoded as UTF-8 for classification and diagnostics, lossily
    /// and deliberately: a body that does not decode is binary, and binary is
    /// read through `body` itself.
    #[must_use]
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    /// The first value recorded for one allow-listed header.
    #[must_use]
    pub fn header(&self, which: ResponseHeader) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| *name == which)
            .map(|(_, value)| value.as_str())
    }
}

/// Keeps a recorded body legible: a UTF-8 body serialises as a JSON string,
/// exactly as it did when bodies were `String`, so every committed cassette
/// fixture stays byte-identical. Anything else falls back to an array of
/// bytes, which is faithful where a string would not be.
mod body_bytes {
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        match core::str::from_utf8(bytes) {
            Ok(text) => serializer.serialize_str(text),
            Err(_) => serializer.collect_seq(bytes),
        }
    }

    struct BodyVisitor;

    impl<'de> Visitor<'de> for BodyVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter.write_str("a response body as a UTF-8 string or an array of bytes")
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
            Ok(value.as_bytes().to_vec())
        }

        fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
            Ok(value.to_vec())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut bytes = Vec::new();
            while let Some(byte) = seq.next_element::<u8>()? {
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_any(BodyVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The request provably never left. The only class that is safe to retry.
    NotSent(ConnectFailure),
    /// The request was (or may have been) sent and no usable response came
    /// back; the write state is unknown and the caller classifies it as
    /// ambiguous, never as failed.
    AfterSend { detail: String },
    /// The response arrived whole and this transport refused it.
    ///
    /// Distinct from [`Self::AfterSend`], and the distinction is what a caller
    /// does next. `AfterSend` means the outcome is unknown, so a write is
    /// classified ambiguous and an operator is asked. This means the outcome
    /// is known and we declined it — a body past the bound we read it under,
    /// say — so there is nothing uncertain to escalate and the sentence
    /// explaining it must survive to the caller rather than being flattened
    /// into a lost response.
    Refused { detail: String },
    /// Produced only by the cassette transport when a flow diverges from the
    /// recording; a live transport never returns it.
    Harness { detail: String },
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotSent(cause) => write!(f, "request never left: {cause:?}"),
            Self::AfterSend { detail } => {
                write!(f, "response lost after send, state unknown: {detail}")
            }
            Self::Refused { detail } => write!(f, "the transport refused the response: {detail}"),
            Self::Harness { detail } => write!(f, "cassette divergence: {detail}"),
        }
    }
}

impl core::error::Error for TransportError {}

/// The one door to the network. Written as `fn -> impl Future` rather than
/// `async fn`, because `async_fn_in_trait` is a hard error under a
/// deny-warnings build and the desugaring is what a multi-threaded runtime
/// requires anyway.
pub trait Transport: Send + Sync {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl core::future::Future<Output = Result<HttpResponse, TransportError>> + Send;
}
