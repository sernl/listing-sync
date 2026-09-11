//! The hub end to end over the real captures: the measured crosswalk seeds
//! the database, a subject projects GB-to-NZ through repo-loaded edges as
//! Exact, a residue topic projects Absent and raises exactly one item, and
//! resolution drains the queue so the next product finds the edge waiting.

#![cfg(feature = "pg-tests")]

mod common;

use sqlx::PgPool;
use tam_domain::{Decider, EdgeKind, ProjectionEdge, TermKind, TermProjection, VocabularyId};
use tam_storage::{DrainStats, MappingRepo, ProductRepo, RaiseScope, TaxonomyRepo};
use tam_types::{InventoryId, MappingId, PriceIntent, PriceRule, Timestamp, Uuid};

use common::{minimal_product, seed_org_a, ORG_A};

const T0: Timestamp = Timestamp(1_000);
const MAPPING_1: MappingId = MappingId(Uuid([0x31; 16]));

const TES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/data/tes-taxonomy-GB.json"
));

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn fixture_mapping(app: &PgPool) -> MappingId {
    ProductRepo::new(app.clone())
        .insert(ORG_A, &minimal_product(), T0)
        .await
        .expect("the fixture product inserts");
    MappingRepo::new(app.clone())
        .insert(
            ORG_A,
            &tam_domain::Mapping {
                id: MAPPING_1,
                org: ORG_A,
                product: minimal_product().id,
                inventory: InventoryId::Tes,
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
            T0,
        )
        .await
        .expect("the fixture mapping inserts");
    MAPPING_1
}

#[sqlx::test(migrations = "./migrations")]
async fn the_measured_crosswalk_seeds_projects_and_drains(app: PgPool) {
    let tree = tam_taxonomy::tes::parse_tree(TES_JSON).expect("the Tes capture parses");
    let crosswalk = tam_taxonomy::tes::derive_crosswalk(&tree, T0);

    let repo = TaxonomyRepo::new(app.clone());
    let report = repo
        .seed(&crosswalk.terms, &crosswalk.edges)
        .await
        .expect("the crosswalk seeds");
    assert_eq!(
        (report.terms_inserted, report.terms_existing),
        (496, 0),
        "every captured node — 43 subjects and 453 topics — seeds one canonical term"
    );
    assert_eq!(
        (report.edges_inserted, report.edges_existing),
        (494, 0),
        "one edge per term, minus the two withdrawn from the contested duplicate path"
    );

    // A seeded subject projects Exact through repo-loaded edges.
    let subjects = VocabularyId(InventoryId::Tes, TermKind::Subject);
    let edges = repo
        .edges_into(subjects)
        .await
        .expect("the subject edges load");
    let maths = crosswalk
        .terms
        .iter()
        .find(|term| term.label == "Maths for early years" && term.kind == TermKind::Subject)
        .expect("the probe's worked example is in the capture");
    let projected = tam_taxonomy::project::project(maths.id, subjects, &edges);
    let TermProjection::Exact { to } = projected else {
        panic!("the measured node must project Exact, got {projected:?}");
    };
    assert_eq!(
        to.native_id.as_deref(),
        Some("1000454"),
        "the path carries the node's own native id"
    );

    // A withdrawn topic — two nodes contesting one path — projects Absent,
    // raises once, dedups, and resolution drains it.
    let withdrawn = crosswalk
        .residue
        .mismatched
        .first()
        .expect("the capture carries two Whole school topics under one description");
    // Found by the deterministic id, not the label: the uniform tes:{id} key
    // is what separates two nodes that share a description.
    let residue_id = tam_types::CanonicalTermId(tam_types::Uuid(
        *uuid::Uuid::new_v5(
            &uuid::Uuid::from_bytes(tam_types::NAMESPACE_TAM_TAXONOMY.0),
            format!("tes:{}", withdrawn.node.native_id).as_bytes(),
        )
        .as_bytes(),
    ));
    let residue_term = crosswalk
        .terms
        .iter()
        .find(|term| term.id == residue_id)
        .expect("a withdrawn node still seeds a canonical term under the uniform key");
    let topics = VocabularyId(InventoryId::Tes, TermKind::Topic);
    let topic_edges = repo.edges_into(topics).await.expect("the topic edges load");
    assert_eq!(
        tam_taxonomy::project::project(residue_term.id, topics, &topic_edges),
        TermProjection::Absent,
        "a withdrawn term holds no edge and is a first-class Absent"
    );

    seed_org_a(&app).await.expect("org-a seeds");
    let mapping = fixture_mapping(&app).await;
    let scope = RaiseScope {
        mapping,
        target: InventoryId::Tes,
        at: T0,
    };
    let causes = [(residue_term.id, TermKind::Topic)];
    let first = repo.raise(ORG_A, scope, &causes).await.expect("raise runs");
    let second = repo
        .raise(ORG_A, scope, &causes)
        .await
        .expect("re-raise runs");
    assert_eq!(
        (first.new, second.already_open),
        (1, 1),
        "one gap, one item, however many products carry it"
    );

    let open = repo.open_items(ORG_A).await.expect("open items load");
    repo.resolve_with_edge(
        ORG_A,
        open[0].id,
        &ProjectionEdge {
            from: residue_term.id,
            to: tam_domain::VocabularyPath {
                vocabulary: topics,
                segments: vec![
                    "Mathematics".to_owned(),
                    "A founder-authored twin".to_owned(),
                ],
                native_id: None,
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Human {
                user: tam_types::UserId(Uuid([0x05; 16])),
                org: ORG_A,
            },
            decided_at: T0,
        },
    )
    .await
    .expect("the founder resolves the gap once");

    let drained = repo
        .edges_into(topics)
        .await
        .expect("the NZ topic edges reload");
    assert_eq!(
        tam_taxonomy::project::project(residue_term.id, topics, &drained),
        TermProjection::Exact {
            to: tam_domain::VocabularyPath {
                vocabulary: topics,
                segments: vec![
                    "Mathematics".to_owned(),
                    "A founder-authored twin".to_owned()
                ],
                native_id: None,
            }
        },
        "the second product carrying the term finds the edge waiting"
    );
    assert_eq!(
        repo.drain_stats(ORG_A).await.expect("stats load"),
        DrainStats {
            open: 0,
            resolved: 1,
            no_counterpart: 0
        },
        "the drain counters the kill gate reads reflect the run"
    );
}

const TPT_VOCABULARY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/data/tpt-vocabulary.json"
));
const TES_VOCABULARY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/design/data/tes-vocabulary.json"
));

