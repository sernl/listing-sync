//! The seed producer against a live catalogue: a projectable mapping yields
//! the machine seed field-for-field through the real Tes adapter's own
//! rendering, and a blocked one raises its queue items and reports the park
//! gate — the worker cannot lease what it cannot project.
//!
//! The adapter here is the real one over a transport that would refuse every
//! request, because rendering a field set is pure: a projection that reached
//! the network would fail this test rather than pass it.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_engine::seed::{prepare_item, seed_from_projection, ItemPreparation};
use tam_marketplace::cassette::{Cassette, CassetteTransport};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::{FileContent, FileSource, FileSourceError};
use tam_marketplace_tes::TesAdapter;
use tam_storage::{LeasedItem, MappingRepo, ProductRepo, TaxonomyRepo};
use tam_types::{
    CanonicalTermId, ContentHash, CopyFormat, FieldKey, FileId, FileKind, FileRole, InventoryId,
    JobId, ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Timestamp, Title, Uuid,
};

/// The rendering reads no files, so the source is a refusal.
struct NoFiles;

impl FileSource for NoFiles {
    fn fetch(
        &self,
        file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Err(FileSourceError::Missing(file)))
    }
}

const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
const PRODUCT: ProductId = ProductId(Uuid([0x01; 16]));
const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const GRADE: CanonicalTermId = CanonicalTermId(Uuid([0x79; 16]));
const NOW: Timestamp = Timestamp(1_000);

async fn provision(pool: &PgPool, with_nz_edge: bool, binding: tam_domain::Binding) {
    provision_lifecycle(
        pool,
        with_nz_edge,
        binding,
        tam_marketplace::RemoteLifecycle::Absent,
    )
    .await;
}

/// The same fixture with the mapping's recorded lifecycle stated, which is
/// the half of `admission` every other fixture leaves incomparable.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision_lifecycle(
    pool: &PgPool,
    with_nz_edge: bool,
    binding: tam_domain::Binding,
    lifecycle: tam_marketplace::RemoteLifecycle,
) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(pool)
        .await
        .expect("the org seeds");
    let taxonomy = TaxonomyRepo::new(pool.clone());
    let terms = [CanonicalTerm {
        id: SUBJECT,
        kind: TermKind::Subject,
        parent: None,
        label: "Maths for early years".to_owned(),
    }];
    let terms = [
        terms[0].clone(),
        CanonicalTerm {
            id: GRADE,
            kind: TermKind::Phase,
            parent: None,
            label: "Kindergarten".to_owned(),
        },
    ];
    // The grade axis is seeded on both sides deliberately: the product
    // declares the source's own year-group path, and the projection reaches
    // the target by ingesting it into a term and projecting out again, which
    // is what makes the id the target's rather than the source's.
    let mut edges = vec![
        edge(InventoryId::TesGb, "1000454"),
        phase_edge(InventoryId::TesUs, "17"),
        phase_edge(InventoryId::TesNz, "17"),
    ];
    if with_nz_edge {
        edges.push(edge(InventoryId::TesNz, "7000454"));
    }
    taxonomy
        .seed(&terms, &edges)
        .await
        .expect("the crosswalk seeds");

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
                byte_len: 4,
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
            source: tam_domain::DeclarationSource::Imported {
                vocabulary: VocabularyId(InventoryId::TesUs, TermKind::Phase),
            },
            raw: vec![VocabularyPath {
                vocabulary: VocabularyId(InventoryId::TesUs, TermKind::Phase),
                segments: vec!["Kindergarten".to_owned()],
                native_id: Some("17".to_owned()),
            }],
            derived: Some(tam_domain::AgeInterval::new(5, 7).expect("a bounded range")),
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
                binding,
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
                lifecycle,
            },
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
}

