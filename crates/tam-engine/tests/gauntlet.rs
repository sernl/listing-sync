//! The gauntlet: the whole duplication flow from an enqueued item to a
//! settled outcome, through the real lease, the real seed, the real driver
//! and the real Tes adapter — against a programmable fake marketplace whose
//! routes carry the exact shapes the M0 spike measured. The clean run
//! settles Committed without ever touching publish (dry-run is the same
//! code path with the gate shut); the hostile run proves per-item honesty.

#![cfg(feature = "pg-tests")]

use std::collections::HashMap;
use tokio::sync::Mutex;

use base64::Engine as _;
use serde_json::{json, Value};
use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalTerm, Decider, EdgeKind, ItemOutcome, ProjectionEdge, TermKind, Verification,
    VocabularyId, VocabularyPath,
};
use tam_engine::driver::{run_item, DriverContext, NowSource, RunVerdict};
use tam_engine::seed::{project_for_item, seed_from_projection, ProjectionOutcome};
use tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX;
use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestBody, Transport, TransportError,
};
use tam_marketplace::{FileContent, FileSource, FileSourceError, RemoteListingId};
use tam_marketplace_tes::TesAdapter;
use tam_storage::{
    BudgetGrant, HaltRepo, JobRepo, LeaseRepo, MappingRepo, NewJob, NewJobItem, ProductRepo,
    RateBudgetRepo, TaxonomyRepo, WriteAttemptRepo,
};
use tam_types::{
    CanonicalTermId, ContentHash, FileId, FileKind, FileRole, InventoryId, JobId, ListingCopy,
    MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, Uuid,
};
use tokio_util::sync::CancellationToken;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const JOB: JobId = JobId(Uuid([0x42; 16]));
const NOW: Timestamp = Timestamp(1_000);

struct Clock;

impl NowSource for Clock {
    fn now(&self) -> Timestamp {
        NOW
    }
}

/// Bytes for the one payload file, served the way the pipeline would.
struct OneFile;

impl FileSource for OneFile {
    fn fetch(
        &self,
        _file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Ok(FileContent {
            file_name: "worksheet.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            bytes: b"%PDF-1.7 body".to_vec(),
        }))
    }
}

#[derive(Default)]
struct FakeState {
    next_id: i64,
    drafts: HashMap<i64, Value>,
    creates_seen: usize,
    reject_create_after: Option<usize>,
    publishes: usize,
}

/// A programmable Tes: every route the adapter's flows hit, answering the
/// measured shapes; the metadata write lands in the stored draft so the
/// read-back verification verifies something real.
struct FakeTes {
    state: Mutex<FakeState>,
}

impl FakeTes {
    fn new(reject_create_after: Option<usize>) -> Self {
        Self {
            state: Mutex::new(FakeState {
                next_id: 9_000,
                reject_create_after,
                ..FakeState::default()
            }),
        }
    }

    fn base_draft(id: i64) -> Value {
        json!({
            "id": id,
            "draft": true,
            "title": "",
            "descriptionRaw": "",
            "descriptionRawType": "markdown",
            "licence": null,
            "categories": [],
            "ageRanges": [],
            "ages": [],
            "yearGroups": [],
            "mainAge": null,
            "mainType": null
        })
    }

