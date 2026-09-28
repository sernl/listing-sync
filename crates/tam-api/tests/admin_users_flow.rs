//! The operator's user listing over the wire: one row per app user, carrying
//! the organisation they belong to, the plan it holds, and when the identity
//! plane last saw them.
//!
//! The severity is in the nullable join. `app_user.auth_subject` is nullable,
//! so a user provisioned before the identity plane existed has no subject to
//! join a sign-in to; a listing that dropped such a row, or that reported an
//! instant for it, would be wrong in the one case an operator consults this
//! page about. Both users are here for that reason: one linked and seen, one
//! linked to nothing.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::admin::{OperatorMarkingView, UsersView};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_limits::Plan;
use tam_storage::{EntitlementRepo, GrantedBy, NewGrant, OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

/// The identity-plane subject the operator's app user is linked to.
/// Deliberately not their `UserId`: the two planes number their users
/// separately, and a test that reused one id for both would assert they do
/// not.
const SUBJECT_OPERATOR: [u8; 16] = [0xA1; 16];

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

fn state(pool: PgPool, backoffice: Option<PgPool>) -> AppState {
    AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice,
        blobs: None,
    }
}

/// Two tenants: one holding a granted plan and a user the identity plane
/// knows, one holding nothing and a user it does not.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name, slug) in [
        (ORG_A, "Riverbend Resources", Some("riverbend")),
        (ORG_B, "org-b", None),
    ] {
        sqlx::query(
            "INSERT INTO organisation (id, name, slug, created_at) VALUES ($1, $2, $3, now())",
        )
        .bind(uuid::Uuid::from_bytes(org.0 .0))
        .bind(name)
        .bind(slug)
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
            .mint(&token, user, Timestamp(100_000), Timestamp(1_000))
            .await
            .expect("the session mints");
    }
    // One of the two is linked to an identity subject; the other is not, and
    // that asymmetry is the thing this file is about.
    sqlx::query("UPDATE app_user SET auth_subject = $2 WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(USER_OPERATOR.0 .0))
        .bind(uuid::Uuid::from_bytes(SUBJECT_OPERATOR))
        .execute(pool)
        .await
        .expect("the subject links");

    EntitlementRepo::new(pool.clone())
        .grant(
            ORG_A,
            &NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: Plan::Subscriber,
                rung: None,
                granted_by: GrantedBy::Operator,
                grantor_user: Some(USER_OPERATOR.0),
                reason: Some("the test fixture"),
                source_ref: None,
                granted_at: Timestamp(2_000),
                expires_at: None,
            },
        )
        .await
        .expect("the grant lands");

    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", NOW)
        .await
        .expect("the operator marking lands");
}

