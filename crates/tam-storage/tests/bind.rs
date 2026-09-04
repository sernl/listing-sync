//! The bind folded into the attempt settle, proven over the tam_engine role:
//! a landed write binds an unbound mapping and leaves it stale, re-landing
//! the same identifier preserves `first_seen_at`, a different identifier is
//! reported rather than written over the first, and one remote listing
//! cannot be claimed twice inside an organisation's inventory.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    ItemOperation, ItemOutcome, JobItemId, Mapping, PublishMode, Verification,
};
use tam_marketplace::{IdempotencyKey, ListingState, RemoteLifecycle, RemoteListingId};
use tam_storage::{
    AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, ClaimPolicy, DeviceClaim,
    DeviceRef, ItemVerdict, JobRepo, LandingEffect, LeaseRef, LeaseRepo, MappingRepo, NewAttempt,
    NewJob, NewJobItem, ProductRepo, StorageError, WriteAttemptRepo,
};
use tam_types::{
    Actor, CanonicalTermId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole,
    InventoryId, JobId, ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule,
    ProductFile, ProductId, ScanOutcome, Stamp, SystemComponent, Timestamp, Title, Uuid,
};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const LANDED_AT: Timestamp = Timestamp(1_756_000_001_000);
const RELANDED_AT: Timestamp = Timestamp(1_756_000_002_000);

const ORG: OrgId = OrgId(Uuid([0xC1; 16]));
const JOB: JobId = JobId(Uuid([0xC2; 16]));
const ITEM: JobItemId = JobItemId(Uuid([0xC3; 16]));
const MAPPING: MappingId = MappingId(Uuid([0xC4; 16]));
const RIVAL: MappingId = MappingId(Uuid([0xC5; 16]));
const RIVAL_JOB: JobId = JobId(Uuid([0xCC; 16]));
const RIVAL_ITEM: JobItemId = JobItemId(Uuid([0xCD; 16]));
const WAITING_JOB: JobId = JobId(Uuid([0xE1; 16]));
const WAITING_ITEM: JobItemId = JobItemId(Uuid([0xE2; 16]));
const BYSTANDER_JOB: JobId = JobId(Uuid([0xE4; 16]));
const BYSTANDER_ITEM: JobItemId = JobItemId(Uuid([0xE5; 16]));
const PRODUCT: u8 = 0xC6;
const RIVAL_PRODUCT: u8 = 0xD0;

const LANDED: &str = "https://www.tes.com/teaching-resource/fractions-9001";
const ELSEWHERE: &str = "https://www.tes.com/teaching-resource/fractions-9002";

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

fn tes(url: &str) -> RemoteListingId {
    RemoteListingId::Tes {
        url: url.to_owned(),
    }
}

