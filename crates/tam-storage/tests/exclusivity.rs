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
use tam_storage::LeaseRepo;
use tam_types::{InventoryId, OrgId, Uuid};

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
