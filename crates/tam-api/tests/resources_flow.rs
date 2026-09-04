//! Catalogue, connections and reconciliation over the wire: the product
//! page and aggregate view, the connections listing with its transport
//! badge, revocation as a write on the connection row, and the founder's
//! drain workflow driven entirely through the API.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resources::{
    ConnectionsView, DecisionsView, ProductView, ProductsPage, QueueView, RevokedView, StatsView,
};
use tam_api::{router, APIError, AppState, Config, SESSION_COOKIE};
use tam_domain::equivalence::{Election, ElectionTrigger, Loss, PricingBranch};
use tam_domain::{
    Binding, CanonicalTerm, EdgeKind, FieldPolicies, FieldPolicy, Mapping, PublishMode, TermKind,
    VocabularyId, VocabularyPath,
};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{
    ElectionRepo, LossScope, MappingRepo, ProductRepo, RaiseScope, SessionRepo, SessionToken,
    TaxonomyRepo,
};
use tam_types::{
    AttemptId, CanonicalTermId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole,
    InventoryId, ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Timestamp, Title, TransportClass, UserId, Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x41; 32]);
const NOW: Timestamp = Timestamp(5_000);
const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
const TERM: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const TERM_2: CanonicalTermId = CanonicalTermId(Uuid([0x78; 16]));

