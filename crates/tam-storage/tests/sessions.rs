//! The session floor: the digest is the verifier, expiry is absolute, and an
//! unknown token is indistinguishable from an expired one.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{Timestamp, UserId, Uuid};

use common::{seed_org_a, ORG_A};

const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x42; 32]);

#[sqlx::test(migrations = "./migrations")]
async fn a_minted_session_resolves_until_it_expires(pool: PgPool) {
    seed_org_a(&pool).await.expect("org-a seeds");
    let repo = SessionRepo::new(pool);
    repo.create_user(ORG_A, USER, "founder@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    repo.mint(&TOKEN, USER, Timestamp(10_000), Timestamp(1_000))
        .await
        .expect("the session mints");

    let live = repo
        .resolve(&TOKEN, Timestamp(9_999))
        .await
        .expect("the resolve runs")
        .expect("the live session resolves");
    assert_eq!(
        (live.org, live.user),
        (ORG_A, USER),
        "the session speaks for the user and their organisation"
    );
    assert_eq!(
        repo.resolve(&TOKEN, Timestamp(10_000))
            .await
            .expect("the resolve runs"),
        None,
        "expiry is absolute: at the boundary instant the session is gone"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_token_and_a_deleted_one_read_the_same(pool: PgPool) {
    seed_org_a(&pool).await.expect("org-a seeds");
    let repo = SessionRepo::new(pool);
    repo.create_user(ORG_A, USER, "founder@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    assert_eq!(
        repo.resolve(&TOKEN, Timestamp(1_000))
            .await
            .expect("the resolve runs"),
        None,
        "a token never minted resolves to nothing"
    );
    repo.mint(&TOKEN, USER, Timestamp(10_000), Timestamp(1_000))
        .await
        .expect("the session mints");
    assert!(
        repo.expire(&TOKEN).await.expect("the expire runs"),
        "the session deletes"
    );
    assert_eq!(
        repo.resolve(&TOKEN, Timestamp(1_001))
            .await
            .expect("the resolve runs"),
        None,
        "a deleted session is indistinguishable from one never minted"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_session_cannot_hang_off_a_missing_user(pool: PgPool) {
    seed_org_a(&pool).await.expect("org-a seeds");
    let repo = SessionRepo::new(pool);
    let refused = repo
        .mint(&TOKEN, USER, Timestamp(10_000), Timestamp(1_000))
        .await;
    assert!(
        refused.is_err(),
        "minting for an unprovisioned user is refused, not silently empty"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_mint_path_finds_an_existing_email(pool: PgPool) {
    seed_org_a(&pool).await.expect("org-a seeds");
    let repo = SessionRepo::new(pool);
    assert_eq!(
        repo.user_by_email("founder@example.test")
            .await
            .expect("the lookup runs"),
        None,
        "an unprovisioned email has no user"
    );
    repo.create_user(ORG_A, USER, "founder@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    assert_eq!(
        repo.user_by_email("founder@example.test")
            .await
            .expect("the lookup runs"),
        Some(USER),
        "re-minting for one email finds the existing user"
    );
}
