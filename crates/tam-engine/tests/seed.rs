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
use tam_domain::equivalence::{
    ElectionAnswer, ElectionRule, ElectionTriggerKind, NewElectionRule, PricingBranch,
};
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_engine::driver::{seed_refused, DriverContext, NowSource, RunVerdict};
use tam_engine::seed::{prepare_item, seed_from_projection, ItemPreparation};
use tam_marketplace::cassette::{Cassette, CassetteTransport};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::{FileContent, FileSource, FileSourceError};
use tam_marketplace_tes::TesAdapter;
use tam_storage::{ElectionRepo, LeaseRepo, LeasedItem, MappingRepo, ProductRepo, TaxonomyRepo};
use tam_types::{
    CanonicalTermId, ContentHash, CopyFormat, FieldKey, FileId, FileKind, FileRole, InventoryId,
    JobId, ListingCopy, MappingId, OrgId, PayloadSet, PriceIntent, PriceRule, ProductFile,
    ProductId, ScanOutcome, Timestamp, Title, UserId, Uuid,
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
async fn provision_lifecycle(
    pool: &PgPool,
    with_nz_edge: bool,
    binding: tam_domain::Binding,
    lifecycle: tam_marketplace::RemoteLifecycle,
) {
    provision_undecided(pool, with_nz_edge, binding, lifecycle).await;
    seed_licence_policy(pool).await;
}

/// The same fixture with no standing licence policy, so the licence axis
/// raises the `Supply` election the tests about answering are about.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision_undecided(
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
    let product = canonical_product(PRODUCT, 0x21);
    ProductRepo::new(pool.clone())
        .insert(ORG, &product, NOW)
        .await
        .expect("the product inserts");
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &mapping_of(MAPPING, PRODUCT, binding, lifecycle),
            0,
            NOW,
        )
        .await
        .expect("the mapping inserts");
}

