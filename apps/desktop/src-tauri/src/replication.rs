//! Teachouse's copy of the files this device imported.
//!
//! An import leaves each original on the device that read it. So the seller
//! can view, cut a preview from and download a file wherever they sign in,
//! this device copies the originals it holds to Teachouse: it asks which
//! imported files the server has no copy of, plans which of those it holds
//! and has room to send, and sends them one at a time.
//!
//! Run on start, on each check-in, and whenever the library has changed —
//! which is what an import that kept a file does — and never two at once.
//! Every failure is logged and left for the next run: a copy that did not
//! land is a file that stays on the device a little longer, and the console
//! says so.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tam_secrets::hex_encode;
use tam_types::ContentHash;

use crate::control_plane::{CopyOutcome, HttpControlPlane, MissingCopies};
use crate::library::{hash_from_hex, Library, LibraryEntry};

/// Which files to send, and which cannot go and why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopyPlan {
    /// In the order to send them: the server's, smallest first, so a plan
    /// short of room lands as many files as it can.
    pub send: Vec<ContentHash>,
    /// Would take the plan past its storage.
    pub full: Vec<ContentHash>,
    /// Larger than the server takes in one copy.
    pub too_large: Vec<ContentHash>,
}

/// Plans one run: every file the server is missing that this library holds,
/// in the server's order, charged against the room the plan has left.
///
/// The library's own length is what is charged, because it is what will be
/// sent; the server's figure is the device's report of the same bytes and
/// agrees with it whenever the digest does. A file that does not fit is
/// skipped rather than ending the plan, so a smaller one after it still goes.
#[must_use]
pub fn plan(held: &[LibraryEntry], missing: &MissingCopies) -> CopyPlan {
    let lengths: HashMap<ContentHash, u64> = held
        .iter()
        .map(|entry| (entry.hash, entry.byte_len))
        .collect();
    let mut used = missing.stored_bytes;
    let mut planned = CopyPlan::default();
    for file in &missing.files {
        let Some(hash) = hash_from_hex(&file.hash) else {
            continue;
        };
        let Some(&len) = lengths.get(&hash) else {
            continue;
        };
        if planned.send.contains(&hash) {
            continue;
        }
        if len > missing.copy_bytes_max {
            planned.too_large.push(hash);
        } else if used.saturating_add(len) > missing.storage_bytes_max {
            planned.full.push(hash);
        } else {
            used = used.saturating_add(len);
            planned.send.push(hash);
        }
    }
    planned
}

/// What one run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub copied: usize,
    pub full: usize,
    pub too_large: usize,
    pub failed: usize,
}

/// This device's side of the copy: the server, the library, and the one
/// flag that keeps two runs apart.
#[derive(Clone)]
pub struct Replicator {
    pub plane: Arc<HttpControlPlane>,
    pub library: Arc<Library>,
    running: Arc<AtomicBool>,
}

