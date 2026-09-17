//! A sync's enqueue half: the write jobs a canonicalised request has earned,
//! minted under the tenant's own role.
//!
//! Phase A of a sync reads the source marketplace, builds a canonical product
//! and projects it at the target. That read runs on the seller's own device
//! under D1, so nothing here reaches a marketplace; this crate takes the
//! breadcrumbs the device wrote and mints the jobs. Phase B is an ordinary
//! create item, which is the item pump's. The two are split because a job
//! carries one inventory, so a job holding both legs is not expressible.
//!
//! The role is the point. `tam_app` has forced RLS, so this pins one
//! organisation per request and cannot read two tenants' rows in one
//! statement; the item pump's BYPASSRLS scan cannot make that promise.
//! Nothing here is granted to `tam_engine`, and `sync_request` has no engine
//! grant at all.
//!
//! There is no binary. A poller that found work it could not service would be
//! worse than none, so `POST /{v}/sync` queues the rows and the enqueue below
//! is folded into the device's own import route.

#![forbid(unsafe_code)]

use tam_domain::{Binding, ItemOperation, JobItemId, Mapping, PublishMode, Verification};
use tam_import::ImportRun;
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::{ListingState, RemoteLifecycle, RemoteListingId};
use tam_storage::{
    job_request_key, ConsentRepo, Disposition, Enqueued, JobOrigin, JobRepo, LoweringRefusal,
    MappingRepo, Minted, NewJob, NewJobItem, StorageError, SyncRequestRecord, SyncRequestRepo,
    SyncResourceRecord, CREATE_LEG, REMOVE_LEG,
};
use tam_types::{
    Actor, JobId, MappingId, OrgId, Stamp, SystemComponent, TransportClass, Uuid,
    CONSENT_NOTICE_VERSION,
};

/// What one request's drain produced, so a caller reports it rather than
/// reading it back out of the row it just wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainReport {
    pub request: Uuid,
    pub skipped: usize,
    pub failed: usize,
    pub create_job: Option<Uuid>,
    pub remove_job: Option<Uuid>,
}

#[derive(Debug)]
pub enum DrainError {
    Storage(StorageError),
    /// The seller addressed a listing in a way the source cannot resolve.
    Locator(String),
    /// The request's stated intent has no lowering against the mapping the
    /// canonicalisation just minted.
    Lowering(LoweringRefusal),
}

impl core::fmt::Display for DrainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "storage: {error}"),
            Self::Locator(detail) => write!(f, "locator: {detail}"),
            Self::Lowering(refusal) => write!(f, "lowering: {refusal}"),
        }
    }
}

impl core::error::Error for DrainError {}

