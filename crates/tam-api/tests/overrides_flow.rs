//! The seller's own projection overrides over the wire: what the route records,
//! what it refuses before writing, and the tenant fence around the listing.
//!
//! The refusals are the interesting half. Two of them exist because a value
//! that reached the database would be wrong in a way nothing downstream could
//! correct: a licence override the CHECK would reject anyway, and a native
//! identifier of a shape the marketplace does not issue, which would be
//! discovered only when a listing carrying it was refused.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resources::OverridesView;
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::{CanonicalTerm, TermKind};
use tam_storage::{SessionRepo, SessionToken, TaxonomyRepo};
use tam_types::{CanonicalTermId, FileBytes, OrgId, Timestamp, UserId, Uuid};
use tower::ServiceExt;

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const USER_A: UserId = UserId(Uuid([0x0A; 16]));
const USER_B: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN_A: SessionToken = SessionToken([0x41; 32]);
const TOKEN_B: SessionToken = SessionToken([0x42; 32]);
const TERM: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
/// A grade term, so a TPT projection has the phase axis it needs and the
/// subject axis this file is about is the only one under test.
const GRADE: CanonicalTermId = CanonicalTermId(Uuid([0x79; 16]));
/// What the global relation answers for [`TERM`] on TPT, and what the seller
/// says instead. They must differ, or the assertion cannot tell a decision
/// that won from one that was never consulted.
const BY_RELATION: &str = "maths-from-the-relation";
const BY_SELLER: &str = "maths-the-seller-chose";
const NOW: Timestamp = Timestamp(5_000);

const PATH: &str = "/v1/mappings/overrides";

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

/// The canonical taxonomy an override points out of.
///
/// `projection_override.from_term` carries a foreign key onto `canonical_term`,
/// which is global rather than per tenant, so it is seeded once per test
/// database rather than per organisation.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_term(pool: &PgPool) {
    TaxonomyRepo::new(pool.clone())
        .seed(
            &[CanonicalTerm {
                id: TERM,
                kind: TermKind::Subject,
                parent: None,
                label: "Fractions".to_owned(),
            }],
            &[],
        )
        .await
        .expect("the term seeds");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    token: Option<&SessionToken>,
    method: Method,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(PATH);
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

fn override_body(kind: &str) -> serde_json::Value {
    serde_json::json!({
        "inventory": "Tpt",
        "axis": "subject",
        "from_term": TERM.0.to_hyphenated(),
        "to": { "segments": ["Maths"], "native_id": "math-elementary" },
        "kind": kind,
    })
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_override_records_and_lists_back(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        Some(override_body("broader")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    assert_eq!(status, StatusCode::OK);
    let view: OverridesView = parse(&body);
    assert_eq!(view.overrides.len(), 1);
    assert_eq!(view.overrides[0].from_term, TERM);
    assert_eq!(view.overrides[0].segments, vec!["Maths".to_owned()]);
    assert_eq!(
        view.overrides[0].kind, "broader",
        "the kind the seller chose survives the round trip"
    );
}

/// A licence is a legal statement about the work rather than a mapping choice.
/// The constructor refuses it and the database CHECK refuses it; the seller is
/// told which, rather than meeting a five-hundred from the second.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_licence_override_is_refused_as_validation(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["axis"] = serde_json::json!("licence");
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "the refused override wrote nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_empty_path_is_refused_as_validation(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["to"] = serde_json::json!({ "segments": [] });
    let (status, _body) = call(pool, Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// The check that runs before the write rather than at the listing: TPT
/// addresses its tag namespace by slug, and an identifier of another shape
/// would be discovered only when a listing carrying it was refused.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_native_identifier_of_the_wrong_shape_is_refused_before_the_write(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    for native in ["1234", "Math-Elementary", "math elementary", ""] {
        let mut body = override_body("exact");
        body["to"] = serde_json::json!({ "segments": ["Maths"], "native_id": native });
        let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "not a slug TPT issues: {native:?}"
        );
    }

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "every refusal happened before anything was written"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_withdrawal_removes_one_and_forgives_a_miss(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;
    call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        Some(override_body("exact")),
    )
    .await;

    let withdraw = serde_json::json!({
        "inventory": "Tpt",
        "axis": "subject",
        "from_term": TERM.0.to_hyphenated(),
    });
    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::DELETE,
        Some(withdraw.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_status, body) = call(pool.clone(), Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(view.overrides.is_empty(), "the override is gone");

    let (status, _body) = call(pool, Some(&TOKEN_A), Method::DELETE, Some(withdraw)).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "withdrawing what is already absent is the state the seller asked for, not an error"
    );
}

