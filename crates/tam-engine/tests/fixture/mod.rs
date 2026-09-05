//! The Postgres fixture both engine suites seed from.
//!
//! Lifted verbatim from `tests/driver.rs` so the conformance instantiation and
//! the Postgres-only tests seed identically; a fixture that drifted between
//! them would make a divergence between the two runs unreadable.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tam_marketplace::{IdempotencyKey, RemoteLifecycle};
use tam_storage::{ClaimPolicy, DeviceRef, JobRepo, MappingRepo, NewJob, NewJobItem, ProductRepo};
use tam_types::{
    Actor, ContentHash, CopyFormat, FileBytes, InventoryId, JobId, MappingId, OrgId, Stamp,
    SystemComponent, Timestamp, Uuid,
};

pub(crate) const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const T0: Timestamp = Timestamp(1_756_000_000_000);

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
pub(crate) async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects")
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
pub(crate) async fn seed(app: &PgPool, engine: &PgPool) -> MappingId {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(app)
        .await
        .expect("the org inserts");
    let product = tam_types::ProductId(Uuid([0x01; 16]));
    let mapping = MappingId(Uuid([0x02; 16]));
    ProductRepo::new(app.clone())
        .insert(
            ORG,
            &tam_domain::CanonicalProduct {
                id: product,
                org: ORG,
                title: tam_types::Title("Fixture".to_owned()),
                body: tam_types::ListingCopy {
                    body: "Fixture".to_owned(),
                    format: CopyFormat::Markdown,
                },
                payload: Some(tam_types::PayloadSet::new(
                    tam_types::ProductFile {
                        id: tam_types::FileId(Uuid([0x03; 16])),
                        role: tam_types::FileRole::Payload,
                        kind: tam_types::FileKind::Pdf,
                        bytes: FileBytes::Held {
                            hash: ContentHash([0x04; 32]),
                            byte_len: 4,
                            scan: tam_types::ScanOutcome::Pending,
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
                price: tam_types::PriceIntent::Free,
                rights: tam_domain::RightsDeclaration::Unstated,
                native_residue: vec![],
            },
            T0,
        )
        .await
        .expect("the product inserts");
    MappingRepo::new(app.clone())
        .insert(
            ORG,
            &tam_domain::Mapping {
                id: mapping,
                org: ORG,
                product,
                inventory: InventoryId::TesGb,
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
                lifecycle: RemoteLifecycle::Absent,
            },
            0,
            T0,
        )
        .await
        .expect("the mapping inserts");
    let mut tx = app.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes([0x05; 16]))
    .execute(&mut *tx)
    .await
    .expect("the connection inserts");
    tx.commit().await.expect("the fixture commits");

    JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &NewJob {
                job: JobId(Uuid([0x06; 16])),
                inventory: InventoryId::TesGb,
                stamp: Stamp {
                    at: T0,
                    actor: Actor::System(SystemComponent::Engine),
                },
            },
            &[NewJobItem {
                item: tam_domain::JobItemId(Uuid([0x07; 16])),
                mapping,
                idempotency_key: IdempotencyKey(Uuid([0x08; 16])),
                operation: tam_domain::ItemOperation::Create,
                requires_bound_on: None,
            }],
        )
        .await
        .expect("the job enqueues");
    mapping
}

/// The seller's device, which the seller-device claim admits only if it is
/// registered and unrevoked.
pub(crate) const DEVICE: &str = "engine-test-device";

/// Tes is the seller-device branch, so a fixture that means to lease claims as
/// a device rather than through `acquire`, which no longer sees these items.
///
/// The claim is org-pinned by forced row-level security, so it runs on the app
/// pool; the engine pool is BYPASSRLS and would not be pinned by it.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
pub(crate) async fn claim(app: &PgPool, device: &str, ttl: i64) -> Option<tam_storage::LeasedItem> {
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
    .bind(device)
    .execute(&mut *tx)
    .await
    .expect("the fixture sessions register");
    tx.commit().await.expect("the fixture device commits");
    match tam_storage::LeaseRepo::new(app.clone())
        .claim_for_device(
            &DeviceRef { org: ORG, device },
            &ClaimPolicy {
                ttl_seconds: ttl,
                grace_hours: 24,
                marketplace: None,
                reconcile: true,
            },
            T0,
        )
        .await
        .expect("the claim runs")
    {
        tam_storage::DeviceClaim::Leased(item) => Some(*item),
        tam_storage::DeviceClaim::Empty | tam_storage::DeviceClaim::HeldByAnotherDevice => None,
    }
}
