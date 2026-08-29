//! The Tes licence vocabulary as canonical terms, so an elected rights grant
//! is a relation member rather than a string the adapter reinvents.
//!
//! Licence is the one axis the projection may never settle on the seller's
//! behalf, which is exactly why it has to be *seedable*: an election offers
//! candidates, and a candidate is a `VocabularyPath` carrying the target
//! vocabulary's own label and wire token. Without these terms the decision
//! surface would have nothing to offer and the seven-value vocabulary would
//! only exist inside an adapter's enum.
//!
//! The path's segment is the token rather than the title, because the two
//! legacy rows share the title "Legacy Licence" and a shared segment is two
//! terms claiming one path, which `projection_edge_exact_reverse` refuses. The
//! title is the canonical term's label, where a collision costs nothing.

use std::collections::BTreeMap;

use serde::Deserialize;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_types::{InventoryId, Timestamp};

use crate::grades::{derived, OptionSet};

/// The inventories that bind a licence axis. TPT binds none anywhere on its
/// wire, which is the legal exemplar and is declared in the registry rather
/// than restated here.
const LICENCE_INVENTORIES: [InventoryId; 3] =
    [InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TesLicenceVocabulary {
    #[serde(rename = "_source")]
    pub source: String,
    pub licences: OptionSet<Licence>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Licence {
    pub title: String,
    pub paid: bool,
    pub publishable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenceCrosswalk {
    pub terms: Vec<CanonicalTerm>,
    pub edges: Vec<ProjectionEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenceError {
    pub detail: String,
}

impl core::fmt::Display for LicenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "tes-vocabulary.json does not carry the expected licence shape: {}",
            self.detail
        )
    }
}

impl core::error::Error for LicenceError {}

/// Mints the seven Tes licence tokens as canonical terms with identity edges
/// into each Tes inventory's licence vocabulary.
///
/// Identity rather than a crosswalk: the three inventories share one licence
/// vocabulary, so every edge is `Exact` and the projection between them is
/// lossless. That is the wedge's third row — a GB resource duplicated into New
/// Zealand carries the seller's own grant unchanged — and it holds only
/// because the terms exist.
pub fn derive_licence_crosswalk(
    tes_json: &str,
    decided_at: Timestamp,
) -> Result<LicenceCrosswalk, LicenceError> {
    let vocabulary: TesLicenceVocabulary =
        serde_json::from_str(tes_json).map_err(|error| LicenceError {
            detail: error.to_string(),
        })?;
    let decided_by = Decider::Imported {
        source: vocabulary.source.clone(),
    };
    let mut out = LicenceCrosswalk {
        terms: Vec::new(),
        edges: Vec::new(),
    };
    for (token, licence) in &vocabulary.licences.options {
        let term = licence_term_id(token);
        out.terms.push(CanonicalTerm {
            id: term,
            kind: TermKind::Licence,
            parent: None,
            label: licence.title.clone(),
        });
        for inventory in LICENCE_INVENTORIES {
            out.edges.push(ProjectionEdge {
                from: term,
                to: licence_path(inventory, token),
                kind: EdgeKind::Exact,
                decided_by: decided_by.clone(),
                decided_at,
            });
        }
    }
    Ok(out)
}

/// Which licence tokens a product may be published under on each side of the
/// price gate, read off the vocabulary rather than restated.
///
/// The election surface offers these; the seven-row table is not the candidate
/// list, because two rows are read-back-only and two more are refused on the
/// free branch.
pub fn publishable(tes_json: &str, paid: bool) -> Result<BTreeMap<String, Licence>, LicenceError> {
    let vocabulary: TesLicenceVocabulary =
        serde_json::from_str(tes_json).map_err(|error| LicenceError {
            detail: error.to_string(),
        })?;
    Ok(vocabulary
        .licences
        .options
        .into_iter()
        .filter(|(_, licence)| licence.publishable && licence.paid == paid)
        .collect())
}

pub fn licence_path(inventory: InventoryId, token: &str) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(inventory, TermKind::Licence),
        segments: vec![token.to_owned()],
        native_id: Some(token.to_owned()),
    }
}

