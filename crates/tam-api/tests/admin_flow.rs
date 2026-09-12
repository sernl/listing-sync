//! The operator surface over the wire: what an operator reads, what everyone
//! else is told, and what a deployment without a backoffice database answers.
//!
//! The severity of this file is in its negatives. Two tenants exist, and the
//! seller session belonging to one of them must be refused on every operator
//! route with the same body an anonymous caller gets -- if a non-operator ever
//! learned the difference between "your session is dead" and "you are not an
//! operator", `/admin` would be an oracle telling an attacker both that their
//! session is live and that the operator surface is real.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_api::admin::{
    DeadLettersView, FailedWriteView, FailedWritesView, ImpersonationsView, ImportDrainView,
    OrgDetailView, OrgsView, SignupsView, SyncHealthView,
};
use tam_api::openapi::ROUTES;
use tam_api::{router, APIError, APIErrorCode, APIErrorKind, AppState, Config, SESSION_COOKIE};
use tam_storage::{BackofficeRepo, OperatorRepo, SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_OPERATOR: UserId = UserId(Uuid([0x0A; 16]));
const USER_SELLER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_OPERATOR: SessionToken = SessionToken([0x41; 32]);
const TOKEN_SELLER: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(5_000);

/// Every operator route, with the organisation path already concrete. Used
/// whole by the refusal tests, so a route added to the router and forgotten
/// here is a gap a reviewer can see rather than one the suite hides.
const ADMIN_PATHS: [&str; 9] = [
    "/v1/admin/signups",
    "/v1/admin/orgs",
    "/v1/admin/orgs/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    "/v1/admin/sync-health",
    "/v1/admin/failed-writes",
    "/v1/admin/import-drain",
    "/v1/admin/dead-letters",
    "/v1/admin/impersonations",
    "/v1/admin/users",
];

/// The mounted operator route `ADMIN_PATHS` does not carry, named rather than
/// left absent.
///
/// `marketplace_requests::list_all` is the one operator route outside
/// `admin.rs`, and the fixture it needs belongs to that module's own slice.
/// Listing it here keeps the closure below failing for the next route somebody
/// forgets, which is the whole point of closing the world; an unlisted absence
/// would make the closure vacuous instead.
/// The two plan-grant routes are here rather than in `ADMIN_PATHS` because
/// that list drives GET refusal loops, and a POST route answered by those
/// loops would be testing method routing rather than the operator fence.
/// Their own refusal is asserted by `a_seller_cannot_grant_themselves_a_plan`.
const ADMIN_PATHS_UNCOVERED: [&str; 12] = [
    "/{version}/admin/marketplace-requests",
    "/{version}/admin/orgs/{org}/plan",
    "/{version}/admin/orgs/{org}/plan/{grant}/revoke",
    // The guide corpus is global and lives on the application pool, so these
    // three are the operator routes that keep serving when no backoffice
    // database is configured -- which is exactly what the second loop below
    // asserts does not happen, and it is right about every route that reads
    // across the tenant fence. They are not those routes. Their refusal for a
    // caller who is not an operator is asserted in `guides_flow`, against the
    // same blank 401 this file demands everywhere else.
    "/{version}/admin/guides",
    "/{version}/admin/guides/images",
    "/{version}/admin/guides/_preview",
    "/{version}/admin/guides/_taxonomy",
    "/{version}/admin/guides/_taxonomy/{kind}",
    "/{version}/admin/guides/_taxonomy/{kind}/{id}",
    "/{version}/admin/guides/{slug}",
    "/{version}/admin/guides/{slug}/publish",
    "/{version}/admin/guides/{slug}/unpublish",
];

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
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice,
        blobs: None,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
/// Runs a group of fixture statements under one tenant pin in one
/// transaction. A group rather than a statement: the catalogue's payload
/// assertion is a deferred constraint trigger, so a product and the file that
/// satisfies it must land together or the commit refuses them both.
async fn pinned(pool: &PgPool, org: OrgId, statements: &[String]) {
    let mut tx = pool.begin().await.expect("the fixture transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    for sql in statements {
        sqlx::query(sql)
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .execute(&mut *tx)
            .await
            .expect("the fixture row inserts");
    }
    tx.commit().await.expect("the fixture transaction commits");
}

/// Two tenants, each carrying one of everything the operator surface reads,
/// plus a seller session in one of them and an operator session in the other.
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

    for (org, product, mapping, job, item, attempt) in [
        (ORG_A, 0x11u8, 0x21u8, 0x31u8, 0x41u8, 0x51u8),
        (ORG_B, 0x12, 0x22, 0x32, 0x42, 0x52),
    ] {
        let id = |byte: u8| uuid::Uuid::from_bytes([byte; 16]).to_string();
        pinned(
            pool,
            org,
            &[
                format!(
                    "INSERT INTO blob (org_id, hash, byte_len, object_key, dek_key_version, \
                         first_seen_at) \
                     VALUES ($1, '\\x{product:02x}'::bytea, 4, 'fixture', 1, now())"
                ),
                format!(
                    "INSERT INTO product (org_id, id, title, body, body_format, price_kind, \
                         rights_state, created_at, updated_at) \
                     VALUES ($1, '{}', 'fixture', 'body', 'markdown', 'free', 'unstated', \
                         now(), now())",
                    id(product)
                ),
                format!(
                    "INSERT INTO product_file (org_id, id, product_id, position, role, kind, \
                         hash, scan_state, created_at) \
                     VALUES ($1, '{}', '{}', 0, 'payload', 'pdf', '\\x{product:02x}'::bytea, \
                         'pending', now())",
                    id(product.wrapping_add(0x70)),
                    id(product)
                ),
            ],
        )
        .await;
        pinned(
            pool,
            org,
            &[format!(
                "INSERT INTO mapping (org_id, id, product_id, inventory, marketplace, \
                     binding_state, verify_state, normaliser_version, policy_title, \
                     policy_description, policy_price, policy_taxonomy, policy_grades, \
                     policy_files, price_rule_kind, price_explicit_kind, publish_mode, \
                     lifecycle_state, created_at, updated_at) \
                 VALUES ($1, '{}', '{}', 'tes', 'tes', 'unbound', 'stale', 1, \
                     'managed', 'managed', 'managed', 'managed', 'managed', 'managed', \
                     'explicit', 'free', 'dry_run', 'absent', now(), now())",
                id(mapping),
                id(product)
            )],
        )
        .await;
        pinned(
            pool,
            org,
            &[format!(
                "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
                 VALUES ($1, '{}', 'tes', 'linked', now(), now())",
                id(product.wrapping_add(0x60))
            )],
        )
        .await;
        pinned(
            pool,
            org,
            &[format!(
                "INSERT INTO job (org_id, id, inventory, marketplace, created_at, actor_kind) \
                 VALUES ($1, '{}', 'tes', 'tes', now(), 'system')",
                id(job)
            )],
        )
        .await;
        pinned(
            pool,
            org,
            &[format!(
                "INSERT INTO job_item (org_id, id, job_id, mapping_id, idempotency_key, state, \
                     operation, outcome, failure_code, failure_detail, settled_at, created_at, \
                     marketplace) \
                 SELECT $1, '{}', '{}', '{}', gen_random_uuid(), 'settled', 'create', \
                     'failed', 'Other', 'the fixture failure', now(), now(), m.marketplace \
                 FROM mapping m WHERE m.org_id = $1 AND m.id = '{}'",
                id(item),
                id(job),
                id(mapping),
                id(mapping)
            )],
        )
        .await;
        pinned(
            pool,
            org,
            &[format!(
                "INSERT INTO write_attempt (org_id, id, job_item_id, mapping_id, lease_epoch, \
                     intent, intent_hash, state, opened_at, settled_at, failure_code, \
                     actor_kind) \
                 VALUES ($1, '{}', '{}', '{}', 0, '{{}}'::jsonb, '\\x00'::bytea, 'settled', \
                     now(), now(), 'Other', 'system')",
                id(attempt),
                id(item),
                id(mapping)
            )],
        )
        .await;
    }

    pinned(
        pool,
        ORG_B,
        &[
            "INSERT INTO org_halt (org_id, raised_by, reason, raised_at) \
             VALUES ($1, 'founder', 'a fixture halt', now())"
                .to_owned(),
        ],
    )
    .await;

    // Under one tenant only. A subscription seeded under both would let a
    // detail read that ignored the organisation it was given still pass.
    pinned(
        pool,
        ORG_B,
        &["INSERT INTO billing_subscription \
             (org_id, paddle_subscription_id, paddle_customer_id, status, \
              current_period_end, occurred_at, updated_at) \
             VALUES ($1, 'sub_fixture', 'ctm_fixture', 'active', NULL, \
                 timestamptz '2026-02-03T04:05:06Z', now())"
            .to_owned()],
    )
    .await;
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn grant_operator(pool: &PgPool) {
    OperatorRepo::new(pool.clone())
        .grant(USER_OPERATOR, "the test fixture", NOW)
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
    pool: PgPool,
    backoffice: Option<PgPool>,
    path: &str,
    token: Option<&SessionToken>,
) -> Answer {
    let request = Request::builder().uri(path);
    let request = match token {
        Some(token) => request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        ),
        None => request,
    };
    let response = router(state(pool, backoffice))
        .oneshot(request.body(Body::empty()).expect("the request builds"))
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

fn assert_blank_refusal(answer: &Answer, what: &str) {
    assert_eq!(
        answer.status,
        StatusCode::UNAUTHORIZED,
        "{what} must be refused with 401"
    );
    let error: APIError = answer.json();
    assert_eq!(
        (error.errors[0].code, error.errors[0].kind),
        (
            Some(APIErrorCode::SessionRequired),
            Some(APIErrorKind::Unauthenticated)
        ),
        "{what} must be refused in exactly the words an anonymous caller gets, \
         so the status cannot be read as an answer about who is an operator"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_reads_every_tenant_from_one_request(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/orgs",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the operator is admitted");
    let view: OrgsView = answer.json();
    assert_eq!(
        view.orgs.len(),
        2,
        "both tenants appear in one unpinned listing"
    );
    for org in &view.orgs {
        assert_eq!(
            (org.products, org.mappings, org.connections, org.users),
            (1, 1, 1, 1),
            "{} carries the fenced rows seeded under it, counted across the fence",
            org.name
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_ledger_aggregate_spans_tenants(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/sync-health",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: SyncHealthView = answer.json();
    assert_eq!(view.jobs, 2, "one job under each tenant");
    assert_eq!(
        (view.items, view.settled, view.failed),
        (2, 2, 2),
        "the item counts are the sum over tenants, in the stored state vocabulary"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn failed_writes_carry_both_tenants_with_the_owning_item(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/failed-writes",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: FailedWritesView = answer.json();
    assert_eq!(view.writes.len(), 2, "one failed attempt under each tenant");
    let mut orgs: Vec<OrgId> = view.writes.iter().map(|write| write.org).collect();
    orgs.sort_by_key(|org| org.0 .0);
    assert_eq!(orgs, vec![ORG_A, ORG_B], "both tenants are represented");
    for write in &view.writes {
        assert_eq!(
            write.item_failure_detail.as_deref(),
            Some("the fixture failure"),
            "the owning item's detail travels beside the attempt's own code"
        );
    }
}

/// A stranded attempt reaches an operator; one whose run is still going does
/// not.
///
/// The definition is what makes this a view rather than a firehose, and it is
/// the lease rather than the clock: a heartbeat renews for as long as a device
/// keeps working, so elapsed time says nothing. An attempt whose item has moved
/// on — settled, or leased again at a higher epoch — belonged to a run that is
/// over, and nothing will settle it now.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_stranded_attempt_reaches_an_operator_and_a_live_one_does_not(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let id = |byte: u8| uuid::Uuid::from_bytes([byte; 16]).to_string();
    // Org A's attempt is stranded: its item has settled, so the run that
    // could have settled the attempt is gone. Org B's is healthy: its item is
    // still leased at the same epoch the attempt names, which is a run in
    // progress and holds its attempt open for as long as it keeps
    // heartbeating. `write_attempt_one_in_flight` is unique per mapping,
    // which is why they are on different tenants.
    pinned(
        &pool,
        ORG_B,
        &[
            "UPDATE job_item SET state = 'leased', lease_owner = 'a-working-device', \
           lease_expires_at = now() + interval '5 minutes', outcome = NULL, \
           failure_code = NULL, failure_detail = NULL, settled_at = NULL \
           WHERE org_id = $1"
                .to_owned(),
        ],
    )
    .await;
    for (org, attempt, item, mapping, age) in [
        (ORG_A, 0x53u8, 0x41u8, 0x21u8, "1 day"),
        (ORG_B, 0x54u8, 0x42u8, 0x22u8, "0 seconds"),
    ] {
        // Org B's is the live one: same epoch as its item, which is leased.
        pinned(
            &pool,
            org,
            &[format!(
                "INSERT INTO write_attempt (org_id, id, job_item_id, mapping_id, lease_epoch, \
                     intent, intent_hash, state, opened_at, actor_kind) \
                 VALUES ($1, '{}', '{}', '{}', 0, '{{}}'::jsonb, '\\x01'::bytea, 'in_flight', \
                     now() - interval '{age}', 'system')",
                id(attempt),
                id(item),
                id(mapping)
            )],
        )
        .await;
    }

    let backoffice = backoffice_pool(&pool).await;
    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/failed-writes",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: FailedWritesView = answer.json();
    // Before the bump, org B's attempt is live: same epoch as its item, and
    // the item is leased. It does not surface, which is the half a
    // clock-keyed view got wrong.
    assert_eq!(
        view.writes
            .iter()
            .filter(|write| write.failure_code.is_none())
            .count(),
        1,
        "only the settled item's attempt is stranded so far: {:?}",
        view.writes.iter().map(|w| w.attempt).collect::<Vec<_>>()
    );

    // And now the stolen-lease half: org B's item is leased again at a higher
    // epoch than its attempt names. The item is live, so the
    // item-state disjunct says nothing about it; only the epoch does.
    pinned(
        &pool,
        ORG_B,
        &["UPDATE job_item SET lease_epoch = lease_epoch + 1 WHERE org_id = $1".to_owned()],
    )
    .await;

    let view: FailedWritesView = call(
        pool.clone(),
        Some(backoffice_pool(&pool).await),
        "/v1/admin/failed-writes",
        Some(&TOKEN_OPERATOR),
    )
    .await
    .json();

    let stranded: Vec<&FailedWriteView> = view
        .writes
        .iter()
        .filter(|write| write.failure_code.is_none())
        .collect();
    assert_eq!(
        stranded.len(),
        2,
        "both halves of the definition surface: the settled item's attempt and the one \
         whose lease was stolen: {:?}",
        view.writes.iter().map(|w| w.attempt).collect::<Vec<_>>()
    );
    assert!(
        stranded
            .iter()
            .any(|write| write.attempt == tam_types::Uuid([0x54; 16])),
        "including the stolen-lease one, which no clock and no item state would catch"
    );
    for write in &stranded {
        assert_eq!(
            write.state, "in_flight",
            "the rows say what they are; there is no failure code to say it with"
        );
    }
    // By index rather than by filter: stranded rows are the oldest by
    // construction, so a newest-first page would push them off the end and an
    // operator would never see the ones that most need them.
    assert!(
        view.writes[0].failure_code.is_none() && view.writes[1].failure_code.is_none(),
        "stranded rows come first in the page, ahead of every failure: {:?}",
        view.writes
            .iter()
            .map(|w| (w.attempt, w.failure_code))
            .collect::<Vec<_>>()
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_organisation_renders_its_connections_and_halts(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/orgs/bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: OrgDetailView = answer.json();
    assert_eq!(view.org.org, ORG_B, "the organisation the path named");
    assert_eq!(view.connections.len(), 1, "its one connection is rendered");
    assert_eq!(
        view.connections[0].state, "linked",
        "the stored link state travels beside the derived status"
    );
    assert_eq!(view.halts.len(), 1, "its tenant-wide halt is rendered");
    assert!(
        view.halts[0].inventory.is_none(),
        "a halt with no inventory is the tenant-wide one"
    );
    let subscription = view
        .subscription
        .expect("the tenant carrying a subscription reports one");
    assert_eq!(
        subscription.status, "active",
        "Paddle's own vocabulary reaches the operator untranslated"
    );
    assert!(
        subscription.current_period_end.is_none(),
        "a subscription Paddle sent no billing period for reports none, \
         rather than a fabricated instant"
    );
    assert_eq!(
        subscription.occurred_at,
        Timestamp(1_770_091_506_000),
        "the instant Paddle stamped on the notification, not the instant of the write"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_organisation_that_never_reached_checkout_reports_no_subscription(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/orgs/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: OrgDetailView = answer.json();
    assert_eq!(view.org.org, ORG_A, "the organisation the path named");
    assert!(
        view.subscription.is_none(),
        "a tenant that never reached checkout carries no subscription at all, \
         which is a different fact from a cancelled one"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_organisation_nobody_holds_is_a_structured_not_found(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/orgs/cccccccc-cccc-cccc-cccc-cccccccccccc",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::NOT_FOUND,
        "an operator asking after an organisation that does not exist is told so"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn signups_omit_the_identity_series_where_that_schema_is_absent(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/signups",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: SignupsView = answer.json();
    assert!(
        view.identity.is_none(),
        "a database carrying no identity schema reports no identity series at all, \
         rather than a zero that would read as nobody having signed up"
    );
    assert_eq!(
        view.provisioned.iter().map(|day| day.count).sum::<i64>(),
        2,
        "both provisioned users are counted from app_user"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn signups_carry_the_identity_series_where_that_schema_is_present(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    // A stand-in for db/auth/0002_audit_event.sql, which a throwaway test
    // database does not carry: those files are applied by `just auth-migrate`
    // as tam_auth, and this crate's migrations are the only DDL sqlx::test
    // runs. Only the two columns this read names are reproduced.
    for statement in [
        "CREATE SCHEMA auth",
        "CREATE TABLE auth.auth_event (event text NOT NULL, at timestamptz NOT NULL)",
        "INSERT INTO auth.auth_event (event, at) VALUES \
         ('user_signed_up', now()), ('user_signed_up', now()), ('user_signed_in', now())",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("the identity stand-in builds");
    }
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/signups",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: SignupsView = answer.json();
    let identity = view.identity.expect("the identity series is present");
    assert_eq!(
        identity.iter().map(|day| day.count).sum::<i64>(),
        2,
        "only signup events are counted; the sign-in beside them is not one"
    );
}

/// The two identity-plane subject ids one impersonation names. Deliberately
/// not `UserId`s: `auth.auth_event` records `auth."user".id`, which reaches
/// `app_user` only through the `auth_subject` join, and a test that reused a
/// platform user id here would assert the two planes number their users the
/// same way when they do not.
const SUBJECT_ADMIN: [u8; 16] = [0xA1; 16];
const SUBJECT_IMPERSONATED: [u8; 16] = [0xB2; 16];

/// A stand-in for `auth.auth_event` as `db/auth/0002_audit_event.sql` and
/// `0003_impersonation_event.sql` leave it, which a throwaway test database
/// does not carry: those files are applied by `just auth-migrate` as tam_auth,
/// and this crate's migrations are the only DDL sqlx::test runs. Only the
/// columns the impersonation read names are reproduced, `id` among them
/// because the ordering tiebreak uses it -- and the constraint naming both
/// parties, because that is what entitles the read to treat them as non-null.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn identity_audit_standin(pool: &PgPool) {
    let subject = |bytes: [u8; 16]| uuid::Uuid::from_bytes(bytes).to_string();
    for statement in [
        "CREATE SCHEMA auth".to_owned(),
        "CREATE TABLE auth.auth_event (              id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,              event text NOT NULL, user_id uuid, target_user_id uuid,              ip_address text, at timestamptz NOT NULL, CONSTRAINT auth_event_impersonation_parties_identified CHECK (event NOT IN ('user_impersonated', 'user_impersonation_stopped') OR (user_id IS NOT NULL AND target_user_id IS NOT NULL)))"
            .to_owned(),
        // Newest last, so a read that forgot to reverse would return them in
        // insertion order and fail rather than pass by accident. The sign-in
        // beside them belongs to neither party and must not be listed.
        format!(
            "INSERT INTO auth.auth_event (event, user_id, target_user_id, ip_address, at) VALUES \
             ('user_signed_in', '{admin}', NULL, '198.51.100.9', now() - interval '3 minutes'), \
             ('user_impersonated', '{admin}', '{target}', '203.0.113.7', \
              now() - interval '2 minutes'), \
             ('user_impersonation_stopped', '{admin}', '{target}', '203.0.113.7', \
              now() - interval '1 minute')",
            admin = subject(SUBJECT_ADMIN),
            target = subject(SUBJECT_IMPERSONATED),
        ),
    ] {
        sqlx::query(&statement)
            .execute(pool)
            .await
            .expect("the identity stand-in builds");
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_reads_who_impersonated_whom(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    identity_audit_standin(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/impersonations",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: ImpersonationsView = answer.json();
    let events = view
        .impersonations
        .expect("the identity audit trail is visible from here");

    assert_eq!(
        events
            .iter()
            .map(|row| row.event.as_str())
            .collect::<Vec<_>>(),
        vec!["user_impersonation_stopped", "user_impersonated"],
        "both new event names round-trip, newest first, and the sign-in row \
         beside them is not an impersonation"
    );
    for row in &events {
        assert_eq!(
            (row.actor, row.target),
            (Uuid(SUBJECT_ADMIN), Uuid(SUBJECT_IMPERSONATED)),
            "the actor is the admin and the target is the party signed in as, \
             in that order -- reversing them would name the wrong impersonator"
        );
        assert_eq!(
            row.ip.as_deref(),
            Some("203.0.113.7"),
            "the network origin travels with the record"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn impersonations_are_absent_where_the_identity_schema_is(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/impersonations",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: ImpersonationsView = answer.json();
    assert!(
        view.impersonations.is_none(),
        "a database carrying no identity schema reports no record at all, rather \
         than an empty list that would read as nobody having been impersonated"
    );
}

/// Every operator route the router mounts is reached by `ADMIN_PATHS`.
///
/// The two refusal loops below read that array whole, so a route mounted and
/// not listed is covered by neither: the suite stays green while an operator
/// route goes unproven against a seller, an anonymous caller and a deployment
/// with no backoffice database. Closing the world here is what turns the next
/// omission into a failure rather than something a reader has to notice.
///
/// The comparison is by prefix because `ADMIN_PATHS` holds concrete paths and
/// the route table holds patterns: `/{version}/admin/orgs/{org}` is reached by
/// the entry naming an actual organisation.
#[test]
fn every_mounted_admin_route_is_reached_by_the_refusal_loops() {
    let mounted: Vec<&str> = ROUTES
        .iter()
        .map(|route| route.path)
        .filter(|path| path.starts_with("/{version}/admin"))
        .filter(|path| !ADMIN_PATHS_UNCOVERED.contains(path))
        .collect();
    for path in &mounted {
        let concrete = path.replacen("{version}", "v1", 1);
        let prefix = concrete
            .split_once('{')
            .map_or(concrete.clone(), |(head, _)| head.to_owned());
        assert!(
            ADMIN_PATHS.iter().any(|listed| listed.starts_with(&prefix)),
            "{path} is mounted but no ADMIN_PATHS entry reaches it, so no refusal \
             test covers it"
        );
    }
    assert_eq!(
        mounted.len(),
        ADMIN_PATHS.len(),
        "every mounted operator route has exactly one entry; a count that drifts \
         means a path was listed twice or one was covered by another's prefix"
    );
}

/// One drain measurement in a tenant's ledger, at the position given.
///
/// Written as SQL rather than through `record_drain_report`, because this
/// crate holds no dependency on `tam-import` and gaining one to seed a fixture
/// would put a marketplace adapter in the API's test graph. The payload
/// spelling that writer produces is pinned byte for byte by its own test; what
/// this file proves is the route between the ledger and the client.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_drain(
    pool: &PgPool,
    org: OrgId,
    job_byte: u8,
    seq: i64,
    payload: &serde_json::Value,
) {
    let job = uuid::Uuid::from_bytes([job_byte; 16]).to_string();
    let body = serde_json::to_string(payload).expect("the fixture payload serialises");
    pinned(
        pool,
        org,
        &[format!(
            "INSERT INTO job_event \
             (org_id, org_seq, job_id, kind, payload, created_at, actor_kind) \
             VALUES ($1, {seq}, '{job}', 'ImportDrainMeasured', '{body}'::jsonb, now(), \
                 'system')"
        )],
    )
    .await;
}

/// A measurement in the spelling `record_drain_report` writes.
fn measurement(items_new: u32) -> serde_json::Value {
    serde_json::json!({
        "source": "Tes",
        "target": "Tes",
        "rows": 4,
        "terms_seen": 20,
        "terms_unmapped": 2,
        "terms_covered": 6,
        "items_new": items_new,
        "items_already_open": 1
    })
}

/// The rung between the role and the client: `backoffice_grants` proves the
/// role reads exactly these rows and `drain.test.ts` proves the client shapes
/// them, and until now nothing proved the route between them answers at all.
///
/// The sequences are seeded out of order within a tenant, so the ordering the
/// page's notion of "first" and "tenth" rests on is proved rather than
/// inherited from the order the fixture inserted.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_drain_route_carries_every_tenant_in_ledger_order(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    seed_drain(&pool, ORG_A, 0x31, 7, &measurement(3)).await;
    seed_drain(&pool, ORG_A, 0x31, 3, &measurement(9)).await;
    seed_drain(&pool, ORG_B, 0x32, 5, &measurement(4)).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/import-drain",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the operator is admitted");
    let view: ImportDrainView = answer.json();
    assert_eq!(
        view.rows
            .iter()
            .map(|row| (row.org, row.org_name.clone(), row.org_seq))
            .collect::<Vec<_>>(),
        vec![
            (ORG_A, "org-a".to_owned(), 3),
            (ORG_A, "org-a".to_owned(), 7),
            (ORG_B, "org-b".to_owned(), 5),
        ],
        "both tenants come back in one read, ordered by organisation name and \
         then by ledger position"
    );
    assert_eq!(
        view.rows
            .iter()
            .map(|row| row.payload.clone())
            .collect::<Vec<_>>(),
        vec![measurement(9), measurement(3), measurement(4)],
        "each payload travels verbatim: the route parses none of it, which is \
         what lets the client drop one malformed row and draw the rest"
    );
    assert!(
        !view.truncated,
        "three rows do not fill a limit of five hundred"
    );
}

/// A read that fills its limit says so.
///
/// The ordering is by organisation name, so a truncated read drops whole
/// tenants off the end of the alphabet and leaves rows that look exactly like
/// the complete platform. The exact-fit case is asserted beside it because it
/// is the one a length comparison gets wrong: a page of precisely the limit is
/// complete, and reporting it truncated would put a permanent warning on a
/// page that is telling the whole truth.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_drain_read_that_fills_its_limit_says_so(pool: PgPool) {
    provision(&pool).await;
    seed_drain(&pool, ORG_A, 0x31, 1, &measurement(3)).await;
    seed_drain(&pool, ORG_A, 0x31, 2, &measurement(2)).await;
    seed_drain(&pool, ORG_B, 0x32, 1, &measurement(1)).await;
    let repo = BackofficeRepo::new(backoffice_pool(&pool).await);

    let cut = repo.import_drain(2).await.ok();
    assert_eq!(
        cut.map(|page| (page.runs.len(), page.truncated)),
        Some((2, true)),
        "a limit of two over three rows carries two and reports the cut; the row \
         read past the limit is a probe and must not be served"
    );

    let whole = repo.import_drain(3).await.ok();
    assert_eq!(
        whole.map(|page| (page.runs.len(), page.truncated)),
        Some((3, false)),
        "a read that exactly fits its limit is complete, not truncated"
    );
}

/// One outbox row in the state given, written as the tenant's own settle
/// writes it. The dedupe key is the row's id so every seeded row is distinct
/// under `outbox_dedupe`.
async fn seed_outbox(pool: &PgPool, org: OrgId, id_byte: u8, topic: &str, state: &str) {
    let id = uuid::Uuid::from_bytes([id_byte; 16]).to_string();
    pinned(
        pool,
        org,
        &[format!(
            "INSERT INTO outbox_message \
             (org_id, id, topic, dedupe_key, payload, state, created_at, available_at, \
              attempts, last_error) \
             VALUES ($1, '{id}', '{topic}', '{id}', '{{\"event\":\"JobSettled\"}}'::jsonb, \
                 '{state}', now(), now(), 12, 'the mail relay refused: 422')"
        )],
    )
    .await;
}

/// Dead letters are counted by topic across every tenant, and a pending row
/// is not one: the read is of what the drainer gave up on, not of its queue.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn dead_letters_are_counted_by_topic_across_tenants(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    seed_outbox(&pool, ORG_A, 0x61, "email.job_settled", "dead").await;
    seed_outbox(&pool, ORG_A, 0x62, "email.job_settled", "dead").await;
    seed_outbox(&pool, ORG_B, 0x63, "email.job_settled", "dead").await;
    seed_outbox(&pool, ORG_B, 0x64, "push.job_settled", "dead").await;
    seed_outbox(&pool, ORG_B, 0x65, "email.job_settled", "pending").await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/dead-letters",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the operator is admitted");
    let view: DeadLettersView = answer.json();
    assert_eq!(
        view.topics
            .iter()
            .map(|topic| (topic.topic.as_str(), topic.messages, topic.orgs))
            .collect::<Vec<_>>(),
        vec![("email.job_settled", 3, 2), ("push.job_settled", 1, 1)],
        "three dead completion mails across two tenants and one dead push, by topic; \
         the pending row is the live queue and is not counted"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn every_admin_route_refuses_a_seller_and_an_anonymous_caller(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;

    for path in ADMIN_PATHS {
        let backoffice = backoffice_pool(&pool).await;
        let seller = call(
            pool.clone(),
            Some(backoffice.clone()),
            path,
            Some(&TOKEN_SELLER),
        )
        .await;
        assert_blank_refusal(&seller, &format!("a seller's live session on {path}"));

        let anonymous = call(pool.clone(), Some(backoffice), path, None).await;
        assert_blank_refusal(&anonymous, &format!("an anonymous request to {path}"));
        assert_eq!(
            seller.body, anonymous.body,
            "the two refusals on {path} must be byte-identical, or the difference \
             is itself the answer a prober wanted"
        );
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_revoked_operator_is_refused_on_the_very_next_request(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let admitted = call(
        pool.clone(),
        Some(backoffice.clone()),
        "/v1/admin/orgs",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_eq!(admitted.status, StatusCode::OK, "the grant admits");

    let withdrawn = OperatorRepo::new(pool.clone())
        .revoke(USER_OPERATOR, NOW)
        .await;
    assert_eq!(
        withdrawn.ok(),
        Some(true),
        "there was an active grant to withdraw"
    );

    let refused = call(
        pool.clone(),
        Some(backoffice),
        "/v1/admin/orgs",
        Some(&TOKEN_OPERATOR),
    )
    .await;
    assert_blank_refusal(
        &refused,
        "a revoked operator holding the same live session cookie",
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_deployment_without_a_backoffice_database_refuses_every_route(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;

    for path in ADMIN_PATHS {
        let operator = call(pool.clone(), None, path, Some(&TOKEN_OPERATOR)).await;
        assert_eq!(
            operator.status,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path} refuses cleanly rather than serving half a surface"
        );
        let refusal: APIError = operator.json();
        assert_eq!(
            (refusal.errors[0].code, refusal.errors[0].kind),
            (
                Some(APIErrorCode::BackofficeUnavailable),
                Some(APIErrorKind::Internal)
            ),
            "{path} names the condition, so a client can tell an unconfigured \
             operator surface from a fault"
        );
        // The order matters more than the status: a seller must still be
        // refused by the operator check, which runs first, so an unconfigured
        // deployment does not become a way to learn who is an operator.
        let seller = call(pool.clone(), None, path, Some(&TOKEN_SELLER)).await;
        assert_blank_refusal(&seller, &format!("a seller on unconfigured {path}"));
    }
}

// ------------------------------------------------------- the one write here

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn post_json(
    pool: PgPool,
    backoffice: Option<PgPool>,
    path: &str,
    token: Option<&SessionToken>,
    body: serde_json::Value,
) -> Answer {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    let request = match token {
        Some(token) => request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        ),
        None => request,
    };
    let response = router(state(pool, backoffice))
        .oneshot(
            request
                .body(Body::from(body.to_string()))
                .expect("the request builds"),
        )
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

fn org_path(org: OrgId) -> String {
    format!("/v1/admin/orgs/{}", org.0.to_hyphenated())
}

/// The backoffice grant end to end: an operator sets a plan with a reason and
/// an expiry, the org detail reports it, and revoking it puts the
/// organisation back on Free while keeping the row.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operator_grants_a_plan_and_takes_it_back(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = post_json(
        pool.clone(),
        Some(backoffice.clone()),
        &format!("{}/plan", org_path(ORG_A)),
        Some(&TOKEN_OPERATOR),
        serde_json::json!({
            "plan": "studio",
            "expires_at": 9_000_000,
            "reason": "customer zero, for the migration"
        }),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "the grant lands");
    let view: OrgDetailView = answer.json();
    assert_eq!(view.plan.plan, tam_limits::Plan::Studio);
    assert_eq!(view.plan.granted_by.as_deref(), Some("operator"));
    assert_eq!(view.grants.len(), 1);
    assert_eq!(
        view.grants[0].reason.as_deref(),
        Some("customer zero, for the migration"),
        "the audit row carries the operator's own words"
    );
    assert_eq!(
        view.grants[0].grantor_user.map(|user| user.to_hyphenated()),
        Some(USER_OPERATOR.0.to_hyphenated()),
        "and names who made it"
    );
    let grant = view.grants[0].id.to_hyphenated();

    let answer = post_json(
        pool.clone(),
        Some(backoffice.clone()),
        &format!("{}/plan/{grant}/revoke", org_path(ORG_A)),
        Some(&TOKEN_OPERATOR),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let view: OrgDetailView = answer.json();
    assert_eq!(
        view.plan.plan,
        tam_limits::Plan::Free,
        "a revoked grant stops entitling"
    );
    assert_eq!(
        view.grants.len(),
        1,
        "and stays in the record, which is what the audit trail is for"
    );
    assert!(view.grants[0].revoked_at.is_some());
}

/// A grant with no reason is refused, because an audit row whose reason is
/// blank answers none of the questions an audit row exists for.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_grant_without_a_reason_is_refused(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = post_json(
        pool.clone(),
        Some(backoffice.clone()),
        &format!("{}/plan", org_path(ORG_A)),
        Some(&TOKEN_OPERATOR),
        serde_json::json!({ "plan": "subscriber", "reason": "   " }),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);

    let answer = post_json(
        pool.clone(),
        Some(backoffice),
        &format!("{}/plan", org_path(ORG_A)),
        Some(&TOKEN_OPERATOR),
        serde_json::json!({ "plan": "migration_only", "reason": "bought over the phone" }),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a Catalogue Import grant with no rung would grant nothing, so it is refused \
         rather than written"
    );
}

/// The seller session is refused on the write exactly as it is on the reads:
/// a 401 that says nothing about whether the operator surface exists.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_cannot_grant_themselves_a_plan(pool: PgPool) {
    provision(&pool).await;
    grant_operator(&pool).await;
    let backoffice = backoffice_pool(&pool).await;

    let answer = post_json(
        pool.clone(),
        Some(backoffice),
        &format!("{}/plan", org_path(ORG_A)),
        Some(&TOKEN_SELLER),
        serde_json::json!({ "plan": "studio", "reason": "please" }),
    )
    .await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
}
