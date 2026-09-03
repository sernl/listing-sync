//! The gauntlet: the whole duplication flow from an enqueued item to a
//! settled outcome, through the real lease, the real seed, the real driver
//! and the real Tes adapter — against a programmable fake marketplace whose
//! routes carry the exact shapes the M0 spike measured. The clean run
//! settles Committed without ever touching publish (dry-run is the same
//! code path with the gate shut); the hostile run proves per-item honesty.

#![cfg(feature = "pg-tests")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex;

use base64::Engine as _;
use serde_json::{json, Value};
use sqlx::PgPool;
use tam_domain::equivalence::{
    ElectionAnswer, ElectionRule, ElectionTriggerKind, NewElectionRule, PricingBranch,
};
use tam_domain::{
    Binding, CanonicalTerm, Decider, EdgeKind, ItemOutcome, ProjectionEdge, TermKind, Verification,
    VocabularyId, VocabularyPath,
};
use tam_engine::ledger::{to_wire_item, PgLedger, RandomIds, TokenCancellation};
use tam_engine::seed::{prepare_item, ItemPreparation};
use tam_engine_driver::driver::{run_item, DriverContext, EngineError, NowSource, RunVerdict};
use tam_engine_driver::seed::{seed_for_removal, seed_from_projection};
use tam_limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX;
use tam_marketplace::transport::{
    HttpRequest, HttpResponse, Method, RequestBody, Transport, TransportError,
};
use tam_marketplace::{
    FileContent, FileSource, FileSourceError, LifecycleTransition, ListingState, Pause,
    RemoteLifecycle, RemoteListingId,
};
use tam_marketplace_tes::TesAdapter;
use tam_storage::{
    BudgetGrant, ClaimPolicy, DeviceRef, ElectionRepo, JobRepo, LeaseRepo, MappingRepo, NewJob,
    NewJobItem, ProductRepo, RateBudgetRepo, TaxonomyRepo,
};
use tam_types::{
    Actor, CanonicalTermId, ContentHash, CopyFormat, FileId, FileKind, FileRole, InventoryId,
    JobId, ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Stamp, SystemComponent, Timestamp, Title, UserId, Uuid,
};
use tokio_util::sync::CancellationToken;

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const JOB: JobId = JobId(Uuid([0x42; 16]));
const NOW: Timestamp = Timestamp(1_000);
/// The listing the removal fixture's mapping is already bound to.
const REMOVAL_ID: i64 = 9_001;

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
    /// How many reads of a given resource, after the first, answer 404 for
    /// something that is really there — the platform lag both live runs
    /// measured.
    lag_reads: usize,
    reads_of: HashMap<i64, usize>,
    /// Reads still owed before an accepted publish shows in the resource,
    /// which is the lag the live runs measured. `None` where nothing has
    /// been published.
    publish_pending: Option<usize>,
    publish_lag: usize,
    /// Whether the delete answers its measured 204 without removing
    /// anything — the misleading-204 the read-back exists to catch.
    swallow_deletes: bool,
}

impl FakeState {
    /// Counted per resource and never on the first read of one, because the
    /// first read of the resource a create just made is that create's own:
    /// a create that cannot read what it created fails rather than lags, and
    /// the lag this models begins after it.
    fn lagging_for(&self, id: i64) -> bool {
        let seen = self.reads_of.get(&id).copied().unwrap_or_default();
        self.lag_reads > 0 && seen > 1 && seen - 1 <= self.lag_reads
    }

    /// One read's worth of publish lag. The resource keeps answering
    /// `draft: true` until the owed reads are spent, which is what makes the
    /// verification poll poll rather than settle on its first try.
    ///
    /// Counted on the draft route alone: `resource_state` reads that one
    /// first and only falls through to `/resources/{id}` on a 404, so one
    /// try is one decrement.
    fn advance_publish(&mut self, id: i64) {
        let Some(owed) = self.publish_pending else {
            return;
        };
        let owed = owed.saturating_sub(1);
        self.publish_pending = Some(owed);
        if owed == 0 {
            if let Some(draft) = self.drafts.get_mut(&id) {
                draft["draft"] = json!(false);
            }
        }
    }
}

/// A [`Pause`] that counts instead of waiting, so the poll's shape is
/// asserted at zero wall time.
#[derive(Default)]
struct CountingPause {
    pauses: AtomicU32,
}

impl Pause for CountingPause {
    fn pause(&self, _ms: u32) -> impl core::future::Future<Output = ()> + Send {
        self.pauses.fetch_add(1, Ordering::Relaxed);
        core::future::ready(())
    }
}