impl From<StorageError> for DrainError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Mint the write jobs a canonicalised request has earned.
///
/// Keyed on the per-resource breadcrumb rather than on the request, because
/// the canonicalisation is the device's and arrives a page at a time: a
/// resource an earlier page finished is skipped here and its removal item is
/// still built, from the breadcrumb `sync_request_resource` carries.
pub async fn drain_request(
    requests: &SyncRequestRepo,
    run: &ImportRun,
    request: Uuid,
) -> Result<DrainReport, DrainError> {
    // The fence, before the request is read as work. A deleted request is
    // hidden from `get`, so this is what turns a late drain into an honest
    // "nothing to do" rather than a locator error — and `create_job_in_tx`
    // holds the same fence inside the mint, for the case where the Delete
    // commits after this read.
    if requests.deletion_status(run.org, request).await?.is_some() {
        return Ok(DrainReport {
            request,
            skipped: 0,
            failed: 0,
            create_job: None,
            remove_job: None,
        });
    }
    let record = requests
        .get(run.org, request)
        .await?
        .ok_or_else(|| DrainError::Locator("no such sync request".to_owned()))?;
    requests.mark_draining(run.org, request).await?;

    let mut report = DrainReport {
        request,
        skipped: 0,
        failed: 0,
        create_job: None,
        remove_job: None,
    };
    let mut mappings = Vec::new();
    // Every canonicalised resource of the request, not this pass's alone. A
    // resource an earlier pass finished is skipped here and its removal item
    // still has to be minted, so it is rehydrated from its own breadcrumb --
    // which is why the breadcrumb records what the read observed about the
    // source and not only that the work is done.
    let mut canonicalised: Vec<Canonicalisation> = Vec::new();
    for resource in &record.resources {
        if resource.is_canonicalised() {
            report.skipped += 1;
            if let Some(mapping) = resource.mapping {
                mappings.push(mapping);
            }
            if let Some(done) = rehydrate(resource) {
                canonicalised.push(done);
            }
            continue;
        }
        // Recorded rather than skipped. One uncanonicalised resource does not
        // abandon the others -- the seller asked for a batch and gets a
        // per-row account of it -- but it must not pass silently either, or
        // `enqueue_create` mints a short list against a request the seller
        // asked to be whole and `record_enqueued` marks it done.
        requests
            .record_resource_failure(
                run.org,
                request,
                resource.ordinal,
                "no canonicalisation was reported for this resource; the source read runs \
                 on the seller's own device",
            )
            .await?;
        report.failed += 1;
    }

    if mappings.is_empty() {
        requests
            .record_failure(
                run.org,
                request,
                "no resource in this request could be canonicalised",
                run.now,
            )
            .await?;
        return Ok(report);
    }
    // The seller's explicit permission for a no-API marketplace, read here
    // as well as at the route, because this is the mint: a grant withdrawn
    // between the request and its drain must leave no job behind.
    if let Some(marketplace) = ungranted(run, record.source, record.target).await? {
        requests
            .record_failure(
                run.org,
                request,
                &format!(
                    "{marketplace:?} needs the seller's permission before Teachouse can work \
                     with it; nothing was queued"
                ),
                run.now,
            )
            .await?;
        return Ok(report);
    }
    // A leg refused because the seller deleted the request mid-drain stops
    // the pass where it stands. Nothing was written for that leg, the legs
    // already minted carry their request's id and are fenced through it, and
    // `record_enqueued` deliberately never runs: marking a deleted request
    // `enqueued` would resurrect it in the seller's list.
    let Some(create_job) = enqueue_create(run, &record, &mappings).await? else {
        return Ok(report);
    };
    report.create_job = Some(create_job);
    // A migrate is two jobs, forced: `job.inventory` is single-valued, so the
    // create on the target and the removal on the source cannot share one.
    // The request row is what holds the pair together and is what the seller
    // polls.
    let remove_job = if removes_the_source(record.disposition) {
        let Some(removal) = enqueue_removal(run, &record, &canonicalised).await? else {
            return Ok(report);
        };
        Some(removal)
    } else {
        None
    };
    report.remove_job = remove_job;
    requests
        .record_enqueued(
            run.org,
            &Enqueued {
                request,
                create_job: Some(create_job),
                remove_job,
                at: run.now,
            },
        )
        .await?;
    Ok(report)
}

/// The first of the two legs' marketplaces that publishes no official API and
/// has no standing grant, else `None`.
async fn ungranted(
    run: &ImportRun,
    source: tam_types::InventoryId,
    target: tam_types::InventoryId,
) -> Result<Option<tam_types::Marketplace>, StorageError> {
    let consents = ConsentRepo::new(run.pool.clone());
    for marketplace in [source.marketplace(), target.marketplace()] {
        if marketplace.transport_class() != TransportClass::SellerDevice {
            continue;
        }
        if consents
            .standing(run.org, marketplace, CONSENT_NOTICE_VERSION)
            .await?
            .is_none()
        {
            return Ok(Some(marketplace));
        }
    }
    Ok(None)
}

/// A resource an earlier pass canonicalised, read back off its breadcrumb.
///
/// `is_canonicalised` already requires the source identifier, so the fields
/// are present whenever the branch is taken; the option is unwrapped rather
/// than asserted because a row that somehow lost one is a row whose removal
/// item cannot be built, and dropping it silently is the defect this whole
/// change exists to remove -- `enqueue_removal` refuses the short list.
fn rehydrate(resource: &SyncResourceRecord) -> Option<Canonicalisation> {
    Some(Canonicalisation {
        product: resource.product?,
        mapping: resource.mapping?,
        source: resource.source.clone()?,
        source_state: resource.source_state,
    })
}

/// One resource's canonicalisation, including what the read observed about
/// the source listing. A migrate's removal names both, and they come from the
/// read that already happened rather than from a second one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Canonicalisation {
    product: tam_types::ProductId,
    mapping: tam_types::MappingId,
    source: RemoteListingId,
    source_state: Option<ListingState>,
}

