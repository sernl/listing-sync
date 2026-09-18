//! The wire vocabulary survives a round trip.
//!
//! These are the shapes the desktop client will import, and `tam-api` puts the
//! same types on the wire rather than restating them. A round trip is the
//! cheapest proof that the definition is genuinely one definition: a field that
//! serialises but cannot be read back is a client that parses the wrong shape,
//! and it would otherwise not be noticed until a device existed.

use tam_domain::{ItemOperation, ItemOutcome, JobItemId, StepBudget};
use tam_engine_driver::driver::VerifyPolicy;
use tam_engine_driver::import::{
    ContentType, Cover, FileName, Fingerprint, ImportPage, ListedResource, Locator, ObservedFile,
    ObservedResource, Reason, SkippedResource,
};
use tam_engine_driver::vocabulary::{
    AttemptIntent, AttemptRef, AttemptVerdict, BindDisposition, BudgetGrant, ClaimView, Committed,
    GrantKind, ItemPreparation, ItemVerdict, LandingEffect, LeaseRef, LeasedItem, LedgerAnswer,
    LedgerCall, LedgerError, NewAttempt, PayloadManifest, PayloadSource, PreflightStreak,
    ReconcileSubject, SettleEnvelope, WorkOrder,
};
use tam_marketplace::{
    CreateStrategy, FormId, IdempotencyKey, ImportedListing, LifecycleTransition, ListingState,
    RemoteLifecycle, RemoteListingId,
};
use tam_types::{
    ContentHash, CopyFormat, FailureCode, FailureDetail, FileKind, ImportedPrice, InventoryId,
    JobId, MappingId, OrgId, ScanOutcome, Timestamp, Uuid,
};

fn lease() -> LeaseRef {
    LeaseRef {
        org: OrgId(Uuid([0xAA; 16])),
        item: JobItemId(Uuid([0x11; 16])),
        lease_epoch: 7,
    }
}

/// A revise rather than a create, because it is the operation carrying the
/// most embedded vocabulary: a remote id, both lifecycle states and the
/// transition between them.
fn revision() -> ItemOperation {
    ItemOperation::Revise {
        subject: RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/9001".to_owned(),
        },
        transition: LifecycleTransition {
            from: ListingState::Draft,
            to: ListingState::Live,
        },
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a shape that will not serialise should stop the run here"
)]
fn round_trip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let json = serde_json::to_string(value).expect("the value serialises");
    serde_json::from_str(&json).expect("the value reads back")
}

/// The whole claim envelope, as the device receives it.
#[test]
fn a_full_work_envelope_round_trips() {
    let envelope = LeasedItem {
        org: OrgId(Uuid([0xAA; 16])),
        item: JobItemId(Uuid([0x11; 16])),
        job: JobId(Uuid([0x22; 16])),
        mapping: MappingId(Uuid([0x33; 16])),
        inventory: InventoryId::Tes,
        idempotency_key: IdempotencyKey(Uuid([0x55; 16])),
        operation: revision(),
        lease_epoch: 7,
        attempt_count: 2,
        requires_bound_on: Some(InventoryId::Tpt),
    };
    assert_eq!(
        round_trip(&envelope),
        envelope,
        "every field of the work order must survive the wire, or the client reads a \
         different item from the one the server leased"
    );
    assert_eq!(
        round_trip(&envelope).lease_ref(),
        lease(),
        "and the fence the device carries back must be the fence it was issued"
    );
}

/// The whole settle envelope, as the server receives it.
#[test]
fn a_full_settle_envelope_round_trips() {
    let verdict = ItemVerdict {
        outcome: ItemOutcome::Degraded,
        failure_code: Some(FailureCode::UploadRejected),
        failure_detail: Some(FailureDetail("the upload was refused".to_owned())),
    };
    assert_eq!(
        round_trip(&verdict),
        verdict,
        "the item verdict round trips"
    );

    let attempt = AttemptVerdict {
        state: "committed".to_owned(),
        failure_code: None,
        ambiguity: None,
        landing: LandingEffect::Landed {
            id: RemoteListingId::Tpt {
                product_id: 17_511_712,
            },
            lifecycle: RemoteLifecycle::Live {
                since: tam_types::Timestamp(1_756_000_000_000),
            },
        },
    };
    assert_eq!(
        round_trip(&attempt),
        attempt,
        "the attempt verdict carries the listing the write landed on, which is what the \
         ledger reconciles against"
    );

    let opened = NewAttempt {
        attempt: Uuid([0x7B; 16]),
        mapping: MappingId(Uuid([0x33; 16])),
        intent: AttemptIntent {
            body: serde_json::json!({ "title": "Fixture" }),
            hash: vec![0x0A; 32],
        },
    };
    assert_eq!(
        round_trip(&opened),
        opened,
        "the caller-minted attempt id is what makes a lost response recoverable, so it \
         must survive the wire exactly"
    );
}

