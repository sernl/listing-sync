//! The inbox over the wire: a toast the seller did not look at is kept as a
//! notice, once however often it is posted; it is its poster's alone; the
//! unread count follows reading and dismissing; and a finished run comes in
//! beside the notices in the tone of its worst outcome.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::notifications::{NotificationPage, NotificationView, SourceView};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{NotificationRepo, SessionRepo, SessionToken};
use tam_types::{NoticeTone, NotificationCounts, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const NOW: Timestamp = Timestamp(1_790_640_000_000);
const SESSION_EXPIRY: Timestamp = Timestamp(4_102_444_800_000);
const ORG: OrgId = OrgId(Uuid([0xF1; 16]));
const OTHER_ORG: OrgId = OrgId(Uuid([0xF2; 16]));
const SELLER: UserId = UserId(Uuid([0x0B; 16]));
const COLLEAGUE: UserId = UserId(Uuid([0x0C; 16]));
const STRANGER: UserId = UserId(Uuid([0x0D; 16]));
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
const TOKEN_COLLEAGUE: SessionToken = SessionToken([0x43; 32]);
const TOKEN_STRANGER: SessionToken = SessionToken([0x44; 32]);

fn state(pool: PgPool) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

/// Two sellers sharing one organisation, and a third in another.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for org in [ORG, OTHER_ORG] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(format!("org {:02x}", org.0 .0[0]))
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, token) in [
        (ORG, SELLER, TOKEN_SELLER),
        (ORG, COLLEAGUE, TOKEN_COLLEAGUE),
        (OTHER_ORG, STRANGER, TOKEN_STRANGER),
    ] {
        sessions
            .create_user(
                org,
                user,
                &format!("{:02x}@example.test", user.0 .0[0]),
                Timestamp(1_000),
            )
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, SESSION_EXPIRY, Timestamp(1_000))
            .await
            .expect("the session mints");
    }
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: &PgPool,
    method: Method,
    path: &str,
    token: &SessionToken,
    body: Option<serde_json::Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router(state(pool.clone()))
        .oneshot(request.body(body).expect("the request builds"))
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
    reason = "a body that does not parse should panic"
)]
fn parsed<T: serde::de::DeserializeOwned>(answer: &Answer) -> T {
    serde_json::from_slice(&answer.body).expect("the body parses")
}

async fn inbox(pool: &PgPool, token: &SessionToken) -> NotificationPage {
    let answer = call(pool, Method::GET, "/v1/notifications", token, None).await;
    assert_eq!(answer.status, StatusCode::OK, "the inbox reads");
    parsed(&answer)
}

async fn post(pool: &PgPool, client_id: &str, tone: &str, title: &str) -> Answer {
    call(
        pool,
        Method::POST,
        "/v1/notifications",
        &TOKEN_SELLER,
        Some(serde_json::json!({
            "client_id": client_id,
            "tone": tone,
            "title": title,
            "body": ""
        })),
    )
    .await
}