/// A programmable Tes: every route the adapter's flows hit, answering the
/// measured shapes; the metadata write lands in the stored draft so the
/// read-back verification verifies something real.
struct FakeTes {
    state: Mutex<FakeState>,
}

impl FakeTes {
    /// Whether the fake still holds a resource, which is how a test asserts
    /// that no removal reached it.
    async fn still_holds(&self, id: i64) -> bool {
        self.state.lock().await.drafts.contains_key(&id)
    }

    fn new(reject_create_after: Option<usize>) -> Self {
        Self {
            state: Mutex::new(FakeState {
                next_id: 9_000,
                reject_create_after,
                ..FakeState::default()
            }),
        }
    }

    /// A fake whose reads lag a create by `lag_reads` answers.
    fn lagging(lag_reads: usize) -> Self {
        Self {
            state: Mutex::new(FakeState {
                next_id: 9_000,
                lag_reads,
                ..FakeState::default()
            }),
        }
    }

    /// A fake already holding the listing a removal will take down.
    fn holding(id: i64) -> Self {
        Self::holding_with(id, 0)
    }

    /// A fake holding `id` whose publish shows in the resource only after
    /// `lag` further reads.
    fn publishing(id: i64, lag: usize) -> Self {
        Self::holding_with(id, lag)
    }

    /// A fake holding `id` that accepts the publish and never shows it. The
    /// lag outlasts the whole try budget, which is the case the poll cannot
    /// distinguish from a publish that will arrive one read later — and so
    /// the case it must refuse to call a success.
    fn never_publishing(id: i64) -> Self {
        Self::holding_with(id, usize::MAX)
    }

    /// A fake holding `id` whose delete answers 204 and removes nothing,
    /// which is the misleading-204 measured in M0.
    fn swallowing(id: i64) -> Self {
        let fake = Self::holding_with(id, 0);
        Self {
            state: Mutex::new(FakeState {
                swallow_deletes: true,
                ..fake.state.into_inner()
            }),
        }
    }

