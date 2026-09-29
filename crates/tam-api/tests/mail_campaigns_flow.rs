//! The operators' mail over the wire: only an operator composes, counts or
//! sends; the audience count is the one the send queues; a campaign queues one
//! outbox row per recipient; an unsubscribe link is honoured and shows on the
//! seller's own preferences; and deleting frees the rows.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::mail_campaigns::{MailAudienceCount, MailCampaignDetail, MailPreviewView};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const NOW: Timestamp = Timestamp(1_790_640_000_000);
const SESSION_EXPIRY: Timestamp = Timestamp(4_102_444_800_000);
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const USER_PAID: UserId = UserId(Uuid([0x0C; 16]));
const USER_NO_SIGN_IN: UserId = UserId(Uuid([0x0D; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn backoffice_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the test database names itself");
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_backoffice:tam_backoffice_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the backoffice role connects")
}

fn state(pool: PgPool, backoffice: Option<PgPool>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice,
        blobs: None,
    }
}

/// Four sellers in four organisations: the operator, a free seller, a paid
/// (Sync) seller, and one who never signed in.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    let sessions = SessionRepo::new(pool.clone());
    for (user, subject) in [
        (USER_OPERATOR, true),
        (USER_SELLER, true),
        (USER_PAID, true),
        (USER_NO_SIGN_IN, false),
    ] {
        let org = OrgId(Uuid([user.0 .0[0] ^ 0xF0; 16]));
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(format!("org {:02x}", user.0 .0[0]))
            .execute(pool)
            .await
            .expect("the org seeds");
        sessions
            .create_user(
                org,
                user,
                &format!("{:02x}@example.test", user.0 .0[0]),
                Timestamp(1_000),
            )
            .await
            .expect("the user provisions");
        if subject {
            sqlx::query("UPDATE app_user SET auth_subject = $2 WHERE id = $1")
                .bind(uuid::Uuid::from_bytes(user.0 .0))
                .bind(uuid::Uuid::from_bytes([user.0 .0[0] ^ 0x55; 16]))
                .execute(pool)
                .await
                .expect("the subject lands");
        }
    }
    let paid_org = uuid::Uuid::from_bytes([0x0C ^ 0xF0; 16]);
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(paid_org.to_string())
        .execute(&mut *tx)
        .await
        .expect("the org pins");
    sqlx::query(
        "INSERT INTO entitlement_grant (org_id, id, plan, granted_by, granted_at) \
         VALUES ($1, gen_random_uuid(), 'subscriber', 'operator', now())",
    )
    .bind(paid_org)
    .execute(&mut *tx)
    .await
    .expect("the paid seller's grant lands");
    tx.commit().await.expect("the grant commits");
    for (user, token) in [(USER_OPERATOR, TOKEN_OPERATOR), (USER_SELLER, TOKEN_SELLER)] {
        sessions
            .mint(&token, user, SESSION_EXPIRY, Timestamp(1_000))
            .await
            .expect("the session mints");
    }
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", Timestamp(2_000))
        .await
        .expect("the operator marking lands");
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
    state: AppState,
    method: Method,
    path: &str,
    token: Option<&SessionToken>,
    body: Option<serde_json::Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = router(state)
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

fn draft() -> serde_json::Value {
    serde_json::json!({
        "subject": "News for @first_name",
        "body_html": "<h2>Hello</h2><p>Hi @name from @org</p><p>@link</p><script>x()</script>",
        "link_url": "https://teachouse.io/new",
        "link_label": "See what's new"
    })
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn only_an_operator_reaches_the_mail_routes(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;
    let id = "c1c1c1c1-c1c1-c1c1-c1c1-c1c1c1c1c1c1";
    let routes = [
        (
            Method::GET,
            "/v1/admin/mail/audience?segment=all".to_owned(),
            None,
        ),
        (
            Method::POST,
            "/v1/admin/mail/preview".to_owned(),
            Some(draft()),
        ),
        (Method::POST, "/v1/admin/mail/images".to_owned(), None),
        (
            Method::POST,
            "/v1/admin/mail/test".to_owned(),
            Some(serde_json::json!({"draft": draft()})),
        ),
        (Method::GET, "/v1/admin/mail/campaigns".to_owned(), None),
        (
            Method::POST,
            "/v1/admin/mail/campaigns".to_owned(),
            Some(serde_json::json!({"draft": draft(), "audience": {"segment": "all"}})),
        ),
        (Method::GET, format!("/v1/admin/mail/campaigns/{id}"), None),
        (
            Method::DELETE,
            format!("/v1/admin/mail/campaigns/{id}"),
            None,
        ),
        (
            Method::POST,
            format!("/v1/admin/mail/campaigns/{id}/retry"),
            None,
        ),
    ];
    for token in [None, Some(&TOKEN_SELLER)] {
        for (method, path, body) in &routes {
            let answer = call(
                state(pool.clone(), Some(backoffice.clone())),
                method.clone(),
                path,
                token,
                body.clone(),
            )
            .await;
            assert_eq!(
                answer.status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} refuses a caller who is not an operator"
            );
        }
    }
    let campaigns: i64 = sqlx::query_scalar("SELECT count(*) FROM mail_campaign")
        .fetch_one(&pool)
        .await
        .unwrap_or(-1);
    assert_eq!(campaigns, 0, "nothing was written by a refused caller");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_count_is_what_the_send_queues_and_delete_frees_it(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;
    let serve = || state(pool.clone(), Some(backoffice.clone()));

    let all: MailAudienceCount = parsed(
        &call(
            serve(),
            Method::GET,
            "/v1/admin/mail/audience?segment=all",
            Some(&TOKEN_OPERATOR),
            None,
        )
        .await,
    );
    assert_eq!(
        (
            all.sellers,
            all.recipients,
            all.operators_excluded,
            all.no_sign_in
        ),
        (4, 2, 1, 1)
    );
    let paid: MailAudienceCount = parsed(
        &call(
            serve(),
            Method::GET,
            "/v1/admin/mail/audience?segment=paid",
            Some(&TOKEN_OPERATOR),
            None,
        )
        .await,
    );
    assert_eq!((paid.sellers, paid.recipients), (1, 1));

    let preview: MailPreviewView = parsed(
        &call(
            serve(),
            Method::POST,
            "/v1/admin/mail/preview",
            Some(&TOKEN_OPERATOR),
            Some(draft()),
        )
        .await,
    );
    assert_eq!(preview.subject, "News for Ana");
    assert!(
        preview
            .html
            .contains("Hi Ana Ruiz from Ana&#39;s Classroom")
            || preview.html.contains("Hi Ana Ruiz from Ana's Classroom")
    );
    assert!(
        !preview.html.contains("x()"),
        "a script's text is dropped whole"
    );

    let created = call(
        serve(),
        Method::POST,
        "/v1/admin/mail/campaigns",
        Some(&TOKEN_OPERATOR),
        Some(serde_json::json!({
            "draft": draft(),
            "audience": {"segment": "all", "exclude_operators": true, "verified_only": true},
            "created_by_label": "ops@example.test"
        })),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let detail: MailCampaignDetail = parsed(&created);
    assert_eq!(
        detail.summary.counts.queued, 2,
        "one outbox row per counted recipient"
    );
    assert_eq!(detail.recipients.len(), 2);
    let stored = detail.draft.expect("a live campaign has its body");
    assert!(
        !stored.body_html.contains("script"),
        "the stored body is the reduction"
    );
    let queued_users: Vec<UserId> = detail.recipients.iter().map(|r| r.user).collect();
    assert!(queued_users.contains(&USER_SELLER) && queued_users.contains(&USER_PAID));
    let paid_row = detail
        .recipients
        .iter()
        .find(|r| r.user == USER_PAID)
        .expect("the paid seller's row");
    assert_eq!(paid_row.plan, "Sync");

    let path = format!(
        "/v1/admin/mail/campaigns/{}",
        detail.summary.id.to_hyphenated()
    );
    let deleted: MailCampaignDetail =
        parsed(&call(serve(), Method::DELETE, &path, Some(&TOKEN_OPERATOR), None).await);
    assert!(deleted.draft.is_none());
    assert!(deleted.recipients.is_empty());
    assert_eq!(
        deleted.summary.counts.total, 2,
        "the log line keeps its count"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM mail_campaign_recipient")
        .fetch_one(&pool)
        .await
        .unwrap_or(-1);
    assert_eq!(rows, 0);

    let empty = call(
        serve(),
        Method::POST,
        "/v1/admin/mail/campaigns",
        Some(&TOKEN_OPERATOR),
        Some(serde_json::json!({
            "draft": draft(),
            "audience": {"segment": "studio", "exclude_operators": true, "verified_only": true}
        })),
    )
    .await;
    assert_eq!(
        empty.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "nobody to send to"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unsubscribe_link_is_honoured_and_shows_in_preferences(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;
    let serve = || state(pool.clone(), Some(backoffice.clone()));
    let token: uuid::Uuid =
        sqlx::query_scalar("SELECT marketing_token FROM app_user WHERE id = $1")
            .bind(uuid::Uuid::from_bytes(USER_SELLER.0 .0))
            .fetch_one(&pool)
            .await
            .unwrap_or_default();
    let link = format!("/v1/mail/unsubscribe?t={token}");

    let page = call(serve(), Method::GET, &link, None, None).await;
    assert_eq!(page.status, StatusCode::OK);
    let prefs = call(
        serve(),
        Method::GET,
        "/v1/notifications/preferences",
        Some(&TOKEN_SELLER),
        None,
    )
    .await;
    let view: serde_json::Value = parsed(&prefs);
    assert_eq!(
        view["marketing_email"], true,
        "opening the link changes nothing"
    );

    let done = call(serve(), Method::POST, &link, None, None).await;
    assert_eq!(done.status, StatusCode::OK);
    let view: serde_json::Value = parsed(
        &call(
            serve(),
            Method::GET,
            "/v1/notifications/preferences",
            Some(&TOKEN_SELLER),
            None,
        )
        .await,
    );
    assert_eq!(view["marketing_email"], false);
    assert_eq!(view["notify_email"], true, "completion mail is untouched");

    let all: MailAudienceCount = parsed(
        &call(
            serve(),
            Method::GET,
            "/v1/admin/mail/audience?segment=all",
            Some(&TOKEN_OPERATOR),
            None,
        )
        .await,
    );
    assert_eq!(
        (all.recipients, all.opted_out),
        (1, 1),
        "an opted-out seller is not queued"
    );

    let unknown = call(
        serve(),
        Method::POST,
        "/v1/mail/unsubscribe?t=77777777-7777-7777-7777-777777777777",
        None,
        None,
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let back: serde_json::Value = parsed(
        &call(
            serve(),
            Method::PATCH,
            "/v1/notifications/preferences",
            Some(&TOKEN_SELLER),
            Some(serde_json::json!({"marketing_email": true})),
        )
        .await,
    );
    assert_eq!(
        (
            back["marketing_email"].clone(),
            back["notify_email"].clone()
        ),
        (true.into(), true.into())
    );
}
