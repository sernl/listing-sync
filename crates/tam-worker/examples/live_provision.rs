//! The live-proof provisioner: the ledger rows one supervised end-to-end
//! engine run needs, written through the same repositories the API path
//! writes through, so the roles and the row-level security a live run leans
//! on are exercised rather than bypassed.
//!
//! Four modes, in the order a proof uses them:
//!
//!   live_provision create    <db-url> <kek-path> <store-root> <payload> <cover>
//!   live_provision preflight <db-url> <kek-path> <store-root> <mapping-id>
//!   live_provision remove    <db-url> <mapping-id>
//!   live_provision show      <db-url> <mapping-id>
//!
//! `create` seeds the crosswalk, stores both files as real per-tenant blobs,
//! inserts the product and an unbound Tpt mapping, links the `tpt` connection
//! the lease scan gates on, and enqueues one create item. `preflight` runs
//! the ledger's own admission and projection over that mapping and renders
//! the field set Tpt would receive, without leasing anything and without a
//! byte leaving the process. `remove` reads the listing the create bound and
//! enqueues the removal that names it, which the engine admits only while the
//! mapping still holds that exact identifier. `show` prints the mapping, item
//! and attempt rows the acceptance is read off.
//!
//! Nothing here talks to a marketplace. `tam-worker` does, and it is the
//! thing under test; this only puts the queue in front of it.
//!
//! The database url is the app role's, deliberately: every write below is one
//! the API path would make, so a policy that would refuse the real caller
//! refuses this too. The worker itself runs against the engine role.

#![forbid(unsafe_code)]

use std::io::Read as _;
use std::path::PathBuf;

use sqlx::{PgPool, Row as _};
use tam_domain::{
    Binding, CanonicalProduct, CanonicalTerm, Decider, DeclarationSource, EdgeKind, FieldPolicies,
    FieldPolicy, GradeDeclaration, ItemOperation, JobItemId, Mapping, ProjectionEdge, PublishMode,
    TermKind, VocabularyId, VocabularyPath,
};
use tam_engine::seed::{prepare_item, ItemPreparation};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::{FileSource as _, IdempotencyKey, ListingState, RemoteLifecycle};
use tam_marketplace_tpt::write_model;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    BlobRepo, JobRepo, LeasedItem, MappingRepo, NewJob, NewJobItem, PipelineFileSource,
    ProductRepo, SessionRepo, TaxonomyRepo,
};
use tam_types::{
    CanonicalTermId, ContentHash, FileId, FileKind, FileRole, InventoryId, JobId, ListingCopy,
    MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, Uuid,
};

const USAGE: &str = "usage: live_provision create <db-url> <kek-path> <store-root> <payload> \
                     <cover> | live_provision preflight <db-url> <kek-path> <store-root> \
                     <mapping-id> | live_provision remove <db-url> <mapping-id> | live_provision \
                     show <db-url> <mapping-id>";

/// The dev organisation `just dev-session --ensure-org founder-dev` mints,
/// and the one `TAM_TPT_ORG` must name for the worker to drive these items.
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
/// The `tpt` connection row. It holds no secret: the worker's cookie jar is
/// the credential and this row is the queue gate `LeaseRepo::acquire`
/// requires, exactly as `crates/tam-worker/src/main.rs` records.
const CONNECTION: Uuid = Uuid([0x3C; 16]);
/// The one crosswalked subject this fixture projects through.
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x7A; 16]));
/// The create's intent version, and the removal's. They differ because
/// `job_item` is unique on `(org_id, idempotency_key)` and both items name
/// the same tenant, inventory, product and payload hash.
const CREATE_INTENT: u32 = 1;
const REMOVE_INTENT: u32 = 2;

type Failure = Box<dyn std::error::Error>;

#[expect(
    clippy::disallowed_methods,
    reason = "the provisioner is a clock-reading process boundary; time enters the ledger as data from here"
)]
fn wall_now() -> Result<Timestamp, Failure> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// One run's identifiers, all derived from the instant it started so a second
/// provisioning never collides with the first one's rows. The tag separates
/// the ids minted within one run.
fn seeded(now: Timestamp, tag: u8) -> Uuid {
    let mut bytes = [0_u8; 16];
    let millis = now.0.to_le_bytes();
    for (slot, byte) in bytes.iter_mut().zip(millis.iter().chain(millis.iter())) {
        *slot = *byte;
    }
    bytes[15] = tag;
    Uuid(bytes)
}

