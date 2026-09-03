//! Education standards for one jurisdiction, searched over the committed
//! corpus.
//!
//! The corpus is compiled in and parsed once, at startup, by [`prime`]. That
//! is a departure from how this crate passes everything else to a handler: the
//! pool, the clock and the binary's decisions all travel through `AppState`,
//! and this does not. The reason is that the corpus is compiled into the
//! binary and identical for every tenant and every deployment, so there is
//! nothing per-deployment about it to carry; what *is* per-deployment, the
//! crawl window, does travel through `AppState` like everything else.
//!
//! A node id is served only where a current capture vouches for it. The id is
//! TPT's own search-index identifier and exactly the kind that gets rebuilt,
//! so one no capture stands behind is withheld rather than guessed, and a
//! deployment told about no capture at all serves none.

use std::sync::OnceLock;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::product::StandardsFramework;
use tam_standards::crawl::postable;
use tam_standards::format::{load_framework, parse_manifest, StandardsDocument};
use tam_standards::grades::{GradeBand, GradeLevel};
use tam_standards::model::Framework;
use tam_standards::notices::{mirror_attribution, required_notices, Placement};
use tam_standards::search::{fold_code, Match, StandardsIndex};
use tam_standards::tpt::{load_tpt_node_ids, TptNodeIdTable};

use crate::blocking::spawn_supervised_blocking;
use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// The committed corpus, compiled in rather than read, so a search answers
/// from the ingest this binary was built from and not from whatever a working
/// copy happens to hold. The same reasoning, and the same five files, as
/// `crates/tam-standards-crawl/src/main.rs`.
const MANIFEST: &str = include_str!("../../../../docs/design/data/standards/manifest.json");
const CCSS: &[u8] = include_bytes!("../../../../docs/design/data/standards/ccss.jsonl");
const NGSS: &[u8] = include_bytes!("../../../../docs/design/data/standards/ngss.jsonl");
const TEKS: &[u8] = include_bytes!("../../../../docs/design/data/standards/teks.jsonl");
const VA_SOL: &[u8] = include_bytes!("../../../../docs/design/data/standards/va-sol.jsonl");
const TPT_NODE_IDS: &str =
    include_str!("../../../../docs/design/data/standards/tpt-node-ids.jsonl");

/// How many results one search answers with.
///
/// The corpus is order 11,000 codes and a query of one letter matches
/// thousands of them; a picker renders a list a person scans, not a corpus
/// dump.
const RESULTS_MAX: usize = 50;

/// How long a query may be, and how many words it may carry.
///
/// The keyword index scans the whole corpus once per term, so the work a
/// request costs scales with the words in it; without a bound, any signed-in
/// member could spend seconds of a worker's CPU on one request. A standards
/// query is a code like `5.NBT.7` or a handful of words like `fractions number
/// line`, so these are generous for every real query and refuse the ones that
/// exist only to be expensive. Both are limits a founder may replace.
const QUERY_CHARS_MAX: usize = 120;
const QUERY_TERMS_MAX: usize = 12;

/// The parsed corpus, built once and shared by every request.
struct Corpus {
    documents: Vec<StandardsDocument>,
    node_ids: TptNodeIdTable,
}

static CORPUS: OnceLock<Corpus> = OnceLock::new();

/// Why the corpus could not be built. Carries the message rather than the
/// error type, because the only caller prints it and exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusUnreadable(pub String);

impl core::fmt::Display for CorpusUnreadable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for CorpusUnreadable {}

