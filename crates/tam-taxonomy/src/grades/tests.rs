//! The grade derivation's tests, kept beside it as a child module so the
//! derivation itself stays readable. `super::` still names `grades`, so
//! every private item these reach is reached the same way it was inline.

use super::{derive_grade_crosswalk, year_group_id, GradeCrosswalk};
use crate::project::{ingest_by_native_id, ingest_grades, project, project_axis, AxisRequest};
use tam_domain::equivalence::{ElectionTrigger, Loss, PricingBranch};
use tam_domain::registry::{registry, AxisBinding};
use tam_domain::{
    DeclarationSource, EdgeKind, GradeDeclaration, TermKind, TermProjection, VocabularyId,
    VocabularyPath,
};
use tam_types::natives::is_tpt_tag_slug;
use tam_types::{InventoryId, ProductId, Timestamp, Uuid};

const TPT: &str = include_str!("../../../../docs/design/data/tpt-vocabulary.json");
const TES: &str = include_str!("../../../../docs/design/data/tes-vocabulary.json");
const AT: Timestamp = Timestamp(1_700_000_000_000);
const PRODUCT: ProductId = ProductId(Uuid([0x0f; 16]));

fn crosswalk() -> GradeCrosswalk {
    derive_grade_crosswalk(TPT, TES, AT).expect("the committed captures derive")
}

fn edges_into(
    crosswalk: &GradeCrosswalk,
    inventory: InventoryId,
) -> Vec<&tam_domain::ProjectionEdge> {
    crosswalk
        .edges
        .iter()
        .filter(|edge| edge.to.vocabulary == VocabularyId(inventory, TermKind::Phase))
        .collect()
}

/// The two rows no band covers, named by the term each is minted as. US
/// Pre-K is TPT's Preschool under the TPT base, so its term carries TPT's
/// identity while the coverage failure it reports is still Tes's.
#[test]
fn no_band_is_invented_where_none_covers() {
    let crosswalk = crosswalk();
    for (year_group, term, why) in [
        (1_u64, year_group_id(1), "GB Nursery, ages 1-4"),
        (16, super::tpt_grade_term_id(1), "US Pre-K, ages 1-5"),
    ] {
        assert!(
            !crosswalk.edges.iter().any(|edge| edge.from == term
                && edge.to.vocabulary == VocabularyId(InventoryId::TesGb, TermKind::Phase)),
            "{why} covers no band and a nearest-band rule would file it under 3-5"
        );
        assert!(
            crosswalk
                .uncovered
                .iter()
                .any(|row| row.year_group == year_group),
            "{why} is reported rather than silently omitted"
        );
        assert!(
            crosswalk
                .no_counterparts
                .iter()
                .any(|record| record.term == term
                    && record.target == VocabularyId(InventoryId::TesGb, TermKind::Phase)),
            "{why} is a measured absence, so it omits rather than blocking every listing"
        );
    }
}

#[test]
fn the_not_applicable_sentinel_takes_one_exact_band_and_not_every_bounded_one() {
    let crosswalk = crosswalk();
    // TPT's Not Grade Specific, which the pairing table names for Tes's
    // not-applicable year group, so the sentinel is minted from the base.
    let term = super::tpt_grade_term_id(23);
    let gb: Vec<_> = crosswalk
        .edges
        .iter()
        .filter(|edge| {
            edge.from == term
                && edge.to.vocabulary == VocabularyId(InventoryId::TesGb, TermKind::Phase)
        })
        .collect();
    assert_eq!(
        gb.len(),
        1,
        "an empty humanAges set is vacuously inside every bounded band, which is the \
         branch a covering rule gets wrong"
    );
    assert_eq!(gb[0].kind, EdgeKind::Exact);
    assert_eq!(gb[0].to.native_id.as_deref(), Some("7"));
}

