//! The device's pull, and the settle that closes it.
//!
//! This is D1's declarative-intent surface. The server answers what is due, in
//! terms of an outcome the device's own adapter composes a request for, and it
//! never says now: the seller's local timer decides when to ask. Nothing in
//! the envelope is a URL, a header, a form field name or an encoding, which is
//! the property that keeps this at S1 rather than S3 in the legal-control
//! spectrum — if a field here could not be computed without composing a
//! marketplace request, the boundary has moved.
//!
//! Every route is org-scoped through [`OrgContext`], so the request carries no
//! organisation identifier a caller could substitute, and the claim is pinned
//! again in SQL beneath that.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::Json;
use tam_engine::ledger::{to_wire_item, PgLedger};
use tam_engine::seed::{preparation as preparation_for, prepare_item, ItemPreparation};
use tam_engine_driver::vocabulary::{
    ClaimView, LeaseRef, LedgerAnswer, LedgerCall, LedgerError, PayloadManifest, SettleEnvelope,
    WorkOrder,
};
use tam_pipeline::store::LocalObjectStore;
use tam_storage::{describe_files, BlobRepo, DeviceClaim, DeviceRef, LeaseRepo};
use tam_types::Timestamp;

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// How long a device's claim stands before the reaper may steal it.
const CLAIM_TTL_SECS: i64 = 300;

/// How long a lapsed plan keeps working. D11's grace, stated once.
const ENTITLEMENT_GRACE_HOURS: i64 = 24;

/// The jittered wait before the next ask. The server suggests; the device's own
/// timer decides, which is the half of the cron move that matters legally.
const fn next_poll_ms(held: bool) -> u64 {
    if held {
        30_000
    } else {
        10_000
    }
}

/// The device asks for work.
pub(crate) async fn claim(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
) -> Result<Json<ClaimView>, APIError> {
    let now = (state.wall)();
    let claimed = LeaseRepo::new(state.pool.clone())
        .claim_for_device(
            &DeviceRef {
                org: context.org,
                device: &device,
            },
            CLAIM_TTL_SECS,
            ENTITLEMENT_GRACE_HOURS,
            now,
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let leased = match claimed {
        DeviceClaim::Empty => {
            return Ok(Json(ClaimView::Idle {
                next_poll_ms: next_poll_ms(false),
            }))
        }
        DeviceClaim::HeldByAnotherDevice => {
            return Ok(Json(ClaimView::Held {
                next_poll_ms: next_poll_ms(true),
            }))
        }
        DeviceClaim::Leased(item) => *item,
    };
    // The whole taxonomy stays here: `prepare_item` writes the seller's
    // decision surface and answers a projected listing, so D1's declarative
    // intent is literally the return value of one server-side call.
    // The storage row crosses into the interpreter's vocabulary once, here,
    // through the same conversion the worker uses.
    let driven = to_wire_item(&leased);
    let prepared = prepare_item(&state.pool, &leased, now)
        .await
        .map_err(|error| state.internal(&format!("{error:?}")))?;
    let (operation, projected) = match prepared {
        ItemPreparation::Ready {
            operation,
            projected,
        } => (operation, projected),
        // A blocked item, or one whose counterpart can never bind, is not the
        // device's to run. Both are already recorded server-side, so the
        // device is told the queue is idle rather than handed work it would
        // only park again.
        ItemPreparation::Blocked { .. } | ItemPreparation::CounterpartLost { .. } => {
            return Ok(Json(ClaimView::Idle {
                next_poll_ms: next_poll_ms(false),
            }))
        }
    };
    // The server commits to the bytes before they move: the device fetches
    // them separately and checks what arrived against these hashes and
    // lengths, so a truncated transfer is caught on the device.
    let preparation = preparation_for(&leased, operation, projected);
    let payload = match preparation.projected.as_ref() {
        Some(listing) => describe_files(&state.pool, context.org, &listing.files)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
            .into_iter()
            .map(|file| PayloadManifest {
                file: file.id,
                file_name: file.file_name,
                content_type: file.content_type,
                hash: file.hash,
                byte_len: file.byte_len,
            })
            .collect(),
        None => Vec::new(),
    };
    Ok(Json(ClaimView::Work(Box::new(WorkOrder {
        lease: driven,
        preparation,
        payload,
        server_now_ms: now.0,
        server_deadline_ms: now.0 + CLAIM_TTL_SECS * 1_000,
        next_poll_ms: next_poll_ms(false),
    }))))
}

/// The device settles what it claimed.
///
/// The epoch is the fence and the device id is the holder: a settle from a
/// device other than the one holding the lease is refused, because the write
/// it is asking for belongs to a run it is not in.
pub(crate) async fn settle(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(body): Json<SettleEnvelope>,
) -> Result<StatusCode, APIError> {
    let holder: Option<String> = sqlx::query_scalar(
        "SELECT lease_owner FROM job_item \
         WHERE org_id = $1 AND lease_epoch = $2 AND state IN ('leased', 'running', 'verifying') \
         LIMIT 1",
    )
    .bind(uuid::Uuid::from_bytes(context.org.0 .0))
    .bind(body.lease.lease_epoch)
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| state.internal(&error.to_string()))?
    .flatten();
    if holder.as_deref() != Some(device.as_str()) {
        // 409 with the validation kind rather than a conflict kind: the kind
        // vocabulary is a closed set that regenerates the client's `vocab.ts`,
        // and widening it is not this change's to make.
        return Err(APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(
                "that lease is held by another device, so this settle is not yours to make",
            )
            .kind(APIErrorKind::Validation),
        ));
    }
    Ok(StatusCode::ACCEPTED)
}