/// A stand-in for `auth.auth_event` as `db/auth/0002_audit_event.sql` leaves
/// it, which a throwaway test database does not carry: those files are applied
/// by `just auth-migrate` as tam_auth, and this crate's migrations are the only
/// DDL sqlx::test runs. Only the three columns the sign-in read names are
/// reproduced.
///
/// Two sign-ins for the one linked subject, so the read has to take the later
/// of them rather than whichever row it met first.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn identity_audit_standin(pool: &PgPool) {
    let subject = uuid::Uuid::from_bytes(SUBJECT_OPERATOR).to_string();
    for statement in [
        "CREATE SCHEMA auth".to_owned(),
        "CREATE TABLE auth.auth_event (event text NOT NULL, user_id uuid, \
         at timestamptz NOT NULL)"
            .to_owned(),
        format!(
            "INSERT INTO auth.auth_event (event, user_id, at) VALUES \
             ('user_signed_in', '{subject}', timestamptz '2026-03-01T09:00:00Z'), \
             ('user_signed_in', '{subject}', timestamptz '2026-03-04T18:30:00Z'), \
             ('user_signed_up', '{subject}', timestamptz '2026-02-01T09:00:00Z')"
        ),
    ] {
        sqlx::query(&statement)
            .execute(pool)
            .await
            .expect("the identity stand-in builds");
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn send(
    state: AppState,
    method: &str,
    uri: &str,
    token: Option<&SessionToken>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri);
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

async fn call(state: AppState, token: Option<&SessionToken>) -> (StatusCode, Vec<u8>) {
    send(state, "GET", "/v1/admin/users", token).await
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_listing_carries_each_users_tenant_plan_and_last_sign_in(pool: PgPool) {
    provision(&pool).await;
    identity_audit_standin(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let (status, body) = call(state(pool, Some(backoffice)), Some(&TOKEN_OPERATOR)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: UsersView = serde_json::from_slice(&body).expect("the listing parses");
    assert_eq!(
        view.users.len(),
        2,
        "both tenants' users appear in one read"
    );

    let linked = view
        .users
        .iter()
        .find(|row| row.user == USER_OPERATOR)
        .expect("the linked user is listed");
    assert_eq!(
        (
            linked.organisation.org,
            linked.organisation.name.as_str(),
            linked.organisation.slug.as_deref()
        ),
        (ORG_A, "Riverbend Resources", Some("riverbend")),
        "the row names the tenant, so a support email quoting a slug finds its user"
    );
    assert_eq!(
        linked.plan,
        Plan::Subscriber,
        "and the plan that tenant holds, derived from its live grant"
    );
    assert_eq!(
        linked.auth_subject,
        Some(Uuid(SUBJECT_OPERATOR)),
        "the subject the console's identity listing keys on travels with the row"
    );
    assert_eq!(
        linked.last_sign_in_at,
        Some(Timestamp(1_772_649_000_000)),
        "the later of the two sign-ins, and not the sign-up beside them"
    );

    let unlinked = view
        .users
        .iter()
        .find(|row| row.user == USER_SELLER)
        .expect("the user with no subject is listed too, rather than dropped by the join");
    assert_eq!(
        unlinked.auth_subject, None,
        "a user the identity plane never linked carries no subject"
    );
    assert_eq!(
        unlinked.last_sign_in_at, None,
        "and no sign-in instant, which is a different fact from the epoch"
    );
    assert_eq!(
        unlinked.plan,
        Plan::Free,
        "a tenant with no grant holds free, which is what the entitlement read says"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_database_without_the_identity_schema_still_answers_the_listing(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let (status, body) = call(state(pool, Some(backoffice)), Some(&TOKEN_OPERATOR)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the two DDL sets are applied by separate commands, so a database \
         holding one and not the other answers what it can: {}",
        String::from_utf8_lossy(&body)
    );
    let view: UsersView = serde_json::from_slice(&body).unwrap_or(UsersView {
        users: Vec::new(),
        total: 0,
        next_cursor: None,
    });
    assert_eq!(view.users.len(), 2, "every user is listed");
    assert!(
        view.users.iter().all(|row| row.last_sign_in_at.is_none()),
        "with the one column that needed the identity schema absent, rather than \
         the whole page refusing for it"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_and_an_anonymous_caller_are_refused_identically(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let (seller_status, seller_body) = call(
        state(pool.clone(), Some(backoffice.clone())),
        Some(&TOKEN_SELLER),
    )
    .await;
    assert_eq!(
        seller_status,
        StatusCode::UNAUTHORIZED,
        "a seller holding a live session is not an operator"
    );
    let (anonymous_status, anonymous_body) = call(state(pool, Some(backoffice)), None).await;
    assert_eq!(anonymous_status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        seller_body, anonymous_body,
        "the two refusals are byte-identical, or the difference is itself the \
         answer a prober wanted"
    );
}

// ------------------------------------------------------------------ paging

/// Seven more sellers in the second tenant, two of them created in the same
/// millisecond as each other, so the cursor's tie-break on the user id is
/// what keeps them apart.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn more_sellers(pool: &PgPool) -> Vec<UserId> {
    let sessions = SessionRepo::new(pool.clone());
    let mut made = Vec::new();
    for (index, at) in [2_000, 3_000, 3_000, 4_000, 4_500, 6_000, 7_000]
        .into_iter()
        .enumerate()
    {
        let byte = 0xC0 + u8::try_from(index).expect("a handful of sellers");
        let user = UserId(Uuid([byte; 16]));
        sessions
            .create_user(
                ORG_B,
                user,
                &format!("seller{index}@example.test"),
                Timestamp(at),
            )
            .await
            .expect("the seller provisions");
        made.push(user);
    }
    made
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn paging_walks_every_user_once_newest_first(pool: PgPool) {
    provision(&pool).await;
    let extra = more_sellers(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let mut seen: Vec<(Timestamp, UserId)> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let uri = match &cursor {
            Some(next) => format!("/v1/admin/users?limit=2&cursor={next}"),
            None => "/v1/admin/users?limit=2".to_owned(),
        };
        let (status, body) = send(
            state(pool.clone(), Some(backoffice.clone())),
            "GET",
            &uri,
            Some(&TOKEN_OPERATOR),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let view: UsersView = serde_json::from_slice(&body).expect("the page parses");
        assert_eq!(view.total, 9, "the total counts every user, not the page");
        assert!(view.users.len() <= 2, "a page holds at most what was asked");
        seen.extend(view.users.iter().map(|row| (row.created_at, row.user)));
        pages += 1;
        match view.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
        assert!(pages < 10, "the walk ends");
    }
    assert_eq!(pages, 5, "nine users at two a page");
    assert_eq!(seen.len(), 9, "every user appears");
    let mut unique: Vec<UserId> = seen.iter().map(|&(_, user)| user).collect();
    unique.sort_by_key(|user| user.0 .0);
    unique.dedup();
    assert_eq!(unique.len(), 9, "and none appears twice across pages");
    assert!(
        extra.iter().all(|user| unique.contains(user)),
        "the two sellers sharing a millisecond are both reached"
    );
    assert!(
        seen.windows(2)
            .all(|pair| (pair[0].0, pair[0].1 .0 .0) > (pair[1].0, pair[1].1 .0 .0)),
        "newest first, ties broken by the user id, strictly: {seen:?}"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_page_size_or_cursor_the_listing_never_gave_is_refused(pool: PgPool) {
    provision(&pool).await;
    let backoffice = backoffice_pool(&pool).await;
    for uri in [
        "/v1/admin/users?limit=0",
        "/v1/admin/users?limit=101",
        "/v1/admin/users?cursor=yesterday",
        "/v1/admin/users?cursor=12_not-a-uuid",
    ] {
        let (status, _) = send(
            state(pool.clone(), Some(backoffice.clone())),
            "GET",
            uri,
            Some(&TOKEN_OPERATOR),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
    }
}

// --------------------------------------------------------------- operators

/// The seller's identity subject, linked in the tests that act on them.
const SUBJECT_SELLER: [u8; 16] = [0xB1; 16];

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn link_seller(pool: &PgPool) {
    sqlx::query("UPDATE app_user SET auth_subject = $2 WHERE id = $1")
        .bind(uuid::Uuid::from_bytes(USER_SELLER.0 .0))
        .bind(uuid::Uuid::from_bytes(SUBJECT_SELLER))
        .execute(pool)
        .await
        .expect("the subject links");
}

fn operator_uri(subject: [u8; 16]) -> String {
    format!(
        "/v1/admin/operators/{}",
        uuid::Uuid::from_bytes(subject).hyphenated()
    )
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_grants_and_withdraws_the_marking_with_a_trail(pool: PgPool) {
    provision(&pool).await;
    link_seller(&pool).await;
    let repo = OperatorRepo::new(pool.clone());
    let actor = format!(
        "operator:{}",
        uuid::Uuid::from_bytes(USER_OPERATOR.0 .0).hyphenated()
    );

    for _ in 0..2 {
        let (status, body) = send(
            state(pool.clone(), None),
            "POST",
            &operator_uri(SUBJECT_SELLER),
            Some(&TOKEN_OPERATOR),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let view: OperatorMarkingView = serde_json::from_slice(&body).expect("the answer parses");
        assert_eq!((view.user, view.operator), (USER_SELLER, true));
    }
    assert!(
        repo.is_active(USER_SELLER)
            .await
            .expect("the marking reads"),
        "the seller is now an operator"
    );
    let trail = repo.events(USER_SELLER).await.expect("the trail reads");
    assert_eq!(
        trail
            .iter()
            .map(|event| (event.granted, event.actor.as_str()))
            .collect::<Vec<_>>(),
        vec![(true, actor.as_str())],
        "one grant naming who made it: the second POST found them an operator \
         already and wrote nothing"
    );

    let (status, body) = send(
        state(pool.clone(), None),
        "DELETE",
        &operator_uri(SUBJECT_SELLER),
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let view: OperatorMarkingView = serde_json::from_slice(&body).expect("the answer parses");
    assert!(!view.operator);
    assert!(
        !repo
            .is_active(USER_SELLER)
            .await
            .expect("the marking reads"),
        "the withdrawal takes effect"
    );
    let trail = repo.events(USER_SELLER).await.expect("the trail reads");
    assert_eq!(
        trail
            .iter()
            .map(|event| (event.granted, event.actor.as_str()))
            .collect::<Vec<_>>(),
        vec![(true, actor.as_str()), (false, actor.as_str())],
        "the withdrawal is on the trail too, naming who made it"
    );
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_cannot_withdraw_their_own_marking(pool: PgPool) {
    provision(&pool).await;
    let (status, body) = send(
        state(pool.clone(), None),
        "DELETE",
        &operator_uri(SUBJECT_OPERATOR),
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        String::from_utf8_lossy(&body).contains("\"refusal\":\"self\""),
        "the refusal says why: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        OperatorRepo::new(pool.clone())
            .is_active(USER_OPERATOR)
            .await
            .expect("the marking reads"),
        "and the marking stands"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unknown_user_cannot_be_granted_or_withdrawn(pool: PgPool) {
    provision(&pool).await;
    // USER_SELLER exists but carries no subject, so this names nobody.
    for method in ["POST", "DELETE"] {
        let (status, _) = send(
            state(pool.clone(), None),
            method,
            &operator_uri(SUBJECT_SELLER),
            Some(&TOKEN_OPERATOR),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} names nobody");
    }
    let (status, _) = send(
        state(pool.clone(), None),
        "POST",
        "/v1/admin/operators/not-a-subject",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert!(status.is_client_error(), "a malformed subject is refused");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_cannot_make_themselves_an_operator(pool: PgPool) {
    provision(&pool).await;
    link_seller(&pool).await;
    let (status, _) = send(
        state(pool.clone(), None),
        "POST",
        &operator_uri(SUBJECT_SELLER),
        Some(&TOKEN_SELLER),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        !OperatorRepo::new(pool.clone())
            .is_active(USER_SELLER)
            .await
            .expect("the marking reads"),
        "and nothing was granted"
    );
}
