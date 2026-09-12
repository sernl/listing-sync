//! Collections end to end: the set, its order, the plan's allowance, and the
//! four verbs a seller reaches for once they have one.
//!
//! The severity is that every assertion is about something observable to a
//! seller rather than about a row. The order is read back from the detail read
//! and not from the write's own echo; the publish preview answers per member,
//! and the submit mints exactly one job for the whole collection rather than
//! one per resource; the label reaches every member; and the export carries
//! the members and not the resource sitting beside them in the catalogue. The
//! plan's own ceiling is asserted on a second tenant, because Free includes no
//! collections at all and that refusal is the one a seller meets first.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::collections::{
    AddedLabelsView, CollectionPublishAck, CollectionPublishPlanView, CollectionView,
    CollectionsView,
};
use tam_api::migrations::MigrationVerdict;
use tam_api::{router, APIError, AppState, Config, SESSION_COOKIE};
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    Mapping, PublishMode, RightsDeclaration, TermKind, Verification, VocabularyId, VocabularyPath,
};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{LabelRepo, MappingRepo, ProductRepo, SessionRepo, SessionToken};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, ListingCopy,
    MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const FREE_ORG: OrgId = OrgId(Uuid([0xBB; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const FREE_USER: UserId = UserId(Uuid([0x0B; 16]));
const TOKEN: SessionToken = SessionToken([0x41; 32]);
const FREE_TOKEN: SessionToken = SessionToken([0x42; 32]);
const NOW: Timestamp = Timestamp(1_756_000_000_000);
const KEY: &str = "11111111-1111-4111-8111-111111111111";

/// A member with no listing on the target: the publish creates it.
const FRESH: u8 = 0x01;
/// A member already bound on the target: the publish has nothing to do.
const LANDED: u8 = 0x02;
/// In the catalogue and in no collection, which is what the export filter has
/// to leave out.
const OUTSIDE: u8 = 0x03;
/// A member carrying a rights grant of its own, which is the half of the
/// creation check a licence-less member fails.
const GRANTED: u8 = 0x04;

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

fn product(org: OrgId, tag: u8) -> CanonicalProduct {
    CanonicalProduct {
        id: ProductId(Uuid([tag; 16])),
        org,
        title: Title(format!("Fixture {tag}")),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([tag.wrapping_add(0x60); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Held {
                    hash: ContentHash([tag; 32]),
                    byte_len: 4,
                    scan: ScanOutcome::Clean { at: NOW },
                },
            },
            vec![],
        )),
        cover: None,
        previews: vec![],
        subjects: vec![],
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

/// The same resource with the licence Tes declares required already stated on
/// it, so the two members differ in exactly the thing under test.
fn granted(org: OrgId, tag: u8) -> CanonicalProduct {
    CanonicalProduct {
        rights: RightsDeclaration::Declared {
            source: VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Tes, TermKind::Licence),
                segments: vec!["CC-BY".to_owned()],
                native_id: Some("CC-BY".to_owned()),
            },
        },
        ..product(org, tag)
    }
}

fn policies() -> FieldPolicies {
    FieldPolicies {
        title: FieldPolicy::Managed,
        description: FieldPolicy::Managed,
        price: FieldPolicy::Managed,
        taxonomy: FieldPolicy::Managed,
        grades: FieldPolicy::Managed,
        files: FieldPolicy::Managed,
    }
}

fn bound_on_tpt(tag: u8) -> Mapping {
    Mapping {
        id: MappingId(Uuid([tag.wrapping_add(0x50); 16])),
        org: ORG,
        product: ProductId(Uuid([tag; 16])),
        inventory: InventoryId::Tpt,
        binding: Binding::Bound {
            id: RemoteListingId::Tpt { product_id: 9_001 },
            first_seen: NOW,
            verified: Verification::Clean { at: NOW },
        },
        policies: policies(),
        price_rule: PriceRule::Explicit(PriceIntent::Free),
        publish: PublishMode::DryRun,
        lifecycle: RemoteLifecycle::Live { since: NOW },
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn grant(pool: &PgPool, org: OrgId, plan: tam_limits::Plan) {
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            org,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan,
                rung: None,
                granted_by: tam_storage::GrantedBy::Operator,
                grantor_user: None,
                reason: Some("the collections suite's fixture tenant"),
                source_ref: None,
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("the fixture grant seeds");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    for (org, name) in [(ORG, "org-a"), (FREE_ORG, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the org seeds");
    }
    grant(pool, ORG, tam_limits::Plan::Studio).await;
    // Free includes no collections at all, which is the refusal the second
    // test asserts. Granted rather than left absent so the plan is stated.
    grant(pool, FREE_ORG, tam_limits::Plan::Free).await;
    let sessions = SessionRepo::new(pool.clone());
    for (org, user, email, token) in [
        (ORG, USER, "a@example.test", TOKEN),
        (FREE_ORG, FREE_USER, "b@example.test", FREE_TOKEN),
    ] {
        sessions
            .create_user(org, user, email, Timestamp(1_000))
            .await
            .expect("the user provisions");
        sessions
            .mint(
                &token,
                user,
                Timestamp(NOW.0 + 86_400_000),
                Timestamp(NOW.0 - 1_000),
            )
            .await
            .expect("the session mints");
    }
    let products = ProductRepo::new(pool.clone());
    for tag in [FRESH, LANDED, OUTSIDE] {
        products
            .insert(ORG, &product(ORG, tag), Timestamp(1_000))
            .await
            .expect("the product inserts");
    }
    MappingRepo::new(pool.clone())
        .insert(ORG, &bound_on_tpt(LANDED), 0, NOW)
        .await
        .expect("the landed mapping inserts");
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

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn call(
    pool: &PgPool,
    token: &SessionToken,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .header("idempotency-key", KEY);
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

fn hyphenated(tag: u8) -> String {
    uuid::Uuid::from_bytes([tag; 16]).to_string()
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_seller_files_resources_and_acts_on_the_set(pool: PgPool) {
    provision(&pool).await;

    let made = call(
        &pool,
        &TOKEN,
        Method::POST,
        "/v1/collections",
        Some(serde_json::json!({
            "name": "  Autumn unit \n",
            "description": "  Everything for the first half of term.  ",
        })),
    )
    .await;
    assert_eq!(
        made.status,
        StatusCode::CREATED,
        "a collection that lands answers 201: {}",
        made.text()
    );
    let collection: CollectionView = made.json();
    assert_eq!(
        (
            collection.name.as_str(),
            collection.description.as_deref(),
            collection.count
        ),
        (
            "Autumn unit",
            Some("Everything for the first half of term."),
            0
        ),
        "and answers the row as stored: both strings trimmed, and empty of members"
    );
    let id = uuid::Uuid::from_bytes(collection.id.0).to_string();

    // Named in the seller's own order, which is deliberately not the
    // catalogue's: `LANDED` was created after `FRESH`.
    let filed = call(
        &pool,
        &TOKEN,
        Method::PUT,
        &format!("/v1/collections/{id}/members"),
        Some(serde_json::json!({
            "products": [hyphenated(LANDED), hyphenated(FRESH), hyphenated(LANDED)],
        })),
    )
    .await;
    assert_eq!(filed.status, StatusCode::OK, "{}", filed.text());
    let filled: CollectionView = filed.json();
    assert_eq!(
        filled
            .members
            .iter()
            .map(|member| (member.title.as_str(), member.position))
            .collect::<Vec<_>>(),
        vec![("Fixture 2", 0), ("Fixture 1", 1)],
        "the order is the seller's and the position is its index; a resource named twice is \
         one member"
    );
    assert_eq!(
        filled
            .members
            .first()
            .map(|member| member.inventories.clone()),
        Some(vec![InventoryId::Tpt]),
        "and a member carries the marketplaces it is listed on"
    );

    let listed: CollectionsView = call(&pool, &TOKEN, Method::GET, "/v1/collections", None)
        .await
        .json();
    assert_eq!(
        listed
            .collections
            .iter()
            .map(|head| (head.name.as_str(), head.count, head.inventories.clone()))
            .collect::<Vec<_>>(),
        vec![("Autumn unit", 2, vec![InventoryId::Tpt])],
        "the list row carries the count and the marks without a read per collection"
    );

    let holding: CollectionsView = call(
        &pool,
        &TOKEN,
        Method::GET,
        &format!("/v1/products/{}/collections", hyphenated(FRESH)),
        None,
    )
    .await
    .json();
    assert_eq!(
        holding
            .collections
            .iter()
            .map(|head| head.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Autumn unit"],
        "and the resource page reads which collections hold it"
    );

    let previewed = call(
        &pool,
        &TOKEN,
        Method::POST,
        &format!("/v1/collections/{id}/publish/plan"),
        Some(serde_json::json!({ "inventory": "Tpt", "intent": "draft" })),
    )
    .await;
    assert_eq!(previewed.status, StatusCode::OK, "{}", previewed.text());
    let plan: CollectionPublishPlanView = previewed.json();
    assert_eq!(
        plan.rows
            .iter()
            .map(|row| (row.title.as_str(), row.verdict))
            .collect::<Vec<_>>(),
        vec![
            ("Fixture 2", MigrationVerdict::AlreadyThere),
            ("Fixture 1", MigrationVerdict::WillCreate),
        ],
        "the preview answers per member, in the collection's own order: {plan:?}"
    );
    assert_eq!(
        (plan.counts.will_create, plan.counts.already_there),
        (1, 1),
        "and the counts are the rows"
    );

    let queued = call(
        &pool,
        &TOKEN,
        Method::POST,
        &format!("/v1/collections/{id}/publish"),
        Some(serde_json::json!({ "inventory": "Tpt", "intent": "draft" })),
    )
    .await;
    assert_eq!(queued.status, StatusCode::OK, "{}", queued.text());
    let ack: CollectionPublishAck = queued.json();
    assert_eq!(
        (ack.queued, ack.skipped),
        (1, 1),
        "the submit queues what the plan admitted and skips the rest"
    );
    assert!(
        ack.job.is_some(),
        "on one job for the whole collection rather than one per resource: {ack:?}"
    );

    // The second press re-plans rather than trusting the first answer, and the
    // plan now knows the create is on its way: nothing is queued and no
    // second job is minted, which is the property that matters — a seller who
    // presses twice does not send the same create twice.
    let again: CollectionPublishPlanView = call(
        &pool,
        &TOKEN,
        Method::POST,
        &format!("/v1/collections/{id}/publish/plan"),
        Some(serde_json::json!({ "inventory": "Tpt", "intent": "draft" })),
    )
    .await
    .json();
    assert_eq!(
        again
            .rows
            .iter()
            .find(|row| row.title == "Fixture 1")
            .and_then(|row| row.reason.clone()),
        Some(
            "already on its way to TPT; it will show as there once your device has sent it"
                .to_owned()
        ),
        "the second preview says so in the seller's own words: {again:?}"
    );
    let replayed: CollectionPublishAck = call(
        &pool,
        &TOKEN,
        Method::POST,
        &format!("/v1/collections/{id}/publish"),
        Some(serde_json::json!({ "inventory": "Tpt", "intent": "draft" })),
    )
    .await
    .json();
    assert_eq!(
        (replayed.job, replayed.queued),
        (None, 0),
        "and the second press mints nothing: {replayed:?}"
    );

    let labelled = call(
        &pool,
        &TOKEN,
        Method::PUT,
        &format!("/v1/collections/{id}/labels"),
        Some(serde_json::json!({ "add": ["  Autumn  "] })),
    )
    .await;
    assert_eq!(labelled.status, StatusCode::OK, "{}", labelled.text());
    let added: AddedLabelsView = labelled.json();
    assert_eq!(
        (added.added.as_slice(), added.members),
        (["Autumn".to_owned()].as_slice(), 2),
        "the label is stored trimmed and reaches every member"
    );
    let labels = LabelRepo::new(pool.clone());
    for tag in [FRESH, LANDED] {
        let carried = labels
            .for_product(ORG, ProductId(Uuid([tag; 16])))
            .await
            .unwrap_or_default();
        assert!(
            carried.iter().any(|record| record.name == "Autumn"),
            "member {tag} carries it: {carried:?}"
        );
    }
    let outsider = labels
        .for_product(ORG, ProductId(Uuid([OUTSIDE; 16])))
        .await
        .unwrap_or_default();
    assert!(
        outsider.is_empty(),
        "and the resource outside the collection does not: {outsider:?}"
    );

    let exported = call(
        &pool,
        &TOKEN,
        Method::GET,
        &format!("/v1/products/export?collection={id}"),
        None,
    )
    .await;
    assert_eq!(exported.status, StatusCode::OK, "{}", exported.text());
    let csv = exported.text();
    assert!(
        csv.contains("Fixture 1") && csv.contains("Fixture 2"),
        "the document carries the members: {csv}"
    );
    assert!(
        !csv.contains("Fixture 3"),
        "and not the resource sitting beside them in the catalogue: {csv}"
    );

    let whole = call(&pool, &TOKEN, Method::GET, "/v1/products/export", None)
        .await
        .text();
    assert!(
        whole.contains("Fixture 3"),
        "while the unnarrowed export is still the whole catalogue: {whole}"
    );

    let removed = call(
        &pool,
        &TOKEN,
        Method::DELETE,
        &format!("/v1/collections/{id}"),
        None,
    )
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let after = ProductRepo::new(pool.clone())
        .list(ORG)
        .await
        .unwrap_or_default();
    assert_eq!(
        after.len(),
        3,
        "removing a collection removes no resource: it is a way of looking at the catalogue \
         and not part of it"
    );
}

/// The plan's own allowance, on the plan that includes none. Asserted as the
/// refusal a seller reads rather than as an accepted value, so a gate that was
/// deleted fails here.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_plan_that_includes_no_collections_refuses_the_first_one(pool: PgPool) {
    provision(&pool).await;
    let refused = call(
        &pool,
        &FREE_TOKEN,
        Method::POST,
        "/v1/collections",
        Some(serde_json::json!({ "name": "Autumn unit" })),
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a bound the plan applies is a refusal a seller can act on: {}",
        refused.text()
    );
    let error: APIError = refused.json();
    let entry = error
        .errors
        .first()
        .unwrap_or_else(|| panic!("the refusal carries an entry"));
    assert_eq!(
        entry.message, "Your plan does not include collections. Upgrade to use them.",
        "and says what the plan includes and the one thing they can do about it"
    );
    assert_eq!(
        entry
            .detail
            .as_ref()
            .and_then(|detail| detail.get("quota"))
            .and_then(serde_json::Value::as_str),
        Some("collections_max"),
        "carrying the bound's own name, so the console branches on it rather than on the \
         sentence"
    );
    let listed: CollectionsView = call(&pool, &FREE_TOKEN, Method::GET, "/v1/collections", None)
        .await
        .json();
    assert!(
        listed.collections.is_empty(),
        "and nothing was written: {listed:?}"
    );
}

/// A member the target would refuse blocks its own row and nothing else.
///
/// Tes declares its licence required, and a resource imported from a
/// marketplace that carries no licence on its wire reaches the catalogue with
/// its rights unstated. Until the creation check stood, a collection
/// published to Tes admitted every member holding a file, minted each one a
/// mapping and queued each one a create, and the seller learnt which of them
/// could never land from the items that failed at the marketplace.
///
/// The two members differ in the rights grant alone, so the eligible one
/// proves this is the requirement talking and not the route.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_member_the_target_would_refuse_blocks_its_row_and_not_the_collection(pool: PgPool) {
    provision(&pool).await;
    ProductRepo::new(pool.clone())
        .insert(ORG, &granted(ORG, GRANTED), Timestamp(1_000))
        .await
        .expect("the granted product inserts");

    let made = call(
        &pool,
        &TOKEN,
        Method::POST,
        "/v1/collections",
        Some(serde_json::json!({ "name": "Cross-list set" })),
    )
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text());
    let collection: CollectionView = made.json();
    let id = uuid::Uuid::from_bytes(collection.id.0).to_string();

    let filed = call(
        &pool,
        &TOKEN,
        Method::PUT,
        &format!("/v1/collections/{id}/members"),
        Some(serde_json::json!({
            "products": [hyphenated(FRESH), hyphenated(GRANTED)],
        })),
    )
    .await;
    assert_eq!(filed.status, StatusCode::OK, "{}", filed.text());

    let previewed = call(
        &pool,
        &TOKEN,
        Method::POST,
        &format!("/v1/collections/{id}/publish/plan"),
        Some(serde_json::json!({ "inventory": "Tes", "intent": "draft" })),
    )
    .await;
    assert_eq!(previewed.status, StatusCode::OK, "{}", previewed.text());
    let plan: CollectionPublishPlanView = previewed.json();
    assert_eq!(
        plan.rows
            .iter()
            .map(|row| (row.title.as_str(), row.verdict))
            .collect::<Vec<_>>(),
        vec![
            ("Fixture 1", MigrationVerdict::Blocked),
            ("Fixture 4", MigrationVerdict::WillCreate),
        ],
        "the member with no licence is blocked and the one carrying a grant is not: {plan:?}"
    );
    assert_eq!(
        (plan.counts.blocked, plan.counts.will_create),
        (1, 1),
        "and the blocked member is counted as blocked rather than as one that will be created"
    );
    assert!(
        plan.rows
            .first()
            .and_then(|row| row.reason.as_deref())
            .is_some_and(|reason| reason.contains("Licence")),
        "the row says which field the marketplace wants, which is the only thing the seller \
         can act on: {plan:?}"
    );

    let queued = call(
        &pool,
        &TOKEN,
        Method::POST,
        &format!("/v1/collections/{id}/publish"),
        Some(serde_json::json!({ "inventory": "Tes", "intent": "draft" })),
    )
    .await;
    assert_eq!(queued.status, StatusCode::OK, "{}", queued.text());
    let ack: CollectionPublishAck = queued.json();
    assert_eq!(
        (ack.queued, ack.skipped),
        (1, 1),
        "the confirm re-plans, so the blocked member is skipped there too rather than queued \
         behind a preview that said no: {ack:?}"
    );

    // And nothing was minted for it: a mapping written for a resource that
    // can never land is the artefact this check exists to avoid.
    let mappings = MappingRepo::new(pool.clone())
        .list_for_product(ORG, ProductId(Uuid([FRESH; 16])))
        .await
        .expect("the mappings read back");
    assert!(
        mappings
            .iter()
            .all(|record| record.mapping.inventory != InventoryId::Tes),
        "the blocked member has no Tes mapping: {mappings:?}"
    );
}
