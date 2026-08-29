//! The engine's client to the credential broker: a lease request over the
//! unix socket, answered with a gateway endpoint and never any secret. The
//! wire shape mirrors the broker's private protocol module deliberately —
//! the privilege boundary keeps its types to itself.

use serde::{Deserialize, Serialize};
use tam_types::{ConnectionId, Marketplace, OrgId};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Serialize)]
struct LeaseRequest {
    op: &'static str,
    org: OrgId,
    connection: ConnectionId,
    marketplace: Marketplace,
    purpose: &'static str,
}

/// Which process is asking. The broker keys a gateway on it, so the item pump
/// and the canonicalisation drain hold separate sessions on the one Tes
/// connection a tenant has rather than cancelling each other's mid-write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeasePurpose {
    Pump,
    Drain,
}

impl LeasePurpose {
    #[must_use]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pump => "pump",
            Self::Drain => "drain",
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BrokerAnswer {
    Leased {
        endpoint: String,
        expires_ms: i64,
    },
    Error {
        detail: String,
    },
    #[serde(other)]
    Unexpected,
}

/// A live gateway endpoint and when it stops answering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayLease {
    pub endpoint: String,
    pub expires_ms: i64,
}

#[derive(Debug)]
pub struct BrokerClientError(pub String);

impl core::fmt::Display for BrokerClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "broker: {}", self.0)
    }
}

impl core::error::Error for BrokerClientError {}

pub async fn request_lease(
    socket: &std::path::Path,
    org: OrgId,
    connection: ConnectionId,
    marketplace: Marketplace,
    purpose: LeasePurpose,
) -> Result<GatewayLease, BrokerClientError> {
    let request = serde_json::to_string(&LeaseRequest {
        op: "lease",
        org,
        connection,
        marketplace,
        purpose: purpose.as_str(),
    })
    .map_err(|error| BrokerClientError(error.to_string()))?;
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|error| BrokerClientError(format!("connect: {error}")))?;
    let (read_half, mut write_half) = stream.into_split();
    write_half
        .write_all(format!("{request}\n").as_bytes())
        .await
        .map_err(|error| BrokerClientError(format!("write: {error}")))?;
    let mut line = String::new();
    BufReader::new(read_half)
        .read_line(&mut line)
        .await
        .map_err(|error| BrokerClientError(format!("read: {error}")))?;
    match serde_json::from_str(&line) {
        Ok(BrokerAnswer::Leased {
            endpoint,
            expires_ms,
        }) => Ok(GatewayLease {
            endpoint,
            expires_ms,
        }),
        Ok(BrokerAnswer::Error { detail }) => Err(BrokerClientError(detail)),
        Ok(BrokerAnswer::Unexpected) | Err(_) => {
            Err(BrokerClientError("unexpected broker answer".to_owned()))
        }
    }
}
