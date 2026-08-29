//! The request handler: one connection, one request, one response, holding
//! the vault, the leased gateways and the upstream bases. Errors become an
//! Error response rather than a panic, because the broker must stay up.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::gateway::{self, Gateway, LeaseGateway, SessionSink};
use crate::protocol::{Authorship as WireAuthorship, ErrorCode, LeasePurpose, Request, Response};
use crate::vault::{Authorship, LinkRequest, Vault, VaultError};
use tam_secrets::Secret;
use tam_types::{ConnectionId, Marketplace, OrgId};

/// How long a lease's gateway lives before it is aborted. A judgement: long
/// enough for one item's flow, short enough that an abandoned lease frees its
/// listener; ordinary const beside its caller.
const LEASE_TTL_MS: i64 = 10 * 60 * 1000;

/// Where each marketplace's session hops go.
///
/// One base per marketplace rather than one per process: a Tpt lease proxying
/// to the Tes origin would send the seller's Tpt cookie to a host that has no
/// business receiving it, and a single base made that a configuration mistake
/// rather than an impossible one.
pub(crate) struct Upstreams {
    pub(crate) tes: String,
    pub(crate) tpt: String,
}

impl Upstreams {
    /// `None` for a marketplace with no connector, which is a lease that
    /// cannot be opened rather than one pointed at a default.
    fn base(&self, marketplace: Marketplace) -> Option<&str> {
        match marketplace {
            Marketplace::Tes => Some(&self.tes),
            Marketplace::Tpt => Some(&self.tpt),
            Marketplace::Etsy => None,
        }
    }
}

/// One lease's whole identity: which tenant's connection, on which
/// marketplace, for which process. The four travel together because the
/// gateway map is keyed on three of them and the fourth opens the secret.
struct LeaseFor {
    org: OrgId,
    connection: ConnectionId,
    marketplace: tam_types::Marketplace,
    purpose: LeasePurpose,
}

/// The vault, seen by a gateway as somewhere to put a renewal.
///
/// The gateway holds this rather than the vault itself so it can be driven by
/// a recording double in its own tests; here the renewal is sealed under the
/// same tenant context the credential was sealed in.
struct VaultSink {
    vault: Arc<Vault>,
    org: OrgId,
    connection: ConnectionId,
    marketplace: Marketplace,
}

impl SessionSink for VaultSink {
    fn reseal(
        &self,
        cookie_header: String,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // A failed reseal loses the renewal and nothing else: the lease
            // keeps working on the jar it holds, and the stored copy stays at
            // its previous value. Reported rather than propagated, because
            // failing the seller's request over a custody bookkeeping write
            // would trade a working hop for a lost one.
            if let Err(error) = self
                .vault
                .reseal(
                    self.org,
                    self.connection,
                    self.marketplace,
                    Secret::new(cookie_header),
                )
                .await
            {
                eprintln!("tam-session-broker: a session renewal could not be resealed: {error}");
            }
        })
    }

    fn record_verified(
        &self,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Err(error) = self.vault.record_verified(self.org, self.connection).await {
                eprintln!(
                    "tam-session-broker: a proven-live session could not be recorded: {error}"
                );
            }
        })
    }

    fn record_failure(
        &self,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Err(error) = self
                .vault
                .record_refresh_failure(self.org, self.connection)
                .await
            {
                eprintln!("tam-session-broker: a session failure could not be recorded: {error}");
            }
        })
    }
}

pub(crate) struct Broker {
    vault: Arc<Vault>,
    upstreams: Upstreams,
    gateways: Mutex<HashMap<(OrgId, ConnectionId, LeasePurpose), Gateway>>,
    root: CancellationToken,
}

/// An error response carrying the machine-readable code where the fault is one
/// a caller can act on.
fn failed(error: &VaultError) -> Response {
    let code = match error {
        VaultError::AccountAlreadyLinked(_) => Some(ErrorCode::PlatformAccountAlreadyLinked),
        VaultError::Db(_)
        | VaultError::Seal
        | VaultError::Open(_)
        | VaultError::NoSecret
        | VaultError::NotClaimable => None,
    };
    Response::Error {
        detail: error.to_string(),
        code,
    }
}

fn refused(detail: String) -> Response {
    Response::Error { detail, code: None }
}

