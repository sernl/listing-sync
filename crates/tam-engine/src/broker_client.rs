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
    /// The analytics capture, which only reads. Its own purpose rather than a
    /// reuse of `Drain`: the broker's audit says which process opened a
    /// session, and a scheduled read of a seller's sales figures answering to
    /// the canonicalisation drain's name would make that record wrong.
    Analytics,
}

impl LeasePurpose {
    /// The token the broker's own `LeasePurpose` deserialises. The two enums
    /// are deliberately separate types across the privilege boundary, so these
    /// strings are the whole contract between them.
    #[must_use]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pump => "pump",
            Self::Drain => "drain",
            Self::Analytics => "analytics",
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BrokerAnswer {
    Leased {
        endpoint: String,
        token: String,
        expires_ms: i64,
    },
    Claimed,
    Error {
        detail: String,
        #[serde(default)]
        code: Option<BrokerErrorCode>,
    },
    #[serde(other)]
    Unexpected,
}

/// The machine-readable half of a broker error, for the faults a caller can
/// act on rather than merely report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    /// Another organisation already holds this marketplace account.
    PlatformAccountAlreadyLinked,
    #[serde(other)]
    Unrecognised,
}

/// A live gateway endpoint, the bearer token every request on it must carry,
/// and when it stops answering.
///
/// The token is not a convenience. The endpoint is a loopback listener that
/// every process on the host can reach, so without a per-lease credential,
/// holding a lease would be a matter of guessing a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayLease {
    pub endpoint: String,
    pub token: String,
    pub expires_ms: i64,
}

#[derive(Debug)]
pub struct BrokerClientError(pub String);

/// Why a claim did not complete.
///
/// A separate type from [`BrokerClientError`] because exactly one of its
/// outcomes is a decision the caller renders to a seller rather than a fault
/// it reports: the account is somebody else's, and no retry will change that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// Another organisation already holds this marketplace account. Names the
    /// marketplace and never the holder.
    AccountAlreadyLinked(Marketplace),
    /// Anything else, in the broker's own words.
    Refused(String),
}

impl core::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AccountAlreadyLinked(marketplace) => write!(
                f,
                "that {marketplace:?} account is already linked to another organisation"
            ),
            Self::Refused(detail) => write!(f, "broker: {detail}"),
        }
    }
}

impl core::error::Error for ClaimError {}

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
    let line = exchange(socket, &request).await?;
    match serde_json::from_str(&line) {
        Ok(BrokerAnswer::Leased {
            endpoint,
            token,
            expires_ms,
        }) => Ok(GatewayLease {
            endpoint,
            token,
            expires_ms,
        }),
        Ok(BrokerAnswer::Error { detail, .. }) => Err(BrokerClientError(detail)),
        Ok(BrokerAnswer::Claimed | BrokerAnswer::Unexpected) | Err(_) => {
            Err(BrokerClientError("unexpected broker answer".to_owned()))
        }
    }
}

#[derive(Serialize)]
struct ClaimRequest<'a> {
    op: &'static str,
    org: OrgId,
    connection: ConnectionId,
    marketplace: Marketplace,
    account_ref: &'a str,
}

/// Names the marketplace account a `linking` connection speaks for and
/// completes the link.
///
/// `account_ref` must be an identity the marketplace itself asserted, read
/// back through a lease on this very connection. A seller-typed value would be
/// a denial-of-service primitive: anyone could claim a storefront they do not
/// own and lock its real owner out of ever linking.
pub async fn claim_account(
    socket: &std::path::Path,
    org: OrgId,
    connection: ConnectionId,
    marketplace: Marketplace,
    account_ref: &str,
) -> Result<(), ClaimError> {
    let request = serde_json::to_string(&ClaimRequest {
        op: "claim",
        org,
        connection,
        marketplace,
        account_ref,
    })
    .map_err(|error| ClaimError::Refused(error.to_string()))?;
    let line = exchange(socket, &request)
        .await
        .map_err(|error| ClaimError::Refused(error.0))?;
    match serde_json::from_str(&line) {
        Ok(BrokerAnswer::Claimed) => Ok(()),
        Ok(BrokerAnswer::Error { detail, code }) => Err(
            if code == Some(BrokerErrorCode::PlatformAccountAlreadyLinked) {
                ClaimError::AccountAlreadyLinked(marketplace)
            } else {
                ClaimError::Refused(detail)
            },
        ),
        Ok(BrokerAnswer::Leased { .. } | BrokerAnswer::Unexpected) | Err(_) => {
            Err(ClaimError::Refused("unexpected broker answer".to_owned()))
        }
    }
}

/// One request, one answering line, over the broker's unix socket.
async fn exchange(socket: &std::path::Path, request: &str) -> Result<String, BrokerClientError> {
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
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::LeasePurpose;

    /// The engine's `LeasePurpose` and the broker's are separate types on
    /// either side of the privilege boundary, joined only by these tokens, so
    /// a typo in one of them is a lease the broker refuses at runtime with
    /// nothing failing at compile time. Pinned literally rather than derived,
    /// because a derivation would move with the mistake.
    #[test]
    fn every_purpose_spells_the_token_the_broker_deserialises() {
        assert_eq!(
            [
                LeasePurpose::Pump.as_str(),
                LeasePurpose::Drain.as_str(),
                LeasePurpose::Analytics.as_str(),
            ],
            ["pump", "drain", "analytics"],
            "these are the snake_case names the broker's protocol enum declares"
        );
    }
}
