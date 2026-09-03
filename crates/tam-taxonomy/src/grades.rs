//! The grade axis, derived from the two polled vocabularies rather than
//! transcribed.
//!
//! The canonical grade vocabulary is minted from TPT's own grade options,
//! because TPT is the base product model: a canonical phase carries TPT's
//! identity and TPT's label, and Tes is a projection target like any other.
//! Fifteen of TPT's nineteen options pair against a Tes `yearGroups` row
//! through one authored table, and everything else — labels, ages, band
//! coverage — is read out of the JSONs, so a re-poll cannot leave a
//! transcription behind.
//!
//! TPT is a superset of the axis rather than the whole of it, and the
//! extension pass is where that is admitted. Fifteen `yearGroups` rows name a
//! phase TPT has no option for — GB Nursery, Reception and Years 1 to 13 —
//! and all thirty rows are selectable on the Tes US and NZ wires, so those
//! fifteen are minted under Tes's identity. The split is the whole content of
//! the base decision: which side a term's identifier and label come from, and
//! nothing about the projection functions, which decide on the edge set alone.
//!
//! Ages are the one thing the base cannot supply. TPT's `gradeLevels` carries
//! form labels and no age semantics at all, so a TPT-minted term reaches a Tes
//! GB age band only through the year group the pairing table names for it, and
//! the four TPT options no Tes row pairs reach no band at all.
//!
//! The GB fork is the interesting half. The Tes editor takes `ageRanges` when
//! the country is GB and `yearGroups` otherwise, so `VocabularyId(TesGb,
//! Phase)` denotes the seven age bands while `VocabularyId(TesUs, Phase)`
//! denotes the thirty year groups. A phase reaches a band by *covering*: the
//! narrowest band that admits the lowest declared age and does not end before
//! the highest. Where no band covers, the derivation emits no edge and a
//! no-counterpart record, because a nearest-band rule would silently file ages
//! 1-4 under the 3-5 band and pass every count-based check.
//!
//! Neither half of a TPT grade path is in `gradeLevels`, which holds a bare
//! id-to-form-label map; the seller-facing label and the slug TPT addresses
//! the grade by both live in `taxonomyTags`, behind a `legacyId` join. That
//! join is only unique within a category — `legacyId` 1 names both the
//! `Grade-Level` facet `preschool` and the `Price-Range` facet `free` — so it
//! is scoped to the two categories the grade selector draws from, and an
//! ambiguous or missing join is refused rather than guessed.
//!
//! The slug is the identifier the path carries, not the `legacyId`. TPT posts
//! and reads a grade as a `taxonomyTags` member — `6th-grade` beside `math`
//! and `pdf` — while `legacyId` addresses the create form's own selector, and
//! the two are not interchangeable on the wire. A path seeded under the
//! numeric id is refused by the TPT adapter's slug-shape guard on the way
//! out, and fails to ingest a TPT-sourced grade on the way in, because
//! neither direction ever sees that number.

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
///
/// The table survives the re-base onto TPT unchanged, and reading it in the
/// other direction is the whole of what the re-base does to it: the pairs are
/// the same fifteen, and which side mints the term is decided by which pass
/// visits it first.
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
    /// The slug of the facet this one hangs under, where the capture states
    /// one. `taxonomyTags` addresses a parent by slug rather than by
    /// `legacyId`, so this is a key into the same map.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// A retired facet: it reads back on existing products and is never
    /// offered on create.
    #[serde(default)]
    pub is_hidden: Option<bool>,
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// found more than one. Either way both the label and the slug are
    /// unknown: a grade seeded under its form label would carry a different
    /// string than the one TPT shows its own sellers, and one seeded under
    /// its `legacyId` would carry an identifier TPT's tag namespace does not
    /// answer to.
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
    let mint = Mint::read(tpt_json, tes_json)?;
    let mut building = Building::default();
    mint.mint_from_tpt(&mut building, decided_at)?;
    mint.mint_tes_extensions(&mut building, decided_at);
    mint.seed_bands(&mut building, decided_at);
    Ok(building.out)
}

/// Both captures as the derivation reads them, plus the two deciders every
/// row it emits is attributed to.
struct Mint {
    year_groups: BTreeMap<u64, YearGroup>,
    bands: BTreeMap<u64, AgeRange>,
    grade_levels: BTreeMap<u64, GradeLevel>,
    facets: BTreeMap<String, Vec<GradeFacet>>,
    /// TPT grade to Tes year group, and its inverse, from `GRADE_PAIRS`.
    /// Both directions are held because the first pass asks "what does this
    /// TPT grade pair to" and the second asks "is this Tes row already
    /// spoken for".
    paired: BTreeMap<u64, u64>,
    spoken_for: BTreeMap<u64, u64>,
    tes_source: Decider,
    tpt_source: Decider,
}

