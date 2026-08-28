//! The machine seed produced from the projection: the worker cannot lease
//! what it cannot project, so this is the gate between the ledger and the
//! marketplace. A blocked projection raises its queue items and parks the
//! item rather than settling it, because a drained queue un-parks it into a
//! clean retry — the treadmill inverted into the drain's own retry loop.
//!
//! Two halves, deliberately separate. [`project_for_item`] is the ledger's:
//! it reads, projects, and parks, and it needs no adapter, so an item that
//! cannot project never costs a gateway session. [`seed_from_projection`] is
//! the marketplace's: the adapter renders its own wire shape, so no
//! platform's encoding lives here.

use sqlx::PgPool;
use tam_domain::{ProjectionBlocked, StepBudget, TermKind, VocabularyId, VocabularyPath};
use tam_marketplace::{
    AgeSpan, CreateStrategy, FormId, MarketplaceAdapter, NativeTerm, ProjectedListing,
    RemoteLifecycleKind,
};
use tam_storage::{
    LeasedItem, MappingRepo, ProductRepo, RaiseReport, RaiseScope, StorageError, TaxonomyRepo,
};
use tam_taxonomy::listing::{project_listing, ListingContext};
use tam_types::Timestamp;

use crate::driver::{EngineError, MachineSeed};

/// Per-item action ceiling; generous against the longest measured flow
/// (create, metadata, three-step file upload per file, read-back).
const ACTIONS_PER_ITEM: u32 = 32;

pub enum ProjectionOutcome {
    Ready(ProjectedListing),
    /// The projection refused; the gaps (when taxonomy) are raised already.
    Blocked {
        gate: &'static str,
        raised: RaiseReport,
    },
}

pub async fn project_for_item(
    pool: &PgPool,
    lease: &LeasedItem,
    now: Timestamp,
) -> Result<ProjectionOutcome, EngineError> {
    let mapping = MappingRepo::new(pool.clone())
        .get(lease.org, lease.mapping)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "a leased item's mapping must exist".to_owned(),
        })?;
    let product = ProductRepo::new(pool.clone())
        .get(lease.org, mapping.mapping.product)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "a mapped product must exist".to_owned(),
        })?
        .product;

    let taxonomy = TaxonomyRepo::new(pool.clone());
    let terms = taxonomy.terms().await?;
    let mut edges = taxonomy
        .edges_into(VocabularyId(lease.inventory, TermKind::Subject))
        .await?;
    edges.extend(
        taxonomy
            .edges_into(VocabularyId(lease.inventory, TermKind::Topic))
            .await?,
    );
    let no_counterparts = taxonomy.no_counterparts_into(lease.inventory).await?;

    let projection = match project_listing(
        &product,
        &ListingContext {
            org: lease.org,
            mapping: lease.mapping,
            inventory: lease.inventory,
            now,
            terms: &terms,
            edges: &edges,
            no_counterparts: &no_counterparts,
        },
    ) {
        Ok(projection) => projection,
        Err(ProjectionBlocked::Taxonomy { items }) => {
            let causes: Vec<_> = items
                .iter()
                .map(|item| {
                    let VocabularyId(_, kind) = item.target;
                    (item.term, kind)
                })
                .collect();
            let raised = taxonomy
                .raise(
                    lease.org,
                    RaiseScope {
                        mapping: lease.mapping,
                        target: lease.inventory,
                        at: now,
                    },
                    &causes,
                )
                .await?;
            return Ok(ProjectionOutcome::Blocked {
                gate: "reconciliation",
                raised,
            });
        }
        Err(ProjectionBlocked::CurrencyUnknown { .. }) => {
            return Ok(ProjectionOutcome::Blocked {
                gate: "currency_unknown",
                raised: RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            });
        }
        Err(ProjectionBlocked::CoverMissing) => {
            return Ok(ProjectionOutcome::Blocked {
                gate: "cover_missing",
                raised: RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            });
        }
        Err(ProjectionBlocked::ScanIncomplete { .. }) => {
            return Ok(ProjectionOutcome::Blocked {
                gate: "scan_incomplete",
                raised: RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            });
        }
    };

    Ok(ProjectionOutcome::Ready(ProjectedListing {
        title: projection.title,
        body: projection.body,
        price: projection.price,
        taxonomy: projection.taxonomy.iter().map(native_term).collect(),
        grades: projection.grades.iter().map(native_term).collect(),
        ages: product.grades.derived.map(|interval| AgeSpan {
            low_years: interval.low_years(),
            high_years: interval.high_years(),
        }),
        files: projection.files,
    }))
}

fn native_term(path: &VocabularyPath) -> NativeTerm {
    NativeTerm {
        native_id: path.native_id.clone(),
        segments: path.segments.clone(),
    }
}

/// The adapter half: it renders the field set, and the intent hash is taken
/// over what it rendered, so the recorded intent is the bytes the submit will
/// carry rather than a shape the engine guessed at.
pub fn seed_from_projection<A: MarketplaceAdapter>(
    adapter: &A,
    lease: &LeasedItem,
    listing: &ProjectedListing,
) -> Result<MachineSeed, EngineError> {
    let fields = adapter.project_fields(listing)?;
    let intent_hash = tam_pipeline::hash::content_hash(
        serde_json::json!({
            "entries": fields
                .entries
                .iter()
                .map(|(key, value)| (format!("{key:?}"), value))
                .collect::<Vec<_>>(),
            "files": fields
                .files
                .iter()
                .map(|file| file.0.to_hyphenated())
                .collect::<Vec<_>>(),
        })
        .to_string()
        .as_bytes(),
    );

    // One form identity per inventory, deterministic: byte one is the
    // idempotency module's durable inventory ordinal reused as a tag.
    let form_tag = match lease.inventory {
        tam_types::InventoryId::TesGb => 0x01,
        tam_types::InventoryId::TesUs => 0x02,
        tam_types::InventoryId::TesNz => 0x03,
        tam_types::InventoryId::Etsy => 0x04,
        tam_types::InventoryId::Tpt => 0x05,
    };
    Ok(MachineSeed {
        form: FormId(tam_types::Uuid([form_tag; 16])),
        fields,
        intent_hash,
        strategy: CreateStrategy::DraftThenPublish {
            draft_state: RemoteLifecycleKind::Draft,
        },
        budget: StepBudget {
            actions_remaining: ACTIONS_PER_ITEM,
        },
    })
}
