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
    /// The scheduled analytics capture, which only reads. A third purpose
    /// rather than a reuse of `Drain`, so this audit keeps saying which
    /// process opened a session on a seller's account.
    Analytics,
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
        /// The seller's authorship attestation, where the marketplace needs
        /// one. Supplied with the link because the instant is the seller's
        /// own; a broker that minted it would be attesting on their behalf.
        #[serde(default)]
        authorship: Option<Authorship>,
    },
    /// Name the marketplace account a `linking` connection speaks for and
    /// complete the link.
    ///
    /// The caller runs the identity read through its own lease and passes what
    /// the marketplace asserted, so the broker never parses a marketplace
    /// response: response parsing in the one process holding the
    /// key-encryption key is exactly the blast radius the boundary exists to
    /// keep small. Only a server-asserted identity is admissible — a
    /// seller-typed one would let anybody lock out a real storefront's owner.
    Claim {
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
        account_ref: String,
    },
    /// Drive the marketplace's own session-renewal route and reseal whatever
    /// it hands back, without any item work riding along.
    Refresh {
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
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

/// The seller's declaration of who authored what a connection publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Authorship {
    pub(crate) name: String,
    pub(crate) attested_at_ms: i64,
}

/// The machine-readable half of an error response, for the faults a caller can
/// act on rather than merely report. Absent where the detail string is all
/// there is to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorCode {
    /// Another organisation already holds this marketplace account. Carries no
    /// hint of which one: the constraint must not become a directory of every
    /// seller on the platform.
    PlatformAccountAlreadyLinked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Response {
    Linked,
    /// The account is now named and the connection is fully linked.
    Claimed,
    /// The renewal route answered, and any renewal it carried is resealed.
    /// Named `upstream_status` because `status` is the envelope's own tag.
    Refreshed {
        upstream_status: u16,
    },
    /// A loopback gateway endpoint, the bearer token a caller must present on
    /// it, and its expiry. Never any secret material.
    ///
    /// The token exists because the endpoint is a loopback listener and every
    /// process on the host can reach it; without one, holding a lease is a
    /// matter of guessing a port rather than of being the worker the broker
    /// answered.
    Leased {
        endpoint: String,
        token: String,
        expires_ms: i64,
    },
    Revoked {
        connections: u32,
        elapsed_ms: i64,
    },
    Healthy,
    Error {
        detail: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<ErrorCode>,
    },
}

#[cfg(test)]
mod tests {
    use super::{LeasePurpose, Request};
    use tam_types::{ConnectionId, Marketplace, OrgId, Uuid};

    fn lease(purpose: LeasePurpose) -> Request {
        Request::Lease {
            org: OrgId(Uuid([0x11; 16])),
            connection: ConnectionId(Uuid([0x22; 16])),
            marketplace: Marketplace::Tpt,
            purpose,
        }
    }

    /// The other half of the contract `tam-engine`'s `LeasePurpose::as_str`
    /// pins: the caller writes these tokens by hand, so this side must accept
    /// exactly them and no others.
    #[test]
    fn each_purpose_deserialises_from_the_token_the_caller_writes() {
        for (token, expected) in [
            ("pump", LeasePurpose::Pump),
            ("drain", LeasePurpose::Drain),
            ("analytics", LeasePurpose::Analytics),
        ] {
            let raw = format!(
                r#"{{"op":"lease","org":"11111111-1111-1111-1111-111111111111",
                     "connection":"22222222-2222-2222-2222-222222222222",
                     "marketplace":"Tpt","purpose":"{token}"}}"#
            );
            let parsed: Request =
                serde_json::from_str(&raw).expect("the lease request parses at all");
            assert_eq!(parsed, lease(expected), "{token} names {expected:?}");
        }
    }

    /// An unpinned purpose is the pump, which is what keeps a caller predating
    /// the field working rather than failing closed on a field it never sent.
    #[test]
    fn a_lease_naming_no_purpose_is_the_pump() {
        let raw = r#"{"op":"lease","org":"11111111-1111-1111-1111-111111111111",
                      "connection":"22222222-2222-2222-2222-222222222222",
                      "marketplace":"Tpt"}"#;
        let parsed: Request = serde_json::from_str(raw).expect("the lease request parses");
        assert_eq!(parsed, lease(LeasePurpose::Pump), "the default is the pump");
    }
}
