//! Copying and moving between two marketplaces, end to end: the preview's
//! verdict per resource, the pair the tree cannot cross yet, the request
//! written with every resource already canonicalised, the jobs the drain
//! mints from it, and the two refusals the month's allowance makes.

#![cfg(feature = "pg-tests")]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tam_api::migrations::{MigrationAck, MigrationPlanView, MigrationVerdict};
use tam_api::{router, APIError, AppState, Config, SESSION_COOKIE};
use tam_domain::{Binding, FieldPolicies, FieldPolicy, Mapping, PublishMode, Verification};
use tam_marketplace::{RemoteLifecycle, RemoteListingId};
use tam_storage::{
    Disposition, MappingRepo, ProductRepo, SessionRepo, SessionToken, SyncRequestRepo,
};
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId, JobId,
    ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId,
    ScanOutcome, Timestamp, Title, UserId, Uuid,
};
use tower::ServiceExt;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const TOKEN: SessionToken = SessionToken([0x41; 32]);
const NOW: Timestamp = Timestamp(1_756_000_000_000);
const KEY: &str = "11111111-1111-4111-8111-111111111111";
/// The seller asking again after the first attempt was refused, which is a
/// second migration and therefore a second key.
const SECOND_KEY: &str = "22222222-2222-4222-8222-222222222222";

/// The resource that crosses: bound on Tes, with a file.
const MOVES: u8 = 0x01;
/// Authored here but never published on Tes, so there is nothing to move.
const UNLISTED: u8 = 0x02;
/// Already bound on TPT, so the migration has nothing to create.
const LANDED: u8 = 0x03;
/// Held on TPT only, with a file and a paid price, declaring no rights: the
/// resource whose licence the seller's approved rule is the only answer to.
const GRANTED: u8 = 0x04;

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

