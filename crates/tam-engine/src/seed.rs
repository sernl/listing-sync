//! The machine seed produced from the ledger and the projection: the worker
//! cannot lease what it cannot project, so this is the gate between the
//! ledger and the marketplace. A blocked preparation raises its queue items
//! and parks the item rather than settling it, because answering the question
//! un-parks it into a clean retry — the treadmill inverted into the drain's
//! own retry loop. The answer is what revives it; the day-long park expiry is
//! the backstop for an answer nobody gives.
//!
//! Two halves, deliberately separate. [`prepare_item`] is the ledger's: it
//! reads, checks the item's operation against the binding the mapping
//! actually holds, projects, and parks, and it needs no adapter, so an item
//! that cannot run never costs a gateway session. [`seed_from_projection`]
//! and [`seed_for_removal`] are the marketplace's: the adapter renders its
//! own wire shape, so no platform's encoding lives here.

use sqlx::PgPool;
use tam_domain::{
    Binding, ItemOperation, ProjectionBlocked, StepBudget, VocabularyId, VocabularyPath,
};
use tam_marketplace::{
    AgeSpan, CreateStrategy, FieldSet, FormId, LifecycleTransition, ListingState,
    MarketplaceAdapter, NativeAxis, NativeTerm, ProjectedListing, RemoteLifecycle,
    RemoteLifecycleKind,
};
use tam_storage::{
    ElectionRepo, LeasedItem, LossScope, MappingRepo, ProductRepo, RaiseReport, RaiseScope,
    StorageError, TaxonomyRepo,
};
use tam_taxonomy::listing::{project_listing, projection_vocabularies, ListingContext};
use tam_types::{AttemptId, InventoryId, OrgId, Timestamp, Uuid};

use crate::driver::{intent_as_json, EngineError, MachineSeed, VerifyPolicy};

/// Per-item action ceiling; generous against the longest measured flow
/// (create, metadata, three-step file upload per file, read-back).
const ACTIONS_PER_ITEM: u32 = 32;

/// The verification poll's budget, per inventory, sized to the slowest
/// convergence each platform was measured at: Tpt twenty seconds
/// (`crates/tam-marketplace-tpt/examples/live_write.rs`, 2026-08-28), Tes
/// sixteen (`crates/tam-marketplace-tes/examples/live_smoke.rs`,
/// 2026-08-29). Configuration in the engine's seed beside `ACTIONS_PER_ITEM`,
/// which is the existing precedent for a driver-shaped bound living here
/// rather than in `tam-limits`, where it would be a third bound on a resource
/// `WALL_CLOCK_MAX` and the lease TTL already bound.
#[must_use]
pub const fn verify_policy(inventory: InventoryId) -> VerifyPolicy {
    match inventory {
        // No adapter serves Etsy, so this policy is unexercised; it takes the
        // Tes numbers rather than one invented for a platform nothing has
        // measured.
        InventoryId::TesGb | InventoryId::TesUs | InventoryId::TesNz | InventoryId::Etsy => {
            VerifyPolicy {
                tries: 8,
                interval_ms: 2_000,
            }
        }
        InventoryId::Tpt => VerifyPolicy {
            tries: 11,
            interval_ms: 2_000,
        },
    }
}

/// One form identity per inventory, deterministic: byte one is the
/// idempotency module's durable inventory ordinal reused as a tag.
const fn form_id(inventory: InventoryId) -> FormId {
    let tag = match inventory {
        InventoryId::TesGb => 0x01,
        InventoryId::TesUs => 0x02,
        InventoryId::TesNz => 0x03,
        InventoryId::Etsy => 0x04,
        InventoryId::Tpt => 0x05,
    };
    FormId(tam_types::Uuid([tag; 16]))
}

/// What the ledger says an item is, once its operation has been checked
/// against the binding the mapping actually holds.
pub enum ItemPreparation {
    /// The item may run. `projected` is `None` for a removal, which describes
    /// nothing: routing a removal through `project_listing` would let a
    /// taxonomy gap, a missing cover or an unmeasured currency park a delete,
    /// and the listing is being taken down rather than described.
    Ready {
        operation: ItemOperation,
        projected: Option<ProjectedListing>,
    },
    /// The item may not run; the gaps (when taxonomy) are raised already.
    Blocked {
        gate: &'static str,
        raised: RaiseReport,
    },
    /// The item waits on a counterpart that will never bind, so waiting is
    /// over rather than merely unsatisfied. Settled `skipped` naming the
    /// inventory it waited on, because a migrate whose target create failed
    /// leaves the seller with a listing on both platforms and no statement
    /// that the migration ended.
    CounterpartLost { counterpart: InventoryId },
}

/// The gate's own park reason, which reaches the ledger and the job report.
pub const AWAITING_COUNTERPART: &str = "awaiting_counterpart";

