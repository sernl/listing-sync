//! The live-proof provisioner: the ledger rows one supervised end-to-end
//! engine run needs, written through the same repositories the API path
//! writes through, so the roles and the row-level security a live run leans
//! on are exercised rather than bypassed.
//!
//! Five modes, in the order a proof uses them:
//!
//!   live_provision create    <db-url> <inventory> <kek-path> <store-root> \
//!                            <payload> <cover> [--price <minor units>]
//!   live_provision preflight <db-url> <kek-path> <store-root> <mapping-id>
//!   live_provision retitle   <db-url> <mapping-id> <suffix>
//!   live_provision remove    <db-url> <mapping-id>
//!   live_provision show      <db-url> <mapping-id>
//!
//! `<inventory>` is `tpt` or `tes-gb`, and `create` is the only mode that
//! takes one: every other mode names a mapping, and a mapping already records
//! which inventory it is for. Reading it there rather than accepting it again
//! is what makes the two impossible to disagree.
//!
//! `create` seeds the crosswalk, stores both files as real per-tenant blobs,
//! inserts the product and an unbound mapping, links the connection the lease
//! scan gates on, and enqueues one create item; `--price` makes the fixture a
//! paid listing in the inventory's own currency. `preflight` runs the ledger's
//! own admission and projection over that mapping and renders the field set
//! its target would receive, without leasing anything and without a byte
//! leaving the process. `retitle` appends to the canonical title, which is the
//! write a seller-facing edit makes and what gives a revise something to
//! carry. `remove` reads the listing the create bound and enqueues the removal
//! that names it, which the engine admits only while the mapping still holds
//! that exact identifier — and only while the mapping records the listing as a
//! draft, so a listing this proof published is cleaned up through the
//! adapter's own delete-by-id path instead. `show` prints the mapping, item
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
    RightsDeclaration, TermKind, VocabularyId, VocabularyPath,
};
use tam_engine::seed::{prepare_item, ItemPreparation};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::transport::{HttpRequest, HttpResponse, Transport, TransportError};
use tam_marketplace::{
    FieldSet, FileContent, FileSource, FileSourceError, IdempotencyKey, ListingState,
    MarketplaceAdapter as _, ProjectedListing, RemoteLifecycle,
};
use tam_marketplace_tes::endpoints::TesLicence;
use tam_marketplace_tes::TesAdapter;
use tam_marketplace_tpt::write_model;
use tam_pipeline::store::LocalObjectStore;
use tam_secrets::Kek;
use tam_storage::{
    BlobRepo, JobRepo, LeasedItem, MappingRepo, NewJob, NewJobItem, PipelineFileSource,
    ProductRepo, SessionRepo, TaxonomyRepo,
};
use tam_types::{
    CanonicalTermId, ContentHash, CopyFormat, CurrencyRule, FileBytes, FileId, FileKind, FileRole,
    InventoryId, JobId, ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, PriceRule,
    ProductFile, ProductId, ScanOutcome, Stamp, SystemComponent, Timestamp, Title, Uuid,
};

const USAGE: &str = "usage: live_provision create <db-url> <inventory> <kek-path> <store-root> \
                     <payload> <cover> [--price <minor units>] | live_provision preflight \
                     <db-url> <kek-path> <store-root> <mapping-id> | live_provision retitle \
                     <db-url> <mapping-id> <suffix> | live_provision remove <db-url> \
                     <mapping-id> | live_provision show <db-url> <mapping-id>";