fn title_of(view: &NotificationView) -> Option<&str> {
    match &view.source {
        SourceView::Notice { title, .. } => Some(title),
        SourceView::Run { .. } => None,
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unread_toast_is_kept_once_and_counted(pool: PgPool) {
    provision(&pool).await;

    let first = post(&pool, "toast-1", "error", "The copy wasn't requested.").await;
    assert_eq!(first.status, StatusCode::OK, "the notice is kept");
    let kept: NotificationView = parsed(&first);
    assert_eq!(kept.tone, NoticeTone::Error, "it keeps the toast's tone");
    assert!(kept.read_at.is_none(), "a kept notice is unread");

    let again: NotificationView =
        parsed(&post(&pool, "toast-1", "error", "The copy wasn't requested.").await);
    assert_eq!(
        again.id, kept.id,
        "the same toast posted twice is one notice"
    );

    let page = inbox(&pool, &TOKEN_SELLER).await;
    assert_eq!(page.notifications.len(), 1, "one notice");
    assert_eq!(page.unread, 1, "and it is counted unread");
    assert_eq!(
        title_of(&page.notifications[0]),
        Some("The copy wasn't requested.")
    );

    assert_eq!(
        post(&pool, "toast-2", "error", "   ").await.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a notice with nothing to say is refused"
    );
    assert_eq!(
        post(&pool, "toast-3", "shouting", "Saved.").await.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a tone outside the four is refused"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_notice_is_its_posters_alone(pool: PgPool) {
    provision(&pool).await;
    let kept: NotificationView = parsed(&post(&pool, "toast-1", "warning", "Too big.").await);
    let one = format!("/v1/notifications/{}", uuid::Uuid::from_bytes(kept.id.0));

    for token in [&TOKEN_COLLEAGUE, &TOKEN_STRANGER] {
        let page = inbox(&pool, token).await;
        assert!(page.notifications.is_empty(), "nobody else reads it");
        assert_eq!(page.unread, 0, "or counts it");
        assert_eq!(
            call(&pool, Method::POST, &format!("{one}/read"), token, None)
                .await
                .status,
            StatusCode::NOT_FOUND,
            "or marks it"
        );
        assert_eq!(
            call(&pool, Method::DELETE, &one, token, None).await.status,
            StatusCode::NOT_FOUND,
            "or dismisses it"
        );
    }
    assert_eq!(
        call(
            &pool,
            Method::POST,
            "/v1/notifications/read-all",
            &TOKEN_COLLEAGUE,
            None
        )
        .await
        .status,
        StatusCode::OK,
        "a colleague may mark their own inbox read"
    );
    let page = inbox(&pool, &TOKEN_SELLER).await;
    assert_eq!(page.unread, 1, "a colleague's read-all leaves it unread");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn reading_and_dismissing_move_the_count(pool: PgPool) {
    provision(&pool).await;
    NotificationRepo::new(pool.clone())
        .record_import(
            ORG,
            Uuid([0xA1; 16]),
            NotificationCounts {
                succeeded: 4,
                blocked: 1,
                ..NotificationCounts::default()
            },
            Timestamp(NOW.0 - 60_000),
        )
        .await
        .expect("the import records");
    let saved: NotificationView = parsed(&post(&pool, "toast-1", "success", "Saved.").await);

    let page = inbox(&pool, &TOKEN_SELLER).await;
    assert_eq!(page.unread, 2, "the run and the notice are both unread");
    let run = &page.notifications[1];
    assert!(
        matches!(run.source, SourceView::Run { .. }),
        "the older row is the run"
    );
    assert_eq!(
        run.tone,
        NoticeTone::Warning,
        "a run with a blocked item reads as a warning"
    );
    assert_eq!(
        inbox(&pool, &TOKEN_COLLEAGUE).await.notifications.len(),
        1,
        "the run is the organisation's; the notice is not"
    );

    let saved_path = format!("/v1/notifications/{}", uuid::Uuid::from_bytes(saved.id.0));
    let marked = call(
        &pool,
        Method::POST,
        &format!("{saved_path}/read"),
        &TOKEN_SELLER,
        None,
    )
    .await;
    assert_eq!(marked.status, StatusCode::OK, "the mark is accepted");
    assert_eq!(
        parsed::<serde_json::Value>(&marked)["marked"],
        1,
        "one row was marked"
    );
    assert_eq!(
        inbox(&pool, &TOKEN_SELLER).await.unread,
        1,
        "one unread row is left"
    );

    let all = call(
        &pool,
        Method::POST,
        "/v1/notifications/read-all",
        &TOKEN_SELLER,
        None,
    )
    .await;
    assert_eq!(
        parsed::<serde_json::Value>(&all)["marked"],
        1,
        "read-all marks the run"
    );
    assert_eq!(
        inbox(&pool, &TOKEN_SELLER).await.unread,
        0,
        "nothing is unread"
    );

    assert_eq!(
        call(&pool, Method::DELETE, &saved_path, &TOKEN_SELLER, None)
            .await
            .status,
        StatusCode::NO_CONTENT,
        "the seller dismisses their notice"
    );
    assert_eq!(
        call(&pool, Method::DELETE, &saved_path, &TOKEN_SELLER, None)
            .await
            .status,
        StatusCode::NO_CONTENT,
        "dismissing twice is still dismissed"
    );
    assert_eq!(
        inbox(&pool, &TOKEN_SELLER).await.notifications.len(),
        1,
        "only the run is left"
    );

    let swept = call(
        &pool,
        Method::POST,
        "/v1/notifications/dismiss-read",
        &TOKEN_SELLER,
        None,
    )
    .await;
    assert_eq!(
        parsed::<serde_json::Value>(&swept)["dismissed"],
        1,
        "the read run goes"
    );
    assert!(
        inbox(&pool, &TOKEN_SELLER).await.notifications.is_empty(),
        "the inbox is empty"
    );
    assert_eq!(
        call(
            &pool,
            Method::DELETE,
            "/v1/notifications/not-an-id",
            &TOKEN_SELLER,
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND,
        "an id that is not one is not found"
    );
}
