//! The grade axis, derived from the two polled vocabularies rather than
//! transcribed.
//!
//! The canonical grade vocabulary is minted from Tes `yearGroups`, the richest
//! set on file: thirty rows spanning both countries, every row carrying the
//! ages it covers. TPT's nineteen grade options pair against it through one
//! authored fifteen-row table, and everything else — labels, ages, band
//! coverage — is read out of the JSONs, so a re-poll cannot leave a
//! transcription behind.
//!
//! The GB fork is the interesting half. The Tes editor takes `ageRanges` when
//! the country is GB and `yearGroups` otherwise, so `VocabularyId(TesGb,
//! Phase)` denotes the seven age bands while `VocabularyId(TesUs, Phase)`
//! denotes the thirty year groups. A year group reaches a band by *covering*:
//! the narrowest band that admits the lowest declared age and does not end
//! before the highest. Where no band covers, the derivation emits no edge and
//! a no-counterpart record, because a nearest-band rule would silently file
//! ages 1-4 under the 3-5 band and pass every count-based check.
//!
//! TPT's grade labels are not in `gradeLevels`, which holds a bare id-to-form
//! -label map; the seller-facing labels live in `taxonomyTags` behind a
//! `legacyId` join. That join is only unique within a category — `legacyId`
//! 1 names both the `Grade-Level` facet `preschool` and the `Price-Range`
//! facet `free` — so it is scoped to the two categories the grade selector
//! draws from, and an ambiguous or missing join is refused rather than
//! guessed.

use std::collections::BTreeMap;

use serde::Deserialize;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, NoCounterpart, ProjectionEdge, TermKind, VocabularyId,
    VocabularyPath,
};
use tam_types::{CanonicalTermId, InventoryId, Timestamp, NAMESPACE_TAM_TAXONOMY};

use crate::tes::AgeBounds;

/// The authored half of the derivation, and the only authored half: which Tes
/// `yearGroups` row each TPT grade option denotes.
///
/// Fifteen rows against nineteen TPT options. The four with no counterpart —
/// Higher Education (15), Adult Education (16), Homeschool (17) and Staff
/// (19) — are absent here deliberately and become no-counterpart records
/// against all three Tes inventories; Homeschool and Staff are `audience`
/// facets TPT's grade selector happens to offer, which is a routing hint for a
/// later axis and not a reason to force a grade edge.
const GRADE_PAIRS: [(u64, u64); 15] = [
    (1, 16),
    (2, 17),
    (3, 18),
    (4, 19),
    (5, 20),
    (6, 21),
    (7, 22),
    (8, 23),
    (9, 24),
    (10, 25),
    (11, 26),
    (12, 27),
    (13, 28),
    (14, 29),
    (23, 30),
];

/// The categories TPT's grade selector draws its options from. `legacyId` is
/// unique within a category and not across them, so the label join is scoped
/// to these two.
const GRADE_TAG_CATEGORIES: [&str; 2] = ["Grade-Level", "audience"];

/// The three Tes inventories a TPT-only grade has no counterpart in.
const TES_INVENTORIES: [InventoryId; 3] =
    [InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz];