/// The answers the ledger gives back, which the device branches on.
#[test]
fn every_ledger_answer_round_trips() {
    for grant in [BudgetGrant::Granted { used: 3 }, BudgetGrant::Exhausted] {
        assert_eq!(round_trip(&grant), grant, "{grant:?} round trips");
    }
    for kind in [GrantKind::Write, GrantKind::VerifyRead] {
        assert_eq!(round_trip(&kind), kind, "{kind:?} round trips");
    }
    for error in [
        LedgerError::AttemptInFlight,
        LedgerError::MappingAlreadyBound,
        LedgerError::StaleLease,
        LedgerError::Refused {
            detail: "the aggregate is inconsistent".to_owned(),
        },
    ] {
        assert_eq!(
            round_trip(&error),
            error,
            "{error:?} round trips: the device branches on the first two, so a variant \
             that did not read back would be branched on wrongly"
        );
    }
    let streak = PreflightStreak {
        failures: 2,
        edge_only: false,
    };
    assert_eq!(
        round_trip(&streak),
        streak,
        "the preflight streak round trips"
    );
    let reference = AttemptRef {
        attempt: Uuid([0x7B; 16]),
        mapping: MappingId(Uuid([0x33; 16])),
    };
    assert_eq!(
        round_trip(&reference),
        reference,
        "the attempt reference round trips"
    );
    let disposition = BindDisposition::DivergentLanding {
        existing: RemoteListingId::Tes {
            url: "https://www.tes.com/api/v2/resources/1".to_owned(),
        },
    };
    assert_eq!(
        round_trip(&disposition),
        disposition,
        "the bind disposition carries a remote id in three of its variants"
    );
}

fn leased() -> LeasedItem {
    LeasedItem {
        org: OrgId(Uuid([0xAA; 16])),
        item: JobItemId(Uuid([0x11; 16])),
        job: JobId(Uuid([0x22; 16])),
        mapping: MappingId(Uuid([0x33; 16])),
        inventory: InventoryId::Tes,
        idempotency_key: IdempotencyKey(Uuid([0x55; 16])),
        operation: revision(),
        lease_epoch: 7,
        attempt_count: 2,
        requires_bound_on: Some(InventoryId::Tpt),
    }
}

/// The whole claim view, as `tam-api` serves it and the desktop client reads
/// it, including the payload the server commits to before the bytes move.
#[test]
fn the_whole_claim_view_round_trips() {
    let order = WorkOrder {
        reconcile: None,
        attestation: None,
        lease: leased(),
        preparation: ItemPreparation {
            operation: revision(),
            // A revise carries no projection in this fixture; the projected
            // listing has its own round trip through `ProjectedListing`.
            projected: None,
            form: tam_marketplace::FormId(Uuid([0x09; 16])),
            strategy: tam_marketplace::CreateStrategy::DraftThenPublish {
                draft_state: tam_marketplace::RemoteLifecycleKind::Draft,
            },
            budget: tam_domain::StepBudget {
                actions_remaining: 32,
            },
            verify: VerifyPolicy {
                tries: 3,
                interval_ms: 2_000,
            },
        },
        payload: vec![PayloadManifest {
            file: tam_types::FileId(Uuid([0x0F; 16])),
            file_name: "abc.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            source: PayloadSource::ControlPlane {
                committed: Committed {
                    hash: tam_types::ContentHash([0x0A; 32]),
                    byte_len: 4_096,
                },
            },
        }],
        server_now_ms: 1_756_000_000_000,
        server_deadline_ms: 1_756_000_300_000,
        next_poll_ms: 10_000,
    };
    let view = ClaimView::Work(Box::new(order));
    assert_eq!(
        round_trip(&view),
        view,
        "the whole work order must survive the wire: the manifest is the server's \
         commitment to the bytes, so a field lost here is a transfer the device cannot check"
    );
    for idle in [
        ClaimView::Idle {
            next_poll_ms: 10_000,
        },
        ClaimView::Held {
            next_poll_ms: 30_000,
        },
    ] {
        assert_eq!(
            round_trip(&idle),
            idle,
            "the device branches on these three states, so each must read back as itself"
        );
    }
}

