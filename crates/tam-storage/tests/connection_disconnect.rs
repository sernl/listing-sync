//! The seller's disconnect against a real database, and the one fact that
//! decides its shape: `unlink` is reversible and `revoke` is not.
//!
//! Both verbs write the same table through the same repository, and nothing
//! in either statement says which one a check-in may lift — that is decided
//! by `derive_link`'s own guard list, three files away. So the pair is
//! asserted here together, driven through the device heartbeat the seller's
//! machine actually performs: unlink then reconnect on the same row, and
//! revoke then the same reconnect finding nothing to lift.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{
    ConnectionAudit, ConnectionRepo, DeviceRegistration, DeviceRepo, DeviceSessionReport,
    DeviceSessionStatus, StorageError,
};
use tam_types::{Actor, ConnectionId, Marketplace, OrgId, Stamp, Timestamp, UserId, Uuid};

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const OTHER: OrgId = OrgId(Uuid([0xBB; 16]));
const SELLER: UserId = UserId(Uuid([0x5E; 16]));
const LAPTOP: &str = "11112222333344445555666677778888";
const T0: Timestamp = Timestamp(1_756_000_000_000);
const T1: Timestamp = Timestamp(1_756_000_060_000);
const T2: Timestamp = Timestamp(1_756_000_120_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG, "org-a"), (OTHER, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the fixture org inserts");
    }
}

fn laptop() -> DeviceRegistration<'static> {
    DeviceRegistration {
        id: LAPTOP,
        name: "founder-pc",
        os: "windows",
        arch: "x86_64",
        app_version: "0.1.0",
    }
}

fn holding(status: DeviceSessionStatus) -> [DeviceSessionReport<'static>; 1] {
    [DeviceSessionReport {
        marketplace: Marketplace::Tpt,
        account_label: None,
        status,
    }]
}

fn seller_at(at: Timestamp) -> Stamp {
    Stamp {
        at,
        actor: Actor::Person(SELLER),
    }
}

/// Drives one machine's check-in, which is the only path that creates or
/// lifts a connection: the seller's device reporting a live TPT login.
async fn checks_in(
    devices: &DeviceRepo,
    status: DeviceSessionStatus,
    at: Timestamp,
) -> Result<(), StorageError> {
    devices.heartbeat(ORG, LAPTOP, &holding(status), at).await?;
    Ok(())
}

async fn state_of(connections: &ConnectionRepo, at: Timestamp) -> Result<String, StorageError> {
    let rows = connections.list(ORG, at).await?;
    Ok(rows
        .into_iter()
        .find(|row| row.marketplace == Marketplace::Tpt)
        .map_or_else(|| "absent".to_owned(), |row| row.state))
}

async fn connection_id(
    connections: &ConnectionRepo,
    at: Timestamp,
) -> Result<ConnectionId, StorageError> {
    let rows = connections.list(ORG, at).await?;
    #[expect(
        clippy::panic,
        reason = "a fixture with no connection is a broken fixture, not a failed assertion"
    )]
    let Some(row) = rows
        .into_iter()
        .find(|row| row.marketplace == Marketplace::Tpt)
    else {
        panic!("the fixture connection must exist");
    };
    Ok(row.id)
}