#[test]
fn the_gb_band_relation_is_twenty_seven_broader_edges_over_seven_band_members() {
    let crosswalk = crosswalk();
    let gb = edges_into(&crosswalk, InventoryId::TesGb);
    let broader = gb
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Broader)
        .count();
    let exact = gb
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Exact)
        .count();
    assert_eq!(
        (broader, exact),
        (27, 7),
        "fourteen coverings from the GB half and thirteen from the US half, over six band \
         members and the sentinel the two vocabularies share"
    );
}

#[test]
fn a_gb_age_band_enters_the_relation_as_a_term_of_its_own() {
    let crosswalk = crosswalk();
    for band in 1_u64..=6 {
        assert_eq!(
            ingest_by_native_id(
                &band.to_string(),
                VocabularyId(InventoryId::TesGb, TermKind::Phase),
                &crosswalk.edges,
            ),
            Some(super::age_range_id(band)),
            "a GB source path ingests, or every GB-sourced listing carries an \
             unrecognised grade"
        );
    }
    assert_eq!(
        ingest_by_native_id(
            "7",
            VocabularyId(InventoryId::TesGb, TermKind::Phase),
            &crosswalk.edges,
        ),
        Some(super::tpt_grade_term_id(23)),
        "the two not-applicable sentinels denote one thing and share one term, which under \
         the TPT base is TPT's Not Grade Specific"
    );
}

#[test]
fn a_band_names_every_year_group_it_covers_and_derives_none_of_them() {
    let crosswalk = crosswalk();
    let band = super::age_range_id(3);
    let candidates: Vec<&str> = crosswalk
        .edges
        .iter()
        .filter(|edge| {
            edge.from == band
                && edge.kind == EdgeKind::Narrower
                && edge.to.vocabulary == VocabularyId(InventoryId::TesUs, TermKind::Phase)
        })
        .filter_map(|edge| edge.to.native_id.as_deref())
        .collect();
    assert_eq!(
        candidates,
        vec!["5", "6", "7", "8", "19", "20", "21", "22"],
        "ages 7-11 covers four GB years and four US grades, and which of them a listing \
         carries is the seller's to say"
    );
    assert_eq!(
        project(
            band,
            VocabularyId(InventoryId::TesUs, TermKind::Phase),
            &crosswalk.edges,
        ),
        TermProjection::Absent,
        "the projection never derives across a narrower edge, so the candidates are a \
         question rather than an answer"
    );
}

#[test]
fn every_tpt_grade_is_paired_or_recorded_absent_and_never_both() {
    let crosswalk = crosswalk();
    let projecting: Vec<_> = edges_into(&crosswalk, InventoryId::Tpt)
        .into_iter()
        .filter(|edge| edge.kind != EdgeKind::Narrower)
        .collect();
    assert_eq!(
        projecting.len(),
        19,
        "nineteen TPT grade options, each claimed once"
    );
    let mut ids: Vec<&str> = projecting
        .iter()
        .filter_map(|edge| edge.to.native_id.as_deref())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 19, "no TPT id is the target of two edges");

    let absent: Vec<_> = crosswalk
        .no_counterparts
        .iter()
        .filter(|record| record.target == VocabularyId(InventoryId::Tpt, TermKind::Phase))
        .collect();
    assert_eq!(
        absent.len(),
        16,
        "the fifteen GB year groups have no TPT counterpart, and neither does the one \
         band that covers none"
    );
    for record in &absent {
        assert!(
            !crosswalk.edges.iter().any(|edge| edge.from == record.term
                && edge.to.vocabulary == VocabularyId(InventoryId::Tpt, TermKind::Phase)),
            "a term is paired or recorded absent, never both"
        );
    }
}

