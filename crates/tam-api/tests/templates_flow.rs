//! The Template Manager's second tab over the wire: what a seller may save,
//! what the route refuses, and what the tenant fence keeps them from reaching.
//!
//! The severity is in the fence and in the bounds. Two tenants each save a
//! template, and neither may read, rename or remove the other's through the
//! pin the API path runs under — without that, every assertion about who sees
//! what would hold trivially. The bounds are asserted as refusals rather than
//! as accepted values, so a validation that was deleted fails here rather than
//! passing silently, and the draft bound is asserted as a value the create
//! form itself would refuse, which is the property the whole tab rests on: a
//! template can never prefill the New resource form with something that form
//! will not take.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resource_templates::{
    ResourceTemplateView, ResourceTemplatesView, DRAFT_MAX_BYTES, NAME_MAX_CHARS,
};
use tam_api::{router, APIError, APIErrorKind, AppState, Config, SESSION_COOKIE};
use tam_storage::{SessionRepo, SessionToken, TEMPLATES_PER_ORG_MAX};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const MADE: Timestamp = Timestamp(5_000);

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || MADE,
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
        (ORG_A, USER_A, "a@example.test", TOKEN_A),
        (ORG_B, USER_B, "b@example.test", TOKEN_B),
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
}

struct Answer {
    status: StatusCode,
    body: Vec<u8>,
}

impl Answer {
    #[expect(
        clippy::expect_used,
        reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
    )]
    fn json<T: serde::de::DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).expect("the answer body parses")
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: &PgPool,
    method: Method,
    path: &str,
    token: &SessionToken,
    body: Option<serde_json::Value>,
) -> Answer {
    let request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", token.to_hex()),
    );
    let request = match body {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string()))
            .expect("the request builds"),
        None => request.body(Body::empty()).expect("the request builds"),
    };
    let response = router(state(pool.clone()))
        .oneshot(request)
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

fn draft() -> serde_json::Value {
    serde_json::json!({
        "name": "Phonics pack",
        "description": "Differentiated three ways.\n\n- Easy\n- Medium",
        "grades": ["1st-grade"],
        "free": true,
    })
}

