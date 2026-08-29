//! The broker's one narrow surface: line-delimited JSON over the unix socket.
//! Every request names its tenant explicitly, because the broker holds no
//! ambient tenant context — it is the privilege boundary, and a boundary that
//! inferred its scope would be no boundary.

use serde::{Deserialize, Serialize};
use tam_types::{ConnectionId, Marketplace, OrgId};

/// Which process a gateway belongs to. The broker holds one gateway per
/// (tenant, connection, purpose), so the item pump and the sync drain can
/// hold a session on the same connection at the same time without either
/// cancelling the other's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LeasePurpose {
    /// The item pump, which writes. The default because it is the caller
    /// that existed before there were two.
    #[default]
    Pump,
    /// The canonicalisation drain, which only reads.
    Drain,
}

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
    ///
    /// `purpose` is part of the gateway's identity because there are now two
    /// leasing processes. `connection_one_per_marketplace` means one Tes
    /// connection per organisation, so a key of (org, connection) always
    /// collides: the sync drain's lease would abort the item pump's gateway
    /// mid-write, failing an in-flight submit at the transport or, worse,
    /// landing between submit and verification and settling the create
    /// ambiguous. Two purposes, two gateways, no eviction.
    Lease {
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
        #[serde(default)]
        purpose: LeasePurpose,
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
