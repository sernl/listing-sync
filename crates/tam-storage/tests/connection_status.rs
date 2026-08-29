//! The four-state status as the API actually gets it: derived from a real
//! row read back through the repository, not from a struct built in a test.
//!
//! The derivation itself is unit-tested in `tam-types`. What this covers is
//! the wiring the unit tests cannot see — that `list` selects the columns the
//! derivation reads, hands them over in the right order, and uses the caller's
//! instant rather than one of its own.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::ConnectionRepo;
use tam_types::{ConnectionStatus, OrgId, Timestamp, Uuid, VERIFICATION_FRESHNESS_MS};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const CONNECTION: Uuid = Uuid([0xC0; 16]);
const NOW: Timestamp = Timestamp(1_756_000_000_000);

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(db_uuid(ORG.0))
        .execute(pool)
        .await
        .expect("the fixture org inserts");
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection \
         (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(CONNECTION))
    .execute(&mut *tx)
    .await
    .expect("the fixture connection inserts");
    tx.commit().await.expect("the fixture commits");
}

/// Drives one row through every status and reads each back through `list`.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn set(pool: &PgPool, columns: &str) {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(&format!(
        "UPDATE connection SET {columns} WHERE org_id = $1 AND id = $2"
    ))
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(CONNECTION))
    .execute(&mut *tx)
    .await
    .expect("the row updates");
    tx.commit().await.expect("the update commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn status(pool: &PgPool) -> ConnectionStatus {
    let rows = ConnectionRepo::new(pool.clone())
        .list(ORG, NOW)
        .await
        .expect("the connections read");
    assert_eq!(rows.len(), 1, "the fixture holds exactly one connection");
    rows[0].status
}

fn at(offset_ms: i64) -> String {
    format!("to_timestamp({}::bigint / 1000.0)", NOW.0 + offset_ms)
}

#[sqlx::test(migrations = "./migrations")]
async fn the_status_follows_the_row_through_every_value(pool: PgPool) {
    seed(&pool).await;

    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Checking,
        "linked and never verified is not yet a working connection"
    );

    set(
        &pool,
        &format!(
            "session_verified_at = {}, session_refresh_after = {}",
            at(-60_000),
            at(60_000)
        ),
    )
    .await;
    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Connected,
        "a recent verification with nothing due is the only connected case"
    );

    set(&pool, "refresh_failures = 2").await;
    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Unstable,
        "a failure since the last success outranks that success"
    );

    set(&pool, "refresh_failures = 0").await;
    set(&pool, &format!("session_refresh_after = {}", at(-1))).await;
    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Checking,
        "a verification past due is in flight, not proof"
    );

    set(
        &pool,
        &format!(
            "session_verified_at = {}, session_refresh_after = {}",
            at(-VERIFICATION_FRESHNESS_MS - 1),
            at(60_000)
        ),
    )
    .await;
    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Checking,
        "a success older than the window is no longer evidence"
    );

    set(&pool, "state = 'needs_reauth'").await;
    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Disconnected,
        "the engine's gate is what the seller must act on"
    );

    set(&pool, "state = 'revoked'").await;
    assert_eq!(
        status(&pool).await,
        ConnectionStatus::Disconnected,
        "a tombstoned credential is nothing to verify"
    );
}

/// The instant is the caller's. A row that reads `connected` now reads
/// `checking` when asked about a moment past its freshness window, which is
/// what proves `list` does not consult a clock of its own.
#[sqlx::test(migrations = "./migrations")]
async fn the_status_is_answered_against_the_instant_it_was_asked_about(pool: PgPool) {
    seed(&pool).await;
    set(
        &pool,
        &format!(
            "session_verified_at = {}, session_refresh_after = {}",
            at(-60_000),
            at(VERIFICATION_FRESHNESS_MS * 4)
        ),
    )
    .await;
    assert_eq!(status(&pool).await, ConnectionStatus::Connected);

    let later = Timestamp(NOW.0 + VERIFICATION_FRESHNESS_MS + 60_001);
    let rows = ConnectionRepo::new(pool.clone())
        .list(ORG, later)
        .await
        .expect("the connections read");
    assert_eq!(
        rows[0].status,
        ConnectionStatus::Checking,
        "the same row, asked about a later instant, is no longer proven"
    );
}