#[sqlx::test(migrations = "./migrations")]
async fn the_whole_grade_relation_survives_every_index_and_reseeds_as_a_no_op(app: PgPool) {
    let crosswalk =
        tam_taxonomy::grades::derive_grade_crosswalk(TPT_VOCABULARY, TES_VOCABULARY, T0)
            .expect("the grade crosswalk derives");
    let repo = TaxonomyRepo::new(app);

    let report = repo
        .seed(&crosswalk.terms, &crosswalk.edges)
        .await
        .expect("the grade relation seeds");
    assert_eq!(
        (report.terms_inserted, report.edges_inserted),
        (
            u64::try_from(crosswalk.terms.len()).expect("the term count fits"),
            u64::try_from(crosswalk.edges.len()).expect("the edge count fits")
        ),
        "the reverse-Exact uniqueness index and the single-valued index both admit the \
         derivation, which no pure test can establish"
    );
    assert_eq!(
        report.ambiguous_terms, 0,
        "no seeded term projects two ways"
    );

    let absences = repo
        .seed_no_counterparts(&crosswalk.no_counterparts)
        .await
        .expect("the measured absences record");
    assert_eq!(
        absences.inserted,
        u64::try_from(crosswalk.no_counterparts.len()).expect("the absence count fits")
    );

    let again = repo
        .seed(&crosswalk.terms, &crosswalk.edges)
        .await
        .expect("the reseed runs");
    assert_eq!(
        (again.terms_inserted, again.edges_inserted),
        (0, 0),
        "deterministic ids make a re-run an explicit no-op even under an upsert"
    );

    let gb = repo
        .edges_into(VocabularyId(InventoryId::Tes, TermKind::Phase))
        .await
        .expect("the GB phase edges load");
    assert_eq!(
        gb.iter()
            .filter(|edge| edge.kind == EdgeKind::Broader)
            .count(),
        27,
        "the covering relation reads back whole"
    );
    let tpt = repo
        .edges_into(VocabularyId(InventoryId::Tpt, TermKind::Phase))
        .await
        .expect("the TPT phase edges load");
    assert_eq!(
        tpt.iter()
            .filter(|edge| edge.kind == EdgeKind::Narrower)
            .count(),
        13,
        "the band relation reaches TPT through the same index, so a GB band cross-lists as a \
         question the seller can answer rather than a gap nobody can"
    );
}