/// Both arms of where an upload's bytes come from.
///
/// The arms are what the device dispatches on before it reads anything, so an
/// arm that did not read back as itself would be a device fetching from the
/// wrong place, or refusing bytes it can reach. The marketplace arm is
/// exercised in both of its forms, because the distinction between them is
/// the whole integrity story: `Some` is a transfer checked against a previous
/// observation, and `None` is a first observation with nothing to check
/// against, which the wire must be able to say rather than approximate with a
/// zero digest.
#[test]
fn every_payload_source_round_trips() {
    let file = tam_types::FileId(Uuid([0x0F; 16]));
    let committed = Committed {
        hash: tam_types::ContentHash([0x0A; 32]),
        byte_len: 4_096,
    };
    let held = |entry: Option<&str>, expected: Option<Committed>| PayloadSource::Marketplace {
        marketplace: tam_types::Marketplace::Tes,
        resource: "13549126".to_owned(),
        entry: entry.map(str::to_owned),
        expected,
    };
    // Both axes, both ways: an entry names a file inside a bundle and an
    // expectation is a previous observation, and neither implies the other.
    // A bundle sent whole can still have been observed before, and a named
    // entry can still be a first sight of it.
    for source in [
        PayloadSource::ControlPlane { committed },
        held(None, None),
        held(None, Some(committed)),
        held(Some("worksheets/integers.pdf"), None),
        held(Some("worksheets/integers.pdf"), Some(committed)),
    ] {
        let manifest = PayloadManifest {
            file,
            file_name: "abc.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            source: source.clone(),
        };
        assert_eq!(
            round_trip(&manifest),
            manifest,
            "a manifest that lost its source on the wire is a device that cannot tell whose \
             bytes these are"
        );
    }

    let uncommitted = PayloadManifest {
        file,
        file_name: "abc.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        source: PayloadSource::Marketplace {
            marketplace: tam_types::Marketplace::Tes,
            resource: "13549126".to_owned(),
            entry: None,
            expected: None,
        },
    };
    assert_eq!(
        uncommitted.committed(),
        None,
        "a first observation carries no commitment, and the manifest says so rather than \
         offering a value a caller would verify against"
    );
    let held = PayloadManifest {
        source: PayloadSource::ControlPlane { committed },
        ..uncommitted
    };
    assert_eq!(
        held.committed(),
        Some(&committed),
        "bytes we hold are bytes we committed to before they moved"
    );
}

/// The published client keeps working, in both directions.
///
/// Desktop 0.1.3 is auto-updating and its own `PayloadManifest` declares
/// `hash` and `byte_len` as required top-level fields with no serde
/// attributes. Two things therefore have to hold at once during the
/// deprecation window, and a break in either is not a decode error somebody
/// notices in a log: the item has already been claimed, so it sits leased
/// until its lease expires while the device reports a failure every poll.
#[test]
fn the_pre_source_manifest_shape_still_works_in_both_directions() {
    let committed = Committed {
        hash: tam_types::ContentHash([0x0A; 32]),
        byte_len: 4_096,
    };
    let manifest = PayloadManifest {
        file: tam_types::FileId(Uuid([0x0F; 16])),
        file_name: "abc.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        source: PayloadSource::ControlPlane { committed },
    };

    // Forward: what we emit is still readable by a client that knows only the
    // old fields.
    let emitted: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&manifest).expect("the manifest serialises"))
            .expect("the emitted manifest is json");
    assert_eq!(
        emitted.get("hash"),
        Some(&serde_json::to_value(committed.hash).expect("a hash is a value")),
        "the pre-source hash is still emitted, or every 0.1.3 device fails to decode every \
         item carrying a file"
    );
    assert_eq!(
        emitted.get("byte_len"),
        Some(&serde_json::json!(4_096)),
        "and so is the length"
    );
    assert!(
        emitted.get("source").is_some(),
        "beside the new shape rather than instead of it"
    );

    // Backward: an envelope written before `source` existed still decodes
    // here, which is a new server reading an old recording and a new client
    // reading an old server.
    let old_shape = serde_json::json!({
        "file": manifest.file,
        "file_name": "abc.pdf",
        "content_type": "application/pdf",
        "hash": committed.hash,
        "byte_len": 4_096,
        "source": { "control_plane": { "committed": committed } },
    });
    let read: PayloadManifest =
        serde_json::from_value(old_shape).expect("the old shape still decodes");
    assert_eq!(
        read, manifest,
        "the duplicated old fields are ignored rather than fought over"
    );
}