/// The band-to-TPT table, as counts and exact memberships. The ages in
/// each message are the capture's own: a band names a TPT grade when it
/// covers the year group the pairing table names for that grade, and
/// `a_named_tpt_grade_sits_inside_the_band_that_names_it` reads those ages
/// back out of the JSON so a wrong row here cannot agree with itself.
#[test]
fn a_band_names_the_tpt_grades_its_covered_year_groups_pair_to() {
    let crosswalk = crosswalk();
    let table = [
        (
            1_u64,
            vec![],
            "3-5 covers Reception alone, which TPT does not pair",
        ),
        (
            2,
            vec!["kindergarten", "1st-grade"],
            "5-7 covers Kindergarten 5-6 and 1st 6-7",
        ),
        (
            3,
            vec!["2nd-grade", "3rd-grade", "4th-grade", "5th-grade"],
            "7-11 covers 2nd through 5th, ages 7-11",
        ),
        (
            4,
            vec!["6th-grade", "7th-grade", "8th-grade"],
            "11-14 covers 6th through 8th, ages 11-14",
        ),
        (
            5,
            vec!["9th-grade", "10th-grade"],
            "14-16 covers 9th 14-15 and 10th 15-16",
        ),
        (
            6,
            vec!["11th-grade", "12th-grade"],
            "16+ covers 11th 16-17 and 12th 17-18",
        ),
    ];
    let mut named_total = 0_usize;
    for (band, expected, why) in table {
        let named = tpt_grades_named_by(&crosswalk, band);
        assert_eq!(named, expected, "{why}");
        named_total += named.len();
        assert_eq!(
            crosswalk
                .no_counterparts
                .iter()
                .filter(|record| record.term == super::age_range_id(band)
                    && record.target == VocabularyId(InventoryId::Tpt, TermKind::Phase))
                .count(),
            usize::from(expected.is_empty()),
            "a band is related or recorded absent, never neither and never both: {why}"
        );
    }
    assert_eq!(
        named_total, 13,
        "thirteen of the fifteen paired year groups fall inside a band; Pre-K and the \
         not-applicable sentinel are the two that do not"
    );
}