/// The organisation comes from the session and from nowhere else, so one
/// tenant's overrides are invisible to another and neither can write the
/// other's.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn one_tenants_overrides_are_invisible_to_another(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;

    call(
        pool.clone(),
        Some(&TOKEN_A),
        Method::POST,
        Some(override_body("exact")),
    )
    .await;

    let (status, body) = call(pool.clone(), Some(&TOKEN_B), Method::GET, None).await;
    assert_eq!(status, StatusCode::OK);
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "tenant b sees none of tenant a's overrides"
    );

    // The same term, from the other tenant: one override each, not one shared.
    let (status, _body) = call(
        pool.clone(),
        Some(&TOKEN_B),
        Method::POST,
        Some(override_body("broader")),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert_eq!(
        view.overrides[0].kind, "exact",
        "tenant b's write did not touch tenant a's answer for the same term"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_surface_is_closed_without_a_session(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    for (method, body) in [
        (Method::POST, Some(override_body("exact"))),
        (Method::GET, None),
        (
            Method::DELETE,
            Some(serde_json::json!({
                "inventory": "Tpt",
                "axis": "subject",
                "from_term": TERM.0.to_hyphenated(),
            })),
        ),
    ] {
        let (status, _body) = call(pool.clone(), None, method.clone(), body).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} without a session"
        );
    }
}

/// The bound on what a stranger may make this server write. The domain refuses
/// an empty path because it names nothing; this refuses an enormous one because
/// nobody's vocabulary is that deep and the array is otherwise the caller's to
/// fill.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_oversized_path_is_refused_before_the_write(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut deep = override_body("exact");
    deep["to"] = serde_json::json!({
        "segments": (0..9).map(|n| format!("level {n}")).collect::<Vec<_>>(),
        "native_id": "math-elementary",
    });
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(deep)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "nine segments is deeper than any marketplace publishes"
    );

    let mut long = override_body("exact");
    long["to"] = serde_json::json!({
        "segments": ["x".repeat(201)],
        "native_id": "math-elementary",
    });
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(long)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "one segment longer than two hundred characters"
    );

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "both refusals happened before anything was written"
    );
}

/// A term the taxonomy does not hold reaches the foreign key, and the seller is
/// told to pick again rather than shown a fault of ours. Reachable from an
/// ordinary client: the screen picks from a cached list, so a tab open across a
/// taxonomy change can name a term that has since gone.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_term_the_taxonomy_does_not_hold_is_refused_rather_than_faulting(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["from_term"] = serde_json::json!(Uuid([0x66; 16]).to_hyphenated());
    let (status, _body) = call(pool, Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unknown term is the caller's to correct, not a five-hundred"
    );
}

/// An axis the marketplace does not bind is refused by the constructor, not by
/// the database: Etsy carries no equivalence axis at all, so an override for
/// one would be a durable answer to a question that marketplace never asks.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_axis_the_marketplace_does_not_bind_is_refused(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    seed_term(&pool).await;

    let mut body = override_body("exact");
    body["inventory"] = serde_json::json!("Etsy");
    let (status, _body) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_status, body) = call(pool, Some(&TOKEN_A), Method::GET, None).await;
    let view: OverridesView = parse(&body);
    assert!(
        view.overrides.is_empty(),
        "the refusal happened before anything was written"
    );
}

/// One canonical term's counterpart in one inventory's vocabulary.
fn path(
    inventory: tam_types::InventoryId,
    kind: TermKind,
    label: &str,
    native: &str,
) -> tam_domain::VocabularyPath {
    tam_domain::VocabularyPath {
        vocabulary: tam_domain::VocabularyId(inventory, kind),
        segments: vec![label.to_owned()],
        native_id: Some(native.to_owned()),
    }
}

fn crosswalk_edge(
    from: CanonicalTermId,
    to: tam_domain::VocabularyPath,
) -> tam_domain::ProjectionEdge {
    tam_domain::ProjectionEdge {
        from,
        to,
        kind: tam_domain::EdgeKind::Exact,
        decided_by: tam_domain::Decider::Imported {
            source: "overrides_flow fixture".to_owned(),
        },
        decided_at: NOW,
    }
}

