//! The grant store: what an organisation holds when it holds several grants,
//! what it holds when a grant has lapsed, what a revocation does, and the
//! usage counters the Account page and the migration gate read.
//!
//! The request-level counterparts live in `tam-api/tests/billing_flow.rs` and
//! `tam-api/tests/admin_flow.rs`.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_limits::Plan;
use tam_storage::{EntitlementRepo, GrantedBy, NewGrant};
use tam_types::{OrgId, Timestamp, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));

/// 2026-09-12T00:00:00Z, so the month arithmetic has a real month either side.
const NOW: Timestamp = Timestamp(1_789_171_200_000);
const EARLIER: Timestamp = Timestamp(1_789_000_000_000);
const LATER: Timestamp = Timestamp(1_790_000_000_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the fixture org inserts");
    }
}

fn id(byte: u8) -> Uuid {
    Uuid([byte; 16])
}

fn paddle(grant: u8, plan: Plan, granted_at: Timestamp) -> NewGrant<'static> {
    NewGrant {
        id: id(grant),
        plan,
        rung: None,
        granted_by: GrantedBy::Paddle,
        grantor_user: None,
        reason: None,
        source_ref: None,
        granted_at,
        expires_at: None,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn an_organisation_with_no_grant_holds_free(pool: PgPool) {
    provision(&pool).await;
    let held = EntitlementRepo::new(pool)
        .current(ORG_A, NOW)
        .await
        .expect("the read runs");
    assert_eq!(held.plan, Plan::Free);
    assert_eq!(
        held.granted_by, None,
        "nobody granted free, so naming a grantor would state something false"
    );
}

/// The case newest-wins would get wrong: a seller who bought a one-off import
/// and later subscribed must be served the subscription.
#[sqlx::test(migrations = "./migrations")]
async fn the_strongest_unexpired_grant_wins_regardless_of_which_landed_last(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(ORG_A, &paddle(1, Plan::Subscriber, EARLIER))
        .await
        .expect("the subscription grant writes");
    repo.grant(
        ORG_A,
        &NewGrant {
            rung: Some(50),
            ..paddle(2, Plan::MigrationOnly, LATER)
        },
    )
    .await
    .expect("the one-off grant writes");

    let held = repo.current(ORG_A, NOW).await.expect("the read runs");
    assert_eq!(
        held.plan,
        Plan::Subscriber,
        "the later one-off must not hold the subscription down to its narrower set"
    );
    assert_eq!(held.rung, None, "the subscription's row carries no rung");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_lapsed_grant_stops_entitling_and_a_revoked_one_stops_at_once(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(
        ORG_A,
        &NewGrant {
            expires_at: Some(EARLIER),
            ..paddle(1, Plan::Subscriber, EARLIER)
        },
    )
    .await
    .expect("the expired grant writes");
    assert_eq!(
        repo.current(ORG_A, NOW).await.expect("the read runs").plan,
        Plan::Free,
        "an expiry in the past stops granting rather than granting forever"
    );

    repo.grant(ORG_A, &paddle(2, Plan::Subscriber, EARLIER))
        .await
        .expect("the live grant writes");
    assert_eq!(
        repo.current(ORG_A, NOW).await.expect("the read runs").plan,
        Plan::Subscriber
    );
    assert!(
        repo.revoke(ORG_A, id(2), NOW)
            .await
            .expect("the revoke runs"),
        "a live grant revokes"
    );
    assert!(
        !repo
            .revoke(ORG_A, id(2), NOW)
            .await
            .expect("the second revoke runs"),
        "revoking twice answers that nothing moved rather than silently succeeding"
    );
    assert_eq!(
        repo.current(ORG_A, NOW).await.expect("the read runs").plan,
        Plan::Free
    );
}

/// The cancellation path: Paddle says the subscription ends at the period
/// end, so the grant keeps entitling until then and not a moment past it.
#[sqlx::test(migrations = "./migrations")]
async fn moving_a_grants_expiry_is_what_a_cancellation_does(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(
        ORG_A,
        &NewGrant {
            source_ref: Some("sub_01"),
            ..paddle(1, Plan::Subscriber, EARLIER)
        },
    )
    .await
    .expect("the grant writes");
    let found = repo
        .paddle_grant(ORG_A, "sub_01")
        .await
        .expect("the lookup runs")
        .expect("the subscription's grant is found by its Paddle id");
    assert_eq!(found, id(1));

    assert!(repo
        .set_expiry(ORG_A, found, Some(LATER))
        .await
        .expect("the expiry moves"));
    assert_eq!(
        repo.current(ORG_A, NOW).await.expect("the read runs").plan,
        Plan::Subscriber,
        "a cancellation dated in the future still entitles until it arrives"
    );
    assert_eq!(
        repo.current(ORG_A, Timestamp(LATER.0 + 1))
            .await
            .expect("the read runs")
            .plan,
        Plan::Free,
        "and stops at it"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn one_organisations_grant_is_invisible_to_another(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(ORG_A, &paddle(1, Plan::Subscriber, EARLIER))
        .await
        .expect("the grant writes");
    assert_eq!(
        repo.current(ORG_B, NOW).await.expect("the read runs").plan,
        Plan::Free,
        "the tenant fence holds over the grant table"
    );
    assert!(
        repo.history(ORG_B).await.expect("the read runs").is_empty(),
        "and over its history"
    );
}

/// The history is what the backoffice panel renders, so it has to keep the
/// rows the entitlement read drops.
#[sqlx::test(migrations = "./migrations")]
async fn the_history_keeps_revoked_and_expired_rows_with_their_attribution(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    let operator = Uuid([0x11; 16]);
    repo.grant(
        ORG_A,
        &NewGrant {
            id: id(1),
            plan: Plan::Studio,
            rung: None,
            granted_by: GrantedBy::Operator,
            grantor_user: Some(operator),
            reason: Some("customer zero, for the migration"),
            source_ref: None,
            granted_at: EARLIER,
            expires_at: Some(LATER),
        },
    )
    .await
    .expect("the operator grant writes");
    assert!(repo
        .revoke(ORG_A, id(1), NOW)
        .await
        .expect("the revoke runs"));

    let history = repo.history(ORG_A).await.expect("the read runs");
    assert_eq!(history.len(), 1, "a revoked grant stays in the record");
    let row = &history[0];
    assert_eq!(row.plan, Plan::Studio);
    assert_eq!(row.granted_by, GrantedBy::Operator);
    assert_eq!(row.grantor_user, Some(operator));
    assert_eq!(
        row.reason.as_deref(),
        Some("customer zero, for the migration")
    );
    assert_eq!(row.revoked_at, Some(NOW));
}

#[sqlx::test(migrations = "./migrations")]
async fn the_migration_counter_opens_at_the_first_of_the_month_and_names_its_reset(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    let usage = repo.usage(ORG_A, NOW).await.expect("the read runs");
    assert_eq!(usage.resources, 0);
    assert_eq!(usage.migrations_this_month, 0);
    assert_eq!(
        usage.migrations_reset_at,
        Timestamp(1_790_812_800_000),
        "the counter resets at 2026-10-01T00:00:00Z"
    );
    assert_eq!(
        repo.migrations_used_this_month(ORG_A, NOW)
            .await
            .expect("the read runs"),
        0,
        "an organisation that has migrated nothing has used nothing"
    );
}
