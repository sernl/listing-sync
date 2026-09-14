//! The activity log's page boundary, which is the one place this list can
//! lose a row.
//!
//! The log merges three sources by clock, so a page ends at an instant. Two
//! lines of one millisecond are ordinary — a pass materialises every
//! marketplace of a tick at the same instant — and a cursor of "strictly
//! older than the last instant" silently dropped the rest of that instant,
//! while one of "that instant or older" would repeat it for ever. The cursor
//! is therefore the instant *and* the line's key, and these tests are the
//! record of it: the walk must hand over every line exactly once.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_storage::{ActivityCursor, SyncSettingRepo};
use tam_types::{OrgId, Uuid};

use common::{seed_org_a, ORG_A};

const TICK: &str = "2026-09-01T09:00:00Z";

fn db(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

/// Two schedules, each with one run row at the very same tick.
///
/// A schedule run is the cheapest of the log's three sources to write: it
/// needs no product, no mapping and no blob, and it is the source whose rows
/// most reliably share an instant, because one pass writes them together.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn two_ticks_at_one_instant(pool: &PgPool, org: OrgId) {
    let mut tx = pool.begin().await.expect("the transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db(org.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the organisation pins");
    for (index, name) in [(0xC1u8, "Friday drop"), (0xC2u8, "Monday drop")] {
        let schedule = db(Uuid([index; 16]));
        sqlx::query(
            "INSERT INTO schedule (org_id, id, name, selection_kind, selection_label, intent, \
                                   at_minute_of_day, timezone, repeat, weekday, \
                                   republish_on_update, enabled, created_at) \
             VALUES ($1, $2, $3, 'label', 'new', 'draft', 540, 'Pacific/Auckland', 'daily', \
                     NULL, false, true, now())",
        )
        .bind(db(org.0))
        .bind(schedule)
        .bind(name)
        .execute(&mut *tx)
        .await
        .expect("the schedule writes");
        sqlx::query(
            "INSERT INTO schedule_run (org_id, schedule_id, tick, product_id, inventory, state, \
                                       job_id, reason, recorded_at) \
             VALUES ($1, $2, $3::timestamptz, $4, 'tes', 'sent', $5, NULL, now())",
        )
        .bind(db(org.0))
        .bind(schedule)
        .bind(TICK)
        .bind(db(Uuid([index ^ 0x0F; 16])))
        .bind(db(Uuid([index ^ 0xF0; 16])))
        .execute(&mut *tx)
        .await
        .expect("the run writes");
    }
    tx.commit().await.expect("the fixture commits");
}

/// The property: a walk of one-row pages hands over both lines of the shared
/// instant, each exactly once, and then stops.
///
/// Before the key was part of the cursor this test failed on its second page:
/// the first page ended at the tick, the second asked for lines strictly
/// older than it, and the other line of that tick was unreachable for ever.
#[sqlx::test(migrations = "./migrations")]
async fn a_walk_loses_no_line_of_a_shared_instant(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    two_ticks_at_one_instant(&pool, ORG_A).await;
    let repo = SyncSettingRepo::new(pool);

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<ActivityCursor> = None;
    // One more turn than there are lines, so the walk has to stop of its own
    // accord rather than because the loop ran out.
    for _turn in 0..4 {
        let page = repo
            .activity(ORG_A, cursor.clone(), 1)
            .await
            .expect("the page reads");
        let Some(line) = page.first() else {
            cursor = None;
            break;
        };
        seen.push(line.key.clone());
        cursor = Some(ActivityCursor {
            at: line.at,
            key: line.key.clone(),
        });
    }

    assert_eq!(
        seen.len(),
        2,
        "both lines of the shared instant are handed over: {seen:?}"
    );
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "and neither is handed over twice: {seen:?}"
    );
    assert!(
        cursor.is_none(),
        "the walk reaches the end and stops rather than looping"
    );
}

/// The same rows read as one page are in one total order, and the key is what
/// makes it total: the two lines share an instant, so ordering by the clock
/// alone leaves their order up to the plan.
#[sqlx::test(migrations = "./migrations")]
async fn one_page_orders_a_shared_instant_by_the_key(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    two_ticks_at_one_instant(&pool, ORG_A).await;
    let page = SyncSettingRepo::new(pool)
        .activity(ORG_A, None, 25)
        .await
        .expect("the page reads");

    let keys: Vec<&str> = page.iter().map(|line| line.key.as_str()).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted, "within one instant the keys ascend: {keys:?}");
    assert_eq!(page.len(), 2);
}

/// A recurring schedule's runs are separate lines, so they carry separate
/// identities.
///
/// A daily schedule sends to the same marketplace every day. The identity
/// left the tick out at first, so every one of those runs was named
/// `tick:<schedule>:<inventory>` — and two of them on one page were two rows
/// under one key, which the rendered list refuses outright.
#[sqlx::test(migrations = "./migrations")]
async fn two_ticks_of_one_schedule_carry_two_identities(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let mut tx = pool.begin().await.expect("the transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db(ORG_A.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the organisation pins");
    let schedule = db(Uuid([0xD1; 16]));
    sqlx::query(
        "INSERT INTO schedule (org_id, id, name, selection_kind, selection_label, intent, \
                               at_minute_of_day, timezone, repeat, weekday, \
                               republish_on_update, enabled, created_at) \
         VALUES ($1, $2, 'Daily drop', 'label', 'new', 'draft', 540, 'Pacific/Auckland', \
                 'daily', NULL, false, true, now())",
    )
    .bind(db(ORG_A.0))
    .bind(schedule)
    .execute(&mut *tx)
    .await
    .expect("the schedule writes");
    for (day, product) in [(TICK, 0xD2u8), ("2026-09-02T09:00:00Z", 0xD3u8)] {
        sqlx::query(
            "INSERT INTO schedule_run (org_id, schedule_id, tick, product_id, inventory, state, \
                                       job_id, reason, recorded_at) \
             VALUES ($1, $2, $3::timestamptz, $4, 'tes', 'sent', $5, NULL, now())",
        )
        .bind(db(ORG_A.0))
        .bind(schedule)
        .bind(day)
        .bind(db(Uuid([product; 16])))
        .bind(db(Uuid([0xD9; 16])))
        .execute(&mut *tx)
        .await
        .expect("the run writes");
    }
    tx.commit().await.expect("the fixture commits");

    let page = SyncSettingRepo::new(pool)
        .activity(ORG_A, None, 25)
        .await
        .expect("the page reads");
    let keys: Vec<&str> = page.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(page.len(), 2, "both ticks are lines: {keys:?}");
    assert_ne!(
        keys.first(),
        keys.last(),
        "the same schedule on the same marketplace on two days is two identities: {keys:?}"
    );
}
