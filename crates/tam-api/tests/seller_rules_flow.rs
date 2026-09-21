#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tam_api::{router, AppState, Config, SESSION_COOKIE};
use tam_domain::{CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration};
use tam_storage::{ProductRepo, SessionRepo, SessionToken};
use tam_types::{
    CopyFormat, Currency, ListingCopy, Money, OrgId, PriceIntent, ProductId, Timestamp, Title,
    UserId, Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAB; 16]));
const USER: UserId = UserId(Uuid([0xBC; 16]));
const TOKEN: SessionToken = SessionToken([0x52; 32]);
const NOW: Timestamp = Timestamp(1_789_603_200_000);

#[expect(
    clippy::expect_used,
    reason = "integration fixture failures must stop the scenario"
)]
async fn call_as(
    pool: &PgPool,
    token: &SessionToken,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(
            header::COOKIE,
            format!("{SESSION_COOKIE}={}", token.to_hex()),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builds");
    let response = router(AppState {
        telemetry: tam_api::telemetry::Telemetry::default(),
        exchange_rates: None,
        pool: pool.clone(),
        config: Config::default(),
        wall: || NOW,
        auth: None,
        backoffice: None,
        blobs: None,
    })
    .oneshot(request)
    .await
    .expect("router answers");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body reads")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("response is JSON")
    };
    (status, value)
}

async fn call(pool: &PgPool, method: Method, path: &str, body: Value) -> (StatusCode, Value) {
    call_as(pool, &TOKEN, method, path, body).await
}

