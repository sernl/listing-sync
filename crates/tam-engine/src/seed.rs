//! The machine seed produced from the projection: the worker cannot lease
//! what it cannot project, so this is the gate between the ledger and the
//! marketplace. A blocked projection raises its queue items and parks the
//! item rather than settling it, because a drained queue un-parks it into a
//! clean retry — the treadmill inverted into the drain's own retry loop.

use sqlx::PgPool;
use tam_domain::{ProjectionBlocked, StepBudget, TermKind, VocabularyId};
use tam_marketplace::{CreateStrategy, FieldSet, FormId, RemoteLifecycleKind};
use tam_storage::{
    LeasedItem, MappingRepo, ProductRepo, RaiseReport, RaiseScope, StorageError, TaxonomyRepo,
};
use tam_taxonomy::listing::{project_listing, ListingContext};
use tam_types::{FieldKey, PriceIntent, Timestamp};

use crate::driver::{EngineError, MachineSeed};

/// Per-item action ceiling; generous against the longest measured flow
/// (create, metadata, three-step file upload per file, read-back).
const ACTIONS_PER_ITEM: u32 = 32;

pub enum SeedOutcome {
    Ready(MachineSeed),
    /// The projection refused; the gaps (when taxonomy) are raised already.
    Blocked {
        gate: &'static str,
        raised: RaiseReport,
    },
}

pub async fn seed_for_item(
    pool: &PgPool,
    lease: &LeasedItem,
    now: Timestamp,
) -> Result<SeedOutcome, EngineError> {
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
            return Ok(SeedOutcome::Blocked {
                gate: "reconciliation",
                raised,
            });
        }
        Err(ProjectionBlocked::CurrencyUnknown { .. }) => {
            return Ok(SeedOutcome::Blocked {
                gate: "currency_unknown",
                raised: RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            });
        }
        Err(ProjectionBlocked::CoverMissing) => {
            return Ok(SeedOutcome::Blocked {
                gate: "cover_missing",
                raised: RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            });
        }
        Err(ProjectionBlocked::ScanIncomplete { .. }) => {
            return Ok(SeedOutcome::Blocked {
                gate: "scan_incomplete",
                raised: RaiseReport {
                    new: 0,
                    already_open: 0,
                },
            });
        }
    };

    let licence = match projection.price {
        PriceIntent::Free => "CC-BY".to_owned(),
        // The Tes write path's licence vocabulary is Creative Commons only
        // so far; a paid seed reaches the adapter's own closed refusal and
        // settles honestly rather than being silently freed here.
        PriceIntent::Paid(_) => "TES-PAID".to_owned(),
    };
    let mut categories: Vec<i64> = Vec::new();
    for path in &projection.taxonomy {
        let native = path
            .native_id
            .as_deref()
            .ok_or(StorageError::Inconsistent {
                reason: "a Tes taxonomy path must carry its numeric id".to_owned(),
            })?;
        categories.push(native.parse().map_err(|_| StorageError::Inconsistent {
            reason: format!("non-numeric Tes category id {native:?}"),
        })?);
    }
    let mut age_ranges: Vec<i64> = Vec::new();
    for path in &projection.grades {
        if let Some(native) = path.native_id.as_deref() {
            if let Ok(id) = native.parse() {
                age_ranges.push(id);
            }
        }
    }
    let (ages, main_age): (Vec<i64>, i64) = match product.grades.derived {
        Some(interval) => (
            (i64::from(interval.low_years())..=i64::from(interval.high_years())).collect(),
            i64::from(interval.low_years()),
        ),
        None => (Vec::new(), 0),
    };

    let entries = vec![
        (FieldKey::Title, projection.title.clone()),
        (FieldKey::Description, projection.body.clone()),
        (FieldKey::Price, licence),
        (
            FieldKey::Taxonomy,
            serde_json::json!({ "categories": categories, "mainType": 0 }).to_string(),
        ),
        (
            FieldKey::Grades,
            serde_json::json!({
                "ageRanges": age_ranges,
                "ages": ages,
                "mainAge": main_age
            })
            .to_string(),
        ),
    ];
    let fields = FieldSet {
        entries,
        files: projection.files.clone(),
    };
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
    Ok(SeedOutcome::Ready(MachineSeed {
        form: FormId(tam_types::Uuid([form_tag; 16])),
        fields,
        intent_hash,
        strategy: CreateStrategy::DraftThenPublish {
            draft_state: RemoteLifecycleKind::Draft,
        },
        budget: StepBudget {
            actions_remaining: ACTIONS_PER_ITEM,
        },
    }))
}
