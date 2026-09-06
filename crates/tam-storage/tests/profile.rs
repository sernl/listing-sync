//! The seller's own account picture: one user's row, one organisation's
//! blobs, and nothing reachable across either line.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{AvatarWrite, BlobRepo, ProfileRepo, SessionRepo};
use tam_types::{ContentHash, OrgId, Timestamp, UserId, Uuid};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool) {
    for (org, user, name) in [(ORG_A, USER_A, "org-a"), (ORG_B, USER_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
        SessionRepo::new(pool.clone())
            .create_user(org, user, &format!("{name}@example.test"), T0)
            .await
            .expect("the user provisions");
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
fn blobs(pool: PgPool, name: &str) -> BlobRepo<LocalObjectStore> {
    let dir = std::env::temp_dir().join(format!("tam-avatar-{name}-{}", std::process::id()));
    BlobRepo::new(
        pool,
        LocalObjectStore::new(dir),
        Kek::from_bytes(&[0x22; 32]).expect("a 32-byte key is a key"),
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn a_user_sets_reads_and_clears_their_own_picture(pool: PgPool) {
    seed(&pool).await;
    let hash = blobs(pool.clone(), "own")
        .put(ORG_A, b"a picture's bytes", T0)
        .await
        .expect("the picture seals");
    let profiles = ProfileRepo::new(pool);

    assert_eq!(
        profiles.avatar(ORG_A, USER_A).await.expect("the read runs"),
        Some(None),
        "a user who has set no picture reads as present and empty"
    );
    assert_eq!(
        profiles
            .set_avatar(ORG_A, USER_A, Some(hash))
            .await
            .expect("the write runs"),
        AvatarWrite::Stored(Some(hash))
    );
    assert_eq!(
        profiles.avatar(ORG_A, USER_A).await.expect("the read runs"),
        Some(Some(hash))
    );
    assert_eq!(
        profiles
            .set_avatar(ORG_A, USER_A, None)
            .await
            .expect("the clear runs"),
        AvatarWrite::Stored(None)
    );
    assert_eq!(
        profiles.avatar(ORG_A, USER_A).await.expect("the read runs"),
        Some(None)
    );
}

/// The column names only bytes this organisation sealed. A hash nobody
/// uploaded and a hash another tenant uploaded are the same refusal, decided
/// by the foreign key rather than by a check that could race.
#[sqlx::test(migrations = "./migrations")]
async fn a_picture_names_only_bytes_this_organisation_holds(pool: PgPool) {
    seed(&pool).await;
    let theirs = blobs(pool.clone(), "theirs")
        .put(ORG_B, b"the other tenant's picture", T0)
        .await
        .expect("the other tenant's picture seals");
    let profiles = ProfileRepo::new(pool);

    assert_eq!(
        profiles
            .set_avatar(ORG_A, USER_A, Some(ContentHash([0x51; 32])))
            .await
            .expect("the write runs"),
        AvatarWrite::NotHeld,
        "a hash nobody uploaded is refused, not stored"
    );
    assert_eq!(
        profiles
            .set_avatar(ORG_A, USER_A, Some(theirs))
            .await
            .expect("the write runs"),
        AvatarWrite::NotHeld,
        "a hash another tenant sealed is refused exactly as an unknown one is"
    );
    assert_eq!(
        profiles.avatar(ORG_A, USER_A).await.expect("the read runs"),
        Some(None),
        "neither refusal wrote anything"
    );
}

/// The organisation in the predicate is not redundant: with no row policy on
/// `app_user`, it is what keeps one organisation's session from naming a user
/// of another.
#[sqlx::test(migrations = "./migrations")]
async fn the_write_and_the_read_name_one_user_of_one_organisation(pool: PgPool) {
    seed(&pool).await;
    let mine = blobs(pool.clone(), "fence")
        .put(ORG_A, b"my picture", T0)
        .await
        .expect("the picture seals");
    let profiles = ProfileRepo::new(pool);
    assert_eq!(
        profiles
            .set_avatar(ORG_A, USER_A, Some(mine))
            .await
            .expect("the write runs"),
        AvatarWrite::Stored(Some(mine))
    );

    assert_eq!(
        profiles
            .set_avatar(ORG_B, USER_A, None)
            .await
            .expect("the write runs"),
        AvatarWrite::NoSuchUser,
        "organisation B cannot clear a user of organisation A"
    );
    assert_eq!(
        profiles.avatar(ORG_B, USER_A).await.expect("the read runs"),
        None,
        "nor read them"
    );
    assert_eq!(
        profiles.avatar(ORG_A, USER_A).await.expect("the read runs"),
        Some(Some(mine)),
        "and the picture stands"
    );
    assert_eq!(
        profiles.avatar(ORG_B, USER_B).await.expect("the read runs"),
        Some(None),
        "the other organisation's own user still has none"
    );
}