    fn holding_with(id: i64, publish_lag: usize) -> Self {
        let mut drafts = HashMap::new();
        drafts.insert(id, Self::base_draft(id));
        Self {
            state: Mutex::new(FakeState {
                next_id: id,
                drafts,
                publish_lag,
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
                    *state.reads_of.entry(id).or_default() += 1;
                    if state.lagging_for(id) {
                        return HttpResponse::plain(404, Vec::new());
                    }
                    state.advance_publish(id);
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
                    if !state.swallow_deletes {
                        state.drafts.remove(&id);
                    }
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
            // 404 once the resource is gone, per the measured semantics. It
            // lags with the draft route rather than independently: a
            // resource_state 404 means *neither* route answered, which is the
            // union the live runner proves a deletion with.
            let id = path_id(url, "/api/v2/resources/", "");
            if state.lagging_for(id) {
                return HttpResponse::plain(404, Vec::new());
            }
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
            // Accepted here and visible in the resource `publish_lag` reads
            // later, which is the shape both live runs measured: the POST
            // answers long before the read agrees. One read is the floor,
            // because a publish nothing ever reads back is not observable.
            state.publish_pending = Some(state.publish_lag.max(1));
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

async fn provision(pool: &PgPool) {
    provision_with(pool, Fixture::default()).await;
}

/// What varies between the gauntlet's runs: the operation the item carries,
/// the binding and lifecycle its mapping starts in, and whether the
/// crosswalk covers the inventory the job targets.
struct Fixture {
    operation: tam_domain::ItemOperation,
    binding: Binding,
    lifecycle: RemoteLifecycle,
    crosswalked: bool,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            operation: tam_domain::ItemOperation::Create,
            binding: Binding::Unbound,
            lifecycle: RemoteLifecycle::Absent,
            crosswalked: true,
        }
    }
}

impl Fixture {
    /// The mapping already holds the listing `REMOVAL_ID` names, in the
    /// state the removal states it will delete from.
    fn removal(crosswalked: bool) -> Self {
        Self {
            operation: tam_domain::ItemOperation::Remove {
                subject: removal_subject(),
                state: ListingState::Draft,
            },
            binding: Binding::Bound {
                id: removal_subject(),
                first_seen: NOW,
                verified: Verification::Stale { since: NOW },
            },
            lifecycle: RemoteLifecycle::Draft,
            crosswalked,
        }
    }
}

impl Fixture {
    /// The mapping already holds `REMOVAL_ID` and records it as a draft;
    /// the item is the publish that takes it live.
    fn publication() -> Self {
        Self {
            operation: tam_domain::ItemOperation::Revise {
                subject: removal_subject(),
                transition: LifecycleTransition {
                    from: ListingState::Draft,
                    to: ListingState::Live,
                },
            },
            binding: Binding::Bound {
                id: removal_subject(),
                first_seen: NOW,
                verified: Verification::Stale { since: NOW },
            },
            lifecycle: RemoteLifecycle::Draft,
            crosswalked: true,
        }
    }
}

fn removal_subject() -> RemoteListingId {
    RemoteListingId::Tes {
        url: format!("https://www.tes.com/api/v2/resources/{REMOVAL_ID}"),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision_with(pool: &PgPool, fixture: Fixture) {
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
            &if fixture.crosswalked {
                vec![
                    edge(InventoryId::TesGb, "1000454"),
                    edge(InventoryId::TesNz, "7000454"),
                ]
            } else {
                // No NZ counterpart: a create would park on reconciliation,
                // which is exactly what a removal must not do.
                vec![edge(InventoryId::TesGb, "1000454")]
            },
        )
        .await
        .expect("the crosswalk seeds");
    // The seller's standing licence policy. Tes declares the licence
    // required, so without one every create here parks on the election
    // instead of exercising the write path the gauntlet is about.
    let elections = ElectionRepo::new(pool.clone());
    for inventory in [InventoryId::TesGb, InventoryId::TesNz] {
        let rule = ElectionRule::new(NewElectionRule {
            org: ORG,
            inventory,
            axis: TermKind::Licence,
            trigger_kind: ElectionTriggerKind::Supply,
            trigger_key: Some(PricingBranch::Free.as_str().to_owned()),
            answer: ElectionAnswer::Value {
                path: VocabularyPath {
                    vocabulary: VocabularyId(inventory, TermKind::Licence),
                    segments: vec!["CC-BY-SA".to_owned()],
                    native_id: Some("CC-BY-SA".to_owned()),
                },
            },
            decided_by: Decider::Human {
                user: UserId(Uuid([0x5E; 16])),
                org: ORG,
            },
            decided_at: NOW,
        })
        .expect("a licence value is a legal answer; only delegation is not");
        elections.upsert_rule(&rule).await.expect("the rule seeds");
    }
    let product = tam_domain::CanonicalProduct {
        id: PRODUCT,
        org: ORG,
        title: Title("Fractions practice".to_owned()),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
            format: CopyFormat::Markdown,
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
        rights: tam_domain::RightsDeclaration::Unstated,
        native_residue: vec![],
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
                binding: fixture.binding,
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
                lifecycle: fixture.lifecycle,
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
                stamp: Stamp {
                    at: NOW,
                    actor: Actor::System(SystemComponent::Engine),
                },
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
                operation: fixture.operation,
                requires_bound_on: None,
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

async fn drive(app: &PgPool, fake: &FakeTes) -> RunVerdict {
    drive_counting(app, fake).await.0
}

/// Whether the item's lease is stolen out from under the run before it
/// starts, the way `expire_and_steal` does to a stalled worker: the epoch is
/// bumped and the item settled, so every later epoch-fenced item write finds
/// no row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lease {
    Held,
    Stolen,
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn drive_counting(app: &PgPool, fake: &FakeTes) -> (RunVerdict, u32) {
    let (verdict, pauses) = pump(app, fake, Lease::Held).await;
    (verdict.expect("the driver runs"), pauses)
}

/// The whole pump for one item, with the poll's pauses counted: lease,
/// prepare, seed, drive. A removal takes `seed_for_removal`, which is the
/// branch that keeps a taxonomy gap from parking a delete. The run's own
/// verdict travels as a `Result`, because the stolen-lease case fences the
/// item settle and has no verdict to report.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
#[expect(
    clippy::panic,
    reason = "a fixture that cannot be prepared is a broken test, and the message names the gate"
)]
async fn pump(
    app: &PgPool,
    fake: &FakeTes,
    lease_state: Lease,
) -> (Result<RunVerdict, EngineError>, u32) {
    let pool = &engine_pool(app).await;
    let item = claim(app, DEVICE, i64::from(tam_domain::LEASE_TTL_SECS))
        .await
        .expect("the enqueued item leases");
    if lease_state == Lease::Stolen {
        sqlx::query(
            "UPDATE job_item \
             SET state = 'settled', outcome = 'failed', failure_code = 'Other', \
                 settled_at = now(), lease_owner = NULL, lease_expires_at = NULL, \
                 lease_epoch = lease_epoch + 1 \
             WHERE org_id = $1 AND id = $2",
        )
        .bind(uuid::Uuid::from_bytes(item.org.0 .0))
        .bind(uuid::Uuid::from_bytes(item.item.0 .0))
        .execute(pool)
        .await
        .expect("the steal settles the item and bumps the epoch");
    }
    let (operation, projected) = match prepare_item(pool, &item, NOW)
        .await
        .expect("the preparation runs")
    {
        ItemPreparation::Ready {
            operation,
            projected,
        } => (operation, projected),
        ItemPreparation::Blocked { gate, .. } => {
            panic!("the fixture prepares; blocked on {gate}")
        }
        ItemPreparation::CounterpartLost { counterpart } => {
            panic!("the fixture's counterpart binds; {counterpart:?} did not")
        }
    };
    let adapter = TesAdapter::new(InventoryId::TesNz, fake, OneFile).expect("a Tes inventory");
    let prepared = tam_engine::seed::preparation(&item, operation.clone(), projected.clone());
    let seed = prepared.projected.as_ref().map_or_else(
        || seed_for_removal(&prepared),
        |listing| seed_from_projection(&adapter, &prepared, listing).expect("the adapter renders"),
    );
    let cancel = CancellationToken::new();
    let pause = CountingPause::default();
    let ledger = PgLedger::new(pool.clone(), item.job);
    let cancel = TokenCancellation(&cancel);
    let ctx = DriverContext {
        adapter: &adapter,
        ledger: &ledger,
        clock: &Clock,
        ids: &RandomIds,
        cancel: &cancel,
        pause: &pause,
    };
    let verdict = run_item(&ctx, &to_wire_item(&item), seed).await;
    (verdict, pause.pauses.load(Ordering::Relaxed))
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

/// The lag the poll exists for. `FakeTes` answers the read-back 404 for the
/// three reads after the create's own, then the draft: a single-shot
/// read-back — today's behaviour — would settle this run Ambiguous and halt
/// the tenant's inventory on a listing that is really there.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_create_whose_read_lags_settles_committed_after_polling(pool: PgPool) {
    provision(&pool).await;
    let fake = FakeTes::lagging(3);
    let (verdict, pauses) = drive_counting(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "the write landed and the poll saw it; the lag is not an outcome"
    );
    assert_eq!(
        pauses, 3,
        "one pause per lagged read and none after the read that answered: a poll that \
         paused after its last try would spend interval_ms of the lease for nothing"
    );
    let record = MappingRepo::new(pool.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert!(
        matches!(record.mapping.binding, Binding::Bound { .. }),
        "a run that polled through the lag binds, or the poll bought nothing"
    );
    assert_eq!(
        record.mapping.lifecycle,
        RemoteLifecycle::Draft,
        "the bound lifecycle is the one the verification read observed, not one derived \
         from the adapter's create convention"
    );
}

/// The other side of the same branch: a poll that settles Committed when its
/// budget runs out would record a listing we never saw.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_create_that_never_appears_settles_ambiguous_and_halts(pool: PgPool) {
    provision(&pool).await;
    let fake = FakeTes::lagging(usize::MAX);
    let (verdict, pauses) = drive_counting(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Ambiguous),
        "the write may have landed and we cannot see it, which is Ambiguous and never a \
         silent commit"
    );
    assert_eq!(
        pauses, 7,
        "the Tes policy's eight tries pause between them and not after the last"
    );
    let engine = engine_pool(&pool).await;
    let halts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt WHERE org_id = $1")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0))
            .fetch_one(&engine)
            .await
            .expect("the halt rows read");
    assert_eq!(
        halts, 1,
        "an unverifiable create halts this tenant's inventory"
    );
    let record = MappingRepo::new(pool.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping.binding,
        Binding::Unbound,
        "nothing was observed, so nothing is bound"
    );
}

/// A grant per `read_back` call, not per HTTP request: a Tes read is one or
/// two requests, because `resource_state` reads the draft route and falls
/// through, so this pins the accounting the driver actually does rather than
/// a request ceiling it does not enforce.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn the_poll_consumes_one_grant_per_read_back_call(pool: PgPool) {
    provision(&pool).await;
    let fake = FakeTes::lagging(3);
    let verdict = drive(&pool, &fake).await;
    assert_eq!(verdict, RunVerdict::Settled(ItemOutcome::Succeeded));
    let engine = engine_pool(&pool).await;
    let used: i32 = sqlx::query_scalar("SELECT actions_used FROM rate_budget WHERE org_id = $1")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .fetch_one(&engine)
        .await
        .expect("the budget row reads");
    assert_eq!(
        used, 5,
        "one grant for the submit and one for each of the four reads; a poll placed \
         outside the budget would leave this at 1 and spend the connection's ceiling \
         invisibly"
    );
}

/// The rate window closing mid-poll is evidence about us, not about the
/// listing. Handing the machine the last answer instead would feed it
/// `Absent` during lag, which under the create polarity halts the tenant's
/// inventory on pure throughput; abandoning without settling would deadlock
/// the mapping on `write_attempt_one_in_flight` until `ATTEMPTS_MAX`.
/// Leaves exactly one grant in the connection's minute window, so the write
/// takes it and the poll's first read finds none.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn exhaust_all_but_one_grant(engine: &PgPool) {
    let budgets = RateBudgetRepo::new(engine.clone());
    let connection = tam_types::ConnectionId(Uuid([0x33; 16]));
    let window = Timestamp(NOW.0 - NOW.0.rem_euclid(60_000));
    let ceiling = i32::try_from(OUTBOUND_REQUESTS_PER_MINUTE_MAX.get()).unwrap_or(i32::MAX);
    for _ in 1..ceiling {
        assert_ne!(
            budgets
                .consume(ORG, connection, window, ceiling)
                .await
                .expect("the budget consumes"),
            BudgetGrant::Exhausted,
            "the fixture must leave exactly one grant for the write"
        );
    }
}

/// Requeues the item the way `expire_and_steal` does behind a worker whose
/// run abandoned, so a second pass can be driven over the same fixture.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn requeue(engine: &PgPool) {
    // The lease expiry is the database's own fact now, so the fixture ages the
    // row rather than advancing a clock the statement no longer reads.
    sqlx::query("UPDATE job_item SET lease_expires_at = now() - interval '1 hour'")
        .execute(engine)
        .await
        .expect("the lease ages");
    let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
    LeaseRepo::new(engine.clone())
        .expire_and_steal(Timestamp(NOW.0 + 400_000), attempts_max)
        .await
        .expect("the steal runs");
}

