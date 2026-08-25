//! The broker's one narrow surface: line-delimited JSON over the unix socket.
//! Every request names its tenant explicitly, because the broker holds no
//! ambient tenant context — it is the privilege boundary, and a boundary that
//! inferred its scope would be no boundary.

use serde::{Deserialize, Serialize};
use tam_types::{ConnectionId, Marketplace, OrgId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Request {
    /// Seal a supplied credential and mark the connection linked. The cookie
    /// header is the secret; it is consumed here and never returned.
    Link {
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
        cookie_header: String,
    },
    /// Open a lease: an authenticating gateway endpoint the worker drives,
    /// with the cookie injected server-side so the worker never sees it.
    Lease {
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
    },
    /// Revoke one connection: tombstone its ciphertext and revoke it.
    Revoke {
        org: OrgId,
        connection: ConnectionId,
    },
    /// The global kill switch: every connection, every tenant.
    RevokeAll,
    Health,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Response {
    Linked,
    /// A loopback gateway endpoint and its expiry. Never any secret material.
    Leased {
        endpoint: String,
        expires_ms: i64,
    },
    Revoked {
        connections: u32,
        elapsed_ms: i64,
    },
    Healthy,
    Error {
        detail: String,
    },
}