fn read_bytes(path: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn load_kek(path: &str) -> Result<Kek, Failure> {
    Ok(Kek::from_bytes(&read_bytes(path)?)?)
}

async fn connect(db_url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await
}

/// The tenant pin the row-level-security policies read. `tam_storage` pins
/// inside its own transactions and keeps the helper crate-private, so the two
/// raw statements below carry their own.
async fn pin(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(ORG.0.to_hyphenated())
        .execute(&mut **tx)
        .await
        .map(drop)
}

/// The canonical subject the fixture carries, and its Tpt counterpart.
///
/// `algebra` and `not-grade-specific` are TPT's own slugs, taken from the
/// captured create that `examples/live_write.rs` proved live. Neither names a
/// seller shelf, so nothing lands on one of the founder's own categories.
fn crosswalk(at: Timestamp) -> (Vec<CanonicalTerm>, Vec<ProjectionEdge>) {
    let terms = vec![CanonicalTerm {
        id: SUBJECT,
        kind: TermKind::Subject,
        parent: None,
        label: "Algebra".to_owned(),
    }];
    let edges = vec![ProjectionEdge {
        from: SUBJECT,
        to: VocabularyPath {
            vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Subject),
            segments: vec!["Math".to_owned(), "Algebra".to_owned()],
            native_id: Some("algebra".to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "live_provision fixture".to_owned(),
        },
        decided_at: at,
    }];
    (terms, edges)
}

/// The two stored blobs a product is made of, already content-addressed.
struct Stored {
    payload: ContentHash,
    payload_len: u64,
    cover: ContentHash,
    cover_len: u64,
}

/// Both files through the real per-tenant blob path — sealed, stored, and
/// deduplicated on the hash — so the worker's `PipelineFileSource` reads them
/// back exactly as it would read an uploaded seller's file.
async fn store_files(
    repo: &BlobRepo<LocalObjectStore>,
    payload: &str,
    cover: &str,
    at: Timestamp,
) -> Result<Stored, Failure> {
    let payload_bytes = read_bytes(payload)?;
    let cover_bytes = read_bytes(cover)?;
    Ok(Stored {
        payload: repo.put(ORG, &payload_bytes, at).await?,
        payload_len: u64::try_from(payload_bytes.len())?,
        cover: repo.put(ORG, &cover_bytes, at).await?,
        cover_len: u64::try_from(cover_bytes.len())?,
    })
}

fn fixture_product(
    id: ProductId,
    title: String,
    stored: &Stored,
    at: Timestamp,
) -> CanonicalProduct {
    CanonicalProduct {
        id,
        org: ORG,
        title: Title(title),
        body: ListingCopy {
            body: "A single-page test file. Delete on sight.".to_owned(),
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(seeded(at, 0x11)),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: stored.payload,
                byte_len: stored.payload_len,
                scan: ScanOutcome::Clean { at },
            },
            vec![],
        ),
        // Tpt uploads the payload alone, but the projection's cover gate is
        // unconditional, so the cover is a real stored blob rather than a row
        // pointing at bytes that were never written.
        cover: Some(ProductFile {
            id: FileId(seeded(at, 0x12)),
            role: FileRole::Cover,
            kind: FileKind::Image,
            hash: stored.cover,
            byte_len: stored.cover_len,
            scan: ScanOutcome::Clean { at },
        }),
        previews: vec![],
        subjects: vec![SUBJECT],
        grades: GradeDeclaration {
            source: DeclarationSource::Imported {
                vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
            },
            raw: vec![VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
                segments: vec!["Not grade specific".to_owned()],
                native_id: Some("not-grade-specific".to_owned()),
            }],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: tam_domain::RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

fn fixture_mapping(id: MappingId, product: ProductId) -> Mapping {
    Mapping {
        id,
        org: ORG,
        product,
        inventory: InventoryId::Tpt,
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
        // The create binds the lifecycle its read-back observes; nothing has
        // been observed yet, and `Absent` is the only honest starting value.
        binding: Binding::Unbound,
        lifecycle: RemoteLifecycle::Absent,
    }
}

/// The queue gate. `LeaseRepo::acquire`'s candidate CTE admits an item only
/// where its org holds a `linked` connection for the marketplace, and
/// `gate_connection` flipping this row to `needs_reauth` still stops Tpt
/// items even though the secret it would carry was never the one sent.
async fn link_connection(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    pin(&mut tx).await?;
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, 'tpt', 'linked', now(), now()) \
         ON CONFLICT (org_id, marketplace) DO UPDATE SET state = 'linked', updated_at = now()",
    )
    .bind(ORG.0.to_hyphenated())
    .bind(CONNECTION.to_hyphenated())
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// One job carrying one item, which is the shape both halves of the proof
/// take.
async fn enqueue_one(
    pool: &PgPool,
    operation: ItemOperation,
    keyed: (ProductId, MappingId, ContentHash, u32),
    at: Timestamp,
) -> Result<(JobId, JobItemId), Failure> {
    let (product, mapping, payload_hash, intent_version) = keyed;
    let job = JobId(seeded(at, 0x40 | u8::try_from(intent_version)?));
    let item = JobItemId(seeded(at, 0x50 | u8::try_from(intent_version)?));
    JobRepo::new(pool.clone())
        .enqueue(
            ORG,
            &NewJob {
                job,
                inventory: InventoryId::Tpt,
                at,
            },
            &[NewJobItem {
                item,
                mapping,
                idempotency_key: derive_idempotency_key(
                    ORG,
                    InventoryId::Tpt,
                    product,
                    intent_version,
                    payload_hash,
                ),
                operation,
            }],
        )
        .await?;
    Ok((job, item))
}

/// What `create` was told to build from.
struct CreateInputs<'a> {
    kek_path: &'a str,
    store_root: &'a str,
    payload: &'a str,
    cover: &'a str,
}