fn tes(tag: u8) -> RemoteListingId {
    RemoteListingId::Tes {
        url: format!("https://www.tes.com/teaching-resource/fixture-{tag}"),
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

fn product(tag: u8) -> tam_domain::CanonicalProduct {
    tam_domain::CanonicalProduct {
        id: ProductId(Uuid([tag; 16])),
        org: ORG,
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

fn bound(tag: u8, inventory: InventoryId, remote: RemoteListingId) -> Mapping {
    Mapping {
        id: MappingId(Uuid([tag; 16])),
        org: ORG,
        product: ProductId(Uuid([tag & 0x0F; 16])),
        inventory,
        binding: Binding::Bound {
            id: remote,
            first_seen: NOW,
            verified: Verification::Clean { at: NOW },
        },
        policies: policies(),
        price_rule: PriceRule::Explicit(PriceIntent::Free),
        publish: PublishMode::DryRun,
        lifecycle: RemoteLifecycle::Live { since: NOW },
    }
}

/// The seller-device consent, granted for every marketplace that needs it,
/// so the mints under test are answered by the machinery rather than by the
/// consent gate; `consent_flow.rs` is where that gate is exercised.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn consented(pool: &PgPool, org: OrgId) {
    let consents = tam_storage::ConsentRepo::new(pool.clone());
    for marketplace in tam_types::Marketplace::ALL {
        if marketplace.transport_class() == tam_types::TransportClass::SellerDevice {
            consents
                .grant(
                    org,
                    marketplace,
                    tam_types::CONSENT_NOTICE_VERSION,
                    Uuid([0xC0; 16]),
                    Timestamp(1_000),
                )
                .await
                .expect("the fixture consent grants");
        }
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool, plan: Option<tam_limits::Plan>) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    consented(pool, ORG).await;
    if let Some(plan) = plan {
        tam_storage::EntitlementRepo::new(pool.clone())
            .grant(
                ORG,
                &tam_storage::NewGrant {
                    id: Uuid(*uuid::Uuid::new_v4().as_bytes()),
                    plan,
                    rung: None,
                    granted_by: tam_storage::GrantedBy::Paddle,
                    grantor_user: None,
                    reason: None,
                    source_ref: Some("org-a"),
                    granted_at: Timestamp(1_000),
                    expires_at: None,
                },
            )
            .await
            .expect("the fixture grant seeds");
    }
    let sessions = SessionRepo::new(pool.clone());
    sessions
        .create_user(
            ORG,
            UserId(Uuid([0x0A; 16])),
            "a@example.test",
            Timestamp(1_000),
        )
        .await
        .expect("the user provisions");
    sessions
        .mint(
            &TOKEN,
            UserId(Uuid([0x0A; 16])),
            Timestamp(NOW.0 + 86_400_000),
            Timestamp(NOW.0 - 1_000),
        )
        .await
        .expect("the session mints");

    let products = ProductRepo::new(pool.clone());
    let mappings = MappingRepo::new(pool.clone());
    for tag in [MOVES, UNLISTED, LANDED] {
        products
            .insert(ORG, &product(tag), Timestamp(1_000))
            .await
            .expect("the product inserts");
    }
    // Two of the three are on Tes; the third was authored here and never
    // published there, which is what makes it unmovable.
    for tag in [MOVES, LANDED] {
        mappings
            .insert(ORG, &bound(tag, InventoryId::Tes, tes(tag)), 0, NOW)
            .await
            .expect("the source mapping inserts");
    }
    mappings
        .insert(
            ORG,
            &bound(
                LANDED | 0x30,
                InventoryId::Tpt,
                RemoteListingId::Tpt { product_id: 9001 },
            ),
            0,
            NOW,
        )
        .await
        .expect("the target mapping inserts");
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
    path: &str,
    idempotency: Option<&str>,
    body: serde_json::Value,
) -> Answer {
    let mut request = Request::builder().method(Method::POST).uri(path).header(
        header::COOKIE,
        format!("{SESSION_COOKIE}={}", TOKEN.to_hex()),
    );
    if let Some(key) = idempotency {
        request = request.header("Idempotency-Key", key);
    }
    let request = request
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
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
    Answer { status, body }
}

fn move_body() -> serde_json::Value {
    serde_json::json!({
        "source": "Tes",
        "target": "Tpt",
        "disposition": "migrate",
        "selection": { "all": true },
    })
}

/// A resource whose only answer to Tes's required licence is the seller's own
/// approved rule is admitted, and the licence it was admitted on is the one
/// the device will post.
///
/// The production refusal this pins: the seller approved a TPT-to-Tes mapping
/// supplying `TES-PAID`, and the plan refused every row with "that
/// marketplace requires Licence", because the required-field gate could not
/// see the approval. This resource declares no rights and has settled no
/// election, so the approval is the only answer there is.
///
/// Both halves are asserted, in order: the same plan with no rule written
/// still refuses the row, which is the rule that must not be lost, and the
/// frozen output the confirm writes carries the native id, which is what the
/// engine reads at claim and the Tes adapter renders into the create.
///
/// The db lane, because an approval exists only once a rule row is written
/// and resolved through a `rule_capture` transaction.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_approved_mapping_answers_the_targets_required_licence(pool: PgPool) {
    provision(&pool, Some(tam_limits::Plan::Subscriber)).await;
    // Held on TPT and nowhere else, priced in the currency Tes sells in, and
    // declaring no rights at all: everything a create on Tes needs except the
    // licence it declares required.
    let mut granted = product(GRANTED);
    granted.price = PriceIntent::Paid(
        tam_types::Money::new(450, tam_types::Currency::Gbp).expect("a positive price"),
    );
    ProductRepo::new(pool.clone())
        .insert(ORG, &granted, Timestamp(1_000))
        .await
        .expect("the granted resource inserts");
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &bound(
                GRANTED | 0x30,
                InventoryId::Tpt,
                RemoteListingId::Tpt { product_id: 9002 },
            ),
            0,
            NOW,
        )
        .await
        .expect("the source mapping inserts");

    let row = |plan: &MigrationPlanView| {
        plan.rows
            .iter()
            .find(|row| row.product == ProductId(Uuid([GRANTED; 16])))
            .expect("the granted resource is previewed")
            .clone()
    };

    // Before the approval exists, the refusal stands: no declaration, no
    // election and no approved mapping is the case that must stay blocked.
    let unanswered: MigrationPlanView = call(pool.clone(), "/v1/migrations/plan", None, sync_up())
        .await
        .json();
    let unanswered = row(&unanswered);
    assert_eq!(unanswered.verdict, MigrationVerdict::Blocked);
    assert!(
        unanswered
            .reason
            .as_deref()
            .is_some_and(|why| why.contains("requires Licence")),
        "the licence is what blocks it: {:?}",
        unanswered.reason
    );

    // The seller's own rule, opted into moves, supplying the paid licence.
    tam_storage::seller_rules::SellerRuleRepo::new(pool.clone())
        .create(
            ORG,
            UserId(Uuid([0x0A; 16])),
            &tam_domain::seller_rules::SellerRuleDefinition {
                title: "Paid on Tes".to_owned(),
                description: "Everything I move to Tes is sold, not given away.".to_owned(),
                enabled: true,
                source: InventoryId::Tpt,
                target: InventoryId::Tes,
                auto_apply: vec![tam_domain::seller_rules::RuleUse::Move],
                conditions: tam_domain::seller_rules::RuleConditions::default(),
                action: tam_domain::seller_rules::RuleAction::Mapping {
                    licence: Some("TES-PAID".to_owned()),
                    resource_type: None,
                },
            },
            Timestamp(1_000),
        )
        .await
        .expect("the mapping rule writes");

    let answered: MigrationPlanView = call(pool.clone(), "/v1/migrations/plan", None, sync_up())
        .await
        .json();
    let answered = row(&answered);
    assert_eq!(
        (answered.verdict, answered.reason.as_deref()),
        (MigrationVerdict::WillCreate, None),
        "the approved mapping answers the field the target requires"
    );

    // And the licence the plan admitted on is the licence the write carries:
    // the confirm freezes it into the request snapshot, the mint copies that
    // into the item's own frozen output, and the engine reads exactly this
    // row when a device claims the create.
    let accepted = call(pool.clone(), "/v1/migrations", Some(KEY), sync_up()).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    let ack: MigrationAck = accepted.json();
    assert_eq!(ack.queued, 1, "only the granted resource crosses");
    let create_job = SyncRequestRepo::new(pool.clone())
        .get(ORG, ack.request)
        .await
        .expect("the request reads")
        .expect("it exists")
        .create_job
        .expect("a migration creates on the target");
    let items = tam_storage::JobReadRepo::new(pool.clone())
        .items_page(
            ORG,
            JobId(create_job),
            tam_storage::ItemsPageParams {
                cursor: None,
                limit: 10,
                outcome: None,
            },
        )
        .await
        .expect("the create job's items read");
    assert_eq!(items.len(), 1);
    let frozen = tam_storage::rule_capture::frozen_output(&pool, ORG, items[0].item)
        .await
        .expect("the frozen output reads")
        .expect("a create the seller approved carries one");
    assert_eq!(
        frozen.licence.map(|choice| choice.native_id).as_deref(),
        Some("TES-PAID"),
        "the grant the device posts is the one the seller's rule authored"
    );
}

/// The same body the approved-licence test plans and confirms with: TPT is
/// the source Tes takes the listing from.
fn sync_up() -> serde_json::Value {
    serde_json::json!({
        "source": "Tpt",
        "target": "Tes",
        "disposition": "migrate",
        // The one resource, rather than the whole catalogue: the other
        // fixtures are free, and a rule that sets `TES-PAID` on a free
        // resource is refused outright — a real refusal, and a different
        // one from the licence gate this test is about.
        "selection": { "products": ["04040404-0404-0404-0404-040404040404"] },
    })
}

/// The preview answers per resource, and the confirm moves exactly what it
/// admitted.
///
/// The three rows are the three answers: a resource bound on the source with a
/// file will be created, a resource never published on the source is blocked
/// because there is no listing to move, and a resource already bound on the
/// target is already there. Only the first spends the allowance, and only the
/// first reaches the drain.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_move_previews_each_resource_and_queues_only_what_it_admitted(pool: PgPool) {
    provision(&pool, Some(tam_limits::Plan::Subscriber)).await;

    let planned = call(pool.clone(), "/v1/migrations/plan", None, move_body()).await;
    assert_eq!(planned.status, StatusCode::OK);
    let plan: MigrationPlanView = planned.json();
    assert!(plan.pair.allowed, "Tes to TPT is the pair that works today");
    assert_eq!(
        (
            plan.counts.will_create,
            plan.counts.already_there,
            plan.counts.blocked
        ),
        (1, 1, 1),
    );
    let verdict = |tag: u8| {
        plan.rows
            .iter()
            .find(|row| row.product == ProductId(Uuid([tag; 16])))
            .expect("every chosen resource has a row")
    };
    assert_eq!(verdict(MOVES).verdict, MigrationVerdict::WillCreate);
    assert_eq!(verdict(UNLISTED).verdict, MigrationVerdict::Blocked);
    assert_eq!(
        verdict(UNLISTED).reason.as_deref(),
        Some("not listed on Tes"),
    );
    assert_eq!(verdict(LANDED).verdict, MigrationVerdict::AlreadyThere);
    assert_eq!(verdict(LANDED).remote.as_deref(), Some("9001"));
    assert_eq!(
        (plan.cap.limit, plan.cap.used, plan.cap.remaining),
        (20, 0, 20),
        "the preview states the allowance against the selection, before any of it is spent"
    );

    // A source whose download nobody has captured is offered and refused with
    // the reason, rather than being absent: Etsy is the capture this tree is
    // still waiting on.
    let unreadable = call(
        pool.clone(),
        "/v1/migrations/plan",
        None,
        serde_json::json!({
            "source": "Etsy",
            "target": "Tes",
            "disposition": "sync",
            "selection": { "all": true },
        }),
    )
    .await;
    let unreadable: MigrationPlanView = unreadable.json();
    assert!(!unreadable.pair.allowed);
    assert!(
        unreadable
            .pair
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("etsy.download_resource_bundle")),
        "the refusal names the capture, which is what the console shows under the disabled \
         option"
    );
    assert_eq!(unreadable.counts.blocked, 3, "no pair, no admitted rows");

    // And the direction that was refused until 2026-09-13 is now offered: the
    // seller's own device performs the TPT download the capture witnessed, so
    // the pair carries no reason at all.
    let backwards = call(
        pool.clone(),
        "/v1/migrations/plan",
        None,
        serde_json::json!({
            "source": "Tpt",
            "target": "Tes",
            "disposition": "sync",
            "selection": { "all": true },
        }),
    )
    .await;
    let backwards: MigrationPlanView = backwards.json();
    assert_eq!(
        (backwards.pair.allowed, backwards.pair.reason.as_deref()),
        (true, None),
        "TPT as a source is a captured download now, so nothing about the pair refuses it"
    );

    let accepted = call(pool.clone(), "/v1/migrations", Some(KEY), move_body()).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED);
    let ack: MigrationAck = accepted.json();
    assert_eq!((ack.queued, ack.skipped), (1, 2));

    let requests = SyncRequestRepo::new(pool.clone());
    let record = requests
        .get(ORG, ack.request)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert_eq!(record.disposition, Disposition::Migrate);
    assert_eq!(
        record.resources.len(),
        1,
        "the skipped rows are not written"
    );
    let resource = &record.resources[0];
    assert_eq!(resource.state, "canonicalised");
    assert_eq!(resource.product, Some(ProductId(Uuid([MOVES; 16]))));
    assert_eq!(resource.source, Some(tes(MOVES)));
    assert!(
        resource.is_canonicalised(),
        "a migration needs no marketplace read, so its breadcrumb is complete at submit"
    );

    // A replay of the same key is the same migration.
    let replayed = call(pool.clone(), "/v1/migrations", Some(KEY), move_body()).await;
    assert_eq!(replayed.status, StatusCode::OK);
    assert_eq!(replayed.json::<MigrationAck>().request, ack.request);

    // A second preview knows the create is on its way: the binding has not
    // moved, because no device has claimed the item, and the preview must not
    // offer the same create twice on the strength of that.
    let again = call(pool.clone(), "/v1/migrations/plan", None, move_body()).await;
    let again: MigrationPlanView = again.json();
    let moving = again
        .rows
        .iter()
        .find(|row| row.product == ProductId(Uuid([MOVES; 16])))
        .expect("the moved resource is previewed");
    assert_eq!(moving.verdict, MigrationVerdict::Blocked);
    assert!(
        moving
            .reason
            .as_deref()
            .is_some_and(|why| why.contains("on its way")),
        "a confirmed create is not offered twice: {:?}",
        moving.reason
    );
    assert_eq!(again.counts.will_create, 0);

    // The confirm drained the request itself: the create on the target and
    // the removal on the source are already minted, because no device page
    // follows a catalogue migration to do it. Read straight off the record.
    let record = requests
        .get(ORG, ack.request)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert_eq!(record.state, "enqueued", "the confirm left nothing pending");
    let create_job = record
        .create_job
        .expect("a migration creates on the target");
    let remove_job = record.remove_job.expect("a move removes from the source");

    let reads = tam_storage::JobReadRepo::new(pool.clone());
    let created = reads
        .items_page(
            ORG,
            JobId(create_job),
            tam_storage::ItemsPageParams {
                cursor: None,
                limit: 10,
                outcome: None,
            },
        )
        .await
        .expect("the create job's items read");
    assert_eq!(created.len(), 1, "one create for the one admitted resource");

    // Read under a pinned organisation, because `job_item` has forced RLS
    // and the gate column is on the item rather than on any projection the
    // repos expose.
    let mut tx = pool.begin().await.expect("the read transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let gate: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT j.inventory, i.requires_bound_on FROM job_item i \
         JOIN job j ON j.org_id = i.org_id AND j.id = i.job_id \
         WHERE i.job_id = $1",
    )
    .bind(uuid::Uuid::from_bytes(remove_job.0))
    .fetch_all(&mut *tx)
    .await
    .expect("the removal items read");
    assert_eq!(
        gate,
        vec![("tes".to_owned(), Some("tpt".to_owned()))],
        "the source removal is on Tes and waits on the target's own binding, so the unsafe \
         direction -- source gone, target absent -- is unreachable"
    );
}

