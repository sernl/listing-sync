//! The malware-scan seam. `ScanOutcome::Infected` is already a first-class
//! file state, so a positive scan blocks the publish through the same path a
//! missing cover does. The trait lets a live ClamAV socket stand where the
//! EICAR test scanner stands, so the infected path is reachable in tests
//! without a scanning daemon.

use tam_types::{ScanOutcome, Timestamp};

pub trait Scanner: Send + Sync {
    fn scan(
        &self,
        bytes: &[u8],
        now: Timestamp,
    ) -> impl core::future::Future<Output = ScanOutcome> + Send;
}

/// Passes everything. For tests whose subject is not the scanner.
pub struct AllowAllScanner;

impl Scanner for AllowAllScanner {
    fn scan(
        &self,
        _bytes: &[u8],
        now: Timestamp,
    ) -> impl core::future::Future<Output = ScanOutcome> + Send {
        core::future::ready(ScanOutcome::Clean { at: now })
    }
}

/// The standard EICAR anti-malware test string, so the infected path is
/// exercised without a real signature or a live daemon.
pub const EICAR: &[u8] = br"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

/// Flags exactly the EICAR string; everything else is clean. Stands in for a
/// signature scanner at the seam.
pub struct EicarScanner;

impl Scanner for EicarScanner {
    fn scan(
        &self,
        bytes: &[u8],
        now: Timestamp,
    ) -> impl core::future::Future<Output = ScanOutcome> + Send {
        let outcome = if contains(bytes, EICAR) {
            ScanOutcome::Infected {
                signature: "EICAR-Test-File".to_owned(),
            }
        } else {
            ScanOutcome::Clean { at: now }
        };
        core::future::ready(outcome)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return needle.is_empty();
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::{EicarScanner, Scanner, EICAR};
    use tam_types::{ScanOutcome, Timestamp};

    #[test]
    fn eicar_flags_infected_and_clean_bytes_pass() {
        let scanner = EicarScanner;
        let infected = futures::executor::block_on(scanner.scan(EICAR, Timestamp(1)));
        assert!(
            matches!(infected, ScanOutcome::Infected { .. }),
            "the EICAR string flags infected, blocking the publish"
        );
        let clean =
            futures::executor::block_on(scanner.scan(b"an ordinary worksheet", Timestamp(1)));
        assert!(
            matches!(clean, ScanOutcome::Clean { .. }),
            "ordinary content is clean"
        );
    }
}
