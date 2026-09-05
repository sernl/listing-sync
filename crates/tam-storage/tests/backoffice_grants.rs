//! The backoffice boundary, probed rather than trusted: the operator role
//! reads across tenants, and everything else it might reach is denied.
//!
//! This is the counterpart to `custody.rs`. That test asserts the vault is out
//! of the application's reach; this one asserts the same vault is out of the
//! reach of the role invented to read everything else, and that a role able to
//! read every tenant's ledger cannot write one row of it.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_types::{OrgId, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));

async fn role_pool(app: &PgPool, role: &str, password: &str) -> Result<PgPool, sqlx::Error> {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://{role}:{password}@127.0.0.1:5433/{database}"
        ))
        .await
}

async fn backoffice_pool(app: &PgPool) -> Result<PgPool, sqlx::Error> {
    role_pool(app, "tam_backoffice", "tam_backoffice_dev").await
}

/// One product under each of two organisations, written through the pin the
/// application path uses, so the rows exist exactly as a tenant's own writes
/// leave them.
async fn seed_two_tenants(app: &PgPool) -> Result<(), sqlx::Error> {
    for (org, name, mark) in [(ORG_A, "org-a", 0x11u8), (ORG_B, "org-b", 0x12)] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(app)
            .await?;
        // One transaction for the whole product: the catalogue's payload
        // assertion is a deferred constraint trigger, so a product without the
        // file that satisfies it is refused at commit rather than at insert.
        let mut tx = app.begin().await?;
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
            .execute(&mut *tx)
            .await?;
        let org_uuid = uuid::Uuid::from_bytes(org.0 .0);
        let product = uuid::Uuid::from_bytes([mark; 16]);
        let file = uuid::Uuid::from_bytes([mark.wrapping_add(0x70); 16]);
        let hash = vec![mark];
        sqlx::query(
            "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
             VALUES ($1, $2, 4, 'fixture', 1, now())",
        )
        .bind(org_uuid)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO product \
             (org_id, id, title, body, body_format, price_kind, rights_state, \
              created_at, updated_at) \
             VALUES ($1, $2, 'fixture', 'body', 'markdown', 'free', 'unstated', now(), now())",
        )
        .bind(org_uuid)
        .bind(product)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO product_file \
             (org_id, id, product_id, position, role, kind, hash, scan_state, created_at) \
             VALUES ($1, $2, $3, 0, 'payload', 'pdf', $4, 'pending', now())",
        )
        .bind(org_uuid)
        .bind(file)
        .bind(product)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
        // One subscription per tenant, so the read policy migration 0039 adds
        // is asserted by rows rather than by an empty count: row-level
        // security filters rows and does not deny the statement, so a table
        // holding nothing answers zero whether or not the policy is there.
        sqlx::query(
            "INSERT INTO billing_subscription \
             (org_id, paddle_subscription_id, paddle_customer_id, status, \
              current_period_end, occurred_at, updated_at) \
             VALUES ($1, $2, $3, 'active', now(), now(), now())",
        )
        .bind(org_uuid)
        .bind(format!("sub_{name}"))
        .bind(format!("ctm_{name}"))
        .execute(&mut *tx)
        .await?;
        // One request per tenant, for the reason the subscription above gives:
        // row-level security filters rows rather than denying the statement, so
        // an empty table answers zero whether or not the read policy is there.
        // The author is seeded with it, because `requested_by` references
        // `app_user`.
        sqlx::query(
            "INSERT INTO app_user (id, org_id, email, created_at) \
             VALUES ($1, $2, $3, now())",
        )
        .bind(uuid::Uuid::from_bytes([mark.wrapping_add(0x30); 16]))
        .bind(org_uuid)
        .bind(format!("{name}@example.test"))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO marketplace_request \
             (org_id, id, requested_by, name, url, reason, created_at) \
             VALUES ($1, $2, $3, 'Somewhere', 'https://somewhere.test', 'why', now())",
        )
        .bind(org_uuid)
        .bind(uuid::Uuid::from_bytes([mark.wrapping_add(0x40); 16]))
        .bind(uuid::Uuid::from_bytes([mark.wrapping_add(0x30); 16]))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_backoffice_role_reads_across_tenants_and_the_app_role_does_not(app: PgPool) {
    seed_two_tenants(&app).await.expect("the tenants seed");

    let unpinned: i64 = sqlx::query_scalar("SELECT count(*) FROM product")
        .fetch_one(&app)
        .await
        .expect("the application role may run the query");
    assert_eq!(
        unpinned, 0,
        "the application role with no tenant pinned must see no product at all; \
         without this contrast the backoffice count below proves nothing"
    );

    let backoffice = backoffice_pool(&app).await.expect("the role connects");
    let across: i64 = sqlx::query_scalar("SELECT count(*) FROM product")
        .fetch_one(&backoffice)
        .await
        .expect("the backoffice role may read the catalogue");
    assert_eq!(
        across, 2,
        "the backoffice role reads both tenants' products in one unpinned query"
    );

    let unpinned_subscriptions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM billing_subscription")
            .fetch_one(&app)
            .await
            .expect("the application role may run the query");
    assert_eq!(
        unpinned_subscriptions, 0,
        "the application role with no tenant pinned must see no subscription; \
         without this contrast the backoffice count below proves nothing"
    );
    let subscriptions: i64 = sqlx::query_scalar("SELECT count(*) FROM billing_subscription")
        .fetch_one(&backoffice)
        .await
        .expect("the backoffice role may read billing state");
    assert_eq!(
        subscriptions, 2,
        "migration 0039's grant and read policy together let this role see \
         both tenants' subscriptions past the fence migration 0038 raised"
    );

    let unpinned_requests: i64 = sqlx::query_scalar("SELECT count(*) FROM marketplace_request")
        .fetch_one(&app)
        .await
        .expect("the application role may run the query");
    assert_eq!(
        unpinned_requests, 0,
        "the application role with no tenant pinned must see no marketplace \
         request; without this contrast the backoffice count below proves nothing"
    );
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM marketplace_request")
        .fetch_one(&backoffice)
        .await
        .expect("the backoffice role may read what sellers have asked for");
    assert_eq!(
        requests, 2,
        "migration 0055's grant and read policy together let this role see \
         both tenants' requests, which is the whole point of the table"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_backoffice_role_cannot_reach_the_credential_vault(app: PgPool) {
    let backoffice = backoffice_pool(&app).await.expect("the role connects");
    for statement in [
        "SELECT count(*) FROM connection_secret",
        "SELECT ciphertext FROM connection_secret",
        "SELECT wrapped_dek FROM connection_secret",
    ] {
        let denied = sqlx::query(statement).fetch_one(&backoffice).await;
        assert!(
            denied.is_err(),
            "the backoffice role must not reach the credential vault: {statement}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_backoffice_role_cannot_write_a_table_it_reads(app: PgPool) {
    seed_two_tenants(&app).await.expect("the tenants seed");
    let backoffice = backoffice_pool(&app).await.expect("the role connects");
    for statement in [
        "INSERT INTO product (org_id, id, title, body, body_format, price_kind, \
             rights_state, created_at, updated_at) \
         VALUES (gen_random_uuid(), gen_random_uuid(), 'x', 'y', 'markdown', 'free', \
             'unstated', now(), now())",
        "UPDATE product SET title = 'rewritten'",
        "DELETE FROM product",
        "UPDATE job_item SET state = 'settled'",
        "DELETE FROM write_attempt",
        "UPDATE billing_subscription SET status = 'active'",
        "DELETE FROM billing_subscription",
        // The one table on this list any tenant can append to, which is what
        // makes a read policy widened to FOR ALL worth catching here: an
        // operator pool able to write it could rewrite what a seller asked
        // for, in a table whose whole purpose is to carry a seller's words to
        // a person.
        "UPDATE marketplace_request SET name = 'rewritten'",
        "DELETE FROM marketplace_request",
        "INSERT INTO marketplace_request \
             (org_id, id, requested_by, name, url, reason, created_at) \
         VALUES (gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), 'x', \
             'https://x.test', 'y', now())",
        "INSERT INTO org_halt (org_id, raised_by, reason, raised_at) \
         VALUES (gen_random_uuid(), 'nobody', 'because', now())",
    ] {
        let denied = sqlx::query(statement).execute(&backoffice).await;
        assert!(
            denied.is_err(),
            "the backoffice role is granted SELECT and nothing else: {statement}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_backoffice_role_sees_only_the_tables_it_was_granted(app: PgPool) {
    let backoffice = backoffice_pool(&app).await.expect("the role connects");
    // Fenced tables absent from migration 0037's grant list, and the operator
    // marking itself. The marking matters most: the pool that reads every
    // tenant cannot read, and therefore cannot forge, the list of who is
    // allowed to use it. The identity schema is deliberately not probed here
    // -- a throwaway test database carries only this crate's migrations, so
    // its absence would pass for a denial and prove nothing.
    for table in [
        "election_item",
        "connection_audit",
        "field_audit",
        "blob",
        "platform_operator",
    ] {
        let denied = sqlx::query(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&backoffice)
            .await;
        assert!(
            denied.is_err(),
            "the backoffice role must not read {table}, which nothing granted it"
        );
    }

    for table in [
        "product",
        "mapping",
        "connection",
        "job",
        "job_item",
        "write_attempt",
        "org_halt",
        "org_inventory_halt",
        "organisation",
        "app_user",
        "billing_subscription",
        "marketplace_request",
    ] {
        let allowed = sqlx::query(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&backoffice)
            .await;
        assert!(
            allowed.is_ok(),
            "the grant list must name {table}, and the read policy beside it; \
             one without the other shows this role nothing"
        );
    }
}