/// The create's fence, which is the one this system cannot replace: neither
/// marketplace offers an idempotent create, so `write_attempt_one_in_flight`
/// is all that stands between a requeued item and a second listing on the
/// seller's store. A run that walked away from an unverified create leaves
/// its attempt standing, and the next pass abandons on `AttemptInFlight`
/// rather than creating again.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_rate_window_closing_mid_create_holds_the_duplicate_fence(pool: PgPool) {
    provision(&pool).await;
    let engine = engine_pool(&pool).await;
    exhaust_all_but_one_grant(&engine).await;

    let fake = FakeTes::lagging(3);
    let verdict = drive(&pool, &fake).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "we stopped looking; the run abandons into the stealer rather than inventing an outcome"
    );
    let landed = { fake.state.lock().await.drafts.keys().copied().max() };
    let landed = landed.expect("the create reached the fake before the window closed");

    let settled: Vec<(String, bool)> =
        sqlx::query_as("SELECT state, settled_at IS NOT NULL FROM write_attempt WHERE org_id = $1")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0))
            .fetch_all(&engine)
            .await
            .expect("the attempt rows read");
    assert_eq!(
        settled,
        vec![("in_flight".to_owned(), false)],
        "the create went out unverified, so its attempt is held: settling it would release \
         the only fence there is against making the listing twice"
    );

    requeue(&engine).await;
    let (second, _) = pump(&pool, &fake, Lease::Held).await;
    assert!(
        matches!(second, Ok(RunVerdict::Abandoned { .. })),
        "the requeued item abandons on the held fence rather than crashing: {second:?}"
    );
    let mut after: Vec<i64> = {
        let ids = fake.state.lock().await.drafts.keys().copied().collect();
        ids
    };
    after.sort_unstable();
    assert_eq!(
        after,
        vec![landed],
        "the second pass minted no second listing; the schema probe it does make is \
         created and deleted within the preflight"
    );

    let record = MappingRepo::new(pool.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping.binding,
        Binding::Unbound,
        "an unverified write binds nothing"
    );
}