/// The seller's standing licence policy, without which every projection parks
/// on the licence election rather than reaching the gate the test is about.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn seed_licence_policy(pool: &PgPool) {
    let elections = ElectionRepo::new(pool.clone());
    for inventory in [InventoryId::TesGb, InventoryId::TesNz] {
        let rule = ElectionRule::new(NewElectionRule {
            org: ORG,
            inventory,
            axis: TermKind::Licence,
            trigger_kind: ElectionTriggerKind::Supply,
            trigger_key: Some(PricingBranch::Free.as_str().to_owned()),
            answer: ElectionAnswer::Value {
                path: licence_path(inventory, "CC-BY-SA"),
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
}

fn licence_path(inventory: InventoryId, token: &str) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(inventory, TermKind::Licence),
        segments: vec![token.to_owned()],
        native_id: Some(token.to_owned()),
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn canonical_product(id: ProductId, files: u8) -> tam_domain::CanonicalProduct {
    tam_domain::CanonicalProduct {
        id,
        org: ORG,
        title: Title("Fractions practice".to_owned()),
        body: ListingCopy {
            body: "A worksheet.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([files; 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([0x51; 32]),
                byte_len: 4,
                scan: ScanOutcome::Clean { at: NOW },
            },
            vec![],
        ),
        cover: Some(ProductFile {
            id: FileId(Uuid([files.wrapping_add(1); 16])),
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
    }
}

fn mapping_of(
    id: MappingId,
    product: ProductId,
    binding: tam_domain::Binding,
    lifecycle: tam_marketplace::RemoteLifecycle,
) -> tam_domain::Mapping {
    tam_domain::Mapping {
        id,
        org: ORG,
        product,
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
    }
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
        requires_bound_on: None,
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
    assert_eq!(
        entry(&seed, FieldKey::Price),
        "CC-BY-SA",
        "the licence is the one the seller's standing rule elected, not a default"
    );
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
    assert!(
        grades.get("ages").is_none() && grades.get("mainAge").is_none(),
        "both derived age fields belong to the ageRanges vocabulary -- mainAge names one of \
         the seven GB bands rather than an age in years -- so a year-group listing states \
         neither rather than filling them from a table that does not address it: {grades}"
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

/// The whole safety property of a migrate. The source listing does not go
/// until the target listing exists and the driver's own verification read saw
/// it, so the unsafe direction -- source gone, target absent -- is
/// unreachable and the failure mode is a duplicate.
/// Publish-to-live from nothing is two writes, and the second cannot name its
/// subject in advance: the create binds the id minutes after the seller
/// asked. The lowering happens where the id first exists.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_publish_resolves_its_subject_from_the_binding_the_create_wrote(pool: PgPool) {
    let bound = tes("https://www.tes.com/teaching-resource/fractions-9001");
    provision_lifecycle(
        &pool,
        true,
        tam_domain::Binding::Bound {
            id: bound.clone(),
            first_seen: NOW,
            verified: tam_domain::Verification::Clean { at: NOW },
        },
        tam_marketplace::RemoteLifecycle::Draft,
    )
    .await;
    let outcome = prepare_item(
        &pool,
        &leasing(tam_domain::ItemOperation::Publish {
            to: tam_marketplace::ListingState::Live,
        }),
        NOW,
    )
    .await
    .expect("the preparation runs");
    let ItemPreparation::Ready { operation, .. } = outcome else {
        panic!("a bound mapping has a subject to publish");
    };
    assert_eq!(
        operation,
        tam_domain::ItemOperation::Revise {
            subject: bound,
            transition: tam_marketplace::LifecycleTransition {
                from: tam_marketplace::ListingState::Draft,
                to: tam_marketplace::ListingState::Live,
            },
        },
        "the subject is the id the create bound and `from` is the lifecycle we observed, \
         neither of which the seller could have stated when they asked"
    );
}

/// The variant exists because `Revise` cannot serve this case, so the arm
/// that refuses it has to be the binding rather than a diverged subject:
/// there is no subject to diverge.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_publish_against_an_unbound_mapping_is_refused_for_being_unbound(pool: PgPool) {
    provision(&pool, true, tam_domain::Binding::Unbound).await;
    let outcome = prepare_item(
        &pool,
        &leasing(tam_domain::ItemOperation::Publish {
            to: tam_marketplace::ListingState::Live,
        }),
        NOW,
    )
    .await
    .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, .. } = outcome else {
        panic!("there is nothing to publish yet");
    };
    assert_eq!(gate, "unbound");
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_waits_while_its_counterpart_is_still_unbound(pool: PgPool) {
    provision(&pool, true, tam_domain::Binding::Unbound).await;
    let mut waiting = leasing(tam_domain::ItemOperation::Remove {
        subject: tes("https://www.tes.com/teaching-resource/fractions-9001"),
        state: tam_marketplace::ListingState::Live,
    });
    // The counterpart is the mapping this fixture already wrote, and it is
    // Unbound: the target create has not landed.
    waiting.requires_bound_on = Some(InventoryId::TesNz);
    let outcome = prepare_item(&pool, &waiting, NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, .. } = outcome else {
        panic!("a removal whose counterpart has not bound must not run");
    };
    assert_eq!(
        gate, "awaiting_counterpart",
        "the park names what it waits on, so the report can say why the removal has not run"
    );
}

/// The gate's terminal arm. A counterpart that settled somewhere it will not
/// leave is never going to bind, and without this the waiting item cycles
/// park to queue to park every day forever: it is invisible to every existing
/// reaper, and the job it belongs to reads active for good.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_whose_counterpart_can_never_bind_stops_waiting(pool: PgPool) {
    provision(
        &pool,
        true,
        tam_domain::Binding::AmbiguousCreate {
            attempt: tam_types::AttemptId(Uuid([0x61; 16])),
            candidates: vec![tes("https://www.tes.com/teaching-resource/fractions-9001")],
            since: NOW,
        },
    )
    .await;
    let mut waiting = leasing(tam_domain::ItemOperation::Remove {
        subject: tes("https://www.tes.com/teaching-resource/fractions-9001"),
        state: tam_marketplace::ListingState::Live,
    });
    waiting.requires_bound_on = Some(InventoryId::TesNz);
    let outcome = prepare_item(&pool, &waiting, NOW)
        .await
        .expect("the preparation runs");
    assert!(
        matches!(
            outcome,
            ItemPreparation::CounterpartLost {
                counterpart: InventoryId::TesNz
            }
        ),
        "nobody knows what an ambiguous create did, so removing on the strength of it \
         would be removing on the strength of a guess"
    );
}

/// A severed counterpart is waiting, not lost.
///
/// `admission` admits a fresh create against a severed mapping -- that is what
/// re-creating a listing the seller deleted *is* -- so the paired create will
/// land and bind it. Reading severed as unreachable settled the gated item
/// skipped on whichever of the two the lease order happened to pick first, and
/// the two are inserted with one `created_at` so the tie breaks on a random
/// uuid: a publish-to-live on a re-created listing gave up on a coin flip.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_severed_counterpart_is_waited_for_rather_than_given_up_on(pool: PgPool) {
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
    let mut gated = leasing(tam_domain::ItemOperation::Publish {
        to: tam_marketplace::ListingState::Live,
    });
    gated.requires_bound_on = Some(InventoryId::TesNz);
    let outcome = prepare_item(&pool, &gated, NOW)
        .await
        .expect("the preparation runs");
    assert!(
        matches!(
            outcome,
            ItemPreparation::Blocked {
                gate: tam_engine::seed::AWAITING_COUNTERPART,
                ..
            }
        ),
        "the create that re-makes the severed listing has not run yet, so the gate waits \
         for it rather than settling the publish skipped"
    );
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_removal_runs_once_its_counterpart_has_bound(pool: PgPool) {
    provision(
        &pool,
        true,
        tam_domain::Binding::Bound {
            id: tes("https://www.tes.com/teaching-resource/fractions-9001"),
            first_seen: NOW,
            verified: tam_domain::Verification::Clean { at: NOW },
        },
    )
    .await;
    let mut waiting = leasing(tam_domain::ItemOperation::Remove {
        subject: tes("https://www.tes.com/teaching-resource/fractions-9001"),
        state: tam_marketplace::ListingState::Live,
    });
    waiting.requires_bound_on = Some(InventoryId::TesNz);
    let outcome = prepare_item(&pool, &waiting, NOW)
        .await
        .expect("the preparation runs");
    assert!(
        matches!(outcome, ItemPreparation::Ready { .. }),
        "the counterpart exists and we saw it, which is exactly what the column asks"
    );
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

/// The seller's answer is what un-blocks the job. Without the settled queue
/// reaching the projection, the revive the answer performs requeues an item
/// that re-raises the identical question and parks again, so the decision
/// surface would be inert and only a standing rule could ever release
/// anything.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_answered_election_releases_the_item_it_parked(pool: PgPool) {
    provision_undecided(
        &pool,
        true,
        tam_domain::Binding::Unbound,
        tam_marketplace::RemoteLifecycle::Absent,
    )
    .await;
    let elections = ElectionRepo::new(pool.clone());
    assert!(
        matches!(
            prepare_item(&pool, &lease(), NOW)
                .await
                .expect("the preparation runs"),
            ItemPreparation::Blocked {
                gate: "election",
                ..
            }
        ),
        "an unstated licence against a target that requires one is the seller's question"
    );
    let open = elections.open_items(ORG).await.expect("the queue reads");
    let [question] = open.as_slice() else {
        panic!("exactly one question, got {open:?}");
    };
    assert_eq!(question.axis, TermKind::Licence);

    let report = elections
        .answer(
            ORG,
            tam_storage::NewAnswer {
                item: question.id,
                paths: &[licence_path(InventoryId::TesNz, "CC-BY")],
                at: NOW,
                promote: None,
            },
        )
        .await
        .expect("the answer records");
    assert!(
        !report.promoted,
        "an answer applies to this product alone until the seller says otherwise"
    );

    let ItemPreparation::Ready {
        projected: Some(projected),
        ..
    } = prepare_item(&pool, &lease(), NOW)
        .await
        .expect("the preparation runs")
    else {
        panic!("the answered question no longer blocks");
    };
    assert_eq!(
        projected
            .natives
            .iter()
            .filter(|native| native.axis == TermKind::Licence)
            .filter_map(|native| native.value.native_id.as_deref())
            .collect::<Vec<_>>(),
        vec!["CC-BY"],
        "the licence the projection carries is the one the seller named"
    );
    assert!(
        elections
            .open_items(ORG)
            .await
            .expect("the queue reads")
            .is_empty(),
        "and the re-projection asks nothing, where before it minted a duplicate open row \
         on every pass"
    );
}

/// Promotion is what reconciles "the seller decides" with "do not ask again":
/// one answer, applied forward, and the next product never raises the question
/// at all.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn an_answer_applied_to_the_future_settles_the_next_product_without_asking(pool: PgPool) {
    provision_undecided(
        &pool,
        true,
        tam_domain::Binding::Unbound,
        tam_marketplace::RemoteLifecycle::Absent,
    )
    .await;
    let elections = ElectionRepo::new(pool.clone());
    prepare_item(&pool, &lease(), NOW)
        .await
        .expect("the preparation runs");
    let open = elections.open_items(ORG).await.expect("the queue reads");
    let [question] = open.as_slice() else {
        panic!("exactly one question, got {open:?}");
    };
    let rule = ElectionRule::new(NewElectionRule {
        org: ORG,
        inventory: question.inventory,
        axis: question.axis,
        trigger_kind: question.trigger_kind,
        trigger_key: question.trigger_key.clone(),
        answer: ElectionAnswer::Value {
            path: licence_path(InventoryId::TesNz, "CC-BY"),
        },
        decided_by: Decider::Human {
            user: UserId(Uuid([0x5E; 16])),
            org: ORG,
        },
        decided_at: NOW,
    })
    .expect("a licence value is a legal answer; only delegation is not");
    let report = elections
        .answer(
            ORG,
            tam_storage::NewAnswer {
                item: question.id,
                paths: &[licence_path(InventoryId::TesNz, "CC-BY")],
                at: NOW,
                promote: Some(&rule),
            },
        )
        .await
        .expect("the answer records");
    assert!(report.promoted);
    assert_eq!(
        elections
            .rules(ORG)
            .await
            .expect("the rules read")
            .as_slice(),
        &[rule],
        "the standing rule lands in the same transaction the answer did"
    );

    let second_product = ProductId(Uuid([0x02; 16]));
    let second_mapping = MappingId(Uuid([0x32; 16]));
    ProductRepo::new(pool.clone())
        .insert(ORG, &canonical_product(second_product, 0x23), NOW)
        .await
        .expect("the second product inserts");
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &mapping_of(
                second_mapping,
                second_product,
                tam_domain::Binding::Unbound,
                tam_marketplace::RemoteLifecycle::Absent,
            ),
            0,
            NOW,
        )
        .await
        .expect("the second mapping inserts");
    let mut next = lease();
    next.item = tam_domain::JobItemId(Uuid([0x43; 16]));
    next.mapping = second_mapping;
    assert!(
        matches!(
            prepare_item(&pool, &next, NOW)
                .await
                .expect("the preparation runs"),
            ItemPreparation::Ready { .. }
        ),
        "the second product never asked, because the rule answered before the raise"
    );
    assert!(
        elections
            .open_items(ORG)
            .await
            .expect("the queue reads")
            .is_empty(),
        "five hundred listings must not ask the same question five hundred times"
    );
}

/// The founder's direction as an assertion: a Tes-to-TPT licence drop is a
/// surfaced loss and never a silent one.
///
/// TPT was measured to hold no licence field anywhere on its wire, so no edge
/// could ever answer a question about one and no question is raised. What the
/// seller gets instead is the record the decision surface reads, written by
/// the same projection that dropped the value.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_licence_dropped_into_tpt_is_recorded_against_the_mapping_that_dropped_it(pool: PgPool) {
    provision(&pool, true, tam_domain::Binding::Unbound).await;
    let product = ProductId(Uuid([0x03; 16]));
    let mut declared = canonical_product(product, 0x25);
    declared.rights = tam_domain::RightsDeclaration::Declared {
        source: licence_path(InventoryId::TesGb, "CC-BY"),
    };
    ProductRepo::new(pool.clone())
        .insert(ORG, &declared, NOW)
        .await
        .expect("the declared product inserts");
    let mapping = MappingId(Uuid([0x33; 16]));
    MappingRepo::new(pool.clone())
        .insert(
            ORG,
            &tam_domain::Mapping {
                inventory: InventoryId::Tpt,
                ..mapping_of(
                    mapping,
                    product,
                    tam_domain::Binding::Unbound,
                    tam_marketplace::RemoteLifecycle::Absent,
                )
            },
            0,
            NOW,
        )
        .await
        .expect("the TPT mapping inserts");
    let mut leased = lease();
    leased.item = tam_domain::JobItemId(Uuid([0x44; 16]));
    leased.mapping = mapping;
    leased.inventory = InventoryId::Tpt;
    prepare_item(&pool, &leased, NOW)
        .await
        .expect("the preparation runs");

    let losses = MappingRepo::new(pool.clone())
        .losses(ORG, mapping)
        .await
        .expect("the decision surface reads the mapping's losses");
    let [loss] = losses.as_slice() else {
        panic!("one loss, got {losses:?}");
    };
    assert_eq!(
        (loss.kind, loss.axis),
        (
            tam_domain::equivalence::LossKind::NoTargetField,
            Some(TermKind::Licence)
        ),
        "the Creative Commons grant does not reach TPT, and this row is what makes the \
         drop disclosed rather than silent"
    );
    assert_eq!(loss.detail["value"]["native_id"], "CC-BY");
    assert!(
        ElectionRepo::new(pool.clone())
            .open_items(ORG)
            .await
            .expect("the queue reads")
            .iter()
            .all(|item| item.axis != TermKind::Licence || item.product != product),
        "a measured absence is disclosed, never asked: no edge could answer it"
    );
}

