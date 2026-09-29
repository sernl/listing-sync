//! Migration 0099 destroys 0.15.0's server copies of imported files and
//! nothing else.
//!
//! A server copy was a `blob` row under the digest an imported (sourced)
//! `product_file` names. Deleting the row destroys its wrapped data key, so
//! the sealed object left in the store can never be opened again. A file the
//! seller uploaded is a blob a `product_file.hash` references and must
//! survive -- including when the seller uploaded the very bytes they also
//! imported.
//!
//! The fixture is seeded on the schema before 0099, and 0099 is then run as
//! text on a connection with no tenant pinned, as `teachouse-migrate` runs it.

#![cfg(feature = "pg-tests")]

use sqlx::{Executor, PgPool};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

const DEVICE_STREAMS: &str = include_str!("../migrations/0099_device_streams.sql");
const DEVICE_STREAMS_VERSION: i64 = 99;

const ORG: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const CONNECTION: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";

/// Four digests: an upload, an import with a server copy, an import whose
/// bytes the seller also uploaded, and an uploaded cover.
const UPLOADED: &str = "\\x1111111111111111111111111111111111111111111111111111111111111111";
const COPIED: &str = "\\x2222222222222222222222222222222222222222222222222222222222222222";
const BOTH: &str = "\\x3333333333333333333333333333333333333333333333333333333333333333";
const COVER: &str = "\\x4444444444444444444444444444444444444444444444444444444444444444";
/// Imported bytes the seller also uploaded as a TPT thumbnail, and as a TPT
/// video preview: uploads, which an import never writes.
const THUMBNAIL: &str = "\\x5555555555555555555555555555555555555555555555555555555555555555";
const VIDEO: &str = "\\x6666666666666666666666666666666666666666666666666666666666666666";

#[expect(
    clippy::panic,
    reason = "a migration that will not apply is a broken fixture, not a failed assertion"
)]
async fn schema_before_device_streams(pool: &PgPool) {
    for migration in MIGRATOR
        .iter()
        .filter(|m| m.version < DEVICE_STREAMS_VERSION)
    {
        pool.execute(&*migration.sql)
            .await
            .unwrap_or_else(|error| panic!("migration {} applies: {error}", migration.version));
    }
}

fn blob(hash: &str) -> String {
    format!(
        "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, first_seen_at)
             VALUES ('{ORG}', '{hash}', 4, 'k-{}', 1, now());",
        hash.len()
    )
}

fn product(id: char) -> String {
    format!(
        "INSERT INTO product (org_id, id, title, body, body_format, price_kind, rights_state,
                              created_at, updated_at)
             VALUES ('{ORG}', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb{id}', 'Resource {id}', 'body',
                     'markdown', 'free', 'unstated', now(), now());
         INSERT INTO grade_declaration (org_id, product_id, source)
             VALUES ('{ORG}', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb{id}', 'seller');"
    )
}

fn uploaded_file(product: char, file: char, role: &str, hash: &str) -> String {
    format!(
        "INSERT INTO product_file (org_id, id, product_id, position, role, kind, hash, scan_state,
                                   created_at)
             VALUES ('{ORG}', 'dddddddd-dddd-4ddd-8ddd-ddddddddddd{file}',
                     'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb{product}', {position}, '{role}', 'pdf',
                     '{hash}', 'pending', now());",
        position = if role == "cover" { 9 } else { 0 }
    )
}

fn tpt_base(product: char, thumbnail: &str, video: &str) -> String {
    format!(
        "INSERT INTO product_tpt_base (org_id, product_id, thumbnail_mode, thumbnail_hashes,
                                       video_preview_hash, status_user, updated_at)
             VALUES ('{ORG}', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb{product}', 2,
                     ARRAY['{thumbnail}'::bytea], '{video}', 0, now());"
    )
}