/// The dev organisation `just dev-session --ensure-org founder-dev` mints,
/// and the one `TAM_TPT_ORG` must name for the worker to drive TPT items.
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
/// The connection row minted where the organisation holds none for the
/// marketplace yet. It holds no secret, which is all a TPT run needs — the
/// worker's cookie jar is the credential and this row is only the queue gate
/// `LeaseRepo::acquire` requires. A Tes run reaches its account through the
/// broker, so a row minted here would gate the queue open on a credential
/// that does not exist; the upsert below keeps an existing row's id, and with
/// it the sealed secret the broker reads.
const CONNECTION: Uuid = Uuid([0x3C; 16]);
/// The one crosswalked subject this fixture projects through.
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x7A; 16]));
/// The licence this fixture's rights name, where the target binds a licence
/// axis, and the grade band it declares. Both are minted only where the hub
/// holds no term for the native id already.
const LICENCE: CanonicalTermId = CanonicalTermId(Uuid([0x7B; 16]));
const GRADE: CanonicalTermId = CanonicalTermId(Uuid([0x7C; 16]));
/// What this fixture's own terms and edges are labelled by, in the decider
/// and in the first segment of every path it mints.
const FIXTURE: &str = "live_provision fixture";
/// The Tes category the fixture projects onto: node 1000448, "Algebra" under
/// Mathematics, which is the id `endpoints::probe_listing` carries and the one
/// the M0 spike's create posted live. Two segments, deliberately:
/// `derive_crosswalk` writes a seeded subject as one segment and a seeded
/// topic as two, so a two-segment subject path is one no seeded edge claims,
/// and `projection_edge_exact_reverse` admits only one term per path.
const TES_CATEGORY: &str = "1000448";
/// The Tes age band the fixture declares: `ageRanges` 4, which the 2026-08-28
/// publish capture carried and which `AGE_BANDS` spans ages 11 to 14.
const TES_AGE_BAND: &str = "4";
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
/// inside its own transactions and keeps the helper crate-private, so the
/// raw statements below carry their own.
async fn pin(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(ORG.0.to_hyphenated())
        .execute(&mut **tx)
        .await
        .map(drop)
}

/// The terms and edges one fixture needs in the hub for its declarations to
/// ingest and its subject to project.
struct Crosswalk {
    terms: Vec<CanonicalTerm>,
    edges: Vec<ProjectionEdge>,
}

/// Everything one inventory's fixture differs in.
struct Fixture {
    inventory: InventoryId,
    price: PriceIntent,
    grades: GradeDeclaration,
    rights: RightsDeclaration,
    crosswalk: Crosswalk,
}

/// The term a fixture mints for an axis where the hub holds none.
struct Minted {
    term: CanonicalTermId,
    label: String,
    segments: Vec<String>,
}

/// What an axis resolved to: the path the product declares, and whatever the
/// fixture must seed for it to ingest.
struct Resolved {
    path: VocabularyPath,
    crosswalk: Crosswalk,
}

fn fixture_edge(from: CanonicalTermId, to: VocabularyPath, at: Timestamp) -> ProjectionEdge {
    ProjectionEdge {
        from,
        to,
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: FIXTURE.to_owned(),
        },
        decided_at: at,
    }
}

/// The canonical subject the fixture carries. One term across both targets,
/// so a product provisioned into either projects through the same catalogue
/// entry.
fn subject_term() -> CanonicalTerm {
    CanonicalTerm {
        id: SUBJECT,
        kind: TermKind::Subject,
        parent: None,
        label: "Algebra".to_owned(),
    }
}

/// The path a declaration names on an axis a seeded hub may already hold.
///
/// `projection_edge_exact_reverse` admits one Exact edge per target path and
/// `ingest_by_native_id` refuses a native id two terms claim, so a fixture
/// that always minted its own term would fail its insert against a hub seeded
/// from the committed captures, and one that always assumed the hub would
/// declare a path nothing recognises against an empty one. It reads first and
/// mints only what is missing.
async fn resolved_axis(
    taxonomy: &TaxonomyRepo,
    vocabulary: VocabularyId,
    native: &str,
    minted: Minted,
    at: Timestamp,
) -> Result<Resolved, Failure> {
    let held = taxonomy.edges_into(vocabulary).await?;
    let mut claiming = held.iter().filter(|edge| {
        edge.kind == EdgeKind::Exact && edge.to.native_id.as_deref() == Some(native)
    });
    if let Some(edge) = claiming.next() {
        if claiming.next().is_some() {
            return Err(format!(
                "two terms claim {native:?} in {vocabulary:?}; the hub is ambiguous there and \
                 no declaration against it can ingest"
            )
            .into());
        }
        return Ok(Resolved {
            path: edge.to.clone(),
            crosswalk: Crosswalk {
                terms: Vec::new(),
                edges: Vec::new(),
            },
        });
    }
    let path = VocabularyPath {
        vocabulary,
        segments: minted.segments,
        native_id: Some(native.to_owned()),
    };
    Ok(Resolved {
        path: path.clone(),
        crosswalk: Crosswalk {
            terms: vec![CanonicalTerm {
                id: minted.term,
                kind: vocabulary.1,
                parent: None,
                label: minted.label,
            }],
            edges: vec![fixture_edge(minted.term, path, at)],
        },
    })
}

