//! The sans-io HTTP seam adapters call through, so the cassette harness can
//! stand exactly where the network stands while this crate takes no runtime
//! or client dependency.
//!
//! `HttpRequest` deliberately has no headers field: authentication is
//! constructor state on the live transport, never request data, so a recorded
//! cassette cannot contain a session secret by construction.

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub body: RequestBody,
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
}

impl HttpResponse {
    /// The body decoded as UTF-8 for classification and diagnostics, lossily
    /// and deliberately: a body that does not decode is binary, and binary is
    /// read through `body` itself.
    #[must_use]
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
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
