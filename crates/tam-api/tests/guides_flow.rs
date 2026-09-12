//! The help corpus over the wire: what an operator writes, what a seller can
//! see of it, and what happens to markup inside a guide body.
//!
//! Two properties carry this file. A draft is invisible to a seller and
//! answers exactly as a slug nobody has used, because a 404 that told them
//! apart would publish the titles of unpublished guides. And raw HTML in a
//! body arrives escaped, because the console renders the answered HTML into
//! its own page: if `<script>` survived the render, this server would be
//! shipping the console a script to run.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::guides::{GuideImageView, GuideView, GuidesView, PublishedGuideView};
use tam_api::{router, AppState, BlobStore, Config, SESSION_COOKIE};
use tam_storage::{OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_OPERATOR: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_SELLER: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

/// A body carrying both things the renderer has to get right: markup that
/// must not survive as markup, and a table that must.
const BODY: &str = "<script>alert(1)</script>\n\n\
                    | Marketplace | Ships |\n| --- | --- |\n| TPT | yes |\n";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn store_root(name: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("guides-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("the store root is creatable");
    root
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn state(pool: PgPool, root: Option<&std::path::Path>) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        // No backoffice pool anywhere in this file, and that is part of what
        // it proves: the guide corpus is global and lives on the application
        // pool, so it serves a deployment that configures no operator
        // database at all.
        backoffice: None,
        blobs: root.map(|root| BlobStore {
            kek: tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
            root: root.to_path_buf(),
        }),
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
            .mint(&token, user, Timestamp(100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", NOW)
        .await
        .expect("the operator marking lands");
}

struct Call<'a> {
    method: Method,
    path: &'a str,
    body: Option<Body>,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    state: AppState,
    token: Option<&SessionToken>,
    call: Call<'_>,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let Call { method, path, body } = call;
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let response = router(state)
        .oneshot(
            request
                .body(body.unwrap_or_else(Body::empty))
                .expect("the request builds"),
        )
        .await
        .expect("the router serves");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes()
        .to_vec();
    (status, content_type, bytes)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    serde_json::from_slice(bytes).expect("the answer body parses")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_draft_is_invisible_until_it_is_published_and_its_markup_arrives_escaped(pool: PgPool) {
    provision(&pool).await;

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides",
            body: Some(Body::from(
                serde_json::json!({
                    "slug": "getting-started",
                    "title": "Getting started",
                    "body": BODY,
                    "status": "draft",
                })
                .to_string(),
            )),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let written: GuideView = json(&body);
    assert_eq!(
        written.body, BODY,
        "the editor reads back exactly the Markdown it sent; the rendering is a \
         second field and not a rewrite of the first"
    );

    // The seller, before publication.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/getting-started",
            body: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a draft answers a seller exactly as a slug nobody has used: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed: GuidesView = json(&body);
    assert!(
        listed.guides.is_empty(),
        "nor is a draft's title listed, which is the same disclosure by another route"
    );

    // Published.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::PUT,
            path: "/v1/admin/guides/getting-started",
            body: Some(Body::from(
                serde_json::json!({
                    "title": "Getting started",
                    "body": BODY,
                    "status": "published",
                })
                .to_string(),
            )),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/getting-started",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let read: PublishedGuideView = json(&body);
    assert!(
        read.html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "raw HTML in a body is escaped into text, so the console can render this \
         answer into its own page: {}",
        read.html
    );
    assert!(
        !read.html.contains("<script"),
        "and no script tag survives anywhere in it: {}",
        read.html
    );
    assert!(
        read.html.contains("<table>") && read.html.contains("<td>TPT</td>"),
        "while the Markdown a guide is written in still renders, tables included: {}",
        read.html
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_guide_picture_is_readable_by_a_seller_whose_own_upload_route_cannot_see_it(
    pool: PgPool,
) {
    provision(&pool).await;
    let root = store_root("picture");

    let (status, _kind, body) = call(
        state(pool.clone(), Some(&root)),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/images",
            body: Some(Body::from(tiny_jpeg())),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let stored: GuideImageView = json(&body);

    let (status, kind, bytes) = call(
        state(pool.clone(), Some(&root)),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: &format!("/v1/guides/images/{}", stored.handle),
            body: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a seller in another organisation reads the picture the guide points at"
    );
    assert_eq!(kind.as_deref(), Some("image/jpeg"));
    assert_eq!(
        bytes,
        tiny_jpeg(),
        "and reads the bytes that were sealed, not an error document"
    );

    // The same handle through the tenant's own upload route, which pins the
    // reader's organisation: these bytes were sealed under the platform's, so
    // they are as absent there as bytes nobody stored. That is the whole
    // reason `/guides/images/{handle}` exists as a separate route.
    let (status, _kind, _body) = call(
        state(pool.clone(), Some(&root)),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: &format!("/v1/uploads/{}", stored.handle),
            body: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a guide picture is not reachable through a tenant-pinned read"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_can_neither_write_a_guide_nor_upload_a_picture_for_one(pool: PgPool) {
    provision(&pool).await;
    let root = store_root("refusal");

    for (path, body) in [
        (
            "/v1/admin/guides",
            Body::from(
                serde_json::json!({
                    "slug": "mine", "title": "Mine", "body": "hello", "status": "published",
                })
                .to_string(),
            ),
        ),
        ("/v1/admin/guides/images", Body::from(tiny_jpeg())),
    ] {
        let (status, _kind, refused) = call(
            state(pool.clone(), Some(&root)),
            Some(&TOKEN_SELLER),
            Call {
                method: Method::POST,
                path,
                body: Some(body),
            },
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a seller's live session is refused on {path} exactly as an anonymous \
             caller is: {}",
            String::from_utf8_lossy(&refused)
        );

        // The same method, because the image route serves only POST and a GET
        // there would be answered by the router's method table rather than by
        // the operator fence -- which is what this is asserting.
        let (status, _kind, anonymous) = call(
            state(pool.clone(), Some(&root)),
            None,
            Call {
                method: Method::POST,
                path,
                body: Some(Body::empty()),
            },
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            refused, anonymous,
            "and the two refusals on {path} are byte-identical, or the difference \
             is itself the answer a prober wanted"
        );
    }

    // Nothing was written by either refusal, which is the half a status code
    // does not assert.
    let guides: i64 = sqlx::query_scalar("SELECT count(*) FROM guide")
        .fetch_one(&pool)
        .await
        .unwrap_or(-1);
    assert_eq!(guides, 0, "a refused write leaves no guide behind");
}

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

/// The same eight-by-eight JPEG `catalogue_flow` uploads, so both files send
/// the pipeline a picture it genuinely accepts rather than a byte string that
/// happens to start with the right magic.
const TINY_JPEG_BASE64: &str = "\
/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAA0JCgsKCA0LCgsODg0PEyAVExISEyccHhcgLikxMC4pLSwz\
Oko+MzZGNywtQFdBRkxOUlNSMj5aYVpQYEpRUk//2wBDAQ4ODhMREyYVFSZPNS01T09PT09PT09PT09P\
T09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT09PT0//wAARCAAIAAgDASIAAhEBAxEB/8QA\
FQABAQAAAAAAAAAAAAAAAAAAAAT/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAA\
AAAABf/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AIgDYF//2Q==";