    fn presign_body() -> Value {
        let policy = json!({
            "conditions": [
                { "bucket": "fake-bucket" },
                ["starts-with", "$name", ""],
                ["starts-with", "$Content-Type", ""]
            ]
        });
        let encoded = base64::engine::general_purpose::STANDARD.encode(policy.to_string());
        json!([{
            "s3pending": {
                "params": {
                    "policy": encoded,
                    "key": "uploads/fake",
                    "signature": "sig"
                }
            }
        }])
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "the guard IS the fake's whole state transition; holding it across the match is the point"
    )]
    async fn answer(&self, request: &HttpRequest) -> HttpResponse {
        let mut state = self.state.lock().await;
        let url = request.url.as_str();
        let ok = |body: String| HttpResponse::plain(200, body.into_bytes());

        if url.ends_with("/api/v2/resources") && request.method == Method::Post {
            state.creates_seen += 1;
            if let Some(after) = state.reject_create_after {
                if state.creates_seen > after {
                    return HttpResponse::plain(
                        400,
                        json!({"error": "upload refused by the fake"})
                            .to_string()
                            .into_bytes(),
                    );
                }
            }
            state.next_id += 1;
            let id = state.next_id;
            state.drafts.insert(id, Self::base_draft(id));
            return ok(json!({ "id": id }).to_string());
        }
        if url.contains("/api/v2/resources/") && url.ends_with("/draft") {
            let id = path_id(url, "/api/v2/resources/", "/draft");
            match request.method {
                Method::Get => {
                    return state
                        .drafts
                        .get(&id)
                        .map_or(HttpResponse::plain(404, Vec::new()), |draft| {
                            ok(draft.to_string())
                        });
                }
                Method::Post => {
                    if let RequestBody::Json(metadata) = &request.body {
                        let metadata = metadata.clone();
                        if let Some(draft) = state.drafts.get_mut(&id) {
                            for key in ["title", "descriptionRaw", "licence"] {
                                if let Some(value) = metadata.get(key) {
                                    draft[key] = value.clone();
                                }
                            }
                        }
                    }
                    // The measured contract: the write echoes the resource,
                    // and the classifier verifies the echoed id.
                    let echoed = state
                        .drafts
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| json!({ "id": id }));
                    return ok(echoed.to_string());
                }
                Method::Delete => {
                    state.drafts.remove(&id);
                    return HttpResponse::plain(204, Vec::new());
                }
                Method::Put => {}
            }
        }
        if url.contains("/api/resources/v3/draft/") && url.ends_with("/attachment") {
            // Presign and confirm share the route and both send arrays; only
            // the confirm echo carries the s3pending object back.
            let is_confirm = matches!(
                &request.body,
                RequestBody::Json(Value::Array(entries))
                    if entries.first().is_some_and(|entry| entry.get("s3pending").is_some())
            );
            if is_confirm {
                if let RequestBody::Json(Value::Array(echo)) = &request.body {
                    let mut confirmed = echo.clone();
                    if let Some(first) = confirmed.first_mut() {
                        first["isUploaded"] = json!(true);
                    }
                    return ok(Value::Array(confirmed).to_string());
                }
            }
            return ok(Self::presign_body().to_string());
        }
        if url.contains("/api/v2/resources/") && request.method == Method::Get {
            // The authoritative read the delete verification insists on:
            // 404 once the resource is gone, per the measured semantics.
            let id = path_id(url, "/api/v2/resources/", "");
            return state
                .drafts
                .get(&id)
                .map_or(HttpResponse::plain(404, Vec::new()), |draft| {
                    ok(draft.to_string())
                });
        }
        if url.starts_with("https://fake-bucket.s3.amazonaws.com/") {
            return HttpResponse::plain(204, Vec::new());
        }
        if url.ends_with("/publish") {
            state.publishes += 1;
            return ok(json!({}).to_string());
        }
        if request.method == Method::Delete {
            let id = path_id(url, "/api/v2/resources/", "");
            state.drafts.remove(&id);
            return HttpResponse::plain(204, Vec::new());
        }
        HttpResponse::plain(
            500,
            format!(
                "the fake has no route for {} {url}",
                method_name(request.method)
            )
            .into_bytes(),
        )
    }
}

const fn method_name(method: Method) -> &'static str {
    match method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    }
}

fn path_id(url: &str, prefix: &str, suffix: &str) -> i64 {
    url.split(prefix)
        .nth(1)
        .map(|tail| tail.trim_end_matches(suffix).trim_end_matches('/'))
        .and_then(|id| id.parse().ok())
        .unwrap_or(0)
}