/// The bytes of one file the device is about to upload.
///
/// This is D27's interim path, and it is interim on purpose: the memo's target
/// is client-side ingest, where the seller's own machine holds the bytes and
/// the server never has them. Until that exists the bytes are ours and the
/// device fetches them here, which is why this route is guarded as tightly as
/// it is.
///
/// Three things gate it. The device must hold a live lease; the item that lease
/// names must reference this file in its projection; and the file must be this
/// organisation's. The response carries the bytes and nothing else — no digest,
/// no name, no type — because the manifest in the work order is the
/// commitment, and a second copy travelling beside the bytes would be a second
/// thing to disagree with.
pub(crate) async fn payload(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device, file)): Path<(String, String, String)>,
) -> Result<([(header::HeaderName, &'static str); 1], Vec<u8>), APIError> {
    let Some(blobs) = state.blobs.clone() else {
        return Err(APIError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            APIErrorEntry::new("this deployment holds no object store")
                .kind(APIErrorKind::Internal),
        ));
    };
    // Parsed here rather than extracted, so a malformed id is a validation
    // answer rather than a route that did not match.
    let file = uuid::Uuid::parse_str(&file).map_err(|_| {
        APIError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            APIErrorEntry::new("a file id must be a uuid").kind(APIErrorKind::Validation),
        )
    })?;
    let file = tam_types::FileId(tam_types::Uuid(*file.as_bytes()));
    // The lease is the authorisation, and it is read rather than asserted: a
    // device with no live lease has no business fetching a seller's files.
    let held = live_lease_files(&state, context.org, &device).await?;
    if !held.contains(&file) {
        return Err(APIError::new(
            StatusCode::FORBIDDEN,
            APIErrorEntry::new(
                "this device holds no live lease on an item whose projection names that file",
            )
            .kind(APIErrorKind::Validation),
        ));
    }
    let described = describe_files(&state.pool, context.org, &[file])
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let Some(described) = described.into_iter().next() else {
        return Err(APIError::new(
            StatusCode::NOT_FOUND,
            APIErrorEntry::new("no such file")
                .code(APIErrorCode::ResourceMissing)
                .kind(APIErrorKind::NotFound),
        ));
    };
    let bytes = BlobRepo::new(
        state.pool.clone(),
        LocalObjectStore::new(blobs.root.clone()),
        blobs.kek.clone(),
    )
    .get(context.org, described.hash)
    .await
    .map_err(|error| state.internal(&format!("{error:?}")))?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes))
}

/// The files every item this device currently holds a live lease on projects.
///
/// Read through `prepare_item` rather than from a stored list, because the
/// projection is what the work order committed to and a second source could
/// name a file the order never did.
async fn live_lease_files(
    state: &AppState,
    org: tam_types::OrgId,
    device: &str,
) -> Result<Vec<tam_types::FileId>, APIError> {
    let held: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM job_item \
         WHERE org_id = $1 AND lease_owner = $2 \
           AND state IN ('leased', 'running', 'verifying')",
    )
    .bind(uuid::Uuid::from_bytes(org.0 .0))
    .bind(device)
    .fetch_all(&state.pool)
    .await
    .map_err(|error| state.internal(&error.to_string()))?;
    let mut files = Vec::new();
    for item in held {
        let leased = LeaseRepo::new(state.pool.clone())
            .leased_item(
                org,
                tam_domain::JobItemId(tam_types::Uuid(*item.as_bytes())),
            )
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        let Some(leased) = leased else { continue };
        if let Ok(ItemPreparation::Ready {
            projected: Some(listing),
            ..
        }) = prepare_item(&state.pool, &leased, (state.wall)()).await
        {
            files.extend(listing.files);
        }
    }
    Ok(files)
}

