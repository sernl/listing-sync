//! The sign-up agreement over the wire: the identity service's internal write,
//! the signed-in person's status and re-consent, and the operator's two reads.
//!
//! The internal route's fence is the part with teeth. With no secret
//! configured, a wrong secret, or none offered, it answers the same 404 an
//! unknown path does, and only the shared secret writes a row.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::account_consent::{
    ConsentStatusView, ConsentSummariesView, InternalSecret, SubjectConsentView, TERMS_VERSION,
};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(1_790_000_000_000);
const SECRET: &str = "the-shared-secret";
const SELLER_SUBJECT: &str = "b1b1b1b1-b1b1-4b1b-8b1b-b1b1b1b1b1b1";
const NEWCOMER_SUBJECT: &str = "c2c2c2c2-c2c2-4c2c-8c2c-c2c2c2c2c2c2";

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

fn state(pool: PgPool, backoffice: Option<PgPool>, secret: Option<&str>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            consent_secret: secret.map(|value| InternalSecret::new(value.to_owned())),
            ..Config::default()
        },
        wall: || NOW,
        auth: None,
        backoffice,
        blobs: None,
    }
}

/// An operator with no identity subject, and a seller whose subject is
/// [`SELLER_SUBJECT`].
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
        (
            ORG_A,
            USER_OPERATOR,
            "operator@example.test",
            TOKEN_OPERATOR,
        ),
        (ORG_B, USER_SELLER, "seller@example.test", TOKEN_SELLER),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(&token, user, Timestamp(NOW.0 + 100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
    sqlx::query("UPDATE app_user SET auth_subject = $2::uuid WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(USER_SELLER.0 .0))
        .bind(SELLER_SUBJECT)
        .execute(pool)
        .await
        .expect("the subject links");
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", Timestamp(1_000))
        .await
        .expect("the operator marking lands");
}

struct Call<'a> {
    method: &'a str,
    uri: &'a str,
    token: Option<&'a SessionToken>,
    headers: &'a [(&'a str, &'a str)],
    body: Option<serde_json::Value>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn send(state: AppState, call: Call<'_>) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(call.method).uri(call.uri);
    if let Some(token) = call.token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    for (name, value) in call.headers {
        request = request.header(*name, *value);
    }
    let body = match call.body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = router(state)
        .oneshot(request.body(body).expect("the request builds"))
        .await
        .expect("the router serves");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

fn agreed(version: &str) -> serde_json::Value {
    serde_json::json!({
        "terms_privacy": true,
        "ip_ownership": true,
        "age_18": true,
        "version": version,
    })
}

fn internal_body(consent: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "subject": NEWCOMER_SUBJECT,
        "email": "new@example.test",
        "ip_address": "203.0.113.7",
        "user_agent": "Mozilla/5.0 (test)",
        "consent": consent,
    })
}

async fn rows(pool: &PgPool, subject: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM account_consent WHERE subject = $1::uuid")
        .bind(subject)
        .fetch_one(pool)
        .await
        .unwrap_or(-1)
}

async fn internal(
    state: AppState,
    secret: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, Vec<u8>) {
    let headers: Vec<(&str, &str)> = secret
        .map(|value| vec![("x-tam-internal-secret", value)])
        .unwrap_or_default();
    send(
        state,
        Call {
            method: "POST",
            uri: "/internal/consent",
            token: None,
            headers: &headers,
            body: Some(body),
        },
    )
    .await
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_internal_route_is_a_404_without_the_secret(pool: PgPool) {
    let body = internal_body(&agreed(TERMS_VERSION));
    let (unconfigured, _) =
        internal(state(pool.clone(), None, None), Some(SECRET), body.clone()).await;
    assert_eq!(unconfigured, StatusCode::NOT_FOUND, "no secret configured");
    let (wrong, _) = internal(
        state(pool.clone(), None, Some(SECRET)),
        Some("not-it"),
        body.clone(),
    )
    .await;
    assert_eq!(wrong, StatusCode::NOT_FOUND, "a wrong secret");
    let (absent, _) = internal(state(pool.clone(), None, Some(SECRET)), None, body).await;
    assert_eq!(absent, StatusCode::NOT_FOUND, "no secret offered");
    let (garbage, _) = internal(
        state(pool.clone(), None, Some(SECRET)),
        Some("not-it"),
        serde_json::json!("not a consent"),
    )
    .await;
    assert_eq!(
        garbage,
        StatusCode::NOT_FOUND,
        "an unreadable body says nothing before the secret matches"
    );
    assert_eq!(rows(&pool, NEWCOMER_SUBJECT).await, 0);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_identity_service_records_three_rows_with_the_secret(pool: PgPool) {
    let configured = state(pool.clone(), None, Some(SECRET));
    let (status, body) = internal(
        configured.clone(),
        Some(SECRET),
        internal_body(&agreed(TERMS_VERSION)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(rows(&pool, NEWCOMER_SUBJECT).await, 3);

    let mut unticked = agreed(TERMS_VERSION);
    unticked["age_18"] = serde_json::json!(false);
    let (refused, body) = internal(configured, Some(SECRET), internal_body(&unticked)).await;
    assert_eq!(refused, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&body).contains("Please tick both boxes to continue."));
    assert_eq!(
        rows(&pool, NEWCOMER_SUBJECT).await,
        3,
        "nothing more written"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_signed_in_person_is_asked_until_they_agree_to_the_current_terms(pool: PgPool) {
    provision(&pool).await;
    let app = state(pool.clone(), None, None);
    let status = |app: AppState| async move {
        let (code, body) = send(
            app,
            Call {
                method: "GET",
                uri: "/v1/consent/status",
                token: Some(&TOKEN_SELLER),
                headers: &[],
                body: None,
            },
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        serde_json::from_slice::<ConsentStatusView>(&body).unwrap_or_else(|error| {
            panic!("{error}: {}", String::from_utf8_lossy(&body));
        })
    };
    let before = status(app.clone()).await;
    assert!(before.required);
    assert_eq!(before.accepted_version, None);
    assert_eq!(before.current_version, TERMS_VERSION);

    let accept = |app: AppState, body: serde_json::Value| async move {
        send(
            app,
            Call {
                method: "POST",
                uri: "/v1/consent",
                token: Some(&TOKEN_SELLER),
                headers: &[
                    ("cf-connecting-ip", "198.51.100.9"),
                    ("user-agent", "Mozilla/5.0 (console)"),
                ],
                body: Some(body),
            },
        )
        .await
        .0
    };
    assert_eq!(
        accept(app.clone(), agreed("2020-01-01")).await,
        StatusCode::CONFLICT,
        "a stale page is told to reload"
    );
    let mut unticked = agreed(TERMS_VERSION);
    unticked["terms_privacy"] = serde_json::json!(false);
    assert_eq!(
        accept(app.clone(), unticked).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(rows(&pool, SELLER_SUBJECT).await, 0);

    assert_eq!(
        accept(app.clone(), agreed(TERMS_VERSION)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        accept(app.clone(), agreed(TERMS_VERSION)).await,
        StatusCode::NO_CONTENT,
        "agreeing twice is not a second agreement"
    );
    assert_eq!(rows(&pool, SELLER_SUBJECT).await, 3);

    let after = status(app).await;
    assert!(!after.required);
    assert_eq!(after.accepted_version.as_deref(), Some(TERMS_VERSION));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_user_with_no_identity_subject_is_never_asked(pool: PgPool) {
    provision(&pool).await;
    let (code, body) = send(
        state(pool, None, None),
        Call {
            method: "GET",
            uri: "/v1/consent/status",
            token: Some(&TOKEN_OPERATOR),
            headers: &[],
            body: None,
        },
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let view: ConsentStatusView = serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!("{error}: {}", String::from_utf8_lossy(&body));
    });
    assert!(!view.required);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_operator_reads_the_column_and_the_detail(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;
    let app = state(pool.clone(), Some(backoffice), Some(SECRET));
    let (recorded, _) = internal(
        app.clone(),
        Some(SECRET),
        internal_body(&agreed(TERMS_VERSION)),
    )
    .await;
    assert_eq!(recorded, StatusCode::NO_CONTENT);

    let get = |app: AppState, uri: &'static str, token: &'static SessionToken| async move {
        send(
            app,
            Call {
                method: "GET",
                uri,
                token: Some(token),
                headers: &[],
                body: None,
            },
        )
        .await
    };

    let (code, body) = get(app.clone(), "/v1/admin/consents", &TOKEN_OPERATOR).await;
    assert_eq!(code, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let summaries: ConsentSummariesView = serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!("{error}: {}", String::from_utf8_lossy(&body));
    });
    assert_eq!(summaries.consents.len(), 1);
    assert_eq!(summaries.consents[0].document_version, TERMS_VERSION);
    assert_eq!(summaries.consents[0].accepted_at, NOW);

    let (code, body) = get(
        app.clone(),
        "/v1/admin/users/c2c2c2c2-c2c2-4c2c-8c2c-c2c2c2c2c2c2/consent",
        &TOKEN_OPERATOR,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let detail: SubjectConsentView = serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!("{error}: {}", String::from_utf8_lossy(&body));
    });
    let mut kinds: Vec<&str> = detail
        .consents
        .iter()
        .map(|row| row.kind.as_str())
        .collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["age_18", "ip_ownership", "terms_privacy"]);
    assert!(detail.consents.iter().all(|row| {
        row.ip_address.as_deref() == Some("203.0.113.7")
            && row.user_agent.as_deref() == Some("Mozilla/5.0 (test)")
            && row.email.as_deref() == Some("new@example.test")
    }));

    let (refused, _) = get(app, "/v1/admin/consents", &TOKEN_SELLER).await;
    assert_eq!(refused, StatusCode::UNAUTHORIZED, "a seller reads nothing");
}
