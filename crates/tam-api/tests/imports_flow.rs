//! The spreadsheet import over the wire: what a seller may upload, what the
//! route refuses, and what the tenant fence keeps them from reaching.
//!
//! The severity is in three places. The fence: two tenants each upload a
//! sheet, and neither reads or abandons the other's — without that, every
//! assertion about who sees what would hold trivially. The reject-whole
//! property: a sheet with one bad row among many creates no product, asserted
//! by counting products before and after rather than by trusting the route's
//! own answer. And the round trip: the workbook this API generates is uploaded
//! back to it filled, so a template the seller downloads is a template this
//! parser reads.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::catalogue::{FileHandle, UploadedView};
use tam_api::import_batch::attach::{BindBody, BoundRowView};
use tam_api::import_batch::commit::CommitAck;
use tam_api::import_batch::sheet::{columns, example_row, tab_of, Cell};
use tam_api::import_batch::{
    BatchStateView, ImportBatchDetailView, ImportsView, RowStateView, UploadedBatchView, Warning,
};
use tam_api::{router, APIError, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_domain::{CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration};
use tam_storage::{LabelRepo, MappingRepo, ProductRepo, RowAddress, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, ListingCopy, MappingId, OrgId,
    PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome, Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const MADE: Timestamp = Timestamp(1_700_000_000_000);
const KEY_A: &str = "11111111-1111-4111-8111-111111111111";
const KEY_B: &str = "22222222-2222-4222-8222-222222222222";

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || MADE,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

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
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_A, USER_A, "a@example.test", TOKEN_A),
        (ORG_B, USER_B, "b@example.test", TOKEN_B),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(
                &token,
                user,
                Timestamp(100_000_000_000_000),
                Timestamp(1_000),
            )
            .await
            .expect("the session mints");
    }
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).expect("the answer body parses")
    }
}

async fn send(pool: &PgPool, request: Request<Body>) -> Answer {
    send_with(state(pool.clone()), request).await
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(pool: &PgPool, method: Method, path: &str, token: &SessionToken) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    send(pool, request).await
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn upload(
    pool: &PgPool,
    token: &SessionToken,
    key: &str,
    name: &str,
    bytes: Vec<u8>,
) -> Answer {
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/imports?name={}", urlencode(name)))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .header("idempotency-key", key)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("the request builds");
    send(pool, request).await
}

/// Enough percent-encoding for a tab name with a space in it, which is every
/// marketplace tab.
fn urlencode(raw: &str) -> String {
    raw.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character.to_string()
            } else {
                format!("%{:02X}", character as u32)
            }
        })
        .collect()
}