fn product(seed: u8) -> CanonicalProduct {
    CanonicalProduct {
        id: ProductId(Uuid([seed; 16])),
        org: ORG,
        title: Title("Fixture".to_owned()),
        body: ListingCopy {
            body: "Fixture".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([seed.wrapping_add(1); 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Held {
                    hash: ContentHash([seed; 32]),
                    byte_len: 4,
                    scan: ScanOutcome::Pending,
                },
            },
            vec![],
        ),
        cover: None,
        previews: vec![],
        subjects: Vec::<CanonicalTermId>::new(),
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: tam_domain::RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

fn mapping_of(id: MappingId, product_seed: u8, binding: Binding) -> Mapping {
    Mapping {
        id,
        org: ORG,
        product: ProductId(Uuid([product_seed; 16])),
        inventory: InventoryId::TesGb,
        binding,
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

/// The whole mapping a landed create leaves behind: bound and stale, and
/// carrying the lifecycle the verification read observed. The bind writes the
/// lifecycle columns now, so a fixture still expecting `Absent` here would be
/// asserting the gap this closes -- a mapping whose listing was published
/// reading 'absent' is what makes a later removal state the wrong route.
fn landed_mapping(id: MappingId, product_seed: u8, url: &str, at: Timestamp) -> Mapping {
    Mapping {
        lifecycle: RemoteLifecycle::Draft,
        ..mapping_of(id, product_seed, freshly_bound(url, at))
    }
}

/// The binding a fresh insert would hold for a listing landed at `at`: bound,
/// and stale because the report handed to the settle was never normalised.
fn freshly_bound(url: &str, at: Timestamp) -> Binding {
    Binding::Bound {
        id: tes(url),
        first_seen: at,
        verified: Verification::Stale { since: at },
    }
}

/// The engine connects as its own role; the per-test database name comes from
/// the app pool. Host and port are the dev database's, same as DATABASE_URL.
async fn engine_pool(app: &PgPool) -> Result<PgPool, sqlx::Error> {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await?;
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
}

async fn seed(app: &PgPool, engine: &PgPool) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-bind', now())")
        .bind(db_uuid(ORG.0))
        .execute(app)
        .await?;
    ProductRepo::new(app.clone())
        .insert(ORG, &product(PRODUCT), T0)
        .await?;
    MappingRepo::new(app.clone())
        .insert(ORG, &mapping_of(MAPPING, PRODUCT, Binding::Unbound), 0, T0)
        .await?;
    let mut tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG.0).to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(Uuid([0xC8; 16])))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: JOB,
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[NewJobItem {
                item: ITEM,
                mapping: MAPPING,
                idempotency_key: IdempotencyKey(Uuid([0xC9; 16])),
                operation: ItemOperation::Create,
                requires_bound_on: None,
            }],
        )
        .await?;
    // Tes is the seller-device branch, so the fixture claims as a device
    // rather than through the server scan, which no longer sees these items.
    let mut tx = app.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO device (org_id, id, name, os, arch, app_version, \
                             first_seen_at, last_seen_at) \
         VALUES ($1, $2, 'fixture', 'linux', 'x86_64', '0.0.0', now(), now()) \
         ON CONFLICT (org_id, id) DO NOTHING",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(DEVICE)
    .execute(&mut *tx)
    .await?;
    // The claim serves a device only work whose marketplace it holds a
    // connected session for, which is what the real device establishes on its
    // first check-in.
    sqlx::query(
        "INSERT INTO device_marketplace_session \
             (org_id, device_id, marketplace, linked_at, last_used_at, status) \
         VALUES ($1, $2, 'tes', now(), now(), 'connected'), \
                ($1, $2, 'tpt', now(), now(), 'connected') \
         ON CONFLICT (org_id, device_id, marketplace) DO NOTHING",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(DEVICE)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

const DEVICE: &str = "bind-test";

/// The seller-device claim, mapped back to the shape these fixtures expect.
async fn claim_lease(app: &PgPool) -> Result<Option<LeaseRef>, StorageError> {
    Ok(
        match LeaseRepo::new(app.clone())
            .claim_for_device(
                &DeviceRef {
                    org: ORG,
                    device: DEVICE,
                },
                &ClaimPolicy {
                    ttl_seconds: 600,
                    grace_hours: 24,
                    marketplace: None,
                    reconcile: true,
                },
                T0,
            )
            .await?
        {
            DeviceClaim::Leased(item) => Some(item.lease_ref()),
            DeviceClaim::Empty | DeviceClaim::HeldByAnotherDevice => None,
        },
    )
}

async fn seeded_lease(app: &PgPool, engine: &PgPool) -> Result<Option<LeaseRef>, StorageError> {
    seed(app, engine).await?;
    claim_lease(app).await
}

/// A second product and mapping in the same organisation and inventory, so
/// the two can contend for one remote listing.
async fn seed_rival_mapping(app: &PgPool) -> Result<(), StorageError> {
    ProductRepo::new(app.clone())
        .insert(ORG, &product(RIVAL_PRODUCT), T0)
        .await?;
    MappingRepo::new(app.clone())
        .insert(
            ORG,
            &mapping_of(RIVAL, RIVAL_PRODUCT, Binding::Unbound),
            0,
            T0,
        )
        .await?;
    Ok(())
}

async fn rival_lease(app: &PgPool, engine: &PgPool) -> Result<Option<LeaseRef>, StorageError> {
    seed_rival_mapping(app).await?;
    JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: RIVAL_JOB,
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[NewJobItem {
                item: RIVAL_ITEM,
                mapping: RIVAL,
                idempotency_key: IdempotencyKey(Uuid([0xCB; 16])),
                operation: ItemOperation::Create,
                requires_bound_on: None,
            }],
        )
        .await?;
    claim_lease(app).await
}

/// Puts the item back to a create, for a step that is genuinely one.
///
/// A create against a severed mapping is admitted, which is what makes the
/// row reusable rather than orphaned, so the test that proves it needs the
/// operation the production path would carry.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should stop the run"
)]
async fn make_it_a_create(engine: &PgPool) {
    sqlx::query(
        "UPDATE job_item SET operation = 'create', subject_kind = NULL, \
             subject_url = NULL, subject_numeric_id = NULL, state_from = NULL, \
             state_to = NULL",
    )
    .execute(engine)
    .await
    .expect("the item becomes a create again");
}

