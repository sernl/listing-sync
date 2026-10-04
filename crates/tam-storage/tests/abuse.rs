//! Abuse prevention at the schema level (migrations 0105 and 0106).
//!
//! The free-moves farming loop end to end through the check-in that binds a
//! shop: one shop's five moves are granted once, whichever account names it
//! next and whether or not the first account still exists. Then the scorer's
//! rules, an operator's decisions, and the gates those decisions close.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_secrets::Kek;
use tam_storage::{
    AbuseAction, AbuseBackofficeRepo, AbuseRepo, BannedKind, ConnectionRepo, Decision,
    DeviceRegistration, DeviceRepo, DeviceSessionReport, DeviceSessionStatus, EntitlementRepo,
    FlagKind, SessionRepo, SessionToken, Signal, SignalKind, StorageError,
};
use tam_types::{
    Actor, ConnectionId, Marketplace, OrgId, Stamp, Timestamp, UserId, Uuid,
};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const ORG_C: OrgId = OrgId(Uuid([0xCC; 16]));
const OPERATOR: Uuid = Uuid([0x0F; 16]);
const STOREFRONT: &str = "900000001";
const NOW: Timestamp = Timestamp(1_760_000_000_000);
const LATER: Timestamp = Timestamp(1_760_000_600_000);

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_orgs(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b"), (ORG_C, "org-c")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(db_uuid(org.0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org inserts");
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn checking_in(app: &PgPool) -> DeviceRepo {
    DeviceRepo::new(app.clone())
        .with_account_key(Kek::from_bytes(&[0x11; 32]).expect("a 32-byte key is a key"))
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn enrol(devices: &DeviceRepo, org: OrgId, id: &str) {
    devices
        .register(
            org,
            &DeviceRegistration {
                id,
                name: "a classroom laptop",
                os: "linux",
                arch: "x86_64",
                app_version: "0.21.0",
            },
            NOW,
        )
        .await
        .expect("the device registers");
}

fn holding(storefront: &str) -> DeviceSessionReport<'_> {
    DeviceSessionReport {
        marketplace: Marketplace::Tpt,
        account_label: None,
        external_id: Some(storefront),
        status: DeviceSessionStatus::Connected,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn bind(devices: &DeviceRepo, org: OrgId, device: &str) {
    devices
        .heartbeat(org, device, &[holding(STOREFRONT)], NOW)
        .await
        .expect("the check-in binds the shop")
        .expect("the device it named is registered");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn balance(app: &PgPool, org: OrgId) -> i64 {
    EntitlementRepo::new(app.clone())
        .move_balance(org, NOW)
        .await
        .expect("the balance reads")
        .available
}

/// The seller's own disconnect, through the repository the console calls.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn unlink(app: &PgPool, org: OrgId) -> ConnectionId {
    let mut tx = app.begin().await.expect("the read opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin sets");
    let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM connection WHERE marketplace = 'tpt'")
        .fetch_one(&mut *tx)
        .await
        .expect("the connection reads");
    tx.commit().await.expect("the read commits");
    let connection = ConnectionId(Uuid(*id.as_bytes()));
    assert!(ConnectionRepo::new(app.clone())
        .unlink(
            org,
            connection,
            Stamp {
                at: NOW,
                actor: Actor::Person(UserId(Uuid([0x01; 16]))),
            },
        )
        .await
        .expect("the unlink runs"));
    connection
}

/// Spends one move, the way a settled commit does.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn spend(app: &PgPool, org: OrgId, item: u8) {
    EntitlementRepo::new(app.clone())
        .debit_move(org, Uuid([item; 16]), NOW)
        .await
        .expect("the debit runs")
        .expect("a fresh item is charged");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn open_flags(app: &PgPool, org: OrgId) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT kind FROM abuse_flag WHERE org_id = $1 AND resolved_at IS NULL ORDER BY kind",
    )
    .bind(db_uuid(org.0))
    .fetch_all(app)
    .await
    .expect("the flags read")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn erase(app: &PgPool, org: OrgId) {
    let mut tx = app.begin().await.expect("the erasure opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin sets");
    sqlx::query("SELECT erase_organisation($1)")
        .bind(db_uuid(org.0))
        .execute(&mut *tx)
        .await
        .expect("the organisation erases");
    tx.commit().await.expect("the erasure commits");
}

/// The farming loop, closed: connect a shop, spend, unlink, connect the same
/// shop from a second account, and from a third after the first account is
/// deleted. Only the first is credited, and the shared shop is flagged.
#[sqlx::test(migrations = "./migrations")]
async fn one_shops_free_moves_are_granted_once_whichever_account_names_it(app: PgPool) {
    seed_orgs(&app).await;
    let devices = checking_in(&app);
    for (org, device) in [(ORG_A, "laptop-a"), (ORG_B, "laptop-b"), (ORG_C, "laptop-c")] {
        enrol(&devices, org, device).await;
    }

    bind(&devices, ORG_A, "laptop-a").await;
    assert_eq!(balance(&app, ORG_A).await, 5, "the shop's first account is credited");
    spend(&app, ORG_A, 0x71).await;
    unlink(&app, ORG_A).await;

    bind(&devices, ORG_B, "laptop-b").await;
    assert_eq!(
        balance(&app, ORG_B).await,
        0,
        "the same shop in a second account is credited nothing"
    );
    assert_eq!(open_flags(&app, ORG_A).await, ["shared_shop"]);
    assert_eq!(open_flags(&app, ORG_B).await, ["shared_shop"]);

    // The hole migration 0105 closes: erasure walks every org_id column, so
    // without the grant record a deleted account would free its shop's five.
    unlink(&app, ORG_B).await;
    erase(&app, ORG_A).await;
    erase(&app, ORG_B).await;
    bind(&devices, ORG_C, "laptop-c").await;
    assert_eq!(
        balance(&app, ORG_C).await,
        0,
        "deleting the accounts that held a shop does not credit it again"
    );
}

/// The quick unlink: free moves granted, some spent, unlinked within the
/// week. What is left of the five is taken back and the account flagged.
#[sqlx::test(migrations = "./migrations")]
async fn an_unlink_soon_after_spending_free_moves_zeroes_the_rest(app: PgPool) {
    seed_orgs(&app).await;
    let devices = checking_in(&app);
    enrol(&devices, ORG_A, "laptop-a").await;
    bind(&devices, ORG_A, "laptop-a").await;
    spend(&app, ORG_A, 0x71).await;
    spend(&app, ORG_A, 0x72).await;
    assert_eq!(balance(&app, ORG_A).await, 3);

    let connection = unlink(&app, ORG_A).await;
    let abuse = AbuseRepo::new(app.clone());
    assert!(abuse
        .quick_unlink(ORG_A, connection.0, NOW)
        .await
        .expect("the check runs"));
    assert_eq!(balance(&app, ORG_A).await, 0, "the three left are taken back");
    assert_eq!(open_flags(&app, ORG_A).await, ["quick_unlink"]);
    assert!(
        abuse
            .quick_unlink(ORG_A, connection.0, NOW)
            .await
            .expect("the second check runs"),
        "still a quick unlink"
    );
    assert_eq!(balance(&app, ORG_A).await, 0, "and nothing is taken twice");
}

/// Shared devices and cards are flagged once; a second pass raises nothing;
/// a dismissed sharing is not raised again until a new one begins.
#[sqlx::test(migrations = "./migrations")]
async fn the_scorer_flags_shared_values_once_and_respects_a_dismissal(app: PgPool) {
    seed_orgs(&app).await;
    let abuse = AbuseRepo::new(app.clone());
    let device = [0x44; 32];
    for org in [ORG_A, ORG_B] {
        abuse
            .record_signal(Signal {
                org,
                kind: SignalKind::DeviceFingerprint,
                value: &device,
                at: NOW,
            })
            .await
            .expect("the signal records");
    }
    // Browsers and email domains are context, never a link.
    for org in [ORG_A, ORG_B, ORG_C] {
        for kind in [SignalKind::UserAgentHash, SignalKind::EmailDomain] {
            abuse
                .record_signal(Signal {
                    org,
                    kind,
                    value: &[0x99; 32],
                    at: NOW,
                })
                .await
                .expect("the signal records");
        }
    }
    assert_eq!(open_flags(&app, ORG_A).await, ["shared_device"]);
    assert_eq!(open_flags(&app, ORG_B).await, ["shared_device"]);
    assert!(open_flags(&app, ORG_C).await.is_empty());
    assert_eq!(
        abuse.score(None, NOW).await.expect("the pass runs").raised,
        0,
        "the nightly pass over the same evidence raises nothing new"
    );

    let flag: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM abuse_flag WHERE org_id = $1")
            .bind(db_uuid(ORG_A.0))
            .fetch_one(&app)
            .await
            .expect("the flag reads");
    abuse
        .decide(Decision {
            flag: Uuid(*flag.as_bytes()),
            action: AbuseAction::None,
            reason: Some("Siblings sharing a laptop"),
            by: OPERATOR,
            at: LATER,
            identities: &[],
        })
        .await
        .expect("the dismissal runs")
        .expect("the flag exists");
    assert_eq!(abuse.score(None, LATER).await.expect("the pass runs").raised, 0);
    assert!(open_flags(&app, ORG_A).await.is_empty(), "dismissed stays dismissed");

    let backoffice = AbuseBackofficeRepo::new(app.clone());
    let rows = backoffice.flagged(true, None).await.expect("the list reads");
    let row = rows.iter().find(|row| row.org == ORG_A).expect("A is listed");
    assert_eq!(row.linked_orgs, 1, "only the device links A to anyone");
    assert_eq!(row.signal_kinds, [SignalKind::DeviceFingerprint]);
    assert_eq!(row.standing, AbuseAction::None);
}

/// Three accounts made from one address within a day are a burst; two are not.
#[sqlx::test(migrations = "./migrations")]
async fn three_sign_ups_from_one_address_in_a_day_are_flagged(app: PgPool) {
    seed_orgs(&app).await;
    let abuse = AbuseRepo::new(app.clone());
    let address = [0x1F; 32];
    for (org, at) in [(ORG_A, NOW), (ORG_B, LATER)] {
        abuse
            .record_signal(Signal {
                org,
                kind: SignalKind::Ip,
                value: &address,
                at,
            })
            .await
            .expect("the signal records");
    }
    assert!(
        !open_flags(&app, ORG_A).await.contains(&"signup_burst".to_owned()),
        "two sign-ups are not a burst"
    );
    abuse
        .record_signal(Signal {
            org: ORG_C,
            kind: SignalKind::Ip,
            value: &address,
            at: LATER,
        })
        .await
        .expect("the signal records");
    for org in [ORG_A, ORG_B, ORG_C] {
        assert!(
            open_flags(&app, org).await.contains(&"signup_burst".to_owned()),
            "every account in the burst is flagged"
        );
    }
}

/// A ban: sessions end, the session gate reads it, the shop is refused to
/// every account, the free moves go, and a dismissal lifts all of it.
#[sqlx::test(migrations = "./migrations")]
async fn a_ban_suspends_refuses_its_shop_and_lifts_cleanly(app: PgPool) {
    seed_orgs(&app).await;
    let devices = checking_in(&app);
    enrol(&devices, ORG_A, "laptop-a").await;
    enrol(&devices, ORG_B, "laptop-b").await;
    bind(&devices, ORG_A, "laptop-a").await;

    let sessions = SessionRepo::new(app.clone());
    let user = UserId(Uuid([0x0A; 16]));
    sessions
        .create_user(ORG_A, user, "a@subject.invalid", NOW)
        .await
        .expect("the user inserts");
    let token = SessionToken([0x42; 32]);
    sessions
        .mint(&token, user, Timestamp(NOW.0 + 3_600_000), NOW)
        .await
        .expect("the session mints");

    let abuse = AbuseRepo::new(app.clone());
    assert!(abuse
        .raise(ORG_A, FlagKind::SharedShop, "Shares a shop with 1 other account.", NOW)
        .await
        .expect("the flag raises"));
    let flag: uuid::Uuid = sqlx::query_scalar("SELECT id FROM abuse_flag WHERE org_id = $1")
        .bind(db_uuid(ORG_A.0))
        .fetch_one(&app)
        .await
        .expect("the flag reads");
    let flag = Uuid(*flag.as_bytes());
    let email = [0xE1; 32];
    abuse
        .decide(Decision {
            flag,
            action: AbuseAction::Ban,
            reason: Some("Ten accounts on one shop"),
            by: OPERATOR,
            at: NOW,
            identities: &[(BannedKind::Email, email)],
        })
        .await
        .expect("the ban runs")
        .expect("the flag exists");

    assert_eq!(abuse.standing(ORG_A).await.expect("standing reads"), AbuseAction::Ban);
    assert_eq!(
        sessions.resolve(&token, NOW).await.expect("the resolve runs"),
        None,
        "a ban ends every session the organisation held"
    );
    sessions
        .mint(&token, user, Timestamp(NOW.0 + 3_600_000), NOW)
        .await
        .expect("a session minted after the ban");
    assert!(
        sessions
            .resolve(&token, NOW)
            .await
            .expect("the resolve runs")
            .expect("the session exists")
            .suspended,
        "and the gate refuses any session minted after it"
    );
    assert!(abuse
        .is_banned(BannedKind::Email, &email, NOW)
        .await
        .expect("the ban reads"));
    assert_eq!(balance(&app, ORG_A).await, 0, "a ban takes the free moves back");

    unlink(&app, ORG_A).await;
    let refused = devices
        .heartbeat(ORG_B, "laptop-b", &[holding(STOREFRONT)], NOW)
        .await;
    assert!(
        matches!(
            refused,
            Err(StorageError::StorefrontSuspended {
                marketplace: Marketplace::Tpt
            })
        ),
        "a banned shop is refused to every account: {refused:?}"
    );

    abuse
        .decide(Decision {
            flag,
            action: AbuseAction::None,
            reason: None,
            by: OPERATOR,
            at: LATER,
            identities: &[],
        })
        .await
        .expect("the lift runs")
        .expect("the flag exists");
    assert_eq!(abuse.standing(ORG_A).await.expect("standing reads"), AbuseAction::None);
    assert!(!abuse
        .is_banned(BannedKind::Email, &email, NOW)
        .await
        .expect("the ban reads"));
    bind(&devices, ORG_B, "laptop-b").await;
}

/// A limit: no free moves for a new shop, no new connection.
#[sqlx::test(migrations = "./migrations")]
async fn a_limit_withholds_free_moves_and_new_connections(app: PgPool) {
    seed_orgs(&app).await;
    let abuse = AbuseRepo::new(app.clone());
    abuse
        .raise(ORG_A, FlagKind::SignupBurst, "One of 3 accounts.", NOW)
        .await
        .expect("the flag raises");
    let flag: uuid::Uuid = sqlx::query_scalar("SELECT id FROM abuse_flag WHERE org_id = $1")
        .bind(db_uuid(ORG_A.0))
        .fetch_one(&app)
        .await
        .expect("the flag reads");
    abuse
        .decide(Decision {
            flag: Uuid(*flag.as_bytes()),
            action: AbuseAction::Limit,
            reason: None,
            by: OPERATOR,
            at: NOW,
            identities: &[],
        })
        .await
        .expect("the limit runs")
        .expect("the flag exists");

    let devices = checking_in(&app);
    enrol(&devices, ORG_A, "laptop-a").await;
    devices
        .heartbeat(ORG_A, "laptop-a", &[holding(STOREFRONT)], NOW)
        .await
        .expect("the check-in runs")
        .expect("the device is registered");
    assert_eq!(balance(&app, ORG_A).await, 0, "no free moves");
    let mut tx = app.begin().await.expect("the read opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_A.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin sets");
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM connection WHERE state = 'linked'")
        .fetch_one(&mut *tx)
        .await
        .expect("the count reads");
    assert_eq!(linked, 0, "no new connection");
}
