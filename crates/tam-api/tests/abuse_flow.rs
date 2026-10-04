//! Abuse prevention over the wire: the identity service's sign-up screen, the
//! operator's Abuse page, and the suspension a ban turns every seller route
//! into.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::abuse::{
    AbuseFlagsView, AbuseOrgView, DISPOSABLE_REFUSAL, SUSPENDED, VELOCITY_REFUSAL,
};
use tam_api::account_consent::InternalSecret;
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_secrets::Kek;
use tam_storage::{AbuseRepo, FlagKind, OperatorRepo, SessionRepo, SessionToken};
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
const KEK: [u8; 32] = [0x11; 32];

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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
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
        blobs: Some(BlobStore::local(
            Kek::from_bytes(&KEK).expect("a 32-byte key is a key"),
            std::env::temp_dir().join("tam-abuse-flow"),
        )),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "Kiwi Maths")] {
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
    consent(pool, SELLER_SUBJECT, "seller@school.nz", "198.51.100.1", 0).await;
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", Timestamp(1_000))
        .await
        .expect("the operator marking lands");
}

/// One sign-up's agreement, `ago_ms` before now.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn consent(pool: &PgPool, subject: &str, email: &str, ip: &str, ago_ms: i64) {
    sqlx::query(
        "INSERT INTO account_consent \
         (subject, email, kind, document_version, accepted_at, ip_address) \
         VALUES ($1::uuid, $2, 'terms_privacy', '2026-10-04', \
                 to_timestamp($3::double precision / 1000), $4::inet)",
    )
    .bind(subject)
    .bind(email)
    .bind(NOW.0 - ago_ms)
    .bind(ip)
    .execute(pool)
    .await
    .expect("the agreement inserts");
}