/// Makes the item a revise, which is what a landing on a mapping that is
/// already bound belongs to when the listing is being changed rather than
/// taken down.
///
/// Same reason as [`make_it_a_removal`]: `open` refuses a second create
/// against a bound mapping. Which of the two a test wants is decided by what
/// it is modelling, so the row matches the narrative rather than merely
/// getting past the fence.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should stop the run"
)]
async fn make_it_a_revise(engine: &PgPool, url: &str) {
    sqlx::query(
        "UPDATE job_item SET operation = 'revise', subject_kind = 'tes', \
             subject_url = $1, state_from = 'draft', state_to = 'live'",
    )
    .bind(url)
    .execute(engine)
    .await
    .expect("the item becomes a revise");
}

/// Makes the item a removal, which is what a second landing on a bound
/// mapping belongs to in production.
///
/// `WriteAttemptRepo::open` re-checks admission since 2026-09-03, so a create
/// cannot open an attempt against a mapping that is already bound — that is
/// the duplicate-listing fence. These tests are about what a landing does to
/// the mapping rather than about admission, and the landings they model reach
/// a bound mapping through a revise or a removal.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should stop the run"
)]
async fn make_it_a_removal(engine: &PgPool, url: &str) {
    sqlx::query(
        "UPDATE job_item SET operation = 'remove', subject_kind = 'tes', \
             subject_url = $1, state_from = 'draft', state_to = NULL",
    )
    .bind(url)
    .execute(engine)
    .await
    .expect("the item becomes a removal");
}

/// One whole write attempt: opened against the mapping, then settled
/// committed with whatever the write did to the listing it addressed.
async fn settle_attempt(
    engine: &PgPool,
    lease: &LeaseRef,
    mapping: MappingId,
    landing: LandingEffect,
    at: Timestamp,
) -> Result<BindDisposition, StorageError> {
    let attempts = WriteAttemptRepo::new(engine.clone());
    // The id is the caller's now, and `open` is idempotent on it, so a fixture
    // reusing one would silently re-open the row it already settled. Drawn
    // fresh per call for that reason.
    let attempt = tam_types::Uuid(*uuid::Uuid::new_v4().as_bytes());
    attempts
        .open(
            lease,
            attempt,
            &NewAttempt {
                mapping,
                intent: &AttemptIntent {
                    body: serde_json::json!({}),
                    hash: vec![0x01],
                },
                stamp: Stamp::system(SystemComponent::Engine, at),
            },
        )
        .await?;
    attempts
        .settle(
            lease,
            AttemptRef { attempt, mapping },
            &AttemptVerdict {
                state: "committed".to_owned(),
                failure_code: None,
                landing,
            },
            at,
        )
        .await
}

