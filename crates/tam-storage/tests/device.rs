//! The device registry against a real database: the register-and-refresh
//! upsert, the heartbeat that replaces what a device holds, revocation and its
//! idempotence, and the two-tenant case that proves one seller cannot see or
//! sign out another seller's machine.
//!
//! The isolation test reads through the repository rather than with raw SQL,
//! because the repository is the only way the API reaches these tables and it
//! pins the tenant itself; a raw probe would prove something the request path
//! never does.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{
    DeviceRegistration, DeviceRepo, DeviceSessionReport, DeviceSessionStatus, StorageError,
};
use tam_types::{Marketplace, OrgId, Timestamp, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const LAPTOP: &str = "11112222333344445555666677778888";
const DESKTOP: &str = "99998888777766665555444433332222";
const T0: Timestamp = Timestamp(1_756_000_000_000);
const T1: Timestamp = Timestamp(1_756_000_060_000);
const T2: Timestamp = Timestamp(1_756_000_120_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
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

fn held(marketplace: Marketplace, label: Option<&str>) -> DeviceSessionReport<'_> {
    DeviceSessionReport {
        marketplace,
        account_label: label,
        external_id: None,
        status: DeviceSessionStatus::Connected,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_device_registers_once_and_a_second_registration_refreshes_it(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);

    let first = repo.register(ORG_A, &laptop(), T0).await?;
    assert_eq!(first.id, LAPTOP);
    assert_eq!(
        (first.first_seen_at, first.last_seen_at),
        (T0, T0),
        "a first run is both first and last seen at the same instant"
    );
    assert_eq!(first.revoked_at, None);
    assert!(first.sessions.is_empty(), "registering holds nothing yet");

    let renamed = DeviceRegistration {
        name: "studio-pc",
        app_version: "0.2.0",
        ..laptop()
    };
    let second = repo.register(ORG_A, &renamed, T1).await?;
    assert_eq!(
        second.first_seen_at, T0,
        "a renamed machine is the same device, so its first sighting stands"
    );
    assert_eq!(second.last_seen_at, T1);
    assert_eq!(second.name, "studio-pc");
    assert_eq!(second.app_version, "0.2.0");
    assert_eq!(
        repo.list(ORG_A).await?.len(),
        1,
        "re-registering must not mint a second row"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_heartbeat_replaces_what_the_device_holds(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);
    repo.register(ORG_A, &laptop(), T0).await?;

    let beat = repo
        .heartbeat(
            ORG_A,
            LAPTOP,
            &[
                held(Marketplace::Tpt, Some("Founder's Classroom")),
                held(Marketplace::Tes, None),
            ],
            T1,
        )
        .await?
        .expect("a registered device heartbeats");
    assert!(!beat.revoked(), "nothing has revoked this device");

    let device = repo.list(ORG_A).await?.remove(0);
    assert_eq!(device.last_seen_at, T1);
    assert_eq!(device.sessions.len(), 2);
    let tpt = device
        .sessions
        .iter()
        .find(|session| session.marketplace == Marketplace::Tpt)
        .expect("the TPT session is held");
    assert_eq!(tpt.account_label.as_deref(), Some("Founder's Classroom"));
    assert_eq!(
        (tpt.linked_at, tpt.last_used_at),
        (T1, T1),
        "a first report links and uses at the same instant"
    );

    // The second report names TPT alone, and says the seller signed out of it.
    let mut signed_out = held(Marketplace::Tpt, Some("Founder's Classroom"));
    signed_out.status = DeviceSessionStatus::SignedOut;
    repo.heartbeat(ORG_A, LAPTOP, &[signed_out], T2)
        .await?
        .expect("the device heartbeats again");

    let device = repo.list(ORG_A).await?.remove(0);
    assert_eq!(
        device.sessions.len(),
        1,
        "a marketplace the device no longer names is gone, not left standing stale"
    );
    let tpt = &device.sessions[0];
    assert_eq!(tpt.status, DeviceSessionStatus::SignedOut);
    assert_eq!(
        tpt.linked_at, T1,
        "when the session was first linked survives a status change"
    );
    assert_eq!(
        tpt.last_used_at, T1,
        "a session the device no longer holds was not used at the heartbeat instant"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_heartbeat_from_an_unregistered_device_records_nothing(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);
    assert_eq!(
        repo.heartbeat(ORG_A, LAPTOP, &[held(Marketplace::Tes, None)], T0)
            .await?,
        None,
        "a heartbeat is not a registration and must not create one"
    );
    assert!(repo.list(ORG_A).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn revocation_stands_across_a_re_registration_and_reaches_the_next_heartbeat(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);
    repo.register(ORG_A, &laptop(), T0).await?;
    repo.heartbeat(ORG_A, LAPTOP, &[held(Marketplace::Tpt, None)], T0)
        .await?
        .expect("the device heartbeats");

    assert_eq!(
        repo.revoke(ORG_A, LAPTOP, T1).await?,
        Some(T1),
        "signing the device out answers the instant it was signed out"
    );
    assert_eq!(
        repo.revoke(ORG_A, LAPTOP, T2).await?,
        Some(T1),
        "revoking twice keeps the first decision's instant rather than restamping"
    );
    assert_eq!(
        repo.revoke(ORG_A, DESKTOP, T2).await?,
        None,
        "a device nobody registered is not-found rather than a silent success"
    );

    let re_registered = repo.register(ORG_A, &laptop(), T2).await?;
    assert_eq!(
        re_registered.revoked_at,
        Some(T1),
        "a device must not be able to undo its own revocation by restarting"
    );

    let beat = repo
        .heartbeat(ORG_A, LAPTOP, &[held(Marketplace::Tpt, None)], T2)
        .await?
        .expect("a revoked device still gets an answer, and the answer is the revocation");
    assert_eq!(beat.revoked_at, Some(T1));
    assert!(beat.revoked());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn one_tenant_can_neither_see_nor_sign_out_another_tenants_device(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);

    repo.register(ORG_A, &laptop(), T0).await?;
    repo.heartbeat(
        ORG_A,
        LAPTOP,
        &[held(Marketplace::Tpt, Some("A's store"))],
        T0,
    )
    .await?
    .expect("A's laptop heartbeats");
    let b_device = DeviceRegistration {
        id: DESKTOP,
        name: "other-pc",
        ..laptop()
    };
    repo.register(ORG_B, &b_device, T0).await?;

    let a_devices = repo.list(ORG_A).await?;
    assert_eq!(
        a_devices.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        vec![LAPTOP],
        "one tenant's listing carries its own machines and no other's"
    );
    assert_eq!(
        repo.list(ORG_B)
            .await?
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        vec![DESKTOP]
    );
    assert!(
        repo.list(ORG_B).await?[0].sessions.is_empty(),
        "A's marketplace session must not appear under B's device"
    );

    assert_eq!(
        repo.revoke(ORG_B, LAPTOP, T1).await?,
        None,
        "B naming A's device id signs nothing out"
    );
    assert_eq!(
        repo.list(ORG_A).await?[0].revoked_at,
        None,
        "and A's device is untouched by the attempt"
    );
    assert_eq!(
        repo.heartbeat(ORG_B, LAPTOP, &[], T1).await?,
        None,
        "nor can B heartbeat as A's device and clear what it holds"
    );
    assert_eq!(
        repo.list(ORG_A).await?[0].sessions.len(),
        1,
        "A's session survives B's attempt to replace its set"
    );
    Ok(())
}

/// A restore changes what a device may do and says nothing about where it is.
///
/// The founder's report turns on this. He signed a machine out from his
/// phone's browser, signed it back in later, and the console said "checked in
/// just now" about a laptop that was shut in a bag — so he could not tell the
/// machine that had come back from the one that had not been switched on since.
/// `restore` stamped `last_seen_at = now`, which made authorisation look like
/// contact.
///
/// Both halves are asserted against one run: the restore leaves the instant
/// alone, and the check-in after it moves it. Either alone passes an
/// implementation that is wrong in the other direction — a restore that
/// stamped, or a heartbeat that had stopped stamping.
#[sqlx::test(migrations = "./migrations")]
async fn a_restore_clears_the_sign_out_without_claiming_the_device_is_present(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);
    repo.register(ORG_A, &laptop(), T0).await?;
    repo.revoke(ORG_A, LAPTOP, T1).await?;

    let restored = repo
        .restore(ORG_A, LAPTOP)
        .await?
        .expect("the device is this tenant's");
    assert_eq!(
        restored.revoked_at, None,
        "the explicit restore is the one act that clears the mark"
    );
    assert_eq!(
        restored.last_seen_at, T0,
        "and it leaves presence at the last instant the device actually spoke, which is before \
         the sign-out"
    );

    let beat = repo
        .heartbeat(ORG_A, LAPTOP, &[], T2)
        .await?
        .expect("the restored device is heartbeatable");
    assert_eq!(
        beat.revoked_at, None,
        "a restored device is answered as standing"
    );
    assert_eq!(
        repo.list(ORG_A).await?[0].last_seen_at,
        T2,
        "only contact advances presence, and this is the contact"
    );
    Ok(())
}

/// A revoked installation that re-registers stays revoked.
///
/// The upgrade path runs through `register`: a new build reports a new
/// `app_version` under the same installation id every launch. So this is both
/// the "an upgrade updates the same row" property and the "nothing but the
/// explicit restore lifts a sign-out" property, and they are the same SQL.
#[sqlx::test(migrations = "./migrations")]
async fn an_upgrade_updates_the_same_row_and_never_lifts_a_sign_out(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = DeviceRepo::new(pool);
    repo.register(ORG_A, &laptop(), T0).await?;
    repo.revoke(ORG_A, LAPTOP, T1).await?;

    let upgraded = DeviceRegistration {
        app_version: "0.9.1",
        ..laptop()
    };
    let after = repo.register(ORG_A, &upgraded, T2).await?;
    assert_eq!(
        repo.list(ORG_A).await?.len(),
        1,
        "an upgrade is the same installation, so it updates its row rather than adding one"
    );
    assert_eq!(after.app_version, "0.9.1", "and the row learns the version");
    assert_eq!(
        after.revoked_at,
        Some(T1),
        "while the sign-out stands: a device that could lift its own revocation by \
         reinstalling would make the seller's decision a delay"
    );
    assert_eq!(
        after.first_seen_at, T0,
        "and the installation's own history is kept"
    );
    Ok(())
}
