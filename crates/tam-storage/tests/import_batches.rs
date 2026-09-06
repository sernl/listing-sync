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
    BatchState, BatchWrite, BindOutcome, CommitOpening, ImportBatchRepo, NewImportBatch,
    NewImportBatchRow, RowAddress, RowFile, RowFiles, RowIntent, RowState, UnbindOutcome,
};
use tam_types::{ContentHash, InventoryId, OrgId, Timestamp, Uuid};

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

/// Bytes this organisation has sealed, as the handle a bind names them by.
///
/// Written straight into `blob` because the upload route that seals them lives
/// in another crate; what the bind reads is this row, and the foreign key on
/// `(org_id, hash)` is what makes it a per-tenant fact.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
async fn seal(pool: &PgPool, org: OrgId, marker: u8) -> RowFile {
    let hash = vec![marker; 32];
    let mut tx = pinned(pool, org).await.expect("the pin sets");
    sqlx::query(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at) \
         VALUES ($1, $2, 9, 'k', 1, now()) ON CONFLICT DO NOTHING",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(&hash)
    .execute(&mut *tx)
    .await
    .expect("the blob writes");
    tx.commit().await.expect("the seal commits");
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&hash);
    RowFile {
        hash: ContentHash(bytes),
        kind: "pdf".to_owned(),
        byte_len: 9,
    }
}

/// How many of a batch's rows hold a cover.
///
/// Read by direct statement rather than through the repository, because the
/// column has no reader on the report: the commit is what writes it onto the
/// product, and until that route exists a test is the only thing that can say
/// the cover is there at all.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
async fn covers_held(pool: &PgPool, org: OrgId, id: Uuid) -> i64 {
    let mut tx = pinned(pool, org).await.expect("the pin sets");
    let counted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM import_batch_row \
          WHERE org_id = $1 AND batch_id = $2 AND cover_hash IS NOT NULL",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes(id.0))
    .fetch_one(&mut *tx)
    .await
    .expect("the count runs");
    tx.commit().await.expect("the read commits");
    counted
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
    drop(tx);

    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    let half_a_cover = sqlx::query(
        "UPDATE import_batch_row SET cover_kind = 'image' \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = 'TES GB' AND ordinal = 4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .execute(&mut *tx)
    .await;
    assert!(
        half_a_cover.is_err(),
        "and the cover handle is held to the same three-or-none rule as the payload's"
    );
    drop(tx);

    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    let claimed_without_an_identifier = sqlx::query(
        "UPDATE import_batch_row SET state = 'creating' \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = 'TES GB' AND ordinal = 4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .execute(&mut *tx)
    .await;
    assert!(
        claimed_without_an_identifier.is_err(),
        "a claimed row names the product identifier the claim reserved, which is the whole \
         point of the state: a resumed commit reads it rather than minting a second product"
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
/// on bytes.
///
/// Both handles, because a row holding a cover for a payload it no longer
/// names is a thumbnail of nothing, and the cover is charged storage the
/// seller can no longer see either.
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

    let payload = seal(&pool, ORG_A, 0x51).await;
    let cover = seal(&pool, ORG_A, 0x52).await;
    assert!(matches!(
        repo.bind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
        .await,
        Ok(BindOutcome::Bound(_))
    ));
    assert!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .first()
            .is_some_and(|row| row.file.is_some()),
        "the row holds bytes before the sweep runs"
    );
    assert_eq!(
        covers_held(&pool, ORG_A, BATCH_A).await,
        1,
        "and it holds the cover the upload generated beside them"
    );

    let report = ImportBatchRepo::new(engine_pool(&pool).await)
        .sweep_pass(AFTER_DEADLINE, 10, SWEPT)
        .await
        .expect("the sweep runs");
    assert_eq!(
        (report.abandoned, report.released),
        (1, 1),
        "the pass settles the batch and releases the one row that held handles"
    );
    assert!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .first()
            .is_some_and(|row| row.file.is_none()),
        "and the row no longer names bytes it will never publish"
    );
    assert_eq!(
        covers_held(&pool, ORG_A, BATCH_A).await,
        0,
        "nor the cover, which is the handle the payload's release would otherwise leave standing"
    );
}

/// The bind's two writes and the counts it answers with.
#[sqlx::test(migrations = "./migrations")]
async fn a_bind_moves_the_row_and_the_batch_and_counts_what_still_waits(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [
        row("TES GB", 4, RowIntent::Live, &held, &clean),
        row("TES GB", 5, RowIntent::Draft, &held, &clean),
        // A Teachouse row names no marketplace, so D32 asks it for no file and
        // it is never counted as waiting for one.
        row("Teachouse", 4, RowIntent::Draft, &held, &clean),
    ];
    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));

    let payload = seal(&pool, ORG_A, 0x61).await;
    let cover = seal(&pool, ORG_A, 0x62).await;
    let bound = repo
        .bind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
        .await
        .expect("the bind runs");
    let BindOutcome::Bound(bound) = bound else {
        panic!("a clean row on an open batch binds: {bound:?}");
    };
    assert_eq!(
        (
            bound.row.state,
            bound.batch_state,
            bound.counts.attached,
            bound.counts.awaiting
        ),
        (RowState::Attached, BatchState::Attaching, 1, 1),
        "one of the two marketplace rows holds bytes, one still waits, and the Teachouse row          waits for nothing"
    );
    assert_eq!(
        repo.get(ORG_A, BATCH_A)
            .await
            .expect("the read runs")
            .map(|record| record.state),
        Some(BatchState::Attaching),
        "the batch moved with the row, in one transaction, so no reader sees one without the          other"
    );
    assert_eq!(
        repo.draft_page(ORG_A, BATCH_A, None, 10)
            .await
            .expect("the drafts read")
            .into_iter()
            .find(|draft| draft.sheet == "TES GB" && draft.ordinal == 4)
            .and_then(|draft| draft.cover),
        Some(cover),
        "and the cover reaches the commit, which is the reader that writes it onto the product"
    );
}