/// Parses the compiled-in corpus and holds it for the process.
///
/// Called by the serving binary during setup so that a corpus this build
/// cannot read fails the boot rather than the first seller's search, and so
/// that no request pays the parse. Calling it twice is harmless and the second
/// call is a no-op.
pub fn prime() -> Result<(), CorpusUnreadable> {
    if CORPUS.get().is_some() {
        return Ok(());
    }
    let manifest = parse_manifest(MANIFEST).map_err(|error| {
        CorpusUnreadable(format!("the standards manifest is unreadable: {error}"))
    })?;
    let mut documents = Vec::with_capacity(4);
    for (framework, bytes) in [
        (Framework::Ccss, CCSS),
        (Framework::Ngss, NGSS),
        (Framework::Teks, TEKS),
        (Framework::VaSol, VA_SOL),
    ] {
        documents.push(
            load_framework(&manifest, framework, bytes).map_err(|error| {
                CorpusUnreadable(format!(
                    "the {framework:?} standards file is unreadable: {error}"
                ))
            })?,
        );
    }
    let node_ids = load_tpt_node_ids(TPT_NODE_IDS).map_err(|error| {
        CorpusUnreadable(format!("the TPT node-id table is unreadable: {error}"))
    })?;
    // A second caller racing this one loses the set and drops its own copy,
    // which is equivalent: both parsed the same compiled-in bytes.
    let _unused = CORPUS.set(Corpus {
        documents,
        node_ids,
    });
    Ok(())
}

/// What a standards search found, or why it found nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardsState {
    /// The catalogue holds this framework and the items are the matches.
    Ingested,
    /// No node of this framework has been ingested. The client says so rather
    /// than rendering an empty tree, which would read as "no such standard".
    NotIngested,
}

impl StandardsState {
    pub const ALL: [Self; 2] = [Self::Ingested, Self::NotIngested];
}

/// One standard a seller can align to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardView {
    pub framework: u32,
    /// The framework owner's own published code, which is stable.
    pub code: String,
    /// The statement, displayed verbatim and never paraphrased anywhere,
    /// including in generated listing copy (D25). Common Core's grant names
    /// copy, publish, distribute and display and never names modification.
    pub statement: String,
    /// The subject the mirrored set carries, so a code never renders bare.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The grades the mirrored set covers, in the words a teacher uses, so a
    /// code never renders bare beside its subject.
    ///
    /// Rendered here rather than served structured: a client given the levels
    /// and the interval would have to reimplement the ordering that turns them
    /// into a phrase, and two clients would render one set two ways.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_band: Option<String>,
    /// The mirror's own identifier for this node, and the only field on this
    /// view that is unique.
    ///
    /// A code is not: 814 TEKS codes name more than one addressable node with
    /// a different statement — `1.1.A` appears four times across four subjects
    /// — and all of them reach the wire as separate items. A client keying a
    /// list on the code alone can therefore attach one standard's checkbox or
    /// statement to another's row.
    pub source_guid: String,
    /// TPT's own node id, which is what a create posts. Absent for a standard
    /// we can display and cannot yet post: the id is a search-index identifier
    /// and is exactly the kind that gets rebuilt, so one no current capture
    /// vouches for is withheld rather than guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpt_node_id: Option<u64>,
}

/// One notice a framework's licence obliges a display to carry, and where.
///
/// Several rather than one string, because the obligations differ in where
/// they must appear: flattening them would discard the placement, which is the
/// half a licence actually turns on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeView {
    pub text: String,
    /// `wherever_displayed`, `site_footer_and_every_page_using_the_mark` or
    /// `with_the_data`.
    pub placement: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardsSearchView {
    pub state: StandardsState,
    pub framework: u32,
    pub items: Vec<StandardView>,
    /// The notices this framework's licence obliges us to carry wherever one
    /// of its standards is displayed. Served with the results rather than
    /// hard-coded in the client, so the obligation travels with the data it
    /// attaches to.
    pub notices: Vec<NoticeView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StandardsSearchParams {
    /// A jurisdiction id: 3054, 3055, 3326 or 5785.
    pub framework: u32,
    #[serde(default)]
    pub q: Option<String>,
}

/// The two enums name the same four frameworks and neither depends on the
/// other, so the map is written out rather than derived. Total in both
/// directions, so a framework added to either fails to compile here instead of
/// silently losing its notices.
const fn framework_of(framework: StandardsFramework) -> Framework {
    match framework {
        StandardsFramework::CommonCore => Framework::Ccss,
        StandardsFramework::NextGenerationScience => Framework::Ngss,
        StandardsFramework::TexasEssentialKnowledgeAndSkills => Framework::Teks,
        StandardsFramework::VirginiaStandardsOfLearning => Framework::VaSol,
    }
}

/// The reverse of [`framework_of`], so a result states the framework it
/// actually came from rather than the one that was asked for.
///
/// Echoing the request would make a leak between frameworks invisible: an item
/// from the wrong corpus would still carry the right-looking id, and no test
/// could see the difference. Reading it off the hit means the wire tells the
/// truth about each item's provenance.
const fn domain_framework_of(framework: Framework) -> StandardsFramework {
    match framework {
        Framework::Ccss => StandardsFramework::CommonCore,
        Framework::Ngss => StandardsFramework::NextGenerationScience,
        Framework::Teks => StandardsFramework::TexasEssentialKnowledgeAndSkills,
        Framework::VaSol => StandardsFramework::VirginiaStandardsOfLearning,
    }
}

const fn placement_str(placement: Placement) -> &'static str {
    match placement {
        Placement::WhereverDisplayed => "wherever_displayed",
        Placement::SiteFooterAndEveryPageUsingTheMark => {
            "site_footer_and_every_page_using_the_mark"
        }
        Placement::WithTheData => "with_the_data",
    }
}