/// A marketplace source emits no pre-source pair, and that is the deliberate
/// half of the shim.
///
/// There is no value that would let a 0.1.3 client succeed here — it has no
/// marketplace fetcher and our object store holds no bytes for the file — so
/// the only choice is how it fails. Emitting nothing fails it at the envelope,
/// which is louder and truer than a synthesised hash sending it to a payload
/// route that answers 404. The cost is that the item stays leased until its
/// lease expires, which is why nothing may emit a marketplace source until
/// every registered device reports an `app_version` at or past this change.
#[test]
fn a_marketplace_source_emits_no_legacy_pair() {
    let manifest = PayloadManifest {
        file: tam_types::FileId(Uuid([0x0F; 16])),
        file_name: "abc.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        source: PayloadSource::Marketplace {
            marketplace: tam_types::Marketplace::Tes,
            resource: "13549126".to_owned(),
            entry: None,
            expected: None,
        },
    };
    let emitted: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&manifest).expect("the manifest serialises"))
            .expect("the emitted manifest is json");
    assert_eq!(
        emitted.get("hash"),
        None,
        "a commitment nobody made is not restated as one: a hash here would be invented"
    );
    assert_eq!(emitted.get("byte_len"), None);
    assert_eq!(
        round_trip(&manifest),
        manifest,
        "and the current shape still round trips regardless"
    );
}

/// The settle, fenced on the lease it ran under.
#[test]
fn the_whole_settle_envelope_round_trips() {
    let envelope = SettleEnvelope {
        lease: lease(),
        verdict: ItemVerdict {
            outcome: ItemOutcome::Succeeded,
            failure_code: None,
            failure_detail: None,
        },
        at_ms: 1_756_000_042_000,
    };
    assert_eq!(
        round_trip(&envelope),
        envelope,
        "the fence the device carries back must be the fence it was issued"
    );
}