/// What the counterpart's own mapping says about whether this item may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CounterpartState {
    Bound,
    /// Still in flight: unbound, severed, or a create that has not landed
    /// yet. Severed sits here rather than beside the terminal arm because a
    /// severed mapping is exactly what a fresh create is admitted against --
    /// `admission` says so -- so the paired create will land and bind it, and
    /// calling it unreachable settles the waiting item skipped on whichever
    /// of the two the lease order happened to pick first.
    Waiting,
    /// Settled somewhere it will not leave. An ambiguous create is here alone
    /// and deliberately: nobody knows what it did, so a removal on the
    /// strength of it would be a removal on the strength of a guess.
    Unreachable,
}

/// One `SELECT` against the sibling mapping. `mapping_one_per_inventory`
/// guarantees at most one row, and its absence is the honest wait: the
/// counterpart's mapping is minted by the same drain that mints this item's,
/// so a missing row means the pass has not reached it.
async fn counterpart_binding(
    pool: &PgPool,
    org: OrgId,
    product: tam_types::ProductId,
    inventory: InventoryId,
) -> Result<CounterpartState, EngineError> {
    let found = MappingRepo::new(pool.clone())
        .list_for_product(org, product)
        .await?
        .into_iter()
        .find(|record| record.mapping.inventory == inventory);
    Ok(match found.as_ref().map(|record| &record.mapping.binding) {
        Some(Binding::Bound { .. }) => CounterpartState::Bound,
        None | Some(Binding::Unbound | Binding::Creating { .. } | Binding::Severed { .. }) => {
            CounterpartState::Waiting
        }
        Some(Binding::AmbiguousCreate { .. }) => CounterpartState::Unreachable,
    })
}

fn blocked(gate: &'static str) -> ItemPreparation {
    ItemPreparation::Blocked {
        gate,
        raised: RaiseReport {
            new: 0,
            already_open: 0,
        },
    }
}

/// Which side of the draft line a stored lifecycle puts a listing on, where
/// it says anything at all. `Absent` — which every mapping written before the
/// bind recorded a lifecycle reads — and the moderation states no write
/// addresses say nothing an item's stated `from` can be checked against.
const fn observed_state(lifecycle: &RemoteLifecycle) -> Option<ListingState> {
    match lifecycle {
        RemoteLifecycle::Draft => Some(ListingState::Draft),
        RemoteLifecycle::Live { .. } => Some(ListingState::Live),
        RemoteLifecycle::Absent
        | RemoteLifecycle::Submitted { .. }
        | RemoteLifecycle::InReview { .. }
        | RemoteLifecycle::Rejected { .. }
        | RemoteLifecycle::Withdrawn { .. } => None,
    }
}

/// The gate, as a total function of the operation and the mapping. `None`
/// admits.
///
/// The item states the listing it means to act on and the mapping's binding
/// stays the authority for it: an unstated subject would let a revise
/// enqueued against listing X silently retarget to listing Y if the mapping
/// were rebound between enqueue and lease, and nothing would notice. Storing
/// it is what makes the divergence detectable; refusing on it is what keeps
/// the mapping authoritative.
fn admission(
    operation: &ItemOperation,
    binding: &Binding,
    lifecycle: &RemoteLifecycle,
) -> Option<&'static str> {
    let stated = match operation {
        // A create against a bound mapping makes a second listing, and the
        // duplicate refusal does not catch it: the ordinary trigger is the
        // seller editing the product and re-syncing, and a changed payload
        // mints a fresh idempotency key. Every other state admits it,
        // `Severed` included — `mapping_one_per_inventory` is unpredicated,
        // so a re-create has no row but that one to reuse, and the bind's
        // fence now reuses it.
        ItemOperation::Create => {
            return matches!(binding, Binding::Bound { .. }).then_some("binding");
        }
        // A publish states no subject, so there is nothing to diverge from
        // and nothing to compare a `from` against. What it does need is a
        // binding: the id it addresses is the one the create bound, and an
        // unbound mapping has none yet.
        ItemOperation::Publish { .. } => match binding {
            Binding::Bound { .. } => return None,
            Binding::Unbound
            | Binding::Creating { .. }
            | Binding::AmbiguousCreate { .. }
            | Binding::Severed { .. } => return Some("unbound"),
        },
        ItemOperation::Revise {
            subject,
            transition,
        } => match binding {
            Binding::Bound { id, .. } if id == subject => transition.from,
            Binding::Bound { .. } => return Some("subject_diverged"),
            Binding::Unbound
            | Binding::Creating { .. }
            | Binding::AmbiguousCreate { .. }
            | Binding::Severed { .. } => return Some("unbound"),
        },
        ItemOperation::Remove { subject, state } => match binding {
            Binding::Bound { id, .. } if id == subject => *state,
            Binding::Bound { .. } => return Some("subject_diverged"),
            Binding::Unbound
            | Binding::Creating { .. }
            | Binding::AmbiguousCreate { .. }
            | Binding::Severed { .. } => return Some("unbound"),
        },
    };
    // The bind writes a truthful lifecycle for everything this system
    // creates, so a mapping that reads 'draft' or 'live' is something the
    // item's stated `from` can genuinely contradict. A create binds 'draft',
    // a publish states `from: Draft` and agrees, the publish binds 'live', a
    // later revise states `from: Live` and agrees; no legacy row parks here,
    // because 'absent' is not comparable.
    match observed_state(lifecycle) {
        Some(observed) if observed == stated => None,
        Some(_) => Some("lifecycle_diverged"),
        None => None,
    }
}