/// Every refusal on this surface is the caller's error carrying the kind the
/// client branches on, never a fault.
fn refusal(answer: &Answer) -> Vec<String> {
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a bound this route applies is a refusal a seller can act on"
    );
    let error: APIError = answer.json();
    for entry in &error.errors {
        assert_eq!(
            entry.kind,
            Some(APIErrorKind::Validation),
            "and it carries the kind the client branches on"
        );
    }
    error
        .errors
        .iter()
        .map(|entry| entry.message.clone())
        .collect()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_saves_lists_edits_and_removes_a_template(pool: PgPool) {
    provision(&pool).await;

    let saved = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "  Autumn unit \n", "draft": draft() })),
    )
    .await;
    assert_eq!(
        saved.status,
        StatusCode::CREATED,
        "a template that lands answers 201 with the row it made"
    );
    let written: ResourceTemplateView = saved.json();
    assert_eq!(
        (written.name.as_str(), &written.draft, written.created_at),
        ("Autumn unit", &draft(), MADE),
        "the answer is the row as stored: the name trimmed, the draft verbatim, and \
         the instant from the injected clock"
    );
    assert_eq!(
        written.updated_at, MADE,
        "a template nobody has edited was last changed when it was made"
    );

    let listed: ResourceTemplatesView = call(&pool, Method::GET, "/v1/templates", &TOKEN_A, None)
        .await
        .json();
    assert_eq!(
        listed
            .templates
            .iter()
            .map(|template| template.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Autumn unit"],
        "and the picker reads it back"
    );
    let head = listed
        .templates
        .first()
        .and_then(|head| serde_json::to_value(head).ok())
        .expect("the picker answered a row");
    assert!(
        head.get("draft").is_none(),
        "the picker carries no drafts: the row count alone does not bound a response \
         whose every row may hold sixty-four kilobytes, and this one holds {head}"
    );

    let path = format!("/v1/templates/{}", uuid::Uuid::from_bytes(written.id.0));
    let fetched: ResourceTemplateView =
        call(&pool, Method::GET, &path, &TOKEN_A, None).await.json();
    assert_eq!(
        (fetched.id, &fetched.draft),
        (written.id, &draft()),
        "and the fetch is what carries the draft the form is prefilled from"
    );
    let replacement = serde_json::json!({ "grades": ["2nd-grade"], "free": false });
    let edited = call(
        &pool,
        Method::PATCH,
        &path,
        &TOKEN_A,
        Some(serde_json::json!({ "draft": replacement })),
    )
    .await;
    assert_eq!(edited.status, StatusCode::OK, "an edit answers the new row");
    let changed: ResourceTemplateView = edited.json();
    assert_eq!(
        (changed.name.as_str(), &changed.draft),
        ("Autumn unit", &replacement),
        "the draft is replaced whole and the name it was saved under is kept"
    );

    let removed = call(&pool, Method::DELETE, &path, &TOKEN_A, None).await;
    assert_eq!(
        removed.status,
        StatusCode::NO_CONTENT,
        "a removal answers with no body"
    );
    let again = call(&pool, Method::DELETE, &path, &TOKEN_A, None).await;
    assert_eq!(
        again.status,
        StatusCode::NOT_FOUND,
        "and a second removal is told, rather than shown a success that did nothing"
    );
    let empty: ResourceTemplatesView = call(&pool, Method::GET, "/v1/templates", &TOKEN_A, None)
        .await
        .json();
    assert!(
        empty.templates.is_empty(),
        "the picker is empty once the last template is gone"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenant_neither_reads_nor_changes_another_s_template(pool: PgPool) {
    provision(&pool).await;

    let mine: ResourceTemplateView = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "Autumn unit", "draft": draft() })),
    )
    .await
    .json();
    let theirs = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_B,
        // The same name, deliberately: one organisation's naming is its own,
        // and a fence that made this a conflict would be leaking the other
        // tenant's rows through the refusal.
        Some(serde_json::json!({ "name": "Autumn unit", "draft": draft() })),
    )
    .await;
    assert_eq!(
        theirs.status,
        StatusCode::CREATED,
        "the second tenant may use a name the first already has"
    );

    let listed: ResourceTemplatesView = call(&pool, Method::GET, "/v1/templates", &TOKEN_B, None)
        .await
        .json();
    assert_eq!(
        listed.templates.len(),
        1,
        "each tenant's listing holds its own template and no other's"
    );
    assert_ne!(
        listed.templates.first().map(|template| template.id),
        Some(mine.id),
        "and the row it holds is not the first tenant's"
    );

    let path = format!("/v1/templates/{}", uuid::Uuid::from_bytes(mine.id.0));
    for (method, body) in [
        (Method::GET, None),
        (Method::PATCH, Some(serde_json::json!({ "name": "stolen" }))),
        (Method::DELETE, None),
    ] {
        let refused = call(&pool, method.clone(), &path, &TOKEN_B, body).await;
        assert_eq!(
            refused.status,
            StatusCode::NOT_FOUND,
            "another tenant's template answers as absent rather than as forbidden, \
             because the identifier is the one thing a caller could guess: {method}"
        );
    }
    let survived: ResourceTemplatesView = call(&pool, Method::GET, "/v1/templates", &TOKEN_A, None)
        .await
        .json();
    assert_eq!(
        survived
            .templates
            .iter()
            .map(|template| template.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Autumn unit"],
        "and the first tenant's template is untouched afterwards"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_route_refuses_a_name_no_column_could_hold(pool: PgPool) {
    provision(&pool).await;
    for (name, refused) in [
        ("   ", "a name that is only whitespace"),
        (
            "n".repeat(NAME_MAX_CHARS + 1).as_str(),
            "a name one character past the bound",
        ),
        (
            "Autumn\u{0}unit",
            "a zero byte no text column can hold, refused here rather than by Postgres",
        ),
        (
            "Autumn\nunit",
            "a newline, which a single-line picker entry has no use for",
        ),
    ] {
        let answer = call(
            &pool,
            Method::POST,
            "/v1/templates",
            &TOKEN_A,
            Some(serde_json::json!({ "name": name, "draft": draft() })),
        )
        .await;
        assert!(
            !refusal(&answer).is_empty(),
            "the route refuses this and says why: {refused}"
        );
    }
}

/// The property the tab rests on: a saved template can never prefill the New
/// resource form with a value that form will not take.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_route_refuses_a_draft_the_create_form_would_refuse(pool: PgPool) {
    provision(&pool).await;
    for (held, refused) in [
        (
            serde_json::json!([]),
            "a draft that is not a JSON object at all",
        ),
        (
            serde_json::json!({ "name": "t".repeat(400) }),
            "a title past the create form's own cap",
        ),
        (
            serde_json::json!({ "payload_hash": "not-a-digest" }),
            "an upload handle this server did not issue",
        ),
        (
            serde_json::json!({ "tax_code_id": 200 }),
            "a listbox id outside its vocabulary",
        ),
        (
            serde_json::json!({
                "grades": ["1st-grade", "2nd-grade", "3rd-grade", "4th-grade", "5th-grade"],
            }),
            "a picker over its measured cap, which the check endpoint cannot reach for \
             a draft with no title",
        ),
        (
            serde_json::json!({ "description": "a".repeat(DRAFT_MAX_BYTES) }),
            "a draft past the byte ceiling",
        ),
        (
            serde_json::json!({ "description": "before\u{0}after" }),
            "a zero byte, which no jsonb value may carry",
        ),
    ] {
        let answer = call(
            &pool,
            Method::POST,
            "/v1/templates",
            &TOKEN_A,
            Some(serde_json::json!({ "name": "Autumn unit", "draft": held })),
        )
        .await;
        assert!(
            !refusal(&answer).is_empty(),
            "the route refuses this and says why: {refused}"
        );
    }

    // M1 over the wire: the rule `POST /v1/products` refuses a submission by,
    // applied to the one half of it that is a value the seller typed.
    let under_floor = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({
            "name": "Paid unit",
            "draft": { "free": false, "price_minor_units": 1 },
        })),
    )
    .await;
    assert!(
        refusal(&under_floor)
            .iter()
            .any(|message| message == "Raise the price to at least $0.95."),
        "a template holding a price the create form refuses would prefill a form \
         that cannot submit"
    );

    // And the half that makes the refusals above mean something: a template is
    // a form nobody has finished, so nearly-empty is the ordinary case.
    let accepted = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "Blank start", "draft": serde_json::json!({}) })),
    )
    .await;
    assert_eq!(
        accepted.status,
        StatusCode::CREATED,
        "no title, no payload, no price and no copyright attestation are all ordinary \
         states of a template"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_route_refuses_a_second_template_of_one_name_and_an_edit_that_names_nothing(
    pool: PgPool,
) {
    provision(&pool).await;
    let first: ResourceTemplateView = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "Autumn Unit", "draft": draft() })),
    )
    .await
    .json();
    let second = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "autumn unit", "draft": draft() })),
    )
    .await;
    assert!(
        !refusal(&second).is_empty(),
        "a seller who types the same name in another casing means the template they have"
    );

    let path = format!("/v1/templates/{}", uuid::Uuid::from_bytes(first.id.0));
    let nothing = call(
        &pool,
        Method::PATCH,
        &path,
        &TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;
    assert!(
        !refusal(&nothing).is_empty(),
        "an edit that names neither part would still move updated_at, which would \
         misstate when the seller last touched the template"
    );

    let malformed = call(
        &pool,
        Method::DELETE,
        "/v1/templates/not-a-uuid",
        &TOKEN_A,
        None,
    )
    .await;
    assert!(
        !refusal(&malformed).is_empty(),
        "an identifier that is not a UUID is refused as the caller's error"
    );

    let absent = call(
        &pool,
        Method::PATCH,
        &format!("/v1/templates/{}", uuid::Uuid::from_bytes([0xEE; 16])),
        &TOKEN_A,
        Some(serde_json::json!({ "name": "renamed" })),
    )
    .await;
    assert_eq!(
        absent.status,
        StatusCode::NOT_FOUND,
        "and one naming no row of this organisation is not found"
    );
}