/// One tab as a comma-separated document laid out the way the template writes
/// one: the header, the marker row, the example, then the rows named.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn sheet(tab_title: &str, rows: &[&[(Cell, &str)]]) -> Vec<u8> {
    let tab = tab_of(tab_title).expect("the fixture names a tab");
    let held = columns(tab);
    let field = |value: &str| -> String {
        if value.contains([',', '"']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_owned()
        }
    };
    let mut records = vec![
        held.iter()
            .map(|column| field(&column.title))
            .collect::<Vec<_>>()
            .join(","),
        held.iter()
            .map(|column| column.marker.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(","),
        example_row(tab)
            .iter()
            .map(|value| field(value))
            .collect::<Vec<_>>()
            .join(","),
    ];
    for row in rows {
        records.push(
            held.iter()
                .map(|column| {
                    field(
                        row.iter()
                            .find(|(cell, _)| *cell == column.cell)
                            .map_or("", |(_, value)| *value),
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    records.join("\r\n").into_bytes()
}

async fn products_held(pool: &PgPool, org: OrgId) -> i64 {
    counted(pool, org, "SELECT count(*) FROM product WHERE org_id = $1").await
}

/// One tab laid out as the template writes it, with every data row built from
/// that tab's own example row and then overridden.
///
/// Starting from the example rather than from blanks is what keeps a test
/// about the bind rather than about the parse: every native the registry
/// declares required is already filled with a value the tab's own vocabulary
/// holds, so a row is refused only for a reason the test wrote in.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn filled_sheet(tab_title: &str, rows: &[&[(Cell, &str)]]) -> Vec<u8> {
    let tab = tab_of(tab_title).expect("the fixture names a tab");
    let held = columns(tab);
    let example = example_row(tab);
    let field = |value: &str| -> String {
        if value.contains([',', '"']) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_owned()
        }
    };
    let mut records = vec![
        held.iter()
            .map(|column| field(&column.title))
            .collect::<Vec<_>>()
            .join(","),
        held.iter()
            .map(|column| column.marker.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(","),
        example
            .iter()
            .map(|value| field(value))
            .collect::<Vec<_>>()
            .join(","),
    ];
    for row in rows {
        records.push(
            held.iter()
                .enumerate()
                .map(|(index, column)| {
                    let value = row
                        .iter()
                        .find(|(cell, _)| *cell == column.cell)
                        .map_or_else(
                            || example.get(index).cloned().unwrap_or_default(),
                            |(_, held)| (*held).to_owned(),
                        );
                    field(&value)
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    records.join("\r\n").into_bytes()
}

/// Bytes this organisation has sealed, written straight into `blob` because
/// this test state holds no object store: the handle a bind names exists
/// because an upload sealed the bytes, and what the bind reads is the row.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seal(pool: &PgPool, org: OrgId, marker: u8) -> FileHandle {
    let hash = vec![marker; 32];
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
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
    FileHandle {
        // Thirty-two copies of one byte, so the digest is that byte's own two
        // hex characters repeated rather than a fold over the array.
        hash: format!("{marker:02x}").repeat(32),
        kind: "pdf".to_owned(),
        byte_len: 9,
        name: Some("fractions-task-cards.pdf".to_owned()),
    }
}

fn row_path(batch: Uuid, sheet: &str, ordinal: u32) -> String {
    format!(
        "/v1/imports/{}/rows/{}/{ordinal}/file",
        batch.to_hyphenated(),
        urlencode(sheet)
    )
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn bind(
    pool: &PgPool,
    token: &SessionToken,
    path: &str,
    payload: &FileHandle,
    cover: &FileHandle,
) -> Answer {
    let body = serde_json::to_vec(&BindBody {
        payload: payload.clone(),
        cover: cover.clone(),
    })
    .expect("the bind body serialises");
    let request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("the request builds");
    send(pool, request).await
}

/// The one row of a batch the test is about, read back from the batch view
/// rather than from the answer that wrote it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn read_row(
    pool: &PgPool,
    token: &SessionToken,
    batch: Uuid,
    ordinal: u32,
) -> (BatchStateView, RowStateView, bool) {
    let view: ImportBatchDetailView = call(
        pool,
        Method::GET,
        &format!("/v1/imports/{}", batch.to_hyphenated()),
        token,
    )
    .await
    .json();
    let row = view
        .rows
        .into_iter()
        .find(|row| row.ordinal == ordinal)
        .expect("the batch holds the row the test named");
    (view.batch.state, row.state, row.file_attached)
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_template_is_a_workbook_named_for_download(pool: PgPool) {
    provision(&pool).await;
    let answer = call(&pool, Method::GET, "/v1/imports/template", &TOKEN_A).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(
        answer.body.starts_with(b"PK"),
        "an xlsx is a zip, so the bytes open with the local file header signature"
    );
}

/// The property the whole reject-whole model rests on, asserted by counting
/// products rather than by believing the route: a sheet with one bad row among
/// many creates nothing at all.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sheet_with_one_bad_row_among_many_creates_no_product(pool: PgPool) {
    provision(&pool).await;
    let before = products_held(&pool, ORG_A).await;
    let document = sheet(
        "Teachouse",
        &[
            &[(Cell::Title, "One"), (Cell::Price, "free")],
            &[(Cell::Title, "Two"), (Cell::Price, "free")],
            // The bad one: no title.
            &[(Cell::Price, "free")],
            &[(Cell::Title, "Four"), (Cell::Price, "free")],
        ],
    );
    let answer = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "the batch is written and the report is the answer: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let view: UploadedBatchView = answer.json();
    assert_eq!(view.detail.batch.row_count, 4);
    assert_eq!(
        view.detail.batch.failed_count, 1,
        "one row of four was refused"
    );
    assert_eq!(
        view.detail
            .rows
            .iter()
            .filter(|row| row.state == RowStateView::Failed)
            .map(|row| row.ordinal)
            .collect::<Vec<_>>(),
        vec![6],
        "the refusal cites the seller's own spreadsheet row number, which is the off-by-one \
         this feature would otherwise ship"
    );
    assert_eq!(
        products_held(&pool, ORG_A).await,
        before,
        "an upload creates nothing; the commit is a second, deliberate action"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_upload_states_its_deadline_and_the_labels_it_would_create(pool: PgPool) {
    provision(&pool).await;
    let document = sheet(
        "Teachouse",
        &[
            &[
                (Cell::Title, "One"),
                (Cell::Price, "free"),
                (Cell::Labels, "Autumn Term"),
            ],
            &[
                (Cell::Title, "Two"),
                (Cell::Price, "free"),
                (Cell::Labels, "Autumn Term; Autum Term"),
            ],
        ],
    );
    let answer = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document).await;
    assert_eq!(answer.status, StatusCode::CREATED);
    let view: UploadedBatchView = answer.json();
    assert_eq!(view.detail.batch.state, BatchStateView::Parsed);
    assert!(
        view.detail.batch.expires_at.0 > view.detail.batch.created_at.0,
        "the batch states the deadline the sweep will settle it at, so the console can show it"
    );
    assert_eq!(
        view.detail.warnings,
        vec![
            Warning::NewLabel {
                name: "Autumn Term".to_owned(),
                rows: 2
            },
            Warning::NewLabel {
                name: "Autum Term".to_owned(),
                rows: 1
            },
        ],
        "a one-row label beside a near-identical two-row one is the typo signature, and the \
         seller reads it before pressing import"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_import_is_open_at_a_time_and_abandoning_releases_the_hold(pool: PgPool) {
    provision(&pool).await;
    let document = || {
        sheet(
            "Teachouse",
            &[&[(Cell::Title, "One"), (Cell::Price, "free")]],
        )
    };

    let first = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document()).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let opened: UploadedBatchView = first.json();

    let second = upload(&pool, &TOKEN_A, KEY_B, "Teachouse.csv", document()).await;
    assert_eq!(
        second.status,
        StatusCode::CONFLICT,
        "a second open import is refused rather than silently replacing the first"
    );
    let error: APIError = second.json();
    let detail = error
        .errors
        .first()
        .and_then(|entry| entry.detail.clone())
        .and_then(|detail| detail.get("open_batch").cloned());
    assert_eq!(
        detail,
        Some(serde_json::json!(opened.detail.batch.id)),
        "the refusal names the open import so the console can link to it: {detail:?}"
    );

    let replayed = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document()).await;
    assert_eq!(
        replayed.status,
        StatusCode::OK,
        "the same idempotency key is the same import, which is what a double-clicked submit \
         sends"
    );
    let replayed: UploadedBatchView = replayed.json();
    assert!(!replayed.created, "and it reports that it created nothing");
    assert_eq!(replayed.detail.batch.id, opened.detail.batch.id);

    let abandoned = call(
        &pool,
        Method::DELETE,
        &format!("/v1/imports/{}", opened.detail.batch.id.to_hyphenated()),
        &TOKEN_A,
    )
    .await;
    assert_eq!(abandoned.status, StatusCode::NO_CONTENT);
    let third = upload(&pool, &TOKEN_A, KEY_B, "Teachouse.csv", document()).await;
    assert_eq!(
        third.status,
        StatusCode::CREATED,
        "and the next import is admitted once the first is closed"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn neither_tenant_reads_or_abandons_the_others_import(pool: PgPool) {
    provision(&pool).await;
    let document = || {
        sheet(
            "Teachouse",
            &[&[(Cell::Title, "One"), (Cell::Price, "free")]],
        )
    };
    // Named for the tab rather than for the tenant: a comma-separated file
    // carries one tab and no tab name, so the filename is the name, and
    // "mine.csv" is refused by the parser rather than read as a Teachouse tab.
    let mine: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document())
        .await
        .json();
    let theirs: UploadedBatchView = upload(&pool, &TOKEN_B, KEY_B, "Teachouse.csv", document())
        .await
        .json();

    let listed: ImportsView = call(&pool, Method::GET, "/v1/imports", &TOKEN_A)
        .await
        .json();
    assert_eq!(
        listed
            .imports
            .iter()
            .map(|held| held.id)
            .collect::<Vec<_>>(),
        vec![mine.detail.batch.id],
        "a listing holds this organisation's imports and no other's"
    );
    assert_eq!(
        listed.open,
        Some(mine.detail.batch.id),
        "and it names the open one, which is what the Import page's card keys on"
    );

    let across = call(
        &pool,
        Method::GET,
        &format!("/v1/imports/{}", theirs.detail.batch.id.to_hyphenated()),
        &TOKEN_A,
    )
    .await;
    assert_eq!(
        across.status,
        StatusCode::NOT_FOUND,
        "another organisation's import is absent rather than readable, and its identifier is \
         the one thing a caller could guess"
    );
    let settled = call(
        &pool,
        Method::DELETE,
        &format!("/v1/imports/{}", theirs.detail.batch.id.to_hyphenated()),
        &TOKEN_A,
    )
    .await;
    assert_eq!(
        settled.status,
        StatusCode::NOT_FOUND,
        "nor may it be abandoned across the fence"
    );

    let still: ImportBatchDetailView = call(
        &pool,
        Method::GET,
        &format!("/v1/imports/{}", theirs.detail.batch.id.to_hyphenated()),
        &TOKEN_B,
    )
    .await
    .json();
    assert_eq!(
        still.batch.state,
        BatchStateView::Parsed,
        "and the other tenant's import is still open afterwards"
    );
}

/// The round trip the whole feature rests on: the workbook this API generates
/// is filled and uploaded back to it. A template a seller downloads has to be
/// a template this parser reads, and both sides derive from one column model.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_generated_workbook_is_filled_and_uploaded_back(pool: PgPool) {
    provision(&pool).await;
    let downloaded = call(&pool, Method::GET, "/v1/imports/template", &TOKEN_A).await;
    assert_eq!(downloaded.status, StatusCode::OK);

    let filled = fill();
    let answer = upload(&pool, &TOKEN_A, KEY_A, "teachouse-import.xlsx", filled).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "the workbook this API wrote reads back through its own parser: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let view: UploadedBatchView = answer.json();
    assert_eq!(
        (view.detail.batch.row_count, view.detail.batch.failed_count),
        (1, 0),
        "the one row typed into the downloaded workbook is read and passes"
    );
    assert_eq!(
        view.detail.rows.first().map(|row| row.sheet.clone()),
        Some("Teachouse".to_owned()),
        "and it is read off the tab it was typed on"
    );
}

/// A workbook laid out as the template writes one, with one row typed onto the
/// platform-only tab. Written with the same crate the generator uses, so the
/// fixture cannot drift from the real file's shape.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn fill() -> Vec<u8> {
    use rust_xlsxwriter::{Workbook, Worksheet};
    let mut book = Workbook::new();
    for tab in tam_api::import_batch::sheet::TABS {
        let mut worksheet = Worksheet::new();
        worksheet.set_name(tab.title).expect("the tab names");
        let held = columns(tab);
        for (index, column) in held.iter().enumerate() {
            let at = u16::try_from(index).unwrap_or(u16::MAX);
            worksheet
                .write_string(0, at, &column.title)
                .expect("the header writes");
            worksheet
                .write_string(1, at, column.marker.as_str())
                .expect("the marker writes");
            if let Some(cell) = example_row(tab).get(index) {
                worksheet
                    .write_string(2, at, cell)
                    .expect("the example writes");
            }
            if tab.title == "Teachouse" {
                let value = match column.cell {
                    Cell::Title => "A planning grid",
                    Cell::Price => "free",
                    Cell::Status => "draft",
                    Cell::ResourceId
                    | Cell::Description
                    | Cell::Labels
                    | Cell::File
                    | Cell::Currency
                    | Cell::Native(_) => "",
                };
                if !value.is_empty() {
                    worksheet
                        .write_string(3, at, value)
                        .expect("the row writes");
                }
            }
        }
        book.push_worksheet(worksheet);
    }
    book.save_to_buffer().expect("the fixture workbook writes")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_sheet_whose_shape_is_wrong_writes_no_batch_at_all(pool: PgPool) {
    provision(&pool).await;
    let document = sheet(
        "Teachouse",
        &[&[(Cell::Title, "One"), (Cell::Price, "free")]],
    );
    let text = String::from_utf8_lossy(&document).replacen("Title,", "", 1);
    let answer = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", text.into_bytes()).await;
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a sheet missing a column is refused before any batch exists"
    );
    let listed: ImportsView = call(&pool, Method::GET, "/v1/imports", &TOKEN_A)
        .await
        .json();
    assert!(
        listed.imports.is_empty(),
        "and no batch is left behind for the seller to abandon"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_upload_without_an_idempotency_key_is_refused(pool: PgPool) {
    provision(&pool).await;
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/imports?name=Teachouse.csv")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", TOKEN_A.to_hex()),
        )
        .body(Body::from(sheet(
            "Teachouse",
            &[&[(Cell::Title, "One"), (Cell::Price, "free")]],
        )));
    let Ok(request) = request else {
        panic!("the request builds");
    };
    let answer = send(&pool, request).await;
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the key is what makes a double-clicked submit one import"
    );
}

/// The bind, its counts, and the two states it moves.
///
/// The counts are the panel's own readout and the commit's own gate, so this
/// asserts them from the answer and then reads the batch back rather than
/// trusting either: a route that answered the right numbers and wrote nothing
/// would pass on the answer alone.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bind_holds_one_row_and_moves_the_batch_to_attaching(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet(
        "TES",
        &[
            &[(Cell::Title, "One"), (Cell::File, "one.pdf")],
            &[(Cell::Title, "Two"), (Cell::File, "two.pdf")],
        ],
    );
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    assert_eq!(
        uploaded.detail.batch.failed_count, 0,
        "the fixture is about the bind, so the parse takes both rows: {:?}",
        uploaded.detail.rows
    );
    let batch = uploaded.detail.batch.id;

    let payload = seal(&pool, ORG_A, 0x11).await;
    let cover = seal(&pool, ORG_A, 0x12).await;
    let answer = bind(
        &pool,
        &TOKEN_A,
        &row_path(batch, "TES", 4),
        &payload,
        &cover,
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the bind names bytes this organisation holds: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let bound: BoundRowView = answer.json();
    assert_eq!(
        (
            bound.row.state,
            bound.row.file_attached,
            bound.batch_state,
            bound.attached,
            bound.awaiting
        ),
        (
            RowStateView::Attached,
            true,
            BatchStateView::Attaching,
            1,
            1
        ),
        "one of the two marketplace rows now holds bytes and one still waits"
    );

    assert_eq!(
        read_row(&pool, &TOKEN_A, batch, 4).await,
        (BatchStateView::Attaching, RowStateView::Attached, true),
        "and the batch read back says the same, which is what a refreshed page reads"
    );
    assert_eq!(
        read_row(&pool, &TOKEN_A, batch, 5).await,
        (BatchStateView::Attaching, RowStateView::Parsed, false),
        "the row nobody bound is untouched"
    );
}

/// Re-binding replaces. This is what "every binding is reversible before the
/// commit" means for a seller who matched the wrong file to a row.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_second_bind_on_one_row_replaces_the_handle(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("TES", &[&[(Cell::Title, "One"), (Cell::File, "one.pdf")]]);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    let batch = uploaded.detail.batch.id;
    let path = row_path(batch, "TES", 4);

    let first = seal(&pool, ORG_A, 0x21).await;
    let second = seal(&pool, ORG_A, 0x22).await;
    let cover = seal(&pool, ORG_A, 0x23).await;
    assert_eq!(
        bind(&pool, &TOKEN_A, &path, &first, &cover).await.status,
        StatusCode::OK
    );
    let answer = bind(&pool, &TOKEN_A, &path, &second, &cover).await;
    assert_eq!(answer.status, StatusCode::OK);
    let bound: BoundRowView = answer.json();
    assert_eq!(
        (bound.attached, bound.awaiting),
        (1, 0),
        "the second bind replaced the first rather than binding a second row"
    );

    let view: ImportBatchDetailView = call(
        &pool,
        Method::GET,
        &format!("/v1/imports/{}", batch.to_hyphenated()),
        &TOKEN_A,
    )
    .await
    .json();
    assert_eq!(
        view.rows.len(),
        1,
        "and the batch still holds the one row the sheet named"
    );
}

/// The unbind, and its second delivery.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unbind_returns_the_row_to_parsed_and_answers_twice(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("TES", &[&[(Cell::Title, "One"), (Cell::File, "one.pdf")]]);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    let batch = uploaded.detail.batch.id;
    let path = row_path(batch, "TES", 4);
    let payload = seal(&pool, ORG_A, 0x31).await;
    let cover = seal(&pool, ORG_A, 0x32).await;
    assert_eq!(
        bind(&pool, &TOKEN_A, &path, &payload, &cover).await.status,
        StatusCode::OK
    );

    let cleared = call(&pool, Method::DELETE, &path, &TOKEN_A).await;
    assert_eq!(cleared.status, StatusCode::NO_CONTENT);
    assert_eq!(
        read_row(&pool, &TOKEN_A, batch, 4).await,
        (BatchStateView::Attaching, RowStateView::Parsed, false),
        "the row waits again, and the batch does not walk back with it: a batch that has begun \
         attaching has begun"
    );

    let again = call(&pool, Method::DELETE, &path, &TOKEN_A).await;
    assert_eq!(
        again.status,
        StatusCode::NO_CONTENT,
        "clearing what is already clear is what a double-clicked remove sends"
    );
}

/// A handle naming bytes nobody uploaded, which is the one refusal that keeps
/// this route from minting a hold on an object that does not exist.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bind_naming_bytes_this_organisation_never_uploaded_is_refused(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("TES", &[&[(Cell::Title, "One"), (Cell::File, "one.pdf")]]);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    let batch = uploaded.detail.batch.id;
    let cover = seal(&pool, ORG_A, 0x41).await;
    let fabricated = FileHandle {
        hash: "ab".repeat(32),
        kind: "pdf".to_owned(),
        byte_len: 9,
        name: Some("nothing.pdf".to_owned()),
    };

    let answer = bind(
        &pool,
        &TOKEN_A,
        &row_path(batch, "TES", 4),
        &fabricated,
        &cover,
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = answer.json();
    assert_eq!(
        error.errors[0].code,
        Some(tam_api::APIErrorCode::UploadRejected),
        "the refusal is the create route's own code, because it is the create route's own check"
    );
    assert_eq!(
        read_row(&pool, &TOKEN_A, batch, 4).await,
        (BatchStateView::Parsed, RowStateView::Parsed, false),
        "and the row is unchanged, read back rather than trusted"
    );
}

/// Bytes another tenant sealed are bytes this one has not uploaded.
///
/// The severe half of the fence: a hash is guessable in a way a batch
/// identifier is not, so a handle naming a blob row that exists under a
/// different `org_id` is the shape a cross-tenant read would take.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bind_naming_the_other_tenants_bytes_is_refused(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("TES", &[&[(Cell::Title, "One"), (Cell::File, "one.pdf")]]);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    let batch = uploaded.detail.batch.id;
    let theirs = seal(&pool, ORG_B, 0x51).await;
    let mine = seal(&pool, ORG_A, 0x52).await;

    let answer = bind(&pool, &TOKEN_A, &row_path(batch, "TES", 4), &theirs, &mine).await;
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bytes are content-addressed per tenant, so another organisation's blob is absent"
    );
    assert_eq!(
        read_row(&pool, &TOKEN_A, batch, 4).await,
        (BatchStateView::Parsed, RowStateView::Parsed, false),
        "and nothing was written"
    );
}

/// The fence over the two new routes, driven by the two sessions already
/// provisioned.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn neither_tenant_binds_or_unbinds_the_others_row(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("TES", &[&[(Cell::Title, "One"), (Cell::File, "one.pdf")]]);
    let theirs: UploadedBatchView = upload(&pool, &TOKEN_B, KEY_B, "TES.csv", document)
        .await
        .json();
    let batch = theirs.detail.batch.id;
    let path = row_path(batch, "TES", 4);
    let payload = seal(&pool, ORG_A, 0x61).await;
    let cover = seal(&pool, ORG_A, 0x62).await;

    assert_eq!(
        bind(&pool, &TOKEN_A, &path, &payload, &cover).await.status,
        StatusCode::NOT_FOUND,
        "another organisation's import is absent rather than bindable"
    );
    assert_eq!(
        call(&pool, Method::DELETE, &path, &TOKEN_A).await.status,
        StatusCode::NOT_FOUND,
        "nor may its rows be unbound across the fence"
    );
    assert_eq!(
        read_row(&pool, &TOKEN_B, batch, 4).await,
        (BatchStateView::Parsed, RowStateView::Parsed, false),
        "and the batch that owns the row is untouched by either attempt"
    );
}

