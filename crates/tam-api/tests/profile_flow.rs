//! The seller's own profile picture over the wire: set from an upload's own
//! handle, read back as bytes by its owner alone, cleared, and refused where
//! the handle is not a picture, not this organisation's, or not a handle.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::profile::ProfileView;
use tam_api::{
    router, APIError, APIErrorCode, AppState, BlobStore, Config, UploadedView, SESSION_COOKIE,
};
use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("profile-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("the store root is creatable");
    root
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn configured(pool: PgPool, root: &std::path::Path) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: Some(BlobStore::local(
            tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
            root.to_path_buf(),
        )),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool, org: OrgId, user: UserId, token: &SessionToken, name: &str) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(name)
        .execute(pool)
        .await
        .expect("the org seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(org, user, &format!("{name}@example.test"), Timestamp(1_000))
        .await
        .expect("the user provisions");
    sessions
        .mint(token, user, Timestamp(100_000), Timestamp(1_000))
        .await
        .expect("the session mints");
}

/// One answer: the status, the content type, and the body.
struct Answer {
    status: StatusCode,
    kind: Option<String>,
    body: Vec<u8>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    state: AppState,
    token: Option<&SessionToken>,
    method: Method,
    path: &str,
    body: Option<(&str, Vec<u8>)>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    let request = match body {
        Some((kind, bytes)) => request
            .header(header::CONTENT_TYPE, kind)
            .body(Body::from(bytes)),
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state)
        .oneshot(request)
        .await
        .expect("the router serves");
    let status = response.status();
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    Answer { status, kind, body }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).expect("the body parses")
}

async fn upload(state: AppState, token: &SessionToken, bytes: Vec<u8>, query: &str) -> String {
    let answer = call(
        state,
        Some(token),
        Method::POST,
        &format!("/v1/uploads{query}"),
        Some(("application/octet-stream", bytes)),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "the upload is accepted: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let uploaded: UploadedView = parse(&answer.body);
    uploaded.payload[0].hash.clone()
}

async fn put_avatar(state: AppState, token: &SessionToken, body: &serde_json::Value) -> Answer {
    call(
        state,
        Some(token),
        Method::PUT,
        "/v1/profile/avatar",
        Some(("application/json", body.to_string().into_bytes())),
    )
    .await
}

async fn profile(state: AppState, token: &SessionToken) -> ProfileView {
    let answer = call(state, Some(token), Method::GET, "/v1/profile", None).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "the profile reads: {}",
        String::from_utf8_lossy(&answer.body)
    );
    parse(&answer.body)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn refusal_of(body: &[u8]) -> (Option<APIErrorCode>, String) {
    let error: APIError = parse(body);
    let entry = error.errors.first().expect("a refusal carries an entry");
    (entry.code, entry.message.clone())
}

fn pdf(marker: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    bytes.extend_from_slice(marker.as_bytes());
    bytes
}

/// An 8x8 JPEG, generated once with `magick -size 8x8 xc:'#2080C0' -quality
/// 60 tiny.jpg`, the same fixture the catalogue tests upload.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn tiny_jpeg() -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(TINY_JPEG_BASE64)
        .expect("the fixture decodes")
}

const TINY_JPEG_BASE64: &str = "\
/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAA0JCgsKCA0LCgsODg0PEyAVExISEyccHhcgLikxMC4pLSwz\
Oko+MzZGNywtQFdBRkxOUlNSMj5aYVpQYEpRUk//2wBDAQ4ODhMREyYVFSZPNS01T09PT09PT09PT09P\
T09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT0//wAARCAAIAAgDASIAAhEBAxEB/8QA\
FQABAQAAAAAAAAAAAAAAAAAAAAT/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAA\
AAAABf/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AIgDYF//2Q==";

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_sets_reads_and_clears_their_picture(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("round-trip");
    let state = configured(pool, &root);

    let before = profile(state.clone(), &TOKEN_A).await;
    assert_eq!(
        before,
        ProfileView {
            user: USER_A,
            avatar_hash: None
        },
        "a fresh user has no picture"
    );
    let none = call(
        state.clone(),
        Some(&TOKEN_A),
        Method::GET,
        "/v1/profile/avatar",
        None,
    )
    .await;
    assert_eq!(
        none.status,
        StatusCode::NOT_FOUND,
        "no picture is a 404, not a fault"
    );

    let hash = upload(
        state.clone(),
        &TOKEN_A,
        tiny_jpeg(),
        "?slot=image&archive=keep_whole",
    )
    .await;
    let set = put_avatar(
        state.clone(),
        &TOKEN_A,
        &serde_json::json!({ "hash": hash }),
    )
    .await;
    assert_eq!(
        set.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&set.body)
    );
    let stored: ProfileView = parse(&set.body);
    assert_eq!(stored.avatar_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(
        profile(state.clone(), &TOKEN_A).await,
        stored,
        "the profile read answers what the write answered"
    );

    let bytes = call(
        state.clone(),
        Some(&TOKEN_A),
        Method::GET,
        "/v1/profile/avatar",
        None,
    )
    .await;
    assert_eq!(bytes.status, StatusCode::OK);
    assert_eq!(
        bytes.kind.as_deref(),
        Some("image/jpeg"),
        "the type is read off the bytes"
    );
    assert_eq!(bytes.body, tiny_jpeg(), "the bytes that went in come back");

    let cleared = call(
        state.clone(),
        Some(&TOKEN_A),
        Method::DELETE,
        "/v1/profile/avatar",
        None,
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK);
    let cleared: ProfileView = parse(&cleared.body);
    assert_eq!(cleared.avatar_hash, None);
    let gone = call(
        state,
        Some(&TOKEN_A),
        Method::GET,
        "/v1/profile/avatar",
        None,
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

/// A worksheet's handle is refused as a picture, and the profile is untouched.
/// The bytes are read back to decide it: the handle carries no kind and the
/// client's word is not asked.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn bytes_that_are_not_a_picture_are_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    let root = store_root("not-a-picture");
    let state = configured(pool, &root);
    let worksheet = upload(state.clone(), &TOKEN_A, pdf("a worksheet"), "").await;

    let refused = put_avatar(
        state.clone(),
        &TOKEN_A,
        &serde_json::json!({ "hash": worksheet }),
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&refused.body)
    );
    let (code, said) = refusal_of(&refused.body);
    assert_eq!(code, Some(APIErrorCode::UploadRejected));
    assert!(
        said.contains("profile picture") && said.contains("JPEG"),
        "the sentence names this slot and the formats it takes: {said}"
    );
    assert_eq!(
        profile(state, &TOKEN_A).await.avatar_hash,
        None,
        "nothing was written"
    );
}