/// The engine connects as its own role; the per-test database name comes from
/// the app pool. `acquire` is a cross-tenant scan and `job_item` carries
/// FORCE ROW LEVEL SECURITY, so the app role sees an empty queue.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &PgPool) -> PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects to the test database")
}

/// The tenant's linked connection, without which `acquire`'s candidate CTE
/// matches nothing and the queue reads empty for a reason the test is not
/// about.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn link_connection(app: &PgPool) {
    let mut tx = app.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO connection (org_id, id, marketplace, state, created_at, updated_at) \
         VALUES ($1, $2, 'tes', 'linked', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG.0 .0))
    .bind(uuid::Uuid::from_bytes(Uuid([0xC8; 16]).0))
    .execute(&mut *tx)
    .await
    .expect("the connection links");
    tx.commit().await.expect("the link commits");
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn gate_of(engine: &PgPool, item: tam_domain::JobItemId) -> (String, Option<String>) {
    sqlx::query_as("SELECT state, blocked_on FROM job_item WHERE org_id = $1 AND id = $2")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .bind(uuid::Uuid::from_bytes(item.0 .0))
        .fetch_one(engine)
        .await
        .expect("the item reads")
}

/// The whole publish-to-live ordering, in the order that used to lose.
///
/// One live intent lowers to a create and a publish inserted with the job's
/// single `created_at`, so `acquire`'s FIFO tie-breaks on the item id and
/// roughly half of all publish-to-live enqueues lease the publish first. Here
/// the ids make that half deterministic. The publish then parks on
/// `awaiting_counterpart` — which is right, and used to be the end of it: the
/// two answer-driven revives were wired to the reconciliation and election
/// gates, the bind path called neither, and the twenty-four-hour park expiry
/// was the only thing that ever cleared it. The create lands seconds later.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_publish_that_leased_before_its_create_is_woken_by_the_binding(pool: PgPool) {
    provision(&pool, true, tam_domain::Binding::Unbound).await;
    link_connection(&pool).await;
    let engine = engine_pool(&pool).await;

    // The lowering is the API's own, so the pair under test is the pair a
    // seller's one live request actually produces.
    let seeds = tam_storage::JobReadRepo::new(pool.clone())
        .mapping_seeds(ORG, InventoryId::TesNz, &[MAPPING])
        .await
        .expect("the seed reads");
    let [seed] = seeds.as_slice() else {
        panic!("one mapping, one seed: {seeds:?}");
    };
    let operations = tam_storage::lower(
        tam_marketplace::ListingState::Live,
        InventoryId::TesNz,
        seed,
    )
    .expect("an unbound mapping lowers");
    let job = JobId(Uuid([0x42; 16]));
    // The publish takes the lower id, so the tie-break the queue makes at
    // random is made here on purpose.
    let ids = [
        tam_domain::JobItemId(Uuid([0x20; 16])),
        tam_domain::JobItemId(Uuid([0x10; 16])),
    ];
    let items: Vec<tam_storage::NewJobItem> = operations
        .iter()
        .zip(ids)
        .map(|(operation, item)| tam_storage::NewJobItem {
            item,
            mapping: seed.mapping,
            idempotency_key: derive_idempotency_key(
                ORG,
                InventoryId::TesNz,
                seed.product,
                1,
                tam_storage::intent_digest(
                    operation,
                    job,
                    &seed.payload_hashes,
                    seed.sever_generation,
                ),
            ),
            requires_bound_on: tam_storage::requires_bound_on(operation, InventoryId::TesNz),
            operation: operation.clone(),
        })
        .collect();
    tam_storage::JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &tam_storage::NewJob {
                job,
                inventory: InventoryId::TesNz,
                at: NOW,
            },
            &items,
        )
        .await
        .expect("the live intent enqueues");

    let leases = tam_storage::LeaseRepo::new(engine.clone());
    let publish = leases
        .acquire("w1", NOW, 600)
        .await
        .expect("the scan runs")
        .expect("an item leases");
    assert_eq!(
        (publish.item, publish.requires_bound_on),
        (ids[1], Some(InventoryId::TesNz)),
        "the publish leased first, which is the half of the coin flip this test is about"
    );

    let outcome = prepare_item(&pool, &publish, NOW)
        .await
        .expect("the preparation runs");
    let ItemPreparation::Blocked { gate, .. } = outcome else {
        panic!("a publish whose create has not landed has nothing to name");
    };
    assert_eq!(gate, tam_storage::AWAITING_COUNTERPART);
    leases
        .park(&publish.lease_ref(), gate, Timestamp(NOW.0 + 86_400_000))
        .await
        .expect("the publish parks");

    let create = leases
        .acquire("w1", NOW, 600)
        .await
        .expect("the scan runs")
        .expect("the create leases next");
    assert_eq!(create.item, ids[0]);
    let attempts = tam_storage::WriteAttemptRepo::new(engine.clone());
    let attempt = attempts
        .open(
            &create.lease_ref(),
            MAPPING,
            &tam_storage::AttemptIntent {
                body: serde_json::json!({}),
                hash: vec![0x01],
            },
            NOW,
        )
        .await
        .expect("the attempt opens");
    assert_eq!(
        attempts
            .settle(
                &create.lease_ref(),
                tam_storage::AttemptRef {
                    attempt,
                    mapping: MAPPING
                },
                &tam_storage::AttemptVerdict {
                    state: "committed".to_owned(),
                    failure_code: None,
                    landing: tam_storage::LandingEffect::Landed {
                        id: tes(HELD),
                        lifecycle: tam_marketplace::RemoteLifecycle::Draft,
                    },
                },
                NOW,
            )
            .await
            .expect("the create settles"),
        tam_storage::BindDisposition::Bound,
    );

    assert_eq!(
        gate_of(&engine, ids[1]).await,
        ("queued".to_owned(), None),
        "the binding woke the publish; before this it waited out the full day"
    );
    leases
        .settle(
            &create.lease_ref(),
            &tam_storage::ItemVerdict {
                outcome: tam_domain::ItemOutcome::Succeeded,
                failure_code: None,
                failure_detail: None,
            },
            NOW,
        )
        .await
        .expect("the create's item settles");

    let woken = leases
        .acquire("w1", NOW, 600)
        .await
        .expect("the scan runs")
        .expect("the revived publish leases");
    assert_eq!(woken.item, ids[1]);
    let ItemPreparation::Ready { operation, .. } = prepare_item(&pool, &woken, NOW)
        .await
        .expect("the preparation runs")
    else {
        panic!("the counterpart is bound, so the publish is ready");
    };
    assert_eq!(
        operation,
        tam_domain::ItemOperation::Revise {
            subject: tes(HELD),
            transition: tam_marketplace::LifecycleTransition {
                from: tam_marketplace::ListingState::Draft,
                to: tam_marketplace::ListingState::Live,
            },
        },
        "the publish names the listing the create bound, which is the whole reason the \
         pair is two items rather than one"
    );
}

