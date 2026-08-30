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
    ContentHash, CopyFormat, FileId, FileKind, FileRole, InventoryId, ListingCopy, MappingId,
    OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome, Timestamp,
    Title, UserId, Uuid,
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
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([seed.wrapping_add(0x10); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([seed; 32]),
                byte_len: 4,
                scan: ScanOutcome::Pending,
            },
            vec![],
        ),
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