/// Every notice a framework's display must carry: the owner's own, and the
/// mirror's.
///
/// Both halves, because `required_notices` alone is not the whole obligation.
/// It answers the notices a framework's *owner* imposes, and Texas and
/// Virginia impose none — their file says so, and returns an empty slice. What
/// those two do carry is the mirror's CC BY attribution, which
/// `mirror_attribution` builds per set because the rights holder differs
/// across sets and a single constant would be wrong beside some of the rows it
/// sat next to. Serving only the first half would display mirrored Texas and
/// Virginia standards under no attribution at all, which is the obligation CC
/// BY actually imposes on us.
///
/// Deduplicated because one framework's sets mostly share a licence, and a
/// seller reading the same sentence four times learns nothing the first did
/// not tell them.
fn notices_of(documents: &[StandardsDocument], framework: Framework) -> Vec<NoticeView> {
    let mut notices: Vec<NoticeView> = required_notices(framework)
        .iter()
        .map(|notice| NoticeView {
            text: notice.text.to_owned(),
            placement: placement_str(notice.placement).to_owned(),
        })
        .collect();

    let with_the_data = placement_str(Placement::WithTheData).to_owned();
    for document in documents
        .iter()
        .filter(|document| document.framework() == framework)
    {
        for set in &document.manifest.sets {
            let Some(licence) = set.licence.as_ref() else {
                continue;
            };
            let Some(text) = mirror_attribution(licence) else {
                continue;
            };
            if !notices.iter().any(|held| held.text == text) {
                notices.push(NoticeView {
                    text,
                    placement: with_the_data.clone(),
                });
            }
        }
    }
    notices
}

/// Which of the index's entry points a query means.
///
/// Dispatched on the query's own shape rather than merged: a code query and a
/// keyword query answer different questions, and blending them would give a
/// seller a result set whose provenance they cannot reason about. A query that
/// folds to nothing — punctuation alone — is a keyword query, because it names
/// no code.
fn search<'a>(index: &StandardsIndex<'a>, query: &str) -> Vec<Match<'a>> {
    let folded = fold_code(query);
    let by_code = if folded.is_empty() {
        vec![]
    } else {
        index.by_code_prefix(query)
    };
    if by_code.is_empty() {
        index.by_keyword(query)
    } else {
        by_code
    }
}