/// The membership half of the table, read back out of the capture: every
/// TPT grade a band names is a grade whose paired year group declares only
/// ages the band admits. Independent of the narrowest-covering choice,
/// which the literal table above pins instead.
#[test]
fn a_named_tpt_grade_sits_inside_the_band_that_names_it() {
    let crosswalk = crosswalk();
    let tes: super::TesVocabulary = serde_json::from_str(TES).expect("the capture parses");
    let captured = captured_grade_facets();
    let paired: std::collections::BTreeMap<String, u64> = super::GRADE_PAIRS
        .iter()
        .map(|&(tpt_id, year_group)| {
            let (slug, _) = captured
                .get(&tpt_id.to_string())
                .unwrap_or_else(|| panic!("the pairing names TPT grade {tpt_id}"));
            (slug.clone(), year_group)
        })
        .collect();
    let mut checked = 0_usize;
    for band in 1_u64..=6 {
        let row = tes
            .age_ranges
            .options
            .get(&band.to_string())
            .expect("the band is a captured row");
        let (low, high) = (
            row.age_low.expect("a bounded band declares its low age"),
            row.age_high.unwrap_or(u8::MAX),
        );
        for slug in tpt_grades_named_by(&crosswalk, band) {
            let year_group = paired[slug];
            let ages = &tes
                .year_groups
                .options
                .get(&year_group.to_string())
                .expect("the pairing names a captured year group")
                .human_ages;
            assert!(
                ages.iter().all(|&age| age >= low && age <= high),
                "TPT grade {slug} is year group {year_group}, ages {ages:?}, which the \
                 {} band does not admit",
                row.label
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 13,
        "every named grade is checked against the capture"
    );
}

fn tpt_grades_named_by(crosswalk: &GradeCrosswalk, band: u64) -> Vec<&str> {
    let term = super::age_range_id(band);
    crosswalk
        .edges
        .iter()
        .filter(|edge| {
            edge.from == term
                && edge.kind == EdgeKind::Narrower
                && edge.to.vocabulary == VocabularyId(InventoryId::Tpt, TermKind::Phase)
        })
        .filter_map(|edge| edge.to.native_id.as_deref())
        .collect()
}

#[test]
fn the_tpt_only_grades_are_absent_from_all_three_tes_inventories() {
    let crosswalk = crosswalk();
    for tpt_id in [15_u64, 16, 17, 19] {
        let term = super::tpt_grade_term_id(tpt_id);
        let targets: Vec<_> = crosswalk
            .no_counterparts
            .iter()
            .filter(|record| record.term == term)
            .map(|record| record.target.0)
            .collect();
        assert_eq!(
            targets,
            vec![InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz],
            "TPT grade {tpt_id} has no Tes counterpart anywhere"
        );
    }
}

#[test]
fn the_tpt_label_is_the_taxonomy_tag_and_not_the_form_label() {
    let crosswalk = crosswalk();
    let preschool = edges_into(&crosswalk, InventoryId::Tpt)
        .into_iter()
        .find(|edge| edge.to.native_id.as_deref() == Some("preschool"))
        .expect("TPT grade 1 is paired, and its path is keyed by its facet's slug");
    assert_eq!(
        preschool.to.segments,
        vec!["Preschool".to_owned()],
        "TPT's own label for grade 1 is Preschool; PreK is the create form's abbreviation"
    );
}

/// The `legacyId` to facet join read straight out of the capture, so the
/// pairing table's numeric ids and the slugs the edges carry can be held
/// against each other without going back through the derivation.
fn captured_grade_facets() -> std::collections::BTreeMap<String, (String, String)> {
    let capture: serde_json::Value = serde_json::from_str(TPT).expect("the capture parses");
    let options = capture["taxonomyTags"]["options"]
        .as_object()
        .expect("the capture holds a taxonomy tag map");
    let mut out = std::collections::BTreeMap::new();
    for (slug, tag) in options {
        let (Some(legacy), Some(category), Some(name)) = (
            tag["legacyId"].as_str(),
            tag["category"].as_str(),
            tag["name"].as_str(),
        ) else {
            continue;
        };
        if !super::GRADE_TAG_CATEGORIES.contains(&category) {
            continue;
        }
        assert!(
            out.insert(legacy.to_owned(), (slug.clone(), name.to_owned()))
                .is_none(),
            "legacyId {legacy} names two grade facets, so any slug join would be a guess"
        );
    }
    out
}

/// TPT addresses a grade twice over and by different identifiers: the
/// create form's selector takes the `legacyId`, and the product's own wire
/// takes the `taxonomyTags` slug. The path is the wire's, so every edge
/// into `(Tpt, Phase)` carries the slug, and the slug and the label are
/// read off one facet rather than joined from two.
#[test]
fn a_tpt_grade_path_is_addressed_by_its_taxonomy_tag_slug_and_never_its_legacy_id() {
    let crosswalk = crosswalk();
    let captured = captured_grade_facets();
    let edges = edges_into(&crosswalk, InventoryId::Tpt);
    assert_eq!(
        edges.len(),
        32,
        "nineteen grade options claimed once each, and the thirteen band coverings"
    );
    for edge in &edges {
        let native = edge
            .to
            .native_id
            .as_deref()
            .expect("a TPT grade path is addressed or it cannot be posted");
        let (legacy, (_, name)) = captured
            .iter()
            .find(|(_, (slug, _))| slug == native)
            .unwrap_or_else(|| {
                panic!(
                    "{native:?} is no slug the capture keys a grade facet by, so TPT's tag                          namespace does not answer to it"
                )
            });
        assert_eq!(
            &edge.to.segments,
            &vec![name.clone()],
            "legacyId {legacy}'s slug and label are one facet's, not two joins'"
        );
        assert!(
            is_tpt_tag_slug(native),
            "{native:?} fails the adapter's slug-shape guard and would be refused rather                  than posted"
        );
    }
}

/// The arithmetic half: the nineteen `gradeLevels` options are exactly the
/// nineteen facets the projecting edges address, so no option is left
/// carrying a form-facing identifier.
#[test]
fn every_gradelevels_option_projects_under_the_slug_its_legacy_id_joins_to() {
    let crosswalk = crosswalk();
    let captured = captured_grade_facets();
    let tpt: super::TptVocabulary = serde_json::from_str(TPT).expect("the capture parses");
    let mut expected: Vec<&str> = tpt
        .grade_levels
        .options
        .keys()
        .map(|legacy| {
            let Some((slug, _)) = captured.get(legacy) else {
                panic!("gradeLevels {legacy} joins no grade facet");
            };
            slug.as_str()
        })
        .collect();
    let mut addressed: Vec<&str> = edges_into(&crosswalk, InventoryId::Tpt)
        .into_iter()
        .filter(|edge| edge.kind != EdgeKind::Narrower)
        .filter_map(|edge| edge.to.native_id.as_deref())
        .collect();
    expected.sort_unstable();
    addressed.sort_unstable();
    assert_eq!(
        expected.len(),
        19,
        "the capture holds nineteen grade options"
    );
    assert_eq!(
        addressed, expected,
        "each option is claimed once, under the slug and not the number"
    );
}

#[test]
fn the_seeded_grade_relation_holds_no_ambiguity() {
    let crosswalk = crosswalk();
    for term in &crosswalk.terms {
        for inventory in InventoryId::ALL {
            let projection = project(
                term.id,
                VocabularyId(inventory, TermKind::Phase),
                &crosswalk.edges,
            );
            assert!(
                !matches!(projection, TermProjection::Ambiguous { .. }),
                "{} projects ambiguously into {inventory:?}",
                term.label
            );
        }
    }
}

#[test]
fn one_term_holds_at_most_one_projecting_edge_into_each_vocabulary() {
    let crosswalk = crosswalk();
    let mut keys: Vec<_> = crosswalk
        .edges
        .iter()
        .filter(|edge| edge.kind != EdgeKind::Narrower)
        .map(|edge| (edge.from.0 .0, edge.to.vocabulary.0, edge.kind))
        .collect();
    let total = keys.len();
    keys.sort_unstable_by_key(|&(from, inventory, kind)| {
        (from, format!("{inventory:?}"), format!("{kind:?}"))
    });
    keys.dedup();
    assert_eq!(
        keys.len(),
        total,
        "the seeder's output satisfies projection_edge_single_valued"
    );
    assert!(
        crosswalk
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Narrower),
        "narrower edges are excluded from that index precisely because the band relation \
         needs several of them from one term"
    );
}

fn declaration(inventory: InventoryId, native: &str) -> GradeDeclaration {
    GradeDeclaration {
        source: DeclarationSource::Imported {
            vocabulary: VocabularyId(inventory, TermKind::Phase),
        },
        raw: vec![VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Phase),
            segments: vec!["whatever the seller saw".to_owned()],
            native_id: Some(native.to_owned()),
        }],
        derived: None,
    }
}