pub async fn prepare_item(
    pool: &PgPool,
    lease: &LeasedItem,
    now: Timestamp,
) -> Result<ItemPreparation, EngineError> {
    let mapping = MappingRepo::new(pool.clone())
        .get(lease.org, lease.mapping)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "a leased item's mapping must exist".to_owned(),
        })?;
    let operation = lease.operation.clone();
    // The counterpart gate, here rather than in `admission`, which is a
    // synchronous total function of the operation and the mapping and is
    // unit-tested without a database precisely because it is. This check
    // needs the pool and the product, so folding it in would destroy that
    // property for every caller.
    if let Some(counterpart) = lease.requires_bound_on {
        match counterpart_binding(pool, lease.org, mapping.mapping.product, counterpart).await? {
            CounterpartState::Bound => {}
            CounterpartState::Waiting => return Ok(blocked(AWAITING_COUNTERPART)),
            // The gate's terminal arm. A counterpart that settled anything
            // other than bound is never going to bind, and without this the
            // waiting item cycles park to queue to park every day forever:
            // it is invisible to `expire_and_steal`, which filters on the
            // live states a parked row does not have. The listing is safely
            // on both platforms, which is the point of the gate -- but
            // nobody would ever be told the migration was over.
            CounterpartState::Unreachable => {
                return Ok(ItemPreparation::CounterpartLost { counterpart });
            }
        }
    }
    if let Some(gate) = admission(
        &operation,
        &mapping.mapping.binding,
        &mapping.mapping.lifecycle,
    ) {
        return Ok(blocked(gate));
    }
    // The lowering, here rather than at the API, because here is the first
    // place the id exists: the create bound it minutes ago and the seller
    // could not have named it when they asked. `from` is the binding's own
    // observed lifecycle rather than a state the enqueuer guessed.
    let operation = match operation {
        ItemOperation::Publish { to } => match &mapping.mapping.binding {
            Binding::Bound { id, .. } => ItemOperation::Revise {
                subject: id.clone(),
                transition: LifecycleTransition {
                    from: observed_state(&mapping.mapping.lifecycle).unwrap_or(ListingState::Draft),
                    to,
                },
            },
            // `admission` refused every other binding above, so this is
            // unreachable rather than merely unhandled.
            Binding::Unbound
            | Binding::Creating { .. }
            | Binding::AmbiguousCreate { .. }
            | Binding::Severed { .. } => return Ok(blocked("unbound")),
        },
        other @ (ItemOperation::Create
        | ItemOperation::Revise { .. }
        | ItemOperation::Remove { .. }) => other,
    };
    if matches!(operation, ItemOperation::Remove { .. }) {
        return Ok(ItemPreparation::Ready {
            operation,
            projected: None,
        });
    }
    let product = ProductRepo::new(pool.clone())
        .get(lease.org, mapping.mapping.product)
        .await?
        .ok_or(StorageError::Inconsistent {
            reason: "a mapped product must exist".to_owned(),
        })?
        .product;

    let taxonomy = TaxonomyRepo::new(pool.clone());
    let terms = taxonomy.terms().await?;
    // Which vocabularies to load is the registry's answer, not a second
    // hardcoded kind list: a target that binds a phase axis needs its phase
    // edges, and the product's own grade declaration names the source
    // vocabulary the grade ingests from before it projects.
    let edges = taxonomy
        .edges_into_all(&projection_vocabularies(lease.inventory, &product))
        .await?;
    let no_counterparts = taxonomy.no_counterparts_into(lease.inventory).await?;
    let elections = ElectionRepo::new(pool.clone());
    let rules = elections.rules(lease.org).await?;
    // The seller's own settled questions, beside the standing policies. The
    // answer that revived this item lives here and nowhere else, so a
    // projection reading rules alone re-raises the identical question and
    // parks again -- the decision surface would revive forever and release
    // nothing.
    let settled = elections
        .answered_for(lease.org, mapping.mapping.product)
        .await?;

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
            rules: &rules,
            settled: &settled,
        },
    ) {
        Ok(projection) => {
            record_losses(pool, lease, &projection.loss, now).await?;
            projection
        }
        Err(ProjectionBlocked::Blocked {
            gaps,
            elections: raised_elections,
            loss,
            ..
        }) => {
            // Before the raise, so the decision surface the raise populates
            // already carries what this listing gives up whatever the seller
            // picks. A loss never blocks; recording it only on the path that
            // did not block would show the seller nothing at the one moment
            // the disclosure is for.
            record_losses(pool, lease, &loss, now).await?;
            // Gaps first, because they drain: the answer is a durable edge
            // every later product finds waiting, so raising them is progress
            // even when an election blocks the same item. An election is this
            // product's own question and its queue is the seller's decision
            // surface, so the gate names whichever is still outstanding.
            let causes: Vec<_> = gaps
                .iter()
                .map(|gap| {
                    let VocabularyId(_, kind) = gap.target;
                    (gap.term, kind)
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
            // The seller's questions are enqueued under the engine pool for
            // the same reason the reconciliation items above are: this is
            // where the projection discovers them, and a question discovered
            // and not recorded is a park with nothing behind it.
            elections
                .raise(lease.org, lease.mapping, &raised_elections, now)
                .await?;
            let gate = if gaps.is_empty() && !raised_elections.is_empty() {
                "election"
            } else {
                "reconciliation"
            };
            return Ok(ItemPreparation::Blocked { gate, raised });
        }
        Err(ProjectionBlocked::CurrencyUnknown { .. }) => {
            return Ok(blocked("currency_unknown"));
        }
        Err(ProjectionBlocked::CoverMissing) => {
            return Ok(blocked("cover_missing"));
        }
        Err(ProjectionBlocked::ScanIncomplete { .. }) => {
            return Ok(blocked("scan_incomplete"));
        }
    };

    Ok(ItemPreparation::Ready {
        operation,
        projected: Some(ProjectedListing {
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
            body_format: projection.body_format,
            // The lowering beside the two that already exist: a resolved axis
            // the seam names no field for travels as the target vocabulary's
            // own term, labelled by the axis it answers.
            natives: projection
                .natives
                .iter()
                .map(|(axis, path)| NativeAxis {
                    axis: *axis,
                    value: native_term(path),
                })
                .collect(),
        }),
    })
}

/// What one projection could not carry, recorded against the mapping that
/// carried it under the engine's own INSERT grant.
///
/// The attempt is minted here rather than taken from the write attempt, which
/// does not exist yet: `prepare_item` runs before the driver opens one, and a
/// blocked projection never opens one at all. The table is append-only and
/// keyed on the attempt precisely so a re-projection writes its own rows
/// rather than replacing the last pass's, which is what buys the no-delete
/// rule, and a fresh id per projection is that rule stated at the writer.
async fn record_losses(
    pool: &PgPool,
    lease: &LeasedItem,
    losses: &[tam_domain::equivalence::Loss],
    now: Timestamp,
) -> Result<(), EngineError> {
    if losses.is_empty() {
        return Ok(());
    }
    MappingRepo::new(pool.clone())
        .record_losses(
            LossScope {
                org: lease.org,
                mapping: lease.mapping,
                attempt: AttemptId(fresh_uuid()),
                at: now,
            },
            losses,
        )
        .await?;
    Ok(())
}

/// Row identity for the loss record, minted at the engine boundary.
fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
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

    Ok(MachineSeed {
        form: form_id(lease.inventory),
        fields,
        intent_hash,
        strategy: CreateStrategy::DraftThenPublish {
            draft_state: RemoteLifecycleKind::Draft,
        },
        budget: StepBudget {
            actions_remaining: ACTIONS_PER_ITEM,
        },
        verify: verify_policy(lease.inventory),
    })
}

/// The removal's seed. A sibling of [`seed_from_projection`] rather than an
/// `Option` argument on it, because a removal has nothing for an adapter to
/// render: `project_fields` renders a listing, and a removal describes none.
/// The intent hash is taken over the removal's own recorded intent, so what
/// the ledger fingerprints is what the ledger stores.
#[must_use]
pub fn seed_for_removal(lease: &LeasedItem, operation: &ItemOperation) -> MachineSeed {
    let fields = FieldSet {
        entries: vec![],
        files: vec![],
    };
    let intent_hash =
        tam_pipeline::hash::content_hash(intent_as_json(operation, &fields).to_string().as_bytes());
    MachineSeed {
        form: form_id(lease.inventory),
        fields,
        intent_hash,
        strategy: CreateStrategy::DraftThenPublish {
            draft_state: RemoteLifecycleKind::Draft,
        },
        budget: StepBudget {
            actions_remaining: ACTIONS_PER_ITEM,
        },
        verify: verify_policy(lease.inventory),
    }
}