/// A row the parse refused can never be created, so binding to it would be
/// work the seller loses.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_row_the_parse_refused_takes_no_file(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet(
        "TES",
        &[
            &[(Cell::Title, "One"), (Cell::File, "one.pdf")],
            // No title, which the parse refuses by name.
            &[(Cell::Title, ""), (Cell::File, "two.pdf")],
        ],
    );
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    assert_eq!(
        uploaded.detail.batch.failed_count, 1,
        "the fixture's second row is refused: {:?}",
        uploaded.detail.rows
    );
    let batch = uploaded.detail.batch.id;
    let payload = seal(&pool, ORG_A, 0x71).await;
    let cover = seal(&pool, ORG_A, 0x72).await;

    let answer = bind(
        &pool,
        &TOKEN_A,
        &row_path(batch, "TES", 5),
        &payload,
        &cover,
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        read_row(&pool, &TOKEN_A, batch, 5).await,
        (BatchStateView::Parsed, RowStateView::Failed, false),
        "the refused row stays refused and holds no bytes"
    );
}

/// A settled batch takes no more files, and says which settlement it is.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_settled_batch_takes_no_more_files(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("TES", &[&[(Cell::Title, "One"), (Cell::File, "one.pdf")]]);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    let batch = uploaded.detail.batch.id;
    let path = row_path(batch, "TES", 4);
    assert_eq!(
        call(
            &pool,
            Method::DELETE,
            &format!("/v1/imports/{}", batch.to_hyphenated()),
            &TOKEN_A
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );

    let payload = seal(&pool, ORG_A, 0x81).await;
    let cover = seal(&pool, ORG_A, 0x82).await;
    let answer = bind(&pool, &TOKEN_A, &path, &payload, &cover).await;
    assert_eq!(answer.status, StatusCode::CONFLICT);
    let error: APIError = answer.json();
    assert_eq!(
        error.errors[0]
            .detail
            .as_ref()
            .and_then(|detail| detail.get("batch_state"))
            .and_then(serde_json::Value::as_str),
        Some("abandoned"),
        "the refusal names the state, because what the seller does next differs by which it is"
    );
    assert_eq!(
        call(&pool, Method::DELETE, &path, &TOKEN_A).await.status,
        StatusCode::CONFLICT,
        "and the unbind is refused the same way rather than clearing a settled batch's row"
    );
}

