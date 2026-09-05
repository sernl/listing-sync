//! What a spreadsheet import batch is, and who may see one.
//!
//! Four halves, and the first is the one the rest are measured against. Two
//! tenants each hold a batch, and neither reaches the other's through any
//! statement this repository serves — if that failed, every other assertion
//! here would be about a table with no fence.
//!
//! The second is the columns' own CHECK constraints and the partial unique
//! index, asserted by driving the tables directly. The repository's own
//! answers refuse first and would otherwise hide them, and the point of a
//! CHECK is that it holds for a writer that is not the repository.
//!
//! The third is the two answers a create gives that are not errors: a replayed
//! idempotency key, and an organisation that already has an import open.
//!
//! The fourth is the expiry sweep, which is the one cross-tenant pass over
//! these tables and therefore the one thing here that runs as `tam_engine`.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use tam_storage::{
    BatchState, BatchWrite, ImportBatchRepo, NewImportBatch, NewImportBatchRow, RowIntent, RowState,
};
use tam_types::{InventoryId, OrgId, Timestamp, Uuid};

mod common;
use common::{seed_org_a, ORG_A};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const BATCH_A: Uuid = Uuid([0x0A; 16]);
const BATCH_B: Uuid = Uuid([0x0B; 16]);
const MADE: Timestamp = Timestamp(1_000_000);
const DEADLINE: Timestamp = Timestamp(2_000_000);
const AFTER_DEADLINE: Timestamp = Timestamp(3_000_000);
const SWEPT: &str = "the deadline passed";

async fn seed_org_b(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}

/// A transaction with the tenant pin set, for the direct statements that have
/// to answer as the column rather than as the repository.
async fn pinned(pool: &PgPool, org: OrgId) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// The cross-tenant role's own connection to this test's database, which is
/// what the sweep runs as. Under `tam_app`'s forced row-level security an
/// unpinned pass sees nothing at all, so a sweep driven through the wrong pool
/// would settle nothing and prove nothing.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name reads");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects")
}

fn draft(title: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "body": "",
        "price": "Free",
        "labels": ["Autumn Term"],
        "inventories": [],
        "natives": {},
        "file_name": null
    })
}

fn no_problems() -> serde_json::Value {
    serde_json::json!([])
}

fn one_problem() -> serde_json::Value {
    serde_json::json!([{ "column": "Title", "problem": "a resource needs a name" }])
}

fn row<'a>(
    sheet: &'a str,
    ordinal: u32,
    intent: RowIntent,
    held: &'a serde_json::Value,
    problems: &'a serde_json::Value,
) -> NewImportBatchRow<'a> {
    NewImportBatchRow {
        sheet,
        ordinal,
        inventory: if sheet == "Teachouse" {
            None
        } else {
            Some(InventoryId::TesGb)
        },
        intent,
        draft: held,
        problems,
        file_name: None,
    }
}