/// `algebra` and `not-grade-specific` are TPT's own slugs, taken from the
/// captured create that `tam-marketplace-tpt`'s `live_write` proved live.
/// Neither names a seller shelf, so nothing lands on one of the founder's own
/// categories. TPT holds no licence field anywhere on its wire, so a grant
/// declared here would be a disclosed loss rather than a value, and the
/// fixture states none.
fn tpt_fixture(price: PriceIntent, at: Timestamp) -> Fixture {
    let phase = VocabularyId(InventoryId::Tpt, TermKind::Phase);
    Fixture {
        inventory: InventoryId::Tpt,
        price,
        grades: GradeDeclaration {
            source: DeclarationSource::Imported { vocabulary: phase },
            raw: vec![VocabularyPath {
                vocabulary: phase,
                segments: vec!["Not grade specific".to_owned()],
                native_id: Some("not-grade-specific".to_owned()),
            }],
            derived: None,
        },
        rights: RightsDeclaration::Unstated,
        crosswalk: Crosswalk {
            terms: vec![subject_term()],
            edges: vec![fixture_edge(
                SUBJECT,
                VocabularyPath {
                    vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Subject),
                    segments: vec!["Math".to_owned(), "Algebra".to_owned()],
                    native_id: Some("algebra".to_owned()),
                },
                at,
            )],
        },
    }
}

/// Tes requires a licence and will never let one be chosen on the seller's
/// behalf, so the fixture states one: the free branch takes `CC-BY` and a
/// priced one takes `TES-PAID`, which is the only paid token Tes writes and
/// the only one `paid_price_token` accepts beside a price.
async fn tes_fixture(
    taxonomy: &TaxonomyRepo,
    inventory: InventoryId,
    price: PriceIntent,
    at: Timestamp,
) -> Result<Fixture, Failure> {
    let token = match price {
        PriceIntent::Free => TesLicence::CcBy,
        PriceIntent::Paid(_) => TesLicence::TesPaid,
    }
    .as_str();
    let licence = resolved_axis(
        taxonomy,
        VocabularyId(inventory, TermKind::Licence),
        token,
        Minted {
            term: LICENCE,
            label: token.to_owned(),
            segments: vec![FIXTURE.to_owned(), token.to_owned()],
        },
        at,
    )
    .await?;
    let phase = VocabularyId(inventory, TermKind::Phase);
    let grade = resolved_axis(
        taxonomy,
        phase,
        TES_AGE_BAND,
        Minted {
            term: GRADE,
            label: "11-14".to_owned(),
            segments: vec![FIXTURE.to_owned(), "11-14".to_owned()],
        },
        at,
    )
    .await?;
    let mut terms = vec![subject_term()];
    let mut edges = vec![fixture_edge(
        SUBJECT,
        VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Subject),
            segments: vec!["Mathematics".to_owned(), "Algebra".to_owned()],
            native_id: Some(TES_CATEGORY.to_owned()),
        },
        at,
    )];
    for held in [licence.crosswalk, grade.crosswalk] {
        terms.extend(held.terms);
        edges.extend(held.edges);
    }
    Ok(Fixture {
        inventory,
        price,
        grades: GradeDeclaration {
            source: DeclarationSource::Imported { vocabulary: phase },
            raw: vec![grade.path],
            derived: None,
        },
        rights: RightsDeclaration::Declared {
            source: licence.path,
        },
        crosswalk: Crosswalk { terms, edges },
    })
}