/// A store root of this test's own, under the directory cargo gives an
/// integration binary for exactly this.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("imports-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("the store root is creatable");
    root
}

/// A deployment that can seal bytes, which the commit needs and the bind does
/// not.
///
/// The create writes a cover, and the create route reads a cover's bytes back
/// to prove they are a picture before it stores the role grant. So a commit on
/// a deployment with no key-encryption key and no object-store root refuses
/// with 503 rather than creating, and a test of the commit that ran without
/// one would be testing that refusal.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn sealing(pool: PgPool, root: &std::path::Path) -> AppState {
    AppState {
        blobs: Some(BlobStore {
            kek: tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
            root: root.to_path_buf(),
        }),
        ..state(pool)
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn send_with(state: AppState, request: Request<Body>) -> Answer {
    let response = router(state)
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer { status, body }
}

/// Bytes through the real upload route, which is where a cover comes from.
///
/// `archive=keep_whole` on every tab, because a row holds one payload handle
/// and an exploded archive answers several.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn sealed(state: AppState, token: &SessionToken, marker: &str) -> UploadedView {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(marker.as_bytes());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/uploads?archive=keep_whole")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .expect("the request builds");
    let answer = send_with(state, request).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "the upload is accepted: {}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn commit(state: AppState, token: &SessionToken, batch: Uuid) -> Answer {
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/imports/{}/commit", batch.to_hyphenated()))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    send_with(state, request).await
}