/// The derivation in progress: the crosswalk being built, and the band
/// bookkeeping the two minting passes accumulate and `seed_bands` spends.
#[derive(Default)]
struct Building {
    out: GradeCrosswalk,
    /// Band to the year groups it covers, filled by whichever pass routed
    /// that year group's ages.
    covered: BTreeMap<u64, Vec<u64>>,
    /// The TPT grade path each paired year group resolves to, keyed by year
    /// group, so the band relation reads the same join the first pass did
    /// rather than re-deriving it.
    tpt_grades: BTreeMap<u64, VocabularyPath>,
}

impl Mint {
    fn read(tpt_json: &str, tes_json: &str) -> Result<Self, GradeError> {
        let tpt: TptVocabulary =
            serde_json::from_str(tpt_json).map_err(|error| GradeError::Parse {
                file: "tpt-vocabulary.json",
                detail: error.to_string(),
            })?;
        let tes: TesVocabulary =
            serde_json::from_str(tes_json).map_err(|error| GradeError::Parse {
                file: "tes-vocabulary.json",
                detail: error.to_string(),
            })?;
        let mint = Self {
            year_groups: numbered(&tes.year_groups.options, "yearGroups")?,
            bands: numbered(&tes.age_ranges.options, "ageRanges")?,
            grade_levels: numbered(&tpt.grade_levels.options, "gradeLevels")?,
            facets: grade_facets(&tpt.taxonomy_tags.options),
            paired: GRADE_PAIRS.iter().copied().collect(),
            spoken_for: GRADE_PAIRS
                .iter()
                .map(|&(tpt_id, year_group)| (year_group, tpt_id))
                .collect(),
            tes_source: Decider::Imported {
                source: tes.source.clone(),
            },
            tpt_source: Decider::Imported {
                source: tpt.source.clone(),
            },
        };
        for &(tpt_id, year_group) in &GRADE_PAIRS {
            if !mint.year_groups.contains_key(&year_group) {
                return Err(GradeError::UnknownYearGroup { id: year_group });
            }
            if !mint.grade_levels.contains_key(&tpt_id) {
                return Err(GradeError::UnknownTptGrade { id: tpt_id });
            }
        }
        Ok(mint)
    }

