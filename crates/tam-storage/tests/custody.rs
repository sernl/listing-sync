//! The custody boundary: the application role cannot read the credential
//! vault. This is the severity test behind the whole milestone — a
//! compromised API process holds no ciphertext to steal.
//!
//! The vault's only reader is being retired. `tam_broker` is the one role that
//! could select from `connection_secret`, and D1 leaves no server-side seller
//! session for a no-API marketplace and so nothing to unseal a credential for.
//! A later migration revokes every privilege it holds; the role itself stays
//! an inert name in dev and CI, because three applied migrations grant to it,
//! and production drops it by an operator step. The column stays frozen,
//! holding the only copy of what was sealed before that, so the guarantee this
//! file asserts is unchanged and is becoming the whole of it: no role reaches
//! the vault, and the application role least of all.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

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

#[sqlx::test(migrations = "./migrations")]
async fn the_app_role_cannot_read_the_vault(app: PgPool) {
    let app_role = role_pool(&app, "tam_app", "tam_dev_password")
        .await
        .expect("the app role connects");
    let denied = sqlx::query("SELECT count(*) FROM connection_secret")
        .fetch_one(&app_role)
        .await;
    // The SQLSTATE rather than `is_err`, and that is the severity. A bare
    // error passes the day `connection_secret` is disposed of — which
    // `decisions.md` explicitly foresees as a later founder call — and would
    // pass equally on a renamed column or a malformed statement. 42501 is
    // insufficient_privilege: it says the relation is there and the grant is
    // not, which is the thing being asserted. It replaces the positive control
    // the broker role used to provide, and unlike that control it survives the
    // role's retirement.
    let error = denied.expect_err("the application role must not read the credential vault");
    let sqlx::Error::Database(refusal) = &error else {
        panic!("the refusal must come from Postgres, not from the client: {error:?}");
    };
    assert_eq!(
        refusal.code().as_deref(),
        Some("42501"),
        "the vault must be denied for want of a grant, not missing or malformed: {refusal:?}"
    );
}
