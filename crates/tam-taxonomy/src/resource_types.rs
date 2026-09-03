//! The resource-type axis, bound in the registry since the seam landed and
//! unrouted until now because its crosswalk was unseeded.
//!
//! Seventy-one TPT `Type-of-Resource` facets against nine writable Tes
//! `mainType` values. Seventy-one onto nine can only be many-to-one, which
//! `docs/design/taxonomy-projection.md:36-37` makes legal through `Broader`
//! and illegal through `Exact`, so every facet mints its own canonical term
//! holding an `Exact` edge into TPT and a `Broader` edge into each of the
//! three Tes inventories. Nothing here is `Exact` on the Tes side, and that
//! is the shape of the vocabularies rather than a hedge: Tes offers one value
//! where TPT offers eight.
//!
//! Tes takes the axis at `Cardinality::One`
//! (`crates/tam-domain/src/registry/tes.rs:203-238`), so a product carrying
//! several resource-type facets resolves several Tes values and overflows the
//! cap. That is an election with the whole resolved set as candidates rather
//! than a truncation, which `crates/tam-taxonomy/src/project.rs` already
//! implements; this module only supplies the relation that makes the overflow
//! reachable.
//!
//! The authored half is `docs/design/data/tpt-tes-resource-type-pairs.json`,
//! and it is twenty-one rows rather than seventy-one because a child inherits
//! its root's value unless it is named. Eleven are named, each because
//! inheriting would file it under a value Tes distinguishes from the right
//! one: a game is not a worksheet and a unit of work is not `Other`.
//!
//! The hidden facet `independent-work` seeds neither term nor edge, on the
//! same rule a hidden subject facet follows: it is never offered on create,
//! so an `Exact` edge would post a retired slug, and a term reachable from
//! neither direction would deliver nothing.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use tam_domain::{
    CanonicalTerm, Decider, EdgeKind, ProjectionEdge, TermKind, VocabularyId, VocabularyPath,
};
use tam_types::{CanonicalTermId, InventoryId, Timestamp};

use crate::grades::{derived, TaxonomyTag, TptVocabulary};
use crate::tes::ResidueNode;

const RESOURCE_CATEGORY: &str = "Type-of-Resource";

/// The three inventories whose registries bind `mainType`.
const TES_INVENTORIES: [InventoryId; 3] =
    [InventoryId::TesGb, InventoryId::TesUs, InventoryId::TesNz];

/// The resource-type relation as the captures and the authored pairing state
/// it. No `no_counterparts`: a Tes value no facet reaches is a gap in the
/// other direction, and this derivation seeds nothing into Tes's own
/// vocabulary that could claim otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceTypeCrosswalk {
    pub terms: Vec<CanonicalTerm>,
    pub edges: Vec<ProjectionEdge>,
    /// Facets that seed a term and reach no Tes value.
    pub unreached: Vec<ResidueNode>,
    /// Hidden facets, which seed neither term nor edge.
    pub skipped_hidden: Vec<ResidueNode>,
    /// Tes values no facet reaches, which is a real absence rather than a
    /// defect: Tes carries `Assembly` and TPT has no facet for it.
    pub unclaimed_targets: Vec<ResidueNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceTypeError {
    Parse {
        file: &'static str,
        detail: String,
    },
    UnknownFacet {
        slug: String,
    },
    /// The pairing names a `mainType` value the capture does not hold, or one
    /// the uploader's own select filters out.
    UnknownTesValue {
        id: u64,
    },
    HiddenFacetPaired {
        slug: String,
    },
    MalformedRow {
        slug: String,
        detail: &'static str,
    },
}

impl core::fmt::Display for ResourceTypeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse { file, detail } => write!(f, "{file} is not the expected shape: {detail}"),
            Self::UnknownFacet { slug } => write!(
                f,
                "the pairing names {slug}, which is no {RESOURCE_CATEGORY} facet"
            ),
            Self::UnknownTesValue { id } => write!(
                f,
                "the pairing names mainType {id}, which the writable capture does not hold"
            ),
            Self::HiddenFacetPaired { slug } => write!(
                f,
                "{slug} is hidden and therefore holds no edge, so it may not be paired"
            ),
            Self::MalformedRow { slug, detail } => write!(f, "the row for {slug} {detail}"),
        }
    }
}