/// A removal addresses a listing that already exists, so re-running it mints
/// nothing: its attempt settles, and the settled row names what the write
/// was about rather than nulling the columns that say so.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_rate_window_closing_mid_removal_settles_the_attempt_ambiguous(pool: PgPool) {
    provision_with(&pool, Fixture::removal(true)).await;
    let engine = engine_pool(&pool).await;
    exhaust_all_but_one_grant(&engine).await;

    let fake = FakeTes::holding(REMOVAL_ID);
    let verdict = drive(&pool, &fake).await;
    assert!(
        matches!(verdict, RunVerdict::Abandoned { .. }),
        "we stopped looking; the run abandons into the stealer rather than inventing an outcome"
    );

    let settled: Vec<(String, bool, Option<String>)> = sqlx::query_as(
        "SELECT state, settled_at IS NOT NULL, remote_url FROM write_attempt WHERE org_id = $1",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .fetch_all(&engine)
    .await
    .expect("the attempt rows read");
    assert_eq!(
        settled,
        vec![(
            "ambiguous".to_owned(),
            true,
            Some(format!("https://www.tes.com/api/v2/resources/{REMOVAL_ID}"))
        )],
        "the delete went out and could not be verified, which is ambiguous rather than \
         abandoned; and the row names the listing it addressed, so an operator reading the \
         ledger is not left guessing"
    );
    let record = MappingRepo::new(pool.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert!(
        matches!(record.mapping.binding, Binding::Bound { .. }),
        "an unverified removal severs nothing; the mapping still holds the listing"
    );
}

/// A removal end to end: the column, the effect, the adapter's delete, the
/// absence the poll verifies, and the sever the verdict writes.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_settles_succeeded_and_severs(pool: PgPool) {
    provision_with(&pool, Fixture::removal(true)).await;
    let fake = FakeTes::holding(REMOVAL_ID);
    let verdict = drive(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "a removal whose verification read finds nothing is a committed removal"
    );
    let gone = { !fake.state.lock().await.drafts.contains_key(&REMOVAL_ID) };
    assert!(gone, "the delete reached the marketplace");

    let engine = engine_pool(&pool).await;
    let attempt: (String, Option<String>) =
        sqlx::query_as("SELECT state, remote_url FROM write_attempt WHERE org_id = $1")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0))
            .fetch_one(&engine)
            .await
            .expect("the attempt row reads");
    assert_eq!(
        (attempt.0.as_str(), attempt.1.as_deref()),
        (
            "committed",
            Some(format!("https://www.tes.com/api/v2/resources/{REMOVAL_ID}").as_str())
        ),
        "the settled attempt names the listing it removed"
    );
    let severed: (String, Option<String>, String) = sqlx::query_as(
        "SELECT binding_state, sever_cause, lifecycle_state FROM mapping \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(MAPPING.0 .0))
    .fetch_one(&engine)
    .await
    .expect("the mapping row reads");
    assert_eq!(
        (severed.0.as_str(), severed.1.as_deref(), severed.2.as_str()),
        ("severed", Some("removed_by_seller"), "absent"),
        "a committed removal severs; binding it would record the mapping against a \
         listing that no longer exists"
    );
}