async fn create(pool: &PgPool, inputs: &CreateInputs<'_>) -> Result<(), Failure> {
    let now = wall_now()?;
    let title = format!("ZZ-DELETE-ME-engine-{}", now.0);

    SessionRepo::new(pool.clone())
        .ensure_org(ORG, "founder-dev", now)
        .await?;
    let (terms, edges) = crosswalk(now);
    let seeded_crosswalk = TaxonomyRepo::new(pool.clone()).seed(&terms, &edges).await?;

    let blobs = BlobRepo::new(
        pool.clone(),
        LocalObjectStore::new(PathBuf::from(inputs.store_root)),
        load_kek(inputs.kek_path)?,
    );
    let stored = store_files(&blobs, inputs.payload, inputs.cover, now).await?;

    let product = ProductId(seeded(now, 0x01));
    let mapping = MappingId(seeded(now, 0x31));
    let canonical = fixture_product(product, title.clone(), &stored, now);
    ProductRepo::new(pool.clone())
        .insert(ORG, &canonical, now)
        .await?;
    MappingRepo::new(pool.clone())
        .insert(ORG, &fixture_mapping(mapping, product), 0, now)
        .await?;
    link_connection(pool).await?;

    let (job, item) = enqueue_one(
        pool,
        ItemOperation::Create,
        (product, mapping, stored.payload, CREATE_INTENT),
        now,
    )
    .await?;

    println!("provisioned a create for {title:?}");
    println!("  crosswalk    {seeded_crosswalk:?}");
    println!("  TAM_TPT_ORG  {}", ORG.0.to_hyphenated());
    println!("  product      {}", product.0.to_hyphenated());
    println!("  mapping      {}", mapping.0.to_hyphenated());
    println!("  job          {}", job.0.to_hyphenated());
    println!("  item         {}", item.0.to_hyphenated());
    Ok(())
}

