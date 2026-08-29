//! The canonicalisation drain: a sync's read leg, run outside the item pump
//! and under the tenant's own role.
//!
//! Phase A of a sync reads the source marketplace, builds a canonical product
//! and projects it at the target. It writes the catalogue and no marketplace.
//! Phase B is an ordinary create item, which is the item pump's. The two are
//! split here because `prepare_item` is deliberately adapter-free -- an item
//! that cannot run never costs a gateway session -- and because a job carries
//! one inventory, so a job holding both legs is not expressible.
//!
//! The role is the point. `tam_app` has forced RLS, so this drain pins one
//! organisation per request and cannot read two tenants' rows in one
//! statement; the item pump's BYPASSRLS scan cannot make that promise.
//! Nothing here is granted to `tam_engine`, and `sync_request` has no engine
//! grant at all.

#![forbid(unsafe_code)]

use tam_domain::{Binding, ItemOperation, JobItemId, Mapping, PublishMode, Verification};
use tam_import::{import_one, ImportEntry, ImportError, ImportRun, NamedBytes};
use tam_marketplace::idempotency::derive_idempotency_key;
use tam_marketplace::{
    FetchReason, FirstPartyExport, ListingState, RemoteLifecycle, RemoteListingId,
};
use tam_storage::{
    job_request_key, Canonicalised, Disposition, Enqueued, JobReadRepo, JobRepo, MappingRepo,
    NewJob, NewJobItem, StorageError, SyncRequestRecord, SyncRequestRepo, SyncResourceRecord,
    CREATE_LEG, REMOVE_LEG,
};
use tam_types::{InventoryId, JobId, MappingId, OrgId, Uuid};

/// What one request's drain produced, so a caller reports it rather than
/// reading it back out of the row it just wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainReport {
    pub request: Uuid,
    pub canonicalised: usize,
    pub skipped: usize,
    pub failed: usize,
    pub create_job: Option<Uuid>,
    pub remove_job: Option<Uuid>,
}

#[derive(Debug)]
pub enum DrainError {
    Storage(StorageError),
    Import(ImportError),
    /// The seller addressed a listing in a way the source cannot resolve.
    Locator(String),
}

impl core::fmt::Display for DrainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "storage: {error}"),
            Self::Import(error) => write!(f, "import: {error:?}"),
            Self::Locator(detail) => write!(f, "locator: {detail}"),
        }
    }
}

impl core::error::Error for DrainError {}

impl From<StorageError> for DrainError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

