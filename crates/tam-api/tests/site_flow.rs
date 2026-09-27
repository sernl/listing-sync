//! The site-wide switches over the wire: anyone reads them, only an operator
//! changes them, and the maintenance gate lets exactly the operator through.
//!
//! The refusal matters most. A seller who could flip maintenance mode could
//! take the whole site down for everybody else, so every write and the
//! uncached read answer a seller the same blank 401 an anonymous caller gets,
//! as every other operator route does.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::site::{maintenance_gate, SiteView, ThemeName};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_OPERATOR: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_SELLER: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
/// 2026-10-15T12:00:00Z: inside October, so a Halloween theme set for the
/// month is showing.
const MID_OCTOBER: Timestamp = Timestamp(1_792_065_600_000);
/// 2100-01-01: long after every instant this file reads the clock at.
const SESSION_EXPIRY: Timestamp = Timestamp(4_102_444_800_000);

fn state(pool: PgPool) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || MID_OCTOBER,
        auth: None,
        // None, deliberately: the switches live on the application pool, so a
        // deployment with no operator database still serves them.
        backoffice: None,
        blobs: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_OPERATOR, "org-operator"), (ORG_SELLER, "org-seller")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_OPERATOR, USER_OPERATOR, "operator@example.test", TOKEN_OPERATOR),
        (ORG_SELLER, USER_SELLER, "seller@example.test", TOKEN_SELLER),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
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

fn cookie(token: &SessionToken) -> String {
    format!("{SESSION_COOKIE}={}", token.to_hex())
}

struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
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
        request = request.header(header::COOKIE, cookie(token));
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
    let cache_control = response
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer {
        status,
        cache_control,
        body,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn site(answer: &Answer) -> SiteView {
    serde_json::from_slice(&answer.body).expect("the body is the site view")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn anyone_reads_the_switches_and_a_fresh_site_has_them_all_off(pool: PgPool) {
    let answer = call(state(pool), Method::GET, "/v1/site", None, None).await;
    assert_eq!(answer.status, StatusCode::OK, "no session is needed");
    assert_eq!(
        answer.cache_control.as_deref(),
        Some("public, max-age=60"),
        "the public read is cached for a minute"
    );
    let view = site(&answer);
    assert!(!view.maintenance.on);
    assert_eq!(view.theme.name, ThemeName::None);
    assert!(!view.theme.active);
    assert_eq!(view.banner, None);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn only_an_operator_reads_uncached_or_writes(pool: PgPool) {
    provision(&pool).await;
    let patch = serde_json::json!({"maintenance": {"on": true, "message": null}});
    for token in [None, Some(&TOKEN_SELLER)] {
        let read = call(state(pool.clone()), Method::GET, "/v1/admin/site", token, None).await;
        assert_eq!(read.status, StatusCode::UNAUTHORIZED, "a non-operator cannot read");
        let write = call(
            state(pool.clone()),
            Method::PATCH,
            "/v1/admin/site",
            token,
            Some(patch.clone()),
        )
        .await;
        assert_eq!(write.status, StatusCode::UNAUTHORIZED, "a non-operator cannot write");
    }
    let view = site(&call(state(pool.clone()), Method::GET, "/v1/site", None, None).await);
    assert!(!view.maintenance.on, "the refused writes changed nothing");

    let read = call(
        state(pool),
        Method::GET,
        "/v1/admin/site",
        Some(&TOKEN_OPERATOR),
        None,
    )
    .await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(
        read.cache_control.as_deref(),
        Some("no-store"),
        "the operator's read is never a stale copy"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_patch_changes_only_what_it_names(pool: PgPool) {
    provision(&pool).await;
    let first = call(
        state(pool.clone()),
        Method::PATCH,
        "/v1/admin/site",
        Some(&TOKEN_OPERATOR),
        Some(serde_json::json!({
            "theme": {"name": "halloween", "from": "2026-10-01", "until": "2026-10-31"},
            "banner": {"text": "  Spooky sale  ", "href": "/pricing/"}
        })),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    let view = site(&first);
    assert_eq!(view.theme.name, ThemeName::Halloween);
    assert!(view.theme.active, "mid-October is inside the theme's days");
    assert_eq!(
        view.banner.as_ref().map(|banner| banner.text.as_str()),
        Some("Spooky sale"),
        "the banner text is stored trimmed"
    );

    let second = site(
        &call(
            state(pool.clone()),
            Method::PATCH,
            "/v1/admin/site",
            Some(&TOKEN_OPERATOR),
            Some(serde_json::json!({"maintenance": {"on": true, "message": "  Back by 3pm UTC. "}})),
        )
        .await,
    );
    assert!(second.maintenance.on);
    assert_eq!(second.maintenance.message.as_deref(), Some("Back by 3pm UTC."));
    assert_eq!(second.theme.name, ThemeName::Halloween, "an absent theme is left alone");
    assert!(second.banner.is_some(), "an absent banner is left alone");

    let third = site(
        &call(
            state(pool),
            Method::PATCH,
            "/v1/admin/site",
            Some(&TOKEN_OPERATOR),
            Some(serde_json::json!({"banner": null})),
        )
        .await,
    );
    assert_eq!(third.banner, None, "an explicit null removes the banner");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_patch_with_one_bad_field_writes_nothing(pool: PgPool) {
    provision(&pool).await;
    let answer = call(
        state(pool.clone()),
        Method::PATCH,
        "/v1/admin/site",
        Some(&TOKEN_OPERATOR),
        Some(serde_json::json!({
            "maintenance": {"on": true, "message": null},
            "theme": {"name": "christmas", "from": "2026-12-31", "until": "2026-12-01"}
        })),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    let view = site(&call(state(pool), Method::GET, "/v1/site", None, None).await);
    assert!(!view.maintenance.on, "the valid half was not written either");
    assert_eq!(view.theme.name, ThemeName::None);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_maintenance_gate_lets_only_an_operator_through(pool: PgPool) {
    provision(&pool).await;
    let headers = |token: Option<&SessionToken>| {
        let mut headers = HeaderMap::new();
        if let Some(token) = token {
            if let Ok(value) = HeaderValue::from_str(&cookie(token)) {
                headers.insert(header::COOKIE, value);
            }
        }
        headers
    };
    let app = state(pool.clone());
    assert_eq!(
        maintenance_gate(&app, &headers(None)).await,
        None,
        "maintenance is off until somebody turns it on"
    );

    let _on = call(
        state(pool),
        Method::PATCH,
        "/v1/admin/site",
        Some(&TOKEN_OPERATOR),
        Some(serde_json::json!({"maintenance": {"on": true, "message": "Back soon"}})),
    )
    .await;
    for (who, token) in [("a visitor", None), ("a seller", Some(&TOKEN_SELLER))] {
        let gated = maintenance_gate(&app, &headers(token)).await;
        assert_eq!(
            gated.and_then(|maintenance| maintenance.message).as_deref(),
            Some("Back soon"),
            "{who} is shown the maintenance page"
        );
    }
    assert_eq!(
        maintenance_gate(&app, &headers(Some(&TOKEN_OPERATOR))).await,
        None,
        "an operator uses the site as usual, so they can turn maintenance off"
    );
}