/// Re-binding replaces, and unbinding is idempotent. Both are what "reversible
/// before the commit" means for a seller who matched the wrong file.
#[sqlx::test(migrations = "./migrations")]
async fn a_rebind_replaces_the_handle_and_an_unbind_converges(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let rows = [row("TES GB", 4, RowIntent::Live, &held, &clean)];
    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));

    let first = seal(&pool, ORG_A, 0x71).await;
    let second = seal(&pool, ORG_A, 0x72).await;
    let cover = seal(&pool, ORG_A, 0x73).await;
    for payload in [&first, &second] {
        assert!(matches!(
            repo.bind_file(
                ORG_A,
                BATCH_A,
                RowAddress {
                    sheet: "TES GB",
                    ordinal: 4,
                },
                RowFiles {
                    payload,
                    cover: &cover,
                },
            )
            .await,
            Ok(BindOutcome::Bound(_))
        ));
    }
    assert_eq!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .into_iter()
            .map(|row| row.file)
            .collect::<Vec<_>>(),
        vec![Some(second)],
        "the second bind replaced the first rather than writing a second row"
    );

    assert_eq!(
        repo.unbind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
        )
        .await
        .expect("the unbind runs"),
        UnbindOutcome::Cleared
    );
    assert_eq!(
        repo.rows(ORG_A, BATCH_A)
            .await
            .expect("the rows read")
            .into_iter()
            .map(|row| (row.state, row.file))
            .collect::<Vec<_>>(),
        vec![(RowState::Parsed, None)],
        "the row waits again"
    );
    assert_eq!(
        covers_held(&pool, ORG_A, BATCH_A).await,
        0,
        "and the cover went with it"
    );
    assert_eq!(
        repo.get(ORG_A, BATCH_A)
            .await
            .expect("the read runs")
            .map(|record| record.state),
        Some(BatchState::Attaching),
        "the batch does not walk back: a batch that has begun attaching has begun"
    );
    assert_eq!(
        repo.unbind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
        )
        .await
        .expect("the second unbind runs"),
        UnbindOutcome::Cleared,
        "clearing what is already clear converges rather than refusing"
    );
}

