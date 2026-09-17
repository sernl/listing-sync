//! The server half of the migration read: what a seller's own device observed
//! of their catalogue, applied.
//!
//! D27 puts the seller's file bytes on the seller's machine, so this route
//! receives a description and never the thing described. Each page carries what
//! one device saw of some resources — a listing read verbatim, a file's digest
//! and length and name, and a derived cover — and this applies them through the
//! same import preparation and application as the operator path, writing the payload as a
//! marketplace-sourced file that names where its bytes are and the cover as the
//! one blob Q-c allows us to keep.
//!
//! The completing page is what mints the write jobs, in the same transaction
//! that settles the request. That is not tidiness: a device gets one answer to
//! its page and a 200 is not retried, so a window where the job exists and the
//! request still reads `draining` would leave the seller's migration
//! permanently mid-flight beside a job nobody had told it about.
//!
//! What this route refuses is as much of its job as what it applies. The
//! vocabulary bounds every string it defines, and the two things it cannot
//! bound — the seller's own listing copy, and the scan signature `tam_types`
//! owns — are bounded here, because the server has never seen the device that
//! posts a page.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_engine_driver::import::{ImportPage, ObservedResource};
use tam_import::{
    apply_import, prepare_import, AppliedResource, HeldFile, ImportRun, ImportedFile,
    PreparedImport,
};
use tam_storage::{
    job_request_key, BlobRepo, Completion, DeviceRepo, Disposition, EventScope, JobOrigin, JobRepo,
    Mint, NewJob, Observed, ResourceAdmission, ResourceCoverage, SyncRequestRecord,
    SyncRequestRepo, CREATE_LEG, IMPORT_LEG,
};
use tam_types::{
    Actor, FileBytes, FileKind, JobEventPayload, JobId, Observation, OrgId, ScanOutcome, Stamp,
    SystemComponent, Timestamp, TransportClass, Uuid,
};

use crate::entitlement::feature_refusal;
use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::jobs::{missing, storage_fault, validation, workflow_deleted};
use crate::{AppState, OrgContext};

/// The longest listing copy this route will accept, in bytes.
///
/// The seller's own title and body are free text and no vocabulary can bound
/// them, so they are bounded here instead. Generous against any real listing
/// and small against a payload: a body at this size is prose, and a body an
/// order of magnitude beyond it is something else wearing a body's name.
const COPY_MAX: usize = 64 * 1024;

/// The longest scan signature this route will accept.
///
/// `ScanOutcome::Infected { signature }` is `tam_types`' own shape, shared with
/// paths that predate this one, so narrowing the type would reach further than
/// this vocabulary. It is bounded at the one place a device can put a value
/// into it.
const SIGNATURE_MAX: usize = 200;

/// What the server did with one page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportAck {
    /// Resources this page added to the request. A re-posted page reports
    /// zero, which is how a device tells a retry from a first delivery.
    pub applied: u32,
    pub skipped: u32,
    /// The request's totals, so a resumed device knows where it is without a
    /// second call.
    pub described_total: u32,
    /// The create job, once a completing page has minted it. `null` on every
    /// page before the last, and on a completion that had nothing to publish.
    pub create_job: Option<Uuid>,
    pub complete: bool,
}

pub(crate) async fn import_page(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(page): Json<ImportPage>,
) -> Result<(StatusCode, Json<ImportAck>), APIError> {
    // Before the blob check, because a plan that does not read shops is a
    // refusal the seller can act on and a missing object store is not.
    if !context.entitlement.caps.import_marketplace {
        return Err(feature_refusal(
            "import_marketplace",
            "Your plan does not include reading your shop. Upgrade to import from a \
             marketplace.",
        ));
    }
    let blobs = state.blobs.clone().ok_or_else(|| {
        APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new(
                "this deployment holds no key-encryption key or object-store root, so no cover \
                 could be stored; no part of this page was applied",
            )
            .code(APIErrorCode::BlobStoreUnavailable)
            .kind(APIErrorKind::Internal),
        )
    })?;
    let now = (state.wall)();
    admissible_device(&state, context.org, &device).await?;
    for resource in &page.resources {
        bounded_copy(resource)?;
    }

    // Which import this page belongs to, and the two are not interchangeable.
    // A page naming a `run` is phase 2's own: it is reviewed before anything
    // is created, and it drafts nowhere. A page naming a `request` is the
    // migrate leg, which creates as it goes and mints a create job on its
    // completing page. A device posts one or the other; naming neither is a
    // page with no import to belong to.
    if let Some(request) = page.request {
        return legacy_page(state, context, device, page, blobs, request, now).await;
    }
    // A catalogue page must name the attempt its device was granted, and an
    // unfenced one is refused here — before the cover is stored, before the
    // matcher is asked, before anything is written. An old client cannot be
    // given a default attempt: a default is a fence nobody granted, and a
    // page under it cannot be told from one a taken-over phone sent. The
    // separately authorised migration path above is untouched, which is what
    // keeps an in-flight migration from an older build working.
    if page.attempt.is_none() {
        return Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(
                "this app is too old to import into your catalogue: update it and start the \
                 import again",
            )
            .code(APIErrorCode::ImportClientUpdateRequired)
            .kind(APIErrorKind::Validation),
        ));
    }
    crate::import_runs::run_page(&state, &context, &device, &page, now).await
}

