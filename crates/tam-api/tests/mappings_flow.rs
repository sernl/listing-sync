//! Adding a marketplace to a product over the wire: what the write answers,
//! what a repeat answers, and the two fences around it.
//!
//! Two tenants throughout, because the interesting refusal is the one a
//! single-tenant fixture cannot express: another organisation's product is
//! not found rather than forbidden, so the endpoint is never an oracle for
//! which identifiers exist.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resources::{MappingHeadView, MappingsView};
use tam_api::{router, APIError, APIErrorCode, AppState, Config, SESSION_COOKIE};
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode, Verification};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{ProductRepo, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, FileId, FileKind, FileRole, ListingCopy, MappingId, OrgId, PayloadSet,
    PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome, Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const PRODUCT_A: ProductId = ProductId(Uuid([0x01; 16]));
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

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_product(pool: &PgPool, org: OrgId, product: ProductId) {
    ProductRepo::new(pool.clone())
        .insert(
            org,
            &tam_domain::CanonicalProduct {
                id: product,
                org,
                title: Title("Fixture product".to_owned()),
                body: ListingCopy {
                    body: "Fixture body.".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: PayloadSet::new(
                    ProductFile {
                        id: FileId(Uuid([0x21; 16])),
                        role: FileRole::Payload,
                        kind: FileKind::Pdf,
                        hash: ContentHash([0x51; 32]),
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
            },
            Timestamp(1_000),
        )
        .await
        .expect("the product inserts");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    token: Option<&SessionToken>,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        );
    }
    let request = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            request.body(Body::from(json.to_string()))
        }
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state(pool))
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
    (status, body)
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).expect("the body parses")
}