/// A clock that does not move, so a settle and the scan that follows it are
/// provably the same instant: an item that leases here does so because the
/// mutex was released and not because a lease expired.
struct FixedClock;

impl NowSource for FixedClock {
    fn now(&self) -> Timestamp {
        NOW
    }
}

/// The NZ subject edge as the reconciliation queue's own resolution writes
/// it: the segments a human named, and no native id, because the answer
/// endpoint defaults it to absent and the shipped client sends none.
fn unaddressed_edge() -> ProjectionEdge {
    ProjectionEdge {
        from: SUBJECT,
        to: VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesNz, TermKind::Subject),
            segments: vec!["Maths for early years".to_owned()],
            native_id: None,
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Human {
            user: UserId(Uuid([0x61; 16])),
            org: ORG,
        },
        decided_at: NOW,
    }
}

const REFUSING_JOB: JobId = JobId(Uuid([0x62; 16]));
const FIRST_ITEM: tam_domain::JobItemId = tam_domain::JobItemId(Uuid([0x63; 16]));
const SECOND_ITEM: tam_domain::JobItemId = tam_domain::JobItemId(Uuid([0x64; 16]));

/// The whole fixture the refusal test drives: the mapping projects, its NZ
/// subject path carries no id Tes can address, and two items sit behind the
/// organisation's one live lease.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision_refusing_queue(app: &PgPool, engine: &PgPool) {
    provision(app, false, tam_domain::Binding::Unbound).await;
    TaxonomyRepo::new(app.clone())
        .seed(&[], &[unaddressed_edge()])
        .await
        .expect("the founder's own resolution seeds");
    link_connection(app).await;
    tam_storage::JobRepo::new(engine.clone())
        .enqueue(
            ORG,
            &tam_storage::NewJob {
                job: REFUSING_JOB,
                inventory: InventoryId::TesNz,
                at: NOW,
            },
            &[
                tam_storage::NewJobItem {
                    item: FIRST_ITEM,
                    mapping: MAPPING,
                    idempotency_key: tam_marketplace::IdempotencyKey(Uuid([0x66; 16])),
                    operation: tam_domain::ItemOperation::Create,
                    requires_bound_on: None,
                },
                tam_storage::NewJobItem {
                    item: SECOND_ITEM,
                    mapping: MAPPING,
                    idempotency_key: tam_marketplace::IdempotencyKey(Uuid([0x67; 16])),
                    operation: tam_domain::ItemOperation::Create,
                    requires_bound_on: None,
                },
            ],
        )
        .await
        .expect("the job enqueues");
}