/// A create landing on `url`, in the lifecycle every captured create lands
/// in. Draft carries no `lifecycle_since`, which is what
/// `mapping_lifecycle_total` demands of it.
async fn land(
    engine: &PgPool,
    lease: &LeaseRef,
    mapping: MappingId,
    landed: Option<RemoteListingId>,
    at: Timestamp,
) -> Result<BindDisposition, StorageError> {
    let landing = landed.map_or(LandingEffect::None, |id| LandingEffect::Landed {
        id,
        lifecycle: RemoteLifecycle::Draft,
    });
    settle_attempt(engine, lease, mapping, landing, at).await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_landed_write_binds_the_mapping_stale(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");

    let disposition = land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
        .await
        .expect("the settle runs");

    assert_eq!(
        disposition,
        BindDisposition::Bound,
        "a committed attempt carrying a landed listing binds the mapping it wrote for"
    );
    let record = MappingRepo::new(app.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping,
        landed_mapping(MAPPING, PRODUCT, LANDED, LANDED_AT),
        "the bound row must decode to exactly the mapping a fresh insert of that \
         binding would hold, or the bind left a state the codec cannot round-trip"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn re_landing_the_same_listing_preserves_first_seen(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the first settle runs"),
        BindDisposition::Bound,
        "the setup landing binds before the re-land under test runs"
    );

    make_it_a_revise(&engine, LANDED).await;
    let again = land(&engine, &lease, MAPPING, Some(tes(LANDED)), RELANDED_AT)
        .await
        .expect("the second settle runs");

    assert_eq!(
        again,
        BindDisposition::AlreadyBound,
        "re-landing the identifier the mapping already holds is an idempotent no-op"
    );
    let record = MappingRepo::new(app.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping.binding,
        freshly_bound(LANDED, LANDED_AT),
        "first_seen_at names when the listing was first seen, so a second landing \
         must not move it to the second instant"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_divergent_landing_is_reported_never_written(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the first settle runs"),
        BindDisposition::Bound,
        "the setup landing binds before the divergent one under test runs"
    );

    make_it_a_revise(&engine, LANDED).await;
    let divergent = land(&engine, &lease, MAPPING, Some(tes(ELSEWHERE)), RELANDED_AT)
        .await
        .expect("the second settle runs");

    assert_eq!(
        divergent,
        BindDisposition::DivergentLanding {
            existing: tes(LANDED)
        },
        "a second listing under a bound mapping is reported with the identifier the \
         mapping holds; the attempt still settles, because refusing to record a write \
         that landed would retry the item and mint a third listing"
    );
    let record = MappingRepo::new(app.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping.binding,
        freshly_bound(LANDED, LANDED_AT),
        "a bind never overwrites: the mapping must still hold the first listing"
    );
    assert_eq!(
        record.mapping.lifecycle,
        RemoteLifecycle::Draft,
        "and the lifecycle-only write a re-landing gets is fenced on the identity: a \
         landing on some other listing says nothing about the state of this one"
    );
}

/// The publish's half of a landing. The bind's fence excludes `'bound'` so
/// that re-landing preserves `first_seen_at` and the binding identity — but
/// the lifecycle the verification read observed is new information every
/// time, and it is the only record of which side of the draft line the
/// listing now sits on. Without this write a committed publish left the
/// mapping reading `'draft'`, and `admission`'s `lifecycle_diverged` gate
/// then parked every later item on that mapping for good.
#[sqlx::test(migrations = "./migrations")]
async fn a_relanding_records_the_lifecycle_the_read_observed(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the create's settle runs"),
        BindDisposition::Bound,
        "the create binds the mapping as a draft before the publish under test runs"
    );

    make_it_a_revise(&engine, LANDED).await;
    let published = settle_attempt(
        &engine,
        &lease,
        MAPPING,
        LandingEffect::Landed {
            id: tes(LANDED),
            lifecycle: RemoteLifecycle::Live { since: RELANDED_AT },
        },
        RELANDED_AT,
    )
    .await
    .expect("the publish's settle runs");

    assert_eq!(
        published,
        BindDisposition::AlreadyBound,
        "the binding is unchanged, which is what AlreadyBound has always meant; what \
         changed is that the observation no longer goes nowhere"
    );
    let record = MappingRepo::new(app.clone())
        .get(ORG, MAPPING)
        .await
        .expect("the mapping reads back")
        .expect("the mapping exists");
    assert_eq!(
        record.mapping.lifecycle,
        RemoteLifecycle::Live { since: RELANDED_AT },
        "the publish's read observed a live listing, so the mapping says so"
    );
    assert_eq!(
        record.mapping.binding,
        freshly_bound(LANDED, LANDED_AT),
        "and it says so without moving first_seen_at or re-minting the binding, which is \
         why the lifecycle write is separate from the bind rather than a relaxed fence"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_refused_bind_still_settles_the_attempt(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    sqlx::query(
        "UPDATE mapping \
         SET binding_state = 'ambiguous_create', binding_attempt = $3, ambiguous_since = now() \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(MAPPING.0))
    .bind(db_uuid(Uuid([0xCA; 16])))
    .execute(&engine)
    .await
    .expect("the fixture moves the mapping out of a bindable state");

    let refused = land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
        .await
        .expect("the settle runs");

    assert_eq!(
        refused,
        BindDisposition::Refused {
            state: "ambiguous_create".to_owned()
        },
        "a bind is fenced on the states it may leave, and reports the one it found"
    );
    let attempt: (String, Option<String>) =
        sqlx::query_as("SELECT state, remote_url FROM write_attempt LIMIT 1")
            .fetch_one(&engine)
            .await
            .expect("the attempt row reads");
    assert_eq!(
        (attempt.0.as_str(), attempt.1.as_deref()),
        ("committed", Some(LANDED)),
        "a refused bind must never roll back the settle: the write landed, and an \
         attempt that does not say so would be retried and mint a second listing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_listing_another_mapping_holds_still_settles_the_attempt(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let leases = LeaseRepo::new(engine.clone());
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the first settle runs"),
        BindDisposition::Bound,
        "the first mapping holds the listing the rival then lands on"
    );
    leases
        .settle(
            &lease,
            &ItemVerdict {
                outcome: ItemOutcome::Succeeded,
                failure_code: None,
                failure_detail: None,
            },
            LANDED_AT,
        )
        .await
        .expect("the first item settles, releasing the tenant mutex");
    let rival = rival_lease(&app, &engine)
        .await
        .expect("the rival fixture seeds")
        .expect("the rival item leases");

    let clash = land(&engine, &rival, RIVAL, Some(tes(LANDED)), RELANDED_AT)
        .await
        .expect("the second settle runs");

    assert_eq!(
        clash,
        BindDisposition::ClaimedElsewhere {
            existing_mapping: MAPPING
        },
        "a listing another mapping in the inventory already holds is the cross-mapping \
         form of a divergent landing, and names the mapping holding it"
    );
    let attempt: (String, Option<String>) =
        sqlx::query_as("SELECT state, remote_url FROM write_attempt WHERE mapping_id = $1")
            .bind(db_uuid(RIVAL.0))
            .fetch_one(&engine)
            .await
            .expect("the rival attempt row reads");
    assert_eq!(
        (attempt.0.as_str(), attempt.1.as_deref()),
        ("committed", Some(LANDED)),
        "the clash must not roll the settle back: the write landed, and an attempt that \
         does not say so would be retried into a third listing"
    );
    let repo = MappingRepo::new(app.clone());
    let rival_record = repo
        .get(ORG, RIVAL)
        .await
        .expect("the rival mapping reads back")
        .expect("the rival mapping exists");
    assert_eq!(
        rival_record.mapping.binding,
        Binding::Unbound,
        "the refused bind wrote nothing to the mapping it was refused for"
    );
    let held = repo
        .get(ORG, MAPPING)
        .await
        .expect("the holding mapping reads back")
        .expect("the holding mapping exists");
    assert_eq!(
        held.mapping.binding,
        freshly_bound(LANDED, LANDED_AT),
        "and nothing touched the mapping that already held the listing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn one_remote_listing_cannot_be_claimed_twice(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the settle runs"),
        BindDisposition::Bound,
        "the first claim binds, so the index has something to refuse against"
    );
    seed_rival_mapping(&app)
        .await
        .expect("the rival product and mapping insert");

    let refused = sqlx::query(
        "UPDATE mapping \
         SET binding_state = 'bound', remote_id_kind = 'tes', remote_url = $3, \
             first_seen_at = now(), verify_state = 'stale', verify_stale_since = now() \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(RIVAL.0))
    .bind(LANDED)
    .execute(&engine)
    .await;

    assert!(
        refused.is_err(),
        "two mappings in one organisation's inventory must not both claim the same \
         remote listing; mapping_one_per_inventory guards the product side only"
    );
}

/// The mapping's whole state after a sever, in the columns the CHECK
/// constraints govern together. Read over the engine role, which is
/// BYPASSRLS: `tam_app` forces row-level security and this helper takes no
/// tenant pin.
type SeverRow = (
    String,
    Option<String>,
    Option<String>,
    bool,
    String,
    bool,
    String,
    bool,
);

async fn sever_row(pool: &PgPool, mapping: MappingId) -> Result<SeverRow, sqlx::Error> {
    sqlx::query_as(
        "SELECT binding_state, remote_url, sever_cause, severed_at IS NOT NULL, \
                verify_state, verify_stale_since IS NULL, \
                lifecycle_state, lifecycle_since IS NULL \
         FROM mapping WHERE org_id = $1 AND id = $2",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(mapping.0))
    .fetch_one(pool)
    .await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_committed_removal_severs_and_releases_the_bound_claim(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the create settles"),
        BindDisposition::Bound,
        "the removal needs a binding to sever"
    );

    make_it_a_removal(&engine, LANDED).await;
    let severed = settle_attempt(
        &engine,
        &lease,
        MAPPING,
        LandingEffect::Severed { id: tes(LANDED) },
        RELANDED_AT,
    )
    .await
    .expect("the removal settles");

    assert_eq!(
        severed,
        BindDisposition::Severed,
        "a committed removal severs rather than binding the listing it took down"
    );
    assert_eq!(
        sever_row(&engine, MAPPING)
            .await
            .expect("the mapping row reads"),
        (
            "severed".to_owned(),
            Some(LANDED.to_owned()),
            Some("removed_by_seller".to_owned()),
            true,
            "stale".to_owned(),
            true,
            "absent".to_owned(),
            true,
        ),
        "three plausible wrong severs are refused here: one that nulls the remote id \
         (mapping_binding_total demands it on a severed row), one that leaves \
         verify_stale_since set (mapping_verify_bound demands it NULL off a bound row), \
         and one that writes a lifecycle_state without its matching since"
    );

    seed_rival_mapping(&app)
        .await
        .expect("the rival product and mapping insert");
    let reclaimed = sqlx::query(
        "UPDATE mapping \
         SET binding_state = 'bound', remote_id_kind = 'tes', remote_url = $3, \
             first_seen_at = now(), verify_state = 'stale', verify_stale_since = now() \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(RIVAL.0))
    .bind(LANDED)
    .execute(&engine)
    .await;
    assert!(
        reclaimed.is_ok(),
        "both of 0018's unique indexes are predicated on binding_state = 'bound', so a \
         sever releases the claim; a sever that left the row bound would refuse this, \
         and the migrate the sever exists for would be unrunnable"
    );
}

/// A constraint proof, not a proof of a production race: while a removal is
/// in flight `write_attempt_one_in_flight` admits no second attempt on the
/// mapping, so nothing can rebind it underneath. The predicate this pins is
/// defence in depth against a fixture-level write and a future concurrent
/// binder.
#[sqlx::test(migrations = "./migrations")]
async fn a_sever_of_a_mapping_rebound_elsewhere_is_refused_not_written(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(ELSEWHERE)), LANDED_AT)
            .await
            .expect("the create settles"),
        BindDisposition::Bound,
        "the mapping holds a listing other than the one the removal took down"
    );

    make_it_a_removal(&engine, LANDED).await;
    let refused = settle_attempt(
        &engine,
        &lease,
        MAPPING,
        LandingEffect::Severed { id: tes(LANDED) },
        RELANDED_AT,
    )
    .await
    .expect("the removal settles");

    assert_eq!(
        refused,
        BindDisposition::SeverDiverged {
            existing: tes(ELSEWHERE)
        },
        "a rebound sever fails on the remote identity while the row is still bound, so \
         reporting it as a refusal against the binding state would be a self-contradiction"
    );
    let row = sever_row(&engine, MAPPING)
        .await
        .expect("the mapping row reads");
    assert_eq!(
        (row.0.as_str(), row.1.as_deref()),
        ("bound", Some(ELSEWHERE)),
        "an unconditional sever would erase a live binding on the strength of a stale removal"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_severed_mapping_re_creates_through_the_same_row(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the create settles"),
        BindDisposition::Bound,
        "the removal needs a binding to sever"
    );
    make_it_a_removal(&engine, LANDED).await;
    assert_eq!(
        settle_attempt(
            &engine,
            &lease,
            MAPPING,
            LandingEffect::Severed { id: tes(LANDED) },
            RELANDED_AT,
        )
        .await
        .expect("the removal settles"),
        BindDisposition::Severed,
        "the re-create needs a severed row to reuse"
    );

    // Back to a create for the step this test exists to prove: a create
    // against a severed mapping is admitted, which is what makes the row
    // reusable rather than orphaned.
    make_it_a_create(&engine).await;
    let rebound = land(&engine, &lease, MAPPING, Some(tes(ELSEWHERE)), RELANDED_AT)
        .await
        .expect("the re-create settles");

    assert_eq!(
        rebound,
        BindDisposition::Bound,
        "mapping_one_per_inventory is unpredicated, so a severed row cannot be replaced \
         by a fresh mapping and the re-create must reuse it; a fence excluding 'severed' \
         would leave a live listing the ledger has no record of"
    );
    assert_eq!(
        sever_row(&engine, MAPPING)
            .await
            .expect("the mapping row reads"),
        (
            "bound".to_owned(),
            Some(ELSEWHERE.to_owned()),
            None,
            false,
            "stale".to_owned(),
            false,
            "draft".to_owned(),
            true,
        ),
        "the re-bound row is constraint-legal whole: the sever columns clear, the verify \
         columns take a bound row's shape, and the lifecycle is the one observed"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_committed_create_records_the_lifecycle_it_landed_in(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let leases = LeaseRepo::new(engine.clone());
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the enqueued item leases");
    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the create settles"),
        BindDisposition::Bound,
        "the create binds before its lifecycle can be read back"
    );
    let drafted = sever_row(&engine, MAPPING)
        .await
        .expect("the mapping row reads");
    assert_eq!(
        (drafted.6.as_str(), drafted.7),
        ("draft", true),
        "a create observed as a draft records 'draft', and mapping_lifecycle_total \
         demands no lifecycle_since beside it"
    );

    leases
        .settle(
            &lease,
            &ItemVerdict {
                outcome: ItemOutcome::Succeeded,
                failure_code: None,
                failure_detail: None,
            },
            LANDED_AT,
        )
        .await
        .expect("the first item settles, releasing the tenant mutex");
    let rival = rival_lease(&app, &engine)
        .await
        .expect("the rival fixture seeds")
        .expect("the rival item leases");
    assert_eq!(
        settle_attempt(
            &engine,
            &rival,
            RIVAL,
            LandingEffect::Landed {
                id: tes(ELSEWHERE),
                lifecycle: RemoteLifecycle::Live { since: RELANDED_AT },
            },
            RELANDED_AT,
        )
        .await
        .expect("the publish settles"),
        BindDisposition::Bound,
        "the publish binds the rival mapping"
    );

    let published = sever_row(&engine, RIVAL)
        .await
        .expect("the mapping row reads");
    assert_eq!(
        (published.6.as_str(), published.7),
        ("live", false),
        "a bind that keeps ignoring the lifecycle columns leaves every published listing \
         reading 'absent', which is what makes a later removal state the wrong route"
    );
}

/// The publish half of one live intent, parked exactly as the worker parks it
/// when it leases before the create it publishes.
async fn park_a_waiting_publish(
    engine: &PgPool,
    job: JobId,
    item: JobItemId,
    mapping: MappingId,
    key: u8,
) -> Result<(), StorageError> {
    JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &NewJob {
                job,
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[NewJobItem {
                item,
                mapping,
                idempotency_key: IdempotencyKey(Uuid([key; 16])),
                operation: ItemOperation::Publish {
                    to: ListingState::Live,
                },
                requires_bound_on: Some(InventoryId::TesGb),
            }],
        )
        .await?;
    sqlx::query(
        "UPDATE job_item SET state = 'parked_live', blocked_on = $2, \
                park_expires_at = now() + interval '24 hours' \
         WHERE org_id = $1 AND id = $3",
    )
    .bind(db_uuid(ORG.0))
    .bind(tam_storage::AWAITING_COUNTERPART)
    .bind(db_uuid(item.0))
    .execute(engine)
    .await?;
    Ok(())
}

async fn gate_of(
    engine: &PgPool,
    item: JobItemId,
) -> Result<(String, Option<String>), sqlx::Error> {
    sqlx::query_as("SELECT state, blocked_on FROM job_item WHERE org_id = $1 AND id = $2")
        .bind(db_uuid(ORG.0))
        .bind(db_uuid(item.0))
        .fetch_one(engine)
        .await
}

/// The binding releases what was waiting for it, in the transaction that
/// wrote it.
///
/// A live intent lowers to a create and a publish sharing one `created_at`,
/// so `acquire`'s FIFO tie-breaks on a fresh uuid and the publish leases first
/// about half the time. It parks on `awaiting_counterpart`, and the only thing
/// that cleared that gate was the twenty-four-hour park expiry -- for a create
/// that landed seconds later. The revive is keyed on the product and the
/// inventory the waiter named, so a bystander waiting on another product stays
/// parked.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_landing_revives_the_item_that_waited_for_it(app: PgPool) {
    let engine = engine_pool(&app).await.expect("the engine role connects");
    let lease = seeded_lease(&app, &engine)
        .await
        .expect("the fixture seeds")
        .expect("the create leases");
    park_a_waiting_publish(&engine, WAITING_JOB, WAITING_ITEM, MAPPING, 0xE3)
        .await
        .expect("the publish parks on its counterpart");
    seed_rival_mapping(&app)
        .await
        .expect("a second product seeds");
    park_a_waiting_publish(&engine, BYSTANDER_JOB, BYSTANDER_ITEM, RIVAL, 0xE6)
        .await
        .expect("another product's publish parks too");

    assert_eq!(
        land(&engine, &lease, MAPPING, Some(tes(LANDED)), LANDED_AT)
            .await
            .expect("the create settles"),
        BindDisposition::Bound,
    );

    assert_eq!(
        gate_of(&engine, WAITING_ITEM)
            .await
            .expect("the waiting item reads"),
        ("queued".to_owned(), None),
        "the publish is queued by the binding rather than by the clock"
    );
    assert_eq!(
        gate_of(&engine, BYSTANDER_ITEM)
            .await
            .expect("the bystander reads"),
        (
            "parked_live".to_owned(),
            Some(tam_storage::AWAITING_COUNTERPART.to_owned())
        ),
        "another product's counterpart is not this binding's to release"
    );

    let resumed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_event \
         WHERE org_id = $1 AND job_item_id = $2 AND kind = 'ItemResumed'",
    )
    .bind(db_uuid(ORG.0))
    .bind(db_uuid(WAITING_ITEM.0))
    .fetch_one(&engine)
    .await
    .expect("the ledger reads");
    assert_eq!(
        resumed, 1,
        "the ledger says why a parked item is running again"
    );
}
