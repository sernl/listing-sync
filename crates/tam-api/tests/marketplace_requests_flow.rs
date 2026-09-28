//! The request-a-marketplace surface over the wire: what a seller may write,
//! what the tenant fence keeps them from seeing, and what an operator reads
//! across every tenant.
//!
//! The severity here is in the fence and in the bounds. Two tenants each write
//! one request, and neither may read the other's through the pin the API path
//! runs under -- if that failed, the operator listing below would prove
//! nothing, because every tenant would already be reading every row. The
//! bounds are asserted as refusals rather than as accepted values, so a
//! validation that was deleted fails here rather than passing silently.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::marketplace_requests::{MarketplaceRequestView, MarketplaceRequestsPage};
use tam_api::{router, APIError, APIErrorCode, APIErrorKind, AppState, Config, SESSION_COOKIE};
use tam_storage::{
    OperatorRepo, SessionRepo, SessionToken, MARKETPLACE_REQUESTED_TOPIC, REQUESTS_PER_ORG_MAX,
    REQUESTS_PER_USER_PER_DAY, REQUEST_RATE_WINDOW_MS,
};
use tam_types::{MarketplaceRequestedNotice, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

fn state(pool: PgPool, backoffice: Option<PgPool>) -> AppState {
    state_at(pool, backoffice, || NOW)
}

/// The same state on another clock, which is how the rate window's far edge
/// is reached without waiting a day.
fn state_at(pool: PgPool, backoffice: Option<PgPool>, wall: fn() -> Timestamp) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall,
        auth: None,
        backoffice,
        blobs: None,
    }
}

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

/// Two tenants, one seller session each, and the operator marking on the first
/// of them. The marking is on a seller of ORG_A deliberately: the operator
/// listing must answer with ORG_B's row too, and an operator who could only
/// see their own organisation would still pass a one-tenant test.
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
    OperatorRepo::new(pool.clone())
        .grant(USER_A, "the test fixture", NOW)
        .await
        .expect("the operator marking lands");
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
    state: AppState,
    method: Method,
    path: &str,
    token: Option<&SessionToken>,
    body: Option<serde_json::Value>,
) -> Answer {
    let request = Request::builder().method(method).uri(path);
    let request = match token {
        Some(token) => request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        ),
        None => request,
    };
    let request = match body {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string()))
            .expect("the request builds"),
        None => request.body(Body::empty()).expect("the request builds"),
    };
    let response = router(state)
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

fn request_body(name: &str, url: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "url": url, "reason": reason })
}

