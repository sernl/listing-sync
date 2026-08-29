//! The two audit tables are append-only for the application role: insert and
//! select are granted, update and delete are revoked, and the revocation
//! binds the owner too because privileges are checked against the ACL, not
//! ownership. The actor columns are exercised here too, because an audit row
//! that cannot say who wrote it is the failure these tables exist to prevent.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{ConnectionAudit, ConnectionEventRecord, StorageError};
use tam_types::{
    Actor, ConnectionEvent, ConnectionId, OrgId, Stamp, SystemComponent, Timestamp, UserId, Uuid,
};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));

async fn seed_org_a(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}

/// A transaction with the tenant pin already applied.
///
/// Every step below needs its OWN transaction, which is the whole point of
/// this test's shape: a statement error aborts a Postgres transaction, and a
/// `COMMIT` issued on an aborted transaction rolls back and reports success.
/// Appending and then attempting the denied write in one transaction
/// therefore discards the append, leaving the immutability assertions to run
/// against an empty table and prove nothing.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn pinned(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    tx
}

#[sqlx::test(migrations = "./migrations")]
async fn the_audit_trail_cannot_be_rewritten(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");

    let mut tx = pinned(&pool).await;
    sqlx::query(
        "INSERT INTO field_audit \
         (org_id, mapping_id, field, intended, observed_before, observed_after, \
          class, normaliser_version, created_at, actor_kind, actor_id) \
         VALUES ($1, $2, 'title', 'a', NULL, 'a', 'normalised', 1, now(), \
                 'system', 'engine')",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x31; 16]))
    .execute(&mut *tx)
    .await
    .expect("the application role may append");
    tx.commit().await.expect("the append commits");

    // Read in a fresh transaction, so what is asserted from here on is a
    // durable row rather than one visible only to the writer.
    let mut tx = pinned(&pool).await;
    let landed: i64 = sqlx::query_scalar("SELECT count(*) FROM field_audit")
        .fetch_one(&mut *tx)
        .await
        .expect("the application role may read");
    assert_eq!(landed, 1, "the appended row survived its own commit");
    tx.commit().await.expect("the read commits");

    let mut tx = pinned(&pool).await;
    let rewritten = sqlx::query("UPDATE field_audit SET intended = 'b'")
        .execute(&mut *tx)
        .await;
    assert!(
        rewritten.is_err(),
        "update on field_audit must be denied to the application role"
    );
    drop(tx);

    let mut tx = pinned(&pool).await;
    let erased = sqlx::query("DELETE FROM field_audit")
        .execute(&mut *tx)
        .await;
    assert!(
        erased.is_err(),
        "delete on field_audit must be denied to the application role"
    );
    drop(tx);

    // The assertion the test is named for, and the one it never made: the
    // committed row is still there and still says what it said. Without this,
    // both refusals above would pass just as happily against an empty table.
    let mut tx = pinned(&pool).await;
    let surviving: Vec<(String, i64)> =
        sqlx::query_as("SELECT intended, count(*) OVER () FROM field_audit")
            .fetch_all(&mut *tx)
            .await
            .expect("the application role may read");
    assert_eq!(
        surviving,
        vec![("a".to_owned(), 1_i64)],
        "the row outlived both a rewrite and an erasure attempt, unchanged"
    );
}

