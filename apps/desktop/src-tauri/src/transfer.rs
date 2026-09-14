//! Moving one of the seller's files from one of their machines to another,
//! directly.
//!
//! Each machine runs a QUIC endpoint dialled by public key, with relays
//! disabled at the builder: two of the seller's machines either reach each
//! other over the addresses they reported to the server, or the transfer
//! waits. Nothing of ours ever sits between them, and the server, which
//! carries node addresses and digests only, is not a party to the bytes.
//!
//! The protocol is the smallest one that works. The asking side opens one
//! bidirectional stream and writes the thirty-two-byte digest it wants; the
//! holding side answers with the entry's metadata as a length-prefixed JSON
//! line and the plaintext as a length-prefixed body, both read out of its
//! sealed library on demand, and closes the stream. The bytes are verified
//! against the digest on arrival, so a holder cannot substitute a file, and
//! a connection from a node the server has not listed for this organisation
//! is closed before it can ask anything.
//!
//! Not `iroh-blobs`, deliberately. Its store model wants the plaintext on
//! disk in its own layout, and this crate's library keeps every byte sealed
//! at rest; a request-and-answer over the same QUIC connection keeps that
//! property and needs nothing the endpoint does not already give.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey};
use tam_types::ContentHash;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::Mutex;

use crate::library::{Library, LibraryEntry};

/// The application-layer protocol this endpoint speaks, and the only one.
pub const ALPN: &[u8] = b"teachouse/library/1";

/// Relays are off. A constant rather than a literal at the one call site,
/// so the test that holds this property reads the same value the endpoint
/// is built with.
pub const RELAY: RelayMode = RelayMode::Disabled;

/// The largest file one answer may carry. The same ceiling the library's
/// imports are bounded by on the way in.
pub const BYTES_MAX: u64 = 2 * 1024 * 1024 * 1024;

/// The QUIC close code for a peer the server never listed.
const NOT_TRUSTED: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    /// No address the peer reported could be reached directly.
    Unreachable(String),
    /// The peer answered, but not with the file: it no longer holds it, or
    /// the answer was malformed.
    Refused(String),
    /// The bytes that arrived do not hash to the digest asked for.
    DigestMismatch(ContentHash),
    /// The endpoint could not be brought up or bound.
    Endpoint(String),
}

impl core::fmt::Display for TransferError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreachable(why) => write!(f, "the other machine could not be reached: {why}"),
            Self::Refused(why) => write!(f, "the other machine did not hand the file over: {why}"),
            Self::DigestMismatch(_) => {
                f.write_str("the bytes that arrived are not the file that was asked for")
            }
            Self::Endpoint(why) => write!(f, "the transfer endpoint failed: {why}"),
        }
    }
}

impl core::error::Error for TransferError {}

/// Where one holder can be reached, as the server relayed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAddress {
    pub node_id: String,
    pub direct_addrs: Vec<String>,
}

/// The endpoint and the protocol behind it, for the life of the process.
pub struct Transfer {
    endpoint: Endpoint,
    router: Router,
    trusted: Arc<Mutex<HashSet<EndpointId>>>,
}

impl core::fmt::Debug for Transfer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Transfer")
            .field("node_id", &self.endpoint.id())
            .finish_non_exhaustive()
    }
}

/// The endpoint builder every transfer endpoint comes from: relays disabled,
/// one protocol, the node key handed in.
fn endpoint_builder(secret: [u8; 32]) -> iroh::endpoint::Builder {
    // The empty preset and an explicit provider, rather than the crate's own
    // `Minimal`: this binary already installs ring as the process provider
    // for its HTTP client, and one named provider is one fewer thing two
    // stacks can disagree about.
    Endpoint::builder(iroh::endpoint::presets::Empty)
        .crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .secret_key(SecretKey::from_bytes(&secret))
        .relay_mode(RELAY)
        .alpns(vec![ALPN.to_vec()])
}

impl Transfer {
    /// Binds the endpoint and starts serving this machine's library to the
    /// peers it is later told to trust. Nothing is trusted at start.
    pub async fn start(library: Arc<Library>, secret: [u8; 32]) -> Result<Self, TransferError> {
        let endpoint = endpoint_builder(secret)
            .bind()
            .await
            .map_err(|why| TransferError::Endpoint(why.to_string()))?;
        let trusted = Arc::new(Mutex::new(HashSet::new()));
        let router = Router::builder(endpoint.clone())
            .accept(
                ALPN,
                Serve {
                    library,
                    trusted: Arc::clone(&trusted),
                },
            )
            .spawn();
        Ok(Self {
            endpoint,
            router,
            trusted,
        })
    }