/// What one tenant's own pin sees, which is the fence the operator listing is
/// measured against.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn names_under_pin(pool: &PgPool, org: OrgId) -> Vec<String> {
    let mut tx = pool.begin().await.expect("the read transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM marketplace_request ORDER BY name")
            .fetch_all(&mut *tx)
            .await
            .expect("the pinned read runs");
    tx.commit().await.expect("the read transaction commits");
    names
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn each_tenant_writes_its_own_request_and_the_operator_reads_both(pool: PgPool) {
    provision(&pool).await;

    let answer = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(request_body(
            "  Amped Up Learning  ",
            " https://ampeduplearning.com/shop ",
            "  Science units, mostly middle school.  ",
        )),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "a request that lands answers 201 with the row it made"
    );
    let written: MarketplaceRequestView = answer.json();
    assert_eq!(
        (
            written.org,
            written.requested_by,
            written.name.as_str(),
            written.url.as_str(),
            written.reason.as_str(),
            written.created_at
        ),
        (
            ORG_A,
            USER_A,
            "Amped Up Learning",
            "https://ampeduplearning.com/shop",
            "Science units, mostly middle school.",
            NOW
        ),
        "the answer is the row as stored: trimmed, attributed to the session's \
         own organisation and user, and stamped from the injected clock"
    );

    let created = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_B),
        Some(request_body(
            "Made By Teachers",
            "https://madebyteachers.com",
            "My phonics packs sell there.",
        )),
    )
    .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "the second tenant's request lands the same way"
    );

    assert_eq!(
        (
            names_under_pin(&pool, ORG_A).await,
            names_under_pin(&pool, ORG_B).await
        ),
        (
            vec!["Amped Up Learning".to_owned()],
            vec!["Made By Teachers".to_owned()]
        ),
        "each tenant sees its own request and nothing of the other's"
    );

    // One older row, so the ordering is asserted against an instant that
    // differs. The two written above share the injected clock, and ordering
    // them against each other would only be asserting how two random
    // identifiers happen to compare, which the ordering does not promise.
    seed_older_request(&pool, ORG_B, USER_B).await;

    let backoffice = backoffice_pool(&pool).await;
    let across: MarketplaceRequestsPage = call(
        state(pool.clone(), Some(backoffice)),
        Method::GET,
        "/v1/admin/marketplace-requests",
        Some(&TOKEN_A),
        None,
    )
    .await
    .json();
    let mut newest: Vec<(OrgId, UserId, Option<&str>, &str)> = across
        .requests
        .iter()
        .take(2)
        .map(|request| {
            (
                request.org,
                request.requested_by,
                request.requested_by_email.as_deref(),
                request.name.as_str(),
            )
        })
        .collect();
    newest.sort_unstable_by(|left, right| left.3.cmp(right.3));
    assert_eq!(
        (
            across.requests.len(),
            newest,
            across.requests.last().map(|request| request.name.as_str()),
            across.next_cursor.is_some()
        ),
        (
            3,
            vec![
                (ORG_A, USER_A, Some("a@example.test"), "Amped Up Learning"),
                (ORG_B, USER_B, Some("b@example.test"), "Made By Teachers")
            ],
            Some("An Older Ask"),
            false
        ),
        "the operator reads every tenant's requests with the person to answer \
         at, the oldest is last, and a page that did not fill offers no next one"
    );
}

/// Every outbox message one tenant's pin sees, oldest first.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn outbox_under_pin(pool: &PgPool, org: OrgId) -> Vec<(String, serde_json::Value)> {
    let mut tx = pool.begin().await.expect("the read transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT topic, payload FROM outbox_message ORDER BY created_at, id")
            .fetch_all(&mut *tx)
            .await
            .expect("the pinned read runs");
    tx.commit().await.expect("the read transaction commits");
    rows
}

/// The operators hear about a request in the same transaction that stores it:
/// one message on the request topic carrying exactly who asked, from where,
/// for what and why — and a refused request, which stores nothing, tells
/// nobody.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_stored_request_enqueues_one_operator_mail_and_a_refused_one_none(pool: PgPool) {
    provision(&pool).await;
    let subject = Uuid([0x5A; 16]);
    sqlx::query("UPDATE app_user SET auth_subject = $1 WHERE id = $2")
        .bind(uuid::Uuid::from_bytes(subject.0))
        .bind(uuid::Uuid::from_bytes(USER_A.0 .0))
        .execute(&pool)
        .await
        .expect("the requester has an identity-service subject");

    let body = request_body(
        "  Amped Up Learning  ",
        "https://ampeduplearning.com/shop",
        "  Science units, mostly middle school.  ",
    );
    let created = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(body.clone()),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);

    let expected = serde_json::to_value(MarketplaceRequestedNotice {
        requester_subject: Some(subject),
        org: ORG_A,
        org_name: "org-a".to_owned(),
        marketplace_name: "Amped Up Learning".to_owned(),
        note: "Science units, mostly middle school.".to_owned(),
    })
    .expect("a notice serialises");
    assert_eq!(
        (
            names_under_pin(&pool, ORG_A).await,
            outbox_under_pin(&pool, ORG_A).await
        ),
        (
            vec!["Amped Up Learning".to_owned()],
            vec![(MARKETPLACE_REQUESTED_TOPIC.to_owned(), expected.clone())]
        ),
        "the row is stored beside exactly one message, whose payload is the \
         trimmed request and nothing else: no address, no web address"
    );

    let again = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(body),
    )
    .await;
    assert_eq!(again.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        outbox_under_pin(&pool, ORG_A).await,
        vec![(MARKETPLACE_REQUESTED_TOPIC.to_owned(), expected)],
        "a request refused as already asked tells the operators nothing new"
    );
}