/// Lowers the running flag however the run ends.
struct Running(Arc<AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Replicator {
    #[must_use]
    pub fn new(plane: Arc<HttpControlPlane>, library: Arc<Library>) -> Self {
        Self {
            plane,
            library,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts a run in the background unless one is already going. The
    /// schedule's loop is never held behind a copy: a 60 MB file on a home
    /// connection takes minutes, and the check-in must not wait for it.
    pub fn start(&self) {
        if self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let _running = Running(Arc::clone(&this.running));
            let report = this.run().await;
            if report != CopyReport::default() {
                eprintln!(
                    "copies to Teachouse: {} sent, {} waiting for storage, {} too large, {} failed",
                    report.copied, report.full, report.too_large, report.failed
                );
            }
        });
    }

    /// One run: read what is missing, plan, send.
    pub async fn run(&self) -> CopyReport {
        let missing = match self.plane.library_missing().await {
            Ok(missing) => missing,
            Err(why) => {
                eprintln!("could not ask which files Teachouse is missing: {why}");
                return CopyReport::default();
            }
        };
        let planned = plan(&self.library.entries().await, &missing);
        let mut report = CopyReport {
            full: planned.full.len(),
            too_large: planned.too_large.len(),
            ..CopyReport::default()
        };
        for hash in planned.send {
            let hex = hex_encode(&hash.0);
            let bytes = match self.library.read(hash).await {
                Ok(Some(bytes)) => bytes,
                // Removed since the plan was made: nothing to send.
                Ok(None) => continue,
                Err(why) => {
                    eprintln!("the kept file {hex} could not be read to copy: {why}");
                    report.failed += 1;
                    continue;
                }
            };
            match self.plane.library_copy(&hex, bytes).await {
                Ok(CopyOutcome::Stored) => report.copied += 1,
                Ok(CopyOutcome::StorageFull) => {
                    // The server's total moved under the plan: nothing after
                    // this fits either.
                    report.full += 1;
                    break;
                }
                Ok(CopyOutcome::Refused(why)) => {
                    eprintln!("Teachouse refused the copy of {hex}: {why}");
                    report.failed += 1;
                }
                Err(why) => {
                    eprintln!("the copy of {hex} did not reach Teachouse: {why}");
                    report.failed += 1;
                    break;
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::{plan, CopyPlan};
    use crate::control_plane::{MissingCopies, MissingCopy};
    use crate::library::LibraryEntry;
    use tam_secrets::hex_encode;
    use tam_types::{ContentHash, Marketplace, Timestamp};

    const MIB: u64 = 1024 * 1024;

    fn hash(byte: u8) -> ContentHash {
        ContentHash([byte; 32])
    }

    fn kept(byte: u8, byte_len: u64) -> LibraryEntry {
        LibraryEntry {
            hash: hash(byte),
            file_name: format!("{byte}.pdf"),
            content_type: "application/pdf".to_owned(),
            byte_len,
            marketplace: Marketplace::Tpt,
            resource: byte.to_string(),
            kept_at: Timestamp(0),
            pinned: false,
        }
    }

    fn missing(files: &[(u8, u64)], stored: u64, cap: u64) -> MissingCopies {
        MissingCopies {
            files: files
                .iter()
                .map(|&(byte, byte_len)| MissingCopy {
                    hash: hex_encode(&hash(byte).0),
                    byte_len,
                })
                .collect(),
            stored_bytes: stored,
            storage_bytes_max: cap,
            copy_bytes_max: 96 * MIB,
        }
    }

    #[test]
    fn sends_only_what_the_server_lacks_and_this_device_holds() {
        let held = [kept(1, MIB), kept(2, MIB), kept(3, MIB)];
        // 2 is already on the server (not listed); 4 is missing but held
        // by another device.
        let answer = missing(&[(1, MIB), (3, MIB), (4, MIB)], 0, 1024 * MIB);
        assert_eq!(
            plan(&held, &answer),
            CopyPlan {
                send: vec![hash(1), hash(3)],
                ..CopyPlan::default()
            }
        );
    }

    #[test]
    fn stops_charging_at_the_plans_storage_and_still_sends_a_smaller_file() {
        let held = [kept(1, 40 * MIB), kept(2, 30 * MIB), kept(3, 5 * MIB)];
        // 60 of 100 MiB used: 40 fits exactly, 30 does not, 5 would not
        // either once 40 is charged.
        let answer = missing(
            &[(1, 40 * MIB), (2, 30 * MIB), (3, 5 * MIB)],
            60 * MIB,
            100 * MIB,
        );
        assert_eq!(
            plan(&held, &answer),
            CopyPlan {
                send: vec![hash(1)],
                full: vec![hash(2), hash(3)],
                too_large: Vec::new(),
            }
        );
        // With 40 skipped for room, the smaller one after it still goes.
        let answer = missing(&[(1, 40 * MIB), (3, 5 * MIB)], 70 * MIB, 100 * MIB);
        assert_eq!(
            plan(&held, &answer),
            CopyPlan {
                send: vec![hash(3)],
                full: vec![hash(1)],
                too_large: Vec::new(),
            }
        );
    }

    #[test]
    fn a_file_above_the_copy_ceiling_is_named_rather_than_sent() {
        let held = [kept(1, 97 * MIB), kept(2, 96 * MIB)];
        let answer = missing(&[(2, 96 * MIB), (1, 97 * MIB)], 0, 1024 * MIB);
        assert_eq!(
            plan(&held, &answer),
            CopyPlan {
                send: vec![hash(2)],
                full: Vec::new(),
                too_large: vec![hash(1)],
            }
        );
    }

    #[test]
    fn charges_the_library_length_not_the_reported_one() {
        // The server's figure is a report; the library's is what is sent.
        let held = [kept(1, 60 * MIB)];
        let answer = missing(&[(1, MIB)], 50 * MIB, 100 * MIB);
        assert_eq!(plan(&held, &answer).full, vec![hash(1)]);
    }

    #[test]
    fn a_malformed_digest_is_passed_over() {
        let held = [kept(1, MIB)];
        let mut answer = missing(&[(1, MIB)], 0, 1024 * MIB);
        answer.files.insert(
            0,
            MissingCopy {
                hash: "not-a-digest".to_owned(),
                byte_len: 1,
            },
        );
        assert_eq!(plan(&held, &answer).send, vec![hash(1)]);
    }
}
