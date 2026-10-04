//! The operators' site analytics page over the wire, against a loopback
//! PostHog double that answers the query API and records every call.
//!
//! What is proven: only an operator reads it; an unconfigured deployment says
//! so with a 503; the eleven queries go to the project's query endpoint with
//! the personal key as the bearer and are folded into one page; a second read
//! inside five minutes asks PostHog nothing; a refused key is a 502 with a
//! sentence and is not remembered.

#![cfg(feature = "pg-tests")]

use core::sync::atomic::{AtomicI64, AtomicU16, Ordering};
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, Method, Request, StatusCode},
    routing::post,
    Json,
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::admin_analytics::{PersonalKey, Range, SiteAnalytics, SiteAnalyticsView};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_storage::{OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tokio::sync::Mutex;
use tower::ServiceExt;

const ORG_OPERATOR: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_SELLER: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);

/// 2026-10-04 09:00 in New Zealand.
const NOW: Timestamp = Timestamp(1_791_057_600_000);
const PROJECT: &str = "4242";
const KEY: &str = "phx_test_development_only";

/// The clock every state in this file reads. A static because `AppState`'s
/// wall is a plain `fn`, and the cache test has to move it.
static CLOCK: AtomicI64 = AtomicI64::new(NOW.0);

fn wall() -> Timestamp {
    Timestamp(CLOCK.load(Ordering::SeqCst))
}

/// What the double saw: each call's project, bearer and body.
#[derive(Default)]
struct Double {
    calls: Vec<(String, Option<String>, serde_json::Value)>,
}

struct Shared {
    seen: Mutex<Double>,
    /// The status every call answers; 200 serves the fixtures.
    status: AtomicU16,
}

type Handle = Arc<Shared>;

/// The fixture rows each named query answers, in the column order the module
/// selects them.
fn rows_for(name: &str) -> serde_json::Value {
    match name {
        "teachouse admin site: totals" => serde_json::json!([[180, 64]]),
        "teachouse admin site: daily" => {
            serde_json::json!([["2026-09-30", 40, 15], ["2026-10-04", 12, 6]])
        }
        "teachouse admin site: pages" => serde_json::json!([["/", 50, 120], ["/pricing", 20, 31]]),
        "teachouse admin site: referrers" => {
            serde_json::json!([["$direct", 30, 70], ["www.google.com", 18, 40]])
        }
        "teachouse admin site: utm sources" => serde_json::json!([["newsletter", 9, 11]]),
        "teachouse admin site: countries" => {
            serde_json::json!([["New Zealand", 50, 150], [null, 2, 2]])
        }
        "teachouse admin site: cities" => {
            serde_json::json!([["Auckland", "New Zealand", 21, 60]])
        }
        "teachouse admin site: devices" => serde_json::json!([["Mobile", 40, 90]]),
        "teachouse admin site: browsers" => serde_json::json!([["Chrome", 30, 80]]),
        "teachouse admin site: operating systems" => serde_json::json!([["iOS", 25, 60]]),
        "teachouse admin site: conversions" => serde_json::json!([[4, 19]]),
        _ => serde_json::json!([]),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn posthog_double() -> (String, Handle) {
    let shared: Handle = Arc::new(Shared {
        seen: Mutex::default(),
        status: AtomicU16::new(200),
    });
    let app = axum::Router::new()
        .route(
            "/api/projects/{project}/query/",
            post(
                |State(shared): State<Handle>,
                 Path(project): Path<String>,
                 headers: HeaderMap,
                 Json(body): Json<serde_json::Value>| async move {
                    let bearer = headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let name = body["name"].as_str().unwrap_or_default().to_owned();
                    shared.seen.lock().await.calls.push((project, bearer, body));
                    let status = StatusCode::from_u16(shared.status.load(Ordering::SeqCst))
                        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                    if status != StatusCode::OK {
                        return (
                            status,
                            Json(serde_json::json!({
                                "type": "authentication_error",
                                "detail": "Invalid personal API key."
                            })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "columns": [], "types": [], "hogql": "",
                            "results": rows_for(&name)
                        })),
                    )
                },
            ),
        )
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the double binds a loopback port");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the double has an address")
    );
    tam_api::blocking::spawn_supervised("posthog double", async move {
        let _served: Result<(), std::io::Error> = axum::serve(listener, app).await;
    });
    (base, shared)
}