fn imported_file(product: char, file: char, hash: &str) -> String {
    format!(
        "INSERT INTO product_file (org_id, id, product_id, position, role, kind, created_at,
                                   source_marketplace, source_connection, source_resource,
                                   observed_hash, observed_byte_len, asserted_scan_state,
                                   observed_by_device, observed_at, recorded_at,
                                   payload_file_name, payload_content_type)
             VALUES ('{ORG}', 'dddddddd-dddd-4ddd-8ddd-ddddddddddd{file}',
                     'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbb{product}', 1, 'payload', 'pdf', now(),
                     'tpt', '{CONNECTION}', '1234', '{hash}', 4, 'pending', 'device-1', now(),
                     now(), '1234.pdf', 'application/pdf');"
    )
}

#[expect(
    clippy::expect_used,
    reason = "a fixture that will not seed is a broken fixture, not a failed assertion"
)]
async fn seed(connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>) {
    let statements = [
        format!("SELECT set_config('app.current_org', '{ORG}', false);"),
        format!("INSERT INTO organisation (id, name, created_at) VALUES ('{ORG}', 'Shred', now());"),
        format!(
            "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at)
                 VALUES ('{ORG}', '{CONNECTION}', 'tpt', 'linked', now(), now());"
        ),
        format!(
            "INSERT INTO device (org_id, id, name, os, arch, app_version, first_seen_at,
                                 last_seen_at)
                 VALUES ('{ORG}', 'device-1', 'laptop', 'linux', 'x86_64', '0.15.0', now(), now());"
        ),
        blob(UPLOADED),
        blob(COPIED),
        blob(BOTH),
        blob(COVER),
        blob(THUMBNAIL),
        blob(VIDEO),
        "BEGIN;".to_owned(),
        product('1'),
        uploaded_file('1', '1', "payload", UPLOADED),
        uploaded_file('1', '4', "cover", COVER),
        product('2'),
        uploaded_file('2', '2', "payload", BOTH),
        imported_file('2', '3', COPIED),
        product('3'),
        imported_file('3', '5', BOTH),
        product('4'),
        imported_file('4', '6', THUMBNAIL),
        product('5'),
        imported_file('5', '7', VIDEO),
        tpt_base('4', THUMBNAIL, VIDEO),
        "COMMIT;".to_owned(),
    ];
    connection
        .as_mut()
        .execute(statements.join("\n").as_str())
        .await
        .expect("the tenant, its uploads, its imports and their copies seed");
}

#[sqlx::test(migrations = false)]
async fn only_the_copies_of_imported_files_are_shredded(pool: PgPool) {
    schema_before_device_streams(&pool).await;
    let mut connection = pool.acquire().await.expect("a connection");
    seed(&mut connection).await;

    connection
        .as_mut()
        .execute("SELECT set_config('app.current_org', '', false);")
        .await
        .expect("the tenant unpins");
    connection
        .as_mut()
        .execute(DEVICE_STREAMS)
        .await
        .expect("0099 applies with no tenant pinned");
    connection
        .as_mut()
        .execute(format!("SELECT set_config('app.current_org', '{ORG}', false);").as_str())
        .await
        .expect("the tenant pins again");

    let left: Vec<String> =
        sqlx::query_scalar("SELECT encode(hash, 'hex') FROM blob WHERE org_id = $1::uuid ORDER BY 1")
            .bind(ORG)
            .fetch_all(connection.as_mut())
            .await
            .expect("the blobs read");
    let hex = |literal: &str| literal.trim_start_matches("\\x").to_owned();
    assert_eq!(
        left,
        vec![
            hex(UPLOADED),
            hex(BOTH),
            hex(COVER),
            hex(THUMBNAIL),
            hex(VIDEO)
        ],
        "every upload survives, including bytes also imported; the import's copy does not"
    );

    let forced: Vec<bool> = sqlx::query_scalar(
        "SELECT relforcerowsecurity FROM pg_class
          WHERE relname IN ('blob', 'product_file', 'import_batch_row', 'import_run_item',
                            'product_tpt_base', 'device_stream')
          ORDER BY relname",
    )
    .fetch_all(connection.as_mut())
    .await
    .expect("the fences read");
    assert_eq!(
        forced,
        vec![true; 6],
        "every table the migration lifted is fenced again, and the new one is fenced"
    );
}
