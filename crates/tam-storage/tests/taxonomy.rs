//! The durable taxonomy hub: seeding idempotence, the reconciliation queue's
//! dedup and drain, resolution guardrails including the reverse-Exact
//! uniqueness law, and tenant isolation of the queue.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalTerm, Decider, EdgeKind, FieldPolicies, FieldPolicy, Mapping, NoCounterpart,
    ProjectionEdge, PublishMode, TermKind, VocabularyId, VocabularyPath,
};
use tam_marketplace::RemoteLifecycle;
use tam_storage::{DrainStats, MappingRepo, ProductRepo, RaiseScope, StorageError, TaxonomyRepo};
use tam_types::{
    CanonicalTermId, InventoryId, MappingId, OrgId, PriceIntent, PriceRule, Timestamp, Uuid,
};

use common::{minimal_product, seed_org_a, ORG_A};

const T0: Timestamp = Timestamp(1_000);
const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));
const SUBJECT: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
const TOPIC: CanonicalTermId = CanonicalTermId(Uuid([0x78; 16]));

fn subject_term() -> CanonicalTerm {
    CanonicalTerm {
        id: SUBJECT,
        kind: TermKind::Subject,
        parent: None,
        label: "Maths for early years".to_owned(),
    }
}

fn topic_term() -> CanonicalTerm {
    CanonicalTerm {
        id: TOPIC,
        kind: TermKind::Topic,
        parent: Some(SUBJECT),
        label: "Time".to_owned(),
    }
}

fn gb_edge(from: CanonicalTermId, segments: &[&str], native: &str) -> ProjectionEdge {
    edge(from, InventoryId::Tes, segments, native)
}

fn nz_edge(from: CanonicalTermId, segments: &[&str], native: &str) -> ProjectionEdge {
    edge(from, InventoryId::Tpt, segments, native)
}

