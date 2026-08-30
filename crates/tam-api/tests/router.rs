//! The router driven in-process. No listener is bound and no port is chosen,
//! which is the whole reason `tam-api` is a library rather than a binary.

use axum::{
    body::Body,
    http::{header::CONTENT_TYPE, Request, StatusCode},
};
use http_body_util::BodyExt;
use tam_api::{router, APIError, APIErrorCode, APIErrorKind, APIVersion, AppState, Config, Health};
use tam_types::Timestamp;
use tower::ServiceExt;

struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "clippy's allow-expect-in-tests reaches #[test] functions and #[cfg(test)] modules, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).expect("the answer body is the JSON the test expects")
    }
}

/// A state whose pool never dials: `connect_lazy` defers until first use,
/// and the hermetic routes here never use it.
#[expect(
    clippy::expect_used,
    reason = "clippy's allow-expect-in-tests reaches #[test] functions and #[cfg(test)] modules, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
fn test_state() -> AppState {
    AppState {
        pool: sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(core::time::Duration::from_millis(200))
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("a lazy pool parses its url without dialling"),
        config: Config::default(),
        wall: || Timestamp(0),
        auth: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "clippy's allow-expect-in-tests reaches #[test] functions and #[cfg(test)] modules, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
async fn get(path: &str) -> Answer {
    let request = Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("the test request is well formed");
    let response = router(test_state())
        .oneshot(request)
        .await
        .expect("the router is infallible as a service");
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the answer body collects")
        .to_bytes()
        .to_vec();
    Answer {
        status,
        content_type,
        body,
    }
}

#[tokio::test]
async fn the_unversioned_probe_answers_two_hundred() {
    let answer = get("/healthz").await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the liveness probe answers 200 in-process, with no port bound"
    );
}

#[tokio::test]
async fn the_versioned_probe_answers_for_every_supported_version() {
    for version in APIVersion::SUPPORTED {
        let answer = get(&format!("/{version}/healthz")).await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "the probe answers 200 for {version}"
        );
        let health: Health = answer.json();
        assert_eq!(
            health.version, version,
            "the extractor hands the handler the version the path named"
        );
    }
}

#[tokio::test]
async fn an_unsupported_version_is_refused_with_the_structured_body() {
    let answer = get("/v9/healthz").await;
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "a version this build does not serve is the caller's error"
    );
    assert_eq!(
        answer.content_type.as_deref(),
        Some("application/json"),
        "the refusal is the structured body, not a bare status string"
    );

    let error: APIError = answer.json();
    assert_eq!(
        error.status, 400,
        "the body repeats the status of the envelope carrying it"
    );
    let entry = error.errors.first().expect("the refusal carries one entry");
    assert_eq!(
        entry.code,
        Some(APIErrorCode::UnsupportedApiVersion),
        "the client matches on the code rather than on the message"
    );
    assert_eq!(
        entry.kind,
        Some(APIErrorKind::Validation),
        "an unsupported version is a validation fault"
    );
    assert!(
        entry.message.contains("v9"),
        "the refusal names the token it refused: {}",
        entry.message
    );
    assert_eq!(
        entry.reason.as_deref(),
        Some("supported versions: v1, v2"),
        "the refusal states which versions would have worked"
    );
}

/// The contrast that gives the test above its meaning: a path no route matches
/// is a bare 404 from the router itself, so the structured body proves the
/// extractor ran rather than a catch-all handler.
#[tokio::test]
async fn an_unmounted_path_is_a_bare_not_found() {
    let answer = get("/v1/nothing-here").await;
    assert_eq!(
        answer.status,
        StatusCode::NOT_FOUND,
        "no route matches, so the router answers before any extractor runs"
    );
    assert!(
        answer.body.is_empty(),
        "the router's own not-found carries no structured body"
    );
}

#[tokio::test]
async fn a_protected_route_without_a_session_is_a_structured_401() {
    let answer = get("/v1/whoami").await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    let error: APIError = answer.json();
    assert_eq!(
        (error.errors[0].code, error.errors[0].kind),
        (
            Some(APIErrorCode::SessionRequired),
            Some(APIErrorKind::Unauthenticated)
        ),
        "the refusal is the closed vocabulary, not a bare status"
    );
}

/// The structural half of the OpenAPI parity: every documented operation must
/// be mounted. (The reverse — every mounted route documented — has no
/// introspection hook in axum and is held by review plus the one-constant
/// discipline.) A bare 404 means unmounted; a 405 means the documented method
/// is wrong; anything else, including 401 and 204, proves the route exists.
#[tokio::test]
async fn every_documented_operation_is_mounted() {
    for route in tam_api::openapi::ROUTES {
        let path = route
            .path
            .replace("{version}", "v1")
            .replace("{job}", "11111111-1111-4111-8111-111111111111")
            .replace("{item}", "11111111-1111-4111-8111-111111111111")
            .replace("{product}", "11111111-1111-4111-8111-111111111111")
            .replace("{connection}", "11111111-1111-4111-8111-111111111111");
        let method = route.method.to_uppercase();
        let request = Request::builder()
            .method(method.as_str())
            .uri(&path)
            .body(Body::empty())
            .expect("the probe request builds");
        let response = router(test_state())
            .oneshot(request)
            .await
            .expect("the router serves");
        assert!(
            response.status() != StatusCode::NOT_FOUND
                && response.status() != StatusCode::METHOD_NOT_ALLOWED,
            "documented {} {} is not mounted (answered {})",
            route.method,
            route.path,
            response.status()
        );
    }
}
