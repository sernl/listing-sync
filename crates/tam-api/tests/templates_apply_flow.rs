//! Applying a template to resources the catalogue already holds, end to end.
//!
//! The severity is in the three answers and in the write matching them. A
//! template that states a description, a price and a copyright attestation is
//! applied to three resources: one authored blank, which takes all three; one
//! the seller has already priced and described, which takes nothing; and one
//! live on Tes, whose edit-published transition no capture supports and which
//! is therefore refused with the sentence the edit route refuses it with.
//!
//! Then the submit, and what makes it worth testing over the plan: the rows it
//! writes are exactly the rows the plan admitted, the stored values are the
//! template's, and a replay writes nothing — because a field is listed only
//! where the merged value differs from the stored one, which is the property
//! the whole route rests on.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::resource_templates::ResourceTemplateView;
use tam_api::template_apply::{TemplateApplyAck, TemplateApplyPlanView, TemplateApplyVerdict};
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::product::{
    CategoryGroup, CopyrightDeclaration, DetailGroup, ListingStatus, ThumbnailMode,
};
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    Mapping, PublishMode, RightsDeclaration, Verification,
};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{
    MappingRepo, ProductRepo, SessionRepo, SessionToken, TptBaseRecord, TptBaseRepo,
};
use tam_types::{
    ContentHash, CopyFormat, Currency, FileBytes, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const USER: UserId = UserId(Uuid([0x0A; 16]));
const TOKEN: SessionToken = SessionToken([0x41; 32]);
const NOW: Timestamp = Timestamp(1_756_000_000_000);
const KEY: &str = "11111111-1111-4111-8111-111111111111";

/// Authored blank: free, no description, no sidecar. Takes everything.
const BLANK: u8 = 0x01;
/// The seller's own: described, priced, and attested. Takes nothing.
const ANSWERED: u8 = 0x02;
/// Live on Tes, whose edit-published transition is uncaptured.
const ON_TES: u8 = 0x03;

fn state(pool: PgPool) -> AppState {
    AppState {
        exchange_rates: None,
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
fn paid(minor: i64) -> PriceIntent {
    PriceIntent::Paid(Money::new(minor, Currency::Usd).expect("a positive amount"))
}

fn product(tag: u8, body: &str, price: PriceIntent) -> CanonicalProduct {
    CanonicalProduct {
        id: ProductId(Uuid([tag; 16])),
        org: ORG,
        title: Title(format!("Fixture {tag}")),
        body: ListingCopy {
            body: body.to_owned(),
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
        price,
        rights: RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

/// A sidecar holding one answer and nothing else, which is what makes the
/// answered resource's row `unchanged` rather than a partial fill.
fn attested(copyright: CopyrightDeclaration) -> TptBaseRecord {
    TptBaseRecord {
        thumbnail_mode: ThumbnailMode::AutoGenerate,
        thumbnails: vec![],
        video_preview: None,
        additional_licence_minor_units: None,
        bundle_discount_minor_units: None,
        tax_code: None,
        categories: CategoryGroup {
            grades: vec![],
            subject_areas: vec![],
            tags: vec![],
            formats: vec![],
            custom_categories: vec![],
            appropriate_for_country: None,
        },
        standards: vec![],
        details: DetailGroup {
            teaching_duration: None,
            pages_or_slides: None,
            answer_key: None,
        },
        copyright: Some(copyright),
        status: ListingStatus::Draft,
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

/// Bound and live, which is the pair that makes the edit uncapturable on Tes.
fn live_on_tes(tag: u8) -> Mapping {
    Mapping {
        id: MappingId(Uuid([tag.wrapping_add(0x50); 16])),
        org: ORG,
        product: ProductId(Uuid([tag; 16])),
        inventory: InventoryId::Tes,
        binding: Binding::Bound {
            id: RemoteListingId::Tes {
                url: format!("https://www.tes.com/teaching-resource/fixture-{tag}"),
            },
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
async fn provision(pool: &PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            ORG,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Studio,
                rung: None,
                granted_by: tam_storage::GrantedBy::Operator,
                grantor_user: None,
                reason: Some("the template apply suite's fixture tenant"),
                source_ref: None,
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("the fixture grant seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(ORG, USER, "a@example.test", Timestamp(1_000))
        .await
        .expect("the user provisions");
    sessions
        .mint(
            &TOKEN,
            USER,
            Timestamp(NOW.0 + 86_400_000),
            Timestamp(NOW.0 - 1_000),
        )
        .await
        .expect("the session mints");

    let products = ProductRepo::new(pool.clone());
    products
        .insert(
            ORG,
            &product(BLANK, "", PriceIntent::Free),
            Timestamp(1_000),
        )
        .await
        .expect("the blank resource inserts");
    products
        .insert(
            ORG,
            &product(ANSWERED, "Its own description.", paid(1_200)),
            Timestamp(1_000),
        )
        .await
        .expect("the answered resource inserts");
    products
        .insert(
            ORG,
            &product(ON_TES, "", PriceIntent::Free),
            Timestamp(1_000),
        )
        .await
        .expect("the Tes resource inserts");
    TptBaseRepo::new(pool.clone())
        .upsert(
            ORG,
            ProductId(Uuid([ANSWERED; 16])),
            &attested(CopyrightDeclaration::UsedCopyrightedMaterials),
            NOW,
        )
        .await
        .expect("the answered resource's sidecar inserts");
    MappingRepo::new(pool.clone())
        .insert(ORG, &live_on_tes(ON_TES), 0, NOW)
        .await
        .expect("the Tes mapping inserts");
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
    body: Option<serde_json::Value>,
) -> Answer {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
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

/// A template that states one field of each kind the merge has a rule for: a
/// description on the product, a price on the product, and an attestation in
/// the sidecar.
fn draft() -> serde_json::Value {
    serde_json::json!({
        "description": "Differentiated three ways.",
        "price_minor_units": 450,
        "copyright_declaration_id": 1,
    })
}

fn ids() -> (String, String, String) {
    (
        uuid::Uuid::from_bytes([BLANK; 16]).to_string(),
        uuid::Uuid::from_bytes([ANSWERED; 16]).to_string(),
        uuid::Uuid::from_bytes([ON_TES; 16]).to_string(),
    )
}

async fn saved_template(pool: &PgPool) -> String {
    let saved = call(
        pool,
        Method::POST,
        "/v1/templates",
        Some(serde_json::json!({
            "name": "Phonics pack",
            "description": "For the phonics packs, not the assessments.",
            "scope": "Tpt",
            "draft": draft(),
        })),
    )
    .await;
    assert_eq!(
        saved.status,
        StatusCode::CREATED,
        "the template saves: {}",
        String::from_utf8_lossy(&saved.body)
    );
    let written: ResourceTemplateView = saved.json();
    assert_eq!(
        (written.description.as_deref(), written.scope),
        (
            Some("For the phonics packs, not the assessments."),
            Some(InventoryId::Tpt)
        ),
        "and answers the note and the scope it stored"
    );
    uuid::Uuid::from_bytes(written.id.0).to_string()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a missing row should panic"
)]
fn row<'a>(
    plan: &'a TemplateApplyPlanView,
    product: &str,
) -> &'a tam_api::template_apply::TemplateApplyRow {
    plan.rows
        .iter()
        .find(|row| uuid::Uuid::from_bytes(row.product.0 .0).to_string() == product)
        .expect("the plan answered a row for this resource")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_template_fills_what_is_empty_and_says_what_it_will_not_touch(pool: PgPool) {
    provision(&pool).await;
    let template = saved_template(&pool).await;
    let (blank, answered, on_tes) = ids();
    let selection = serde_json::json!({
        "selection": { "products": [blank, answered, on_tes] },
        "overwrite": false,
    });

    let planned = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply/plan"),
        Some(selection.clone()),
    )
    .await;
    assert_eq!(
        planned.status,
        StatusCode::OK,
        "a preview writes nothing and answers: {}",
        String::from_utf8_lossy(&planned.body)
    );
    let plan: TemplateApplyPlanView = planned.json();

    let filling = row(&plan, &blank);
    assert_eq!(
        (filling.verdict, filling.fields.as_slice()),
        (
            TemplateApplyVerdict::WillChange,
            [
                "description".to_owned(),
                "price".to_owned(),
                "copyright_declaration_id".to_owned()
            ]
            .as_slice()
        ),
        "a resource authored blank takes every field the template answers, named so the \
         seller reads which: {filling:?}"
    );
    let kept = row(&plan, &answered);
    assert_eq!(
        (kept.verdict, kept.fields.is_empty()),
        (TemplateApplyVerdict::Unchanged, true),
        "and a resource that has answered its own description, price and attestation takes \
         none of them: {kept:?}"
    );
    let refused = row(&plan, &on_tes);
    assert_eq!(refused.verdict, TemplateApplyVerdict::Blocked);
    assert_eq!(
        refused.reason.as_deref(),
        Some(
            "this listing is live on a platform whose edit-published transition is uncaptured, \
             so the edit cannot be attempted"
        ),
        "a row the edit route would refuse carries that route's own sentence rather than \
         failing the whole preview"
    );
    assert_eq!(
        (
            plan.counts.will_change,
            plan.counts.unchanged,
            plan.counts.blocked
        ),
        (1, 1, 1),
        "and the counts are the rows"
    );

    // Nothing was written by the preview: the answered resource's own price is
    // what a second plan still reports.
    let again: TemplateApplyPlanView = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply/plan"),
        Some(selection.clone()),
    )
    .await
    .json();
    assert_eq!(
        row(&again, &blank).verdict,
        TemplateApplyVerdict::WillChange,
        "a plan is a read: the blank resource is still blank after it"
    );

    let applied = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply"),
        Some(selection.clone()),
    )
    .await;
    assert_eq!(
        applied.status,
        StatusCode::OK,
        "the submit answers: {}",
        String::from_utf8_lossy(&applied.body)
    );
    let ack: TemplateApplyAck = applied.json();
    assert_eq!(
        (ack.changed, ack.unchanged, ack.blocked),
        (1, 1, 1),
        "the submit wrote exactly the rows the plan admitted"
    );

    let stored = ProductRepo::new(pool.clone())
        .get(ORG, ProductId(Uuid([BLANK; 16])))
        .await
        .unwrap_or_default()
        .map(|record| record.product);
    let stored = stored.unwrap_or_else(|| panic!("the filled resource reads back"));
    assert_eq!(
        (stored.body.body.as_str(), stored.price),
        ("Differentiated three ways.", paid(450)),
        "and the stored values are the template's"
    );
    let sidecar = TptBaseRepo::new(pool.clone())
        .get(ORG, ProductId(Uuid([BLANK; 16])))
        .await
        .unwrap_or_default();
    assert_eq!(
        sidecar.map(|held| held.copyright),
        Some(Some(CopyrightDeclaration::OriginalWork)),
        "including the sidecar the resource did not have before"
    );
    let untouched = ProductRepo::new(pool.clone())
        .get(ORG, ProductId(Uuid([ANSWERED; 16])))
        .await
        .unwrap_or_default()
        .map(|record| record.product.price);
    assert_eq!(
        untouched,
        Some(paid(1_200)),
        "and the resource the plan called unchanged was not written"
    );

    let replayed: TemplateApplyAck = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply"),
        Some(selection),
    )
    .await
    .json();
    assert_eq!(
        (replayed.changed, replayed.unchanged, replayed.blocked),
        (0, 2, 1),
        "a replay writes nothing, because a field is listed only where the merged value \
         differs from the stored one"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn overwrite_is_what_replaces_an_answer_the_seller_already_gave(pool: PgPool) {
    provision(&pool).await;
    let template = saved_template(&pool).await;
    let (_blank, answered, _on_tes) = ids();
    let selection = serde_json::json!({
        "selection": { "products": [answered] },
        "overwrite": true,
    });

    let plan: TemplateApplyPlanView = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply/plan"),
        Some(selection.clone()),
    )
    .await
    .json();
    let replacing = row(&plan, &answered);
    assert_eq!(
        (replacing.verdict, replacing.fields.as_slice()),
        (
            TemplateApplyVerdict::WillChange,
            [
                "description".to_owned(),
                "price".to_owned(),
                "copyright_declaration_id".to_owned()
            ]
            .as_slice()
        ),
        "under overwrite the template wins a field the resource had answered: {replacing:?}"
    );

    let ack: TemplateApplyAck = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply"),
        Some(selection.clone()),
    )
    .await
    .json();
    assert_eq!((ack.changed, ack.blocked), (1, 0));
    let stored = ProductRepo::new(pool.clone())
        .get(ORG, ProductId(Uuid([ANSWERED; 16])))
        .await
        .unwrap_or_default()
        .map(|record| record.product.price);
    assert_eq!(stored, Some(paid(450)), "and the price is the template's");

    let replayed: TemplateApplyAck = call(
        &pool,
        Method::POST,
        &format!("/v1/templates/{template}/apply"),
        Some(selection),
    )
    .await
    .json();
    assert_eq!(
        (replayed.changed, replayed.unchanged),
        (0, 1),
        "and overwrite is idempotent too: the second apply finds every value already there"
    );
}
