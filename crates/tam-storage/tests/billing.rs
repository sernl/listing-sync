//! The billing store: the tenant fence on the subscription row, the ordering
//! fence that makes a reordered provider delivery harmless, and the honest
//! absence a tenant that never reached checkout reads.
//!
//! Every property here is a property of the write, not of the route above it;
//! the request-level counterpart lives in `tam-api/tests/billing_flow.rs`.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{BillingRepo, SubscriptionState};
use tam_types::{OrgId, Timestamp, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const ORG_C: OrgId = OrgId(Uuid([0xCC; 16]));

const EARLY: Timestamp = Timestamp(1_000);
const LATER: Timestamp = Timestamp(2_000);
const WROTE_AT: Timestamp = Timestamp(9_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b"), (ORG_C, "org-c")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the fixture org inserts");
    }
}

fn state(subscription: &str, status: &str, occurred_at: Timestamp) -> SubscriptionState {
    SubscriptionState {
        provider_subscription_id: subscription.to_owned(),
        provider_customer_id: format!("ctm_for_{subscription}"),
        status: status.to_owned(),
        provider_price_id: Some(format!("price_for_{subscription}")),
        current_period_end: Some(Timestamp(occurred_at.0 + 30_000)),
        occurred_at,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_tenant_reads_back_the_state_its_own_event_recorded(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    assert!(
        repo.apply(ORG_A, &state("sub_a", "active", EARLY), WROTE_AT)
            .await
            .expect("the first event applies"),
        "a first event on an organisation with no row lands"
    );
    let stored = repo
        .get(ORG_A)
        .await
        .expect("the read runs")
        .expect("the row is there to read");
    assert_eq!(stored, state("sub_a", "active", EARLY), "what was written");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_tenant_that_never_subscribed_reads_none(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    repo.apply(ORG_A, &state("sub_a", "active", EARLY), WROTE_AT)
        .await
        .expect("the other tenant's event applies");
    assert_eq!(
        repo.get(ORG_C).await.expect("the read runs"),
        None,
        "no subscription is None rather than a fabricated free-tier row"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn one_tenants_event_never_touches_anothers_row(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    repo.apply(ORG_A, &state("sub_a", "active", EARLY), WROTE_AT)
        .await
        .expect("the first tenant's event applies");
    repo.apply(ORG_B, &state("sub_b", "trialing", EARLY), WROTE_AT)
        .await
        .expect("the second tenant's event applies");

    // The event that would corrupt the fence if it were not there: same
    // instant, same shape, a different tenant's identifiers.
    repo.apply(ORG_A, &state("sub_a", "canceled", LATER), WROTE_AT)
        .await
        .expect("the first tenant's later event applies");

    assert_eq!(
        repo.get(ORG_A)
            .await
            .expect("the read runs")
            .map(|stored| stored.status),
        Some("canceled".to_owned()),
        "the tenant the event named moved"
    );
    assert_eq!(
        repo.get(ORG_B).await.expect("the read runs"),
        Some(state("sub_b", "trialing", EARLY)),
        "and the other tenant's row is byte for byte what it was"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_event_older_than_the_stored_one_does_not_regress_the_state(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    repo.apply(ORG_A, &state("sub_a", "active", LATER), WROTE_AT)
        .await
        .expect("the newer event applies");

    // A redelivery of the cancellation that preceded it: Stripe retries, and a
    // retry can overtake. Applying it would leave a paying tenant recorded as
    // cancelled until the next event happened to arrive.
    assert!(
        !repo
            .apply(ORG_A, &state("sub_a", "canceled", EARLY), WROTE_AT)
            .await
            .expect("the stale event is handled rather than raising"),
        "a stale event reports that it was not applied"
    );
    assert_eq!(
        repo.get(ORG_A)
            .await
            .expect("the read runs")
            .map(|stored| stored.status),
        Some("active".to_owned()),
        "the state the provider last described is what stands"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_event_at_the_stored_instant_is_applied(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    repo.apply(ORG_A, &state("sub_a", "active", EARLY), WROTE_AT)
        .await
        .expect("the first event applies");
    assert!(
        repo.apply(ORG_A, &state("sub_a", "past_due", EARLY), WROTE_AT)
            .await
            .expect("the equal-instant event is handled"),
        "the fence declines what is older, not what is equal, so a corrected \
         redelivery of one event still lands"
    );
    assert_eq!(
        repo.get(ORG_A)
            .await
            .expect("the read runs")
            .map(|stored| stored.status),
        Some("past_due".to_owned()),
        "and the correction is what is stored"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_providers_own_status_vocabulary_is_stored_as_received(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    // A status this codebase has never heard of, which is the case the open
    // column exists for: it must land in a readable row rather than abort a
    // webhook Stripe would then retry forever.
    repo.apply(
        ORG_A,
        &state("sub_a", "some_status_stripe_added_later", EARLY),
        WROTE_AT,
    )
    .await
    .expect("an unrecognised status still applies");
    assert_eq!(
        repo.get(ORG_A)
            .await
            .expect("the read runs")
            .map(|stored| stored.status),
        Some("some_status_stripe_added_later".to_owned()),
        "stored verbatim, so an operator reads what the provider actually said"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn one_provider_subscription_cannot_be_claimed_by_two_tenants(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    repo.apply(ORG_A, &state("sub_shared", "active", EARLY), WROTE_AT)
        .await
        .expect("the first tenant's event applies");
    assert!(
        repo.apply(ORG_B, &state("sub_shared", "active", LATER), WROTE_AT)
            .await
            .is_err(),
        "a second tenant claiming one provider subscription is refused by the \
         constraint rather than silently attaching a paying customer twice"
    );
    assert_eq!(
        repo.get(ORG_B).await.expect("the read runs"),
        None,
        "and the refused write left nothing behind"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_subscription_carrying_no_billing_period_stores_none(pool: PgPool) {
    provision(&pool).await;
    let repo = BillingRepo::new(pool);
    repo.apply(
        ORG_A,
        &SubscriptionState {
            current_period_end: None,
            ..state("sub_a", "trialing", EARLY)
        },
        WROTE_AT,
    )
    .await
    .expect("the event applies");
    assert_eq!(
        repo.get(ORG_A)
            .await
            .expect("the read runs")
            .and_then(|stored| stored.current_period_end),
        None,
        "an absent period reads back absent rather than as an invented instant"
    );
}