fn state_with(pool: PgPool, config: Config) -> AppState {
    AppState {
        pool,
        config,
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
async fn provision(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG, USER, "a@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    sessions
        .mint(&TOKEN, USER, Timestamp(100_000), Timestamp(1_000))
        .await
        .expect("the session mints");
    ProductRepo::new(pool.clone())
        .insert(
            ORG,
            &tam_domain::CanonicalProduct {
                id: PRODUCT,
                org: ORG,
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
                        bytes: FileBytes::Held {
                            hash: ContentHash([0x51; 32]),
                            byte_len: 4,
                            scan: ScanOutcome::Pending,
                        },
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
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &Mapping {
                id: MAPPING,
                org: ORG,
                product: PRODUCT,
                inventory: InventoryId::TesNz,
                binding: Binding::Unbound,
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
        .expect("the mapping inserts");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: PgPool,
    config: Config,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
    );
    let request = match body {
        Some(json) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            request.body(Body::from(json.to_string()))
        }
        None => request.body(Body::empty()),
    }
    .expect("the request builds");
    let response = router(state_with(pool, config))
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

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_catalogue_lists_and_the_aggregate_reads_back(pool: PgPool) {
    provision(&pool).await;
    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/products",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page: ProductsPage = parse(&body);
    assert_eq!(page.products.len(), 1, "the fixture product lists");
    assert_eq!(page.products[0].title, "Fixture product");
    assert_eq!(page.next_cursor, None, "a short page mints no cursor");

    let path = format!("/v1/products/{}", PRODUCT.0.to_hyphenated());
    let (status, body) = call(pool, Config::default(), Method::GET, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let view: ProductView = parse(&body);
    assert_eq!(view.files.len(), 1, "the payload file reads back");
    assert_eq!(view.files[0].role, "payload");
    assert_eq!(view.grades.source, "seller");
}

/// What this proves ends at the route: the connection reads back revoked, a
/// second revocation moves nothing, and exactly one audit row is written.
///
/// It deliberately does not assert that a check-in cannot lift a revoked
/// connection. That belongs to the beat route and is proved through it by
/// `a_check_in_never_lifts_a_revoked_connection` in `devices_flow.rs`. An
/// assertion here could only restate the guard's SQL by hand, which would stay
/// green if the real statement were loosened — worse than no assertion, because
/// it reads as coverage.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_listed_connection_carries_its_transport_badge_and_revoking_it_is_terminal(pool: PgPool) {
    provision(&pool).await;
    let connection = Uuid([0x33; 16]);
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(connection.0))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    tx.commit().await.expect("the connection commits");

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/connections",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: ConnectionsView = parse(&body);
    assert_eq!(
        (view.connections.len(), view.connections[0].state.as_str()),
        (1, "linked"),
        "the linked connection lists"
    );
    assert_eq!(
        view.connections[0].transport,
        TransportClass::SellerDevice,
        "Tes publishes no official API, so its row carries the seller-device badge"
    );

    // The revocation is a write on the row, with no socket anywhere.
    let path = format!("/v1/connections/{}/revoke", connection.to_hyphenated());
    let (status, body) = call(pool.clone(), Config::default(), Method::POST, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let revoked: RevokedView = parse(&body);
    assert_eq!(revoked.connections, 1, "the row moved, so the count is one");

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/connections",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: ConnectionsView = parse(&body);
    assert_eq!(
        view.connections[0].state.as_str(),
        "revoked",
        "the connection reads revoked afterwards"
    );

    // Revoking again moves nothing, which is what stops a second audit row.
    let (status, body) = call(pool.clone(), Config::default(), Method::POST, &path, None).await;
    assert_eq!(status, StatusCode::OK);
    let again: RevokedView = parse(&body);
    assert_eq!(again.connections, 0, "a second revocation moves no row");

    // One audit row, not two. The count above is derived from the same boolean
    // that gates the audit write, so it cannot tell "no second row" from "the
    // write is unconditional and the count happens to be right"; this asks the
    // table instead.
    let mut tx = pool.begin().await.expect("tx begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the pin applies");
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM connection_audit \
         WHERE org_id = $1 AND connection_id = $2 AND event = 'revoked'",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(connection.0))
    .fetch_one(&mut *tx)
    .await
    .expect("the audit count reads");
    tx.commit().await.expect("the transaction commits");
    assert_eq!(
        events, 1,
        "two revocations leave one audit row: the second moved nothing, so it recorded nothing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_drain_workflow_runs_entirely_through_the_api(pool: PgPool) {
    provision(&pool).await;
    let taxonomy = TaxonomyRepo::new(pool.clone());
    taxonomy
        .seed(
            &[
                CanonicalTerm {
                    id: TERM,
                    kind: TermKind::Topic,
                    parent: None,
                    label: "Orphan topic".to_owned(),
                },
                CanonicalTerm {
                    id: TERM_2,
                    kind: TermKind::Topic,
                    parent: None,
                    label: "Second orphan".to_owned(),
                },
            ],
            &[],
        )
        .await
        .expect("the terms seed");
    taxonomy
        .raise(
            ORG,
            RaiseScope {
                mapping: MAPPING,
                target: InventoryId::TesNz,
                at: Timestamp(2_000),
            },
            &[(TERM, TermKind::Topic), (TERM_2, TermKind::Topic)],
        )
        .await
        .expect("the items raise");

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/reconciliation/items",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let queue: QueueView = parse(&body);
    assert_eq!(queue.items.len(), 2, "both gaps are visible");

    let first = queue
        .items
        .iter()
        .find(|item| item.term == TERM)
        .expect("the first term's item is in the queue");
    let resolve_path = format!(
        "/v1/reconciliation/items/{}/resolve",
        first.id.to_hyphenated()
    );
    // The same bound the override route applies, on the same shared check: a
    // path a client names is durable, and an array nobody bounds is the
    // caller's to fill.
    for oversized in [
        serde_json::json!({ "segments": (0..9).map(|n| format!("level {n}")).collect::<Vec<_>>() }),
        serde_json::json!({ "segments": ["x".repeat(201)] }),
    ] {
        let (status, _body) = call(
            pool.clone(),
            Config::default(),
            Method::POST,
            &resolve_path,
            Some(oversized),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "a path deeper or longer than any vocabulary publishes is refused"
        );
    }

    let (status, _body) = call(
        pool.clone(),
        Config::default(),
        Method::POST,
        &resolve_path,
        Some(serde_json::json!({ "segments": ["Mathematics", "A founder-authored twin"] })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the founder resolves once");

    let second = queue
        .items
        .iter()
        .find(|item| item.term == TERM_2)
        .expect("the second term's item is in the queue");
    let nc_path = format!(
        "/v1/reconciliation/items/{}/no-counterpart",
        second.id.to_hyphenated()
    );
    let (status, _body) = call(
        pool.clone(),
        Config::default(),
        Method::POST,
        &nc_path,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the omission records");

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/reconciliation/stats",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stats: StatsView = parse(&body);
    assert_eq!(
        (stats.open, stats.resolved, stats.no_counterpart),
        (0, 1, 1),
        "the drain counters reflect both resolutions"
    );

    let edges = taxonomy
        .edges_into(VocabularyId(InventoryId::TesNz, TermKind::Topic))
        .await
        .expect("the edges load");
    assert_eq!(
        (edges.len(), edges[0].kind),
        (1, EdgeKind::Exact),
        "the resolution wrote the durable edge the next product finds waiting"
    );
}

/// The decision surface exists to show the seller what this listing gives up
/// whatever they pick, and until the projection recorded a loss its `losses`
/// column read a table nothing wrote. This drives the whole route: a raised
/// question, a recorded loss, and both read back off the surface.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_decision_surface_shows_the_loss_recorded_against_the_mapping(pool: PgPool) {
    provision(&pool).await;
    let grant = VocabularyPath {
        vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Licence),
        segments: vec!["CC-BY".to_owned()],
        native_id: Some("CC-BY".to_owned()),
    };
    ElectionRepo::new(pool.clone())
        .raise(
            ORG,
            MAPPING,
            &[Election {
                product: PRODUCT,
                inventory: InventoryId::TesNz,
                axis: TermKind::Licence,
                trigger: ElectionTrigger::Supply {
                    pricing: PricingBranch::Free,
                },
            }],
            NOW,
        )
        .await
        .expect("the question raises");
    MappingRepo::new(pool.clone())
        .record_losses(
            LossScope {
                org: ORG,
                mapping: MAPPING,
                attempt: AttemptId(Uuid([0x7A; 16])),
                at: NOW,
            },
            &[Loss::NoTargetField {
                axis: TermKind::Licence,
                value: grant,
            }],
        )
        .await
        .expect("the loss records");

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/elections/items",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: DecisionsView = parse(&body);
    let [decision] = view.items.as_slice() else {
        panic!("one open question, got {:?}", view.items);
    };
    assert_eq!(
        decision.resolution, "seller_decides",
        "choosing a rights grant is issuing one, so no opt-in delegates it"
    );
    let [loss] = decision.losses.as_slice() else {
        panic!("one loss, got {:?}", decision.losses);
    };
    assert_eq!(
        (loss.kind.as_str(), loss.axis),
        ("no_target_field", Some(TermKind::Licence)),
        "the seller sees the licence drop at the moment they decide rather than after publish"
    );
    assert_eq!(loss.detail["value"]["native_id"], "CC-BY");
}

/// M3. The reconciliation queue's own resolution is the only production path
/// that authors a TPT taxonomy edge, and the answer shape makes the native id
/// optional: the shipped client sends none, so an ordinary resolution of a
/// TPT tag-axis item used to write an edge addressed by nothing at all.
/// `projection_edge` carries no organisation and both uniqueness indexes
/// refuse a corrected row, so that edge is global and permanent, and every
/// tenant's TPT cross-listing of the term then refuses at the write model
/// with nobody having been told. The refusal moves to the moment of writing.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_tpt_tag_axis_resolution_without_an_identifier_is_refused_rather_than_written(
    pool: PgPool,
) {
    provision(&pool).await;
    let taxonomy = TaxonomyRepo::new(pool.clone());
    taxonomy
        .seed(
            &[CanonicalTerm {
                id: TERM,
                kind: TermKind::Topic,
                parent: None,
                label: "Fractions".to_owned(),
            }],
            &[],
        )
        .await
        .expect("the term seeds");
    taxonomy
        .raise(
            ORG,
            RaiseScope {
                mapping: MAPPING,
                target: InventoryId::Tpt,
                at: Timestamp(2_000),
            },
            &[(TERM, TermKind::Topic)],
        )
        .await
        .expect("the gap raises");

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::GET,
        "/v1/reconciliation/items",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let queue: QueueView = parse(&body);
    let item = queue.items.first().expect("the TPT gap is in the queue");
    let path = format!(
        "/v1/reconciliation/items/{}/resolve",
        item.id.to_hyphenated()
    );

    let (status, body) = call(
        pool.clone(),
        Config::default(),
        Method::POST,
        &path,
        Some(serde_json::json!({ "segments": ["Math", "Fractions"] })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an edge TPT cannot answer to is the request's defect, not the server's"
    );
    let error: APIError = parse(&body);
    let entry = error.errors.first().expect("the refusal names itself");
    assert_eq!(entry.kind, Some(tam_api::APIErrorKind::Validation));
    assert!(
        entry.message.contains("Tpt")
            && entry.message.contains("Topic")
            && entry.message.contains(&TERM.0.to_hyphenated())
            && entry.message.contains("no identifier at all"),
        "the seller is told the axis, the term and what is missing: {}",
        entry.message
    );
    assert!(
        taxonomy
            .edges_into(VocabularyId(InventoryId::Tpt, TermKind::Topic))
            .await
            .expect("the edges load")
            .is_empty(),
        "and nothing durable was written, because the row could never be corrected"
    );

    // The same item, addressed: the guard is about provenance, not about
    // resolving TPT items at all.
    let (status, _body) = call(
        pool.clone(),
        Config::default(),
        Method::POST,
        &path,
        Some(serde_json::json!({
            "segments": ["Math", "Fractions"],
            "native_id": "fractions"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "a TPT slug resolves");
    let edges = taxonomy
        .edges_into(VocabularyId(InventoryId::Tpt, TermKind::Topic))
        .await
        .expect("the edges load");
    assert_eq!(
        edges.first().and_then(|edge| edge.to.native_id.as_deref()),
        Some("fractions"),
        "the slug TPT issues is what the edge carries"
    );
}