/// Every ledger call the device can make, and the answer it gets back.
///
/// The device reaches the ledger only through these, so a variant that did not
/// read back as itself would be a capability it silently loses.
#[test]
fn every_ledger_call_and_answer_round_trips() {
    let calls = vec![
        LedgerCall::ConnectionFor {
            lease: lease(),
            inventory: InventoryId::Tes,
        },
        LedgerCall::PreflightSucceeded { lease: lease() },
        LedgerCall::PreflightFailed {
            lease: lease(),
            edge_class: true,
        },
        LedgerCall::Park {
            lease: lease(),
            blocked_on: "reauth_required".to_owned(),
        },
        LedgerCall::OpenAttempt {
            lease: lease(),
            new: NewAttempt {
                attempt: Uuid([0x7B; 16]),
                mapping: MappingId(Uuid([0x33; 16])),
                intent: AttemptIntent {
                    body: serde_json::json!({ "title": "Fixture" }),
                    hash: vec![0x0A; 32],
                },
            },
            at_ms: 1_756_000_001_000,
        },
        LedgerCall::SettleAttempt {
            lease: lease(),
            attempt: AttemptRef {
                attempt: Uuid([0x7B; 16]),
                mapping: MappingId(Uuid([0x33; 16])),
            },
            verdict: AttemptVerdict {
                state: "committed".to_owned(),
                failure_code: None,
                ambiguity: None,
                landing: LandingEffect::None,
            },
            at_ms: 1_756_000_002_000,
        },
        LedgerCall::GateConnection {
            lease: lease(),
            inventory: InventoryId::Tes,
            at_ms: 1_756_000_003_000,
        },
        LedgerCall::HaltThisTenant {
            lease: lease(),
            inventory: InventoryId::Tes,
            reason: "the transition table demanded a halt".to_owned(),
            at_ms: 1_756_000_004_000,
        },
        LedgerCall::RequestGrant {
            lease: lease(),
            connection: tam_types::ConnectionId(Uuid([0x44; 16])),
            kind: GrantKind::Write,
            at_ms: 1_756_000_005_000,
        },
        LedgerCall::Renew { lease: lease() },
        LedgerCall::RecordEvent {
            lease: lease(),
            payload: tam_types::JobEventPayload::ItemLeased {
                worker: "device-a".to_owned(),
                lease_epoch: 7,
            },
            at_ms: 1_756_000_006_000,
        },
        LedgerCall::Notify {
            lease: lease(),
            event: tam_domain::SellerEvent::ReauthRequired,
            at_ms: 1_756_000_007_000,
        },
    ];
    for call in calls {
        assert_eq!(round_trip(&call), call, "{call:?} round trips");
        assert_eq!(
            round_trip(&call).lease(),
            &lease(),
            "every call names the lease it speaks for, because the server derives the \
             tenant from that rather than from anything else the caller says"
        );
    }

    let answers = vec![
        LedgerAnswer::Done,
        LedgerAnswer::Connection {
            connection: Some(tam_types::ConnectionId(Uuid([0x44; 16]))),
        },
        LedgerAnswer::Streak {
            streak: PreflightStreak {
                failures: 2,
                edge_only: true,
            },
        },
        LedgerAnswer::Bound {
            disposition: BindDisposition::Bound,
        },
        LedgerAnswer::Grant {
            grant: BudgetGrant::Granted { used: 3 },
        },
        // The two the interpreter branches on must survive as answers rather
        // than as transport faults, or a device retries what it must not.
        LedgerAnswer::Refused {
            error: LedgerError::AttemptInFlight,
        },
        // The refusal a create meets when the mapping was bound while it ran.
        // It travels as an answer for the same reason the other two do, though
        // the interpreter does something different with it: it abandons on
        // `AttemptInFlight` and on `StaleLease`, and settles on this one,
        // because a bound mapping is permanent for this item where the other
        // two may clear. A device that saw any of the three as a transport
        // fault would retry what it must not.
        LedgerAnswer::Refused {
            error: LedgerError::MappingAlreadyBound,
        },
        LedgerAnswer::Refused {
            error: LedgerError::StaleLease,
        },
    ];
    for answer in answers {
        assert_eq!(round_trip(&answer), answer, "{answer:?} round trips");
    }
}

/// The instant the device asserts is carried on exactly the calls whose ledger
/// method takes one, and on the settle.
#[test]
fn the_asserted_instant_travels_with_every_call_that_records_one() {
    let with = LedgerCall::RecordEvent {
        lease: lease(),
        payload: tam_types::JobEventPayload::ItemLeased {
            worker: "device-a".to_owned(),
            lease_epoch: 7,
        },
        at_ms: 1_756_000_006_000,
    };
    assert_eq!(
        round_trip(&with).asserted_at_ms(),
        Some(1_756_000_006_000),
        "a call that writes a dated row carries the seller's own instant"
    );
    for without in [
        LedgerCall::PreflightSucceeded { lease: lease() },
        // The heartbeat is the one that tempts: it is a device-originated
        // call like the rest, but it writes no `job_event` and no
        // `write_attempt`, so an instant on it would be recorded nowhere.
        LedgerCall::Renew { lease: lease() },
    ] {
        assert_eq!(
            round_trip(&without).asserted_at_ms(),
            None,
            "a call that writes no dated row asserts nothing, rather than carrying an \
             instant nobody will read: {without:?}"
        );
    }
}