const CONNECTION: ConnectionId = ConnectionId(Uuid([0xC0; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const T0: Timestamp = Timestamp(1_756_000_000_000);

#[sqlx::test(migrations = "./migrations")]
async fn the_lifecycle_audit_records_both_halves_of_the_actor(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    let audit = ConnectionAudit::new(pool.clone());

    audit
        .record(&ConnectionEventRecord {
            org: ORG_A,
            connection: CONNECTION,
            event: ConnectionEvent::Linked,
            detail: Some("tes"),
            stamp: Stamp {
                at: T0,
                actor: Actor::Person(USER),
            },
        })
        .await
        .expect("a seller-driven link records");
    audit
        .record(&ConnectionEventRecord {
            org: ORG_A,
            connection: CONNECTION,
            event: ConnectionEvent::NeedsReauth,
            detail: None,
            stamp: Stamp {
                at: Timestamp(T0.0 + 1),
                actor: Actor::System(SystemComponent::Engine),
            },
        })
        .await
        .expect("an engine gate records");

    let history = audit
        .history(ORG_A, CONNECTION)
        .await
        .expect("the history reads back");
    assert_eq!(history.len(), 2, "both events are kept");
    assert_eq!(
        (history[0].event.as_str(), history[0].actor_kind.as_str()),
        ("needs_reauth", "system"),
        "newest first, and the engine names itself"
    );
    assert_eq!(
        history[0].actor_id.as_deref(),
        Some("engine"),
        "an unattended write is never anonymous"
    );
    assert_eq!(
        (history[1].event.as_str(), history[1].actor_kind.as_str()),
        ("linked", "person"),
        "the seller-driven half is attributed to the seller"
    );
    assert_eq!(
        history[1].actor_id.as_deref(),
        Some("0a0a0a0a-0a0a-0a0a-0a0a-0a0a0a0a0a0a"),
        "the person is identified by the session's own user"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_lifecycle_audit_cannot_be_rewritten_or_erased(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    ConnectionAudit::new(pool.clone())
        .record(&ConnectionEventRecord {
            org: ORG_A,
            connection: CONNECTION,
            event: ConnectionEvent::Revoked,
            detail: None,
            stamp: Stamp {
                at: T0,
                actor: Actor::System(SystemComponent::Broker),
            },
        })
        .await
        .expect("the application role may append");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    assert!(
        sqlx::query("UPDATE connection_audit SET event = 'linked'")
            .execute(&mut *tx)
            .await
            .is_err(),
        "update on connection_audit must be denied to the application role"
    );
    tx.commit().await.ok();

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    assert!(
        sqlx::query("DELETE FROM connection_audit")
            .execute(&mut *tx)
            .await
            .is_err(),
        "delete on connection_audit must be denied to the application role"
    );
    drop(tx);

    // Same reason as the field_audit test: without reading the row back, both
    // refusals would pass against an empty table.
    let history = ConnectionAudit::new(pool.clone())
        .history(ORG_A, CONNECTION)
        .await
        .expect("the history reads back");
    assert_eq!(history.len(), 1, "the appended row outlived both attempts");
    assert_eq!(
        history[0].event.as_str(),
        "revoked",
        "and still says what it said"
    );
}

/// The closed set is the database's, not only the writer's: a verb the
/// migration does not name is refused even when the insert bypasses the
/// `ConnectionEvent` enum entirely.
#[sqlx::test(migrations = "./migrations")]
async fn an_unplanned_verb_is_refused_by_the_constraint(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let refused = sqlx::query(
        "INSERT INTO connection_audit \
         (org_id, connection_id, event, actor_kind, actor_id, at) \
         VALUES ($1, $2, 'exfiltrated', 'system', 'engine', now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(CONNECTION.0 .0))
    .execute(&mut *tx)
    .await;
    assert!(refused.is_err(), "the event set is closed at the database");
}

/// A person with no identifier is refused by the database, so the invariant
/// does not depend on every writer going through `Actor`.
#[sqlx::test(migrations = "./migrations")]
async fn an_anonymous_person_is_refused_by_the_constraint(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let refused = sqlx::query(
        "INSERT INTO connection_audit \
         (org_id, connection_id, event, actor_kind, actor_id, at) \
         VALUES ($1, $2, 'linked', 'person', NULL, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(CONNECTION.0 .0))
    .execute(&mut *tx)
    .await;
    assert!(
        refused.is_err(),
        "a person must be identified, whatever wrote the row"
    );
}

/// Migration 0033 backfilled the rows that predate attribution rather than
/// guessing at who wrote them, and dropped the defaults so a later insert
/// naming no actor fails instead of quietly inheriting one.
#[sqlx::test(migrations = "./migrations")]
async fn an_unattributed_insert_is_refused_after_the_backfill(pool: PgPool) {
    seed_org_a(&pool).await.expect("fixture org inserts");
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let refused = sqlx::query(
        "INSERT INTO field_audit \
         (org_id, mapping_id, field, class, normaliser_version, created_at) \
         VALUES ($1, $2, 'title', 'normalised', 1, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x31; 16]))
    .execute(&mut *tx)
    .await;
    assert!(
        matches!(refused, Err(sqlx::Error::Database(_))),
        "actor_kind is NOT NULL with no default, so an unattributed append fails"
    );
    let _unused: Option<StorageError> = None;
}