/// A removal describes nothing, so it never reaches the projection. Leaving
/// `pump_item` projecting before it branches would make every delete of an
/// imperfectly-mapped product unrunnable — the listing is being taken down,
/// not described.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_never_projects(pool: PgPool) {
    provision_with(&pool, Fixture::removal(false)).await;
    let fake = FakeTes::holding(REMOVAL_ID);
    let verdict = drive(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "the product's subject has no counterpart in this inventory, which parks a create \
         on reconciliation and must not park a delete"
    );
    let open = TaxonomyRepo::new(pool)
        .open_items(ORG)
        .await
        .expect("the queue reads");
    assert!(
        open.is_empty(),
        "a removal raises no reconciliation item either: nothing about it needs mapping"
    );
}

/// A stolen lease is discovered at the heartbeat, before any request goes out.
///
/// This test used to record a divergence: `write_attempt.lease_epoch` is
/// written and compared from the same `LeaseRef`, so a stalled worker whose
/// lease `expire_and_steal` had already stolen still settled its attempt and
/// severed the mapping, with only the item settle fenced against the bumped
/// epoch and running after the sever had landed.
///
/// The heartbeat narrows that. The interpreter renews the lease immediately
/// before every network-bearing effect, so a run whose lease was stolen before
/// it reached one now stops there — no request is issued and the mapping is
/// untouched. That is the stall bias applied to a hazard the design had
/// recorded rather than fenced.
///
/// It is narrowed rather than closed, and the difference is worth stating: a
/// steal landing between the last heartbeat and the settle still reaches the
/// old path, and `SeveredAfterSteal` still exists for it. What has gone is the
/// wide window in which a run could discover the steal only after writing to a
/// marketplace.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_stolen_lease_is_caught_at_the_heartbeat_before_any_request(pool: PgPool) {
    provision_with(&pool, Fixture::removal(true)).await;
    let fake = FakeTes::holding(REMOVAL_ID);
    let (verdict, _) = pump(&pool, &fake, Lease::Stolen).await;
    assert!(
        verdict.is_err(),
        "the run cannot report a verdict on an item it no longer holds"
    );

    let engine = engine_pool(&pool).await;
    let binding: String =
        sqlx::query_scalar("SELECT binding_state FROM mapping WHERE org_id = $1 AND id = $2")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0))
            .bind(uuid::Uuid::from_bytes(MAPPING.0 .0))
            .fetch_one(&engine)
            .await
            .expect("the mapping row reads");
    assert_eq!(
        binding, "bound",
        "the heartbeat refused before the removal went out, so the mapping is untouched \
         rather than severed by a run that no longer held the item"
    );
    assert!(
        fake.still_holds(REMOVAL_ID).await,
        "and the listing is still there: nothing reached the marketplace, which is the \
         whole point of renewing before the slow part rather than after it"
    );
}