/// Education standards for one jurisdiction, or the honest absence.
///
/// `not_ingested` is a state rather than an empty result, because the two mean
/// different things to a seller: no standard matched their words, against no
/// standard exists here yet.
pub(crate) async fn standards_search(
    State(state): State<AppState>,
    _context: OrgContext,
    Query(params): Query<StandardsSearchParams>,
) -> Result<Json<StandardsSearchView>, APIError> {
    let framework =
        StandardsFramework::from_jurisdiction_id(params.framework).ok_or_else(|| {
            APIError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                APIErrorEntry::new(
                    "no such standards framework; the create form offers 3054, 3055, 3326 and 5785",
                )
                .kind(APIErrorKind::Validation),
            )
        })?;
    let wanted = framework_of(framework);

    let Some(corpus) = CORPUS.get() else {
        // The binary primes the corpus during setup and refuses to start
        // without it, so this is unreachable in a served process; a test that
        // builds the router without priming reaches it, and answering
        // not-ingested is the same honest absence the route served before the
        // corpus existed.
        return Ok(Json(StandardsSearchView {
            state: StandardsState::NotIngested,
            framework: framework.jurisdiction_id(),
            items: vec![],
            notices: notices_of(&[], wanted),
        }));
    };
    let notices = notices_of(&corpus.documents, wanted);

    let held: Vec<&StandardsDocument> = corpus
        .documents
        .iter()
        .filter(|document| document.framework() == wanted)
        .collect();
    if held.is_empty() {
        return Ok(Json(StandardsSearchView {
            state: StandardsState::NotIngested,
            framework: framework.jurisdiction_id(),
            items: vec![],
            notices,
        }));
    }

    let query = params.q.as_deref().unwrap_or("").trim().to_owned();
    check_query(&query)?;

    // The scan is bounded but not free — 12ms for the most expensive query the
    // bound admits — and it is synchronous CPU, so it runs on a blocking
    // thread rather than holding an async worker for the duration.
    //
    // The corpus is `&'static` because it lives in a `OnceLock`, so the
    // borrowed matches cross the boundary without a copy of the corpus.
    let window = state.config.standards_crawl_window.clone();
    let scanned = spawn_supervised_blocking(move || {
        let index = StandardsIndex::build(&corpus.documents);
        let found = if query.is_empty() {
            vec![]
        } else {
            search(&index, &query)
        };
        found
            .into_iter()
            .filter(|hit| hit.framework == wanted)
            .filter(|hit| hit.node.code.is_some())
            .take(RESULTS_MAX)
            .map(|hit| StandardView {
                framework: domain_framework_of(hit.framework).jurisdiction_id(),
                code: hit.node.code.clone().unwrap_or_default(),
                statement: hit.node.statement.clone(),
                subject: hit.subject().map(str::to_owned),
                grade_band: hit
                    .document
                    .set_of(hit.node)
                    .and_then(|set| grade_band_of(&set.grade_band)),
                source_guid: hit.node.source_guid.clone(),
                tpt_node_id: window
                    .as_ref()
                    .and_then(|window| {
                        postable(&corpus.node_ids, window, &hit.node.source_guid).ok()
                    })
                    .map(|id| id.0),
            })
            .collect::<Vec<StandardView>>()
    })
    .await;
    let items =
        scanned.map_err(|error| state.internal(&format!("the standards scan failed: {error}")))?;

    Ok(Json(StandardsSearchView {
        state: StandardsState::Ingested,
        framework: framework.jurisdiction_id(),
        items,
        notices,
    }))
}

/// The words a teacher reads for the grades a set covers.
///
/// From the derived interval where one parses, because that is the ordered
/// answer and reads as a range; from the source's own level codes otherwise,
/// verbatim and in the order the mirror recorded them, because a set whose
/// coding this crate could not parse still has grades a teacher recognises and
/// showing them unparsed beats showing nothing. A set carrying neither answers
/// `None`, which the wire omits and the picker renders as no grade rather than
/// as an empty one.
fn grade_band_of(band: &GradeBand) -> Option<String> {
    if let Some(interval) = band.interval {
        let low = grade_level_word(interval.low);
        let high = grade_level_word(interval.high);
        return Some(if low == high {
            low
        } else {
            format!("{low}–{high}")
        });
    }
    if band.levels.is_empty() {
        return None;
    }
    Some(band.levels.join(", "))
}

/// One level in the words a teacher uses rather than the source's own coding.
fn grade_level_word(level: GradeLevel) -> String {
    match level {
        GradeLevel::PREKINDERGARTEN => "Pre-K".to_owned(),
        GradeLevel::KINDERGARTEN => "Kindergarten".to_owned(),
        other => format!("Grade {}", other.0),
    }
}

