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
use tam_domain::LEASE_TTL_SECS;
use tam_engine::ledger::{to_storage_lease, to_wire_item, PgLedger};
use tam_engine::seed::{
    preparation as preparation_for, prepare_and_dispose, prepare_item, reconcile_is_available,
    Disposed, ItemPreparation,
};
use tam_engine_driver::vocabulary::{
    AttemptRef, Attestation, ClaimView, Committed, LeaseRef, LeasedItem, LedgerAnswer, LedgerCall,
    LedgerError, PayloadManifest, PayloadSource, ReconcileSubject, SettleEnvelope, WorkFilter,
    WorkOrder,
};
use tam_storage::{
    describe_files, BlobRepo, Charged, ClaimPolicy, ConnectionFactsRepo, DeviceClaim, DeviceRef,
    LeaseRepo,
};
use tam_types::{FailureDetail, FileBytes, FileId, Timestamp};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

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
    // Optional: a device that has gated its own readiness asks for the one
    // marketplace it is ready for, and a device that has not asks for
    // whatever is due. An absent or unparseable body means the latter, so a
    // client that sends nothing keeps the behaviour it had.
    filter: Option<Json<WorkFilter>>,
) -> Result<Json<ClaimView>, APIError> {
    let now = (state.wall)();
    let filter = filter.map(|Json(filter)| filter).unwrap_or_default();
    let claimed = LeaseRepo::new(state.pool.clone())
        .claim_for_device(
            &DeviceRef {
                org: context.org,
                device: &device,
            },
            &ClaimPolicy {
                ttl_seconds: i64::from(LEASE_TTL_SECS),
                grace_hours: i64::from(tam_domain::ENTITLEMENT_GRACE_HOURS),
                marketplace: filter.marketplace,
                // The engine owns the create strategy, so it owns whether a
                // stranded create is worth taking out of its park.
                reconcile: reconcile_is_available(),
            },
            now,
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let leased = match claimed {
        DeviceClaim::Empty => {
            return Ok(Json(ClaimView::Idle {
                next_poll_ms: next_poll_ms(false),
            }));
        }
        DeviceClaim::HeldByAnotherDevice => {
            return Ok(Json(ClaimView::Held {
                next_poll_ms: next_poll_ms(true),
            }));
        }
        DeviceClaim::Leased(item) => *item,
    };
    // The claim has taken the item, so nothing below may end while still
    // holding it. A refused preparation is parked or settled by the
    // disposition itself; a failure is handed back to the queue here, which
    // is the only exit that releases.
    let driven = to_wire_item(&leased);
    let lease = to_storage_lease(&driven.lease_ref());
    match work_order(&state, context.org, &leased, driven, now).await {
        Ok(Some(order)) => Ok(Json(ClaimView::Work(Box::new(order)))),
        // Parked or settled by the disposition rather than released: the lease
        // ends with the item, and handing it back to the queue is what would
        // serve it again on the next poll.
        Ok(None) => Ok(Json(ClaimView::Idle {
            next_poll_ms: next_poll_ms(false),
        })),
        Err(error) => {
            // The caller keeps the error it asked about, so a charge that
            // fails on top of it must not replace it.
            drop(charge(&state, &lease, now).await);
            Err(error)
        }
    }
}

/// Charges the attempt and hands back a claim this request could not serve.
async fn charge(
    state: &AppState,
    lease: &tam_storage::LeaseRef,
    now: Timestamp,
) -> Result<Charged, APIError> {
    let attempts_max = i32::try_from(tam_limits::job::ATTEMPTS_MAX).unwrap_or(i32::MAX);
    LeaseRepo::new(state.pool.clone())
        .charge_and_requeue(
            lease,
            attempts_max,
            Some(FailureDetail(
                "the item could not be prepared for this device".to_owned(),
            )),
            now,
        )
        .await
        .map_err(|error| state.internal(&error.to_string()))
}

/// The reconcile an order carries, where the claim found one.
///
/// Both or neither. An attempt whose intent named no title cannot be
/// identified by the catalogue walk, so an order carrying the attempt without
/// the title would send a device to search for nothing; it is not a reconcile
/// and the device runs it as the ordinary create it still is. The claim
/// already refuses to return one without the other, and this keeps the pairing
/// true of the type rather than of the query alone.
fn reconcile_subject(leased: &tam_storage::LeasedItem) -> Option<ReconcileSubject> {
    leased
        .stranded_attempt
        .zip(leased.stranded_title.clone())
        .map(|(attempt, title)| ReconcileSubject {
            attempt: AttemptRef {
                attempt,
                mapping: leased.mapping,
            },
            title,
        })
}

/// What the device is to do, or `None` where the item turned out not to be the
/// device's to run.
/// One described file as the work order states it.
///
/// The length is fallible rather than saturating, and the two arms differ in
/// what a failure would mean. On the held branch it is our own blob's length,
/// so a value that will not fit is a corrupt row. On the sourced branch it is
/// the number the device checks the fetched bytes against, and saturating it
/// to `i64::MAX` would hand the device a length nothing can match: a storage
/// fault would surface on the seller's machine as the marketplace having
/// served the wrong bytes. Refusing here says what actually happened.
fn manifest_for(file: tam_storage::StoredFile) -> Result<PayloadManifest, String> {
    let source = match file.bytes {
        FileBytes::Held { hash, byte_len, .. } => PayloadSource::ControlPlane {
            committed: Committed {
                hash,
                byte_len: length(byte_len, "a stored blob")?,
            },
        },
        FileBytes::Sourced {
            marketplace,
            resource,
            entry,
            observed,
            ..
        } => PayloadSource::Marketplace {
            marketplace,
            resource,
            entry,
            // The device verifies against what a device observed, never
            // against a commitment of ours, because we never held these bytes
            // to commit to them.
            expected: Some(Committed {
                hash: observed.hash,
                byte_len: length(observed.byte_len, "a device's observation")?,
            }),
        },
    };
    Ok(PayloadManifest {
        file: file.id,
        file_name: file.file_name,
        content_type: file.content_type,
        source,
    })
}

fn length(byte_len: u64, whose: &str) -> Result<i64, String> {
    i64::try_from(byte_len).map_err(|_| format!("{whose} states a byte length that cannot be sent"))
}

async fn work_order(
    state: &AppState,
    org: tam_types::OrgId,
    leased: &tam_storage::LeasedItem,
    driven: LeasedItem,
    now: Timestamp,
) -> Result<Option<WorkOrder>, APIError> {
    // The whole taxonomy stays here: `prepare_item` writes the seller's
    // decision surface and answers a projected listing, so D1's declarative
    // intent is literally the return value of one server-side call.
    let ledger = LeaseRepo::new(state.pool.clone());
    let disposed = prepare_and_dispose(&state.pool, &ledger, leased, now)
        .await
        .map_err(|error| state.internal(&format!("{error:?}")))?;
    let (operation, projected) = match disposed {
        Disposed::Ready {
            operation,
            projected,
        } => (operation, projected),
        // A blocked item, or one whose counterpart can never bind, is not the
        // device's to run. The disposition has already parked or settled it —
        // the same one the worker applies — so the device is told the queue is
        // idle rather than handed work it would only refuse again.
        Disposed::Parked { .. } | Disposed::Skipped { .. } => return Ok(None),
    };
    // The server commits to the bytes before they move: the device fetches
    // them separately and checks what arrived against these hashes and
    // lengths, so a truncated transfer is caught on the device. A
    // marketplace-sourced file is named by its locator instead: we hold no
    // blob to describe it from, and what the device checks against is the
    // digest another device asserted rather than one we committed to.
    let preparation = preparation_for(leased, operation, projected);
    let payload = match preparation.projected.as_ref() {
        Some(listing) => {
            // The cover travels the same way the files do, described here so
            // the device fetches and checks it before the adapter asks.
            let wanted: Vec<FileId> = listing.files.iter().copied().chain(listing.cover).collect();
            describe_files(&state.pool, org, &wanted)
                .await
                .map_err(|error| state.internal(&error.to_string()))?
                .into_iter()
                .map(manifest_for)
                .collect::<Result<Vec<_>, String>>()
                .map_err(|error| state.internal(&error))?
        }
        None => Vec::new(),
    };
    // The seller's own declaration, off the connection they linked. The device
    // holds no connection row, so this is the only place it can come from, and
    // without it the TPT adapter refuses every write — correctly, since the
    // copyright declaration is the seller's statement and not a constant a
    // connector may supply for them.
    let attestation = ConnectionFactsRepo::new(state.pool.clone())
        .authorship_for(org, leased.inventory.marketplace())
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .map(|record| Attestation {
            attested_by: record.name,
            attested_at_ms: record.attested_at.0,
        });

    Ok(Some(WorkOrder {
        lease: driven,
        attestation,
        // Set only where the claim took a stranded create out of its park.
        // Its presence is the whole of how a device tells a reconcile from an
        // ordinary run, because the operation cannot: a stranded create is
        // still a create.
        reconcile: reconcile_subject(leased),
        preparation,
        payload,
        server_now_ms: now.0,
        server_deadline_ms: now.0 + i64::from(LEASE_TTL_SECS) * 1_000,
        next_poll_ms: next_poll_ms(false),
    }))
}

/// The device settles what it claimed.
///
/// The epoch is the fence and the device id is the holder: a settle naming an
/// epoch the item has moved past, or arriving from a device other than the one
/// holding the lease, is refused, because the write it is asking for belongs
/// to a run it is not in. Both halves come from [`held_by`], which is the one
/// fence every device-reachable route here passes through.
pub(crate) async fn settle(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Json(body): Json<SettleEnvelope>,
) -> Result<StatusCode, APIError> {
    use tam_engine_driver::ports::ItemLedger;

    let leased = held_by(&state, context.org, &device, body.lease).await?;
    // `body.at_ms` is dropped rather than passed: `job_item` records one
    // instant and it is our receipt, so the device's assertion has nowhere on
    // this row to go until the split note's second column exists.
    PgLedger::for_device(state.pool.clone(), leased.job, device)
        .settle_item(&body.lease, &body.verdict, (state.wall)())
        .await
        .map_err(|error| match error {
            // The fence again, this time as the write's own answer: the reaper
            // can take the lease between the read above and this update.
            LedgerError::StaleLease => lease_refusal(),
            LedgerError::AttemptInFlight
            | LedgerError::MappingAlreadyBound
            | LedgerError::Refused { .. } => state.internal(&error.to_string()),
        })?;
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
    // This route serves bytes we hold. A marketplace-sourced file has none
    // here by construction, and its manifest tells the device where they are,
    // so asking us for them is a device on a path it should not be on rather
    // than a file that has gone missing.
    let FileBytes::Held { hash, .. } = described.bytes else {
        return Err(APIError::new(
            StatusCode::NOT_FOUND,
            APIErrorEntry::new(
                "this file names a marketplace resource rather than a stored blob; \
                 its bytes are fetched on the device under the seller's own session",
            )
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
        ));
    };
    let bytes = BlobRepo::new(state.pool.clone(), blobs.object_store(), blobs.kek.clone())
        .get(context.org, hash)
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
    let leased = LeaseRepo::new(state.pool.clone())
        .leased_item(org, lease.item)
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(lease_refusal)?;
    let holder = LeaseRepo::new(state.pool.clone())
        .holder(org, lease.item)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    if holder.as_deref() != Some(device) || leased.lease_epoch != lease.lease_epoch {
        return Err(lease_refusal());
    }
    Ok(leased)
}

/// What every fence failure on a device-reachable route answers with, in one
/// place so the read's refusal and the write's are the same refusal.
fn lease_refusal() -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(
            "that lease is held by another device, so this call is not yours to make",
        )
        .kind(APIErrorKind::Validation),
    )
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
        LedgerCall::Park { lease, blocked_on } => {
            ledger.park(&lease, &blocked_on).await?;
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
        LedgerCall::Renew { lease } => LedgerAnswer::Renewed {
            renewed: ledger.renew(&lease).await?,
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
        LedgerCall::HandBack { lease, at_ms } => {
            ledger.hand_back(&lease, Timestamp(at_ms)).await?;
            LedgerAnswer::Done
        }
    })
}