async fn fixture(
    taxonomy: &TaxonomyRepo,
    inventory: InventoryId,
    price: PriceIntent,
    at: Timestamp,
) -> Result<Fixture, Failure> {
    match inventory {
        InventoryId::Tpt => Ok(tpt_fixture(price, at)),
        InventoryId::Tes => tes_fixture(taxonomy, inventory, price, at).await,
        InventoryId::Etsy => {
            Err(format!("{inventory:?} has no fixture here; this provisions tpt and tes").into())
        }
    }
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
    fixture: &Fixture,
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
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            ProductFile {
                id: FileId(seeded(at, 0x11)),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                bytes: FileBytes::Held {
                    hash: stored.payload,
                    byte_len: stored.payload_len,
                    scan: ScanOutcome::Clean { at },
                },
            },
            vec![],
        )),
        // Tpt uploads the payload alone, but the projection's cover gate is
        // unconditional, so the cover is a real stored blob rather than a row
        // pointing at bytes that were never written.
        cover: Some(ProductFile {
            id: FileId(seeded(at, 0x12)),
            role: FileRole::Cover,
            kind: FileKind::Image,
            bytes: FileBytes::Held {
                hash: stored.cover,
                byte_len: stored.cover_len,
                scan: ScanOutcome::Clean { at },
            },
        }),
        previews: vec![],
        subjects: vec![SUBJECT],
        grades: fixture.grades.clone(),
        price: fixture.price,
        rights: fixture.rights.clone(),
        native_residue: vec![],
    }
}

fn fixture_mapping(fixture: &Fixture, id: MappingId, product: ProductId) -> Mapping {
    Mapping {
        id,
        org: ORG,
        product,
        inventory: fixture.inventory,
        policies: FieldPolicies {
            title: FieldPolicy::Managed,
            description: FieldPolicy::Managed,
            price: FieldPolicy::Managed,
            taxonomy: FieldPolicy::Managed,
            grades: FieldPolicy::Managed,
            files: FieldPolicy::Managed,
        },
        price_rule: PriceRule::Explicit(fixture.price),
        publish: PublishMode::DryRun,
        // The create binds the lifecycle its read-back observes; nothing has
        // been observed yet, and `Absent` is the only honest starting value.
        binding: Binding::Unbound,
        lifecycle: RemoteLifecycle::Absent,
    }
}

/// The queue gate. `LeaseRepo::acquire`'s candidate CTE admits an item only
/// where its org holds a `linked` connection for the marketplace, and
/// `gate_connection` flipping this row to `needs_reauth` still stops the
/// inventory's items even where the secret it would carry was never the one
/// sent.
///
/// The upsert names the marketplace, never the row: `connection` is unique on
/// `(org_id, marketplace)` and the conflict clause writes `state` alone, so an
/// account already linked keeps its id — and `connection_secret`, which is
/// keyed on that id, keeps the sealed credential the broker reads. What was
/// inserted or found is printed, because that is the only thing that
/// distinguishes a gate opened over a real credential from one opened over
/// nothing.
async fn link_connection(pool: &PgPool, marketplace: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    pin(&mut tx).await?;
    let row = sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, $3, 'linked', now(), now()) \
         ON CONFLICT (org_id, marketplace) DO UPDATE SET state = 'linked', updated_at = now() \
         RETURNING id::text AS id, (xmax = 0) AS inserted",
    )
    .bind(ORG.0.to_hyphenated())
    .bind(CONNECTION.to_hyphenated())
    .bind(marketplace)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    println!(
        "  connection   {} on {marketplace} ({})",
        row.try_get::<String, _>("id")?,
        if row.try_get::<bool, _>("inserted")? {
            "minted here, so it carries no secret"
        } else {
            "already linked; its sealed secret is untouched"
        },
    );
    Ok(())
}