/// What the two writes refuse, and the fence they refuse across.
#[sqlx::test(migrations = "./migrations")]
async fn a_bind_is_refused_across_the_fence_and_on_a_row_that_cannot_take_one(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("A worksheet");
    let clean = no_problems();
    let refused = one_problem();
    let rows = [
        row("TES GB", 4, RowIntent::Live, &held, &clean),
        row("TES GB", 5, RowIntent::Live, &held, &refused),
    ];
    assert!(matches!(
        repo.create(ORG_A, &batch(BATCH_A, &rows)).await,
        Ok(BatchWrite::Saved(_))
    ));

    let payload = seal(&pool, ORG_A, 0x81).await;
    let cover = seal(&pool, ORG_A, 0x82).await;
    assert_eq!(
        repo.bind_file(
            ORG_B,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
            .await
            .expect("the bind runs"),
        BindOutcome::NoSuchRow,
        "another organisation's batch is absent rather than bindable, and its identifier is the          one thing a caller could guess"
    );
    assert_eq!(
        repo.unbind_file(
            ORG_B,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
        )
        .await
        .expect("the unbind runs"),
        UnbindOutcome::NoSuchRow,
        "nor may its rows be cleared across the fence"
    );
    assert_eq!(
        repo.bind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 9,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
        .await
        .expect("the bind runs"),
        BindOutcome::NoSuchRow,
        "a row the sheet never held is absent"
    );
    assert_eq!(
        repo.bind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 5,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
            .await
            .expect("the bind runs"),
        BindOutcome::RowClosed(RowState::Failed),
        "a row the parse refused can never be created, so binding to it would be work the          seller loses"
    );

    assert!(repo
        .abandon(ORG_A, BATCH_A, AFTER_DEADLINE, "given up on")
        .await
        .expect("the abandon runs"));
    assert_eq!(
        repo.bind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
        .await
        .expect("the bind runs"),
        BindOutcome::BatchClosed(BatchState::Abandoned),
        "a settled batch takes no more files, and the refusal names which settlement it is"
    );
    assert_eq!(
        repo.unbind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal: 4,
            },
        )
        .await
        .expect("the unbind runs"),
        UnbindOutcome::BatchClosed(BatchState::Abandoned)
    );
}

/// A second `tam_app` connection, which is what a seller with the page open in
/// two tabs actually has.
///
/// The point of `SKIP LOCKED` is a lock another transaction is holding right
/// now, and a lock cannot be held from the connection that is asking, so the
/// rival needs a pool of its own.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
async fn rival_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name reads");
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&format!(
            "postgres://tam_app:tam_dev_password@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the second app connection opens")
}

/// Two chunks running at once claim different rows.
///
/// The severity is that the rival's lock is real and held across the claim: a
/// claim written without `SKIP LOCKED` would block on it until this test's
/// timeout rather than answer, and one written without `FOR UPDATE` at all
/// would hand the same row to both chunks and create the seller's resource
/// twice.
#[sqlx::test(migrations = "./migrations")]
async fn a_claim_skips_the_rows_another_chunk_is_holding(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("One");
    let clean = no_problems();
    let rows = [
        row("TES GB", 4, RowIntent::Draft, &held, &clean),
        row("TES GB", 5, RowIntent::Draft, &held, &clean),
    ];
    repo.create(ORG_A, &batch(BATCH_A, &rows))
        .await
        .expect("the batch writes");
    let payload = seal(&pool, ORG_A, 0x31).await;
    let cover = seal(&pool, ORG_A, 0x32).await;
    for ordinal in [4, 5] {
        repo.bind_file(
            ORG_A,
            BATCH_A,
            RowAddress {
                sheet: "TES GB",
                ordinal,
            },
            RowFiles {
                payload: &payload,
                cover: &cover,
            },
        )
        .await
        .expect("the bind writes");
    }

    let rival = rival_pool(&pool).await;
    let mut holding = pinned(&rival, ORG_A).await.expect("the rival pin sets");
    sqlx::query(
        "SELECT state FROM import_batch_row \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = 'TES GB' AND ordinal = 4 \
          FOR UPDATE",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(BATCH_A.0))
    .fetch_one(&mut *holding)
    .await
    .expect("the rival takes the row");

    let claimed = repo
        .claim_page(ORG_A, BATCH_A, 10)
        .await
        .expect("the claim runs");
    assert_eq!(
        claimed.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
        vec![5],
        "the row the rival holds is left to the rival, and the other is claimed"
    );
    let Some(taken) = claimed.first() else {
        panic!("the claim answered one row");
    };
    assert!(
        taken.mapping.is_some(),
        "a row naming an inventory reserves a mapping identifier beside its product, \
         which is what import_batch_row_mapping_follows_inventory holds"
    );

    holding.rollback().await.expect("the rival lets go");
    let after = repo
        .claim_page(ORG_A, BATCH_A, 10)
        .await
        .expect("the second claim runs");
    assert_eq!(
        after.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
        vec![4, 5],
        "once the rival lets go both rows are claimable, and the one already claimed \
         comes back rather than being stranded"
    );
    let Some(again) = after.iter().find(|row| row.ordinal == 5) else {
        panic!("the second claim answered row five");
    };
    assert_eq!(
        again.product, taken.product,
        "a re-claimed row keeps the identifier reserved for it, which is what stops a \
         resumed pass minting a second product for one spreadsheet row"
    );
}