#[cfg(test)]
mod tests {
    use super::reconcile_subject;
    use tam_engine_driver::vocabulary::AttemptRef;
    use tam_marketplace::IdempotencyKey;
    use tam_types::{InventoryId, JobId, MappingId, OrgId, Uuid};

    const MAPPING: MappingId = MappingId(Uuid([0x04; 16]));
    const ATTEMPT: Uuid = Uuid([0x5A; 16]);

    /// A leased item with whatever the claim said about a stranded create.
    fn leased(
        stranded_attempt: Option<Uuid>,
        stranded_title: Option<&str>,
    ) -> tam_storage::LeasedItem {
        tam_storage::LeasedItem {
            org: OrgId(Uuid([0x01; 16])),
            item: tam_domain::JobItemId(Uuid([0x02; 16])),
            job: JobId(Uuid([0x03; 16])),
            mapping: MAPPING,
            inventory: InventoryId::Tes,
            idempotency_key: IdempotencyKey(Uuid([0x05; 16])),
            operation: tam_domain::ItemOperation::Create,
            lease_epoch: 7,
            attempt_count: 0,
            requires_bound_on: None,
            stranded_attempt,
            stranded_title: stranded_title.map(str::to_owned),
        }
    }

    /// The recorded title reaches the order, beside the attempt it identifies.
    ///
    /// This is the whole of what the device is given to search with: the
    /// attempt says which row to settle and the title says which listing to
    /// look for, and a subject carrying one without the other would send a
    /// device to enumerate a seller's catalogue for nothing.
    #[test]
    fn a_stranded_create_becomes_a_subject_carrying_its_recorded_title() {
        let subject = reconcile_subject(&leased(Some(ATTEMPT), Some("Fractions pack")))
            .expect("the claim named both, so the order is a reconcile");
        assert_eq!(
            subject,
            tam_engine_driver::vocabulary::ReconcileSubject {
                attempt: AttemptRef {
                    attempt: ATTEMPT,
                    mapping: MAPPING,
                },
                title: "Fractions pack".to_owned(),
            },
            "the title is carried through unaltered and the attempt names this item's mapping"
        );
    }

    /// Both or neither, in both directions.
    ///
    /// The claim already refuses to return an attempt whose intent named no
    /// title, so the first of these is unreachable through it; stating it here
    /// keeps the pairing a property of this function rather than of that query
    /// alone, because this is what builds the thing the device acts on.
    #[test]
    fn half_a_reconcile_is_not_a_reconcile() {
        assert_eq!(
            reconcile_subject(&leased(Some(ATTEMPT), None)),
            None,
            "an attempt with no recorded title identifies no listing, so the device runs the \
             ordinary create it still is"
        );
        assert_eq!(
            reconcile_subject(&leased(None, Some("Fractions pack"))),
            None,
            "and a title with no attempt names nothing to settle"
        );
        assert_eq!(
            reconcile_subject(&leased(None, None)),
            None,
            "an ordinary claim is not a reconcile"
        );
    }
}