impl Transport for &FakeTes {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        Ok(self.answer(&request).await)
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
    TaxonomyRepo::new(pool.clone())
        .seed(
            &[CanonicalTerm {
                id: SUBJECT,
                kind: TermKind::Subject,
                parent: None,
                label: "Maths for early years".to_owned(),
            }],
            &[
                edge(InventoryId::TesGb, "1000454"),
                edge(InventoryId::TesNz, "7000454"),
            ],
        )
        .await
        .expect("the crosswalk seeds");
    let product = tam_domain::CanonicalProduct {
        id: PRODUCT,
        org: ORG,
        title: Title("Fractions practice".to_owned()),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([0x21; 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([0x51; 32]),
                byte_len: 13,
                scan: ScanOutcome::Clean { at: NOW },
            },
            vec![],
        ),
        cover: Some(ProductFile {
            id: FileId(Uuid([0x22; 16])),
            role: FileRole::Cover,
            kind: FileKind::Image,
            hash: ContentHash([0x52; 32]),
            byte_len: 4,
            scan: ScanOutcome::Clean { at: NOW },
        }),
        previews: vec![],
        subjects: vec![SUBJECT],
        grades: tam_domain::GradeDeclaration {
            source: tam_domain::DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
    };
    ProductRepo::new(pool.clone())
        .insert(ORG, &product, NOW)
        .await
        .expect("the product inserts");
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &tam_domain::Mapping {
                id: MAPPING,
                org: ORG,
                product: PRODUCT,
                inventory: InventoryId::TesNz,
                binding: tam_domain::Binding::Unbound,
                policies: tam_domain::FieldPolicies {
                    title: tam_domain::FieldPolicy::Managed,
                    description: tam_domain::FieldPolicy::Managed,
                    price: tam_domain::FieldPolicy::Managed,
                    taxonomy: tam_domain::FieldPolicy::Managed,
                    grades: tam_domain::FieldPolicy::Managed,
                    files: tam_domain::FieldPolicy::Managed,
                },
                price_rule: PriceRule::Explicit(PriceIntent::Free),
                publish: tam_domain::PublishMode::DryRun,
                lifecycle: tam_marketplace::RemoteLifecycle::Absent,
            },
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
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
    .bind(uuid::Uuid::from_bytes([0x33; 16]))
    .execute(&mut *tx)
    .await
    .expect("the connection links");
    tx.commit().await.expect("the fixture commits");

    JobRepo::new(pool.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: JOB,
                inventory: InventoryId::TesNz,
                at: NOW,
            },
            &[NewJobItem {
                item: tam_domain::JobItemId(Uuid([0x41; 16])),
                mapping: MAPPING,
                idempotency_key: tam_marketplace::idempotency::derive_idempotency_key(
                    ORG,
                    InventoryId::TesNz,
                    PRODUCT,
                    1,
                    ContentHash([0x51; 32]),
                ),
                operation: tam_domain::ItemOperation::Create,
            }],
        )
        .await
        .expect("the job enqueues");
}

fn edge(inventory: InventoryId, native: &str) -> ProjectionEdge {
    ProjectionEdge {
        from: SUBJECT,
        to: VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Subject),
            segments: vec!["Maths for early years".to_owned()],
            native_id: Some(native.to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: NOW,
    }
}

