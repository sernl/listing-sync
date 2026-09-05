//! What a seller may ask for, and what the column refuses whoever asks.
//!
//! Two halves. The repository's own bounds — one request per address, and a
//! cap on how many an organisation may hold — are what stop one tenant filling
//! the operator's only view of this table. The column's CHECK constraints are
//! the half that holds for a writer that is not this API, and they are asserted
//! by driving the table directly, because every bound the API applies refuses
//! first and would otherwise hide them.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_storage::{
    LedgerCursor, MarketplaceRequestBackofficeRepo, MarketplaceRequestRepo,
    MarketplaceRequestWrite, NewMarketplaceRequest, REQUESTS_PER_ORG_MAX, REQUEST_PAGE_LIMIT_MAX,
};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

mod common;
use common::{seed_org_a, ORG_A};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));

async fn seed_user(
    pool: &PgPool,
    org: OrgId,
    user: UserId,
    email: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO app_user (id, org_id, email, created_at) VALUES ($1, $2, $3, now())")
        .bind(uuid::Uuid::from_bytes(user.0 .0))
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(email)
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_org_b(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}

/// The operator role's own connection to this test's database. The listing
/// repository reads unpinned, which under forced row-level security is the
/// application role seeing nothing at all -- so a page read through the wrong
/// pool would be empty rather than wrong, and prove nothing.
async fn backoffice_pool(app: &PgPool) -> Result<PgPool, sqlx::Error> {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_backoffice:tam_backoffice_dev@127.0.0.1:5433/{database}"
        ))
        .await
}

fn request(user: UserId, url: &str, at: i64) -> NewMarketplaceRequest<'_> {
    NewMarketplaceRequest {
        id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
        requested_by: user,
        name: "Somewhere",
        url,
        reason: "I sell there",
        created_at: Timestamp(at),
    }
}

/// Direct inserts, under the tenant pin, so the column answers rather than the
/// API. Every one of these is refused by a CHECK constraint and by nothing
/// else in the schema.
#[sqlx::test(migrations = "./migrations")]
async fn the_column_refuses_a_writer_that_is_not_this_api(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_user(&pool, ORG_A, USER_A, "a@example.test")
        .await
        .expect("the user seeds");

    let long_name = "a".repeat(121);
    let long_url = format!("https://x.test/{}", "a".repeat(2_040));
    let long_reason = "a".repeat(2_001);
    for (what, name, url, reason) in [
        ("an empty name", "", "https://x.test", "why"),
        (
            "a name over its bound",
            long_name.as_str(),
            "https://x.test",
            "why",
        ),
        ("an empty address", "Somewhere", "", "why"),
        (
            "an address over its bound",
            "Somewhere",
            long_url.as_str(),
            "why",
        ),
        ("an empty description", "Somewhere", "https://x.test", ""),
        (
            "a description over its bound",
            "Somewhere",
            "https://x.test",
            long_reason.as_str(),
        ),
    ] {
        let mut tx = pool.begin().await.expect("the transaction opens");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
            .execute(&mut *tx)
            .await
            .expect("the tenant pins");
        let refused = sqlx::query(
            "INSERT INTO marketplace_request \
             (org_id, id, requested_by, name, url, reason, created_at) \
             VALUES ($1, gen_random_uuid(), $2, $3, $4, $5, now())",
        )
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .bind(uuid::Uuid::from_bytes(USER_A.0 .0))
        .bind(name)
        .bind(url)
        .bind(reason)
        .execute(&mut *tx)
        .await;
        assert!(
            refused.is_err(),
            "the column refuses {what} whatever writes it"
        );
    }
}

/// One organisation asking about one address twice has asked once, whatever
/// case it typed the second time in.
#[sqlx::test(migrations = "./migrations")]
async fn one_address_is_one_request_per_organisation(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    seed_user(&pool, ORG_A, USER_A, "a@example.test")
        .await
        .expect("org a's user seeds");
    seed_user(&pool, ORG_B, USER_B, "b@example.test")
        .await
        .expect("org b's user seeds");
    let repo = MarketplaceRequestRepo::new(pool.clone());

    let first = repo
        .create(
            ORG_A,
            &request(USER_A, "https://Somewhere.test/shop", 5_000),
        )
        .await
        .expect("the first request records");
    assert!(
        matches!(first, MarketplaceRequestWrite::Recorded(_)),
        "the first request lands"
    );
    assert_eq!(
        repo.create(
            ORG_A,
            &request(USER_A, "https://somewhere.TEST/shop", 6_000)
        )
        .await
        .expect("the second request is answered rather than faulted"),
        MarketplaceRequestWrite::AlreadyAsked,
        "the address is compared case-folded, so this is the same shop"
    );
    assert!(
        matches!(
            repo.create(
                ORG_B,
                &request(USER_B, "https://somewhere.test/shop", 7_000)
            )
            .await
            .expect("org b's request records"),
            MarketplaceRequestWrite::Recorded(_)
        ),
        "the bound is per organisation: another tenant asking about the same \
         shop is a different request and must not be refused"
    );
}

