//! Every resource bound in the system. Nothing outside this crate declares one.
//!
//! Admission rule: a constant belongs here only when exceeding it is a
//! shared-resource incident affecting tenants other than the one that caused
//! it, and when no type, database constraint, or OS-level limit already bounds
//! it. A number that bounds only its own caller is an ordinary `const` next to
//! that caller. This crate is deliberately small; a large limits module is a
//! maintenance surface impersonating discipline.
//!
//! Every constant admitted to the resource-bound set carries a provenance
//! marker in its doc comment, a factual claim about where the number came
//! from; the three per-tier rates are covered by the marker on `Tier::quota`.
//! `MEASURED` cites an observation recorded during the spike, named in the
//! comment. No constant carries this marker yet, because the spike has not run.
//! `SIZED` means derived by arithmetic from a quantity that is known
//! independently of the spike, such as the box's RAM or a protocol limit.
//! `UNCALIBRATED` is a guess, and is a release blocker for the first paying
//! deployment rather than a wish. The test below pins how many of them exist,
//! so lowering the budget is a deliberate edit and raising it cannot pass
//! unnoticed. Drive it to zero before taking money.
//!
//! This crate has no dependencies and must keep none, so that every other
//! crate can depend on it without acquiring an edge.

#![forbid(unsafe_code)]

use std::num::NonZeroU32;

/// Billing tier.
///
/// Declared here rather than in the billing crate because `limits` is a leaf
/// with no workspace dependencies and the quota table below is keyed by it.
/// The billing crate re-exports this type; it does not redeclare it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    Free,
    Pro,
    Studio,
}

/// The quotas a single tier grants.
///
/// One struct per tier rather than parallel arrays indexed by a discriminant.
/// Arrays would need an index, a bounds check, and a length assertion per
/// array; a struct returned from an exhaustive `match` needs none of those,
/// and adding a tier becomes a compile error at the one site that matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierQuota {
    pub api_requests_per_minute: NonZeroU32,
    pub listings_max: u32,
    pub storage_bytes_max: u64,
}

impl Tier {
    /// Every tier. Test-only scaffolding: nothing in production indexes it.
    ///
    /// Rust cannot check on stable that this array is total over the enum, so
    /// the forcing function is the exhaustive `match` in `all_is_total_over_the_enum`,
    /// which fails to compile when a variant is added. `wildcard_enum_match_arm`
    /// is denied workspace-wide, so that match cannot be silenced with `_`.
    pub const ALL: [Self; 3] = [Self::Free, Self::Pro, Self::Studio];

    /// UNCALIBRATED. Rates are placeholders chosen so the free tier cannot
    /// saturate one box at the concurrency below; listing and storage ceilings
    /// are placeholders until pricing is set. Nothing here is measured.
    #[must_use]
    pub const fn quota(self) -> TierQuota {
        match self {
            Self::Free => TierQuota {
                api_requests_per_minute: FREE_RPM,
                listings_max: 100,
                storage_bytes_max: 1 << 30,
            },
            Self::Pro => TierQuota {
                api_requests_per_minute: PRO_RPM,
                listings_max: 5_000,
                storage_bytes_max: 20 << 30,
            },
            Self::Studio => TierQuota {
                api_requests_per_minute: STUDIO_RPM,
                listings_max: 100_000,
                storage_bytes_max: 200 << 30,
            },
        }
    }
}

const FREE_RPM: NonZeroU32 = NonZeroU32::new(60).unwrap();
const PRO_RPM: NonZeroU32 = NonZeroU32::new(600).unwrap();
const STUDIO_RPM: NonZeroU32 = NonZeroU32::new(3_000).unwrap();

pub mod http {
    /// SIZED against RAM, not against traffic: at this ceiling the concurrent
    /// request limit cannot buffer more than a small fraction of the box's
    /// memory. Applies to every route except the ingestion upload routes.
    /// Revisit once the spike records real listing-metadata payload sizes.
    pub const REQUEST_BODY_BYTES_MAX: u64 = 2 * 1024 * 1024;

    /// UNCALIBRATED. No file-size census of any connected marketplace's
    /// resources has been taken. The spike's recording obligation covers it.
    pub const UPLOAD_BODY_BYTES_MAX: u64 = 256 * 1024 * 1024;
}

pub mod ingest {
    /// SIZED: the absolute ceiling on bytes written out of an archive,
    /// independent of the ratio check below. Bounding compressed input is not
    /// a zip-bomb defence; bounding decompressed output is.
    pub const ARCHIVE_UNCOMPRESSED_BYTES_MAX: u64 = 1024 * 1024 * 1024;

    /// UNCALIBRATED. Uncompressed divided by compressed, checked incrementally
    /// rather than after the fact. The number must be set from the ratio
    /// distribution of real seller bundles, which has not been sampled; set
    /// deliberately loose so a false rejection is unlikely before it is.
    pub const ARCHIVE_COMPRESSION_RATIO_MAX: u64 = 200;
}

pub mod job {
    use std::time::Duration;

    /// UNCALIBRATED: retry count is only meaningful once the fault taxonomy
    /// from the spike says which faults are worth retrying at all.
    pub const ATTEMPTS_MAX: u32 = 5;

    /// SIZED against support response time, not against publish duration: a
    /// wedged job must free its lease well inside one working day. Attempts
    /// multiplied by exponential backoff can exceed any sane duration at a
    /// small attempt count, so this deadline is enforced alongside the count
    /// rather than derived from it.
    pub const WALL_CLOCK_MAX: Duration = Duration::from_mins(30);