fn phase_axis(inventory: InventoryId) -> AxisBinding {
    registry(inventory)
        .axis(TermKind::Phase)
        .expect("every inventory under test binds a phase axis")
}

#[test]
fn a_us_year_group_broadens_onto_a_gb_band_and_names_what_it_dropped() {
    let crosswalk = crosswalk();
    let declared = declaration(InventoryId::TesUs, "21");
    let ingested = ingest_grades(&declared, &crosswalk.edges);
    assert_eq!(
        ingested.terms,
        vec![super::tpt_grade_term_id(6)],
        "a US grade ingests by its own yearGroups id and lands on the term TPT minted, \
         because year group 21 and TPT grade 6 are one phase and the base names it"
    );

    let outcome = project_axis(
        AxisRequest {
            product: PRODUCT,
            inventory: InventoryId::TesGb,
            binding: phase_axis(InventoryId::TesGb),
            terms: &ingested.terms,
            sources: &ingested.sources,
            pricing: PricingBranch::Free,
            rules: &[],
            settled: &[],
        },
        &crosswalk.edges,
        &[],
    );
    assert_eq!(
        outcome
            .resolved
            .iter()
            .filter_map(|path| path.native_id.as_deref())
            .collect::<Vec<_>>(),
        vec!["3"],
        "US 4th grade, ages 9-10, publishes into the GB 7-11 band and not under its own id"
    );
    assert!(
        outcome.elections.is_empty(),
        "the lossy direction is derived"
    );
    assert!(
        matches!(outcome.loss.as_slice(), [Loss::Broadened { .. }]),
        "the widening is disclosed rather than hidden"
    );
}