/// The cap, at its own boundary: the last request inside it lands and the
/// first outside it does not.
#[sqlx::test(migrations = "./migrations")]
async fn an_organisation_holds_at_most_the_capped_number_of_requests(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_user(&pool, ORG_A, USER_A, "a@example.test")
        .await
        .expect("the user seeds");
    let repo = MarketplaceRequestRepo::new(pool.clone());

    for index in 0..REQUESTS_PER_ORG_MAX {
        let url = format!("https://shop-{index}.test");
        assert!(
            matches!(
                repo.create(ORG_A, &request(USER_A, &url, 5_000 + index))
                    .await
                    .expect("the request records"),
                MarketplaceRequestWrite::Recorded(_)
            ),
            "request {index} is inside the cap and must land"
        );
    }
    assert_eq!(
        repo.create(ORG_A, &request(USER_A, "https://one-too-many.test", 9_000))
            .await
            .expect("the refused request is an answer rather than a fault"),
        MarketplaceRequestWrite::TooMany,
        "the request past the cap is refused, which is what stops one tenant \
         filling the operator's only view of this table"
    );
}

/// The operator page: keyset-walked to the end, and clamped however much it
/// asks for.
#[sqlx::test(migrations = "./migrations")]
async fn the_operator_page_walks_by_cursor_and_cannot_ask_for_the_whole_table(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_user(&pool, ORG_A, USER_A, "a@example.test")
        .await
        .expect("the user seeds");
    let repo = MarketplaceRequestRepo::new(pool.clone());
    for index in 0..5 {
        let url = format!("https://shop-{index}.test");
        repo.create(ORG_A, &request(USER_A, &url, 5_000 + index))
            .await
            .expect("the request records");
    }

    let backoffice = MarketplaceRequestBackofficeRepo::new(
        backoffice_pool(&pool)
            .await
            .expect("the operator role connects"),
    );
    let first = backoffice
        .newest(None, 2)
        .await
        .expect("the first page reads");
    assert_eq!(
        first.iter().map(|row| row.url.as_str()).collect::<Vec<_>>(),
        vec!["https://shop-4.test", "https://shop-3.test"],
        "the page is the newest two, newest first"
    );
    assert_eq!(
        first
            .first()
            .and_then(|row| row.requested_by_email.as_deref()),
        Some("a@example.test"),
        "the operator listing joins the address a request is answered at"
    );

    let last = first.last().expect("the page has rows");
    let second = backoffice
        .newest(
            Some(LedgerCursor {
                created_at: last.created_at,
                id: last.id,
            }),
            2,
        )
        .await
        .expect("the second page reads");
    assert_eq!(
        second
            .iter()
            .map(|row| row.url.as_str())
            .collect::<Vec<_>>(),
        vec!["https://shop-2.test", "https://shop-1.test"],
        "the cursor resumes after the row it names rather than repeating it"
    );

    // The clamp is the reason the parameter cannot be used to page the whole
    // table: asking for more than the ceiling is asking for the ceiling. Five
    // rows exist, so this also proves the clamp does not floor the answer.
    let asked_for_everything = backoffice
        .newest(None, REQUEST_PAGE_LIMIT_MAX * 10)
        .await
        .expect("the oversized page reads");
    assert_eq!(
        asked_for_everything.len(),
        5,
        "a limit past the ceiling answers what is there, up to the ceiling"
    );
    let one = backoffice.newest(None, 0).await.expect("the page reads");
    assert_eq!(
        one.len(),
        1,
        "a limit below one is clamped up rather than answering nothing"
    );
}

/// Two requests written in the same instant, which the operator page orders by
/// identifier so that the walk cannot repeat or skip one.
///
/// The tie-break is what the index carries it for, and it is unobservable in
/// the page tests above: those stamp every row a millisecond apart, so the
/// order they assert would hold with the identifier dropped from the clause.
#[sqlx::test(migrations = "./migrations")]
async fn one_instant_holding_two_requests_is_ordered_by_identifier(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_user(&pool, ORG_A, USER_A, "a@example.test")
        .await
        .expect("the user seeds");
    let repo = MarketplaceRequestRepo::new(pool.clone());
    for (mark, url) in [
        (0x01u8, "https://lower.test"),
        (0x02, "https://higher.test"),
    ] {
        repo.create(
            ORG_A,
            &NewMarketplaceRequest {
                id: Uuid([mark; 16]),
                requested_by: USER_A,
                name: "Somewhere",
                url,
                reason: "I sell there",
                created_at: Timestamp(5_000),
            },
        )
        .await
        .expect("the request records");
    }

    let backoffice = MarketplaceRequestBackofficeRepo::new(
        backoffice_pool(&pool)
            .await
            .expect("the operator role connects"),
    );
    let first = backoffice
        .newest(None, 1)
        .await
        .expect("the first page reads");
    assert_eq!(
        first.first().map(|row| row.url.as_str()),
        Some("https://higher.test"),
        "of two rows sharing an instant, the higher identifier is the newer"
    );

    let last = first.last().expect("the page has a row");
    let second = backoffice
        .newest(
            Some(LedgerCursor {
                created_at: last.created_at,
                id: last.id,
            }),
            1,
        )
        .await
        .expect("the second page reads");
    assert_eq!(
        second.first().map(|row| row.url.as_str()),
        Some("https://lower.test"),
        "the cursor resumes past a row it shares an instant with, rather than \
         answering it again or skipping the one beneath it"
    );
}
