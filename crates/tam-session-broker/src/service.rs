//! The request handler: one connection, one request, one response, holding
//! the vault, the leased gateways and the upstream base. Errors become an
//! Error response rather than a panic, because the broker must stay up.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::gateway::{self, Gateway};
use crate::protocol::{Request, Response};
use crate::vault::Vault;
use tam_secrets::Secret;
use tam_types::{ConnectionId, OrgId};

/// How long a lease's gateway lives before it is aborted. A judgement: long
/// enough for one item's flow, short enough that an abandoned lease frees its
/// listener; ordinary const beside its caller.
const LEASE_TTL_MS: i64 = 10 * 60 * 1000;

pub(crate) struct Broker {
    vault: Vault,
    upstream_base: String,
    gateways: Mutex<HashMap<(OrgId, ConnectionId), Gateway>>,
    root: CancellationToken,
}

impl Broker {
    #[must_use]
    pub(crate) fn new(vault: Vault, upstream_base: String, root: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            vault,
            upstream_base,
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
            } => match self
                .vault
                .link(org, connection, marketplace, Secret::new(cookie_header))
                .await
            {
                Ok(()) => Response::Linked,
                Err(error) => Response::Error {
                    detail: error.to_string(),
                },
            },
            Request::Lease {
                org,
                connection,
                marketplace,
            } => self.lease(org, connection, marketplace, now_ms).await,
            Request::Revoke { org, connection } => {
                self.abort_gateway(org, connection).await;
                match self.vault.revoke(org, connection).await {
                    Ok(count) => Response::Revoked {
                        connections: count,
                        elapsed_ms: 0,
                    },
                    Err(error) => Response::Error {
                        detail: error.to_string(),
                    },
                }
            }
            Request::RevokeAll => self.revoke_all().await,
            Request::Health => Response::Healthy,
        }
    }

    async fn lease(
        &self,
        org: OrgId,
        connection: ConnectionId,
        marketplace: tam_types::Marketplace,
        now_ms: i64,
    ) -> Response {
        let secret = match self.vault.open_secret(org, connection, marketplace).await {
            Ok(secret) => secret,
            Err(error) => {
                return Response::Error {
                    detail: error.to_string(),
                }
            }
        };
        match gateway::spawn(self.upstream_base.clone(), secret, &self.root).await {
            Ok(gateway) => {
                let endpoint = gateway.endpoint.clone();
                self.abort_gateway(org, connection).await;
                self.gateways
                    .lock()
                    .await
                    .insert((org, connection), gateway);
                Response::Leased {
                    endpoint,
                    expires_ms: now_ms + LEASE_TTL_MS,
                }
            }
            Err(error) => Response::Error {
                detail: error.to_string(),
            },
        }
    }

    async fn abort_gateway(&self, org: OrgId, connection: ConnectionId) {
        if let Some(gateway) = self.gateways.lock().await.remove(&(org, connection)) {
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
            Err(error) => Response::Error {
                detail: error.to_string(),
            },
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