struct Call<'a> {
    method: &'a str,
    uri: &'a str,
    token: Option<&'a SessionToken>,
    secret: Option<&'a str>,
    body: Option<serde_json::Value>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn send(state: AppState, call: Call<'_>) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method(call.method).uri(call.uri);
    if let Some(token) = call.token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    if let Some(secret) = call.secret {
        request = request.header("x-tam-internal-secret", secret);
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
        .to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn screen(
    pool: &PgPool,
    secret: Option<&str>,
    offered: Option<&str>,
    email: &str,
) -> (StatusCode, serde_json::Value) {
    send(
        state(pool.clone(), None, secret),
        Call {
            method: "POST",
            uri: "/internal/abuse/screen",
            token: None,
            secret: offered,
            body: Some(serde_json::json!({ "email": email, "ip_address": "203.0.113.9" })),
        },
    )
    .await
}

fn message(body: &serde_json::Value) -> &str {
    body["errors"][0]["message"].as_str().unwrap_or_default()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_sign_up_screen_is_fenced_like_the_consent_route(app: PgPool) {
    for (secret, offered) in [
        (None, Some(SECRET)),
        (Some(SECRET), None),
        (Some(SECRET), Some("wrong")),
    ] {
        let (status, _) = screen(&app, secret, offered, "teacher@school.nz").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "an unfenced caller meets an unknown path"
        );
    }
    let (status, _) = screen(&app, Some(SECRET), Some(SECRET), "teacher@school.nz").await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "an ordinary sign-up proceeds"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_throwaway_domain_is_refused_with_the_sentence(app: PgPool) {
    let (status, body) = screen(&app, Some(SECRET), Some(SECRET), "x@mailinator.com").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(message(&body), DISPOSABLE_REFUSAL);
    assert_eq!(body["errors"][0]["code"], "signup_refused");
    assert_eq!(body["errors"][0]["detail"]["refusal"], "disposable_email");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_sixth_sign_up_from_one_address_in_a_day_is_refused(app: PgPool) {
    for n in 0..4_u8 {
        let subject = format!("d{n}d{n}d{n}d{n}-0000-4000-8000-00000000000{n}");
        consent(&app, &subject, "t@school.nz", "203.0.113.9", 3_600_000).await;
    }
    // Older than a day, and a second agreement by an existing account: neither
    // is a sign-up today.
    consent(
        &app,
        "e0e0e0e0-0000-4000-8000-000000000000",
        "t@school.nz",
        "203.0.113.9",
        90_000_000,
    )
    .await;
    consent(
        &app,
        "d0d0d0d0-0000-4000-8000-000000000000",
        "t@school.nz",
        "203.0.113.9",
        60_000,
    )
    .await;
    let (status, _) = screen(&app, Some(SECRET), Some(SECRET), "fifth@school.nz").await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "four today: the fifth is allowed"
    );

    consent(
        &app,
        "d9d9d9d9-0000-4000-8000-000000000009",
        "t@school.nz",
        "203.0.113.9",
        60_000,
    )
    .await;
    let (status, body) = screen(&app, Some(SECRET), Some(SECRET), "sixth@school.nz").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "five today: the sixth is refused"
    );
    assert_eq!(message(&body), VELOCITY_REFUSAL);
}

/// The operator's whole loop: a flag listed and searched, a ban that turns
/// the seller's session into the suspension, and the dismissal that lifts it.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_bans_a_flagged_seller_and_lifts_it(app: PgPool) {
    provision(&app).await;
    let backoffice = backoffice_pool(&app).await;
    let operator = |pool: &PgPool| state(pool.clone(), Some(backoffice.clone()), Some(SECRET));
    assert!(AbuseRepo::new(app.clone())
        .raise(
            ORG_B,
            FlagKind::SharedShop,
            "Shares a shop with 1 other account.",
            NOW
        )
        .await
        .expect("the flag raises"));

    for token in [Some(&TOKEN_SELLER), None] {
        for (method, uri) in [
            ("GET", "/v1/admin/abuse/flags".to_owned()),
            (
                "GET",
                "/v1/admin/abuse/orgs/bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".to_owned(),
            ),
            (
                "POST",
                "/v1/admin/abuse/flags/bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb/dismiss".to_owned(),
            ),
        ] {
            let (status, _) = send(
                operator(&app),
                Call {
                    method,
                    uri: &uri,
                    token,
                    secret: None,
                    body: None,
                },
            )
            .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {uri} is the operator's"
            );
        }
    }

    let (status, body) = send(
        operator(&app),
        Call {
            method: "GET",
            uri: "/v1/admin/abuse/flags",
            token: Some(&TOKEN_OPERATOR),
            secret: None,
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: AbuseFlagsView = serde_json::from_value(body).expect("the list reads");
    assert_eq!(listed.counters.open_flags, 1);
    let row = listed.orgs.first().expect("the flagged seller is listed");
    assert_eq!(row.org, ORG_B.0);
    assert_eq!(row.kinds, ["shared_shop"]);
    assert_eq!(row.score, 60);
    let flag = row.flag;

    let (_, body) = send(
        operator(&app),
        Call {
            method: "GET",
            uri: "/v1/admin/abuse/flags?q=SELLER@school.nz",
            token: Some(&TOKEN_OPERATOR),
            secret: None,
            body: None,
        },
    )
    .await;
    let searched: AbuseFlagsView = serde_json::from_value(body).expect("the search reads");
    assert_eq!(searched.query, "email");
    assert_eq!(
        searched.orgs.len(),
        1,
        "the seller is found by the address they signed up with"
    );

    let ban = format!("/v1/admin/abuse/flags/{}/ban", flag.to_hyphenated());
    let (status, body) = send(
        operator(&app),
        Call {
            method: "POST",
            uri: &ban,
            token: Some(&TOKEN_OPERATOR),
            secret: None,
            body: Some(serde_json::json!({ "reason": "  " })),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a ban says why: {body}"
    );

    let (status, body) = send(
        operator(&app),
        Call {
            method: "POST",
            uri: &ban,
            token: Some(&TOKEN_OPERATOR),
            secret: None,
            body: Some(serde_json::json!({ "reason": "Ten accounts on one shop" })),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let decided: AbuseOrgView = serde_json::from_value(body).expect("the cluster reads");
    assert_eq!(decided.standing, "ban");
    assert_eq!(decided.flags[0].action, "ban");

    // The ban ended the seller's session; one minted after it meets the gate.
    SessionRepo::new(app.clone())
        .mint(&TOKEN_SELLER, USER_SELLER, Timestamp(NOW.0 + 100_000), NOW)
        .await
        .expect("a session mints after the ban");
    let (status, body) = send(
        operator(&app),
        Call {
            method: "GET",
            uri: "/v1/whoami",
            token: Some(&TOKEN_SELLER),
            secret: None,
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["errors"][0]["code"], "account_suspended");
    assert_eq!(message(&body), SUSPENDED);

    // The banned address cannot sign up again.
    let (status, body) = screen(&app, Some(SECRET), Some(SECRET), "Seller@School.nz").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"][0]["detail"]["refusal"], "suspended");

    let dismiss = format!("/v1/admin/abuse/flags/{}/dismiss", flag.to_hyphenated());
    let (status, body) = send(
        operator(&app),
        Call {
            method: "POST",
            uri: &dismiss,
            token: Some(&TOKEN_OPERATOR),
            secret: None,
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = send(
        operator(&app),
        Call {
            method: "GET",
            uri: "/v1/whoami",
            token: Some(&TOKEN_SELLER),
            secret: None,
            body: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the lift takes effect on the next request"
    );
    let (status, _) = screen(&app, Some(SECRET), Some(SECRET), "seller@school.nz").await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "and the address may sign up again"
    );
}