/// The write leg, minted with a key derived from the request rather than with
/// the request's own id.
///
/// `job_request_idempotent` admits one job per key per organisation. A
/// migrate is two jobs, so reusing the request id would make the second
/// creation a replay of the first, returning the first job with `replay:
/// true` and dropping the second job's items with no error anywhere. The
/// removal would never exist and the migrate would silently become a sync.
///
/// The request's stated intent is lowered here through the same table
/// `POST /{v}/jobs` lowers through, because a create is only what `draft`
/// means. Both adapters create a draft, so `live` is a create and a publish
/// gated on the create's own binding; minting the create alone left a seller
/// who asked for a live listing with a draft, a request that settled
/// `enqueued` and a job that settled succeeded, with nothing anywhere saying
/// the listing is not live.
async fn enqueue_create(
    run: &ImportRun,
    record: &SyncRequestRecord,
    mappings: &[tam_types::MappingId],
) -> Result<Option<Uuid>, DrainError> {
    // The items are `tam-import`'s, because the device import's completing page
    // mints the same rows inside its own transaction and two copies of this
    // lowering would let the two callers diverge on what a `live` request means.
    let job = JobId(fresh_uuid());
    let items = tam_import::create_items(run, record.intent, job, mappings)
        .await
        .map_err(|error| match error {
            tam_import::ImportError::Lowering(refusal) => DrainError::Lowering(refusal),
            tam_import::ImportError::Storage(error) => DrainError::Storage(error),
            // `create_items` lowers an intent and derives keys. It reads no
            // price, ingests no payload and resolves no currency, so these
            // arms exist because the shared error type is wider than this
            // call rather than because this call can reach them — and saying
            // so as an inconsistency is truer than filing them under a
            // locator failure, which is what a wildcard arm did here before.
            impossible @ (tam_import::ImportError::NoPayload
            | tam_import::ImportError::Price(_)
            | tam_import::ImportError::CurrencyUnknown { .. }
            | tam_import::ImportError::NoTarget) => {
                DrainError::Storage(StorageError::Inconsistent {
                    reason: format!("the create items answered {impossible}"),
                })
            }
        })?;
    let minted = JobRepo::new(run.pool.clone())
        .create_with_request_key(
            run.org,
            JobOrigin {
                request_key: job_request_key(record.id, CREATE_LEG),
                // The job names its request as it is minted, in this
                // same statement. `record_enqueued` names it the other
                // way in a later transaction, and a settle landing
                // before that -- or after it failed -- would otherwise
                // find no run to notify a seller about. It is also how a
                // Delete finds this leg before those columns are written
                // at all.
                run: Some(record.id),
                // A migration's legs are not an import's publication.
                import_run: None,
            },
            &NewJob {
                job,
                inventory: record.target,
                stamp: Stamp {
                    at: run.now,
                    actor: Actor::System(SystemComponent::Worker),
                },
            },
            &items,
            // Cron-driven. sync_request carries no requester column, so the
            // person who asked for the sync is not recoverable here; the
            // worker names itself rather than guessing at one.
        )
        .await?;
    Ok(match minted {
        Minted::Job(created) => Some(created.job.0),
        Minted::WorkflowDeleted(_) => None,
    })
}

