//! The resource-type derivation's tests, over the committed captures and the
//! authored pairing, with no database.

use std::collections::BTreeSet;

use super::{derive_resource_type_crosswalk, resource_term_id, ResourceTypeCrosswalk};
use crate::project::{project_axis, AxisRequest};
use tam_domain::equivalence::{ElectionTrigger, Loss, PricingBranch};
use tam_domain::registry::{registry, AxisBinding};
use tam_domain::{EdgeKind, TermKind, VocabularyId};
use tam_types::{InventoryId, ProductId, Timestamp, Uuid};

const TPT: &str = include_str!("../../../../docs/design/data/tpt-vocabulary.json");
const TES: &str = include_str!("../../../../docs/design/data/tes-vocabulary.json");
const PAIRS: &str = include_str!("../../../../docs/design/data/tpt-tes-resource-type-pairs.json");
const AT: Timestamp = Timestamp(1_787_000_000_000);
const PRODUCT: ProductId = ProductId(Uuid([0x2c; 16]));

fn crosswalk() -> ResourceTypeCrosswalk {
    derive_resource_type_crosswalk(TPT, TES, PAIRS, AT).expect("the committed captures derive")
}

fn resource_axis(inventory: InventoryId) -> AxisBinding {
    *registry(inventory)
        .equivalence_axes
        .iter()
        .find(|binding| binding.axis == TermKind::ResourceType)
        .expect("Tes binds the resource-type axis")
}

#[test]
fn the_derivation_seeds_the_pinned_counts() {
    let derived = crosswalk();
    assert_eq!(
        derived.terms.len(),
        70,
        "71 Type-of-Resource facets less the one hidden root"
    );
    assert_eq!(
        derived.edges.len(),
        280,
        "one Exact edge into TPT for each of the 70, and a Broader edge into each of the \
         three Tes inventories"
    );
    assert_eq!(derived.skipped_hidden.len(), 1, "`independent-work`");
    assert!(
        derived.unreached.is_empty(),
        "every writable facet reaches a Tes value, by its own row or by inheriting its root's"
    );
    assert_eq!(
        derived.unclaimed_targets.len(),
        1,
        "Tes carries `Assembly` and TPT has no facet for it"
    );
    assert_eq!(derived.unclaimed_targets[0].native_id, "99001");
}

/// Seventy-one onto nine can only be many-to-one, and many-to-one is legal
/// through `Broader` and illegal through `Exact`. An `Exact` edge into Tes
/// here would be rejected at insert by `projection_edge_exact_reverse` the
/// moment a second facet claimed the same value.
#[test]
fn every_facet_reaches_tes_through_broader_alone() {
    let derived = crosswalk();
    for edge in &derived.edges {
        if edge.to.vocabulary.0 == InventoryId::Tpt {
            assert_eq!(edge.kind, EdgeKind::Exact, "a facet claims its own path");
        } else {
            assert_eq!(
                edge.kind,
                EdgeKind::Broader,
                "an Exact edge into Tes would claim the value exclusively: {:?}",
                edge.to.segments
            );
        }
    }
}

#[test]
fn every_facet_claims_its_own_tpt_path_exactly_once() {
    let derived = crosswalk();
    let mut claimed: BTreeSet<Vec<String>> = BTreeSet::new();
    for edge in &derived.edges {
        if edge.to.vocabulary.0 != InventoryId::Tpt {
            continue;
        }
        assert!(
            claimed.insert(edge.to.segments.clone()),
            "two facets name the TPT path {:?}",
            edge.to.segments
        );
    }
    assert_eq!(claimed.len(), 70);
}

/// The inheritance rule, which is what keeps the authored file at twenty-one
/// rows. `posters` is named nowhere and takes `Visual aid/Display` from
/// `classroom-decor`; `games` is named, and takes `Game/puzzle/quiz` rather
/// than the `Worksheet/Activity` it would have inherited.
#[test]
fn a_child_inherits_its_roots_value_unless_it_is_named() {
    let derived = crosswalk();
    let value_of = |slug: &str| -> Option<String> {
        let term = resource_term_id(slug);
        derived
            .edges
            .iter()
            .find(|edge| {
                edge.from == term
                    && edge.to.vocabulary
                        == VocabularyId(InventoryId::TesGb, TermKind::ResourceType)
            })
            .and_then(|edge| edge.to.native_id.clone())
    };
    assert_eq!(value_of("posters").as_deref(), Some("99008"));
    assert_eq!(value_of("classroom-decor").as_deref(), Some("99008"));
    assert_eq!(value_of("games").as_deref(), Some("99003"));
    assert_eq!(
        value_of("hands-on-activities").as_deref(),
        Some("99009"),
        "the override moves the child alone and leaves its root where it was"
    );
}