/// A handle from another organisation's upload is refused exactly as one
/// nobody uploaded is, and a handle that is not a handle is a validation
/// refusal before anything is looked up.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_organisations_upload_and_a_malformed_handle_are_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    let root = store_root("theirs");
    let state = configured(pool, &root);
    let theirs = upload(
        state.clone(),
        &TOKEN_B,
        tiny_jpeg(),
        "?slot=image&archive=keep_whole",
    )
    .await;

    let refused = put_avatar(
        state.clone(),
        &TOKEN_A,
        &serde_json::json!({ "hash": theirs }),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let (code, _) = refusal_of(&refused.body);
    assert_eq!(code, Some(APIErrorCode::UploadRejected));

    let malformed = put_avatar(
        state.clone(),
        &TOKEN_A,
        &serde_json::json!({ "hash": "not-a-handle" }),
    )
    .await;
    assert_eq!(malformed.status, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(profile(state, &TOKEN_A).await.avatar_hash, None);
}

/// No route names a user, so a body that tries to is refused rather than
/// narrowed, and another user's session reads its own empty profile rather
/// than the picture the first user set.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_picture_is_the_owners_alone(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    let root = store_root("owner-alone");
    let state = configured(pool, &root);
    let mine = upload(
        state.clone(),
        &TOKEN_A,
        tiny_jpeg(),
        "?slot=image&archive=keep_whole",
    )
    .await;

    let named = put_avatar(
        state.clone(),
        &TOKEN_B,
        &serde_json::json!({ "hash": mine, "user": USER_A }),
    )
    .await;
    assert!(
        named.status.is_client_error(),
        "a body naming a user is refused, answered {}",
        named.status
    );

    let set = put_avatar(
        state.clone(),
        &TOKEN_A,
        &serde_json::json!({ "hash": mine }),
    )
    .await;
    assert_eq!(set.status, StatusCode::OK);

    assert_eq!(
        profile(state.clone(), &TOKEN_B).await,
        ProfileView {
            user: USER_B,
            avatar_hash: None
        },
        "the other user's profile is their own, and carries no picture"
    );
    let theirs = call(
        state.clone(),
        Some(&TOKEN_B),
        Method::GET,
        "/v1/profile/avatar",
        None,
    )
    .await;
    assert_eq!(
        theirs.status,
        StatusCode::NOT_FOUND,
        "and their picture read answers nothing rather than the first user's bytes"
    );
    let ours = call(
        state,
        Some(&TOKEN_A),
        Method::GET,
        "/v1/profile/avatar",
        None,
    )
    .await;
    assert_eq!(ours.status, StatusCode::OK);
}

/// Every profile route is behind the session, which is the whole of how the
/// picture is one user's: there is no route here a caller reaches without one.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn every_profile_route_is_behind_the_session(pool: PgPool) {
    let root = store_root("no-session");
    let state = configured(pool, &root);
    for (method, path) in [
        (Method::GET, "/v1/profile"),
        (Method::GET, "/v1/profile/avatar"),
        (Method::PUT, "/v1/profile/avatar"),
        (Method::DELETE, "/v1/profile/avatar"),
    ] {
        let answer = call(
            state.clone(),
            None,
            method.clone(),
            path,
            Some(("application/json", b"{}".to_vec())),
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must refuse before it reads anything"
        );
    }
}