#[derive(Debug, Clone, Deserialize)]
pub struct OptionSet<T> {
    pub options: BTreeMap<String, T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TesVocabulary {
    #[serde(rename = "_source")]
    pub source: String,
    pub year_groups: OptionSet<YearGroup>,
    pub age_ranges: OptionSet<AgeRange>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YearGroup {
    pub group: String,
    pub country: String,
    pub human_ages: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeRange {
    pub label: String,
    pub age_low: Option<u8>,
    pub age_high: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TptVocabulary {
    #[serde(rename = "_source")]
    pub source: String,
    pub grade_levels: OptionSet<GradeLevel>,
    pub taxonomy_tags: OptionSet<TaxonomyTag>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeLevel {
    pub form_label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonomyTag {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub legacy_id: Option<String>,
}

/// A year group whose declared ages no Tes GB band admits. Reported rather
/// than snapped to the nearest band, and carried into the seeder's report so
/// the omission is a visible figure rather than a silent absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uncovered {
    pub year_group: u64,
    pub label: String,
    pub human_ages: Vec<u8>,
}

/// The grade axis as the two captures state it: the canonical terms, their
/// edges into all four target vocabularies, the measured absences, and the
/// rows no band covers.
///
/// Separate from `tes::Crosswalk` because the two derivations refuse
/// differently. A GB subject with no NZ counterpart is a capture gap and stays
/// residue for the reconciliation queue; a year group no age band covers is a
/// measured absence in a vocabulary we hold whole, which is a no-counterpart
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeCrosswalk {
    pub terms: Vec<CanonicalTerm>,
    pub edges: Vec<ProjectionEdge>,
    pub no_counterparts: Vec<NoCounterpart>,
    pub uncovered: Vec<Uncovered>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GradeError {
    Parse {
        file: &'static str,
        detail: String,
    },
    NonNumericId {
        field: &'static str,
        id: String,
    },
    UnknownYearGroup {
        id: u64,
    },
    UnknownTptGrade {
        id: u64,
    },
    /// The `legacyId` join found no `Grade-Level` or `audience` facet, or
    /// found more than one. Either way the label is unknown, and a grade
    /// seeded under its form label would carry a different string than the
    /// one TPT shows its own sellers.
    TptLabelNotJoinable {
        legacy_id: u64,
        matches: usize,
    },
}

impl core::fmt::Display for GradeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse { file, detail } => write!(f, "{file} is not the expected shape: {detail}"),
            Self::NonNumericId { field, id } => write!(f, "{field} carries a non-numeric id {id}"),
            Self::UnknownYearGroup { id } => {
                write!(
                    f,
                    "the pairing table names year group {id}, which the capture does not hold"
                )
            }
            Self::UnknownTptGrade { id } => {
                write!(
                    f,
                    "the pairing table names TPT grade {id}, which the capture does not hold"
                )
            }
            Self::TptLabelNotJoinable { legacy_id, matches } => write!(
                f,
                "TPT legacy grade {legacy_id} joins {matches} grade or audience facets, not one"
            ),
        }
    }
}

impl core::error::Error for GradeError {}

/// Derives the whole grade axis from the two committed vocabulary captures.
///
/// Pure: both captures arrive as text and the caller does the I/O, so
/// `tam-taxonomy` stays inside the purity gate.
pub fn derive_grade_crosswalk(
    tpt_json: &str,
    tes_json: &str,
    decided_at: Timestamp,
) -> Result<GradeCrosswalk, GradeError> {
    let tpt: TptVocabulary = serde_json::from_str(tpt_json).map_err(|error| GradeError::Parse {
        file: "tpt-vocabulary.json",
        detail: error.to_string(),
    })?;
    let tes: TesVocabulary = serde_json::from_str(tes_json).map_err(|error| GradeError::Parse {
        file: "tes-vocabulary.json",
        detail: error.to_string(),
    })?;

    let year_groups = numbered(&tes.year_groups.options, "yearGroups")?;
    let bands = numbered(&tes.age_ranges.options, "ageRanges")?;
    let grade_levels = numbered(&tpt.grade_levels.options, "gradeLevels")?;
    let labels = grade_labels(&tpt.taxonomy_tags.options);

    let tes_source = Decider::Imported {
        source: tes.source.clone(),
    };
    let tpt_source = Decider::Imported {
        source: tpt.source.clone(),
    };

    for &(tpt_id, year_group) in &GRADE_PAIRS {
        if !year_groups.contains_key(&year_group) {
            return Err(GradeError::UnknownYearGroup { id: year_group });
        }
        if !grade_levels.contains_key(&tpt_id) {
            return Err(GradeError::UnknownTptGrade { id: tpt_id });
        }
    }

    let mut out = GradeCrosswalk {
        terms: Vec::new(),
        edges: Vec::new(),
        no_counterparts: Vec::new(),
        uncovered: Vec::new(),
    };
    let paired: BTreeMap<u64, u64> = GRADE_PAIRS
        .iter()
        .map(|&(tpt_id, year_group)| (year_group, tpt_id))
        .collect();
    let mut covered: BTreeMap<u64, Vec<u64>> = BTreeMap::new();

    for (&id, group) in &year_groups {
        let term = year_group_id(id);
        out.terms.push(CanonicalTerm {
            id: term,
            kind: TermKind::Phase,
            parent: None,
            label: group.group.clone(),
        });
        match gb_route(group, &bands) {
            GbRoute::Onto { band, kind } => {
                if let Some(row) = bands.get(&band) {
                    out.edges.push(ProjectionEdge {
                        from: term,
                        to: band_path(band, row),
                        kind,
                        decided_by: tes_source.clone(),
                        decided_at,
                    });
                    if kind == EdgeKind::Broader {
                        covered.entry(band).or_default().push(id);
                    }
                }
            }
            GbRoute::NoBand => {
                out.uncovered.push(uncovered(id, group));
                out.no_counterparts.push(NoCounterpart {
                    term,
                    target: VocabularyId(InventoryId::TesGb, TermKind::Phase),
                    decided_by: tes_source.clone(),
                    decided_at,
                });
            }
        }
        for inventory in [InventoryId::TesUs, InventoryId::TesNz] {
            out.edges.push(ProjectionEdge {
                from: term,
                to: year_group_path(inventory, id, group),
                kind: EdgeKind::Exact,
                decided_by: tes_source.clone(),
                decided_at,
            });
        }
        match paired.get(&id) {
            Some(&tpt_id) => out.edges.push(ProjectionEdge {
                from: term,
                to: tpt_grade_path(tpt_id, &labels, &grade_levels)?,
                kind: EdgeKind::Exact,
                decided_by: tpt_source.clone(),
                decided_at,
            }),
            None => out.no_counterparts.push(NoCounterpart {
                term,
                target: VocabularyId(InventoryId::Tpt, TermKind::Phase),
                decided_by: tpt_source.clone(),
                decided_at,
            }),
        }
    }

    for &tpt_id in grade_levels.keys() {
        if GRADE_PAIRS
            .iter()
            .any(|&(paired_id, _)| paired_id == tpt_id)
        {
            continue;
        }
        let term = tpt_grade_term_id(tpt_id);
        let path = tpt_grade_path(tpt_id, &labels, &grade_levels)?;
        out.terms.push(CanonicalTerm {
            id: term,
            kind: TermKind::Phase,
            parent: None,
            label: path.segments.first().cloned().unwrap_or_default(),
        });
        out.edges.push(ProjectionEdge {
            from: term,
            to: path,
            kind: EdgeKind::Exact,
            decided_by: tpt_source.clone(),
            decided_at,
        });
        for inventory in TES_INVENTORIES {
            out.no_counterparts.push(NoCounterpart {
                term,
                target: VocabularyId(inventory, TermKind::Phase),
                decided_by: tpt_source.clone(),
                decided_at,
            });
        }
    }

    seed_bands(
        &mut out,
        &BandSeed {
            bands: &bands,
            covered: &covered,
            year_groups: &year_groups,
        },
        &tes_source,
        decided_at,
    );
    Ok(out)
}

/// The Tes GB age bands as vocabulary members in their own right, and the
/// relation from each band to the year groups it covers.
///
/// Without these terms a GB source path ingests to nothing: the projection
/// reverses `Exact` edges only, and every edge a year group holds into
/// `(TesGb, Phase)` is `Broader`, so a GB `ageRanges` id would enter the
/// relation as an unrecognised value on every GB-sourced listing.
///
/// The relation from a band to its year groups is genuinely `Narrower` — one
/// band covers several year groups — which the projection will never derive
/// across, because inverting it would invent the distinction the band dropped.
/// So a GB grade going to a US target is a question for the seller, and these
/// edges are what the question offers as its candidates.
///
/// Six band terms, not seven. The not-applicable band and the not-applicable
/// year group denote the same thing, and the sentinel `Exact` edge seeded
/// beside the covering pass already is that band's identity edge; minting a
/// second term for it would have two terms claim one path as `Exact`, which
/// the reverse-uniqueness index refuses.
struct BandSeed<'a> {
    bands: &'a BTreeMap<u64, AgeRange>,
    covered: &'a BTreeMap<u64, Vec<u64>>,
    year_groups: &'a BTreeMap<u64, YearGroup>,
}

fn seed_bands(
    out: &mut GradeCrosswalk,
    seed: &BandSeed<'_>,
    decided_by: &Decider,
    decided_at: Timestamp,
) {
    for (&band, row) in seed.bands {
        if bounds_of(row) == AgeBounds::NotApplicable {
            continue;
        }
        let term = age_range_id(band);
        out.terms.push(CanonicalTerm {
            id: term,
            kind: TermKind::Phase,
            parent: None,
            label: row.label.clone(),
        });
        out.edges.push(ProjectionEdge {
            from: term,
            to: band_path(band, row),
            kind: EdgeKind::Exact,
            decided_by: decided_by.clone(),
            decided_at,
        });
        let covering: &[u64] = seed.covered.get(&band).map_or(&[], Vec::as_slice);
        for &year_group in covering {
            let Some(group) = seed.year_groups.get(&year_group) else {
                continue;
            };
            for inventory in [InventoryId::TesUs, InventoryId::TesNz] {
                out.edges.push(ProjectionEdge {
                    from: term,
                    to: year_group_path(inventory, year_group, group),
                    kind: EdgeKind::Narrower,
                    decided_by: decided_by.clone(),
                    decided_at,
                });
            }
        }
    }
}

pub fn age_range_id(id: u64) -> CanonicalTermId {
    derived(&format!("tes-age-range:{id}"))
}

/// Where one year group lands in the Tes GB age bands.
enum GbRoute {
    Onto { band: u64, kind: EdgeKind },
    NoBand,
}

/// The GB half of one year group's routing: the covering band, the sentinel,
/// or a measured absence.
///
/// The empty declared-age set is the sentinel and is answered before the
/// covering test, deliberately: "every declared age falls inside the band" is
/// vacuously true of an empty set, so a covering rule that did not answer it
/// first would emit an edge into every bounded band and make the sentinel
/// `Ambiguous`.
fn gb_route(group: &YearGroup, bands: &BTreeMap<u64, AgeRange>) -> GbRoute {
    let (band, kind) = if group.human_ages.is_empty() {
        (sentinel_band(bands), EdgeKind::Exact)
    } else {
        (
            narrowest_covering(&group.human_ages, bands),
            EdgeKind::Broader,
        )
    };
    band.map_or(GbRoute::NoBand, |band| GbRoute::Onto { band, kind })
}

fn uncovered(id: u64, group: &YearGroup) -> Uncovered {
    Uncovered {
        year_group: id,
        label: group.group.clone(),
        human_ages: group.human_ages.clone(),
    }
}

/// The band the not-applicable year group denotes: the one row that bounds
/// nothing. Found rather than hardcoded, so a re-poll that renumbers the
/// sentinel does not silently pair it with a bounded band.
fn sentinel_band(bands: &BTreeMap<u64, AgeRange>) -> Option<u64> {
    bands
        .iter()
        .find(|(_, row)| bounds_of(row) == AgeBounds::NotApplicable)
        .map(|(&id, _)| id)
}

/// The narrowest band that admits every declared age, or none.
///
/// Narrowest rather than nearest: nearest is the plausible wrong
/// implementation and it invents a band for ages no band covers. Ties break on
/// the band's own id so the derivation is deterministic; the captured table
/// has no tie.
fn narrowest_covering(ages: &[u8], bands: &BTreeMap<u64, AgeRange>) -> Option<u64> {
    let min = ages.iter().copied().min()?;
    let max = ages.iter().copied().max()?;
    bands
        .iter()
        .filter(|(_, row)| covers(bounds_of(row), min, max))
        .min_by_key(|(&id, row)| (width(bounds_of(row)), id))
        .map(|(&id, _)| id)
}

/// A band covers a declaration when it admits the lowest declared age and does
/// not end before the highest. The half-open `16+` row therefore covers Years
/// 12 and 13 and US grades 11 and 12, which is the four rows the collapsed
/// `Option<(u8, u8)>` lost.
fn covers(bounds: AgeBounds, min: u8, max: u8) -> bool {
    let Some(low) = bounds.low() else {
        return false;
    };
    let within_upper = match bounds {
        AgeBounds::Between { high, .. } => max <= high,
        AgeBounds::From { .. } => true,
        AgeBounds::NotApplicable => false,
    };
    low <= min && within_upper
}

/// How wide a band is, for the narrowest-covering choice. A half-open band is
/// the widest thing there is, so a bounded band that also covers always wins.
fn width(bounds: AgeBounds) -> u16 {
    match bounds {
        AgeBounds::Between { low, high } => u16::from(high).saturating_sub(u16::from(low)),
        AgeBounds::From { .. } | AgeBounds::NotApplicable => u16::MAX,
    }
}

fn bounds_of(row: &AgeRange) -> AgeBounds {
    match (row.age_low, row.age_high) {
        (Some(low), Some(high)) => AgeBounds::Between { low, high },
        (Some(low), None) => AgeBounds::From { low },
        (None, _) => AgeBounds::NotApplicable,
    }
}

fn numbered<T: Clone>(
    options: &BTreeMap<String, T>,
    field: &'static str,
) -> Result<BTreeMap<u64, T>, GradeError> {
    options
        .iter()
        .map(|(id, value)| {
            let parsed = id.parse().map_err(|_| GradeError::NonNumericId {
                field,
                id: id.clone(),
            })?;
            Ok((parsed, value.clone()))
        })
        .collect()
}

/// The `legacyId` to seller-facing-label join, scoped to the two categories
/// TPT's grade selector draws from. A legacy id that resolves in more than one
/// of them is left out and refused at the call site rather than won by
/// whichever facet the capture happened to list first.
fn grade_labels(tags: &BTreeMap<String, TaxonomyTag>) -> BTreeMap<String, Vec<String>> {
    let mut labels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tag in tags.values() {
        let (Some(legacy), Some(category)) = (tag.legacy_id.as_ref(), tag.category.as_ref()) else {
            continue;
        };
        if !GRADE_TAG_CATEGORIES.contains(&category.as_str()) {
            continue;
        }
        labels
            .entry(legacy.clone())
            .or_default()
            .push(tag.name.clone());
    }
    labels
}

fn tpt_grade_path(
    tpt_id: u64,
    labels: &BTreeMap<String, Vec<String>>,
    grade_levels: &BTreeMap<u64, GradeLevel>,
) -> Result<VocabularyPath, GradeError> {
    if !grade_levels.contains_key(&tpt_id) {
        return Err(GradeError::UnknownTptGrade { id: tpt_id });
    }
    let joined: &[String] = labels.get(&tpt_id.to_string()).map_or(&[], Vec::as_slice);
    let [label] = joined else {
        return Err(GradeError::TptLabelNotJoinable {
            legacy_id: tpt_id,
            matches: joined.len(),
        });
    };
    Ok(VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
        segments: vec![label.clone()],
        native_id: Some(tpt_id.to_string()),
    })
}

fn year_group_path(inventory: InventoryId, id: u64, group: &YearGroup) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(inventory, TermKind::Phase),
        segments: vec![group.group.clone()],
        native_id: Some(id.to_string()),
    }
}

