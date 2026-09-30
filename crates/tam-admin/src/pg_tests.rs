//! The two deployment-time guide writes against a real database: the corpus
//! seed and the picture placement, each run twice. The migration step runs
//! both on every deploy, so a second run over an unchanged corpus has to
//! write nothing -- no revision bumped, no guide re-published, no picture
//! re-sealed -- and a changed file has to be the one thing that moves.

use std::path::{Path, PathBuf};

use sqlx::PgPool;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{platform_org, BlobRepo, GuideRepo};
use tam_types::{Timestamp, UserId, Uuid};

use crate::guides_images::{self, handle_of};
use crate::guides_seed::{self, Outcome};

const T0: Timestamp = Timestamp(1_790_000_000_000);
const T1: Timestamp = Timestamp(1_790_000_600_000);
const AUTHOR: UserId = UserId(Uuid([0xA1; 16]));

/// A PNG signature and nothing else: enough for the probe the upload route
/// and the placement both ask, and two of them differ only in the tail.
fn png(tail: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(tail);
    bytes
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free fixture helper; a broken fixture should panic"
)]
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tam-admin-{name}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("the scratch directory is created");
    dir
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free fixture helper; a broken fixture should panic"
)]
fn guide(dir: &Path, slug: &str, body: &str) {
    std::fs::write(
        dir.join(format!("{slug}.md")),
        format!("---\ntopic: basics\ntags: start, files\n---\n# {slug} title\n\n{body}\n"),
    )
    .expect("the guide file is written");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free fixture helper; a broken fixture should panic"
)]
async fn provision_author(pool: &PgPool) {
    let org = uuid::Uuid::from_bytes([0xA0; 16]);
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(org)
        .bind("Alice's Classroom")
        .execute(pool)
        .await
        .expect("the author's organisation stores");
    sqlx::query("INSERT INTO app_user (id, org_id, email, created_at) VALUES ($1, $2, $3, now())")
        .bind(uuid::Uuid::from_bytes(AUTHOR.0 .0))
        .bind(org)
        .bind("alice@example.test")
        .execute(pool)
        .await
        .expect("the author stores");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_second_seed_of_an_unchanged_corpus_writes_nothing(pool: PgPool) {
    provision_author(&pool).await;
    let dir = scratch("seed");
    guide(&dir, "account", "Your account.");
    guide(&dir, "your-files", "Your files.");
    let repo = GuideRepo::new(pool.clone());

    let files = guides_seed::read_directory(&dir).expect("the corpus reads");
    let first = guides_seed::seed(&repo, &files, AUTHOR, T0)
        .await
        .expect("the first seed writes");
    assert_eq!(
        first,
        vec![
            ("account".to_owned(), Outcome::Created),
            ("your-files".to_owned(), Outcome::Created),
        ],
        "a first seed creates every guide"
    );
    let before = repo.get("account").await.expect("reads").expect("stored");

    let again = guides_seed::seed(&repo, &files, AUTHOR, T1)
        .await
        .expect("the second seed runs");
    assert!(
        again
            .iter()
            .all(|(_, outcome)| *outcome == Outcome::Unchanged),
        "a re-run over the same files changes no guide: {again:?}"
    );
    let after = repo.get("account").await.expect("reads").expect("stored");
    assert_eq!(
        (
            after.revision,
            after.published.map(|published| published.published_at)
        ),
        (
            before.revision,
            before.published.map(|published| published.published_at)
        ),
        "the unchanged guide keeps its revision and its publication"
    );

    guide(&dir, "your-files", "Your files, retaken.");
    let files = guides_seed::read_directory(&dir).expect("the corpus reads");
    let changed = guides_seed::seed(&repo, &files, AUTHOR, T1)
        .await
        .expect("the third seed writes");
    assert_eq!(
        changed,
        vec![
            ("account".to_owned(), Outcome::Unchanged),
            ("your-files".to_owned(), Outcome::Updated),
        ],
        "only the edited file moves"
    );
    let published = repo
        .get("your-files")
        .await
        .expect("reads")
        .expect("stored")
        .published
        .expect("still published");
    assert!(
        published.body.contains("retaken"),
        "sellers read the edited prose: {:?}",
        published.body
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn placing_the_same_pictures_twice_stores_them_once(pool: PgPool) {
    let pictures = scratch("pictures");
    let store = scratch("store");
    std::fs::write(pictures.join("account-1.png"), png(b"one")).expect("writes");
    std::fs::write(pictures.join("account-2.png"), png(b"two")).expect("writes");
    std::fs::write(pictures.join("README.txt"), b"not a picture").expect("writes");
    let repo = BlobRepo::new(
        pool.clone(),
        LocalObjectStore::new(store.clone()),
        Kek::from_bytes(&[0x22; 32]).expect("kek"),
    );

    let first = guides_images::place(&pool, &repo, &pictures, T0)
        .await
        .expect("the first placement stores");
    assert_eq!(
        first
            .iter()
            .map(|placed| (placed.file.as_str(), placed.stored))
            .collect::<Vec<_>>(),
        vec![("account-1.png", true), ("account-2.png", true)],
        "both pictures are stored and the text file is passed over"
    );
    let objects = std::fs::read_dir(&store).expect("store").count();

    let again = guides_images::place(&pool, &repo, &pictures, T1)
        .await
        .expect("the second placement runs");
    assert!(
        again.iter().all(|placed| !placed.stored),
        "a re-run stores nothing: {again:?}"
    );
    assert_eq!(
        again
            .iter()
            .map(|placed| placed.handle.clone())
            .collect::<Vec<_>>(),
        first
            .iter()
            .map(|placed| placed.handle.clone())
            .collect::<Vec<_>>(),
        "the same bytes answer the same handles"
    );
    assert_eq!(
        std::fs::read_dir(&store).expect("store").count(),
        objects,
        "no object was written a second time"
    );

    // What the reader route does: the platform organisation, by slug, and
    // the bytes under the handle a guide names.
    let org = platform_org(&pool)
        .await
        .expect("reads")
        .expect("the placement created the platform organisation");
    let wanted = png(b"one");
    let hash = tam_pipeline::hash::content_hash(&wanted);
    assert_eq!(
        handle_of(hash),
        first[0].handle,
        "the handle is the BLAKE3 hash of the file's bytes"
    );
    assert_eq!(
        repo.get(org, hash).await.expect("the picture opens"),
        wanted,
        "the placed picture reads back as its own bytes"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_file_that_is_not_a_picture_stops_the_placement(pool: PgPool) {
    let pictures = scratch("not-a-picture");
    std::fs::write(
        pictures.join("a.png"),
        b"plain text wearing a picture's name",
    )
    .expect("writes");
    let repo = BlobRepo::new(
        pool.clone(),
        LocalObjectStore::new(scratch("store-refused")),
        Kek::from_bytes(&[0x22; 32]).expect("kek"),
    );
    let refused = guides_images::place(&pool, &repo, &pictures, T0).await;
    assert!(
        refused.is_err(),
        "bytes the upload route refuses are refused here too"
    );
}
