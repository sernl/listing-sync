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
use tam_api::import_batch::sheet::{columns, example_row, tab_of, Cell};
use tam_api::import_batch::{
    BatchStateView, ImportBatchDetailView, ImportsView, RowStateView, UploadedBatchView, Warning,
};
use tam_api::{router, APIError, AppState, Config, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn send(pool: &PgPool, request: Request<Body>) -> Answer {
    let response = router(state(pool.clone()))
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn products_held(pool: &PgPool, org: OrgId) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM product WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .fetch_one(pool)
        .await
        .expect("the count runs")
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