    /// UNCALIBRATED: bounded by how many ingestion subprocesses and browser
    /// sessions fit in the box's RAM alongside the connection pool, which has
    /// not been measured. Automation runs here, on our own infrastructure, so
    /// it is inside this bound rather than outside it. Deliberately low.
    pub const CONCURRENT_JOBS_GLOBAL_MAX: u32 = 8;
}

pub mod browser {
    use std::num::NonZeroU32;

    /// UNCALIBRATED. The count of browser sessions spawned at startup and never
    /// spawned on demand. One process drives every seller's authenticated
    /// session, so an unbounded pool is a cross-tenant memory incident rather
    /// than one tenant's slow job. The hardware ceiling is roughly thirty
    /// concurrent sessions on the current box; the operating point is set by
    /// marketplace pacing, which is an order of magnitude lower and unmeasured,
    /// so this starts at the serialised end and moves only on evidence.
    pub const SESSIONS_MAX: NonZeroU32 = NonZeroU32::new(2).unwrap();
}

pub mod llm {
    /// SIZED as a loss ceiling rather than from a token price: this is the
    /// per-tenant daily spend the business is willing to lose to a runaway
    /// loop before a human looks. Recompute against the provider's actual
    /// per-token price once one is chosen. Spend is a first-class bounded
    /// resource here, not a proxy for request count.
    pub const CENTS_PER_TENANT_PER_DAY_MAX: u32 = 500;
}

pub mod marketplace {
    use std::num::NonZeroU32;

    /// UNCALIBRATED, and the only constant here with a legal rather than an
    /// operational justification. Tes publishes no throttle and none has been
    /// observed; where one is published, as on Etsy, the lower figure binds.
    /// Raising it requires the written-terms answer in the charter's section 1.
    pub const OUTBOUND_REQUESTS_PER_MINUTE_MAX: NonZeroU32 = NonZeroU32::new(30).unwrap();
}

/// Relationships between constants, checked at compile time rather than at run
/// time. Each is a claim that could actually be false after an edit; a claim
/// the declaration already guarantees is not written here.
const _: () = {
    assert!(
        http::UPLOAD_BODY_BYTES_MAX >= http::REQUEST_BODY_BYTES_MAX,
        "the upload ceiling must not be tighter than the ordinary body ceiling"
    );
    assert!(
        ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX >= http::UPLOAD_BODY_BYTES_MAX,
        "an archive at the upload ceiling must be able to expand at all"
    );
    assert!(
        http::UPLOAD_BODY_BYTES_MAX * ingest::ARCHIVE_COMPRESSION_RATIO_MAX
            > ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
        "the ratio cap must be able to bind before the absolute cap, or one of them is dead code"
    );
    assert!(
        usize::BITS >= 64,
        "byte bounds are u64 and are converted to usize at the axum boundary"
    );
    assert!(
        browser::SESSIONS_MAX.get() <= job::CONCURRENT_JOBS_GLOBAL_MAX,
        "a pre-spawned session with no job that can reach it is memory held for nothing"
    );
};

#[cfg(test)]
mod tests {
    use super::{http, ingest, job, Tier};

    /// The `UNCALIBRATED` markers are a countdown, not decoration.
    ///
    /// Pinning the count makes removing a marker a deliberate edit and makes
    /// adding one impossible to do quietly. Drive the number to zero before the
    /// first paying deployment; that is what makes the marker a release
    /// blocker rather than a wish.
    #[test]
    fn uncalibrated_markers_are_ratcheted() {
        let marked = include_str!("lib.rs")
            .lines()
            .filter(|line| line.trim_start().starts_with("/// UNCALIBRATED"))
            .count();
        assert_eq!(
            marked, UNCALIBRATED_BUDGET,
            "the uncalibrated-constant budget moved; lower it deliberately or calibrate the number"
        );
    }

    /// Counts doc-comment markers only, so prose and assertion messages that
    /// mention the marker do not inflate it.
    const UNCALIBRATED_BUDGET: usize = 7;

    #[test]
    fn all_is_total_over_the_enum() {
        for tier in Tier::ALL {
            match tier {
                Tier::Free | Tier::Pro | Tier::Studio => {}
            }
        }
        assert_eq!(
            Tier::ALL.len(),
            3,
            "a variant was added to Tier without being added to Tier::ALL"
        );
    }

    #[test]
    fn every_tier_grants_a_strictly_larger_listing_quota_than_the_one_below() {
        let quotas: Vec<u32> = Tier::ALL.iter().map(|t| t.quota().listings_max).collect();
        for pair in quotas.windows(2) {
            assert!(pair[1] > pair[0], "tier quotas must increase: {pair:?}");
        }
    }

    #[test]
    fn the_wall_clock_deadline_outlives_the_full_retry_budget_at_the_backoff_base() {
        let base = std::time::Duration::from_millis(500);
        let worst = base * job::ATTEMPTS_MAX;
        assert!(
            job::WALL_CLOCK_MAX > worst,
            "the deadline must not fire before the retries are exhausted"
        );
    }

    #[test]
    fn a_maximal_upload_expanding_at_the_maximal_ratio_exceeds_the_absolute_cap() {
        let expanded = http::UPLOAD_BODY_BYTES_MAX * ingest::ARCHIVE_COMPRESSION_RATIO_MAX;
        assert!(
            expanded > ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
            "the absolute cap is the binding one for a maximal upload"
        );
    }
}