#[expect(
    clippy::expect_used,
    reason = "integration fixture failures must stop the scenario"
)]
async fn provision_account(pool: &PgPool, org: OrgId, user: UserId, token: &SessionToken) {
    sqlx::query(
        "INSERT INTO organisation (id, name, created_at) VALUES ($1, 'pricing fixture', now())",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .execute(pool)
    .await
    .expect("org seeds");
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(
            org,
            user,
            &format!("{}@example.test", org.0.to_hyphenated()),
            Timestamp(1_000),
        )
        .await
        .expect("user seeds");
    sessions
        .mint(
            token,
            user,
            Timestamp(NOW.0 + 86_400_000),
            Timestamp(NOW.0 - 1_000),
        )
        .await
        .expect("session seeds");
    tam_storage::EntitlementRepo::new(pool.clone())
        .grant(
            org,
            &tam_storage::NewGrant {
                id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                plan: tam_limits::Plan::Studio,
                rung: None,
                granted_by: tam_storage::GrantedBy::Operator,
                grantor_user: None,
                reason: Some("pricing fixture"),
                source_ref: None,
                granted_at: Timestamp(1_000),
                expires_at: None,
            },
        )
        .await
        .expect("entitlement seeds");
}

#[expect(
    clippy::expect_used,
    reason = "integration fixture failures must stop the scenario"
)]
async fn provision(pool: &PgPool) {
    provision_account(pool, ORG, USER, &TOKEN).await;
    for tag in [1_u8, 2] {
        ProductRepo::new(pool.clone())
            .insert(
                ORG,
                &CanonicalProduct {
                    id: ProductId(Uuid([tag; 16])),
                    org: ORG,
                    title: Title(format!("Resource {tag}")),
                    body: ListingCopy {
                        body: "An arithmetic worksheet.".to_owned(),
                        format: CopyFormat::Markdown,
                    },
                    payload: None,
                    cover: None,
                    previews: vec![],
                    subjects: vec![],
                    grades: GradeDeclaration {
                        source: DeclarationSource::Seller,
                        raw: vec![],
                        derived: None,
                    },
                    price: PriceIntent::Paid(
                        Money::new(300, Currency::Usd).expect("positive source price"),
                    ),
                    rights: RightsDeclaration::Unstated,
                    native_residue: vec![],
                },
                Timestamp(1_000),
            )
            .await
            .expect("resource seeds");
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn accepting_a_selected_price_preserves_source_money_and_other_resources(pool: PgPool) {
    provision(&pool).await;
    let first = uuid::Uuid::from_bytes([1; 16]).to_string();
    let second = uuid::Uuid::from_bytes([2; 16]).to_string();
    let body = json!({
        "source": "Tpt", "target": "Tes", "selection": {"products": [first]},
        "rule_ids": [], "overrides": {"rate": "0.75"}
    });
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        body.clone(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a selected resource has a proposal: {preview}"
    );
    assert_eq!(preview["rows"][0]["product"], first);
    assert_eq!(
        preview["rows"][0]["proposed"]["price"],
        json!({"Paid":{"minor_units":225,"currency":"Gbp"}})
    );
    assert_eq!(
        preview["rows"].as_array().expect("rows").len(),
        1,
        "the other resource was not selected"
    );
    let id = preview["id"].as_str().expect("preview identity");
    let path = format!("/v1/seller-rules/previews/{id}/decision");
    let decision = json!({"decision":"accept", "selection":{"all":true}});
    let (status, accepted) = call(&pool, Method::POST, &path, decision.clone()).await;
    assert_eq!(status, StatusCode::OK, "acceptance succeeds: {accepted}");
    assert_eq!(accepted["accepted"], 1);
    let (status, replay) = call(&pool, Method::POST, &path, decision).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a retry keeps the same decision: {replay}"
    );

    let (status, next) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt", "target":"Tes", "selection":{"products":[first, second]}, "rule_ids":[]
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "target choices remain inspectable: {next}"
    );
    let rows = next["rows"].as_array().expect("rows");
    let chosen = rows
        .iter()
        .find(|row| row["product"] == first)
        .expect("chosen resource");
    let untouched = rows
        .iter()
        .find(|row| row["product"] == second)
        .expect("untouched resource");
    assert_eq!(
        chosen["source_price"],
        json!({"Paid":{"minor_units":300,"currency":"Usd"}})
    );
    assert_eq!(
        chosen["before"]["price"],
        json!({"Paid":{"minor_units":225,"currency":"Gbp"}})
    );
    assert_eq!(untouched["before"]["price"], Value::Null);
    assert_eq!(
        untouched["source_price"],
        json!({"Paid":{"minor_units":300,"currency":"Usd"}})
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn same_clock_source_edits_refuse_confirmation_and_rejection_changes_no_target(pool: PgPool) {
    provision(&pool).await;
    let product = uuid::Uuid::from_bytes([1; 16]).to_string();
    let source_path = format!("/v1/products/{product}");
    let (status, changed) = call(
        &pool,
        Method::PATCH,
        &source_path,
        json!({
            "price":{"Paid":{"minor_units":400,"currency":"Usd"}}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the seller edits source money: {changed}"
    );
    let request = json!({
        "source":"Tpt","target":"Tes","selection":{"products":[product]},
        "rule_ids":[],"overrides":{"rate":"0.75"}
    });
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "preview answers: {preview}");
    assert_eq!(
        preview["rows"][0]["proposed"]["price"],
        json!({"Paid":{"minor_units":300,"currency":"Gbp"}})
    );
    let (status, changed) = call(
        &pool,
        Method::PATCH,
        &source_path,
        json!({
            "price":{"Paid":{"minor_units":500,"currency":"Usd"}}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "another edit at the same wall instant succeeds: {changed}"
    );
    let decision_path = format!(
        "/v1/seller-rules/previews/{}/decision",
        preview["id"].as_str().expect("preview id")
    );
    let (status, refused) = call(
        &pool,
        Method::POST,
        &decision_path,
        json!({
            "decision":"accept","selection":{"all":true}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "same timestamps cannot hide changed source facts: {refused}"
    );
    let (status, fresh) = call(&pool, Method::POST, "/v1/seller-rules/preview", request).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a fresh proposal is possible: {fresh}"
    );
    assert_eq!(
        fresh["rows"][0]["before"]["price"],
        Value::Null,
        "the stale acceptance wrote nothing"
    );
    assert_eq!(
        fresh["rows"][0]["proposed"]["price"],
        json!({"Paid":{"minor_units":375,"currency":"Gbp"}})
    );
    let decision_path = format!(
        "/v1/seller-rules/previews/{}/decision",
        fresh["id"].as_str().expect("preview id")
    );
    let (status, rejected) = call(
        &pool,
        Method::POST,
        &decision_path,
        json!({
            "decision":"reject","selection":{"all":true}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "rejecting does not apply the proposal: {rejected}"
    );
    assert_eq!(rejected["rejected"], 1);
    let (status, unchanged) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[product]},"rule_ids":[]
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the target remains inspectable: {unchanged}"
    );
    assert_eq!(unchanged["rows"][0]["before"]["price"], Value::Null);
    assert_eq!(
        unchanged["rows"][0]["source_price"],
        json!({"Paid":{"minor_units":500,"currency":"Usd"}})
    );
}

fn price_rule(rate: &str) -> Value {
    json!({"definition":{
        "title":format!("USD to GBP {rate}"),"description":"A seller's directional estimate.",
        "enabled":true,"source":"Tpt","target":"Tes","auto_apply":[],
        "conditions":{"pricing":"paid","resource_types":[],"keywords":[],"keyword_mode":"all","attributes":[]},
        "action":{"kind":"pricing","rate":rate,"rounding":"Nearest","reference":null}
    }})
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn new_matching_rules_stale_a_preview_and_conflicts_need_an_explicit_override(pool: PgPool) {
    provision(&pool).await;
    let (status, saved) = call(&pool, Method::POST, "/v1/seller-rules", price_rule("0.75")).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the first estimate saves: {saved}"
    );
    let product = uuid::Uuid::from_bytes([1; 16]).to_string();
    let mut request = json!({"source":"Tpt","target":"Tes","selection":{"products":[product]}});
    let (status, first) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        request.clone(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the first proposal answers: {first}"
    );
    let (status, saved) = call(&pool, Method::POST, "/v1/seller-rules", price_rule("0.80")).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a competing estimate saves: {saved}"
    );
    let path = format!(
        "/v1/seller-rules/previews/{}/decision",
        first["id"].as_str().expect("preview id")
    );
    let (status, refused) = call(
        &pool,
        Method::POST,
        &path,
        json!({"decision":"accept","selection":{"all":true}}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a new member of the implicit set invalidates approval: {refused}"
    );
    let (status, conflicting) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        request.clone(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "conflicts are previewable: {conflicting}"
    );
    assert_eq!(conflicting["rows"][0]["status"], "blocked");
    request["overrides"] = json!({"rate":"0.70"});
    let (status, resolved) = call(&pool, Method::POST, "/v1/seller-rules/preview", request).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an explicit rate resolves the conflict: {resolved}"
    );
    assert_eq!(resolved["rows"][0]["status"], "proposed");
    assert_eq!(
        resolved["rows"][0]["proposed"]["price"],
        json!({"Paid":{"minor_units":210,"currency":"Gbp"}})
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn another_tenant_cannot_read_rules_or_accept_a_known_preview(pool: PgPool) {
    provision(&pool).await;
    let other = SessionToken([0x63; 32]);
    provision_account(
        &pool,
        OrgId(Uuid([0xCD; 16])),
        UserId(Uuid([0xDE; 16])),
        &other,
    )
    .await;
    let (status, saved) = call(&pool, Method::POST, "/v1/seller-rules", price_rule("0.75")).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the owner's rule saves: {saved}"
    );
    let product = uuid::Uuid::from_bytes([1; 16]).to_string();
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[product]}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the owner's preview answers: {preview}"
    );
    let (status, hidden) =
        call_as(&pool, &other, Method::GET, "/v1/seller-rules", Value::Null).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the other tenant can read its own empty rule list: {hidden}"
    );
    assert_eq!(hidden["counts"]["all"], 0);
    let path = format!(
        "/v1/seller-rules/previews/{}/decision",
        preview["id"].as_str().expect("preview id")
    );
    let (status, hidden) = call_as(
        &pool,
        &other,
        Method::POST,
        &path,
        json!({
            "decision":"accept","selection":{"all":true}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a known foreign preview is still invisible: {hidden}"
    );
    let read_path = format!(
        "/v1/seller-rules/previews/{}",
        preview["id"].as_str().expect("preview id")
    );
    let (status, hidden) = call_as(&pool, &other, Method::GET, &read_path, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "foreign proposals and decisions cannot be read: {hidden}"
    );
    let (status, accepted) = call(
        &pool,
        Method::POST,
        &path,
        json!({
            "decision":"accept","selection":{"all":true}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the failed foreign decision changed nothing: {accepted}"
    );
    assert_eq!(accepted["accepted"], 1);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn rejecting_selected_rows_does_not_prevent_accepting_the_remainder(pool: PgPool) {
    provision(&pool).await;
    let first = uuid::Uuid::from_bytes([1; 16]).to_string();
    let second = uuid::Uuid::from_bytes([2; 16]).to_string();
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[first,second]},
            "rule_ids":[],"overrides":{"rate":"0.75"}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "both resources are previewed: {preview}"
    );
    let path = format!(
        "/v1/seller-rules/previews/{}/decision",
        preview["id"].as_str().expect("preview id")
    );
    let (status, rejected) = call(
        &pool,
        Method::POST,
        &path,
        json!({
            "decision":"reject","selection":{"products":[second]}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the selected proposal is rejected: {rejected}"
    );
    let read_path = format!(
        "/v1/seller-rules/previews/{}",
        preview["id"].as_str().expect("preview id")
    );
    let (status, held) = call(&pool, Method::GET, &read_path, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the console can reload the same proposal: {held}"
    );
    assert_eq!(
        held["id"], preview["id"],
        "reloading never starts a new proposal"
    );
    let held_rows = held["rows"].as_array().expect("stored rows");
    assert_eq!(
        held_rows
            .iter()
            .find(|row| row["product"] == second)
            .expect("rejected row")["decision"],
        "rejected",
        "a reload retains the seller's rejection"
    );
    assert_eq!(
        held_rows
            .iter()
            .find(|row| row["product"] == first)
            .expect("remaining row")["decision"],
        "pending",
        "the other proposal remains available"
    );
    let (status, accepted) = call(
        &pool,
        Method::POST,
        &path,
        json!({
            "decision":"accept","selection":{"all":true}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "all means the remaining eligible proposals: {accepted}"
    );
    assert_eq!(accepted["accepted"], 1);
    assert_eq!(accepted["remaining"], 0);
    let (status, held) = call(&pool, Method::GET, &read_path, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "decisions remain inspectable: {held}"
    );
    let held_rows = held["rows"].as_array().expect("decided rows");
    assert_eq!(
        held_rows
            .iter()
            .find(|row| row["product"] == first)
            .expect("accepted row")["decision"],
        "accepted",
        "the remaining proposal is approved"
    );
    assert_eq!(
        held_rows
            .iter()
            .find(|row| row["product"] == second)
            .expect("rejected row")["decision"],
        "rejected",
        "approving the remainder never reverses the rejection"
    );
    let (status, reversed) = call(
        &pool,
        Method::POST,
        &path,
        json!({
            "decision":"accept","selection":{"products":[second]}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "explicitly reversing a rejection still refuses: {reversed}"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn repeated_resource_ids_are_one_approval(pool: PgPool) {
    provision(&pool).await;
    let product = uuid::Uuid::from_bytes([1; 16]).to_string();
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[product]},
            "rule_ids":[],"overrides":{"rate":"0.75"}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "one resource is previewed: {preview}"
    );
    let path = format!(
        "/v1/seller-rules/previews/{}/decision",
        preview["id"].as_str().expect("preview id")
    );
    let decision = json!({"decision":"accept","selection":{"products":[product,product]}});
    for _ in 0..2 {
        let (status, result) = call(&pool, Method::POST, &path, decision.clone()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "duplicate IDs and delivery replay are idempotent: {result}"
        );
        assert_eq!(result["accepted"], 1, "one resource is one approval");
    }
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn unmatched_empty_rows_do_not_abort_other_approvals(pool: PgPool) {
    provision(&pool).await;
    let first = uuid::Uuid::from_bytes([1; 16]).to_string();
    let second = uuid::Uuid::from_bytes([2; 16]).to_string();
    let (status, changed) = call(
        &pool,
        Method::PATCH,
        &format!("/v1/products/{second}"),
        json!({"price":"Free"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the second source resource is free: {changed}"
    );
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[first,second]},
            "rule_ids":[],"draft":price_rule("0.75")["definition"]
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the mixed selection is previewable: {preview}"
    );
    assert_eq!(preview["counts"]["proposed"], 1);
    assert_eq!(preview["counts"]["unchanged"], 1);
    let path = format!(
        "/v1/seller-rules/previews/{}/decision",
        preview["id"].as_str().expect("preview id")
    );
    let (status, accepted) = call(
        &pool,
        Method::POST,
        &path,
        json!({
            "decision":"accept","selection":{"all":true}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the unmatched empty row cannot roll back a valid approval: {accepted}"
    );
    let (status, after) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[first,second]},"rule_ids":[]
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the resulting target choices are readable: {after}"
    );
    let rows = after["rows"].as_array().expect("rows");
    assert_eq!(
        rows.iter()
            .find(|row| row["product"] == first)
            .expect("paid row")["before"]["price"],
        json!({"Paid":{"minor_units":225,"currency":"Gbp"}})
    );
    assert_eq!(
        rows.iter()
            .find(|row| row["product"] == second)
            .expect("free row")["before"]["price"],
        Value::Null,
        "an unmatched source did not gain a fabricated target choice"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn all_means_the_bound_source_catalogue_not_other_sources_or_drafts(pool: PgPool) {
    use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode, Verification};
    use tam_marketplace::{RemoteLifecycle, RemoteListingId};
    use tam_types::{
        ContentHash, FileBytes, FileId, FileKind, FileRole, InventoryId, MappingId, PriceRule,
        ProductFile, ScanOutcome,
    };

    provision(&pool).await;
    let first = ProductId(Uuid([1; 16]));
    let second = ProductId(Uuid([2; 16]));
    for product in [first, second] {
        ProductRepo::new(pool.clone())
            .add_file(
                ORG,
                product,
                (
                    &ProductFile {
                        id: FileId(product.0),
                        role: FileRole::Payload,
                        kind: FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: ContentHash([0x91; 32]),
                            byte_len: 4,
                            scan: ScanOutcome::Clean { at: NOW },
                        },
                    },
                    None,
                ),
                NOW,
            )
            .await
            .expect("payload storage answers")
            .expect("the listing has a live payload");
    }
    let bound = |id| Binding::Bound {
        id,
        first_seen: NOW,
        verified: Verification::Clean { at: NOW },
    };
    for (tag, product, inventory, binding, lifecycle) in [
        (
            0x71,
            first,
            InventoryId::Tpt,
            bound(RemoteListingId::Tpt { product_id: 9001 }),
            RemoteLifecycle::Live { since: NOW },
        ),
        (
            0x72,
            second,
            InventoryId::Tes,
            bound(RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/fixture-9002".to_owned(),
            }),
            RemoteLifecycle::Live { since: NOW },
        ),
        (
            0x73,
            second,
            InventoryId::Tpt,
            Binding::Unbound,
            RemoteLifecycle::Absent,
        ),
    ] {
        tam_storage::MappingRepo::new(pool.clone())
            .insert(
                ORG,
                &Mapping {
                    id: MappingId(Uuid([tag; 16])),
                    org: ORG,
                    product,
                    inventory,
                    binding,
                    lifecycle,
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
                },
                0,
                NOW,
            )
            .await
            .expect("source membership seeds");
    }
    let (status, preview) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"all":true},
            "rule_ids":[],"overrides":{"rate":"0.75"}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the source catalogue previews: {preview}"
    );
    let rows = preview["rows"].as_array().expect("source rows");
    assert_eq!(
        rows.iter()
            .map(|row| row["product"].as_str().expect("product id"))
            .collect::<Vec<_>>(),
        vec![first.0.to_hyphenated()],
        "only a bound TPT listing belongs to all TPT resources"
    );
    assert_eq!(
        rows[0]["proposed"]["price"],
        json!({"Paid":{"minor_units":225,"currency":"Gbp"}}),
        "the selected source resource receives its proposal"
    );
    let (status, unrelated) = call(
        &pool,
        Method::POST,
        "/v1/seller-rules/preview",
        json!({
            "source":"Tpt","target":"Tes","selection":{"products":[second.0.to_hyphenated()]},
            "rule_ids":[],"overrides":{"rate":"0.75"}
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an explicit selection explains its refusal: {unrelated}"
    );
    assert_eq!(
        unrelated["rows"][0]["status"], "blocked",
        "an unbound TPT draft does not relabel a TES listing as a TPT source"
    );
}