/// One lease, one projection, one refusal: the sequence the worker runs, up
/// to and including what it does with the error.
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "allow-expect-in-tests and allow-panic-in-tests reach #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn refuse_one(app: &PgPool, engine: &PgPool, leases: &LeaseRepo) -> (LeasedItem, RunVerdict) {
    let held = leases
        .acquire("seed-refusal-test", NOW, 600)
        .await
        .expect("the scan runs")
        .expect("an item leases");
    let ItemPreparation::Ready {
        projected: Some(projected),
        ..
    } = prepare_item(app, &held, NOW).await.expect("it prepares")
    else {
        panic!("the mapping projects; it is the rendering that refuses");
    };
    let adapter = TesAdapter::new(
        InventoryId::TesNz,
        CassetteTransport::new(Cassette {
            interactions: vec![],
        }),
        NoFiles,
    )
    .expect("a Tes inventory");
    let error = seed_from_projection(&adapter, &held, &projected)
        .expect_err("a path Tes cannot address is refused rather than rendered");
    let cancel = tokio_util::sync::CancellationToken::new();
    let ctx = DriverContext {
        adapter: &adapter,
        leases,
        halts: &tam_storage::HaltRepo::new(engine.clone()),
        attempts: &tam_storage::WriteAttemptRepo::new(engine.clone()),
        budgets: &tam_storage::RateBudgetRepo::new(engine.clone()),
        pool: engine,
        clock: &FixedClock,
        cancel: &cancel,
        pause: &tam_marketplace::InstantPause,
    };
    let verdict = seed_refused(&ctx, &held, &error, NOW)
        .await
        .expect("the settle runs");
    (held, verdict)
}