impl core::error::Error for ResourceTypeError {}

/// Derives the resource-type relation from the TPT vocabulary capture, the
/// Tes vocabulary capture and the authored pairing. Pure: the caller does the
/// I/O.
pub fn derive_resource_type_crosswalk(
    tpt_json: &str,
    tes_json: &str,
    pairs_json: &str,
    decided_at: Timestamp,
) -> Result<ResourceTypeCrosswalk, ResourceTypeError> {
    let tpt: TptVocabulary =
        serde_json::from_str(tpt_json).map_err(|error| ResourceTypeError::Parse {
            file: "tpt-vocabulary.json",
            detail: error.to_string(),
        })?;
    let tes: TesResourceVocabulary =
        serde_json::from_str(tes_json).map_err(|error| ResourceTypeError::Parse {
            file: "tes-vocabulary.json",
            detail: error.to_string(),
        })?;
    let file: PairFile =
        serde_json::from_str(pairs_json).map_err(|error| ResourceTypeError::Parse {
            file: "tpt-tes-resource-type-pairs.json",
            detail: error.to_string(),
        })?;

    let facets = resource_facets(&tpt.taxonomy_tags.options);
    let mut values: BTreeMap<u64, String> = BTreeMap::new();
    for (id, label) in &tes.resource_types.options {
        let parsed = id.parse().map_err(|_| ResourceTypeError::Parse {
            file: "tes-vocabulary.json",
            detail: format!("resourceTypes carries a non-numeric id {id}"),
        })?;
        values.insert(parsed, label.clone());
    }

    let mut stated: BTreeMap<String, Option<u64>> = BTreeMap::new();
    for row in &file.pairs {
        let facet = facets
            .get(&row.tpt)
            .ok_or_else(|| ResourceTypeError::UnknownFacet {
                slug: row.tpt.clone(),
            })?;
        let target = match row.tes.as_slice() {
            [] => None,
            [only] => Some(only.id),
            _ => {
                return Err(ResourceTypeError::MalformedRow {
                    slug: row.tpt.clone(),
                    detail: "names more than one Tes value, and mainType takes one",
                })
            }
        };
        match (target, facet.hidden) {
            (Some(_), true) => {
                return Err(ResourceTypeError::HiddenFacetPaired {
                    slug: row.tpt.clone(),
                })
            }
            (Some(id), false) if !values.contains_key(&id) => {
                return Err(ResourceTypeError::UnknownTesValue { id })
            }
            _ => {}
        }
        if stated.insert(row.tpt.clone(), target).is_some() {
            return Err(ResourceTypeError::MalformedRow {
                slug: row.tpt.clone(),
                detail: "appears twice",
            });
        }
    }
    Ok(emit(&facets, &values, &stated, decided_at, &tpt.source))
}

