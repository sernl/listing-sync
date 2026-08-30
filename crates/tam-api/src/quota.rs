//! Which tier's quota a tenant's writes are bounded by.
//!
//! `Tier::quota` has been declared since M1 and enforced nowhere; the
//! authoring flow is the first surface a tenant's own writes control, so it
//! is the first place the numbers can bind. `listings_max` binds at the
//! product create and `storage_bytes_max` at the upload, which are the two
//! quantities a seller moves.
//!
//! The tier is derived from the recorded Paddle subscription rather than read
//! off a column, because no tier column exists: `billing_subscription` is the
//! only per-tenant billing fact in the tree, and it stores Paddle's own
//! status vocabulary verbatim. An organisation with no subscription, or one
//! Paddle last described as anything other than active or trialing, is on
//! Free. `Studio` is unreachable until a price-to-tier map exists, which is a
//! pricing decision rather than an inference this module may make.

use tam_limits::{Tier, TierQuota};
use tam_storage::BillingRepo;
use tam_types::OrgId;

use crate::error::APIError;
use crate::AppState;

/// The Paddle statuses that carry an entitlement. Anything else — `past_due`,
/// `paused`, `canceled`, or a status Paddle adds later — falls to Free, which
/// is the fail-closed direction: a lapsed subscription stops granting the
/// larger quota rather than granting it indefinitely.
const ENTITLING: [&str; 2] = ["active", "trialing"];

/// Which bound was exceeded, so the refusal names one rather than saying
/// "quota".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaKind {
    Listings,
    StorageBytes,
}

impl QuotaKind {
    pub const ALL: [Self; 2] = [Self::Listings, Self::StorageBytes];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Listings => "listings_max",
            Self::StorageBytes => "storage_bytes_max",
        }
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Listings => "this plan's listing quota is full",
            Self::StorageBytes => "this plan's storage quota is full",
        }
    }
}

/// The tier one recorded subscription status implies.
#[must_use]
pub fn tier_of(status: Option<&str>) -> Tier {
    match status {
        Some(status) if ENTITLING.contains(&status) => Tier::Pro,
        Some(_) | None => Tier::Free,
    }
}

/// This organisation's quota, read through the billing record.
pub async fn quota_for(state: &AppState, org: OrgId) -> Result<TierQuota, APIError> {
    let subscription = BillingRepo::new(state.pool.clone())
        .get(org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(tier_of(subscription.as_ref().map(|state| state.status.as_str())).quota())
}

#[cfg(test)]
mod tests {
    use super::{tier_of, QuotaKind};
    use tam_limits::Tier;

    #[test]
    fn an_organisation_with_no_subscription_is_on_the_free_tier() {
        assert_eq!(
            tier_of(None),
            Tier::Free,
            "a tenant that never reached checkout is on Free"
        );
    }

    #[test]
    fn only_an_entitling_status_lifts_the_quota() {
        assert_eq!(tier_of(Some("active")), Tier::Pro);
        assert_eq!(tier_of(Some("trialing")), Tier::Pro);
        for lapsed in ["past_due", "paused", "canceled", "something_new"] {
            assert_eq!(
                tier_of(Some(lapsed)),
                Tier::Free,
                "{lapsed} does not carry an entitlement, so the quota falls back rather than \
                 staying granted"
            );
        }
    }

    #[test]
    fn the_free_quota_is_strictly_smaller_than_the_one_it_falls_back_from() {
        let free = Tier::Free.quota();
        let pro = Tier::Pro.quota();
        assert!(
            free.listings_max < pro.listings_max && free.storage_bytes_max < pro.storage_bytes_max,
            "falling back to Free must actually bind, or the derivation is decoration"
        );
    }

    #[test]
    fn every_quota_kind_names_the_constant_it_bounds() {
        for kind in QuotaKind::ALL {
            match kind {
                QuotaKind::Listings | QuotaKind::StorageBytes => {}
            }
            assert!(
                kind.as_str().ends_with("_max"),
                "the wire name is the tam-limits field it bounds"
            );
        }
    }
}