    /// This machine's node id, as the server records it.
    #[must_use]
    pub fn node_id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// The socket addresses this endpoint is reachable at right now. Empty
    /// until the endpoint has discovered its own addresses, which a caller
    /// treats as "not yet" rather than "none".
    pub fn direct_addrs(&self) -> Vec<String> {
        self.endpoint
            .addr()
            .ip_addrs()
            .map(ToString::to_string)
            .collect()
    }

    /// Replaces the set of node ids a connection is accepted from: the
    /// organisation's own live devices, as the server listed them.
    pub async fn trust(&self, node_ids: &[String]) {
        let parsed: HashSet<EndpointId> =
            node_ids.iter().filter_map(|raw| raw.parse().ok()).collect();
        *self.trusted.lock().await = parsed;
    }

    /// Fetches one file directly from a peer. The bytes are verified against
    /// the digest before they are handed back.
    pub async fn fetch(
        &self,
        peer: &PeerAddress,
        hash: ContentHash,
    ) -> Result<(LibraryEntry, Vec<u8>), TransferError> {
        let node_id: EndpointId = peer
            .node_id
            .parse()
            .map_err(|_| TransferError::Unreachable("the peer's node id is not one".to_owned()))?;
        let addrs: Vec<SocketAddr> = peer
            .direct_addrs
            .iter()
            .filter_map(|raw| raw.parse().ok())
            .collect();
        if addrs.is_empty() {
            return Err(TransferError::Unreachable(
                "the peer reported no direct address".to_owned(),
            ));
        }
        let address =
            EndpointAddr::new(node_id).with_addrs(addrs.into_iter().map(iroh::TransportAddr::Ip));
        let connection = self
            .endpoint
            .connect(address, ALPN)
            .await
            .map_err(|why| TransferError::Unreachable(why.to_string()))?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|why| TransferError::Unreachable(why.to_string()))?;
        send.write_all(&hash.0)
            .await
            .map_err(|why| TransferError::Unreachable(why.to_string()))?;
        send.finish()
            .map_err(|why| TransferError::Unreachable(why.to_string()))?;

        let meta_len = recv
            .read_u32()
            .await
            .map_err(|why| TransferError::Refused(why.to_string()))?;
        if meta_len == 0 {
            return Err(TransferError::Refused(
                "the other machine no longer holds that file".to_owned(),
            ));
        }
        if u64::from(meta_len) > 64 * 1024 {
            return Err(TransferError::Refused(
                "the entry description is too long".to_owned(),
            ));
        }
        let mut meta = vec![0u8; meta_len as usize];
        recv.read_exact(&mut meta)
            .await
            .map_err(|why| TransferError::Refused(why.to_string()))?;
        let entry: LibraryEntry = serde_json::from_slice(&meta)
            .map_err(|why| TransferError::Refused(format!("unreadable entry: {why}")))?;
        let len = recv
            .read_u64()
            .await
            .map_err(|why| TransferError::Refused(why.to_string()))?;
        if len > BYTES_MAX {
            return Err(TransferError::Refused(
                "the file is over the transfer ceiling".to_owned(),
            ));
        }
        let mut bytes = vec![0u8; usize::try_from(len).unwrap_or(usize::MAX)];
        recv.read_exact(&mut bytes)
            .await
            .map_err(|why| TransferError::Refused(why.to_string()))?;
        connection.close(0u32.into(), b"done");
        if ContentHash(*blake3::hash(&bytes).as_bytes()) != hash || entry.hash != hash {
            return Err(TransferError::DigestMismatch(hash));
        }
        Ok((entry, bytes))
    }

    /// Stops serving and closes the endpoint.
    pub async fn shutdown(&self) {
        self.router.shutdown().await.ok();
    }
}

/// The serving half: answers one digest per stream, to trusted peers only.
#[derive(Debug)]
struct Serve {
    library: Arc<Library>,
    trusted: Arc<Mutex<HashSet<EndpointId>>>,
}