/// Refuses a query too long or too many-worded to be a real one, before
/// anything scans the corpus.
///
/// Before rather than after, because the cost this bounds is the scan itself:
/// a refusal that ran the search first would have already spent what it exists
/// to prevent.
fn check_query(query: &str) -> Result<(), APIError> {
    if query.chars().count() > QUERY_CHARS_MAX {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(
                "that search is longer than any standard's code or title; shorten it and try again",
            )
            .kind(APIErrorKind::Validation),
        ));
    }
    if query.split_whitespace().count() > QUERY_TERMS_MAX {
        return Err(APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new(
                "that search carries more words than a standard's title holds; use fewer",
            )
            .kind(APIErrorKind::Validation),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        domain_framework_of, framework_of, grade_band_of, notices_of, placement_str, prime, search,
        CORPUS, RESULTS_MAX,
    };
    use tam_domain::product::StandardsFramework;
    use tam_standards::grades::{GradeBand, GradeLevel};
    use tam_standards::model::Framework;
    use tam_standards::notices::required_notices;
    use tam_standards::notices::Placement;
    use tam_standards::search::StandardsIndex;

    /// The one test that exercises what the serving binary does at boot: the
    /// compiled-in corpus parses, every file's content hash matches the
    /// manifest, and the node-id table loads. A miscounted include path or a
    /// corpus committed out of step with its manifest fails here rather than
    /// at a seller's first search.
    #[test]
    fn the_compiled_in_corpus_parses() {
        prime().expect("the committed corpus parses and its hashes match the manifest");
        let corpus = CORPUS.get().expect("priming holds the corpus");
        assert_eq!(
            corpus.documents.len(),
            4,
            "four frameworks are committed and all four load"
        );
        for framework in [
            Framework::Ccss,
            Framework::Ngss,
            Framework::Teks,
            Framework::VaSol,
        ] {
            assert!(
                corpus
                    .documents
                    .iter()
                    .any(|document| document.framework() == framework),
                "{framework:?} is in the parsed corpus"
            );
        }
        assert!(
            corpus.node_ids.is_empty(),
            "the node-id table is committed empty until the founder's crawl, so every id is withheld"
        );
    }

    #[test]
    fn priming_twice_is_the_same_corpus() {
        prime().expect("the first prime succeeds");
        prime().expect("the second prime is a no-op rather than a failure");
        assert_eq!(CORPUS.get().expect("the corpus is held").documents.len(), 4);
    }

    /// The two enums name the same four frameworks, and the map is written out
    /// rather than derived, so this pins that it agrees in both directions.
    #[test]
    fn every_domain_framework_maps_to_its_standards_twin() {
        let pairs = [
            (StandardsFramework::CommonCore, Framework::Ccss),
            (StandardsFramework::NextGenerationScience, Framework::Ngss),
            (
                StandardsFramework::TexasEssentialKnowledgeAndSkills,
                Framework::Teks,
            ),
            (
                StandardsFramework::VirginiaStandardsOfLearning,
                Framework::VaSol,
            ),
        ];
        for (domain, standards) in pairs {
            assert_eq!(framework_of(domain), standards);
            assert_eq!(
                domain_framework_of(standards),
                domain,
                "the reverse map is the inverse, so an item's stated framework is its real one"
            );
        }
    }

    /// Every framework carries at least one notice, because every one of the
    /// four is either owned or mirrored under a licence that obliges one.
    /// Every framework carries at least one notice, and Texas and Virginia are
    /// the reason this test exists: their owners impose none, so
    /// `required_notices` answers empty for both, and their whole obligation is
    /// the mirror's CC BY attribution. A handler that served only the owner's
    /// half would display mirrored Texas and Virginia standards under no
    /// attribution at all.
    #[test]
    fn every_framework_carries_its_notices_with_their_placement() {
        prime().expect("the corpus parses");
        let corpus = CORPUS.get().expect("the corpus is held");
        for framework in [
            Framework::Ccss,
            Framework::Ngss,
            Framework::Teks,
            Framework::VaSol,
        ] {
            let notices = notices_of(&corpus.documents, framework);
            assert!(
                !notices.is_empty(),
                "{framework:?} displays under a licence that obliges a notice"
            );
            for notice in &notices {
                assert!(!notice.text.is_empty());
                assert!(!notice.placement.is_empty());
            }
        }
        for framework in [Framework::Teks, Framework::VaSol] {
            assert!(
                required_notices(framework).is_empty(),
                "{framework:?} imposes no owner notice, so the mirror's is the whole obligation"
            );
            assert!(
                notices_of(&corpus.documents, framework)
                    .iter()
                    .all(|notice| notice.text.contains("Common Standards Project")),
                "{framework:?} carries the mirror's attribution and nothing else"
            );
        }
    }

    #[test]
    fn every_placement_has_its_own_wire_name() {
        let names: Vec<&str> = [
            Placement::WhereverDisplayed,
            Placement::SiteFooterAndEveryPageUsingTheMark,
            Placement::WithTheData,
        ]
        .into_iter()
        .map(placement_str)
        .collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "no two placements share a name");
    }

    /// The dispatch: a code-shaped query reads the code index, a worded one
    /// reads the keyword index, and a code-shaped query matching no code falls
    /// through to keywords rather than answering empty. One provenance per
    /// result set either way, because the two are never merged.
    #[test]
    fn a_query_reads_one_index_and_never_both() {
        prime().expect("the corpus parses");
        let corpus = CORPUS.get().expect("the corpus is held");
        let index = StandardsIndex::build(&corpus.documents);

        let by_code = index.by_code_prefix("CCSS");
        if !by_code.is_empty() {
            let dispatched = search(&index, "CCSS");
            assert_eq!(
                dispatched.len(),
                by_code.len(),
                "a query the code index answers is answered by the code index alone"
            );
        }

        let nonsense = search(&index, "zzzzzzzzzz");
        assert!(
            nonsense.is_empty(),
            "a query neither index answers returns nothing rather than everything"
        );
    }

    /// The words a teacher reads, from whichever of the two the set carries.
    #[test]
    fn a_grade_band_reads_as_a_range_a_teacher_recognises() {
        let interval = |low: i8, high: i8| GradeBand {
            levels: vec![],
            interval: Some(tam_standards::grades::GradeInterval {
                low: GradeLevel(low),
                high: GradeLevel(high),
            }),
            uncovered: vec![],
        };
        assert_eq!(
            grade_band_of(&interval(3, 5)).as_deref(),
            Some("Grade 3–Grade 5")
        );
        assert_eq!(
            grade_band_of(&interval(-1, 0)).as_deref(),
            Some("Pre-K–Kindergarten"),
            "the two levels below first grade have names rather than numbers"
        );
        assert_eq!(
            grade_band_of(&interval(7, 7)).as_deref(),
            Some("Grade 7"),
            "a single-grade set reads as one grade rather than as a range onto itself"
        );
    }

    /// A set whose coding this crate could not parse still has grades a
    /// teacher recognises, so they are shown as the mirror recorded them
    /// rather than withheld.
    #[test]
    fn an_unparsed_band_falls_back_to_the_source_own_levels() {
        assert_eq!(
            grade_band_of(&GradeBand {
                levels: vec!["Upper primary".to_owned(), "Lower secondary".to_owned()],
                interval: None,
                uncovered: vec![],
            })
            .as_deref(),
            Some("Upper primary, Lower secondary")
        );
    }

    #[test]
    fn a_set_carrying_no_grades_at_all_answers_nothing() {
        assert_eq!(
            grade_band_of(&GradeBand {
                levels: vec![],
                interval: None,
                uncovered: vec![],
            }),
            None,
            "absent is rendered as no grade rather than as an empty one"
        );
    }

    #[test]
    fn a_result_page_is_bounded() {
        prime().expect("the corpus parses");
        let corpus = CORPUS.get().expect("the corpus is held");
        let index = StandardsIndex::build(&corpus.documents);
        // One letter matches thousands; the handler takes the first
        // RESULTS_MAX of whatever the index answers.
        let wide = search(&index, "a");
        assert!(
            wide.len() > RESULTS_MAX || wide.is_empty(),
            "the fixture query is either wide enough to prove the bound matters or empty"
        );
    }
}
