//! Education standards for one jurisdiction, or the honest absence.
//!
//! `not_ingested` is a state rather than an empty result, because the two mean
//! different things to a seller: no standard matched their words, against no
//! standard exists here yet. The four frameworks' catalogues are a separate
//! stream — only Texas publishes a machine-readable feed of its own, three of
//! the four come from a mirror, and the code-to-node-id table TPT needs is its
//! own crawl — so this answers `not_ingested` for every framework today and
//! says so on the wire rather than rendering an empty tree.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_domain::product::StandardsFramework;

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// What a standards search found, or why it found nothing.
///
/// `not_ingested` is a state rather than an empty result, because the two mean
/// different things to a seller: no standard matched their words, against no
/// standard exists here yet. The tree is order 11,000 addressable codes across
/// four frameworks and its ingestion is its own stream, so this is the honest
/// answer until that lands.
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
    /// TPT's own node id, which is what a create posts. Absent for a standard
    /// we can display and cannot yet post: the id is a search-index identifier
    /// and is exactly the kind that gets rebuilt, so an unverified one is
    /// withheld rather than guessed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpt_node_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandardsSearchView {
    pub state: StandardsState,
    pub framework: u32,
    pub items: Vec<StandardView>,
    /// The attribution this framework's licence obliges us to carry wherever
    /// one of its standards is displayed. Served with the results rather than
    /// hard-coded in the client, so the obligation travels with the data it
    /// attaches to.
    pub attribution: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StandardsSearchParams {
    /// A jurisdiction id: 3054, 3055, 3326 or 5785.
    pub framework: u32,
    #[serde(default)]
    pub q: Option<String>,
}

/// The notice each framework's licence obliges a display to carry.
const fn attribution(framework: StandardsFramework) -> &'static str {
    match framework {
        StandardsFramework::CommonCore => {
            "© Copyright 2010. National Governors Association Center for Best Practices and \
             Council of Chief State School Officers. All rights reserved."
        }
        StandardsFramework::NextGenerationScience => {
            "NGSS® is a registered trademark of Achieve. Neither Achieve nor the lead states \
             and partners that developed the Next Generation Science Standards was involved \
             in the production of, and does not endorse, this product."
        }
        StandardsFramework::TexasEssentialKnowledgeAndSkills
        | StandardsFramework::VirginiaStandardsOfLearning => {
            "Standards data from the Common Standards Project, used under CC BY."
        }
    }
}

/// Stubbed deliberately, and it says so on the wire.
///
/// The four frameworks' ingestion is a separate stream: no owner but Texas
/// publishes a machine-readable feed, three of the four come from a mirror,
/// and the code-to-node-id table TPT needs is its own crawl. Until that lands
/// there is nothing to search, and `not_ingested` is the difference between
/// "nothing matched" and "nothing is here yet".
pub(crate) async fn standards_search(
    State(_state): State<AppState>,
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
    Ok(Json(StandardsSearchView {
        state: StandardsState::NotIngested,
        framework: framework.jurisdiction_id(),
        items: vec![],
        attribution: attribution(framework).to_owned(),
    }))
}