/// A count of one tenant's rows, read with the tenant pin set.
///
/// The pin is the whole helper. `product`, `mapping` and `job` all force
/// row-level security under `tam_app`, so an unpinned count answers zero
/// whatever the table holds — which reads as "nothing was created" and passes
/// an assertion that nothing was created without looking at anything.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn counted(pool: &PgPool, org: OrgId, statement: &str) -> i64 {
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let held: i64 = sqlx::query_scalar(statement)
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .fetch_one(&mut *tx)
        .await
        .expect("the count runs");
    tx.commit().await.expect("the read commits");
    held
}

async fn mappings_held(pool: &PgPool, org: OrgId) -> i64 {
    counted(pool, org, "SELECT count(*) FROM mapping WHERE org_id = $1").await
}

/// Jobs this organisation holds, which is the D1 assertion of this slice: the
/// commit creates resources and enqueues nothing, so a live row reaches
/// `created` and publishing happens later, on the seller's own device.
async fn jobs_held(pool: &PgPool, org: OrgId) -> i64 {
    counted(pool, org, "SELECT count(*) FROM job WHERE org_id = $1").await
}

/// The whole report, so a test can read one row's outcome and its product.
async fn report(pool: &PgPool, token: &SessionToken, batch: Uuid) -> ImportBatchDetailView {
    call(
        pool,
        Method::GET,
        &format!("/v1/imports/{}", batch.to_hyphenated()),
        token,
    )
    .await
    .json()
}

/// D32's gate, which is the founder's "once every live row has a file"
/// generalised to every marketplace row: the commit refuses rather than
/// importing the rows that are ready and leaving the rest.
///
/// The count and the rows are asserted because a refusal that says only "some
/// row is missing a file" sends the seller back to read a five-hundred-line
/// report to find out which.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_commit_is_refused_while_a_marketplace_row_holds_no_file(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet(
        "TES",
        &[
            &[(Cell::Title, "One"), (Cell::File, "one.pdf")],
            &[(Cell::Title, "Two"), (Cell::File, "two.pdf")],
        ],
    );
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    assert_eq!(uploaded.detail.batch.failed_count, 0, "both rows parse");
    let batch = uploaded.detail.batch.id;

    let payload = seal(&pool, ORG_A, 0x91).await;
    let cover = seal(&pool, ORG_A, 0x92).await;
    assert_eq!(
        bind(
            &pool,
            &TOKEN_A,
            &row_path(batch, "TES", 4),
            &payload,
            &cover
        )
        .await
        .status,
        StatusCode::OK
    );

    let before = products_held(&pool, ORG_A).await;
    let answer = commit(state(pool.clone()), &TOKEN_A, batch).await;
    assert_eq!(answer.status, StatusCode::CONFLICT);
    let error: APIError = answer.json();
    let detail = error.errors[0]
        .detail
        .clone()
        .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        detail.get("awaiting").and_then(serde_json::Value::as_u64),
        Some(1),
        "one row still waits, and the refusal counts it: {detail}"
    );
    assert_eq!(
        detail.get("rows"),
        Some(&serde_json::json!([{ "sheet": "TES", "ordinal": 5 }])),
        "and names it, so the panel can point at the row rather than the report"
    );
    assert_eq!(
        products_held(&pool, ORG_A).await,
        before,
        "nothing was created: the gate runs before the first row is claimed"
    );
    assert_eq!(
        report(&pool, &TOKEN_A, batch).await.batch.state,
        BatchStateView::Attaching,
        "and the batch is still taking files rather than having been moved to importing"
    );
}

