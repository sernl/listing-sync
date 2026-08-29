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

use tam_domain::{ItemOperation, JobItemId};
use tam_import::{import_one, ImportEntry, ImportError, ImportRun, NamedBytes};
use tam_marketplace::{FetchReason, FirstPartyExport};
use tam_storage::{
    job_request_key, Canonicalised, Disposition, Enqueued, JobReadRepo, JobRepo, NewJob,
    NewJobItem, StorageError, SyncRequestRecord, SyncRequestRepo, CREATE_LEG, REMOVE_LEG,
};
use tam_types::{InventoryId, JobId, OrgId, Uuid};

/// What one request's drain produced, so a caller reports it rather than
/// reading it back out of the row it just wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainReport {
    pub request: Uuid,
    pub canonicalised: usize,
    pub skipped: usize,
    pub failed: usize,
    pub create_job: Option<Uuid>,
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
    };
    let mut mappings = Vec::new();
    for resource in &record.resources {
        if resource.is_canonicalised() {
            report.skipped += 1;
            if let Some(mapping) = resource.mapping {
                mappings.push(mapping);
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
                            product: row.0,
                            mapping: row.1,
                        },
                    )
                    .await?;
                mappings.push(row.1);
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
    report.create_job = Some(create_job.0);
    requests
        .record_enqueued(
            run.org,
            &Enqueued {
                request,
                create_job: Some(create_job.0),
                remove_job: None,
                at: run.now,
            },
        )
        .await?;
    Ok(report)
}

async fn canonicalise_one<A>(
    run: &ImportRun<'_, A>,
    locator: &str,
    reason: &FetchReason,
) -> Result<(tam_types::ProductId, tam_types::MappingId), DrainError>
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
    Ok((row.product, row.mapping))
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
) -> Result<(Uuid, bool), DrainError>
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
            idempotency_key: tam_marketplace::idempotency::derive_idempotency_key(
                run.org,
                record.target,
                seed.product,
                INTENT_VERSION,
                tam_storage::job_reads::intent_digest(
                    &ItemOperation::Create,
                    job,
                    &seed.payload_hashes,
                ),
            ),
            operation: ItemOperation::Create,
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
    Ok((created.job.0, created.replay))
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