impl Broker {
    #[must_use]
    pub(crate) fn new(vault: Vault, upstreams: Upstreams, root: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            vault: Arc::new(vault),
            upstreams,
            gateways: Mutex::new(HashMap::new()),
            root,
        })
    }

    pub(crate) async fn handle(&self, request: Request, now_ms: i64) -> Response {
        match request {
            Request::Link {
                org,
                connection,
                marketplace,
                cookie_header,
                authorship,
            } => {
                let declared = authorship.map(Authorship::from);
                match self
                    .vault
                    .link(LinkRequest {
                        org,
                        connection,
                        marketplace,
                        secret: Secret::new(cookie_header),
                        authorship: declared.as_ref(),
                    })
                    .await
                {
                    Ok(()) => Response::Linked,
                    Err(error) => failed(&error),
                }
            }
            Request::Claim {
                org,
                connection,
                marketplace,
                account_ref,
            } => match self
                .vault
                .claim(org, connection, marketplace, &account_ref)
                .await
            {
                Ok(()) => Response::Claimed,
                Err(error) => failed(&error),
            },
            Request::Refresh {
                org,
                connection,
                marketplace,
            } => self.refresh(org, connection, marketplace).await,
            Request::Lease {
                org,
                connection,
                marketplace,
                purpose,
            } => {
                self.lease(
                    &LeaseFor {
                        org,
                        connection,
                        marketplace,
                        purpose,
                    },
                    now_ms,
                )
                .await
            }
            Request::Revoke { org, connection } => {
                self.abort_gateway(org, connection).await;
                match self.vault.revoke(org, connection).await {
                    Ok(count) => Response::Revoked {
                        connections: count,
                        elapsed_ms: 0,
                    },
                    Err(error) => failed(&error),
                }
            }
            Request::RevokeAll => self.revoke_all().await,
            Request::Health => Response::Healthy,
        }
    }

    /// Drives the marketplace's renewal route with the sealed jar. The lease
    /// gateway absorbs renewals that arrive alongside item work; this is the
    /// same absorption on a connection nobody is currently driving, which is
    /// where a session that is used rarely would otherwise expire.
    async fn refresh(
        &self,
        org: OrgId,
        connection: ConnectionId,
        marketplace: Marketplace,
    ) -> Response {
        let Some(base) = self.upstreams.base(marketplace) else {
            return refused(format!(
                "{marketplace:?} has no upstream to refresh against"
            ));
        };
        let secret = match self.vault.open_secret(org, connection, marketplace).await {
            Ok(secret) => secret,
            Err(error) => return failed(&error),
        };
        let sink = VaultSink {
            vault: Arc::clone(&self.vault),
            org,
            connection,
            marketplace,
        };
        // `refresh_session` records the answer's own verdict — a non-2xx is
        // evidence about the session and advances the counter there. Only a
        // refresh that never reached the marketplace is recorded here, and it
        // is deliberately the same counter: a refresh loop that could not run
        // is a connection nobody has proved live, however innocent the cause.
        match gateway::refresh_session(base, marketplace, &secret, &sink).await {
            Ok(upstream_status) => Response::Refreshed { upstream_status },
            Err(error) => {
                if let Err(recorded) = self.vault.record_refresh_failure(org, connection).await {
                    eprintln!(
                        "tam-session-broker: the refresh failure could not be recorded: {recorded}"
                    );
                }
                refused(error.to_string())
            }
        }
    }

    async fn lease(&self, request: &LeaseFor, now_ms: i64) -> Response {
        let LeaseFor {
            org,
            connection,
            marketplace,
            purpose,
        } = *request;
        let Some(base) = self.upstreams.base(marketplace) else {
            return refused(format!("{marketplace:?} has no connector to lease"));
        };
        let base = base.to_owned();
        let secret = match self.vault.open_secret(org, connection, marketplace).await {
            Ok(secret) => secret,
            Err(error) => return failed(&error),
        };
        let sink: Arc<dyn SessionSink> = Arc::new(VaultSink {
            vault: Arc::clone(&self.vault),
            org,
            connection,
            marketplace,
        });
        match gateway::spawn(
            LeaseGateway {
                upstream_base: base,
                marketplace,
                cookie: secret,
                sink,
            },
            &self.root,
        )
        .await
        {
            Ok(gateway) => {
                let endpoint = gateway.endpoint.clone();
                let token = gateway.token.clone();
                self.abort_one(org, connection, purpose).await;
                self.gateways
                    .lock()
                    .await
                    .insert((org, connection, purpose), gateway);
                Response::Leased {
                    endpoint,
                    token,
                    expires_ms: now_ms + LEASE_TTL_MS,
                }
            }
            Err(error) => refused(error.to_string()),
        }
    }

    /// Every purpose for one connection, which is what a revoke means: the
    /// credential is gone, so no process may keep a session opened with it.
    async fn abort_gateway(&self, org: OrgId, connection: ConnectionId) {
        let mut gateways = self.gateways.lock().await;
        let keys: Vec<_> = gateways
            .keys()
            .filter(|(key_org, key_connection, _)| *key_org == org && *key_connection == connection)
            .copied()
            .collect();
        for key in keys {
            if let Some(gateway) = gateways.remove(&key) {
                gateway.cancel.cancel();
            }
        }
    }

    /// One purpose's gateway, replaced by the lease that supersedes it. The
    /// other purpose's stays up, which is the whole point of the key.
    async fn abort_one(&self, org: OrgId, connection: ConnectionId, purpose: LeasePurpose) {
        if let Some(gateway) = self
            .gateways
            .lock()
            .await
            .remove(&(org, connection, purpose))
        {
            gateway.cancel.cancel();
        }
    }

    /// The drill: every gateway aborted, every credential cleared, every
    /// connection revoked, and the wall-clock reported.
    pub(crate) async fn revoke_all(&self) -> Response {
        let start = now_ms_from(&self.root);
        let mut gateways = self.gateways.lock().await;
        for (_key, gateway) in gateways.drain() {
            gateway.cancel.cancel();
        }
        drop(gateways);
        match self.vault.revoke_all().await {
            Ok(count) => Response::Revoked {
                connections: count,
                elapsed_ms: now_ms_from(&self.root).saturating_sub(start),
            },
            Err(error) => failed(&error),
        }
    }
}

/// Bridges the wire shape to the vault's, so the protocol module stays the
/// only place the wire names live.
impl From<WireAuthorship> for Authorship {
    fn from(wire: WireAuthorship) -> Self {
        Self {
            name: wire.name,
            attested_at_ms: wire.attested_at_ms,
        }
    }
}

/// The drill measures its own wall-clock, which is a genuine clock read at the
/// process boundary; the elapsed figure is the whole point of the drill.
#[expect(
    clippy::disallowed_methods,
    reason = "the revocation drill reports its own wall-clock; that elapsed figure is the containment window the design publishes"
)]
fn now_ms_from(_root: &CancellationToken) -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
    )
    .unwrap_or(i64::MAX)
}