/// The migrate leg's own page handling, unchanged.
///
/// Kept whole rather than folded into the run path, and the reason is phase 3:
/// a migration creates on the target as it reads, so its pages apply
/// immediately and its completing page mints the create job. Nothing about
/// that is a special case of a catalogue-only run, and making one shape serve
/// both would put a review in front of a migration that has no use for one.
#[allow(clippy::too_many_lines)]
#[expect(
    clippy::too_many_arguments,
    reason = "the page, its device, its request and the two things the common gates already \
              resolved; a struct over them would name this call's argument list and nothing \
              else, and the split exists precisely so the migrate leg stays one function"
)]
async fn legacy_page(
    state: AppState,
    context: OrgContext,
    device: String,
    page: ImportPage,
    blobs: crate::BlobStore,
    request: Uuid,
    now: Timestamp,
) -> Result<(StatusCode, Json<ImportAck>), APIError> {
    let requests = SyncRequestRepo::new(state.pool.clone());
    let record = requests
        .get(context.org, request)
        .await
        .map_err(|error| storage_fault(&state, &error))?
        // Another organisation's request is missing rather than forbidden: the
        // caller learns nothing about whether the id exists, which is the
        // posture every other org-scoped read here takes.
        .ok_or_else(|| missing("no such sync request"))?;
    let device_enumerated = record.source.marketplace().transport_class()
        == TransportClass::SellerDevice
        && record.disposition == Disposition::Migrate;
    if !device_enumerated {
        return Err(validation(
            "this request is not one a device enumerates: its source marketplace publishes an \
             official API, or it is a sync rather than a migrate",
        ));
    }

    // A page that says the import stopped. Settled here, before anything else
    // is considered, because the device sends nothing else with it: the seller
    // reads the request's own page, and a terminal failure that posted no
    // resources leaves that page holding nothing at all unless this writes the
    // reason into it. Completion is implied — a stopped import is over — so
    // there is no state in which a request is both failed and still expecting
    // pages.
    if let Some(why) = page.failed.as_ref() {
        if settled(&record) {
            // Already terminal. A late failure report does not reopen a
            // request that completed, and does not overwrite the reason a
            // request already failed with.
            return Ok((StatusCode::OK, Json(ack(&record, 0, 0, true))));
        }
        requests
            .record_failure(context.org, request, why.as_str(), now)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        let settled = requests
            .get(context.org, request)
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .ok_or_else(|| missing("no such sync request"))?;
        return Ok((StatusCode::OK, Json(ack(&settled, 0, 0, true))));
    }

    // What the request already knows about. Read once per page rather than once
    // per resource, and consulted BEFORE anything is canonicalised: `import_one`
    // mints a fresh product every time it runs, so a re-posted page checked
    // afterwards would leave a second product behind for every resource.
    //
    // Pending rows may be prelisted resources or interrupted admissions.
    // Neither is described yet; the admission fence below decides whether
    // this page may start or resume applying it.
    let described: Vec<&str> = record
        .resources
        .iter()
        .filter(|row| row.state != "pending")
        .map(|row| row.locator.as_str())
        .collect();
    let fresh = page
        .resources
        .iter()
        .filter(|resource| !described.contains(&resource.locator.as_str()))
        .count()
        + page
            .skipped
            .iter()
            .filter(|skip| !described.contains(&skip.locator.as_str()))
            .count();
    // Deletion settles the request before admitted resources can finish.
    // Only a replay naming a pending locator may reach the admission fence,
    // which distinguishes an interrupted admission from an unstarted row.
    let interrupted = settled(&record)
        && record.resources.iter().any(|row| {
            row.state == "pending"
                && page
                    .resources
                    .iter()
                    .any(|resource| resource.locator.as_str() == row.locator.as_str())
        });
    let resuming = interrupted
        && requests
            .deletion_status(context.org, request)
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .is_some();
    if settled(&record) && !resuming {
        if fresh == 0 {
            return Ok((StatusCode::OK, Json(ack(&record, 0, 0, false))));
        }
        return Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(
                "this import has already completed, and this page brings a resource it never \
                 described; a completed import is not extended",
            )
            .kind(APIErrorKind::Validation),
        ));
    }

    requests
        .mark_draining_if_pending(context.org, request)
        .await
        .map_err(|error| storage_fault(&state, &error))?;
    let anchor = anchor_job(&state, &record, now).await?;

    let run = ImportRun {
        request: Some(request),
        pool: state.pool.clone(),
        org: context.org,
        source: record.source,
        target: Some(record.target),
        now,
    };
    let repo = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone());

    let applying = Applying {
        run: &run,
        repo: &repo,
        request,
        device: &device,
    };
    let mut applied = 0u32;
    // Recovery may finish prior admissions, but cannot extend or complete
    // the deleted request.
    let mut stopped = resuming;
    for resource in &page.resources {
        if described.contains(&resource.locator.as_str()) {
            continue;
        }
        match apply_one(&state, &applying, resource).await? {
            ResourceApply::Applied => applied = applied.saturating_add(1),
            ResourceApply::AlreadyDescribed => {}
            ResourceApply::Stopped => {
                stopped = true;
                // Recovery must search past refused locators to find its prior admission.
                if !resuming {
                    break;
                }
            }
        }
    }

    // A stopped request takes no further breadcrumbs either. A skip creates
    // nothing, so this is not a fence — it is not writing onto a row the
    // seller has already had settled with the reason it ended.
    let mut skipped = 0u32;
    for skip in &page.skipped {
        if stopped || described.contains(&skip.locator.as_str()) {
            continue;
        }
        if requests
            .append_skipped(
                context.org,
                request,
                skip.locator.as_str(),
                skip.why.as_str(),
            )
            .await
            .map_err(|error| storage_fault(&state, &error))?
        {
            skipped = skipped.saturating_add(1);
        }
    }

    JobRepo::new(state.pool.clone())
        .record_event(
            &EventScope {
                org: context.org,
                job: anchor,
                item: None,
            },
            &JobEventPayload::ImportPageApplied {
                request,
                described: applied,
                skipped,
            },
            Stamp::system(SystemComponent::Import, now),
        )
        .await
        .map_err(|error| storage_fault(&state, &error))?;

    if stopped {
        // Completion would mint exactly the create job the seller stopped,
        // so it is not attempted. The request already records why it ended;
        // the device is told what this page managed and that there is
        // nothing further to send.
        let after = requests
            .get(context.org, request)
            .await
            .map_err(|error| storage_fault(&state, &error))?;
        return match after {
            Some(after) => Ok((StatusCode::OK, Json(ack(&after, applied, skipped, true)))),
            // Retired between the refusal and this read, which is the same
            // answer with nothing left to read it from.
            None => Err(APIError::new(
                StatusCode::CONFLICT,
                APIErrorEntry::new(
                    "this migration was deleted while its import was running, so nothing \
                     further was imported for it",
                )
                .kind(APIErrorKind::Validation),
            )),
        };
    }
    if !page.complete {
        let after = requests
            .get(context.org, request)
            .await
            .map_err(|error| storage_fault(&state, &error))?
            .ok_or_else(|| missing("no such sync request"))?;
        return Ok((StatusCode::OK, Json(ack(&after, applied, skipped, false))));
    }
    complete(&state, &run, request, anchor).await
}

