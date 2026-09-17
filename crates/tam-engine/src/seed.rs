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
    Binding, ItemOperation, ItemOutcome, ProjectionBlocked, StepBudget, VocabularyId,
    VocabularyPath, PARK_TTL_MS,
};
use tam_marketplace::{
    AgeSpan, CreateStrategy, FormId, LifecycleTransition, ListingState, NativeAxis, NativeTerm,
    ProjectedListing, RemoteLifecycle, RemoteLifecycleKind,
};
use tam_storage::{
    ElectionRepo, ItemVerdict, LeaseRepo, LeasedItem, LossScope, MappingRepo, OverrideRepo,
    ProductRepo, RaiseReport, RaiseScope, StorageError, TaxonomyRepo, TptBaseRepo, ELECTION,
};
use tam_taxonomy::listing::{
    project_listing_with_overrides, projection_vocabularies, ListingContext,
};
use tam_types::{
    AttemptId, FailureCode, FailureDetail, InventoryId, Marketplace, OrgId, Timestamp, Uuid,
};

use tam_engine_driver::driver::{EngineError, VerifyPolicy};
use tam_engine_driver::vocabulary::ItemPreparation as WirePreparation;

/// Per-item action ceiling; generous against the longest measured flow
/// (create, metadata, three-step file upload per file, read-back).
pub const ACTIONS_PER_ITEM: u32 = 32;

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
        InventoryId::Tes | InventoryId::Etsy => VerifyPolicy {
            tries: 8,
            interval_ms: 2_000,
        },
        InventoryId::Tpt => VerifyPolicy {
            tries: 11,
            interval_ms: 2_000,
        },
    }
}