/// The engine-role pool over the same per-test database: the lease scan is
/// cross-tenant by construction and sees nothing under the app role's
/// forced row-level security — the same split production runs.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name reads");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[expect(
    clippy::panic,
    reason = "a fixture that cannot project is a broken test, and the message names the gate"
)]
async fn drive(app: &PgPool, fake: &FakeTes) -> RunVerdict {
    let pool = &engine_pool(app).await;
    let leases = LeaseRepo::new(pool.clone());
    let item = leases
        .acquire("gauntlet", NOW, 300)
        .await
        .expect("the acquire runs")
        .expect("the enqueued item leases");
    let projected = match project_for_item(pool, &item, NOW)
        .await
        .expect("the projection runs")
    {
        ProjectionOutcome::Ready(projected) => projected,
        ProjectionOutcome::Blocked { gate, .. } => {
            panic!("the fixture projects; blocked on {gate}")
        }
    };
    let adapter = TesAdapter::new(InventoryId::TesNz, fake, OneFile).expect("a Tes inventory");
    let seed = seed_from_projection(&adapter, &item, &projected).expect("the adapter renders");
    let halts = HaltRepo::new(pool.clone());
    let attempts = WriteAttemptRepo::new(pool.clone());
    let budgets = RateBudgetRepo::new(pool.clone());
    let cancel = CancellationToken::new();
    let ctx = DriverContext {
        adapter: &adapter,
        leases: &leases,
        halts: &halts,
        attempts: &attempts,
        budgets: &budgets,
        pool,
        clock: &Clock,
        cancel: &cancel,
    };
    run_item(&ctx, &item, seed).await.expect("the driver runs")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_clean_run_settles_committed_and_never_touches_publish(pool: PgPool) {
    provision(&pool).await;
    let fake = FakeTes::new(None);
    let verdict = drive(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "the whole flow — create, metadata, presign, S3, confirm, read-back — settled honest"
    );
    let (publishes, landed, minted) = {
        let state = fake.state.lock().await;
        (
            state.publishes,
            state
                .drafts
                .values()
                .any(|draft| draft["title"] == "Fractions practice"),
            state.drafts.keys().copied().max(),
        )
    };
    assert_eq!(publishes, 0, "dry-run never touches publish");
    assert!(
        landed,
        "the metadata the projection produced landed on the marketplace draft"
    );
    let minted = minted.expect("the fake minted a draft for the create");
    let record = MappingRepo::new(pool.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping.binding,
        Binding::Bound {
            id: RemoteListingId::Tes {
                url: format!("https://www.tes.com/api/v2/resources/{minted}"),
            },
            first_seen: NOW,
            verified: Verification::Stale { since: NOW },
        },
        "the run's binding must carry the listing the marketplace minted, stale until \
         the drift verifier reads it back"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refused_create_settles_failed_not_green(pool: PgPool) {
    provision(&pool).await;
    // The pre-flight probe's create succeeds; the real create is refused.
    let fake = FakeTes::new(Some(1));
    let verdict = drive(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Failed),
        "a marketplace refusal is a per-item Failed, never a job-wide lie"
    );
}

/// The rate window closing before a submit leaves an open attempt on the
/// mapping, and `expire_and_steal` settles no orphan: without this settle the
/// re-leased item abandons on `AttemptInFlight` at `RecordIntent` on every
/// pass until it burns its whole retry allowance, having sent nothing.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_exhausted_rate_window_settles_the_attempt_before_it_abandons(pool: PgPool) {
    provision(&pool).await;
    let engine = engine_pool(&pool).await;
    let budgets = RateBudgetRepo::new(engine.clone());
    let connection = tam_types::ConnectionId(Uuid([0x33; 16]));
    let window = Timestamp(NOW.0 - NOW.0.rem_euclid(60_000));
    let ceiling = i32::try_from(OUTBOUND_REQUESTS_PER_MINUTE_MAX.get()).unwrap_or(i32::MAX);
    while budgets
        .consume(ORG, connection, window, ceiling)
        .await
        .expect("the budget consumes")
        != BudgetGrant::Exhausted
    {}

    let fake = FakeTes::new(None);
    let verdict = drive(&pool, &fake).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "a closed rate window abandons the run rather than inventing an outcome"
    );

    let settled: Vec<(String, bool)> =
        sqlx::query_as("SELECT state, settled_at IS NOT NULL FROM write_attempt WHERE org_id = $1")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0))
            .fetch_all(&engine)
            .await
            .expect("the attempt rows read");
    assert_eq!(
        settled,
        vec![("abandoned".to_owned(), true)],
        "the attempt the run opened is settled abandoned, so the mapping is free \
         for the next lease rather than deadlocked on write_attempt_one_in_flight"
    );

    let creates = fake.state.lock().await.creates_seen;
    assert_eq!(
        creates, 1,
        "only the preflight probe's create was sent; the refused submit sent nothing"
    );
}