/// Settles the request and mints its create job, in one transaction.
async fn complete(
    state: &AppState,
    run: &ImportRun,
    request: Uuid,
    anchor: JobId,
) -> Result<(StatusCode, Json<ImportAck>), APIError> {
    let requests = SyncRequestRepo::new(state.pool.clone());
    let record = requests
        .get(run.org, request)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such sync request"))?;
    let mappings: Vec<tam_types::MappingId> = record
        .resources
        .iter()
        .filter_map(|row| row.mapping)
        .collect();

    let job = JobId(fresh_uuid());
    let items = if mappings.is_empty() {
        Vec::new()
    } else {
        tam_import::create_items(run, record.intent, job, &mappings)
            .await
            .map_err(|error| match error {
                tam_import::ImportError::Storage(error) => storage_fault(state, &error),
                // The seller asked for a state this target has no captured
                // route to, which is theirs to change rather than a fault.
                tam_import::ImportError::Lowering(refusal) => validation(&refusal.to_string()),
                // `create_items` lowers and derives keys; it reads no price,
                // ingests no payload and resolves no currency. These arms
                // exist because the error type is wider than this call.
                impossible @ (tam_import::ImportError::NoPayload
                | tam_import::ImportError::Price(_)
                | tam_import::ImportError::CurrencyUnknown { .. }
                | tam_import::ImportError::NoTarget) => {
                    state.internal(&format!("the create items answered {impossible}"))
                }
            })?
    };
    crate::consent::require_grant(state, run.org, record.target.marketplace()).await?;
    let new = NewJob {
        job,
        inventory: record.target,
        stamp: Stamp {
            at: run.now,
            actor: Actor::System(SystemComponent::Import),
        },
    };
    // An empty catalogue completes and mints nothing. An itemless job reads
    // back settled, because zero settled of zero is complete, so creating one
    // would report a migration finished that never had anything in it.
    let mint = (!items.is_empty()).then(|| Mint {
        request_key: job_request_key(request, CREATE_LEG),
        job: &new,
        items: &items,
    });
    let created = requests
        .complete_with_create_job(
            run.org,
            &Completion {
                request,
                at: run.now,
                mint,
            },
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;

    let settled = requests
        .get(run.org, request)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .ok_or_else(|| missing("no such sync request"))?;
    let described = u32::try_from(
        settled
            .resources
            .iter()
            .filter(|row| row.state != "failed")
            .count(),
    )
    .unwrap_or(u32::MAX);
    let skipped = u32::try_from(
        settled
            .resources
            .iter()
            .filter(|row| row.state == "failed")
            .count(),
    )
    .unwrap_or(u32::MAX);
    JobRepo::new(state.pool.clone())
        .record_event(
            &EventScope {
                org: run.org,
                job: anchor,
                item: None,
            },
            &JobEventPayload::ImportCompleted {
                request,
                create_job: created.map(|job| job.job),
                described,
                skipped,
            },
            Stamp::system(SystemComponent::Import, run.now),
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    Ok((
        StatusCode::OK,
        Json(ImportAck {
            applied: 0,
            skipped: 0,
            described_total: described,
            create_job: created.map(|job| job.job.0),
            complete: true,
        }),
    ))
}

/// Everything one page holds constant while its resources are applied.
struct Applying<'a> {
    run: &'a ImportRun,
    repo: &'a BlobRepo<tam_blob_store::AnyObjectStore>,
    request: Uuid,
    /// Which machine reported this, recorded as the device's own assertion
    /// beside our receipt rather than restated as something we verified.
    device: &'a str,
}

enum ResourceApply {
    Applied,
    AlreadyDescribed,
    Stopped,
}

async fn apply_one(
    state: &AppState,
    applying: &Applying<'_>,
    resource: &ObservedResource,
) -> Result<ResourceApply, APIError> {
    let run = applying.run;
    let requests = SyncRequestRepo::new(state.pool.clone());
    // Admit before storing the cover or preparing a projection, so deletion
    // observes work that has started even before its catalogue transaction.
    let ordinal = match requests
        .admit_resource(
            run.org,
            applying.request,
            resource.locator.as_str(),
            run.now,
        )
        .await
        .map_err(|error| storage_fault(state, &error))?
    {
        ResourceAdmission::Admitted(ordinal) => ordinal,
        ResourceAdmission::AlreadyDescribed => return Ok(ResourceApply::AlreadyDescribed),
        ResourceAdmission::Stopped => return Ok(ResourceApply::Stopped),
    };
    let result = apply_admitted(state, applying, resource).await;
    if let Err(refusal) = &result {
        let why = refusal
            .errors
            .first()
            .map_or("this resource could not be imported", |entry| {
                entry.message.as_str()
            });
        requests
            .record_resource_failure(run.org, applying.request, ordinal, why)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    }
    result
}

/// Prepares one observed resource without holding an application transaction.
///
/// The payload is `Sourced` and names where the seller's bytes are; the cover
/// is the one thing we keep, stored as a held blob directly rather than through
/// the ingest pipeline, because the pipeline would derive a cover from the
/// cover.
async fn prepare_resource(
    state: &AppState,
    applying: &Applying<'_>,
    resource: &ObservedResource,
) -> Result<PreparedImport, APIError> {
    let run = applying.run;
    // A migration moves a file to another marketplace, so a read that named
    // none has nothing to migrate. Refused per resource rather than per page:
    // one unreadable resource in a shop of hundreds costs that resource.
    let (Some(file), Some(cover_png)) = (resource.file.as_ref(), resource.cover_png.as_ref())
    else {
        return Err(validation(
            "this resource carries no file, so there is nothing to move to another \
             marketplace; import it to your catalogue instead",
        ));
    };
    let connection = source_connection(state, run.org, run.source, run.now).await?;
    let hash = applying
        .repo
        .put(run.org, cover_png.bytes(), run.now)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let cover_len = u64::try_from(cover_png.bytes().len()).unwrap_or(u64::MAX);

    let applied = AppliedResource {
        // The row report's own handle on the resource. A locator is not
        // inherently numeric — a Tes resource is a URL — so an unparseable one
        // reports zero rather than refusing an import over a display value.
        resource: resource.locator.as_str().parse().unwrap_or(0),
        product: tam_types::ProductId(fresh_uuid()),
        listing: resource.listing.clone(),
        payload: vec![ImportedFile {
            kind: file.kind,
            bytes: FileBytes::Sourced {
                marketplace: run.source.marketplace(),
                connection,
                resource: resource.locator.as_str().to_owned(),
                entry: file.entry.as_ref().map(|e| e.as_str().to_owned()),
                payload_file_name: file.payload_file_name.as_str().to_owned(),
                payload_content_type: file.payload_content_type.as_str().to_owned(),
                observed: Observation {
                    device: applying.device.to_owned(),
                    hash: file.hash,
                    byte_len: file.byte_len,
                    scan: file.scan.clone(),
                    observed_at: run.now,
                },
            },
        }],
        cover: Some(HeldFile {
            kind: FileKind::Image,
            hash,
            byte_len: cover_len,
            scan: ScanOutcome::Clean { at: run.now },
        }),
    };
    prepare_import(run, &applied)
        .await
        .map_err(|error| import_fault(state, error))
}

async fn apply_admitted(
    state: &AppState,
    applying: &Applying<'_>,
    resource: &ObservedResource,
) -> Result<ResourceApply, APIError> {
    let run = applying.run;
    let prepared = prepare_resource(state, applying, resource).await?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|error| storage_fault(state, &error.into()))?;
    tam_storage::pin_tenant(&mut tx, run.org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    // The request lock covers the product, mappings and receipt. A concurrent
    // replay either sees the receipt or takes over a fully rolled-back apply.
    match SyncRequestRepo::admit_resource_in(
        &mut tx,
        run.org,
        applying.request,
        resource.locator.as_str(),
        run.now,
    )
    .await
    .map_err(|error| storage_fault(state, &error))?
    {
        ResourceAdmission::Admitted(_) => {}
        ResourceAdmission::AlreadyDescribed => return Ok(ResourceApply::AlreadyDescribed),
        ResourceAdmission::Stopped => return Ok(ResourceApply::Stopped),
    }
    let report = apply_import(&mut tx, run.org, &prepared, run.now)
        .await
        .map_err(|error| import_fault(state, error))?;
    SyncRequestRepo::append_observed(
        &mut tx,
        run.org,
        applying.request,
        &Observed {
            locator: resource.locator.as_str(),
            product: report.product,
            mapping: report.mapping.ok_or_else(|| {
                state.internal("a migrate import minted no mapping for its target")
            })?,
            source: &report.source,
            source_state: report.source_state,
            coverage: ResourceCoverage {
                terms_seen: u32::try_from(report.terms_seen).unwrap_or(u32::MAX),
                terms_mapped: u32::try_from(report.terms_mapped).unwrap_or(u32::MAX),
                terms_unmapped: u32::try_from(report.unmapped_native_ids.len()).unwrap_or(u32::MAX),
                terms_uncovered: u32::try_from(report.terms_uncovered).unwrap_or(u32::MAX),
            },
        },
    )
    .await
    .map_err(|error| storage_fault(state, &error))?;
    tx.commit()
        .await
        .map_err(|error| storage_fault(state, &error.into()))?;
    Ok(ResourceApply::Applied)
}

fn import_fault(state: &AppState, error: tam_import::ImportError) -> APIError {
    match error {
        tam_import::ImportError::Storage(error) => storage_fault(state, &error),
        refused @ (tam_import::ImportError::NoPayload
        | tam_import::ImportError::Price(_)
        | tam_import::ImportError::CurrencyUnknown { .. }) => validation(&refused.to_string()),
        // Applying a resource never lowers a publishing intent.
        impossible @ (tam_import::ImportError::Lowering(_) | tam_import::ImportError::NoTarget) => {
            state.internal(&format!("the import answered {impossible}"))
        }
    }
}

/// Which of the seller's connections can fetch this resource's bytes.
pub(crate) async fn source_connection(
    state: &AppState,
    org: OrgId,
    source: tam_types::InventoryId,
    now: Timestamp,
) -> Result<tam_types::ConnectionId, APIError> {
    let marketplace = source.marketplace();
    tam_storage::ConnectionRepo::new(state.pool.clone())
        .list(org, now)
        .await
        .map_err(|error| storage_fault(state, &error))?
        .into_iter()
        .find(|row| row.marketplace == marketplace && row.state == "linked")
        .map(|row| row.id)
        .ok_or_else(|| {
            validation(
                "this seller has no linked connection for the source marketplace, so a file \
                 named in it could never be fetched back",
            )
        })
}

/// The itemless job an import's events hang from, found or created.
///
/// Keyed on the request, so every page of one import reaches the same job
/// rather than minting one each.
///
/// The request link routes anchor deletion to the owning migration and puts
/// new anchors behind its deletion fence. Otherwise an itemless anchor could
/// disappear while the device continued importing resources.
///
/// An existing request key still resolves its tombstoned anchor, allowing a
/// replay to finish an earlier resource admission without minting new work.
async fn anchor_job(
    state: &AppState,
    record: &SyncRequestRecord,
    now: Timestamp,
) -> Result<JobId, APIError> {
    crate::consent::require_grant(state, record.org, record.source.marketplace()).await?;
    let minted = JobRepo::new(state.pool.clone())
        .create_with_request_key(
            record.org,
            JobOrigin {
                request_key: job_request_key(record.id, IMPORT_LEG),
                run: Some(record.id),
                import_run: None,
            },
            &NewJob {
                job: JobId(fresh_uuid()),
                inventory: record.source,
                stamp: Stamp {
                    at: now,
                    actor: Actor::System(SystemComponent::Import),
                },
            },
            &[],
        )
        .await
        .map_err(|error| storage_fault(state, &error))?;
    match minted {
        tam_storage::Minted::Job(created) => Ok(created.job),
        // The seller stopped the migration between this page's own read of
        // the request and this mint. There is nothing to anchor.
        tam_storage::Minted::WorkflowDeleted(workflow) => Err(workflow_deleted(workflow)),
    }
}

/// The device must be this organisation's and must not be revoked.
pub(crate) async fn admissible_device(
    state: &AppState,
    org: OrgId,
    device: &str,
) -> Result<(), APIError> {
    let devices = DeviceRepo::new(state.pool.clone())
        .list(org)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let known = devices
        .iter()
        .find(|candidate| candidate.id == device)
        .ok_or_else(|| missing("no such device"))?;
    if known.revoked_at.is_some() {
        return Err(APIError::new(
            StatusCode::FORBIDDEN,
            APIErrorEntry::new("this device is revoked and may not report a catalogue")
                .kind(APIErrorKind::Validation),
        ));
    }
    Ok(())
}

/// The two strings the page vocabulary cannot bound, bounded here.
fn bounded_copy(resource: &ObservedResource) -> Result<(), APIError> {
    if resource.listing.title.len() > COPY_MAX || resource.listing.body.len() > COPY_MAX {
        return Err(validation(
            "a listing's title and body are the seller's own copy and are bounded: this page \
             carries one beyond that bound",
        ));
    }
    if let Some(ScanOutcome::Infected { signature }) = resource.file.as_ref().map(|file| &file.scan)
    {
        if signature.len() > SIGNATURE_MAX {
            return Err(validation(
                "a scan signature is a signature and this one is longer than any",
            ));
        }
    }
    Ok(())
}

/// Whether the request has already reached a terminal state.
fn settled(record: &SyncRequestRecord) -> bool {
    record.state == "enqueued" || record.state == "failed"
}

fn ack(record: &SyncRequestRecord, applied: u32, skipped: u32, complete: bool) -> ImportAck {
    let described = u32::try_from(
        record
            .resources
            .iter()
            .filter(|row| row.state != "failed")
            .count(),
    )
    .unwrap_or(u32::MAX);
    ImportAck {
        applied,
        skipped,
        described_total: described,
        create_job: record.create_job,
        complete,
    }
}

fn fresh_uuid() -> Uuid {
    Uuid(*uuid::Uuid::new_v4().as_bytes())
}