/// A renew carries the gate's own instant and nothing else, and a body that
/// tries to name a duration is refused rather than ignored.
///
/// The refusal is the point. A renew whose `ttl_seconds` the server read
/// verbatim let a device set the length of its own lease, and a device naming
/// `i64::MAX` saturated the server's conversion into an expiry decades out —
/// a lease no reaper reclaims and a marketplace mutex nothing releases.
/// Dropping the field is what fixes it; refusing the field is what stops a
/// client from believing the field still works.
#[test]
fn a_renew_cannot_name_a_duration() {
    let accepted: LedgerCall = serde_json::from_value(serde_json::json!({
        "call": "renew",
        "lease": { "org": OrgId(Uuid([0x11; 16])), "item": JobItemId(Uuid([0x22; 16])),
                   "lease_epoch": 3 },
    }))
    .expect("the shape the device sends is accepted");
    assert!(
        matches!(accepted, LedgerCall::Renew { .. }),
        "and it is a renew: {accepted:?}"
    );

    let refused = serde_json::from_value::<LedgerCall>(serde_json::json!({
        "call": "renew",
        "lease": { "org": OrgId(Uuid([0x11; 16])), "item": JobItemId(Uuid([0x22; 16])),
                   "lease_epoch": 3 },
        "ttl_seconds": i64::MAX,
    }));
    assert!(
        refused.is_err(),
        "a duration on a renew is refused at the surface rather than silently dropped, so \
         a client that still sends one is told: {refused:?}"
    );
}

/// A work order's reconcile field is absent unless the claim put one there,
/// and survives a round trip either way.
///
/// The absent case is the one that matters. Its presence is what makes an
/// order a reconcile, so a field that appeared on every order — or that a
/// client without it decoded as anything but absent — would make every run
/// look like a reconcile to something branching on it.
#[test]
fn a_work_order_carries_a_reconcile_only_when_it_is_one() {
    let ordinary = serde_json::to_value(order_fixture(None)).expect("an ordinary order encodes");
    assert!(
        ordinary.get("reconcile").is_none(),
        "an ordinary order does not carry the field at all, so a client that has never \
         heard of it reads exactly what it read before: {ordinary}"
    );
    assert_eq!(
        round_trip(&order_fixture(None)).reconcile,
        None,
        "and it comes back absent"
    );

    let subject = ReconcileSubject {
        attempt: AttemptRef {
            attempt: Uuid([0x77; 16]),
            mapping: MappingId(Uuid([0x88; 16])),
        },
        title: "Fractions pack".to_owned(),
    };
    assert_eq!(
        round_trip(&order_fixture(Some(subject.clone()))).reconcile,
        Some(subject),
        "a reconcile order carries the attempt it is reconciling and the title that attempt \
         recorded it sent, which is what the device needs to search for the listing and to \
         settle the row rather than open another"
    );

    // An order encoded before the field existed decodes as an ordinary one
    // rather than failing, which is what `serde(default)` is for.
    let mut older = serde_json::to_value(order_fixture(None)).expect("encodes");
    older
        .as_object_mut()
        .expect("an object")
        .remove("reconcile");
    let decoded: WorkOrder = serde_json::from_value(older).expect("an older order still decodes");
    assert_eq!(decoded.reconcile, None);
}

/// One work order, with or without a reconcile.
fn order_fixture(reconcile: Option<ReconcileSubject>) -> WorkOrder {
    WorkOrder {
        reconcile,
        attestation: None,
        lease: leased(),
        preparation: ItemPreparation {
            operation: revision(),
            projected: None,
            form: FormId(Uuid([0x31; 16])),
            strategy: CreateStrategy::HaltOnAmbiguity,
            budget: StepBudget {
                actions_remaining: 32,
            },
            verify: VerifyPolicy {
                tries: 3,
                interval_ms: 1_000,
            },
        },
        payload: Vec::new(),
        server_now_ms: 1_756_000_000_000,
        server_deadline_ms: 1_756_000_600_000,
        next_poll_ms: 10_000,
    }
}

/// One PNG, small enough to be a cover and real enough to pass the check.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn cover() -> Cover {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(b"IHDR and not much else");
    Cover::encode(&png).expect("the fixture is a PNG within the ceiling")
}

fn listing() -> ImportedListing {
    ImportedListing {
        remote: RemoteListingId::Tes {
            url: "https://www.tes.com/teaching-resource/-13549794".to_owned(),
        },
        title: "Fractions practice".to_owned(),
        body: "A worksheet.".to_owned(),
        body_format: CopyFormat::Markdown,
        native: Vec::new(),
        rights: None,
        price: ImportedPrice::Free,
        state: Some(ListingState::Live),
    }
}