impl Serve {
    async fn answer(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = connection.remote_id();
        if !self.trusted.lock().await.contains(&remote) {
            connection.close(NOT_TRUSTED.into(), b"not a machine of this account");
            return Ok(());
        }
        let (mut send, mut recv) = connection.accept_bi().await?;
        let mut wanted = [0u8; 32];
        recv.read_exact(&mut wanted)
            .await
            .map_err(AcceptError::from_err)?;
        let hash = ContentHash(wanted);
        let entry = self
            .library
            .entries()
            .await
            .into_iter()
            .find(|entry| entry.hash == hash);
        let bytes = match &entry {
            Some(_) => self.library.read(hash).await.ok().flatten(),
            None => None,
        };
        match (entry, bytes) {
            (Some(entry), Some(bytes)) => {
                let meta = serde_json::to_vec(&entry).map_err(AcceptError::from_err)?;
                send.write_u32(u32::try_from(meta.len()).map_err(AcceptError::from_err)?)
                    .await?;
                send.write_all(&meta).await.map_err(AcceptError::from_err)?;
                send.write_u64(bytes.len() as u64).await?;
                send.write_all(&bytes)
                    .await
                    .map_err(AcceptError::from_err)?;
            }
            _ => {
                send.write_u32(0).await?;
            }
        }
        send.finish()?;
        // Wait for the peer to read everything before the connection goes.
        connection.closed().await;
        Ok(())
    }
}

impl ProtocolHandler for Serve {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        self.answer(connection).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;
    use tam_secrets::Kek;
    use tam_types::{Marketplace, Timestamp};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "teachouse-transfer-{name}-{}-{}",
            std::process::id(),
            tam_secrets::random_token()
        ));
        std::fs::create_dir_all(&dir).expect("the scratch directory is creatable");
        dir
    }

    fn library(name: &str) -> Arc<Library> {
        let key = Kek::from_bytes(&[0x11; 32]).expect("32 bytes is a key");
        Arc::new(Library::open(&scratch(name), key).expect("opens"))
    }

    fn entry(bytes: &[u8]) -> LibraryEntry {
        LibraryEntry {
            hash: ContentHash(*blake3::hash(bytes).as_bytes()),
            file_name: "worksheet.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            byte_len: bytes.len() as u64,
            marketplace: Marketplace::Tpt,
            resource: "101".to_owned(),
            kept_at: Timestamp(1_000),
            pinned: false,
        }
    }

    #[test]
    fn relays_are_disabled_at_the_builder() {
        assert!(matches!(RELAY, RelayMode::Disabled));
    }

    #[tokio::test]
    async fn a_trusted_peer_receives_the_file_and_an_untrusted_one_is_closed() {
        let holder_library = library("holder");
        let bytes = b"%PDF-1.4 the seller's own file";
        holder_library
            .keep(entry(bytes), bytes)
            .await
            .expect("keeps");
        let holder = Transfer::start(holder_library, [0x21; 32])
            .await
            .expect("the holder binds");
        let asker = Transfer::start(library("asker"), [0x22; 32])
            .await
            .expect("the asker binds");
        // Wait for the holder to know its own addresses.
        let mut addrs = holder.direct_addrs();
        for _ in 0..50 {
            if !addrs.is_empty() {
                break;
            }
            tokio::time::sleep(core::time::Duration::from_millis(100)).await;
            addrs = holder.direct_addrs();
        }
        assert!(!addrs.is_empty(), "the holder discovers a direct address");
        let peer = PeerAddress {
            node_id: holder.node_id(),
            direct_addrs: addrs,
        };
        let hash = entry(bytes).hash;

        // Not trusted yet: the holder closes the connection before answering.
        let refused = asker.fetch(&peer, hash).await;
        assert!(
            refused.is_err(),
            "an unlisted node gets nothing: {refused:?}"
        );

        holder.trust(&[asker.node_id()]).await;
        let (got_entry, got) = asker
            .fetch(&peer, hash)
            .await
            .expect("the trusted peer is served");
        assert_eq!(got, bytes.to_vec());
        assert_eq!(got_entry.file_name, "worksheet.pdf");

        let missing = asker
            .fetch(&peer, ContentHash([0xEE; 32]))
            .await
            .expect_err("a file the holder does not keep is refused");
        assert!(matches!(missing, TransferError::Refused(_)));
        holder.shutdown().await;
        asker.shutdown().await;
    }
}
