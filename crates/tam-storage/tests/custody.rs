//! The custody boundary: the application role cannot read the credential
//! vault, and the broker role can. This is the severity test behind the whole
//! milestone — a compromised API process holds no ciphertext to steal.

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
async fn the_app_role_cannot_read_the_vault_and_the_broker_can(app: PgPool) {
    let app_role = role_pool(&app, "tam_app", "tam_dev_password")
        .await
        .expect("the app role connects");
    let denied = sqlx::query("SELECT count(*) FROM connection_secret")
        .fetch_one(&app_role)
        .await;
    assert!(
        denied.is_err(),
        "the application role must not be able to read the credential vault"
    );

    let broker = role_pool(&app, "tam_broker", "tam_broker_dev")
        .await
        .expect("the broker role connects");
    let allowed: i64 = sqlx::query_scalar("SELECT count(*) FROM connection_secret")
        .fetch_one(&broker)
        .await
        .expect("the broker role may read the vault");
    assert_eq!(allowed, 0, "the empty vault reads zero for the broker");
}