/// The whole commit: what it creates, what it leaves alone, and what it does
/// not enqueue.
///
/// Three assertions carry it. Products and mappings are counted before and
/// after rather than read off the route's own answer. The job table is counted
/// the same way, which is D1 in this slice: a live row becomes a resource here
/// and reaches a marketplace only from the seller's own device. And the row the
/// parse refused is read back to show it was never created.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_commit_creates_one_resource_per_passing_row_with_its_labels(pool: PgPool) {
    provision(&pool).await;
    let root = store_root("commit");
    let sealing = sealing(pool.clone(), &root);
    let document = filled_sheet(
        "TES",
        &[
            &[
                (Cell::Status, "live"),
                (Cell::Title, "Fractions task cards"),
                (Cell::Labels, "Autumn Term; Year 5"),
                (Cell::File, "one.pdf"),
            ],
            // No title, which the parse refuses by name.
            &[(Cell::Title, ""), (Cell::File, "two.pdf")],
        ],
    );
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    assert_eq!(
        uploaded.detail.batch.failed_count, 1,
        "the fixture's second row is refused: {:?}",
        uploaded.detail.rows
    );
    let batch = uploaded.detail.batch.id;

    let bytes = sealed(sealing.clone(), &TOKEN_A, "one").await;
    let Some(payload) = bytes.payload.first() else {
        panic!("a single pdf is its own payload file");
    };
    assert_eq!(
        bytes.cover.kind, "image",
        "the upload generated the cover the create writes"
    );
    assert_eq!(
        bind(
            &pool,
            &TOKEN_A,
            &row_path(batch, "TES", 4),
            payload,
            &bytes.cover
        )
        .await
        .status,
        StatusCode::OK
    );

    let products_before = products_held(&pool, ORG_A).await;
    let mappings_before = mappings_held(&pool, ORG_A).await;
    assert_eq!(
        jobs_held(&pool, ORG_A).await,
        0,
        "no job exists before the commit"
    );

    let answer = commit(sealing, &TOKEN_A, batch).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the commit runs: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let ack: CommitAck = answer.json();
    assert_eq!(
        (
            ack.applied,
            ack.skipped,
            ack.failed,
            ack.total,
            ack.remaining
        ),
        (1, 0, 0, 1, 0),
        "one row was created and it was the only row the parse accepted"
    );
    assert!(ack.complete, "and the batch is finished in one chunk");
    assert_eq!(
        ack.batch_state,
        BatchStateView::Imported,
        "a commit with no failed row settles as imported, whatever the parse refused"
    );

    assert_eq!(
        products_held(&pool, ORG_A).await - products_before,
        1,
        "one product per passing row, counted rather than taken on the route's word"
    );
    assert_eq!(
        mappings_held(&pool, ORG_A).await - mappings_before,
        1,
        "and one mapping per grid row"
    );
    assert_eq!(
        jobs_held(&pool, ORG_A).await,
        0,
        "and no job: a live row is created here and published from the seller's own device"
    );

    let rows = report(&pool, &TOKEN_A, batch).await.rows;
    let Some(created) = rows.iter().find(|row| row.ordinal == 4) else {
        panic!("the report holds the created row");
    };
    assert_eq!(created.state, RowStateView::Created);
    let Some(product) = created.product else {
        panic!("a created row names the resource it became");
    };
    let Some(refused) = rows.iter().find(|row| row.ordinal == 5) else {
        panic!("the report holds the refused row");
    };
    assert_eq!(
        (refused.state, refused.product),
        (RowStateView::Failed, None),
        "a row the parse refused is never created"
    );

    let labels: Vec<String> = LabelRepo::new(pool.clone())
        .for_product(ORG_A, product)
        .await
        .expect("the labels read")
        .into_iter()
        .map(|record| record.name)
        .collect();
    assert_eq!(
        labels,
        vec!["Autumn Term".to_owned(), "Year 5".to_owned()],
        "the sheet's labels are on the resource it created"
    );

    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let binding: (String, String) = sqlx::query_as(
        "SELECT binding_state, publish_mode FROM mapping WHERE org_id = $1 AND product_id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(product.0 .0))
    .fetch_one(&mut *tx)
    .await
    .expect("the mapping reads");
    tx.commit().await.expect("the read commits");
    assert_eq!(
        binding,
        ("unbound".to_owned(), "dry_run".to_owned()),
        "the mapping starts exactly where a create form's own would"
    );
}

/// D32's other half: a row on the platform-only tab needs no file, is created,
/// and gets no mapping, because it names no marketplace to be mapped to.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_teachouse_row_needs_no_file_and_gets_no_mapping(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("Teachouse", &[&[(Cell::Title, "A revision pack")]]);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document)
        .await
        .json();
    assert_eq!(
        uploaded.detail.batch.failed_count, 0,
        "a Teachouse row with no file is a clean row: {:?}",
        uploaded.detail.rows
    );
    let batch = uploaded.detail.batch.id;

    let answer = commit(state(pool.clone()), &TOKEN_A, batch).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "no file is needed and no blob store either, because no cover is written: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let ack: CommitAck = answer.json();
    assert_eq!((ack.applied, ack.failed), (1, 0));
    assert_eq!(products_held(&pool, ORG_A).await, 1);
    assert_eq!(
        mappings_held(&pool, ORG_A).await,
        0,
        "a resource kept on Teachouse is mapped to nothing"
    );
}