/// The global relation, which answers for both terms on TPT.
///
/// The subject edge is the half that matters: it gives the relation a real
/// answer for [`TERM`], so an override is measured against a value rather than
/// against a gap. The grade edges are scaffolding -- seeded on the source side
/// as well, because the product declares its grade as a Tes path and the
/// projection reaches TPT by ingesting that into the canonical term and
/// projecting out again.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_crosswalk(pool: &PgPool) {
    TaxonomyRepo::new(pool.clone())
        .seed(
            &[CanonicalTerm {
                id: GRADE,
                kind: TermKind::Phase,
                parent: None,
                label: "Kindergarten".to_owned(),
            }],
            &[
                crosswalk_edge(
                    TERM,
                    path(
                        tam_types::InventoryId::Tpt,
                        TermKind::Subject,
                        "Maths",
                        BY_RELATION,
                    ),
                ),
                crosswalk_edge(
                    GRADE,
                    path(
                        tam_types::InventoryId::Tpt,
                        TermKind::Phase,
                        "Kindergarten",
                        "tpt-kindergarten",
                    ),
                ),
                crosswalk_edge(
                    GRADE,
                    path(
                        tam_types::InventoryId::Tes,
                        TermKind::Phase,
                        "Kindergarten",
                        "17",
                    ),
                ),
            ],
        )
        .await
        .expect("the crosswalk seeds");
}

/// One tenant's product and TPT mapping, and the lease a projection of it runs
/// under. The seed byte separates the two tenants' rows.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_projectable(pool: &PgPool, org: OrgId, seed: u8) -> tam_storage::LeasedItem {
    let product = tam_types::ProductId(Uuid([seed; 16]));
    let mapping = tam_types::MappingId(Uuid([seed.wrapping_add(1); 16]));
    let declared = path(
        tam_types::InventoryId::Tes,
        TermKind::Phase,
        "Kindergarten",
        "17",
    );
    tam_storage::ProductRepo::new(pool.clone())
        .insert(
            org,
            &tam_domain::CanonicalProduct {
                id: product,
                org,
                title: tam_types::Title("Fractions practice".to_owned()),
                body: tam_types::ListingCopy {
                    body: "A worksheet.".to_owned(),
                    format: tam_types::CopyFormat::Markdown,
                },
                payload: Some(tam_types::PayloadSet::new(
                    tam_types::ProductFile {
                        id: tam_types::FileId(Uuid([seed.wrapping_add(2); 16])),
                        role: tam_types::FileRole::Payload,
                        kind: tam_types::FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: tam_types::ContentHash([seed.wrapping_add(3); 32]),
                            byte_len: 4,
                            scan: tam_types::ScanOutcome::Clean { at: NOW },
                        },
                    },
                    vec![],
                )),
                cover: Some(tam_types::ProductFile {
                    id: tam_types::FileId(Uuid([seed.wrapping_add(4); 16])),
                    role: tam_types::FileRole::Cover,
                    kind: tam_types::FileKind::Image,
                    bytes: FileBytes::Held {
                        hash: tam_types::ContentHash([seed.wrapping_add(5); 32]),
                        byte_len: 4,
                        scan: tam_types::ScanOutcome::Clean { at: NOW },
                    },
                }),
                previews: vec![],
                subjects: vec![TERM],
                grades: tam_domain::GradeDeclaration {
                    source: tam_domain::DeclarationSource::Imported {
                        vocabulary: tam_domain::VocabularyId(
                            tam_types::InventoryId::Tes,
                            TermKind::Phase,
                        ),
                    },
                    raw: vec![declared],
                    derived: Some(tam_domain::AgeInterval::new(5, 7).expect("a bounded range")),
                },
                price: tam_types::PriceIntent::Free,
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            NOW,
        )
        .await
        .expect("the product inserts");
    tam_storage::MappingRepo::new(pool.clone())
        .insert(
            org,
            &tam_domain::Mapping {
                id: mapping,
                org,
                product,
                inventory: tam_types::InventoryId::Tpt,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: tam_types::PriceRule::Explicit(tam_types::PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
    tam_storage::LeasedItem {
        org,
        item: tam_domain::JobItemId(Uuid([seed.wrapping_add(6); 16])),
        job: tam_types::JobId(Uuid([seed.wrapping_add(7); 16])),
        mapping,
        inventory: tam_types::InventoryId::Tpt,
        idempotency_key: tam_marketplace::idempotency::derive_idempotency_key(
            org,
            tam_types::InventoryId::Tpt,
            product,
            1,
            tam_types::ContentHash([seed.wrapping_add(3); 32]),
        ),
        operation: tam_domain::ItemOperation::Create,
        lease_epoch: 0,
        attempt_count: 0,
        requires_bound_on: None,
        stranded_attempt: None,
        stranded_title: None,
    }
}

/// The native identifier a projection of this item carries on the subject
/// axis.
///
/// The pool is the caller's and it must be the `tam_app` one the rest of this
/// file uses. `projection_override` carries forced row-level security and
/// `OverrideRepo::for_org` pins the tenant to read it, so a projection run over
/// a BYPASSRLS engine role would answer correctly even with that pin gone --
/// which is the defect this test exists to catch. Do not reach for an engine
/// pool here.
///
/// Every non-projecting answer is named rather than lumped into one `else`,
/// because which one came back is the whole diagnosis when this stops
/// projecting.
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should stop the run"
)]
async fn projected_subject(pool: &PgPool, leased: &tam_storage::LeasedItem) -> Option<String> {
    match tam_engine::seed::prepare_item(pool, leased, NOW)
        .await
        .expect("the preparation runs")
    {
        tam_engine::seed::ItemPreparation::Ready {
            projected: Some(projected),
            ..
        } => {
            assert_eq!(
                projected.taxonomy.len(),
                1,
                "the product declares one subject, so the projection carries one: {:?}",
                projected.taxonomy
            );
            projected.taxonomy[0].native_id.clone()
        }
        tam_engine::seed::ItemPreparation::Ready {
            projected: None, ..
        } => panic!("ready, but describing nothing: this fixture is a create and creates project"),
        tam_engine::seed::ItemPreparation::Blocked { gate, raised } => {
            panic!("blocked on {gate}, having raised {raised:?}")
        }
        tam_engine::seed::ItemPreparation::CounterpartLost { counterpart } => {
            panic!("waiting on {counterpart:?}, which this fixture has no counterpart for")
        }
    }
}