pub fn licence_term_id(token: &str) -> tam_types::CanonicalTermId {
    derived(&format!("tes-licence:{token}"))
}

#[cfg(test)]
mod tests {
    use super::{derive_licence_crosswalk, licence_term_id, publishable};
    use tam_domain::registry::{registry, NativeVocabulary};
    use tam_domain::{TermKind, VocabularyId};
    use tam_types::{InventoryId, Timestamp};

    const TES: &str = include_str!("../../../docs/design/data/tes-vocabulary.json");
    const AT: Timestamp = Timestamp(1_700_000_000_000);

    #[test]
    fn the_seeded_tokens_are_the_registry_vocabulary() {
        let crosswalk = derive_licence_crosswalk(TES, AT).expect("the committed capture derives");
        let Some(NativeVocabulary::Closed(declared)) = registry(InventoryId::TesGb)
            .native("licence")
            .map(|field| field.vocabulary)
        else {
            panic!("the Tes licence vocabulary is captured closed");
        };
        let mut seeded: Vec<&str> = crosswalk
            .edges
            .iter()
            .filter(|edge| {
                edge.to.vocabulary == VocabularyId(InventoryId::TesGb, TermKind::Licence)
            })
            .filter_map(|edge| edge.to.native_id.as_deref())
            .collect();
        seeded.sort_unstable();
        let mut expected: Vec<&str> = declared.to_vec();
        expected.sort_unstable();
        assert_eq!(
            seeded, expected,
            "the seeded relation and the registry restate one polled vocabulary and must agree"
        );
    }

    #[test]
    fn every_token_holds_one_identity_edge_into_each_tes_inventory() {
        let crosswalk = derive_licence_crosswalk(TES, AT).expect("the committed capture derives");
        assert_eq!(crosswalk.terms.len(), 7);
        assert_eq!(crosswalk.edges.len(), 21);
        for inventory in [InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz] {
            let term = licence_term_id("CC-BY-ND");
            assert!(
                crosswalk.edges.iter().any(|edge| edge.from == term
                    && edge.to.vocabulary == VocabularyId(inventory, TermKind::Licence)),
                "a GB grant reaches {inventory:?} unchanged, which is the wedge's third row"
            );
        }
    }

    #[test]
    fn the_two_legacy_rows_share_a_title_and_not_a_path() {
        let crosswalk = derive_licence_crosswalk(TES, AT).expect("the committed capture derives");
        let legacy: Vec<&str> = crosswalk
            .terms
            .iter()
            .filter(|term| term.label == "Legacy Licence")
            .map(|term| term.label.as_str())
            .collect();
        assert_eq!(legacy.len(), 2, "TES-V1 and TES-V2 share one title");
        let mut paths: Vec<&[String]> = crosswalk
            .edges
            .iter()
            .filter(|edge| {
                edge.to.vocabulary == VocabularyId(InventoryId::TesGb, TermKind::Licence)
            })
            .map(|edge| edge.to.segments.as_slice())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(
            paths.len(),
            7,
            "seven distinct paths, or the reverse-uniqueness index rejects the seed"
        );
    }

    #[test]
    fn the_free_branch_offers_three_tokens_and_the_paid_branch_two() {
        let free = publishable(TES, false).expect("the committed capture derives");
        let paid = publishable(TES, true).expect("the committed capture derives");
        assert_eq!(
            free.keys().collect::<Vec<_>>(),
            vec!["CC-BY", "CC-BY-ND", "CC-BY-SA"],
            "the legacy pair reads back and is never offered on write"
        );
        assert_eq!(
            paid.keys().collect::<Vec<_>>(),
            vec!["TES-PAID", "TES-PAID-SCHOOL"]
        );
    }
}
