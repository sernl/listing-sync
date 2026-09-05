//! The slug's guarantees at the database, proved independently of the route
//! that relies on them.
//!
//! The API refuses a malformed slug, lowercases what it stores and reports a
//! collision as a typed outcome, and every one of those is a promise made by
//! one code path. The unique index and the shape CHECK are what make the same
//! promises true of any other writer -- a psql session, a backfill script, a
//! future migration -- so they are asserted here against raw statements rather
//! than through the repository that already agrees with them.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_storage::{OrgRepo, OrgWrite};
use tam_types::{OrgId, Uuid};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(db_uuid(org.0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn each_tenant_reads_back_its_own_slug_and_no_other(pool: PgPool) {
    seed(&pool).await;
    let repo = OrgRepo::new(pool.clone());
    for (org, slug) in [(ORG_A, "riverbend"), (ORG_B, "harbourview")] {
        assert_eq!(
            repo.update(org, None, Some(slug))
                .await
                .expect("the claim writes"),
            OrgWrite::Stored,
            "each tenant claims a slug of its own"
        );
    }
    for (org, slug) in [(ORG_A, "riverbend"), (ORG_B, "harbourview")] {
        let record = repo
            .get(org)
            .await
            .expect("the row reads")
            .expect("the row is there");
        assert_eq!(
            record.slug.as_deref(),
            Some(slug),
            "the row read back is the tenant's own"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_unique_index_refuses_a_duplicate_written_directly(pool: PgPool) {
    seed(&pool).await;
    sqlx::query("UPDATE organisation SET slug = 'riverbend' WHERE id = $1")
        .bind(db_uuid(ORG_A.0))
        .execute(&pool)
        .await
        .expect("the first slug writes");
    let refused = sqlx::query("UPDATE organisation SET slug = 'riverbend' WHERE id = $1")
        .bind(db_uuid(ORG_B.0))
        .execute(&pool)
        .await
        .expect_err("a duplicate slug is refused by the database itself");
    assert!(
        refused
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::constraint)
            == Some("organisation_slug_lower"),
        "the refusal names the index the repository catches by name: {refused}"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_index_compares_slugs_case_insensitively(pool: PgPool) {
    seed(&pool).await;
    sqlx::query("UPDATE organisation SET slug = 'riverbend' WHERE id = $1")
        .bind(db_uuid(ORG_A.0))
        .execute(&pool)
        .await
        .expect("the first slug writes");
    // The write path lowercases, so a plain unique index on `slug` would pass
    // every other test in this file. This is the one assertion that separates
    // an index on `slug` from an index on `lower(slug)`, and it has to reach
    // past the shape CHECK to make it -- which is why the probe is a SELECT
    // rather than an insert of an uppercase value.
    let collides: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM organisation WHERE lower(slug) = lower($1))",
    )
    .bind("RiverBend")
    .fetch_one(&pool)
    .await
    .expect("the probe reads");
    assert!(
        collides,
        "the index's own expression matches a slug differing only in case"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_shape_check_refuses_a_malformed_slug_written_directly(pool: PgPool) {
    seed(&pool).await;
    for malformed in [
        "ab",
        "-riverbend",
        "riverbend-",
        "river--bend",
        "River_bend",
        "RiverBend",
        "river bend",
        &"a".repeat(33),
    ] {
        let refused = sqlx::query("UPDATE organisation SET slug = $2 WHERE id = $1")
            .bind(db_uuid(ORG_A.0))
            .bind(malformed)
            .execute(&pool)
            .await
            .expect_err("the CHECK refuses a malformed slug whatever wrote it");
        assert!(
            refused
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::constraint)
                == Some("organisation_slug_shape"),
            "{malformed:?} is refused by the shape constraint: {refused}"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn many_organisations_carry_no_slug_at_once(pool: PgPool) {
    seed(&pool).await;
    let unclaimed: i64 = sqlx::query_scalar("SELECT count(*) FROM organisation WHERE slug IS NULL")
        .fetch_one(&pool)
        .await
        .expect("the count reads");
    assert_eq!(
        unclaimed, 2,
        "the unique index is NULLS DISTINCT, so every unclaimed row coexists under it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_write_naming_no_slug_leaves_the_claimed_one_standing(pool: PgPool) {
    seed(&pool).await;
    let repo = OrgRepo::new(pool.clone());
    repo.update(ORG_A, None, Some("riverbend"))
        .await
        .expect("the claim writes");
    repo.update(ORG_A, Some("Riverbend Resources"), None)
        .await
        .expect("the rename writes");
    let record = repo
        .get(ORG_A)
        .await
        .expect("the row reads")
        .expect("the row is there");
    assert_eq!(
        (record.name.as_str(), record.slug.as_deref()),
        ("Riverbend Resources", Some("riverbend")),
        "an omitted slug means leave it, never clear it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_collision_is_a_typed_outcome_rather_than_a_fault(pool: PgPool) {
    seed(&pool).await;
    let repo = OrgRepo::new(pool.clone());
    repo.update(ORG_A, None, Some("riverbend"))
        .await
        .expect("the first claim writes");
    assert_eq!(
        repo.update(ORG_B, Some("Harbourview"), Some("riverbend"))
            .await
            .expect("a collision is an answer, not an error"),
        OrgWrite::SlugTaken,
        "the repository reports the named index rather than raising a storage fault"
    );
    let record = repo
        .get(ORG_B)
        .await
        .expect("the row reads")
        .expect("the row is there");
    assert_eq!(
        (record.name.as_str(), record.slug),
        ("org-b", None),
        "one statement, so the refused slug took the name in the same body with it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn availability_excludes_the_asking_tenants_own_row(pool: PgPool) {
    seed(&pool).await;
    let repo = OrgRepo::new(pool.clone());
    repo.update(ORG_A, None, Some("riverbend"))
        .await
        .expect("the claim writes");
    assert!(
        repo.slug_taken(ORG_B, "riverbend")
            .await
            .expect("the probe reads"),
        "another tenant's slug is taken"
    );
    assert!(
        !repo
            .slug_taken(ORG_A, "riverbend")
            .await
            .expect("the probe reads"),
        "a tenant's own slug is free to it, which is what re-writing it would do"
    );
    assert!(
        !repo
            .slug_taken(ORG_B, "harbourview")
            .await
            .expect("the probe reads"),
        "an unclaimed slug is free"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_write_to_an_organisation_that_is_not_there_says_so(pool: PgPool) {
    seed(&pool).await;
    assert_eq!(
        OrgRepo::new(pool.clone())
            .update(OrgId(Uuid([0xCC; 16])), None, Some("riverbend"))
            .await
            .expect("a missing row is an answer, not an error"),
        OrgWrite::NoSuchOrg,
        "nothing was there to write to"
    );
}