/// Canonicalise one request whole, then mint the write jobs it earned.
///
/// Resumable per resource rather than per request. `import_one` commits four
/// times internally and mints a fresh `ProductId` on every pass, and
/// `mapping_one_per_inventory` is keyed on the product, so a second pass over
/// an already-canonicalised resource inserts a duplicate no index refuses.
/// The breadcrumb on `sync_request_resource` is what makes the skip possible,
/// and it is written in the same transaction that marks the row done.
pub async fn drain_request<A>(
    requests: &SyncRequestRepo,
    run: &ImportRun<'_, A>,
    request: Uuid,
    reason: &FetchReason,
) -> Result<DrainReport, DrainError>
where
    A: FirstPartyExport,
    A::Resource: TryFrom<i64> + Copy,
    <A::Resource as TryFrom<i64>>::Error: core::fmt::Display,
{
    let record = requests
        .get(run.org, request)
        .await?
        .ok_or_else(|| DrainError::Locator("no such sync request".to_owned()))?;
    requests.mark_draining(run.org, request).await?;

    let mut report = DrainReport {
        request,
        canonicalised: 0,
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
        match canonicalise_one(run, &resource.locator, reason).await {
            Ok(row) => {
                requests
                    .record_canonicalised(
                        run.org,
                        &Canonicalised {
                            request,
                            ordinal: resource.ordinal,
                            product: row.product,
                            mapping: row.mapping,
                            source: row.source.clone(),
                            source_state: row.source_state,
                        },
                    )
                    .await?;
                mappings.push(row.mapping);
                canonicalised.push(row);
                report.canonicalised += 1;
            }
            Err(error) => {
                // One unreadable resource does not abandon the others: the
                // seller asked for a batch and gets a per-row account of it,
                // which is what the drain's own totals concealed before.
                requests
                    .record_resource_failure(run.org, request, resource.ordinal, &error.to_string())
                    .await?;
                report.failed += 1;
            }
        }
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
    let create_job = enqueue_create(run, &record, &mappings).await?;
    report.create_job = Some(create_job);
    // A migrate is two jobs, forced: `job.inventory` is single-valued, so the
    // create on the target and the removal on the source cannot share one.
    // The request row is what holds the pair together and is what the seller
    // polls.
    let remove_job = if removes_the_source(record.disposition) {
        Some(enqueue_removal(run, &record, &canonicalised).await?)
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

async fn canonicalise_one<A>(
    run: &ImportRun<'_, A>,
    locator: &str,
    reason: &FetchReason,
) -> Result<Canonicalisation, DrainError>
where
    A: FirstPartyExport,
    A::Resource: TryFrom<i64> + Copy,
    <A::Resource as TryFrom<i64>>::Error: core::fmt::Display,
{
    let numeric: i64 = locator
        .parse()
        .map_err(|_| DrainError::Locator(format!("{locator:?} is not a resource id")))?;
    let resource = A::Resource::try_from(numeric)
        .map_err(|error| DrainError::Locator(format!("{locator:?}: {error}")))?;
    let bundle = run
        .adapter
        .download_resource_bundle(reason, resource)
        .await
        .map_err(|error| DrainError::Import(ImportError::Adapter(error)))?;
    let entry = ImportEntry {
        resource: numeric,
        files: vec![NamedBytes {
            name: format!("{numeric}-bundle.zip"),
            bytes: bundle,
        }],
    };
    let row = import_one(run, &entry).await.map_err(DrainError::Import)?;
    Ok(Canonicalisation {
        product: row.product,
        mapping: row.mapping,
        source: row.source,
        source_state: row.source_state,
    })
}

/// The write leg, minted with a key derived from the request rather than with
/// the request's own id.
///
/// `job_request_idempotent` admits one job per key per organisation. A
/// migrate is two jobs, so reusing the request id would make the second
/// creation a replay of the first, returning the first job with `replay:
/// true` and dropping the second job's items with no error anywhere. The
/// removal would never exist and the migrate would silently become a sync.
async fn enqueue_create<A>(
    run: &ImportRun<'_, A>,
    record: &SyncRequestRecord,
    mappings: &[tam_types::MappingId],
) -> Result<Uuid, DrainError>
where
    A: FirstPartyExport,
{
    let seeds = JobReadRepo::new(run.pool.clone())
        .mapping_seeds(run.org, record.target, mappings)
        .await?;
    let job = JobId(fresh_uuid());
    let items: Vec<NewJobItem> = seeds
        .iter()
        .map(|seed| NewJobItem {
            item: JobItemId(fresh_uuid()),
            mapping: seed.mapping,
            idempotency_key: derive_idempotency_key(
                run.org,
                record.target,
                seed.product,
                INTENT_VERSION,
                tam_storage::job_reads::intent_digest(
                    &ItemOperation::Create,
                    job,
                    &seed.payload_hashes,
                    seed.sever_generation,
                ),
            ),
            operation: ItemOperation::Create,
            requires_bound_on: None,
        })
        .collect();
    let created = JobRepo::new(run.pool.clone())
        .create_with_request_key(
            run.org,
            job_request_key(record.id, CREATE_LEG),
            &NewJob {
                job,
                inventory: record.target,
                at: run.now,
            },
            &items,
        )
        .await?;
    Ok(created.job.0)
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
/// A source whose read carried no state is refused rather than removed: a
/// removal must never post a lifecycle nobody has observed.
///
/// An empty leg is refused for the same reason and is louder: a removal job
/// with no items is created without complaint, reads back as settled because
/// zero settled of zero is complete, and lets `record_enqueued` mark the
/// request done naming a job that will never remove anything. The migrate
/// would degrade into a plain sync and say nothing, which is exactly what the
/// request key was split in two to prevent.
async fn enqueue_removal<A>(
    run: &ImportRun<'_, A>,
    record: &SyncRequestRecord,
    canonicalised: &[Canonicalisation],
) -> Result<Uuid, DrainError>
where
    A: FirstPartyExport,
{
    if canonicalised.is_empty() {
        return Err(DrainError::Locator(
            "a migrate's removal leg carries no resources, so the source listings would \
             survive a request recorded as enqueued"
                .to_owned(),
        ));
    }
    let mappings = MappingRepo::new(run.pool.clone());
    let mut items = Vec::new();
    let job = JobId(fresh_uuid());
    for row in canonicalised {
        let Some(state) = row.source_state else {
            return Err(DrainError::Locator(
                "the source read carried no lifecycle, so its removal would state one                  nobody observed"
                    .to_owned(),
            ));
        };
        // The policies and the price rule are the target mapping's, which the
        // canonicalisation just derived from this very read. Inventing a
        // second set here would be two answers to one question about one
        // product.
        let target = mappings
            .get(run.org, row.mapping)
            .await?
            .ok_or_else(|| DrainError::Locator("the canonicalised mapping vanished".to_owned()))?;
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
    let created = JobRepo::new(run.pool.clone())
        .create_with_request_key(
            run.org,
            job_request_key(record.id, REMOVE_LEG),
            &NewJob {
                job,
                inventory: record.source,
                at: run.now,
            },
            &items,
        )
        .await?;
    Ok(created.job.0)
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

/// The read the drain declares itself as. Every marketplace read in this
/// process is a first-party export of the seller's own catalogue.
#[must_use]
pub const fn reason_for(source: InventoryId) -> FetchReason {
    FetchReason::FirstPartyExport { inventory: source }
}

/// The intent version the ledger's keys are minted under, restated here for
/// the same reason the API restates it: it is a property of the key's
/// derivation and changing it is a deliberate re-keying of the whole ledger.
const INTENT_VERSION: u32 = 1;

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}