/// The resumability property, in the shape a closed browser makes: the commit
/// is chunked, and a batch committed one chunk at a time and then asked again
/// creates no second product for any row.
///
/// Platform-only rows, because this test is about the chunking rather than
/// about files: thirty of them exceed one chunk, which is the whole point, and
/// none of them needs bytes.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_chunked_commit_finishes_without_creating_a_second_product(pool: PgPool) {
    provision(&pool).await;
    let titles: Vec<String> = (0..30).map(|index| format!("Pack {index}")).collect();
    let rows: Vec<Vec<(Cell, &str)>> = titles
        .iter()
        .map(|title| vec![(Cell::Title, title.as_str())])
        .collect();
    let borrowed: Vec<&[(Cell, &str)]> = rows.iter().map(Vec::as_slice).collect();
    let document = filled_sheet("Teachouse", &borrowed);
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document)
        .await
        .json();
    assert_eq!(uploaded.detail.batch.row_count, 30);
    let batch = uploaded.detail.batch.id;

    let first: CommitAck = commit(state(pool.clone()), &TOKEN_A, batch).await.json();
    assert_eq!(
        (first.applied, first.remaining, first.complete),
        (25, 5, false),
        "one chunk is a page rather than the whole batch, and the answer says what is left"
    );
    assert_eq!(
        first.batch_state,
        BatchStateView::Importing,
        "and the batch says a commit is in flight, which is what a reopened page reads"
    );

    let second: CommitAck = commit(state(pool.clone()), &TOKEN_A, batch).await.json();
    assert_eq!(
        (
            second.applied,
            second.skipped,
            second.remaining,
            second.complete
        ),
        (5, 0, 0, true),
        "the next chunk finishes the batch and creates nothing it already created"
    );
    assert_eq!(second.batch_state, BatchStateView::Imported);
    assert_eq!(
        products_held(&pool, ORG_A).await,
        30,
        "one product per row and no duplicate, counted rather than trusted"
    );
    assert_eq!(jobs_held(&pool, ORG_A).await, 0, "and still no job");

    assert_eq!(
        commit(state(pool.clone()), &TOKEN_A, batch).await.status,
        StatusCode::CONFLICT,
        "a settled batch creates nothing more, however many times the button is pressed"
    );
    assert_eq!(
        products_held(&pool, ORG_A).await,
        30,
        "and the refused re-commit created nothing"
    );
}

/// A pass killed between the claim and the create, which is the state the
/// reserved identifier exists for.
///
/// Both halves are driven: a row left `creating` whose product was never
/// written is created under the identifier already reserved for it, and a row
/// left `creating` whose product *was* written is recorded rather than created
/// again. Without the reservation the second case mints a second charged
/// product for one spreadsheet row.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_row_left_creating_is_finished_under_the_identifier_it_reserved(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet(
        "Teachouse",
        &[&[(Cell::Title, "One")], &[(Cell::Title, "Two")]],
    );
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "Teachouse.csv", document)
        .await
        .json();
    let batch = uploaded.detail.batch.id;

    // The crash: identifiers reserved, no product written, the batch already
    // moved to importing. Written by statement rather than by the route,
    // because the route never leaves this state on purpose.
    let reserved = uuid::Uuid::from_bytes([0xC1; 16]);
    claim_by_hand(&pool, batch, 4, reserved).await;

    let first: CommitAck = commit(state(pool.clone()), &TOKEN_A, batch).await.json();
    assert_eq!(
        (first.applied, first.skipped, first.failed, first.complete),
        (2, 0, 0, true),
        "the next chunk finishes the stranded row alongside the untouched one"
    );
    assert_eq!(products_held(&pool, ORG_A).await, 2);
    let rows = report(&pool, &TOKEN_A, batch).await.rows;
    let Some(stranded) = rows.iter().find(|row| row.ordinal == 4) else {
        panic!("the report holds the stranded row");
    };
    assert_eq!(
        stranded.product.map(|id| uuid::Uuid::from_bytes(id.0 .0)),
        Some(reserved),
        "the row was finished under the identifier reserved for it, not a fresh one"
    );

    // The other half: the product exists and the breadcrumb does not, which is
    // a pass killed a moment later.
    claim_by_hand(&pool, batch, 4, reserved).await;
    let second: CommitAck = commit(state(pool.clone()), &TOKEN_A, batch).await.json();
    assert_eq!(
        (second.applied, second.skipped, second.failed),
        (0, 1, 0),
        "a row whose reserved product already exists is recorded rather than created again"
    );
    assert_eq!(
        products_held(&pool, ORG_A).await,
        2,
        "so no second product was minted for one spreadsheet row"
    );
}

/// Puts one Teachouse row back into the state a killed pass leaves it in, and
/// the batch with it.
async fn claim_by_hand(pool: &PgPool, batch: Uuid, ordinal: u32, product: uuid::Uuid) {
    reserve_by_hand(
        pool,
        batch,
        RowAddress {
            sheet: "Teachouse",
            ordinal,
        },
        ProductId(Uuid(*product.as_bytes())),
        None,
    )
    .await;
}

/// Puts one row of any tab back into the state a killed pass leaves it in —
/// `creating`, with the identifiers the claim reserved — and the batch with
/// it. Written by statement rather than by the route, because the route never
/// leaves this state on purpose.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn reserve_by_hand(
    pool: &PgPool,
    batch: Uuid,
    at: RowAddress<'_>,
    product: ProductId,
    mapping: Option<MappingId>,
) {
    let ordinal = i32::try_from(at.ordinal).expect("a spreadsheet row number fits the column");
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    sqlx::query(
        "UPDATE import_batch_row SET state = 'creating', product_id = $5, mapping_id = $6 \
          WHERE org_id = $1 AND batch_id = $2 AND sheet = $3 AND ordinal = $4",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(batch.0))
    .bind(at.sheet)
    .bind(ordinal)
    .bind(uuid::Uuid::from_bytes(product.0 .0))
    .bind(mapping.map(|id| uuid::Uuid::from_bytes(id.0 .0)))
    .execute(&mut *tx)
    .await
    .expect("the row is claimed by hand");
    sqlx::query("UPDATE import_batch SET state = 'importing', settled_at = NULL, failure_detail = NULL WHERE org_id = $1 AND id = $2")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .bind(uuid::Uuid::from_bytes(batch.0))
        .execute(&mut *tx)
        .await
        .expect("the batch is reopened by hand");
    tx.commit().await.expect("the surgery commits");
}

/// The product a create writes first, and nothing that trails it: the shape a
/// pass killed straight after the product insert leaves behind. The bytes are
/// the row's own, so the mapping the resume writes finds the payload the
/// deferred trigger requires.
fn half_created(product: ProductId, payload: &FileHandle, cover: &FileHandle) -> CanonicalProduct {
    CanonicalProduct {
        id: product,
        org: ORG_A,
        title: Title("Stranded".to_owned()),
        body: ListingCopy {
            body: "Left after the insert.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            held_file(payload, FileRole::Payload, FileKind::Pdf),
            vec![],
        )),
        cover: Some(held_file(cover, FileRole::Cover, FileKind::Image)),
        previews: vec![],
        subjects: vec![],
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

fn held_file(handle: &FileHandle, role: FileRole, kind: FileKind) -> ProductFile {
    ProductFile {
        id: FileId(Uuid(*uuid::Uuid::new_v4().as_bytes())),
        role,
        kind,
        bytes: FileBytes::Held {
            hash: hash_of(&handle.hash),
            byte_len: handle.byte_len,
            scan: ScanOutcome::Pending,
        },
    }
}

/// The digest an upload handle carries, read back out of its hex.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn hash_of(hex: &str) -> ContentHash {
    assert_eq!(
        hex.len(),
        64,
        "the upload answers a 64-character hex digest"
    );
    let mut bytes = [0_u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).expect("a hex digest is ASCII");
        *byte = u8::from_str_radix(pair, 16).expect("a hex digest parses two characters at a time");
    }
    ContentHash(bytes)
}