/// A plan that includes no moving at all is refused as a missing feature,
/// not as an exhausted count.
///
/// The two send the console to different places: one is an upgrade, the other
/// is a date to wait for.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_free_plan_is_refused_the_capability_rather_than_the_count(pool: PgPool) {
    provision(&pool, None).await;
    let refused = call(pool.clone(), "/v1/migrations", Some(KEY), move_body()).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = refused.json();
    let detail = error.errors[0]
        .detail
        .as_ref()
        .expect("a quota refusal carries its detail");
    assert_eq!(detail["quota"], "plan_feature");
    assert_eq!(detail["feature"], "migrations_per_month");
}

/// A month's allowance already spent refuses the confirm and says when the
/// next one arrives.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_spent_allowance_refuses_the_confirm(pool: PgPool) {
    provision(&pool, Some(tam_limits::Plan::Subscriber)).await;
    // Twenty resources copied this month, which is a subscriber's whole
    // allowance. Counted in resources rather than in requests, so one request
    // of twenty exhausts it exactly as twenty of one would.
    SyncRequestRepo::new(pool.clone())
        .create(
            ORG,
            &tam_storage::NewSyncRequest {
                id: Uuid([0x99; 16]),
                source: InventoryId::Tes,
                target: InventoryId::Tpt,
                disposition: Disposition::Sync,
                intent: tam_storage::SyncIntent::Draft,
                requested_at: NOW,
                locators: (0..20).map(|n| n.to_string()).collect(),
            },
        )
        .await
        .expect("the earlier month's work seeds");

    let planned: MigrationPlanView = call(pool.clone(), "/v1/migrations/plan", None, move_body())
        .await
        .json();
    assert_eq!(
        (planned.cap.used, planned.cap.remaining),
        (20, 0),
        "the preview says the allowance is gone before the seller confirms"
    );

    let refused = call(pool.clone(), "/v1/migrations", Some(KEY), move_body()).await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error: APIError = refused.json();
    let detail = error.errors[0]
        .detail
        .as_ref()
        .expect("a quota refusal carries its detail");
    assert_eq!(detail["quota"], "migrations_per_month");
    assert_eq!(detail["used"], 20);
    assert_eq!(detail["limit"], 20);
}

