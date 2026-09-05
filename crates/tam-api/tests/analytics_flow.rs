//! The analytics summary over the wire: what a tenant reads, what it cannot
//! read, and what a tenant whose first capture has not run is told.
//!
//! `/v1/analytics/summary` carries no organisation identifier, so the tenancy
//! statement provable at this surface is the one asserted here: each session
//! sees the figures captured for its own listings and none of the other
//! tenant's, even though both tenants hold snapshots at the same instants for
//! the same metrics.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::analytics::AnalyticsSummary;
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode, Verification};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{
    AnalyticsRepo, MappingRepo, MetricSnapshot, ProductRepo, SessionRepo, SessionToken,
};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, ListingCopy,
    MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const ORG_C: OrgId = OrgId(Uuid([0xCC; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const TOKEN_C: SessionToken = SessionToken([0x43; 32]);
const MAPPING_A: MappingId = MappingId(Uuid([0x1A; 16]));
const MAPPING_B: MappingId = MappingId(Uuid([0x1B; 16]));
const NOW: Timestamp = Timestamp(5_000);

fn state(pool: PgPool) -> AppState {
    AppState {
        pool,
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    }
}

fn snapshot(mapping: MappingId, metric: &str, at: i64, value: f64) -> MetricSnapshot {
    MetricSnapshot {
        mapping,
        metric: metric.to_owned(),
        observed_at: Timestamp(at),
        total_value: value,
    }
}

/// The canonical product a mapping hangs off. Built through the domain type
/// rather than raw SQL, because `product` carries a deferred trigger that
/// refuses a product with no live payload file.
fn product(org: OrgId, id: MappingId) -> tam_domain::CanonicalProduct {
    let seed = id.0 .0[0];
    tam_domain::CanonicalProduct {
        id: ProductId(id.0),
        org,
        title: Title(format!("Fixture {seed}")),
        body: ListingCopy {
            body: "Fixture body.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([seed.wrapping_add(0x10); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Held {
                    hash: ContentHash([seed; 32]),
                    byte_len: 4,
                    scan: ScanOutcome::Pending,
                },
            },
            vec![],
        )),
        cover: None,
        previews: vec![],
        subjects: vec![],
        grades: tam_domain::GradeDeclaration {
            source: tam_domain::DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: tam_domain::RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

/// One product and the TPT mapping bound to it, per tenant. The mapping is
/// bound because an unbound one names no remote listing, and a snapshot is
/// about a listing.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_listing(pool: &PgPool, org: OrgId, mapping: MappingId, remote: u64) {
    ProductRepo::new(pool.clone())
        .insert(org, &product(org, mapping), Timestamp(1_000))
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(pool.clone())
        .insert(
            org,
            &Mapping {
                id: mapping,
                org,
                product: ProductId(mapping.0),
                inventory: InventoryId::Tpt,
                binding: Binding::Bound {
                    id: RemoteListingId::Tpt { product_id: remote },
                    first_seen: Timestamp(1_000),
                    verified: Verification::Stale {
                        since: Timestamp(1_000),
                    },
                },
                policies: FieldPolicies {
                    title: FieldPolicy::Managed,
                    description: FieldPolicy::Managed,
                    price: FieldPolicy::Managed,
                    taxonomy: FieldPolicy::Managed,
                    grades: FieldPolicy::Managed,
                    files: FieldPolicy::Managed,
                },
                price_rule: PriceRule::Explicit(PriceIntent::Free),
                publish: PublishMode::DryRun,
                lifecycle: RemoteLifecycle::Absent,
            },
            0,
            Timestamp(1_000),
        )
        .await
        .expect("the fixture mapping inserts");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b"), (ORG_C, "org-c")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG_A, UserId(Uuid([0x0A; 16])), "a@example.test", TOKEN_A),
        (ORG_B, UserId(Uuid([0x0B; 16])), "b@example.test", TOKEN_B),
        (ORG_C, UserId(Uuid([0x0C; 16])), "c@example.test", TOKEN_C),
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

    // Two tenants with a listing apiece; the third deliberately has none, so
    // the empty answer below is a tenant that exists rather than one that
    // failed to resolve.
    seed_listing(pool, ORG_A, MAPPING_A, 9_001).await;
    seed_listing(pool, ORG_B, MAPPING_B, 9_002).await;

    let analytics = AnalyticsRepo::new(pool.clone());
    // Identical metrics at identical instants on both tenants: only the org_id
    // separates them, which is exactly the fence under test.
    analytics
        .record(
            ORG_A,
            &[
                snapshot(MAPPING_A, "sales_count", 2_000, 17.0),
                snapshot(MAPPING_A, "sales_count", 1_000, 4.0),
                snapshot(MAPPING_A, "resource_views", 2_000, 1234.0),
            ],
        )
        .await
        .expect("the first tenant's snapshots record");
    analytics
        .record(
            ORG_B,
            &[
                snapshot(MAPPING_B, "sales_count", 2_000, 999.0),
                snapshot(MAPPING_B, "resource_views", 2_000, 5555.0),
            ],
        )
        .await
        .expect("the second tenant's snapshots record");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn summary(pool: PgPool, token: &SessionToken) -> AnalyticsSummary {
    let request = Request::builder()
        .uri("/v1/analytics/summary")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the summary answers the session that asked"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    serde_json::from_slice(&body).expect("the answer is the summary shape")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_tenant_reads_the_newest_figure_for_each_of_its_listings(pool: PgPool) {
    provision(&pool).await;
    let view = summary(pool, &TOKEN_A).await;
    assert_eq!(view.listings.len(), 1, "one listing, one row");
    let listing = &view.listings[0];
    assert_eq!(
        (listing.mapping, listing.inventory),
        (MAPPING_A, InventoryId::Tpt),
        "the row names the mapping and the inventory that mapping carries"
    );
    assert_eq!(
        listing.metrics.get("sales_count"),
        Some(&17.0),
        "the newest capture of the metric wins, not the first"
    );
    assert_eq!(
        listing.metrics.get("resource_views"),
        Some(&1234.0),
        "each captured metric travels under the name the capture stored"
    );
    assert_eq!(
        listing.observed_at,
        Timestamp(2_000),
        "the row states when its figures were taken"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_figures_are_invisible_to_the_other(pool: PgPool) {
    provision(&pool).await;
    let a = summary(pool.clone(), &TOKEN_A).await;
    let b = summary(pool, &TOKEN_B).await;
    assert_eq!(
        a.listings
            .iter()
            .map(|listing| listing.mapping)
            .collect::<Vec<_>>(),
        vec![MAPPING_A],
        "the first tenant sees only its own listing"
    );
    assert_eq!(
        b.listings
            .iter()
            .map(|listing| listing.mapping)
            .collect::<Vec<_>>(),
        vec![MAPPING_B],
        "and the second only its own, though both were captured at one instant"
    );
    assert_eq!(
        b.listings[0].metrics.get("sales_count"),
        Some(&999.0),
        "neither tenant's figure has leaked into the other's row"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_tenant_whose_capture_has_not_run_answers_an_empty_list(pool: PgPool) {
    provision(&pool).await;
    let view = summary(pool, &TOKEN_C).await;
    assert!(
        view.listings.is_empty(),
        "no captures is an empty list, which is what a first render needs"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_is_closed_to_a_session_that_does_not_resolve(pool: PgPool) {
    provision(&pool).await;
    let unknown = SessionToken([0x44; 32]);
    let request = Request::builder()
        .uri("/v1/analytics/summary")
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", unknown.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "a request without a live session is refused before any handler runs"
    );
}

// The capture's own two routes: what a device is told to read, and the reading
// it sends back. They live beside the summary tests for the reason the routes
// live beside the summary handler -- a reader looking for anything
// analytics-shaped looks once.

const DEVICE: &str = "aaaabbbbccccddddeeeeffff00001111";

/// A registered device reporting a connected Tpt session, written directly
/// because these tests are about the capture routes rather than about the
/// device registry, which has its own file.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_device(pool: &PgPool, org: OrgId, status: &str) {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'laptop', 'windows', 'x86_64', '0.1.0', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(DEVICE)
    .execute(&mut *tx)
    .await
    .expect("the device seeds");
    sqlx::query(
        "INSERT INTO device_marketplace_session \
         (org_id, device_id, marketplace, account_label, linked_at, last_used_at, status) \
         VALUES ($1, $2, 'tpt', NULL, now(), now(), $3)",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(DEVICE)
    .bind(status)
    .execute(&mut *tx)
    .await
    .expect("the session seeds");
    tx.commit().await.expect("the fixture commits");
}

/// A subscription that lapsed and stayed lapsed past the grace, which is what
/// D11 blocks on — not the absence of one, which is a Free tenant and entitled.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn lapse_entitlement(pool: &PgPool, org: OrgId) {
    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO billing_subscription \
         (org_id, paddle_subscription_id, paddle_customer_id, status, current_period_end, \
          occurred_at, updated_at) \
         VALUES ($1, $2, 'ctm_x', 'canceled', now() - interval '30 days', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(format!("sub_{}", org.0 .0[0]))
    .execute(&mut *tx)
    .await
    .expect("the lapsed subscription seeds");
    tx.commit().await.expect("the fixture commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn reads(pool: PgPool, token: &SessionToken) -> serde_json::Value {
    let request = Request::builder()
        .uri(format!("/v1/devices/{DEVICE}/reads"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .body(Body::empty())
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(response.status(), StatusCode::OK, "the read order answers");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    serde_json::from_slice(&body).expect("the read order parses")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn capture(pool: PgPool, token: &SessionToken, body: serde_json::Value) -> serde_json::Value {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/v1/devices/{DEVICE}/reads"))
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds");
    let response = router(state(pool))
        .oneshot(request)
        .await
        .expect("the router serves");
    assert_eq!(response.status(), StatusCode::OK, "the capture is accepted");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    serde_json::from_slice(&body).expect("the acceptance parses")
}

fn wire_snapshot(mapping: MappingId, metric: &str, at: i64, value: f64) -> serde_json::Value {
    serde_json::json!({
        "mapping": mapping,
        "metric": metric,
        "observed_at": at,
        "total_value": value,
    })
}

/// The loop a capture actually runs: the device is told what to read, reads it,
/// sends it back, and the summary the console renders is what it sent.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_is_told_what_to_capture_and_what_it_sends_back_is_recorded(pool: PgPool) {
    provision(&pool).await;
    seed_device(&pool, ORG_A, "connected").await;

    let order = reads(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        order["listings"].as_array().map(Vec::len),
        Some(1),
        "the tenant's one bound Tpt listing is what there is to capture: {order}"
    );
    assert_eq!(
        order["listings"][0]["remote"]["tpt"]["product_id"], 9_001,
        "and it carries the remote id the capture reads against: {order}"
    );

    let accepted = capture(
        pool.clone(),
        &TOKEN_A,
        serde_json::json!({
            "snapshots": [wire_snapshot(MAPPING_A, "sales_count", 7_000, 42.0)],
        }),
    )
    .await;
    assert_eq!(accepted["written"], 1, "the snapshot is stored: {accepted}");

    let summary = summary(pool, &TOKEN_A).await;
    assert_eq!(
        summary.listings[0].metrics.get("sales_count"),
        Some(&42.0),
        "and the console reads back what the device captured, which is the whole loop"
    );
}

/// A device holding no connected session is told to capture nothing, and told
/// it as an empty list rather than as a refusal.
///
/// Both halves matter. The empty list is what stops an hourly poll logging an
/// error nobody can act on; the post writing nothing is what stops a device
/// that lost its session mid-capture from reporting readings it should not
/// have been able to take.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_without_a_connected_session_captures_nothing(pool: PgPool) {
    provision(&pool).await;
    seed_device(&pool, ORG_A, "signed_out").await;

    let order = reads(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        order["listings"].as_array().map(Vec::len),
        Some(0),
        "a signed-out session is not one a capture can be made under: {order}"
    );

    let accepted = capture(
        pool.clone(),
        &TOKEN_A,
        serde_json::json!({
            "snapshots": [wire_snapshot(MAPPING_A, "sales_count", 7_000, 42.0)],
        }),
    )
    .await;
    assert_eq!(
        accepted["written"], 0,
        "and the post is re-checked rather than trusted from the order that prompted it: \
         {accepted}"
    );
}

/// A plan that lapsed past the grace is the same answer, which is D11 applied
/// to a read rather than to a write.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_lapsed_entitlement_captures_nothing(pool: PgPool) {
    provision(&pool).await;
    seed_device(&pool, ORG_A, "connected").await;
    lapse_entitlement(&pool, ORG_A).await;

    let order = reads(pool.clone(), &TOKEN_A).await;
    assert_eq!(
        order["listings"].as_array().map(Vec::len),
        Some(0),
        "a capture spends the connection's budget, so it waits on the same entitlement a \
         write does: {order}"
    );
    let accepted = capture(
        pool,
        &TOKEN_A,
        serde_json::json!({
            "snapshots": [wire_snapshot(MAPPING_A, "sales_count", 7_000, 42.0)],
        }),
    )
    .await;
    assert_eq!(accepted["written"], 0, "and nothing is stored: {accepted}");
}

/// One tenant's device is served none of another's listings, and cannot write
/// against one either.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_device_reaches_only_its_own_tenants_listings(pool: PgPool) {
    provision(&pool).await;
    seed_device(&pool, ORG_A, "connected").await;
    seed_device(&pool, ORG_B, "connected").await;

    let order = reads(pool.clone(), &TOKEN_A).await;
    let listings = order["listings"].as_array().expect("listings is a list");
    assert_eq!(listings.len(), 1, "one tenant, one listing: {order}");
    assert_eq!(
        listings[0]["remote"]["tpt"]["product_id"], 9_001,
        "and it is this tenant's, not the other's: {order}"
    );

    // The device id is the same string in both tenants, which is the case a
    // path-derived organisation would get wrong: the session decides whose
    // device it is, and the body names no organisation at all.
    let accepted = capture(
        pool.clone(),
        &TOKEN_A,
        serde_json::json!({
            "snapshots": [wire_snapshot(MAPPING_B, "sales_count", 7_000, 99.0)],
        }),
    )
    .await;
    assert_eq!(
        accepted["written"], 0,
        "a snapshot naming another tenant's mapping writes nothing. It is dropped rather \
         than refused, because the alternative is a foreign-key violation surfacing as a \
         five-hundred -- a caller's mistake reported as ours: {accepted}"
    );
}

/// One unstorable instant costs its own row and nothing else.
///
/// The snapshots in a capture are independent readings and the writer is a
/// device. Before the skip, `timestamp_to_db` failed on the first out-of-range
/// value, the transaction rolled back, every good row beside it was lost and
/// the device got a fault — a whole capture discarded because one number was
/// wrong. The row is dropped now and the count says how many landed.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_unstorable_instant_costs_its_own_row_and_no_other(pool: PgPool) {
    provision(&pool).await;
    seed_device(&pool, ORG_A, "connected").await;

    let accepted = capture(
        pool.clone(),
        &TOKEN_A,
        serde_json::json!({
            "snapshots": [
                wire_snapshot(MAPPING_A, "sales_count", i64::MAX, 1.0),
                wire_snapshot(MAPPING_A, "resource_views", 7_000, 88.0),
            ],
        }),
    )
    .await;
    assert_eq!(
        accepted["written"], 1,
        "the good row lands and the impossible one does not: {accepted}"
    );

    let summary = summary(pool, &TOKEN_A).await;
    let metrics = &summary.listings[0].metrics;
    assert_eq!(
        metrics.get("resource_views"),
        Some(&88.0),
        "the reading beside it survived, which is the whole point of dropping per row"
    );
    assert_ne!(
        metrics.get("sales_count"),
        Some(&1.0),
        "and the unstorable one is simply absent rather than recorded at some other instant"
    );
}