fn state(pool: PgPool, base: Option<&str>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config {
            site_analytics: base.map(|base| {
                SiteAnalytics::new(
                    PersonalKey::new(KEY.to_owned()),
                    base,
                    PROJECT.to_owned(),
                    Some("teachouse.io".to_owned()),
                )
            }),
            ..Config::default()
        },
        wall,
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
    for (org, name) in [(ORG_OPERATOR, "Teachouse"), (ORG_SELLER, "Kauri Room")] {
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
            ORG_OPERATOR,
            USER_OPERATOR,
            "operator@example.test",
            TOKEN_OPERATOR,
        ),
        (ORG_SELLER, USER_SELLER, "seller@example.test", TOKEN_SELLER),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(
                &token,
                user,
                Timestamp(NOW.0 + 86_400_000),
                Timestamp(1_000),
            )
            .await
            .expect("the session mints");
    }
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", Timestamp(1_000))
        .await
        .expect("the operator marking lands");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn get(state: AppState, token: Option<&SessionToken>, path: &str) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(Method::GET).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    let response = router(state)
        .oneshot(request.body(Body::empty()).expect("the request builds"))
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn message(bytes: &[u8]) -> String {
    let body: serde_json::Value = serde_json::from_slice(bytes).expect("an error body parses");
    body["errors"][0]["message"]
        .as_str()
        .expect("the error carries a message")
        .to_owned()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn only_an_operator_reads_site_analytics(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = posthog_double().await;
    for token in [Some(&TOKEN_SELLER), None] {
        let (status, _) = get(
            state(pool.clone(), Some(&base)),
            token,
            "/v1/admin/analytics/site",
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?} is refused");
    }
    assert!(
        double.seen.lock().await.calls.is_empty(),
        "a refused caller costs PostHog nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unconfigured_deployment_says_so(pool: PgPool) {
    provision(&pool).await;
    let (status, bytes) = get(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site?range=30d",
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(message(&bytes), "Site analytics is not configured");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_range_outside_the_three_is_refused(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = posthog_double().await;
    let (status, bytes) = get(
        state(pool.clone(), Some(&base)),
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site?range=365d",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(message(&bytes), "Pick a range of 7d, 30d or 90d.");
    assert!(double.seen.lock().await.calls.is_empty(), "nothing asked");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_page_is_folded_from_posthog_and_remembered_five_minutes(pool: PgPool) {
    provision(&pool).await;
    CLOCK.store(NOW.0, Ordering::SeqCst);
    let (base, double) = posthog_double().await;
    let served = state(pool.clone(), Some(&base));

    let (status, bytes) = get(
        served.clone(),
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site?range=7d",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let page: SiteAnalyticsView = serde_json::from_slice(&bytes).expect("the page parses");
    assert_eq!(page.range, Range::Week);
    assert_eq!(
        (page.from.as_str(), page.to.as_str()),
        ("2026-09-28", "2026-10-04")
    );
    assert_eq!(page.site_host.as_deref(), Some("teachouse.io"));
    assert_eq!(
        (
            page.totals.visitors,
            page.totals.pageviews,
            page.totals.signups,
            page.totals.cta_clicks
        ),
        (64, 180, 4, 19)
    );
    assert_eq!(page.days.len(), 7, "every day of the week");
    assert_eq!(page.days[2].day, "2026-09-30");
    assert_eq!((page.days[2].pageviews, page.days[2].visitors), (40, 15));
    assert_eq!(page.days[0].pageviews, 0, "a quiet day is zero");
    assert_eq!(page.pages[1].label.as_deref(), Some("/pricing"));
    assert_eq!(page.referrers[0].label.as_deref(), Some("$direct"));
    assert_eq!(page.utm_sources[0].label.as_deref(), Some("newsletter"));
    assert_eq!(page.countries[1].label, None, "an unknown country is None");
    assert_eq!(page.cities[0].detail.as_deref(), Some("New Zealand"));
    assert_eq!(page.devices[0].label.as_deref(), Some("Mobile"));
    assert_eq!(page.browsers[0].label.as_deref(), Some("Chrome"));
    assert_eq!(page.systems[0].label.as_deref(), Some("iOS"));
    assert_eq!(page.fetched_at, NOW);

    let calls = double.seen.lock().await.calls.clone();
    assert_eq!(calls.len(), 11, "one query per section");
    for (project, bearer, body) in &calls {
        assert_eq!(project, PROJECT, "the configured project");
        assert_eq!(
            bearer.as_deref(),
            Some(format!("Bearer {KEY}").as_str()),
            "the personal key is the bearer"
        );
        assert_eq!(body["query"]["kind"], "HogQLQuery");
    }
    let daily = calls
        .iter()
        .find(|(_, _, body)| body["name"] == "teachouse admin site: daily")
        .expect("the daily query was asked");
    assert!(
        daily.2["query"]["query"]
            .as_str()
            .is_some_and(|sql| sql.contains("properties.$host = 'teachouse.io'")),
        "pageviews are the landing's"
    );

    // Four minutes later: the same page, nothing asked.
    CLOCK.store(NOW.0 + 4 * 60_000, Ordering::SeqCst);
    let (status, again) = get(
        served.clone(),
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again, bytes, "the remembered page, byte for byte");
    assert_eq!(
        double.seen.lock().await.calls.len(),
        11,
        "served from memory"
    );

    // Another range is its own entry.
    let (status, _) = get(
        served.clone(),
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site?range=90d",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        double.seen.lock().await.calls.len(),
        22,
        "90 days asked once"
    );

    // Past five minutes: asked again.
    CLOCK.store(NOW.0 + 6 * 60_000, Ordering::SeqCst);
    let (status, _) = get(
        served,
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site?range=7d",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        double.seen.lock().await.calls.len(),
        33,
        "stale, so fetched"
    );
    CLOCK.store(NOW.0, Ordering::SeqCst);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refused_key_is_a_sentence_and_is_not_remembered(pool: PgPool) {
    provision(&pool).await;
    let (base, double) = posthog_double().await;
    double.status.store(403, Ordering::SeqCst);
    let served = state(pool.clone(), Some(&base));
    let (status, bytes) = get(
        served.clone(),
        Some(&TOKEN_OPERATOR),
        "/v1/admin/analytics/site",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        message(&bytes),
        "PostHog refused this server's key. Check it has Query Read on the project."
    );
    let refused = double.seen.lock().await.calls.len();
    assert!(refused >= 1, "PostHog was asked");

    double.status.store(200, Ordering::SeqCst);
    let (status, _) = get(served, Some(&TOKEN_OPERATOR), "/v1/admin/analytics/site").await;
    assert_eq!(status, StatusCode::OK, "the failure was not cached");
    assert_eq!(
        double.seen.lock().await.calls.len(),
        refused + 11,
        "asked afresh"
    );
}
