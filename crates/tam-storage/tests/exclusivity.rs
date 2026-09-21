//! Global marketplace-account exclusivity, at the schema level.
//!
//! Every other uniqueness constraint in this schema is tenant-scoped, and
//! `connection_platform_account_exclusive` is the one deliberate exception:
//! its purpose is the crossing, because a constraint admitting the same
//! storefront once per tenant would let two organisations both drive one
//! seller's account and would enforce nothing at all.
//!
//! These are the two-tenant negative tests for that index, plus the reason a
//! half-completed link is safe: the lease scan requires `linked`, so a row
//! stranded in `linking` is inert rather than half-usable.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_secrets::Kek;
use tam_storage::{
    DeviceRegistration, DeviceRepo, DeviceSessionReport, DeviceSessionStatus, LeaseRepo,
    StorageError,
};
use tam_types::{InventoryId, Marketplace, OrgId, Timestamp, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const CONNECTION_A: Uuid = Uuid([0xC1; 16]);
const CONNECTION_B: Uuid = Uuid([0xC2; 16]);

/// Stands in for the HMAC a claim writer computes; these tests are about the
/// index rather than about the digest, so the bytes only need to be equal or
/// not.
const SHARED_DIGEST: [u8; 32] = [0x5A; 32];
const OTHER_DIGEST: [u8; 32] = [0x5B; 32];

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_orgs(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(db_uuid(org.0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org inserts");
    }
}

/// Inserts one connection with an explicit state and digest, under the tenant
/// pin that row belongs to.
///
/// Written as `tam_app`, which is the role that actually writes `connection`:
/// with the session broker gone, the device check-in's link derivation and the
/// revoke are the only writers left and both run here. The subject of these
/// tests is therefore the writer the schema has rather than one it used to
/// have.
///
/// The pin is per statement because `connection` carries FORCE ROW LEVEL
/// SECURITY and the policy admits only the pinned organisation, so two
/// tenants' rows cannot be written from one transaction. That is the property
/// under test rather than an inconvenience: the second insert cannot see the
/// row it collides with and the index refuses it anyway, which is exactly why
/// the constraint has to be a database index rather than a read-then-write
/// check in application code.
async fn insert_connection(
    app: &PgPool,
    org: OrgId,
    connection: Uuid,
    state: &str,
    digest: Option<&[u8]>,
) -> Result<(), sqlx::Error> {
    let mut tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(org.0).to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO connection \
         (org_id, id, marketplace, state, created_at, updated_at, platform_account_digest) \
         VALUES ($1, $2, 'tpt', $3, now(), now(), $4)",
    )
    .bind(db_uuid(org.0))
    .bind(db_uuid(connection))
    .bind(state)
    .bind(digest)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

#[sqlx::test(migrations = "./migrations")]
async fn one_marketplace_account_belongs_to_one_organisation_across_tenants(app: PgPool) {
    seed_orgs(&app).await;

    insert_connection(&app, ORG_A, CONNECTION_A, "linked", Some(&SHARED_DIGEST))
        .await
        .expect("the first organisation takes the account");

    let collision =
        insert_connection(&app, ORG_B, CONNECTION_B, "linked", Some(&SHARED_DIGEST)).await;
    let Err(sqlx::Error::Database(refusal)) = collision else {
        panic!("a second tenant claiming a held account must be refused by the database");
    };
    assert_eq!(
        refusal.constraint(),
        Some("connection_platform_account_exclusive"),
        "a claim writer maps this refusal by constraint name to a closed-set error code, \
         so the name is a contract rather than an implementation detail; nothing writes the \
         claim today, which makes this test what keeps the name stable until the \
         exclusivity-claim lift restores one"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn two_organisations_may_hold_different_accounts_on_the_same_marketplace(app: PgPool) {
    seed_orgs(&app).await;

    insert_connection(&app, ORG_A, CONNECTION_A, "linked", Some(&SHARED_DIGEST))
        .await
        .expect("the first organisation takes its account");
    insert_connection(&app, ORG_B, CONNECTION_B, "linked", Some(&OTHER_DIGEST))
        .await
        .expect(
            "exclusivity is per account, not per marketplace; two sellers must both be able \
             to link their own storefronts",
        );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_connection_without_a_digest_takes_no_lock(app: PgPool) {
    seed_orgs(&app).await;

    // This is the state a crash between the two link phases leaves, and the
    // state every Tes link stays in, since no Tes response names an account.
    insert_connection(&app, ORG_A, CONNECTION_A, "linking", None)
        .await
        .expect("a half-linked connection inserts");
    insert_connection(&app, ORG_B, CONNECTION_B, "linking", None)
        .await
        .expect(
            "the index is partial on a non-null digest, so unclaimed connections must not \
             collide with each other — otherwise the second tenant to begin a link would be \
             refused by the first tenant's unfinished one",
        );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_revoked_connection_releases_its_account(app: PgPool) {
    seed_orgs(&app).await;

    insert_connection(&app, ORG_A, CONNECTION_A, "revoked", Some(&SHARED_DIGEST))
        .await
        .expect("a revoked tombstone inserts");
    insert_connection(&app, ORG_B, CONNECTION_B, "linked", Some(&SHARED_DIGEST))
        .await
        .expect(
            "the index predicate names only the live states, so unlinking releases the account \
         and a seller who moves organisations is never permanently locked out",
        );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_half_linked_connection_is_not_leasable(app: PgPool) {
    seed_orgs(&app).await;
    insert_connection(&app, ORG_A, CONNECTION_A, "linking", None)
        .await
        .expect("a half-linked connection inserts");

    let leases = LeaseRepo::new(app.clone());
    let found = leases
        .connection_for(ORG_A, InventoryId::Tpt)
        .await
        .expect("the connection lookup runs");
    assert_eq!(
        found, None,
        "a crash between sealing the credential and claiming the account must leave the \
         connection inert: the lease scan requires 'linked', so no item is ever driven \
         against an account this organisation has not been shown to own"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_linked_connection_is_leasable_through_the_application_role(app: PgPool) {
    seed_orgs(&app).await;
    insert_connection(&app, ORG_A, CONNECTION_A, "linked", Some(&SHARED_DIGEST))
        .await
        .expect("the linked connection inserts");

    let found = LeaseRepo::new(app)
        .connection_for(ORG_A, InventoryId::Tpt)
        .await
        .expect("the connection lookup runs");
    assert_eq!(
        found.map(|connection| connection.0),
        Some(CONNECTION_A),
        "the device ledger runs on tam_app, so its tenant-scoped connection lookup must pin RLS \
         before reading the linked row"
    );
}

/// The storefront a device reports, as TPT spells one.
const STOREFRONT: &str = "900000001";

/// Any instant; nothing under test compares two.
const NOW: Timestamp = Timestamp(1_760_000_000_000);

/// The check-in's own claim writer, keyed like the deployment's.
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
                app_version: "0.10.25",
            },
            NOW,
        )
        .await
        .expect("the device registers");
}

/// One reported TPT session naming a storefront, connected.
fn holding(storefront: &str) -> DeviceSessionReport<'_> {
    DeviceSessionReport {
        marketplace: Marketplace::Tpt,
        account_label: None,
        external_id: Some(storefront),
        status: DeviceSessionStatus::Connected,
    }
}

/// The claim's only writer is the check-in, so this is the test that the
/// standing index refuses a second tenant through the path that actually
/// reaches it.
///
/// The digest is never supplied by the test: the repository derives it from
/// the deployment's key and the storefront the device reported, which is the
/// property that matters -- two organisations reporting the same shop produce
/// the same bytes without either ever seeing them.
#[sqlx::test(migrations = "./migrations")]
async fn a_second_organisation_checking_in_for_one_storefront_is_refused(app: PgPool) {
    seed_orgs(&app).await;
    let devices = checking_in(&app);
    enrol(&devices, ORG_A, "machine-a").await;
    enrol(&devices, ORG_B, "machine-b").await;

    devices
        .heartbeat(ORG_A, "machine-a", &[holding(STOREFRONT)], NOW)
        .await
        .expect("the first organisation's check-in binds the shop it holds")
        .expect("the device it named is registered");

    let refusal = devices
        .heartbeat(ORG_B, "machine-b", &[holding(STOREFRONT)], NOW)
        .await;
    assert!(
        matches!(
            refusal,
            Err(StorageError::StorefrontBoundElsewhere {
                marketplace: Marketplace::Tpt
            })
        ),
        "a second organisation reporting a storefront the first holds must be refused by \
         name, because the API layer owes that seller a sentence naming the marketplace \
         rather than a 500"
    );

    let linked = LeaseRepo::new(app)
        .connection_for(ORG_B, InventoryId::Tpt)
        .await
        .expect("the connection lookup runs");
    assert_eq!(
        linked, None,
        "the refused beat must link nothing at all: a connection left `linked` with its \
         claim refused is exactly the exclusivity hole the claim closes, so the link \
         derivation and the claim stand or fall together"
    );
}

/// Two sellers, two shops, one marketplace: the fence is per storefront.
#[sqlx::test(migrations = "./migrations")]
async fn two_organisations_checking_in_for_their_own_storefronts_both_bind(app: PgPool) {
    seed_orgs(&app).await;
    let devices = checking_in(&app);
    enrol(&devices, ORG_A, "machine-a").await;
    enrol(&devices, ORG_B, "machine-b").await;

    for (org, device, storefront) in [
        (ORG_A, "machine-a", STOREFRONT),
        (ORG_B, "machine-b", "90000001"),
    ] {
        devices
            .heartbeat(org, device, &[holding(storefront)], NOW)
            .await
            .expect("each seller's own shop binds")
            .expect("the device it named is registered");
    }

    let leases = LeaseRepo::new(app);
    for org in [ORG_A, ORG_B] {
        let found = leases
            .connection_for(org, InventoryId::Tpt)
            .await
            .expect("the connection lookup runs");
        assert!(
            found.is_some(),
            "exclusivity is per shop, not per marketplace: both sellers must be able to \
             work their own catalogues"
        );
    }
}

/// Signing out does not hand the free allowance back.
///
/// The index predicate releases the *lock* on an unlink, deliberately, so a
/// seller moving a shop between their own organisations is not stuck. The
/// allowance is a different fact: it is keyed on the storefront and written
/// once ever, so the five free moves cannot be minted again by disconnecting
/// and signing up afresh.
#[sqlx::test(migrations = "./migrations")]
async fn signing_out_does_not_return_the_free_allowance(app: PgPool) {
    seed_orgs(&app).await;
    let devices = checking_in(&app);
    enrol(&devices, ORG_A, "machine-a").await;
    devices
        .heartbeat(ORG_A, "machine-a", &[holding(STOREFRONT)], NOW)
        .await
        .expect("the check-in binds the shop")
        .expect("the device it named is registered");

    devices
        .heartbeat(
            ORG_A,
            "machine-a",
            &[DeviceSessionReport {
                status: DeviceSessionStatus::SignedOut,
                ..holding(STOREFRONT)
            }],
            NOW,
        )
        .await
        .expect("the sign-out check-in runs")
        .expect("the device it named is registered");

    // Pinned, because `storefront_allowance` carries FORCE ROW LEVEL
    // SECURITY: an unpinned count under `tam_app` matches nothing and would
    // pass this test by seeing no rows at all.
    let mut tx = app.begin().await.expect("the read transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_A.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let allowances: i64 =
        sqlx::query_scalar("SELECT count(*) FROM storefront_allowance WHERE marketplace = 'tpt'")
            .fetch_one(&mut *tx)
            .await
            .expect("the allowance count reads");
    assert_eq!(
        allowances, 1,
        "the allowance is keyed on the shop and written once ever, so a seller who signs \
         out has spent their free moves rather than parked them"
    );
}