#[test]
fn a_gb_band_asks_which_year_groups_it_means_rather_than_picking_one() {
    let crosswalk = crosswalk();
    let declared = declaration(InventoryId::TesGb, "3");
    let ingested = ingest_grades(&declared, &crosswalk.edges);
    assert_eq!(ingested.terms, vec![super::age_range_id(3)]);

    let outcome = project_axis(
        AxisRequest {
            product: PRODUCT,
            inventory: InventoryId::TesUs,
            binding: phase_axis(InventoryId::TesUs),
            terms: &ingested.terms,
            sources: &ingested.sources,
            pricing: PricingBranch::Free,
            rules: &[],
            settled: &[],
        },
        &crosswalk.edges,
        &[],
    );
    assert!(
        outcome.gaps.is_empty(),
        "a band is not a missing equivalence; the relation holds every candidate already"
    );
    let [election] = outcome.elections.as_slice() else {
        panic!("one election, naming the choice the band leaves open");
    };
    let ElectionTrigger::Narrow { from, candidates } = &election.trigger else {
        panic!("a band covering several year groups is a Narrow trigger");
    };
    assert_eq!(from.native_id.as_deref(), Some("3"));
    assert_eq!(
        candidates
            .iter()
            .filter_map(|path| path.native_id.as_deref())
            .collect::<Vec<_>>(),
        vec!["5", "6", "7", "8", "19", "20", "21", "22"]
    );
    assert!(
        outcome.resolved.is_empty(),
        "nothing publishes until the seller answers, so no year group is picked for them"
    );
    assert_eq!(
        election.trigger.key().as_deref(),
        Some("3"),
        "the answer keys on the band, so five hundred GB listings ask once"
    );
}

fn recorded_absences(
    crosswalk: &GradeCrosswalk,
) -> Vec<(tam_types::CanonicalTermId, VocabularyId)> {
    crosswalk
        .no_counterparts
        .iter()
        .map(|record| (record.term, record.target))
        .collect()
}

fn into_tpt(
    crosswalk: &GradeCrosswalk,
    inventory: InventoryId,
    native: &str,
) -> tam_domain::equivalence::AxisOutcome {
    let declared = declaration(inventory, native);
    let ingested = ingest_grades(&declared, &crosswalk.edges);
    project_axis(
        AxisRequest {
            product: PRODUCT,
            inventory: InventoryId::Tpt,
            binding: phase_axis(InventoryId::Tpt),
            terms: &ingested.terms,
            sources: &ingested.sources,
            pricing: PricingBranch::Free,
            rules: &[],
            settled: &[],
        },
        &crosswalk.edges,
        &recorded_absences(crosswalk),
    )
}