    /// The base pass. Every TPT grade option becomes a canonical term under
    /// TPT's own identity and TPT's own label, whether or not Tes pairs it,
    /// and the pairing table decides only what that term reaches on the Tes
    /// side.
    fn mint_from_tpt(
        &self,
        building: &mut Building,
        decided_at: Timestamp,
    ) -> Result<(), GradeError> {
        for &tpt_id in self.grade_levels.keys() {
            let term = tpt_grade_term_id(tpt_id);
            let path = tpt_grade_path(tpt_id, &self.facets, &self.grade_levels)?;
            building.out.terms.push(CanonicalTerm {
                id: term,
                kind: TermKind::Phase,
                parent: None,
                label: path.segments.first().cloned().unwrap_or_default(),
            });
            building.out.edges.push(ProjectionEdge {
                from: term,
                to: path.clone(),
                kind: EdgeKind::Exact,
                decided_by: self.tpt_source.clone(),
                decided_at,
            });
            match self.paired.get(&tpt_id) {
                Some(&year_group) => {
                    building.tpt_grades.insert(year_group, path);
                    self.route_tes(building, term, year_group, decided_at);
                }
                None => {
                    for inventory in TES_INVENTORIES {
                        building.out.no_counterparts.push(NoCounterpart {
                            term,
                            target: VocabularyId(inventory, TermKind::Phase),
                            decided_by: self.tpt_source.clone(),
                            decided_at,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// The extension pass, and the reason the canonical model is a TPT
    /// superset rather than TPT alone.
    ///
    /// Fifteen `yearGroups` rows — GB Nursery, Reception and Years 1 to 13 —
    /// name a phase TPT's own vocabulary has no option for, and the whole
    /// thirty-row set is selectable on the Tes US and NZ wires. Left unminted
    /// they would ingest to nothing, so every Tes-sourced listing declaring
    /// one would carry an unrecognised grade. They are minted under Tes's
    /// identity and Tes's label, which is what marks them as the target's
    /// contribution rather than the base's.
    fn mint_tes_extensions(&self, building: &mut Building, decided_at: Timestamp) {
        for (&year_group, group) in &self.year_groups {
            if self.spoken_for.contains_key(&year_group) {
                continue;
            }
            let term = year_group_id(year_group);
            building.out.terms.push(CanonicalTerm {
                id: term,
                kind: TermKind::Phase,
                parent: None,
                label: group.group.clone(),
            });
            self.route_tes(building, term, year_group, decided_at);
            building.out.no_counterparts.push(NoCounterpart {
                term,
                target: VocabularyId(InventoryId::Tpt, TermKind::Phase),
                decided_by: self.tpt_source.clone(),
                decided_at,
            });
        }
    }

    /// One canonical term's whole Tes side: the GB band it covers into, and
    /// its identity in the two `yearGroups` inventories.
    ///
    /// Reached from both passes and from the same year group either way,
    /// because the ages a band is matched on are Tes's. TPT's `gradeLevels`
    /// carries labels and nothing else, so a TPT-minted term has no ages of
    /// its own and reaches a GB band only through the row the pairing table
    /// names for it.
    fn route_tes(
        &self,
        building: &mut Building,
        term: CanonicalTermId,
        year_group: u64,
        decided_at: Timestamp,
    ) {
        // Unreachable: `Mint::read` refuses a pairing table naming a year
        // group the capture does not hold, and the extension pass iterates
        // the capture itself. Returning rather than panicking keeps the
        // derivation total, and a lookup that did fire would drop one term's
        // Tes side rather than corrupt another's.
        let Some(group) = self.year_groups.get(&year_group) else {
            return;
        };
        match gb_route(group, &self.bands) {
            GbRoute::Onto { band, kind } => {
                if let Some(row) = self.bands.get(&band) {
                    building.out.edges.push(ProjectionEdge {
                        from: term,
                        to: band_path(band, row),
                        kind,
                        decided_by: self.tes_source.clone(),
                        decided_at,
                    });
                    if kind == EdgeKind::Broader {
                        building.covered.entry(band).or_default().push(year_group);
                    }
                }
            }
            GbRoute::NoBand => {
                building.out.uncovered.push(uncovered(year_group, group));
                building.out.no_counterparts.push(NoCounterpart {
                    term,
                    target: VocabularyId(InventoryId::TesGb, TermKind::Phase),
                    decided_by: self.tes_source.clone(),
                    decided_at,
                });
            }
        }
        for inventory in [InventoryId::TesUs, InventoryId::TesNz] {
            building.out.edges.push(ProjectionEdge {
                from: term,
                to: year_group_path(inventory, year_group, group),
                kind: EdgeKind::Exact,
                decided_by: self.tes_source.clone(),
                decided_at,
            });
        }
    }

    /// The Tes GB age bands as vocabulary members in their own right, and the
    /// relation from each band to the phases it covers.
    ///
    /// Without these terms a GB source path ingests to nothing: the projection
    /// reverses `Exact` edges only, and every edge a covered phase holds into
    /// `(TesGb, Phase)` is `Broader`, so a GB `ageRanges` id would enter the
    /// relation as an unrecognised value on every GB-sourced listing.
    ///
    /// The relation from a band to what it covers is genuinely `Narrower` —
    /// one band covers several year groups — which the projection will never
    /// derive across, because inverting it would invent the distinction the
    /// band dropped. So a GB grade going to a US target is a question for the
    /// seller, and these edges are what the question offers as its candidates.
    ///
    /// Six band terms, not seven. The not-applicable band and the
    /// not-applicable phase denote the same thing, and the sentinel `Exact`
    /// edge seeded beside the covering pass already is that band's identity
    /// edge; minting a second term for it would have two terms claim one path
    /// as `Exact`, which the reverse-uniqueness index refuses.
    ///
    /// TPT is related the same way and from the same measurement. A band
    /// holds no age semantics of TPT's own, so a band reaches a TPT grade only
    /// through the year group the pairing table names for it: the band covers
    /// the year group, the year group is that TPT grade, so the band covers
    /// that grade. Every band the captures produce covers either several TPT
    /// grades or none, so the relation is the same `Narrower` fan-out and asks
    /// the seller the same question. Seeding it `Exact` instead is not
    /// available even where a band covered one grade: the TPT-minted term
    /// already claims that TPT path as `Exact`, and the reverse-uniqueness
    /// index admits one such claim per path.
    ///
    /// A band that covers no TPT grade at all is a measured absence and takes
    /// a no-counterpart record, on the same rule the uncovered year groups
    /// take one: the 3-5 band covers only Reception, which TPT does not pair,
    /// because TPT's Preschool spans an age range wider at the bottom than the
    /// band and its Kindergarten one wider at the top. Left as neither an edge
    /// nor a record it would be an unanswerable gap, blocking every GB listing
    /// that declares it.
    fn seed_bands(&self, building: &mut Building, decided_at: Timestamp) {
        for (&band, row) in &self.bands {
            if bounds_of(row) == AgeBounds::NotApplicable {
                continue;
            }
            let term = age_range_id(band);
            building.out.terms.push(CanonicalTerm {
                id: term,
                kind: TermKind::Phase,
                parent: None,
                label: row.label.clone(),
            });
            building.out.edges.push(ProjectionEdge {
                from: term,
                to: band_path(band, row),
                kind: EdgeKind::Exact,
                decided_by: self.tes_source.clone(),
                decided_at,
            });
            let mut covering: Vec<u64> = building.covered.get(&band).cloned().unwrap_or_default();
            // Ascending by year group rather than by visit order. The covering
            // set is filled by whichever minting pass routed each row, so
            // under the TPT base the paired rows arrive before the extension
            // rows and the raw order is US grades before GB years. That order
            // is seller-facing — it is the candidate list a `Narrow` election
            // offers — so it is decided here rather than inherited from which
            // side minted the term.
            covering.sort_unstable();
            self.seed_band_candidates(building, term, &covering, decided_at);
        }
    }

    /// One band's `Narrower` fan-out into the three target vocabularies, and
    /// the measured absence a band covering no TPT grade takes instead.
    fn seed_band_candidates(
        &self,
        building: &mut Building,
        term: CanonicalTermId,
        covering: &[u64],
        decided_at: Timestamp,
    ) {
        for &year_group in covering {
            let Some(group) = self.year_groups.get(&year_group) else {
                continue;
            };
            for inventory in [InventoryId::TesUs, InventoryId::TesNz] {
                building.out.edges.push(ProjectionEdge {
                    from: term,
                    to: year_group_path(inventory, year_group, group),
                    kind: EdgeKind::Narrower,
                    decided_by: self.tes_source.clone(),
                    decided_at,
                });
            }
        }
        let grades: Vec<VocabularyPath> = covering
            .iter()
            .filter_map(|year_group| building.tpt_grades.get(year_group).cloned())
            .collect();
        if grades.is_empty() {
            building.out.no_counterparts.push(NoCounterpart {
                term,
                target: VocabularyId(InventoryId::Tpt, TermKind::Phase),
                decided_by: self.tpt_source.clone(),
                decided_at,
            });
        }
        for path in grades {
            building.out.edges.push(ProjectionEdge {
                from: term,
                to: path,
                kind: EdgeKind::Narrower,
                decided_by: self.tpt_source.clone(),
                decided_at,
            });
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

/// The two halves of a TPT grade path, as `taxonomyTags` states them: the
/// slug the facet is keyed by, which is what TPT's wire addresses it with, and
/// the seller-facing name.
struct GradeFacet {
    slug: String,
    name: String,
}

/// The `legacyId` to grade-facet join, scoped to the two categories TPT's
/// grade selector draws from. A legacy id that resolves in more than one of
/// them is left out and refused at the call site rather than won by whichever
/// facet the capture happened to list first.
fn grade_facets(tags: &BTreeMap<String, TaxonomyTag>) -> BTreeMap<String, Vec<GradeFacet>> {
    let mut facets: BTreeMap<String, Vec<GradeFacet>> = BTreeMap::new();
    for (slug, tag) in tags {
        let (Some(legacy), Some(category)) = (tag.legacy_id.as_ref(), tag.category.as_ref()) else {
            continue;
        };
        if !GRADE_TAG_CATEGORIES.contains(&category.as_str()) {
            continue;
        }
        facets.entry(legacy.clone()).or_default().push(GradeFacet {
            slug: slug.clone(),
            name: tag.name.clone(),
        });
    }
    facets
}

fn tpt_grade_path(
    tpt_id: u64,
    facets: &BTreeMap<String, Vec<GradeFacet>>,
    grade_levels: &BTreeMap<u64, GradeLevel>,
) -> Result<VocabularyPath, GradeError> {
    if !grade_levels.contains_key(&tpt_id) {
        return Err(GradeError::UnknownTptGrade { id: tpt_id });
    }
    let joined: &[GradeFacet] = facets.get(&tpt_id.to_string()).map_or(&[], Vec::as_slice);
    let [facet] = joined else {
        return Err(GradeError::TptLabelNotJoinable {
            legacy_id: tpt_id,
            matches: joined.len(),
        });
    };
    Ok(VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Phase),
        segments: vec![facet.name.clone()],
        native_id: Some(facet.slug.clone()),
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
mod tests;