/// One job carrying one item, which is the shape both halves of the proof
/// take.
async fn enqueue_one(
    pool: &PgPool,
    inventory: InventoryId,
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
                inventory,
                stamp: Stamp::system(SystemComponent::Worker, at),
            },
            &[NewJobItem {
                item,
                mapping,
                idempotency_key: derive_idempotency_key(
                    ORG,
                    inventory,
                    product,
                    intent_version,
                    payload_hash,
                ),
                operation,
                requires_bound_on: None,
            }],
        )
        .await?;
    Ok((job, item))
}

/// What `create` was told to build from.
struct CreateInputs<'a> {
    inventory: InventoryId,
    price: PriceIntent,
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
    let taxonomy = TaxonomyRepo::new(pool.clone());
    let fixture = fixture(&taxonomy, inputs.inventory, inputs.price, now).await?;
    let seeded_crosswalk = taxonomy
        .seed(&fixture.crosswalk.terms, &fixture.crosswalk.edges)
        .await?;

    let blobs = BlobRepo::new(
        pool.clone(),
        LocalObjectStore::new(PathBuf::from(inputs.store_root)),
        load_kek(inputs.kek_path)?,
    );
    let stored = store_files(&blobs, inputs.payload, inputs.cover, now).await?;

    let product = ProductId(seeded(now, 0x01));
    let mapping = MappingId(seeded(now, 0x31));
    let canonical = fixture_product(&fixture, product, title.clone(), &stored, now);
    ProductRepo::new(pool.clone())
        .insert(ORG, &canonical, now)
        .await?;
    MappingRepo::new(pool.clone())
        .insert(ORG, &fixture_mapping(&fixture, mapping, product), 0, now)
        .await?;

    let (job, item) = enqueue_one(
        pool,
        fixture.inventory,
        ItemOperation::Create,
        (product, mapping, stored.payload, CREATE_INTENT),
        now,
    )
    .await?;

    println!("provisioned a create for {title:?}");
    println!("  inventory    {:?}", fixture.inventory);
    println!("  price        {:?}", fixture.price);
    println!("  crosswalk    {seeded_crosswalk:?}");
    println!("  org          {}", ORG.0.to_hyphenated());
    link_connection(pool, marketplace_of(fixture.inventory)).await?;
    println!("  product      {}", product.0.to_hyphenated());
    println!("  mapping      {}", mapping.0.to_hyphenated());
    println!("  job          {}", job.0.to_hyphenated());
    println!("  item         {}", item.0.to_hyphenated());
    Ok(())
}

/// The `connection.marketplace` value one inventory's account is held under.
/// The column's own check constraint is the closed set.
const fn marketplace_of(inventory: InventoryId) -> &'static str {
    match inventory {
        InventoryId::Tes => "tes",
        InventoryId::Tpt => "tpt",
        InventoryId::Etsy => "etsy",
    }
}