#[test]
fn the_hidden_facet_seeds_neither_a_term_nor_an_edge() {
    let derived = crosswalk();
    let hidden = resource_term_id("independent-work");
    assert!(!derived.terms.iter().any(|term| term.id == hidden));
    assert!(!derived.edges.iter().any(|edge| edge.from == hidden));
    assert_eq!(derived.skipped_hidden[0].native_id, "independent-work");
}

/// The step-3 verification. Tes takes `mainType` at `Cardinality::One`, so a
/// TPT product carrying four resource-type facets that resolve to four
/// different Tes values raises one election carrying all four, and publishes
/// nothing: a cap overflow is never a truncation, so no partially-narrowed
/// set exists for a caller to publish by accident.
#[test]
fn four_facets_into_a_single_valued_axis_elect_rather_than_truncate() {
    let derived = crosswalk();
    let terms = [
        resource_term_id("games"),
        resource_term_id("songs"),
        resource_term_id("posters"),
        resource_term_id("rubrics"),
    ];
    let outcome = project_axis(
        AxisRequest {
            product: PRODUCT,
            inventory: InventoryId::TesGb,
            binding: resource_axis(InventoryId::TesGb),
            terms: &terms,
            sources: &[],
            pricing: PricingBranch::Paid,
            rules: &[],
            settled: &[],
        },
        &derived.edges,
        &[],
    );
    assert!(
        outcome.resolved.is_empty(),
        "the resolved set goes into the election whole, so nothing narrowed is publishable"
    );
    assert_eq!(outcome.elections.len(), 1, "one election, not one per term");
    let ElectionTrigger::ElectOne { from } = &outcome.elections[0].trigger else {
        panic!(
            "a single-valued axis over a resolved set of four elects one: {:?}",
            outcome.elections[0].trigger
        );
    };
    let candidates: BTreeSet<&str> = from
        .iter()
        .filter_map(|path| path.native_id.as_deref())
        .collect();
    assert_eq!(
        candidates,
        BTreeSet::from(["99002", "99003", "99004", "99008"]),
        "all four resolved values are candidates"
    );
    assert!(
        outcome.gaps.is_empty(),
        "a broadening is a disclosed loss rather than a gap"
    );
}

/// Four facets that broaden onto one Tes value resolve to one, so the axis
/// publishes without asking. The election above is a property of the resolved
/// set rather than of the number of terms carried, and this is the case that
/// tells the two apart.
#[test]
fn four_facets_onto_one_value_resolve_without_an_election() {
    let derived = crosswalk();
    let terms = [
        resource_term_id("homework"),
        resource_term_id("worksheets"),
        resource_term_id("task-cards"),
        resource_term_id("workbooks"),
    ];
    let outcome = project_axis(
        AxisRequest {
            product: PRODUCT,
            inventory: InventoryId::TesGb,
            binding: resource_axis(InventoryId::TesGb),
            terms: &terms,
            sources: &[],
            pricing: PricingBranch::Paid,
            rules: &[],
            settled: &[],
        },
        &derived.edges,
        &[],
    );
    assert_eq!(outcome.resolved.len(), 1);
    assert_eq!(outcome.resolved[0].native_id.as_deref(), Some("99009"));
    assert!(outcome.elections.is_empty());
    let [Loss::Broadened { to, dropped }] = &outcome.loss[..] else {
        panic!(
            "one broadening onto one path, naming what it dropped: {:?}",
            outcome.loss
        );
    };
    assert_eq!(to.native_id.as_deref(), Some("99009"));
    assert_eq!(
        dropped.len(),
        4,
        "the loss merges per target path and names every term it broadened away"
    );
}

/// Whether routing this axis can write a resource type into TPT that TPT did
/// not itself supply, which is what D13 forbids.
///
/// It cannot, and the reason is the shape of the relation rather than a
/// declaration. Inbound resolution follows `Exact` edges alone
/// (`crates/tam-taxonomy/src/project.rs:377-395`), and no canonical term
/// claims a Tes `mainType` path as `Exact` — every Tes-side edge here is
/// `Broader`. So a Tes-sourced product ingests no resource-type term, carries
/// none, and writes none into TPT; only a TPT-sourced product carries them,
/// and those are the slugs TPT issued.
#[test]
fn no_tes_resource_type_value_ingests_to_a_canonical_term() {
    use crate::project::ingest;
    let derived = crosswalk();
    for edge in &derived.edges {
        if edge.to.vocabulary.0 == InventoryId::Tpt {
            assert_eq!(
                ingest(&edge.to, &derived.edges),
                Some(edge.from),
                "a TPT facet ingests to its own term: {:?}",
                edge.to.segments
            );
        } else {
            assert_eq!(
                ingest(&edge.to, &derived.edges),
                None,
                "a Tes value must ingest to nothing, or a Tes source could carry a \
                 resource type into a TPT create: {:?}",
                edge.to.segments
            );
        }
    }
}