/// One observed file on each side of the unwrap decision.
///
/// `entry` present is a bundle that reduced to one file and `entry` absent is
/// the bundle whole, and the two are not interchangeable: the name and digest
/// describe the bytes handed onward, so a device that read one arm as the
/// other would upload a worksheet named as a zip or the reverse.
#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
fn observed(entry: Option<&str>) -> ObservedFile {
    ObservedFile {
        payload_file_name: FileName::new("worksheet.pdf").expect("a plain name"),
        payload_content_type: ContentType::new("application/pdf").expect("a media type"),
        kind: FileKind::Pdf,
        hash: ContentHash([0x5A; 32]),
        byte_len: 4_096,
        scan: ScanOutcome::Clean {
            at: Timestamp(1_756_000_000_000),
        },
        entry: entry.map(|path| FileName::new(path).expect("a plain entry name")),
    }
}

/// A page survives the round trip over both file arms, a fileless resource
/// and a skipped resource.
///
/// The device serialises this and `tam-api` deserialises it, and since C4a
/// they are one definition rather than two that agree today. A field that
/// serialises but cannot be read back is a page the server rejects after the
/// device has already done the work of reading the seller's shop.
#[test]
fn an_import_page_round_trips_over_both_file_arms_and_a_skip() {
    let page = ImportPage {
        run: Uuid([0x71; 16]),
        request: None,
        attempt: Some(3),
        receipt: Some(Uuid([0x7C; 16])),
        enumeration_complete: false,
        listed: None,
        resources: vec![
            ObservedResource {
                locator: Locator::from_resource_id(13_549_794),
                listing: listing(),
                file: Some(observed(None)),
                cover_png: Some(cover()),
                fingerprint: Some(Fingerprint::of_title("Fractions on a number line | TPT")),
            },
            ObservedResource {
                locator: Locator::from_resource_id(13_549_795),
                listing: listing(),
                file: Some(observed(Some("worksheet.pdf"))),
                cover_png: Some(cover()),
                fingerprint: None,
            },
            // The TPT arm: a listing read whose file this device could not
            // fetch, which must be representable or a TPT import cannot be
            // posted at all.
            ObservedResource {
                locator: Locator::from_resource_id(13_549_797),
                listing: listing(),
                file: None,
                cover_png: None,
                fingerprint: Some(Fingerprint::of_title("A poster")),
            },
        ],
        skipped: vec![SkippedResource {
            locator: Locator::from_resource_id(13_549_796),
            why: Reason::truncating("the bundle download was refused"),
        }],
        complete: true,
        failed: None,
    };

    let wire = serde_json::to_string(&page).expect("a page serialises");
    let decoded: ImportPage = serde_json::from_str(&wire).expect("and reads back");
    assert_eq!(decoded, page);
    assert_eq!(
        decoded.resources[0]
            .file
            .as_ref()
            .and_then(|file| file.entry.as_ref()),
        None,
        "the bundle-whole arm keeps its absent entry, which is what says no unwrap happened"
    );
    assert!(
        decoded.resources[1]
            .file
            .as_ref()
            .is_some_and(|file| file.entry.is_some()),
        "and the unwrapped arm keeps its entry, which is what says one did"
    );
}

/// The selection step's page: the shop as the enumeration saw it, and not one
/// resource read.
#[test]
fn a_listed_page_round_trips() {
    let page = ImportPage {
        run: Uuid([0x71; 16]),
        request: None,
        attempt: Some(3),
        receipt: Some(Uuid([0x7C; 16])),
        enumeration_complete: false,
        listed: Some(vec![ListedResource {
            locator: Locator::from_resource_id(13_549_794),
            title: "Fractions on a number line".to_owned(),
            price_minor: Some(450),
            currency: Some("GBP".to_owned()),
            state: Some(ListingState::Live),
        }]),
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: false,
        failed: None,
    };
    let wire = serde_json::to_string(&page).expect("a page serialises");
    let decoded: ImportPage = serde_json::from_str(&wire).expect("and reads back");
    assert_eq!(decoded, page);
}