/// The canonical write a seller-facing edit makes, so a revise has something
/// to carry: `POST /v1/jobs` on a bound mapping lowers to `Revise`, which
/// re-projects the product, and with the product unchanged that is a no-op
/// re-post.
///
/// The tenant is pinned rather than assumed: the statement is raw, so the
/// row-level-security policy sees a pin only if this puts one there.
async fn retitle(pool: &PgPool, mapping: MappingId, suffix: &str) -> Result<(), Failure> {
    let record = MappingRepo::new(pool.clone())
        .get(ORG, mapping)
        .await?
        .ok_or("no such mapping in this organisation")?;
    let mut tx = pool.begin().await?;
    pin(&mut tx).await?;
    let row = sqlx::query(
        "UPDATE product SET title = title || $3, updated_at = now() \
         WHERE org_id = $1::uuid AND id = $2::uuid AND deleted_at IS NULL \
         RETURNING title",
    )
    .bind(ORG.0.to_hyphenated())
    .bind(record.mapping.product.0.to_hyphenated())
    .bind(suffix)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or("the mapped product has gone")?;
    tx.commit().await?;
    println!(
        "retitled {} to {:?}",
        record.mapping.product.0.to_hyphenated(),
        row.try_get::<String, _>("title")?
    );
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
             removal whose stated state diverges from the mapping is parked rather than run. \
             A listing this proof published is taken down through the adapter's own \
             delete-by-id path instead",
            record.mapping.lifecycle
        )
        .into());
    }
    let product = ProductRepo::new(pool.clone())
        .get(ORG, record.mapping.product)
        .await?
        .ok_or("the mapped product has gone")?;
    // `digest` rather than a match on the arm, and this is the one place the
    // verified-versus-asserted distinction genuinely does not change the
    // answer: the key covers the bytes that will be uploaded, and both arms
    // describe exactly those bytes. It is the same reasoning `job_reads`
    // applies in SQL, where the equivalent select coalesces the two columns.
    let payload_hash = product
        .product
        .payload_files()
        .next()
        .ok_or("the mapped product carries no payload file")?
        .bytes
        .digest();

    let (job, item) = enqueue_one(
        pool,
        record.mapping.inventory,
        ItemOperation::Remove {
            subject: id.clone(),
            state: ListingState::Draft,
        },
        (record.mapping.product, mapping, payload_hash, REMOVE_INTENT),
        now,
    )
    .await?;

    println!("provisioned a removal of {id:?}");
    println!("  inventory    {:?}", record.mapping.inventory);
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
    println!("  inventory    {:?}", record.mapping.inventory);
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

/// A transport no preflight reaches, and a file source no projection reads.
///
/// `project_fields` is pure on both adapters; Tes's is a seam method where
/// TPT's is a free function, so rendering the Tes shape needs an adapter and
/// an adapter needs these. Standing them in for the session this mode
/// deliberately does not hold is what makes a preview unable to become a
/// request.
struct NoTransport;

impl Transport for NoTransport {
    fn send(
        &self,
        _request: HttpRequest,
    ) -> impl core::future::Future<Output = Result<HttpResponse, TransportError>> + Send {
        core::future::ready(Err(TransportError::AfterSend {
            detail: "the preflight holds no session and sends nothing".to_owned(),
        }))
    }
}

/// See [`NoTransport`].
struct NoFiles;

impl FileSource for NoFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Missing(file)))
    }
}

/// The wire shape the target's own adapter renders from the projection.
fn render(inventory: InventoryId, projected: &ProjectedListing) -> Result<FieldSet, Failure> {
    match inventory {
        InventoryId::Tpt => write_model::project_fields(projected)
            .map_err(|error| format!("Tpt refused the projection: {error:?}").into()),
        InventoryId::Tes => TesAdapter::new(inventory, NoTransport, NoFiles)?
            .project_fields(projected)
            .map_err(|error| format!("Tes refused the projection: {error:?}").into()),
        InventoryId::Etsy => Err("no adapter renders Etsy".into()),
    }
}

/// What `preflight` was told to read blobs back through.
struct PreflightInputs<'a> {
    kek_path: &'a str,
    store_root: &'a str,
}