/// The expired session as the device reports it: every item of the job
/// settles `blocked` having written nothing, so the listing is still not on
/// the target.
///
/// Written straight onto the ledger rather than through a claim and a settle,
/// because what this test needs is the state the nine refused creates were
/// left in; `crates/tam-storage/tests/leases.rs` is where the settle itself
/// is exercised.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn settle_blocked(pool: &PgPool, job: Uuid) {
    let mut tx = pool.begin().await.expect("the fixture transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    let settled = sqlx::query(
        "UPDATE job_item SET state = 'settled', outcome = 'blocked', \
           failure_code = 'SessionExpired', settled_at = now() \
         WHERE org_id = $1 AND job_id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(job.0))
    .execute(&mut *tx)
    .await
    .expect("the items settle");
    assert_eq!(settled.rows_affected(), 1, "the job had its one create");
    tx.commit().await.expect("the fixture commits");
}

/// A resource whose last create was refused can be migrated again.
///
/// The production defect: nine resources whose earlier items settled
/// `blocked` on an expired Tes session were re-submitted, and the confirm
/// answered 500 with the `sync_request` row already committed — state
/// `draining`, no jobs, nothing saying why. The item's idempotency key is
/// derived from the content and nothing about the request, `job_item` rows are
/// never deleted, and so the second attempt met `job_item_idempotent` and left
/// through the fault path.
///
/// A second key, because the same key is the retry of one migration and is
/// answered as a replay before the plan is ever recomputed. This is the
/// seller asking again, which is a migration of its own.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_migration_whose_create_was_blocked_can_be_asked_for_again(pool: PgPool) {
    provision(&pool, Some(tam_limits::Plan::Subscriber)).await;
    let first = call(pool.clone(), "/v1/migrations", Some(KEY), move_body()).await;
    assert_eq!(first.status, StatusCode::ACCEPTED);
    let requests = SyncRequestRepo::new(pool.clone());
    let refused = requests
        .get(ORG, first.json::<MigrationAck>().request)
        .await
        .expect("the request reads")
        .expect("it exists");
    let refused_job = refused
        .create_job
        .expect("the confirm drained a create onto the target");
    settle_blocked(&pool, refused_job).await;

    let again = call(
        pool.clone(),
        "/v1/migrations",
        Some(SECOND_KEY),
        move_body(),
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::ACCEPTED,
        "a refused create is work still owed, not a duplicate: {}",
        String::from_utf8_lossy(&again.body)
    );
    let ack: MigrationAck = again.json();
    assert_eq!(ack.queued, 1, "the resource is offered again, and admitted");
    let record = requests
        .get(ORG, ack.request)
        .await
        .expect("the request reads")
        .expect("it exists");
    assert_eq!(
        record.state, "enqueued",
        "the drain finished, so the request is not left draining with no jobs"
    );
    let queued_job = record
        .create_job
        .expect("the re-submit mints a create of its own");
    assert_ne!(
        queued_job, refused_job,
        "a new job, rather than the one whose item was refused"
    );
    let items = tam_storage::JobReadRepo::new(pool.clone())
        .items_page(
            ORG,
            JobId(queued_job),
            tam_storage::ItemsPageParams {
                cursor: None,
                limit: 10,
                outcome: None,
            },
        )
        .await
        .expect("the create job's items read");
    assert_eq!(items.len(), 1, "one create for the one admitted resource");
}