fn batch<'a>(id: Uuid, rows: &'a [NewImportBatchRow<'a>]) -> NewImportBatch<'a> {
    NewImportBatch {
        id,
        source_name: "catalogue.xlsx",
        created_at: MADE,
        expires_at: DEADLINE,
        rows,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn each_tenant_keeps_its_own_import_and_neither_reaches_the_other(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    let repo = ImportBatchRepo::new(pool.clone());

    let mine = draft("Phonics pack");
    let theirs = draft("Fractions unit");
    let clean = no_problems();
    for (org, id, held) in [(ORG_A, BATCH_A, &mine), (ORG_B, BATCH_B, &theirs)] {
        let rows = [row("Teachouse", 4, RowIntent::Draft, held, &clean)];
        assert!(
            matches!(
                repo.create(org, &batch(id, &rows)).await,
                Ok(BatchWrite::Saved(_))
            ),
            "each tenant writes its own batch"
        );
    }

    assert_eq!(
        repo.list(ORG_A)
            .await
            .expect("the listing runs")
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec![BATCH_A],
        "a listing under one pin holds that organisation's batches and no other's"
    );
    assert_eq!(
        repo.get(ORG_A, BATCH_B).await.expect("the read runs"),
        None,
        "another organisation's batch is absent rather than readable, and its identifier is \
         the one thing a caller could guess"
    );
    assert_eq!(
        repo.rows(ORG_A, BATCH_B).await.expect("the read runs"),
        vec![],
        "nor are its rows reachable, which is where the seller's own listing copy sits"
    );
    assert!(
        !repo
            .abandon(ORG_A, BATCH_B, AFTER_DEADLINE, "stolen")
            .await
            .expect("the settle runs"),
        "and it cannot be settled across the fence"
    );
    assert_eq!(
        repo.get(ORG_B, BATCH_B)
            .await
            .expect("the read runs")
            .map(|record| record.state),
        Some(BatchState::Parsed),
        "so the other tenant's batch is still open afterwards"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_batch_counts_its_own_rows_and_a_refused_row_is_failed(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let refused = one_problem();
    let rows = [
        row("Teachouse", 4, RowIntent::Draft, &held, &clean),
        row("TES GB", 4, RowIntent::Live, &held, &clean),
        row("TES GB", 5, RowIntent::Draft, &held, &refused),
    ];
    let written = repo
        .create(ORG_A, &batch(BATCH_A, &rows))
        .await
        .expect("the create runs");
    let BatchWrite::Saved(record) = written else {
        panic!("a first create saves");
    };
    assert_eq!(
        (record.row_count, record.live_count, record.failed_count),
        (3, 1, 1),
        "the counts are computed from the rows rather than stated by the caller"
    );

    let read = repo.rows(ORG_A, BATCH_A).await.expect("the rows read");
    assert_eq!(
        read.iter()
            .map(|held| (held.sheet.clone(), held.ordinal, held.state))
            .collect::<Vec<_>>(),
        vec![
            ("TES GB".to_owned(), 4, RowState::Parsed),
            ("TES GB".to_owned(), 5, RowState::Failed),
            ("Teachouse".to_owned(), 4, RowState::Parsed),
        ],
        "rows come back in byte-collation sheet order and then row order, which is a fact of \
         this query rather than of the cluster's locale; the workbook's own tab order is the \
         API's to apply, because that is the order a seller fixes a sheet against"
    );
    assert_eq!(
        read.first().map(|held| held.labels.clone()),
        Some(vec!["Autumn Term".to_owned()]),
        "the labels a row's draft names are lifted out without shipping the draft"
    );
    assert_eq!(
        read.first().and_then(|held| held.inventory),
        Some(InventoryId::TesGb),
        "a grid row names its inventory and a Teachouse row names none"
    );
    assert_eq!(
        read.get(2).and_then(|held| held.inventory),
        None,
        "a Teachouse row names no inventory"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn one_import_is_open_at_a_time_and_the_refusal_names_the_open_one(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [row("Teachouse", 4, RowIntent::Draft, &held, &clean)];

    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));
    let second = repo
        .create(ORG_A, &batch(BATCH_B, &rows))
        .await
        .expect("the second create runs");
    let BatchWrite::AlreadyOpen(open) = second else {
        panic!("a second open batch is refused as an answer rather than as a fault: {second:?}");
    };
    assert_eq!(
        open.id, BATCH_A,
        "the refusal carries the open batch, so the console can link to it"
    );

    assert!(
        repo.abandon(ORG_A, BATCH_A, AFTER_DEADLINE, "given up")
            .await
            .expect("the settle runs"),
        "abandoning the open one releases the hold"
    );
    assert!(
        matches!(
            repo.create(ORG_A, &batch(BATCH_B, &rows)).await,
            Ok(BatchWrite::Saved(_))
        ),
        "and the next import is admitted"
    );
    assert!(
        !repo
            .abandon(ORG_A, BATCH_A, AFTER_DEADLINE, "again")
            .await
            .expect("the second settle runs"),
        "a settled batch is never re-settled, which is what makes a double-clicked button one \
         action"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_replayed_idempotency_key_is_the_first_batch_and_writes_no_second_set_of_rows(
    pool: PgPool,
) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [row("Teachouse", 4, RowIntent::Draft, &held, &clean)];

    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));
    let again = repo
        .create(ORG_A, &batch(BATCH_A, &rows))
        .await
        .expect("the replay runs");
    let BatchWrite::Replay(record) = again else {
        panic!("a re-posted key is a replay rather than a second batch: {again:?}");
    };
    assert_eq!(record.id, BATCH_A);
    assert_eq!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .len(),
        1,
        "a replay appends no second copy of every row"
    );
}

/// The column's own constraints, driven directly. Each of these is a rule the
/// repository also enforces, and each is asserted here because the point of a
/// CHECK is that it binds a writer that is not the repository.
#[sqlx::test(migrations = "./migrations")]
async fn the_columns_refuse_what_the_repository_would_never_write(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [row("TES GB", 4, RowIntent::Live, &held, &clean)];
    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));

    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    let settled_without_an_instant =
        sqlx::query("UPDATE import_batch SET state = 'imported' WHERE org_id = $1 AND id = $2")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
            .bind(uuid::Uuid::from_bytes(BATCH_A.0))
            .execute(&mut *tx)
            .await;
    assert!(
        settled_without_an_instant.is_err(),
        "a settled batch states when it settled, and an unsettled one states nothing"
    );
    drop(tx);

    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    let created_without_a_product = sqlx::query(
        "UPDATE import_batch_row SET state = 'created' \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = 'TES GB' AND ordinal = 4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .execute(&mut *tx)
    .await;
    assert!(
        created_without_a_product.is_err(),
        "a created row names the product it created, which is the breadcrumb a resumed commit \
         reads to avoid a duplicate"
    );
    drop(tx);

    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    let refused_but_not_failed = sqlx::query(
        "UPDATE import_batch_row SET problems = $3 \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = 'TES GB' AND ordinal = 4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .bind(one_problem())
    .execute(&mut *tx)
    .await;
    assert!(
        refused_but_not_failed.is_err(),
        "a row carrying a refusal is failed, so a report and a state cannot disagree"
    );
    drop(tx);

    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    let half_a_handle = sqlx::query(
        "UPDATE import_batch_row SET file_kind = 'pdf' \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = 'TES GB' AND ordinal = 4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .execute(&mut *tx)
    .await;
    assert!(
        half_a_handle.is_err(),
        "a file handle is three facts or none, so a hash without its length is refused"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_sweep_settles_an_expired_batch_and_leaves_a_live_one_alone(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [row("Teachouse", 4, RowIntent::Draft, &held, &clean)];

    for (org, id) in [(ORG_A, BATCH_A), (ORG_B, BATCH_B)] {
        assert!(matches!(
            repo.create(org, &batch(id, &rows)).await,
            Ok(BatchWrite::Saved(_))
        ));
    }
    // One of the two is pushed past its deadline; the other keeps the deadline
    // the create wrote, which is in the future for the cutoff below.
    let mut tx = pinned(&pool, ORG_B).await.expect("the pin sets");
    sqlx::query("UPDATE import_batch SET expires_at = $3 WHERE org_id = $1 AND id = $2")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .bind(uuid::Uuid::from_bytes(BATCH_B.0))
        .bind(
            chrono::DateTime::from_timestamp_millis(9_000_000_000)
                .expect("the instant is in range"),
        )
        .execute(&mut *tx)
        .await
        .expect("the deadline moves");
    tx.commit().await.expect("the move commits");

    let engine = engine_pool(&pool).await;
    let report = ImportBatchRepo::new(engine)
        .sweep_pass(AFTER_DEADLINE, 10, SWEPT)
        .await
        .expect("the sweep runs");
    assert_eq!(
        (report.abandoned, report.released),
        (1, 0),
        "one batch was past its deadline, and no row of it held bytes to release"
    );

    assert_eq!(
        repo.get(ORG_A, BATCH_A)
            .await
            .expect("the read runs")
            .map(|record| (record.state, record.settled_at, record.failure_detail)),
        Some((
            BatchState::Abandoned,
            Some(AFTER_DEADLINE),
            Some(SWEPT.to_owned())
        )),
        "the swept batch is settled at the cutoff and says why, because the seller reads that \
         line on a batch that closed without them closing it"
    );
    assert_eq!(
        repo.get(ORG_B, BATCH_B)
            .await
            .expect("the read runs")
            .map(|record| record.state),
        Some(BatchState::Parsed),
        "a batch inside its deadline is untouched, and the pass crossed a tenant to check"
    );

    let second = ImportBatchRepo::new(engine_pool(&pool).await)
        .sweep_pass(AFTER_DEADLINE, 10, SWEPT)
        .await
        .expect("the second sweep runs");
    assert_eq!(
        (second.abandoned, second.released),
        (0, 0),
        "a settled batch is never settled twice, so the pass converges"
    );
}

/// The one thing the sweep does that no other pass does: release a row's hold
/// on bytes. Driven with a handle written directly, because the route that
/// binds one is the phase after this.
#[sqlx::test(migrations = "./migrations")]
async fn the_sweep_releases_the_file_handles_its_rows_hold(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [row("TES GB", 4, RowIntent::Live, &held, &clean)];
    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));

    let hash = vec![0x51_u8; 32];
    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    sqlx::query(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, 4, 'k', 1, now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(&hash)
    .execute(&mut *tx)
    .await
    .expect("the blob writes");
    sqlx::query(
        "UPDATE import_batch_row \
            SET file_hash = $3, file_kind = 'pdf', file_byte_len = 4, state = 'attached' \
          WHERE org_id = $1 AND batch_id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .bind(&hash)
    .execute(&mut *tx)
    .await
    .expect("the handle binds");
    tx.commit().await.expect("the bind commits");

    assert!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .first()
            .is_some_and(|row| row.file.is_some()),
        "the row holds bytes before the sweep runs"
    );

    let report = ImportBatchRepo::new(engine_pool(&pool).await)
        .sweep_pass(AFTER_DEADLINE, 10, SWEPT)
        .await
        .expect("the sweep runs");
    assert_eq!(
        (report.abandoned, report.released),
        (1, 1),
        "the pass settles the batch and releases the one handle it held"
    );
    assert!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .first()
            .is_some_and(|row| row.file.is_none()),
        "and the row no longer names bytes it will never publish"
    );
}
