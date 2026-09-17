//! The help corpus over the wire: what an operator writes, what a seller can
//! see of it, and what happens to markup inside a guide body.
//!
//! Three properties carry this file. A draft is invisible to a seller and
//! answers exactly as a slug nobody has used, because a 404 that told them
//! apart would publish the titles of unpublished guides. Raw HTML in a body
//! arrives escaped, because the console renders the answered HTML into its own
//! page: if `<script>` survived the render, this server would be shipping the
//! console a script to run. And saving is not publishing — an operator
//! rewriting a published guide changes nothing a seller can read, search or
//! filter until they publish again, and a write that names a guide or a
//! revision other than the one that is stored is refused with the identifier
//! and revision that are, rather than overwriting somebody's paragraph — or
//! somebody's whole guide, when the address has changed hands.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::guides::{
    GuideImageView, GuidePreviewView, GuideTaxonView, GuideTaxonomyView, GuideView, GuidesView,
    PublishedGuideView,
};
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
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        // No backoffice pool anywhere in this file, and that is part of what
        // it proves: the guide corpus is global and lives on the application
        // pool, so it serves a deployment that configures no operator
        // database at all.
        backoffice: None,
        blobs: root.map(|root| {
            BlobStore::local(
                tam_secrets::Kek::from_bytes(&[0x7Cu8; 32]).expect("a 32-byte key is a key"),
                root.to_path_buf(),
            )
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

/// The JSON body, whatever it says: for reading a refusal's own fields rather
/// than a view's.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn value(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).expect("the refusal body parses")
}

fn body_of(value: &serde_json::Value) -> Body {
    Body::from(value.to_string())
}

/// One word in a guide vocabulary, created over the operator's own route.
async fn word(pool: &PgPool, kind: &str, slug: &str, name: &str) -> GuideTaxonView {
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: &format!("/v1/admin/guides/_taxonomy/{kind}"),
            body: Some(body_of(&serde_json::json!({ "slug": slug, "name": name }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the word stores: {}",
        String::from_utf8_lossy(&body)
    );
    json(&body)
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
            body: Some(body_of(&serde_json::json!({
                "slug": "getting-started",
                "title": "Getting started",
                "body": BODY,
            }))),
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
    assert_eq!(
        written.status, "draft",
        "a create is always a draft: there is no word a create can carry that \
         publishes what it writes"
    );
    assert_eq!(written.revision, 1, "and it starts at revision 1");
    assert!(
        written.published.is_none(),
        "with nothing published beside it"
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
    assert_eq!(status, StatusCode::OK, "the listing serves");
    let listed: GuidesView = json(&body);
    assert!(
        listed.guides.is_empty(),
        "nor is a draft's title listed, which is the same disclosure by another route"
    );
    assert_eq!(listed.total, 0, "and the count agrees with the rows");

    // Published, by the route that exists for it and at the revision the
    // operator read.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/getting-started/publish",
            body: Some(body_of(&serde_json::json!({
                "expected_id": written.id,
                "expected_revision": 1,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let live: GuideView = json(&body);
    assert_eq!(live.status, "published", "the guide is out");
    assert_eq!(live.revision, 2, "and publishing moved the revision on");
    assert_eq!(
        live.id, written.id,
        "and it is still the same guide: publishing writes the guide, it does \
         not replace it"
    );
    let snapshot = live.published.expect("the snapshot is answered");
    assert_eq!(
        snapshot.source_revision, 1,
        "the snapshot names the draft the publisher had read"
    );
    assert_eq!(snapshot.published_at, NOW, "and when it went out");

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
    assert_eq!(status, StatusCode::OK, "the page serves");
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
    assert_eq!(
        read.updated_at, NOW,
        "and the reader's update time is the publication"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_save_after_publication_reaches_no_seller_and_a_stale_write_is_refused(pool: PgPool) {
    provision(&pool).await;
    let selling = word(&pool, "topics", "selling", "Selling").await;
    let pricing = word(&pool, "topics", "pricing", "Pricing").await;
    let tpt = word(&pool, "tags", "tpt", "TPT").await;

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides",
            body: Some(body_of(&serde_json::json!({
                "slug": "fees",
                "title": "Fees",
                "body": "What a marketplace keeps.",
                "topic_id": selling.id,
                "tag_ids": [tpt.id],
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let created: GuideView = json(&body);
    let (status, _kind, _body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/fees/publish",
            body: Some(body_of(&serde_json::json!({
                "expected_id": created.id,
                "expected_revision": 1,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the guide is published");

    // The operator rewrites every field a seller can see.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::PUT,
            path: "/v1/admin/guides/fees",
            body: Some(body_of(&serde_json::json!({
                "title": "Fees, rewritten",
                "body": "An unfinished note about quarantined dragonfruit.",
                "topic_id": pricing.id,
                "tag_ids": [],
                "expected_id": created.id,
                "expected_revision": 2,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let saved: GuideView = json(&body);
    assert_eq!(saved.revision, 3, "the save moved the revision on");

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/fees",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the page still serves");
    let page: PublishedGuideView = json(&body);
    assert_eq!(
        page.title, "Fees",
        "the seller reads the published title, not the one being typed"
    );
    assert!(
        page.html.contains("What a marketplace keeps."),
        "and the published prose: {}",
        page.html
    );
    assert_eq!(
        page.topic.map(|topic| topic.slug),
        Some("selling".to_owned()),
        "filed where it was published, not where the draft moved it"
    );
    assert_eq!(
        page.tags.len(),
        1,
        "and carrying the tags it was published with, which the save removed"
    );

    // The unpublished phrase is not searchable, and the draft's topic is not a
    // filter that finds it.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides?q=dragonfruit",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the search serves");
    let found: GuidesView = json(&body);
    assert_eq!(
        found.total,
        0,
        "a phrase that exists only in an unsaved edit is unsearchable: {:?}",
        found
            .guides
            .iter()
            .map(|head| &head.slug)
            .collect::<Vec<_>>()
    );

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: &format!("/v1/guides?topic={}", pricing.id.to_hyphenated()),
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the filter serves");
    assert_eq!(
        json::<GuidesView>(&body).total,
        0,
        "and the topic the draft was moved to finds nothing"
    );

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: &format!(
                "/v1/guides?q=keeps&topic={}&tags={}",
                selling.id.to_hyphenated(),
                tpt.id.to_hyphenated(),
            ),
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the combined filter serves");
    let combined: GuidesView = json(&body);
    assert_eq!(
        combined.total, 1,
        "while the published text, topic and tag together still find it"
    );
    assert_eq!(
        combined.guides[0].updated_at, NOW,
        "shown at its publication time rather than its last autosave"
    );

    // A second operator still holding revision 2 cannot overwrite the save,
    // and cannot publish what they were looking at either.
    for path in ["/v1/admin/guides/fees", "/v1/admin/guides/fees/publish"] {
        let method = if path.ends_with("publish") {
            Method::POST
        } else {
            Method::PUT
        };
        let (status, _kind, refused) = call(
            state(pool.clone(), None),
            Some(&TOKEN_OPERATOR),
            Call {
                method,
                path,
                body: Some(body_of(&serde_json::json!({
                    "title": "Fees",
                    "body": "The other operator's paragraph.",
                    "topic_id": selling.id,
                    "tag_ids": [tpt.id],
                    "expected_id": created.id,
                    "expected_revision": 2,
                }))),
            },
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a stale write on {path} is refused: {}",
            String::from_utf8_lossy(&refused)
        );
        let refusal = value(&refused);
        let detail = &refusal["errors"][0]["detail"];
        assert_eq!(
            detail["expected_revision"],
            serde_json::json!(3),
            "and the refusal names the revision that is stored, which is what \
             the editor reconciles against: {}",
            String::from_utf8_lossy(&refused)
        );
        assert_eq!(
            detail["expected_id"],
            serde_json::json!(created.id.to_hyphenated()),
            "beside the guide it is stored under, which is how the console \
             tells an edit it can reconcile from a guide that is gone: {}",
            String::from_utf8_lossy(&refused)
        );
    }

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::GET,
            path: "/v1/admin/guides/fees",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the operator's read serves");
    let after: GuideView = json(&body);
    assert_eq!(
        after.body, "An unfinished note about quarantined dragonfruit.",
        "the refused writes changed nothing"
    );
    assert_eq!(after.revision, 3, "and moved no revision");
    assert_eq!(
        after.published.map(|snapshot| snapshot.title),
        Some("Fees".to_owned()),
        "and published nothing"
    );

    // A preview renders and writes nothing: no revision, no draft, no page.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/_preview",
            body: Some(body_of(&serde_json::json!({
                "body": "A preview of *something else* entirely.",
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let preview: GuidePreviewView = json(&body);
    assert!(
        preview.html.contains("<em>something else</em>"),
        "the preview renders: {}",
        preview.html
    );
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::GET,
            path: "/v1/admin/guides/fees",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the operator's read serves");
    let unchanged: GuideView = json(&body);
    assert_eq!(
        unchanged.revision, 3,
        "and the preview wrote nothing: the revision is where the last save \
         left it"
    );
    assert_eq!(
        unchanged.body, "An unfinished note about quarantined dragonfruit.",
        "and the draft is the draft"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_reader_is_offered_only_the_vocabulary_published_guides_use(pool: PgPool) {
    provision(&pool).await;
    let selling = word(&pool, "topics", "selling", "Selling").await;
    let unused = word(&pool, "topics", "unused", "Unused").await;
    let tpt = word(&pool, "tags", "tpt", "TPT").await;

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides",
            body: Some(body_of(&serde_json::json!({
                "slug": "fees",
                "title": "Fees",
                "body": "What a marketplace keeps.",
                "topic_id": selling.id,
                "tag_ids": [tpt.id],
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let created: GuideView = json(&body);

    // Before publication the reader's vocabulary is empty, because it is a
    // projection of what is published rather than of the table.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/_taxonomy",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the reader's taxonomy serves");
    let empty: GuideTaxonomyView = json(&body);
    assert!(
        empty.topics.is_empty() && empty.tags.is_empty(),
        "a word only a draft carries is not a filter a seller is offered"
    );

    let (status, _kind, _body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/fees/publish",
            body: Some(body_of(&serde_json::json!({
                "expected_id": created.id,
                "expected_revision": 1,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the guide is published");

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/_taxonomy",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the reader's taxonomy serves");
    let offered: GuideTaxonomyView = json(&body);
    assert_eq!(
        offered
            .topics
            .iter()
            .map(|topic| topic.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["selling"],
        "only the words published content is filed under"
    );
    assert_eq!(offered.tags.len(), 1, "and the published tags");

    // The operator's own taxonomy is the whole table, including the word
    // nothing uses yet: it is their management screen.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::GET,
            path: "/v1/admin/guides/_taxonomy",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the operator's taxonomy serves");
    assert_eq!(
        json::<GuideTaxonomyView>(&body).topics.len(),
        2,
        "an operator sees the word they created and have not used"
    );

    // A retirement is not a delete, and the guide filed under the word keeps
    // reading.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::PUT,
            path: &format!(
                "/v1/admin/guides/_taxonomy/topics/{}",
                selling.id.to_hyphenated(),
            ),
            body: Some(body_of(&serde_json::json!({
                "name": "Selling",
                "retired": true,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(
        json::<GuideTaxonView>(&body).retired,
        "the word is withdrawn"
    );
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: &format!("/v1/guides?topic={}", selling.id.to_hyphenated()),
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the retired filter still serves");
    assert_eq!(
        json::<GuidesView>(&body).total,
        1,
        "a retired word a published guide names is still a filter that answers"
    );

    // A word this server does not have is a refusal rather than a filter that
    // is quietly dropped, so a console reading a moved vocabulary is told.
    for query in [
        "topic=not-an-identifier".to_owned(),
        format!("tags={}", uuid::Uuid::nil()),
        format!("tags={}", selling.id.to_hyphenated()),
    ] {
        let (status, _kind, refused) = call(
            state(pool.clone(), None),
            Some(&TOKEN_SELLER),
            Call {
                method: Method::GET,
                path: &format!("/v1/guides?{query}"),
                body: None,
            },
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "?{query} is refused rather than ignored: {}",
            String::from_utf8_lossy(&refused)
        );
    }

    // And the same for a write, including a topic identifier sent as a tag.
    let (status, _kind, refused) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::PUT,
            path: "/v1/admin/guides/fees",
            body: Some(body_of(&serde_json::json!({
                "title": "Fees",
                "body": "What a marketplace keeps.",
                "topic_id": selling.id,
                "tag_ids": [unused.id],
                "expected_id": created.id,
                "expected_revision": 2,
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a topic cannot be filed as a tag: {}",
        String::from_utf8_lossy(&refused)
    );

    // A withdrawal is explicit, and it takes the page and the vocabulary with
    // it.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/fees/unpublish",
            body: Some(body_of(&serde_json::json!({
                "expected_id": created.id,
                "expected_revision": 2,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let withdrawn: GuideView = json(&body);
    assert_eq!(withdrawn.status, "draft", "the guide is a draft again");
    assert!(
        withdrawn.published.is_none(),
        "with nothing published beside it"
    );
    assert_eq!(
        withdrawn.body, "What a marketplace keeps.",
        "and the operator keeps every word they had"
    );
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/_taxonomy",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the reader's taxonomy serves");
    let after: GuideTaxonomyView = json(&body);
    assert!(
        after.topics.is_empty() && after.tags.is_empty(),
        "and the reader's vocabulary empties with the page"
    );
}

/// `taxonomy` and `preview` are ordinary guide slugs, and stay ordinary.
///
/// The routes that would otherwise have shadowed them are spelled
/// `_taxonomy` and `_preview`, and the leading underscore is outside the
/// alphabet `guide_slug_shape` admits — so a guide can never be addressed at
/// one of them, and no word needs reserving in the slug validator. This is
/// the test that fails if either route loses its underscore: a slug that was
/// valid and published would start answering the taxonomy.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_guide_may_be_called_taxonomy_or_preview(pool: PgPool) {
    provision(&pool).await;

    for slug in ["taxonomy", "preview"] {
        let (status, _kind, body) = call(
            state(pool.clone(), None),
            Some(&TOKEN_OPERATOR),
            Call {
                method: Method::POST,
                path: "/v1/admin/guides",
                body: Some(body_of(&serde_json::json!({
                    "slug": slug,
                    "title": "Our words",
                    "body": "How we file guides.",
                }))),
            },
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "a guide may live at /{slug}: {}",
            String::from_utf8_lossy(&body)
        );
        let written: GuideView = json(&body);
        assert_eq!(written.slug, slug, "at the address it asked for");

        let (status, _kind, body) = call(
            state(pool.clone(), None),
            Some(&TOKEN_OPERATOR),
            Call {
                method: Method::POST,
                path: &format!("/v1/admin/guides/{slug}/publish"),
                body: Some(body_of(&serde_json::json!({
                    "expected_id": written.id,
                    "expected_revision": 1,
                }))),
            },
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "and is published by its own address: {}",
            String::from_utf8_lossy(&body)
        );

        let (status, _kind, body) = call(
            state(pool.clone(), None),
            Some(&TOKEN_SELLER),
            Call {
                method: Method::GET,
                path: &format!("/v1/guides/{slug}"),
                body: None,
            },
        )
        .await;
        assert_eq!(status, StatusCode::OK, "and read at it by a seller");
        let page: PublishedGuideView = json(&body);
        assert_eq!(page.slug, slug, "as the guide rather than as a vocabulary");
        assert!(
            page.html.contains("How we file guides."),
            "with its own prose: {}",
            page.html
        );
    }

    // The underscored addresses are the vocabulary and the preview, and no
    // guide can be reached at one: the validator refuses the slug outright,
    // which is why nothing here reserves a word.
    let (status, _kind, refused) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides",
            body: Some(body_of(&serde_json::json!({
                "slug": "_taxonomy",
                "title": "Not a slug",
                "body": "",
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an underscore is not in a guide slug's alphabet, so the route's own \
         address cannot be claimed: {}",
        String::from_utf8_lossy(&refused)
    );

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/_taxonomy",
            body: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "and the reader's vocabulary still answers at its own address: {}",
        String::from_utf8_lossy(&body)
    );
    let vocabulary: GuideTaxonomyView = json(&body);
    assert!(
        vocabulary.topics.is_empty() && vocabulary.tags.is_empty(),
        "with the taxonomy, not with a guide"
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
    assert_eq!(kind.as_deref(), Some("image/jpeg"), "as a picture");
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
                    "slug": "mine", "title": "Mine", "body": "hello",
                })
                .to_string(),
            ),
        ),
        (
            "/v1/admin/guides/_preview",
            Body::from(serde_json::json!({ "body": "hello" }).to_string()),
        ),
        (
            "/v1/admin/guides/_taxonomy/tags",
            Body::from(serde_json::json!({ "slug": "mine", "name": "Mine" }).to_string()),
        ),
        (
            "/v1/admin/guides/mine/publish",
            Body::from(
                serde_json::json!({
                    "expected_id": "0a0a0a0a-0a0a-0a0a-0a0a-0a0a0a0a0a0a",
                    "expected_revision": 1,
                })
                .to_string(),
            ),
        ),
        (
            "/v1/admin/guides/mine/unpublish",
            Body::from(
                serde_json::json!({
                    "expected_id": "0a0a0a0a-0a0a-0a0a-0a0a-0a0a0a0a0a0a",
                    "expected_revision": 1,
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
        assert_eq!(status, StatusCode::UNAUTHORIZED, "as is nobody at all");
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
    let words: i64 = sqlx::query_scalar("SELECT count(*) FROM guide_taxon")
        .fetch_one(&pool)
        .await
        .unwrap_or(-1);
    assert_eq!(words, 0, "and no vocabulary either");
}

/// The wire's half of the delete-and-recreate regression.
///
/// An operator deletes a guide; somebody writes a new guide at the freed slug;
/// and the first operator's editor is still open on a tab that knows only the
/// address and revision 1 — which is exactly where the new guide is. Every
/// write that tab can make names numbers that would have matched a fence made
/// of the slug and the revision, and every one of them has to be refused with
/// the guide that is actually there, so the console can say "what you were
/// editing is gone" rather than offering to overwrite a stranger's document.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_write_aimed_at_a_deleted_guide_cannot_reach_the_guide_that_replaced_it(pool: PgPool) {
    provision(&pool).await;

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides",
            body: Some(body_of(&serde_json::json!({
                "slug": "fees",
                "title": "Fees",
                "body": "The first guide's paragraph.",
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let first: GuideView = json(&body);

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::DELETE,
            path: "/v1/admin/guides/fees",
            body: Some(body_of(&serde_json::json!({
                "expected_id": first.id,
                "expected_revision": 1,
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the operator deletes the guide they read: {}",
        String::from_utf8_lossy(&body)
    );

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides",
            body: Some(body_of(&serde_json::json!({
                "slug": "fees",
                "title": "Fees, rewritten",
                "body": "The second guide's paragraph.",
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "and the freed slug takes a new guide: {}",
        String::from_utf8_lossy(&body)
    );
    let second: GuideView = json(&body);
    assert_ne!(
        second.id, first.id,
        "which is a different guide, and says so in the one field that can"
    );
    assert_eq!(
        second.revision, 1,
        "starting where every guide starts, which is where the deleted guide's \
         editor still thinks it is"
    );

    // The four writes the stale tab can make, each naming the deleted guide
    // and the revision the replacement happens to be at.
    for (method, path, body) in [
        (
            Method::PUT,
            "/v1/admin/guides/fees",
            serde_json::json!({
                "title": "Fees",
                "body": "The first guide's paragraph, still being typed.",
                "expected_id": first.id,
                "expected_revision": 1,
            }),
        ),
        (
            Method::POST,
            "/v1/admin/guides/fees/publish",
            serde_json::json!({ "expected_id": first.id, "expected_revision": 1 }),
        ),
        (
            Method::POST,
            "/v1/admin/guides/fees/unpublish",
            serde_json::json!({ "expected_id": first.id, "expected_revision": 1 }),
        ),
        (
            Method::DELETE,
            "/v1/admin/guides/fees",
            serde_json::json!({ "expected_id": first.id, "expected_revision": 1 }),
        ),
    ] {
        let (status, _kind, refused) = call(
            state(pool.clone(), None),
            Some(&TOKEN_OPERATOR),
            Call {
                method,
                path,
                body: Some(body_of(&body)),
            },
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a write naming the deleted guide is refused on {path}: {}",
            String::from_utf8_lossy(&refused)
        );
        let refusal = value(&refused);
        let detail = &refusal["errors"][0]["detail"];
        assert_eq!(
            detail["expected_id"],
            serde_json::json!(second.id.to_hyphenated()),
            "and names the guide that is stored, which is not the one the \
             caller named: {}",
            String::from_utf8_lossy(&refused)
        );
        assert_eq!(
            detail["expected_revision"],
            serde_json::json!(1),
            "at the revision it is stored at, so the console has both halves \
             of what a write would have to name: {}",
            String::from_utf8_lossy(&refused)
        );
    }

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::GET,
            path: "/v1/admin/guides/fees",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the operator's read serves");
    let standing: GuideView = json(&body);
    assert_eq!(
        standing.id, second.id,
        "the guide at that address is still the replacement"
    );
    assert_eq!(
        standing.revision, 1,
        "which four refused writes left where its author put it"
    );
    assert_eq!(
        standing.body, "The second guide's paragraph.",
        "with its author's prose, word for word"
    );
    assert_eq!(standing.title, "Fees, rewritten", "and its author's title");
    assert!(
        standing.published.is_none(),
        "and the refused publish published nothing"
    );

    // The operator's listing carries the identifier too, which is what lets a
    // row be deleted without a second read: the row names the guide it is,
    // and a guide replaced since the listing was drawn cannot be mistaken for
    // it on a revision that happens to match.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::GET,
            path: "/v1/admin/guides",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the operator's listing serves");
    let listed: GuidesView = json(&body);
    let row = listed
        .guides
        .into_iter()
        .find(|head| head.slug == "fees")
        .expect("the replacement is listed");
    assert_eq!(
        row.id, second.id,
        "the listed row is the guide that is there, not the one it replaced"
    );
    assert_eq!(
        row.revision, standing.revision,
        "at the revision the detail read reports, so a write made from the \
         listing names exactly what a write made from the editor would"
    );

    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_SELLER),
        Call {
            method: Method::GET,
            path: "/v1/guides/fees",
            body: None,
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "so no seller is reading a draft nobody approved: {}",
        String::from_utf8_lossy(&body)
    );

    // The replacement's own author is not blocked by any of this.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::POST,
            path: "/v1/admin/guides/fees/publish",
            body: Some(body_of(&serde_json::json!({
                "expected_id": second.id,
                "expected_revision": 1,
            }))),
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let published: GuideView = json(&body);
    assert_eq!(
        published.id, second.id,
        "the guide that publishes is the one that was named"
    );
    assert_eq!(published.revision, 2, "and its own write moves it on");
    assert_eq!(
        published
            .published
            .map(|snapshot| snapshot.body)
            .unwrap_or_default(),
        "The second guide's paragraph.",
        "with its author's prose in the snapshot sellers read"
    );

    // And a delete made straight from a listing row lands, because the row
    // carried both halves of what the write has to name.
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::GET,
            path: "/v1/admin/guides",
            body: None,
        },
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the operator's listing serves");
    let row = json::<GuidesView>(&body)
        .guides
        .into_iter()
        .find(|head| head.slug == "fees")
        .expect("the guide is listed");
    let (status, _kind, body) = call(
        state(pool.clone(), None),
        Some(&TOKEN_OPERATOR),
        Call {
            method: Method::DELETE,
            path: "/v1/admin/guides/fees",
            body: Some(body_of(&serde_json::json!({
                "expected_id": row.id,
                "expected_revision": row.revision,
            }))),
        },
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a delete naming the listed row's own guide and revision lands: {}",
        String::from_utf8_lossy(&body)
    );
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