#[test]
fn a_gb_band_cross_lists_to_tpt_as_the_same_question_it_asks_the_tes_us_side() {
    let crosswalk = crosswalk();
    let outcome = into_tpt(&crosswalk, InventoryId::TesGb, "3");
    assert!(
        outcome.gaps.is_empty(),
        "a GB band reaching TPT is a choice the seller makes, not an equivalence \
         nobody authored"
    );
    let [election] = outcome.elections.as_slice() else {
        panic!("one election, naming the TPT grades the 7-11 band leaves open");
    };
    let ElectionTrigger::Narrow { from, candidates } = &election.trigger else {
        panic!("a band covering several TPT grades is a Narrow trigger");
    };
    assert_eq!(from.native_id.as_deref(), Some("3"));
    assert_eq!(
        candidates
            .iter()
            .filter_map(|path| path.native_id.as_deref())
            .collect::<Vec<_>>(),
        vec!["2nd-grade", "3rd-grade", "4th-grade", "5th-grade"],
        "2nd through 5th grade, which is what ages 7-11 means to TPT"
    );
    assert!(
        outcome.resolved.is_empty(),
        "nothing publishes until the seller answers"
    );
}

#[test]
fn the_band_no_tpt_grade_sits_inside_omits_rather_than_blocking_the_cross_list() {
    let crosswalk = crosswalk();
    let outcome = into_tpt(&crosswalk, InventoryId::TesGb, "1");
    assert!(
        outcome.gaps.is_empty() && outcome.elections.is_empty(),
        "the 3-5 band's absence from TPT is measured, so it proceeds rather than parking \
         every GB early-years listing on a question with no candidates"
    );
    assert_eq!(outcome.omitted, vec![super::age_range_id(1)]);
    assert!(outcome.is_publishable());
}

/// The seam a live cross-list crosses. `tam-marketplace-tpt` refuses a
/// grade whose native is not slug-shaped, because a foreign marketplace's
/// own identifier posted into `taxonomyTags` writes that number into the
/// seller's listing verbatim. The identifier is chosen here, so the shape
/// is asserted here, on both the direction that resolves and the one that
/// asks.
#[test]
fn a_tes_grade_reaching_tpt_arrives_under_a_slug_the_adapter_will_post() {
    let crosswalk = crosswalk();
    let resolving = into_tpt(&crosswalk, InventoryId::TesUs, "23");
    let resolved: Vec<&str> = resolving
        .resolved
        .iter()
        .filter_map(|path| path.native_id.as_deref())
        .collect();
    assert_eq!(
        resolved,
        vec!["6th-grade"],
        "US 6th grade is year group 23, which the pairing table names TPT grade 8, whose \
         facet the capture keys `6th-grade`"
    );

    let electing = into_tpt(&crosswalk, InventoryId::TesGb, "4");
    let [election] = electing.elections.as_slice() else {
        panic!("the 11-14 band covers three TPT grades and asks which");
    };
    let ElectionTrigger::Narrow { candidates, .. } = &election.trigger else {
        panic!("a band covering several TPT grades is a Narrow trigger");
    };
    let offered: Vec<&str> = candidates
        .iter()
        .filter_map(|path| path.native_id.as_deref())
        .collect();
    assert_eq!(offered, vec!["6th-grade", "7th-grade", "8th-grade"]);

    for native in resolved.into_iter().chain(offered) {
        assert!(
            is_tpt_tag_slug(native),
            "{native:?} fails the adapter's slug-shape guard, so answering the election \
             with it would refuse the upload rather than publish it"
        );
    }
}

#[test]
fn a_path_the_relation_does_not_recognise_is_carried_out_rather_than_dropped() {
    let crosswalk = crosswalk();
    let declared = declaration(InventoryId::TesGb, "4242");
    let ingested = ingest_grades(&declared, &crosswalk.edges);
    assert!(ingested.terms.is_empty());
    assert_eq!(
        ingested.unrecognised.len(),
        1,
        "reconciliation_item.term references canonical_term, so an unrecognised path \
         cannot become a queue item and must travel as itself"
    );
}

#[test]
fn the_derivation_is_deterministic() {
    assert_eq!(crosswalk(), crosswalk());
}