/// The same fence on the create path, where the first marketplace request is
/// the form scrape rather than the write.
///
/// `AssertFormSchema` is write-bearing on Tes: it creates a probe draft on the
/// seller's own account. It is also the first effect of every create, so a run
/// whose lease was stolen while the device was idle would otherwise post to the
/// marketplace under a lease it no longer held, and find out only at the
/// attempt it opened afterwards.
///
/// `creates_seen` is the discriminator, not `drafts`: the adapter deletes its
/// probe, so the draft map comes back empty whether or not the scrape ran,
/// while the counter only ever increments.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_stolen_lease_on_a_create_is_caught_before_the_form_scrape(pool: PgPool) {
    provision(&pool).await;
    let fake = FakeTes::new(None);
    let (verdict, _) = pump(&pool, &fake, Lease::Stolen).await;
    assert!(
        verdict.is_err(),
        "the run cannot report a verdict on an item it no longer holds"
    );

    let (drafts, creates) = {
        let state = fake.state.lock().await;
        (state.drafts.len(), state.creates_seen)
    };
    assert_eq!(
        creates, 0,
        "the heartbeat refused before the form scrape, so nothing was posted to the \
         seller's account under a lease the run had already lost"
    );
    assert_eq!(
        drafts, 0,
        "and nothing was left behind either, which the probe's own delete would also \
         achieve — this is the weaker of the two assertions, kept because it is free"
    );
}

/// The publish path end to end, which nothing drove before: the column, the
/// machine's `Effect::Revise`, the adapter's publish POST, the poll that
/// waits for the resource to agree, and the lifecycle the settle records.
///
/// The predicate is the point. A publish is proved by observing `Live`, not
/// by observing anything at all, and the mapping must come out reading
/// 'live' — without which the next item on it parks on `lifecycle_diverged`
/// for good.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_publish_that_lags_settles_succeeded_and_binds_live(pool: PgPool) {
    provision_with(&pool, Fixture::publication()).await;
    let fake = FakeTes::publishing(REMOVAL_ID, 3);
    let (verdict, pauses) = drive_counting(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Succeeded),
        "a publish whose resource catches up inside the budget is a committed publish"
    );
    assert!(
        pauses > 0,
        "the resource answered 'draft' first, so the poll had to wait for it"
    );
    let publishes = { fake.state.lock().await.publishes };
    assert_eq!(
        publishes, 1,
        "the publish reached the marketplace exactly once"
    );

    let engine = engine_pool(&pool).await;
    let attempt: (String, Option<String>) =
        sqlx::query_as("SELECT state, remote_url FROM write_attempt WHERE org_id = $1")
            .bind(uuid::Uuid::from_bytes(ORG.0 .0))
            .fetch_one(&engine)
            .await
            .expect("the attempt row reads");
    assert_eq!(
        (attempt.0.as_str(), attempt.1.as_deref()),
        (
            "committed",
            Some(format!("https://www.tes.com/api/v2/resources/{REMOVAL_ID}").as_str())
        ),
        "the settled attempt names the listing it published"
    );
    let mapping: (String, String, bool) = sqlx::query_as(
        "SELECT binding_state, lifecycle_state, lifecycle_since IS NOT NULL FROM mapping \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(MAPPING.0 .0))
    .fetch_one(&engine)
    .await
    .expect("the mapping row reads");
    assert_eq!(
        (mapping.0.as_str(), mapping.1.as_str(), mapping.2),
        ("bound", "live", true),
        "the binding is unchanged and the lifecycle is the one the read observed; a bind \
         that refuses to write it leaves every later item on this mapping parked"
    );
}