/// A commit settles to `imported` where every row created and to `failed` where
/// any did not, and the sentence counts what happened.
///
/// Two organisations rather than two batches, because one import is open per
/// organisation at a time. The rows are Teachouse rows, which name no
/// marketplace and so need no file: D32's gate is about the rows that do.
#[sqlx::test(migrations = "./migrations")]
async fn a_settle_writes_imported_with_no_failed_row_and_failed_with_one(pool: PgPool) {
    seed_org_a(&pool).await.expect("the org seeds");
    seed_org_b(&pool).await.expect("the second org seeds");
    let repo = ImportBatchRepo::new(pool.clone());
    let held = draft("One");
    let clean = no_problems();
    let rows = [
        row("Teachouse", 4, RowIntent::Draft, &held, &clean),
        row("Teachouse", 5, RowIntent::Draft, &held, &clean),
    ];
    for (org, id) in [(ORG_A, BATCH_A), (ORG_B, BATCH_B)] {
        repo.create(org, &batch(id, &rows))
            .await
            .expect("the batch writes");
        assert_eq!(
            repo.open_commit(org, id).await.expect("the commit opens"),
            CommitOpening::Open,
            "a batch of Teachouse rows needs no file to begin"
        );
        let claimed = repo.claim_page(org, id, 10).await.expect("the claim runs");
        assert_eq!(claimed.len(), 2, "both rows are claimed");
        assert!(
            claimed.iter().all(|row| row.mapping.is_none()),
            "a Teachouse row names no inventory, so it reserves no mapping"
        );
    }

    for ordinal in [4, 5] {
        assert!(
            repo.record_created(
                ORG_A,
                BATCH_A,
                RowAddress {
                    sheet: "Teachouse",
                    ordinal
                }
            )
            .await
            .expect("the breadcrumb writes"),
            "a claimed row records its create"
        );
    }
    assert_eq!(
        repo.settle(ORG_A, BATCH_A, AFTER_DEADLINE)
            .await
            .expect("the settle runs"),
        BatchState::Imported,
        "a commit with no failed row is imported"
    );
    let settled = repo
        .get(ORG_A, BATCH_A)
        .await
        .expect("the batch reads")
        .expect("the batch is held");
    assert_eq!(settled.state, BatchState::Imported);
    assert_eq!(
        settled.failure_detail, None,
        "an imported batch carries no failure, which import_batch_failure_detail also holds"
    );
    assert_eq!(settled.settled_at, Some(AFTER_DEADLINE));

    repo.record_created(
        ORG_B,
        BATCH_B,
        RowAddress {
            sheet: "Teachouse",
            ordinal: 4,
        },
    )
    .await
    .expect("the breadcrumb writes");
    assert!(
        repo.record_row_failed(
            ORG_B,
            BATCH_B,
            RowAddress {
                sheet: "Teachouse",
                ordinal: 5,
            },
            "the price is not one a listing can carry",
        )
        .await
        .expect("the refusal writes"),
        "a claimed row records its refusal"
    );
    assert_eq!(
        repo.settle(ORG_B, BATCH_B, AFTER_DEADLINE)
            .await
            .expect("the settle runs"),
        BatchState::Failed,
        "one failed row settles the batch as failed"
    );
    let failed = repo
        .get(ORG_B, BATCH_B)
        .await
        .expect("the batch reads")
        .expect("the batch is held");
    assert_eq!(
        failed.failure_detail.as_deref(),
        Some("1 of 2 rows did not create"),
        "the sentence counts what happened rather than saying only that something did"
    );
    let counts = repo
        .pending_counts(ORG_B, BATCH_B)
        .await
        .expect("the counts read");
    assert_eq!(
        (counts.outstanding, counts.created, counts.failed),
        (0, 1, 1),
        "and nothing is outstanding once every row has an outcome"
    );
}