fn emit(
    facets: &BTreeMap<String, Facet>,
    values: &BTreeMap<u64, String>,
    stated: &BTreeMap<String, Option<u64>>,
    decided_at: Timestamp,
    tpt_source: &str,
) -> ResourceTypeCrosswalk {
    let decided_by = Decider::Imported {
        source: tpt_source.to_owned(),
    };
    let mut out = ResourceTypeCrosswalk::default();
    let mut reached: BTreeSet<u64> = BTreeSet::new();
    let mut roots = Vec::new();
    let mut children = Vec::new();

    for (slug, facet) in facets {
        if facet.hidden {
            out.skipped_hidden.push(facet_residue(facet));
            continue;
        }
        let term = CanonicalTerm {
            id: resource_term_id(slug),
            kind: TermKind::ResourceType,
            parent: facet.parent.as_ref().map(|root| resource_term_id(root)),
            label: facet.name.clone(),
        };
        if facet.parent.is_some() {
            children.push(term);
        } else {
            roots.push(term);
        }
    }
    out.terms.extend(roots);
    out.terms.extend(children);

    for (slug, facet) in facets {
        if facet.hidden {
            continue;
        }
        let id = resource_term_id(slug);
        out.edges.push(ProjectionEdge {
            from: id,
            to: tpt_path(facet, facets),
            kind: EdgeKind::Exact,
            decided_by: decided_by.clone(),
            decided_at,
        });
        // A child not named in the pairing takes its root's value, which is
        // what keeps the authored file at twenty-one rows rather than
        // seventy-one.
        let target = stated
            .get(slug)
            .copied()
            .or_else(|| {
                facet
                    .parent
                    .as_ref()
                    .and_then(|root| stated.get(root).copied())
            })
            .flatten();
        let Some(value) = target else {
            out.unreached.push(facet_residue(facet));
            continue;
        };
        let Some(label) = values.get(&value) else {
            out.unreached.push(facet_residue(facet));
            continue;
        };
        reached.insert(value);
        for inventory in TES_INVENTORIES {
            out.edges.push(ProjectionEdge {
                from: id,
                to: VocabularyPath {
                    vocabulary: VocabularyId(inventory, TermKind::ResourceType),
                    segments: vec![label.clone()],
                    native_id: Some(value.to_string()),
                },
                kind: EdgeKind::Broader,
                decided_by: decided_by.clone(),
                decided_at,
            });
        }
    }

    for (&value, label) in values {
        if !reached.contains(&value) {
            out.unclaimed_targets.push(ResidueNode {
                market: InventoryId::TesGb,
                kind: TermKind::ResourceType,
                native_id: value.to_string(),
                description: label.clone(),
            });
        }
    }
    out
}

#[derive(Debug, Clone, Deserialize)]
struct PairFile {
    pairs: Vec<PairRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct PairRow {
    tpt: String,
    tes: Vec<PairvalueId>,
}

#[derive(Debug, Clone, Deserialize)]
struct PairvalueId {
    id: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct TesResourceVocabulary {
    #[serde(rename = "resourceTypes")]
    resource_types: ResourceTypes,
}

#[derive(Debug, Clone, Deserialize)]
struct ResourceTypes {
    options: BTreeMap<String, String>,
}

struct Facet {
    slug: String,
    name: String,
    parent: Option<String>,
    hidden: bool,
}

fn resource_facets(tags: &BTreeMap<String, TaxonomyTag>) -> BTreeMap<String, Facet> {
    tags.iter()
        .filter(|(_, tag)| tag.category.as_deref() == Some(RESOURCE_CATEGORY))
        .map(|(slug, tag)| {
            (
                slug.clone(),
                Facet {
                    slug: slug.clone(),
                    name: tag.name.clone(),
                    parent: tag.parent_id.clone(),
                    hidden: tag.is_hidden.unwrap_or(false),
                },
            )
        })
        .collect()
}

fn tpt_path(facet: &Facet, facets: &BTreeMap<String, Facet>) -> VocabularyPath {
    let mut segments = Vec::new();
    if let Some(parent) = facet.parent.as_ref().and_then(|slug| facets.get(slug)) {
        segments.push(parent.name.clone());
    }
    segments.push(facet.name.clone());
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tpt, TermKind::ResourceType),
        segments,
        native_id: Some(facet.slug.clone()),
    }
}

fn facet_residue(facet: &Facet) -> ResidueNode {
    ResidueNode {
        market: InventoryId::Tpt,
        kind: TermKind::ResourceType,
        native_id: facet.slug.clone(),
        description: facet.name.clone(),
    }
}

/// Deterministic canonical ids in the scheme the other derivations use, so a
/// re-seed is an explicit no-op.
#[must_use]
pub fn resource_term_id(slug: &str) -> CanonicalTermId {
    derived(&format!("tpt-resource-type:{slug}"))
}

#[cfg(test)]
mod tests;
