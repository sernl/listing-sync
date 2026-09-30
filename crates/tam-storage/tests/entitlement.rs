//! The grant store: what an organisation holds when it holds several grants,
//! what it holds when a grant has lapsed, what a revocation does, the usage
//! counters the Account page reads, and the move ledger the commit gate
//! spends against.
//!
//! The request-level counterparts live in `tam-api/tests/billing_flow.rs` and
//! `tam-api/tests/admin_flow.rs`.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_limits::Plan;
use tam_storage::{
    Accrual, CounterWindow, EntitlementRepo, GrantedBy, MonthlyCharge, MonthlyKind, MonthlySpend,
    MoveCredit, MoveSource, NewGrant, StorefrontAllowance,
};
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

fn purchase(grant: u8, plan: Plan, granted_at: Timestamp) -> NewGrant<'static> {
    NewGrant {
        id: id(grant),
        plan,
        rung: None,
        granted_by: GrantedBy::Stripe,
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

/// The case newest-wins would get wrong: an organisation on a subscription
/// that an operator later lifts to Studio must be served Studio, and the
/// reverse order must not hold a subscriber down to Free.
#[sqlx::test(migrations = "./migrations")]
async fn the_strongest_unexpired_grant_wins_regardless_of_which_landed_last(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(ORG_A, &purchase(1, Plan::Studio, EARLIER))
        .await
        .expect("the studio grant writes");
    repo.grant(ORG_A, &purchase(2, Plan::Subscriber, LATER))
        .await
        .expect("the subscription grant writes");

    let held = repo.current(ORG_A, NOW).await.expect("the read runs");
    assert_eq!(
        held.plan,
        Plan::Studio,
        "the later subscription must not hold Studio down to its narrower set"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_lapsed_grant_stops_entitling_and_a_revoked_one_stops_at_once(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(
        ORG_A,
        &NewGrant {
            expires_at: Some(EARLIER),
            ..purchase(1, Plan::Subscriber, EARLIER)
        },
    )
    .await
    .expect("the expired grant writes");
    assert_eq!(
        repo.current(ORG_A, NOW).await.expect("the read runs").plan,
        Plan::Free,
        "an expiry in the past stops granting rather than granting forever"
    );

    repo.grant(ORG_A, &purchase(2, Plan::Subscriber, EARLIER))
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

/// The cancellation path: the provider says the subscription ends at the period
/// end, so the grant keeps entitling until then and not a moment past it.
#[sqlx::test(migrations = "./migrations")]
async fn moving_a_grants_expiry_is_what_a_cancellation_does(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.grant(
        ORG_A,
        &NewGrant {
            source_ref: Some("sub_01"),
            ..purchase(1, Plan::Subscriber, EARLIER)
        },
    )
    .await
    .expect("the grant writes");
    let found = repo
        .provider_grant(ORG_A, "sub_01")
        .await
        .expect("the lookup runs")
        .expect("the subscription's grant is found by the processor's own id");
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
    repo.grant(ORG_A, &purchase(1, Plan::Subscriber, EARLIER))
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

/// 2026-08-06, a month before `NOW`.
const LAST_MONTH: Timestamp = Timestamp(1_786_000_000_000);

/// Runs one statement with `org` pinned, which is how every tenant table's
/// row-level policy admits a write.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool, org: OrgId, statement: &str) {
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the organisation pins");
    sqlx::query(statement)
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|error| panic!("{statement}: {error}"));
    tx.commit().await.expect("the seed commits");
}

fn preview(amount: u32, window: CounterWindow, at: Timestamp) -> MonthlyCharge {
    MonthlyCharge {
        kind: MonthlyKind::Preview,
        amount,
        cap: u32::MAX,
        window,
        at,
    }
}

/// Every figure the Billing page prints is a live count of its own source:
/// deleted resources, revoked devices, unlinked marketplaces and the
/// marketplaces' own system labels are not the seller's, last month's
/// previews and moves are not this month's, and another organisation's rows
/// are nobody's here.
#[sqlx::test(migrations = "./migrations")]
async fn the_usage_read_counts_every_figure_from_its_own_source(pool: PgPool) {
    provision(&pool).await;
    for (org, n) in [(ORG_A, 3), (ORG_B, 1)] {
        for _ in 0..n {
            seed(
                &pool,
                org,
                "INSERT INTO product (org_id, id, title, body, price_kind, created_at, updated_at) \
                 VALUES ($1, gen_random_uuid(), 'A worksheet', '', 'free', now(), now())",
            )
            .await;
        }
    }
    seed(
        &pool,
        ORG_A,
        "UPDATE product SET deleted_at = now() \
          WHERE org_id = $1 AND id = (SELECT id FROM product WHERE org_id = $1 LIMIT 1)",
    )
    .await;
    for name in ["Weekly", "Holiday"] {
        seed(
            &pool,
            ORG_A,
            &format!(
                "INSERT INTO resource_template (org_id, id, name, draft, created_at, updated_at) \
                 VALUES ($1, gen_random_uuid(), '{name}', '{{}}', now(), now())"
            ),
        )
        .await;
    }
    seed(
        &pool,
        ORG_A,
        "INSERT INTO collection (org_id, id, name, created_at, updated_at) \
         VALUES ($1, gen_random_uuid(), 'Autumn', now(), now())",
    )
    .await;
    for (name, system) in [("Maths", false), ("Year 3", false), ("TPT", true)] {
        seed(
            &pool,
            ORG_A,
            &format!(
                "INSERT INTO label (org_id, id, name, colour, created_at, system) \
                 VALUES ($1, gen_random_uuid(), '{name}', 'teal', now(), {system})"
            ),
        )
        .await;
    }
    for (device, revoked) in [("laptop", "NULL"), ("desktop", "NULL"), ("old", "now()")] {
        seed(
            &pool,
            ORG_A,
            &format!(
                "INSERT INTO device (org_id, id, name, os, arch, app_version, first_seen_at, \
                                     last_seen_at, revoked_at) \
                 VALUES ($1, '{device}', '{device}', 'linux', 'x86_64', '0.17.0', now(), now(), \
                         {revoked})"
            ),
        )
        .await;
    }
    for (marketplace, state) in [("tpt", "linked"), ("tes", "unlinked")] {
        seed(
            &pool,
            ORG_A,
            &format!(
                "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
                 VALUES ($1, gen_random_uuid(), '{marketplace}', '{state}', now(), now())"
            ),
        )
        .await;
    }

    let repo = EntitlementRepo::new(pool.clone());
    for (amount, at) in [(2, LAST_MONTH), (3, NOW)] {
        repo.spend_monthly(ORG_A, preview(amount, CounterWindow::Month, at))
            .await
            .expect("the spend runs");
    }
    repo.credit_moves(
        ORG_A,
        MoveCredit {
            delta: 10,
            source: MoveSource::Pack,
            source_ref: Some("cs_test_usage"),
            expires_at: Some(LATER),
            at: LAST_MONTH,
        },
    )
    .await
    .expect("the credit writes");
    for (item, at) in [(0x41, LAST_MONTH), (0x42, NOW), (0x43, NOW)] {
        repo.debit_move(ORG_A, id(item), at)
            .await
            .expect("the debit runs");
    }

    let usage = repo.usage(ORG_A, NOW).await.expect("the read runs");
    assert_eq!(usage.resources, 2, "the deleted resource is not counted");
    assert_eq!(usage.templates, 2);
    assert_eq!(usage.collections, 1);
    assert_eq!(
        usage.labels, 2,
        "a marketplace's system label is not the seller's"
    );
    assert_eq!(
        usage.devices, 2,
        "a signed-out device is in the record, not the count"
    );
    assert_eq!(
        usage.marketplaces, 1,
        "only a linked marketplace is connected"
    );
    assert_eq!(usage.previews, 3, "this month's previews");
    assert_eq!(usage.previews_lifetime, 5, "every month's previews");
    assert_eq!(usage.moves_this_month, 2, "this month's commits");

    let other = repo.usage(ORG_B, NOW).await.expect("the read runs");
    assert_eq!(other.resources, 1);
    assert_eq!(
        (other.templates, other.labels, other.devices, other.previews),
        (0, 0, 0, 0),
        "one organisation's rows are not another's"
    );
}

/// A lifetime allowance is measured against every month's previews together,
/// so a new month gives none back, and a refusal spends nothing.
#[sqlx::test(migrations = "./migrations")]
async fn a_lifetime_allowance_counts_every_month_and_refuses_past_its_cap(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    let lifetime = |amount, at| MonthlyCharge {
        cap: 5,
        ..preview(amount, CounterWindow::Lifetime, at)
    };
    assert_eq!(
        repo.spend_monthly(ORG_A, lifetime(3, LAST_MONTH))
            .await
            .expect("the spend runs"),
        MonthlySpend::Granted { used: 3 }
    );
    assert_eq!(
        repo.spend_monthly(ORG_A, lifetime(3, NOW))
            .await
            .expect("the spend runs"),
        MonthlySpend::Refused { used: 3 },
        "three more would be six of five, though this month has used none"
    );
    assert_eq!(
        repo.spend_monthly(ORG_A, lifetime(2, NOW))
            .await
            .expect("the spend runs"),
        MonthlySpend::Granted { used: 5 }
    );
    assert_eq!(
        repo.spend_monthly(ORG_A, lifetime(1, LATER))
            .await
            .expect("the spend runs"),
        MonthlySpend::Refused { used: 5 },
        "a later month gives none back"
    );
    let usage = repo.usage(ORG_A, NOW).await.expect("the read runs");
    assert_eq!((usage.previews, usage.previews_lifetime), (2, 5));
    assert_eq!(
        repo.counted_used(ORG_A, MonthlyKind::Preview, CounterWindow::Lifetime, LATER)
            .await
            .expect("the read runs"),
        5
    );

    // The same rows answer a monthly plan: an upgrade counts this month
    // alone, with no conversion.
    assert_eq!(
        repo.spend_monthly(
            ORG_A,
            MonthlyCharge {
                cap: 20,
                ..preview(1, CounterWindow::Month, NOW)
            }
        )
        .await
        .expect("the spend runs"),
        MonthlySpend::Granted { used: 3 }
    );
    assert_eq!(
        repo.spend_monthly(ORG_B, lifetime(5, NOW))
            .await
            .expect("the spend runs"),
        MonthlySpend::Granted { used: 5 },
        "one organisation's lifetime is not another's"
    );
}

/// The balance is a sum over what has not expired, and a credit whose
/// reference has already been seen is not a second credit.
#[sqlx::test(migrations = "./migrations")]
async fn a_credit_is_idempotent_on_its_reference_and_lapses_on_its_expiry(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    let pack = MoveCredit {
        delta: 100,
        source: MoveSource::Pack,
        source_ref: Some("cs_test_one"),
        expires_at: Some(LATER),
        at: EARLIER,
    };
    assert!(repo
        .credit_moves(ORG_A, pack)
        .await
        .expect("the credit writes"));
    assert!(
        !repo
            .credit_moves(ORG_A, pack)
            .await
            .expect("the replay runs"),
        "a replayed webhook credits once"
    );

    let held = repo.move_balance(ORG_A, NOW).await.expect("the read runs");
    assert_eq!(held.available, 100);
    assert_eq!(held.expiring_soonest, Some(LATER));

    let lapsed = repo
        .move_balance(ORG_A, Timestamp(LATER.0 + 1))
        .await
        .expect("the read runs");
    assert_eq!(
        lapsed.available, 0,
        "a pack's moves are gone the instant they expire"
    );
    assert_eq!(
        repo.move_balance(ORG_B, NOW)
            .await
            .expect("the read runs")
            .available,
        0,
        "one organisation's balance is not another's"
    );
}

/// The property the whole expiry design exists for: a move spent against a
/// credit leaves with that credit, so a lapsed pack cannot drive a balance
/// negative for a seller who did nothing wrong.
#[sqlx::test(migrations = "./migrations")]
async fn a_spent_move_lapses_with_the_credit_it_drew_against(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    repo.credit_moves(
        ORG_A,
        MoveCredit {
            delta: 20,
            source: MoveSource::Pack,
            source_ref: Some("cs_test_two"),
            expires_at: Some(LATER),
            at: EARLIER,
        },
    )
    .await
    .expect("the credit writes");

    assert_eq!(
        repo.debit_move(ORG_A, id(0x31), NOW)
            .await
            .expect("the debit runs"),
        Some(1),
        "the first move ever is the first"
    );
    assert_eq!(
        repo.debit_move(ORG_A, id(0x31), NOW)
            .await
            .expect("the replay runs"),
        None,
        "a retried settle spends nothing"
    );
    assert_eq!(
        repo.debit_move(ORG_A, id(0x32), NOW)
            .await
            .expect("the debit runs"),
        Some(2)
    );
    assert_eq!(
        repo.move_balance(ORG_A, NOW)
            .await
            .expect("the read runs")
            .available,
        18
    );
    assert_eq!(
        repo.move_balance(ORG_A, Timestamp(LATER.0 + 1))
            .await
            .expect("the read runs")
            .available,
        0,
        "the pack and the two moves drawn against it leave together"
    );
}

/// The subscription tops up to its cap and never past it, and the free
/// lifetime grant is given to a storefront once across every tenant.
#[sqlx::test(migrations = "./migrations")]
async fn the_accrual_stops_at_the_cap_and_a_storefront_is_granted_once(pool: PgPool) {
    provision(&pool).await;
    let repo = EntitlementRepo::new(pool);
    let mut month = EARLIER;
    for expected in [25_i64, 50, 75, 75] {
        repo.accrue_subscription_moves(
            ORG_A,
            Accrual {
                period_start: month,
                per_period: 25,
                accrual_cap: 75,
                at: month,
            },
        )
        .await
        .expect("the accrual runs");
        assert_eq!(
            repo.move_balance(ORG_A, month)
                .await
                .expect("the read runs")
                .available,
            expected,
            "the rolling allowance stops at its cap"
        );
        month = Timestamp(month.0 + 2_592_000_000);
    }
    assert!(
        !repo
            .accrue_subscription_moves(
                ORG_A,
                Accrual {
                    period_start: EARLIER,
                    per_period: 25,
                    accrual_cap: 75,
                    at: EARLIER,
                },
            )
            .await
            .expect("the replay runs"),
        "a period already credited is not credited again"
    );

    let digest = [0x5A_u8; 32];
    let claim = StorefrontAllowance {
        marketplace: "tpt",
        digest: &digest,
        moves: 5,
        at: NOW,
    };
    assert!(repo
        .grant_storefront_allowance(ORG_B, claim)
        .await
        .expect("the first claim runs"));
    assert_eq!(
        repo.move_balance(ORG_B, NOW)
            .await
            .expect("the read runs")
            .available,
        5
    );
    assert!(
        !repo
            .grant_storefront_allowance(ORG_A, claim)
            .await
            .expect("the second claim runs"),
        "a shop already claimed grants a second organisation nothing"
    );
    assert_eq!(
        repo.move_balance(ORG_A, NOW)
            .await
            .expect("the read runs")
            .available,
        75,
        "and credits it nothing"
    );
}