/// The whole path up to the first outbound byte, run against the real
/// ledger: the admission `prepare_item` applies, the projection it produces,
/// the wire shape the target's write model renders from it, and the payload
/// blob read back through the same `FileSource` the worker hands its adapter.
///
/// The inventory is the mapping's own, never an argument: the ledger projects
/// into the inventory the mapping records, and a preview that took a second
/// answer could render a shape the live run would never send.
///
/// The item is synthesised rather than leased. `prepare_item` reads a leased
/// item's organisation, mapping, inventory and operation and nothing else, so
/// the identifiers and the epoch below are inert; leasing for a preview would
/// bump the item's epoch and hand the real run a stolen lease to recover
/// from. A removal renders nothing — `seed_for_removal` builds an empty field
/// set — so the create is the only projection there is to preview.
async fn preflight(
    pool: &PgPool,
    inputs: &PreflightInputs<'_>,
    mapping: MappingId,
) -> Result<(), Failure> {
    let now = wall_now()?;
    let record = MappingRepo::new(pool.clone())
        .get(ORG, mapping)
        .await?
        .ok_or("no such mapping in this organisation")?;
    let inventory = record.mapping.inventory;
    let lease = LeasedItem {
        org: ORG,
        stranded_attempt: None,
        stranded_title: None,
        item: JobItemId(Uuid([0; 16])),
        job: JobId(Uuid([0; 16])),
        mapping,
        inventory,
        idempotency_key: IdempotencyKey(Uuid([0; 16])),
        operation: ItemOperation::Create,
        lease_epoch: 0,
        attempt_count: 0,
        requires_bound_on: None,
    };
    let projected = match prepare_item(pool, &lease, now).await? {
        ItemPreparation::CounterpartLost { counterpart } => {
            return Err(format!("the counterpart on {counterpart:?} never bound").into())
        }
        ItemPreparation::Ready { projected, .. } => {
            projected.ok_or("a create must project a listing")?
        }
        ItemPreparation::Blocked { gate, raised } => {
            return Err(format!(
                "the ledger would park this item on {gate} ({} raised, {} already open); the \
                 live run would park the same way",
                raised.new, raised.already_open
            )
            .into())
        }
    };
    let fields = render(inventory, &projected)?;
    println!("the item projects; {inventory:?} would receive");
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

/// The inventories this fixture provisions, named as the operator names them
/// rather than as the enum spells them.
fn inventory_argument(raw: Option<&String>) -> Result<InventoryId, Failure> {
    match raw.ok_or(USAGE)?.as_str() {
        "tpt" => Ok(InventoryId::Tpt),
        "tes-gb" => Ok(InventoryId::Tes),
        other => Err(
            format!("{other:?} is not a provisionable inventory; expected tpt or tes-gb").into(),
        ),
    }
}

/// The price the fixture states, in the inventory's own currency: a listing
/// is priced in one currency per marketplace and the seller does not choose
/// it, so naming one on the command line would only be a way to disagree with
/// the registry.
fn price_argument(arguments: &[String], inventory: InventoryId) -> Result<PriceIntent, Failure> {
    let named = arguments
        .iter()
        .position(|argument| argument == "--price")
        .map(|at| {
            arguments
                .get(at.saturating_add(1))
                .ok_or("--price needs an amount in minor units")
        });
    let Some(raw) = named.transpose()? else {
        return Ok(PriceIntent::Free);
    };
    let minor_units: i64 = raw.parse()?;
    match inventory.currency_rule() {
        CurrencyRule::Fixed(currency) => Ok(PriceIntent::Paid(Money::new(minor_units, currency)?)),
        CurrencyRule::SellerScoped | CurrencyRule::Unmeasured => Err(format!(
            "{inventory:?} has no measured fixed currency, so a price cannot be stated for it"
        )
        .into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mode = arguments.first().ok_or(USAGE)?.as_str();
    let pool = connect(arguments.get(1).ok_or(USAGE)?).await?;
    match mode {
        "create" => {
            let inventory = inventory_argument(arguments.get(2))?;
            create(
                &pool,
                &CreateInputs {
                    inventory,
                    price: price_argument(&arguments, inventory)?,
                    kek_path: arguments.get(3).ok_or(USAGE)?,
                    store_root: arguments.get(4).ok_or(USAGE)?,
                    payload: arguments.get(5).ok_or(USAGE)?,
                    cover: arguments.get(6).ok_or(USAGE)?,
                },
            )
            .await
        }
        "preflight" => {
            preflight(
                &pool,
                &PreflightInputs {
                    kek_path: arguments.get(2).ok_or(USAGE)?,
                    store_root: arguments.get(3).ok_or(USAGE)?,
                },
                mapping_argument(arguments.get(4))?,
            )
            .await
        }
        "retitle" => {
            retitle(
                &pool,
                mapping_argument(arguments.get(2))?,
                arguments.get(3).ok_or(USAGE)?,
            )
            .await
        }
        "remove" => remove(&pool, mapping_argument(arguments.get(2))?).await,
        "show" => show(&pool, mapping_argument(arguments.get(2))?).await,
        _ => Err(USAGE.into()),
    }
}