/// How many rows one table holds against one product, read with the tenant
/// pin set for the reason `counted` gives.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn per_product(pool: &PgPool, product: ProductId, table: &str) -> i64 {
    let mut tx = pool.begin().await.expect("the transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pin sets");
    let held: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {table} WHERE org_id = $1 AND product_id = $2"
    ))
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes(product.0 .0))
    .fetch_one(&mut *tx)
    .await
    .expect("the count runs");
    tx.commit().await.expect("the read commits");
    held
}

/// A pass killed between the product insert and the writes that trail it —
/// the mapping, the elections and the labels — is finished by the next chunk
/// rather than settled with them missing.
///
/// The parts are performed separately here, by hand, and stopped after the
/// product insert: the row's identifiers are reserved, its product is written
/// under the reserved identifier with the bytes the row holds, and nothing
/// else. A control row in the same sheet takes the whole road through the
/// route, so the shape of a finished row is read off a real create rather than
/// asserted from this test's idea of one, and the stranded row is held to it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_pass_killed_after_the_product_insert_is_completed_rather_than_settled_half_done(
    pool: PgPool,
) {
    provision(&pool).await;
    let root = store_root("resume-trailing");
    let sealing = sealing(pool.clone(), &root);
    let document = filled_sheet(
        "TES",
        &[
            &[(Cell::Title, "Control"), (Cell::File, "one.pdf")],
            &[(Cell::Title, "Stranded"), (Cell::File, "one.pdf")],
        ],
    );
    let uploaded: UploadedBatchView = upload(&pool, &TOKEN_A, KEY_A, "TES.csv", document)
        .await
        .json();
    assert_eq!(
        uploaded.detail.batch.failed_count, 0,
        "the fixture is about the resume, so the parse takes both rows: {:?}",
        uploaded.detail.rows
    );
    let batch = uploaded.detail.batch.id;

    let bytes = sealed(sealing.clone(), &TOKEN_A, "one").await;
    let Some(payload) = bytes.payload.first() else {
        panic!("a single pdf is its own payload file");
    };
    for ordinal in [4, 5] {
        assert_eq!(
            bind(
                &pool,
                &TOKEN_A,
                &row_path(batch, "TES", ordinal),
                payload,
                &bytes.cover
            )
            .await
            .status,
            StatusCode::OK
        );
    }

    // The crash, by hand: the identifiers reserved, the product written under
    // them, and nothing after it.
    let product = ProductId(Uuid([0xC2; 16]));
    let mapping = MappingId(Uuid([0xC3; 16]));
    reserve_by_hand(
        &pool,
        batch,
        RowAddress {
            sheet: "TES",
            ordinal: 5,
        },
        product,
        Some(mapping),
    )
    .await;
    ProductRepo::new(pool.clone())
        .insert(ORG_A, &half_created(product, payload, &bytes.cover), MADE)
        .await
        .expect("the stranded product inserts");
    assert_eq!(
        (
            per_product(&pool, product, "mapping").await,
            per_product(&pool, product, "election_item").await,
            per_product(&pool, product, "product_label").await,
        ),
        (0, 0, 0),
        "the crash left the product with none of the writes that trail it"
    );

    let answer = commit(sealing, &TOKEN_A, batch).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the next chunk runs: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let ack: CommitAck = answer.json();
    assert_eq!(
        (ack.applied, ack.skipped, ack.failed, ack.complete),
        (1, 1, 0, true),
        "the control row is created and the stranded one is completed rather than created again"
    );
    assert_eq!(ack.batch_state, BatchStateView::Imported);
    assert_eq!(
        products_held(&pool, ORG_A).await,
        2,
        "one product per row and no second one for the stranded row"
    );

    let rows = report(&pool, &TOKEN_A, batch).await.rows;
    let Some(control) = rows
        .iter()
        .find(|row| row.ordinal == 4)
        .and_then(|row| row.product)
    else {
        panic!("the control row names the product it became");
    };
    let Some(stranded) = rows.iter().find(|row| row.ordinal == 5) else {
        panic!("the report holds the stranded row");
    };
    assert_eq!(
        (stranded.state, stranded.product),
        (RowStateView::Created, Some(product)),
        "the stranded row settled as created, under the identifier reserved for it"
    );

    for table in ["mapping", "election_item", "product_label"] {
        let expected = per_product(&pool, control, table).await;
        assert!(
            expected > 0,
            "the control row's create wrote {table} rows, so the shape held against is real"
        );
        assert_eq!(
            per_product(&pool, product, table).await,
            expected,
            "the stranded row's product holds the {table} rows a whole create writes"
        );
    }
    let Some(bound) = MappingRepo::new(pool.clone())
        .get(ORG_A, mapping)
        .await
        .expect("the mapping reads")
    else {
        panic!("the mapping was written under the identifier reserved for it, not a fresh one");
    };
    assert_eq!(bound.mapping.product, product);
    let labels: Vec<String> = LabelRepo::new(pool.clone())
        .for_product(ORG_A, product)
        .await
        .expect("the labels read")
        .into_iter()
        .map(|record| record.name)
        .collect();
    assert_eq!(
        labels,
        vec!["Autumn Term".to_owned(), "Year 5".to_owned()],
        "and the sheet's labels are on it"
    );
}

/// The fence over the commit, driven by the two sessions already provisioned.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn neither_tenant_commits_the_others_import(pool: PgPool) {
    provision(&pool).await;
    let document = filled_sheet("Teachouse", &[&[(Cell::Title, "One")]]);
    let theirs: UploadedBatchView = upload(&pool, &TOKEN_B, KEY_B, "Teachouse.csv", document)
        .await
        .json();
    let batch = theirs.detail.batch.id;

    assert_eq!(
        commit(state(pool.clone()), &TOKEN_A, batch).await.status,
        StatusCode::NOT_FOUND,
        "another organisation's import is absent rather than committable"
    );
    assert_eq!(
        products_held(&pool, ORG_A).await + products_held(&pool, ORG_B).await,
        0,
        "and the attempt created nothing on either side of the fence"
    );
    assert_eq!(
        report(&pool, &TOKEN_B, batch).await.batch.state,
        BatchStateView::Parsed,
        "the batch that owns the row is untouched"
    );
}