/// The page an operator walks, and the reason it is walkable: one tenant
/// writing steadily must not be able to hold the whole answer.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_operator_page_is_walked_by_cursor_and_refuses_a_cursor_it_did_not_issue(pool: PgPool) {
    provision(&pool).await;
    for index in 0..3 {
        let created = call(
            state(pool.clone(), None),
            Method::POST,
            "/v1/marketplace-requests",
            Some(&TOKEN_A),
            Some(request_body(
                &format!("Shop {index}"),
                &format!("https://shop-{index}.test"),
                "why",
            )),
        )
        .await;
        assert_eq!(created.status, StatusCode::CREATED, "request {index} lands");
    }

    let first: MarketplaceRequestsPage = call(
        state(pool.clone(), Some(backoffice_pool(&pool).await)),
        Method::GET,
        "/v1/admin/marketplace-requests?limit=2",
        Some(&TOKEN_A),
        None,
    )
    .await
    .json();
    let cursor = first
        .next_cursor
        .clone()
        .expect("a page that filled offers the next one");
    assert_eq!(
        first.requests.len(),
        2,
        "the page is the size the operator asked for"
    );

    let second: MarketplaceRequestsPage = call(
        state(pool.clone(), Some(backoffice_pool(&pool).await)),
        Method::GET,
        &format!("/v1/admin/marketplace-requests?limit=2&cursor={cursor}"),
        Some(&TOKEN_A),
        None,
    )
    .await
    .json();
    assert_eq!(
        (second.requests.len(), second.next_cursor.is_some()),
        (1, false),
        "the walk reaches the last row and stops rather than looping"
    );
    let walked: Vec<&str> = first
        .requests
        .iter()
        .chain(second.requests.iter())
        .map(|request| request.name.as_str())
        .collect();
    assert_eq!(
        walked.len(),
        3,
        "every row is reached exactly once across the walk: {walked:?}"
    );

    let refused = call(
        state(pool.clone(), Some(backoffice_pool(&pool).await)),
        Method::GET,
        "/v1/admin/marketplace-requests?cursor=not-a-cursor",
        Some(&TOKEN_A),
        None,
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a cursor this server did not issue is refused rather than guessed at"
    );
}

/// The two bounds that stop one tenant filling the operator's only view.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenant_cannot_fill_the_table(pool: PgPool) {
    provision(&pool).await;

    let repeated = |suffix: &str| request_body("Amped Up Learning", suffix, "Science units.");
    let first = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(repeated("https://ampeduplearning.com/shop")),
    )
    .await;
    assert_eq!(first.status, StatusCode::CREATED, "the first ask lands");

    let again = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(repeated("HTTPS://AmpedUpLearning.com/shop")),
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the same address in another case is the same ask, refused as the \
         seller's to correct rather than written twice"
    );
    assert_eq!(
        again.json::<APIError>().errors[0].kind,
        Some(APIErrorKind::Validation),
        "the refusal carries the kind the form branches on"
    );

    // Up to the cap, each naming an address of its own and each asked more
    // than a day ago, so the per-person rate is not what answers; then one
    // more through the API to reach the cap, and one past it.
    seed_requests_long_ago(&pool, ORG_A, USER_A, REQUESTS_PER_ORG_MAX - 2).await;
    let last_inside = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(request_body(
            "Last inside",
            "https://last-inside.test",
            "why",
        )),
    )
    .await;
    assert_eq!(
        last_inside.status,
        StatusCode::CREATED,
        "the request that reaches the cap is inside it"
    );
    let past = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(request_body(
            "One too many",
            "https://one-too-many.test",
            "why",
        )),
    )
    .await;
    assert_eq!(
        past.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the request past the cap is refused"
    );
    assert_eq!(
        names_under_pin(&pool, ORG_A).await.len(),
        usize::try_from(REQUESTS_PER_ORG_MAX).unwrap_or(usize::MAX),
        "exactly the cap stands, and nothing past it was written"
    );

    // The other tenant is unaffected: the cap is per organisation, so a
    // flooded neighbour must not be able to refuse anyone else's first ask.
    let neighbour = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_B),
        Some(request_body(
            "Made By Teachers",
            "https://madebyteachers.com",
            "why",
        )),
    )
    .await;
    assert_eq!(
        neighbour.status,
        StatusCode::CREATED,
        "another tenant's first request lands however full its neighbour is"
    );
}