fn edge(
    from: CanonicalTermId,
    inventory: InventoryId,
    segments: &[&str],
    native: &str,
) -> ProjectionEdge {
    let kind = if segments.len() == 1 {
        TermKind::Subject
    } else {
        TermKind::Topic
    };
    ProjectionEdge {
        from,
        to: VocabularyPath {
            vocabulary: VocabularyId(inventory, kind),
            segments: segments.iter().map(|s| (*s).to_owned()).collect(),
            native_id: Some(native.to_owned()),
        },
        kind: EdgeKind::Exact,
        decided_by: Decider::Imported {
            source: "test fixture".to_owned(),
        },
        decided_at: T0,
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn fixture_mapping(app: &PgPool, org: OrgId) -> MappingId {
    ProductRepo::new(app.clone())
        .insert(org, &minimal_product(), T0)
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(app.clone())
        .insert(
            org,
            &Mapping {
                id: MAPPING_1,
                org,
                product: minimal_product().id,
                inventory: InventoryId::Tpt,
                binding: Binding::Unbound,
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
            },
            0,
            T0,
        )
        .await
        .expect("the fixture mapping inserts");
    MAPPING_1
}

fn scope(mapping: MappingId) -> RaiseScope {
    RaiseScope {
        mapping,
        target: InventoryId::Tpt,
        at: T0,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn seeding_is_idempotent_and_orders_parents_first(app: PgPool) {
    let repo = TaxonomyRepo::new(app);
    let terms = [topic_term(), subject_term()];
    let edges = [
        gb_edge(SUBJECT, &["Maths for early years"], "1000454"),
        nz_edge(SUBJECT, &["Maths for early years"], "7000454"),
        gb_edge(TOPIC, &["Maths for early years", "Time"], "1000732"),
    ];
    let first = repo
        .seed(&terms, &edges)
        .await
        .expect("the first seed runs");
    assert_eq!(
        (first.terms_inserted, first.edges_inserted),
        (2, 3),
        "a child listed before its parent still inserts, parents first"
    );
    let second = repo.seed(&terms, &edges).await.expect("the reseed runs");
    assert_eq!(
        (
            second.terms_inserted,
            second.terms_existing,
            second.edges_inserted,
            second.edges_existing
        ),
        (0, 2, 0, 3),
        "deterministic ids make reseeding a no-op, reported as existing"
    );
    let nz = repo
        .edges_into(VocabularyId(InventoryId::Tpt, TermKind::Subject))
        .await
        .expect("the NZ subject edges load");
    assert_eq!(nz.len(), 1, "one NZ subject edge was seeded");
    assert_eq!(nz[0].from, SUBJECT, "the edge belongs to the subject");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_batch_raises_one_item_per_gap_and_resolution_drains_it(app: PgPool) {
    seed_org_a(&app).await.expect("org-a seeds");
    let mapping = fixture_mapping(&app, ORG_A).await;
    let repo = TaxonomyRepo::new(app);
    repo.seed(
        &[subject_term()],
        &[gb_edge(SUBJECT, &["Maths for early years"], "1000454")],
    )
    .await
    .expect("the term seeds with its GB edge only");

    let causes = [(SUBJECT, TermKind::Subject)];
    let first = repo
        .raise(ORG_A, scope(mapping), &causes)
        .await
        .expect("the first raise runs");
    assert_eq!((first.new, first.already_open), (1, 0), "a gap raises once");
    let second = repo
        .raise(ORG_A, scope(mapping), &causes)
        .await
        .expect("the second raise runs");
    assert_eq!(
        (second.new, second.already_open),
        (0, 1),
        "the second product carrying the same gap deduplicates"
    );

    let open = repo.open_items(ORG_A).await.expect("open items load");
    assert_eq!(open.len(), 1, "one open item for one gap");
    assert_eq!(open[0].term, SUBJECT, "the item names the term");

    repo.resolve_with_edge(
        ORG_A,
        open[0].id,
        &nz_edge(SUBJECT, &["Maths for early years"], "7000454"),
    )
    .await
    .expect("resolution writes the durable edge");

    assert!(
        repo.open_items(ORG_A)
            .await
            .expect("open items reload")
            .is_empty(),
        "the resolved item left the queue"
    );
    let nz = repo
        .edges_into(VocabularyId(InventoryId::Tpt, TermKind::Subject))
        .await
        .expect("the NZ edges load");
    assert_eq!(
        nz.len(),
        1,
        "the durable edge is waiting for the next product carrying the term"
    );
    assert_eq!(
        repo.drain_stats(ORG_A).await.expect("stats load"),
        DrainStats {
            open: 0,
            resolved: 1,
            no_counterpart: 0
        },
        "the drain counters reflect the resolution"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_edge_must_answer_the_item_it_resolves(app: PgPool) {
    seed_org_a(&app).await.expect("org-a seeds");
    let mapping = fixture_mapping(&app, ORG_A).await;
    let repo = TaxonomyRepo::new(app);
    repo.seed(&[subject_term(), topic_term()], &[])
        .await
        .expect("the terms seed");
    repo.raise(ORG_A, scope(mapping), &[(SUBJECT, TermKind::Subject)])
        .await
        .expect("the item raises");
    let open = repo.open_items(ORG_A).await.expect("open items load");
    let wrong_term = repo
        .resolve_with_edge(
            ORG_A,
            open[0].id,
            &nz_edge(TOPIC, &["Maths for early years", "Time"], "7000732"),
        )
        .await;
    assert!(
        matches!(wrong_term, Err(StorageError::Inconsistent { .. })),
        "an edge for a different term cannot resolve the item"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_second_exact_claim_on_a_target_path_is_rejected(app: PgPool) {
    seed_org_a(&app).await.expect("org-a seeds");
    let mapping = fixture_mapping(&app, ORG_A).await;
    let repo = TaxonomyRepo::new(app);
    repo.seed(
        &[subject_term(), topic_term()],
        &[nz_edge(SUBJECT, &["Maths for early years"], "7000454")],
    )
    .await
    .expect("the first exact claim seeds");
    repo.raise(
        ORG_A,
        RaiseScope {
            mapping,
            target: InventoryId::Tpt,
            at: T0,
        },
        &[(TOPIC, TermKind::Subject)],
    )
    .await
    .expect("the item raises");
    let open = repo.open_items(ORG_A).await.expect("open items load");
    let stolen = repo
        .resolve_with_edge(
            ORG_A,
            open[0].id,
            &nz_edge(TOPIC, &["Maths for early years"], "7000454"),
        )
        .await;
    assert!(
        matches!(stolen, Err(StorageError::Inconsistent { .. })),
        "many-to-one is illegal through Exact and rejected at insert"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn no_counterpart_settles_the_item_and_records_the_omission(app: PgPool) {
    seed_org_a(&app).await.expect("org-a seeds");
    let mapping = fixture_mapping(&app, ORG_A).await;
    let repo = TaxonomyRepo::new(app);
    repo.seed(&[subject_term()], &[])
        .await
        .expect("the term seeds");
    repo.raise(ORG_A, scope(mapping), &[(SUBJECT, TermKind::Subject)])
        .await
        .expect("the item raises");
    let open = repo.open_items(ORG_A).await.expect("open items load");
    repo.resolve_no_counterpart(
        ORG_A,
        open[0].id,
        &Decider::Human {
            user: tam_types::UserId(Uuid([0x05; 16])),
            org: ORG_A,
        },
        T0,
    )
    .await
    .expect("the no-counterpart resolution runs");
    let records = repo
        .no_counterparts_into(InventoryId::Tpt)
        .await
        .expect("the records load");
    assert_eq!(
        records,
        vec![(SUBJECT, VocabularyId(InventoryId::Tpt, TermKind::Subject))],
        "the omission is durable, so the term is omitted rather than blocking"
    );
    assert_eq!(
        repo.drain_stats(ORG_A).await.expect("stats load"),
        DrainStats {
            open: 0,
            resolved: 0,
            no_counterpart: 1
        },
        "the counters split no-counterpart from edge resolution"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_queue_is_tenant_isolated(app: PgPool) {
    seed_org_a(&app).await.expect("org-a seeds");
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes([0xBB; 16]))
        .execute(&app)
        .await
        .expect("org-b seeds");
    let mapping = fixture_mapping(&app, ORG_A).await;
    let repo = TaxonomyRepo::new(app);
    repo.seed(&[subject_term()], &[])
        .await
        .expect("the term seeds");
    repo.raise(ORG_A, scope(mapping), &[(SUBJECT, TermKind::Subject)])
        .await
        .expect("org-a raises");
    let org_b = OrgId(Uuid([0xBB; 16]));
    assert!(
        repo.open_items(org_b)
            .await
            .expect("org-b reads")
            .is_empty(),
        "one tenant's queue is invisible to another"
    );
    assert_eq!(
        repo.drain_stats(org_b).await.expect("org-b stats"),
        DrainStats::default(),
        "the counters are tenant-scoped"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_repolled_label_replaces_the_edge_it_renames_rather_than_shadowing_it(app: PgPool) {
    let repo = TaxonomyRepo::new(app);
    let first = gb_edge(SUBJECT, &["Maths for early years"], "1000454");
    repo.seed(&[subject_term()], &[first])
        .await
        .expect("the first seed runs");

    let renamed = gb_edge(SUBJECT, &["Maths for the early years"], "1000454");
    let second = repo
        .seed(&[subject_term()], &[renamed])
        .await
        .expect("the reseed runs");
    assert_eq!(
        (second.edges_inserted, second.edges_existing),
        (0, 1),
        "the renamed edge replaces its predecessor rather than joining it"
    );
    assert_eq!(
        second.ambiguous_terms, 0,
        "under DO NOTHING the renamed edge would be a second row and the term would \
         project ambiguously"
    );

    let edges = repo
        .edges_into(VocabularyId(InventoryId::Tes, TermKind::Subject))
        .await
        .expect("the GB subject edges load");
    assert_eq!(edges.len(), 1, "one edge, carrying the new label");
    assert_eq!(edges[0].to.segments, vec!["Maths for the early years"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_term_holds_several_narrower_edges_into_one_vocabulary(app: PgPool) {
    let repo = TaxonomyRepo::new(app);
    let narrower = |segments: &[&str], native: &str| ProjectionEdge {
        kind: EdgeKind::Narrower,
        ..edge(SUBJECT, InventoryId::Tpt, segments, native)
    };
    let report = repo
        .seed(
            &[subject_term()],
            &[narrower(&["Third"], "20"), narrower(&["Fourth"], "21")],
        )
        .await
        .expect("the seed runs");
    assert_eq!(
        report.edges_inserted, 2,
        "a broader source value names several narrower targets, which is what the seller \
         is asked to choose between"
    );
    assert_eq!(
        report.ambiguous_terms, 0,
        "narrower edges never participate in a projection, so they cannot make one ambiguous"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_seeder_records_a_measured_absence_in_bulk_and_idempotently(app: PgPool) {
    let repo = TaxonomyRepo::new(app);
    repo.seed(&[subject_term(), topic_term()], &[])
        .await
        .expect("the terms seed");
    let records = [
        NoCounterpart {
            term: SUBJECT,
            target: VocabularyId(InventoryId::Tpt, TermKind::Subject),
            decided_by: Decider::Imported {
                source: "test fixture".to_owned(),
            },
            decided_at: T0,
        },
        NoCounterpart {
            term: TOPIC,
            target: VocabularyId(InventoryId::Tpt, TermKind::Topic),
            decided_by: Decider::Imported {
                source: "test fixture".to_owned(),
            },
            decided_at: T0,
        },
    ];
    let first = repo
        .seed_no_counterparts(&records)
        .await
        .expect("the absences record");
    assert_eq!((first.inserted, first.existing), (2, 0));
    let second = repo
        .seed_no_counterparts(&records)
        .await
        .expect("the reseed runs");
    assert_eq!(
        (second.inserted, second.existing),
        (0, 2),
        "a global seed of the same absence is an explicit no-op"
    );

    let recorded = repo
        .no_counterparts_into(InventoryId::Tpt)
        .await
        .expect("the absences load");
    assert_eq!(
        recorded,
        vec![
            (SUBJECT, VocabularyId(InventoryId::Tpt, TermKind::Subject)),
            (TOPIC, VocabularyId(InventoryId::Tpt, TermKind::Topic)),
        ],
        "the projection reads these as omissions rather than raising a queue item per product"
    );
}