/// The removal leg: the source mapping bound from the read, and one removal
/// item gated on the target's binding.
///
/// The source mapping is inserted `Bound` because the import read *is* the
/// observation -- not a presumption -- and its lifecycle comes from the same
/// read. Following `import_one`'s own precedent and writing `Absent` would
/// manufacture a fresh bound-but-absent row, which the API refuses as
/// unverifiable, so a listing migrated away could never be published,
/// revised or re-synced again with an error saying nothing was verified.
///
/// Inserted only where the product has no mapping on the source yet, which is
/// the device-import case this leg was written for: the read discovered the
/// listing, so nothing in the catalogue named it before. A migration started
/// from the catalogue is the other case -- the product is eligible precisely
/// because it already holds a bound mapping on the source -- and
/// `mapping_one_per_inventory` admits one row per product and inventory, so
/// inserting there would refuse the whole drain. The existing row is the
/// same observation by a different route, so the removal item names it.
///
/// A source whose read carried no state is refused rather than removed: a
/// removal must never post a lifecycle nobody has observed.
///
/// An empty leg is refused for the same reason and is louder: a removal job
/// with no items is created without complaint, reads back as settled because
/// zero settled of zero is complete, and lets `record_enqueued` mark the
/// request done naming a job that will never remove anything. The migrate
/// would degrade into a plain sync and say nothing, which is exactly what the
/// request key was split in two to prevent.
async fn enqueue_removal(
    run: &ImportRun,
    record: &SyncRequestRecord,
    canonicalised: &[Canonicalisation],
) -> Result<Option<Uuid>, DrainError> {
    if canonicalised.is_empty() {
        return Err(DrainError::Locator(
            "a migrate's removal leg carries no resources, so the source listings would \
             survive a request recorded as enqueued"
                .to_owned(),
        ));
    }
    let mappings = MappingRepo::new(run.pool.clone());
    let products: Vec<tam_types::ProductId> = canonicalised.iter().map(|row| row.product).collect();
    let on_source: Vec<(tam_types::ProductId, MappingId)> = mappings
        .heads_for_products(run.org, record.source, &products)
        .await?
        .into_iter()
        .map(|head| (head.product, head.id))
        .collect();
    let mut items = Vec::new();
    let job = JobId(fresh_uuid());
    for row in canonicalised {
        let Some(state) = row.source_state else {
            return Err(DrainError::Locator(
                "the source read carried no lifecycle, so its removal would state one \
                 nobody observed"
                    .to_owned(),
            ));
        };
        let mapping = if let Some(&(_, existing)) = on_source
            .iter()
            .find(|(product, _)| *product == row.product)
        {
            existing
        } else {
            // The policies and the price rule are the target mapping's,
            // which the canonicalisation just derived from this very read.
            // Inventing a second set here would be two answers to one
            // question about one product.
            let target = mappings.get(run.org, row.mapping).await?.ok_or_else(|| {
                DrainError::Locator("the canonicalised mapping vanished".to_owned())
            })?;
            let mapping = MappingId(fresh_uuid());
            let source = Mapping {
                id: mapping,
                org: run.org,
                product: row.product,
                inventory: record.source,
                binding: Binding::Bound {
                    id: row.source.clone(),
                    first_seen: run.now,
                    verified: Verification::Clean { at: run.now },
                },
                policies: target.mapping.policies,
                price_rule: target.mapping.price_rule,
                publish: PublishMode::DryRun,
                lifecycle: lifecycle_of(state, run.now),
            };
            mappings.insert(run.org, &source, 0, run.now).await?;
            mapping
        };
        // No frozen output travels with a removal, and the ledger's capture
        // skips one for it: a removal posts no price and no terms, so there
        // is nothing to approve and nothing an approval could change. It
        // therefore keeps the key derived here, where a create's is mixed
        // with what it will post.
        items.push(NewJobItem {
            item: JobItemId(fresh_uuid()),
            mapping,
            idempotency_key: derive_idempotency_key(
                run.org,
                record.source,
                row.product,
                INTENT_VERSION,
                tam_storage::job_reads::intent_digest(
                    &ItemOperation::Remove {
                        subject: row.source.clone(),
                        state,
                    },
                    job,
                    &[],
                    0,
                ),
            ),
            operation: ItemOperation::Remove {
                subject: row.source.clone(),
                state,
            },
            // The whole safety property. The source listing does not go until
            // the target listing exists and the driver's own verification
            // read saw it, so the unsafe direction -- source gone, target
            // absent -- is unreachable and the failure mode is a duplicate.
            requires_bound_on: Some(record.target),
        });
    }
    let minted = JobRepo::new(run.pool.clone())
        .create_with_request_key(
            run.org,
            JobOrigin {
                request_key: job_request_key(record.id, REMOVE_LEG),
                // The job names its request as it is minted, in this
                // same statement. `record_enqueued` names it the other
                // way in a later transaction, and a settle landing
                // before that -- or after it failed -- would otherwise
                // find no run to notify a seller about. It is also how a
                // Delete finds this leg before those columns are written
                // at all, which for a removal is the difference between
                // stopping a migration and taking the seller's source
                // listing down after they stopped it.
                run: Some(record.id),
                // A migration's legs are not an import's publication.
                import_run: None,
            },
            &NewJob {
                job,
                inventory: record.source,
                stamp: Stamp {
                    at: run.now,
                    actor: Actor::System(SystemComponent::Worker),
                },
            },
            &items,
            // Cron-driven. sync_request carries no requester column, so the
            // person who asked for the sync is not recoverable here; the
            // worker names itself rather than guessing at one.
        )
        .await?;
    Ok(match minted {
        Minted::Job(created) => Some(created.job.0),
        Minted::WorkflowDeleted(_) => None,
    })
}

const fn lifecycle_of(state: ListingState, at: tam_types::Timestamp) -> RemoteLifecycle {
    match state {
        ListingState::Draft => RemoteLifecycle::Draft,
        ListingState::Live => RemoteLifecycle::Live { since: at },
    }
}

/// A migrate's removal leg needs its own key, and this is where it comes
/// from. Named here beside the create's so the pair is one reading.
#[must_use]
pub fn removal_request_key(request: Uuid) -> Uuid {
    job_request_key(request, REMOVE_LEG)
}

/// Whether this request's disposition asks for the source listing to go.
#[must_use]
pub const fn removes_the_source(disposition: Disposition) -> bool {
    matches!(disposition, Disposition::Migrate)
}

/// The tenants a drain pass visits, and the requests each holds.
pub async fn pending_work(
    requests: &SyncRequestRepo,
    limit: i64,
) -> Result<Vec<(OrgId, Vec<Uuid>)>, StorageError> {
    let mut out = Vec::new();
    for org in requests.tenants().await? {
        let pending = requests.pending(org, limit).await?;
        if !pending.is_empty() {
            out.push((org, pending));
        }
    }
    Ok(out)
}

/// The intent version the ledger's keys are minted under, restated here for
/// the same reason the API restates it: it is a property of the key's
/// derivation and changing it is a deliberate re-keying of the whole ledger.
const INTENT_VERSION: u32 = 1;

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}
