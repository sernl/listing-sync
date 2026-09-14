//! One cycle of keeping this machine's library in step with the seller's
//! other machines: say what is held and where this endpoint is, then fetch
//! what the seller asked this machine to fetch.
//!
//! Called from the schedule beside the check-in. The advert is set before
//! the heartbeat so the server learns the holdings on the same beat, and the
//! wants are served after it, so a want the seller just placed is seen on
//! the next beat rather than the one after. Every step that fails is logged
//! and the cycle goes on: a peer that cannot be reached is a want that waits,
//! which is what the console says about it, and never a reason to stop
//! checking in.

use std::sync::Arc;

use tam_secrets::hex_encode;

use crate::control_plane::{HoldingLine, HttpControlPlane, LibraryAdvert};
use crate::device::DeviceId;
use crate::library::Library;
use crate::transfer::{PeerAddress, Transfer, TransferError};

/// What the schedule holds to run a cycle.
pub struct LibrarySync {
    pub device: DeviceId,
    pub plane: Arc<HttpControlPlane>,
    pub library: Arc<Library>,
    pub transfer: Arc<Transfer>,
}

impl LibrarySync {
    /// Sets the advert the next heartbeat carries.
    pub async fn advertise(&self) {
        let holdings = self
            .library
            .entries()
            .await
            .into_iter()
            .map(|entry| HoldingLine {
                hash: hex_encode(&entry.hash.0),
                byte_len: entry.byte_len,
            })
            .collect();
        self.plane
            .advertise(Some(LibraryAdvert {
                node_id: self.transfer.node_id(),
                direct_addrs: self.transfer.direct_addrs(),
                holdings,
            }))
            .await;
    }

    /// Fetches, directly, every file the seller asked this machine for that
    /// an online holder can hand over. Answers how many landed.
    pub async fn serve_wants(&self) -> usize {
        let wants = match self.plane.library_wants(&self.device).await {
            Ok(wants) => wants,
            Err(why) => {
                eprintln!("the files this machine was asked for could not be read: {why}");
                return 0;
            }
        };
        let mut landed = 0;
        for hash_hex in wants {
            let Some(hash) = crate::library::hash_from_hex(&hash_hex) else {
                continue;
            };
            if self.library.holds(hash).await {
                self.plane
                    .library_unwant(&self.device, &hash_hex)
                    .await
                    .ok();
                continue;
            }
            let peers = match self.plane.library_peers(&self.device, &hash_hex).await {
                Ok(peers) => peers,
                Err(why) => {
                    eprintln!("the holders of {hash_hex} could not be read: {why}");
                    continue;
                }
            };
            self.transfer.trust(&peers.trusted).await;
            let mut last: Option<TransferError> = None;
            for peer in peers.peers {
                let address = PeerAddress {
                    node_id: peer.node_id,
                    direct_addrs: peer.direct_addrs,
                };
                match self.transfer.fetch(&address, hash).await {
                    Ok((entry, bytes)) => match self.library.keep(entry, &bytes).await {
                        Ok(()) => {
                            landed += 1;
                            self.plane
                                .library_unwant(&self.device, &hash_hex)
                                .await
                                .ok();
                            last = None;
                            break;
                        }
                        Err(why) => {
                            eprintln!(
                                "{hash_hex} arrived from {} but was not kept: {why}",
                                peer.device
                            );
                        }
                    },
                    Err(why) => {
                        eprintln!(
                            "{hash_hex} could not be fetched from {} directly: {why}",
                            peer.device
                        );
                        last = Some(why);
                    }
                }
            }
            if let Some(why) = last {
                // Waiting, not failed: the want stands until a holder is
                // reachable, and the console says which machine it waits on.
                eprintln!("{hash_hex} waits for a machine that holds it to be reachable: {why}");
            }
        }
        landed
    }
}
