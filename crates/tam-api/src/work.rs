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
use axum::http::StatusCode;
use axum::Json;
use tam_engine::ledger::to_wire_item;
use tam_engine::seed::{preparation as preparation_for, prepare_item, ItemPreparation};
use tam_engine_driver::vocabulary::{ClaimView, PayloadManifest, SettleEnvelope, WorkOrder};
use tam_storage::{describe_files, DeviceClaim, DeviceRef, LeaseRepo};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
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