/// The removal names the listing the create bound, and states the side of the
/// draft line it will delete from. `prepare_item` refuses both if either has
/// moved, so reading them here rather than accepting them as arguments is
/// what makes the enqueue answerable to the ledger.
async fn remove(pool: &PgPool, mapping: MappingId) -> Result<(), Failure> {
    let now = wall_now()?;
    let record = MappingRepo::new(pool.clone())
        .get(ORG, mapping)
        .await?
        .ok_or("no such mapping in this organisation")?;
    let Binding::Bound { ref id, .. } = record.mapping.binding else {
        return Err(format!(
            "the mapping is {:?} rather than bound, so there is no listing to remove",
            record.mapping.binding
        )
        .into());
    };
    if !matches!(record.mapping.lifecycle, RemoteLifecycle::Draft) {
        return Err(format!(
            "the mapping records the listing as {:?}; this proof removes a draft, and a \
             removal whose stated state diverges from the mapping is parked rather than run",
            record.mapping.lifecycle
        )
        .into());
    }
    let product = ProductRepo::new(pool.clone())
        .get(ORG, record.mapping.product)
        .await?
        .ok_or("the mapped product has gone")?;
    let payload_hash = product
        .product
        .payload
        .iter()
        .next()
        .ok_or("the mapped product carries no payload file")?
        .hash;

    let (job, item) = enqueue_one(
        pool,
        ItemOperation::Remove {
            subject: id.clone(),
            state: ListingState::Draft,
        },
        (record.mapping.product, mapping, payload_hash, REMOVE_INTENT),
        now,
    )
    .await?;

    println!("provisioned a removal of {id:?}");
    println!("  mapping      {}", mapping.0.to_hyphenated());
    println!("  job          {}", job.0.to_hyphenated());
    println!("  item         {}", item.0.to_hyphenated());
    Ok(())
}

/// The acceptance read: the mapping's binding and lifecycle, every item that
/// has addressed it, and every attempt those items opened.
async fn show(pool: &PgPool, mapping: MappingId) -> Result<(), Failure> {
    let record = MappingRepo::new(pool.clone())
        .get(ORG, mapping)
        .await?
        .ok_or("no such mapping in this organisation")?;
    println!("mapping {}", mapping.0.to_hyphenated());
    println!(
        "  product      {}",
        record.mapping.product.0.to_hyphenated()
    );
    println!("  binding      {:?}", record.mapping.binding);
    println!("  lifecycle    {:?}", record.mapping.lifecycle);

    let mut tx = pool.begin().await?;
    pin(&mut tx).await?;
    let items = sqlx::query(
        "SELECT id::text AS id, job_id::text AS job, operation, state, outcome, failure_code, \
                blocked_on, attempt_count, subject_numeric_id, state_from, state_to \
         FROM job_item WHERE org_id = $1::uuid AND mapping_id = $2::uuid ORDER BY created_at, id",
    )
    .bind(ORG.0.to_hyphenated())
    .bind(mapping.0.to_hyphenated())
    .fetch_all(&mut *tx)
    .await?;
    println!("  {} item(s)", items.len());
    for row in &items {
        println!(
            "    {} job={} op={} state={} outcome={:?} failure={:?} blocked={:?} attempts={} \
             subject={:?} from={:?} to={:?}",
            row.try_get::<String, _>("id")?,
            row.try_get::<String, _>("job")?,
            row.try_get::<String, _>("operation")?,
            row.try_get::<String, _>("state")?,
            row.try_get::<Option<String>, _>("outcome")?,
            row.try_get::<Option<String>, _>("failure_code")?,
            row.try_get::<Option<String>, _>("blocked_on")?,
            row.try_get::<i32, _>("attempt_count")?,
            row.try_get::<Option<i64>, _>("subject_numeric_id")?,
            row.try_get::<Option<String>, _>("state_from")?,
            row.try_get::<Option<String>, _>("state_to")?,
        );
    }

    let attempts = sqlx::query(
        "SELECT id::text AS id, state, remote_id_kind, remote_numeric_id, remote_url, \
                failure_code, ambiguity_cause, opened_at::text AS opened, \
                settled_at::text AS settled \
         FROM write_attempt WHERE org_id = $1::uuid AND mapping_id = $2::uuid \
         ORDER BY opened_at, id",
    )
    .bind(ORG.0.to_hyphenated())
    .bind(mapping.0.to_hyphenated())
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    println!("  {} attempt(s)", attempts.len());
    for row in &attempts {
        println!(
            "    {} state={} remote={:?}/{:?}/{:?} failure={:?} ambiguity={:?} opened={} \
             settled={:?}",
            row.try_get::<String, _>("id")?,
            row.try_get::<String, _>("state")?,
            row.try_get::<Option<String>, _>("remote_id_kind")?,
            row.try_get::<Option<i64>, _>("remote_numeric_id")?,
            row.try_get::<Option<String>, _>("remote_url")?,
            row.try_get::<Option<String>, _>("failure_code")?,
            row.try_get::<Option<String>, _>("ambiguity_cause")?,
            row.try_get::<String, _>("opened")?,
            row.try_get::<Option<String>, _>("settled")?,
        );
    }
    Ok(())
}