/// An override authored the way a seller authors one is what a listing then
/// carries.
///
/// The two halves of this were already proven separately and the wire between
/// them was not. `an_override_records_and_lists_back` reads back through the
/// same repository and the same codec that wrote, so a symmetric fault in
/// either is invisible to it; `a_sellers_override_answers_a_gap_for_that_seller_only`
/// in the engine suite writes through `OverrideRepo` directly, so it cannot see
/// anything this route does differently. Nothing until here authored one
/// through the route and then asked a projection what it holds.
///
/// It is measured against a relation that answers rather than against a gap.
/// A gap being filled proves only that something answered where nothing had; a
/// value being replaced proves the seller's decision beat the global relation,
/// which is the property `project_axis_with_overrides` is built around -- and
/// an implementation that folded the override in beside the global edges rather
/// than filtering the term out of the relation would see two paths for one term,
/// answer `Ambiguous`, and block here.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_override_recorded_through_the_route_is_what_a_projection_carries(pool: PgPool) {
    provision(&pool, ORG_A, USER_A, &TOKEN_A, "org-a").await;
    provision(&pool, ORG_B, USER_B, &TOKEN_B, "org-b").await;
    seed_term(&pool).await;
    seed_crosswalk(&pool).await;
    let mine = seed_projectable(&pool, ORG_A, 0x11).await;
    let theirs = seed_projectable(&pool, ORG_B, 0x21).await;

    assert_eq!(
        projected_subject(&pool, &mine).await.as_deref(),
        Some(BY_RELATION),
        "the premise, without which the assertion below cannot tell an override that won \
         from a gap that got filled: the relation has an answer of its own here"
    );

    let mut body = override_body("exact");
    body["to"] = serde_json::json!({ "segments": ["Maths"], "native_id": BY_SELLER });
    let (status, refused) = call(pool.clone(), Some(&TOKEN_A), Method::POST, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the seller's own answer records: {}",
        String::from_utf8_lossy(&refused)
    );

    assert_eq!(
        projected_subject(&pool, &mine).await.as_deref(),
        Some(BY_SELLER),
        "and it is what the listing carries. The relation still holds its own answer and \
         no longer decides, which is what makes an override a decision rather than a \
         second claimant"
    );
    assert_eq!(
        projected_subject(&pool, &theirs).await.as_deref(),
        Some(BY_RELATION),
        "one tenant deciding for a global term decides nothing for another: an override is \
         theirs, and the projection reads it under their own pin"
    );
}