fn phase_edge(inventory: InventoryId, native: &str) -> ProjectionEdge {
    ProjectionEdge {
        from: GRADE,
        to: VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Phase),
            segments: vec!["Kindergarten".to_owned()],
            native_id: Some(native.to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: NOW,
    }
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

fn lease() -> LeasedItem {
    leasing(tam_domain::ItemOperation::Create)
}

fn leasing(operation: tam_domain::ItemOperation) -> LeasedItem {
    LeasedItem {
        org: ORG,
        item: tam_domain::JobItemId(Uuid([0x41; 16])),
        job: JobId(Uuid([0x42; 16])),
        mapping: MAPPING,
        inventory: InventoryId::TesNz,
        idempotency_key: derive_idempotency_key(
            ORG,
            InventoryId::TesNz,
            PRODUCT,
            1,
            ContentHash([0x51; 32]),
        ),
        operation,
        lease_epoch: 0,
        attempt_count: 0,
    }
}

fn tes(url: &str) -> tam_marketplace::RemoteListingId {
    tam_marketplace::RemoteListingId::Tes {
        url: url.to_owned(),
    }
}

const HELD: &str = "https://www.tes.com/teaching-resource/fractions-9001";
const OTHER: &str = "https://www.tes.com/teaching-resource/fractions-9002";

fn bound_to(url: &str) -> tam_domain::Binding {
    tam_domain::Binding::Bound {
        id: tes(url),
        first_seen: NOW,
        verified: tam_domain::Verification::Stale { since: NOW },
    }
}

fn removing(url: &str) -> tam_domain::ItemOperation {
    tam_domain::ItemOperation::Remove {
        subject: tes(url),
        state: tam_marketplace::ListingState::Draft,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn entry(seed: &tam_engine::driver::MachineSeed, key: FieldKey) -> String {
    seed.fields
        .entries
        .iter()
        .find(|(field, _)| *field == key)
        .map(|(_, value)| value.clone())
        .expect("the seed carries the field")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_projectable_mapping_seeds_the_machine(pool: PgPool) {
    provision(&pool, true, tam_domain::Binding::Unbound).await;
    let outcome = prepare_item(&pool, &lease(), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Ready {
        projected: Some(projected),
        ..
    } = outcome
    else {
        panic!("a covered mapping seeds");
    };
    let adapter = TesAdapter::new(
        InventoryId::TesNz,
        CassetteTransport::new(Cassette {
            interactions: vec![],
        }),
        NoFiles,
    )
    .expect("a Tes inventory");
    let seed = seed_from_projection(&adapter, &lease(), &projected).expect("the adapter renders");
    assert_eq!(entry(&seed, FieldKey::Title), "Fractions practice");
    assert_eq!(entry(&seed, FieldKey::Price), "CC-BY", "free is CC-BY");
    let taxonomy: serde_json::Value =
        serde_json::from_str(&entry(&seed, FieldKey::Taxonomy)).expect("taxonomy is JSON");
    assert_eq!(
        taxonomy["categories"],
        serde_json::json!([7_000_454]),
        "the NZ category id travelled from the crosswalk into the seed"
    );
    let grades: serde_json::Value =
        serde_json::from_str(&entry(&seed, FieldKey::Grades)).expect("grades are JSON");
    assert_eq!(
        grades["yearGroups"],
        serde_json::json!([17]),
        "an NZ resource takes year groups, and the id is the one the target vocabulary \
         uses rather than the one the source declared"
    );
    assert!(
        grades.get("ageRanges").is_none(),
        "posting a year group under ageRanges would be a wrong field, not a wrong label"
    );
    assert_eq!(
        grades["ages"],
        serde_json::json!([5, 6, 7]),
        "the derived interval expands into the ages list"
    );
    assert_eq!(seed.fields.files, vec![FileId(Uuid([0x21; 16]))]);
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_gap_parks_the_item_behind_the_queue_it_just_raised(pool: PgPool) {
    provision(&pool, false, tam_domain::Binding::Unbound).await;
    let outcome = prepare_item(&pool, &lease(), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, raised } = outcome else {
        panic!("no NZ edge means the seed blocks");
    };
    assert_eq!(gate, "reconciliation");
    assert_eq!(raised.new, 1, "the gap raised its item");
    let open = TaxonomyRepo::new(pool)
        .open_items(ORG)
        .await
        .expect("the queue reads");
    assert_eq!(open.len(), 1, "the founder sees the gap the worker hit");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_bound_mapping_parks_behind_the_binding_gate(pool: PgPool) {
    provision(
        &pool,
        true,
        tam_domain::Binding::Bound {
            id: tam_marketplace::RemoteListingId::Tes {
                url: "https://www.tes.com/teaching-resource/fractions-9001".to_owned(),
            },
            first_seen: NOW,
            verified: tam_domain::Verification::Stale { since: NOW },
        },
    )
    .await;
    let outcome = prepare_item(&pool, &lease(), NOW)
        .await
        .expect("the preparation runs");
    // Blocked rather than Ready is the whole assertion: there is no
    // ProjectedListing to hand seed_from_projection, so the create that would
    // have minted a second listing cannot be built.
    let ItemPreparation::Blocked { gate, raised } = outcome else {
        panic!("a mapping already bound has no create left to make");
    };
    assert_eq!(gate, "binding", "the gate names the binding, not a gap");
    assert_eq!(
        (raised.new, raised.already_open),
        (0, 0),
        "a bound mapping raises no reconciliation item; nothing is missing"
    );
}

/// The item states the listing it means to act on and the mapping's binding
/// stays the authority for it. An unstored subject would let a removal
/// enqueued against listing X silently retarget to listing Y if the mapping
/// were rebound between enqueue and lease, and nothing would notice; storing
/// it is what makes the divergence detectable, and refusing on it is what
/// keeps the mapping authoritative.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operation_naming_a_listing_the_mapping_does_not_hold_is_refused(pool: PgPool) {
    provision(&pool, true, bound_to(OTHER)).await;
    let outcome = prepare_item(&pool, &leasing(removing(HELD)), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, raised } = outcome else {
        panic!("a removal must not run against a listing the mapping does not hold");
    };
    assert_eq!(
        gate, "subject_diverged",
        "the gate names the divergence, not a missing binding: the mapping is bound, \
         just to something else"
    );
    assert_eq!(
        (raised.new, raised.already_open),
        (0, 0),
        "a divergence raises no reconciliation item; nothing is missing"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_operation_against_an_unbound_mapping_is_refused(pool: PgPool) {
    provision(&pool, true, tam_domain::Binding::Unbound).await;
    let outcome = prepare_item(&pool, &leasing(removing(HELD)), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, .. } = outcome else {
        panic!("there is nothing to remove");
    };
    assert_eq!(gate, "unbound", "a removal needs a binding to address");
}

/// A removal reaches `Ready` with no projection at all. Routing it through
/// `project_listing` would let a taxonomy gap park a delete, and this fixture
/// has exactly that gap.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_is_ready_without_a_projection(pool: PgPool) {
    provision(&pool, false, bound_to(HELD)).await;
    let outcome = prepare_item(&pool, &leasing(removing(HELD)), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Ready {
        operation,
        projected,
    } = outcome
    else {
        panic!("a removal against the listing the mapping holds is ready");
    };
    assert_eq!(operation, removing(HELD), "the operation travels whole");
    assert!(
        projected.is_none(),
        "a removal describes nothing, so nothing was projected and no gap could park it"
    );
}

fn removing_live(url: &str) -> tam_domain::ItemOperation {
    tam_domain::ItemOperation::Remove {
        subject: tes(url),
        state: tam_marketplace::ListingState::Live,
    }
}

fn published_at(at: Timestamp) -> tam_marketplace::RemoteLifecycle {
    tam_marketplace::RemoteLifecycle::Live { since: at }
}

/// The 2026-08-29 incident shape, refused before it reaches a marketplace.
/// The stated state selects the delete route — `Draft` posts the overlay
/// delete, `Live` the resource delete — so a stale item stating `draft`
/// against a mapping that has since gone live takes down only the overlay
/// and leaves the listing standing. Nothing downstream can undo that: the
/// gate is the only place it is stoppable.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_item_whose_stated_state_contradicts_the_mapping_is_refused(pool: PgPool) {
    provision_lifecycle(&pool, true, bound_to(HELD), published_at(NOW)).await;
    let outcome = prepare_item(&pool, &leasing(removing(HELD)), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, raised } = outcome else {
        panic!("an item that thinks the listing is a draft must not delete a live one");
    };
    assert_eq!(
        gate, "lifecycle_diverged",
        "the gate names the contradiction between the item's stated state and the one \
         the mapping records"
    );
    assert_eq!(
        (raised.new, raised.already_open),
        (0, 0),
        "a stale state raises no reconciliation item; nothing is missing"
    );
}

/// The other side of the same gate, and what makes the publish path usable
/// at all: a mapping the publish bound `'live'` admits the next item that
/// states it. Were the bind not writing the lifecycle, this would be the
/// permanent park instead.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_published_mapping_admits_the_item_that_states_it(pool: PgPool) {
    provision_lifecycle(&pool, true, bound_to(HELD), published_at(NOW)).await;
    let outcome = prepare_item(&pool, &leasing(removing_live(HELD)), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Ready { operation, .. } = outcome else {
        panic!("an item that states the state the mapping records is admitted");
    };
    assert_eq!(
        operation,
        removing_live(HELD),
        "the operation travels whole through the gate that agreed with it"
    );
}

/// Section K item 16's engine half. The sever exists so a mapping can be
/// re-created through the same row — `mapping_one_per_inventory` leaves no
/// other row to use — and the storage fence admitting `'severed'` is only
/// half of that. If `admission` refused a create here, the sever's whole
/// stated purpose would be unreachable, and no test would say so: the
/// Create arm is a predicate rather than an exhaustive match, so tightening
/// it compiles clean.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_severed_mapping_admits_a_fresh_create(pool: PgPool) {
    provision(
        &pool,
        true,
        tam_domain::Binding::Severed {
            was: tes(HELD),
            noticed: NOW,
            cause: tam_domain::SeverCause::RemovedBySeller,
        },
    )
    .await;
    let outcome = prepare_item(&pool, &lease(), NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Ready {
        projected: Some(_), ..
    } = outcome
    else {
        panic!("a severed mapping holds no listing, so a create has one to make");
    };
}