fn mapping(
    id: MappingId,
    inventory: tam_types::InventoryId,
    remote: Option<RemoteListingId>,
) -> Mapping {
    Mapping {
        id,
        org: ORG_A,
        product: PRODUCT_A,
        inventory,
        binding: match remote {
            None => Binding::Unbound,
            Some(id) => Binding::Bound {
                id,
                first_seen: NOW,
                verified: Verification::Stale { since: NOW },
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
    }
}

fn add_path(product: ProductId) -> String {
    format!("/v1/products/{}/mappings", product.0.to_hyphenated())
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_add_answers_the_new_mapping_and_the_listing_carries_it(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        &add_path(PRODUCT_A),
        Some(serde_json::json!({ "inventory": "Tpt" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let head: MappingHeadView = parse(&body);
    assert_eq!(head.product, PRODUCT_A);
    assert_eq!(head.inventory, tam_types::InventoryId::Tpt);
    assert_eq!(
        (head.binding_state.as_str(), head.lifecycle_state.as_str()),
        ("unbound", "absent"),
        "the added marketplace starts where a create's own mapping starts"
    );

    let (status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    assert_eq!(status, StatusCode::OK);
    let view: MappingsView = parse(&body);
    assert_eq!(view.mappings.len(), 1);
    assert_eq!(view.mappings[0].id, head.id);
}

/// The listing's page reaches the client on the mapping, derived from the
/// identifier the binding holds. Two marketplaces in one read, because the
/// derivation is per marketplace and a single one would not show that.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bound_mapping_serves_the_page_a_seller_opens(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;

    let repo = tam_storage::MappingRepo::new(pool.clone());
    for (id, inventory, remote) in [
        (
            MappingId(Uuid([0x31; 16])),
            tam_types::InventoryId::Tpt,
            Some(RemoteListingId::Tpt {
                product_id: 17_511_712,
            }),
        ),
        (
            MappingId(Uuid([0x32; 16])),
            tam_types::InventoryId::TesGb,
            Some(RemoteListingId::Tes {
                url: "https://www.tes.com/api/v2/resources/13264370".to_owned(),
            }),
        ),
        (
            MappingId(Uuid([0x33; 16])),
            tam_types::InventoryId::TesUs,
            None,
        ),
    ] {
        repo.insert(ORG_A, &mapping(id, inventory, remote), 0, NOW)
            .await
            .expect("the mapping inserts");
    }

    let (status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    assert_eq!(status, StatusCode::OK);
    let view: MappingsView = parse(&body);
    let url_of = |id: MappingId| -> Option<String> {
        view.mappings
            .iter()
            .find(|head| head.id == id)
            .and_then(|head| head.listing_url.clone())
    };
    assert_eq!(
        url_of(MappingId(Uuid([0x31; 16]))).as_deref(),
        Some("https://www.teacherspayteachers.com/Product/listing-17511712"),
        "a bound TPT listing serves its product page under the constant slug"
    );
    assert_eq!(
        url_of(MappingId(Uuid([0x32; 16]))).as_deref(),
        Some("https://www.tes.com/teaching-resource/-13264370"),
        "the stored Tes API route is normalised to the page rather than served raw"
    );
    assert_eq!(
        url_of(MappingId(Uuid([0x33; 16]))),
        None,
        "an unbound mapping binds no listing, so it serves no page"
    );
}

/// The freshly added mapping is unbound, so the write that mints it answers a
/// null page rather than one derived from nothing.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_added_marketplace_serves_no_page_yet(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;

    let (status, body) = call(
        pool,
        Some(&TOKEN_A),
        Method::POST,
        &add_path(PRODUCT_A),
        Some(serde_json::json!({ "inventory": "Tpt" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let head: MappingHeadView = parse(&body);
    assert_eq!(head.listing_url, None);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_repeated_add_is_refused_by_name_and_mints_nothing(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let body = serde_json::json!({ "inventory": "Tpt" });

    let (status, _first) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        &add_path(PRODUCT_A),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, refusal) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        &add_path(PRODUCT_A),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&refusal);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::MappingAlreadyExists),
        "the repeat names what it refused rather than reading as a fault"
    );

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    let view: MappingsView = parse(&body);
    assert_eq!(
        view.mappings.len(),
        1,
        "the refused repeat leaves exactly the one mapping the first add made"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_tenants_product_is_not_found(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_B),
        Method::POST,
        &add_path(PRODUCT_A),
        Some(serde_json::json!({ "inventory": "Tpt" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::ResourceMissing));

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    let view: MappingsView = parse(&body);
    assert!(
        view.mappings.is_empty(),
        "the refused write left no mapping on the owning tenant's product"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_add_without_a_session_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;

    let (status, _body) = call(
        pool.clone(),
        None,
        Method::POST,
        &add_path(PRODUCT_A),
        Some(serde_json::json!({ "inventory": "Tpt" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    let view: MappingsView = parse(&body);
    assert!(
        view.mappings.is_empty(),
        "an unauthenticated add wrote nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_add_naming_no_such_product_is_not_found(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;

    let (status, body) = call(
        pool,
        Some(&TOKEN_A),
        Method::POST,
        &add_path(ProductId(Uuid([0x09; 16]))),
        Some(serde_json::json!({ "inventory": "Tpt" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::ResourceMissing));
}

fn bind_path(mapping: MappingId) -> String {
    format!("/v1/mappings/{}/bind", mapping.0.to_hyphenated())
}

/// The whole point of the verb: a listing this tree never created becomes the
/// mapping's, with no marketplace contacted on the way.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_pasted_page_binds_the_mapping_and_serves_its_url_back(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let id = MappingId(Uuid([0x31; 16]));
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &mapping(id, tam_types::InventoryId::Tpt, None),
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");

    let (status, body) = call(
        pool,
        Some(&TOKEN_A),
        Method::POST,
        &bind_path(id),
        Some(serde_json::json!({
            "listing_url": "https://www.teacherspayteachers.com/Product/fractions-pack-17511712"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let head: MappingHeadView = parse(&body);
    assert_eq!(head.binding_state, "bound");
    assert_eq!(
        head.listing_url.as_deref(),
        Some("https://www.teacherspayteachers.com/Product/listing-17511712"),
        "the bound mapping serves the page the seller can open, under the canonical slug"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_link_for_another_marketplace_is_refused_by_name(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let id = MappingId(Uuid([0x31; 16]));
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &mapping(id, tam_types::InventoryId::Tpt, None),
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        &bind_path(id),
        Some(serde_json::json!({
            "listing_url": "https://www.tes.com/teaching-resource/pack-13264370"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(
        error.errors[0].code,
        Some(APIErrorCode::ListingUrlUnusable),
        "a Tes page on a TPT row is the right link on the wrong item"
    );

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    let view: MappingsView = parse(&body);
    assert_eq!(view.mappings[0].binding_state, "unbound");
    assert_eq!(view.mappings[0].listing_url, None);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_link_that_is_not_a_listing_page_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let id = MappingId(Uuid([0x31; 16]));
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &mapping(id, tam_types::InventoryId::Tpt, None),
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");

    for pasted in [
        "17511712",
        "https://example.com/Product/x-17511712",
        "https://www.teacherspayteachers.com/Store/seller-17511712",
    ] {
        let (status, body) = call(
            pool.clone(),
            Some(&TOKEN_A),
            Method::POST,
            &bind_path(id),
            Some(serde_json::json!({ "listing_url": pasted })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "not a TPT product page: {pasted}"
        );
        let error: APIError = parse(&body);
        assert_eq!(error.errors[0].code, Some(APIErrorCode::ListingUrlUnusable));
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_mapping_that_already_binds_a_listing_refuses_a_paste(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let id = MappingId(Uuid([0x31; 16]));
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &mapping(
                id,
                tam_types::InventoryId::Tpt,
                Some(RemoteListingId::Tpt {
                    product_id: 17_511_712,
                }),
            ),
            0,
            NOW,
        )
        .await
        .expect("the bound mapping inserts");

    let (status, body) = call(
        pool,
        Some(&TOKEN_A),
        Method::POST,
        &bind_path(id),
        Some(serde_json::json!({
            "listing_url": "https://www.teacherspayteachers.com/Product/other-99999999"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::MappingNotBindable));
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_tenants_mapping_is_not_found(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let id = MappingId(Uuid([0x31; 16]));
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &mapping(id, tam_types::InventoryId::Tpt, None),
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");

    let (status, body) = call(
        pool.clone(),
        Some(&TOKEN_B),
        Method::POST,
        &bind_path(id),
        Some(serde_json::json!({
            "listing_url": "https://www.teacherspayteachers.com/Product/x-17511712"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let error: APIError = parse(&body);
    assert_eq!(error.errors[0].code, Some(APIErrorCode::ResourceMissing));

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, "/v1/mappings", None).await;
    let view: MappingsView = parse(&body);
    assert_eq!(
        view.mappings[0].binding_state, "unbound",
        "the other tenant's refused paste changed nothing here"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bind_without_a_session_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_product(&pool, ORG_A, PRODUCT_A).await;
    let id = MappingId(Uuid([0x31; 16]));
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            ORG_A,
            &mapping(id, tam_types::InventoryId::Tpt, None),
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");

    let (status, _body) = call(
        pool,
        None,
        Method::POST,
        &bind_path(id),
        Some(serde_json::json!({
            "listing_url": "https://www.teacherspayteachers.com/Product/x-17511712"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