/// The whole path up to the first outbound byte, run against the real
/// ledger: the admission `prepare_item` applies, the projection it produces,
/// the wire shape Tpt's write model renders from it, and the payload blob
/// read back through the same `FileSource` the worker hands its adapter.
///
/// The item is synthesised rather than leased. `prepare_item` reads a leased
/// item's organisation, mapping, inventory and operation and nothing else, so
/// the identifiers and the epoch below are inert; leasing for a preview would
/// bump the item's epoch and hand the real run a stolen lease to recover
/// from. A removal renders nothing — `seed_for_removal` builds an empty field
/// set — so the create is the only projection there is to preview.
async fn preflight(
    pool: &PgPool,
    inputs: &CreateInputs<'_>,
    mapping: MappingId,
) -> Result<(), Failure> {
    let now = wall_now()?;
    let lease = LeasedItem {
        org: ORG,
        item: JobItemId(Uuid([0; 16])),
        job: JobId(Uuid([0; 16])),
        mapping,
        inventory: InventoryId::Tpt,
        idempotency_key: IdempotencyKey(Uuid([0; 16])),
        operation: ItemOperation::Create,
        lease_epoch: 0,
        attempt_count: 0,
    };
    let projected = match prepare_item(pool, &lease, now).await? {
        ItemPreparation::Ready { projected, .. } => {
            projected.ok_or("a create must project a listing")?
        }
        ItemPreparation::Blocked { gate, raised } => {
            return Err(format!(
                "the ledger would park this item on {gate} ({} raised, {} already open);                  the live run would park the same way",
                raised.new, raised.already_open
            )
            .into())
        }
    };
    let fields = write_model::project_fields(&projected)
        .map_err(|error| format!("Tpt refused the projection: {error:?}"))?;
    println!("the item projects; Tpt would receive");
    for (key, value) in &fields.entries {
        println!("  {key:?} = {value}");
    }

    let files = PipelineFileSource::new(
        BlobRepo::new(
            pool.clone(),
            LocalObjectStore::new(PathBuf::from(inputs.store_root)),
            load_kek(inputs.kek_path)?,
        ),
        ORG,
        pool.clone(),
    );
    for id in &fields.files {
        let content = files
            .fetch(*id)
            .await
            .map_err(|error| format!("the payload blob does not read back: {error:?}"))?;
        println!(
            "  file {} reads back as {} ({}, {} bytes)",
            id.0.to_hyphenated(),
            content.file_name,
            content.content_type,
            content.bytes.len()
        );
    }
    Ok(())
}

fn mapping_argument(raw: Option<&String>) -> Result<MappingId, Failure> {
    let raw = raw.ok_or(USAGE)?;
    Uuid::parse_hyphenated(raw)
        .map(MappingId)
        .ok_or_else(|| format!("{raw:?} is not a hyphenated mapping id").into())
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mode = arguments.first().ok_or(USAGE)?.as_str();
    let pool = connect(arguments.get(1).ok_or(USAGE)?).await?;
    match mode {
        "create" => {
            create(
                &pool,
                &CreateInputs {
                    kek_path: arguments.get(2).ok_or(USAGE)?,
                    store_root: arguments.get(3).ok_or(USAGE)?,
                    payload: arguments.get(4).ok_or(USAGE)?,
                    cover: arguments.get(5).ok_or(USAGE)?,
                },
            )
            .await
        }
        "preflight" => {
            preflight(
                &pool,
                &CreateInputs {
                    kek_path: arguments.get(2).ok_or(USAGE)?,
                    store_root: arguments.get(3).ok_or(USAGE)?,
                    payload: "",
                    cover: "",
                },
                mapping_argument(arguments.get(4))?,
            )
            .await
        }
        "remove" => remove(&pool, mapping_argument(arguments.get(2))?).await,
        "show" => show(&pool, mapping_argument(arguments.get(2))?).await,
        _ => Err(USAGE.into()),
    }
}