/// `count` requests by `user`, each naming its own address and each stamped
/// in 1960, so they count against the organisation's cap and not against the
/// person's daily rate.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_requests_long_ago(pool: &PgPool, org: OrgId, user: UserId, count: i64) {
    let mut tx = pool.begin().await.expect("the fixture transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO marketplace_request \
         (org_id, id, requested_by, name, url, reason, created_at) \
         SELECT $1, gen_random_uuid(), $2, 'Shop ' || n, 'https://shop-' || n || '.test', \
             'why', timestamptz '1960-01-01T00:00:00Z' + make_interval(days => n) \
           FROM generate_series(1, $3::int) AS n",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes(user.0 .0))
    .bind(i32::try_from(count).expect("a small count"))
    .execute(&mut *tx)
    .await
    .expect("the older requests insert");
    tx.commit().await.expect("the fixture transaction commits");
}

/// One person sends at most three requests in any rolling day, because each
/// one mails the Teachouse team. The fourth is refused with the sentence the
/// form shows, stores nothing and mails nobody; a colleague is unaffected;
/// and the first request leaves the window exactly a day after it was sent.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_person_sends_at_most_three_requests_a_day(pool: PgPool) {
    provision(&pool).await;
    assert_eq!(
        REQUESTS_PER_USER_PER_DAY, 3,
        "the sentence below says three"
    );
    for index in 0..REQUESTS_PER_USER_PER_DAY {
        let landed = call(
            state(pool.clone(), None),
            Method::POST,
            "/v1/marketplace-requests",
            Some(&TOKEN_A),
            Some(request_body(
                &format!("Shop {index}"),
                &format!("https://shop-{index}.test"),
                "why",
            )),
        )
        .await;
        assert_eq!(landed.status, StatusCode::CREATED, "request {index} lands");
    }

    let fourth = request_body("Fourth", "https://fourth.test", "why");
    let refused = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(fourth.clone()),
    )
    .await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        refused.json::<APIError>().errors[0].message,
        "You’ve sent 3 requests today. Try again tomorrow.",
        "the refusal is a sentence the form renders as it stands"
    );
    assert_eq!(
        (
            names_under_pin(&pool, ORG_A).await.len(),
            outbox_under_pin(&pool, ORG_A).await.len()
        ),
        (3, 3),
        "the refused request stores nothing and mails nobody"
    );

    let colleague = call(
        state(pool.clone(), None),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_B),
        Some(request_body("Elsewhere", "https://elsewhere.test", "why")),
    )
    .await;
    assert_eq!(
        colleague.status,
        StatusCode::CREATED,
        "the rate is one person's, so somebody else's first ask lands"
    );

    let just_inside = call(
        state_at(pool.clone(), None, || {
            Timestamp(NOW.0 + REQUEST_RATE_WINDOW_MS - 1)
        }),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(fourth.clone()),
    )
    .await;
    assert_eq!(
        just_inside.status,
        StatusCode::TOO_MANY_REQUESTS,
        "a millisecond short of a day, the first three still count"
    );
    let a_day_on = call(
        state_at(pool.clone(), None, || {
            Timestamp(NOW.0 + REQUEST_RATE_WINDOW_MS)
        }),
        Method::POST,
        "/v1/marketplace-requests",
        Some(&TOKEN_A),
        Some(fourth),
    )
    .await;
    assert_eq!(
        a_day_on.status,
        StatusCode::CREATED,
        "a day after the first three, the window has rolled past them"
    );
}

