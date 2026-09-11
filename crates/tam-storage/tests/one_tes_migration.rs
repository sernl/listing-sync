//! The one-Tes migration refuses rather than merges.
//!
//! A seller holding one product on two Tes inventories has two live listings
//! on Tes, and rewriting both rows to `tes` would violate
//! `mapping_one_per_inventory` -- or, worse, silently drop a binding to a
//! resource that still exists on the marketplace. The migration raises and
//! names the organisation instead, which is the behaviour
//! `docs/design/decisions.md` ("Tes is one marketplace with no regions,
//! 2026-09-12") records.
//!
//! The schema this asserts against no longer exists in the repository's head
//! state, so the test builds it: every migration before `0068_one_tes` is
//! applied by hand, the colliding rows are seeded, and `0068` is then run as
//! text.

#![cfg(feature = "pg-tests")]

use sqlx::{Executor, PgPool};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const ONE_TES: &str = include_str!("../migrations/0068_one_tes.sql");

/// The version `0068_one_tes.sql` carries, and therefore the exclusive upper
/// bound of the schema the collision is seeded against.
const ONE_TES_VERSION: i64 = 68;

const ORG: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const PRODUCT: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

#[expect(
    clippy::panic,
    reason = "a migration that will not apply is a broken fixture, not a failed assertion"
)]
async fn schema_before_one_tes(pool: &PgPool) {
    for migration in MIGRATOR.iter().filter(|m| m.version < ONE_TES_VERSION) {
        pool.execute(&*migration.sql)
            .await
            .unwrap_or_else(|error| panic!("migration {} applies: {error}", migration.version));
    }
    assert!(
        MIGRATOR
            .iter()
            .any(|migration| migration.version == ONE_TES_VERSION),
        "0068_one_tes.sql is the migration under test, so it must still be numbered 68"
    );
}

#[sqlx::test(migrations = false)]
async fn the_one_tes_migration_refuses_an_org_holding_one_product_on_two_tes_inventories(
    pool: PgPool,
) {
    schema_before_one_tes(&pool).await;

    let mut connection = pool.acquire().await.expect("a connection");
    connection
        .as_mut()
        .execute(
            format!(
                "SELECT set_config('app.current_org', '{ORG}', false);
                 INSERT INTO organisation (id, name, created_at)
                     VALUES ('{ORG}', 'Two Tes listings', now());
                 INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version,
                                   first_seen_at)
                     VALUES ('{ORG}', '\\x51', 4, 'k', 1, now());
                 INSERT INTO product (org_id, id, title, body, body_format, price_kind,
                                      rights_state, created_at, updated_at)
                     VALUES ('{ORG}', '{PRODUCT}', 'Fractions Pack', 'body', 'markdown',
                             'free', 'unstated', now(), now());
                 INSERT INTO product_file (org_id, id, product_id, position, role, kind,
                                           hash, scan_state, created_at)
                     VALUES ('{ORG}', '{PRODUCT}', '{PRODUCT}', 0, 'payload', 'pdf',
                             '\\x51', 'pending', now());"
            )
            .as_str(),
        )
        .await
        .expect("the organisation, its product and its payload seed");

    for (id, inventory) in [('1', "tes_gb"), ('2', "tes_nz")] {
        connection
            .as_mut()
            .execute(
                format!(
                    "INSERT INTO mapping (org_id, id, product_id, inventory, marketplace,
                                          binding_state, verify_state, normaliser_version,
                                          policy_title, policy_description, policy_price,
                                          policy_taxonomy, policy_grades, policy_files,
                                          price_rule_kind, price_explicit_kind, publish_mode,
                                          lifecycle_state, created_at, updated_at)
                         VALUES ('{ORG}', 'cccccccc-cccc-4ccc-8ccc-cccccccccc0{id}',
                                 '{PRODUCT}', '{inventory}', 'tes', 'unbound', 'stale', 1,
                                 'managed', 'managed', 'managed', 'managed', 'managed',
                                 'managed', 'explicit', 'free', 'dry_run', 'absent',
                                 now(), now());"
                )
                .as_str(),
            )
            .await
            .unwrap_or_else(|error| panic!("the {inventory} mapping seeds: {error}"));
    }

    let refusal = connection
        .as_mut()
        .execute(ONE_TES)
        .await
        .expect_err("one product on two Tes inventories is a seller to talk to");
    let message = refusal.to_string();
    assert!(
        message.contains(ORG),
        "the refusal names the organisation an operator must reconcile, got {message:?}"
    );
    assert!(
        message.contains("two Tes inventories"),
        "the refusal says what is wrong rather than reporting a constraint, got {message:?}"
    );
}

#[sqlx::test(migrations = false)]
async fn the_one_tes_migration_rewrites_a_single_tes_mapping_and_retires_the_old_codes(
    pool: PgPool,
) {
    schema_before_one_tes(&pool).await;

    let mut connection = pool.acquire().await.expect("a connection");
    connection
        .as_mut()
        .execute(
            format!(
                "SELECT set_config('app.current_org', '{ORG}', false);
                 INSERT INTO organisation (id, name, created_at)
                     VALUES ('{ORG}', 'One Tes listing', now());
                 INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version,
                                   first_seen_at)
                     VALUES ('{ORG}', '\\x51', 4, 'k', 1, now());
                 INSERT INTO product (org_id, id, title, body, body_format, price_kind,
                                      rights_state, created_at, updated_at)
                     VALUES ('{ORG}', '{PRODUCT}', 'Fractions Pack', 'body', 'markdown',
                             'free', 'unstated', now(), now());
                 INSERT INTO product_file (org_id, id, product_id, position, role, kind,
                                           hash, scan_state, created_at)
                     VALUES ('{ORG}', '{PRODUCT}', '{PRODUCT}', 0, 'payload', 'pdf',
                             '\\x51', 'pending', now());
                 INSERT INTO mapping (org_id, id, product_id, inventory, marketplace,
                                      binding_state, verify_state, normaliser_version,
                                      policy_title, policy_description, policy_price,
                                      policy_taxonomy, policy_grades, policy_files,
                                      price_rule_kind, price_explicit_kind, publish_mode,
                                      lifecycle_state, created_at, updated_at)
                     VALUES ('{ORG}', 'cccccccc-cccc-4ccc-8ccc-cccccccccc01',
                             '{PRODUCT}', 'tes_gb', 'tes', 'unbound', 'stale', 1,
                             'managed', 'managed', 'managed', 'managed', 'managed',
                             'managed', 'explicit', 'free', 'dry_run', 'absent',
                             now(), now());"
            )
            .as_str(),
        )
        .await
        .expect("one Tes mapping seeds");

    connection
        .as_mut()
        .execute(ONE_TES)
        .await
        .expect("a seller with one Tes listing per product migrates");

    let inventory: String = sqlx::query_scalar("SELECT inventory FROM mapping")
        .fetch_one(connection.as_mut())
        .await
        .expect("the mapping survives");
    assert_eq!(
        inventory, "tes",
        "the binding moved rather than being dropped"
    );

    let codes: Vec<String> =
        sqlx::query_scalar("SELECT code FROM marketplace_inventory ORDER BY code")
            .fetch_all(connection.as_mut())
            .await
            .expect("the reference table reads");
    assert_eq!(
        codes,
        vec!["etsy".to_owned(), "tes".to_owned(), "tpt".to_owned()],
        "the three Tes codes retire to one"
    );
}
