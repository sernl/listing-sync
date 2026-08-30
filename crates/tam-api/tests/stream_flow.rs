//! The progress stream driven in-process: replay from a cursor, the
//! header-over-query resume rule, the resync below the watermark, and the
//! 204 stop signal on a dead session.

#![cfg(feature = "pg-tests")]

use core::time::Duration;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{append_event, EventScope, JobRepo, NewJob, SessionRepo, SessionToken};
use tam_types::{
    Actor, InventoryId, JobEventPayload, JobId, OrgId, Stamp, SystemComponent, Timestamp, UserId,
    Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x41; 32]);
const JOB: JobId = JobId(Uuid([0x21; 16]));
const NOW: Timestamp = Timestamp(5_000);

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG, USER, "a@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    sessions
        .mint(&TOKEN, USER, Timestamp(100_000), Timestamp(1_000))
        .await
        .expect("the session mints");
    JobRepo::new(pool.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: JOB,
                inventory: InventoryId::TesNz,
                stamp: Stamp {
                    at: Timestamp(1_000),
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[],
        )
        .await
        .expect("the job enqueues with its JobQueued event");
    for _ in 0..2 {
        let mut tx = pool.begin().await.expect("tx begins");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
            .execute(&mut *tx)
            .await
            .expect("the pin applies");
        append_event(
            &mut tx,
            &EventScope {
                org: ORG,
                job: JOB,
                item: None,
            },
            &JobEventPayload::JobQueued { items: 0 },
            Stamp::system(SystemComponent::Engine, Timestamp(2_000)),
        )
        .await
        .expect("the extra event appends");
        tx.commit().await.expect("the event commits");
    }
}

/// Reads SSE text from the response body until `wanted` `id:` lines arrived
/// or two seconds passed, whichever is first, then drops the stream.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn read_stream(response: axum::response::Response, wanted: usize) -> String {
    let mut body = response.into_body();
    let mut text = String::new();
    while text.matches("id:").count() < wanted {
        let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("the stream yields before the timeout")
            .expect("the stream is not exhausted")
            .expect("the frame reads");
        if let Some(data) = frame.data_ref() {
            text.push_str(core::str::from_utf8(data).expect("SSE frames are UTF-8"));
        }
    }
    text
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn open_stream(
    pool: PgPool,
    path: &str,
    last_event_id: Option<&str>,
    with_session: bool,
) -> axum::response::Response {
    let mut request = Request::builder().uri(path);
    if with_session {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
        );
    }
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id);
    }
    router(state(pool))
        .oneshot(request.body(Body::empty()).expect("the request builds"))
        .await
        .expect("the router serves")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_ledger_replays_from_the_cursor(pool: PgPool) {
    provision(&pool).await;
    let response = open_stream(pool, "/v1/events/stream", None, true).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-accel-buffering")
            .and_then(|value| value.to_str().ok()),
        Some("no"),
        "the buffering opt-out travels on the stream response"
    );
    let text = read_stream(response, 3).await;
    assert!(
        text.contains("id: 1") && text.contains("id: 2") && text.contains("id: 3"),
        "all three ledger events replay in order: {text}"
    );
    assert!(
        text.contains("event: JobQueued"),
        "the event name is the ledger's kind: {text}"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_last_event_id_resumes_past_what_the_client_saw(pool: PgPool) {
    provision(&pool).await;
    let response = open_stream(pool, "/v1/events/stream", Some("1"), true).await;
    let text = read_stream(response, 2).await;
    assert!(
        !text.contains("id: 1\n") && text.contains("id: 2") && text.contains("id: 3"),
        "the replay starts strictly after the cursor: {text}"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_cursor_below_the_watermark_gets_a_resync_not_a_partial_replay(pool: PgPool) {
    provision(&pool).await;
    // org_event_counter is a tenant table under forced row-level security,
    // so the watermark advance must run pinned or it silently matches nothing.
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    sqlx::query("UPDATE org_event_counter SET prune_watermark = 10 WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(&mut *tx)
        .await
        .expect("the watermark advances");
    tx.commit().await.expect("the watermark commits");
    let response = open_stream(pool, "/v1/events/stream?cursor=1", None, true).await;
    let text = read_stream(response, 1).await;
    assert!(
        text.contains("event: resync") && text.contains("{\"cursor\":3}"),
        "a pruned-past cursor is told to resync at the snapshot cursor: {text}"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_dead_session_gets_the_stop_signal_not_a_401(pool: PgPool) {
    provision(&pool).await;
    let response = open_stream(pool, "/v1/events/stream", None, false).await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "204 stops EventSource; a 401 stream response would strand it permanently"
    );
}