/// One form identity per inventory, deterministic: byte one is the
/// idempotency module's durable inventory ordinal reused as a tag.
pub const fn form_id(inventory: InventoryId) -> FormId {
    let tag = match inventory {
        InventoryId::Tes => 0x01,
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

/// One item prepared, with the answers that are not "run it" already applied.
///
/// Both hosts prepare the same way and must dispose the same way, which is why
/// this exists rather than each writing its own arm. An item the preparation
/// refuses is parked or settled here and never handed back to the queue: the
/// claim orders by `created_at` and a release advances nothing, so a host that
/// released a permanently blocked item would be served the same item on its
/// next poll for ever, and would starve every sibling on that marketplace
/// behind the per-connection mutex while doing it.
pub enum Disposed {
    Ready {
        operation: ItemOperation,
        projected: Option<ProjectedListing>,
    },
    Parked {
        gate: &'static str,
        raised: RaiseReport,
    },
    Skipped {
        counterpart: InventoryId,
    },
}

/// Prepares the item and applies the disposition its answer calls for.
///
/// The park states a duration rather than an instant, because the reaper reads
/// the expiry against the database's clock.
pub async fn prepare_and_dispose(
    pool: &PgPool,
    leases: &LeaseRepo,
    item: &LeasedItem,
    now: Timestamp,
) -> Result<Disposed, EngineError> {
    match prepare_item(pool, item, now).await? {
        ItemPreparation::Ready {
            operation,
            projected,
        } => Ok(Disposed::Ready {
            operation,
            projected,
        }),
        ItemPreparation::Blocked { gate, raised } => {
            leases
                .park(&item.lease_ref(), gate, PARK_TTL_MS.div_euclid(1_000))
                .await
                .map_err(|error| crate::ledger::to_wire_error(&error))?;
            Ok(Disposed::Parked { gate, raised })
        }
        // Waiting is over rather than merely unsatisfied. The listing is
        // safely on both platforms, which is what the gate is for; what the
        // seller needs now is to be told the migration ended and why.
        ItemPreparation::CounterpartLost { counterpart } => {
            let verdict = ItemVerdict {
                outcome: ItemOutcome::Skipped,
                failure_code: Some(FailureCode::Other),
                failure_detail: Some(FailureDetail(format!(
                    "the listing on {counterpart:?} never bound, so this item's counterpart \
                     will not arrive; the source listing was left in place"
                ))),
            };
            leases
                .settle(&item.lease_ref(), &verdict, now)
                .await
                .map_err(|error| crate::ledger::to_wire_error(&error))?;
            Ok(Disposed::Skipped { counterpart })
        }
    }
}

/// The gate's own park reason, which reaches the ledger and the job report.
/// Re-exported rather than restated: `REVIVABLE_GATES` and the bind's own
/// revive both read this string, and a second spelling of it is a park
/// nothing ever wakes.
pub use tam_storage::AWAITING_COUNTERPART;

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
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?
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
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?
        .ok_or_else(|| {
            crate::ledger::to_wire_error(&StorageError::Inconsistent {
                reason: "a leased item's mapping must exist".to_owned(),
            })
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
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?
        .ok_or_else(|| {
            crate::ledger::to_wire_error(&StorageError::Inconsistent {
                reason: "a mapped product must exist".to_owned(),
            })
        })?
        .product;

    let taxonomy = TaxonomyRepo::new(pool.clone());
    let terms = taxonomy
        .terms()
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;
    // Which vocabularies to load is the registry's answer, not a second
    // hardcoded kind list: a target that binds a phase axis needs its phase
    // edges, and the product's own grade declaration names the source
    // vocabulary the grade ingests from before it projects.
    let edges = taxonomy
        .edges_into_all(&projection_vocabularies(lease.inventory, &product))
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;
    let no_counterparts = taxonomy
        .no_counterparts_into(lease.inventory)
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;
    let elections = ElectionRepo::new(pool.clone());
    let rules = elections
        .rules(lease.org)
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;
    // The seller's own settled questions, beside the standing policies. The
    // answer that revived this item lives here and nowhere else, so a
    // projection reading rules alone re-raises the identical question and
    // parks again -- the decision surface would revive forever and release
    // nothing.
    let settled = elections
        .answered_for(lease.org, mapping.mapping.product)
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;

    // The seller's own mapping decisions, consulted ahead of the global
    // relation. Without them this projection re-raises questions the seller
    // has already answered, and worse, publishes under the relation's answer
    // rather than theirs — so the decision surface would show a choice that
    // never reached the listing.
    let overrides = OverrideRepo::new(pool.clone())
        .for_org(lease.org)
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;

    // What this item was enqueued to post, frozen at that instant. Read here
    // rather than re-evaluated because a rule is a policy the seller may edit
    // at any moment and this item is money and terms they already
    // authorised: re-reading the policy would let an edit made after the
    // enqueue change what a device posts, with nothing recording that the two
    // differed.
    //
    // `None` for an item enqueued before the freeze existed, and that absence
    // is the whole of the cutover: those items project exactly as they did,
    // from the canonical price and the relation's own answers. Nothing
    // backfills them, because a canonical price relabelled as an approval
    // would be indistinguishable afterwards from one a seller actually
    // approved.
    //
    // An immutable row, which is what makes the two projections of one lease
    // agree: `live_lease_files` prepares the same item a second time to
    // recover its file list, and a value re-derived from a mutable policy
    // could differ between the two calls, leaving a work order whose files
    // and price came from different worlds.
    let approved = tam_storage::rule_capture::frozen_output(pool, lease.org, lease.item)
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;

    let projection = match project_listing_with_overrides(
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
        &overrides,
        approved.as_ref(),
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
                .await
                .map_err(|error| crate::ledger::to_wire_error(&error))?;
            // The seller's questions are enqueued under the engine pool for
            // the same reason the reconciliation items above are: this is
            // where the projection discovers them, and a question discovered
            // and not recorded is a park with nothing behind it.
            let elected = elections
                .raise(lease.org, lease.mapping, &raised_elections, now)
                .await
                .map_err(|error| crate::ledger::to_wire_error(&error))?;
            // The report is the gate's own, not the other gate's. The worker
            // re-checks an election park against it -- a raise that minted
            // nothing is a question already answered, and requeueing on that
            // would spin rather than progress.
            let (gate, raised) = if gaps.is_empty() && !raised_elections.is_empty() {
                (ELECTION, elected)
            } else {
                ("reconciliation", raised)
            };
            return Ok(ItemPreparation::Blocked { gate, raised });
        }
        Err(ProjectionBlocked::CurrencyUnknown { .. }) => {
            return Ok(blocked("currency_unknown"));
        }
        Err(ProjectionBlocked::CurrencyMismatch { .. }) => {
            return Ok(blocked("currency_mismatch"));
        }
        Err(ProjectionBlocked::CoverMissing) => {
            return Ok(blocked("cover_missing"));
        }
        Err(ProjectionBlocked::ScanIncomplete { .. }) => {
            return Ok(blocked("scan_incomplete"));
        }
    };

    // The seller's own localisation declaration, out of the sidecar that holds
    // what the canonical model has no field for.
    //
    // `and_then` rather than `map`, and the distinction is the whole reason
    // this waited for a nullable column: no row and a row stating nothing are
    // both "not stated", and only a row that states a value may answer here.
    // Stating a value suppresses the read-back the adapter's edit relies on,
    // so answering `Some(false)` for a row that never said so would repost `0`
    // over a box the seller ticked on TPT — the silent clear the form work
    // fixed, arriving through the sidecar instead of through a constant.
    //
    // Read only where an inventory binds it, so a Tes item costs no query.
    let appropriate_for_country = if lease.inventory.marketplace() == Marketplace::Tpt {
        TptBaseRepo::new(pool.clone())
            .get(lease.org, mapping.mapping.product)
            .await
            .map_err(|error| crate::ledger::to_wire_error(&error))?
            .and_then(|record| record.categories.appropriate_for_country)
    } else {
        None
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
            appropriate_for_country,
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
        .await
        .map_err(|error| crate::ledger::to_wire_error(&error))?;
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

/// The server's decisions about one item, as the device receives them.
///
/// The form, the create strategy, the step budget and the verify policy are
/// all policy: a device that recomputed them would be setting its own budget.
/// They are gathered here, once, so the claim endpoint and the in-process
/// worker hand the interpreter the same preparation.
/// The create strategy every inventory is configured with, and the one place
/// it is decided.
///
/// One constant rather than a per-inventory table because there is one answer
/// today: both device-branch marketplaces create draft-first, and a
/// draft-then-publish create is identified by the title it recorded that it
/// sent, narrowed to the state it leaves a listing in. The correlation
/// marker's carrier stayed an open founder decision and the marker-free route
/// was adopted instead, so no listing carries a marker and none needs to.
/// `reconcile_is_available` reads this rather than restating it, so a strategy
/// change and the claim's admission of stranded creates move together.
const CREATE_STRATEGY: CreateStrategy = CreateStrategy::DraftThenPublish {
    draft_state: RemoteLifecycleKind::Draft,
};

/// Whether a stranded create is worth taking out of its park.
///
/// The question is whether the strategy leaves a create this walk can
/// identify, not whether it embeds a marker. `CorrelationMarker` embeds one.
/// `DraftThenPublish` embeds nothing but leaves the listing in a state the
/// seller's own catalogue exposes, which is what makes its recorded title
/// usable: the narrowing is what turns a title that is not unique into an
/// identification. `HaltOnAmbiguity` leaves neither, and its name says so.
///
/// Under a strategy that leaves nothing there is nothing to search for, and
/// the run would reach the machine's unsearchable arm and settle the item
/// ambiguous to learn what this constant already says. The claim declines to
/// serve one instead, so it stays parked exactly as the reaper left it, still
/// fencing its mapping and still reconcilable by the build that can.
#[must_use]
pub const fn reconcile_is_available() -> bool {
    matches!(
        CREATE_STRATEGY,
        CreateStrategy::CorrelationMarker { .. } | CreateStrategy::DraftThenPublish { .. }
    )
}

#[must_use]
pub fn preparation(
    lease: &LeasedItem,
    operation: ItemOperation,
    projected: Option<ProjectedListing>,
) -> WirePreparation {
    WirePreparation {
        operation,
        projected,
        form: form_id(lease.inventory),
        strategy: CREATE_STRATEGY,
        budget: StepBudget {
            actions_remaining: ACTIONS_PER_ITEM,
        },
        verify: verify_policy(lease.inventory),
    }
}

#[cfg(test)]
mod lease_budget_tests {
    use super::verify_policy;
    use tam_domain::LEASE_TTL_SECS;
    use tam_types::InventoryId;

    /// The one lease TTL, in milliseconds. Imported rather than mirrored: the
    /// value lives in `tam-domain`, which the API, the worker and this crate
    /// all depend on, so there is nothing left to drift.
    fn lease_ms() -> u64 {
        u64::try_from(i64::from(LEASE_TTL_SECS) * 1_000).unwrap_or(u64::MAX)
    }

    /// The slowest Tpt create actually measured, recorded as "nearly three
    /// minutes" at `crates/tam-marketplace-tpt/src/flows.rs`.
    const MEASURED_SUBMIT_WORST_CASE_MS: u64 = 180_000;

    /// Tpt's theoretical worst case for the same submit: two queue-job polls
    /// at `QUEUE_POLL_MAX` × `QUEUE_POLL_INTERVAL_MS` (150 × 1,200ms each)
    /// inside one `submit`, with no renew between them because the renew sits
    /// before the effect rather than inside the adapter.
    const THEORETICAL_SUBMIT_WORST_CASE_MS: u64 = 360_000;

    /// What the heartbeat actually buys, stated as the measured case.
    ///
    /// The interpreter renews immediately before every network-bearing effect
    /// and before every verification try, so a submit starts with a full TTL
    /// in hand and the stretch that must fit is one effect plus one try's
    /// interval, not the whole run.
    #[test]
    fn the_measured_stretch_between_two_heartbeats_fits_inside_the_lease() {
        let lease_ms = lease_ms();
        for inventory in InventoryId::ALL {
            let between =
                MEASURED_SUBMIT_WORST_CASE_MS + u64::from(verify_policy(inventory).interval_ms);
            assert!(
                between < lease_ms,
                "{inventory:?}: the longest stretch between two heartbeats is \
                 {between}ms against a {lease_ms}ms lease; a stretch longer than the \
                 lease is a working device losing its item to the reaper, which is \
                 exactly what the heartbeat exists to prevent"
            );
        }
    }

    /// Tpt's theoretical worst case now fits, which is why the lease was
    /// raised.
    ///
    /// The renew sits before the effect rather than inside the adapter, so two
    /// queue polls inside one `submit` are one uninterrupted stretch: 360s
    /// with no heartbeat in it. Against the old 300s lease that stretch
    /// outlived the lease it began under, and this test asserted the defect.
    /// `LEASE_TTL_SECS` was raised to 600 on 2026-09-03 by founder decision
    /// for this reason — question 7 of
    /// `docs/notes/design/engine-driver-split.md` — so the same measured
    /// numbers now state a guarantee instead.
    #[test]
    fn tpts_theoretical_worst_case_submit_fits_inside_the_lease() {
        assert!(
            THEORETICAL_SUBMIT_WORST_CASE_MS < lease_ms(),
            "the worst-case stretch is {THEORETICAL_SUBMIT_WORST_CASE_MS}ms against a \
             {}ms lease; if this fails the lease has been lowered without the poll being \
             shortened to match, and a working device loses its item mid-submit",
            lease_ms()
        );
    }

    #[test]
    fn the_verification_poll_fits_inside_the_lease() {
        let lease_ms = lease_ms();
        for inventory in InventoryId::ALL {
            let spent = MEASURED_SUBMIT_WORST_CASE_MS + verify_policy(inventory).window_ms();
            assert!(
                spent < lease_ms,
                "{inventory:?}: submit_worst_case + tries * interval is {spent}ms against a \
                 {lease_ms}ms lease; when the lease expires mid-run the epoch-fenced attempt \
                 settle fails after a listing has landed, and the mapping never binds"
            );
        }
    }
}
