//! The consent record against a real database: a grant stands on the version
//! it was read on and no other, a withdrawal ends it, a repeated grant is one
//! row, and a grant on a newer notice retires the older one.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{ConsentRepo, StorageError};
use tam_types::{Marketplace, OrgId, Timestamp, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const FOUNDER: Uuid = Uuid([0x01; 16]);
const T0: Timestamp = Timestamp(1_756_000_000_000);
const T1: Timestamp = Timestamp(1_756_000_060_000);
const T2: Timestamp = Timestamp(1_756_000_120_000);
const CURRENT: &str = "2026-09-14";
const LATER: &str = "2027-01-01";

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

#[sqlx::test(migrations = "./migrations")]
async fn a_grant_stands_on_its_own_version_and_no_other(pool: PgPool) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = ConsentRepo::new(pool);

    assert_eq!(
        repo.standing(ORG_A, Marketplace::Tpt, CURRENT).await?,
        None,
        "nothing stands before a grant"
    );
    let granted = repo
        .grant(ORG_A, Marketplace::Tpt, CURRENT, FOUNDER, T0)
        .await?;
    assert_eq!(granted.notice_version, CURRENT);
    assert_eq!(granted.granted_by, FOUNDER);
    assert_eq!(granted.granted_at, T0);
    assert_eq!(granted.withdrawn_at, None);

    assert_eq!(
        repo.standing(ORG_A, Marketplace::Tpt, CURRENT).await?,
        Some(granted),
        "the grant stands on the version it was read on"
    );
    assert_eq!(
        repo.standing(ORG_A, Marketplace::Tpt, LATER).await?,
        None,
        "a newer notice makes the old grant stop standing"
    );
    assert_eq!(
        repo.standing(ORG_A, Marketplace::Tes, CURRENT).await?,
        None,
        "a grant is per marketplace"
    );
    assert_eq!(
        repo.standing(ORG_B, Marketplace::Tpt, CURRENT).await?,
        None,
        "a grant is per organisation"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_withdrawal_ends_the_grant_and_stays_in_the_history(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = ConsentRepo::new(pool);
    repo.grant(ORG_A, Marketplace::Tpt, CURRENT, FOUNDER, T0)
        .await?;

    let withdrawn = repo
        .withdraw(ORG_A, Marketplace::Tpt, FOUNDER, T1)
        .await?
        .ok_or_else(|| StorageError::Inconsistent {
            reason: "the grant just made withdraws".to_owned(),
        })?;
    assert_eq!(withdrawn.withdrawn_at, Some(T1));
    assert_eq!(
        repo.standing(ORG_A, Marketplace::Tpt, CURRENT).await?,
        None,
        "a withdrawn grant does not stand"
    );
    assert_eq!(
        repo.withdraw(ORG_A, Marketplace::Tpt, FOUNDER, T2).await?,
        None,
        "a second withdrawal finds nothing standing and is not a fault"
    );
    let history = repo.history(ORG_A).await?;
    assert_eq!(history.len(), 1, "the withdrawn row stays in the record");
    assert_eq!(history[0].withdrawn_at, Some(T1));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_repeated_grant_is_one_row_and_a_newer_notice_retires_the_older(
    pool: PgPool,
) -> Result<(), StorageError> {
    provision(&pool).await;
    let repo = ConsentRepo::new(pool);
    let first = repo
        .grant(ORG_A, Marketplace::Tpt, CURRENT, FOUNDER, T0)
        .await?;
    let again = repo
        .grant(ORG_A, Marketplace::Tpt, CURRENT, FOUNDER, T1)
        .await?;
    assert_eq!(
        again, first,
        "the same version is answered with the standing row"
    );
    assert_eq!(repo.history(ORG_A).await?.len(), 1);

    let newer = repo
        .grant(ORG_A, Marketplace::Tpt, LATER, FOUNDER, T2)
        .await?;
    assert_eq!(newer.notice_version, LATER);
    let history = repo.history(ORG_A).await?;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], newer, "newest first");
    assert_eq!(
        history[1].withdrawn_at,
        Some(T2),
        "the older grant is withdrawn at the instant the newer one was made"
    );
    assert_eq!(repo.standing(ORG_A, Marketplace::Tpt, CURRENT).await?, None);
    assert_eq!(
        repo.standing(ORG_A, Marketplace::Tpt, LATER).await?,
        Some(newer)
    );
    Ok(())
}
