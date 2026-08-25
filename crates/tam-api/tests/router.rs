//! The router driven in-process. No listener is bound and no port is chosen,
//! which is the whole reason `tam-api` is a library rather than a binary.

use axum::{
    body::Body,
    http::{header::CONTENT_TYPE, Request, StatusCode},
};
use http_body_util::BodyExt;
use tam_api::{router, APIError, APIErrorCode, APIErrorKind, APIVersion, Config, Health};
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

#[expect(
    clippy::expect_used,
    reason = "clippy's allow-expect-in-tests reaches #[test] functions and #[cfg(test)] modules, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
async fn get(path: &str) -> Answer {
    let request = Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("the test request is well formed");
    let response = router(Config::default())
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