/// One ledger call from the device, under the same fences the settle takes.
///
/// The call names the lease it speaks for and nothing else about whose work it
/// is: the organisation comes from the session, the holder is checked against
/// the lease, and the epoch is the fence. A call naming a lease this device
/// does not hold is refused before anything is dispatched.
///
/// `LedgerError` comes back as a [`LedgerAnswer::Refused`] rather than as a
/// fault, because the interpreter branches on two of its cases: a device that
/// saw `AttemptInFlight` as an HTTP error would retry the duplicate-create
/// fence it must not retry.
pub(crate) async fn ledger(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(call): Json<LedgerCall>,
) -> Result<Json<LedgerAnswer>, APIError> {
    let lease = *call.lease();
    let leased = held_by(&state, context.org, &device, lease).await?;
    let ledger = PgLedger::for_device(state.pool.clone(), leased.job, device);
    let answer = dispatch(&ledger, call).await;
    Ok(Json(match answer {
        Ok(answer) => answer,
        Err(error) => LedgerAnswer::Refused { error },
    }))
}

/// The lease this call names, provided this device is its holder.
async fn held_by(
    state: &AppState,
    org: tam_types::OrgId,
    device: &str,
    lease: LeaseRef,
) -> Result<tam_storage::LeasedItem, APIError> {
    let refusal = || {
        APIError::new(
            StatusCode::CONFLICT,
            APIErrorEntry::new(
                "that lease is held by another device, so this call is not yours to make",
            )
            .kind(APIErrorKind::Validation),
        )
    };
    let leased = LeaseRepo::new(state.pool.clone())
        .leased_item(org, lease.item)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(refusal)?;
    let holder: Option<String> =
        sqlx::query_scalar("SELECT lease_owner FROM job_item WHERE org_id = $1 AND id = $2")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(uuid::Uuid::from_bytes(lease.item.0 .0))
            .fetch_optional(&state.pool)
            .await
            .map_err(|error| state.internal(&error.to_string()))?
            .flatten();
    if holder.as_deref() != Some(device) || leased.lease_epoch != lease.lease_epoch {
        return Err(refusal());
    }
    Ok(leased)
}

/// The call, executed. One arm per variant, so a method added to the port
/// without a route to reach it is a compile error here.
async fn dispatch(ledger: &PgLedger, call: LedgerCall) -> Result<LedgerAnswer, LedgerError> {
    use tam_engine_driver::ports::ItemLedger;
    Ok(match call {
        LedgerCall::ConnectionFor { lease, inventory } => LedgerAnswer::Connection {
            connection: ledger.connection_for(&lease, inventory).await?,
        },
        LedgerCall::PreflightSucceeded { lease } => {
            ledger.preflight_succeeded(&lease).await?;
            LedgerAnswer::Done
        }
        LedgerCall::PreflightFailed { lease, edge_class } => LedgerAnswer::Streak {
            streak: ledger.preflight_failed(&lease, edge_class).await?,
        },
        LedgerCall::Park {
            lease,
            blocked_on,
            park_for_seconds,
        } => {
            ledger.park(&lease, &blocked_on, park_for_seconds).await?;
            LedgerAnswer::Done
        }
        LedgerCall::OpenAttempt { lease, new, at_ms } => {
            ledger.open_attempt(&lease, &new, Timestamp(at_ms)).await?;
            LedgerAnswer::Done
        }
        LedgerCall::SettleAttempt {
            lease,
            attempt,
            verdict,
            at_ms,
        } => LedgerAnswer::Bound {
            disposition: ledger
                .settle_attempt(&lease, attempt, &verdict, Timestamp(at_ms))
                .await?,
        },
        LedgerCall::GateConnection {
            lease,
            inventory,
            at_ms,
        } => {
            ledger
                .gate_connection(&lease, inventory, Timestamp(at_ms))
                .await?;
            LedgerAnswer::Done
        }
        LedgerCall::HaltThisTenant {
            lease,
            inventory,
            reason,
            at_ms,
        } => {
            ledger
                .halt_this_tenant(&lease, inventory, &reason, Timestamp(at_ms))
                .await?;
            LedgerAnswer::Done
        }
        LedgerCall::RequestGrant {
            lease,
            connection,
            kind,
            at_ms,
        } => LedgerAnswer::Grant {
            grant: ledger
                .request_grant(&lease, connection, kind, Timestamp(at_ms))
                .await?,
        },
        LedgerCall::RecordEvent {
            lease,
            payload,
            at_ms,
        } => {
            ledger
                .record_event(&lease, &payload, Timestamp(at_ms))
                .await?;
            LedgerAnswer::Done
        }
        LedgerCall::Notify {
            lease,
            event,
            at_ms,
        } => {
            ledger.notify(&lease, event, Timestamp(at_ms)).await?;
            LedgerAnswer::Done
        }
    })
}