/// The same publish, never agreed to by the resource. The poll exhausts on a
/// draft observation, which is a write we could not confirm rather than a
/// success: settling it Succeeded would report a published listing to the
/// seller and record the mapping 'draft' in the same breath.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_publish_the_read_never_confirms_settles_ambiguous(pool: PgPool) {
    provision_with(&pool, Fixture::publication()).await;
    let fake = FakeTes::never_publishing(REMOVAL_ID);
    let verdict = drive(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Ambiguous),
        "the publish POST was accepted and the resource never said so; the stall bias \
         refuses to call that a success"
    );

    let engine = engine_pool(&pool).await;
    let halted: i64 = sqlx::query_scalar("SELECT count(*) FROM org_inventory_halt")
        .fetch_one(&engine)
        .await
        .expect("the halt reads");
    assert_eq!(
        halted, 1,
        "an unresolved ambiguity halts the tenant's inventory"
    );
    let mapping: (String, String) = sqlx::query_as(
        "SELECT binding_state, lifecycle_state FROM mapping WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(MAPPING.0 .0))
    .fetch_one(&engine)
    .await
    .expect("the mapping row reads");
    assert_eq!(
        (mapping.0.as_str(), mapping.1.as_str()),
        ("bound", "draft"),
        "nothing was proved, so nothing about the listing is rewritten"
    );
}

/// A removal the marketplace did not take. The delete answers its measured
/// 204, the absence poll still finds the listing, and the item settles
/// Failed — the one unproved read that is a refusal rather than an
/// ambiguity, because the ledger can act on it.
///
/// What the settled row must not lose is which listing the delete failed to
/// remove, which is the whole reason `LandingEffect::Addressed` exists; and
/// the mapping must still hold that listing, because severing a mapping
/// whose listing is still live is how the next create mints a duplicate.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_that_does_not_take_records_what_it_addressed(pool: PgPool) {
    provision_with(&pool, Fixture::removal(true)).await;
    let fake = FakeTes::swallowing(REMOVAL_ID);
    let verdict = drive(&pool, &fake).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(ItemOutcome::Failed),
        "the delete answered 204 and removed nothing; the read-back is what catches that"
    );
    let still_there = { fake.state.lock().await.drafts.contains_key(&REMOVAL_ID) };
    assert!(still_there, "the fixture's delete really did swallow");

    let engine = engine_pool(&pool).await;
    let attempt: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT state, failure_code, remote_url FROM write_attempt WHERE org_id = $1",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .fetch_one(&engine)
    .await
    .expect("the attempt row reads");
    assert_eq!(
        (
            attempt.0.as_str(),
            attempt.1.as_deref(),
            attempt.2.as_deref()
        ),
        (
            "failed",
            Some("VerificationMismatch"),
            Some(format!("https://www.tes.com/api/v2/resources/{REMOVAL_ID}").as_str())
        ),
        "a failed removal says what it failed to remove; a row that named nothing would \
         leave an operator with a failure and no subject"
    );
    let mapping: (String, Option<String>, String) = sqlx::query_as(
        "SELECT binding_state, sever_cause, lifecycle_state FROM mapping \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(MAPPING.0 .0))
    .fetch_one(&engine)
    .await
    .expect("the mapping row reads");
    assert_eq!(
        (mapping.0.as_str(), mapping.1.as_deref(), mapping.2.as_str()),
        ("bound", None, "draft"),
        "the listing is still on the seller's store, so the mapping still holds it: \
         severing here releases the bound claim and the next create mints a duplicate \
         the ledger cannot reconcile"
    );
}

/// The seller's device, which the seller-device claim admits only if it is
/// registered and unrevoked.
const DEVICE: &str = "engine-test-device";

/// Tes is the seller-device branch, so a fixture that means to lease claims as
/// a device rather than through `acquire`, which no longer sees these items.
/// The claim is org-pinned by forced row-level security, so it runs on the app
/// pool; the engine pool is BYPASSRLS and would not be pinned by it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn claim(app: &PgPool, device: &str, ttl: i64) -> Option<tam_storage::LeasedItem> {
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, \
                             first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'fixture', 'linux', 'x86_64', '0.0.0', now(), now()) \
         ON CONFLICT (org_id, id) DO NOTHING",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(device)
    .execute(&mut *tx)
    .await
    .expect("the fixture device registers");
    tx.commit().await.expect("the fixture device commits");
    match tam_storage::LeaseRepo::new(app.clone())
        .claim_for_device(
            &DeviceRef { org: ORG, device },
            &ClaimPolicy {
                ttl_seconds: ttl,
                grace_hours: 24,
                marketplace: None,
            },
            NOW,
        )
        .await
        .expect("the claim runs")
    {
        tam_storage::DeviceClaim::Leased(item) => Some(*item),
        tam_storage::DeviceClaim::Empty | tam_storage::DeviceClaim::HeldByAnotherDevice => None,
    }
}