/// The boundary M2 lived on, driven through Postgres rather than through the
/// route's own arithmetic.
///
/// The route measures serde_json's compact rendering and the column measures
/// Postgres re-rendering the parsed value, which is longer for any object with
/// a key in it. A draft sitting exactly on the route's bound is therefore the
/// one witness that tells the two apart, and it has to survive the write and
/// read back byte for byte.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_draft_exactly_on_the_byte_ceiling_is_stored_and_reads_back(pool: PgPool) {
    provision(&pool).await;
    // `{"description":""}` is eighteen bytes of wrapper.
    let on_the_cap = serde_json::json!({ "description": "a".repeat(DRAFT_MAX_BYTES - 18) });
    assert_eq!(
        serde_json::to_string(&on_the_cap)
            .map(|rendered| rendered.len())
            .ok(),
        Some(DRAFT_MAX_BYTES),
        "the fixture renders to exactly the cap, or this test measures something else"
    );

    let saved = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "On the cap", "draft": on_the_cap })),
    )
    .await;
    assert_eq!(
        saved.status,
        StatusCode::CREATED,
        "a draft the route accepts must be one the column accepts; against a CHECK \
         set to the route's own number this answered 500"
    );
    let written: ResourceTemplateView = saved.json();
    let fetched: ResourceTemplateView = call(
        &pool,
        Method::GET,
        &format!("/v1/templates/{}", uuid::Uuid::from_bytes(written.id.0)),
        &TOKEN_A,
        None,
    )
    .await
    .json();
    assert_eq!(
        fetched.draft, on_the_cap,
        "and it reads back exactly as it was sent"
    );

    let past = serde_json::json!({ "description": "a".repeat(DRAFT_MAX_BYTES - 17) });
    let refused_past = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "One past", "draft": past })),
    )
    .await;
    assert!(
        !refusal(&refused_past).is_empty(),
        "one byte past the cap is the caller's error rather than a fault"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_shelf_fills_at_the_ceiling_the_unpaged_listing_rests_on(pool: PgPool) {
    provision(&pool).await;
    // Seeded under the tenant pin rather than through the route: the pool is
    // the application role's, forced row-level security applies its WITH CHECK
    // to this insert too, and an unpinned one is refused rather than written.
    let mut tx = pool.begin().await.expect("the seeding transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO resource_template \
         (org_id, id, name, draft, created_at, updated_at) \
         SELECT $1, gen_random_uuid(), 'seeded ' || n, '{}'::jsonb, now(), now() \
           FROM generate_series(1, $2) AS n",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(i32::try_from(TEMPLATES_PER_ORG_MAX).unwrap_or(i32::MAX))
    .execute(&mut *tx)
    .await
    .expect("the shelf fills");
    tx.commit().await.expect("the seeding commits");

    let answer = call(
        &pool,
        Method::POST,
        "/v1/templates",
        &TOKEN_A,
        Some(serde_json::json!({ "name": "One too many", "draft": draft() })),
    )
    .await;
    let messages = refusal(&answer);
    assert!(
        messages
            .iter()
            .any(|message| message.contains(&TEMPLATES_PER_ORG_MAX.to_string())),
        "the refusal states the ceiling it applied, which is what bounds an answer \
         that has no cursor: {messages:?}"
    );
}