fn band_path(id: u64, row: &AgeRange) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Phase),
        segments: vec![row.label.clone()],
        native_id: Some(id.to_string()),
    }
}

/// Deterministic canonical ids, in `tes.rs`'s scheme, so a re-seed is an
/// explicit no-op and no later capture can re-key a term that already exists.
pub fn year_group_id(id: u64) -> CanonicalTermId {
    derived(&format!("tes-year-group:{id}"))
}

pub fn tpt_grade_term_id(id: u64) -> CanonicalTermId {
    derived(&format!("tpt-grade:{id}"))
}

pub(crate) fn derived(name: &str) -> CanonicalTermId {
    let namespace = uuid::Uuid::from_bytes(NAMESPACE_TAM_TAXONOMY.0);
    let derived = uuid::Uuid::new_v5(&namespace, name.as_bytes());
    CanonicalTermId(tam_types::Uuid(derived.into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::{derive_grade_crosswalk, year_group_id, GradeCrosswalk};
    use crate::project::{ingest_by_native_id, ingest_grades, project, project_axis, AxisRequest};
    use tam_domain::equivalence::{ElectionTrigger, Loss, PricingBranch};
    use tam_domain::registry::{registry, AxisBinding};
    use tam_domain::{
        DeclarationSource, EdgeKind, GradeDeclaration, TermKind, TermProjection, VocabularyId,
        VocabularyPath,
    };
    use tam_types::{InventoryId, ProductId, Timestamp, Uuid};

    const TPT: &str = include_str!("../../../docs/design/data/tpt-vocabulary.json");
    const TES: &str = include_str!("../../../docs/design/data/tes-vocabulary.json");
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

    #[test]
    fn no_band_is_invented_where_none_covers() {
        let crosswalk = crosswalk();
        for (year_group, why) in [(1_u64, "GB Nursery, ages 1-4"), (16, "US Pre-K, ages 1-5")] {
            let term = year_group_id(year_group);
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
        let term = year_group_id(30);
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
            Some(year_group_id(30)),
            "the two not-applicable sentinels denote one thing and share one term"
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
        let tpt = edges_into(&crosswalk, InventoryId::Tpt);
        assert_eq!(
            tpt.len(),
            19,
            "nineteen TPT grade options, each claimed once"
        );
        let mut ids: Vec<&str> = tpt
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
            15,
            "the fifteen GB year groups have no TPT counterpart"
        );
        for record in &absent {
            assert!(
                !crosswalk.edges.iter().any(|edge| edge.from == record.term
                    && edge.to.vocabulary == VocabularyId(InventoryId::Tpt, TermKind::Phase)),
                "a term is paired or recorded absent, never both"
            );
        }
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
            .find(|edge| edge.to.native_id.as_deref() == Some("1"))
            .expect("TPT grade 1 is paired");
        assert_eq!(
            preschool.to.segments,
            vec!["Preschool".to_owned()],
            "TPT's own label for grade 1 is Preschool; PreK is the create form's abbreviation"
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
            .expect("every Tes inventory binds a phase axis")
    }

    #[test]
    fn a_us_year_group_broadens_onto_a_gb_band_and_names_what_it_dropped() {
        let crosswalk = crosswalk();
        let declared = declaration(InventoryId::TesUs, "21");
        let ingested = ingest_grades(&declared, &crosswalk.edges);
        assert_eq!(
            ingested.terms,
            vec![year_group_id(21)],
            "a US grade is a member of the yearGroups vocabulary and ingests by its own id"
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
}
