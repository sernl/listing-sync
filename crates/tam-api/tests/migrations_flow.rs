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

/// The resource that crosses: bound on Tes, with a file.
const MOVES: u8 = 0x01;
/// Authored here but never published on Tes, so there is nothing to move.
const UNLISTED: u8 = 0x02;
/// Already bound on TPT, so the migration has nothing to create.
const LANDED: u8 = 0x03;

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
