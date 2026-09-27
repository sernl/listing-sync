//! The site-wide switches against a real database: an unset key reads as
//! absent, a write replaces the previous value, a prefix lists only its own
//! keys, and a delete is idempotent.

#![cfg(feature = "pg-tests")]

use serde_json::json;
use sqlx::PgPool;
use tam_storage::{SiteSettingRepo, StorageError};
use tam_types::{Timestamp, UserId, Uuid};

const OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const T0: Timestamp = Timestamp(1_756_000_000_000);
const T1: Timestamp = Timestamp(1_756_000_060_000);

async fn provision(pool: &PgPool) -> Result<(), sqlx::Error> {
    let org = uuid::Uuid::from_bytes([0xA0; 16]);
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(org)
        .bind("site setting operator")
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO app_user (id, org_id, email, created_at) VALUES ($1, $2, $3, now())")
        .bind(uuid::Uuid::from_bytes(OPERATOR.0 .0))
        .bind(org)
        .bind("site-operator@example.test")
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_write_replaces_and_a_read_answers_the_latest(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await?;
    let repo = SiteSettingRepo::new(pool.clone());

    assert_eq!(repo.get("maintenance").await?, None, "nothing is set on a fresh database");

    repo.set("maintenance", &json!({"on": true, "message": null}), OPERATOR, T0)
        .await?;
    repo.set("maintenance", &json!({"on": false, "message": "back"}), OPERATOR, T1)
        .await?;
    assert_eq!(
        repo.get("maintenance").await?,
        Some(json!({"on": false, "message": "back"})),
        "the second write replaced the first rather than adding a row"
    );

    let stamped: (i64,) = sqlx::query_as(
        "SELECT (extract(epoch FROM updated_at) * 1000)::bigint FROM site_setting WHERE key = 'maintenance'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stamped.0, T1.0, "the row records when it was last written");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn many_and_prefix_answer_only_the_keys_asked_for(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await?;
    let repo = SiteSettingRepo::new(pool);
    repo.set("theme", &json!({"name": "halloween"}), OPERATOR, T0).await?;
    repo.set("sale.a", &json!(1), OPERATOR, T0).await?;
    repo.set("sale.b", &json!(2), OPERATOR, T0).await?;
    repo.set("sales_report", &json!(3), OPERATOR, T0).await?;

    assert_eq!(
        repo.many(&["banner", "theme", "maintenance"]).await?,
        vec![("theme".to_owned(), json!({"name": "halloween"}))],
        "an unset key is absent, and a key not asked for is not answered"
    );
    assert_eq!(
        repo.with_prefix("sale.").await?,
        vec![("sale.a".to_owned(), json!(1)), ("sale.b".to_owned(), json!(2))],
        "the prefix is literal, so `sales_report` is not a sale"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_delete_happens_once(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await?;
    let repo = SiteSettingRepo::new(pool);
    repo.set("banner", &json!({"text": "Hi", "href": "/"}), OPERATOR, T0)
        .await?;
    assert!(repo.delete("banner").await?, "the first delete removes the row");
    assert!(!repo.delete("banner").await?, "the second finds nothing to remove");
    assert_eq!(repo.get("banner").await?, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_key_must_be_dotted_lowercase_words(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await?;
    let repo = SiteSettingRepo::new(pool);
    for key in ["", "Theme", "sale..a", ".sale", "has space"] {
        assert!(
            repo.set(key, &json!(true), OPERATOR, T0).await.is_err(),
            "{key:?} is refused by the key's shape check"
        );
    }
    Ok(())
}