/// One request stamped before the injected clock, written under the tenant's
/// own pin so it is a row a seller could have left.
///
/// The epoch itself, because the injected clock is five seconds past it: a
/// date that reads as older to a person would be five decades newer than the
/// two rows this is compared against.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_older_request(pool: &PgPool, org: OrgId, user: UserId) {
    let mut tx = pool.begin().await.expect("the fixture transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO marketplace_request \
         (org_id, id, requested_by, name, url, reason, created_at) \
         VALUES ($1, gen_random_uuid(), $2, 'An Older Ask', 'https://older.test', \
             'asked for first', timestamptz '1970-01-01T00:00:00Z')",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(uuid::Uuid::from_bytes(user.0 .0))
    .execute(&mut *tx)
    .await
    .expect("the older request inserts");
    tx.commit().await.expect("the fixture transaction commits");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_operator_listing_is_refused_the_way_every_other_one_is(pool: PgPool) {
    provision(&pool).await;
    let refused = call(
        state(pool.clone(), Some(backoffice_pool(&pool).await)),
        Method::GET,
        "/v1/admin/marketplace-requests",
        Some(&TOKEN_B),
        None,
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNAUTHORIZED,
        "a seller who is not an operator is refused"
    );
    let error: APIError = refused.json();
    assert_eq!(
        (error.errors[0].code, error.errors[0].kind),
        (
            Some(APIErrorCode::SessionRequired),
            Some(APIErrorKind::Unauthenticated)
        ),
        "in exactly the words an anonymous caller gets, so the route is no \
         oracle for who is an operator"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_request_outside_its_bounds_is_refused_and_nothing_is_written(pool: PgPool) {
    provision(&pool).await;

    let over_long_name = "\u{e9}".repeat(121);
    let over_long_reason = "a".repeat(2_001);
    let over_long_url = format!("https://example.test/{}", "a".repeat(2_040));
    for (what, body) in [
        (
            "a blank name",
            request_body("  ", "https://example.test", "why"),
        ),
        (
            "a name over its bound",
            request_body(&over_long_name, "https://example.test", "why"),
        ),
        (
            "an address that is not http",
            request_body("Somewhere", "ftp://example.test", "why"),
        ),
        (
            "an address naming no host",
            request_body("Somewhere", "https://", "why"),
        ),
        (
            "an address over its bound",
            request_body("Somewhere", &over_long_url, "why"),
        ),
        (
            "a blank description",
            request_body("Somewhere", "https://example.test", " \t "),
        ),
        (
            "a description over its bound",
            request_body("Somewhere", "https://example.test", &over_long_reason),
        ),
        // A zero byte passes trimming and every length bound, and no Postgres
        // `text` column can hold one, so without a check at the boundary each
        // of these is a fault rather than the refusal it is.
        (
            "a name carrying a zero byte",
            request_body("a\u{0}b", "https://example.test", "why"),
        ),
        (
            "an address carrying a zero byte",
            request_body("Somewhere", "https://exam\u{0}ple.test", "why"),
        ),
        (
            "a description carrying a zero byte",
            request_body("Somewhere", "https://example.test", "wh\u{0}y"),
        ),
        (
            "a name carrying a newline",
            request_body("two\nlines", "https://example.test", "why"),
        ),
    ] {
        let refused = call(
            state(pool.clone(), None),
            Method::POST,
            "/v1/marketplace-requests",
            Some(&TOKEN_A),
            Some(body),
        )
        .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{what} is the seller's to fix rather than a fault"
        );
        let error: APIError = refused.json();
        assert_eq!(
            error.errors[0].kind,
            Some(APIErrorKind::Validation),
            "{what} carries the kind the form branches on"
        );
    }

    assert!(
        names_under_pin(&pool, ORG_A).await.is_empty(),
        "a refused request writes no row"
    );
}