/// The page's guarantees are re-checked on the way in, not just on the way out.
///
/// The server is the side that has never seen the device, so a page is only as
/// trustworthy as what its types refuse. Each of these is a payload trying to
/// arrive in a field that describes one.
#[test]
fn a_page_field_that_carries_a_payload_is_refused_on_the_way_in() {
    let smuggled: Result<Cover, _> =
        tam_engine_driver::import::base64(b"%PDF-1.7 a payload").try_into();
    assert!(
        smuggled.is_err(),
        "a cover is checked for its magic bytes on decode, so the field cannot carry a pdf"
    );

    let oversized: Result<Cover, _> = {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.resize(tam_engine_driver::import::COVER_BYTES_MAX + 1, 0);
        tam_engine_driver::import::base64(&png).try_into()
    };
    assert!(
        oversized.is_err(),
        "and the ceiling holds on the way in, so it cannot carry a large one either"
    );

    let separator: Result<FileName, _> = "../../etc/passwd".to_owned().try_into();
    assert!(
        separator.is_err(),
        "a file name is a name: a separator would make it a path"
    );

    let long: Result<FileName, _> = "n"
        .repeat(tam_engine_driver::import::NAME_MAX + 1)
        .try_into();
    assert!(long.is_err(), "and it is bounded");

    let traversal: Result<FileName, _> = "..".to_owned().try_into();
    assert!(
        traversal.is_err(),
        "`..` carries no separator, so the separator rule never saw it, and it is a traversal \
         component wherever a consumer joins it to a path"
    );

    let media: Result<ContentType, _> =
        tam_engine_driver::import::base64(b"%PDF-1.7 a payload").try_into();
    assert!(
        media.is_err(),
        "a media type is a type and a subtype, so the field cannot carry an encoded payload"
    );

    let mid_padded: Result<Cover, _> = "iVBORw0KGgo=Zg==Zg==Zg==".to_owned().try_into();
    assert!(
        mid_padded.is_err(),
        "padding outside the final chunk is refused, so a cover cannot hold an encoding our own \
         decoder reads and a browser's atob rejects"
    );
}

/// A page from a newer device still decodes, and a page missing a field a
/// newer server added still decodes.
///
/// The direction that hurts is a required field added here against an
/// already-shipped desktop: that device fails to post after walking the
/// seller's entire shop, and every retry fails identically. The convention in
/// the module's documentation is that any field added after the first shipped
/// desktop takes `serde(default)`; these two assertions are what hold that
/// convention to something rather than leaving it a sentence.
#[test]
fn a_page_decodes_across_a_version_skew_in_both_directions() {
    let page = ImportPage {
        run: Uuid([0x71; 16]),
        request: None,
        attempt: Some(3),
        receipt: Some(Uuid([0x7C; 16])),
        enumeration_complete: false,
        listed: None,
        resources: Vec::new(),
        skipped: Vec::new(),
        complete: true,
        failed: None,
    };
    let mut encoded: serde_json::Value =
        serde_json::to_value(&page).expect("a page serialises to json");
    let object = encoded.as_object_mut().expect("a page is an object");

    object.insert("pages_walked".to_owned(), serde_json::Value::from(17_i64));
    let newer: ImportPage = serde_json::from_value(encoded.clone())
        .expect("a field this server does not know is dropped rather than refused");
    assert_eq!(newer, page);

    let object = encoded.as_object_mut().expect("a page is an object");
    object.remove("pages_walked");
    let decoded: ImportPage =
        serde_json::from_value(encoded).expect("and the page without it is unchanged");
    assert_eq!(decoded, page);
}

/// A page from before the fence decodes, and decodes as unfenced.
///
/// Both halves matter. The decode has to succeed, because the direction that
/// hurts is a newer server against an older device — a required field would
/// make an already-shipped build fail to post after walking the seller's
/// whole shop. And the absence has to survive as an absence: the server
/// refuses an unfenced catalogue page before any effect, and a default
/// attempt would be a fence nobody granted, indistinguishable from one a
/// taken-over phone posted.
#[test]
fn a_page_from_before_the_fence_decodes_as_unfenced() {
    let old = serde_json::json!({
        "run": "71717171-7171-7171-7171-717171717171",
        "resources": [],
        "skipped": [],
        "complete": false,
    });
    let decoded: ImportPage =
        serde_json::from_value(old).expect("a page from before the fence still decodes");
    assert_eq!(
        decoded.attempt, None,
        "an absent attempt stays absent rather than defaulting to a fence"
    );
    assert_eq!(decoded.receipt, None);
    assert!(
        !decoded.enumeration_complete,
        "and an old page claims nothing about whether the shop was walked"
    );
}
