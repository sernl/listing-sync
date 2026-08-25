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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
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