/// M2. `project_fields` is the only fallible step in the seed, and it is a
/// pure function of a projection rebuilt from the same durable rows every
/// lease — so its refusal is permanent. Abandoning it held this
/// organisation's one live lease for the whole TTL, five times over, and
/// then settled the item with no detail at all. It settles here instead,
/// carrying the sentence the adapter wrote.
#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_projection_the_adapter_refuses_settles_the_item_rather_than_holding_the_queue(
    app: PgPool,
) {
    let engine = engine_pool(&app).await;
    provision_refusing_queue(&app, &engine).await;
    let leases = LeaseRepo::new(engine.clone());

    let (first, verdict) = refuse_one(&app, &engine, &leases).await;
    assert_eq!(
        verdict,
        RunVerdict::Settled(tam_domain::ItemOutcome::Failed),
        "a refusal the adapter will repeat is an answer, not something to wait on"
    );

    let settled: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT state, outcome, failure_code, failure_detail FROM job_item WHERE id = $1",
    )
    .bind(uuid::Uuid::from_bytes(first.item.0 .0))
    .fetch_one(&engine)
    .await
    .expect("the item row reads back");
    assert_eq!(
        (
            settled.0.as_str(),
            settled.1.as_deref(),
            settled.2.as_deref()
        ),
        ("settled", Some("failed"), Some("UploadRejected")),
        "the closed code crosses into the ledger exactly as a submit-time rejection's does"
    );
    let detail = settled.3.expect("the refusal's own sentence is recorded");
    assert!(
        detail.contains("numeric id"),
        "the seller reads why the term cannot be posted, not an empty failure_detail: {detail}"
    );

    // The same instant, no expiry, no stealer: the mutex is free because the
    // item settled rather than because a lease ran out.
    let (second, verdict) = refuse_one(&app, &engine, &leases).await;
    assert_ne!(
        second.item, first.item,
        "the organisation's queue moved on within the same scan"
    );
    assert_eq!(
        verdict,
        RunVerdict::Settled(tam_domain::ItemOutcome::Failed)
    );

    // The job carries no state column: it settles by emitting its own event,
    // once, when the last item goes terminal.
    let settled: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT kind, payload FROM job_event WHERE job_id = $1 AND kind = 'JobSettled'",
    )
    .bind(uuid::Uuid::from_bytes(REFUSING_JOB.0 .0))
    .fetch_all(&engine)
    .await
    .expect("the job events read back");
    let [(_, payload)] = settled.as_slice() else {
        panic!("every item terminal settles the job exactly once, got {settled:?}");
    };
    assert_eq!(
        (payload["failed"].as_u64(), payload["succeeded"].as_u64()),
        (Some(2), Some(0)),
        "and the tally the seller reads counts both refusals: {payload}"
    );
}