/// The pair, on one row, in the order a seller performs it. Two assertions
/// rather than one, because either verb alone would pass while the other had
/// silently become its twin: a terminal unlink and a reversible revoke are
/// both single-statement mistakes, and only the contrast catches them.
#[sqlx::test(migrations = "./migrations")]
async fn an_unlinked_connection_reconnects_and_a_revoked_one_never_does(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let devices = DeviceRepo::new(pool.clone());
    let connections = ConnectionRepo::new(pool.clone());
    devices.register(ORG, &laptop(), T0).await?;

    checks_in(&devices, DeviceSessionStatus::Connected, T0).await?;
    assert_eq!(state_of(&connections, T0).await?, "linked");
    let connection = connection_id(&connections, T0).await?;

    assert!(
        connections.unlink(ORG, connection, seller_at(T1)).await?,
        "the seller's disconnect moves a linked row"
    );
    assert_eq!(state_of(&connections, T1).await?, "unlinked");

    // The same check-in the machine was already making. Nothing reconnects the
    // row but this, which is the whole claim: the Connect button a seller
    // presses again is the button they pressed the first time.
    checks_in(&devices, DeviceSessionStatus::Connected, T2).await?;
    assert_eq!(
        state_of(&connections, T2).await?,
        "linked",
        "a disconnected marketplace comes back on the next check-in from a machine holding it"
    );

    assert!(
        connections.revoke(ORG, connection, seller_at(T2)).await?,
        "the operator's revoke moves a linked row"
    );
    checks_in(&devices, DeviceSessionStatus::Connected, T2).await?;
    assert_eq!(
        state_of(&connections, T2).await?,
        "revoked",
        "no check-in lifts a revoked connection, which is what makes the two verbs different"
    );
    Ok(())
}

/// A disconnect over a revocation would hand back the reconnect revocation
/// exists to withhold, so it finds nothing to move and says so.
#[sqlx::test(migrations = "./migrations")]
async fn a_disconnect_cannot_soften_a_revocation(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await;
    let devices = DeviceRepo::new(pool.clone());
    let connections = ConnectionRepo::new(pool.clone());
    devices.register(ORG, &laptop(), T0).await?;
    checks_in(&devices, DeviceSessionStatus::Connected, T0).await?;
    let connection = connection_id(&connections, T0).await?;

    assert!(connections.revoke(ORG, connection, seller_at(T1)).await?);
    assert!(
        !connections.unlink(ORG, connection, seller_at(T2)).await?,
        "a revoked row is not one a disconnect may move"
    );
    assert_eq!(state_of(&connections, T2).await?, "revoked");
    Ok(())
}

/// One audit row per transition rather than one per press, which is what lets
/// the API answer a count without reading the row back.
#[sqlx::test(migrations = "./migrations")]
async fn a_second_disconnect_writes_nothing(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await;
    let devices = DeviceRepo::new(pool.clone());
    let connections = ConnectionRepo::new(pool.clone());
    devices.register(ORG, &laptop(), T0).await?;
    checks_in(&devices, DeviceSessionStatus::Connected, T0).await?;
    let connection = connection_id(&connections, T0).await?;

    assert!(connections.unlink(ORG, connection, seller_at(T1)).await?);
    assert!(
        !connections.unlink(ORG, connection, seller_at(T2)).await?,
        "a second disconnect moves nothing"
    );

    let audit = ConnectionAudit::new(pool.clone())
        .history(ORG, connection)
        .await?;
    let unlinked: Vec<_> = audit.iter().filter(|row| row.event == "unlinked").collect();
    assert_eq!(
        unlinked.len(),
        1,
        "one transition, one audit row: {audit:?}"
    );
    assert_eq!(unlinked[0].actor_kind, "person");
    Ok(())
}

/// The tenant pin, asserted through the repository rather than raw SQL,
/// because the repository is the only way the API reaches this table. Another
/// tenant's connection is not visible and so is not distinguishable from one
/// that is not there: the answer is `false`, never a leak that it exists.
#[sqlx::test(migrations = "./migrations")]
async fn a_disconnect_cannot_reach_another_tenants_connection(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let devices = DeviceRepo::new(pool.clone());
    let connections = ConnectionRepo::new(pool.clone());
    devices.register(ORG, &laptop(), T0).await?;
    checks_in(&devices, DeviceSessionStatus::Connected, T0).await?;
    let connection = connection_id(&connections, T0).await?;

    assert!(
        !connections.unlink(OTHER, connection, seller_at(T1)).await?,
        "one seller cannot disconnect another seller's marketplace"
    );
    assert_eq!(state_of(&connections, T1).await?, "linked");
    Ok(())
}
